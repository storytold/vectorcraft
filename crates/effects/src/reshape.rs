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

use std::sync::Arc;

use vectorcraft_doc::live::Hooks;
use vectorcraft_doc::{Appearance, AppearanceItem, Document, Effect, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Rect, Shape, shapes};

use crate::{Budget, GeomContext, apply_one, is_geometry, merged_params};

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
    matches!(n.kind, NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. })
        && (n.appearance.has_unplaced_gradient() || n.appearance.has_placed_gradient())
        && geometry_effects(n)
            .iter()
            .any(|e| e.id == "distort.transform" && crate::distort::transform_map(Rect::ZERO, &merged_params(&e.id, &e.params), 0).1 > 0)
}

/// Is `n` a path or compound path with a visible Transform effect whose Transform Patterns or
/// Scale Strokes & Effects is on (#885)? Such an effect changes the fills and strokes as well as
/// the geometry, so the object is drawn through its art ([`outline_art`]), each copy an object of
/// its own with its strokes scaled and its patterns moved ([`transform_leaf`]).
pub fn has_transform_options(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Path { guide: false, clipping: false, .. } | NodeKind::Compound { .. })
        && geometry_effects(n)
            .iter()
            // Both options default to off, so the stored params tell without merging the defaults
            // (this runs for every object drawn).
            .any(|e| e.id == "distort.transform" && crate::distort::TransformOptions::of(&e.params).reach_appearance())
}

