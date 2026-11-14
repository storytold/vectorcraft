//! `document.export`, `document.serialize`, `document.exportSelection`, `document.exportForOffice`.

use std::borrow::Cow;
use std::collections::HashSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, SymbolBoxes};

use crate::DocState;

use super::super::{Result, Session, bad, check_switches, edit, number_or, switch_or, text_param};
use super::encode::single_artboard;
use super::{
    ARTBOARD_PARAMS, Format, check_linked_scripts, default_name, encode_all, encode_with_warnings, merge, with_black_output, writable, write_encoded,
    write_or_return,
};

/// The keys the export commands take for every format, beside the path, the format and the
/// options `document.formats` lists for each: `richBlack` is Appearance of Black's setting for a
/// raster export, and `params` an exporter plug-in's own.
pub(crate) const EXPORT_PARAMS: [&str; 4] = ["includeArtboardFill", "selectedOnly", "richBlack", "params"];

/// A note for each option in `p` that some format takes but the file an export of `cmd` writes in
/// `f` doesn't read (`{format: pdf, quality: 80}`), in the wording of a save's note. An exporter
/// plug-in's format gets none. The notes cover the formats' options only: a key of the command's
/// own ([`EXPORT_PARAMS`]) and an artboard choice that a selection export leaves out get none.
pub fn unused_option_notes(cmd: &str, f: &Format, p: &Value) -> Vec<String> {
    let Some(given) = p.as_object() else { return Vec::new() };
    if vectorcraft_plugins::export::by_format(f.id).is_some() {
        return Vec::new();
    }
    let listed = |k: &str| super::FORMATS.iter().filter(|f| f.write).flat_map(|f| f.options).any(|o| o.name == k);
    let written = super::save::written_as(cmd, f, p);
    let legacy = f.id == "ai" && super::legacy_ai::wanted(cmd, f, p).unwrap_or(false);
    let reads = |k: &str| match (f.id, k) {
        // A legacy `.ai` file is not a PDF: it reads its own options only.
        ("ai", _) if legacy => super::legacy_ai::reads(k),
        // The other `.ai` files don't read the legacy file's options, and `preserveEditing` is
        // on for them whatever is given.
        ("ai", "preserveEditing") => false,
        ("ai", _) if super::legacy_ai::LEGACY_ONLY.contains(&k) => false,
        // An export writes one file: only a save writes each artboard to a file of its own, and
        // a native file's `range` names those files.
        ("vectorcraft" | "template" | "ai", "separateArtboards") | ("vectorcraft" | "template", "range") => false,
        _ => super::save::reads_option(f, k),
    };
    given
        .iter()
        .filter(|(k, v)| !v.is_null() && !EXPORT_PARAMS.contains(&k.as_str()) && listed(k) && !reads(k))
        .map(|(k, _)| super::save::unused_option_note(k, &written))
        .collect()
}

pub(super) fn serialize(s: &mut Session, p: &Value) -> Result<Value> {
    let f = writable("document.serialize", Some(text_param(p, "format", "document.serialize")?.unwrap_or("vectorcraft")), None)?;
    check_linked_scripts("document.serialize", f.id, s.doc()?)?;
    let unused = unused_option_notes("document.serialize", f, p);
    let expanded = expand_presets(s, "document.serialize", p)?;
    let p = with_black_output(s, &expanded);
    let (doc, display_doc_title) = source(s, f, &p, "document.serialize")?;
    let doc = &*doc;
    let mut q = p.clone();
    if f.id == "pdf"
        && let Some(o) = q.as_object_mut()
    {
        o.insert("_displayDocTitle".into(), json!(display_doc_title));
    }
    let mut enc = encode_all(doc, f.id, &q)?;
    enc.add_setting_notes(unused);
    let files = enc.named(doc, &default_name(doc, f.extensions[0]));
    let (main, linked) = files.split_at(enc.files.len());
    // Text formats come back as text; an SVG in UTF-16 or ISO 8859-1 also as its bytes.
    let data = |bytes: &[u8]| match f.id {
        "svg" => {
            let text = vectorcraft_svg::text_of(bytes).unwrap_or_else(|_| String::from_utf8_lossy(bytes));
            let mut v = json!({ "text": text });
            if text.as_bytes() != bytes {
                v["dataBase64"] = json!(vectorcraft_format::base64_encode(bytes));
            }
            v
        }
        _ => json!({ "dataBase64": vectorcraft_format::base64_encode(bytes) }),
    };
    let mut out = merge(data(main.first().map_or(&[][..], |m| &m.1)), json!({ "warnings": enc.notes() }));
    if main.len() > 1 {
        out["files"] = main.iter().map(|(name, bytes)| merge(json!({ "name": name }), data(bytes))).collect();
    }
    if !linked.is_empty() {
        out["linked"] = linked.iter().map(|(name, bytes)| json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) })).collect();
    }
    Ok(out)
}

