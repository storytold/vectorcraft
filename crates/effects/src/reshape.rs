//! Object-level effects on objects that aren't paths: type, placed images, symbol instances and
//! live objects (blends, envelopes, meshes, repeats).
//!
//! Geometry effects reach them through outlines: [`outline_art`] turns the object into plain paths
//! (glyph outlines, the evaluated live object, the symbol's art) and [`reshape`] runs each effect
//! over all of them as one piece, with the whole art's bounds as the effect's reference box, so a
//! warp bends a line of type as a whole. An image is clipped by its reshaped frame, and a Warp
//! effect bends its pixels with the frame ([`cut_warped_images`]). Raster effects (shadows, glows, blur,
//! feather) stay on the result for the renderer and exporters. Type without
//! inline graphics whose own fills or strokes have geometry effects is outlined too: each of those
//! effects reshapes the whole outline that its fill or stroke paints. An image's own fills and
//! strokes paint only the shape that a Convert to Shape or Outline Object effect on them gives
//! them ([`image_rows`]): Outline Object follows the outline of the image's opaque area
//! ([`opaque_area`]).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

use vectorcraft_doc::live::Hooks;
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, ImageObject, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Shape, shapes};

use crate::{Budget, GeomContext, OutlineHook, apply_one, is_geometry, merged_params};

/// Can object-level effects apply to `n` only through [`outline_art`] / an offscreen layer (it is
/// neither a path nor a container)?
pub fn needs_outline(n: &Node) -> bool {
    matches!(
        n.kind,
        NodeKind::Text(_)
            | NodeKind::Image(_)
            | NodeKind::SymbolInstance { .. }
            | NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
    )
}

/// Is `n` a path or compound path with a visible Transform effect that makes copies and a gradient
/// in a fill or stroke? Such an object is drawn through its art ([`outline_art`]) so that each copy
/// paints its gradient by itself ([`copy_leaf`]), as a group's copies do.
pub fn has_gradient_copies(n: &Node) -> bool {
    (n.appearance.has_unplaced_gradient() || n.appearance.has_placed_gradient()) && has_transform_copies(n)
}

/// Is `n` a path or compound path with a visible Transform effect that makes copies and fills and
/// strokes that paint the copies differently as one path than as separate objects
/// ([`paints_copies_apart`])? Such an object is drawn through its art ([`outline_art`]), each copy
/// an object of its own below the one before ([`copy_leaf`]), as Expand Appearance makes it
/// (#5920, #6316).
pub(crate) fn has_layered_copies(n: &Node) -> bool {
    leaf_geometry(n).is_some_and(|(_, rule)| paints_copies_apart(&n.appearance, rule)) && has_transform_copies(n)
}

/// Do the fills and strokes `a` of a path with fill rule `rule` paint a Transform effect's copies
/// differently when the copies are one path than when each is an object of its own? They do
/// unless they paint once, in full and in Normal mode, with a stroke or a nonzero fill: then the
/// one path paints what the copies paint together. A fill and a stroke paint each copy's stroke
/// under the fills of the copies above it, a second fill or stroke likewise, the even-odd rule
/// leaves holes where copies overlap, and a partly transparent or blended fill or stroke paints
/// overlapping copies twice (#5920).
fn paints_copies_apart(a: &Appearance, rule: FillRule) -> bool {
    let mut painting = a.items.iter().filter(|i| crate::paints(i));
    let (Some(one), None) = (painting.next(), painting.next()) else {
        // None paints (nothing to draw either way), or several do.
        return a.items.iter().any(crate::paints);
    };
    one.opacity() < 1.0 || one.blend() != Default::default() || (one.is_fill() && rule != FillRule::NonZero)
}

/// Is `n` a path or compound path with a visible Transform effect that makes copies? Expand
/// Appearance makes each copy an object of its own ([`expand_outlined`]).
pub fn has_transform_copies(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. })
        && geometry_effects(n)
            .iter()
            .any(|e| e.id == "distort.transform" && crate::distort::transform_map(Rect::ZERO, &merged_params(&e.id, &e.params), 0).1 > 0)
}

/// Is `n` a path or compound path with a visible Transform effect whose Transform Patterns or
/// Scale Strokes & Effects is on (#885), its own or one of a fill or stroke that paints
/// ([`item_transform_options`])? Such an effect changes the fills and strokes as well as the
/// geometry, so the object is drawn through its art ([`outline_art`]), each copy an object of its
/// own with its strokes scaled and its patterns moved ([`transform_leaf`], [`transform_items`]).
pub fn has_transform_options(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. })
        && (geometry_effects(n)
            .iter()
            // Both options default to off, so the stored params tell without merging the defaults
            // (this runs for every object drawn).
            .any(|e| e.id == "distort.transform" && crate::distort::TransformOptions::of(&e.params).reach_appearance())
            || n.appearance.items.iter().any(item_transform_options))
}

/// Does `item` paint and carry a Transform effect of its own that [`reaches_appearance`]?
fn item_transform_options(item: &AppearanceItem) -> bool {
    crate::paints(item) && item.effects().iter().any(reaches_appearance)
}

/// Is `e` a visible Transform effect whose Transform Patterns or Scale Strokes & Effects is on?
fn reaches_appearance(e: &Effect) -> bool {
    e.visible && e.id == "distort.transform" && crate::distort::TransformOptions::of(&e.params).reach_appearance()
}

/// Is `n` a path or compound path that its Transform effects make into art of separate objects
/// ([`has_gradient_copies`], [`has_layered_copies`], [`has_transform_options`]), or whose
/// live-effect plug-ins paint what they return ([`has_painted_effects`])? It is drawn, baked and
/// expanded through that art ([`reshape`]).
pub fn has_transform_art(n: &Node) -> bool {
    has_gradient_copies(n) || has_layered_copies(n) || has_transform_options(n) || has_painted_effects(n)
}

/// Is `e` a visible live-effect plug-in whose manifest declares `paintOutput`: its id's plug-in id,
/// else `None`.
fn painted_plugin(e: &Effect) -> Option<&str> {
    let id = vectorcraft_plugins::effect::plugin_id(&e.id).filter(|_| e.visible)?;
    vectorcraft_plugins::effect::paints_output(id).then_some(id)
}

/// Is `n` a path or compound path with a live-effect plug-in that paints what it returns
/// ([`painted_plugin`]), its own or on a fill or stroke that paints? Each piece the plug-in returns
/// is then an object of its own with its own paints ([`paint_leaf`], [`transform_item_leaf`]).
pub fn has_painted_effects(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. })
        && (n.appearance.effects.iter().any(|e| painted_plugin(e).is_some())
            || n.appearance.items.iter().any(|i| crate::paints(i) && i.effects().iter().any(|e| painted_plugin(e).is_some())))
}

/// Is `n` type without inline graphics that has a fill or stroke of its own that paints and has a
/// visible geometry effect, an image with a fill or stroke of its own that paints a shape
/// ([`image_rows`]), or a symbol instance whose active fill or stroke has a visible geometry effect
/// ([`active_fill`], [`active_stroke`])? On type, such an effect reshapes the whole outline that its
/// fill or stroke paints ([`outline_text`], which leaves inline graphics out). On a symbol
/// instance, it reshapes the fills or strokes of the art it places ([`add_instance_row_effects`]).
pub fn has_item_geometry(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Text(t) => {
            t.runs.iter().all(|r| r.inline.is_none()) && n.appearance.items.iter().any(|i| crate::paints(i) && crate::has_geometry(i.effects()))
        }
        NodeKind::Image(_) => n.appearance.items.iter().any(shapes_image),
        NodeKind::SymbolInstance { .. } => {
            active_fill(n).is_some_and(|f| crate::has_geometry(&f.effects)) || active_stroke(n).is_some_and(|s| crate::has_geometry(&s.effects))
        }
        _ => false,
    }
}

/// The fill of symbol instance `n` whose effects its art takes: its topmost fill, the one that
/// stains the art, when it is visible and its opacity is above 0. A lower fill's effects do not
/// reach the art.
fn active_fill(n: &Node) -> Option<&vectorcraft_doc::FillLayer> {
    n.appearance.fill().filter(|f| f.visible && f.opacity > 0.0)
}

/// The stroke of symbol instance `n` whose effects its art takes: its topmost stroke, when it is
/// visible and its opacity is above 0.
fn active_stroke(n: &Node) -> Option<&vectorcraft_doc::StrokeLayer> {
    n.appearance.stroke().filter(|s| s.visible && s.opacity > 0.0)
}

/// Is `n` a symbol instance whose active fill or stroke has a visible raster effect (a shadow, glow,
/// blur, feather or other painted effect, [`crate::is_raster`])? Its art takes that effect on each
/// fill or stroke in it that paints ([`add_instance_row_effects`]), and draws it there.
pub fn has_instance_row_raster(n: &Node) -> bool {
    let raster = |fx: &[Effect]| fx.iter().any(|e| e.visible && crate::is_raster(&e.id));
    matches!(n.kind, NodeKind::SymbolInstance { .. })
        && (active_fill(n).is_some_and(|f| raster(&f.effects)) || active_stroke(n).is_some_and(|s| raster(&s.effects)))
}

/// The visible geometry and raster effects in `fx`, in order: those a symbol instance's active fill
/// or stroke passes on to its art.
fn row_effects_of(fx: &[Effect]) -> Vec<Effect> {
    fx.iter().filter(|e| e.visible && (is_geometry(&e.id) || crate::is_raster(&e.id))).cloned().collect()
}

/// `art`, the art symbol instance `n` places, with the visible geometry and raster effects of
/// `n`'s active fill added after the effects of every fill in it that paints, and those of its
/// active stroke after the effects of every stroke in it that paints ([`active_fill`],
/// [`active_stroke`]): a Drop Shadow on the instance's fill shadows each filled shape in the art.
/// Each fill or stroke keeps its paint, and the others keep their shapes. Live objects in it that are
/// reached are evaluated first (with the symbols of `doc`), so the effects reshape what they draw.
/// Clip groups' clipping paths, guides and the art of opacity masks keep their shapes. A symbol
/// instance in the art takes the effects on its own active fill or stroke, or on a new fill or
/// stroke without paint when it has none, and passes them on to its own art in turn; an instance
/// of a symbol whose art (in `doc`) holds an instance of `n`'s symbol, at any depth, keeps its
/// appearance, so the effects never pass round a symbol that holds itself. Without `doc`, no
/// instance in the art takes them.
pub(crate) fn add_instance_row_effects(doc: Option<&Document>, n: &Node, art: &mut Node) {
    let NodeKind::SymbolInstance { symbol, .. } = &n.kind else { return };
    let fill = active_fill(n).map_or(vec![], |f| row_effects_of(&f.effects));
    let stroke = active_stroke(n).map_or(vec![], |s| row_effects_of(&s.effects));
    if !fill.is_empty() || !stroke.is_empty() {
        add_row_effects(doc, symbol, art, &fill, &stroke);
    }
}

/// Does the art of the symbol named `from` in `doc` hold an instance of the symbol named `to`, at
/// any depth of the symbols placed in it? `seen` holds the symbols already looked into.
fn places_symbol(doc: &Document, from: &str, to: &str, seen: &mut HashSet<String>) -> bool {
    if !seen.insert(from.to_string()) {
        return false;
    }
    let Some(sym) = doc.symbols.iter().find(|s| s.name == from) else { return false };
    let mut names = vec![];
    sym.art.walk(&mut |c| {
        if let NodeKind::SymbolInstance { symbol, .. } = &c.kind {
            names.push(symbol.clone());
        }
    });
    names.iter().any(|name| name == to || places_symbol(doc, name, to, seen))
}

/// [`add_instance_row_effects`] of `fill` and `stroke` to `n` and the art in it, the art of an
/// instance of the symbol named `outer`.
fn add_row_effects(doc: Option<&Document>, outer: &str, n: &mut Node, fill: &[Effect], stroke: &[Effect]) {
    match &n.kind {
        NodeKind::Path { clipping: true, .. } | NodeKind::Path { guide: true, .. } | NodeKind::RulerGuide { .. } => return,
        NodeKind::SymbolInstance { symbol, .. } => {
            if let Some(d) = doc
                && !places_symbol(d, symbol, outer, &mut HashSet::new())
            {
                add_to_instance(n, fill, stroke);
            }
            return;
        }
        _ if vectorcraft_doc::live::is_live(n) => *n = evaluated(doc, n),
        _ => {}
    }
    let takes = |i: &AppearanceItem| {
        crate::paints(i) && !matches!(i, AppearanceItem::Fill(_) if fill.is_empty()) && !matches!(i, AppearanceItem::Stroke(_) if stroke.is_empty())
    };
    if n.appearance.items.iter().any(takes) {
        for item in n.appearance.items.iter_mut().filter(|i| takes(i)) {
            let fx = if matches!(item, AppearanceItem::Fill(_)) { fill } else { stroke };
            item.effects_mut().extend_from_slice(fx);
        }
    }
    // A compound path paints its members with its own fills and strokes.
    if matches!(n.kind, NodeKind::Compound { .. }) {
        return;
    }
    // A clip group's first member is its clipping path, whatever its kind.
    let clips = n.clips();
    for (i, c) in n.children_mut().into_iter().flatten().enumerate() {
        if !(clips && i == 0) {
            add_row_effects(doc, outer, Arc::make_mut(c), fill, stroke);
        }
    }
}

/// Add `fill` to the effects of symbol instance `n`'s active fill and `stroke` to those of its
/// active stroke ([`active_fill`], [`active_stroke`]); without one, to a new topmost fill or stroke
/// without paint, which neither stains nor strokes its art.
fn add_to_instance(n: &mut Node, fill: &[Effect], stroke: &[Effect]) {
    if !fill.is_empty() {
        match n.appearance.fill_mut().filter(|f| f.visible && f.opacity > 0.0) {
            Some(f) => f.effects.extend_from_slice(fill),
            None => {
                let mut f = vectorcraft_doc::FillLayer::new(vectorcraft_doc::color::Paint::None);
                f.effects = fill.to_vec();
                n.appearance.items.push(AppearanceItem::Fill(f));
            }
        }
    }
    if !stroke.is_empty() {
        match n.appearance.stroke_mut().filter(|s| s.visible && s.opacity > 0.0) {
            Some(s) => s.effects.extend_from_slice(stroke),
            None => {
                let mut s = vectorcraft_doc::StrokeLayer::new(vectorcraft_doc::color::Paint::None, 0.0);
                s.effects = stroke.to_vec();
                n.appearance.items.push(AppearanceItem::Stroke(s));
            }
        }
    }
}

/// Does effect `e` give an image's fill or stroke a shape to paint (a visible Convert to Shape or
/// Outline Object)?
fn gives_shape(e: &Effect) -> bool {
    e.visible
        && matches!(e.id.as_str(), "convertToShape.rectangle" | "convertToShape.roundedRectangle" | "convertToShape.ellipse" | "path.outlineObject")
}

/// Does `item`, a fill or stroke of an image, paint something: does it paint and carry an effect
/// that gives it a shape ([`gives_shape`])?
fn shapes_image(item: &AppearanceItem) -> bool {
    crate::paints(item) && gives_image_shape(item.effects())
}

/// Do `effects`, those of an image's fill or stroke, give that fill or stroke a shape to paint
/// ([`gives_shape`])?
pub fn gives_image_shape(effects: &[Effect]) -> bool {
    effects.iter().any(gives_shape)
}

/// The fills and strokes of `items` (an image's own) that paint something ([`shapes_image`]), each
/// without the effects before its first Convert to Shape or Outline Object. Convert to Shape sizes
/// its shape from the image's box, and Outline Object gives the outline of the image's opaque
/// area ([`opaque_area`]), or its frame when it has no transparency. The others paint nothing.
fn image_rows(items: &[AppearanceItem]) -> Vec<AppearanceItem> {
    items
        .iter()
        .filter(|i| shapes_image(i))
        .map(|i| {
            let mut i = i.clone();
            let fx = i.effects_mut();
            let first = fx.iter().position(gives_shape).unwrap_or(0);
            fx.drain(..first);
            i
        })
        .collect()
}

/// Does `item`, one of [`image_rows`], start from the outline of the image's opaque area (is its
/// first effect Outline Object rather than Convert to Shape)?
fn follows_opaque_area(item: &AppearanceItem) -> bool {
    item.effects().first().is_some_and(|e| e.id == "path.outlineObject")
}

/// Is `n` an image with a fill or stroke of its own that follows the outline of the image's opaque
/// area ([`image_rows`], [`opaque_area`])? The shape that fill or stroke paints then changes with
/// the image's pixels in the document while `n` stays the same.
pub fn follows_image_pixels(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Image(_))
        && n.appearance
            .items
            .iter()
            .any(|i| shapes_image(i) && i.effects().iter().find(|e| gives_shape(e)).is_some_and(|e| e.id == "path.outlineObject"))
}

/// The outline of a set of pixels' opaque area, in the pixel space they decode to, with that size
/// ([`traced_opaque_area`]). `None` for pixels that have no transparency or could not be traced.
type Traced = Option<(Arc<PathData>, (u32, u32))>;

/// One of [`Outlines`]' entries: the pixels, held weakly, and their outline once it is traced.
type Entry = (Weak<Vec<u8>>, Arc<OnceLock<Traced>>);

/// The outlines [`opaque_area`] has traced, by the address of the pixels each was traced from.
struct Outlines {
    /// Each outline with its pixels. An entry does not keep its pixels, and while it is kept no
    /// other pixels can take their address. Entries whose pixels are gone are dropped before a
    /// new outline is traced and every [`PRUNE_EVERY`] lookups.
    kept: BTreeMap<usize, Entry>,
    /// The lookups since the last pruning.
    lookups: usize,
}

static OUTLINES: Mutex<Outlines> = Mutex::new(Outlines { kept: BTreeMap::new(), lookups: 0 });

/// How many lookups [`OUTLINES`] takes between two prunings of its entries for pixels that are gone.
const PRUNE_EVERY: usize = 256;

