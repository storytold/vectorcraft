//! Barcodes and QR Codes: Object → Barcode and the Barcode panel.
//!
//! A barcode is a group carrying a [`BarcodeSpec`] in its extras, like a graph: its children — the
//! bars or the modules, and the human readable line — are drawn from the spec, and changing the
//! spec redraws them in place. The encoding itself lives in `vectorcraft-barcode`, which knows
//! nothing about documents; everything here is about turning what it returns into art.
//!
//! Sizes come out in millimetres, which is the unit every symbology standard is written in, and
//! are converted once, here. The document is in points, so a 0.33 mm module prints at 0.94 pt.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_barcode::{Bar, BarcodeSpec, Code, Ecc, HriAlign, Symbology};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Document, Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{Affine, BezPath, PathData, Point};

use super::create::{new_type_para, new_type_style};
use super::typecmd::refresh_bounds;
use super::{CommandSpec, EngineError, Result, Session, bad, cmd, has_doc, has_selection, id_param, str_param};

/// Millimetres to document points.
const PT_PER_MM: f64 = 72.0 / 25.4;

/// The digits' height as a fraction of the font size they are set in.
///
/// A font's cap height is its own, and the encoder can only say how tall the human readable line
/// should be. OCR-B, which GS1 §5.2.4 recommends for it, has a cap height of 0.70 em, and the
/// faces a document without it falls back to are within a few hundredths of that.
const CAP_HEIGHT_RATIO: f64 = 0.70;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "barcode.create",
            "Barcode",
            [],
            None,
            "{symbology: ean13|ean8|upcA|upcE|ean128|code128|code128A|code128B|code128C|code39|code93|itf14|itf25|qrCode, data, x?, y? (default: the centre of the first artboard), magnification?: the module size multiplier (1 is the standard one), moduleMm?, heightMm?, ratio?: the wide-to-narrow ratio of Code 39 and Interleaved 2 of 5, text?: the human readable line, outlineText?: draw its digits as outlines (default: true), background?: a white box of the symbol's standard size behind it (default: true), quietExtra?: extra quiet zone in modules, bearer?: an ITF-14's bearer bars, ecc?: low|medium|quartile|high, version?: 1-40, mask?: 0-7, font?, fontStyle?} → {id, data: the digits the symbol carries, with the check digit when it was added}",
            has_doc,
            create
        ),
        cmd!(
            "barcode.check",
            "Check Barcode",
            [],
            None,
            "{id?, …as barcode.create} → {ok: bool, data: the digits the symbol would carry, widthMm, heightMm, message: what is wrong when it is not ok}; asks the encoder rather than drawing anything",
            has_doc,
            check
        ),
        cmd!(
            "barcode.createBatch",
            "Barcodes",
            [],
            None,
            "{data: one code to a line, columns?: how many before the next row (10), gapX?: millimetres between the codes of a row (5), gapY?: millimetres between rows (5), x?, y? (default: the top-left corner of the first artboard), …as barcode.create} → {ids: [..], count, skipped: [{data, message}]}",
            has_doc,
            create_batch
        ),
        cmd!(
            "barcode.set",
            "Barcode Options",
            [],
            None,
            "{id?, …as barcode.create} change the selected barcode and draw it again; no options → {id, data, symbology, …the current ones}",
            has_selection,
            set
        ),
    ]
}

/// A `f64` a command carries, when it carries one.
fn opt_f64(p: &Value, key: &str) -> Option<f64> {
    p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite())
}

fn opt_bool(p: &Value, key: &str) -> Option<bool> {
    p.get(key).and_then(Value::as_bool)
}

fn opt_u8(p: &Value, key: &str) -> Option<u8> {
    p.get(key).and_then(Value::as_u64).and_then(|v| u8::try_from(v).ok())
}

/// Read the options a command carries onto `spec`.
///
/// Only the keys the command actually carries are changed, so `barcode.set` can adjust one thing
/// without a caller having to repeat the rest.
fn read_spec(spec: &mut BarcodeSpec, p: &Value, what: &str) -> Result<()> {
    if let Some(name) = str_param(p, "symbology") {
        spec.symbology = Symbology::from_id(name).ok_or_else(|| bad(what, format!("unknown barcode type `{name}`")))?;
    }
    if let Some(data) = str_param(p, "data") {
        spec.data = data.to_string();
    }
    if let Some(v) = opt_f64(p, "magnification") {
        spec.magnification = v.clamp(0.25, 10.0);
    }
    if p.get("moduleMm").is_some() {
        spec.module_mm = opt_f64(p, "moduleMm");
    }
    if p.get("heightMm").is_some() {
        spec.height_mm = opt_f64(p, "heightMm");
    }
    if let Some(v) = opt_bool(p, "text") {
        spec.text = v;
    }
    if let Some(v) = opt_f64(p, "quietExtra") {
        spec.quiet_extra = v.clamp(0.0, 100.0);
    }
    if let Some(v) = opt_bool(p, "bearer") {
        spec.bearer = v;
    }
    if let Some(v) = opt_f64(p, "ratio") {
        spec.ratio = Some(v.clamp(2.0, 3.0));
    }
    if let Some(name) = str_param(p, "ecc") {
        spec.ecc = match name.to_ascii_lowercase().as_str() {
            "low" | "l" => Ecc::Low,
            "medium" | "m" => Ecc::Medium,
            "quartile" | "q" => Ecc::Quartile,
            "high" | "h" => Ecc::High,
            other => return Err(bad(what, format!("unknown error correction level `{other}`"))),
        };
    }
    if let Some(v) = opt_u8(p, "version") {
        spec.version = match v {
            0 => None,
            1..=40 => Some(v),
            other => return Err(bad(what, format!("a QR Code version is 1 to 40, not {other}"))),
        };
    }
    if p.get("mask").is_some() {
        spec.mask = opt_u8(p, "mask").map(|m| m % 8);
    }
    if p.get("font").is_some() {
        spec.font = str_param(p, "font").map(str::to_string);
    }
    if p.get("fontStyle").is_some() {
        spec.font_style = str_param(p, "fontStyle").map(str::to_string);
    }
    if let Some(v) = opt_bool(p, "outlineText") {
        spec.outline_text = v;
    }
    if let Some(v) = opt_bool(p, "background") {
        spec.background = v;
    }
    Ok(())
}

