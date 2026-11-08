//! Object-level effects on objects that aren't paths: type, placed images, symbol instances and
//! live objects (blends, envelopes, meshes, repeats).
//!
//! Geometry effects reach them through outlines: [`outline_art`] turns the object into plain paths
//! (glyph outlines, the evaluated live object, the symbol's art) and [`reshape`] runs each effect
//! over all of them as one piece, with the whole art's bounds as the effect's reference box, so a
//! warp bends a line of type as a whole. An image is clipped by its reshaped frame. Raster effects
//! (shadows, glows, blur, feather) stay on the result for the renderer and exporters. Type without
//! inline graphics whose own fills or strokes have geometry effects is outlined too: each of those
//! effects reshapes the whole outline that its fill or stroke paints. An image's own fills and
//! strokes paint only the shape that a Convert to Shape or Outline Object effect on them gives
//! them ([`image_rows`]).

use std::collections::HashSet;
use std::sync::Arc;

use vectorcraft_doc::live::Hooks;
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Rect, Shape, shapes};

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
/// an object of its own above the one before ([`copy_leaf`]), as Expand Appearance makes it
/// (#5920).
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
/// visible geometry effect, or an image with a fill or stroke of its own that paints a shape
/// ([`image_rows`])? On type, such an effect reshapes the whole outline that its fill or stroke
/// paints ([`outline_text`], which leaves inline graphics out).
pub fn has_item_geometry(n: &Node) -> bool {
    match &n.kind {
        NodeKind::Text(t) => {
            t.runs.iter().all(|r| r.inline.is_none()) && n.appearance.items.iter().any(|i| crate::paints(i) && crate::has_geometry(i.effects()))
        }
        NodeKind::Image(_) => n.appearance.items.iter().any(shapes_image),
        _ => false,
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
    crate::paints(item) && item.effects().iter().any(gives_shape)
}

/// The fills and strokes of `items` (an image's own) that paint something ([`shapes_image`]), each
/// without the effects before its first Convert to Shape or Outline Object. Convert to Shape sizes
/// its shape from the image's box, and Outline Object gives the image's frame. The others paint
/// nothing.
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

/// The shapes that the image `n`'s own fills and strokes paint ([`image_rows`]), each with its fill
/// rule and the fill or stroke that paints it: the paths (or compound paths) along its frame that
/// [`reshape`] makes, with each fill's or stroke's geometry effects applied as the renderer applies
/// them. Empty for anything but an image with such fills or strokes. The swatches and patterns that
/// live-effect plug-ins painting their pieces name are looked up in `doc`.
pub fn image_paint(doc: Option<&Document>, n: &Node) -> Vec<(BezPath, FillRule, AppearanceItem)> {
    if !matches!(n.kind, NodeKind::Image(_)) || !has_item_geometry(n) {
        return vec![];
    }
    let Some(art) = reshape(doc, n, None) else { return vec![] };
    let mut out = vec![];
    art.walk(&mut |c| {
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
    });
    out
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
/// the instance's transform (through the symbol's 9-slice guides in `doc`, when it has them).
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
        _ if has_transform_art(n) || (apart && has_transform_copies(n)) => {
            let mut one = n.clone();
            one.opacity = 1.0;
            one.blend = Default::default();
            one.isolate = false;
            one.extras_mut().mask = None;
            one.appearance.effects.clear();
            Node::group(n.id, vec![Arc::new(one)])
        }
        _ if vectorcraft_doc::live::is_live(n) => evaluated_with(symbols, n, outline, nodes),
        _ => return None,
    };
    outline_nested_text(paint_doc, &mut art, false);
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
/// fills and strokes that paint a shape ([`image_rows`]) become paths along its frame below and
/// above it, reshaped with it, and keep their effects from their first Convert to Shape or Outline
/// Object on. The result is a group with `n`'s id, name, visibility, transparency, opacity mask
/// and remaining (raster) effects; `None` when `n` has no visible geometry effect, no perspective
/// and is not type or an image with a fill or stroke of its own that [`has_item_geometry`] counts,
/// or has no art (a symbol instance without `symbol_art`). A path with [`has_transform_art`] is
/// always reshaped. The fills and strokes in the art whose own Transform effects reach their
/// appearance become objects of their own ([`transform_items`]).
pub fn reshape(doc: Option<&Document>, n: &Node, symbol_art: Option<&Node>) -> Option<Node> {
    if !has_transform_art(n) && ((geometry_effects(n).is_empty() && n.projection().is_none() && !has_item_geometry(n)) || !needs_outline(n)) {
        return None;
    }
    as_art(doc, n, symbol_art)
}

/// `n` (see [`needs_outline`], or a path with [`has_transform_art`]) as plain art for Expand
/// Appearance: [`reshape`]d when it has
/// geometry effects, else type with fills or strokes of its own outlined, or an image with fills
/// or strokes of its own that paint a shape, in the same form. `None` when it has none of these.
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
            let image = Node::new(n.id, n.kind.clone());
            let clipped = || {
                let mut clip = Node::path(n.id, frame.clone(), Appearance::default());
                if let NodeKind::Path { clipping, .. } = &mut clip.kind {
                    *clipping = true;
                }
                Node::new(n.id, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(image.clone())], clip: true })
            };
            // The fills and strokes that paint a shape, as paths along the frame below and above
            // the image.
            let (below, above) = n.appearance.split_contents();
            let (below, above) = (image_rows(below), image_rows(above));
            if below.is_empty() && above.is_empty() {
                clipped()
            } else {
                let rows = |items: Vec<AppearanceItem>| {
                    (!items.is_empty()).then(|| Arc::new(Node::path(n.id, frame.clone(), Appearance { items, ..Default::default() })))
                };
                // Without geometry effects, the image is not clipped.
                let image = if fx.is_empty() { image.clone() } else { clipped() };
                Node::group(n.id, rows(below).into_iter().chain([Arc::new(image)]).chain(rows(above)).collect())
            }
        }
        _ => outline_art_with(symbols, paint_doc, n, symbol_art, outline, nodes, apart)?,
    };
    reshape_leaves_apart(&mut art, &fx, Budget::EFFECTS, apart, paint_doc);
    if apart {
        hoist_copies(&mut art);
    }
    transform_items(&mut art, paint_doc);
    // Type and symbols in perspective: the art projected onto their plane.
    if let Some(h) = n.projection() {
        project_leaves(&mut art, &h);
    }
    // The art is a group with `n`'s id (for an image without fills or strokes that paint, a clip
    // group).
    let mut out = art;
    out.name = n.name.clone();
    out.visible = n.visible;
    out.opacity = n.opacity;
    out.blend = n.blend;
    out.isolate = n.isolate;
    // One element: its pieces never knock each other out.
    out.knockout = vectorcraft_doc::Knockout::Off;
    out.knockout_shape = n.knockout_shape;
    out.extras_mut().mask = n.extras().mask.clone();
    out.appearance.effects = n.appearance.effects.iter().filter(|e| !is_geometry(&e.id)).cloned().collect();
    Some(out)
}