#[cfg(test)]
thread_local! {
    /// The outlines [`traced_opaque_area`] has traced on this thread.
    pub(crate) static TRACES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Does [`OUTLINES`] keep an entry for `pixels`?
#[cfg(test)]
pub(crate) fn keeps_outline(pixels: &Weak<Vec<u8>>) -> bool {
    OUTLINES.lock().unwrap_or_else(PoisonError::into_inner).kept.values().any(|(w, _)| w.ptr_eq(pixels))
}

/// The outline of the opaque area of the image `im` in the document
/// ([`vectorcraft_trace::opaque_outline`]: its pixels that are at least half opaque, outlined with
/// fitted curves), its pixels taken from `im` itself or from the first of `docs` that holds them.
/// `None` when the image has no transparency, its pixels can't be read or are too large to read
/// ([`vectorcraft_trace::decode_alpha`]), or its outline would be too complex: Outline Object then
/// follows the image's frame. The outline is traced once for each set of pixels and kept while
/// those pixels are: a caller that asks while another thread traces the same pixels waits for that
/// trace.
pub(crate) fn opaque_area(docs: &[Option<&Document>], im: &ImageObject) -> Option<PathData> {
    let blob = im.pixels.as_deref().or_else(|| docs.iter().flatten().find_map(|d| d.images.get(&im.key)))?;
    let bytes = &blob.bytes;
    let cell = {
        let mut o = OUTLINES.lock().unwrap_or_else(PoisonError::into_inner);
        o.lookups += 1;
        if o.lookups >= PRUNE_EVERY {
            o.lookups = 0;
            o.kept.retain(|_, (w, _)| w.strong_count() > 0);
        }
        let key = Arc::as_ptr(bytes).addr();
        match o.kept.get(&key) {
            Some((_, cell)) => Arc::clone(cell),
            None => {
                o.kept.retain(|_, (w, _)| w.strong_count() > 0);
                let cell = Arc::new(OnceLock::new());
                o.kept.insert(key, (Arc::downgrade(bytes), Arc::clone(&cell)));
                cell
            }
        }
    };
    // Traced without the lock: another image's outline need not wait for this one.
    let (path, (w, h)) = cell.get_or_init(|| traced_opaque_area(bytes)).clone()?;
    let to_image = Affine::scale_non_uniform(f64::from(im.width) / f64::from(w.max(1)), f64::from(im.height) / f64::from(h.max(1)));
    Some(path.transformed(im.xf * to_image))
}

/// [`opaque_area`]'s outline of the encoded image `bytes`. An image whose format has no alpha is
/// not decoded.
fn traced_opaque_area(bytes: &[u8]) -> Traced {
    #[cfg(test)]
    TRACES.with(|t| t.set(t.get() + 1));
    let (w, h, alpha) = vectorcraft_trace::decode_alpha(bytes).ok()??;
    let outline = vectorcraft_trace::opaque_outline(w, h, alpha).ok()??;
    Some((Arc::new(outline), (w, h)))
}

/// The shapes that the image `n`'s own fills and strokes paint ([`image_rows`]), each with its fill
/// rule and the fill or stroke that paints it: the paths (or compound paths) that [`reshape`]
/// makes along its frame or, for those whose first effect is Outline Object, along the outline of
/// its opaque area ([`opaque_area`]), with each fill's or stroke's geometry effects applied as the
/// renderer applies them. For a symbol instance whose active fill or stroke has a visible geometry
/// effect ([`has_item_geometry`]), the shapes that the fills and strokes of the art it draws paint
/// ([`instance_paint`]). Empty for anything else. The swatches and patterns that live-effect
/// plug-ins painting their pieces name are looked up in `doc`, and the image's pixels in `doc` when
/// the image does not hold them.
pub fn image_paint(doc: Option<&Document>, n: &Node) -> Vec<(BezPath, FillRule, AppearanceItem)> {
    let mut out = vec![];
    if matches!(n.kind, NodeKind::SymbolInstance { .. }) {
        let mut budget = SYMBOL_ART_MAX_NODES;
        instance_paint(doc, n, 1, &mut budget, &mut out);
        return out;
    }
    if !matches!(n.kind, NodeKind::Image(_)) || !has_item_geometry(n) {
        return out;
    }
    let Some(art) = reshape(doc, n, None) else { return out };
    art.walk(&mut |c| push_painted(doc, c, &mut out));
    out
}

/// The shapes that the fills and strokes of path or compound path `c` paint, each with the
/// geometry effects of that fill or stroke applied ([`image_paint`]); nothing for other objects.
fn push_painted(doc: Option<&Document>, c: &Node, out: &mut Vec<(BezPath, FillRule, AppearanceItem)>) {
    let (NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. }) = &c.kind else { return };
    let Some(g) = c.stroke_path() else { return };
    let ctx = GeomContext { doc, ..GeomContext::of(c) };
    for item in c.appearance.items.iter().filter(|i| crate::paints(i)) {
        let fx = item.effects();
        let shape = if crate::has_geometry(fx) && !g.elements().is_empty() {
            crate::apply_geometry_bez(fx, &g, g.bounding_box(), &ctx.item(item))
        } else {
            g.clone()
        };
        out.push((shape, *rule, item.clone()));
    }
}

/// The shapes that symbol instance `n`, `depth` instances deep, paints with the geometry effects of
/// its active fill and stroke ([`has_item_geometry`]): the paths of the art [`reshape`] makes of it,
/// each with its own geometry effects and those of each fill and stroke applied
/// ([`crate::bake_type_appearance`], [`push_painted`]), the paths that the own fills and strokes of
/// the groups and layers in the art paint ([`crate::evaluate_container`]), the shapes that the own
/// fills and strokes of the images in the art paint ([`image_paint`]), and the instances in that
/// art that took those effects, in turn. Hidden objects, clipping paths, guides and the members of
/// clip groups add nothing, and neither does an instance more than
/// [`vectorcraft_doc::depth::MAX_DEPTH`] deep or whose symbol's art would take the objects measured
/// so far past `budget`.
fn instance_paint(doc: Option<&Document>, n: &Node, depth: usize, budget: &mut u64, out: &mut Vec<(BezPath, FillRule, AppearanceItem)>) {
    let NodeKind::SymbolInstance { symbol, .. } = &n.kind else { return };
    if depth >= vectorcraft_doc::depth::MAX_DEPTH || !has_item_geometry(n) {
        return;
    }
    let Some(sym) = doc.and_then(|d| d.symbols.iter().find(|s| s.name == *symbol)) else { return };
    let count = sym.art.count() as u64;
    if count > *budget {
        return;
    }
    *budget -= count;
    let Some(art) = reshape(doc, n, Some(&vectorcraft_doc::symbol_overrides::dynamic_art(&sym.art, n))) else { return };
    painted_in(doc, &art, depth, budget, out);
}

/// [`instance_paint`] of the objects in `n`, art of an instance `depth` instances deep.
fn painted_in(doc: Option<&Document>, n: &Node, depth: usize, budget: &mut u64, out: &mut Vec<(BezPath, FillRule, AppearanceItem)>) {
    if !n.visible {
        return;
    }
    match &n.kind {
        NodeKind::Path { clipping: false, guide: false, .. } | NodeKind::Compound { .. } => match crate::bake_type_appearance(doc, n) {
            // One path for each fill and stroke, its effects applied.
            Some(pieces) => painted_in(doc, &pieces, depth, budget, out),
            None => push_painted(doc, n, out),
        },
        NodeKind::Path { .. } => {}
        NodeKind::Image(_) => out.extend(image_paint(doc, n)),
        NodeKind::SymbolInstance { .. } => instance_paint(doc, n, depth + 1, budget, out),
        _ if n.clips() => {}
        _ => {
            let evaluated = crate::has_container_appearance(n).then(|| crate::evaluate_container(doc, n)).flatten();
            let g = evaluated.as_ref().unwrap_or(n);
            if g.clips() {
                return;
            }
            for c in g.children().into_iter().flatten() {
                painted_in(doc, c, depth, budget, out);
            }
        }
    }
}

/// The glyph outlines of type `t` in text space with its underline and strikethrough bars: one
/// path per run, and all of them as one.
pub(crate) fn glyph_outlines(t: &vectorcraft_doc::TextObject) -> (Vec<kurbo::BezPath>, kurbo::BezPath) {
    let (runs, all, ..) = laid_out(t);
    (runs, all)
}

/// [`glyph_outlines`], the bounds of `t`'s layout in text space, and its colour glyphs, which have
/// no outlines, placed in the document with their runs: bitmap glyphs (emoji) as images holding
/// their pixels, then COLR and SVG glyphs as their art, as the renderer paints them.
fn laid_out(t: &vectorcraft_doc::TextObject) -> (Vec<kurbo::BezPath>, kurbo::BezPath, Rect, Vec<(usize, Node)>) {
    let db = vectorcraft_text::FontDb::global();
    let layout = vectorcraft_text::layout(db, t);
    let mut runs = vec![kurbo::BezPath::new(); t.runs.len()];
    let mut all = kurbo::BezPath::new();
    // Ruby (furigana) glyphs are painted with their base text's run, as the renderer paints them.
    for g in layout.glyphs.iter().chain(&layout.ruby) {
        if let Some(r) = runs.get_mut(g.run) {
            r.extend(g.outline.iter());
        }
        all.extend(g.outline.iter());
    }
    // Underline and strikethrough bars, painted as their run's type is (#847).
    for (run, bar) in vectorcraft_text::decorations(&layout, db, t) {
        if let Some(r) = runs.get_mut(run) {
            r.extend(bar.iter());
        }
        all.extend(bar.iter());
    }
    let images = layout.glyphs.iter().filter_map(|g| Some((g.run, g.placed_image_node(t.xf)?)));
    let colours = images.chain(layout.glyphs.iter().filter_map(|g| Some((g.run, g.placed_colour(t.xf)?)))).collect();
    (runs, all, layout.bounds, colours)
}

/// Type as glyph outlines in document space: one path per run with the run's fill and stroke,
/// over its colour glyphs (images, art), and (when the object has its own fills/strokes) the whole
/// outline with the object's items below the characters under them and the others over them, as
/// the renderer paints them. Each run's path is placed by the type's transform, which also scales its stroke
/// weights and dashes and transforms its pattern tiles; its unplaced gradients are first fixed to
/// the layout's bounds in text space. The object's own unplaced gradients are fixed to the
/// bounding box of those bounds in the document. The group is marked as type's outlines
/// ([`vectorcraft_doc::NodeExtras::type_outlines`]).
pub fn outline_text(n: &Node) -> Option<Node> {
    let NodeKind::Text(t) = &n.kind else { return None };
    let (runs, all, bounds, colours) = laid_out(t);
    let path = |bp: &kurbo::BezPath| PathData::from_bezpath(bp).transformed(t.xf);
    let scaling = vectorcraft_doc::Scaling { strokes: true, patterns: true, ..Default::default() };
    let mut children: Vec<Arc<Node>> = vec![];
    for (i, (bp, run)) in runs.iter().zip(&t.runs).enumerate() {
        // Colour glyphs paint themselves, under their run's fill, as the renderer paints them.
        children.extend(colours.iter().filter(|(r, _)| *r == i).map(|(_, art)| Arc::new(with_id(art, n.id))));
        if bp.elements().is_empty() {
            continue;
        }
        let mut ap = run.style.appearance();
        ap.pin_gradients(bounds);
        let mut piece = Node::path(n.id, PathData::from_bezpath(bp), ap);
        piece.transform(t.xf, scaling);
        children.push(Arc::new(piece));
    }
    if !n.appearance.items.is_empty() && !all.elements().is_empty() {
        let (below, above) = n.appearance.split_contents();
        let tb = t.xf.transform_rect_bbox(bounds);
        let glyphs = |items: &[AppearanceItem]| {
            let mut ap = Appearance { items: items.to_vec(), ..Default::default() };
            ap.pin_gradients(tb);
            Arc::new(Node::path(n.id, path(&all), ap))
        };
        if !below.is_empty() {
            children.insert(0, glyphs(below));
        }
        if !above.is_empty() {
            children.push(glyphs(above));
        }
    }
    let mut g = Node::group(n.id, children);
    g.extras_mut().type_outlines = true;
    Some(g)
}

/// `art` with `id` for it and all its members, as the other pieces of outlined type have the type's
/// (Expand Appearance gives them ids of their own).
fn with_id(art: &Node, id: NodeId) -> Node {
    let mut art = art.clone();
    art.id = id;
    for c in art.children_mut().into_iter().flatten() {
        *c = Arc::new(with_id(c, id));
    }
    art
}

/// `n` as plain art that geometry effects can reshape, without its transparency and object
/// effects: type outlined ([`outline_text`]), live objects evaluated (type inside them outlined),
/// a symbol instance as `symbol_art` (the symbol's art, stained as the caller draws it) placed by
/// the instance's transform (through the symbol's 9-slice guides in `doc`, when it has them), as
/// the art its members draw when the instance has geometry effects of its own
/// ([`crate::group::members_as_drawn`]), its fills and strokes carrying the geometry and raster
/// effects of the instance's active fill and stroke ([`add_instance_row_effects`]).
/// `None` for other kinds (images, paths, containers), and for an instance whose art placed
/// through the guides could hold more than [`vectorcraft_doc::MAX_SYMBOL_NODES`] objects. A Repeat counts the Repeats in the art of
/// `doc`'s symbols that its instances show toward its instance budget.
pub fn outline_art(doc: Option<&Document>, n: &Node, symbol_art: Option<&Node>) -> Option<Node> {
    outline_art_with(doc, doc, n, symbol_art, &outline_text, &SymbolNodes::default(), false)
}

/// [`outline_art`] with live objects evaluated through `outline` ([`evaluated_with`]); with
/// `apart`, also a path or compound path whose Transform effect makes copies
/// ([`has_transform_copies`]), as the one piece the copies are made from. `symbols` is the document
/// the symbols of the art come from; `paint_doc` is the one the swatches and patterns of painted
/// pieces are looked up in.
fn outline_art_with(
    symbols: Option<&Document>,
    paint_doc: Option<&Document>,
    n: &Node,
    symbol_art: Option<&Node>,
    outline: OutlineHook,
    nodes: &SymbolNodes,
    apart: bool,
) -> Option<Node> {
    let mut art = match &n.kind {
        NodeKind::Text(_) => outline_text(n)?,
        NodeKind::SymbolInstance { symbol, xf } => {
            let a = match symbols {
                Some(d) => d.place_symbol_art(symbol, symbol_art?, *xf)?,
                None => {
                    let mut a = symbol_art?.clone();
                    a.transform(*xf, false);
                    a
                }
            };
            Node::group(n.id, vec![Arc::new(a)])
        }
        // The object as the one piece the copies of its Transform effect are made from.
        _ if has_transform_art(n) || (apart && has_transform_copies(n)) => one_piece(n),
        _ if vectorcraft_doc::live::is_live(n) => evaluated_with(symbols, n, outline, nodes),
        _ => return None,
    };
    outline_nested_text(paint_doc, &mut art, false);
    if matches!(n.kind, NodeKind::SymbolInstance { .. }) {
        // The instance's own geometry effects reshape what the symbol's members draw.
        if !geometry_effects(n).is_empty() {
            crate::group::members_as_drawn(paint_doc, &mut art);
        }
        add_instance_row_effects(symbols, n, &mut art);
    }
    art.opacity = 1.0;
    art.blend = Default::default();
    art.isolate = false;
    // One element: its pieces never knock each other out.
    art.knockout = vectorcraft_doc::Knockout::Off;
    art.extras_mut().mask = None;
    art.appearance.effects.clear();
    Some(art)
}

/// `n` with every live object in it evaluated, type inside them outlined ([`outline_text`]). The
/// Repeats among them count the Repeats in the art of `doc`'s symbols that their instances show
/// toward their instance budget ([`vectorcraft_doc::RepeatSpec::expand`]).
pub(crate) fn evaluated(doc: Option<&Document>, n: &Node) -> Node {
    evaluated_with(doc, n, &outline_text, &SymbolNodes::default())
}

/// Most objects of symbol art that one compound shape or live Pathfinder operation gives its
/// members in all ([`SymbolNodes`]): four for each segment the operation takes in
/// ([`crate::group::PATHFINDER_MAX_SEGMENTS`]).
pub const SYMBOL_ART_MAX_NODES: u64 = 4 * crate::group::PATHFINDER_MAX_SEGMENTS as u64;

/// The objects of symbol art given to one compound shape or live Pathfinder operation so far,
/// counted against [`SYMBOL_ART_MAX_NODES`]. Instances of symbols that hold several instances of
/// others multiply at each level, which the depth limit does not stop.
#[derive(Debug, Default)]
pub(crate) struct SymbolNodes(std::cell::Cell<u64>);

impl SymbolNodes {
    /// Count `art`'s objects: `false` once they would pass [`SYMBOL_ART_MAX_NODES`], and from then
    /// on for all art.
    pub(crate) fn charge(&self, art: &Node) -> bool {
        let total = self.0.get().saturating_add(art.count() as u64);
        let fits = total <= SYMBOL_ART_MAX_NODES;
        self.0.set(if fits { total } else { u64::MAX });
        fits
    }

    /// Has some symbol art been left out for passing [`SYMBOL_ART_MAX_NODES`]?
    pub(crate) fn spent(&self) -> bool {
        self.0.get() > SYMBOL_ART_MAX_NODES
    }
}

/// [`evaluated`] with type and symbol instances inside live objects given as their art by
/// `outline`, and every symbol instance left in the result (a blend's steps, an instance in a
/// plain group) replaced by its art from `outline`, with the instance's name, visibility and
/// transparency. The live objects and symbol instances inside that art are given as their art in
/// turn, except an instance of a symbol that holds an instance of itself, of a symbol nested
/// more than [`vectorcraft_doc::depth::MAX_DEPTH`] deep, or whose art would take `nodes` past
/// [`SYMBOL_ART_MAX_NODES`], which stays as it is. With [`outline_text`], symbol instances stay as
/// they are.
pub(crate) fn evaluated_with(doc: Option<&Document>, n: &Node, outline: OutlineHook, nodes: &SymbolNodes) -> Node {
    resolved(doc, n, outline, nodes, &[])
}

/// `art`, the art `outline` gave an instance of the symbol named `symbol`, with the live objects
/// and symbol instances inside it given as their art as [`evaluated_with`] does.
pub(crate) fn symbol_art_with(art: &Node, outline: OutlineHook, nodes: &SymbolNodes, symbol: &str) -> Node {
    resolved(None, art, outline, nodes, &[symbol.to_string()])
}

/// [`evaluated_with`] inside the art of the symbols named in `shown`, outermost first.
fn resolved(doc: Option<&Document>, n: &Node, outline: OutlineHook, nodes: &SymbolNodes, shown: &[String]) -> Node {
    let inner = |m: &Node| art_of(doc, m, outline, nodes, shown);
    let mut art = vectorcraft_doc::live::expand_deep_hooks(n, Hooks { outline: Some(&inner), symbols: doc, ..Default::default() });
    instances_as_art(doc, &mut art, outline, nodes, shown);
    art
}

/// `n`'s art from `outline`, inside the art of the symbols named in `shown`. For a symbol instance
/// it is the symbol's art with the live objects and instances inside it given as their art in turn
/// ([`resolved`]); `None` for an instance of a symbol in `shown` (one that holds an instance of
/// itself), for every instance once `shown` holds [`vectorcraft_doc::depth::MAX_DEPTH`] symbols,
/// and for an instance whose symbol's art `nodes` cannot take ([`SymbolNodes::charge`]).
fn art_of(doc: Option<&Document>, n: &Node, outline: OutlineHook, nodes: &SymbolNodes, shown: &[String]) -> Option<Node> {
    let NodeKind::SymbolInstance { symbol, .. } = &n.kind else { return outline(n) };
    if shown.len() >= vectorcraft_doc::depth::MAX_DEPTH || shown.contains(symbol) {
        return None;
    }
    let art = outline(n).filter(|art| nodes.charge(art))?;
    let mut inside = shown.to_vec();
    inside.push(symbol.clone());
    Some(resolved(doc, &art, outline, nodes, &inside))
}

/// Replace every symbol instance under `n` for which [`art_of`] gives art by that art. `shown`
/// names the symbols whose art `n` is in, outermost first.
fn instances_as_art(doc: Option<&Document>, n: &mut Node, outline: OutlineHook, nodes: &SymbolNodes, shown: &[String]) {
    let Some(ch) = n.children_mut() else { return };
    for c in ch.iter_mut() {
        if matches!(c.kind, NodeKind::SymbolInstance { .. }) {
            if let Some(mut o) = art_of(doc, c, outline, nodes, shown) {
                o.name = c.name.clone();
                (o.visible, o.opacity, o.blend, o.isolate) = (c.visible, c.opacity, c.blend, c.isolate);
                *c = Arc::new(o);
            }
        } else if c.children().is_some() {
            instances_as_art(doc, Arc::make_mut(c), outline, nodes, shown);
        }
    }
}

