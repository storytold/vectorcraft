//! Eyedropper tool (I).
//!
//! Click an object: copy its attributes (what Eyedropper Options pick up and apply) to the
//! selection and the paint defaults (`appearance.copyFrom`); Alt-click: the selection's attributes
//! onto the clicked object (`reverse`); Shift+Alt-click: add the clicked object's appearance to the
//! selection's (`append`). Shift-click: sample only the colour under the cursor into the active
//! fill/stroke (`paint.sampleColor`; in a CMYK document an RGB color is stored as CMYK): a
//! gradient's color at that point, a pattern's tile art there (after the pattern's transform), a
//! gradient mesh's interpolated patch color, or the art a blend or an envelope draws there (a
//! blend's steps, an envelope's distorted contents). Clicking an image samples its pixels
//! (averaged over the options' raster sample size; a CMYK image gives its ink amounts).
//!
//! The Gradient panel's eyedropper sets the `stop` option (the tool to return to): the next click
//! samples the colour under the cursor into the selected gradient stop (`paint.sampleColor
//! {stop}`) and switches back to that tool.
//!
//! Brush Options' Key Color eyedropper sets `brushKeyColor` and `stop`: the next click samples into
//! the open dialog (`brushKeyColor` UI request) and switches back.

use std::collections::HashSet;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::hit::{HitKind, HitOptions, hit_test, hit_test_visible, hit_test_visible_skipping};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::hit::fill_contains;
use vectorcraft_geom::{Affine, Point, Rect};

use super::paint_owner;
use crate::params::color_json;
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext};

#[derive(Default)]
pub struct EyedropperTool {
    /// Sampling for the selected gradient stop: the tool to switch back to afterwards.
    stop: Option<String>,
    /// The Type tool's highlighted range while sampling, including its selection overlay.
    text_selection: crate::text::TypeTool,
    /// Brush Options › Key Color (Hue Shift): sample into the dialog, then [`Self::stop`].
    brush_key_color: bool,
}

/// The colour of `paint` at `p` (gradients are sampled along their vector, radial ones honouring
/// their aspect ratio).
pub fn paint_color_at(paint: &Paint, bounds: Option<vectorcraft_geom::Rect>, p: Point) -> Option<Color> {
    match paint {
        Paint::Solid { color, .. } => Some(*color),
        Paint::Gradient(g) => {
            let geom = g.geom.or_else(|| bounds.map(|b| g.resolve(b)))?;
            let t = geom.param_at(g.gradient.kind, p);
            Some(g.gradient.sample(t.clamp(0.0, 1.0) as f32).0)
        }
        _ => None,
    }
}

/// How many patterns and symbols deep a sample follows art inside art (tile art painted with a
/// pattern, a symbol whose art holds an instance of another). Art nested deeper gives no color.
const MAX_NESTING: usize = 16;

/// How many patterns deep the canvas draws tile art painted with a pattern. A pattern nested
/// deeper, like a pattern the document lacks, draws (and samples) as [`MISSING_PATTERN`].
const PATTERN_DEPTH: usize = 3;

/// The gray the canvas fills a pattern with when it does not draw its tiles.
const MISSING_PATTERN: [u8; 3] = [128, 128, 128];

/// Most tiles of one pattern a sample looks at, front to back, for art that paints the point. When
/// more tiles reach the point, the sample gives no color.
const MAX_TILES: usize = 1_000;

/// Most work one sample does, in objects looked at: each hit test after the first in the sampled
/// document, and each one in the art of a tile or instance, counts the objects of the document it
/// tests and of the symbols, each pattern tile or symbol instance
/// sampled counts the objects of its art, and every hundred tile positions checked, or points of a
/// mesh's patches looked at, count one.
/// Art inside art multiplies them; a sample that would need more gives no color.
const MAX_WORK: usize = 1_000_000;

/// One color sample: the document, the hit options, the raster sample size and the work left.
struct Sampler<'a> {
    doc: &'a Document,
    opts: HitOptions,
    raster_sample: u32,
    /// The objects in the art of the document's symbols, which a hit test may measure.
    symbol_nodes: usize,
    work: usize,
    /// The work ran out: the sample gives no color.
    exhausted: bool,
    /// How many patterns the art being sampled is inside.
    patterns: usize,
}

impl Sampler<'_> {
    /// Count `n` units of work, or `None` when that is more than is left.
    fn spend(&mut self, n: usize) -> Option<()> {
        match self.work.checked_sub(n) {
            Some(left) => {
                self.work = left;
                Some(())
            }
            None => {
                self.work = 0;
                self.exhausted = true;
                None
            }
        }
    }
}

/// The Eyedropper's hit options: a click inside a fill or among characters samples it too,
/// whatever Object and Type Object Selection by Path Only say.
fn sample_options(cx: &ToolContext) -> HitOptions {
    HitOptions { path_only: false, type_path_only: false, ..cx.hit_options() }
}

/// The visible object under `p`, using the Eyedropper's source selection rules.
pub(crate) fn sample_source(cx: &ToolContext, p: Point) -> Option<(vectorcraft_doc::NodeId, HitKind)> {
    let h = hit_test_visible(cx.doc, p, sample_options(cx))?;
    Some((paint_owner(cx.doc, h.leaf), h.kind))
}

/// The visible color under `p`, including image pixels, pattern tiles, mesh colors, intermediate
/// gradient colors, the steps a blend draws and an envelope's distorted contents.
pub(crate) fn sample_color_at(cx: &ToolContext, p: Point) -> Option<Color> {
    let (doc, opts, raster_sample) = (cx.doc, sample_options(cx), cx.raster_sample);
    vectorcraft_doc::depth::with_deep_stack(|| {
        let symbol_nodes = doc.symbols.iter().map(|s| s.art.count()).sum();
        let mut s = Sampler { doc, opts, raster_sample, symbol_nodes, work: MAX_WORK, exhausted: false, patterns: 0 };
        let mut levels: Vec<Option<Document>> = (0..MAX_NESTING).map(|_| None).collect();
        let c = color_under(&mut s, doc, &mut levels, p, opts);
        c.filter(|_| !s.exhausted)
    })
}

/// The color the front-most object of `doc` hit at `p` with `opts` shows there. Where a stroke
/// gives no color at `p` (a gap between its pattern's tiles), the object's own fill there does;
/// an object that gives none at all is passed over for the ones below it. A hit on a blend or an
/// envelope, or on an object it holds, samples the art it draws ([`live_color_at`]), and where
/// that art leaves `p` empty the whole blend or envelope is passed over. `levels` holds the
/// scratch documents for art inside art, one for each level deeper.
fn color_under(s: &mut Sampler, doc: &Document, levels: &mut [Option<Document>], p: Point, opts: HitOptions) -> Option<Color> {
    let mut passed: HashSet<NodeId> = HashSet::new();
    let mut size = None;
    loop {
        // The first hit test in the sampled document is the one a click makes anyway; each one
        // after it, and every one in the art of a tile or instance, may visit every object again
        // and measure the symbols' art.
        let cost = if passed.is_empty() && std::ptr::eq(doc, s.doc) {
            1
        } else {
            *size.get_or_insert_with(|| s.symbol_nodes + doc.layers.iter().map(|l| l.count()).sum::<usize>())
        };
        s.spend(cost)?;
        let h = hit_test_visible_skipping(doc, p, opts, &|id| passed.contains(&id))?;
        if let Some(live) = drawn_live(doc, &h.ancestry) {
            if let Some(c) = live_color_at(s, levels, live, p) {
                return Some(c);
            }
            if !passed.insert(live.id) {
                return None;
            }
            continue;
        }
        let id = paint_owner(doc, h.leaf);
        let n = doc.node(id)?;
        let on_stroke = h.kind == HitKind::Stroke;
        if let Some(c) = node_color_at(s, levels, n, on_stroke, p) {
            return Some(c);
        }
        if on_stroke
            && fill_covers(n, p)
            && let Some(c) = node_color_at(s, levels, n, false, p)
        {
            return Some(c);
        }
        if !passed.insert(id) {
            return None;
        }
    }
}

/// Whether the fill of path or compound path `n` covers `p`.
fn fill_covers(n: &Node, p: Point) -> bool {
    match &n.kind {
        NodeKind::Path { path, rule, .. } => fill_contains(&path.to_bezpath(), *rule, p),
        NodeKind::Compound { rule, .. } => n.stroke_path().is_some_and(|bp| fill_contains(&bp, *rule, p)),
        _ => false,
    }
}

/// The outermost blend or envelope (other than one whose contents are being edited) in `ancestry`,
/// a hit's path through `doc`: the canvas draws its evaluated art, not the objects it holds.
fn drawn_live<'d>(doc: &'d Document, ancestry: &[NodeId]) -> Option<&'d Node> {
    ancestry.iter().filter_map(|id| doc.node(*id)).find(|n| matches!(n.kind, NodeKind::Blend { .. } | NodeKind::Envelope { editing: false, .. }))
}

/// The color the art blend or envelope `n` draws shows at `p`, sampled as [`art_color_at`] samples
/// art: a blend's keys and steps, an envelope's distorted contents, symbol instances in them by
/// their symbols' art. Evaluating it counts the objects it holds, and the art made counts its
/// objects.
fn live_color_at(s: &mut Sampler, levels: &mut [Option<Document>], n: &Node, p: Point) -> Option<Color> {
    s.spend(n.count())?;
    let art = vectorcraft_effects::expand_live_deep(Some(s.doc), n);
    s.spend(art.count())?;
    art_color_at(s, levels, [art], p)
}