/// What the encoder made of a spec, and the data it ended up carrying — which is not what a caller
/// typed when a check digit was missing.
fn encoded(spec: &BarcodeSpec) -> Result<(Code, String)> {
    let code = vectorcraft_barcode::encode(spec).map_err(|e| EngineError::Other(e.to_string()))?;
    let data = match &code {
        // The data of a finished symbol is its human readable line, which carries every digit the
        // symbol encodes; a caller who typed twelve of thirteen gets the thirteenth back.
        Code::Linear(symbol) => {
            symbol.human_readable.as_ref().map(|h| h.parts.iter().map(|p| p.text.as_str()).collect::<String>()).unwrap_or_else(|| spec.data.clone())
        }
        Code::Matrix(_) => spec.data.clone(),
    };
    Ok((code, data))
}

fn create(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "barcode.create";
    let mut spec = BarcodeSpec::default();
    read_spec(&mut spec, p, C)?;
    if spec.data.trim().is_empty() {
        // A new barcode shows something rather than refusing to draw.
        spec.data = spec.symbology.sample_data().to_string();
    }
    let (code, data) = encoded(&spec)?;
    let (w, h) = code.size_mm();
    let (w, h) = (w * PT_PER_MM, h * PT_PER_MM);
    let parent = s.doc()?.target_parent()?;
    let style = s_style(s, &spec);
    let id = s.edit("Barcode", |d, sel| {
        let origin = match (opt_f64(p, "x"), opt_f64(p, "y")) {
            (Some(x), Some(y)) => Point::new(x, y),
            _ => {
                let centre = d.artboards.first().map(|a| a.rect.center()).unwrap_or(Point::ZERO);
                Point::new(centre.x - w / 2.0, centre.y - h / 2.0)
            }
        };
        let id = place(d, parent, &spec, &code, &data, origin, &style)?;
        sel.set([id]);
        Ok(id)
    })?;
    Ok(json!({ "id": id.0, "data": data }))
}

/// The character style the human readable line is set in: the spec's font, else the one new type
/// in this document gets.
fn s_style(s: &Session, spec: &BarcodeSpec) -> CharStyle {
    let mut style = new_type_style(s, &json!({}));
    // The human readable line is set in the face the standards name for it — the machine's OCR-B —
    // unless the caller asked for another one, and in whatever the document is set to when there is
    // no OCR-B to be had.
    if let Some(font) = spec.font.clone().or_else(|| vectorcraft_text::FontDb::global().ocr_b()) {
        style.font_family = font;
    }
    if let Some(face) = &spec.font_style {
        style.font_style = face.clone();
    }
    style.fill = Paint::solid(Color::BLACK);
    style.stroke = Paint::None;
    style.stroke_width = 0.0;
    style
}

/// The barcode object the selection names, or the one the selection is inside.
fn target(s: &Session, p: &Value, what: &str) -> Result<NodeId> {
    let st = s.doc()?;
    let start = id_param(p, "id").into_iter().chain(st.selection.objects.iter().copied());
    for id in start {
        let mut cur = Some(id);
        while let Some(c) = cur {
            if st.doc.node(c).is_some_and(|n| n.extras().barcode.is_some()) {
                return Ok(c);
            }
            cur = st.doc.parent_of(c);
        }
    }
    Err(bad(what, "select a barcode"))
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "barcode.set";
    let id = target(s, p, C)?;
    let mut spec = s.doc()?.doc.node(id).and_then(|n| n.extras().barcode.as_deref().cloned()).ok_or(EngineError::NoNode(id))?;
    // A query: the command carried no options at all.
    let query = p.as_object().is_none_or(|o| o.keys().all(|k| k == "id"));
    if query {
        let (code, data) = encoded(&spec)?;
        let (w, h) = code.size_mm();
        return Ok(json!({
            "id": id.0,
            "spec": spec,
            "data": data,
            "symbology": spec.symbology.id(),
            "widthMm": w,
            "heightMm": h,
        }));
    }
    read_spec(&mut spec, p, C)?;
    let (code, data) = encoded(&spec)?;
    // The bounds start at the old art's own top-left, which without the white box is a quiet zone
    // (or a digit) inside the symbol: subtract that to get back to the origin the art started at.
    let old = s.doc()?.doc.node(id).and_then(|n| n.extras().barcode.as_deref().cloned()).ok_or(EngineError::NoNode(id))?;
    let corner = top_left(s, id);
    let style = s_style(s, &spec);
    let old_style = s_style(s, &old);
    s.edit("Barcode Options", |d, _| {
        let (dx, dy) = art_offset(d, &old, &old_style)?;
        draw(d, id, &spec, &code, Point::new(corner.x - dx, corner.y - dy), style.clone())
    })?;
    Ok(json!({ "id": id.0, "data": data }))
}