/// Replace type anywhere inside `n` by its outlines, as art that keeps the type's look
/// ([`as_art`]). Type in a clip group's clipping path (all of `n` when `clipping` is set) becomes
/// its plain outlines ([`outline_text`]) with its visibility, which is what
/// [`crate::clip_outline`] clips by. The swatches and patterns that live-effect plug-ins painting
/// their pieces name are looked up in `doc`.
pub(crate) fn outline_nested_text(doc: Option<&Document>, n: &mut Node, clipping: bool) {
    let clips = n.clips();
    let Some(ch) = n.children_mut() else { return };
    for (i, c) in ch.iter_mut().enumerate() {
        let clipping = clipping || (clips && i == 0);
        let art = match &c.kind {
            NodeKind::Text(_) if clipping => outline_text(c).map(|o| Node { visible: c.visible, ..o }),
            NodeKind::Text(_) => as_art(doc, c, None),
            _ => None,
        };
        if let Some(o) = art {
            *c = Arc::new(o);
        } else if c.children().is_some() {
            outline_nested_text(doc, Arc::make_mut(c), clipping);
        }
    }
}

/// `n` (see [`needs_outline`]) with its object-level geometry effects and its perspective applied:
/// its [`outline_art`] reshaped as one piece, or for an image the image clipped by its reshaped
/// frame. Type's own fills and strokes keep their effects on its whole outline. An image's own
/// fills and strokes that paint a shape ([`image_rows`]) become paths below and above it, along
/// its frame or, from Outline Object, along the outline of its opaque area ([`opaque_area`]),
/// reshaped with it, and keep their effects from their first Convert to Shape or Outline Object
/// on. The result is a group with `n`'s id, name, visibility, transparency, opacity mask
/// and remaining (raster) effects; `None` when `n` has no visible geometry effect, no perspective
/// and is not type or an image with a fill or stroke of its own that [`has_item_geometry`] counts
/// or a symbol instance whose active fill or stroke has a raster effect
/// ([`has_instance_row_raster`]), or has no art (a symbol instance without `symbol_art`). A path
/// with [`has_transform_art`] is always reshaped. The fills and strokes in the art whose own
/// Transform effects reach their appearance become objects of their own ([`transform_items`]).
pub fn reshape(doc: Option<&Document>, n: &Node, symbol_art: Option<&Node>) -> Option<Node> {
    let own = !geometry_effects(n).is_empty() || n.projection().is_some() || has_item_geometry(n) || has_instance_row_raster(n);
    if !has_transform_art(n) && (!own || !needs_outline(n)) {
        return None;
    }
    as_art(doc, n, symbol_art)
}

/// `n` (see [`needs_outline`], or a path with [`has_transform_art`]) as plain art for Expand
/// Appearance: [`reshape`]d when it has
/// geometry effects, else type with fills or strokes of its own outlined, an image with fills or
/// strokes of its own that paint a shape, or a symbol instance's art reshaped by the geometry
/// effects of its active fill or stroke ([`has_item_geometry`]), in the same form. `None` when it
/// has none of these.
pub fn expand_outlined(doc: Option<&Document>, n: &Node, symbol_art: Option<&Node>) -> Option<Node> {
    // A path whose Transform effect copies it, or scales its strokes or moves its patterns: each
    // copy an object of its own with the path's fills and strokes, the copies of each Transform
    // effect in a group of their own (copies of copies nest), as the reference app expands them
    // (#5612).
    if has_transform_art(n) || has_transform_copies(n) {
        return as_art_opts(doc, doc, n, symbol_art, &outline_text, &SymbolNodes::default(), true);
    }
    let own_paint = match n.kind {
        NodeKind::Text(_) => !n.appearance.items.is_empty(),
        _ => has_item_geometry(n),
    };
    if !needs_outline(n) || (geometry_effects(n).is_empty() && !own_paint) {
        return None;
    }
    as_art(doc, n, symbol_art)
}

/// [`reshape`] without its checks (no geometry effects: just the art).
pub(crate) fn as_art(doc: Option<&Document>, n: &Node, symbol_art: Option<&Node>) -> Option<Node> {
    as_art_with(doc, doc, n, symbol_art, &outline_text, &SymbolNodes::default())
}

/// [`as_art`] with live objects evaluated through `outline` ([`evaluated_with`]). `symbols` is the
/// document the symbols of the art come from; `paint_doc` is the one the swatches and patterns of
/// painted pieces are looked up in.
pub(crate) fn as_art_with(
    symbols: Option<&Document>,
    paint_doc: Option<&Document>,
    n: &Node,
    symbol_art: Option<&Node>,
    outline: OutlineHook,
    nodes: &SymbolNodes,
) -> Option<Node> {
    as_art_opts(symbols, paint_doc, n, symbol_art, outline, nodes, false)
}

/// [`as_art_with`]; with `apart`, a path's or compound path's Transform effects make each copy an
/// object of its own ([`copy_leaf`]) rather than one path of every copy.
fn as_art_opts(
    symbols: Option<&Document>,
    paint_doc: Option<&Document>,
    n: &Node,
    symbol_art: Option<&Node>,
    outline: OutlineHook,
    nodes: &SymbolNodes,
    apart: bool,
) -> Option<Node> {
    let fx = geometry_effects(n);
    let mut art = match &n.kind {
        NodeKind::Image(im) => {
            let frame = shapes::rectangle(Rect::new(0.0, 0.0, im.width as f64, im.height as f64)).transformed(im.xf);
            let mut image = Node::new(n.id, n.kind.clone());
            // Its pixels draw as the image's do: colorized by a grayscale image's fill color,
            // smoothed or not, and with its color mask's masked pixels.
            {
                let mut e = image.extras_mut();
                e.colorize = n.extras().colorize.clone();
                e.pixelated = n.extras().pixelated;
                e.color_mask = n.extras().color_mask.clone();
            }
            let clipped = || {
                let mut clip = Node::path(n.id, frame.clone(), Appearance::default());
                if let NodeKind::Path { clipping, .. } = &mut clip.kind {
                    *clipping = true;
                }
                Node::new(n.id, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(image.clone())], clip: true })
            };
            // The fills and strokes that paint a shape, as paths below and above the image: along
            // the frame, or for those whose first effect is Outline Object along the outline of
            // the image's opaque area (traced only when one of them needs it). A run of either
            // kind is one path; a fill or stroke whose outline is empty (an image with no opaque
            // pixel) paints nothing.
            let (below, above) = n.appearance.split_contents();
            let (below, above) = (image_rows(below), image_rows(above));
            if below.is_empty() && above.is_empty() {
                clipped()
            } else {
                let opaque = std::cell::OnceCell::new();
                let rows = |items: Vec<AppearanceItem>| {
                    let mut out = vec![];
                    let mut items = items.into_iter().peekable();
                    while let Some(first) = items.next() {
                        let follows = follows_opaque_area(&first);
                        let mut run = vec![first];
                        while let Some(next) = items.next_if(|i| follows_opaque_area(i) == follows) {
                            run.push(next);
                        }
                        let path = if follows { opaque.get_or_init(|| opaque_area(&[symbols, paint_doc], im)).as_ref() } else { None };
                        let path = path.unwrap_or(&frame);
                        if !path.is_empty() {
                            out.push(Arc::new(Node::path(n.id, path.clone(), Appearance { items: run, ..Default::default() })));
                        }
                    }
                    out
                };
                // Without geometry effects, the image is not clipped.
                let image = if fx.is_empty() { image.clone() } else { clipped() };
                Node::group(n.id, rows(below).into_iter().chain([Arc::new(image)]).chain(rows(above)).collect())
            }
        }
        _ => outline_art_with(symbols, paint_doc, n, symbol_art, outline, nodes, apart)?,
    };
    reshape_leaves_apart(&mut art, &fx, &n.appearance.effects, Budget::EFFECTS, apart, paint_doc);
    if apart {
        hoist_copies(&mut art);
    }
    transform_items(&mut art, paint_doc);
    // Type and symbols in perspective: the art projected onto their plane.
    if let Some(h) = n.projection() {
        project_leaves(&mut art, &h, paint_doc);
    }
    // The art is a group with `n`'s id (for an image without fills or strokes that paint, a clip
    // group).
    Some(dressed(art, n))
}

/// `art`, the art made of `n`, with `n`'s name, visibility, transparency, opacity mask and
/// remaining (raster) effects.
fn dressed(mut art: Node, n: &Node) -> Node {
    art.name = n.name.clone();
    art.visible = n.visible;
    art.opacity = n.opacity;
    art.blend = n.blend;
    art.isolate = n.isolate;
    // One element: its pieces never knock each other out.
    art.knockout = vectorcraft_doc::Knockout::Off;
    art.knockout_shape = n.knockout_shape;
    art.extras_mut().mask = n.extras().mask.clone();
    art.appearance.effects = crate::raster::without_geometry(&n.appearance.effects);
    art
}

/// A group with `n`'s id holding `n` without its transparency, opacity mask and effects: the one
/// piece that `n`'s own geometry effects reshape.
fn one_piece(n: &Node) -> Node {
    let mut one = n.clone();
    one.opacity = 1.0;
    one.blend = Default::default();
    one.isolate = false;
    one.extras_mut().mask = None;
    one.appearance.effects.clear();
    Node::group(n.id, vec![Arc::new(one)])
}

/// Path or compound path `n` with its own geometry effects applied, as the art it draws: with
/// geometry effects in a visible fill or stroke of its own, a group of one path for each fill and
/// stroke, its effects and that fill's or stroke's applied ([`crate::bake_type_appearance`]);
/// otherwise a group with `n`'s id holding the path reshaped by its effects ([`reshape_leaves`]).
/// The art keeps `n`'s id, name, visibility, transparency, opacity mask and raster effects. `None`
/// for a guide, for other kinds of objects, for a path with a linked opacity mask or a 3D effect,
/// and when `n` has no visible geometry effect of its own or in a visible fill or stroke.
pub(crate) fn own_effects_art(doc: Option<&Document>, n: &Node) -> Option<Node> {
    if !matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. })
        || n.extras().mask.as_ref().is_some_and(|m| m.linked)
        || crate::has_solid(n)
        || crate::has_revolve(n)
    {
        return None;
    }
    if n.appearance.items.iter().any(|i| crate::paints(i) && crate::has_geometry(i.effects())) {
        // One element: its pieces never knock each other out.
        return crate::bake_type_appearance(doc, n).map(|art| Node { knockout: vectorcraft_doc::Knockout::Off, ..art });
    }
    let fx = geometry_effects(n);
    if fx.is_empty() {
        return None;
    }
    let mut art = one_piece(n);
    let carry = reshape_leaves_carrying(&mut art, &fx, &n.appearance.effects, doc);
    transform_items(&mut art, doc);
    let mut art = dressed(art, n);
    // The raster effects before a Transform effect the path did not take stay as the path does.
    if carry.skipped() {
        let mut ran = n.appearance.effects.clone();
        carry.hide_skipped(&mut ran);
        art.appearance.effects = crate::raster::without_geometry(&ran);
    }
    Some(art)
}

/// Map every path under `art` (and its gradients), and every path in the art of the linked opacity
/// masks of the objects under it, through the projective map `h`, curves split finely enough to
/// follow it. The type and live objects in the art of those masks are outlined and evaluated
/// first ([`outlined_members`], with the symbols of `doc`).
fn project_leaves(art: &mut Node, h: &vectorcraft_geom::Homography, doc: Option<&Document>) {
    project_paths(art, h);
    each_linked_mask_mut(art, false, 0, &mut |m| {
        *m = outlined_mask_art(doc, m);
        project_paths(m, h);
    });
}

/// `art`, the art of an opacity mask, with type and live objects in it outlined and evaluated
/// ([`outlined_members`]), and as art that keeps its look ([`as_art`]) when it is type itself.
fn outlined_mask_art(doc: Option<&Document>, art: &Node) -> Node {
    match art.kind {
        NodeKind::Text(_) => as_art(doc, art, None).unwrap_or_else(|| art.clone()),
        _ => outlined_members(doc, art),
    }
}

/// The paths of [`project_leaves`] outside the art of masks.
fn project_paths(art: &mut Node, h: &vectorcraft_geom::Homography) {
    let f = |p: vectorcraft_geom::Point| h.apply(p).unwrap_or(p);
    let piece = art.geometric_bounds().map_or(1.0, |b| (b.width().max(b.height()) / 64.0).max(0.25));
    let project = |path: &mut PathData| *path = vectorcraft_doc::live::map_nonlinear(path, piece, f);
    for_each_leaf(art, &mut |leaf| {
        leaf.pin_gradients();
        leaf.appearance.warp_gradients(&|p| h.affine_at(p).unwrap_or(vectorcraft_geom::Affine::IDENTITY));
        match &mut leaf.kind {
            NodeKind::Path { path, live, .. } => {
                project(path);
                *live = None;
            }
            NodeKind::Compound { children, .. } => {
                for c in children.iter_mut() {
                    if let NodeKind::Path { path, live, .. } = &mut Arc::make_mut(c).kind {
                        project(path);
                        *live = None;
                    }
                }
            }
            _ => {}
        }
    });
}

/// The visible geometry effects of `n`'s own effect list, in order.
pub(crate) fn geometry_effects(n: &Node) -> Vec<&Effect> {
    n.appearance.effects.iter().filter(|e| e.visible && is_geometry(&e.id)).collect()
}

/// Apply `fx` to every path under `art` as one piece: each effect takes the whole art's bounds as
/// its reference box and the art's seed (a random Transform moves the pieces together). Images and
/// symbol instances follow a Transform effect the same way ([`transform_placed`]), and images
/// follow a Warp effect as a raster mesh warp ([`cut_warped_images`]); other geometry effects leave them
/// as they are. The paths keep to one [`Budget::EFFECTS`] together (or to what they cost, when that is more): a path
/// whose effect would pass what the others leave of it keeps its shape. The swatches and patterns
/// that live-effect plug-ins painting their pieces name are looked up in `doc`, and a deferred one
/// is listed by its index in `owner`, the effect list `fx` was taken from ([`reshape_leaves_apart`]).
pub(crate) fn reshape_leaves(art: &mut Node, fx: &[&Effect], owner: &[Effect], doc: Option<&Document>) {
    reshape_leaves_apart(art, fx, owner, Budget::EFFECTS, false, doc);
}

/// [`reshape_leaves`], and how gradients that paint the art as a whole (a group's own fills and
/// strokes) follow the effects ([`crate::Carry`]).
pub(crate) fn reshape_leaves_carrying(art: &mut Node, fx: &[&Effect], owner: &[Effect], doc: Option<&Document>) -> crate::Carry {
    reshape_leaves_apart(art, fx, owner, Budget::EFFECTS, false, doc)
}

/// [`reshape_leaves_carrying`] whose paths keep to `budget` together, or to what they cost.
#[cfg(test)]
pub(crate) fn reshape_leaves_within(art: &mut Node, fx: &[&Effect], budget: Budget, doc: Option<&Document>) -> crate::Carry {
    reshape_leaves_apart(art, fx, &[], budget, false, doc)
}

/// `art`, the group of the one piece a path's copies are made from ([`outline_art_with`]), holding
/// the group of its copies ([`copy_leaf`]) as its only member: the copies become its members, so
/// the expanded path is one group of its copies rather than a group in a group.
fn hoist_copies(art: &mut Node) {
    let id = art.id;
    let Some(children) = art.children_mut() else { return };
    let [only] = children.as_slice() else { return };
    if only.id != id || !matches!(only.kind, NodeKind::Group { clip: false, .. }) {
        return;
    }
    let copies = only.children().cloned().unwrap_or_default();
    if !copies.is_empty() {
        *children = copies;
    }
}

/// [`reshape_leaves_carrying`] whose paths keep to `budget` together, or to what they cost; with
/// `apart`, a Transform effect's copies of each path are objects
/// of their own ([`copy_leaf`]) whatever paints them, as Expand Appearance makes them. A live-effect
/// plug-in that paints what it returns makes each path a group of its pieces ([`paint_leaf`]),
/// whose swatches and patterns must be in `doc` (without one, pieces that name one are refused).
/// Returns how gradients that paint the art as a whole follow the effects, and the Transform
/// effects that nothing in the art took past the budget ([`crate::Carry::skipped`]). A deferred live-effect
/// plug-in is listed by its index in `owner`, the effect list `fx` was taken from, or in `fx` when
/// `owner` does not hold it. The art of the linked opacity masks of the objects in `art` (not of
/// `art` itself) takes each effect as the paths it masks do ([`Step::reshape_masks`]), with the
/// type and live objects in it outlined and evaluated first ([`outlined_members`]). When a mask
/// stays as it was, or a Transform effect makes fewer copies than it asks for, art with linked
/// masks keeps its shape under that effect.
fn reshape_leaves_apart(art: &mut Node, fx: &[&Effect], owner: &[Effect], budget: Budget, apart: bool, doc: Option<&Document>) -> crate::Carry {
    let mut whole = crate::Carry::default();
    if fx.is_empty() {
        return whole;
    }
    let masked = has_linked_mask(art, false);
    if masked {
        each_linked_mask_mut(art, false, 0, &mut |m| *m = outlined_mask_art(doc, m));
    }
    let mut masks_safe = true;
    each_linked_mask(art, false, 0, &mut |_, m| masks_safe &= leaves_safe(m));
    let art_stroke = art.appearance.stroke().is_some_and(|s| !s.paint.is_none() && s.width > 0.0);
    let style_safe = leaves_safe(art) && masks_safe && !art_stroke;
    let listed = fx;
    let opt_fx = crate::geometry::optimize_geometry_effects(fx, style_safe);
    let fx = opt_fx.as_slice();
    let seed = art.id.0;
    let mut used = art_cost(art);
    each_linked_mask(art, false, 0, &mut |_, m| used = used.plus(art_cost(m)));
    let max = budget.at_least(used);
    // Read before the effects run: a path's own copies don't make it art of several objects. The
    // same holds for the art of each mask, listed by the object it masks.
    let copy_whole = copies_whole(art);
    let mut wholes = HashMap::new();
    each_linked_mask(art, false, 0, &mut |id, m| {
        wholes.entry(id).or_insert_with(|| copies_whole(m));
    });
    let mut warped = WarpedImages::new();
    // The raster effects of what the art holds follow each Transform effect that moves it once,
    // whatever its options, unless one of the effects makes copies or moves at random
    // ([`crate::raster::contents_step`], [`follow_art`]).
    let held = listed.iter().any(|e| crate::raster::holds_back(e));
    for e in fx {
        let Some(bounds) = warped.bounds(art) else { break };
        let params = merged_params(&e.id, &e.params);
        // Where a live-effect plug-in sits, which it is listed by when it is deferred.
        let effect = owner.iter().position(|x| std::ptr::eq(x, *e)).or_else(|| listed.iter().position(|x| std::ptr::eq(*x, *e)));
        let step = Step { e, params: &params, bounds, seed, effect, apart, doc, wholes: &wholes, held };
        // Art with linked masks keeps its shape under an effect that a mask in it could not take
        // as the art it masks did, so no mask is left apart from its art.
        let before = masked.then(|| (art.clone(), used));
        let ran = step.run(art, max, &mut used, copy_whole, Some(&mut warped), false, 0);
        if (ran.masks || ran.short)
            && let Some((was, cost)) = before
        {
            *art = was;
            used = cost;
            continue;
        }
        // A Transform effect that moves the art once and that nothing in it took (past the budget)
        // is left out ([`crate::Carry::hide_skipped`]), listed by its index in `owner`.
        if ran.kept
            && !ran.took
            && e.id == "distort.transform"
            && !crate::raster::holds_back(e)
            && let Some(i) = owner.iter().position(|x| std::ptr::eq(x, *e))
        {
            whole.skip(i);
        }
        if let Some(carry) = ran.whole {
            whole.push(carry);
        }
    }
    cut_warped_images(art, &warped);
    whole
}

