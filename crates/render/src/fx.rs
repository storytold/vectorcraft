//! Live effects in the renderer.
//!
//! Geometry effects (object-level, then per fill/stroke) rewrite the path before painting via
//! `vectorcraft-effects`. Raster effects use vello_cpu filter layers. Those only work in
//! single-threaded contexts, so on the multithreaded pipeline each filter layer is rendered
//! offscreen on the calling thread (cropped to its reach) and composited back with the same blend
//! mode and opacity ([`Renderer::with_filters`]):
//!
//! - Drop Shadow / Outer Glow: the object's silhouette in a `DropShadowOnly` filter layer, painted
//!   below the object with the effect's blend mode and opacity;
//! - Gaussian Blur: the object painted inside a Gaussian filter layer;
//! - Feather: like Gaussian Blur but clipped to the shape, so the edge fades inward;
//! - Inner Glow: a blurred inverse silhouette (Edge) or the blurred silhouette (Center), clipped
//!   to the shape and painted above it;
//! - Photoshop-style filters (Radial Blur, Smart Blur, Unsharp Mask…): the content is rendered
//!   offscreen and its pixels run through the content-changing effects in stack order (those
//!   filters, Gaussian Blur and Feather, see [`effects::pixel`]), then drawn back
//!   ([`Renderer::pixel_content`]). The result is cached per object while it and the zoom don't
//!   change.
//! - Rasterize: the content drawn again on the image's own grid, as Object › Rasterize draws it
//!   ([`Renderer::rasterize_image`]), then into the content's raster
//!   ([`effects::Rasterize::compose`]; an image too large for that is made from the content's
//!   raster, [`effects::Rasterize::apply`]), clipped to its outline with Create Clipping Mask. The
//!   effects before it in the stack, and the shadows and glows after it, are drawn around it by
//!   splitting the stack there ([`Renderer::raster_fx`]).
//!
//! While [`crate::RenderOptions::interacting`], an object whose content only moved draws its
//! cached shadows, glows and filtered content at its new place instead of running the effects
//! again ([`Renderer::move_shadow`], [`Renderer::move_pixel_content`]).
//!
//! While [`crate::RenderOptions::dragging`] is set, the renderer draws filtered content that runs
//! its effects within [`DRAG_PIXEL_BUDGET`] pixels, at a lower resolution where it would need more,
//! and keeps no raster made that way. On the multithreaded pipeline it does the same for shadows
//! and glows. The frame after the drag renders them as a still frame does.
//!
//! A cached raster is made again when something its content reads from outside the object
//! changes ([`Inputs`]): the pattern definitions, symbols, images and brushes it uses, the
//! document's raster resolution, color mode and substitution highlights, the knockout its object
//! inherits, the color settings and fonts, the plug-ins its plug-in effects run, and the options
//! and layer settings it is drawn with.
//!
//! The same applies to one fill or stroke's own raster effects (around that item alone) and to
//! type, images, symbol instances and live objects ([`Renderer::draw_object_fx`]): their art is
//! drawn offscreen in the filter layers, reshaped first by their geometry effects. Groups and
//! layers with an appearance of their own take the same path with their evaluated art
//! ([`effects::evaluate_container`]: their fills and strokes painting the members, their geometry
//! effects reshaping them), so their raster effects apply to the composite (one combined shadow).

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Weak};

use vectorcraft_color::cms::Cms;
use vectorcraft_doc::{AppearanceItem, Document, Effect, Node, NodeId, NodeKind, SymbolBoxes};
use vectorcraft_effects::pixel::{Channels, PixelFx};
use vectorcraft_effects::{self as effects, GeomContext, RasterFx};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathEl, Point, Rect, Shape, Size, Vec2};
use vello_common::filter_effects::{EdgeMode, Filter, FilterPrimitive};
use vello_cpu::RenderContext;
use vello_cpu::peniko;

use crate::ink::Ink;
use crate::{Frame, Renderer, blend_mode, fill_rule, paint};

fn visible(effects: &[Effect]) -> bool {
    effects.iter().any(|e| e.visible)
}

/// Does this node need the effects path (a path or compound path with visible effects)?
pub(crate) fn has_fx(n: &Node) -> bool {
    if !matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. }) {
        return false;
    }
    visible(&n.appearance.effects)
        || n.appearance.items.iter().any(|i| match i {
            AppearanceItem::Fill(f) => visible(&f.effects),
            AppearanceItem::Stroke(s) => visible(&s.effects),
        })
}

/// Does `n` carry visible object-level effects that only apply through its art (type, images,
/// symbol instances, live objects; Crop Marks and colour adjustments on anything), is it type with
/// a geometry effect on a fill or stroke of its own or an image with a fill or stroke of its own
/// that paints a shape ([`effects::has_item_geometry`]), is it a path whose Transform effect copies
/// it with a gradient or changes its fills and strokes ([`effects::has_transform_art`]), or is it a
/// group or layer with an appearance of its own (see [`Renderer::draw_object_fx`])?
pub(crate) fn has_object_fx(n: &Node) -> bool {
    (effects::needs_outline(n) && (visible(&n.appearance.effects) || n.projection().is_some()))
        || effects::has_item_geometry(n)
        || effects::has_transform_art(n)
        || effects::has_container_appearance(n)
        || effects::has_crop_marks(n)
        || effects::has_adjustment(n)
        || effects::has_revolve(n)
        || effects::has_solid(n)
}

/// Is `n` a group or layer (whose evaluated art keeps its knockout setting)?
fn is_container(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. })
}

/// Can the art [`Renderer::fx_art`] makes for `n` be made from live objects it evaluates? It can
/// when `n` is a live object, and when `n` is a group or layer without a Pathfinder effect that
/// holds live objects and whose geometry effects reshape them or whose own fills and strokes paint
/// their evaluated outlines ([`effects::evaluate_container`]).
fn evaluates_live(n: &Node) -> bool {
    use vectorcraft_doc::live::{is_live, subtree_has_live};
    let to_members = effects::has_geometry(&n.appearance.effects) || n.appearance.items.iter().any(effects::paints);
    is_live(n) || (is_container(n) && to_members && !effects::has_pathfinder(n) && subtree_has_live(n))
}

/// Can the art [`Renderer::fx_art`] makes for `n` change while `n` stays the same, other than
/// through live objects it evaluates? It can when `n` holds type (laid out in the fonts installed),
/// a symbol instance (made from its symbol's art), or an object with a 3D effect or a color
/// adjustment (shaded or recolored through the color settings), in it or in the opacity masks of
/// the objects in it.
fn reads_outside(n: &Node) -> bool {
    let mut reads = false;
    n.walk(&mut |c| {
        reads |= matches!(c.kind, NodeKind::Text(_) | NodeKind::SymbolInstance { .. })
            || effects::has_revolve(c)
            || effects::has_solid(c)
            || effects::has_adjustment(c)
            || c.extras().mask.as_ref().is_some_and(|m| reads_outside(&m.art))
    });
    reads
}

/// What [`Renderer::raster_fx`] paints effects around.
struct Content<'a> {
    /// The object the effects belong to (cached shadows are checked against it).
    node: &'a Node,
    /// The `Arc` holding `node`, when known: cached rasters keep it and match it by identity.
    owner: Option<&'a Arc<Node>>,
    /// First shadow-cache slot of these effects (`None`: don't cache).
    cache: Option<usize>,
    /// Document-space bounds of the painted content.
    reach: Rect,
    /// Its silhouette: Feather and Inner Glow clip to it (without one, Feather only blurs and
    /// Inner Glow is skipped).
    outline: Option<(&'a BezPath, FillRule)>,
    /// What Rasterize makes its image over: the object's [`drawn_bounds`] as far as the effects
    /// before it spread them (`None`: `reach`).
    bounds: Option<Rect>,
    /// Where the image of the Rasterize first in these effects is kept for the effects after a
    /// split ([`Renderer::raster_fx`]), which draw this content more than once.
    kept: Option<&'a Kept>,
}

/// The image of the last Rasterize before a split in an effect stack ([`Renderer::raster_fx`]). It
/// is made the first time the effects after the split draw the content and drawn from here the
/// other times, so the content before the split is drawn once.
struct Kept {
    /// Where the effects after the split take the content from: the visible area grown by how
    /// far they paint.
    window: Rect,
    /// The view and pixel size of the split's frame, when the image is over
    /// [`Renderer::rasterize_image`]'s limits: the content is then drawn in that frame.
    frame: Option<(Affine, f64)>,
    image: std::cell::RefCell<Option<KeptImage>>,
}

#[derive(Clone)]
enum KeptImage {
    /// The image on its own grid, as [`Renderer::rasterize_image`] makes it, and how many images
    /// in the content failed to decode while it was made ([`crate::paint::decode_failures`]):
    /// each draw of the image from here counts them again ([`crate::paint::add_decode_failures`]).
    Grid(std::rc::Rc<(Vec<u8>, usize, usize)>, u64),
    /// An image over [`Renderer::rasterize_image`]'s limits: the content's raster with the
    /// Rasterize applied, as the first draw made it, and the transform from its pixels to
    /// document space.
    Raster(Arc<vello_cpu::Pixmap>, Affine),
}

/// The [`Content::bounds`] of `n` under raster effects `rfx`: its [`drawn_bounds`] when `rfx`
/// holds a Rasterize.
fn rasterize_bounds(symbols: &SymbolBoxes, n: &Node, rfx: &[RasterFx]) -> Option<Rect> {
    rfx.iter().any(|fx| matches!(fx, RasterFx::Rasterize(_))).then(|| drawn_bounds(symbols, n)).flatten()
}

/// `f` with the content moved by (`dx`, `dy`) document units (a shadow's offset).
fn shifted<'a>(f: &Frame<'a>, dx: f64, dy: f64) -> Frame<'a> {
    Frame { view: f.view * Affine::translate((dx, dy)), visible: f.visible - Vec2::new(dx, dy), ..*f }
}

fn node_bezpath(n: &Node) -> Option<BezPath> {
    match &n.kind {
        NodeKind::Path { path, .. } => Some(path.to_bezpath()),
        NodeKind::Compound { children, .. } => {
            let mut bp = BezPath::new();
            for c in children {
                if let Some(p) = c.path_data() {
                    bp.extend(p.to_bezpath());
                }
            }
            Some(bp)
        }
        _ => None,
    }
}

fn apply(effects: &[Effect], bp: &BezPath, ctx: &GeomContext) -> BezPath {
    if !effects::has_geometry(effects) || bp.elements().is_empty() {
        return bp.clone();
    }
    effects::apply_geometry_bez(effects, bp, bp.bounding_box(), ctx)
}

/// The object-level effected geometry of `n` (base path `bp`).
pub(crate) fn effected_path(n: &Node, bp: &BezPath) -> BezPath {
    apply(&n.appearance.effects, bp, &GeomContext::of(n))
}

fn item_effects(item: &AppearanceItem) -> &[Effect] {
    match item {
        AppearanceItem::Fill(f) => &f.effects,
        AppearanceItem::Stroke(s) => &s.effects,
    }
}

/// Visual bounds including geometry effects, stroke outsets and shadows/glows (symbol instances
/// measured by their art from `symbols`).
pub(crate) fn visual_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    if let Some(art) = effects::revolve_art(n).or_else(|| effects::solid_art(n)) {
        return cull_bounds(symbols, &art).map(|b| {
            let o = outset(symbols, &n.appearance.effects, b);
            b.inflate(o, o)
        });
    }
    let bp = node_bezpath(n)?;
    let ctx = GeomContext::of(n);
    let g = effected_path(n, &bp);
    let mut r: Option<Rect> = (!g.elements().is_empty()).then(|| g.bounding_box());
    for item in &n.appearance.items {
        let fx = item_effects(item);
        if effects::has_geometry(fx) {
            let ig = apply(fx, &g, &ctx.item(item));
            if !ig.elements().is_empty() {
                let b = ig.bounding_box();
                r = Some(r.map_or(b, |r| r.union(b)));
            }
        }
    }
    let s = n.appearance.outset();
    r.map(|r| {
        let r = r.inflate(s, s);
        // The fills' and strokes' own raster effects, then the object's.
        let r = n.appearance.items.iter().filter(|i| paints(i)).map(|i| spread(symbols, item_effects(i), r)).fold(r, |acc, b| acc.union(b));
        let o = outset(symbols, &n.appearance.effects, r);
        r.inflate(o, o)
    })
}

/// Does `item` paint (as [`Renderer::paint_items`] draws it)?
fn paints(item: &AppearanceItem) -> bool {
    item.visible() && !item.paint().is_none()
}

/// The bounds of what `n` draws, its own and its members' geometry effects applied and the raster
/// effects of its members and of its fills and strokes included (each fill's and stroke's
/// spreading what it paints, [`item_bounds`]), with strokes measured as
/// [`Node::visual_bounds_with`] measures them, without the reach of `n`'s own raster effects
/// (symbol instances measured by their art from `symbols`): what Rasterize makes its image over.
pub(crate) fn drawn_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    let b = art_bounds(symbols, n)?;
    let items = n.appearance.items.iter().filter(|i| paints(i) && !effects::raster_effects(item_effects(i)).is_empty());
    Some(items.map(|i| spread(symbols, item_effects(i), item_bounds(n, i).unwrap_or(b))).fold(b, |acc, r| acc.union(r)))
}

/// The bounds of what `item`, a fill or stroke of the path or compound path `n`, paints: the path
/// with `n`'s and `item`'s geometry effects, and for a stroke its width. `None` for other objects,
/// 3D art, a brushed stroke and an item that draws no path.
fn item_bounds(n: &Node, item: &AppearanceItem) -> Option<Rect> {
    let stroke = matches!(item, AppearanceItem::Stroke(_));
    if !has_fx(n) || effects::has_revolve(n) || effects::has_solid(n) || (stroke && vectorcraft_brush::has_brush(n)) {
        return None;
    }
    let g = effected_path(n, &node_bezpath(n)?);
    let fx = item_effects(item);
    if effects::has_geometry(fx) {
        let ig = apply(fx, &g, &GeomContext::of(n).item(item));
        let o = if let AppearanceItem::Stroke(s) = item { s.reach() } else { 0.0 };
        return (!ig.elements().is_empty()).then(|| ig.bounding_box().inflate(o, o));
    }
    let one = vectorcraft_doc::Appearance { items: vec![item.clone()], ..Default::default() };
    (!g.elements().is_empty()).then(|| one.stroked_bounds(g.bounding_box(), || g.clone()))
}

/// `b` grown by how far raster effects `fx` paint past it, each spreading what the ones before it
/// paint (Rasterize at the document's raster effects resolution where it takes it).
fn spread(symbols: &SymbolBoxes, fx: &[Effect], b: Rect) -> Rect {
    let ppi = doc_raster_ppi(symbols.document());
    effects::raster_effects(fx).iter().fold(b, |r, e| e.at_document_ppi(ppi).spread(r, 72.0 / ppi))
}

/// [`drawn_bounds`] before the raster effects of `n`'s fills and strokes: its art, and the art of
/// its brushed strokes (a symbol's brushed art as far as [`crate::brush_fx::pad`] reaches).
fn art_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    let b = shape_bounds(symbols, n);
    let brushed = vectorcraft_brush::has_brush(n)
        .then(|| vectorcraft_brush::expand(symbols.document(), n))
        .flatten()
        .and_then(|art| art.visual_bounds_with(symbols));
    let b = vectorcraft_geom::union_opt(b, brushed)?;
    let pad = if matches!(n.kind, NodeKind::SymbolInstance { .. }) { crate::brush_fx::pad(symbols, n) } else { 0.0 };
    Some(b.inflate(pad, pad))
}

/// [`art_bounds`] without brush art.
fn shape_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    // The visible members of `g` (a Shaper group's first member is its source art, which paints
    // nothing).
    let union = |g: &Node| {
        let members = g.children()?.get(usize::from(g.extras().shaper.is_some())..).unwrap_or_default();
        members.iter().filter(|c| c.visible).fold(None, |acc, c| vectorcraft_geom::union_opt(acc, painted_reach(symbols, c)))
    };
    match &n.kind {
        _ if effects::has_container_appearance(n) => {
            let art = effects::evaluate_container(Some(symbols.document()), n);
            let art = art.as_ref().unwrap_or(n);
            if art.clips() { art.visual_bounds_with(symbols) } else { union(art) }
        }
        NodeKind::Layer { clip: false, .. } | NodeKind::Group { clip: false, .. } => union(n),
        _ if has_fx(n) => {
            // A 3D effect's art.
            if let Some(art) = effects::revolve_art(n).or_else(|| effects::solid_art(n)) {
                return drawn_bounds(symbols, &art);
            }
            let g = effected_path(n, &node_bezpath(n)?);
            // The path as the fills and strokes without geometry effects of their own paint it
            // (as every fill and stroke would when none paints).
            let mut plain = n.appearance.clone();
            plain.items.retain(|i| !(paints(i) && effects::has_geometry(item_effects(i))));
            let on_path = !n.appearance.items.iter().any(paints) || plain.items.iter().any(paints);
            let mut r = (on_path && !g.elements().is_empty()).then(|| plain.stroked_bounds(g.bounding_box(), || g.clone()));
            let ctx = GeomContext::of(n);
            for item in n.appearance.items.iter().filter(|i| paints(i) && effects::has_geometry(item_effects(i))) {
                let ig = apply(item_effects(item), &g, &ctx.item(item));
                let o = if let AppearanceItem::Stroke(s) = item { s.reach() } else { 0.0 };
                r = vectorcraft_geom::union_opt(r, (!ig.elements().is_empty()).then(|| ig.bounding_box().inflate(o, o)));
            }
            r
        }
        // As [`cull_bounds_in`] measures them, without the object's own raster effects: the
        // reshaped art (type's outlines, and an image and the paths of its own fills and strokes,
        // measured one by one with the effects of those fills and strokes), grown by how far the
        // effects in a symbol's art and a live object's content paint past it.
        _ if has_object_fx(n) => {
            let symbol = match &n.kind {
                NodeKind::SymbolInstance { symbol, .. } => symbols.document().symbols.iter().find(|s| s.name == *symbol).map(|s| s.art.clone()),
                _ => None,
            };
            let reshaped = effects::reshape(Some(symbols.document()), n, symbol.as_deref());
            let b = match &reshaped {
                Some(r) if matches!(n.kind, NodeKind::Text(_)) || (matches!(n.kind, NodeKind::Image(_)) && !r.clips()) => {
                    r.children().into_iter().flatten().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, painted_reach(symbols, c)))
                }
                Some(r) => r.visual_bounds_with(symbols),
                None => None,
            }
            .or_else(|| n.visual_bounds_with(symbols))?;
            let a = match &reshaped {
                Some(r) => reshaped_art_effects_reach(symbols, n, r),
                None => art_effects_reach(symbols, n),
            };
            let a = a.max(live_content_reach(symbols, n));
            Some(b.inflate(a, a))
        }
        _ => {
            let b = n.visual_bounds_with(symbols)?;
            let a = art_effects_reach(symbols, n).max(live_content_reach(symbols, n));
            Some(b.inflate(a, a))
        }
    }
}

/// [`drawn_bounds`] of `n` grown by how far its own raster effects paint past them. With Crop
/// Marks, the [`drawn_bounds`] of its art: the marks and the object, which paints its own raster
/// effects inside the art.
pub(crate) fn painted_reach(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    if let Some(art) = effects::crop_marks_art(n) {
        return drawn_bounds(symbols, &art);
    }
    drawn_bounds(symbols, n).map(|b| spread(symbols, &n.appearance.effects, b))
}

/// The outline the renderer clips `n`'s Feather, Inner Glow and Rasterize with Create Clipping
/// Mask to: a path's or compound path's geometry with its object-level geometry effects applied,
/// the outline of the art [`Renderer::fx_art`] makes for an object with object effects, and with
/// Crop Marks the object's own. For other objects it is [`effects::clip_outline`].
pub(crate) fn drawn_outline(doc: &Document, n: &Node) -> Option<(BezPath, FillRule)> {
    if let Some(art) = effects::crop_marks_art(n) {
        return drawn_outline(doc, art.children()?.first()?);
    }
    if has_object_fx(n) {
        let art = Renderer::new().fx_art(doc, &Arc::new(n.clone()), false);
        return effects::clip_outline(&art);
    }
    match &n.kind {
        NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } if effects::has_geometry(&n.appearance.effects) => {
            Some((effected_path(n, &node_bezpath(n)?), *rule))
        }
        _ => effects::clip_outline(n),
    }
}

/// How far raster effects `fx` paint beyond `b`, each spreading what the ones before it paint
/// ([`spread`]).
fn outset(symbols: &SymbolBoxes, fx: &[Effect], b: Rect) -> f64 {
    let s = spread(symbols, fx, b);
    [b.x0 - s.x0, b.y0 - s.y0, s.x1 - b.x1, s.y1 - b.y1].into_iter().fold(0.0, f64::max)
}

/// Bounds used for culling: like `Node::visual_bounds_with`, but aware of live effects anywhere in
/// the subtree (a shadow can reach the view while its object is outside it). Symbol instances are
/// measured by their art from `symbols`.
pub(crate) fn cull_bounds(symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
    cull_bounds_in(symbols, n, true)
}

/// [`cull_bounds`] of `n`. With `after_reshape` true, the effects in the art of a symbol instance
/// with geometry effects of its own are measured on the art as those effects reshape it
/// ([`reshaped_art_effects_reach`]). With `after_reshape` false, they are measured on the art
/// before those effects run ([`art_effects_reach`]), which `symbols` keeps for each symbol and
/// linear part.
fn cull_bounds_in(symbols: &SymbolBoxes, n: &Node, after_reshape: bool) -> Option<Rect> {
    let inner = |c: &Node| cull_bounds_in(symbols, c, after_reshape);
    match &n.kind {
        // The object and its crop marks.
        _ if effects::has_crop_marks(n) => effects::crop_marks_art(n).and_then(|art| inner(&art)),
        // The evaluated art (its fills, strokes and geometry effects) and the raster effects' reach.
        _ if effects::has_container_appearance(n) => {
            let art = effects::evaluate_container(Some(symbols.document()), n);
            let art = art.as_ref().unwrap_or(n);
            let b = if art.clips() {
                art.visual_bounds_with(symbols)
            } else {
                art.children()?.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, inner(c)))
            }?;
            let o = outset(symbols, &n.appearance.effects, b);
            Some(b.inflate(o, o))
        }
        NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
            children.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, inner(c)))
        }
        _ if has_fx(n) => visual_bounds(symbols, n),
        // The reshaped art (a symbol instance's reshaped art is made from its symbol's art; type's
        // outlines, and an image and the paths of its own fills and strokes, measured one by one
        // with the geometry effects of those fills and strokes), grown by how far the effects in a
        // symbol instance's art paint past that art.
        _ if has_object_fx(n) => {
            let symbol = match &n.kind {
                NodeKind::SymbolInstance { symbol, .. } => symbols.document().symbols.iter().find(|s| s.name == *symbol).map(|s| s.art.clone()),
                _ => None,
            };
            let reshaped = effects::reshape(Some(symbols.document()), n, symbol.as_deref());
            let b = match &reshaped {
                Some(r) if matches!(n.kind, NodeKind::Text(_)) || (matches!(n.kind, NodeKind::Image(_)) && !r.clips()) => {
                    r.children().into_iter().flatten().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, inner(c)))
                }
                Some(r) => r.visual_bounds_with(symbols),
                None => None,
            }
            .or_else(|| n.visual_bounds_with(symbols))?;
            let a = match &reshaped {
                Some(r) if after_reshape => reshaped_art_effects_reach(symbols, n, r),
                _ => art_effects_reach(symbols, n),
            };
            let a = a.max(live_content_reach(symbols, n));
            let b = b.inflate(a, a);
            let o = outset(symbols, &n.appearance.effects, b);
            Some(b.inflate(o, o))
        }
        NodeKind::SymbolInstance { .. } => {
            let b = n.visual_bounds_with(symbols)?;
            let a = art_effects_reach(symbols, n);
            Some(b.inflate(a, a))
        }
        _ => {
            let b = n.visual_bounds_with(symbols)?;
            let b = match &n.kind {
                NodeKind::Text(t) => vectorcraft_geom::union_opt(Some(b), glyphs_box(t, n.appearance.outset())).unwrap_or(b),
                _ => b,
            };
            let a = live_content_reach(symbols, n);
            Some(b.inflate(a, a))
        }
    }
}