/// Where the barcode's top-left corner is now: its children's bounds, or the origin when it has
/// none. Redrawing puts the new art where the old art was.
fn top_left(s: &Session, id: NodeId) -> Point {
    s.doc().ok().and_then(|st| st.doc.bounds_of(&[id], false)).map_or(Point::ZERO, |r| Point::new(r.x0, r.y0))
}

/// Draw `code` into the barcode object `id`, from its top-left corner.
fn draw(d: &mut Document, id: NodeId, spec: &BarcodeSpec, code: &Code, origin: Point, style: CharStyle) -> Result<()> {
    let children = art_children(d, spec, code, origin, &style);
    let node = d.node_mut(id).ok_or(EngineError::NoNode(id))?;
    if let Some(slot) = node.children_mut() {
        *slot = children;
    }
    node.extras_mut().barcode = Some(Box::new(spec.clone()));
    Ok(())
}

/// The art of `code`, as a barcode object whose origin is `origin` draws it.
fn art_children(d: &mut Document, spec: &BarcodeSpec, code: &Code, origin: Point, style: &CharStyle) -> Vec<Arc<Node>> {
    let mut children: Vec<Arc<Node>> = Vec::new();
    match code {
        Code::Linear(symbol) => {
            let bars = bars_path(&symbol.bars, origin);
            if !bars.is_empty() {
                children.push(Arc::new(Node::path(d.alloc_id(), bars, Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0))));
            }
            children.extend(hri_nodes(d, symbol.human_readable.as_ref(), origin, style, spec.outline_text));
            if spec.background {
                let (w, h) = box_mm(&children, origin, symbol.width_mm, symbol.height_mm);
                children.insert(0, white_box(d, origin, w, h));
            }
        }
        Code::Matrix(matrix) => {
            let quiet = matrix.quiet as f64 * matrix.module_mm;
            let mut bp = BezPath::new();
            for run in matrix.runs() {
                let x0 = origin.x + (quiet + run.x as f64 * matrix.module_mm) * PT_PER_MM;
                let y0 = origin.y + (quiet + run.y as f64 * matrix.module_mm) * PT_PER_MM;
                let x1 = x0 + run.width as f64 * matrix.module_mm * PT_PER_MM;
                let y1 = y0 + run.height as f64 * matrix.module_mm * PT_PER_MM;
                rectangle(&mut bp, x0, y0, x1, y1);
            }
            children.push(Arc::new(Node::path(
                d.alloc_id(),
                PathData::from_bezpath(&bp),
                Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
            )));
            if spec.background {
                let side = matrix.width_mm();
                let (w, h) = box_mm(&children, origin, side, side);
                children.insert(0, white_box(d, origin, w, h));
            }
        }
    }
    children
}

/// How far the art `spec` draws starts from the object's origin: nothing with the white box, which
/// starts exactly there, and otherwise the quiet zone — or the digits standing left of the first
/// bar (UPC-A and UPC-E). Redrawing at the bounds rather than at the origin would move the symbol
/// one offset across the artboard on every edit.
///
/// The nodes are built and thrown away; `d` only lends its id counter.
fn art_offset(d: &mut Document, spec: &BarcodeSpec, style: &CharStyle) -> Result<(f64, f64)> {
    if spec.background {
        return Ok((0.0, 0.0));
    }
    let (code, _) = encoded(spec)?;
    let art = art_children(d, spec, &code, Point::ZERO, style);
    let (mut left, mut top) = (f64::INFINITY, f64::INFINITY);
    for n in &art {
        if let Some(b) = n.geometric_bounds().filter(|b| b.width() > 0.0 && b.height() > 0.0) {
            left = left.min(b.x0);
            top = top.min(b.y0);
        }
    }
    Ok(if left.is_finite() { (left, top) } else { (0.0, 0.0) })
}

/// One path holding every bar, so that a symbol is one object a caller can fill, move or recolour
/// rather than a hundred.
fn bars_path(bars: &[Bar], origin: Point) -> PathData {
    let mut bp = BezPath::new();
    for bar in bars {
        let x0 = origin.x + bar.x * PT_PER_MM;
        let y0 = origin.y + bar.y * PT_PER_MM;
        let x1 = x0 + bar.width * PT_PER_MM;
        let y1 = y0 + bar.height * PT_PER_MM;
        rectangle(&mut bp, x0, y0, x1, y1);
    }
    PathData::from_bezpath(&bp)
}

/// The size of the white box: the size the standard gives the symbol, grown when the art reaches
/// past it. The digits' outlines can sit a few hundredths of a millimetre below the nominal
/// baseline in a font whose figures are not exactly cap height, and a box that leaves the ink
/// outside it would not be doing its job.
fn box_mm(art: &[Arc<Node>], origin: Point, width_mm: f64, height_mm: f64) -> (f64, f64) {
    let (mut right, mut bottom) = (origin.x + width_mm * PT_PER_MM, origin.y + height_mm * PT_PER_MM);
    for n in art {
        if let Some(b) = n.geometric_bounds().filter(|b| b.width() > 0.0 && b.height() > 0.0) {
            right = right.max(b.x1);
            bottom = bottom.max(b.y1);
        }
    }
    (((right - origin.x) / PT_PER_MM).max(width_mm), ((bottom - origin.y) / PT_PER_MM).max(height_mm))
}

