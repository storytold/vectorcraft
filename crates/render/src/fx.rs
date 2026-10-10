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
//!
//! The same applies to one fill or stroke's own raster effects (around that item alone) and to
//! type, images, symbol instances and live objects ([`Renderer::draw_object_fx`]): their art is
//! drawn offscreen in the filter layers, reshaped first by their geometry effects. Groups and
//! layers with an appearance of their own take the same path with their evaluated art
//! ([`effects::evaluate_container`]: their fills and strokes painting the members, their geometry
//! effects reshaping them), so their raster effects apply to the composite (one combined shadow).

use std::sync::Arc;

use vectorcraft_doc::{AppearanceItem, Effect, Node, NodeKind};
use vectorcraft_effects::{self as effects, GeomContext, RasterFx};
use vectorcraft_geom::{Affine, BezPath, FillRule, Rect, Shape, Vec2};
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
/// symbol instances, live objects; Crop Marks and colour adjustments on anything), or is it a group
/// or layer with an appearance of its own (see [`Renderer::draw_object_fx`])?
pub(crate) fn has_object_fx(n: &Node) -> bool {
    (effects::needs_outline(n) && (visible(&n.appearance.effects) || n.projection().is_some()))
        || effects::has_container_appearance(n)
        || effects::has_crop_marks(n)
        || effects::has_adjustment(n)
}

/// Is `n` a group or layer (whose evaluated art keeps its knockout setting)?
fn is_container(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. })
}

/// What [`Renderer::raster_fx`] paints effects around.
struct Content<'a> {
    /// The object the effects belong to (cached shadows are checked against it).
    node: &'a Node,
    /// First shadow-cache slot of these effects (`None`: don't cache).
    cache: Option<usize>,
    /// Document-space bounds of the painted content.
    reach: Rect,
    /// Its silhouette: Feather and Inner Glow clip to it (without one, Feather only blurs and
    /// Inner Glow is skipped).
    outline: Option<(&'a BezPath, FillRule)>,
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

/// Visual bounds including geometry effects, stroke outsets and shadows/glows.
pub(crate) fn visual_bounds(n: &Node) -> Option<Rect> {
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
        let o = effects::outset(&n.appearance.effects, r);
        r.inflate(o, o)
    })
}

/// Bounds used for culling: like `Node::visual_bounds`, but aware of live effects anywhere in
/// the subtree (a shadow can reach the view while its object is outside it).
pub(crate) fn cull_bounds(n: &Node) -> Option<Rect> {
    match &n.kind {
        // The object and its crop marks.
        _ if effects::has_crop_marks(n) => effects::crop_marks_art(n).and_then(|art| cull_bounds(&art)),
        // The evaluated art (its fills, strokes and geometry effects) and the raster effects' reach.
        _ if effects::has_container_appearance(n) => {
            let art = effects::evaluate_container(n);
            let art = art.as_ref().unwrap_or(n);
            let b = if art.clips() {
                art.visual_bounds()
            } else {
                art.children()?.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, cull_bounds(c)))
            }?;
            let o = effects::outset(&n.appearance.effects, b);
            Some(b.inflate(o, o))
        }
        NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
            children.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, cull_bounds(c)))
        }
        _ if has_fx(n) => visual_bounds(n),
        // The reshaped art (a symbol's art needs the document: its instance box stands in).
        _ if has_object_fx(n) => {
            let b = effects::reshape(n, None).and_then(|r| r.visual_bounds()).or_else(|| n.visual_bounds())?;
            let o = effects::outset(&n.appearance.effects, b);
            Some(b.inflate(o, o))
        }
        _ => n.visual_bounds(),
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

fn pcolor(ink: Ink, c: &vectorcraft_doc::color::Color) -> peniko::Color {
    let [r, g, b] = ink.rgb(c);
    peniko::Color::new([r, g, b, 1.0])
}