/// The color `n` shows at `p`: an image's pixels, a mesh's interpolated patch color, a symbol
/// instance's art, or the color of its fill (or its stroke, when the stroke was hit or there is no
/// fill) there. Images, symbols and patterns are looked up in the sampled document.
fn node_color_at(s: &mut Sampler, levels: &mut [Option<Document>], n: &Node, on_stroke: bool, p: Point) -> Option<Color> {
    match &n.kind {
        NodeKind::Image(_) => image_color_at(s, n, p),
        NodeKind::Mesh(m) => {
            // Finding the patch point looks at 17 × 17 points of every patch: a hundred count as one unit.
            s.spend((m.rows as usize).saturating_mul(m.cols as usize).saturating_mul(17 * 17) / 100)?;
            let (r, c, u, v) = m.locate(p)?;
            Some(m.color_at(r, c, u, v).0)
        }
        // The instance's fill only stains its art: where the art paints nothing, nothing shows.
        NodeKind::SymbolInstance { symbol, xf } => symbol_color_at(s, levels, n, symbol, *xf, p),
        _ => paint_sample(s, levels, n, on_stroke, p),
    }
}

/// The color the art of symbol `name` shows at `p` as instance `inst` draws it: stained by the
/// instance's fill and placed in the document by `xf`, its strokes and patterns unscaled.
fn symbol_color_at(s: &mut Sampler, levels: &mut [Option<Document>], inst: &Node, name: &str, xf: Affine, p: Point) -> Option<Color> {
    if xf.determinant().abs() < 1e-12 {
        return None;
    }
    let doc = s.doc;
    let sym = doc.symbols.iter().find(|s| s.name == name)?;
    s.spend(sym.art.count())?;
    let mut art = vectorcraft_brush::instance_art(&sym.art, inst);
    art.transform(xf, false);
    art_color_at(s, levels, [art], p)
}

/// Whether the source has fill or stroke paint for an attribute sample.
pub(crate) fn has_sampleable_paint(cx: &ToolContext, id: vectorcraft_doc::NodeId) -> bool {
    cx.doc.node(id).is_some_and(|n| [false, true].into_iter().any(|stroke| n.proxy_paint(stroke, None).is_some_and(|(paint, ..)| !paint.is_none())))
}

/// The colour of `n`'s fill (or its stroke, when the stroke was hit or there is no fill) at `p`,
/// sampled in the paint's own space (text space for type runs) and box. A pattern gives the color
/// of its tile art at `p` ([`pattern_color_at`]).
fn paint_sample(s: &mut Sampler, levels: &mut [Option<Document>], n: &Node, on_stroke: bool, p: Point) -> Option<Color> {
    let fill_none = n.proxy_paint(false, None).is_none_or(|(p, ..)| p.is_none());
    let stroke = on_stroke || fill_none;
    // Type takes its paint from its characters, and a colorized object from its colorize paint.
    let items: Vec<usize> = n.appearance.items.iter().enumerate().filter(|(_, it)| it.is_fill() != stroke).map(|(i, _)| i).collect();
    if items.is_empty() || matches!(n.kind, NodeKind::Text(_)) || (!stroke && n.extras().colorize.is_some()) {
        let (paint, to_doc, bounds) = n.proxy_paint(stroke, None)?;
        return paint_color(s, levels, paint, to_doc, bounds, p);
    }
    // The visible fills (or strokes) from the top: where one leaves a gap, the next shows. Of a
    // path's strokes, only the ones that reach `p` count, when any does (a click within the hit
    // tolerance of a stroke may reach none).
    let mut visible: Vec<usize> = items.into_iter().filter(|&i| n.appearance.items.get(i).is_some_and(|it| it.visible())).collect();
    if stroke && let (NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. }, Some(bp)) = (&n.kind, n.stroke_path()) {
        let reach =
            |i: &usize| matches!(n.appearance.items.get(*i), Some(vectorcraft_doc::AppearanceItem::Stroke(st)) if st.hits(&bp, *rule, p, 0.0));
        if visible.iter().any(&reach) {
            visible.retain(&reach);
        }
    }
    for &i in visible.iter().rev() {
        let Some((paint, to_doc, bounds)) = n.proxy_paint(stroke, Some(i)) else { continue };
        if let Some(c) = paint_color(s, levels, paint, to_doc, bounds, p) {
            return Some(c);
        }
    }
    None
}

/// The color `paint`, in a space `to_doc` places in the document and with box `bounds` there,
/// shows at `p`. A pattern gives the color of its tile art at `p` ([`pattern_color_at`]).
fn paint_color(s: &mut Sampler, levels: &mut [Option<Document>], paint: &Paint, to_doc: Affine, bounds: Rect, p: Point) -> Option<Color> {
    if paint.is_none() {
        return None;
    }
    if let Paint::Pattern { pattern, xf } = paint {
        return pattern_color_at(s, levels, pattern, to_doc * *xf, p);
    }
    let p = if to_doc.determinant().abs() > 1e-12 { to_doc.inverse() * p } else { p };
    paint_color_at(paint, Some(bounds), p)
}

/// The color pattern `name`, placed in the document by `xf`, shows at `p`: the color of the
/// front-most tile art there. A point the tiles leave empty has no color. A pattern the document
/// lacks, or one more than [`PATTERN_DEPTH`] patterns deep, shows [`MISSING_PATTERN`].
fn pattern_color_at(s: &mut Sampler, levels: &mut [Option<Document>], name: &str, xf: Affine, p: Point) -> Option<Color> {
    if xf.determinant().abs() < 1e-12 {
        return None;
    }
    let [r, g, b] = MISSING_PATTERN;
    let doc = s.doc;
    let Some(def) = doc.patterns.iter().find(|d| d.name == name) else { return Some(Color::rgb8(r, g, b)) };
    if def.art.is_empty() {
        return None;
    }
    if s.patterns >= PATTERN_DEPTH {
        return Some(Color::rgb8(r, g, b));
    }
    s.patterns += 1;
    let c = tiles_color_at(s, levels, def, xf, p);
    s.patterns -= 1;
    c
}

/// The color the front-most tile art of pattern `def`, placed in the document by `xf`, shows at `p`.
fn tiles_color_at(s: &mut Sampler, levels: &mut [Option<Document>], def: &vectorcraft_doc::PatternDef, xf: Affine, p: Point) -> Option<Color> {
    let doc = s.doc;
    // Sampled in pattern space, one tile at a time from the front, among the tiles whose art reaches
    // the point (or meets it at an edge). The art moves by whole tiles, and the patterns it is
    // painted with move with it.
    let q = xf.inverse() * p;
    let near = Rect::from_points(q, q).inflate(1e-6, 1e-6);
    let (mut art_nodes, mut instances) = (0, false);
    for a in &def.art {
        a.walk(&mut |n| {
            art_nodes += 1;
            instances |= matches!(n.kind, NodeKind::SymbolInstance { .. });
        });
    }
    // Symbol instances in the art are measured by their symbols' art.
    s.spend(1 + art_nodes + if instances { s.symbol_nodes } else { 0 })?;
    let symbols = doc.symbol_boxes();
    let bounds: Vec<Option<Rect>> = def.art.iter().map(|a| a.visual_bounds_with(&symbols)).collect();
    let reaches = |m: Affine| bounds.iter().any(|b| b.is_none_or(|b| m.transform_rect_bbox(b).inflate(1e-6, 1e-6).contains(q)));
    let scaling = vectorcraft_doc::Scaling { patterns: true, ..Default::default() };
    // The tiles are listed as the canvas lists the ones it draws: by the art's bounds without its
    // symbols. Listing them looks at every tile position near the point widened by those bounds (at
    // most 250,000, past which it lists none): a hundred positions count as one unit, counted first.
    let listed = def.art_bounds().unwrap_or(def.tile).union(def.tile);
    let (w, h) = (def.width(), def.height());
    let (sx, sy) = def.tile_type.steps(w, h);
    let positions = ((near.width() + listed.width() + 2.0 * w) / sx + 3.0) * ((near.height() + listed.height() + 2.0 * h) / sy + 3.0);
    if positions.is_finite() && positions > 0.0 {
        s.spend(positions.min(250_000.0) as usize / 100)?;
    }
    let offsets = def.offsets_covering(near);
    // Each tile position is checked against each piece of art: a hundred checks count as one unit.
    s.spend(offsets.len().saturating_mul(bounds.len().max(1)) / 100)?;
    for (k, m) in offsets.into_iter().rev().map(|o| def.instance_xf(o)).filter(|m| reaches(*m)).enumerate() {
        // A tile past the limit may paint the point: the sample gives no color.
        if k == MAX_TILES {
            s.exhausted = true;
            return None;
        }
        s.spend(1 + art_nodes)?;
        let art = def.art.iter().map(|a| {
            let mut n = (**a).clone();
            vectorcraft_doc::pattern::place_art(&mut n, m, scaling);
            n
        });
        if let Some(c) = art_color_at(s, levels, art, q) {
            return Some(c);
        }
    }
    None
}

/// The color the front-most of `art` (in paint order, in the space of `p`) shows at `p`, sampled
/// as [`node_color_at`] samples an object, with no tolerance around outlines and strokes: only
/// what it paints at `p` counts. The art is hit in the first of `levels`, a scratch document with
/// the sampled document's symbols, made on first use and emptied for each new piece of art; art
/// inside it uses the levels after it. With no level left, the whole sample gives no color.
fn art_color_at(s: &mut Sampler, levels: &mut [Option<Document>], art: impl IntoIterator<Item = Node>, p: Point) -> Option<Color> {
    let Some((slot, deeper)) = levels.split_first_mut() else {
        s.exhausted = true;
        return None;
    };
    let doc = s.doc;
    let scratch = slot.get_or_insert_with(|| {
        let mut d = Document::new(1.0, 1.0);
        d.symbols = doc.symbols.clone();
        d
    });
    let layer = scratch.layers.first()?;
    let (layer, placed) = (layer.id, layer.children().map(|c| c.iter().map(|n| n.id).collect::<Vec<_>>()).unwrap_or_default());
    for id in placed {
        scratch.remove(id).ok()?;
    }
    for mut n in art {
        renumber(scratch, &mut n);
        scratch.insert(Some(layer), usize::MAX, n).ok()?;
    }
    let opts = HitOptions { scope: None, tol: 0.0, outline: false, ..s.opts };
    color_under(s, scratch, deeper, p, opts)
}