pub(super) fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let path = text_param(p, "path", "document.export")?;
    if let Some(path) = path {
        super::check_not_lossy_overwrite(s.doc()?, path, p, "document.export")?;
    }
    let f = writable("document.export", text_param(p, "format", "document.export")?, path)?;
    check_linked_scripts("document.export", f.id, s.doc()?)?;
    let unused = unused_option_notes("document.export", f, p);
    let expanded = expand_presets(s, "document.export", p)?;
    let p = with_black_output(s, &expanded);
    let (doc, display_doc_title) = source(s, f, &p, "document.export")?;
    let doc = &*doc;
    let mut q = p.clone();
    if f.id == "pdf"
        && let Some(o) = q.as_object_mut()
    {
        o.insert("_displayDocTitle".into(), json!(display_doc_title));
    }
    let mut enc = encode_all(doc, f.id, &q)?;
    enc.add_setting_notes(unused);
    write_encoded(path, &default_name(doc, f.extensions[0]), doc, &enc, json!({ "format": f.id, "warnings": enc.notes() }))
}

/// The document an export of `st` in format `f` writes ([`export_source`]): a `.ai` or `.ait`
/// file names the view's active artboard ([`DocState::active_artboard`]) as its active one, as a
/// save does. The open document is unchanged.
pub fn export_doc<'a>(st: &'a DocState, f: &Format, p: &Value) -> Result<Cow<'a, Document>> {
    let mut doc = export_source(st, p)?;
    if let Some(id) = super::save::moved_active_artboard(st, &doc, f) {
        doc.to_mut().opened_artboard = Some(id);
    }
    Ok(doc)
}

/// The document an export of `st` writes: the whole document, or with `selectedOnly` just the
/// selected objects, each in its layers and groups (a clipping group keeps its clipping path).
pub fn export_source<'a>(st: &'a DocState, p: &Value) -> Result<Cow<'a, Document>> {
    if !switch_or(p, "selectedOnly", false, "document.export")? {
        return Ok(Cow::Borrowed(&*st.doc));
    }
    let keep: HashSet<NodeId> = edit::roots_of(&st.doc, st.selection.in_paint_order(&st.doc)).into_iter().collect();
    if keep.is_empty() {
        return Err(bad("document.export", "select something to export (or turn off selectedOnly)"));
    }
    let mut d = Document::clone(&st.doc);
    d.layers = enclosed(&st.doc, &keep, false);
    Ok(Cow::Owned(d))
}

/// The layers of `doc` with only the objects in `keep` (whole, with everything inside them), each
/// in the layers, groups and compound paths that hold it. Those keep what they apply to the object
/// on the canvas: their clipping paths, opacity masks, effects, opacity and blend modes. An object
/// of a blend, envelope, repeat or intertwine goes in on its own, without the art made from it. With
/// `shown`, the layers and groups that hold the objects are visible, and those layers print and are
/// not template layers.
fn enclosed(doc: &Document, keep: &HashSet<NodeId>, shown: bool) -> Vec<Arc<Node>> {
    let holders: HashSet<NodeId> = keep.iter().filter_map(|id| doc.ancestry(*id)).flatten().filter(|id| !keep.contains(id)).collect();
    let mut out = vec![];
    for l in &doc.layers {
        selected_part(l, keep, &holders, shown, &mut out);
    }
    out
}