/// Most pixels the raster of [`Renderer::pixel_content`] holds (a 4K screen's): larger content is
/// cropped to the view when its effects only look nearby, else rendered at a lower resolution.
const PIXEL_BUDGET: f64 = 8.4e6;

/// The effects [`Renderer::pixel_content`] runs on the content's pixels (those changing the
/// content itself), in stack order.
fn content_changing(fx: &&RasterFx) -> bool {
    matches!(fx, RasterFx::Feather { .. } | RasterFx::GaussianBlur { .. } | RasterFx::Pixel(_))
}

/// How far (document units) from a pixel the content deciding it after `fx` lies; `None` when
/// anywhere in the object.
fn looks_around(fx: &RasterFx) -> Option<f64> {
    match fx {
        RasterFx::Feather { radius } | RasterFx::GaussianBlur { radius } => Some(1.5 * radius),
        RasterFx::Pixel(p) => p.reach(),
        _ => Some(0.0),
    }
}

/// The content of an object with Photoshop-style effects as filtered pixels, drawn at
/// (`anchor`'s view position + `rel`) scaled by 1 / `k` (see [`Renderer::pixel_content`]).
pub(crate) struct PixelEntry {
    node: Node,
    linear: [u64; 4],
    anchor: vectorcraft_geom::Point,
    rel: (f64, f64),
    k: f64,
    image: std::sync::Arc<vello_cpu::Pixmap>,
    pub(crate) stamp: u64,
}

/// The four linear coefficients of `view` (zoom and rotation, not the pan), as cache keys.
fn linear_key(view: Affine) -> [u64; 4] {
    let c = view.as_coeffs();
    [c[0].to_bits(), c[1].to_bits(), c[2].to_bits(), c[3].to_bits()]
}

/// Draw `image` with its top-left at (`x`, `y`) device pixels, `1 / k` device pixels a pixel.
fn draw_scaled(ctx: &mut RenderContext, image: std::sync::Arc<vello_cpu::Pixmap>, (x, y): (f64, f64), k: f64) {
    let (w, h) = (image.width() as f64, image.height() as f64);
    ctx.set_transform(Affine::translate((x, y)) * Affine::scale(1.0 / k));
    // On whole pixels at full size the cheapest sampler is exact.
    let quality = if k < 1.0 { peniko::ImageQuality::Medium } else { peniko::ImageQuality::Low };
    let sampler = peniko::ImageSampler { quality, ..Default::default() };
    ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(image), sampler });
    ctx.fill_rect(&Rect::new(0.0, 0.0, w, h));
    ctx.set_transform(Affine::IDENTITY);
}

/// A cached shadow / glow raster: premultiplied, already tinted and faded, positioned relative to
/// the view-space position of `anchor` (a document point).
pub(crate) struct ShadowEntry {
    node: Node,
    linear: [u64; 4],
    anchor: vectorcraft_geom::Point,
    /// Pixel offset of the raster's top-left from the anchor's view position.
    rel: (f64, f64),
    image: std::sync::Arc<vello_cpu::Pixmap>,
    pub(crate) stamp: u64,
}