/// One geometry effect of an object run over its art ([`reshape_leaves_apart`]): the effect, its
/// merged params, the reference box and seed of the object's art, where a deferred live-effect
/// plug-in is listed, whether each Transform copy becomes an object, the document that the
/// swatches and patterns of painted pieces are looked up in, [`copies_whole`] of the art of
/// each linked mask before the first effect, by the id of the object it masks, and whether one of
/// the object's effects holds the raster effects in its art back from its Transform effects
/// ([`follow_art`]).
struct Step<'a> {
    e: &'a Effect,
    params: &'a serde_json::Value,
    bounds: Rect,
    seed: u64,
    effect: Option<usize>,
    apart: bool,
    doc: Option<&'a Document>,
    wholes: &'a HashMap<NodeId, bool>,
    held: bool,
}

/// What [`Step::run`] did to a piece of art.
struct Ran {
    /// A path in the art kept its shape (past the budget).
    kept: bool,
    /// A path, image or symbol instance in the art took the effect.
    took: bool,
    /// The linked opacity mask of an object in the art stayed as it was while the paths it masks
    /// took the effect, or could have ([`Step::reshape_masks`]).
    masks: bool,
    /// A Transform effect made fewer copies of a path, image or symbol instance than it asks for
    /// (past the budget or out of reach), where a linked mask could take a different number.
    short: bool,
    /// How what paints the art as a whole follows the effect, when it does.
    whole: Option<crate::Carry>,
}

impl Ran {
    /// Nothing in the art kept its shape or made fewer copies, and every linked mask in it moved.
    fn complete(&self) -> bool {
        !self.kept && !self.masks && !self.short
    }
}

impl Step<'_> {
    /// Run the effect over `art`, whose paths and those of the linked opacity masks in it keep to
    /// `max` together with what they cost so far (`used`). `copy_whole` is [`copies_whole`] of the
    /// art, or of a mask's art, before the first effect. `warped` lists the images Warp effects
    /// bend in the art ([`WarpedImages`]); a mask's art has none. With `own`, `art` is the art of a
    /// mask, and its own linked mask takes the effect too. `depth` counts the masks `art` is in. A
    /// live-effect plug-in that paints its pieces leaves a mask's art, and art that holds objects
    /// with linked masks, as they are and reports the masks as stayed: a mask can't take the
    /// pieces as the art it masks does. So does a Warp effect that bends an image with a linked
    /// mask.
    #[allow(clippy::too_many_arguments)]
    fn run(
        &self,
        art: &mut Node,
        max: Budget,
        used: &mut Budget,
        copy_whole: bool,
        mut warped: Option<&mut WarpedImages>,
        own: bool,
        depth: usize,
    ) -> Ran {
        let (e, params, bounds, seed) = (self.e, self.params, self.bounds, self.seed);
        let stayed = Ran { kept: false, took: false, masks: true, short: false, whole: None };
        if let Some(plugin) = painted_plugin(e) {
            if own || has_linked_mask(art, false) {
                return stayed;
            }
            let at = self.effect.map(|effect| vectorcraft_plugins::deferred::Stage { object: seed, effect, item: None });
            for_each_leaf(art, &mut |leaf| {
                let Some((path, _)) = leaf_geometry(leaf) else { return };
                let before = Budget::of(&path);
                let room = max.minus(used.minus(before));
                if let Some(made) = paint_leaf(leaf, plugin, params, bounds, room, self.doc, at) {
                    *used = used.minus(before).plus(made);
                }
            });
            return Ran { kept: false, took: true, masks: false, short: false, whole: None };
        }
        // A Warp effect that bends images ([`mark_warped_images`]) bends an image's pixels, not the
        // art of its linked mask.
        let warp = e.id.strip_prefix("warp.").and_then(crate::warp::WarpStyle::from_id).filter(|_| crate::warp::bends(&e.id, params));
        if warp.is_some() && has_masked_image(art) {
            return stayed;
        }
        // Placed gradients follow a Transform effect, Free Distort and the Warp effects
        // ([`crate::Carry`]): under Free Distort and Warp (`boxes`) each path's move and scale from
        // its bounds before the effect to its bounds after it. Under those and the other effects,
        // the placed gradients of type's outlines in `art` (`outline_text`) move and scale with
        // the outlines' bounds.
        let follows = crate::Carry::follows(&e.id, params, bounds, seed);
        let boxes = follows && crate::Carry::by_bounds(&e.id);
        let mut outlines = vec![];
        if !follows || boxes {
            type_outline_bounds(art, &mut outlines);
        }
        let mut leaves = vec![];
        if boxes {
            leaf_bounds(art, &mut leaves);
        }
        let mut groups = vec![];
        if follows {
            group_marks(art, &mut vec![], &HashSet::new(), None, &mut groups);
            // The images earlier Warps bend are measured by their pieces only when a group's
            // gradient needs the bounds.
            if !groups.is_empty()
                && let Some(images) = warped.as_deref().and_then(|w| w.when_cut(art))
            {
                groups.clear();
                group_marks(art, &mut vec![], &HashSet::new(), Some(images), &mut groups);
            }
        }
        // The ids of the paths that kept their shape (past the budget), and their places in the
        // order [`for_each_leaf`] visits the paths; and the ids of the paths whose linked masks
        // moved with them as they were transformed as objects.
        let mut kept_ids = HashSet::new();
        let mut kept_leaves = HashSet::new();
        let mut leaf_at = 0usize;
        let mut moved_ids = HashSet::new();
        let transform = (e.id == "distort.transform").then(|| {
            let (m, copies) = crate::distort::transform_map(bounds, params, seed);
            let options = crate::distort::TransformOptions::of(params);
            (m, copies, options, crate::distort::transform_affine(bounds, params, seed))
        });
        // What the effect does to the raster effects in the art, when they follow it.
        let raster_map = if self.held { None } else { crate::raster::contents_step(std::slice::from_ref(e)) };
        // In a mask's art, images and symbol instances take only a Transform effect
        // ([`transform_placed`]): a mask's art that holds one can't take another effect as the art
        // it masks does.
        if own && transform.is_none() && (is_placed(art) || placed_count(art) > 0) {
            return Ran { kept: false, took: false, masks: true, short: false, whole: None };
        }
        // Art of several objects is copied whole, so the copies don't interleave member by member.
        // The copies of its members carry their linked masks, moved with them.
        if let Some((m, copies, options, _)) = transform
            && copies > 0
            && copy_whole
        {
            if !own {
                *used = copy_art(art, m, copies, copy_scaling(options), max);
                return Ran { kept: false, took: true, masks: false, short: false, whole: None };
            }
            // A mask's art: its copies keep to what the rest of the art leaves of `max`, and the
            // linked mask of the art itself takes them too. That mask's art stays in `used`, as
            // [`copy_art`] counts only the members.
            let before = copy_cost(art);
            let rest = used.minus(before).plus(art.extras().mask.as_deref().map_or(Budget { anchors: 0, lines: 0 }, |k| copy_cost(&k.art)));
            let made = copy_art(art, m, copies, copy_scaling(options), max.minus(rest));
            *used = rest.plus(made);
            let short = art.children().is_some_and(|c| c.iter().filter(|c| !c.is_guide()).count() <= copies);
            let none = HashSet::new();
            let masks = self.reshape_own_mask(art, &Skip { moved: &none, kept: &none }, max, used, depth);
            return Ran { kept: false, took: true, masks, short, whole: None };
        }
        // A shortfall in the copies matters for the objects a linked mask above them covers, which
        // the mask would take a different number of copies than, and anywhere in a mask's art.
        let watch = transform.is_some_and(|(_, copies, options, _)| copies > 0 && options.objects) && (own || has_linked_mask(art, false));
        let mut covered = HashSet::new();
        if watch && !own {
            masked_ids(art, false, &mut covered);
        }
        let counts = |id: NodeId| watch && (own || covered.contains(&id));
        let mut short = false;
        let mut took = false;
        for_each_leaf(art, &mut |leaf| {
            let at = leaf_at;
            leaf_at += 1;
            let Some((path, rule)) = leaf_geometry(leaf) else { return };
            // What the other paths leave of the budget.
            let before = Budget::of(&path);
            let room = max.minus(used.minus(before));
            if let Some((m, copies, options, map)) = transform {
                // Transform Patterns and Scale Strokes & Effects change each copy's fills and
                // strokes; a gradient paints each copy by itself, not the one piece the copies
                // make together; and Expand Appearance (`apart`) makes each copy an object.
                // Fills and strokes that stack paint each copy below the one before (#5920, #6316).
                // Each copy of a path with a linked mask carries the mask, moved with it.
                let separate = copies > 0
                    && (self.apart
                        || leaf.appearance.has_unplaced_gradient()
                        || leaf.appearance.has_placed_gradient()
                        || paints_copies_apart(&leaf.appearance, rule)
                        || leaf.extras().mask.as_ref().is_some_and(|m| m.linked));
                // The copies of a path replace it by a group of them, the path included.
                let copied = |leaf: &Node| leaf.children().map_or(0, Vec::len);
                if options.reach_appearance() {
                    let (made, moved) = transform_leaf(leaf, &path, map, copies, options, room, raster_map.is_some(), None);
                    *used = used.minus(before).plus(made);
                    took |= moved;
                    if moved {
                        // With Transform Objects off the path stays, and its mask takes the
                        // effect's stroke and pattern changes as its own art.
                        if options.objects {
                            moved_ids.insert(leaf.id);
                        }
                        short |= counts(leaf.id) && copied(leaf) <= copies;
                    } else {
                        kept_ids.insert(leaf.id);
                    }
                    return;
                }
                if separate {
                    let scaling = vectorcraft_doc::Scaling { keep_fit_when_flat: true, ..Default::default() };
                    let made = copy_leaf(leaf, &path, m, copies, scaling, room, None);
                    *used = used.minus(before).plus(made);
                    took = true;
                    moved_ids.insert(leaf.id);
                    short |= counts(leaf.id) && copied(leaf) <= copies;
                    return;
                }
            }
            let stage = self.effect.map(|effect| vectorcraft_plugins::deferred::Stage { object: seed, effect, item: None });
            let Some(out) = apply_one(&e.id, params, &path, bounds, &GeomContext { seed, stage, ..GeomContext::of(leaf) }, room) else {
                kept_ids.insert(leaf.id);
                kept_leaves.insert(at);
                return;
            };
            took = true;
            // A Transform effect's copies of a path are subpaths of it, each with the path's anchors.
            if counts(leaf.id)
                && let Some((_, copies, _, _)) = transform
            {
                short |= out.anchor_count() < path.anchor_count().saturating_mul(copies.saturating_add(1));
            }
            *used = used.minus(before).plus(Budget::of(&out));
            if follows && !boxes {
                // A gradient fitted to the path fits the path's own bounds.
                let carry = crate::Carry::of(&e.id, params, bounds, path.bounds(), None, seed);
                if carry.moves(&leaf.appearance) {
                    carry.apply(&mut leaf.appearance);
                }
            }
            set_geometry(leaf, out, rule);
        });
        if boxes {
            rebase_leaves(art, &mut leaves.into_iter());
        }
        // Warp bends images too: each one's pixels follow the warp's map over the same box. The
        // images are cut into pieces once, after the last effect ([`cut_warped_images`]). An image
        // whose clipping frame kept its shape (past the budget) keeps its pixels as they are.
        if let Some(style) = warp
            && let Some(warped) = warped.as_deref_mut()
        {
            let f: WarpMap = Rc::from(crate::warp::warp_map(bounds, style, params));
            warped.reserve(art);
            mark_warped_images(art, &f, &kept_leaves, false, &mut 0, warped);
        }
        if let Some((m, copies, options, _)) = transform
            && options.objects
        {
            // A mask's art that is one image or symbol instance is placed as the only member of a
            // group of it.
            let root = is_placed(art);
            if root {
                let id = art.id;
                let one = std::mem::replace(art, Node::group(id, vec![]));
                *art = Node::group(id, vec![Arc::new(one)]);
            }
            let placed = if watch { masked_placed(art, own) } else { 0 };
            // Images and symbol instances take the effect whatever the budget.
            took = took || placed_count(art) > 0;
            // Scale Strokes & Effects leaves the raster effects that follow the effect to
            // [`follow_art`].
            let mut scaling = copy_scaling(options);
            if raster_map.is_some() {
                scaling.effects = options.strokes.then_some(scale_unfollowed as fn(&mut Effect, f64));
            }
            transform_placed(art, m, copies, scaling);
            short |= watch && masked_placed(art, own) < placed.saturating_mul(copies.saturating_add(1));
            if root && let Some([one]) = art.children().map(Vec::as_slice) {
                *art = (**one).clone();
            }
        }
        // A mask's art takes them as a group inside the art: its own raster effects follow too.
        if let Some(a) = raster_map {
            follow_art(art, a, &kept_ids, Some(&moved_ids), own.then_some(Affine::IDENTITY), depth);
        }
        if outlines.iter().any(Option::is_some) {
            rebase_type_outlines(art, &mut outlines.into_iter());
        }
        // Under Transform copies, a mask takes the copies as the paths it masks do; the copies of a
        // path with a mask of its own each carry it ([`copy_leaf`]).
        let masks = self.reshape_masks(art, own, &Skip { moved: &moved_ids, kept: &kept_ids }, max, used, depth);
        // What paints the art as a whole, and the own fills and strokes of the groups inside it,
        // follow their bounds under Free Distort and Warp (they stay when no path changed them).
        // Each stays when a path under it that its bounds measure kept its shape (past the budget).
        let mut after = vec![];
        let images = if groups.is_empty() { None } else { warped.as_deref().and_then(|w| w.when_cut(art)) };
        let held = (!groups.is_empty() || !kept_ids.is_empty()) && group_marks(art, &mut vec![], &kept_ids, images, &mut after).1;
        if !groups.is_empty() {
            carry_groups(art, groups, after, &|from, to| crate::Carry::of(&e.id, params, bounds, from, to, seed));
        }
        let drawn = warped.as_deref().map_or_else(|| art.geometric_bounds(), |w| w.bounds(art));
        let whole = (follows && !held).then(|| crate::Carry::of(&e.id, params, bounds, Some(bounds), drawn, seed));
        Ran { kept: !kept_ids.is_empty(), took, masks, short, whole }
    }

    /// Run the effect over the art of the linked opacity mask of `n` (with `own`) and of each path,
    /// compound path, group and layer under it, with the art's reference box, as the paths the mask
    /// covers take it: the mask moves and bends with what it masks. The mask of an object
    /// transformed as an object moved with it already and is left as it is. A mask stays as it was
    /// past [`vectorcraft_doc::depth::MAX_DEPTH`] masks inside masks, and for the reasons
    /// [`Step::reshape_own_mask`] gives. Returns whether a mask stayed for one of those reasons.
    fn reshape_masks(&self, n: &mut Node, own: bool, skip: &Skip, max: Budget, used: &mut Budget, depth: usize) -> bool {
        if depth >= vectorcraft_doc::depth::MAX_DEPTH || !has_linked_mask(n, own) {
            return false;
        }
        let mut stayed = own && self.reshape_own_mask(n, skip, max, used, depth);
        if let NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } = &mut n.kind {
            for c in children.iter_mut() {
                if has_linked_mask(c, true) {
                    stayed |= self.reshape_masks(Arc::make_mut(c), true, skip, max, used, depth);
                }
            }
        }
        stayed
    }

    /// Run the effect over the art of the linked opacity mask of `n` itself, when `n` is a path,
    /// compound path, group or layer that `skip` does not list as moved. The mask stays as it was
    /// when a path it masks kept its shape, and when its own art could not take the effect whole: a
    /// path in it would keep its shape, its copies would stop short, or a mask inside it would stay
    /// (no mask is left half moved). Returns whether the mask stayed for one of those reasons.
    fn reshape_own_mask(&self, n: &mut Node, skip: &Skip, max: Budget, used: &mut Budget, depth: usize) -> bool {
        if depth >= vectorcraft_doc::depth::MAX_DEPTH || !reshapes_mask_of(n) || skip.moved.contains(&n.id) {
            return false;
        }
        let stays = holds_any(n, skip.kept);
        let id = n.id;
        let Some(m) = n.extras_if_any().and_then(|x| x.mask.as_mut()).filter(|m| m.linked) else { return false };
        let before = *used;
        let mut art = (*m.art).clone();
        let whole = self.wholes.get(&id).copied().unwrap_or_else(|| copies_whole(&art));
        let ran = (!stays).then(|| self.run(&mut art, max, used, whole, None, true, depth + 1));
        match ran {
            Some(ran) if ran.complete() => {
                // What paints the mask's art as a whole follows the effect as the art does.
                if let Some(carry) = ran.whole
                    && carry.moves(&art.appearance)
                {
                    carry.apply(&mut art.appearance);
                }
                m.art = Arc::new(art);
                false
            }
            _ => {
                *used = before;
                true
            }
        }
    }
}

/// The objects whose linked masks [`Step::reshape_masks`] leaves as they are: those transformed
/// as objects (`moved`), and those holding a path that kept its shape (`kept`).
struct Skip<'a> {
    moved: &'a HashSet<NodeId>,
    kept: &'a HashSet<NodeId>,
}

/// Is `n` an object whose linked opacity mask a geometry effect on the art it is in reshapes with
/// it: a path, compound path, group or layer? An image or symbol instance moves its mask only
/// with the map a Transform effect places it by ([`transform_placed`]).
fn reshapes_mask_of(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } | NodeKind::Group { .. } | NodeKind::Layer { .. })
}

/// Does `n` (with `own`) or a path, compound path, group or layer under it carry a linked opacity
/// mask that [`Step::reshape_masks`] reshapes?
fn has_linked_mask(n: &Node, own: bool) -> bool {
    let mine = own && reshapes_mask_of(n) && n.extras().mask.as_ref().is_some_and(|m| m.linked);
    mine || match &n.kind {
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => children.iter().any(|c| has_linked_mask(c, true)),
        _ => false,
    }
}

/// Run `f` on the id of `n` (with `own`) and of each path, compound path, group and layer under it
/// that carries a linked opacity mask, with the art of that mask, and on the masks inside that
/// art, as [`Step::reshape_masks`] reaches them: `depth` masks deep already, up to
/// [`vectorcraft_doc::depth::MAX_DEPTH`].
fn each_linked_mask(n: &Node, own: bool, depth: usize, f: &mut dyn FnMut(NodeId, &Node)) {
    if depth >= vectorcraft_doc::depth::MAX_DEPTH {
        return;
    }
    if own
        && reshapes_mask_of(n)
        && let Some(m) = n.extras().mask.as_deref().filter(|m| m.linked)
    {
        f(n.id, &m.art);
        each_linked_mask(&m.art, true, depth + 1, f);
    }
    if let NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } = &n.kind {
        for c in children {
            each_linked_mask(c, true, depth, f);
        }
    }
}

/// [`each_linked_mask`] that changes the art of each mask: `f` runs on a mask's art before the
/// masks inside it are reached.
fn each_linked_mask_mut(n: &mut Node, own: bool, depth: usize, f: &mut dyn FnMut(&mut Node)) {
    if depth >= vectorcraft_doc::depth::MAX_DEPTH || !has_linked_mask(n, own) {
        return;
    }
    if own
        && reshapes_mask_of(n)
        && let Some(m) = n.extras_if_any().and_then(|x| x.mask.as_mut()).filter(|m| m.linked)
    {
        let art = Arc::make_mut(&mut m.art);
        f(art);
        each_linked_mask_mut(art, true, depth + 1, f);
    }
    if let NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } = &mut n.kind {
        for c in children.iter_mut() {
            if has_linked_mask(c, true) {
                each_linked_mask_mut(Arc::make_mut(c), true, depth, f);
            }
        }
    }
}