/// The parts of `n` in `keep` (whole, with everything inside them), added to `out`: `n` itself, or
/// `n` with only those parts when it is one of their `holders`, made visible, printable and not a
/// template with `shown`.
fn selected_part(n: &Arc<Node>, keep: &HashSet<NodeId>, holders: &HashSet<NodeId>, shown: bool, out: &mut Vec<Arc<Node>>) {
    if keep.contains(&n.id) {
        out.push(n.clone());
        return;
    }
    let Some(children) = n.children().filter(|_| holders.contains(&n.id)) else { return };
    let mut kept = vec![];
    for c in children {
        selected_part(c, keep, holders, shown, &mut kept);
    }
    if !matches!(n.kind, NodeKind::Layer { .. } | NodeKind::Group { .. } | NodeKind::Compound { .. }) {
        out.extend(kept);
        return;
    }
    if kept.is_empty() {
        return;
    }
    // A clipping group or layer keeps its clipping path; a Shaper group its source art, which
    // draws nothing.
    let first_stays = n.clips() || n.extras().shaper.is_some();
    if let Some(first) = children.first().filter(|f| first_stays && kept.first().is_none_or(|k| k.id != f.id)) {
        kept.insert(0, first.clone());
    }
    let mut part = Node::clone(n);
    if let Some(ch) = part.children_mut() {
        *ch = kept;
    }
    if shown {
        part.visible = true;
        if let NodeKind::Layer { template, printable, .. } = &mut part.kind {
            *template = false;
            *printable = true;
        }
    }
    out.push(Arc::new(part));
}

/// `doc` with only the art over `rect` (an artboard): objects whose visual bounds miss it are left
/// out, at any depth of layers, sublayers and groups that don't clip, unless what the renderer
/// draws for them, their effects included ([`vectorcraft_render::effect_bounds`]), reaches it.
/// Objects that reach it stay whole (the file's bounds clip them), as DXF's crop leaves them.
/// Symbol instances are measured by their art, and the box of one whose art holds brushed strokes
/// grows by as far as their brush art can paint ([`SymbolBoxes::with_brush_art`]). An object that
/// holds a symbol instance also stays when what the canvas paints for it reaches `rect`
/// ([`vectorcraft_render::painted_bounds`]: the effects in the symbols' art and its own effects). A
/// clipping layer keeps every object, its clipping path among them. An artboard's file then holds
/// its own art, not a whole document of artboards. `renderer` measures the 3D art, and keeps it for
/// the next artboard of the same document.
pub(super) fn art_over(doc: &Document, rect: vectorcraft_geom::Rect, renderer: &mut vectorcraft_render::Renderer) -> Document {
    fn kept(
        n: &Arc<Node>,
        doc: &Document,
        symbols: &SymbolBoxes,
        rect: vectorcraft_geom::Rect,
        renderer: &mut vectorcraft_render::Renderer,
    ) -> Option<Arc<Node>> {
        if matches!(n.kind, NodeKind::Layer { clip: false, .. }) {
            let mut layer = Node::clone(n);
            let children = layer.children_mut()?;
            *children = children.iter().filter_map(|c| kept(c, doc, symbols, rect, renderer)).collect();
            return Some(Arc::new(layer));
        }
        // A group that doesn't clip keeps only its members over the artboard, so art gathered in one
        // group across several artboards splits by artboard. Not with effects of its own: a shadow
        // can reach in from a member outside.
        if matches!(n.kind, NodeKind::Group { clip: false, .. }) && n.appearance.effects.is_empty() {
            let mut group = Node::clone(n);
            let children = group.children_mut()?;
            *children = children.iter().filter_map(|c| kept(c, doc, symbols, rect, renderer)).collect();
            return (!children.is_empty()).then(|| Arc::new(group));
        }
        let over = |b: vectorcraft_geom::Rect| b.x0 <= rect.x1 && rect.x0 <= b.x1 && b.y0 <= rect.y1 && rect.y0 <= b.y1;
        let mut holds_instance = false;
        n.walk(&mut |c| holds_instance |= matches!(c.kind, NodeKind::SymbolInstance { .. }));
        let painted = || holds_instance && vectorcraft_render::painted_bounds(doc, n).is_some_and(over);
        (n.visual_bounds_with(symbols).is_none_or(over) || renderer.effect_bounds(symbols, n).is_some_and(over) || painted()).then(|| n.clone())
    }
    let symbols = doc.symbol_boxes().with_brush_art();
    let mut d = doc.clone();
    d.layers = doc.layers.iter().filter_map(|l| kept(l, doc, &symbols, rect, renderer)).collect();
    d
}