/// The white box the standard puts round the symbol: its full size, quiet zones and human readable
/// line included, which is what keeps the quiet zones clear over a photograph or a coloured carton.
fn white_box(d: &mut Document, origin: Point, width_mm: f64, height_mm: f64) -> Arc<Node> {
    let mut bp = BezPath::new();
    rectangle(&mut bp, origin.x, origin.y, origin.x + width_mm * PT_PER_MM, origin.y + height_mm * PT_PER_MM);
    Arc::new(Node::path(d.alloc_id(), PathData::from_bezpath(&bp), Appearance::basic(Paint::solid(Color::WHITE), Paint::None, 0.0)))
}

fn rectangle(bp: &mut BezPath, x0: f64, y0: f64, x1: f64, y1: f64) {
    if !(x1 > x0 && y1 > y0) {
        return;
    }
    bp.move_to(Point::new(x0, y0));
    bp.line_to(Point::new(x1, y0));
    bp.line_to(Point::new(x1, y1));
    bp.line_to(Point::new(x0, y1));
    bp.close_path();
}

/// The human readable line's text objects.
///
/// Each part is set at the size its height asks for and shrunk when the font it lands in would run
/// past the character cells the digits sit under; the anchor is the edge `at_mm` gave, so a digit
/// printed outside the symbol keeps its distance from the guard bars whatever the font.
fn hri_nodes(d: &mut Document, hri: Option<&vectorcraft_barcode::HumanReadable>, origin: Point, style: &CharStyle, outline: bool) -> Vec<Arc<Node>> {
    let Some(hri) = hri else { return Vec::new() };
    let baseline = origin.y + (hri.top_mm + hri.height_mm) * PT_PER_MM;
    let mut out = Vec::new();
    for part in &hri.parts {
        let limit = part.width_mm * PT_PER_MM;
        let mut size = hri.height_mm * PT_PER_MM / CAP_HEIGHT_RATIO;
        let mut laid = layout_part(part, style, size);
        let width = laid.cached_bounds.map_or(0.0, |b| b.width());
        // A font wider than the character cells the digits sit under would run past them and stop
        // lining up with the bars: shrink it rather than let the line drift.
        if limit > 0.0 && width > limit {
            size = (size * limit / width).max(0.05);
            laid = layout_part(part, style, size);
        }
        let width = laid.cached_bounds.map_or(0.0, |b| b.width());
        let x = origin.x + part.at_mm * PT_PER_MM
            - match part.align {
                HriAlign::Left => 0.0,
                HriAlign::Center => width / 2.0,
                HriAlign::Right => width,
            };
        laid.xf = Affine::translate(Point::new(x, baseline).to_vec2());
        let node = Node::new(d.alloc_id(), NodeKind::Text(Box::new(laid)));
        // Outlines carry no font dependency, so the art prints the same where the font is missing,
        // which is what a packaging shop sending the file on to a printer wants by default.
        match outline.then(|| vectorcraft_render::effects::outline_text(&node)).flatten() {
            // The outlined group carries the id of the type it came from, and so does every path
            // inside it: re-number the tree or the art collides with its own ancestors.
            Some(glyphs) => out.push(Arc::new(d.reid(&glyphs))),
            None => out.push(Arc::new(node)),
        }
    }
    out
}

/// One run of the human readable line, laid out and measured, before it is placed.
fn layout_part(part: &vectorcraft_barcode::HriPart, style: &CharStyle, size: f64) -> TextObject {
    let mut st = style.clone();
    st.size = size.max(0.05);
    let mut t = TextObject::point(Point::ZERO, &part.text, st);
    t.para = new_type_para();
    refresh_bounds(&mut t);
    t
}

/// Ask the encoder whether a spec draws anything, without drawing it: what the panel's Check
/// button and an agent about to place a batch both want.
fn check(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "barcode.check";
    let mut spec = BarcodeSpec::default();
    // Without a symbology of its own, check what is on the canvas: the panel asks this way.
    if p.get("symbology").is_none()
        && let Ok(id) = target(s, p, C)
        && let Some(selected) = s.doc()?.doc.node(id).and_then(|n| n.extras().barcode.as_deref())
    {
        spec = selected.clone();
    }
    read_spec(&mut spec, p, C)?;
    match encoded(&spec) {
        Ok((code, data)) => {
            let (w, h) = code.size_mm();
            Ok(json!({
                "ok": true,
                "symbology": spec.symbology.id(),
                "data": data,
                "widthMm": w,
                "heightMm": h,
            }))
        }
        Err(e) => Ok(json!({ "ok": false, "symbology": spec.symbology.id(), "message": e.to_string() })),
    }
}

/// Draw one barcode object: a group carrying its spec, with the art under it.
fn place(d: &mut Document, parent: Option<NodeId>, spec: &BarcodeSpec, code: &Code, data: &str, origin: Point, style: &CharStyle) -> Result<NodeId> {
    let id = d.alloc_id();
    let mut group = Node::group(id, Vec::new());
    group.name = Some(format!("{} {}", spec.symbology.label(), data));
    d.insert(parent, usize::MAX, group)?;
    draw(d, id, spec, code, origin, style.clone())?;
    Ok(id)
}