/// Does [`Step::reshape_masks`] reshape the linked opacity mask of `n` itself?
fn has_own_linked_mask(n: &Node) -> bool {
    reshapes_mask_of(n) && n.extras().mask.as_ref().is_some_and(|m| m.linked)
}

/// Add to `out` the ids of the objects under `n` that the linked opacity mask of an object above
/// them covers (every one of them with `under`).
fn masked_ids(n: &Node, under: bool, out: &mut HashSet<NodeId>) {
    for c in n.children().into_iter().flatten() {
        if under {
            out.insert(c.id);
        }
        masked_ids(c, under || has_own_linked_mask(c), out);
    }
}

/// How many images and symbol instances under `n` the linked opacity mask of an object above them
/// covers (every one of them with `under`).
fn masked_placed(n: &Node, under: bool) -> usize {
    n.children().into_iter().flatten().map(|c| usize::from(under && is_placed(c)) + masked_placed(c, under || has_own_linked_mask(c))).sum()
}

/// Is `n`, or anything under it, one of `ids`?
fn holds_any(n: &Node, ids: &HashSet<NodeId>) -> bool {
    if ids.is_empty() {
        return false;
    }
    let mut any = false;
    n.walk(&mut |c| any |= ids.contains(&c.id));
    any
}

/// Do the paths under `n` that [`for_each_leaf`] visits all fill by the non-zero rule without a
/// stroke that paints? Effects then reshape their outlines alike in any order that the
/// optimization of [`crate::geometry::optimize_geometry_effects`] takes.
fn leaves_safe(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => {
            let has_stroke = n.appearance.stroke().is_some_and(|s| !s.paint.is_none() && s.width > 0.0);
            leaf_geometry(n).is_none_or(|(_, rule)| rule == FillRule::NonZero && !has_stroke)
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => children.iter().all(|c| leaves_safe(c)),
        _ => true,
    }
}

/// Is there an image with a linked opacity mask among the members of `n`, or among its groups'
/// and layers' members, which [`mark_warped_images`] walks?
fn has_masked_image(n: &Node) -> bool {
    let (NodeKind::Group { children, .. } | NodeKind::Layer { children, .. }) = &n.kind else { return false };
    children.iter().any(|c| match c.kind {
        NodeKind::Image(_) => c.extras().mask.as_ref().is_some_and(|m| m.linked),
        _ => has_masked_image(c),
    })
}

/// Largest factor a Transform effect's Scale Strokes & Effects scales one object's strokes by
/// (its copies stop before one would take more), so stroke weights stay finite where the geometry
/// doesn't grow with them (a horizontal line stretched vertically).
const MAX_STROKE_SCALE: f64 = 1.0e6;

/// How a Transform effect with `options` scales what it maps: strokes, effects and patterns as
/// its options say.
fn copy_scaling(options: crate::distort::TransformOptions) -> vectorcraft_doc::Scaling {
    vectorcraft_doc::Scaling {
        strokes: options.strokes,
        effects: options.strokes.then_some(crate::scale_effect as fn(&mut Effect, f64)),
        patterns: options.patterns,
        // A map that flattens the path leaves its fitted gradients fitted, as `fixes_fit` does.
        keep_fit_when_flat: true,
        ..Default::default()
    }
}

/// [`crate::scale_effect`] for the effects that do not follow a Transform effect as raster effects
/// do ([`crate::raster::follows`]): Scale Strokes & Effects scales them, and [`follow_art`] moves
/// the others.
fn scale_unfollowed(e: &mut Effect, s: f64) {
    if !crate::raster::follows(&e.id) {
        crate::scale_effect(e, s);
    }
}

/// Make the raster effects under `art` follow `a`, the linear part of a Transform effect that moved
/// it ([`crate::raster::follow_contents`]): those of every object in it (its own and its fills' and
/// strokes'), the art's own effects excepted (`up` is `None` for the art itself). A path that kept
/// its shape (in `kept`), and a group with such a path under it, keep their own and their fills'
/// and strokes'. `up` is what the Transform effects of the groups between an object and the art do
/// to the object's effects. A Transform effect of an object's own that makes copies or moves at
/// random leaves the effects it holds back as they are, so those follow `a` and the groups' maps
/// alone. The linked opacity mask of an object that took the effect as an object (in `moved`, and
/// every image and symbol instance) moved with it, so the raster effects in the mask's art follow
/// `a` as those of a mask that takes the effect as art do ([`Step::reshape_masks`]); `moved` is
/// `None` inside such a mask's art, where every linked mask moved. The walk reaches the groups and
/// layers that [`for_each_leaf`] made the art's own, so it copies nothing. Returns whether a path in
/// `kept` is under `n`.
fn follow_art(n: &mut Node, a: Affine, kept: &HashSet<NodeId>, moved: Option<&HashSet<NodeId>>, up: Option<Affine>, depth: usize) -> bool {
    if n.is_guide() {
        return false;
    }
    if moved.is_none_or(|moved| moved.contains(&n.id)) || is_placed(n) {
        follow_moved_mask(n, a, depth);
    }
    // What the object's own Transform effects then do to its fills' and strokes' effects and its
    // members' (nothing when one of them holds those back); the art's own effects are the ones
    // being applied.
    let below = up.map_or(Affine::IDENTITY, |up| crate::raster::contents_step(&n.appearance.effects).map_or(up, |s| up * s));
    let keeps = match &mut n.kind {
        NodeKind::Path { .. } | NodeKind::Compound { .. } => kept.contains(&n.id),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
            let mut keeps = false;
            for c in children.iter_mut() {
                keeps |= follow_art(Arc::make_mut(c), a, kept, moved, Some(below), depth);
            }
            keeps
        }
        _ => false,
    };
    if !keeps {
        if let Some(up) = up
            && contents_raster(&n.appearance.effects)
        {
            crate::raster::follow_contents(&mut n.appearance.effects, a, up);
        }
        if n.appearance.items.iter().any(|i| contents_raster(i.effects())) {
            for item in &mut n.appearance.items {
                crate::raster::follow_contents(item.effects_mut(), a, below);
            }
        }
    }
    keeps
}

/// Make the raster effects in the art of the linked opacity mask of `n`, which moved with `n`,
/// follow `a` ([`follow_art`]), with those of the linked masks inside that art, which moved too. A
/// mask past [`vectorcraft_doc::depth::MAX_DEPTH`] masks inside masks stays as it is.
fn follow_moved_mask(n: &mut Node, a: Affine, depth: usize) {
    if depth >= vectorcraft_doc::depth::MAX_DEPTH {
        return;
    }
    let Some(m) = n.extras_if_any().and_then(|x| x.mask.as_mut()).filter(|m| m.linked) else { return };
    follow_art(Arc::make_mut(&mut m.art), a, &HashSet::new(), None, Some(Affine::IDENTITY), depth + 1);
}

/// Does any of `effects` follow a Transform effect as a raster effect ([`crate::raster::follows`])?
fn contents_raster(effects: &[Effect]) -> bool {
    effects.iter().any(|e| crate::raster::follows(&e.id))
}

/// A Transform effect whose Transform Patterns or Scale Strokes & Effects is on (`options`), with
/// map `m` and `copies`, applied to `leaf` (a path or compound path with geometry `path`) as Object
/// › Transform with those options transforms an object (#885): the strokes' weights and dashes
/// and the distances of the effects on its fills and strokes scale by `m`'s mean scale, and its
/// pattern paints transform by `m`. With copies, the original keeps them and copy `k` takes them
/// `k` times, each copy an object of its own ([`copy_leaf`], within `budget`). With Transform
/// Objects off the geometry stays and the options apply once. With `follows`, Scale Strokes &
/// Effects leaves the raster effects that follow the Transform effect ([`follow_art`]) to the
/// caller. The effects `kept` names keep their values on the leaf and on each copy
/// ([`keep_listed`]), whether Scale Strokes & Effects scaled them or left them to the caller.
/// Returns what the leaf costs after, and `false` when the path kept its shape because the moved
/// path would pass `budget`.
#[allow(clippy::too_many_arguments)]
fn transform_leaf(
    leaf: &mut Node,
    path: &PathData,
    m: Affine,
    copies: usize,
    options: crate::distort::TransformOptions,
    budget: Budget,
    follows: bool,
    kept: Kept,
) -> (Budget, bool) {
    let mut scaling = copy_scaling(options);
    if follows && copies == 0 {
        scaling.effects = options.strokes.then_some(scale_unfollowed as fn(&mut Effect, f64));
    }
    if !options.objects {
        if options.strokes
            && let Some(k) = vectorcraft_doc::Scaling::factor(m).filter(|k| k.is_finite() && *k > 0.0)
        {
            leaf.appearance.scale_strokes(k);
            leaf.appearance.scale_effects(k, crate::scale_effect);
        }
        if options.patterns {
            vectorcraft_doc::pattern::transform_pattern_paints(leaf, m);
        }
        keep_listed(leaf, kept);
        return (Budget::of(path), true);
    }
    if copies == 0 {
        // A path scaled up is drawn with more lines; past the budget it keeps its shape.
        let moved = path.transformed(m);
        if !budget.fits(Budget::of(&moved)) {
            return (Budget::of(path), false);
        }
        leaf.transform(m, scaling);
        keep_listed(leaf, kept);
        return (Budget::of(&moved), true);
    }
    (copy_leaf(leaf, path, m, copies, scaling, budget, kept), true)
}

/// Does a Transform effect's copies of `art` copy it whole ([`copy_art`])? They do when `art` is a
/// group or layer of several objects, at least one of which paints, with no type left in it and no
/// fill or stroke of its own that paints. Type that stays type is copied in its place by the baking
/// that keeps it, a clipping group's first member is its clip, and the art's own fills and strokes
/// paint the shape of everything under it, copies included. A member that [`makes_art`] is copied
/// in its place as well: each copy would make that art again, past what [`copy_art`] counts.
fn copies_whole(art: &Node) -> bool {
    fn parts(n: &Node, count: &mut usize, painted: &mut bool, text: &mut bool) {
        match &n.kind {
            NodeKind::Text(_) => *text = true,
            NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => {
                *count += 1;
                *painted |= n.appearance.items.iter().any(crate::paints);
            }
            NodeKind::Image(_) | NodeKind::SymbolInstance { .. } => {
                *count += 1;
                *painted = true;
            }
            NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
                *painted |= n.appearance.items.iter().any(crate::paints);
                children.iter().for_each(|c| parts(c, count, painted, text));
            }
            _ => {}
        }
    }
    if art.clips() || !matches!(art.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) || art.appearance.items.iter().any(crate::paints) {
        return false;
    }
    let (mut count, mut painted, mut text) = (0, false, false);
    parts(art, &mut count, &mut painted, &mut text);
    count > 1 && painted && !text && !art.children().into_iter().flatten().any(|c| makes_art(c))
}

/// Does `n` or anything visible under it, the art of its opacity masks included, carry a visible
/// effect that makes art when it is drawn (its own or that of a fill or stroke that paints), or is
/// any of them a compound shape? Raster effects, color adjustments and unknown effects make none.
fn makes_art(n: &Node) -> bool {
    let makes = |e: &Effect| {
        e.visible
            && (is_geometry(&e.id)
                || crate::is_pathfinder(&e.id)
                || crate::is_three_d(&e.id)
                || e.id == crate::CROP_MARKS
                || e.id == crate::addons::halftone::HALFTONE)
    };
    n.visible
        && (n.is_compound_shape()
            || n.appearance.effects.iter().any(makes)
            || n.appearance.items.iter().any(|i| crate::paints(i) && i.effects().iter().any(makes))
            || n.extras().mask.as_deref().is_some_and(|m| makes_art(&m.art))
            || n.children().into_iter().flatten().any(|c| makes_art(c)))
}

/// What the paths under `n` cost together, those of its opacity masks included.
fn copy_cost(n: &Node) -> Budget {
    let mut cost = Budget { anchors: 0, lines: 0 };
    vectorcraft_doc::swatches::walk_with_masks(n, &mut |c| {
        if let NodeKind::Path { guide: false, path, .. } = &c.kind {
            cost = cost.plus(Budget::of(path));
        }
    });
    cost
}

/// What the geometry effects of `leaf` (with geometry `path`) and of its fills and strokes add to
/// what `path` costs when it is drawn.
fn effects_cost(leaf: &Node, path: &PathData) -> Budget {
    let none = Budget { anchors: 0, lines: 0 };
    let items: Vec<&AppearanceItem> = leaf.appearance.items.iter().filter(|i| crate::paints(i) && crate::has_geometry(i.effects())).collect();
    if items.is_empty() && !crate::has_geometry(&leaf.appearance.effects) {
        return none;
    }
    let Some(bounds) = path.bounds() else { return none };
    let ctx = GeomContext { seed: leaf.id.0, ..GeomContext::of(leaf) };
    let own = crate::apply_geometry_with(&leaf.appearance.effects, path, bounds, &ctx);
    let item_cost = |i: &&AppearanceItem| {
        let b = own.bounds()?;
        Some(Budget::of(&crate::apply_geometry_with(i.effects(), &own, b, &ctx.item(i))))
    };
    // The object's geometry is drawn as it is only by what has no geometry effect of its own.
    let plain = items.is_empty() || leaf.appearance.items.iter().any(|i| crate::paints(i) && !crate::has_geometry(i.effects()));
    let base = if plain { Budget::of(&own) } else { none };
    base.plus(items.iter().filter_map(item_cost).fold(none, Budget::plus)).minus(Budget::of(path))
}

/// Remove the guides anywhere under `n`.
fn without_guides(n: &mut Node) {
    let Some(children) = n.children_mut() else { return };
    children.retain(|c| !c.is_guide());
    children.iter_mut().for_each(|c| without_guides(Arc::make_mut(c)));
}

/// Replace the members of `art` by groups of them: the original on top and `copies` more below it,
/// each mapped once more by `m` than the last (with `scaling`), as a Transform effect copies a
/// group, without its guides. The copies stop where [`copy_leaf`]'s would, before they pass `budget` or
/// [`crate::distort::MAX_COPY_OBJECTS`] nodes in all, the art of opacity masks counted. Returns what
/// the art costs after.
fn copy_art(art: &mut Node, m: Affine, copies: usize, scaling: vectorcraft_doc::Scaling, budget: Budget) -> Budget {
    let Some(mut members) = art.children().cloned() else { return art_cost(art) };
    // A layer's ruler guides are not copied: they stay above the copies, once.
    let guides = members.split_off(Node::guide_run_start(&members));
    let original = Node::group(art.id, members);
    let mut made = copy_cost(&original);
    let mut drawn = original.clone();
    without_guides(&mut drawn);
    let mut nodes = original.count_with_masks();
    let one = drawn.count_with_masks();
    let mut placements: Vec<Arc<Node>> = vec![];
    let mut xf = Affine::IDENTITY;
    for _ in 0..copies {
        xf *= m;
        if scaling.strokes && vectorcraft_doc::Scaling::factor(xf).is_some_and(|k| !(k.is_finite() && k <= MAX_STROKE_SCALE)) {
            break;
        }
        nodes = nodes.saturating_add(one);
        if nodes > crate::distort::MAX_COPY_OBJECTS {
            break;
        }
        let mut copy = drawn.clone();
        copy.transform(xf, scaling);
        let cost = copy_cost(&copy);
        if !budget.fits(made.plus(cost)) || !copy.geometric_bounds().is_some_and(crate::distort::within_reach) {
            break;
        }
        placements.push(Arc::new(copy));
        made = made.plus(cost);
    }
    // The last copy is the lowest.
    placements.reverse();
    placements.push(Arc::new(original));
    placements.extend(guides);
    if let Some(children) = art.children_mut() {
        *children = placements;
    }
    made
}

/// Replace `leaf` (a path or compound path with geometry `path`) by a group of it and `copies`
/// more below it, each mapped once more by `m` than the last (with `scaling`), as a Transform effect
/// copies an object. Each copy is an object of its own, so its gradients fit it (a placed gradient
/// moves with it) rather than spanning every copy, and its strokes and patterns can differ from the
/// others'. The copies stop where [`crate::distort::transform_within`]'s would, before they pass
/// `budget` with what each copy's own geometry effects make ([`effects_cost`]), and before one's
/// strokes would scale past [`MAX_STROKE_SCALE`]. The effects `kept` names keep their values on
/// each copy, before its cost is counted ([`keep_listed`]). Returns what the copies cost, the
/// original included.
fn copy_leaf(leaf: &mut Node, path: &PathData, m: Affine, copies: usize, scaling: vectorcraft_doc::Scaling, budget: Budget, kept: Kept) -> Budget {
    let mut made = Budget::of(path).plus(effects_cost(leaf, path));
    let mut placements: Vec<Arc<Node>> = vec![Arc::new(leaf.clone())];
    let mut xf = Affine::IDENTITY;
    for _ in 0..copies {
        xf *= m;
        // A copy scaled up is drawn with more lines.
        let moved = path.transformed(xf);
        if scaling.strokes && vectorcraft_doc::Scaling::factor(xf).is_some_and(|k| !(k.is_finite() && k <= MAX_STROKE_SCALE)) {
            break;
        }
        let mut copy = leaf.clone();
        copy.transform(xf, scaling);
        keep_listed(&mut copy, kept);
        // Each copy's own geometry effects make a different amount, as the copy and its effects scale.
        let mut cost = Budget::of(&moved).plus(effects_cost(&copy, &moved));
        // Each copy carries the art of its linked masks, moved with it.
        each_linked_mask(&copy, true, 0, &mut |_, m| cost = cost.plus(art_cost(m)));
        if !budget.fits(made.plus(cost)) || !copy.geometric_bounds().is_some_and(crate::distort::within_reach) {
            break;
        }
        placements.push(Arc::new(copy));
        made = made.plus(cost);
    }
    // The copies become nodes of their own under a group with the object's id and name, the object
    // on top.
    placements.reverse();
    let mut g = Node::group(leaf.id, placements);
    g.name = leaf.name.clone();
    *leaf = g;
    made
}

/// Live-effect plug-in `plugin` (one that paints what it returns) with `params` run on `leaf`, a
/// path or compound path, as one of its object's effects with reference box `bounds`: `leaf`
/// becomes a group with its id and name of one object for each piece returned, in order, each with
/// the piece's geometry and the leaf's appearance with the piece's fills and strokes in place of
/// its own ([`vectorcraft_plugins::painted::appearance`]). The swatches and patterns the pieces
/// name must be in `doc`, and without one the pieces may name none. Returns what the pieces cost
/// ([`piece_cost`]); `None` (the leaf stays as it is) when the plug-in fails, the pieces are
/// refused, or they would pass `budget`. A deferred run is listed by `at`.
fn paint_leaf(
    leaf: &mut Node,
    plugin: &str,
    params: &serde_json::Value,
    bounds: Rect,
    budget: Budget,
    doc: Option<&Document>,
    at: Option<vectorcraft_plugins::deferred::Stage>,
) -> Option<Budget> {
    let (path, rule) = leaf_geometry(leaf)?;
    let paints = vectorcraft_plugins::effect::Paints { appearance: Some(&leaf.appearance), item: None, at };
    let pieces = vectorcraft_plugins::effect::apply_painted(plugin, params, &path, bounds, rule, paints, doc)?;
    let made = pieces.iter().map(|p| piece_cost(&p.path)).fold(Budget { anchors: 0, lines: 0 }, Budget::plus);
    if !budget.fits(made) {
        return None;
    }
    let children = pieces
        .iter()
        .map(|p| {
            let mut one = leaf.clone();
            one.name = None;
            one.appearance = vectorcraft_plugins::painted::appearance(&leaf.appearance, p).into();
            set_geometry(&mut one, p.path.clone(), p.rule);
            Arc::new(one)
        })
        .collect();
    let mut g = Node::group(leaf.id, children);
    g.name = leaf.name.clone();
    *leaf = g;
    Some(made)
}