/// The box of type `t`'s glyphs as laid out (its layout bounds, else laid out now), grown by its
/// characters' strokes, miter spikes included, and by `outset`: on a path they stand above it and
/// run past its ends, in an area they can hang past its frame, where its own box
/// ([`Node::visual_bounds_with`]) doesn't.
fn glyphs_box(t: &vectorcraft_doc::TextObject, outset: f64) -> Option<Rect> {
    let b = t.cached_bounds.unwrap_or_else(|| vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).bounds);
    let o = outset.max(t.char_stroke_reach());
    (b.width() > 0.0 || b.height() > 0.0).then(|| t.xf.transform_rect_bbox(b).inflate(o, o))
}

/// How far the content of live object `n` (a blend, envelope or repeat) can paint past its visual
/// bounds: for each object in it, how far the effects in a symbol instance's art paint past that
/// art ([`art_effects_reach`]) and how far brush art can paint ([`crate::brush_fx::pad`]), on the
/// farthest side. 0 for any other kind of object.
fn live_content_reach(symbols: &SymbolBoxes, n: &Node) -> f64 {
    if !vectorcraft_doc::live::is_live(n) {
        return 0.0;
    }
    let mut reach = 0.0f64;
    n.walk(&mut |c| reach = reach.max(art_effects_reach(symbols, c) + crate::brush_fx::pad(symbols, c)));
    reach
}

/// How far the effects in the art of symbol instance `n` (and in the art of the symbols the
/// instances in it show) paint past that art's visual bounds, on its farthest side, measured on the
/// art as the instance places it before its effects run: 0 when the art has no visible effects, and
/// for any other kind of object. The reach is the same wherever the art is moved to, so the art is
/// placed by the linear part of the instance's transform alone, and `symbols` keeps the reach for
/// each symbol and linear part, up to a limit ([`SymbolBoxes::effects_reach`]).
fn art_effects_reach(symbols: &SymbolBoxes, n: &Node) -> f64 {
    let NodeKind::SymbolInstance { symbol, xf } = &n.kind else { return 0.0 };
    if !symbols.has_effects(symbol) {
        return 0.0;
    }
    let measure = |[m0, m1, m2, m3]: [f64; 4]| {
        let past = |art: &Arc<Node>| {
            let mut placed = (**art).clone();
            placed.transform(Affine::new([m0, m1, m2, m3, 0.0, 0.0]), false);
            match (cull_bounds(symbols, &placed), placed.visual_bounds_with(symbols)) {
                (Some(c), Some(v)) => [v.x0 - c.x0, v.y0 - c.y0, c.x1 - v.x1, c.y1 - v.y1].into_iter().fold(0.0, f64::max),
                _ => 0.0,
            }
        };
        symbols.measure_art(symbol, past).unwrap_or(0.0)
    };
    let [m0, m1, m2, m3, _, _] = xf.as_coeffs();
    symbols.effects_reach(symbol, [m0, m1, m2, m3], &measure)
}

/// How far the effects in the art of symbol instance `n` paint past `r`, that art as `n`'s own
/// geometry effects reshape it ([`effects::reshape`]), on its farthest side: 0 when the art has no
/// visible effects, and for any other kind of object. The art in `r` is measured without the raster
/// effects of `n` that `r` carries, by [`cull_bounds_in`] with `after_reshape` false: the effects in
/// the art of an instance in it are measured before that instance's own geometry effects run.
fn reshaped_art_effects_reach(symbols: &SymbolBoxes, n: &Node, r: &Node) -> f64 {
    let NodeKind::SymbolInstance { symbol, .. } = &n.kind else { return 0.0 };
    if !symbols.has_effects(symbol) {
        return 0.0;
    }
    let art = r.children().into_iter().flatten().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, cull_bounds_in(symbols, c, false)));
    match (art, r.visual_bounds_with(symbols)) {
        (Some(c), Some(v)) => [v.x0 - c.x0, v.y0 - c.y0, c.x1 - v.x1, c.y1 - v.y1].into_iter().fold(0.0, f64::max),
        _ => 0.0,
    }
}

fn shadow_filter(dx: f64, dy: f64, blur: f64, color: peniko::Color) -> Filter {
    Filter::from_primitive(FilterPrimitive::DropShadowOnly {
        dx: dx as f32,
        dy: dy as f32,
        std_deviation: (blur / 2.0).max(0.0) as f32,
        color,
        edge_mode: EdgeMode::None,
    })
}

/// The layer filter of a drop shadow with Darkness: the object's own pixels blurred.
fn shade_filter(blur: f64) -> Filter {
    Filter::from_primitive(FilterPrimitive::GaussianBlur { std_deviation: (blur / 2.0).max(0.0) as f32, edge_mode: EdgeMode::None })
}

/// How much darker (0..1) a drop shadow with Darkness `darkness` makes the values `ink` paints:
/// black added leaves the cyan, magenta and yellow plane of a CMYK document as it is and scales
/// the complemented black plane, as it scales screen colors, by 1 − `darkness`.
fn shade(ink: Ink, darkness: f32) -> f32 {
    match ink {
        Ink::Cmy => 0.0,
        Ink::K | Ink::Display | Ink::RichBlack => darkness,
    }
}

/// What a shadow or glow raster is made from ([`Made::pixels`]): the silhouette's alpha, or its
/// premultiplied RGBA when the shadow takes the object's own colors (Darkness).
fn silhouette(pm: &vello_cpu::Pixmap, colored: bool) -> Vec<u8> {
    if colored { pm.data().iter().flat_map(|p| [p.r, p.g, p.b, p.a]).collect() } else { pm.data().iter().map(|p| p.a).collect() }
}

fn pcolor(ink: Ink, c: &vectorcraft_doc::color::Color) -> peniko::Color {
    let [r, g, b] = ink.rgb(c);
    peniko::Color::new([r, g, b, 1.0])
}

/// Most pixels the raster of [`Renderer::pixel_content`] holds (a 4K screen's): larger content is
/// cropped to the view when its effects only look nearby, else rendered at a lower resolution.
const PIXEL_BUDGET: f64 = 8.4e6;

/// Most Rasterize effects of one stack that [`Renderer::raster_fx`] draws (the last ones): each
/// draws the content again inside the one after it.
const MAX_RASTERIZE: usize = 16;

/// The columns and rows of the image Rasterize `r` makes over `rect` (whole pixels of the image,
/// `line` points wide at the document's raster effects resolution). `None` when the image would
/// hold more than [`PIXEL_BUDGET`] pixels or be wider or taller than [`crate::MAX_RASTER_SIDE`].
fn image_size(r: &effects::Rasterize, rect: Rect, line: f64) -> Option<(u16, u16)> {
    let cell = r.cell(line);
    let (cols, rows) = ((rect.width() / cell).round(), (rect.height() / cell).round());
    let side = f64::from(crate::MAX_RASTER_SIDE);
    (cols >= 1.0 && rows >= 1.0 && cols <= side && rows <= side && cols * rows <= PIXEL_BUDGET).then_some((cols as u16, rows as u16))
}

#[cfg(test)]
thread_local! {
    /// How many times a Rasterize has drawn the content before it on this thread.
    pub(crate) static RASTERIZED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Document Raster Effects Settings › Resolution of `doc`, in ppi (1 to 2400; 72 when it isn't a
/// number).
fn doc_raster_ppi(doc: &Document) -> f64 {
    doc.raster_ppi()
}

/// The effects [`Renderer::pixel_content`] runs on the content's pixels (those changing the
/// content itself), in stack order.
fn content_changing(fx: &&RasterFx) -> bool {
    matches!(fx, RasterFx::Feather { .. } | RasterFx::GaussianBlur { .. } | RasterFx::Pixel(_) | RasterFx::Rasterize(_))
}

/// Is the content drawn with hard edges for `chain` into a raster of pixels `px` wide (document
/// units): does it go first through a Rasterize with anti-aliasing off whose pixels are no
/// smaller? (Smaller ones show as the share of each raster pixel they cover, as smooth edges do.)
fn hard_edged(chain: &[&RasterFx], px: f64) -> bool {
    matches!(chain.first(), Some(RasterFx::Rasterize(r)) if !r.anti_alias && r.cell(72.0) >= px)
}

/// How far (document units) from a pixel the content deciding it after `fx` lies; `None` when
/// anywhere in the object.
fn looks_around(fx: &RasterFx) -> Option<f64> {
    match fx {
        RasterFx::Feather { radius } | RasterFx::GaussianBlur { radius } => Some(1.5 * radius),
        RasterFx::Pixel(p) => p.reach(),
        RasterFx::Rasterize(r) => Some(r.reach()),
        _ => Some(0.0),
    }
}

/// The content of an object with Photoshop-style effects as filtered pixels, drawn at
/// (`anchor`'s view position + `rel`) scaled by 1 / `k` (see [`Renderer::pixel_content`]).
pub(crate) struct PixelEntry {
    node: Drawn,
    linear: [u64; 4],
    anchor: vectorcraft_geom::Point,
    rel: (f64, f64),
    k: f64,
    image: std::sync::Arc<vello_cpu::Pixmap>,
    /// For a raster of the content's whole reach at full size, `None`: it draws after a pan too.
    /// For one cropped to the view, or made at a lower resolution, the view and the rect it was
    /// made for: it draws only there, so dragging other art over the object keeps it (a wide blur,
    /// or any blur zoomed in, reaches past the pixel budget).
    view: Option<ViewCrop>,
    made: Made,
    /// What the content read when the raster was made, with the channels the effects ran on and,
    /// under De-Interlace, the raster resolution.
    inputs: Inputs,
    /// Made for the object before it moved ([`Renderer::move_pixel_content`]), or made over
    /// rasters made before their objects moved: only a frame rendered while interacting draws
    /// it.
    moved: bool,
    /// How many images in the content failed to decode while the raster was made
    /// ([`crate::paint::decode_failures`]): each frame that draws the raster counts them
    /// again ([`crate::paint::add_decode_failures`]).
    failed: u64,
    pub(crate) stamp: u64,
}

impl PixelEntry {
    /// The bytes it holds, for [`FX_RASTER_BUDGET`].
    pub(crate) fn bytes(&self) -> usize {
        raster_bytes(&self.image) + self.made.bytes()
    }
}

/// The four linear coefficients of `view` (zoom and rotation, not the pan), as cache keys.
fn linear_key(view: Affine) -> [u64; 4] {
    let c = view.as_coeffs();
    [c[0].to_bits(), c[1].to_bits(), c[2].to_bits(), c[3].to_bits()]
}

/// The pan of the view a raster was cropped for, or made at a lower resolution than the view's, and
/// the device-pixel rect of the frame it was made in ([`PixelEntry::view`], [`ShadowEntry::view`]).
#[derive(Clone, Copy, Debug)]
struct ViewCrop {
    pan: Vec2,
    screen: Rect,
}

impl ViewCrop {
    fn of(view: Affine, screen: Rect) -> Self {
        Self { pan: view.translation(), screen }
    }

    /// Does the raster draw in a frame through `view` (of the same zoom and rotation) onto
    /// `screen`? When the view is panned by whole pixels and the frame's rect, panned back, lies
    /// within the one the raster was made in: the same pixels in the same places, as when a frame
    /// is drawn again only in a part of it.
    fn serves(&self, view: Affine, screen: Rect) -> bool {
        let d = view.translation() - self.pan;
        let whole = Vec2::new(d.x.round(), d.y.round());
        let s = screen - whole;
        (d - whole).hypot() < 1e-6 && s.x0 >= self.screen.x0 && s.y0 >= self.screen.y0 && s.x1 <= self.screen.x1 && s.y1 <= self.screen.y1
    }
}

/// Most pixels a raster of [`Renderer::pixel_content`] or [`Renderer::with_filters`] holds while
/// [`crate::RenderOptions::dragging`] (a 1000 × 1000 raster's), when the frame does not draw a raster
/// kept from an earlier frame instead (#2558).
const DRAG_PIXEL_BUDGET: f64 = 1.0e6;

/// The scale (raster pixels a device pixel) for content of `rw` × `rh` device pixels that would
/// render at scale `k`, when a drag lowers it to stay within [`DRAG_PIXEL_BUDGET`]; `None` when it
/// does not (`dragging` off, or content within the budget at `k`).
fn drag_scale(dragging: bool, (rw, rh): (f64, f64), k: f64) -> Option<f64> {
    let d = (DRAG_PIXEL_BUDGET / (rw * rh)).sqrt();
    (dragging && d < k).then_some(d)
}

/// The raster a shadow or glow blurred by `blur` device pixels is filtered in, for content
/// reaching `reach` (device pixels) in a view of `screen` pixels: (left, top, width, height,
/// scale). It covers what of the reach can show in the view, the blur's spread around it
/// included, at `scale` raster pixels a device pixel: 1, or less to stay within
/// [`PIXEL_BUDGET`]. At a high zoom the spread of a wide blur is tens of thousands of pixels a
/// side, whatever the size of the view (#1706). `None`: nothing to draw.
fn filter_region(reach: Rect, screen: (u16, u16), blur: f64) -> Option<(f64, f64, u16, u16, f64)> {
    filter_region_within(reach, screen, blur, PIXEL_BUDGET)
}

/// The region [`filter_region`] returns, within `budget` pixels instead of [`PIXEL_BUDGET`].
fn filter_region_within(reach: Rect, screen: (u16, u16), blur: f64, budget: f64) -> Option<(f64, f64, u16, u16, f64)> {
    // 3σ of the Gaussian (σ = blur / 2) in pixels, plus a pixel of antialiasing.
    let spread = blur.max(0.0) * 1.5 + 2.0;
    let screen = Rect::new(0.0, 0.0, f64::from(screen.0), f64::from(screen.1)).inflate(spread, spread);
    let r = reach.inflate(spread, spread).intersect(screen);
    let (x0, y0) = (r.x0.floor(), r.y0.floor());
    let (rw, rh) = (r.x1.ceil() - x0, r.y1.ceil() - y0);
    // (Also false for a reach or a blur that isn't a number.)
    if !(rw >= 1.0 && rh >= 1.0 && rw.is_finite() && rh.is_finite()) {
        return None;
    }
    let side = f64::from(u16::MAX);
    let k = (budget / (rw * rh)).sqrt().min(side / rw).min(side / rh).min(1.0);
    let (w, h) = ((rw * k).floor().clamp(1.0, side), (rh * k).floor().clamp(1.0, side));
    Some((x0, y0, w as u16, h as u16, k))
}

/// Draw `image` with its top-left at (`x`, `y`) device pixels, `1 / k` device pixels a pixel.
fn draw_scaled(ctx: &mut RenderContext, image: std::sync::Arc<vello_cpu::Pixmap>, (x, y): (f64, f64), k: f64) {
    let (w, h) = (image.width() as f64, image.height() as f64);
    ctx.set_transform(Affine::translate((x, y)) * Affine::scale(1.0 / k));
    // On whole pixels at full size the cheapest sampler is exact.
    let quality = if k < 1.0 { peniko::ImageQuality::Medium } else { peniko::ImageQuality::Low };
    let sampler = peniko::ImageSampler { quality, ..Default::default() };
    // The raster maps 1:1 onto the rect: a paint transform left by an earlier fill (a pattern,
    // radial or freeform gradient) would resample it (#1536).
    ctx.reset_paint_transform();
    ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(image), sampler });
    ctx.fill_rect(&Rect::new(0.0, 0.0, w, h));
    ctx.set_transform(Affine::IDENTITY);
}

/// Draw `image` with its pixels placed by `xf` (pixels to device pixels). On whole pixels at full
/// size it is drawn as [`draw_scaled`] draws it.
fn draw_placed(ctx: &mut RenderContext, image: std::sync::Arc<vello_cpu::Pixmap>, xf: Affine) {
    let [a, b, c, d, e, f] = xf.as_coeffs();
    let near = |v: f64, to: f64| (v - to).abs() < 1e-6;
    if near(a, 1.0) && near(b, 0.0) && near(c, 0.0) && near(d, 1.0) && near(e, e.round()) && near(f, f.round()) {
        return draw_scaled(ctx, image, (e.round(), f.round()), 1.0);
    }
    let (w, h) = (image.width() as f64, image.height() as f64);
    ctx.set_transform(xf);
    let sampler = peniko::ImageSampler { quality: peniko::ImageQuality::Medium, ..Default::default() };
    ctx.reset_paint_transform();
    ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(image), sampler });
    ctx.fill_rect(&Rect::new(0.0, 0.0, w, h));
    ctx.set_transform(Affine::IDENTITY);
}

/// A cached shadow / glow raster: premultiplied, already tinted and faded, positioned relative to
/// the view-space position of `anchor` (a document point).
pub(crate) struct ShadowEntry {
    node: Drawn,
    linear: [u64; 4],
    anchor: vectorcraft_geom::Point,
    /// Pixel offset of the raster's top-left from the anchor's view position.
    rel: (f64, f64),
    image: std::sync::Arc<vello_cpu::Pixmap>,
    /// View pixels per raster pixel: more than 1 for a glow made coarser to fit the budget.
    scale: f64,
    /// `None` for a raster of the whole silhouette's reach: it draws after a pan too. For one
    /// cropped to the view, or made over such a raster, the view and the rect it was made for: it
    /// draws only there ([`PixelEntry::view`]).
    view: Option<ViewCrop>,
    made: Made,
    /// What the silhouette read when the raster was made.
    inputs: Inputs,
    /// Made for the object before it moved ([`Renderer::move_shadow`]), or made over rasters
    /// made before their objects moved: only a frame rendered while interacting draws it.
    moved: bool,
    pub(crate) stamp: u64,
}

/// A shadow or glow raster ready to draw ([`Renderer::draw_shadow`]).
struct Shadow {
    image: std::sync::Arc<vello_cpu::Pixmap>,
    /// Raster to view pixels.
    at: Affine,
    mode: vectorcraft_doc::color::BlendMode,
    /// Made before its object moved ([`ShadowEntry::moved`]).
    moved: bool,
    /// Cropped to the view, or made over a raster that is ([`ShadowEntry::view`]).
    cropped: bool,
}

impl ShadowEntry {
    /// The bytes it holds, for [`FX_RASTER_BUDGET`].
    pub(crate) fn bytes(&self) -> usize {
        raster_bytes(&self.image) + self.made.bytes()
    }
}

/// What a cached shadow, glow or filtered raster was made from. While
/// [`crate::RenderOptions::interacting`], an object whose content only moved after the raster
/// was made draws from that raster ([`Renderer::move_shadow`], [`Renderer::move_pixel_content`]).
pub(crate) struct Made {
    /// The content's pixels as drawn into the raster before the effects ran: premultiplied RGBA,
    /// or their alpha alone under a shadow or glow that is one color ([`silhouette`]).
    pixels: Vec<u8>,
    /// The effects run on them.
    fx: Vec<RasterFx>,
    /// The size of the content's document-space bounds.
    size: Size,
    /// The color channels the Photoshop-style filters were told of (none for shadows and glows).
    channels: Option<Channels>,
    /// Feather's clip outline, moved to put the raster's anchor at the origin.
    outline: Option<(BezPath, FillRule)>,
}

/// How far (document units) the size of a moved object's content and the points of its Feather
/// outline may lie from those of the content a raster was made from.
const MOVED_SLACK: f64 = 1e-6;

impl Made {
    /// The bytes it holds beyond the raster.
    fn bytes(&self) -> usize {
        self.pixels.len() + self.outline.as_ref().map_or(0, |(g, _)| crate::lru::path_bytes(g))
    }

    /// Whether content of size `size` under the effects `fx`, told of `channels`, with Feather's
    /// `outline` (moved to put its anchor at the origin) may be this content moved: the same
    /// effects, channels and outline rule, and the size and outline within [`MOVED_SLACK`].
    fn fits(&self, fx: &[&RasterFx], size: Size, channels: Option<Channels>, outline: Option<&(BezPath, FillRule)>) -> bool {
        let near = |a: f64, b: f64| (a - b).abs() <= MOVED_SLACK;
        self.fx.len() == fx.len()
            && self.fx.iter().zip(fx).all(|(a, b)| a == *b)
            && near(self.size.width, size.width)
            && near(self.size.height, size.height)
            && self.channels == channels
            && match (&self.outline, outline) {
                (None, None) => true,
                (Some((a, ra)), Some((b, rb))) => ra == rb && same_path(a, b),
                _ => false,
            }
    }
}

/// Whether `a` and `b` have the same elements with points within [`MOVED_SLACK`] of each other.
fn same_path(a: &BezPath, b: &BezPath) -> bool {
    let near = |p: &Point, q: &Point| (*p - *q).hypot() <= MOVED_SLACK;
    a.elements().len() == b.elements().len()
        && a.elements().iter().zip(b.elements()).all(|pair| match pair {
            (PathEl::MoveTo(p), PathEl::MoveTo(q)) | (PathEl::LineTo(p), PathEl::LineTo(q)) => near(p, q),
            (PathEl::QuadTo(p0, p1), PathEl::QuadTo(q0, q1)) => near(p0, q0) && near(p1, q1),
            (PathEl::CurveTo(p0, p1, p2), PathEl::CurveTo(q0, q1, q2)) => near(p0, q0) && near(p1, q1) && near(p2, q2),
            (PathEl::ClosePath, PathEl::ClosePath) => true,
            _ => false,
        })
}

/// Feather's clip outline of `content` when `chain` feathers it, moved to put `anchor` at the
/// origin ([`Made::outline`]).
fn feather_outline(chain: &[&RasterFx], content: &Content, anchor: Point) -> Option<(BezPath, FillRule)> {
    let (g, rule) = content.outline.filter(|_| chain.iter().any(|fx| matches!(fx, RasterFx::Feather { .. })))?;
    let mut g = g.clone();
    g.apply_affine(Affine::translate(-anchor.to_vec2()));
    Some((g, rule))
}