/// Draw one code a line as a grid: left to right from the artboard's top-left corner, wrapping
/// after `columns` of them, so a list pasted in from a spreadsheet lands as art rather than as a
/// pile.
fn create_batch(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "barcode.createBatch";
    let mut spec = BarcodeSpec::default();
    read_spec(&mut spec, p, C)?;
    let codes: Vec<String> = str_param(p, "data").unwrap_or("").lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
    if codes.is_empty() {
        return Err(bad(C, "no codes to draw"));
    }
    let columns = opt_u8(p, "columns").map_or(10usize, |c| usize::from(c).clamp(1, 200));
    // Millimetres in the description and the panel, points in the layout, which is in points.
    let (gap_x, gap_y) =
        (opt_f64(p, "gapX").unwrap_or(5.0).clamp(0.0, 1000.0) * PT_PER_MM, opt_f64(p, "gapY").unwrap_or(5.0).clamp(0.0, 1000.0) * PT_PER_MM);

    // Encode every line first: one the symbology cannot carry is reported rather than drawn, and
    // the rest still land.
    let mut drawn: Vec<(BarcodeSpec, Code, String, f64, f64)> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    for data in codes {
        let one = BarcodeSpec { data: data.clone(), ..spec.clone() };
        match encoded(&one) {
            Ok((code, carried)) => {
                let (w, h) = code.size_mm();
                drawn.push((one, code, carried, w * PT_PER_MM, h * PT_PER_MM));
            }
            Err(e) => skipped.push(json!({ "data": data, "message": e.to_string() })),
        }
    }
    if drawn.is_empty() {
        return Err(bad(C, "no line in the data is a code this symbology can carry"));
    }

    let parent = s.doc()?.target_parent()?;
    let style = s_style(s, &spec);
    let corner = s.doc()?.doc.artboards.first().map_or(Point::ZERO, |a| Point::new(a.rect.x0, a.rect.y0));
    let (left, top) = (opt_f64(p, "x").unwrap_or(corner.x), opt_f64(p, "y").unwrap_or(corner.y));
    let ids = s.edit("Barcodes", |d, sel| {
        let (mut x, mut y, mut row_height) = (left, top, 0.0);
        let mut column = 0usize;
        let mut ids = Vec::with_capacity(drawn.len());
        for (one, code, carried, width, height) in &drawn {
            if column == columns {
                column = 0;
                x = left;
                y += row_height + gap_y;
                row_height = 0.0;
            }
            ids.push(place(d, parent, one, code, carried, Point::new(x, y), &style)?);
            x += width + gap_x;
            row_height = row_height.max(*height);
            column += 1;
        }
        sel.set(ids.iter().copied());
        Ok(ids)
    })?;
    Ok(json!({
        "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(),
        "count": ids.len(),
        "skipped": skipped,
    }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_color::Paint;
    use vectorcraft_doc::NodeKind;

    use crate::{NodeId, Session};

    fn new_doc() -> Session {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width": 800, "height": 800})).ok();
        s
    }

    fn make(s: &mut Session, p: serde_json::Value) -> (NodeId, serde_json::Value) {
        let made = s.execute("barcode.create", &p).expect("barcode.create");
        (NodeId(made["id"].as_u64().expect("id")), made)
    }

    fn children(s: &Session, id: NodeId) -> Vec<std::sync::Arc<vectorcraft_doc::Node>> {
        s.doc().unwrap().doc.node(id).and_then(|n| n.children()).map(|c| c.to_vec()).unwrap_or_default()
    }

    /// A barcode is one group carrying its spec, with every bar in a single path and the human
    /// readable line as type. Asked for as type and without the box, which is what this test is
    /// about; the defaults are the subject of the two tests below.
    #[test]
    fn a_barcode_is_a_group_with_its_spec() {
        let mut s = new_doc();
        let (id, made) = make(&mut s, json!({"symbology": "ean13", "data": "4006381333931", "background": false, "outlineText": false}));
        assert_eq!(made["data"], "4006381333931");
        let node = s.doc().unwrap().doc.node(id).unwrap().clone();
        let spec = node.extras().barcode.as_deref().expect("the spec is kept");
        assert_eq!(spec.symbology, vectorcraft_barcode::Symbology::Ean13);
        assert_eq!(spec.data, "4006381333931");
        let kids = children(&s, id);
        assert_eq!(kids.iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).count(), 1, "the bars are one path");
        assert_eq!(kids.iter().filter(|n| matches!(n.kind, NodeKind::Text(_))).count(), 13, "one object a digit");
        assert!(node.name.as_deref().is_some_and(|n| n.starts_with("EAN-13 ")));
    }

    /// A symbol is the size its standard gives it. The drawn art is the bar pattern itself, which
    /// at an EAN-13's nominal 0.33 mm module is its 95 modules: 31.35 mm wide, 22.85 mm of bar with
    /// the guard bars 1.65 mm below them, and the digits' baseline 25.93 mm down — the nominal
    /// symbol height, which is measured to the digits rather than to the font's descender.
    #[test]
    fn a_symbol_is_the_size_its_standard_gives() {
        let mut s = new_doc();
        let (id, _) = make(&mut s, json!({"symbology": "ean13", "data": "4006381333931", "background": false, "outlineText": false}));
        let kids = children(&s, id);
        let doc = &s.doc().unwrap().doc;
        let bars = kids.iter().find(|n| matches!(n.kind, NodeKind::Path { .. })).expect("the bars");
        let r = doc.bounds_of(&[bars.id], false).expect("bounds");
        let mm = |v: f64| v / (72.0 / 25.4);
        assert!((mm(r.width()) - 95.0 * 0.33).abs() < 0.02, "{}", mm(r.width()));
        assert!((mm(r.height()) - 24.5).abs() < 0.02, "{}", mm(r.height()));
        // The digits' baseline, which is where the nominal symbol height is measured to.
        let first = kids.iter().find(|n| matches!(n.kind, NodeKind::Text(_))).expect("type");
        let NodeKind::Text(t) = &first.kind else { panic!("the human readable line is type") };
        assert!((mm(t.xf.translation().y - r.y0) - 25.93).abs() < 0.02, "{}", mm(t.xf.translation().y - r.y0));
    }

    /// A missing check digit is worked out and reported back.
    #[test]
    fn the_check_digit_is_added_and_reported() {
        let mut s = new_doc();
        let (_, made) = make(&mut s, json!({"symbology": "ean13", "data": "400638133393"}));
        assert_eq!(made["data"], "4006381333931");
        // A wrong one is refused rather than quietly replaced.
        assert!(s.execute("barcode.create", &json!({"symbology": "ean13", "data": "4006381333932"})).is_err());
    }

    /// Changing the data redraws the object in place, and the art follows.
    #[test]
    fn a_change_redraws_the_same_object() {
        let mut s = new_doc();
        let (id, _) = make(&mut s, json!({"symbology": "ean13", "data": "4006381333931"}));
        let before = children(&s, id);
        s.execute("barcode.set", &json!({"data": "5012345678900"})).expect("barcode.set");
        let after = children(&s, id);
        assert_eq!(before.len(), after.len());
        assert_ne!(before[0], after[0], "the bars are drawn again");
        assert_eq!(s.doc().unwrap().doc.node(id).unwrap().extras().barcode.as_deref().unwrap().data, "5012345678900");
        // One undo step.
        s.execute("edit.undo", &json!({})).expect("undo");
        assert_eq!(s.doc().unwrap().doc.node(id).unwrap().extras().barcode.as_deref().unwrap().data, "4006381333931");
    }

    /// Redrawing a barcode without its white box leaves it where it was: the bounds start a quiet
    /// zone inside the origin, and measuring from them would walk the symbol across the artboard.
    #[test]
    fn redrawing_without_the_white_box_keeps_the_place() {
        let mut s = new_doc();
        let (id, _) = make(&mut s, json!({"symbology": "ean13", "data": "6907045517163", "background": false}));
        let before = s.doc().unwrap().doc.bounds_of(&[id], false).expect("bounds");
        for height in [22.85, 30.0, 18.0, 24.0] {
            s.execute("barcode.set", &json!({ "heightMm": height })).expect("set");
        }
        let after = s.doc().unwrap().doc.bounds_of(&[id], false).expect("bounds");
        assert!((after.x0 - before.x0).abs() < 0.01, "{before:?} -> {after:?}");
        assert!((after.y0 - before.y0).abs() < 0.01, "{before:?} -> {after:?}");
    }

    /// `barcode.set` with no options is the query: the spec and what the raw data resolves to.
    #[test]
    fn asking_for_the_options_gives_them_back() {
        let mut s = new_doc();
        make(&mut s, json!({"symbology": "code128", "data": "ABC-123"}));
        let out = s.execute("barcode.set", &json!({})).expect("query");
        assert_eq!(out["symbology"], "code128");
        assert_eq!(out["data"], "ABC-123");
        assert!(out["widthMm"].as_f64().is_some_and(|w| w > 0.0));
    }

    /// The digits come out as outlines by default, so the art carries no font and prints the same
    /// on a machine that has neither the face nor the document.
    #[test]
    fn the_digits_are_outlines_by_default() {
        let mut s = new_doc();
        let (id, _) = make(&mut s, json!({"symbology": "ean13", "data": "4006381333931"}));
        assert_eq!(children(&s, id).iter().filter(|n| matches!(n.kind, NodeKind::Text(_))).count(), 0, "no live type");
        let (edit, _) = make(&mut s, json!({"symbology": "ean13", "data": "4006381333931", "outlineText": false}));
        assert_eq!(children(&s, edit).iter().filter(|n| matches!(n.kind, NodeKind::Text(_))).count(), 13);
    }

    /// The white box is the size its standard gives the symbol — quiet zones and the human readable
    /// line included — and the whole barcode sits inside it, so the quiet zones stay clear over a
    /// photograph or a coloured carton.
    #[test]
    fn a_white_box_the_size_of_the_standard() {
        let mut s = new_doc();
        let (id, _) = make(&mut s, json!({"symbology": "ean13", "data": "4006381333931"}));
        let doc = &s.doc().unwrap().doc;
        let box_ = children(&s, id).first().cloned().expect("the box is first");
        let NodeKind::Path { .. } = &box_.kind else { panic!("the box is a path") };
        assert!(
            box_.appearance.fill().is_some_and(|f| matches!(f.paint, Paint::Solid { color, .. } if color == vectorcraft_color::Color::WHITE)),
            "the box is white"
        );
        let r = doc.bounds_of(&[box_.id], false).expect("bounds");
        let mm = |v: f64| v / (72.0 / 25.4);
        assert!((mm(r.width()) - 37.29).abs() < 0.05, "{}", mm(r.width()));
        // The nominal symbol height measured to the digits' baseline, give or take the hundredth of
        // a millimetre the font's own figures reach below it.
        assert!((mm(r.height()) - 25.93).abs() < 0.1, "{}", mm(r.height()));
        let whole = doc.bounds_of(&[id], false).expect("bounds");
        assert!(
            whole.x0 >= r.x0 - 0.01 && whole.y0 >= r.y0 - 0.01 && whole.x1 <= r.x1 + 0.01 && whole.y1 <= r.y1 + 0.01,
            "the art {whole:?} sits inside the box {r:?}"
        );
        // Off is off.
        let (bare, _) = make(&mut s, json!({"symbology": "ean13", "data": "4006381333931", "background": false}));
        let no_box = s.doc().unwrap().doc.bounds_of(&[bare], false).expect("bounds");
        assert!((no_box.width() - r.width()).abs() > 1.0, "without the box the art is the bars alone");
    }

    /// Check answers without drawing: a wrong check digit is reported, and the art is untouched.
    #[test]
    fn check_reports_without_drawing() {
        let mut s = new_doc();
        let before = s.doc().unwrap().doc.node_count();
        let out = s.execute("barcode.check", &json!({"symbology": "ean13", "data": "690704551716"})).expect("check");
        assert_eq!(out["ok"], true);
        assert_eq!(out["data"], "6907045517163");
        assert!(out["widthMm"].as_f64().is_some_and(|w| w > 0.0));
        let bad = s.execute("barcode.check", &json!({"symbology": "ean13", "data": "6907045517161"})).expect("check");
        assert_eq!(bad["ok"], false);
        let message = bad["message"].as_str().unwrap_or("");
        assert!(message.contains('3'), "the message names the right check digit: {message}");
        // Nothing was added to the document: a check is a question, not an edit.
        assert_eq!(s.doc().unwrap().doc.node_count(), before, "check drew nothing");
    }

    /// A batch is a grid: left to right from the artboard's top-left corner, `columns` of them to a
    /// row and the next row below the tallest of the last, with nothing overlapping.
    #[test]
    fn a_batch_lays_the_codes_out_in_rows() {
        let mut s = new_doc();
        // Twelve digits a line, so the encoder works the thirteenth out: a thirteen-digit line with
        // a made-up check digit is one the symbology refuses.
        let data: String = (0..23).map(|i| format!("6907045517{i:02}\n")).collect();
        let out =
            s.execute("barcode.createBatch", &json!({"symbology": "ean13", "data": data, "columns": 10, "gapX": 5.0, "gapY": 5.0})).expect("batch");
        assert_eq!(out["count"], 23);
        assert_eq!(out["skipped"].as_array().map(Vec::len), Some(0));
        let ids: Vec<NodeId> = out["ids"].as_array().expect("ids").iter().filter_map(|v| v.as_u64()).map(NodeId).collect();
        assert_eq!(ids.len(), 23);
        let doc = &s.doc().unwrap().doc;
        let rect = |id: NodeId| doc.bounds_of(&[id], false).expect("bounds");
        let corner = doc.artboards.first().map(|a| a.rect).expect("an artboard");
        let first = rect(ids[0]);
        assert!((first.x0 - corner.x0).abs() < 0.01 && (first.y0 - corner.y0).abs() < 0.01, "the first code is at the corner");
        // The eleventh wraps: back to the left edge, below the first row.
        let eleventh = rect(ids[10]);
        assert!((eleventh.x0 - first.x0).abs() < 0.01, "a new row starts at the left");
        // The gaps are millimetres, as the command says: 5 mm is 14.17 pt. The pitch is exact —
        // the layout advances by the symbol's nominal size — while the art can reach a hair past
        // it where the digits sit below the nominal height.
        let gap = 5.0 * (72.0 / 25.4);
        assert!((eleventh.y0 - first.y0 - (first.height() + gap)).abs() < 0.2, "the new row starts {} below the first", eleventh.y0 - first.y0);
        // In a row, each code starts one symbol and one gap after the one before it.
        for i in 1..10 {
            let (a, b) = (rect(ids[i - 1]), rect(ids[i]));
            assert!((b.x0 - a.x0 - (a.width() + gap)).abs() < 0.01, "code {i} sits {} pt after the one before it", b.x0 - a.x0);
        }
        // And no two of the twenty-three overlap.
        for i in 0..ids.len() {
            for j in i + 1..ids.len() {
                let (a, b) = (rect(ids[i]), rect(ids[j]));
                let apart = a.x1 <= b.x0 + 0.01 || b.x1 <= a.x0 + 0.01 || a.y1 <= b.y0 + 0.01 || b.y1 <= a.y0 + 0.01;
                assert!(apart, "codes {i} and {j} overlap");
            }
        }
    }

    /// A line the symbology cannot carry is reported, and the rest of the batch still lands.
    #[test]
    fn a_batch_skips_what_it_cannot_carry() {
        let mut s = new_doc();
        let after = s.doc().unwrap().doc.node_count();
        let out =
            s.execute("barcode.createBatch", &json!({"symbology": "ean13", "data": "4006381333931\nnonsense\n\n5012345678900"})).expect("batch");
        assert_eq!(out["count"], 2);
        let skipped = out["skipped"].as_array().expect("skipped");
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0]["data"], "nonsense");
        assert!(skipped[0]["message"].as_str().is_some_and(|m| !m.is_empty()));
        // Nothing to draw at all is an error rather than an empty edit.
        assert!(s.execute("barcode.createBatch", &json!({"symbology": "ean13", "data": "  \n\n"})).is_err());
        assert!(s.execute("barcode.createBatch", &json!({"symbology": "ean13", "data": "nope"})).is_err());
        // One undo takes the whole batch back: it is one edit, however many codes it drew.
        s.execute("edit.undo", &json!({})).expect("undo");
        assert_eq!(s.doc().unwrap().doc.node_count(), after, "the batch is gone");
    }

    /// A QR Code is one path of module runs, and carries no human readable line.
    #[test]
    fn a_qr_code_is_modules_and_no_type() {
        let mut s = new_doc();
        let (id, _) = make(&mut s, json!({"symbology": "qrCode", "data": "https://example.com/", "ecc": "quartile"}));
        let kids = children(&s, id);
        assert_eq!(kids.len(), 2, "the box and one path of modules");
        assert!(matches!(kids[1].kind, NodeKind::Path { .. }));
        let spec = s.doc().unwrap().doc.node(id).unwrap().extras().barcode.as_deref().unwrap().clone();
        assert_eq!(spec.ecc, vectorcraft_barcode::Ecc::Quartile);
    }

    /// Every symbology draws, and the ones that need it get their bearer bars.
    #[test]
    fn every_symbology_draws() {
        let mut s = new_doc();
        for (symbology, data) in [
            ("ean13", "4006381333931"),
            ("ean8", "96385074"),
            ("upcA", "036000291452"),
            ("upcE", "01234565"),
            ("ean128", "(01)12345678901231"),
            ("code128", "ABC-123"),
            ("code128A", "ABC123"),
            ("code128B", "abc123"),
            ("code128C", "1234567890"),
            ("code39", "ABC-123"),
            ("code93", "Code 93 Test"),
            ("itf14", "15400141288763"),
            ("itf25", "1234567890"),
            ("qrCode", "hello"),
        ] {
            let (id, _) = make(&mut s, json!({"symbology": symbology, "data": data}));
            let r = s.doc().unwrap().doc.bounds_of(&[id], false).expect("bounds");
            assert!(r.width() > 1.0 && r.height() > 1.0, "{symbology} is {}", r.width());
        }
    }

    /// What the panel writes into a request is what the description says the command takes: the
    /// two are written apart and drift silently, so a key the panel sends and nobody listed is
    /// refused as unknown the moment an agent sends the same one.
    #[test]
    fn the_options_a_caller_sends_are_the_ones_the_command_takes() {
        use crate::cmd::params::check_params;
        let spec =
            vectorcraft_barcode::BarcodeSpec { symbology: vectorcraft_barcode::Symbology::Ean13, data: "6907045517163".into(), ..Default::default() };
        // As the panel sends it: the whole spec, serialised.
        let mut p = serde_json::to_value(&spec).expect("spec");
        for id in ["barcode.create", "barcode.check"] {
            check_params(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        // The batch adds its own four, and takes everything above too.
        let mut batch = p.clone();
        batch["columns"] = json!(10);
        batch["gapX"] = json!(5.0);
        batch["gapY"] = json!(5.0);
        check_params("barcode.createBatch", &batch).unwrap_or_else(|e| panic!("barcode.createBatch: {e}"));
        p["id"] = json!(1);
        check_params("barcode.set", &p).unwrap_or_else(|e| panic!("barcode.set: {e}"));
    }

    /// What a caller asks for that no symbol can do comes back as an error, not a broken object.
    #[test]
    fn a_bad_request_is_an_error() {
        let mut s = new_doc();
        assert!(s.execute("barcode.create", &json!({"symbology": "pdf417", "data": "x"})).is_err());
        assert!(s.execute("barcode.create", &json!({"symbology": "ean13", "data": "nonsense"})).is_err());
        assert!(s.execute("barcode.create", &json!({"symbology": "qrCode", "data": "x", "ecc": "ultra"})).is_err());
        assert!(s.execute("barcode.create", &json!({"symbology": "qrCode", "data": "x", "version": 99, "ecc": "quartile"})).is_err());
        // Nothing is left behind by a refused command.
        assert!(s.doc().unwrap().doc.node(NodeId(0)).is_none());
    }

    /// The spec survives a save and a reopen, so a barcode can be edited after the file was closed.
    #[test]
    fn the_spec_survives_the_file() {
        let mut s = new_doc();
        let (id, _) = make(&mut s, json!({"symbology": "itf14", "data": "15400141288763", "magnification": 1.5, "text": false}));
        let json = serde_json::to_string(&*s.doc().unwrap().doc).expect("saves");
        let doc: vectorcraft_doc::Document = serde_json::from_str(&json).expect("opens");
        let spec = doc.node(id).and_then(|n| n.extras().barcode.as_deref()).expect("the spec is kept");
        assert_eq!(spec.symbology, vectorcraft_barcode::Symbology::Itf14);
        assert!((spec.magnification - 1.5).abs() < 1e-9);
        assert!(!spec.text);
    }
}