/// Is `n` a path or compound path that its Transform effects make into art of separate objects
/// ([`has_gradient_copies`], [`has_transform_options`])? It is drawn, baked and expanded through
/// that art ([`reshape`]).
pub fn has_transform_art(n: &Node) -> bool {
    has_gradient_copies(n) || has_transform_options(n)
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
/// them. Empty for anything but an image with such fills or strokes.
pub fn image_paint(n: &Node) -> Vec<(BezPath, FillRule, AppearanceItem)> {
    if !matches!(n.kind, NodeKind::Image(_)) || !has_item_geometry(n) {
        return vec![];
    }
    let Some(art) = reshape(None, n, None) else { return vec![] };
    let mut out = vec![];
    art.walk(&mut |c| {
        let (NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. }) = &c.kind else { return };
        let Some(g) = c.stroke_path() else { return };
        let ctx = GeomContext::of(c);
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
/// the instance's transform. `None` for other kinds (images, paths, containers). A Repeat counts
/// the Repeats in the art of `doc`'s symbols that its instances show toward its instance budget.
pub fn outline_art(doc: Option<&Document>, n: &Node, symbol_art: Option<&Node>) -> Option<Node> {
    let mut art = match &n.kind {
        NodeKind::Text(_) => outline_text(n)?,
        NodeKind::SymbolInstance { xf, .. } => {
            let mut a = symbol_art?.clone();
            a.transform(*xf, false);
            Node::group(n.id, vec![Arc::new(a)])
        }
        // The object as the one piece the copies of its Transform effect are made from.
        _ if has_transform_art(n) => {
            let mut one = n.clone();
            one.opacity = 1.0;
            one.blend = Default::default();
            one.isolate = false;
            one.extras_mut().mask = None;
            one.appearance.effects.clear();
            Node::group(n.id, vec![Arc::new(one)])
        }
        _ if vectorcraft_doc::live::is_live(n) => evaluated(doc, n),
        _ => return None,
    };
    outline_nested_text(&mut art, false);
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
    let hook: &dyn Fn(&Node) -> Option<Node> = &outline_text;
    vectorcraft_doc::live::expand_deep_hooks(n, Hooks { outline: Some(hook), symbols: doc, ..Default::default() })
}

/// Replace type anywhere inside `n` by its outlines, as art that keeps the type's look
/// ([`as_art`]). Type in a clip group's clipping path (all of `n` when `clipping` is set) becomes
/// its plain outlines ([`outline_text`]) with its visibility, which is what
/// [`crate::clip_outline`] clips by.
pub(crate) fn outline_nested_text(n: &mut Node, clipping: bool) {
    let clips = n.clips();
    let Some(ch) = n.children_mut() else { return };
    for (i, c) in ch.iter_mut().enumerate() {
        let clipping = clipping || (clips && i == 0);
        let art = match &c.kind {
            NodeKind::Text(_) if clipping => outline_text(c).map(|o| Node { visible: c.visible, ..o }),
            NodeKind::Text(_) => as_art(None, c, None),
            _ => None,
        };
        if let Some(o) = art {
            *c = Arc::new(o);
        } else if c.children().is_some() {
            outline_nested_text(Arc::make_mut(c), clipping);
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
/// or has no art (a symbol instance without `symbol_art`).
pub fn reshape(doc: Option<&Document>, n: &Node, symbol_art: Option<&Node>) -> Option<Node> {
    if (geometry_effects(n).is_empty() && n.projection().is_none() && !has_item_geometry(n)) || !(needs_outline(n) || has_transform_art(n)) {
        return None;
    }
    as_art(doc, n, symbol_art)
}

/// `n` (see [`needs_outline`], or a path with [`has_transform_art`]) as plain art for Expand
/// Appearance: [`reshape`]d when it has
/// geometry effects, else type with fills or strokes of its own outlined, or an image with fills
/// or strokes of its own that paint a shape, in the same form. `None` when it has none of these.
pub fn expand_outlined(doc: Option<&Document>, n: &Node, symbol_art: Option<&Node>) -> Option<Node> {
    // A path whose Transform effect copies it with a gradient, or scales its strokes or moves its
    // patterns: each copy an object of its own.
    if has_transform_art(n) {
        return as_art(doc, n, symbol_art);
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
        _ => outline_art(doc, n, symbol_art)?,
    };
    reshape_leaves(&mut art, &fx);
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
/// whose effect would pass what the others leave of it keeps its shape.
pub(crate) fn reshape_leaves(art: &mut Node, fx: &[&Effect]) {
    reshape_leaves_within(art, fx, Budget::EFFECTS);
}

/// [`reshape_leaves`] whose paths keep to `budget` together, or to what they cost.
pub(crate) fn reshape_leaves_within(art: &mut Node, fx: &[&Effect], budget: Budget) {
    if fx.is_empty() {
        return;
    }
    let seed = art.id.0;
    // The placed gradients of type's outlines in `art` (`outline_text`) move and scale with the
    // pieces, from the outlines' bounds before the effects to their bounds after them.
    let mut outlines = vec![];
    type_outline_bounds(art, &mut outlines);
    let mut used = art_cost(art);
    let max = budget.at_least(used);
    for e in fx {
        let Some(bounds) = art.geometric_bounds() else { break };
        let params = merged_params(&e.id, &e.params);
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
                // make together.
                let gradient = copies > 0 && (leaf.appearance.has_unplaced_gradient() || leaf.appearance.has_placed_gradient());
                if options.reach_appearance() {
                    let made = transform_leaf(leaf, &path, map, copies, options, room);
                    used = used.minus(before).plus(made);
                    return;
                }
                if gradient {
                    let made = copy_leaf(leaf, &path, m, copies, vectorcraft_doc::Scaling::default(), room);
                    used = used.minus(before).plus(made);
                    return;
                }
            }
            let Some(out) = apply_one(&e.id, &params, &path, bounds, &GeomContext { seed, ..GeomContext::of(leaf) }, room) else { return };
            used = used.minus(before).plus(Budget::of(&out));
            set_geometry(leaf, out, rule);
        });
        if let Some((m, copies, options, _)) = transform
            && options.objects
        {
            transform_placed(art, m, copies);
        }
    }
    if outlines.iter().any(Option::is_some) {
        rebase_type_outlines(art, &mut outlines.into_iter());
    }
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
/// Objects off the geometry stays and the options apply once. Returns what the leaf costs after.
fn transform_leaf(leaf: &mut Node, path: &PathData, m: Affine, copies: usize, options: crate::distort::TransformOptions, budget: Budget) -> Budget {
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
        return Budget::of(path);
    }
    if copies == 0 {
        // A path scaled up is drawn with more lines; past the budget it keeps its shape.
        let moved = path.transformed(m);
        if !budget.fits(Budget::of(&moved)) {
            return Budget::of(path);
        }
        leaf.transform(m, scaling);
        return Budget::of(&moved);
    }
    copy_leaf(leaf, path, m, copies, scaling, budget)
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
            if leaf.appearance.has_placed_gradient() {
                follow_boxes(&mut leaf.appearance, from, to);
            }
        });
    }
}

/// Map the placed gradients of `ap` from box `from` to box `to` through the map that takes one box
/// onto the other (strokes: their stroke-inflated boxes). The start and end go where
/// [`Appearance::rebase_gradients`] puts them, and a radial's ellipse and a linear's isolines are
/// mapped with them.
fn follow_boxes(ap: &mut Appearance, from: Rect, to: Rect) {
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
    let mut art = evaluated(doc, n);
    outline_nested_text(&mut art, false);
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