/// Map every path under `art` (and its gradients) through the projective map `h`, curves split
/// finely enough to follow it.
fn project_leaves(art: &mut Node, h: &vectorcraft_geom::Homography) {
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
/// symbol instances follow a Transform effect the same way ([`transform_placed`]); other geometry
/// effects leave them as they are (a raster follows a non-affine map only as a mesh warp). The
/// paths keep to one [`Budget::EFFECTS`] together (or to what they cost, when that is more): a path
/// whose effect would pass what the others leave of it keeps its shape. The swatches and patterns
/// that live-effect plug-ins painting their pieces name are looked up in `doc`
/// ([`reshape_leaves_apart`]).
pub(crate) fn reshape_leaves(art: &mut Node, fx: &[&Effect], doc: Option<&Document>) {
    reshape_leaves_within(art, fx, Budget::EFFECTS, doc);
}

/// [`reshape_leaves`], and how gradients that paint the art as a whole (a group's own fills and
/// strokes) follow the effects ([`crate::Carry`]).
pub(crate) fn reshape_leaves_carrying(art: &mut Node, fx: &[&Effect], doc: Option<&Document>) -> crate::Carry {
    reshape_leaves_within(art, fx, Budget::EFFECTS, doc)
}

/// [`reshape_leaves_carrying`] whose paths keep to `budget` together, or to what they cost.
pub(crate) fn reshape_leaves_within(art: &mut Node, fx: &[&Effect], budget: Budget, doc: Option<&Document>) -> crate::Carry {
    reshape_leaves_apart(art, fx, budget, false, doc)
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

/// [`reshape_leaves_within`]; with `apart`, a Transform effect's copies of each path are objects
/// of their own ([`copy_leaf`]) whatever paints them, as Expand Appearance makes them. A live-effect
/// plug-in that paints what it returns makes each path a group of its pieces ([`paint_leaf`]),
/// whose swatches and patterns must be in `doc` (without one, pieces that name one are refused).
/// Returns how gradients that paint the art as a whole follow the effects.
fn reshape_leaves_apart(art: &mut Node, fx: &[&Effect], budget: Budget, apart: bool, doc: Option<&Document>) -> crate::Carry {
    let mut whole = crate::Carry::default();
    if fx.is_empty() {
        return whole;
    }
    let mut leaves_safe = true;
    for_each_leaf(art, &mut |leaf| {
        if !leaves_safe {
            return;
        }
        if let Some((_, rule)) = leaf_geometry(leaf) {
            let has_stroke = leaf.appearance.stroke().is_some_and(|s| !s.paint.is_none() && s.width > 0.0);
            if rule != FillRule::NonZero || has_stroke {
                leaves_safe = false;
            }
        }
    });
    let art_stroke = art.appearance.stroke().is_some_and(|s| !s.paint.is_none() && s.width > 0.0);
    let style_safe = leaves_safe && !art_stroke;
    let opt_fx = crate::geometry::optimize_geometry_effects(fx, style_safe);
    let fx = opt_fx.as_slice();
    let seed = art.id.0;
    let mut used = art_cost(art);
    let max = budget.at_least(used);
    for e in fx {
        let Some(bounds) = art.geometric_bounds() else { break };
        let params = merged_params(&e.id, &e.params);
        if let Some(plugin) = painted_plugin(e) {
            for_each_leaf(art, &mut |leaf| {
                let Some((path, _)) = leaf_geometry(leaf) else { return };
                let before = Budget::of(&path);
                let room = max.minus(used.minus(before));
                if let Some(made) = paint_leaf(leaf, plugin, &params, bounds, room, doc) {
                    used = used.minus(before).plus(made);
                }
            });
            continue;
        }
        // Placed gradients follow a Transform effect and Free Distort ([`crate::Carry`]): under
        // Free Distort (`boxes`) each path's move and scale from its bounds before the effect to
        // its bounds after it. Under that and the other effects, the placed gradients of type's
        // outlines in `art` (`outline_text`) move and scale with the outlines' bounds.
        let follows = crate::Carry::follows(&e.id, &params, bounds, seed);
        let boxes = follows && e.id == "distort.freeDistort";
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
            group_marks(art, &mut vec![], &HashSet::new(), &mut groups);
        }
        // The ids of the paths that kept their shape (past the budget).
        let mut kept_ids = HashSet::new();
        let transform = (e.id == "distort.transform").then(|| {
            let (m, copies) = crate::distort::transform_map(bounds, &params, seed);
            let options = crate::distort::TransformOptions::of(&params);
            (m, copies, options, crate::distort::transform_affine(bounds, &params, seed))
        });
        for_each_leaf(art, &mut |leaf| {
            let Some((path, rule)) = leaf_geometry(leaf) else { return };
            // What the other paths leave of the budget.
            let before = Budget::of(&path);
            let room = max.minus(used.minus(before));
            if let Some((m, copies, options, map)) = transform {
                // Transform Patterns and Scale Strokes & Effects change each copy's fills and
                // strokes; a gradient paints each copy by itself, not the one piece the copies
                // make together; and Expand Appearance (`apart`) makes each copy an object.
                // Fills and strokes that stack paint each copy above the one before (#5920).
                let separate = copies > 0
                    && (apart
                        || leaf.appearance.has_unplaced_gradient()
                        || leaf.appearance.has_placed_gradient()
                        || paints_copies_apart(&leaf.appearance, rule));
                if options.reach_appearance() {
                    let (made, moved) = transform_leaf(leaf, &path, map, copies, options, room);
                    used = used.minus(before).plus(made);
                    if !moved {
                        kept_ids.insert(leaf.id);
                    }
                    return;
                }
                if separate {
                    let made = copy_leaf(leaf, &path, m, copies, vectorcraft_doc::Scaling::default(), room);
                    used = used.minus(before).plus(made);
                    return;
                }
            }
            let Some(out) = apply_one(&e.id, &params, &path, bounds, &GeomContext { seed, ..GeomContext::of(leaf) }, room) else {
                kept_ids.insert(leaf.id);
                return;
            };
            used = used.minus(before).plus(Budget::of(&out));
            if follows && !boxes && leaf.appearance.has_positioned_gradient() {
                crate::Carry::of(&e.id, &params, bounds, None, None, seed).apply(&mut leaf.appearance);
            }
            set_geometry(leaf, out, rule);
        });
        if boxes {
            rebase_leaves(art, &mut leaves.into_iter());
        }
        if let Some((m, copies, options, _)) = transform
            && options.objects
        {
            transform_placed(art, m, copies);
        }
        if outlines.iter().any(Option::is_some) {
            rebase_type_outlines(art, &mut outlines.into_iter());
        }
        // What paints the art as a whole, and the own fills and strokes of the groups inside it,
        // follow their bounds under Free Distort (they stay when no path changed them). Each stays
        // when a path under it that its bounds measure kept its shape (past the budget).
        let mut after = vec![];
        let held = (!groups.is_empty() || !kept_ids.is_empty()) && group_marks(art, &mut vec![], &kept_ids, &mut after).1;
        if !groups.is_empty() {
            carry_groups(art, groups, after, &|from, to| crate::Carry::of(&e.id, &params, bounds, from, to, seed));
        }
        if follows && !held {
            whole.push(crate::Carry::of(&e.id, &params, bounds, Some(bounds), art.geometric_bounds(), seed));
        }
    }
    whole
}

/// Largest factor a Transform effect's Scale Strokes & Effects scales one object's strokes by
/// (its copies stop before one would take more), so stroke weights stay finite where the geometry
/// doesn't grow with them (a horizontal line stretched vertically).
const MAX_STROKE_SCALE: f64 = 1.0e6;

/// A Transform effect whose Transform Patterns or Scale Strokes & Effects is on (`options`), with
/// map `m` and `copies`, applied to `leaf` (a path or compound path with geometry `path`) as Object
/// › Transform with those options transforms an object (#885): the strokes' weights and dashes
/// and the distances of the effects on its fills and strokes scale by `m`'s mean scale, and its
/// pattern paints transform by `m`. With copies, the original keeps them and copy `k` takes them
/// `k` times, each copy an object of its own ([`copy_leaf`], within `budget`). With Transform
/// Objects off the geometry stays and the options apply once. Returns what the leaf costs after,
/// and `false` when the path kept its shape because the moved path would pass `budget`.
fn transform_leaf(
    leaf: &mut Node,
    path: &PathData,
    m: Affine,
    copies: usize,
    options: crate::distort::TransformOptions,
    budget: Budget,
) -> (Budget, bool) {
    let scaling = vectorcraft_doc::Scaling {
        strokes: options.strokes,
        effects: options.strokes.then_some(crate::scale_effect as fn(&mut Effect, f64)),
        patterns: options.patterns,
        ..Default::default()
    };
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
        return (Budget::of(path), true);
    }
    if copies == 0 {
        // A path scaled up is drawn with more lines; past the budget it keeps its shape.
        let moved = path.transformed(m);
        if !budget.fits(Budget::of(&moved)) {
            return (Budget::of(path), false);
        }
        leaf.transform(m, scaling);
        return (Budget::of(&moved), true);
    }
    (copy_leaf(leaf, path, m, copies, scaling, budget), true)
}