/// `doc` with the background colors of `artboards` drawn behind its art
/// ([`Document::with_artboard_fills`]), what exports and print draw, unless `p` turns
/// `includeArtboardFill` off (Export for Screens › Include Artboard Fill; on by default).
pub(crate) fn with_artboard_fills<'a>(
    doc: &'a Document,
    p: &Value,
    artboards: impl IntoIterator<Item = usize>,
) -> std::result::Result<Cow<'a, Document>, String> {
    let include = match p.get("includeArtboardFill") {
        None | Some(Value::Null) => true,
        Some(v) => v.as_bool().ok_or("`includeArtboardFill` must be true or false")?,
    };
    Ok(include.then(|| doc.with_artboard_fills(artboards)).flatten().map_or(Cow::Borrowed(doc), Cow::Owned))
}

/// `p` without its artboard choice, also inside its SVG options (for documents made of one
/// synthetic artboard, and for callers that pick the artboard themselves).
pub(super) fn without_artboards(p: &Value) -> Value {
    let strip = |o: &mut serde_json::Map<String, Value>| ARTBOARD_PARAMS.iter().for_each(|k| _ = o.remove(*k));
    let mut q = p.clone();
    if let Some(o) = q.as_object_mut() {
        strip(o);
        if let Some(svg) = o.get_mut("svg").and_then(Value::as_object_mut) {
            strip(svg);
        }
    }
    q
}

/// `p` with the saved presets it names (PDF, flattener) written out as options, for the encoders,
/// which know only the built-in ones.
fn expand_presets<'a>(s: &Session, cmd: &str, p: &'a Value) -> Result<Cow<'a, Value>> {
    super::pdf::expand_preset(s, cmd, p)
}

/// The document an export of `f` writes: the active one (see [`export_source`]), or for text
/// with `selectionOnly` the selected objects alone.
fn source<'a>(s: &'a mut Session, f: &Format, p: &Value, cmd: &str) -> Result<(Cow<'a, Document>, bool)> {
    check_switches(p, &["selectedOnly"], cmd)?;
    if switch_or(p, "selectionOnly", false, cmd)? && f.id == "txt" {
        let doc = selection(s, cmd, true)?.0;
        let display_doc_title = super::has_own_title(&doc);
        return Ok((Cow::Owned(doc), display_doc_title));
    }
    let st = s.doc()?;
    let doc = export_doc(st, f, p)?;
    let display_doc_title = super::has_own_title(&doc);
    Ok((super::titled_for_export(&doc, st.path.as_deref(), f.id).map_or(doc, Cow::Owned), display_doc_title))
}

/// The selected objects (not on template layers) in the layers and groups that hold them
/// ([`isolated`]), with one artboard: their crop. With `text`, the objects alone on one layer, so a
/// text export holds the type of the selected objects and not that of a clipping path around them.
fn selection(s: &mut Session, cmd: &str, text: bool) -> Result<(Document, vectorcraft_geom::Rect)> {
    let ids = selected_artwork(s)?;
    let doc = &s.doc()?.doc;
    let (mut d, bounds) = isolated(doc, &ids, "Selection").ok_or_else(|| bad(cmd, "select something to export"))?;
    if text {
        let mut layer = Node::layer(d.alloc_id(), "Selection", vectorcraft_doc::LayerColor::Preset(0));
        if let Some(ch) = layer.children_mut() {
            *ch = ids.iter().filter_map(|id| doc.node(*id).cloned().map(Arc::new)).collect();
        }
        d.layers = vec![Arc::new(layer)];
    }
    Ok((d, bounds))
}

/// The selected objects (their top-most selected ancestors) that are artwork: those on template
/// layers are guides and are left out. With an object of a dynamic symbol instance's art selected,
/// the instance is exported whole.
pub(crate) fn selected_artwork(s: &Session) -> Result<Vec<NodeId>> {
    let st = s.doc()?;
    let ids = edit::roots_of(&st.doc, st.selection.in_paint_order(&st.doc));
    let is_template = |id| st.doc.node(id).is_some_and(|l| matches!(l.kind, NodeKind::Layer { template: true, .. }));
    // On a template layer or sublayer, at any depth.
    let on_template = |id| st.doc.ancestry(id).is_some_and(|a| a.into_iter().any(is_template));
    Ok(ids.into_iter().filter(|id| !on_template(*id)).collect())
}