/// The object a cached raster was made for: the drawn `Arc` itself when known (kept alive, so its
/// address, the cache key, names it alone and a key match is exact), else a copy compared by value.
pub(crate) enum Drawn {
    Shared(Arc<Node>),
    Copy(Box<Node>),
}

impl Drawn {
    fn of(n: &Node, owner: Option<&Arc<Node>>) -> Self {
        match owner {
            Some(a) => Self::Shared(a.clone()),
            None => Self::Copy(Box::new(n.clone())),
        }
    }

    /// Is `n` the object drawn?
    fn is(&self, n: &Node) -> bool {
        match self {
            Self::Shared(a) => std::ptr::eq(&**a, n),
            Self::Copy(c) => **c == *n,
        }
    }

    /// The id of the object drawn.
    fn id(&self) -> NodeId {
        match self {
            Self::Shared(a) => a.id,
            Self::Copy(c) => c.id,
        }
    }
}

/// What an object's content reads from outside its own nodes, in it or in the opacity masks of the
/// objects in it ([`Reads::of`]): which parts of [`Inputs`] can change what its rasters show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reads {
    /// Pattern paints: the pattern definitions. Their art can hold anything, so content painted
    /// with a pattern reads everything else too.
    patterns: bool,
    /// Symbol instances and type with inline graphics: the symbols' art. Symbol art can hold
    /// anything, so content that shows a symbol reads everything else too.
    symbols: bool,
    /// Images and placed documents: their pixels and files, and the options and the layer's Dim
    /// Images they are drawn with.
    images: bool,
    /// Brushed strokes: the brush library.
    brushes: bool,
    /// Type: the fonts, whether overprinting shows ([`crate::proof::overprints`]) and Document
    /// Setup's substitution highlights.
    text: bool,
    /// Plug-in effects on its objects: the plug-ins installed. Those in the symbol and pattern art
    /// it shows are found with the document's inputs ([`DocInputs`]).
    plugins: bool,
}

impl Reads {
    /// What the content of `n` reads.
    pub(crate) fn of(n: &Node) -> Self {
        Self::of_all(std::iter::once(n))
    }

    /// What the content of `nodes` reads together. A pattern's art can hold anything, as a
    /// symbol's can, so content painted with a pattern reads everything else too (#1723).
    pub(crate) fn of_all<'a>(nodes: impl Iterator<Item = &'a Node>) -> Self {
        let mut reads = Self::default();
        nodes.for_each(|n| reads.add(n));
        if reads.symbols || reads.patterns {
            return Self { patterns: true, symbols: true, images: true, brushes: true, text: true, plugins: reads.plugins };
        }
        reads
    }

    /// Does the content read anything outside its own nodes?
    pub(crate) fn any(self) -> bool {
        self != Self::default()
    }

    /// Can installing or removing a plug-in change what the content draws? It can when an object in
    /// it has a plug-in effect, and when it shows symbol or pattern art.
    pub(crate) fn plugins_can_change(self) -> bool {
        self.plugins || self.symbols || self.patterns
    }

    /// Does the content show symbol or pattern art?
    pub(crate) fn shows_definitions(self) -> bool {
        self.symbols || self.patterns
    }

    fn add(&mut self, n: &Node) {
        let pattern = |p: &vectorcraft_color::Paint| matches!(p, vectorcraft_color::Paint::Pattern { .. });
        n.walk(&mut |c| {
            self.plugins |= effects::has_plugin_effect(c);
            for item in &c.appearance.items {
                self.patterns |= pattern(item.paint());
                self.brushes |= matches!(item, AppearanceItem::Stroke(s) if s.brush.is_some());
            }
            match &c.kind {
                NodeKind::SymbolInstance { .. } => self.symbols = true,
                NodeKind::Image(_) | NodeKind::PlacedDocument(_) => self.images = true,
                NodeKind::Text(t) => {
                    self.text = true;
                    self.symbols |= t.runs.iter().any(|r| r.inline.is_some());
                    self.patterns |= t.runs.iter().any(|r| pattern(&r.style.fill) || pattern(&r.style.stroke));
                }
                _ => {}
            }
            if let Some(m) = c.extras().mask.as_deref() {
                self.add(&m.art);
            }
        });
    }
}

/// What a cached shadow, glow or filtered raster was drawn from besides its object, its effects
/// and the view's zoom and rotation ([`Renderer::raster_inputs`]). A raster is drawn from only
/// while its inputs equal the frame's.
#[derive(Clone)]
pub(crate) struct Inputs {
    /// What the content reads.
    reads: Reads,
    /// A hash of the inputs as found when the raster was made: those every raster reads, those
    /// `reads` names, and what the effects read themselves ([`Inputs::with`]).
    key: u64,
    /// The document's resources and the color settings `key` was found with: holding them keeps
    /// the addresses they hashed for their values ([`DocInputs`]).
    _doc: Arc<DocInputs>,
}

impl PartialEq for Inputs {
    fn eq(&self, other: &Self) -> bool {
        (self.reads, self.key) == (other.reads, other.key)
    }
}

impl Inputs {
    /// These inputs with `extra`, something the effects read themselves.
    fn with(mut self, extra: impl Hash) -> Self {
        let mut h = DefaultHasher::new();
        (self.key, extra).hash(&mut h);
        self.key = h.finish();
        self
    }
}

/// What raster contents read from the document by name or key, and the color settings, as a frame
/// finds them ([`Renderer::doc_inputs`]): hashes of the patterns' tilings
/// ([`crate::pattern::hash_tiling`]), of the symbols' names and art and of the images' keys and
/// pixels, the address of the color settings, and whether the patterns' and symbols' art has a
/// plug-in effect. Art, pixels and previews are hashed by the addresses of their shared values,
/// and names, keys, tiles, tile types and overlaps are hashed by value. The live cache keeps these
/// inputs with each evaluated live object, with each mesh's quads and with what each walk of a
/// live object or of effect art found when it found no placed file (`crate::live`), and
/// [`Renderer::fx_art`] keeps them with art that can be made from evaluated live objects.
pub(crate) struct DocInputs {
    patterns: u64,
    symbols: u64,
    images: u64,
    pub(crate) cms: usize,
    /// Whether an object in a pattern's or a symbol's art, or in an opacity mask there, has a
    /// visible plug-in effect.
    plugins: bool,
    /// Weak references to the values whose addresses the hashes hold: a weak reference keeps the
    /// allocation, not the value, so no other value gets one of those addresses while these
    /// inputs are kept.
    _held: Held,
}

impl DocInputs {
    /// Whether `self` and `other` were made from the same patterns and symbols: their names,
    /// tilings and art.
    pub(crate) fn same_art(&self, other: &DocInputs) -> bool {
        (self.patterns, self.symbols) == (other.patterns, other.symbols)
    }
}

/// The pattern and symbol art, the image pixels and the color settings, held weakly.
type Held = (Vec<Weak<Node>>, Vec<Weak<Vec<u8>>>, Weak<Cms>);

/// Most bytes the shadow and filtered-content rasters of one renderer hold together, with what each
/// was made from ([`Made`]): the least recently drawn go beyond it.
pub(crate) const FX_RASTER_BUDGET: usize = 512 << 20;

fn raster_bytes(pm: &vello_cpu::Pixmap) -> usize {
    pm.width() as usize * pm.height() as usize * 4
}

/// Live-effect geometry of a node, kept across frames: the object-level result and each
/// appearance item's own result on it (filled in as items are drawn). The node is kept alive, so
/// its address names it alone and a hit is exact.
pub(crate) struct FxGeom {
    pub(crate) node: Arc<Node>,
    g: Arc<BezPath>,
    items: Vec<Option<Arc<BezPath>>>,
    pub(crate) stamp: u64,
}

impl Renderer {
    /// Drop shadow / outer glow `fx` (shadow-cache slot `i` of `n`) as a cached raster: the
    /// silhouette `paint` draws is rendered once into a small crop, blurred and tinted on the CPU,
    /// and reused while the object, the zoom/rotation and what the silhouette reads ([`Inputs`]) are
    /// unchanged (pans only move it), and while interacting after the object only moved
    /// ([`Self::move_shadow`]). A raster of the whole reach too large for the budget is cropped to
    /// what can show in the `screen` (device pixels): the view, and around it the blur's spread and
    /// `around` (document units: how far the shadows and glows stacked over this one look), made
    /// coarser to fit the budget where the blur hides the coarser grid. Such a raster, and one
    /// made over it (`over_cropped`), is reused only in the same view. `None` when the effect isn't
    /// a shadow/glow or the raster would still be too large (caller falls back to
    /// [`Self::draw_shadow_filter`]).
    /// While dragging on a multithreaded context, a raster that has to be made again counts as too
    /// large when it would hold more than [`DRAG_PIXEL_BUDGET`] pixels.
    #[allow(clippy::too_many_arguments)]
    fn shadow_raster(
        &mut self,
        f: &Frame,
        n: &Node,
        i: usize,
        fx: &RasterFx,
        reach: Rect,
        owner: Option<&Arc<Node>>,
        (screen, around, over_cropped): (Rect, f64, bool),
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) -> Option<Shadow> {
        let (mode, opacity, dx, dy, blur, color, darkness) = match fx {
            RasterFx::DropShadow { mode, opacity, dx, dy, blur, color, darkness } => (*mode, *opacity, *dx, *dy, *blur, *color, *darkness),
            RasterFx::OuterGlow { mode, opacity, blur, color } => (*mode, *opacity, 0.0, 0.0, *blur, *color, None),
            _ => return None,
        };
        let linear = linear_key(f.view);
        let key = (n as *const Node as usize, i, f.ink);
        let stamp = self.stamp;
        let interacting = f.opts.interacting;
        // What the silhouette reads: as the raster kept for `n` found it, else found now.
        let reads = self.shadows.get(&key).filter(|e| e.node.is(n)).map_or_else(|| Reads::of(n), |e| e.inputs.reads);
        let inputs = self.raster_inputs(f, n, reads);
        let in_view = ViewCrop::of(f.view, screen);
        let exact = |e: &ShadowEntry| {
            e.linear == linear && e.node.is(n) && e.inputs == inputs && (interacting || !e.moved) && e.view.is_none_or(|v| v.serves(f.view, screen))
        };
        if interacting && !self.shadows.get(&key).is_some_and(exact) {
            self.move_shadow(f, n, key, fx, reach + Vec2::new(dx, dy), owner, &inputs, paint);
        }
        let hit = self.shadows.get_mut(&key).filter(|e| exact(e));
        let (image, x, y, scale, moved, cropped) = match hit {
            Some(e) => {
                e.stamp = stamp;
                self.stats.moved += usize::from(e.moved);
                let p = f.view * e.anchor;
                (e.image.clone(), (p.x + e.rel.0).round(), (p.y + e.rel.1).round(), e.scale, e.moved, e.view.is_some())
            }
            None => {
                // An Outer Glow's silhouette grows by a quarter of a pixel of the document's raster grid first.
                let grow = if matches!(fx, RasterFx::OuterGlow { .. }) { 18.0 / doc_raster_ppi(f.doc) } else { 0.0 };
                let spread = (blur.max(0.0) * 1.5 + grow) / f.px + 2.0;
                let reach = reach + Vec2::new(dx, dy);
                let whole = f.view.transform_rect_bbox(reach).inflate(spread, spread);
                // The raster's region and scale within `budget` pixels: `None` when over the limits.
                let region = |r: Rect, coarser: bool, budget: f64| {
                    let (x0, y0) = (r.x0.floor(), r.y0.floor());
                    let (w, h) = (r.x1.ceil() - x0, r.y1.ceil() - y0);
                    let scale = if coarser { (w * h / budget).sqrt().ceil().max(1.0) } else { 1.0 };
                    if !scale.is_finite() || scale > 1.0 && blur / 2.0 / f.px / scale < 2.0 {
                        return None;
                    }
                    let (w, h) = ((w / scale).ceil(), (h / scale).ceil());
                    ((1.0..=8192.0).contains(&w) && (1.0..=8192.0).contains(&h) && w * h <= budget).then_some((x0, y0, w, h, scale))
                };
                // A glow too large for the budget is made coarser, where its blur hides the coarser grid: the filter
                // layer the caller falls back to can't make its silhouette solid.
                let full = region(whole, matches!(fx, RasterFx::OuterGlow { .. }), 4.0e6).filter(|_| !over_cropped);
                // Else it is cropped to what can show in the view, within the budget of the filter
                // layer it would otherwise be drawn through, and made coarser in the same way.
                let ((x0, y0, w, h, scale), cropped) = match full {
                    Some(r) => (r, false),
                    None => {
                        let m = spread + around / f.px;
                        let r = whole.intersect(screen.inflate(m, m));
                        if r.width() <= 0.0 || r.height() <= 0.0 {
                            return None;
                        }
                        (region(r, true, PIXEL_BUDGET)?, true)
                    }
                };
                // During a drag, a larger shadow on a multithreaded context is filtered within the
                // drag's budget (`with_filters`), and no raster is kept.
                if f.opts.dragging && f.mt && w * h > DRAG_PIXEL_BUDGET {
                    return None;
                }
                let (w, h) = (w as u16, h as u16);
                // Silhouette (the object's painted alpha, and its colors under Darkness), offset by
                // the shadow distance.
                let mut off = f.offscreen_context(w, h);
                let crop = Frame { mt: false, view: Affine::scale(1.0 / scale) * Affine::translate((-x0, -y0)) * f.view, ..*f };
                let before = self.stats.moved;
                self.inside_layer(|r| paint(r, &mut off, &shifted(&crop, dx, dy)));
                // A silhouette drawn over moved rasters makes a moved shadow.
                let moved = self.stats.moved > before;
                off.flush();
                let mut pm = vello_cpu::Pixmap::new(w, h);
                off.render(&mut pm, &mut self.resources);
                // What the raster is made from, for its object once it moves (`move_shadow`).
                let made =
                    Made { pixels: silhouette(&pm, darkness.is_some()), fx: vec![fx.clone()], size: reach.size(), channels: None, outline: None };
                let (wu, hu) = (w as usize, h as usize);
                let sigma = blur / 2.0 / f.px / scale;
                if let Some(darkness) = darkness {
                    // The object's own colors, blurred, with the black Darkness adds (premultiplied).
                    let keep = 1.0 - shade(f.ink, darkness);
                    let mut planes: [Vec<f32>; 4] = [
                        pm.data().iter().map(|p| p.r as f32).collect(),
                        pm.data().iter().map(|p| p.g as f32).collect(),
                        pm.data().iter().map(|p| p.b as f32).collect(),
                        pm.data().iter().map(|p| p.a as f32).collect(),
                    ];
                    for plane in &mut planes {
                        effects::pixel::blur_plane(plane, wu, hu, sigma);
                    }
                    let [r, g, b, a] = &planes;
                    for (k, px) in pm.data_mut().iter_mut().enumerate() {
                        let al = (a[k] * opacity).clamp(0.0, 255.0).round();
                        let value = |plane: &Vec<f32>| (plane[k] * opacity * keep).round().clamp(0.0, al) as u8;
                        *px = vello_cpu::color::PremulRgba8 { r: value(r), g: value(g), b: value(b), a: al as u8 };
                    }
                } else {
                    // An Outer Glow blurs a solid silhouette: wherever the art has any alpha, as AI files draw a glow round a glow.
                    let solid = matches!(fx, RasterFx::OuterGlow { .. });
                    let mut a: Vec<f32> = pm.data().iter().map(|p| if solid && p.a > 0 { 255.0 } else { p.a as f32 }).collect();
                    effects::pixel::dilate_plane(&mut a, wu, hu, grow / f.px / scale);
                    effects::pixel::blur_plane(&mut a, wu, hu, sigma);
                    let [cr, cg, cb] = f.ink.rgb(&color);
                    let tint = |v: f32, ch: f32| (v * ch).round().clamp(0.0, 255.0) as u8;
                    for (px, av) in pm.data_mut().iter_mut().zip(&a) {
                        let al = (av * opacity).clamp(0.0, 255.0);
                        *px = vello_cpu::color::PremulRgba8 { r: tint(al, cr), g: tint(al, cg), b: tint(al, cb), a: al.round() as u8 };
                    }
                }
                let image = std::sync::Arc::new(pm);
                let anchor = reach.origin();
                let p = f.view * anchor;
                let node = Drawn::of(n, owner);
                let rel = (x0 - p.x, y0 - p.y);
                let view = cropped.then_some(in_view);
                self.shadows.insert(key, ShadowEntry { node, linear, anchor, rel, image: image.clone(), scale, view, made, inputs, moved, stamp });
                self.trim_fx_rasters(FX_RASTER_BUDGET);
                (image, x0, y0, scale, moved, cropped)
            }
        };
        Some(Shadow { image, at: Affine::translate((x, y)) * Affine::scale(scale), mode, moved, cropped })
    }