/// What a painted piece with geometry `path` costs: its anchors and lines, plus a share of the
/// effects' budget for the object it becomes, so that a budget of [`Budget::EFFECTS`] holds at most
/// [`MAX_OBJECTS`](vectorcraft_plugins::objects::MAX_OBJECTS) pieces however small their paths.
fn piece_cost(path: &PathData) -> Budget {
    Budget::of(path).plus(piece_share())
}

/// The share of the effects' budget each painted piece costs besides its path ([`piece_cost`]).
fn piece_share() -> Budget {
    let max = vectorcraft_plugins::objects::MAX_OBJECTS;
    Budget { anchors: Budget::EFFECTS.anchors / max, lines: Budget::EFFECTS.lines / max }
}

/// Live-effect plug-in `plugin` (one that paints what it returns) with `params`, a later effect of
/// `item`, run on `leaf`, a path of [`transform_piece`] painted by `item` (its first fill or stroke),
/// with reference box `bounds`: `leaf` becomes a group of one path for each piece returned, in
/// order. The plug-in is given `appearance`'s fills and strokes and told that it is on item `index`
/// of them. A piece that gives fills (on a fill) or strokes (on a stroke) is painted by copies of
/// `item` with those paints ([`vectorcraft_plugins::painted::items`]), under the leaf's other fills
/// and strokes; one that gives none keeps the leaf's. Returns
/// what the paths cost ([`piece_cost`]); `None` (the leaf stays as it is) when the plug-in fails,
/// the pieces are refused (a swatch or pattern not in `doc`, or any without one), or they would pass
/// `budget`. A deferred run is listed by `at`.
#[allow(clippy::too_many_arguments)]
fn paint_piece(
    leaf: &mut Node,
    item: &AppearanceItem,
    (appearance, index): (&Appearance, usize),
    (plugin, params): (&str, &serde_json::Value),
    bounds: Rect,
    budget: Budget,
    doc: Option<&Document>,
    at: Option<vectorcraft_plugins::deferred::Stage>,
) -> Option<Budget> {
    let (path, rule) = leaf_geometry(leaf)?;
    let paints = vectorcraft_plugins::effect::Paints { appearance: Some(appearance), item: Some(index), at };
    let pieces = vectorcraft_plugins::effect::apply_painted(plugin, params, &path, bounds, rule, paints, doc)?;
    let made = pieces.iter().map(|p| piece_cost(&p.path)).fold(Budget { anchors: 0, lines: 0 }, Budget::plus);
    if !budget.fits(made) {
        return None;
    }
    let mut children = vec![];
    for p in pieces.iter() {
        let gives = match item {
            AppearanceItem::Fill(_) => p.fills.is_some(),
            AppearanceItem::Stroke(_) => p.strokes.is_some(),
        };
        let items = if gives {
            let mut items = vectorcraft_plugins::painted::items(item, p);
            items.extend_from_slice(leaf.appearance.items.get(1..).unwrap_or_default());
            items
        } else {
            leaf.appearance.items.clone()
        };
        if items.is_empty() {
            continue;
        }
        let mut one = leaf.clone();
        one.appearance.items = items;
        set_geometry(&mut one, p.path.clone(), p.rule);
        children.push(Arc::new(one));
    }
    *leaf = Node::group(leaf.id, children);
    Some(made)
}

/// Every path and compound path under `art` (other than clipping paths) with a fill or stroke whose
/// own Transform effect has Transform Patterns or Scale Strokes & Effects on
/// ([`item_transform_options`]), or whose own live-effect plug-in paints what it returns
/// ([`painted_plugin`]), made into a group of objects of its own ([`transform_item_leaf`]). The
/// new paths keep to one [`Budget::EFFECTS`] together (or to what `art` costs, when that is more).
/// The swatches and patterns painted pieces name must be in `doc`; without one they may name none.
fn transform_items(art: &mut Node, doc: Option<&Document>) {
    let mut used = art_cost(art);
    let max = Budget::EFFECTS.at_least(used);
    for_each_leaf(art, &mut |leaf| transform_item_leaf(leaf, max, &mut used, doc));
}

/// Is `e` an effect that makes a fill's or stroke's piece of a path an object of its own: a
/// Transform effect that [`reaches_appearance`], or a live-effect plug-in that paints what it
/// returns ([`painted_plugin`])?
fn makes_piece(e: &Effect) -> bool {
    reaches_appearance(e) || painted_plugin(e).is_some()
}

/// Does `item` paint and carry an effect that [`makes_piece`]?
fn item_makes_piece(item: &AppearanceItem) -> bool {
    crate::paints(item) && item.effects().iter().any(makes_piece)
}

/// `leaf`, a path or compound path with a fill or stroke whose own Transform effect has Transform
/// Patterns or Scale Strokes & Effects on, as a group with its id, name, transparency and object
/// effects. In paint order, each such fill or stroke is a path of its own: the leaf's geometry after
/// the geometry effects of that fill or stroke before its first such Transform effect, transformed
/// as [`transform_leaf`] transforms an object with that effect's map, copies and options, so the
/// stroke's weight and dashes, the distances of the effects before the Transform effect and its
/// pattern follow the map, and the raster effects before it that follow it take their followed
/// values ([`follow_piece`]). The effects after it keep their values and apply to what it made
/// ([`keep_listed`]), and with Transform Objects off so do the effects before it. The reference box
/// is that geometry's bounds and the seed is the leaf's id, as the renderer draws a fill's or
/// stroke's own effects. The geometry effects after the Transform effect then
/// apply to the transformed path and its copies as one piece ([`transform_piece`]), and its other
/// effects stay on each of them. The other fills and strokes stay together on paths of the leaf's
/// geometry between them, with their own effects. `max` is the budget of the whole art and `used`
/// what it has spent.
///
/// A fill or stroke whose first such effect is a live-effect plug-in that paints what it returns
/// ([`painted_plugin`]) becomes, in place of a transformed path, one path for each piece the
/// plug-in returns, painted by copies of the fill or stroke with the piece's paints
/// ([`vectorcraft_plugins::painted::items`]); the plug-in is given the geometry before it and told
/// which fill or stroke it is on. When it fails (or names a swatch or pattern not in `doc`, or any
/// without a document), the
/// fill or stroke paints that geometry as it is. The geometry effects after it then apply to all
/// the pieces as one ([`transform_piece`]).
fn transform_item_leaf(leaf: &mut Node, max: Budget, used: &mut Budget, doc: Option<&Document>) {
    if matches!(leaf.kind, NodeKind::Path { clipping: true, .. }) || !leaf.appearance.items.iter().any(item_makes_piece) {
        return;
    }
    let Some((path, rule)) = leaf_geometry(leaf) else { return };
    let original = leaf.clone();
    let ctx = GeomContext::of(&original);
    let bounds = path.to_bezpath().bounding_box();
    // A path of the leaf's geometry painted by `items` alone, without the leaf's transparency and
    // object effects (the group keeps them).
    let piece = |items: Vec<AppearanceItem>| {
        let mut p = original.clone();
        p.appearance.items = items;
        p.appearance.effects.clear();
        p.appearance.contents_index = None;
        p.opacity = 1.0;
        p.blend = Default::default();
        p.isolate = false;
        p.knockout = Default::default();
        p.extras_mut().mask = None;
        p
    };
    // The pieces replace the leaf's own geometry (counted in `used`): each plain piece costs that
    // geometry again, and each transformed fill or stroke what its paths cost.
    let cost = Budget::of(&path);
    *used = used.minus(cost);
    let mut pieces: Vec<Arc<Node>> = vec![];
    let mut plain: Vec<AppearanceItem> = vec![];
    // Which items become a transformed piece of their own; every other run of items is one plain piece.
    let transformed: Vec<bool> = original.appearance.items.iter().map(item_makes_piece).collect();
    // The number of pieces after item `i` in paint order: each costs at least the leaf's geometry.
    let later = |i: usize| (i + 1..transformed.len()).filter(|&j| transformed[j] || j == i + 1 || transformed[j - 1]).count();
    for (i, item) in original.appearance.items.iter().enumerate() {
        let fx = item.effects();
        let Some(at) = fx.iter().position(makes_piece).filter(|_| crate::paints(item)) else {
            plain.push(item.clone());
            continue;
        };
        if !plain.is_empty() {
            pieces.push(Arc::new(piece(std::mem::take(&mut plain))));
            *used = used.plus(cost);
        }
        let before = &fx[..at];
        // What the art's other paths leave of the budget, less the leaf's geometry for each piece
        // still to come: the geometry before the Transform effect and its copies keep to it.
        let reserved = (0..later(i)).fold(Budget { anchors: 0, lines: 0 }, |r, _| r.plus(cost));
        let limit = max.minus(reserved);
        let room = limit.minus(*used);
        // The fill's or stroke's gradient follows the geometry effects before the Transform effect.
        let mut carry = crate::Carry::default();
        let g = if crate::has_geometry(before) && !path.is_empty() {
            crate::geometry::apply_geometry_carrying_within(before, &path, bounds, &ctx.item(item), room, Some(&mut carry))
        } else {
            path.clone()
        };
        let Some(tb) = g.bounds() else { continue };
        let params = merged_params(&fx[at].id, &fx[at].params);
        // Where each effect the piece keeps sits among the item's effects.
        let kept: Vec<usize> = (0..at).filter(|&k| !is_geometry(&fx[k].id)).chain(at + 1..fx.len()).collect();
        // A Transform effect after this one that makes copies or moves at random holds the raster
        // effects before this one back from every Transform effect ([`crate::raster::followed`]).
        let held_later = fx.get(at + 1..).unwrap_or_default().iter().any(crate::raster::holds_back);
        // The effects before the Transform effect that it keeps, each raster effect following the
        // geometry effects before it that moved the geometry ([`crate::raster::without_geometry`],
        // [`crate::Carry::hide_skipped`]) unless held back.
        let mut ran = std::borrow::Cow::Borrowed(before);
        if carry.skipped() {
            carry.hide_skipped(ran.to_mut());
        }
        let kept_before: Vec<Effect> =
            if held_later { before.iter().filter(|e| !is_geometry(&e.id)).cloned().collect() } else { crate::without_geometry(&ran) };
        if let Some(plugin) = painted_plugin(&fx[at]) {
            let mut it = item.clone();
            let rest: Vec<Effect> = kept_before.iter().chain(&fx[at + 1..]).cloned().collect();
            *it.effects_mut() = rest;
            let mut one = Appearance { items: vec![it.clone()], ..Default::default() };
            if carry.moves(&one) {
                carry.apply(&mut one);
            }
            let it = one.items.pop().unwrap_or(it);
            let stage = vectorcraft_plugins::deferred::Stage { object: ctx.seed, effect: at, item: Some(i) };
            let paints = vectorcraft_plugins::effect::Paints { appearance: Some(&original.appearance), item: Some(i), at: Some(stage) };
            let painted = vectorcraft_plugins::effect::apply_painted(plugin, &params, &g, tb, rule, paints, doc)
                .map(|pieces| pieces.iter().map(|p| (p.path.clone(), p.rule, vectorcraft_plugins::painted::items(&it, p))).collect::<Vec<_>>())
                .filter(|pieces| room.fits(pieces.iter().map(|(p, ..)| piece_cost(p)).fold(Budget { anchors: 0, lines: 0 }, Budget::plus)));
            // A failed run draws the fill or stroke on the geometry it was given.
            let returned = painted.unwrap_or_else(|| vec![(g.clone(), rule, vec![it])]);
            let mut group: Vec<Arc<Node>> = vec![];
            for (path, rule, items) in returned.into_iter().filter(|(_, _, items)| !items.is_empty()) {
                *used = used.plus(piece_cost(&path));
                let mut p = piece(items);
                set_geometry(&mut p, path, rule);
                group.push(Arc::new(p));
            }
            let mut p = Node::group(leaf.id, group);
            transform_piece(&mut p, (ctx.seed, i, &kept), crate::raster::held_back(before), true, limit, used, doc);
            pieces.push(Arc::new(p));
            continue;
        }
        let (_, copies) = crate::distort::transform_map(tb, &params, ctx.seed);
        let options = crate::distort::TransformOptions::of(&params);
        let map = crate::distort::transform_affine(tb, &params, ctx.seed);
        let mut it = item.clone();
        let rest: Vec<Effect> = kept_before.iter().chain(&fx[at + 1..]).cloned().collect();
        *it.effects_mut() = rest.clone();
        let mut p = piece(vec![it]);
        if carry.moves(&p.appearance) {
            carry.apply(&mut p.appearance);
        }
        set_geometry(&mut p, g.clone(), rule);
        let after = kept_before.len();
        let keep = |k: usize| k >= after || !options.objects;
        let (made, moved) = transform_leaf(&mut p, &g, map, copies, options, room, false, Some((&rest, &keep)));
        *used = used.plus(made);
        let from = crate::raster::held_back(before);
        if moved && !held_later {
            follow_piece(&mut p, kept_before.get(from..).unwrap_or_default(), from, &fx[at]);
        }
        transform_piece(&mut p, (ctx.seed, i, &kept), crate::raster::held_back(&fx[..=at]), false, limit, used, doc);
        pieces.push(Arc::new(p));
    }
    if !plain.is_empty() {
        pieces.push(Arc::new(piece(plain)));
        *used = used.plus(cost);
    }
    leaf.appearance.items.clear();
    leaf.appearance.contents_index = None;
    leaf.knockout = vectorcraft_doc::Knockout::Off;
    leaf.kind = NodeKind::Group { children: pieces, clip: false };
}