/// Gives `n` and everything in it new ids from `doc` (pattern instances all have id 0).
fn renumber(doc: &mut Document, n: &mut Node) {
    n.id = doc.alloc_id();
    for c in n.children_mut().into_iter().flatten() {
        renumber(doc, std::sync::Arc::make_mut(c));
    }
}

/// The color of image `n`'s pixels at `p`, averaged over the raster sample size: a CMYK image's ink
/// amounts as they are, other images' sRGB pixels in the working RGB space (the color that shows
/// as that pixel).
fn image_color_at(s: &Sampler, n: &Node, p: Point) -> Option<Color> {
    let NodeKind::Image(im) = &n.kind else { return None };
    if im.xf.determinant().abs() < 1e-12 {
        return None;
    }
    let px = im.xf.inverse() * p;
    let blob = s.doc.images.get(&im.key)?;
    let object = (im.width, im.height);
    if let Some(inks) = blob.sample_cmyk_object(px.x, px.y, object, s.raster_sample) {
        let [c, m, y, k] = inks.map(|v| f32::from(v) / 255.0);
        return Some(Color::cmyk(c, m, y, k));
    }
    let [r, g, b, _] = blob.sample_object(px.x, px.y, object, s.raster_sample)?;
    let [r, g, b] = vectorcraft_color::cms::active().srgb_to_rgb([r, g, b].map(|v| f32::from(v) / 255.0));
    Some(Color::rgb(r, g, b))
}