/// Replace `leaf` (a path or compound path with geometry `path`) by a group of it and `copies`
/// more, each mapped once more by `m` than the last (with `scaling`), as a Transform effect copies
/// an object. Each copy is an object of its own, so its gradients fit it (a placed gradient moves
/// with it) rather than spanning every copy, and its strokes and patterns can differ from the
/// others'. The copies stop where [`crate::distort::transform_within`]'s would, before they pass
/// `budget`, and before one's strokes would scale past [`MAX_STROKE_SCALE`]. Returns what the
/// copies cost, the original included.
fn copy_leaf(leaf: &mut Node, path: &PathData, m: Affine, copies: usize, scaling: vectorcraft_doc::Scaling, budget: Budget) -> Budget {
    let mut made = Budget::of(path);
    let mut placements: Vec<Arc<Node>> = vec![Arc::new(leaf.clone())];
    let mut xf = Affine::IDENTITY;
    for _ in 0..copies {
        xf *= m;
        // A copy scaled up is drawn with more lines.
        let cost = Budget::of(&path.transformed(xf));
        if !budget.fits(made.plus(cost)) {
            break;
        }
        if scaling.strokes && vectorcraft_doc::Scaling::factor(xf).is_some_and(|k| !(k.is_finite() && k <= MAX_STROKE_SCALE)) {
            break;
        }
        let mut copy = leaf.clone();
        copy.transform(xf, scaling);
        if !copy.geometric_bounds().is_some_and(crate::distort::within_reach) {
            break;
        }
        placements.push(Arc::new(copy));
        made = made.plus(cost);
    }
    // The copies become nodes of their own under a group with the object's id and name.
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
/// refused, or they would pass `budget`.
fn paint_leaf(leaf: &mut Node, plugin: &str, params: &serde_json::Value, bounds: Rect, budget: Budget, doc: Option<&Document>) -> Option<Budget> {
    let (path, rule) = leaf_geometry(leaf)?;
    let paints = vectorcraft_plugins::effect::Paints { appearance: Some(&leaf.appearance), item: None };
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
    let max = vectorcraft_plugins::objects::MAX_OBJECTS;
    Budget::of(path).plus(Budget { anchors: Budget::EFFECTS.anchors / max, lines: Budget::EFFECTS.lines / max })
}

/// Live-effect plug-in `plugin` (one that paints what it returns) with `params`, a later effect of
/// `item`, run on `leaf`, a path of [`transform_piece`] painted by copies of `item`, with reference
/// box `bounds`: `leaf` becomes a group of one path for each piece returned, in order. A piece that
/// gives fills (on a fill) or strokes (on a stroke) is painted by copies of `item`, the leaf's first
/// fill or stroke, with those paints ([`vectorcraft_plugins::painted::items`]), under the leaf's
/// other fills and strokes; one that gives none keeps the leaf's. Returns
/// what the paths cost ([`piece_cost`]); `None` (the leaf stays as it is) when the plug-in fails,
/// the pieces are refused (a swatch or pattern not in `doc`, or any without one), or they would pass
/// `budget`.
fn paint_piece(
    leaf: &mut Node,
    item: &AppearanceItem,
    plugin: &str,
    params: &serde_json::Value,
    bounds: Rect,
    budget: Budget,
    doc: Option<&Document>,
) -> Option<Budget> {
    let (path, rule) = leaf_geometry(leaf)?;
    let paints = vectorcraft_plugins::effect::Paints { appearance: Some(&leaf.appearance), item: Some(0) };
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
/// stroke's weight and dashes, the distances of its remaining effects and its pattern follow the
/// map. The reference box is that geometry's bounds and the seed is the leaf's id, as the renderer
/// draws a fill's or stroke's own effects. The geometry effects after the Transform effect then
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
        if let Some(plugin) = painted_plugin(&fx[at]) {
            let mut it = item.clone();
            let rest: Vec<Effect> = before.iter().filter(|e| !is_geometry(&e.id)).chain(&fx[at + 1..]).cloned().collect();
            *it.effects_mut() = rest;
            let mut one = Appearance { items: vec![it.clone()], ..Default::default() };
            if carry.moves(&one) {
                carry.apply(&mut one);
            }
            let it = one.items.pop().unwrap_or(it);
            let paints = vectorcraft_plugins::effect::Paints { appearance: Some(&original.appearance), item: Some(i) };
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
            transform_piece(&mut p, ctx.seed, limit, used, doc);
            pieces.push(Arc::new(p));
            continue;
        }
        let (_, copies) = crate::distort::transform_map(tb, &params, ctx.seed);
        let options = crate::distort::TransformOptions::of(&params);
        let map = crate::distort::transform_affine(tb, &params, ctx.seed);
        let mut it = item.clone();
        let rest: Vec<Effect> = before.iter().filter(|e| !is_geometry(&e.id)).chain(&fx[at + 1..]).cloned().collect();
        *it.effects_mut() = rest;
        let mut p = piece(vec![it]);
        if carry.moves(&p.appearance) {
            carry.apply(&mut p.appearance);
        }
        set_geometry(&mut p, g.clone(), rule);
        let (made, _) = transform_leaf(&mut p, &g, map, copies, options, room);
        *used = used.plus(made);
        transform_piece(&mut p, ctx.seed, limit, used, doc);
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

/// Apply the visible geometry effects left on the one fill or stroke of each path under `p` (a
/// transformed piece of [`transform_item_leaf`] and its copies) to all of `p`'s paths as one piece:
/// each effect takes the bounds of all the paths as its reference box and `seed`, as an object's
/// own effects reshape all its paths ([`reshape_leaves_within`]). Each path applies its own copy of
/// the effect, whose distances its Scale Strokes & Effects scaled. A Transform effect with
/// Transform Patterns or Scale Strokes & Effects on transforms each path as [`transform_leaf`] does.
/// A live-effect plug-in that paints what it returns ([`painted_plugin`]) makes each path one path
/// for each piece it returns ([`paint_piece`]). The geometry effects then leave the fills and
/// strokes; their other effects stay. The paths keep to `max` together with the rest of the art
/// (`used`), and the swatches and patterns painted pieces name must be in `doc`.
fn transform_piece(p: &mut Node, seed: u64, max: Budget, used: &mut Budget, doc: Option<&Document>) {
    let mut count = 0;
    for_each_leaf(p, &mut |l| {
        if let Some(item) = l.appearance.items.first() {
            count = count.max(item.effects().len());
        }
    });
    for j in 0..count {
        let Some(bounds) = p.geometric_bounds() else { break };
        for_each_leaf(p, &mut |leaf| {
            let Some(item) = leaf.appearance.items.first().cloned() else { return };
            let Some(e) = item.effects().get(j).filter(|e| e.visible && is_geometry(&e.id)).cloned() else { return };
            let Some((path, rule)) = leaf_geometry(leaf) else { return };
            // What the other paths leave of the budget.
            let before = Budget::of(&path);
            let room = max.minus(used.minus(before));
            if let Some(plugin) = painted_plugin(&e) {
                if let Some(made) = paint_piece(leaf, &item, plugin, &merged_params(&e.id, &e.params), bounds, room, doc) {
                    *used = used.minus(before).plus(made);
                }
                return;
            }
            if reaches_appearance(&e) {
                let params = merged_params(&e.id, &e.params);
                let (_, copies) = crate::distort::transform_map(bounds, &params, seed);
                let options = crate::distort::TransformOptions::of(&params);
                let map = crate::distort::transform_affine(bounds, &params, seed);
                let (made, _) = transform_leaf(leaf, &path, map, copies, options, room);
                *used = used.minus(before).plus(made);
                return;
            }
            let appearance = leaf.appearance.clone();
            let stroke = match &item {
                AppearanceItem::Stroke(s) => Some(s),
                AppearanceItem::Fill(_) => None,
            };
            let ctx = GeomContext { stroke, rule, seed, appearance: Some(&appearance), item: Some(0), doc };
            let mut carry = crate::Carry::default();
            let out = crate::geometry::apply_geometry_carrying_within(std::slice::from_ref(&e), &path, bounds, &ctx, room, Some(&mut carry));
            *used = used.minus(before).plus(Budget::of(&out));
            // The fill's or stroke's gradient follows the effect as the object's does.
            if carry.moves(&leaf.appearance) {
                carry.apply(&mut leaf.appearance);
            }
            set_geometry(leaf, out, rule);
        });
    }
    for_each_leaf(p, &mut |leaf| {
        for item in &mut leaf.appearance.items {
            item.effects_mut().retain(|e| !(e.visible && is_geometry(&e.id)));
        }
    });
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
/// placed gradient: the child indices that lead to it from the art, its geometric bounds, and
/// whether a path below it that its bounds measure kept its shape.
type GroupMark = (Vec<usize>, Option<Rect>, bool);

/// The geometric bounds of `n` (as [`Node::geometric_bounds`] gives them) and whether a path or
/// compound path below it that those bounds measure is in `kept`, measured in one walk. Each group below `n` that
/// [`GroupMark`] describes goes into `out`, reached from `n` through `at`.
fn group_marks(n: &Node, at: &mut Vec<usize>, kept: &HashSet<NodeId>, out: &mut Vec<GroupMark>) -> (Option<Rect>, bool) {
    match &n.kind {
        NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => (n.geometric_bounds(), kept.contains(&n.id)),
        NodeKind::Group { children, clip } | NodeKind::Layer { children, clip, .. } if !n.extras().type_outlines => {
            let layer = matches!(n.kind, NodeKind::Layer { .. });
            let skip = children.len().saturating_sub(n.editing_children().len());
            let (mut bounds, mut held) = (None, false);
            for (i, c) in children.iter().enumerate() {
                at.push(i);
                let (b, h) = group_marks(c, at, kept, out);
                if is_member_group(c) && c.appearance.has_positioned_gradient() {
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
/// as a path does.
fn transform_placed(n: &mut Node, m: Affine, copies: usize) {
    transform_placed_within(n, m, copies, crate::distort::MAX_COPY_OBJECTS)
}

/// [`transform_placed`] whose copies make at most `max` image and symbol instance nodes of the art,
/// the originals included: each copy adds the object once more, and a Transform after this one
/// copies the lot again, so copies stop where the result would pass `max` (#2042).
pub(crate) fn transform_placed_within(n: &mut Node, m: Affine, copies: usize, max: usize) {
    let mut room = max.saturating_sub(placed_count(n));
    copy_placed(n, m, copies, &mut room);
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
fn copy_placed(n: &mut Node, m: Affine, copies: usize, room: &mut usize) {
    let Some(children) = n.children_mut() else { return };
    for c in children.iter_mut() {
        if !is_placed(c) {
            copy_placed(Arc::make_mut(c), m, copies, room);
            continue;
        }
        let src = (**c).clone();
        let placed = |xf: Affine| {
            let mut one = src.clone();
            match &mut one.kind {
                NodeKind::Image(im) => im.xf = xf * im.xf,
                NodeKind::SymbolInstance { xf: at, .. } => *at = xf * *at,
                _ => {}
            }
            // The placed gradients of its own fills and strokes move with it, as a path's do.
            if xf != Affine::IDENTITY && one.appearance.has_positioned_gradient() {
                one.appearance.transform_gradients(xf);
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