/// Apply the visible geometry effects left on the fills or strokes of each path under `p` (a
/// transformed piece of [`transform_item_leaf`] and its copies) to all of `p`'s paths as one piece:
/// each effect takes the bounds of all the paths as its reference box and `seed`, as an object's
/// own effects reshape all its paths ([`reshape_leaves_carrying`]). Each path applies its own copy of
/// the effect, at the effect's own distances. A Transform effect with Transform Patterns or Scale
/// Strokes & Effects on transforms each path as [`transform_leaf`] does, and leaves the values of
/// the effects after it, and with Transform Objects off those before it, as they are
/// ([`keep_listed`]).
/// A live-effect plug-in that paints what it returns ([`painted_plugin`]) makes each path one path
/// for each piece it returns ([`paint_piece`]). A path with several fills or strokes (those a
/// painted plug-in returned) takes such an effect, a live-effect plug-in or a geometry effect on
/// strokes once for each of them, in paint order, given its own place among them
/// ([`shares_stage`]): each run's pieces replace only that fill or stroke, and a run that fails or
/// would pass the budget leaves that fill or stroke on the path as it was. The raster effects
/// before a Transform effect follow it ([`follow_piece`]), except the first `held` effects, which
/// a Transform effect before the piece's effects that makes copies or moves at random holds back,
/// and the effects before such a Transform effect among the piece's effects. The geometry effects then
/// leave the fills and strokes; their other effects stay. The paths keep to `max` together with
/// the rest of the art (`used`), and the swatches and patterns painted pieces name must be in
/// `doc`. `(seed, source, kept)`: the object's seed, the index in the object's appearance of the
/// fill or stroke the piece was made from, and the index among that item's effects of each effect
/// the piece kept, which a deferred live-effect plug-in is listed by. `painted`: `p`'s paths are
/// the pieces of a painted run, each of which `used` counts at [`piece_cost`].
fn transform_piece(
    p: &mut Node,
    (seed, source, kept): (u64, usize, &[usize]),
    held: usize,
    painted: bool,
    max: Budget,
    used: &mut Budget,
    doc: Option<&Document>,
) {
    let mut count = 0;
    for_each_leaf(p, &mut |l| {
        for item in &l.appearance.items {
            count = count.max(item.effects().len());
        }
    });
    // The effects before a Transform effect among the piece's effects that makes copies or moves at
    // random are held back from the Transform effects before it too.
    let mut held = held;
    for_each_leaf(p, &mut |l| {
        for item in &l.appearance.items {
            if let Some(last) = item.effects().iter().rposition(crate::raster::holds_back) {
                held = held.max(last);
            }
        }
    });
    // Whether `used` counts each path under `p`, in the order `for_each_leaf` visits them, at
    // [`piece_cost`] (its geometry and a painted piece's share) rather than at its geometry alone.
    // The pieces of a painted run are counted so, and so is the first path an effect makes of one
    // that was: the paths of a Transform effect's copies share their original's share.
    let mut shares = vec![painted; leaf_count(p)];
    for j in 0..count {
        let Some(bounds) = p.geometric_bounds() else { break };
        let from = held.min(j);
        let mut holds = false;
        let stage = vectorcraft_plugins::deferred::Stage { object: seed, effect: kept.get(j).copied().unwrap_or(j), item: Some(source) };
        let at = PieceStage { j, from, seed, bounds, doc, stage };
        // Stage `j` on `leaf`, a path `used` counts at `piece_cost` when `share`: whether `used`
        // counts each path it became so, or `None` when it stays as it was.
        let mut on_leaf = |leaf: &mut Node, share: bool| -> Option<Vec<bool>> {
            let effect = |item: &AppearanceItem| item.effects().get(j).filter(|e| e.visible && is_geometry(&e.id)).cloned();
            let items = &leaf.appearance.items;
            let first = items.first()?;
            let (path, _) = leaf_geometry(leaf)?;
            // What `used` counts the path at; what the other paths leave of the budget is the rest.
            let geometry = Budget::of(&path);
            let counted = if share { piece_cost(&path) } else { geometry };
            if shares_stage(items, j) {
                let e = effect(first)?;
                holds |= crate::raster::holds_back(&e);
                let appearance = leaf.appearance.clone();
                // A painted run's pieces each cost a share of their own ([`paint_piece`]); the
                // paths of any other effect keep the leaf's.
                let painted = painted_plugin(&e).is_some();
                let before = if painted { counted } else { geometry };
                let room = max.minus(used.minus(before));
                let made = piece_stage(leaf, &e, &at, (&appearance, 0), room)?;
                *used = used.minus(before).plus(made);
                return Some((0..leaf_count(leaf)).map(|k| painted || (share && k == 0)).collect());
            }
            // Each fill or stroke takes the effect on its own path, given the leaf's fills and
            // strokes and its own place among them; those left with the same path as the one
            // before them stay on it. Each path costs its geometry and a painted piece's share.
            // Each run is first given all the room the art leaves. When a fill or stroke that keeps
            // the leaf's path then finds no room for its copy of it, the results are placed again,
            // each but the last leaving room for one more copy of the leaf's own path; a result
            // that does not fit that room is left out. Only a fill or stroke that keeps the leaf's
            // path right after one that changed costs such a copy (the ones after it stay on that
            // copy), so every copy fits but the first fill's or stroke's, when the art leaves no
            // room for it: the leaf then stays as it was. A live-effect plug-in is given all the
            // room the art leaves, and each placing checks that its result fits; a built-in effect
            // builds within the room of each placing.
            let appearance = leaf.appearance.clone();
            let one = piece_cost(&path);
            let n = appearance.items.len();
            let base = used.minus(counted);
            let alone = |item: &AppearanceItem| {
                let mut sub = leaf.clone();
                sub.appearance.items = vec![item.clone()];
                sub
            };
            // `item`, the `k`th fill or stroke, alone on the leaf's path with effect `e` applied
            // within `room`, and what its paths cost; `None` when it failed or found no room.
            let apply = |item: &AppearanceItem, k: usize, e: &Effect, room: Budget| {
                let painted = painted_plugin(e).is_some();
                let room = if painted { room } else { room.minus(piece_share()) };
                let mut sub = alone(item);
                let made = piece_stage(&mut sub, e, &at, (&appearance, k), room)?;
                Some((sub, if painted { made } else { made.plus(piece_share()) }))
            };
            // Each fill's or stroke's live-effect plug-in run. A result is kept only while a
            // placing places it on a path of its own; one that does not fit is dropped, with what
            // it cost, and the plug-in runs again in the second placing only when that much room
            // is left there (a result small enough to cache comes back from the cache). A result
            // with the same path as the one before it costs nothing: it is placed on that path
            // whatever room is left, and dropped once its fill or stroke is moved there. The
            // results kept at once cost at most what the two placings place.
            let mut runs: Vec<PluginRun> = vec![PluginRun::Not; n];
            let mut kept_cost = Budget { anchors: 0, lines: 0 };
            // Each placing gives the paths, whether `used` counts each of the paths they hold at
            // `piece_cost` (all of a painted run's, and the first of any other result's), and what
            // the art then spends.
            let mut split = |reserve: bool| -> Option<(Vec<Node>, Vec<bool>, Budget)> {
                let mut spent = base;
                let mut out: Vec<Node> = vec![];
                let mut counts: Vec<bool> = vec![];
                for (k, (item, ran)) in appearance.items.iter().zip(runs.iter_mut()).enumerate() {
                    let room = if reserve && k + 1 < n { max.minus(spent).minus(one) } else { max.minus(spent) };
                    let e = effect(item);
                    let paints = e.as_ref().is_some_and(|e| painted_plugin(e).is_some());
                    // Does `sub` stay on the path of the result before it, at no cost?
                    let merges = |sub: &Node| out.last().is_some_and(|last| same_path(last, sub));
                    // What the kept plug-in result this fill or stroke takes cost.
                    let mut taken = None;
                    let result = match e {
                        None => None,
                        Some(e) => {
                            holds |= crate::raster::holds_back(&e);
                            if vectorcraft_plugins::effect::plugin_id(&e.id).is_some() {
                                let again = match ran {
                                    PluginRun::Not | PluginRun::Merged => true,
                                    PluginRun::Left(cost) => room.fits(*cost),
                                    PluginRun::Kept(..) | PluginRun::Failed => false,
                                };
                                if again {
                                    #[cfg(test)]
                                    PLUGIN_RUNS.with(|n| n.set(n.get() + 1));
                                    *ran = match apply(item, k, &e, max.minus(base)) {
                                        None => PluginRun::Failed,
                                        Some((sub, cost)) if !room.fits(cost) && !merges(&sub) => PluginRun::Left(cost),
                                        Some((sub, cost)) => {
                                            kept_cost = kept_cost.plus(cost);
                                            PluginRun::Kept(sub, cost)
                                        }
                                    };
                                }
                                match ran {
                                    PluginRun::Kept(sub, cost) if room.fits(*cost) || merges(sub) => {
                                        taken = Some(*cost);
                                        Some((sub.clone(), *cost))
                                    }
                                    PluginRun::Kept(_, cost) => {
                                        let cost = *cost;
                                        kept_cost = kept_cost.minus(cost);
                                        *ran = PluginRun::Left(cost);
                                        None
                                    }
                                    PluginRun::Not | PluginRun::Merged | PluginRun::Left(_) | PluginRun::Failed => None,
                                }
                            } else {
                                // A built-in effect that finds no room within `room` leaves the
                                // leaf's path. When its result is not on the path before it, it
                                // is built again with all the room the art leaves and taken only
                                // when that result is on the path before it, which costs nothing.
                                let made = apply(item, k, &e, room);
                                let short = match &made {
                                    None => true,
                                    Some((sub, cost)) => !merges(sub) && (!room.fits(*cost) || same_path(sub, leaf)),
                                };
                                match out.last() {
                                    Some(_) if short => match apply(item, k, &e, max.minus(base)) {
                                        Some((sub, cost)) if merges(&sub) => Some((sub, cost)),
                                        _ => made,
                                    },
                                    _ => made,
                                }
                            }
                        }
                    };
                    let placed = match result {
                        Some((sub, cost)) if room.fits(cost) || merges(&sub) => Some((sub, cost, paints)),
                        _ => None,
                    };
                    // A fill or stroke whose result is not placed stays on the leaf's path, which
                    // is copied only when the path before it is another.
                    let (mut sub, cost, all) = match placed {
                        Some(placed) => placed,
                        None => match out.last_mut() {
                            Some(last) if same_path(last, leaf) => {
                                last.appearance.items.push(item.clone());
                                continue;
                            }
                            _ => (alone(item), one, true),
                        },
                    };
                    match out.last_mut() {
                        Some(last) if same_path(last, &sub) => {
                            last.appearance.items.append(&mut sub.appearance.items);
                            if let Some(cost) = taken {
                                kept_cost = kept_cost.minus(cost);
                                *ran = PluginRun::Merged;
                            }
                        }
                        _ if !max.minus(spent).fits(cost) => return None,
                        _ => {
                            spent = spent.plus(cost);
                            counts.extend((0..leaf_count(&sub)).map(|k| all || k == 0));
                            out.push(sub);
                        }
                    }
                    #[cfg(test)]
                    HELD_PEAK.with(|p| p.set(p.get().at_least(kept_cost)));
                }
                Some((out, counts, spent))
            };
            let (mut out, counts, spent) = split(false).or_else(|| split(true))?;
            *used = spent;
            *leaf = match out.len() {
                1 => out.pop().unwrap_or_else(|| leaf.clone()),
                _ => Node::group(leaf.id, out.into_iter().map(Arc::new).collect()),
            };
            Some(counts)
        };
        let mut next: Vec<bool> = vec![];
        let mut visited = 0;
        for_each_leaf(p, &mut |leaf| {
            let share = shares.get(visited).copied().unwrap_or(false);
            visited += 1;
            next.extend(on_leaf(leaf, share).unwrap_or_else(|| vec![share]));
        });
        shares = next;
        if holds {
            held = held.max(j);
        }
    }
    for_each_leaf(p, &mut |leaf| {
        for item in &mut leaf.appearance.items {
            item.effects_mut().retain(|e| !(e.visible && is_geometry(&e.id)));
        }
    });
}

/// A fill's or stroke's live-effect plug-in run in a placing of [`transform_piece`]: not run yet,
/// its result and what that costs while a placing places it on a path of its own, what a result
/// that did not fit cost (the result itself is dropped), a result dropped once its fill or stroke
/// was placed on the path before it, or failed.
#[derive(Clone)]
enum PluginRun {
    Not,
    Kept(Node, Budget),
    Left(Budget),
    Merged,
    Failed,
}

#[cfg(test)]
thread_local! {
    /// The most the live-effect plug-in results [`transform_piece`] keeps from one fill or stroke
    /// to the next have cost at once on this thread.
    pub(crate) static HELD_PEAK: std::cell::Cell<Budget> = const { std::cell::Cell::new(Budget { anchors: 0, lines: 0 }) };
    /// How many times [`transform_piece`] has run a fill's or stroke's live-effect plug-in on this
    /// thread when they did not share the stage.
    pub(crate) static PLUGIN_RUNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Where a stage of [`transform_piece`] runs: `j`, the index of its effect among the piece's
/// effects; `from`, the first of the effects before it whose raster effects follow a Transform
/// effect; the object's `seed`; the reference box `bounds` of all the piece's paths; the document
/// that painted pieces' swatches and patterns must be in; and the `stage` a deferred live-effect
/// plug-in is listed by.
struct PieceStage<'a> {
    j: usize,
    from: usize,
    seed: u64,
    bounds: Rect,
    doc: Option<&'a Document>,
    stage: vectorcraft_plugins::deferred::Stage,
}

/// Can stage `j` of [`transform_piece`] run once for all of a path's `items`: there is at most one,
/// or they hold the same effects and the effect at `j` is not a visible geometry effect, is a
/// Transform effect, or is a built-in geometry effect and they are all fills (an effect that
/// changes a fill's path does not depend on which fill it is on)?
fn shares_stage(items: &[AppearanceItem], j: usize) -> bool {
    let Some(first) = items.first() else { return true };
    if items.len() == 1 {
        return true;
    }
    if !items.iter().all(|i| i.effects() == first.effects()) {
        return false;
    }
    match first.effects().get(j) {
        Some(e) if e.visible && is_geometry(&e.id) => {
            vectorcraft_plugins::effect::plugin_id(&e.id).is_none() && (reaches_appearance(e) || items.iter().all(AppearanceItem::is_fill))
        }
        _ => true,
    }
}

/// Effect `e`, a visible geometry effect at stage `at` of [`transform_piece`], applied to `leaf`, a
/// path painted by its first fill or stroke (and, when they share the stage, [`shares_stage`], the
/// others): `appearance` is what the effect is given of the path's fills and strokes and `index`
/// which of them it is on. A live-effect plug-in that paints what it returns makes `leaf` a group
/// of its pieces ([`paint_piece`]), and a Transform effect that reaches its appearance transforms
/// it ([`transform_leaf`]). Returns what the new paths cost, within `room`; `None` when `leaf`
/// stays as it is.
fn piece_stage(leaf: &mut Node, e: &Effect, at: &PieceStage, (appearance, index): (&Appearance, usize), room: Budget) -> Option<Budget> {
    let item = leaf.appearance.items.first()?.clone();
    // The effects before this one as they were before it moved the path.
    let was = item.effects().get(at.from..at.j).unwrap_or_default();
    let (path, rule) = leaf_geometry(leaf)?;
    if let Some(plugin) = painted_plugin(e) {
        let params = merged_params(&e.id, &e.params);
        return paint_piece(leaf, &item, (appearance, index), (plugin, &params), at.bounds, room, at.doc, Some(at.stage));
    }
    if reaches_appearance(e) {
        let params = merged_params(&e.id, &e.params);
        let (_, copies) = crate::distort::transform_map(at.bounds, &params, at.seed);
        let options = crate::distort::TransformOptions::of(&params);
        let map = crate::distort::transform_affine(at.bounds, &params, at.seed);
        let j = at.j;
        let keep = |k: usize| k > j || (k < j && !options.objects);
        let (made, moved) = transform_leaf(leaf, &path, map, copies, options, room, false, Some((item.effects(), &keep)));
        if moved {
            follow_piece(leaf, was, at.from, e);
        }
        return Some(made);
    }
    let stroke = match &item {
        AppearanceItem::Stroke(s) => Some(s),
        AppearanceItem::Fill(_) => None,
    };
    let ctx = GeomContext { stroke, rule, seed: at.seed, appearance: Some(appearance), item: Some(index), doc: at.doc, stage: Some(at.stage) };
    let mut carry = crate::Carry::default();
    let out = crate::geometry::apply_geometry_carrying_within(std::slice::from_ref(e), &path, at.bounds, &ctx, room, Some(&mut carry));
    let made = Budget::of(&out);
    // The fill's or stroke's gradient follows the effect as the object's does.
    if carry.moves(&leaf.appearance) {
        carry.apply(&mut leaf.appearance);
    }
    set_geometry(leaf, out, rule);
    // A Transform effect left out (past the budget) leaves the path and its raster effects as they were.
    if !carry.skipped() {
        follow_piece(leaf, was, at.from, e);
    }
    Some(made)
}

/// Are `a` and `b` paths or compound paths that differ only in their fills and strokes?
fn same_path(a: &Node, b: &Node) -> bool {
    let leaf = |n: &Node| matches!(n.kind, NodeKind::Path { .. } | NodeKind::Compound { .. });
    if !leaf(a) || !leaf(b) {
        return false;
    }
    // Every field but the fills and strokes, compared in place; the geometry last.
    let Node { id, name, visible, locked, opacity, blend, isolate, knockout, knockout_shape, appearance, kind, bbox_angle, extras } = a;
    let Appearance { items: _, effects, contents_index } = &**appearance;
    *id == b.id
        && *name == b.name
        && *visible == b.visible
        && *locked == b.locked
        && *opacity == b.opacity
        && *blend == b.blend
        && *isolate == b.isolate
        && *knockout == b.knockout
        && *knockout_shape == b.knockout_shape
        && *bbox_angle == b.bbox_angle
        && *extras == b.extras
        && *effects == b.appearance.effects
        && *contents_index == b.appearance.contents_index
        && *kind == b.kind
}

/// The effects a Transform effect in a fill's or stroke's own list leaves as they were
/// ([`keep_listed`]): the list as it was before the Transform effect was applied, and which of its
/// indices keep their values. `None` for an object's own Transform effect.
type Kept<'a> = Option<(&'a [Effect], &'a dyn Fn(usize) -> bool)>;

/// Put back on the fills and strokes of each path under `p` the effect `kept`'s list holds at each
/// index it takes, in place of what a Transform effect in their own list scaled or moved with its
/// options ([`transform_leaf`]). The effects after the Transform effect apply to the art it made,
/// and with Transform Objects off the effects before it keep their values too. A piece's fills and
/// strokes share one list: a live-effect plug-in that paints what it returns gives each paint a copy
/// of the fill or stroke ([`vectorcraft_plugins::painted::items`]).
fn keep_listed(p: &mut Node, kept: Kept) {
    let Some((was, keep)) = kept else { return };
    for_each_leaf(p, &mut |leaf| {
        for item in &mut leaf.appearance.items {
            for (k, (e, old)) in item.effects_mut().iter_mut().zip(was).enumerate() {
                if keep(k) {
                    e.clone_from(old);
                }
            }
        }
    });
}

/// Make the raster effects of each of `leaf`'s fills and strokes from index `from` on follow
/// Transform effect `t`, listed after them, once `t` has moved the leaf
/// ([`crate::raster::follow_one`]). The fills and strokes are copies of one fill or stroke (those a
/// painted plug-in returns, [`vectorcraft_plugins::painted::items`]), with the same effects. `was`
/// holds those effects of the first one as they were before `t` was applied: each one that
/// follows takes the value [`crate::raster::followed`] gives it, in place of what Scale Strokes &
/// Effects made of it, and the others keep theirs.
fn follow_piece(leaf: &mut Node, was: &[Effect], from: usize, t: &Effect) {
    let mut follow = was.to_vec();
    if !crate::raster::follow_one(&mut follow, t) {
        return;
    }
    for item in &mut leaf.appearance.items {
        for (k, (new, old)) in follow.iter().zip(was).enumerate() {
            if new != old
                && let Some(e) = item.effects_mut().get_mut(from + k)
            {
                *e = new.clone();
            }
        }
    }
}

/// The bounds of each group of type's outlines under `n` ([`outline_text`]), `n` included, in
/// order. A group inside one of them is part of it.
fn type_outline_bounds(n: &Node, out: &mut Vec<Option<Rect>>) {
    if n.extras().type_outlines {
        out.push(n.geometric_bounds());
    } else if let NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } = &n.kind {
        for c in children {
            type_outline_bounds(c, out);
        }
    }
}

/// Move and scale the placed gradients of each group of type's outlines under `n` from its bounds
/// in `before` ([`type_outline_bounds`] before its paths changed) to its bounds now.
fn rebase_type_outlines(n: &mut Node, before: &mut std::vec::IntoIter<Option<Rect>>) {
    if !n.extras().type_outlines {
        if let NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } = &mut n.kind {
            for c in children.iter_mut() {
                rebase_type_outlines(Arc::make_mut(c), before);
            }
        }
    } else if let Some(Some(from)) = before.next()
        && let Some(to) = n.geometric_bounds()
        && from != to
    {
        for_each_leaf(n, &mut |leaf| {
            if leaf.appearance.has_positioned_gradient() {
                follow_boxes(&mut leaf.appearance, from, to);
            }
        });
    }
}

/// The geometric bounds of each path under `n` that [`for_each_leaf`] visits, outside groups of
/// type's outlines (those follow the outlines' bounds, [`rebase_type_outlines`]).
fn leaf_bounds(n: &Node, out: &mut Vec<Option<Rect>>) {
    match &n.kind {
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => out.push(n.geometric_bounds()),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } if !n.extras().type_outlines => {
            for c in children {
                leaf_bounds(c, out);
            }
        }
        _ => {}
    }
}

/// A group inside the art whose own fills and strokes [`carry_groups`] moves (groups of type's
/// outlines follow the outlines' bounds, [`rebase_type_outlines`]).
fn is_member_group(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) && !n.extras().type_outlines
}

/// A group below the art that [`is_member_group`] takes and whose own fills or strokes hold a
/// gradient: the child indices that lead to it from the art, its geometric bounds, and
/// whether a path below it that its bounds measure kept its shape.
type GroupMark = (Vec<usize>, Option<Rect>, bool);

/// The geometric bounds of `n` (as [`Node::geometric_bounds`] gives them) and whether a path or
/// compound path below it that those bounds measure is in `kept`, measured in one walk. Each group below `n` that
/// [`GroupMark`] describes goes into `out`, reached from `n` through `at`. An image that `images`
/// lists is measured by the pieces it will be cut into ([`WarpedImages::image_bounds`]).
fn group_marks(
    n: &Node,
    at: &mut Vec<usize>,
    kept: &HashSet<NodeId>,
    images: Option<&WarpedImages>,
    out: &mut Vec<GroupMark>,
) -> (Option<Rect>, bool) {
    match &n.kind {
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => (n.geometric_bounds(), kept.contains(&n.id)),
        NodeKind::Image(_) => (images.map_or_else(|| n.geometric_bounds(), |w| w.image_bounds(n)), false),
        NodeKind::Group { children, clip } | NodeKind::Layer { children, clip, .. } if !n.extras().type_outlines => {
            let layer = matches!(n.kind, NodeKind::Layer { .. });
            let skip = children.len().saturating_sub(n.editing_children().len());
            let (mut bounds, mut held) = (None, false);
            for (i, c) in children.iter().enumerate() {
                at.push(i);
                let (b, h) = group_marks(c, at, kept, images, out);
                if is_member_group(c) && (c.appearance.has_positioned_gradient() || c.appearance.has_unplaced_gradient()) {
                    out.push((at.clone(), b, h));
                }
                at.pop();
                // Only members measured in the bounds hold the group's gradients in place.
                let counts = if *clip { i == 0 } else { i >= skip && (c.visible || !layer) };
                if counts {
                    bounds = vectorcraft_geom::union_opt(bounds, b);
                    held |= h;
                }
            }
            (bounds, held)
        }
        // Type's outlines: measured as a whole, with each outline that kept its shape counted.
        NodeKind::Group { .. } | NodeKind::Layer { .. } => (n.geometric_bounds(), !kept.is_empty() && holds_kept(n, kept)),
        _ => (n.geometric_bounds(), false),
    }
}

/// Is a path or compound path below `n` (`n` included) in `kept`?
fn holds_kept(n: &Node, kept: &HashSet<NodeId>) -> bool {
    match &n.kind {
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => kept.contains(&n.id),
        _ => n.children().into_iter().flatten().any(|c| holds_kept(c, kept)),
    }
}