    /// Draw shadow raster `s` through `to`, from the view the raster was made for to the one `ctx`
    /// draws in. The blend mode applies per draw: a blend layer would be a full-viewport compositing
    /// pass per shadow. A full-size raster drawn as made sits on whole pixels, so the cheapest
    /// sampler is exact.
    fn draw_shadow(&mut self, ctx: &mut RenderContext, s: &Shadow, to: Affine) {
        let (w, h) = (s.image.width() as f64, s.image.height() as f64);
        let at = to * s.at;
        ctx.set_transform(at);
        ctx.set_blend_mode(blend_mode(s.mode));
        let [a, b, c, d, _, _] = at.as_coeffs();
        let exact = (a, b, c, d) == (1.0, 0.0, 0.0, 1.0) && at.translation().x.fract() == 0.0 && at.translation().y.fract() == 0.0;
        let quality = if exact { peniko::ImageQuality::Low } else { peniko::ImageQuality::Medium };
        let sampler = peniko::ImageSampler { quality, ..Default::default() };
        ctx.reset_paint_transform();
        ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(s.image.clone()), sampler });
        ctx.fill_rect(&Rect::new(0.0, 0.0, w, h));
        ctx.set_blend_mode(blend_mode(vectorcraft_doc::color::BlendMode::Normal));
        ctx.set_transform(Affine::IDENTITY);
    }

    /// Drop shadow / outer glow `fx` of what `paint` draws (bounded by `reach`) through a vello
    /// filter layer, when there is no raster for it ([`Self::shadow_raster`]). vello's shadow filter
    /// blurs the soft alpha, so an Outer Glow drawn here lacks the raster's solid silhouette.
    fn draw_shadow_filter(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        fx: &RasterFx,
        reach: Rect,
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) {
        let (mode, opacity, dx, dy, blur, color, darkness) = match fx {
            RasterFx::DropShadow { mode, opacity, dx, dy, blur, color, darkness } => (*mode, *opacity, *dx, *dy, *blur, *color, *darkness),
            RasterFx::OuterGlow { mode, opacity, blur, color } => (*mode, *opacity, 0.0, 0.0, *blur, *color, None),
            _ => return,
        };
        let filter = match darkness {
            Some(_) => shade_filter(blur),
            None => shadow_filter(0.0, 0.0, blur, pcolor(f.ink, &color)),
        };
        // The offset moves the content rather than the filter: vello_cpu drops layer content
        // that lies entirely outside the viewport before filtering.
        let reach = reach + Vec2::new(dx, dy);
        self.with_filters(ctx, f, reach, blur, Some((mode, opacity)), |r, c, fr, comp| {
            c.set_transform(fr.view);
            c.push_layer(None, comp.map(|m| blend_mode(m.0)), comp.map(|m| m.1), None, Some(filter));
            paint(r, c, &shifted(fr, dx, dy));
            // Darkness: black over the object's own colors, keeping their alpha.
            if let Some(black) = darkness.map(|d| shade(f.ink, d)).filter(|d| *d > 0.0) {
                c.set_transform(fr.view);
                c.set_blend_mode(peniko::BlendMode::new(peniko::Mix::Normal, peniko::Compose::SrcAtop));
                c.set_paint(peniko::Color::new([0.0, 0.0, 0.0, black]));
                c.fill_rect(&reach);
                c.set_blend_mode(blend_mode(vectorcraft_doc::color::BlendMode::Normal));
            }
            c.pop_layer();
        });
    }

    /// While interacting, give `n` (shadow-cache key `key`) the shadow or glow raster an earlier
    /// frame made for its object (the same id, effect slot and ink, at this zoom and rotation, with
    /// the same `inputs`) when only the silhouette's place changed after that frame: made by the
    /// same effect `fx` from a silhouette whose bounds have the size of `reach` (the content's
    /// bounds moved by the shadow's offset), and whose alpha (and colors, under Darkness) are those
    /// of `n`'s silhouette drawn into a raster placed as that one was against its anchor
    /// ([`silhouette`]). The raster is kept as `n`'s, marked
    /// moved, and is drawn at the nearest whole pixel.
    #[allow(clippy::too_many_arguments)]
    fn move_shadow(
        &mut self,
        f: &Frame,
        n: &Node,
        key: (usize, usize, Ink),
        fx: &RasterFx,
        reach: Rect,
        owner: Option<&Arc<Node>>,
        inputs: &Inputs,
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) {
        let (dx, dy, colored) = match fx {
            RasterFx::DropShadow { dx, dy, darkness, .. } => (*dx, *dy, darkness.is_some()),
            _ => (0.0, 0.0, false),
        };
        let (linear, stamp, size) = (linear_key(f.view), self.stamp, reach.size());
        let fits = |k: &(usize, usize, Ink), e: &ShadowEntry| {
            (k.1, k.2) == (key.1, key.2)
                && e.stamp < stamp
                && e.linear == linear
                && e.view.is_none()
                && e.node.id() == n.id
                && e.inputs == *inputs
                && e.made.fits(&[fx], size, None, None)
        };
        let Some((old, rel, w, h, scale)) =
            self.shadows.iter().find(|(k, e)| fits(k, e)).map(|(k, e)| (*k, e.rel, e.image.width(), e.image.height(), e.scale))
        else {
            return;
        };
        // The silhouette drawn where the old one lies in its raster.
        let anchor = reach.origin();
        let p = f.view * anchor;
        let mut off = f.offscreen_context(w, h);
        let crop = Frame { mt: false, view: Affine::scale(1.0 / scale) * Affine::translate((-(p.x + rel.0), -(p.y + rel.1))) * f.view, ..*f };
        self.inside_layer(|r| paint(r, &mut off, &shifted(&crop, dx, dy)));
        off.flush();
        let mut pm = vello_cpu::Pixmap::new(w, h);
        off.render(&mut pm, &mut self.resources);
        if self.shadows.get(&old).is_none_or(|e| silhouette(&pm, colored) != e.made.pixels) {
            return;
        }
        if let Some(mut e) = self.shadows.remove(&old) {
            (e.node, e.anchor, e.moved) = (Drawn::of(n, owner), anchor, true);
            self.shadows.insert(key, e);
        }
    }

    /// `draw_shape` for nodes with live effects.
    pub(crate) fn draw_shape_fx(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, bp: &BezPath, rule: FillRule) {
        // The node `draw_arc` is drawing gets its geometry from the cache: `bp` is its own path,
        // made from the node alone, so the effects give the same result every frame while the
        // plug-ins installed stay the same (`check_plugins`).
        let owner = self.fx_owner.as_ref().filter(|o| std::ptr::eq(&***o, n)).cloned();
        let mut cached = owner.map(|o| {
            let key = Arc::as_ptr(&o) as usize;
            let stamp = self.stamp;
            let e = match self.fx_geom.remove(&key) {
                Some(e) if Arc::ptr_eq(&e.node, &o) => e,
                _ => FxGeom { g: Arc::new(effected_path(n, bp)), items: vec![None; n.appearance.items.len()], node: o, stamp },
            };
            (key, FxGeom { stamp, ..e })
        });
        let g = match &cached {
            Some((_, e)) => e.g.clone(),
            None => Arc::new(effected_path(n, bp)),
        };
        self.draw_shape_fx_geom(ctx, f, n, &g, rule, cached.as_mut().map(|(_, e)| &mut e.items));
        if let Some((key, e)) = cached {
            self.fx_geom.insert(key, e);
        }
    }

    /// [`Self::draw_shape_fx`] on the effected geometry `g`, with the items' cached geometry.
    fn draw_shape_fx_geom(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        n: &Node,
        g: &BezPath,
        rule: FillRule,
        items: Option<&mut Vec<Option<Arc<BezPath>>>>,
    ) {
        if g.elements().is_empty() {
            return;
        }
        if f.opts.outline {
            self.hairline(ctx, f, g, [0, 0, 0, 255]);
            return;
        }
        let gctx = GeomContext::of(n);
        let rfx = effects::raster_effects(&n.appearance.effects);
        // Document-space reach of the painted geometry (strokes, arrowheads…).
        let outset = n.appearance.outset();
        let reach = g.bounding_box().inflate(outset, outset);
        let bounds = rasterize_bounds(f.symbols, n, &rfx);
        // Under Rasterize the content reaches as far as its fills' and strokes' raster effects paint.
        let reach = bounds.map_or(reach, |b| reach.union(b));
        let content = Content { node: n, owner: None, cache: Some(0), reach, outline: Some((g, rule)), bounds, kept: None };
        let mut items = items;
        self.raster_fx(ctx, f, &content, &rfx, &mut |r, c, fr| r.paint_items(c, fr, n, g, rule, &gctx, items.as_deref_mut()));
        ctx.set_transform(Affine::IDENTITY);
    }

    /// Draw type, an image, a symbol instance or a live object with object-level effects
    /// ([`has_object_fx`]) inside its transparency group: its art (reshaped by the geometry
    /// effects, see [`effects::reshape`]) with the raster effects painted around it as for paths,
    /// the art standing for the object's silhouette.
    pub(crate) fn draw_object_fx(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>, cache: bool) {
        let opacity = self.opacity_of(a);
        if !f.opts.outline && (opacity < 1.0 || a.blend != vectorcraft_doc::color::BlendMode::Normal || a.isolate) {
            let blends = self.blends_through(a);
            let bounds = if blends { self.bounds_of(f.symbols, a) } else { None };
            let comp = crate::group::Composite { blend: a.blend, opacity, isolated: a.isolate, blends, bounds, ..Default::default() };
            self.group(ctx, f, comp, &mut |r, c, fr| r.draw_object_fx_art(c, fr, a, cache));
        } else {
            self.draw_object_fx_art(ctx, f, a, cache);
        }
        self.stats.drawn += 1;
    }

    /// What [`Self::draw_object_fx`] draws inside the object's transparency group.
    fn draw_object_fx_art(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>, cache: bool) {
        let art = self.fx_art(f.doc, a, cache);
        // Art the cache did not keep is made again in the next frame.
        let kept = cache && self.fx_arts.get(&(Arc::as_ptr(a) as usize), self.stamp).is_some_and(|e| Arc::ptr_eq(&e.1, &art));
        let outer = self.made_again;
        self.made_again = outer || !kept;
        // With Crop Marks, the object inside the art paints its own raster effects.
        let rfx = if effects::has_crop_marks(a) { vec![] } else { effects::raster_effects(&a.appearance.effects) };
        match cull_bounds(f.symbols, &art) {
            Some(reach) if !f.opts.outline && !rfx.is_empty() => {
                // Feather, Inner Glow and Rasterize with Create Clipping Mask clip to the art's outline.
                let clipped = rfx.iter().any(|x| {
                    matches!(x, RasterFx::Feather { .. } | RasterFx::InnerGlow { .. }) || matches!(x, RasterFx::Rasterize(r) if r.clipping_mask)
                });
                let region = if clipped { self.clip_of(&art) } else { None };
                // A symbol's art can change while the instance doesn't: no cached shadows.
                let cache = (cache && !matches!(a.kind, NodeKind::SymbolInstance { .. })).then_some(0);
                let bounds = rasterize_bounds(f.symbols, a, &rfx);
                let reach = bounds.map_or(reach, |b| reach.union(b));
                let content = Content { node: a, owner: Some(a), cache, reach, outline: region.as_deref().map(|r| (&r.0, r.1)), bounds, kept: None };
                self.raster_fx(ctx, f, &content, &rfx, &mut |r, c, fr| r.draw_fx_art(c, fr, a, &art));
                ctx.set_transform(Affine::IDENTITY);
            }
            _ => self.draw_fx_art(ctx, f, a, &art),
        }
        self.made_again = outer;
    }

    /// The art [`Self::draw_object_fx`] draws for `a` (cached by `Arc` identity when `cache`):
    /// reshaped by its geometry effects, else type, images and paths as themselves and the others
    /// as their evaluated art (groups and layers: [`effects::evaluate_container`], keeping their
    /// knockout setting), recoloured by the colour adjustments in it ([`Self::adjusted_art`]);
    /// without the object's transparency and effects (a path keeps its geometry effects). Art that
    /// is made from live objects it evaluated ([`evaluates_live`]), or from type, symbol art or
    /// colors shaded through the color settings ([`reads_outside`]), is kept only while what it
    /// reads from outside `a` stays the same ([`Self::live_found`]).
    fn fx_art(&mut self, doc: &vectorcraft_doc::Document, a: &Arc<Node>, cache: bool) -> Arc<Node> {
        let key = Arc::as_ptr(a) as usize;
        let mut kept = None;
        if cache
            && let Some((node, art, found)) = self.fx_arts.get(&key, self.stamp)
            && Arc::ptr_eq(node, a)
        {
            match found {
                None => return art.clone(),
                Some(was) => kept = Some((art.clone(), was.0)),
            }
        }
        let found = (kept.is_some() || (cache && (evaluates_live(a) || reads_outside(a)))).then(|| self.live_found(doc, a));
        if let Some((art, was)) = kept
            && found.as_ref().is_some_and(|f| f.0 == was)
        {
            return art;
        }
        let symbol = match &a.kind {
            NodeKind::SymbolInstance { symbol, .. } => {
                doc.symbols.iter().find(|s| s.name == *symbol).map(|s| crate::brush_fx::instance_art(&s.art, a))
            }
            _ => None,
        };
        let container = is_container(a);
        let art = match effects::crop_marks_art(a)
            .or_else(|| effects::revolve_art(a))
            .or_else(|| effects::solid_art(a))
            .or_else(|| effects::reshape(Some(doc), a, symbol.as_ref()))
        {
            Some(r) => r,
            None if container => effects::evaluate_container(Some(doc), a).unwrap_or_else(|| (**a).clone()),
            None if matches!(a.kind, NodeKind::Text(_) | NodeKind::Image(_) | NodeKind::Path { .. } | NodeKind::Compound { .. }) => (**a).clone(),
            None => effects::outline_art(Some(doc), a, symbol.as_ref()).unwrap_or_else(|| Node::group(a.id, vec![])),
        };
        let art = if effects::has_revolve(a) || effects::has_solid(a) { effects::evaluate_container(Some(doc), &art).unwrap_or(art) } else { art };
        let mut art = self.adjusted_art(art, effects::color_map(&a.appearance.effects).as_ref());
        // A path's geometry effects still apply when it is drawn.
        let path = matches!(art.kind, NodeKind::Path { .. } | NodeKind::Compound { .. });
        art.appearance.effects.retain(|e| path && effects::is_geometry(&e.id));
        let mut art = art.with_mask(None);
        art.opacity = 1.0;
        art.blend = Default::default();
        art.isolate = false;
        // One element: its pieces never knock each other out (a group's members still do as its
        // knockout setting says).
        art.knockout = if container { a.knockout } else { vectorcraft_doc::Knockout::Off };
        let art = Arc::new(art);
        if cache {
            let bytes = crate::lru::node_bytes(&art);
            self.fx_arts.insert(key, (a.clone(), art.clone(), found), bytes, self.stamp);
        }
        art
    }

    /// Draw `art`, the art [`Self::fx_art`] made for `a`. Type art that `fx_arts` keeps for `a` is
    /// drawn as `fx_owner`, so [`Self::draw_text`] takes its glyph geometry from `texts`.
    fn draw_fx_art(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>, art: &Arc<Node>) {
        let kept =
            matches!(art.kind, NodeKind::Text(_)) && self.fx_arts.get(&(Arc::as_ptr(a) as usize), self.stamp).is_some_and(|e| Arc::ptr_eq(&e.1, art));
        if !kept {
            return self.draw_node(ctx, f, art, true);
        }
        let outer = self.fx_owner.replace(art.clone());
        self.draw_node(ctx, f, art, true);
        self.fx_owner = outer;
    }

    /// `art` with the colour adjustments inside it applied, then `outer` (its object's own) on
    /// everything ([`effects::adjust`]). Embedded images become recoloured copies, cached with the
    /// decoded images under a key naming the adjustment.
    fn adjusted_art(&mut self, mut art: Node, outer: Option<&effects::ColorMap>) -> Node {
        art.appearance.effects.retain(|e| !effects::is_adjustment(&e.id));
        let mut wanted: Vec<(String, String, u64, effects::ColorMap)> = vec![];
        let adjusted = effects::adjust(&art, outer, &mut |key, map| {
            let mut h = std::hash::DefaultHasher::new();
            std::hash::Hash::hash(&format!("{map:?}"), &mut h);
            let hash = std::hash::Hasher::finish(&h);
            let derived = format!("{key}\u{0}adjust\u{0}{hash:016x}");
            wanted.push((key.to_string(), derived.clone(), hash, map.clone()));
            Some(derived)
        });
        for (key, derived, hash, map) in wanted {
            self.adjusted_image(&key, derived, hash, &map);
        }
        adjusted.unwrap_or(art)
    }

    /// Paint `content` with raster effects `rfx` (see the module docs): shadows and outer glows
    /// below it, the content blurred or feathered, inner glows above it (clipped to its outline).
    /// `paint` draws the content in the frame it is given.
    fn raster_fx(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        content: &Content,
        rfx: &[RasterFx],
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) {
        // Only the last MAX_RASTERIZE Rasterize effects are drawn.
        let count = rfx.iter().filter(|fx| matches!(fx, RasterFx::Rasterize(_))).count();
        let capped: Vec<RasterFx>;
        let rfx = if count > MAX_RASTERIZE {
            let mut left_out = count - MAX_RASTERIZE;
            capped = rfx
                .iter()
                .filter(|fx| {
                    let out = left_out > 0 && matches!(fx, RasterFx::Rasterize(_));
                    left_out -= usize::from(out);
                    !out
                })
                .cloned()
                .collect();
            &capped[..]
        } else {
            rfx
        };
        // Rasterize at the document's raster effects resolution takes it here.
        let resolved: Vec<RasterFx>;
        let rfx = if rfx.iter().any(|fx| matches!(fx, RasterFx::Rasterize(r) if r.ppi.is_none())) {
            let ppi = doc_raster_ppi(f.doc);
            resolved = rfx.iter().map(|fx| fx.at_document_ppi(ppi)).collect();
            &resolved[..]
        } else {
            rfx
        };
        // Rasterize makes an image of everything the effects before it paint, shadows and glows
        // included, and the shadows and glows after it paint around that image. When some come
        // after the last Rasterize, the stack splits after it, else when any effect comes before
        // it, the stack splits before it; the effects after the split paint around the content
        // drawn with those before it. Rasterize then comes first in its part.
        if let Some(at) = rfx.iter().rposition(|fx| matches!(fx, RasterFx::Rasterize(_))) {
            let paints_around = |fx: &[RasterFx]| fx.iter().any(|fx| !content_changing(&fx));
            let (_, rest) = rfx.split_at(at + 1);
            let split = if paints_around(rest) { Some(at + 1) } else { (at > 0).then_some(at) };
            if let Some(split) = split {
                let (before, after) = rfx.split_at(split);
                let line = 72.0 / doc_raster_ppi(f.doc);
                let (mut reach, mut spread) = (content.reach, content.bounds);
                // After a Rasterize on white without a clipping mask, the silhouette is its image,
                // and it stays the silhouette through the effects after it (as in `pixel_content`).
                let mut image = None;
                // Whether the image of a Rasterize before the split is over `rasterize_image`'s
                // limits. Such an image is kept only as far as the view needs it, so the effects
                // after the split are not cached.
                let (mut over, mut last_over) = (false, false);
                for fx in before {
                    if let RasterFx::Rasterize(r) = fx {
                        last_over = image_size(r, r.image(spread.unwrap_or(reach), line), line).is_none();
                        over |= last_over;
                    }
                    let o = fx.outset(reach);
                    reach = reach.inflate(o, o);
                    spread = spread.map(|b| fx.spread(b, line));
                    if let RasterFx::Rasterize(r) = fx
                        && r.white
                        && !r.clipping_mask
                    {
                        image = Some(spread.unwrap_or(reach).to_path(0.1));
                    }
                }
                // When the split comes after the Rasterize, the shadows and glows after it and the
                // content each draw the content before the split. The first draw makes the image,
                // and the others draw the one it keeps.
                let window = after.iter().fold(f.visible, |w, fx| {
                    let o = fx.outset(w);
                    w.inflate(o, o)
                });
                let frame = last_over.then_some((f.view, f.px));
                let kept = (split > at).then(|| Kept { window, frame, image: std::cell::RefCell::new(None) });
                let inner = Content {
                    node: content.node,
                    owner: content.owner,
                    cache: None,
                    reach: content.reach,
                    outline: content.outline,
                    bounds: content.bounds,
                    kept: kept.as_ref(),
                };
                let outline = image.as_ref().map(|g| (g, FillRule::NonZero)).or(content.outline);
                // When the split comes before the Rasterize, the part from it on draws the content
                // once, and its image is the one kept for a split above.
                let outer = Content {
                    node: content.node,
                    owner: content.owner,
                    cache: content.cache.filter(|_| !over),
                    reach,
                    outline,
                    bounds: spread,
                    kept: content.kept.filter(|_| split == at),
                };
                self.raster_fx(ctx, f, &outer, after, &mut |r, c, fr| r.raster_fx(c, fr, &inner, before, paint));
                return;
            }
        }
        let reach = content.reach;
        // The `Arc` drawn: the caller's, or the node `draw_arc` is drawing.
        let owner = content.owner.cloned().or_else(|| self.fx_owner.clone().filter(|o| std::ptr::eq(&**o, content.node)));
        // Below the content: shadows and outer glows, each of the content with the ones before it in the stack, as AI
        // files draw a glow round a glow and the SVG export nests them. Their rasters are made first to last, each once
        // and drawn into the later ones' silhouettes as an image (so n of them cost n rasters, not 2^n draws), then
        // drawn with the last lowest. One without a raster (an object drawn uncached, a glow past the budget) is drawn
        // through a filter layer from the content alone, and goes into the later silhouettes that way.
        let mut made: Vec<(&RasterFx, Option<Shadow>)> = vec![];
        let mut stacked = reach;
        let screen = Rect::new(0.0, 0.0, f64::from(ctx.width()), f64::from(ctx.height()));
        // How far (document units) a shadow or glow looks from a pixel to the silhouette under it.
        let looks = |fx: &RasterFx| match fx {
            RasterFx::DropShadow { dx, dy, blur, .. } => blur.max(0.0) * 1.5 + dx.abs() + dy.abs(),
            RasterFx::OuterGlow { blur, .. } => blur.max(0.0) * 1.5,
            _ => 0.0,
        };
        for (i, fx) in rfx.iter().enumerate().filter(|(_, x)| x.is_below()) {
            // A raster cropped to the view covers what the shadows and glows stacked over it read.
            let around: f64 = rfx.iter().skip(i + 1).filter(|x| x.is_below()).map(looks).sum();
            let over_cropped = made.iter().any(|(_, s)| s.as_ref().is_some_and(|s| s.cropped));
            let shadow = {
                let earlier = &made;
                let view = f.view;
                let mut art = |r: &mut Self, c: &mut RenderContext, fr: &Frame| {
                    let to = fr.view * view.inverse();
                    for (fx, s) in earlier.iter().rev() {
                        match s {
                            Some(s) => {
                                r.stats.moved += usize::from(s.moved);
                                r.draw_shadow(c, s, to);
                            }
                            None => r.draw_shadow_filter(c, fr, fx, reach, paint),
                        }
                    }
                    paint(r, c, fr);
                };
                let view = (screen, around, over_cropped);
                content.cache.and_then(|slot| self.shadow_raster(f, content.node, slot + i, fx, stacked, owner.as_ref(), view, &mut art))
            };
            stacked = match fx {
                RasterFx::DropShadow { dx, dy, blur, .. } => stacked.union((stacked + Vec2::new(*dx, *dy)).inflate(blur * 1.5, blur * 1.5)),
                _ => stacked.inflate(fx.outset(stacked), fx.outset(stacked)),
            };
            made.push((fx, shadow));
        }
        for (fx, s) in made.iter().rev() {
            match s {
                Some(s) => self.draw_shadow(ctx, s, Affine::IDENTITY),
                None => self.draw_shadow_filter(ctx, f, fx, reach, paint),
            }
        }
        // The content itself (blurred / feathered / filtered), on its pixels: a filter layer's blur
        // works on a coarser grid tied to the canvas, so the blurred art shifted by up to a pixel
        // and looked blocky as the view moved by parts of a pixel (#755).
        let blurs = rfx.iter().any(|fx| matches!(fx, RasterFx::Feather { radius } | RasterFx::GaussianBlur { radius } if *radius > 0.0));
        if blurs || rfx.iter().any(|fx| matches!(fx, RasterFx::Pixel(_) | RasterFx::Rasterize(_))) {
            self.pixel_content(ctx, f, content, rfx, paint);
        } else {
            paint(self, ctx, f);
        }
        // Above the content: inner glows, clipped to its outline.
        let Some((g, rule)) = content.outline else { return };
        for fx in rfx {
            let RasterFx::InnerGlow { mode, opacity, blur, color, center } = fx else { continue };
            let filter = shadow_filter(0.0, 0.0, *blur, pcolor(f.ink, color));
            self.with_filters(ctx, f, g.bounding_box(), *blur, Some((*mode, *opacity)), |_, c, fr, comp| {
                // The blend layer goes outside the clip: a clip layer is isolated, so a blend
                // inside it would mix with nothing instead of the content below.
                if let Some((m, o)) = comp {
                    c.set_transform(Affine::IDENTITY);
                    c.push_layer(None, Some(blend_mode(m)), Some(o), None, None);
                }
                c.set_transform(fr.view);
                c.set_fill_rule(fill_rule(rule));
                c.push_clip_layer(g);
                c.push_layer(None, None, None, None, Some(filter));
                c.set_paint(peniko::Color::BLACK);
                if *center {
                    c.set_fill_rule(fill_rule(rule));
                    c.fill_path(g);
                } else {
                    // Everything outside the shape, so the glow bleeds in from the edges.
                    let pad = blur * 2.0 + 4.0 * fr.px;
                    let mut inv = g.bounding_box().inflate(pad, pad).to_path(0.1);
                    inv.extend(g.iter());
                    c.set_fill_rule(peniko::Fill::EvenOdd);
                    c.fill_path(&inv);
                }
                c.pop_layer();
                c.pop_layer();
                if comp.is_some() {
                    c.pop_layer();
                }
            });
        }
    }

    /// Paint `content` through the content-changing effects of `rfx` (Gaussian Blur, Feather, the
    /// Photoshop-style filters and Rasterize) in stack order on its pixels: rendered offscreen over
    /// all it can reach, filtered on the CPU and drawn back. A Rasterize first in the chain draws the
    /// content on its image's own grid instead ([`Self::rasterize_image`]) when that image is within
    /// the limits below; content with a [`Kept`] image draws that image after its first draw. The
    /// raster holds at most [`PIXEL_BUDGET`] pixels and [`crate::MAX_RASTER_SIDE`] a side: content
    /// over the budget is cropped to the view (content with a [`Kept`] image to its window) when
    /// every effect only looks nearby, and content still over either limit is rendered at a lower
    /// resolution. A whole raster at full resolution is cached while the object, the
    /// zoom and what the content reads ([`Inputs`]) stay the same (panning only moves it), and
    /// while interacting after the content only moved ([`Self::move_pixel_content`]). A cropped or
    /// lower-resolution raster is cached while the object, the view (its pan included) and what the
    /// content reads stay the same ([`PixelEntry::view`]); a lower-resolution one is kept resampled to
    /// the view's pixels ([`Self::resampled`]).
    /// While dragging, content without such a raster to draw renders within [`DRAG_PIXEL_BUDGET`]
    /// pixels and is not cached.
    fn pixel_content(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        content: &Content,
        rfx: &[RasterFx],
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) {
        let chain: Vec<&RasterFx> = rfx.iter().filter(content_changing).collect();
        // When the first draw of this content kept its raster, that raster is drawn.
        if let Some(KeptImage::Raster(image, place)) = content.kept.and_then(|k| k.image.borrow().clone()) {
            draw_placed(ctx, image, f.view * place);
            return;
        }
        // Content kept with an image over the limits is drawn in the split's frame, over where the
        // effects after the split take it from, then placed in this one.
        let call = f.view;
        let split_frame;
        let (f, screen) = match content.kept.and_then(|k| k.frame.map(|frame| (k.window, frame))) {
            Some((window, (view, px))) => {
                split_frame = Frame { view, px, ..*f };
                (&split_frame, view.transform_rect_bbox(window))
            }
            None => (f, Rect::new(0.0, 0.0, ctx.width() as f64, ctx.height() as f64)),
        };
        // Where the result can paint: each effect spreads what the ones before it painted.
        let reach = chain.iter().fold(content.reach, |r, fx| {
            let o = fx.outset(r);
            r.inflate(o, o)
        });
        let full = f.view.transform_rect_bbox(reach).inflate(2.0, 2.0);
        if !(full.area().is_finite() && full.x1 > screen.x0 && full.y1 > screen.y0 && full.x0 < screen.x1 && full.y0 < screen.y1) {
            return;
        }
        let fits = full.area() <= PIXEL_BUDGET;
        // Device pixels from a pixel to the content deciding it (`None`: anywhere).
        let margin = chain.iter().try_fold(2.0, |m, fx| looks_around(fx).map(|d| m + d / f.px));
        let region = match margin {
            Some(m) if !fits => full.intersect(screen.inflate(m, m)),
            _ => full,
        };
        let (x0, y0) = (region.x0.floor(), region.y0.floor());
        let (rw, rh) = ((region.x1 - x0).max(1.0), (region.y1 - y0).max(1.0));
        // (vello_cpu panics drawing near `u16::MAX` pixels a side.)
        let side = f64::from(crate::MAX_RASTER_SIDE);
        let k = (PIXEL_BUDGET / (rw * rh)).sqrt().min(side / rw).min(side / rh).min(1.0);
        let (w, h) = ((rw * k).ceil().clamp(1.0, side), (rh * k).ceil().clamp(1.0, side));
        // The cached raster: of the whole content at full size at this zoom, else of this view.
        let linear = linear_key(f.view);
        let whole = fits && k >= 1.0;
        let in_view = (!whole).then(|| ViewCrop::of(f.view, screen));
        let key = content.cache.map(|slot| (content.node as *const Node as usize, slot, f.ink));
        let stamp = self.stamp;
        let channels = match f.ink {
            Ink::Cmy => effects::pixel::Channels::CmyPlane,
            Ink::K => effects::pixel::Channels::KPlane,
            Ink::Display | Ink::RichBlack if f.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk => effects::pixel::Channels::Cmyk,
            Ink::Display | Ink::RichBlack => effects::pixel::Channels::Rgb,
        };
        // De-Interlace's field lines: the rows of the document's raster grid (1 to 2400 ppi).
        let ppi = doc_raster_ppi(f.doc);
        // What the content reads (as the raster kept for it found it, else found now), with the
        // channels the effects run on and, under De-Interlace or Rasterize, the resolution.
        let inputs = key.map(|key| {
            let reads = self.pixel_fx.get(&key).filter(|e| e.node.is(content.node)).map_or_else(|| Reads::of(content.node), |e| e.inputs.reads);
            let interlaced = chain.iter().any(|fx| matches!(fx, RasterFx::Pixel(PixelFx::DeInterlace { .. }) | RasterFx::Rasterize(_)));
            self.raster_inputs(f, content.node, reads).with((std::mem::discriminant(&channels), interlaced.then_some(ppi.to_bits())))
        });
        let interacting = f.opts.interacting;
        let in_place = |e: &PixelEntry| match e.view {
            None => whole,
            Some(v) => !whole && v.serves(f.view, screen),
        };
        let exact = |e: &PixelEntry| {
            e.linear == linear && in_place(e) && e.node.is(content.node) && Some(&e.inputs) == inputs.as_ref() && (interacting || !e.moved)
        };
        if interacting
            && whole
            && let Some(key) = key
            && let Some(inputs) = &inputs
            && !self.pixel_fx.get(&key).is_some_and(exact)
        {
            self.move_pixel_content(f, content, key, &chain, channels, inputs, paint);
        }
        if let Some(e) = key.and_then(|key| self.pixel_fx.get_mut(&key)).filter(|e| exact(e)) {
            e.stamp = stamp;
            self.stats.moved += usize::from(e.moved);
            let p = f.view * e.anchor;
            let (image, at, k) = (e.image.clone(), ((p.x + e.rel.0).round(), (p.y + e.rel.1).round()), e.k);
            crate::paint::add_decode_failures(e.failed);
            draw_scaled(ctx, image, at, k);
            return;
        }
        // During a drag, content without a raster to draw renders within the drag's budget and keeps
        // no raster (#2558).
        let (k, w, h, key) = match drag_scale(f.opts.dragging, (rw, rh), k) {
            Some(d) => {
                self.stats.moved += 1;
                self.stats.previewed += 1;
                (d, (rw * d).ceil().clamp(1.0, side), (rh * d).ceil().clamp(1.0, side), None)
            }
            None => (k, w, h, key),
        };
        let (wu, hu) = (w as u16, h as u16);
        let view = Affine::scale(k) * Affine::translate((-x0, -y0)) * f.view;
        let to_doc = view.inverse();
        let px = f.px / k;
        let frame = Frame { mt: false, view, visible: to_doc.transform_rect_bbox(Rect::new(0.0, 0.0, w, h)), px, ..*f };
        let line = 72.0 / ppi;
        let (before, decoded) = (self.stats.moved, crate::paint::decode_failures());
        // A Rasterize first in the chain draws the content on its image's own grid instead, unless
        // that image is too large: the content is drawn once.
        let mut rasterized = match chain.first() {
            Some(RasterFx::Rasterize(r)) => {
                let rect = r.image(content.bounds.unwrap_or(content.reach), line);
                let kept = content.kept.and_then(|k| match &*k.image.borrow() {
                    Some(KeptImage::Grid(image, failed)) => {
                        crate::paint::add_decode_failures(*failed);
                        Some(image.clone())
                    }
                    _ => None,
                });
                let image = kept.or_else(|| {
                    let failures = crate::paint::decode_failures();
                    let image = self.rasterize_image(f, r, rect, paint).map(std::rc::Rc::new);
                    if let (Some(k), Some(image)) = (content.kept, &image) {
                        *k.image.borrow_mut() = Some(KeptImage::Grid(image.clone(), crate::paint::decode_failures() - failures));
                    }
                    image
                });
                image.map(|image| (image, rect))
            }
            _ => None,
        };
        let mut pm = vello_cpu::Pixmap::new(wu, hu);
        if rasterized.is_none() {
            #[cfg(test)]
            if matches!(chain.first(), Some(RasterFx::Rasterize(_))) {
                RASTERIZED.with(|n| n.set(n.get() + 1));
            }
            let mut off = f.offscreen_context(wu, hu);
            if hard_edged(&chain, px) {
                off.set_aliasing_threshold(crate::AntiAlias::None.threshold());
            }
            self.inside_layer(|r| paint(r, &mut off, &frame));
            off.flush();
            off.render(&mut pm, &mut self.resources);
        }
        let failed = crate::paint::decode_failures() - decoded;
        // Content drawn over moved rasters makes a moved raster.
        let moved = self.stats.moved > before;
        // What the raster is made from, for its object once it moves (`move_pixel_content`): a
        // cropped or coarser raster never moves.
        let made = key.map(|_| Made {
            pixels: if whole { pm.data_as_u8_slice().to_vec() } else { Vec::new() },
            fx: chain.iter().map(|fx| (*fx).clone()).collect(),
            size: content.reach.size(),
            channels: Some(channels),
            outline: feather_outline(&chain, content, content.reach.origin()),
        });
        let (center, extent) = (content.reach.center(), 0.5 * content.reach.width().hypot(content.reach.height()));
        let space = effects::PixelSpace { to_doc, px, center, channels, line, extent };
        let (wz, hz) = (wu as usize, hu as usize);
        // What the content covers after the effects run so far: Rasterize makes its image over it.
        let mut bounds = content.bounds.unwrap_or(content.reach);
        // The silhouette Feather and Create Clipping Mask clip to: the content's, or after a Rasterize
        // on white without a clipping mask, its image.
        let mut image_outline: Option<BezPath> = None;
        for (i, fx) in chain.into_iter().enumerate() {
            let before = bounds;
            bounds = fx.spread(before, space.line);
            let outline = image_outline.as_ref().map(|g| (g, FillRule::NonZero)).or(content.outline);
            match fx {
                RasterFx::GaussianBlur { radius } => effects::pixel::gaussian_rgba(pm.data_as_u8_slice_mut(), wz, hz, radius / 2.0 / px),
                RasterFx::Feather { radius } => {
                    effects::pixel::gaussian_rgba(pm.data_as_u8_slice_mut(), wz, hz, radius / 2.0 / px);
                    if let Some((g, rule)) = outline {
                        self.clip_pixels(&mut pm, &frame, g, rule);
                    }
                }
                RasterFx::Pixel(p) => p.apply(pm.data_as_u8_slice_mut(), wz, hz, &space),
                RasterFx::Rasterize(r) => {
                    match rasterized.take().filter(|_| i == 0) {
                        Some((image, rect)) => {
                            let (pixels, cols, rows) = &*image;
                            r.compose(pm.data_as_u8_slice_mut(), wz, hz, &space, pixels, *cols, *rows, rect)
                        }
                        None => r.apply(pm.data_as_u8_slice_mut(), wz, hz, &space, before),
                    }
                    if let Some((g, rule)) = outline.filter(|_| r.clipping_mask) {
                        self.clip_pixels(&mut pm, &frame, g, rule);
                    }
                    if r.white && !r.clipping_mask {
                        image_outline = Some(bounds.to_path(0.1));
                    }
                }
                _ => {}
            }
        }
        let image = std::sync::Arc::new(pm);
        // A raster kept for this view alone at a lower resolution than the view's is resampled to the
        // view's pixels once: each frame that draws it then copies it pixel for pixel, where sampling
        // it again across the whole view cost several times as much as a plain fill.
        let view_raster = key.is_some() && in_view.is_some() && k < 1.0 && content.kept.is_none();
        let (image, x0, y0, k) = match view_raster.then(|| self.resampled(f, image.clone(), (x0, y0), k, screen)).flatten() {
            Some((image, x, y)) => (image, x, y, 1.0),
            None => (image, x0, y0, k),
        };
        // A cancelled export drew part of the content: that raster isn't kept for later frames.
        if !vectorcraft_doc::cancel::is_cancelled()
            && let Some(((key, made), inputs)) = key.zip(made).zip(inputs)
        {
            let anchor = content.reach.origin();
            let p = f.view * anchor;
            let rel = (x0 - p.x, y0 - p.y);
            let owner = content.owner.cloned().or_else(|| self.fx_owner.clone().filter(|o| std::ptr::eq(&**o, content.node)));
            let node = Drawn::of(content.node, owner.as_ref());
            let entry = PixelEntry { node, linear, anchor, rel, k, image: image.clone(), view: in_view, made, inputs, moved, failed, stamp };
            self.pixel_fx.insert(key, entry);
            self.trim_fx_rasters(FX_RASTER_BUDGET);
        }
        // An image over the limits was drawn at a frame's resolution: its raster is kept.
        match content.kept.filter(|kept| kept.image.borrow().is_none()) {
            Some(kept) => {
                let place = f.view.inverse() * Affine::translate((x0, y0)) * Affine::scale(1.0 / k);
                *kept.image.borrow_mut() = Some(KeptImage::Raster(image.clone(), place));
                if f.view == call { draw_scaled(ctx, image, (x0, y0), k) } else { draw_placed(ctx, image, call * place) }
            }
            None => draw_scaled(ctx, image, (x0, y0), k),
        }
    }

    /// `image` as [`draw_scaled`] draws it with its top-left at (`x`, `y`) device pixels, `1 / k`
    /// device pixels a pixel, resampled to the device pixels it covers of `screen`, with their
    /// top-left. `None` when it covers none of them.
    fn resampled(
        &mut self,
        f: &Frame,
        image: std::sync::Arc<vello_cpu::Pixmap>,
        (x, y): (f64, f64),
        k: f64,
        screen: Rect,
    ) -> Option<(std::sync::Arc<vello_cpu::Pixmap>, f64, f64)> {
        let covered = Rect::new(x, y, x + f64::from(image.width()) / k, y + f64::from(image.height()) / k).intersect(screen);
        let (x0, y0) = (covered.x0.floor(), covered.y0.floor());
        let (w, h) = (covered.x1.ceil() - x0, covered.y1.ceil() - y0);
        let side = f64::from(crate::MAX_RASTER_SIDE);
        if !((1.0..=side).contains(&w) && (1.0..=side).contains(&h)) {
            return None;
        }
        let (w, h) = (w as u16, h as u16);
        let mut off = f.offscreen_context(w, h);
        draw_scaled(&mut off, image, (x - x0, y - y0), k);
        off.flush();
        let mut pm = vello_cpu::Pixmap::new(w, h);
        off.render(&mut pm, &mut self.resources);
        Some((std::sync::Arc::new(pm), x0, y0))
    }

    /// While interacting, give `content` (cache key `key`) the filtered raster an earlier frame
    /// made at full size for its object (the same id, slot and ink, at this zoom and rotation, with
    /// the same `inputs`) when only the content's place changed after that frame: made by the same
    /// effects `chain`, told of the same `channels`, from content whose bounds have the same size
    /// (and the same Feather outline against its anchor), and whose pixels are the pixels of
    /// `content` drawn into a raster placed as that one was against its anchor. The raster is kept
    /// as `content`'s, marked moved, and is drawn at the nearest whole pixel. Content under
    /// De-Interlace, or under a Rasterize first in the chain, takes none.
    #[allow(clippy::too_many_arguments)]
    fn move_pixel_content(
        &mut self,
        f: &Frame,
        content: &Content,
        key: (usize, usize, Ink),
        chain: &[&RasterFx],
        channels: Channels,
        inputs: &Inputs,
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) {
        if chain.iter().any(|fx| matches!(fx, RasterFx::Pixel(PixelFx::DeInterlace { .. }))) || matches!(chain.first(), Some(RasterFx::Rasterize(_)))
        {
            return;
        }
        let (linear, stamp, anchor) = (linear_key(f.view), self.stamp, content.reach.origin());
        let outline = feather_outline(chain, content, anchor);
        let fits = |k: &(usize, usize, Ink), e: &PixelEntry| {
            (k.1, k.2) == (key.1, key.2)
                && e.stamp < stamp
                && e.linear == linear
                && e.k >= 1.0
                && e.view.is_none()
                && e.node.id() == content.node.id
                && e.inputs == *inputs
                && e.made.fits(chain, content.reach.size(), Some(channels), outline.as_ref())
        };
        let Some((old, rel, w, h)) = self.pixel_fx.iter().find(|(k, e)| fits(k, e)).map(|(k, e)| (*k, e.rel, e.image.width(), e.image.height()))
        else {
            return;
        };
        // The content drawn where the old content lies in its raster.
        let p = f.view * anchor;
        let view = Affine::translate((-(p.x + rel.0), -(p.y + rel.1))) * f.view;
        let visible = view.inverse().transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        let frame = Frame { mt: false, view, visible, ..*f };
        let mut off = f.offscreen_context(w, h);
        if hard_edged(chain, f.px) {
            off.set_aliasing_threshold(crate::AntiAlias::None.threshold());
        }
        self.inside_layer(|r| paint(r, &mut off, &frame));
        off.flush();
        let mut pm = vello_cpu::Pixmap::new(w, h);
        off.render(&mut pm, &mut self.resources);
        if self.pixel_fx.get(&old).is_none_or(|e| e.made.pixels.as_slice() != pm.data_as_u8_slice()) {
            return;
        }
        let owner = content.owner.cloned().or_else(|| self.fx_owner.clone().filter(|o| std::ptr::eq(&**o, content.node)));
        if let Some(mut e) = self.pixel_fx.remove(&old) {
            (e.node, e.anchor, e.moved) = (Drawn::of(content.node, owner.as_ref()), anchor, true);
            self.pixel_fx.insert(key, e);
        }
    }

    /// The inputs a raster of `n`'s content, which reads `reads`, is drawn with in frame `f`: the
    /// color settings, the edge smoothing, the document's raster effects resolution (which a
    /// Rasterize or De-Interlace anywhere in the content follows), the knockout `n` inherits and,
    /// while `n` holds the isolated group, isolation mode; the pattern definitions, symbols and
    /// brush library it reads; for images and placed documents their pixels and files, the
    /// layer's Dim Images and the options they are drawn with; for type the fonts, whether
    /// overprinting shows ([`crate::proof::overprints`]) and Document Setup's substitution
    /// highlights; and the plug-ins installed, when an object in it has a plug-in effect, and
    /// when it shows symbol or pattern art and an object in the document's symbol or pattern art
    /// has one.
    pub(crate) fn raster_inputs(&mut self, f: &Frame, n: &Node, reads: Reads) -> Inputs {
        let doc = self.doc_inputs(f.doc);
        let mut h = DefaultHasher::new();
        (doc.cms, f.opts.anti_alias.threshold(), self.knockout, f.doc.raster_effects_ppi.to_bits()).hash(&mut h);
        // Cached effects on containers can include paths made into guides.
        f.opts.hide_guides.hash(&mut h);
        if let Some((isolated, path)) = self.isolation_path(f)
            && n.id != isolated
            && path.contains(&n.id)
        {
            isolated.hash(&mut h);
        }
        if reads.patterns {
            doc.patterns.hash(&mut h);
        }
        if reads.symbols {
            doc.symbols.hash(&mut h);
        }
        if reads.images {
            let o = f.opts;
            (doc.images, self.dim_images.map(f32::to_bits), o.smooth_images, o.image_mips, o.trace_views, o.progressive_placed).hash(&mut h);
            o.trace_source.as_deref().hash(&mut h);
        }
        if reads.brushes {
            self.brushes.library_key(f.doc, self.stamp).hash(&mut h);
        }
        if reads.text {
            let setup = &f.doc.setup;
            let highlights = f.opts.highlight_substitutions.then_some((setup.highlight_substituted_fonts, setup.highlight_substituted_glyphs));
            (vectorcraft_text::FontDb::global().generation(), crate::proof::overprints(f.opts), highlights).hash(&mut h);
        }
        if reads.plugins || (doc.plugins && (reads.symbols || reads.patterns)) {
            effects::plugin_revision().hash(&mut h);
        }
        Inputs { reads, key: h.finish(), _doc: doc }
    }

    /// [`DocInputs`] for document `doc`: found once a frame (and again for another document drawn
    /// in it, such as a placed document's own), and kept while they stay the same.
    pub(crate) fn doc_inputs(&mut self, doc: &Document) -> Arc<DocInputs> {
        let at = (self.stamp, std::ptr::from_ref(doc) as usize);
        if let Some((seen, d)) = &self.doc_inputs
            && *seen == at
        {
            return d.clone();
        }
        let cms = vectorcraft_color::cms::active();
        let hash = |each: &mut dyn FnMut(&mut DefaultHasher)| {
            let mut h = DefaultHasher::new();
            each(&mut h);
            h.finish()
        };
        let address = |a: &Arc<Node>| Arc::as_ptr(a) as usize;
        let patterns = hash(&mut |h| doc.patterns.iter().for_each(|p| crate::pattern::hash_tiling(p, h)));
        let symbols = hash(&mut |h| doc.symbols.iter().for_each(|s| (&s.name, address(&s.art)).hash(h)));
        let images = hash(&mut |h| {
            for (key, blob) in &doc.images {
                (key, Arc::as_ptr(&blob.bytes) as usize, blob.proxy.as_ref().map(|p| Arc::as_ptr(p) as usize)).hash(h);
            }
        });
        let found = (patterns, symbols, images, Arc::as_ptr(&cms) as usize);
        let d = match &self.doc_inputs {
            Some((_, d)) if (d.patterns, d.symbols, d.images, d.cms) == found => d.clone(),
            _ => {
                let doc_art = || doc.patterns.iter().flat_map(|p| &p.art).chain(doc.symbols.iter().map(|s| &s.art));
                let art = doc_art().map(Arc::downgrade).collect();
                let pixels = doc.images.values().flat_map(|b| std::iter::once(&b.bytes).chain(&b.proxy)).map(Arc::downgrade).collect();
                let plugins = doc_art().any(|a| crate::holds_plugin_effect(a));
                Arc::new(DocInputs { patterns, symbols, images, cms: found.3, plugins, _held: (art, pixels, Arc::downgrade(&cms)) })
            }
        };
        self.doc_inputs = Some((at, d.clone()));
        d
    }

    /// Drop the least recently drawn shadow and filtered-content rasters beyond `budget` bytes
    /// ([`FX_RASTER_BUDGET`]).
    fn trim_fx_rasters(&mut self, budget: usize) {
        let shadows = self.shadows.iter().map(|(k, e)| (e.stamp, false, *k, e.bytes()));
        let pixels = self.pixel_fx.iter().map(|(k, e)| (e.stamp, true, *k, e.bytes()));
        let mut all: Vec<_> = shadows.chain(pixels).collect();
        let mut total: usize = all.iter().map(|e| e.3).sum();
        if total <= budget {
            return;
        }
        all.sort_unstable_by_key(|e| e.0);
        for (_, pixel, key, bytes) in all {
            if total <= budget {
                break;
            }
            if pixel {
                self.pixel_fx.remove(&key);
            } else {
                self.shadows.remove(&key);
            }
            total -= bytes;
        }
    }

    /// The image Effect › Rasterize `r` makes of the content `paint` draws over `rect` (whole
    /// pixels of the image): drawn on the image's own grid as Object › Rasterize draws it (with
    /// hard edges when `r` doesn't anti-alias), then given `r`'s background and color model. `None`
    /// when it would hold more than [`PIXEL_BUDGET`] pixels or be wider or taller than
    /// [`crate::MAX_RASTER_SIDE`]. → (pixels, columns, rows)
    fn rasterize_image(
        &mut self,
        f: &Frame,
        r: &effects::Rasterize,
        rect: Rect,
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) -> Option<(Vec<u8>, usize, usize)> {
        let line = 72.0 / doc_raster_ppi(f.doc);
        let (w, h) = image_size(r, rect, line)?;
        #[cfg(test)]
        RASTERIZED.with(|n| n.set(n.get() + 1));
        let cell = r.cell(line);
        let view = Affine::scale(1.0 / cell) * Affine::translate(-rect.origin().to_vec2());
        // The effect's own anti-aliasing, whatever the view or the export draws with.
        let anti_alias = match (r.anti_alias, r.hinted) {
            (false, _) => crate::AntiAlias::None,
            (true, true) => crate::AntiAlias::Type,
            (true, false) => crate::AntiAlias::Art,
        };
        let opts = crate::RenderOptions { anti_alias, ..f.opts.clone() };
        let frame = Frame { mt: false, view, visible: rect, px: cell, opts: &opts, ..*f };
        let mut off = frame.offscreen_context(w, h);
        self.inside_layer(|s| paint(s, &mut off, &frame));
        off.flush();
        let mut pm = vello_cpu::Pixmap::new(w, h);
        off.render(&mut pm, &mut self.resources);
        let mut pixels = pm.data_as_u8_slice().to_vec();
        r.finish(&mut pixels);
        Some((pixels, usize::from(w), usize::from(h)))
    }

    /// Keep `pm`'s pixels only inside `g` filled by `rule` as `frame` draws it (Feather's clip).
    fn clip_pixels(&mut self, pm: &mut vello_cpu::Pixmap, frame: &Frame, g: &BezPath, rule: FillRule) {
        let mut m = frame.offscreen_context(pm.width(), pm.height());
        m.set_transform(frame.view);
        m.set_fill_rule(fill_rule(rule));
        m.set_paint(peniko::Color::BLACK);
        m.fill_path(g);
        m.flush();
        let mut mask = vello_cpu::Pixmap::new(pm.width(), pm.height());
        m.render(&mut mask, &mut self.resources);
        for (p, c) in pm.data_mut().iter_mut().zip(mask.data()) {
            let keep = |v: u8| ((v as u32 * c.a as u32 + 127) / 255) as u8;
            *p = vello_cpu::color::PremulRgba8 { r: keep(p.r), g: keep(p.g), b: keep(p.b), a: keep(p.a) };
        }
    }

    /// Run `draw`, which pushes filter layers. Single-threaded contexts draw directly and `draw`
    /// applies `composite` (blend mode, opacity) on its filter layer. On a multithreaded context
    /// the layer is drawn into an offscreen single-threaded context covering `reach` (document
    /// space) plus the blur's spread, then composited back with `composite`.
    /// While dragging, that context holds at most [`DRAG_PIXEL_BUDGET`] pixels.
    fn with_filters(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        reach: Rect,
        blur: f64,
        composite: Option<(vectorcraft_doc::color::BlendMode, f32)>,
        draw: impl FnOnce(&mut Self, &mut RenderContext, &Frame, Option<(vectorcraft_doc::color::BlendMode, f32)>),
    ) {
        if !f.mt {
            return self.inside_layer(|r| draw(r, ctx, f, composite));
        }
        let Some((x0, y0, w, h, k)) = filter_region(f.view.transform_rect_bbox(reach), (ctx.width(), ctx.height()), blur / f.px) else { return };
        // During a drag, the region holds at most `DRAG_PIXEL_BUDGET` pixels (#2558).
        let drag = || filter_region_within(f.view.transform_rect_bbox(reach), (ctx.width(), ctx.height()), blur / f.px, DRAG_PIXEL_BUDGET);
        let (x0, y0, w, h, k) = match f.opts.dragging.then(drag).flatten().filter(|r| r.4 < k) {
            Some(r) => {
                self.stats.moved += 1;
                self.stats.previewed += 1;
                r
            }
            None => (x0, y0, w, h, k),
        };
        let mut off = f.offscreen_context(w, h);
        let shifted = Frame { mt: false, view: Affine::scale(k) * Affine::translate((-x0, -y0)) * f.view, px: f.px / k, ..*f };
        self.inside_layer(|r| draw(r, &mut off, &shifted, None));
        off.flush();
        let mut pm = vello_cpu::Pixmap::new(w, h);
        off.render(&mut pm, &mut self.resources);
        let layered = composite.is_some_and(|(m, o)| m != vectorcraft_doc::color::BlendMode::Normal || o < 1.0);
        if let Some((m, o)) = composite.filter(|_| layered) {
            ctx.set_transform(Affine::IDENTITY);
            ctx.push_layer(None, Some(blend_mode(m)), Some(o), None, None);
        }
        if k < 1.0 {
            draw_scaled(ctx, std::sync::Arc::new(pm), (x0, y0), k);
            ctx.set_transform(Affine::IDENTITY);
        } else {
            crate::group::draw_pixmap(ctx, std::sync::Arc::new(pm), (x0, y0));
        }
        if layered {
            ctx.pop_layer();
        }
    }

    /// Paint the fills and strokes of `n` on geometry `g`, applying per-item geometry effects and
    /// painting per-item raster effects around that item alone.
    #[allow(clippy::too_many_arguments)]
    fn paint_items(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        n: &Node,
        g: &BezPath,
        rule: FillRule,
        gctx: &GeomContext,
        mut cache: Option<&mut Vec<Option<Arc<BezPath>>>>,
    ) {
        let bounds = g.bounding_box();
        for (j, item) in n.appearance.items.iter().enumerate() {
            if !item.visible() || item.paint().is_none() {
                continue;
            }
            // The item's own geometry effects on `g`, from the cache when there is one.
            let slot = cache.as_deref_mut().and_then(|c| c.get_mut(j));
            let ig: Arc<BezPath> = match slot {
                Some(Some(ig)) => ig.clone(),
                Some(s) => s.insert(Arc::new(apply(item_effects(item), g, &gctx.item(item)))).clone(),
                None => Arc::new(apply(item_effects(item), g, &gctx.item(item))),
            };
            let ig = &*ig;
            let ib = if effects::has_geometry(item_effects(item)) && !ig.elements().is_empty() { ig.bounding_box() } else { bounds };
            let rfx = effects::raster_effects(item_effects(item));
            if rfx.is_empty() {
                self.paint_item(ctx, f, n, item, ig, rule, ib);
                continue;
            }
            // The item's silhouette: its fill area, or the outline of its stroke.
            let outline = match item {
                AppearanceItem::Fill(_) => ig.clone(),
                AppearanceItem::Stroke(st) => {
                    let w = effects::stroke::aligned_width(st, effects::stroke::is_closed(ig));
                    effects::stroke::line_outline(&effects::stroke::stroke_pieces(ig, st).line, st, w, f.px * 0.25)
                }
            };
            let outset = n.appearance.outset();
            let orule = if item.is_fill() { rule } else { FillRule::NonZero };
            // Each item's shadows get their own cache slots.
            // Rasterize makes its image over the item's silhouette.
            let bounds = rfx.iter().any(|fx| matches!(fx, RasterFx::Rasterize(_))).then(|| outline.bounding_box());
            let content = Content {
                node: n,
                owner: None,
                cache: Some((j + 1) << 8),
                reach: ib.inflate(outset, outset),
                outline: Some((&outline, orule)),
                bounds,
                kept: None,
            };
            self.raster_fx(ctx, f, &content, &rfx, &mut |r, c, fr| r.paint_item(c, fr, n, item, ig, rule, ib));
            ctx.set_transform(Affine::IDENTITY);
        }
    }

    /// Paint one fill or stroke of `n` on its geometry `ig` (bounds `ib`).
    #[allow(clippy::too_many_arguments)]
    fn paint_item(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, item: &AppearanceItem, ig: &BezPath, rule: FillRule, ib: Rect) {
        match item {
            AppearanceItem::Fill(fl) => {
                let layered = fl.opacity < 1.0 || fl.blend != vectorcraft_doc::color::BlendMode::Normal;
                if layered {
                    ctx.set_transform(Affine::IDENTITY);
                    ctx.push_layer(None, Some(blend_mode(fl.blend)), Some(fl.opacity), None, None);
                }
                ctx.set_transform(f.view);
                if paint::set_paint(ctx, &fl.paint, ib, f) {
                    ctx.set_fill_rule(fill_rule(rule));
                    ctx.fill_path(ig);
                }
                if layered {
                    ctx.pop_layer();
                }
            }
            AppearanceItem::Stroke(st) => {
                if st.width <= 0.0 {
                    return;
                }
                // Brushed strokes keep their brush art under effects (e.g. a glowing scatter brush).
                if st.brush.is_some() && self.draw_brush(ctx, f, n, ig, st) {
                    return;
                }
                self.draw_stroke(ctx, f, Affine::IDENTITY, ig, rule, st, None, ib);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use vectorcraft_doc::color::{Color, Paint};
    use vectorcraft_doc::{Appearance, Document, Effect, Node};
    use vectorcraft_geom::{Affine, Rect, shapes};

    use super::{DRAG_PIXEL_BUDGET, Drawn, PIXEL_BUDGET, drag_scale, filter_region, filter_region_within};
    use crate::tests_fontchange::fonts_steady;
    use crate::{RenderOptions, Rendered, Renderer};

    /// #1706: the raster a shadow or glow is filtered in stays within the pixel budget however
    /// far the blur spreads at the zoom (it was up to 65535 pixels a side: 16 GB).
    #[test]
    fn a_wide_blur_at_a_high_zoom_is_filtered_within_the_pixel_budget() {
        // A 100 pt square at 3200 % and at 6400 %, blurred by 1000 pt, in a 1600 × 1000 view.
        for zoom in [32.0, 64.0] {
            let reach = Rect::new(0.0, 0.0, 100.0 * zoom, 100.0 * zoom);
            let (x0, y0, w, h, k) = filter_region(reach, (1600, 1000), 1000.0 * zoom).unwrap();
            assert!(f64::from(w) * f64::from(h) <= PIXEL_BUDGET, "{w} × {h}");
            assert!(k < 1.0 && x0 < 0.0 && y0 < 0.0);
            // It still covers the view and the blur's spread around it, at that scale.
            assert!(f64::from(w) / k >= 1600.0 + 1500.0 * zoom && f64::from(h) / k >= 1000.0 + 1500.0 * zoom);
        }
        // A small blur is filtered at full size, cropped to what can show.
        assert_eq!(filter_region(Rect::new(10.0, 20.0, 110.0, 70.0), (1600, 1000), 4.0), Some((2.0, 12.0, 116, 66, 1.0)));
        assert_eq!(filter_region(Rect::new(5000.0, 0.0, 5100.0, 100.0), (1600, 1000), 4.0), None, "off the view");
        assert_eq!(filter_region(Rect::new(0.0, 0.0, 100.0, 100.0), (1600, 1000), f64::NAN).map(|r| r.4), Some(1.0));
    }

    /// #2558: while dragging, content and a shadow's region over the drag's budget at their scale
    /// render at a lower scale, within the budget; otherwise their scale stays as it is.
    #[test]
    fn a_drag_lowers_the_scale_of_rasters_over_its_budget() {
        let d = drag_scale(true, (2560.0, 1440.0), 1.0).unwrap();
        assert!(d < 1.0 && (2560.0 * d) * (1440.0 * d) <= DRAG_PIXEL_BUDGET * (1.0 + 1e-9), "{d}");
        assert_eq!(drag_scale(false, (2560.0, 1440.0), 1.0), None, "not dragging");
        assert_eq!(drag_scale(true, (1000.0, 800.0), 1.0), None, "within the budget");
        // At a scale already lowered to 0.5, 1280 × 720 raster pixels: within the budget.
        assert_eq!(drag_scale(true, (2560.0, 1440.0), 0.5), None);
        // A 2000 × 1500 pixel square blurred by 20 pixels in a 1600 × 1000 view.
        let reach = Rect::new(-200.0, -200.0, 1800.0, 1300.0);
        let (x0, y0, w, h, k) = filter_region_within(reach, (1600, 1000), 20.0, DRAG_PIXEL_BUDGET).unwrap();
        assert!(f64::from(w) * f64::from(h) <= DRAG_PIXEL_BUDGET && k < 1.0, "{w} × {h} at {k}");
        // It covers the same pixels as the full-size region, at that scale.
        let full = filter_region(reach, (1600, 1000), 20.0).unwrap();
        assert_eq!((x0, y0, full.4), (full.0, full.1, 1.0));
        assert!((f64::from(w) / k - f64::from(full.2)).abs() <= 1.0 / k && (f64::from(h) / k - f64::from(full.3)).abs() <= 1.0 / k);
        let small = Rect::new(10.0, 20.0, 110.0, 70.0);
        assert_eq!(filter_region_within(small, (1600, 1000), 4.0, DRAG_PIXEL_BUDGET), filter_region(small, (1600, 1000), 4.0));
    }

    fn doc_with(r: Rect, fill: Color, effects: Vec<(&str, serde_json::Value)>) -> Document {
        let mut d = Document::new(100.0, 100.0);
        let id = d.alloc_id();
        let mut n = Node::path(id, shapes::rectangle(r), Appearance::basic(Paint::solid(fill), Paint::None, 0.0));
        n.appearance.effects = effects.into_iter().map(|(id, p)| Effect { id: id.into(), params: p, visible: true }).collect();
        let l = d.layers[0].id;
        d.insert(Some(l), 0, n).unwrap();
        d
    }

    fn render(d: &Document) -> Rendered {
        Renderer::new().render(d, 100, 100, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
    }

    fn lum(p: [u8; 4]) -> u32 {
        p[0] as u32 + p[1] as u32 + p[2] as u32
    }

    const WHITE: u32 = 765;

    /// A page of objects with object-level and item-level geometry effects (Offset Path, Roughen,
    /// Warp, Outline Stroke, Transform) under drop shadows and in groups.
    fn effects_page() -> Document {
        let mut d = Document::new(200.0, 200.0);
        let l = d.layers[0].id;
        let fx = |id: &str, p: serde_json::Value| Effect { id: id.into(), params: p, visible: true };
        for i in 0..24 {
            let (x, y) = (10.0 + (i % 6) as f64 * 30.0, 10.0 + (i / 6) as f64 * 45.0);
            let id = d.alloc_id();
            let r = Rect::new(x, y, x + 20.0, y + 30.0);
            let shape = if i % 2 == 0 { shapes::ellipse(r) } else { shapes::star(r.center(), 12.0, 5.0, 5, 0.0) };
            let mut n = Node::path(id, shape, Appearance::basic(Paint::solid(Color::rgb(0.2, 0.4, 0.9)), Paint::solid(Color::BLACK), 1.5));
            n.appearance.effects = match i % 4 {
                0 => vec![fx("path.offsetPath", json!({"offset": 3}))],
                1 => vec![fx("distort.roughen", json!({"size": 4, "relative": true})), fx("stylize.dropShadow", json!({}))],
                2 => vec![fx("warp.arc", json!({"bend": 30})), fx("path.outlineStroke", json!({}))],
                _ => vec![fx("path.offsetPath", json!({"offset": -2})), fx("distort.transform", json!({"moveH": 4, "rotate": 10}))],
            };
            if let Some(f) = n.appearance.fill_mut() {
                f.effects.push(fx("path.offsetPath", json!({"offset": 2})));
            }
            d.insert(Some(l), usize::MAX, n).unwrap();
        }
        d
    }

    /// Cached live-effect geometry draws exactly what computing it afresh draws: a second (warm)
    /// frame, a frame after panning, and a fresh renderer all give the same pixels, and an edited
    /// object (a new node) draws its new geometry, not the cached one.
    #[test]
    fn cached_effect_geometry_draws_the_same_pixels() {
        let d = effects_page();
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let view = Affine::scale(2.0);
        let mut r = Renderer::new();
        let cold = r.render(&d, 400, 400, view, &opts);
        let warm = r.render(&d, 400, 400, view, &opts);
        assert!(cold.pixels == warm.pixels, "a warm frame differs from the cold one");
        let _ = r.render(&d, 400, 400, Affine::translate((15.0, 0.0)) * view, &opts);
        assert!(r.render(&d, 400, 400, view, &opts).pixels == cold.pixels, "after a pan");
        assert!(Renderer::new().render(&d, 400, 400, view, &opts).pixels == cold.pixels, "a fresh renderer");
        // Edit one object's effect: the warm renderer must draw the new result.
        let mut e = d.clone();
        let id = e.layers[0].children().unwrap()[0].id;
        e.node_mut(id).unwrap().appearance.effects = vec![Effect { id: "path.offsetPath".into(), params: json!({"offset": 9}), visible: true }];
        let edited = r.render(&e, 400, 400, view, &opts);
        assert!(edited.pixels != cold.pixels, "the edit shows");
        assert!(edited.pixels == Renderer::new().render(&e, 400, 400, view, &opts).pixels, "and matches a fresh render");
    }

    /// Shadow rasters of objects in the tree are matched by the node's `Arc` (kept alive), not by
    /// comparing nodes; over the byte budget the least recently drawn go first.
    #[test]
    fn shadow_rasters_keep_their_node_and_a_byte_budget() {
        let d = effects_page();
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let mut r = Renderer::new();
        let first = r.render(&d, 400, 400, Affine::scale(2.0), &opts);
        assert!(!r.shadows.is_empty() && r.shadows.values().all(|e| matches!(e.node, Drawn::Shared(_))));
        // A frame of other objects makes rasters of its own; the older ones go first.
        let _ = r.render(&effects_page(), 400, 400, Affine::scale(2.0), &opts);
        let newest = r.stamp;
        let bytes = |r: &Renderer| r.shadows.values().map(|e| e.bytes()).sum::<usize>();
        let latest = r.shadows.values().filter(|e| e.stamp == newest).map(|e| e.bytes()).sum::<usize>();
        assert!(bytes(&r) > latest);
        r.trim_fx_rasters(latest);
        assert_eq!(bytes(&r), latest);
        assert!(r.shadows.values().all(|e| e.stamp == newest));
        assert!(r.render(&d, 400, 400, Affine::scale(2.0), &opts).pixels == first.pixels, "made again alike");
    }

    /// An Outer Glow whose whole reach is over the budget is made coarser over that whole reach and
    /// kept for any pan: its grid starts at the reach's first whole device pixel, which a pan by whole
    /// pixels moves with the art, so the raster drawn after the pan is the one a fresh renderer makes,
    /// also for pans that aren't multiples of the raster's scale. A pan by a fraction of a pixel
    /// places the raster at the nearest whole pixel, as a full-size raster kept whole is placed, and
    /// here draws as a fresh renderer draws too. A 100 pt glow at 500% (scale 2) and a 2 pt glow
    /// round the page at 800% (scale 3, near the narrowest blur made coarser).
    #[test]
    fn a_coarser_glow_kept_whole_draws_after_a_pan_as_made_again() {
        let size = (640, 360);
        for (rect, blur, zoom, center, scale) in [
            (Rect::new(200.0, 300.0, 400.0, 500.0), 100.0, 5.0, (440.0, 400.0), 2.0),
            (Rect::new(0.0, 0.0, 595.0, 842.0), 2.0, 8.0, (595.0, 421.0), 3.0),
        ] {
            let mut d = Document::new(595.0, 842.0);
            let layer = d.layers[0].id;
            let mut n =
                Node::path(d.alloc_id(), shapes::rectangle(rect), Appearance::basic(Paint::solid(Color::rgb(0.1, 0.3, 0.9)), Paint::None, 0.0));
            n.appearance.effects = vec![Effect { id: "stylize.outerGlow".into(), params: json!({"blur": blur}), visible: true }];
            d.insert(Some(layer), 0, n).unwrap();
            let base = Affine::translate((320.0, 180.0)) * Affine::scale(zoom) * Affine::translate((-center.0, -center.1));
            let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
            let mut r = Renderer::new();
            r.threads = 2;
            r.render(&d, size.0, size.1, base, &opts);
            let image = |r: &Renderer| r.shadows.values().map(|e| std::sync::Arc::as_ptr(&e.image)).collect::<Vec<_>>();
            let kept = image(&r);
            assert!(r.shadows.values().all(|e| e.scale == scale && e.view.is_none()), "blur {blur}: coarser, over the whole reach");
            for pan in [(37.0, -21.0), (1.0, 0.0), (-3.0, 5.0), (0.5, 0.0), (37.3, -21.6)] {
                let xf = Affine::translate(pan) * base;
                let panned = r.render(&d, size.0, size.1, xf, &opts);
                assert_eq!(image(&r), kept, "blur {blur}, pan {pan:?}: the raster kept is drawn");
                let mut fresh = Renderer::new();
                fresh.threads = 2;
                assert!(panned.pixels == fresh.render(&d, size.0, size.1, xf, &opts).pixels, "blur {blur}, pan {pan:?}");
            }
        }
    }

    /// The multithreaded rasterizer draws the same pixels with any number of workers, so the
    /// automatic thread count can follow the machine.
    #[test]
    fn pixels_do_not_depend_on_the_thread_count() {
        let mut d = effects_page();
        // Overlapping translucent and blended objects on top of the effects.
        let l = d.layers[0].id;
        for i in 0..400 {
            let id = d.alloc_id();
            let (x, y) = ((i * 37 % 190) as f64, (i * 53 % 190) as f64);
            let mut n = Node::path(
                id,
                shapes::ellipse(Rect::new(x, y, x + 14.0, y + 9.0)),
                Appearance::basic(Paint::solid(Color::rgb(0.9, 0.3, 0.1)), Paint::solid(Color::BLACK), 0.7),
            );
            n.opacity = 0.6;
            d.insert(Some(l), usize::MAX, n).unwrap();
        }
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let mut frames = vec![];
        // The test sets each renderer's count. `set_default_threads` would also change the count of the
        // renderers that other tests create while this one runs.
        for threads in [0, 1, 4, 8] {
            let mut r = Renderer::new();
            r.threads = threads;
            frames.push(r.render(&d, 1200, 900, Affine::scale(6.0), &opts).pixels);
        }
        assert!(frames.windows(2).all(|w| w[0] == w[1]), "the thread count changed pixels");
    }

    #[test]
    fn thumbnails_of_glowing_objects_render_without_panicking() {
        // Regression: thumbnails used vello's default (multithreaded) context, which panics on filters.
        let d = doc_with(
            Rect::new(20.0, 20.0, 60.0, 60.0),
            Color::BLACK,
            vec![("stylize.outerGlow", json!({"color": "#ff0000", "opacity": 100, "blur": 6}))],
        );
        let id = d.layers[0].children().unwrap()[0].id;
        let t = Renderer::new().render_thumbnail(&d, id, 48).unwrap();
        assert_eq!((t.width, t.height), (48, 48));
    }

    #[test]
    fn per_stroke_raster_effects_use_the_single_threaded_pipeline() {
        let mut d = doc_with(Rect::new(20.0, 20.0, 60.0, 60.0), Color::BLACK, vec![]);
        let l = d.layers[0].id;
        let id = d.layers[0].children().unwrap()[0].id;
        let mut n = (*d.node(id).unwrap()).clone();
        n.appearance = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 4.0).into();
        if let Some(vectorcraft_doc::AppearanceItem::Stroke(s)) =
            n.appearance.items.iter_mut().find(|i| matches!(i, vectorcraft_doc::AppearanceItem::Stroke(_)))
        {
            s.effects =
                vec![Effect { id: "stylize.outerGlow".into(), params: json!({"color": "#ff0000", "opacity": 100, "blur": 6}), visible: true }];
        }
        d.remove(id).unwrap();
        d.insert(Some(l), 0, n).unwrap();
        let mut r = Renderer::new();
        r.threads = 4;
        let _ = r.render(&d, 100, 100, Affine::IDENTITY, &RenderOptions::default());
    }

    #[test]
    fn drop_shadow_darkens_outside_the_shape() {
        let r = Rect::new(20.0, 20.0, 60.0, 60.0);
        let plain = render(&doc_with(r, Color::rgb(1.0, 0.0, 0.0), vec![]));
        assert_eq!(lum(plain.pixel(66, 66)), WHITE);
        let d = doc_with(r, Color::rgb(1.0, 0.0, 0.0), vec![("stylize.dropShadow", json!({"x": 10, "y": 10, "blur": 2, "opacity": 100}))]);
        let img = render(&d);
        assert!(lum(img.pixel(66, 66)) < 200, "shadow below-right: {:?}", img.pixel(66, 66));
        // The object stays on top and unshadowed; the far side has no shadow.
        assert_eq!(img.pixel(40, 40), [255, 0, 0, 255]);
        assert_eq!(lum(img.pixel(15, 15)), WHITE);
    }

    /// A red square's drop shadow at Darkness `darkness` (%), opaque and sharp, 10 pt down and
    /// right; `shade` "color" or "darkness".
    fn shaded_doc(mode: vectorcraft_doc::ColorMode, fill: Color, shade: &str, darkness: f64) -> Document {
        let mut d = doc_with(
            Rect::new(20.0, 20.0, 60.0, 60.0),
            fill,
            vec![(
                "stylize.dropShadow",
                json!({"x": 10, "y": 10, "blur": 0, "opacity": 100, "mode": "normal", "shade": shade, "darkness": darkness}),
            )],
        );
        d.color_mode = mode;
        d
    }

    /// Darkness: the shadow is the object's own color, each channel times 1 − darkness in an RGB
    /// document (the reference app's guide gives the object's color at 0%,
    /// <https://help.adobe.com/archive/en/illustrator/cc/2014/illustrator_reference.pdf> page 428), in the cached
    /// raster and, at 6000 %, in the filter layer that a raster too large to cache is drawn with.
    #[test]
    fn drop_shadow_darkness_darkens_the_objects_own_color() {
        let rgb = vectorcraft_doc::ColorMode::Rgb;
        let red = Color::rgb(1.0, 0.0, 0.0);
        // At 6000 %, the view's center is (65, 65), where only the shadow lies.
        let zoomed = Affine::translate((50.0, 50.0)) * Affine::scale(60.0) * Affine::translate((-65.0, -65.0));
        for (darkness, want) in [(0.0, 255), (50.0, 128), (100.0, 0)] {
            let d = shaded_doc(rgb, red, "darkness", darkness);
            let px = render(&d).pixel(66, 66);
            assert!(px[0].abs_diff(want) <= 1 && px[1] == 0 && px[2] == 0, "{darkness}%: {px:?}");
            let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
            let px = Renderer::new().render(&d, 100, 100, zoomed, &opts).pixel(50, 50);
            assert!(px[0].abs_diff(want) <= 1 && px[1] == 0 && px[2] == 0, "{darkness}% at 6000 %: {px:?}");
        }
    }

    /// In a CMYK document Darkness adds black ink and keeps the object's cyan, magenta and yellow
    /// (same page of the guide: at 100% an object with colors other than black gets a shadow of those colors under
    /// full black, and a black object's shadow is 100% black).
    #[test]
    fn drop_shadow_darkness_adds_black_in_a_cmyk_document() {
        let cmyk = vectorcraft_doc::ColorMode::Cmyk;
        let inks = |d: &Document| {
            let i = Renderer::new().render_region_inks(d, Rect::new(0.0, 0.0, 100.0, 100.0), 1.0, &RenderOptions::default());
            let at = (66 * 100 + 66) * 4;
            [i[at], i[at + 1], i[at + 2], i[at + 3]]
        };
        let red = Color::cmyk(0.0, 1.0, 1.0, 0.0);
        for (darkness, k) in [(0.0, 0), (50.0, 128), (100.0, 255)] {
            let got = inks(&shaded_doc(cmyk, red, "darkness", darkness));
            assert!(got[0] == 0 && got[1] == 255 && got[2] == 255 && got[3].abs_diff(k) <= 1, "{darkness}%: {got:?}");
        }
        // Half black ink, darkened by half: K 50 + 50 × 50% = K 75.
        let got = inks(&shaded_doc(cmyk, Color::cmyk(0.0, 0.0, 0.0, 0.5), "darkness", 50.0));
        assert!(got[..3] == [0, 0, 0] && got[3].abs_diff(191) <= 1, "{got:?}");
        let got = inks(&shaded_doc(cmyk, Color::cmyk(0.0, 0.0, 0.0, 1.0), "darkness", 100.0));
        assert_eq!(got, [0, 0, 0, 255]);
    }

    /// Color stays the default: a shadow without the new keys, and one with Color chosen and a
    /// Darkness value, draw the same pixels, which a shadow with Darkness does not.
    #[test]
    fn a_drop_shadow_with_color_draws_as_before() {
        let rgb = vectorcraft_doc::ColorMode::Rgb;
        let red = Color::rgb(1.0, 0.0, 0.0);
        let old = doc_with(
            Rect::new(20.0, 20.0, 60.0, 60.0),
            red,
            vec![("stylize.dropShadow", json!({"x": 10, "y": 10, "blur": 0, "opacity": 100, "mode": "normal"}))],
        );
        let before = render(&old).pixels;
        assert_eq!(render(&shaded_doc(rgb, red, "color", 40.0)).pixels, before);
        assert_ne!(render(&shaded_doc(rgb, red, "darkness", 40.0)).pixels, before);
    }

    #[test]
    fn shadow_of_offscreen_object_is_not_culled() {
        let d = doc_with(
            Rect::new(-60.0, 20.0, -5.0, 60.0),
            Color::WHITE,
            vec![("stylize.dropShadow", json!({"x": 30, "y": 0, "blur": 0, "opacity": 100}))],
        );
        let img = render(&d);
        let row: Vec<u32> = (0..10).map(|i| lum(img.pixel(i * 10, 40))).collect();
        assert!(lum(img.pixel(10, 40)) < 100, "{row:?}");
    }

    #[test]
    fn geometry_effect_is_rendered() {
        let r = Rect::new(30.0, 30.0, 70.0, 70.0);
        let d = doc_with(r, Color::BLACK, vec![("path.offsetPath", json!({"offset": 10}))]);
        let img = render(&d);
        assert_eq!(lum(img.pixel(25, 50)), 0, "offset grows the fill");
        let d = doc_with(r, Color::BLACK, vec![("distort.transform", json!({"moveH": -25}))]);
        let img = render(&d);
        assert_eq!(lum(img.pixel(10, 50)), 0);
        assert_eq!(lum(img.pixel(60, 50)), WHITE);
    }

    #[test]
    fn per_fill_effects_apply_to_that_fill_only() {
        let mut d = doc_with(Rect::new(30.0, 30.0, 70.0, 70.0), Color::BLACK, vec![]);
        let id = d.layers[0].children().unwrap()[0].id;
        let n = d.node_mut(id).unwrap();
        n.appearance.fill_mut().unwrap().effects.push(Effect { id: "distort.transform".into(), params: json!({"moveH": 20}), visible: true });
        let img = render(&d);
        assert_eq!(lum(img.pixel(85, 50)), 0);
        assert_eq!(lum(img.pixel(40, 50)), WHITE);
    }

    #[test]
    fn blur_glow_and_feather_soften() {
        let r = Rect::new(30.0, 30.0, 70.0, 70.0);
        let blur = render(&doc_with(r, Color::BLACK, vec![("blur.gaussian", json!({"radius": 6}))]));
        let edge = lum(blur.pixel(29, 50));
        assert!(edge > 0 && edge < WHITE, "blur spreads over the edge: {edge}");
        let glow =
            render(&doc_with(r, Color::BLACK, vec![("stylize.outerGlow", json!({"color": "#ff0000", "mode": "normal", "opacity": 100, "blur": 4}))]));
        let g = glow.pixel(27, 50);
        assert!(g[0] > g[1] + 20, "outer glow is red: {g:?}");
        let inner =
            render(&doc_with(r, Color::BLACK, vec![("stylize.innerGlow", json!({"color": "#ffffff", "mode": "normal", "opacity": 100, "blur": 4}))]));
        assert!(lum(inner.pixel(31, 50)) > 60, "inner glow lightens the edge: {:?}", inner.pixel(31, 50));
        assert!(lum(inner.pixel(50, 50)) < 30, "centre stays dark");
        let feather = render(&doc_with(r, Color::BLACK, vec![("stylize.feather", json!({"radius": 8}))]));
        assert!(lum(feather.pixel(31, 50)) > 20, "feather fades the edge inward");
        assert_eq!(lum(feather.pixel(26, 50)), WHITE, "feather stays inside");
    }

    #[test]
    fn an_outer_glow_blurs_a_solid_silhouette_of_faint_art() {
        // GUARDS: the glow's strength comes from where the art is, not how opaque it is (a glow round a glow relies on it).
        let r = Rect::new(30.0, 30.0, 70.0, 70.0);
        let glow = |fill_opacity: f32| {
            let mut d =
                doc_with(r, Color::BLACK, vec![("stylize.outerGlow", json!({"color": "#ff0000", "mode": "normal", "opacity": 100, "blur": 4}))]);
            let id = d.layers[0].children().unwrap()[0].id;
            d.node_mut(id).unwrap().appearance.fill_mut().unwrap().opacity = fill_opacity;
            render(&d).pixel(29, 50)
        };
        let (faint, opaque) = (glow(0.2), glow(1.0));
        assert_eq!(faint[1], opaque[1], "faint {faint:?}, opaque {opaque:?}");
        assert!(faint[1] < 210, "the glow reaches past the edge: {faint:?}");
    }

    #[test]
    fn an_outer_glow_grows_its_silhouette_by_a_quarter_raster_pixel() {
        // GUARDS: the glows AI files export measure as the solid silhouette grown by a quarter of a pixel
        // of the document's raster grid, then blurred: a quarter point at 72 ppi, next to nothing at 720.
        let r = Rect::new(30.0, 30.0, 70.0, 70.0);
        let green_outside = |ppi: f64| {
            let mut d =
                doc_with(r, Color::BLACK, vec![("stylize.outerGlow", json!({"color": "#ff0000", "mode": "normal", "opacity": 100, "blur": 4}))]);
            d.raster_effects_ppi = ppi;
            render(&d).pixel(27, 50)[1]
        };
        let (coarse, fine) = (green_outside(72.0), green_outside(720.0));
        assert!(coarse + 3 < fine, "the red glow is stronger at 72 ppi: 72 ppi {coarse}, 720 ppi {fine}");
    }

    #[test]
    fn a_glow_glows_round_the_glows_before_it_in_the_stack() {
        // GUARDS: the second glow is of the art with the first glow, not of the bare art, as AI files and the SVG export draw it.
        let r = Rect::new(30.0, 30.0, 70.0, 70.0);
        let d = doc_with(
            r,
            Color::BLACK,
            vec![
                ("stylize.outerGlow", json!({"color": "#ff0000", "mode": "normal", "opacity": 100, "blur": 8})),
                ("stylize.outerGlow", json!({"color": "#0000ff", "mode": "normal", "opacity": 100, "blur": 2})),
            ],
        );
        let p = render(&d).pixel(22, 50);
        assert!(p[2] > p[0] + 100, "8 pt out, the tight blue glow round the red one shows: {p:?}");
    }

    #[test]
    fn a_glow_too_large_for_a_full_size_raster_keeps_its_solid_silhouette() {
        // GUARDS: at 40× the glow's raster passes the 4 Mpx budget and is made coarser instead of falling back to the
        // filter layer, which blurs the faint alpha.
        let r = Rect::new(30.0, 30.0, 70.0, 70.0);
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let view = Affine::translate((-1100.0, -1900.0)) * Affine::scale(40.0);
        let glow = |fill_opacity: f32| {
            let mut d =
                doc_with(r, Color::BLACK, vec![("stylize.outerGlow", json!({"color": "#ff0000", "mode": "normal", "opacity": 100, "blur": 4}))]);
            let id = d.layers[0].children().unwrap()[0].id;
            d.node_mut(id).unwrap().appearance.fill_mut().unwrap().opacity = fill_opacity;
            Renderer::new().render(&d, 200, 200, view, &opts).pixel(60, 100)
        };
        let (faint, opaque) = (glow(0.2), glow(1.0));
        assert!(faint[1].abs_diff(opaque[1]) <= 2, "faint {faint:?}, opaque {opaque:?}");
        assert!(faint[1] < 210, "the glow reaches past the edge: {faint:?}");
    }

    #[test]
    fn thirty_stacked_glows_draw_quickly_cached_or_not() {
        // GUARDS: each glow is of the content with the glows before it; drawing those again for every glow costs 2^n.
        // A shape draws through the shadow cache, a symbol instance without it (the filter layers).
        let glows: Vec<(&str, serde_json::Value)> =
            (0..30).map(|k| ("stylize.outerGlow", json!({"color": "#ff0000", "mode": "normal", "opacity": 20, "blur": 1 + k % 4}))).collect();
        let shape = doc_with(Rect::new(30.0, 30.0, 70.0, 70.0), Color::BLACK, glows.clone());
        let mut instanced = Document::new(100.0, 100.0);
        let art = Node::path(
            instanced.alloc_id(),
            shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 40.0)),
            Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        instanced.symbols.push(vectorcraft_doc::Symbol { name: "Square".into(), art: std::sync::Arc::new(art) });
        let mut instance = Node::new(
            instanced.alloc_id(),
            vectorcraft_doc::NodeKind::SymbolInstance { symbol: "Square".into(), xf: Affine::translate((30.0, 30.0)) },
        );
        instance.appearance.effects = glows.into_iter().map(|(id, p)| Effect { id: id.into(), params: p, visible: true }).collect();
        let l = instanced.layers[0].id;
        instanced.insert(Some(l), 0, instance).unwrap();
        for d in [&shape, &instanced] {
            let start = std::time::Instant::now();
            let p = render(d).pixel(25, 50);
            assert!(start.elapsed() < std::time::Duration::from_secs(20), "{:?}", start.elapsed());
            assert!(p[1] < 250, "the glows show: {p:?}");
        }
    }

    #[test]
    fn hidden_effects_are_ignored_and_outline_mode_uses_effected_geometry() {
        let r = Rect::new(30.0, 30.0, 70.0, 70.0);
        let mut d = doc_with(r, Color::BLACK, vec![("distort.transform", json!({"moveH": -25}))]);
        let id = d.layers[0].children().unwrap()[0].id;
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), outline: true, ..Default::default() };
        let img = Renderer::new().render(&d, 100, 100, Affine::IDENTITY, &opts);
        assert!(lum(img.pixel(5, 50)) < 400, "outline follows the moved path");
        d.node_mut(id).unwrap().appearance.effects[0].visible = false;
        let img = render(&d);
        assert_eq!(lum(img.pixel(50, 50)), 0);
        assert_eq!(lum(img.pixel(10, 50)), WHITE);
    }

    #[test]
    fn visual_bounds_include_shadow() {
        let d = doc_with(Rect::new(0.0, 0.0, 10.0, 10.0), Color::BLACK, vec![("stylize.dropShadow", json!({"x": 7, "y": 7, "blur": 5}))]);
        let n = &d.layers[0].children().unwrap()[0];
        let b = super::visual_bounds(&d.symbol_boxes(), n).unwrap();
        assert!(b.x1 >= 10.0 + 7.0 + 7.5 - 1e-9);
    }

    /// Multithreaded rendering filters each effect offscreen; it must match the single-threaded
    /// reference (blend modes against the real backdrop, objects partly off-screen, all effects).
    #[test]
    fn multithreaded_filters_match_single_threaded() {
        let cases: Vec<(Rect, Vec<(&str, serde_json::Value)>)> = vec![
            (Rect::new(20.0, 20.0, 60.0, 60.0), vec![("stylize.dropShadow", json!({"x": 6, "y": 4, "blur": 5, "opacity": 75}))]),
            (Rect::new(20.0, 20.0, 60.0, 60.0), vec![("stylize.outerGlow", json!({"color": "#ff00aa", "blur": 8, "opacity": 90}))]),
            (
                Rect::new(20.0, 20.0, 60.0, 60.0),
                vec![
                    ("stylize.innerGlow", json!({"blur": 6})),
                    ("stylize.innerGlow", json!({"blur": 4, "source": "center", "mode": "multiply", "color": "#0044ff"})),
                ],
            ),
            (Rect::new(20.0, 20.0, 60.0, 60.0), vec![("stylize.feather", json!({"radius": 6}))]),
            (Rect::new(20.0, 20.0, 60.0, 60.0), vec![("blur.gaussian", json!({"radius": 4})), ("stylize.dropShadow", json!({"blur": 3}))]),
            // Partly outside the frame: the blur still spreads in from off-screen content.
            (
                Rect::new(-30.0, 70.0, 30.0, 130.0),
                vec![("stylize.outerGlow", json!({"blur": 12, "mode": "normal"})), ("blur.gaussian", json!({"radius": 3}))],
            ),
        ];
        for (r, fx) in cases {
            let name = format!("{fx:?}");
            let mut d = doc_with(r, Color::rgb(0.2, 0.6, 0.3), fx);
            // A coloured backdrop so blend modes have something to blend with.
            let l = d.layers[0].id;
            let bid = d.alloc_id();
            let bg = Node::path(
                bid,
                shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)),
                Appearance::basic(Paint::solid(Color::rgb(0.9, 0.5, 0.1)), Paint::None, 0.0),
            );
            d.insert(Some(l), 0, bg).unwrap();
            let opts = RenderOptions::default();
            let mut st = Renderer::new();
            st.threads = 0;
            let mut mt = Renderer::new();
            mt.threads = 3;
            let a = st.render(&d, 100, 100, Affine::IDENTITY, &opts);
            let b = mt.render(&d, 100, 100, Affine::IDENTITY, &opts);
            let worst = a.pixels.iter().zip(&b.pixels).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
            // The offscreen pass adds one 8-bit premultiplied round trip: a few levels of rounding.
            assert!(worst <= 4, "{name}: max channel difference {worst}");
        }
    }

    #[test]
    fn photoshop_filters_draw_and_follow_a_pan_from_the_cache() {
        let d = doc_with(Rect::new(30.0, 30.0, 70.0, 70.0), Color::BLACK, vec![("blur.radial", json!({"amount": 40, "quality": "draft"}))]);
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let mut r = Renderer::new();
        let a = r.render(&d, 100, 100, Affine::IDENTITY, &opts);
        assert!(lum(a.pixel(31, 31)) > 60, "the spin softens the corners: {:?}", a.pixel(31, 31));
        assert_eq!(lum(a.pixel(50, 50)), 0, "the centre stays");
        assert!(lum(a.pixel(50, 28)) < WHITE, "turned corners reach past the sides");
        // Panned: the cached raster moves with the view.
        let b = r.render(&d, 100, 100, Affine::translate((10.0, 3.0)), &opts);
        for (x, y) in [(31, 31), (50, 28), (69, 50), (40, 66)] {
            assert_eq!(a.pixel(x, y), b.pixel(x + 10, y + 3), "({x}, {y})");
        }
        // A fresh renderer draws the panned view the same.
        let c = Renderer::new().render(&d, 100, 100, Affine::translate((10.0, 3.0)), &opts);
        assert_eq!(b.pixels, c.pixels);
    }

    /// A blurred image that can't be decoded: every frame that draws it counts the failure
    /// ([`crate::paint::decode_failures`]), from a new raster or from the raster kept.
    #[test]
    fn cached_effect_rasters_count_the_images_they_could_not_decode() {
        use vectorcraft_doc::{ImageBlob, ImageObject, NodeKind};
        let mut d = Document::new(100.0, 100.0);
        // A PNG signature with nothing after it.
        d.images.insert("cut".into(), ImageBlob::new("image/png", b"\x89PNG\r\n\x1a\n".to_vec()));
        let im = ImageObject {
            key: "cut".into(),
            width: 40,
            height: 40,
            xf: Affine::translate((30.0, 30.0)),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
        n.appearance.effects = vec![Effect { id: "blur.gaussian".into(), params: json!({"radius": 4.0}), visible: true }];
        d.insert(Some(d.layers[0].id), 0, n).unwrap();
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let mut r = Renderer::new();
        for frame in 0..3 {
            let before = crate::paint::decode_failures();
            r.render(&d, 100, 100, Affine::IDENTITY, &opts);
            assert!(crate::paint::decode_failures() > before, "frame {frame}");
        }
        assert_eq!(r.pixel_fx.len(), 1, "the later frames draw from the raster kept");
    }

    /// An image that can't be decoded, with Rasterize, a drop shadow and a blur: the shadow draws
    /// the Rasterize image first, and the blurred content draws the image kept from that draw
    /// ([`super::Kept`]). Every frame counts the failure, also when the shadow and the blurred
    /// content are drawn from the rasters kept.
    #[test]
    fn rasters_drawn_from_a_kept_rasterize_image_count_the_images_it_could_not_decode() {
        use vectorcraft_doc::{ImageBlob, ImageObject, NodeKind};
        let mut d = Document::new(100.0, 100.0);
        // A PNG signature with nothing after it.
        d.images.insert("cut".into(), ImageBlob::new("image/png", b"\x89PNG\r\n\x1a\n".to_vec()));
        let im = ImageObject {
            key: "cut".into(),
            width: 40,
            height: 40,
            xf: Affine::translate((30.0, 30.0)),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
        n.appearance.effects = [
            ("rasterize", json!({"background": "white"})),
            ("stylize.dropShadow", json!({"mode": "normal", "opacity": 100, "x": 2, "y": 2, "blur": 1, "color": "#0000ff"})),
            ("blur.gaussian", json!({"radius": 4.0})),
        ]
        .into_iter()
        .map(|(id, params)| Effect { id: id.into(), params, visible: true })
        .collect();
        d.insert(Some(d.layers[0].id), 0, n).unwrap();
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let mut r = Renderer::new();
        for frame in 0..3 {
            let before = crate::paint::decode_failures();
            r.render(&d, 100, 100, Affine::IDENTITY, &opts);
            assert!(crate::paint::decode_failures() > before, "frame {frame}");
        }
        assert_eq!((r.shadows.len(), r.pixel_fx.len()), (1, 1), "the later frames draw from the rasters kept");
    }

    /// While interacting, an object that only moved draws its drop shadow and filtered content
    /// from the rasters of an earlier frame, at the nearest whole pixel, without running the
    /// effects again (#1257). The frame after the interaction runs them again: it is what a fresh
    /// renderer draws.
    #[test]
    fn moved_objects_draw_their_effect_rasters_while_interacting() {
        let effects =
            vec![("stylize.dropShadow", json!({"x": 4, "y": 3, "blur": 2, "opacity": 80})), ("texture.craquelure", json!({"crackSpacing": 6}))];
        let base = doc_with(Rect::new(20.0, 20.0, 50.0, 46.0), Color::rgb(0.9, 0.4, 0.1), effects);
        let id = base.layers[0].children().unwrap()[0].id;
        // The object moved from where it started, as each frame of a drag moves it.
        let moved = |dx: f64, dy: f64| {
            let mut d = base.clone();
            d.node_mut(id).unwrap().transform(Affine::translate((dx, dy)), false);
            d
        };
        let (view, still) = (Affine::scale(2.0), RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() });
        let live = RenderOptions { interacting: true, ..still.clone() };
        let fresh = |d: &Document| Renderer::new().render(d, 200, 200, view, &still).pixels;
        let mut r = Renderer::new();
        assert!(r.render(&base, 200, 200, view, &still).pixels == fresh(&base));
        assert_eq!(r.stats.moved, 0);
        let (mut d, mut last) = (base.clone(), vec![]);
        for (dx, dy) in [(3.0, 0.0), (3.3, 1.2), (5.15, 2.5), (6.3, 2.2)] {
            d = moved(dx, dy);
            last = r.render(&d, 200, 200, view, &live).pixels;
            assert_eq!(r.stats.moved, 2, "the shadow and the filtered content moved by ({dx}, {dy})");
            // At the nearest whole pixel (2 a point): what a fresh renderer draws there.
            let whole = moved((dx * 2.0).round() / 2.0, (dy * 2.0).round() / 2.0);
            assert!(last == fresh(&whole), "moved by ({dx}, {dy})");
        }
        // The frame after the drag, of the same document: the effects run again where the object is.
        let after = r.render(&d, 200, 200, view, &still).pixels;
        assert_eq!(r.stats.moved, 0);
        assert!(after == fresh(&d), "the frame after the drag");
        assert!(after != last, "6.3 is not on a whole pixel");
    }

    /// While dragging (#2558), filtered content and a drop shadow that the frame does not draw from a
    /// raster of an earlier frame render within the drag's budget and keep no raster. At 6400% a
    /// rectangle with Drop Shadow and Texturizer covers more than the raster budget, so the still
    /// frame keeps rasters cropped to its view, which the moved rectangle can't take along, and a
    /// 1600 × 1000 view near its corner needs more than the drag's budget. The
    /// frame after the drag, of the same document, is what a fresh renderer draws, as is a frame
    /// rendered while interacting without a drag (a dialog's preview).
    #[test]
    fn a_drag_renders_raster_effects_within_the_drag_budget() {
        let effects = vec![("stylize.dropShadow", json!({"x": 4, "y": 3, "blur": 7, "opacity": 80})), ("texture.texturizer", json!({}))];
        let base = doc_with(Rect::new(20.0, 20.0, 80.0, 76.0), Color::rgb(0.9, 0.4, 0.1), effects);
        let id = base.layers[0].children().unwrap()[0].id;
        let mut d = base.clone();
        d.node_mut(id).unwrap().transform(Affine::translate((0.3, 0.2)), false);
        // The view is centered 10 pt inside the rectangle's right and bottom edges, and shows the
        // shadow beyond the right edge.
        let view = Affine::translate((800.0, 500.0)) * Affine::scale(64.0) * Affine::translate((-70.0, -66.0));
        let still = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let live = RenderOptions { interacting: true, ..still.clone() };
        let drag = RenderOptions { dragging: true, ..live.clone() };
        // Every renderer here has 3 threads on any machine. A drag renders drop shadows within its
        // budget only with `threads` above 0.
        let threaded = || {
            let mut r = Renderer::new();
            r.threads = 3;
            r
        };
        let fresh = threaded().render(&d, 1600, 1000, view, &still).pixels;
        let mut r = threaded();
        let _ = r.render(&base, 1600, 1000, view, &still);
        assert_eq!(r.stats.previewed, 0);
        let dragged = r.render(&d, 1600, 1000, view, &drag).pixels;
        assert_eq!(r.stats.previewed, 2, "the shadow and the filtered content");
        assert_eq!(r.stats.moved, 2, "counted as moved: the frame after the drag renders them again");
        assert!(r.pixel_fx.is_empty() && r.shadows.values().all(|e| e.stamp < r.stamp), "no raster kept");
        assert!(dragged != fresh, "at the drag's lower resolution");
        let after = r.render(&d, 1600, 1000, view, &still).pixels;
        assert_eq!((r.stats.previewed, r.stats.moved), (0, 0));
        assert!(after == fresh, "the frame after the drag");
        assert!(r.render(&d, 1600, 1000, view, &live).pixels == fresh, "interacting without a drag");
        assert_eq!(r.stats.previewed, 0);
        // A drop shadow of about 2 million pixels at 4000%, within the shadow cache's limit: scaled
        // by a drag, it can't take its raster along, and renders within the drag's budget.
        let shadowed = doc_with(Rect::new(20.0, 20.0, 50.0, 46.0), Color::rgb(0.9, 0.4, 0.1), vec![("stylize.dropShadow", json!({"blur": 2}))]);
        let mut scaled = shadowed.clone();
        let id = scaled.layers[0].children().unwrap()[0].id;
        scaled.node_mut(id).unwrap().transform(Affine::scale_about(1.05, vectorcraft_geom::Point::new(35.0, 33.0)), false);
        let view = Affine::translate((800.0, 500.0)) * Affine::scale(40.0) * Affine::translate((-35.0, -33.0));
        let mut r = threaded();
        let _ = r.render(&shadowed, 1600, 1000, view, &still);
        let kept = r.shadows.len();
        assert_eq!((kept, r.stats.previewed), (1, 0), "a still frame keeps the shadow's raster");
        let _ = r.render(&scaled, 1600, 1000, view, &drag);
        assert_eq!((r.shadows.len(), r.stats.previewed), (kept, 1), "the scaled shadow");
        let after = r.render(&scaled, 1600, 1000, view, &still).pixels;
        assert!(after == threaded().render(&scaled, 1600, 1000, view, &still).pixels, "the frame after the drag");
    }

    /// While interacting, a group holding a symbol instance whose art lies outside the 20 × 20
    /// square around the origin draws its drop shadow from the raster of an earlier frame. The
    /// shadow is made from the instance's art, which the renderer measures the group's content by.
    #[test]
    fn a_moved_group_draws_the_shadow_of_the_instance_art_in_it_while_interacting() {
        let _fonts = fonts_steady();
        let mut base = Document::new(100.0, 100.0);
        let orange = Appearance::basic(Paint::solid(Color::rgb(0.9, 0.4, 0.1)), Paint::None, 0.0);
        let art = Node::path(base.alloc_id(), shapes::rectangle(Rect::new(100.0, 100.0, 130.0, 126.0)), orange);
        base.symbols.push(vectorcraft_doc::Symbol { name: "Far".into(), art: std::sync::Arc::new(art) });
        // The instance draws the art from (20, 20) to (50, 46), and the 20 × 20 square around the
        // origin from (-90, -90) to (-70, -70).
        let xf = Affine::translate((-80.0, -80.0));
        let instance = Node::new(base.alloc_id(), vectorcraft_doc::NodeKind::SymbolInstance { symbol: "Far".into(), xf });
        let mut group = Node::group(base.alloc_id(), vec![std::sync::Arc::new(instance)]);
        let shadow = json!({"x": 4, "y": 3, "blur": 2, "opacity": 80});
        group.appearance.effects.push(Effect { id: "stylize.dropShadow".into(), params: shadow, visible: true });
        let (id, l) = (group.id, base.layers[0].id);
        base.insert(Some(l), 0, group).unwrap();
        // The group moved from where it started, as each frame of a drag moves it.
        let moved = |dx: f64, dy: f64| {
            let mut d = base.clone();
            d.node_mut(id).unwrap().transform(Affine::translate((dx, dy)), false);
            d
        };
        let (view, still) = (Affine::scale(2.0), RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() });
        let live = RenderOptions { interacting: true, ..still.clone() };
        let fresh = |d: &Document| Renderer::new().render(d, 200, 200, view, &still);
        let mut r = Renderer::new();
        let first = r.render(&base, 200, 200, view, &still);
        assert!(first.pixels == fresh(&base).pixels);
        // The shadow, 4 pt right of the art's right edge at x 50, at (104, 70) in pixels.
        assert!(lum(first.pixel(104, 70)) < WHITE, "{:?}", first.pixel(104, 70));
        // Moves on whole pixels (2 a point): the moved shadow lies where a fresh renderer draws it.
        for (dx, dy) in [(3.0, 0.0), (5.5, 2.0)] {
            let d = moved(dx, dy);
            let frame = r.render(&d, 200, 200, view, &live).pixels;
            assert_eq!(r.stats.moved, 1, "the shadow moved by ({dx}, {dy})");
            assert!(frame == fresh(&d).pixels, "moved by ({dx}, {dy})");
        }
    }

    /// While interacting, a raster moves with its object only when the content's pixels (for a
    /// shadow, their alpha) and the effects stayed the same (#1257): a new fill color under
    /// Craquelure, other effect settings, a pattern fill that stays on the page, a resized object,
    /// art added beyond the old raster, a changed Feather outline and De-Interlace run the effects
    /// again. A shadow keeps its raster under a new fill color.
    #[test]
    fn moved_rasters_need_the_same_content_and_effects() {
        let _fonts = fonts_steady();
        let (view, still) = (Affine::scale(2.0), RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() });
        let live = RenderOptions { interacting: true, ..still.clone() };
        // `before` drawn, then `after` while interacting: the rasters drawn moved, and whether the
        // frame is what a fresh renderer draws.
        let drag = |before: &Document, after: &Document| {
            let mut r = Renderer::new();
            let _ = r.render(before, 200, 200, view, &still);
            let frame = r.render(after, 200, 200, view, &live).pixels;
            (r.stats.moved, frame == Renderer::new().render(after, 200, 200, view, &still).pixels)
        };
        // `d`'s object moved 5, 3 (10, 6 pixels), then changed by `edit`.
        let moved = |d: &Document, patterns: bool, edit: &dyn Fn(&mut Node)| {
            let mut d = d.clone();
            let id = d.layers[0].children().unwrap()[0].id;
            let n = d.node_mut(id).unwrap();
            n.transform(Affine::translate((5.0, 3.0)), vectorcraft_doc::Scaling { patterns, ..Default::default() });
            edit(n);
            d
        };
        let cracks = || vec![("texture.craquelure", json!({"crackSpacing": 6}))];
        let base = doc_with(Rect::new(20.0, 20.0, 50.0, 46.0), Color::rgb(0.9, 0.4, 0.1), cracks());
        assert_eq!(drag(&base, &moved(&base, false, &|_| {})), (1, true), "moved");
        let recolor = |n: &mut Node| n.appearance.fill_mut().unwrap().paint = Paint::solid(Color::rgb(0.1, 0.4, 0.9));
        assert_eq!(drag(&base, &moved(&base, false, &recolor)), (0, true), "recolored");
        let spacing = |n: &mut Node| n.appearance.effects[0].params = json!({"crackSpacing": 9});
        assert_eq!(drag(&base, &moved(&base, false, &spacing)), (0, true), "other effect settings");
        let resize = |n: &mut Node| n.transform(Affine::scale_about(1.5, vectorcraft_geom::Point::new(25.0, 23.0)), false);
        assert_eq!(drag(&base, &moved(&base, false, &resize)), (0, true), "resized");
        // A square added beyond the old raster leaves the content inside it as it was.
        let beyond = |n: &mut Node| {
            if let vectorcraft_doc::NodeKind::Path { path, .. } = &mut n.kind {
                path.subpaths.extend(shapes::rectangle(Rect::new(75.0, 30.0, 80.0, 35.0)).subpaths);
            }
        };
        assert_eq!(drag(&base, &moved(&base, false, &beyond)), (0, true), "art added beyond the old raster");
        // At 72 ppi, De-Interlace works on lines 1 point tall. The object moves 3 points down, so
        // De-Interlace makes its other field again.
        let fields = doc_with(Rect::new(20.0, 20.0, 50.0, 46.0), Color::rgb(0.9, 0.4, 0.1), vec![("video.deinterlace", json!({}))]);
        assert_eq!(drag(&fields, &moved(&fields, false, &|_| {})), (0, true), "De-Interlace");
        // A pattern fill: its tiles stay on the page unless they move with the object.
        let mut tiled = base.clone();
        let red = Node::path(
            vectorcraft_doc::NodeId(900),
            shapes::rectangle(Rect::new(0.0, 0.0, 3.0, 3.0)),
            Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0),
        );
        let mut def = vectorcraft_doc::pattern::PatternDef::new("Dots", vec![std::sync::Arc::new(red)]);
        def.tile = Rect::new(0.0, 0.0, 7.0, 7.0);
        tiled.patterns.push(def);
        let id = tiled.layers[0].children().unwrap()[0].id;
        tiled.node_mut(id).unwrap().appearance.fill_mut().unwrap().paint = vectorcraft_doc::pattern::pattern_paint("Dots");
        assert_eq!(drag(&tiled, &moved(&tiled, false, &|_| {})), (0, true), "tiles that stay on the page");
        assert_eq!(drag(&tiled, &moved(&tiled, true, &|_| {})), (1, true), "tiles that move with the object");
        // Feather clips to the outline: a notch in the top edge where the tiles paint nothing
        // (x 29 to 33 and y 20 to 24 once moved) leaves the content's pixels as they were.
        let mut feathered = tiled.clone();
        feathered.node_mut(id).unwrap().appearance.effects =
            vec![Effect { id: "stylize.feather".into(), params: json!({"radius": 3}), visible: true }];
        assert_eq!(drag(&feathered, &moved(&feathered, true, &|_| {})), (1, true), "feathered");
        let notch = |n: &mut Node| {
            let mut bp = vectorcraft_geom::BezPath::new();
            bp.move_to((25.0, 23.0));
            for p in [(30.0, 23.0), (30.5, 23.6), (31.5, 23.6), (32.0, 23.0), (55.0, 23.0), (55.0, 49.0), (25.0, 49.0)] {
                bp.line_to(p);
            }
            bp.close_path();
            if let vectorcraft_doc::NodeKind::Path { path, .. } = &mut n.kind {
                *path = vectorcraft_geom::PathData::from_bezpath(&bp);
            }
        };
        assert_eq!(drag(&feathered, &moved(&feathered, true, &notch)), (0, true), "a notch in the feathered outline");
        // A shadow is made from the silhouette's alpha: recolored, the object takes its shadow
        // along; notched, the object makes its shadow again.
        let shadowed =
            doc_with(Rect::new(20.0, 20.0, 50.0, 46.0), Color::rgb(0.9, 0.4, 0.1), vec![("stylize.dropShadow", json!({"x": 4, "y": 3, "blur": 2}))]);
        assert_eq!(drag(&shadowed, &moved(&shadowed, false, &recolor)), (1, true), "a shadow under a new color");
        assert_eq!(drag(&shadowed, &moved(&shadowed, false, &notch)), (0, true), "a shadow under a notch");
        // A shadow with Darkness is made from the object's colors: recolored, it is made again.
        let shaded = doc_with(
            Rect::new(20.0, 20.0, 50.0, 46.0),
            Color::rgb(0.9, 0.4, 0.1),
            vec![("stylize.dropShadow", json!({"x": 4, "y": 3, "blur": 2, "shade": "darkness", "darkness": 50}))],
        );
        assert_eq!(drag(&shaded, &moved(&shaded, false, &|_| {})), (1, true), "a shadow with Darkness, moved");
        assert_eq!(drag(&shaded, &moved(&shaded, false, &recolor)), (0, true), "a shadow with Darkness under a new color");
    }

    /// A raster made over moved rasters counts as moved itself (#1257): a group with a drop shadow
    /// and Gaussian Blur of its own, whose filtered member moves by parts of a pixel while the
    /// other member stays, draws after the drag what a fresh renderer draws.
    #[test]
    fn rasters_made_over_moved_rasters_count_as_moved() {
        let mut d = Document::new(100.0, 100.0);
        let fill = |c: Color| Appearance::basic(Paint::solid(c), Paint::None, 0.0);
        let (sid, mid, gid) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let still = Node::path(sid, shapes::rectangle(Rect::new(60.0, 60.0, 80.0, 80.0)), fill(Color::rgb(0.1, 0.5, 0.2)));
        let mut member = Node::path(mid, shapes::rectangle(Rect::new(20.0, 20.0, 50.0, 46.0)), fill(Color::rgb(0.9, 0.4, 0.1)));
        member.appearance.effects = vec![Effect { id: "texture.craquelure".into(), params: json!({"crackSpacing": 6}), visible: true }];
        let mut group = Node::group(gid, vec![std::sync::Arc::new(still), std::sync::Arc::new(member)]);
        group.appearance.effects = vec![
            Effect { id: "stylize.dropShadow".into(), params: json!({"x": 4, "y": 3, "blur": 2}), visible: true },
            Effect { id: "blur.gaussian".into(), params: json!({"radius": 1}), visible: true },
        ];
        let l = d.layers[0].id;
        d.insert(Some(l), 0, group).unwrap();
        let (view, opts) = (Affine::scale(2.0), RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() });
        let mut r = Renderer::new();
        let _ = r.render(&d, 200, 200, view, &opts);
        let mut moved = d.clone();
        moved.node_mut(mid).unwrap().transform(Affine::translate((3.3, 1.2)), false);
        let _ = r.render(&moved, 200, 200, view, &RenderOptions { interacting: true, ..opts.clone() });
        assert!(r.stats.moved > 0, "the member's filtered content moved");
        let after = r.render(&moved, 200, 200, view, &opts).pixels;
        assert!(after == Renderer::new().render(&moved, 200, 200, view, &opts).pixels, "the frame after the drag");
    }

    /// Reproduce #1257's moving, filtered rectangle: the first frame renders as before a drag,
    /// and the next eight render as the canvas renders a drag's frames (`interacting`). Then the
    /// eighth position renders as the canvas renders the frame after the drag, and that frame
    /// renders once more with every raster cached.
    /// Run with `cargo test -p vectorcraft-render texture_drag_perf -- --ignored --nocapture`.
    #[test]
    #[ignore = "manual timing probe; wall-clock performance depends on machine load"]
    fn texture_drag_perf() {
        for effect in ["texture.craquelure", "texture.mosaicTiles", "texture.stainedGlass"] {
            let mut base = Document::new(600.0, 500.0);
            base.color_mode = vectorcraft_doc::ColorMode::Cmyk;
            let (layer, id) = (base.layers[0].id, base.alloc_id());
            let mut n = Node::path(
                id,
                shapes::rectangle(Rect::new(100.0, 100.0, 309.298, 370.938)),
                Appearance::basic(Paint::solid(Color::cmyk(0.0, 1.0, 1.0, 0.0)), Paint::solid(Color::BLACK), 1.0),
            );
            n.appearance.effects = vec![
                Effect { id: "stylize.roundCorners".into(), params: json!({"radius": 25.175}), visible: true },
                Effect { id: effect.into(), params: json!({}), visible: true },
            ];
            base.insert(Some(layer), 0, n).unwrap();
            let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
            let live = RenderOptions { interacting: true, ..opts.clone() };
            let view = Affine::scale(1.9007 * 1.5);
            let mut r = Renderer::new();
            r.threads = crate::default_threads();
            let (mut ms, mut doc) = (vec![], base.clone());
            for frame in 0..9 {
                doc = base.clone();
                doc.node_mut(id).unwrap().transform(Affine::translate((3.0 + f64::from(frame) * 0.75, f64::from(frame) * 0.5)), false);
                let start = std::time::Instant::now();
                let image = r.render(&doc, 1800, 1500, view, if frame > 0 { &live } else { &opts });
                let time = start.elapsed().as_secs_f64() * 1000.0;
                if frame > 0 {
                    ms.push(time);
                }
                std::hint::black_box(image);
            }
            let mut time = |o: &RenderOptions| {
                let start = std::time::Instant::now();
                std::hint::black_box(r.render(&doc, 1800, 1500, view, o));
                start.elapsed().as_secs_f64() * 1000.0
            };
            let (after, again) = (time(&opts), time(&opts));
            ms.sort_by(f64::total_cmp);
            println!(
                "{effect}: moved CMYK rounded rectangle at 190.07%, DPI 1.5, 1800x1500: median {:.2} ms; samples {ms:?}; the frame after the drag {after:.2} ms, that frame again {again:.2} ms",
                (ms[3] + ms[4]) * 0.5
            );
        }
    }

    /// Reproduce #2558's drag of a textured or blurred rectangle when zoomed in, in a 2560 × 1440
    /// view centered on the rectangle: the first frame renders as before a drag, and the next eight
    /// render as a drag's frames, once with `interacting` alone and once with `dragging` as well (as
    /// the canvas renders them). Then the eighth position renders as the canvas renders the frame
    /// after the drag. The hashes of that frame and of a fresh renderer's frame compare builds.
    /// Run with `cargo test -p vectorcraft-render zoomed_drag_perf -- --ignored --nocapture`.
    #[test]
    #[ignore = "manual timing probe; wall-clock performance depends on machine load"]
    fn zoomed_drag_perf() {
        let sets: [(&str, Vec<(&str, serde_json::Value)>); 3] = [
            ("Texturizer", vec![("texture.texturizer", json!({}))]),
            ("Gaussian Blur 8 pt", vec![("blur.gaussian", json!({"radius": 8}))]),
            ("Drop Shadow and Craquelure", vec![("stylize.dropShadow", json!({"blur": 7})), ("texture.craquelure", json!({}))]),
        ];
        let hash = |px: &[u8]| {
            use std::hash::{Hash, Hasher};
            let mut h = std::hash::DefaultHasher::new();
            px.hash(&mut h);
            h.finish()
        };
        for (name, effects) in sets {
            for zoom in [1.0, 4.0, 16.0, 64.0] {
                let base = doc_with(Rect::new(100.0, 100.0, 300.0, 250.0), Color::rgb(0.9, 0.4, 0.1), effects.clone());
                let id = base.layers[0].children().unwrap()[0].id;
                let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
                let view = Affine::translate((1280.0, 720.0)) * Affine::scale(zoom) * Affine::translate((-200.0, -175.0));
                for dragging in [false, true] {
                    let live = RenderOptions { interacting: true, dragging, ..opts.clone() };
                    let mut r = Renderer::new();
                    r.threads = crate::default_threads();
                    let (mut ms, mut doc) = (vec![], base.clone());
                    for frame in 0..9 {
                        doc = base.clone();
                        let by = (f64::from(frame) * 0.75 / zoom, f64::from(frame) * 0.5 / zoom);
                        doc.node_mut(id).unwrap().transform(Affine::translate(by), false);
                        let start = std::time::Instant::now();
                        std::hint::black_box(r.render(&doc, 2560, 1440, view, if frame > 0 { &live } else { &opts }));
                        if frame > 0 {
                            ms.push(start.elapsed().as_secs_f64() * 1000.0);
                        }
                    }
                    let previewed = r.stats.previewed;
                    let start = std::time::Instant::now();
                    let still = r.render(&doc, 2560, 1440, view, &opts);
                    let after = start.elapsed().as_secs_f64() * 1000.0;
                    let fresh = Renderer::new().render(&doc, 2560, 1440, view, &opts);
                    ms.sort_by(f64::total_cmp);
                    println!(
                        "{name} at {:.0}%, dragging {dragging}: drag frames median {:.1} ms (min {:.1}, max {:.1}), {previewed} previewed; the frame after the drag {after:.1} ms; hashes {:016x} {:016x}",
                        zoom * 100.0,
                        (ms[3] + ms[4]) * 0.5,
                        ms[0],
                        ms[7],
                        hash(&still.pixels),
                        hash(&fresh.pixels)
                    );
                }
            }
        }
    }

    #[test]
    fn photoshop_filters_on_content_larger_than_the_budget() {
        // Zoomed in 200×: the 40 pt square covers 8000 × 8000 px around a 200 × 200 view.
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let view = Affine::translate((100.0, 100.0)) * Affine::scale(200.0) * Affine::translate((-70.0, -50.0));
        for (id, p) in [("sharpen.unsharpMask", json!({"amount": 200})), ("blur.smart", json!({})), ("blur.radial", json!({"quality": "draft"}))] {
            let d = doc_with(Rect::new(30.0, 30.0, 70.0, 70.0), Color::rgb(0.2, 0.4, 0.8), vec![(id, p)]);
            let img = Renderer::new().render(&d, 200, 200, view, &opts);
            // The view shows the square's right side half way down: inside left, outside right.
            assert!(lum(img.pixel(40, 100)) < 600, "{id}: inside {:?}", img.pixel(40, 100));
            // (Smart Blur's wide samples read a blurred copy: a faint fringe under its threshold.)
            assert!(lum(img.pixel(190, 100)) >= WHITE - 3 * 25, "{id}: outside {:?}", img.pixel(190, 100));
        }
    }
}