impl Renderer {
    /// Drop shadow / outer glow `fx` (shadow-cache slot `i` of `n`) from a cached raster:
    /// the silhouette is rendered once into a small crop, blurred and tinted on the CPU, and reused
    /// while the object and the zoom/rotation are unchanged (pans only move it). Returns false when
    /// the effect isn't a shadow/glow or the raster would be too large (caller falls back).
    #[allow(clippy::too_many_arguments)]
    fn draw_shadow_cached(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        n: &Node,
        i: usize,
        fx: &RasterFx,
        reach: Rect,
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) -> bool {
        let (mode, opacity, dx, dy, blur, color) = match fx {
            RasterFx::DropShadow { mode, opacity, dx, dy, blur, color } => (*mode, *opacity, *dx, *dy, *blur, *color),
            RasterFx::OuterGlow { mode, opacity, blur, color } => (*mode, *opacity, 0.0, 0.0, *blur, *color),
            _ => return false,
        };
        let linear = linear_key(f.view);
        let key = (n as *const Node as usize, i, f.ink);
        let stamp = self.stamp;
        let hit = self.shadows.get_mut(&key).filter(|e| e.linear == linear && e.node == *n);
        let (image, x, y) = match hit {
            Some(e) => {
                e.stamp = stamp;
                let p = f.view * e.anchor;
                (e.image.clone(), (p.x + e.rel.0).round(), (p.y + e.rel.1).round())
            }
            None => {
                let spread = blur.max(0.0) * 1.5 / f.px + 2.0;
                let reach = reach + Vec2::new(dx, dy);
                let r = f.view.transform_rect_bbox(reach).inflate(spread, spread);
                let (x0, y0) = (r.x0.floor(), r.y0.floor());
                let (w, h) = (r.x1.ceil() - x0, r.y1.ceil() - y0);
                if !(1.0..=8192.0).contains(&w) || !(1.0..=8192.0).contains(&h) || w * h > 4.0e6 {
                    return false;
                }
                let (w, h) = (w as u16, h as u16);
                // Silhouette (the object's painted alpha), offset by the shadow distance.
                let mut off = f.offscreen_context(w, h);
                let crop = Frame { mt: false, view: Affine::translate((-x0, -y0)) * f.view, ..*f };
                self.inside_layer(|r| paint(r, &mut off, &shifted(&crop, dx, dy)));
                off.flush();
                let mut pm = vello_cpu::Pixmap::new(w, h);
                off.render(&mut pm, &mut self.resources);
                let (wu, hu) = (w as usize, h as usize);
                let mut a: Vec<f32> = pm.data().iter().map(|p| p.a as f32).collect();
                effects::pixel::blur_plane(&mut a, wu, hu, blur / 2.0 / f.px);
                let [cr, cg, cb] = f.ink.rgb(&color);
                let tint = |v: f32, ch: f32| (v * ch).round().clamp(0.0, 255.0) as u8;
                for (px, av) in pm.data_mut().iter_mut().zip(&a) {
                    let al = (av * opacity).clamp(0.0, 255.0);
                    *px = vello_cpu::color::PremulRgba8 { r: tint(al, cr), g: tint(al, cg), b: tint(al, cb), a: al.round() as u8 };
                }
                let image = std::sync::Arc::new(pm);
                let anchor = reach.origin();
                let p = f.view * anchor;
                self.shadows.insert(key, ShadowEntry { node: n.clone(), linear, anchor, rel: (x0 - p.x, y0 - p.y), image: image.clone(), stamp });
                (image, x0, y0)
            }
        };
        // The blend mode applies per draw: a blend layer would be a full-viewport compositing pass
        // per shadow. The raster sits on whole pixels, so the cheapest sampler is exact.
        let (w, h) = (image.width() as f64, image.height() as f64);
        ctx.set_transform(Affine::translate((x, y)));
        ctx.set_blend_mode(blend_mode(mode));
        let sampler = peniko::ImageSampler { quality: peniko::ImageQuality::Low, ..Default::default() };
        ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(image), sampler });
        ctx.fill_rect(&Rect::new(0.0, 0.0, w, h));
        ctx.set_blend_mode(blend_mode(vectorcraft_doc::color::BlendMode::Normal));
        ctx.set_transform(Affine::IDENTITY);
        true
    }

    /// `draw_shape` for nodes with live effects.
    pub(crate) fn draw_shape_fx(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, bp: &BezPath, rule: FillRule) {
        let g = effected_path(n, bp);
        if g.elements().is_empty() {
            return;
        }
        if f.opts.outline {
            self.hairline(ctx, f, &g, [0, 0, 0, 255]);
            return;
        }
        let gctx = GeomContext::of(n);
        let rfx = effects::raster_effects(&n.appearance.effects);
        // Document-space reach of the painted geometry (strokes, arrowheads…).
        let outset = n.appearance.outset();
        let reach = g.bounding_box().inflate(outset, outset);
        let content = Content { node: n, cache: Some(0), reach, outline: Some((&g, rule)) };
        self.raster_fx(ctx, f, &content, &rfx, &mut |r, c, fr| r.paint_items(c, fr, n, &g, rule, &gctx));
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
            let bounds = if blends { self.bounds_of(a) } else { None };
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
        // With Crop Marks, the object inside the art paints its own raster effects.
        let rfx = if effects::has_crop_marks(a) { vec![] } else { effects::raster_effects(&a.appearance.effects) };
        match cull_bounds(&art) {
            Some(reach) if !f.opts.outline && !rfx.is_empty() => {
                // Feather and Inner Glow clip to the art's outline.
                let clipped = rfx.iter().any(|x| matches!(x, RasterFx::Feather { .. } | RasterFx::InnerGlow { .. }));
                let region = if clipped { self.clip_of(&art) } else { None };
                // A symbol's art can change while the instance doesn't: no cached shadows.
                let cache = (cache && !matches!(a.kind, NodeKind::SymbolInstance { .. })).then_some(0);
                let content = Content { node: a, cache, reach, outline: region.as_deref().map(|r| (&r.0, r.1)) };
                self.raster_fx(ctx, f, &content, &rfx, &mut |r, c, fr| r.draw_node(c, fr, &art, true));
                ctx.set_transform(Affine::IDENTITY);
            }
            _ => self.draw_node(ctx, f, &art, true),
        }
    }

    /// The art [`Self::draw_object_fx`] draws for `a` (cached by `Arc` identity when `cache`):
    /// reshaped by its geometry effects, else type, images and paths as themselves and the others
    /// as their evaluated art (groups and layers: [`effects::evaluate_container`], keeping their
    /// knockout setting), recoloured by the colour adjustments in it ([`Self::adjusted_art`]);
    /// without the object's transparency and effects (a path keeps its geometry effects).
    fn fx_art(&mut self, doc: &vectorcraft_doc::Document, a: &Arc<Node>, cache: bool) -> Arc<Node> {
        let key = Arc::as_ptr(a) as usize;
        if cache
            && let Some((node, art)) = self.fx_arts.get(&key)
            && Arc::ptr_eq(node, a)
        {
            return art.clone();
        }
        let symbol = match &a.kind {
            NodeKind::SymbolInstance { symbol, .. } => {
                doc.symbols.iter().find(|s| s.name == *symbol).map(|s| crate::brush_fx::instance_art(&s.art, a))
            }
            _ => None,
        };
        let container = is_container(a);
        let art = match effects::crop_marks_art(a).or_else(|| effects::reshape(a, symbol.as_ref())) {
            Some(r) => r,
            None if container => effects::evaluate_container(a).unwrap_or_else(|| (**a).clone()),
            None if matches!(a.kind, NodeKind::Text(_) | NodeKind::Image(_) | NodeKind::Path { .. } | NodeKind::Compound { .. }) => (**a).clone(),
            None => effects::outline_art(a, symbol.as_ref()).unwrap_or_else(|| Node::group(a.id, vec![])),
        };
        let mut art = self.adjusted_art(doc, art, effects::color_map(&a.appearance.effects).as_ref());
        // A path's geometry effects still apply when it is drawn.
        let path = matches!(art.kind, NodeKind::Path { .. } | NodeKind::Compound { .. });
        art.appearance.effects.retain(|e| path && effects::is_geometry(&e.id));
        let art = Arc::new(Node {
            opacity: 1.0,
            blend: Default::default(),
            isolate: false,
            // One element: its pieces never knock each other out (a group's members still do as
            // its knockout setting says).
            knockout: if container { a.knockout } else { vectorcraft_doc::Knockout::Off },
            mask: None,
            ..art
        });
        if cache {
            if self.fx_arts.len() > 1024 {
                self.fx_arts.clear();
            }
            self.fx_arts.insert(key, (a.clone(), art.clone()));
        }
        art
    }

    /// `art` with the colour adjustments inside it applied, then `outer` (its object's own) on
    /// everything ([`effects::adjust`]). Embedded images become recoloured copies, cached with the
    /// decoded images under a key naming the adjustment.
    fn adjusted_art(&mut self, doc: &vectorcraft_doc::Document, mut art: Node, outer: Option<&effects::ColorMap>) -> Node {
        art.appearance.effects.retain(|e| !effects::is_adjustment(&e.id));
        let mut wanted: Vec<(String, String, effects::ColorMap)> = vec![];
        let adjusted = effects::adjust(&art, outer, &mut |key, map| {
            let mut h = std::hash::DefaultHasher::new();
            std::hash::Hash::hash(&format!("{map:?}"), &mut h);
            let derived = format!("{key}\u{0}adjust\u{0}{:016x}", std::hash::Hasher::finish(&h));
            wanted.push((key.to_string(), derived.clone(), map.clone()));
            Some(derived)
        });
        for (key, derived, map) in wanted {
            self.adjusted_image(doc, &key, derived, &map);
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
        let reach = content.reach;
        // Below the content: shadows and outer glows.
        for (i, fx) in rfx.iter().enumerate().filter(|(_, x)| x.is_below()) {
            if let Some(slot) = content.cache
                && self.draw_shadow_cached(ctx, f, content.node, slot + i, fx, reach, paint)
            {
                continue;
            }
            let (mode, opacity, dx, dy, blur, color) = match fx {
                RasterFx::DropShadow { mode, opacity, dx, dy, blur, color } => (*mode, *opacity, *dx, *dy, *blur, *color),
                RasterFx::OuterGlow { mode, opacity, blur, color } => (*mode, *opacity, 0.0, 0.0, *blur, *color),
                _ => continue,
            };
            let filter = shadow_filter(0.0, 0.0, blur, pcolor(f.ink, &color));
            // The offset moves the content rather than the filter: vello_cpu drops layer content
            // that lies entirely outside the viewport before filtering.
            let reach = reach + Vec2::new(dx, dy);
            self.with_filters(ctx, f, reach, blur, Some((mode, opacity)), |r, c, fr, comp| {
                c.set_transform(fr.view);
                c.push_layer(None, comp.map(|m| blend_mode(m.0)), comp.map(|m| m.1), None, Some(filter));
                paint(r, c, &shifted(fr, dx, dy));
                c.pop_layer();
            });
        }
        // The content itself (blurred / feathered / filtered), on its pixels: a filter layer's blur
        // works on a coarser grid tied to the canvas, so the blurred art shifted by up to a pixel
        // and looked blocky as the view moved by parts of a pixel (#755).
        let blurs = rfx.iter().any(|fx| matches!(fx, RasterFx::Feather { radius } | RasterFx::GaussianBlur { radius } if *radius > 0.0));
        if blurs || rfx.iter().any(|fx| matches!(fx, RasterFx::Pixel(_))) {
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

    /// Paint `content` through the content-changing effects of `rfx` (Gaussian Blur, Feather and the
    /// Photoshop-style filters) in stack order on its pixels: rendered offscreen over all it can
    /// reach, filtered on the CPU and drawn back. The raster holds at most [`PIXEL_BUDGET`] pixels:
    /// larger content is cropped to the view when every effect only looks nearby, else rendered
    /// at a lower resolution. A whole raster at full resolution is cached while the object and the
    /// zoom stay the same (panning only moves it).
    fn pixel_content(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        content: &Content,
        rfx: &[RasterFx],
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) {
        let chain: Vec<&RasterFx> = rfx.iter().filter(content_changing).collect();
        // Where the result can paint: each effect spreads what the ones before it painted.
        let reach = chain.iter().fold(content.reach, |r, fx| {
            let o = fx.outset(r);
            r.inflate(o, o)
        });
        let full = f.view.transform_rect_bbox(reach).inflate(2.0, 2.0);
        let (sw, sh) = (ctx.width() as f64, ctx.height() as f64);
        if !(full.area().is_finite() && full.x1 > 0.0 && full.y1 > 0.0 && full.x0 < sw && full.y0 < sh) {
            return;
        }
        let fits = full.area() <= PIXEL_BUDGET;
        // Device pixels from a pixel to the content deciding it (`None`: anywhere).
        let margin = chain.iter().try_fold(2.0, |m, fx| looks_around(fx).map(|d| m + d / f.px));
        let region = match margin {
            Some(m) if !fits => full.intersect(Rect::new(0.0, 0.0, sw, sh).inflate(m, m)),
            _ => full,
        };
        let (x0, y0) = (region.x0.floor(), region.y0.floor());
        let (rw, rh) = ((region.x1 - x0).max(1.0), (region.y1 - y0).max(1.0));
        let side = u16::MAX as f64;
        let k = (PIXEL_BUDGET / (rw * rh)).sqrt().min(side / rw).min(side / rh).min(1.0);
        let (w, h) = ((rw * k).ceil().clamp(1.0, side), (rh * k).ceil().clamp(1.0, side));
        // The cached raster, when the whole content was rendered at full size at this zoom.
        let linear = linear_key(f.view);
        let key = content.cache.filter(|_| fits && k >= 1.0).map(|slot| (content.node as *const Node as usize, slot, f.ink));
        let stamp = self.stamp;
        if let Some(e) = key.and_then(|key| self.pixel_fx.get_mut(&key)).filter(|e| e.linear == linear && e.node == *content.node) {
            e.stamp = stamp;
            let p = f.view * e.anchor;
            let (image, at, k) = (e.image.clone(), ((p.x + e.rel.0).round(), (p.y + e.rel.1).round()), e.k);
            draw_scaled(ctx, image, at, k);
            return;
        }
        let (wu, hu) = (w as u16, h as u16);
        let view = Affine::scale(k) * Affine::translate((-x0, -y0)) * f.view;
        let to_doc = view.inverse();
        let px = f.px / k;
        let frame = Frame { mt: false, view, visible: to_doc.transform_rect_bbox(Rect::new(0.0, 0.0, w, h)), px, ..*f };
        let mut off = f.offscreen_context(wu, hu);
        self.inside_layer(|r| paint(r, &mut off, &frame));
        off.flush();
        let mut pm = vello_cpu::Pixmap::new(wu, hu);
        off.render(&mut pm, &mut self.resources);
        let channels = match f.ink {
            Ink::Cmy => effects::pixel::Channels::CmyPlane,
            Ink::K => effects::pixel::Channels::KPlane,
            Ink::Display if f.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk => effects::pixel::Channels::Cmyk,
            Ink::Display => effects::pixel::Channels::Rgb,
        };
        let space = effects::PixelSpace { to_doc, px, center: content.reach.center(), channels };
        let (wz, hz) = (wu as usize, hu as usize);
        for fx in chain {
            match fx {
                RasterFx::GaussianBlur { radius } => effects::pixel::gaussian_rgba(pm.data_as_u8_slice_mut(), wz, hz, radius / 2.0 / px),
                RasterFx::Feather { radius } => {
                    effects::pixel::gaussian_rgba(pm.data_as_u8_slice_mut(), wz, hz, radius / 2.0 / px);
                    if let Some((g, rule)) = content.outline {
                        self.clip_pixels(&mut pm, &frame, g, rule);
                    }
                }
                RasterFx::Pixel(p) => p.apply(pm.data_as_u8_slice_mut(), wz, hz, &space),
                _ => {}
            }
        }
        let image = std::sync::Arc::new(pm);
        if let Some(key) = key {
            let anchor = content.reach.origin();
            let p = f.view * anchor;
            let rel = (x0 - p.x, y0 - p.y);
            self.pixel_fx.insert(key, PixelEntry { node: content.node.clone(), linear, anchor, rel, k, image: image.clone(), stamp });
        }
        draw_scaled(ctx, image, (x0, y0), k);
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
        // 3σ of the Gaussian (σ = blur / 2) in pixels, plus a pixel of antialiasing.
        let spread = blur.max(0.0) * 1.5 / f.px + 2.0;
        let screen = Rect::new(0.0, 0.0, ctx.width() as f64, ctx.height() as f64).inflate(spread, spread);
        let r = f.view.transform_rect_bbox(reach).inflate(spread, spread).intersect(screen);
        let (x0, y0) = (r.x0.floor(), r.y0.floor());
        let (w, h) = ((r.x1.ceil() - x0).min(u16::MAX as f64), (r.y1.ceil() - y0).min(u16::MAX as f64));
        if w < 1.0 || h < 1.0 {
            return;
        }
        let (w, h) = (w as u16, h as u16);
        let mut off = f.offscreen_context(w, h);
        let shifted = Frame { mt: false, view: Affine::translate((-x0, -y0)) * f.view, ..*f };
        self.inside_layer(|r| draw(r, &mut off, &shifted, None));
        off.flush();
        let mut pm = vello_cpu::Pixmap::new(w, h);
        off.render(&mut pm, &mut self.resources);
        let layered = composite.is_some_and(|(m, o)| m != vectorcraft_doc::color::BlendMode::Normal || o < 1.0);
        if let Some((m, o)) = composite.filter(|_| layered) {
            ctx.set_transform(Affine::IDENTITY);
            ctx.push_layer(None, Some(blend_mode(m)), Some(o), None, None);
        }
        crate::group::draw_pixmap(ctx, std::sync::Arc::new(pm), (x0, y0));
        if layered {
            ctx.pop_layer();
        }
    }

    /// Paint the fills and strokes of `n` on geometry `g`, applying per-item geometry effects and
    /// painting per-item raster effects around that item alone.
    fn paint_items(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, g: &BezPath, rule: FillRule, gctx: &GeomContext) {
        let bounds = g.bounding_box();
        for (j, item) in n.appearance.items.iter().enumerate() {
            if !item.visible() || item.paint().is_none() {
                continue;
            }
            let ig = apply(item_effects(item), g, &gctx.item(item));
            let ib = if effects::has_geometry(item_effects(item)) && !ig.elements().is_empty() { ig.bounding_box() } else { bounds };
            let rfx = effects::raster_effects(item_effects(item));
            if rfx.is_empty() {
                self.paint_item(ctx, f, n, item, &ig, rule, ib);
                continue;
            }
            // The item's silhouette: its fill area, or the outline of its stroke.
            let outline = match item {
                AppearanceItem::Fill(_) => ig.clone(),
                AppearanceItem::Stroke(st) => {
                    let w = effects::stroke::aligned_width(st, effects::stroke::is_closed(&ig));
                    effects::stroke::line_outline(&effects::stroke::stroke_pieces(&ig, st).line, st, w, f.px * 0.25)
                }
            };
            let outset = n.appearance.outset();
            let orule = if item.is_fill() { rule } else { FillRule::NonZero };
            // Each item's shadows get their own cache slots.
            let content = Content { node: n, cache: Some((j + 1) << 8), reach: ib.inflate(outset, outset), outline: Some((&outline, orule)) };
            self.raster_fx(ctx, f, &content, &rfx, &mut |r, c, fr| r.paint_item(c, fr, n, item, &ig, rule, ib));
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
                self.draw_stroke(ctx, f, Affine::IDENTITY, ig, rule, st, ib);
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

    use crate::{RenderOptions, Rendered, Renderer};

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
        n.appearance = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 4.0);
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
        let b = super::visual_bounds(n).unwrap();
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