/// Move the placed gradients of the own fills and strokes of each group in `after` (the groups of
/// `before`, measured again once the paths changed) by what `carry` gives for its bounds before
/// and after. A group with a path below it that kept its shape keeps them in place.
fn carry_groups(art: &mut Node, before: Vec<GroupMark>, after: Vec<GroupMark>, carry: &dyn Fn(Option<Rect>, Option<Rect>) -> crate::Carry) {
    for ((at, from, _), (at_after, to, held)) in before.into_iter().zip(after) {
        if held || at != at_after {
            continue;
        }
        let k = carry(from, to);
        if k.is_empty() {
            continue;
        }
        if let Some(n) = node_at(art, &at)
            && is_member_group(n)
            && k.moves(&n.appearance)
        {
            k.apply(&mut n.appearance);
        }
    }
}

/// The node below `n` that the child indices `at` lead to.
fn node_at<'a>(n: &'a mut Node, at: &[usize]) -> Option<&'a mut Node> {
    let Some((first, rest)) = at.split_first() else { return Some(n) };
    let c = n.children_mut()?.get_mut(*first)?;
    node_at(Arc::make_mut(c), rest)
}

/// Move and scale the placed gradients of each path under `n` that [`leaf_bounds`] lists from its
/// bounds in `before` (before its geometry changed) to its bounds now.
fn rebase_leaves(n: &mut Node, before: &mut std::vec::IntoIter<Option<Rect>>) {
    if matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. }) {
        if let Some(Some(from)) = before.next()
            && let Some(to) = n.geometric_bounds()
            && from != to
            && n.appearance.has_positioned_gradient()
        {
            follow_boxes(&mut n.appearance, from, to);
        }
        return;
    }
    if n.extras().type_outlines {
        return;
    }
    if let NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } = &mut n.kind {
        for c in children.iter_mut() {
            if matches!(c.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } | NodeKind::Group { .. } | NodeKind::Layer { .. }) {
                rebase_leaves(Arc::make_mut(c), before);
            }
        }
    }
}

/// Map the placed gradients of `ap` from box `from` to box `to` through the map that takes one box
/// onto the other (strokes: their stroke-inflated boxes). The start and end go where
/// [`Appearance::rebase_gradients`] puts them, and a radial's ellipse and a linear's isolines are
/// mapped with them.
pub(crate) fn follow_boxes(ap: &mut Appearance, from: Rect, to: Rect) {
    for i in &mut ap.items {
        match i {
            AppearanceItem::Fill(f) => {
                if let vectorcraft_doc::color::Paint::Gradient(g) = &mut f.paint {
                    g.transform(box_map(from, to));
                }
            }
            AppearanceItem::Stroke(s) => {
                let m = box_map(s.paint_bounds(from), s.paint_bounds(to));
                if let vectorcraft_doc::color::Paint::Gradient(g) = &mut s.paint {
                    g.transform(m);
                }
            }
        }
    }
}

/// The map that takes box `from` onto box `to` along each axis (along an axis `from` has no extent
/// on, a move from its center to `to`'s).
fn box_map(from: Rect, to: Rect) -> vectorcraft_geom::Affine {
    let axis = |a0: f64, a1: f64, b0: f64, b1: f64| {
        if (a1 - a0).abs() > 1e-12 {
            let s = (b1 - b0) / (a1 - a0);
            (s, b0 - a0 * s)
        } else {
            (1.0, (b0 + b1 - a0 - a1) / 2.0)
        }
    };
    let (sx, tx) = axis(from.x0, from.x1, to.x0, to.x1);
    let (sy, ty) = axis(from.y0, from.y1, to.y0, to.y1);
    vectorcraft_geom::Affine::new([sx, 0.0, 0.0, sy, tx, ty])
}

/// Apply the Transform effect's map `m` to every image and symbol instance under `n`, as
/// [`for_each_leaf`] does to paths: the map goes into the object's placement, and `copies` more
/// follow, each mapped once more than the last. Without copies the object itself takes the map,
/// as a path does. The art of each one's linked opacity mask takes the map with `scaling`, as a
/// path's mask does.
fn transform_placed(n: &mut Node, m: Affine, copies: usize, scaling: vectorcraft_doc::Scaling) {
    let mut room = crate::distort::MAX_COPY_OBJECTS.saturating_sub(placed_count(n));
    copy_placed(n, m, copies, scaling, &mut room);
}

/// [`transform_placed`] whose copies make at most `max` image and symbol instance nodes of the art,
/// the originals included: each copy adds the object once more, and a Transform after this one
/// copies the lot again, so copies stop where the result would pass `max` (#2042).
#[cfg(test)]
pub(crate) fn transform_placed_within(n: &mut Node, m: Affine, copies: usize, max: usize) {
    let mut room = max.saturating_sub(placed_count(n));
    copy_placed(n, m, copies, vectorcraft_doc::Scaling::default(), &mut room);
}

/// Rows and columns of the pieces a Warp effect cuts an image into (VectorCraft's own number).
pub(crate) const WARP_IMAGE_CELLS: usize = 10;

/// A Warp effect's point map, shared by the images it bends.
pub(crate) type WarpMap = Rc<dyn Fn(Point) -> Point>;

/// One image a Warp effect bent: the image's own key, and each warp it took with the placement
/// the image had when it took it, in order.
type WarpedImage = (String, Vec<(WarpMap, Affine)>);

/// The images Warp effects bend while [`reshape_leaves_apart`] runs the effects. Each such image's
/// key is replaced by a key of this table ([`WarpedImages::add`]), which a copy a later Transform
/// effect makes keeps. The images are cut into pieces once, after the last effect
/// ([`cut_warped_images`]): the effects after a Warp reshape the image's frame, not each piece,
/// and a later Warp or Transform moves the one set of pieces.
pub(crate) struct WarpedImages {
    /// What this table's keys start with: no image key in the art starts with it
    /// ([`WarpedImages::reserve`]).
    prefix: String,
    /// The prefix was checked against the art's image keys.
    reserved: bool,
    images: Vec<WarpedImage>,
}

impl WarpedImages {
    pub(crate) fn new() -> Self {
        static TABLES: AtomicUsize = AtomicUsize::new(0);
        Self { prefix: format!("\0warp{}:", TABLES.fetch_add(1, Ordering::Relaxed)), reserved: false, images: vec![] }
    }

    /// Before the first image of `art` is marked: lengthen the prefix until no image key in `art`
    /// starts with it, so no key from the document is read as one of this table's.
    pub(crate) fn reserve(&mut self, art: &Node) {
        if self.reserved {
            return;
        }
        self.reserved = true;
        let mut keys = vec![];
        art.walk(&mut |c| {
            if let NodeKind::Image(im) = &c.kind {
                keys.push(im.key.clone());
            }
        });
        while keys.iter().any(|k| k.starts_with(self.prefix.as_str())) {
            self.prefix.push(':');
        }
    }

    /// What this table's keys start with.
    #[cfg(test)]
    pub(crate) fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The image that `key`, a key of this table, stands for.
    fn get(&self, key: &str) -> Option<&WarpedImage> {
        self.images.get(key.strip_prefix(self.prefix.as_str())?.parse::<usize>().ok()?)
    }

    /// Record that the image with key `key`, placed by `xf`, takes warp `f`; returns the image's
    /// new key.
    fn add(&mut self, key: &str, xf: Affine, f: &WarpMap) -> String {
        let (own, mut warps) = self.get(key).cloned().unwrap_or_else(|| (key.to_string(), vec![]));
        warps.push((f.clone(), xf));
        self.images.push((own, warps));
        format!("{}{}", self.prefix, self.images.len() - 1)
    }

    /// Does the art `n` have room, within `max` nodes, for the pieces of every image under it that
    /// this table lists?
    fn fits(&self, n: &Node, max: usize) -> bool {
        let mut count = 0usize;
        n.walk(&mut |c| {
            if let NodeKind::Image(im) = &c.kind
                && self.get(&im.key).is_some()
            {
                count += 1;
            }
        });
        vectorcraft_doc::live::mesh_warp_cost(WARP_IMAGE_CELLS).saturating_mul(count) <= max.saturating_sub(n.count())
    }

    /// The geometric bounds of `art` as it draws after the effects so far: each image this table
    /// lists measured by the pieces the warps and moves it took would cut it into, when the art
    /// has room for them ([`cut_warped_images`]).
    pub(crate) fn bounds(&self, art: &Node) -> Option<Rect> {
        if self.images.is_empty() || !self.fits(art, crate::distort::MAX_COPY_OBJECTS) {
            return art.geometric_bounds();
        }
        // The pieces the images would be cut into now: each group of pieces is shown or hidden as
        // its image is, and hidden objects are left out of a layer's bounds.
        let mut probe = art.clone();
        restore_warped_images(&mut probe, self, true);
        probe.geometric_bounds()
    }

    /// The geometric bounds of image `n`: when this table lists it, measured by the pieces the
    /// warps and moves it took would cut it into.
    fn image_bounds(&self, n: &Node) -> Option<Rect> {
        let NodeKind::Image(im) = &n.kind else { return n.geometric_bounds() };
        if self.get(&im.key).is_none() {
            return n.geometric_bounds();
        }
        let mut probe = Node::group(n.id, vec![Arc::new(n.clone())]);
        restore_warped_images(&mut probe, self, true);
        probe.geometric_bounds()
    }

    /// This table when the art `art` has room for the pieces of the images it lists, so that a
    /// measure of the art takes them bent ([`WarpedImages::bounds`]); `None` otherwise.
    fn when_cut(&self, art: &Node) -> Option<&Self> {
        (!self.images.is_empty() && self.fits(art, crate::distort::MAX_COPY_OBJECTS)).then_some(self)
    }
}

/// Record in `warped` that each image under `n` takes warp `f`, walking the groups and layers
/// [`for_each_leaf`] walks and counting the paths it passes in `at`. `kept` holds the places of
/// the paths that kept their shape (past the budget), in the order [`for_each_leaf`] visits the
/// paths of the art: an image in a clip group whose clipping art holds one (`held`) does not take
/// the warp.
pub(crate) fn mark_warped_images(n: &mut Node, f: &WarpMap, kept: &HashSet<usize>, held: bool, at: &mut usize, warped: &mut WarpedImages) {
    let (NodeKind::Group { children, clip } | NodeKind::Layer { children, clip, .. }) = &mut n.kind else { return };
    let mut held = held;
    // A clip group's first member is its clipping art: when a path in it kept its shape, it and
    // the members it clips keep theirs.
    if *clip && let Some(first) = children.first() {
        let start = *at;
        held |= (start..start + leaf_count(first)).any(|p| kept.contains(&p));
    }
    for c in children.iter_mut() {
        match &c.kind {
            NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => *at += 1,
            NodeKind::Group { .. } | NodeKind::Layer { .. } => mark_warped_images(Arc::make_mut(c), f, kept, held, at, warped),
            NodeKind::Image(_) if !held => {
                if let NodeKind::Image(im) = &mut Arc::make_mut(c).kind {
                    im.key = warped.add(&im.key, im.xf, f);
                }
            }
            _ => {}
        }
    }
}

/// Give each image under `n` that `warped` lists its own key back, and cut it into the pieces of
/// a raster mesh warp ([`vectorcraft_doc::live::mesh_warp_image`], each piece drawn as the image
/// is, smoothed or not) through the warps it took and the moves between and after them
/// ([`warped_map`]). The art keeps to [`crate::distort::MAX_COPY_OBJECTS`] nodes: when the pieces
/// would pass it, no image is cut. An image whose placement has no inverse is not cut either.
fn cut_warped_images(n: &mut Node, warped: &WarpedImages) {
    cut_warped_images_within(n, warped, crate::distort::MAX_COPY_OBJECTS);
}

/// [`cut_warped_images`] with the art keeping to `max` nodes.
pub(crate) fn cut_warped_images_within(n: &mut Node, warped: &WarpedImages, max: usize) {
    if warped.images.is_empty() {
        return;
    }
    let cut = warped.fits(n, max);
    restore_warped_images(n, warped, cut);
}

/// The walk of [`cut_warped_images`]: each listed image gets its key back, and with `cut` is cut
/// into pieces.
fn restore_warped_images(n: &mut Node, warped: &WarpedImages, cut: bool) {
    let Some(children) = n.children_mut() else { return };
    for c in children.iter_mut() {
        let NodeKind::Image(im) = &c.kind else {
            if c.children().is_some() {
                restore_warped_images(Arc::make_mut(c), warped, cut);
            }
            continue;
        };
        let Some((key, warps)) = warped.get(&im.key) else { continue };
        let mut image = (**c).clone();
        let NodeKind::Image(own) = &mut image.kind else { continue };
        own.key = key.clone();
        // The pieces start from the image as it was placed when it took its first warp.
        let first = warps.first().map(|(_, at)| ImageObject { xf: *at, ..(**own).clone() });
        let map = warped_map(warps, own.xf);
        *c = Arc::new(match (first, map) {
            (Some(first), Some(map)) if cut => {
                let pixelated = image.extras().pixelated;
                vectorcraft_doc::live::mesh_warp_image(&image, &first, &*map, WARP_IMAGE_CELLS, pixelated, false)
            }
            _ => image,
        });
    }
}

/// The map that takes a point of an image placed by the first of `warps`' placements to where the
/// warps and the moves between them take it, for an image now placed by `placed`: each warp in
/// turn, then the move from the placement the image had when it took that warp to the one it had
/// at the next, or to `placed` after the last. `None` when a placement has no inverse.
fn warped_map(warps: &[(WarpMap, Affine)], placed: Affine) -> Option<Box<dyn Fn(Point) -> Point>> {
    let mut steps: Vec<(WarpMap, Affine)> = Vec::with_capacity(warps.len());
    for (i, (f, at)) in warps.iter().enumerate() {
        let det = at.determinant();
        if !det.is_finite() || det == 0.0 {
            return None;
        }
        let next = warps.get(i + 1).map_or(placed, |(_, at)| *at);
        let m = next * at.inverse();
        if !m.is_finite() {
            return None;
        }
        steps.push((f.clone(), m));
    }
    Some(Box::new(move |p| steps.iter().fold(p, |q, (f, m)| *m * f(q))))
}

/// How many paths [`for_each_leaf`] visits in `n`.
fn leaf_count(n: &Node) -> usize {
    match &n.kind {
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => 1,
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => children.iter().map(|c| leaf_count(c)).sum(),
        _ => 0,
    }
}

/// An object that a Transform effect places through its transform rather than reshapes: an image
/// (pixel space to document) or a symbol instance (the symbol's art to document).
fn is_placed(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Image(_) | NodeKind::SymbolInstance { .. })
}

/// The walk of [`transform_placed`]: the map and its copies go to each image and symbol instance in
/// document order, each copy taking one from `room` (what the art can still grow by), until the
/// budget is full. As with a path, the copies stop before one reaches past
/// [`crate::distort::within_reach`].
fn copy_placed(n: &mut Node, m: Affine, copies: usize, scaling: vectorcraft_doc::Scaling, room: &mut usize) {
    let Some(children) = n.children_mut() else { return };
    for c in children.iter_mut() {
        if !is_placed(c) {
            copy_placed(Arc::make_mut(c), m, copies, scaling, room);
            continue;
        }
        let src = (**c).clone();
        let placed = |xf: Affine| {
            let mut one = src.clone();
            // Gradients fitted to it are fixed to their fit first when a refit would not match.
            if crate::geometry::fixes_fit(xf) {
                one.pin_gradients();
            }
            match &mut one.kind {
                NodeKind::Image(im) => im.xf = xf * im.xf,
                NodeKind::SymbolInstance { xf: at, .. } => *at = xf * *at,
                _ => {}
            }
            // The placed gradients of its own fills and strokes move with it, as a path's do.
            if xf != Affine::IDENTITY && one.appearance.has_positioned_gradient() {
                one.appearance.transform_gradients(xf);
            }
            // So does the art of its linked opacity mask.
            if xf != Affine::IDENTITY
                && let Some(m) = one.extras_if_any().and_then(|x| x.mask.as_mut()).filter(|m| m.linked)
            {
                Arc::make_mut(&mut m.art).transform(xf, scaling);
            }
            one
        };
        if copies == 0 {
            *c = Arc::new(placed(m));
            continue;
        }
        // The copies become nodes of their own, under a group with the object's id, name and place
        // in the art (as the runs an outlined text gets).
        let mut placements: Vec<Arc<Node>> = vec![Arc::new(placed(Affine::IDENTITY))];
        let mut xf = Affine::IDENTITY;
        for _ in 0..copies.min(*room) {
            xf *= m;
            let copy = placed(xf);
            if !copy.geometric_bounds().is_some_and(crate::distort::within_reach) {
                break;
            }
            placements.push(Arc::new(copy));
        }
        *room -= placements.len() - 1;
        // The object stays on top of its copies, as a path does.
        placements.reverse();
        let mut g = Node::group(src.id, placements);
        g.name = src.name.clone();
        *c = Arc::new(g);
    }
}

/// How many images and symbol instances `n` holds below itself.
fn placed_count(n: &Node) -> usize {
    n.children().into_iter().flatten().map(|c| usize::from(is_placed(c)) + placed_count(c)).sum()
}

/// `n` with type anywhere inside it outlined and live objects evaluated, so geometry effects on
/// it reach every member.
pub(crate) fn outlined_members(doc: Option<&Document>, n: &Node) -> Node {
    outlined_members_with(doc, doc, n, &outline_text, &SymbolNodes::default())
}

/// [`outlined_members`] with live objects and symbol instances given as art through `outline`
/// ([`evaluated_with`]), with the symbols of the art from `symbols` and the swatches and patterns
/// of painted pieces looked up in `paint_doc`.
pub(crate) fn outlined_members_with(
    symbols: Option<&Document>,
    paint_doc: Option<&Document>,
    n: &Node,
    outline: OutlineHook,
    nodes: &SymbolNodes,
) -> Node {
    let mut art = evaluated_with(symbols, n, outline, nodes);
    outline_nested_text(paint_doc, &mut art, false);
    art
}

/// Run `f` on every path and compound path under `n` (guides skipped).
fn for_each_leaf(n: &mut Node, f: &mut dyn FnMut(&mut Node)) {
    match &mut n.kind {
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => f(n),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
            for c in children.iter_mut() {
                for_each_leaf(Arc::make_mut(c), f);
            }
        }
        _ => {}
    }
}

/// What the paths under `n` that [`for_each_leaf`] visits cost together, as [`leaf_geometry`]
/// gives them.
fn art_cost(n: &Node) -> Budget {
    let none = Budget { anchors: 0, lines: 0 };
    match &n.kind {
        NodeKind::Path { guide: false, path, .. } => Budget::of(path),
        NodeKind::Compound { children, .. } => children.iter().filter_map(|c| c.path_data()).map(Budget::of).fold(none, Budget::plus),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => children.iter().map(|c| art_cost(c)).fold(none, Budget::plus),
        _ => none,
    }
}

pub(crate) fn leaf_geometry(n: &Node) -> Option<(PathData, FillRule)> {
    match &n.kind {
        NodeKind::Path { path, rule, .. } => Some((path.clone(), *rule)),
        NodeKind::Compound { children, rule } => {
            Some((PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()), *rule))
        }
        _ => None,
    }
}

/// Replace the geometry of path or compound `n` (a compound when `path` has several subpaths).
fn set_geometry(n: &mut Node, path: PathData, rule: FillRule) {
    let clipping = matches!(n.kind, NodeKind::Path { clipping: true, .. });
    n.kind = if path.subpaths.len() > 1 {
        let id = n.id;
        let children = path.subpaths.into_iter().map(|sp| Arc::new(Node::path(id, PathData::single(sp), Appearance::default()))).collect();
        NodeKind::Compound { children, rule }
    } else {
        NodeKind::Path { path, rule, live: None, clipping, guide: false }
    };
}