/// The crop of an export of objects `ids` of `doc` alone ([`isolated`]): the bounds the renderer
/// measures for what it draws for them, their effects included ([`vectorcraft_render::effect_bounds`];
/// symbol instances are measured by their art), with what the layers and groups that hold them
/// apply (a clipping group or layer is measured by its clip), `None` when they have none.
pub(crate) fn export_crop(doc: &Document, ids: &[NodeId]) -> Option<vectorcraft_geom::Rect> {
    isolated_art(doc, ids).1
}

/// The layers of [`isolated`] and their crop.
fn isolated_art(doc: &Document, ids: &[NodeId]) -> (Vec<Arc<Node>>, Option<vectorcraft_geom::Rect>) {
    let keep: HashSet<NodeId> = ids.iter().copied().collect();
    let layers = enclosed(doc, &keep, true);
    let symbols = doc.symbol_boxes();
    let bounds = layers.iter().filter_map(|l| vectorcraft_render::effect_bounds(&symbols, l)).reduce(|a, b| a.union(b));
    (layers, bounds)
}

/// Objects `ids` of `doc` alone, each in the layers and groups that hold it ([`enclosed`], made
/// visible), with one artboard named `name`: their [`export_crop`] (`None` when they have none).
pub(crate) fn isolated(doc: &Document, ids: &[NodeId], name: &str) -> Option<(Document, vectorcraft_geom::Rect)> {
    let (layers, bounds) = isolated_art(doc, ids);
    let bounds = bounds?;
    let mut d = single_artboard(doc, bounds, name);
    d.layers = layers;
    Some((d, bounds))
}

/// File → Export Selection: the selected objects alone, cropped to the bounds the renderer
/// measures for what they draw, their effects included ([`isolated`]). Objects on template layers
/// are guides, not artwork, and are left out.
pub(super) fn export_selection(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "document.exportSelection";
    let path = text_param(p, "path", C)?;
    if let Some(path) = path {
        super::check_not_lossy_overwrite(s.doc()?, path, p, C)?;
    }
    let f = writable(C, text_param(p, "format", C)?, path)?;
    check_linked_scripts(C, f.id, s.doc()?)?;
    let unused = unused_option_notes(C, f, p);
    let expanded = expand_presets(s, C, p)?;
    let p = with_black_output(s, &expanded);
    let (d, bounds) = selection(s, C, f.id == "txt")?;
    // The encoder's warnings (options accepted but not applied, features approximated or left
    // out), as document.export reports them.
    let (bytes, mut warnings) = encode_with_warnings(&d, f.id, &without_artboards(&p))?;
    for note in unused {
        if !warnings.contains(&note) {
            warnings.push(note);
        }
    }
    write_or_return(path, &bytes, json!({ "format": f.id, "warnings": warnings, "bounds": [bounds.x0, bounds.y0, bounds.width(), bounds.height()] }))
}

/// File → Save for Office Documents: one artboard as a PNG at `ppi`, on white unless
/// `transparent`, with the encoder's warnings.
pub(super) fn export_for_office(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "document.exportForOffice";
    let path = text_param(p, "path", C)?;
    if let Some(path) = path {
        super::check_not_lossy_overwrite(s.doc()?, path, p, C)?;
    }
    let ppi = number_or(p, "ppi", 150.0, C)?;
    let transparent = switch_or(p, "transparent", false, C)?;
    let doc = &s.doc()?.doc;
    let artboard = match p.get("artboard") {
        None | Some(Value::Null) => 0,
        Some(v) => v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or_else(|| bad(C, "artboard is a 0-based index"))?,
    };
    if artboard >= doc.artboards.len() {
        return Err(bad(C, format!("no artboard {artboard} (0-based; the document has {})", doc.artboards.len())));
    }
    let background = if transparent { "transparent" } else { "white" };
    let opts = with_black_output(s, &json!({ "ppi": ppi, "background": background, "artboard": artboard }));
    let (bytes, warnings) = encode_with_warnings(doc, "png", &opts)?;
    // The pixel size, from the PNG header.
    let side = |at: usize| bytes.get(at..at + 4).and_then(|b| b.try_into().ok()).map_or(0, u32::from_be_bytes);
    write_or_return(path, &bytes, json!({ "width": side(16), "height": side(20), "warnings": warnings }))
}