impl Tool for EyedropperTool {
    fn id(&self) -> &'static str {
        "eyedropper"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if ev.kind != PointerKind::Down {
            return vec![];
        }
        let opts = sample_options(cx);
        if self.brush_key_color {
            let Some(c) = sample_color_at(cx, ev.pos) else { return vec![] };
            let back = self.stop.clone().unwrap_or_else(|| "selection".to_string());
            self.brush_key_color = false;
            self.stop = None;
            return vec![Action::Dialog("brushKeyColor".into(), json!({ "color": color_json(&c) })), Action::SwitchTool(back)];
        }
        let stop = self.stop.as_ref().zip(cx.gradient_stop);
        // Reading a source is independent of its editability. Only Alt's reverse operation
        // writes to the clicked object; Shift+Alt appends its appearance to the selection.
        let source = if ev.mods.alt && !ev.mods.shift && stop.is_none() {
            hit_test(cx.doc, ev.pos, opts).map(|h| (paint_owner(cx.doc, h.leaf), h.kind))
        } else {
            sample_source(cx, ev.pos)
        };
        let Some((src, _)) = source else { return vec![] };
        let Some(n) = cx.doc.node(src) else { return vec![] };
        if ev.mods.alt && stop.is_none() {
            let how = if ev.mods.shift { "append" } else { "reverse" };
            return vec![Action::Exec("appearance.copyFrom".into(), json!({ "source": src.0, how: true }))];
        }
        let image = matches!(n.kind, NodeKind::Image(_));
        if !(ev.mods.shift || stop.is_some() || image) {
            return vec![Action::Exec("appearance.copyFrom".into(), json!({ "source": src.0 }))];
        }
        let Some(c) = sample_color_at(cx, ev.pos) else { return vec![] };
        let color = color_json(&c);
        let Some((back, i)) = stop else { return vec![Action::Exec("paint.sampleColor".into(), json!({ "color": color }))] };
        let out = vec![Action::Exec("paint.sampleColor".into(), json!({ "color": color, "stop": i })), Action::SwitchTool(back.clone())];
        self.stop = None;
        out
    }

    /// Expose a transient text range alongside gradient-stop and brush-key sampling state.
    fn options(&self) -> Value {
        let selected = self.text_selection.options();
        let mut options = if selected.get("editing").is_some_and(|v| !v.is_null()) { selected } else { json!({}) };
        if let Some(o) = options.as_object_mut() {
            o.insert("stop".into(), json!(self.stop));
            o.insert("brushKeyColor".into(), json!(self.brush_key_color));
        }
        options
    }

    /// Accept a stop target or the selection transferred from the Type tool.
    fn set_option(&mut self, key: &str, value: &Value) {
        match key {
            "stop" => self.stop = value.as_str().map(str::to_string),
            "brushKeyColor" => self.brush_key_color = value.as_bool().unwrap_or(false),
            "textSelection" => self.text_selection.set_option(key, value),
            _ => {}
        }
    }

    /// Drop the range when its text is removed or no longer selected.
    fn after_command(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.text_selection.after_command(cx)
    }

    /// Keep the sampled character range highlighted without activating text input.
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        if self.text_selection.options().get("editing").is_some_and(|v| !v.is_null()) { self.text_selection.overlays(cx) } else { vec![] }
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Eyedropper
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::Selection;

    /// Upstream brush-key sampling and highlighted-text transfer keep independent state.
    #[test]
    fn text_range_and_brush_sampling_options_coexist() {
        let mut tool = EyedropperTool::default();
        tool.set_option("textSelection", &json!({"editing": 1, "anchor": 0, "caret": 4}));
        tool.set_option("stop", &json!("selection"));
        tool.set_option("brushKeyColor", &json!(true));
        let options = tool.options();
        assert_eq!((options["editing"].as_u64(), options["start"].as_u64(), options["end"].as_u64()), (Some(1), Some(0), Some(4)));
        assert_eq!(options["stop"], "selection");
        assert_eq!(options["brushKeyColor"], true);
        tool.set_option("brushKeyColor", &json!(false));
        assert_eq!(tool.options()["editing"], 1);
        assert_eq!(tool.options()["brushKeyColor"], false);
    }

    #[test]
    fn click_copies_appearance_shift_samples_color() {
        let (d, id) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = EyedropperTool::default();
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        assert_eq!(a, vec![Action::Exec("appearance.copyFrom".into(), json!({"source": id.0}))]);
        let shift = Mods { shift: true, ..Default::default() };
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0).with_mods(shift));
        assert_eq!(a, vec![Action::Exec("paint.sampleColor".into(), json!({"color": color_json(&Color::WHITE)}))]);
        // On the stroke edge → the stroke colour.
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 150.0).with_mods(shift));
        assert_eq!(a, vec![Action::Exec("paint.sampleColor".into(), json!({"color": color_json(&Color::BLACK)}))]);
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 400.0, 400.0)).is_empty());
    }

    /// On a clip group's mask edge, where the art it clips lies over it, the eyedropper copies the
    /// art's appearance, not the unpainted clipping path's (the selection tools take the path there).
    #[test]
    fn on_a_masks_edge_the_eyedropper_copies_the_art() {
        let (d, _, _, art) = doc_with_masked_square();
        let (s, p) = (Selection::default(), paint());
        let a = EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 498.5, 450.0));
        assert_eq!(a, vec![Action::Exec("appearance.copyFrom".into(), json!({"source": art.0}))]);
    }

    /// The tools that act on what is drawn hit with options that don't take a clipping path on
    /// its outline; only the selection tools' do.
    #[test]
    fn only_the_selection_options_take_clipping_paths_on_their_outline() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let cx = cx(&d, &s, &p);
        assert!(!cx.hit_options().clip_outlines);
        assert!(cx.selection_hit_options().clip_outlines);
    }

    #[test]
    fn the_stop_option_samples_into_the_selected_stop_then_switches_back() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let mut cx = cx(&d, &s, &p);
        let mut t = EyedropperTool::default();
        t.set_option("stop", &json!("gradient"));
        assert_eq!(t.options(), json!({"stop": "gradient", "brushKeyColor": false}));
        // Without a selected stop the click copies the appearance as usual.
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        assert!(matches!(&a[0], Action::Exec(c, _) if c == "appearance.copyFrom"));
        cx.gradient_stop = Some(1);
        // Empty canvas: still waiting.
        assert!(t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 400.0, 400.0)).is_empty());
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 100.0, 150.0));
        assert_eq!(
            a,
            vec![
                Action::Exec("paint.sampleColor".into(), json!({"color": color_json(&Color::BLACK), "stop": 1})),
                Action::SwitchTool("gradient".into())
            ]
        );
        assert_eq!(t.options(), json!({"stop": null, "brushKeyColor": false}), "one sample only");
    }

    #[test]
    fn brush_key_color_samples_into_the_dialog_then_switches_back() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = EyedropperTool::default();
        t.set_option("brushKeyColor", &json!(true));
        t.set_option("stop", &json!("selection"));
        let a = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, 150.0, 150.0));
        assert_eq!(
            a,
            vec![Action::Dialog("brushKeyColor".into(), json!({ "color": color_json(&Color::WHITE) })), Action::SwitchTool("selection".into())]
        );
        assert_eq!(t.options(), json!({ "stop": null, "brushKeyColor": false }));
    }

    /// The hex of the colour a `paint.sampleColor` action samples.
    fn sampled_hex(a: &[Action]) -> String {
        let [Action::Exec(c, p)] = a else { panic!("one action: {a:?}") };
        assert_eq!(c, "paint.sampleColor");
        let v: Vec<f32> = serde_json::from_value(p["color"].clone()).unwrap();
        Color::rgb(v[0], v[1], v[2]).to_hex()
    }

    #[test]
    fn alt_sends_reverse_and_shift_alt_append() {
        let (d, id) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let mut t = EyedropperTool::default();
        let at = |m: Mods| PointerEvent::new(PointerKind::Down, 150.0, 150.0).with_mods(m);
        let alt = Mods { alt: true, ..Default::default() };
        assert_eq!(t.pointer(&cx(&d, &s, &p), &at(alt)), vec![Action::Exec("appearance.copyFrom".into(), json!({"source": id.0, "reverse": true}))]);
        let both = Mods { alt: true, shift: true, ..Default::default() };
        assert_eq!(t.pointer(&cx(&d, &s, &p), &at(both)), vec![Action::Exec("appearance.copyFrom".into(), json!({"source": id.0, "append": true}))]);
    }

    #[test]
    fn sampling_keeps_the_colour_model() {
        let (mut d, id) = doc_with_rect();
        let cmyk = Color::cmyk(0.1, 0.2, 0.3, 0.4);
        d.node_mut(id).unwrap().appearance.set_fill(Paint::solid(cmyk));
        d.node_mut(id).unwrap().locked = true;
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        let a = EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 150.0, 150.0).with_mods(shift));
        assert_eq!(a, vec![Action::Exec("paint.sampleColor".into(), json!({"color": color_json(&cmyk)}))]);
    }

    #[test]
    fn clicking_an_image_samples_its_pixels() {
        use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        // Three pixels, red, green, blue, 10 pt each.
        let img = image::RgbaImage::from_raw(3, 1, [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255]].concat()).unwrap();
        let mut png = vec![];
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        d.images.insert("px".into(), ImageBlob::png(png));
        let im = ImageObject {
            key: "px".into(),
            width: 3,
            height: 1,
            xf: vectorcraft_geom::Affine::scale(10.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(Some(l), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
        let (s, p) = (Selection::default(), paint());
        let mut cx = cx(&d, &s, &p);
        let click = PointerEvent::new(PointerKind::Down, 15.0, 5.0);
        assert_eq!(sampled_hex(&EyedropperTool::default().pointer(&cx, &click)), "#00ff00");
        cx.raster_sample = 3;
        assert_eq!(sampled_hex(&EyedropperTool::default().pointer(&cx, &click)), "#555555", "3 x 3 average");
        // A template layer and a locked image still supply pixels. Hidden layers do not.
        d.node_mut(id).unwrap().locked = true;
        if let NodeKind::Layer { template, .. } = &mut d.node_mut(l).unwrap().kind {
            *template = true;
        }
        assert_eq!(sampled_hex(&EyedropperTool::default().pointer(&crate::testutil::cx(&d, &s, &p), &click)), "#00ff00");
        d.node_mut(l).unwrap().visible = false;
        assert!(EyedropperTool::default().pointer(&crate::testutil::cx(&d, &s, &p), &click).is_empty());
    }

    /// #2276: a CMYK image gives its ink amounts as they are, not the RGB its decoder shows.
    #[test]
    fn clicking_a_cmyk_image_samples_its_inks() {
        use vectorcraft_doc::cmyk::Inks;
        use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        // Two pixels, 10 pt each: C10 M20 Y30 K40 (in bytes), then K100.
        let inks = Inks::new(2, 1, vec![26, 51, 77, 102, 0, 0, 0, 255]).unwrap();
        d.images.insert("ink".into(), ImageBlob::cmyk_tiff(&inks).unwrap());
        let im = ImageObject {
            key: "ink".into(),
            width: 2,
            height: 1,
            xf: vectorcraft_geom::Affine::scale(10.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(Some(l), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
        let (s, p) = (Selection::default(), paint());
        let mut cx = cx(&d, &s, &p);
        let ink = |v: [u8; 4]| {
            let [c, m, y, k] = v.map(|b| f32::from(b) / 255.0);
            json!({ "color": color_json(&Color::cmyk(c, m, y, k)) })
        };
        let click = PointerEvent::new(PointerKind::Down, 5.0, 5.0);
        assert_eq!(EyedropperTool::default().pointer(&cx, &click), vec![Action::Exec("paint.sampleColor".into(), ink([26, 51, 77, 102]))]);
        cx.raster_sample = 3;
        assert_eq!(
            EyedropperTool::default().pointer(&cx, &click),
            vec![Action::Exec("paint.sampleColor".into(), ink([13, 26, 39, 179]))],
            "3 x 3 average of both pixels"
        );
    }

    /// An unstroked rectangle filled with `fill`, with id 0 (as pattern and symbol art is).
    fn filled_rect(r: vectorcraft_geom::Rect, fill: Paint) -> std::sync::Arc<Node> {
        let mut a = vectorcraft_doc::Appearance::default();
        a.set_fill(fill);
        std::sync::Arc::new(Node::path(vectorcraft_doc::NodeId(0), vectorcraft_geom::shapes::rectangle(r), a))
    }

    /// [`doc_with_rect`] plus the pattern "Halves": a 20 × 10 tile, red on its left half and blue
    /// on its right half.
    fn doc_with_halves() -> (vectorcraft_doc::Document, vectorcraft_doc::NodeId) {
        use vectorcraft_geom::Rect;
        let (mut d, id) = doc_with_rect();
        let (red, blue) = (Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
        let mut def = vectorcraft_doc::PatternDef::new(
            "Halves",
            vec![filled_rect(Rect::new(0.0, 0.0, 10.0, 10.0), red), filled_rect(Rect::new(10.0, 0.0, 20.0, 10.0), blue)],
        );
        def.tile = Rect::new(0.0, 0.0, 20.0, 10.0);
        d.patterns.push(def);
        (d, id)
    }

    /// Shift-click on a pattern fill samples the color of the tile art under the cursor, where the
    /// pattern's own transform puts it, however small the tiles are.
    #[test]
    fn shift_click_samples_the_pattern_tile_under_the_cursor() {
        use vectorcraft_geom::Affine;
        let (mut d, id) = doc_with_halves();
        d.node_mut(id).unwrap().appearance.set_fill(Paint::Pattern { pattern: "Halves".into(), xf: Affine::translate((5.0, 0.0)) });
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        let at = |d: &vectorcraft_doc::Document, x: f64| {
            EyedropperTool::default().pointer(&cx(d, &s, &p), &PointerEvent::new(PointerKind::Down, x, 155.0).with_mods(shift))
        };
        // Pattern space x 128 (8 into a tile), then x 137 (17 into a tile).
        assert_eq!(sampled_hex(&at(&d, 133.0)), "#ff0000");
        assert_eq!(sampled_hex(&at(&d, 142.0)), "#0000ff");
        assert_eq!(crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), Point::new(142.0, 155.0)).map(|c| c.to_hex()), Some("#0000ff".into()));
        // Art 300 pt wide on a 1 pt tile: about 90,000 tiles reach every point.
        let mut wide = vectorcraft_doc::PatternDef::new(
            "Wide",
            vec![filled_rect(vectorcraft_geom::Rect::new(0.0, 0.0, 300.0, 300.0), Paint::solid(Color::rgb(0.0, 1.0, 0.0)))],
        );
        wide.tile = vectorcraft_geom::Rect::new(0.0, 0.0, 1.0, 1.0);
        let mut wide_doc = d.clone();
        wide_doc.patterns.push(wide);
        wide_doc.node_mut(id).unwrap().appearance.set_fill(vectorcraft_doc::pattern::pattern_paint("Wide"));
        assert_eq!(sampled_hex(&at(&wide_doc, 142.0)), "#00ff00");
        // Pattern space x 140, on the edge between two tiles.
        let edge = sampled_hex(&at(&d, 145.0));
        assert!(edge == "#ff0000" || edge == "#0000ff", "{edge}");
        // Tiles 0.2 × 0.1 pt, 0.05 pt down: pattern space y 15495, x 15005 (5 into a tile), then
        // x 15015 (15 into a tile).
        let tiny = Affine::translate((0.0, 0.05)) * Affine::scale(0.01);
        d.node_mut(id).unwrap().appearance.set_fill(Paint::Pattern { pattern: "Halves".into(), xf: tiny });
        assert_eq!(sampled_hex(&at(&d, 150.05)), "#ff0000");
        assert_eq!(sampled_hex(&at(&d, 150.15)), "#0000ff");
    }

    /// Tile art painted with another pattern is sampled where that tile draws it: the inner
    /// pattern moves with each tile. A symbol instance is sampled from its symbol's art.
    #[test]
    fn shift_click_samples_patterns_and_symbols_inside_art() {
        use vectorcraft_geom::{Affine, Rect};
        let (mut d, id) = doc_with_halves();
        // A 30 pt tile filled with "Halves": every other tile starts halfway through a "Halves" tile.
        let mut outer = vectorcraft_doc::PatternDef::new(
            "Outer",
            vec![filled_rect(Rect::new(0.0, 0.0, 30.0, 10.0), vectorcraft_doc::pattern::pattern_paint("Halves"))],
        );
        outer.tile = Rect::new(0.0, 0.0, 30.0, 10.0);
        d.patterns.push(outer);
        d.node_mut(id).unwrap().appearance.set_fill(vectorcraft_doc::pattern::pattern_paint("Outer"));
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        // The tile at x 150 draws "Halves" from x 150: x 165 is 15 into it.
        let a = EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 165.0, 155.0).with_mods(shift));
        assert_eq!(sampled_hex(&a), "#0000ff");
        // A symbol of a green square, placed at (300, 300) and doubled.
        d.symbols.push(vectorcraft_doc::Symbol {
            dynamic: false,
            name: "Green".into(),
            art: filled_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(Color::rgb(0.0, 1.0, 0.0))),
        });
        let l = d.layers[0].id;
        let inst = d.alloc_id();
        let kind = NodeKind::SymbolInstance { symbol: "Green".into(), xf: Affine::translate((300.0, 300.0)) * Affine::scale(2.0) };
        d.insert(Some(l), usize::MAX, Node::new(inst, kind)).unwrap();
        let a = EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 310.0, 310.0).with_mods(shift));
        assert_eq!(sampled_hex(&a), "#00ff00");
    }

    /// A symbol instance is sampled as it draws: its art stained by the instance's fill, and a
    /// pattern in its art left where the document puts it when the instance moves.
    #[test]
    fn symbol_samples_follow_the_instances_stain_and_placement() {
        use vectorcraft_geom::{Affine, Rect};
        let (mut d, _) = doc_with_halves();
        let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
        d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Red".into(), art: filled_rect(Rect::new(0.0, 0.0, 20.0, 10.0), red) });
        d.symbols.push(vectorcraft_doc::Symbol {
            dynamic: false,
            name: "Tiled".into(),
            art: filled_rect(Rect::new(0.0, 0.0, 20.0, 10.0), vectorcraft_doc::pattern::pattern_paint("Halves")),
        });
        let l = d.layers[0].id;
        let place = |d: &mut vectorcraft_doc::Document, name: &str, at: (f64, f64), stain: Option<Color>| {
            let id = d.alloc_id();
            let mut n = Node::new(id, NodeKind::SymbolInstance { symbol: name.into(), xf: Affine::translate(at) });
            if let Some(c) = stain {
                n.appearance.set_fill(Paint::solid(c));
            }
            d.insert(Some(l), usize::MAX, n).unwrap();
        };
        place(&mut d, "Red", (300.0, 300.0), Some(Color::rgb(0.0, 0.0, 1.0)));
        place(&mut d, "Tiled", (310.0, 400.0), None);
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        let at = |x: f64, y: f64| {
            sampled_hex(&EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, x, y).with_mods(shift)))
        };
        assert_eq!(at(305.0, 305.0), "#0000ff", "stained");
        // Document x 312 is 12 into a "Halves" tile, though only 2 into the instance's art.
        assert_eq!(at(312.0, 405.0), "#0000ff", "pattern in place");
    }

    /// Between the shapes of a stained instance's art, the eyedropper samples what lies below, not
    /// the stain; and a pattern tile is sampled wherever its symbol art reaches.
    #[test]
    fn symbol_art_decides_where_an_instance_paints() {
        use std::sync::Arc;
        use vectorcraft_doc::NodeId;
        use vectorcraft_geom::{Affine, Rect};
        let (mut d, id) = doc_with_rect();
        let red = || Paint::solid(Color::rgb(1.0, 0.0, 0.0));
        let pair =
            Node::group(NodeId(0), vec![filled_rect(Rect::new(0.0, 0.0, 10.0, 10.0), red()), filled_rect(Rect::new(20.0, 0.0, 30.0, 10.0), red())]);
        d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Pair".into(), art: Arc::new(pair) });
        let (l, inst) = (d.layers[0].id, d.alloc_id());
        let mut n = Node::new(inst, NodeKind::SymbolInstance { symbol: "Pair".into(), xf: Affine::translate((100.0, 140.0)) });
        n.appearance.set_fill(Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
        d.insert(Some(l), usize::MAX, n).unwrap();
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        let at = |d: &vectorcraft_doc::Document, x: f64, y: f64| {
            sampled_hex(&EyedropperTool::default().pointer(&cx(d, &s, &p), &PointerEvent::new(PointerKind::Down, x, y).with_mods(shift)))
        };
        assert_eq!((at(&d, 105.0, 145.0), at(&d, 115.0, 145.0)), ("#0000ff".to_string(), "#ffffff".to_string()));
        // A 200 pt tile holding an instance of a 180 pt green bar, which reaches far past the
        // square an instance is measured by without its symbol.
        let green = Paint::solid(Color::rgb(0.0, 1.0, 0.0));
        d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Bar".into(), art: filled_rect(Rect::new(0.0, 0.0, 180.0, 10.0), green) });
        let bar = Arc::new(Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: "Bar".into(), xf: Affine::IDENTITY }));
        let mut bars = vectorcraft_doc::PatternDef::new("Bars", vec![bar]);
        bars.tile = Rect::new(0.0, 0.0, 200.0, 10.0);
        d.patterns.push(bars);
        d.node_mut(id).unwrap().appearance.set_fill(vectorcraft_doc::pattern::pattern_paint("Bars"));
        assert_eq!(at(&d, 150.0, 155.0), "#00ff00");
    }

    /// A pattern nested more than three patterns deep, and a pattern the document lacks, sample as
    /// the gray the canvas draws them with.
    #[test]
    fn deep_and_missing_patterns_sample_as_the_canvas_draws_them() {
        use vectorcraft_doc::pattern::pattern_paint;
        use vectorcraft_geom::Rect;
        let (mut d, id) = doc_with_rect();
        // "P0" to "P2": a 10 pt tile filled with the next; "P3" a green tile.
        for i in 0..4 {
            let fill = if i == 3 { Paint::solid(Color::rgb(0.0, 1.0, 0.0)) } else { pattern_paint(&format!("P{}", i + 1)) };
            let mut def = vectorcraft_doc::PatternDef::new(&format!("P{i}"), vec![filled_rect(Rect::new(0.0, 0.0, 10.0, 10.0), fill)]);
            def.tile = Rect::new(0.0, 0.0, 10.0, 10.0);
            d.patterns.push(def);
        }
        let (s, p) = (Selection::default(), paint());
        let mut at = |name: &str| {
            d.node_mut(id).unwrap().appearance.set_fill(pattern_paint(name));
            sample_color_at(&cx(&d, &s, &p), Point::new(150.5, 150.5)).map(|c| c.to_hex())
        };
        assert_eq!(at("P1").as_deref(), Some("#00ff00"), "three patterns deep");
        assert_eq!(at("P0").as_deref(), Some("#808080"), "four patterns deep");
        assert_eq!(at("Nope").as_deref(), Some("#808080"), "missing");
    }

    /// Art nested deeper than a sample follows gives no color, rather than the color of what lies
    /// below it.
    #[test]
    fn art_nested_past_the_limit_gives_no_color() {
        use std::sync::Arc;
        use vectorcraft_doc::{NodeId, Symbol};
        use vectorcraft_geom::{Affine, Rect};
        let (mut d, _) = doc_with_rect();
        // "Deep0" to "Deep19": one instance of the next; "Deep20" a green square over the white
        // rectangle.
        for i in 0..20 {
            let next = Arc::new(Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: format!("Deep{}", i + 1), xf: Affine::IDENTITY }));
            d.symbols.push(Symbol { dynamic: false, name: format!("Deep{i}"), art: next });
        }
        d.symbols.push(Symbol {
            dynamic: false,
            name: "Deep20".into(),
            art: filled_rect(Rect::new(100.0, 100.0, 200.0, 200.0), Paint::solid(Color::rgb(0.0, 1.0, 0.0))),
        });
        let (l, inst) = (d.layers[0].id, d.alloc_id());
        d.insert(Some(l), usize::MAX, Node::new(inst, NodeKind::SymbolInstance { symbol: "Deep0".into(), xf: Affine::IDENTITY })).unwrap();
        let (s, p) = (Selection::default(), paint());
        assert_eq!(sample_color_at(&cx(&d, &s, &p), Point::new(150.5, 150.5)).map(|c| c.to_hex()), None);
    }

    /// A pattern with more tiles over the point than a sample looks at gives no color, rather than
    /// the color of what lies below it.
    #[test]
    fn too_many_tiles_over_the_point_give_no_color() {
        use vectorcraft_doc::pattern::pattern_paint;
        use vectorcraft_geom::{PathData, Rect, SubPath};
        let (mut d, _) = doc_with_rect();
        // An L 0.1 pt thick along the top and left of (0, 0)–(200, 200) on 1 pt tiles: thousands of
        // tiles reach (150.5, 150.5), and none of them paints it.
        let l: Vec<Point> = [(0.0, 0.0), (200.0, 0.0), (200.0, 0.1), (0.1, 0.1), (0.1, 200.0), (0.0, 200.0)].map(|(x, y)| Point::new(x, y)).into();
        let mut el = (*filled_rect(Rect::new(0.0, 0.0, 1.0, 1.0), Paint::solid(Color::BLACK))).clone();
        if let NodeKind::Path { path, .. } = &mut el.kind {
            *path = PathData::single(SubPath::polyline(&l, true));
        }
        let mut def = vectorcraft_doc::PatternDef::new("Ls", vec![std::sync::Arc::new(el)]);
        def.tile = Rect::new(0.0, 0.0, 1.0, 1.0);
        d.patterns.push(def);
        let (l, top) = (d.layers[0].id, d.alloc_id());
        let mut over = (*filled_rect(Rect::new(100.0, 100.0, 200.0, 200.0), pattern_paint("Ls"))).clone();
        over.id = top;
        d.insert(Some(l), usize::MAX, over).unwrap();
        let (s, p) = (Selection::default(), paint());
        assert_eq!(sample_color_at(&cx(&d, &s, &p), Point::new(150.5, 150.5)).map(|c| c.to_hex()), None);
    }

    /// Symbol art in a tile is sampled only in the tiles the canvas draws, which it lists by the
    /// art's bounds without its symbols.
    #[test]
    fn symbol_art_far_outside_its_tile_samples_as_the_canvas_draws_it() {
        use std::sync::Arc;
        use vectorcraft_doc::NodeId;
        use vectorcraft_geom::{Affine, Rect};
        let (mut d, id) = doc_with_rect();
        // A 20 pt tile holding an instance of a 10 pt green square that sits 100 pt right of it.
        // The canvas lists only the tile over a point, whose square lies 100 pt away, so it draws
        // no green at x 165, where the square of the tile at x 60 would lie, or at x 175.
        let green = Paint::solid(Color::rgb(0.0, 1.0, 0.0));
        d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Away".into(), art: filled_rect(Rect::new(100.0, 0.0, 110.0, 10.0), green) });
        let away = Arc::new(Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: "Away".into(), xf: Affine::IDENTITY }));
        let mut def = vectorcraft_doc::PatternDef::new("Away", vec![away]);
        def.tile = Rect::new(0.0, 0.0, 20.0, 10.0);
        d.patterns.push(def);
        d.node_mut(id).unwrap().appearance.set_fill(vectorcraft_doc::pattern::pattern_paint("Away"));
        let (s, p) = (Selection::default(), paint());
        let at = |x: f64| sample_color_at(&cx(&d, &s, &p), Point::new(x, 155.0)).map(|c| c.to_hex());
        assert_eq!((at(165.0), at(175.0)), (None, None));
    }

    /// Patterns whose tile art is painted with the next one, on tiles far smaller than their art,
    /// symbols whose art holds four instances of the next, and symbols nested far deeper than a
    /// sample follows: each sample stops within its work limit and gives no color, instead of
    /// trying every tile or instance at every level or running out of stack.
    #[test]
    fn nested_patterns_and_symbols_sample_within_the_work_limit() {
        use std::sync::Arc;
        use vectorcraft_doc::pattern::pattern_paint;
        use vectorcraft_doc::{NodeId, Symbol};
        use vectorcraft_geom::{Affine, Rect};
        let (mut d, id) = doc_with_rect();
        // "Gap": an L 0.1 pt thick along the top and left of (0, 0)–(20, 20) on 1 pt tiles, which
        // reaches every point from hundreds of tiles and paints none at x.5, y.5.
        let l: Vec<Point> = [(0.0, 0.0), (20.0, 0.0), (20.0, 0.1), (0.1, 0.1), (0.1, 20.0), (0.0, 20.0)].map(|(x, y)| Point::new(x, y)).into();
        let mut el = (*filled_rect(Rect::new(0.0, 0.0, 1.0, 1.0), Paint::solid(Color::BLACK))).clone();
        if let NodeKind::Path { path, .. } = &mut el.kind {
            *path = vectorcraft_geom::PathData::single(vectorcraft_geom::SubPath::polyline(&l, true));
        }
        let mut gap = vectorcraft_doc::PatternDef::new("Gap", vec![Arc::new(el)]);
        gap.tile = Rect::new(0.0, 0.0, 1.0, 1.0);
        d.patterns.push(gap);
        // "Nest0" and "Nest1": 300 pt art on 1 pt tiles, painted with the next ("Gap" after
        // "Nest1"), so no tile gives a color and every one is tried.
        for i in 0..2 {
            let next = if i == 1 { "Gap".to_string() } else { format!("Nest{}", i + 1) };
            let mut def =
                vectorcraft_doc::PatternDef::new(&format!("Nest{i}"), vec![filled_rect(Rect::new(0.0, 0.0, 300.0, 300.0), pattern_paint(&next))]);
            def.tile = Rect::new(0.0, 0.0, 1.0, 1.0);
            d.patterns.push(def);
        }
        d.node_mut(id).unwrap().appearance.set_fill(pattern_paint("Nest0"));
        // "Fan0" to "Fan11": four instances of the next symbol; "Fan12" a square filled with "Gap".
        let instance = |name: String| Arc::new(Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: name, xf: Affine::IDENTITY }));
        for i in 0..12 {
            let art = Node::group(NodeId(0), (0..4).map(|_| instance(format!("Fan{}", i + 1))).collect());
            d.symbols.push(Symbol { dynamic: false, name: format!("Fan{i}"), art: Arc::new(art) });
        }
        d.symbols.push(Symbol {
            dynamic: false,
            name: "Fan12".into(),
            art: filled_rect(Rect::new(300.0, 300.0, 400.0, 400.0), pattern_paint("Gap")),
        });
        // "Deep0" to "Deep99": one instance of the next; "Deep100" a green square.
        for i in 0..100 {
            d.symbols.push(Symbol { dynamic: false, name: format!("Deep{i}"), art: instance(format!("Deep{}", i + 1)) });
        }
        let green = Paint::solid(Color::rgb(0.0, 1.0, 0.0));
        d.symbols.push(Symbol { dynamic: false, name: "Deep100".into(), art: filled_rect(Rect::new(300.0, 0.0, 400.0, 100.0), green) });
        let l = d.layers[0].id;
        for name in ["Fan0", "Deep0"] {
            let inst = d.alloc_id();
            d.insert(Some(l), usize::MAX, Node::new(inst, NodeKind::SymbolInstance { symbol: name.into(), xf: Affine::IDENTITY })).unwrap();
        }
        let (s, p) = (Selection::default(), paint());
        let start = std::time::Instant::now();
        for at in [Point::new(150.5, 150.5), Point::new(350.5, 350.5), Point::new(350.5, 50.5)] {
            assert_eq!(sample_color_at(&cx(&d, &s, &p), at), None, "{at:?}");
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(60), "{:?}", start.elapsed());
    }

    /// Each hit test in a tile's art may measure the art of every symbol, so it counts the symbols'
    /// objects, also when it is the first one in that art and hits nothing.
    #[test]
    fn hit_tests_in_tile_art_count_the_symbols_they_may_measure() {
        use std::sync::Arc;
        use vectorcraft_doc::pattern::pattern_paint;
        use vectorcraft_doc::{NodeId, Symbol};
        use vectorcraft_geom::{Affine, PathData, Rect, SubPath};
        let (mut d, id) = doc_with_rect();
        // Tile art: an L 0.1 pt thick along the top and left of (0, 0)–(200, 200), which no 1 pt
        // tile places over (150.5, 150.5) though the art of a thousand tiles reaches it, and below
        // it an instance of "Dots" (200 squares), which most of those tiles place away from it.
        let l: Vec<Point> = [(0.0, 0.0), (200.0, 0.0), (200.0, 0.1), (0.1, 0.1), (0.1, 200.0), (0.0, 200.0)].map(|(x, y)| Point::new(x, y)).into();
        let el = filled_rect(Rect::new(0.0, 0.0, 1.0, 1.0), Paint::solid(Color::BLACK));
        let mut el = (*el).clone();
        if let NodeKind::Path { path, .. } = &mut el.kind {
            *path = PathData::single(SubPath::polyline(&l, true));
        }
        let green = || Paint::solid(Color::rgb(0.0, 1.0, 0.0));
        let dots = (0..200).map(f64::from).map(|k| filled_rect(Rect::new(k, k, k + 0.1, k + 0.1), green())).collect();
        d.symbols.push(Symbol { dynamic: false, name: "Dots".into(), art: Arc::new(Node::group(NodeId(0), dots)) });
        let far = Affine::translate((0.0, -250.0));
        let inst = Arc::new(Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: "Dots".into(), xf: far }));
        let mut def = vectorcraft_doc::PatternDef::new("Dotted", vec![Arc::new(el), inst]);
        def.tile = Rect::new(0.0, 0.0, 1.0, 1.0);
        d.patterns.push(def);
        d.node_mut(id).unwrap().appearance.set_fill(pattern_paint("Dotted"));
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let opts = sample_options(&cx);
        let symbol_nodes = d.symbols.iter().map(|s| s.art.count()).sum();
        let mut s = Sampler { doc: &d, opts, raster_sample: cx.raster_sample, symbol_nodes, work: MAX_WORK, exhausted: false, patterns: 0 };
        let mut levels: Vec<Option<Document>> = (0..MAX_NESTING).map(|_| None).collect();
        color_under(&mut s, &d, &mut levels, Point::new(150.5, 150.5), opts);
        // The symbols' objects count for each of at least a hundred tiles, not once for the pattern.
        assert!(MAX_WORK - s.work >= 100 * symbol_nodes, "{} spent, {symbol_nodes} symbol objects", MAX_WORK - s.work);
    }

    /// A gap in an object's top pattern fill shows the object's own fill below it.
    #[test]
    fn a_gap_in_a_top_fill_samples_the_fill_below() {
        use vectorcraft_doc::pattern::pattern_paint;
        use vectorcraft_doc::{AppearanceItem, FillLayer};
        use vectorcraft_geom::Rect;
        // The rectangle (100, 100)–(200, 200) filled with red 10 pt squares 20 pt apart over blue.
        let (mut d, id) = doc_with_rect();
        let mut gappy =
            vectorcraft_doc::PatternDef::new("Gappy", vec![filled_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)))]);
        gappy.tile = Rect::new(0.0, 0.0, 20.0, 10.0);
        d.patterns.push(gappy);
        let ap = &mut d.node_mut(id).unwrap().appearance;
        ap.set_fill(pattern_paint("Gappy"));
        ap.items.insert(0, AppearanceItem::Fill(FillLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0)))));
        let (s, p) = (Selection::default(), paint());
        let at = |x: f64| sample_color_at(&cx(&d, &s, &p), Point::new(x, 155.0)).map(|c| c.to_hex());
        // x 125 is 5 into a tile, on a red square; x 133 is 13 into one, between them.
        assert_eq!((at(125.0), at(133.0)), (Some("#ff0000".to_string()), Some("#0000ff".to_string())));
        // A hidden top fill is not drawn, so the fill below shows on the squares too.
        let mut d = d.clone();
        d.node_mut(id).unwrap().appearance.fill_mut().unwrap().visible = false;
        let at = |x: f64| sample_color_at(&cx(&d, &s, &p), Point::new(x, 155.0)).map(|c| c.to_hex());
        assert_eq!(at(125.0), Some("#0000ff".to_string()));
    }

    /// Of an object's strokes, the topmost one that reaches the point is sampled.
    #[test]
    fn the_topmost_stroke_that_reaches_the_point_is_sampled() {
        use vectorcraft_doc::{AppearanceItem, StrokeLayer};
        // The rectangle (100, 100)–(200, 200) with a 20 pt blue stroke under a 2 pt red one.
        let (mut d, id) = doc_with_rect();
        let ap = &mut d.node_mut(id).unwrap().appearance;
        ap.items.retain(|it| it.is_fill());
        ap.items.push(AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 20.0)));
        ap.items.push(AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), 2.0)));
        let (s, p) = (Selection::default(), paint());
        let at = |x: f64| sample_color_at(&cx(&d, &s, &p), Point::new(x, 150.0)).map(|c| c.to_hex());
        // x 100 is on both strokes; x 94 is on the blue one only.
        assert_eq!((at(100.0), at(94.0)), (Some("#ff0000".to_string()), Some("#0000ff".to_string())));
    }

    /// A gap between a pattern's tile art shows what lies below it, in the document and in the tile
    /// art of another pattern.
    #[test]
    fn a_gap_in_a_pattern_samples_what_lies_below() {
        use vectorcraft_doc::pattern::pattern_paint;
        use vectorcraft_geom::Rect;
        // The white rectangle (100, 100)–(200, 200), under one filled with red 10 pt squares 20 pt apart.
        let (mut d, _) = doc_with_rect();
        let mut gappy =
            vectorcraft_doc::PatternDef::new("Gappy", vec![filled_rect(Rect::new(0.0, 0.0, 10.0, 10.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)))]);
        gappy.tile = Rect::new(0.0, 0.0, 20.0, 10.0);
        d.patterns.push(gappy);
        let (l, top) = (d.layers[0].id, d.alloc_id());
        let mut over = (*filled_rect(Rect::new(100.0, 100.0, 200.0, 200.0), pattern_paint("Gappy"))).clone();
        over.id = top;
        d.insert(Some(l), usize::MAX, over).unwrap();
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        let at = |d: &vectorcraft_doc::Document, x: f64| {
            sampled_hex(&EyedropperTool::default().pointer(&cx(d, &s, &p), &PointerEvent::new(PointerKind::Down, x, 155.0).with_mods(shift)))
        };
        // x 125 is 5 into a tile, on a red square; x 133 is 13 into one, between them.
        assert_eq!((at(&d, 125.0), at(&d, 133.0)), ("#ff0000".to_string(), "#ffffff".to_string()));
        // Tile art: a green rectangle behind one filled with "Gappy".
        let green = Paint::solid(Color::rgb(0.0, 1.0, 0.0));
        let mut lined = vectorcraft_doc::PatternDef::new(
            "Lined",
            vec![filled_rect(Rect::new(0.0, 0.0, 20.0, 10.0), green), filled_rect(Rect::new(0.0, 0.0, 20.0, 10.0), pattern_paint("Gappy"))],
        );
        lined.tile = Rect::new(0.0, 0.0, 20.0, 10.0);
        d.patterns.push(lined);
        d.node_mut(top).unwrap().appearance.set_fill(pattern_paint("Lined"));
        assert_eq!((at(&d, 125.0), at(&d, 133.0)), ("#ff0000".to_string(), "#00ff00".to_string()));
        // A blue fill under a 20 pt stroke of "Gappy", 5 pt left: x 107 is 12 into a tile, in a gap
        // of the stroke over the fill.
        let shape = &mut d.node_mut(top).unwrap().appearance;
        shape.set_fill(Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
        shape.set_stroke(Paint::Pattern { pattern: "Gappy".into(), xf: vectorcraft_geom::Affine::translate((-5.0, 0.0)) });
        shape.stroke_mut().unwrap().width = 20.0;
        assert_eq!((at(&d, 103.0), at(&d, 107.0)), ("#ff0000".to_string(), "#0000ff".to_string()));
    }

    /// Shift-click on a gradient mesh samples the patch color interpolated at the cursor.
    #[test]
    fn shift_click_samples_the_mesh_color_under_the_cursor() {
        use vectorcraft_doc::{Document, GradientMesh};
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        // Black on the left edge, white on the right edge.
        let mesh = GradientMesh::from_surface(1, 1, &|u, v| Point::new(100.0 + 100.0 * u, 100.0 + 100.0 * v), &|u, _| {
            if u < 0.5 { Color::rgb(0.0, 0.0, 0.0) } else { Color::rgb(1.0, 1.0, 1.0) }
        });
        d.insert(Some(l), 0, Node::new(id, NodeKind::Mesh(mesh))).unwrap();
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        let a = EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 125.0, 150.0).with_mods(shift));
        assert_eq!(sampled_hex(&a), "#404040");
    }

    /// Finding the point in a mesh counts the points it looks at in every patch: a 20 × 20 mesh
    /// takes more than a thousand units of work.
    #[test]
    fn mesh_samples_count_the_work_of_finding_the_point() {
        use vectorcraft_doc::{Document, GradientMesh};
        let mut d = Document::new(500.0, 500.0);
        let (l, id) = (d.layers[0].id, d.alloc_id());
        let mesh = GradientMesh::from_surface(20, 20, &|u, v| Point::new(100.0 + 100.0 * u, 100.0 + 100.0 * v), &|_, _| Color::rgb(0.0, 1.0, 0.0));
        d.insert(Some(l), 0, Node::new(id, NodeKind::Mesh(mesh))).unwrap();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let opts = sample_options(&cx);
        let mut s = Sampler { doc: &d, opts, raster_sample: cx.raster_sample, symbol_nodes: 0, work: 1_000, exhausted: false, patterns: 0 };
        let mut levels: Vec<Option<Document>> = (0..MAX_NESTING).map(|_| None).collect();
        assert_eq!(color_under(&mut s, &d, &mut levels, Point::new(150.0, 150.0), opts).map(|c| c.to_hex()), None);
        assert!(s.exhausted);
    }

    #[test]
    fn gradient_sampled_along_vector() {
        let g = vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default());
        let paint = Paint::Gradient(Box::new(g));
        let r = vectorcraft_geom::Rect::new(0.0, 0.0, 100.0, 10.0);
        assert_eq!(paint_color_at(&paint, Some(r), Point::new(0.0, 5.0)).unwrap().to_hex(), "#ffffff");
        assert_eq!(paint_color_at(&paint, Some(r), Point::new(100.0, 5.0)).unwrap().to_hex(), "#000000");
    }

    #[test]
    fn type_gradients_are_sampled_in_text_space() {
        use vectorcraft_color::{Gradient, GradientGeom, GradientPaint};
        use vectorcraft_doc::{CharStyle, Document, Node, NodeKind, TextObject};
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        // Type turned 90°: its baseline runs down from (100, 100).
        let mut t = TextObject::point(Point::ZERO, "Gradient", CharStyle { size: 20.0, ..Default::default() });
        t.xf = vectorcraft_geom::Affine::translate((100.0, 100.0)) * vectorcraft_geom::Affine::rotate(std::f64::consts::FRAC_PI_2);
        let mut g = GradientPaint::new(Gradient::default());
        g.geom = Some(GradientGeom { start: Point::ZERO, end: Point::new(100.0, 0.0), aspect: 1.0, focal: None });
        t.runs[0].style.fill = Paint::Gradient(Box::new(g));
        d.insert(Some(l), 0, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        // 80 % along the baseline (text space (80, -5)).
        let a = EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 105.0, 180.0).with_mods(shift));
        assert_eq!(sampled_hex(&a), "#333333");
    }

    #[test]
    fn radial_sample_honours_aspect() {
        let mut g = vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient {
            kind: vectorcraft_color::GradientKind::Radial,
            ..Default::default()
        });
        // Radius 100 along x, 50 across: 50 pt above the centre is already the outer (black) edge.
        g.geom = Some(vectorcraft_color::GradientGeom { start: Point::ZERO, end: Point::new(100.0, 0.0), aspect: 0.5, focal: None });
        let paint = Paint::Gradient(Box::new(g));
        assert_eq!(paint_color_at(&paint, None, Point::new(0.0, -50.0)).unwrap().to_hex(), "#000000");
        assert_eq!(paint_color_at(&paint, None, Point::new(50.0, 0.0)).unwrap().to_hex(), "#808080");
    }

    /// An unstroked rectangle of `d` filled with `color`.
    fn solid_rect(d: &mut vectorcraft_doc::Document, r: vectorcraft_geom::Rect, color: Color) -> Node {
        let a = vectorcraft_doc::Appearance::basic(Paint::solid(color), Paint::None, 0.0);
        Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(r), a)
    }

    /// [`doc_with_rect`] plus a green rectangle (290, 290)–(430, 350) and, over it, `live` made by
    /// `make` from the document.
    fn doc_with_live_over_green(make: impl FnOnce(&mut vectorcraft_doc::Document) -> Node) -> vectorcraft_doc::Document {
        use vectorcraft_geom::Rect;
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let green = solid_rect(&mut d, Rect::new(290.0, 290.0, 430.0, 350.0), Color::rgb(0.0, 1.0, 0.0));
        d.insert(Some(l), usize::MAX, green).unwrap();
        let live = make(&mut d);
        d.insert(Some(l), usize::MAX, live).unwrap();
        d
    }

    /// Shift-click on a blend samples the step drawn under the cursor, and a gap between its steps
    /// samples what lies below.
    #[test]
    fn shift_click_samples_the_blend_step_under_the_cursor() {
        use vectorcraft_doc::live::{BlendSpacing, BlendSpec};
        use vectorcraft_geom::Rect;
        // Red (300, 300)–(320, 320) and blue (400, 300)–(420, 320) with 1 step, half red and half
        // blue, at x 350 to 370.
        let d = doc_with_live_over_green(|d| {
            let red = solid_rect(d, Rect::new(300.0, 300.0, 320.0, 320.0), Color::rgb(1.0, 0.0, 0.0));
            let blue = solid_rect(d, Rect::new(400.0, 300.0, 420.0, 320.0), Color::rgb(0.0, 0.0, 1.0));
            let spec = BlendSpec { spacing: BlendSpacing::Steps(1), ..Default::default() };
            Node::new(d.alloc_id(), NodeKind::Blend { children: vec![std::sync::Arc::new(red), std::sync::Arc::new(blue)], spec: Box::new(spec) })
        });
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        let at = |x: f64| EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, x, 310.0).with_mods(shift));
        let mid = Color::rgb(0.5, 0.0, 0.5).to_hex();
        assert_eq!(sampled_hex(&at(360.0)), mid);
        assert_eq!(crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), Point::new(360.0, 310.0)).map(|c| c.to_hex()), Some(mid));
        assert_eq!(sampled_hex(&at(310.0)), "#ff0000");
        // Between the red key and the step (x 320 to 350), and below the step.
        assert_eq!(sampled_hex(&at(335.0)), "#00ff00");
        let below = EyedropperTool::default().pointer(&cx(&d, &s, &p), &PointerEvent::new(PointerKind::Down, 360.0, 325.0).with_mods(shift));
        assert_eq!(sampled_hex(&below), "#00ff00");
    }

    /// A blend of two instances of one symbol draws its steps as instances too: Shift-click on a
    /// step samples the symbol's art there, and where that art leaves the point empty, what lies
    /// below.
    #[test]
    fn shift_click_samples_symbol_art_in_a_blends_steps() {
        use vectorcraft_doc::live::{BlendSpacing, BlendSpec};
        use vectorcraft_geom::{Affine, Rect};
        // A red 20 pt square at the symbol's origin, placed at x 300 and 400, with 1 step at x 350.
        let d = doc_with_live_over_green(|d| {
            let art = solid_rect(d, Rect::new(0.0, 0.0, 20.0, 20.0), Color::rgb(1.0, 0.0, 0.0));
            d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Dot".into(), art: std::sync::Arc::new(art) });
            let mut instance = |x: f64| {
                let kind = NodeKind::SymbolInstance { symbol: "Dot".into(), xf: Affine::translate((x, 300.0)) };
                std::sync::Arc::new(Node::new(d.alloc_id(), kind))
            };
            let children = vec![instance(300.0), instance(400.0)];
            let spec = BlendSpec { spacing: BlendSpacing::Steps(1), ..Default::default() };
            Node::new(d.alloc_id(), NodeKind::Blend { children, spec: Box::new(spec) })
        });
        let (s, p) = (Selection::default(), paint());
        let sample = |x: f64, y: f64| crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), Point::new(x, y)).map(|c| c.to_hex());
        assert_eq!(sample(360.0, 310.0), Some("#ff0000".into()));
        assert_eq!(sample(335.0, 310.0), Some("#00ff00".into()));
        assert_eq!(sample(360.0, 325.0), Some("#00ff00".into()));
    }

    /// Align to Path turns a blend's keys and steps to its spine, past the keys' bounds: a click
    /// there hits the blend and samples what it draws.
    #[test]
    fn shift_click_samples_blend_art_turned_past_its_bounds() {
        use vectorcraft_doc::live::{BlendOrientation, BlendSpacing, BlendSpec};
        use vectorcraft_geom::Rect;
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let green = solid_rect(&mut d, Rect::new(20.0, 20.0, 280.0, 280.0), Color::rgb(0.0, 1.0, 0.0));
        d.insert(Some(l), usize::MAX, green).unwrap();
        // 100 × 10 keys centred on (100, 100) and (200, 200): Align to Path turns them 45°, and the
        // red one reaches (70, 66), above the keys' bounds (y 95 to 205).
        let red = solid_rect(&mut d, Rect::new(50.0, 95.0, 150.0, 105.0), Color::rgb(1.0, 0.0, 0.0));
        let blue = solid_rect(&mut d, Rect::new(150.0, 195.0, 250.0, 205.0), Color::rgb(0.0, 0.0, 1.0));
        let spec = BlendSpec { spacing: BlendSpacing::Steps(1), orientation: BlendOrientation::AlignToPath, ..Default::default() };
        let id = d.alloc_id();
        let blend = Node::new(id, NodeKind::Blend { children: vec![std::sync::Arc::new(red), std::sync::Arc::new(blue)], spec: Box::new(spec) });
        d.insert(Some(l), usize::MAX, blend).unwrap();
        let (s, p) = (Selection::default(), paint());
        let at = Point::new(70.0, 66.0);
        assert!(d.node(id).unwrap().geometric_bounds().is_some_and(|b| !b.contains(at)));
        assert_eq!(hit_test(&d, at, cx(&d, &s, &p).hit_options()).map(|h| h.leaf), Some(id));
        assert_eq!(crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), at).map(|c| c.to_hex()), Some("#ff0000".into()));
        // Where the turned key leaves the point empty, the click passes the blend by.
        assert_eq!(crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), Point::new(60.0, 90.0)).map(|c| c.to_hex()), Some("#00ff00".into()));
        // A hidden red key does not hit there.
        if let Some(NodeKind::Blend { children, .. }) = d.node_mut(id).map(|n| &mut n.kind) {
            std::sync::Arc::make_mut(&mut children[0]).visible = false;
        }
        assert_eq!(hit_test(&d, at, cx(&d, &s, &p).hit_options()).map(|h| h.leaf), Some(d.layers[0].children().unwrap()[1].id));
        // A red key clipped to its right half, centred on (125, 100): the line to the blue key's
        // centre turns it about 53°, and its hidden left half reaches (89, 52).
        let mut clip = solid_rect(&mut d, Rect::new(100.0, 95.0, 150.0, 105.0), Color::BLACK);
        clip.appearance = Default::default();
        if let NodeKind::Path { clipping, .. } = &mut clip.kind {
            *clipping = true;
        }
        let red = solid_rect(&mut d, Rect::new(50.0, 95.0, 150.0, 105.0), Color::rgb(1.0, 0.0, 0.0));
        let key = Node::new(d.alloc_id(), NodeKind::Group { children: vec![std::sync::Arc::new(clip), std::sync::Arc::new(red)], clip: true });
        if let Some(NodeKind::Blend { children, .. }) = d.node_mut(id).map(|n| &mut n.kind) {
            children[0] = std::sync::Arc::new(key);
        }
        let hidden = Point::new(89.0, 52.0);
        assert!(d.node(id).unwrap().geometric_bounds().is_some_and(|b| !b.contains(hidden)));
        assert_eq!(hit_test(&d, hidden, cx(&d, &s, &p).hit_options()).map(|h| h.leaf), Some(d.layers[0].children().unwrap()[1].id));
        assert_eq!(crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), hidden).map(|c| c.to_hex()), Some("#00ff00".into()));
    }

    /// The stroke of an envelope's distorted contents paints past the envelope's bounds: a click
    /// there hits the envelope and samples the stroke.
    #[test]
    fn shift_click_samples_an_envelopes_stroke_past_its_bounds() {
        use vectorcraft_geom::{Affine, Rect};
        let mut id = None;
        let d = doc_with_live_over_green(|d| {
            let a = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::solid(Color::rgb(0.0, 0.0, 1.0)), 10.0);
            let red = Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(300.0, 300.0, 420.0, 320.0)), a);
            let kind = vectorcraft_doc::EnvelopeKind::Warp { style: "arc".into(), bend: 50.0, h: 0.0, v: 0.0, horizontal: true };
            let content = vec![std::sync::Arc::new(red)];
            let n = Node::new(
                d.alloc_id(),
                NodeKind::Envelope {
                    content,
                    kind: Box::new(kind),
                    fidelity: 50.0,
                    editing: false,
                    options: Default::default(),
                    frame: Box::new(Affine::IDENTITY),
                },
            );
            id = Some(n.id);
            n
        });
        let (s, p) = (Selection::default(), paint());
        // The middle of the band's top edge stays at y 300, and its 10 pt stroke reaches y 295.
        let at = Point::new(360.0, 296.0);
        assert!(d.node(id.unwrap()).unwrap().geometric_bounds().is_some_and(|b| !b.inflate(3.0, 3.0).contains(at)));
        assert_eq!(hit_test(&d, at, cx(&d, &s, &p).hit_options()).map(|h| h.leaf), id);
        assert_eq!(crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), at).map(|c| c.to_hex()), Some("#0000ff".into()));
    }

    /// Shift-click on an envelope samples its distorted contents where they are drawn, and a point
    /// in its bounds that they leave empty samples what lies below.
    #[test]
    fn shift_click_samples_the_envelopes_distorted_contents() {
        use vectorcraft_geom::{Affine, Rect};
        let d = doc_with_live_over_green(|d| {
            let red = solid_rect(d, Rect::new(300.0, 300.0, 420.0, 320.0), Color::rgb(1.0, 0.0, 0.0));
            let kind = vectorcraft_doc::EnvelopeKind::Warp { style: "arc".into(), bend: 50.0, h: 0.0, v: 0.0, horizontal: true };
            let content = vec![std::sync::Arc::new(red)];
            let options = Default::default();
            Node::new(
                d.alloc_id(),
                NodeKind::Envelope { content, kind: Box::new(kind), fidelity: 50.0, editing: false, options, frame: Box::new(Affine::IDENTITY) },
            )
        });
        let (s, p) = (Selection::default(), paint());
        let sample = |x: f64, y: f64| crate::xform::eyedropper::sample_color_at(&cx(&d, &s, &p), Point::new(x, y)).map(|c| c.to_hex());
        let art = vectorcraft_effects::expand_live_deep(Some(&d), d.layers[0].children().unwrap().last().unwrap());
        let b = art.geometric_bounds().unwrap();
        // The arc bends the ends of the band down, below the 20 pt it was, and keeps its middle.
        assert!(b.y1 > 330.0, "{b:?}");
        assert_eq!(sample(360.0, 310.0), Some("#ff0000".into()));
        assert_eq!(sample(310.0, 332.0), Some("#ff0000".into()));
        // Only what the art paints counts: just past the bent end's edge, within the click
        // tolerance of it, the green below shows.
        assert_eq!(sample(305.0, 332.0), Some("#00ff00".into()));
        assert_eq!(sample(360.0, 332.0), Some("#00ff00".into()));
        assert_eq!(sample(b.x0 + 0.5, b.y0 + 1.0), Some("#00ff00".into()));
    }
}
