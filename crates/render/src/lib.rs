//! VectorCraft renderer: document → premultiplied RGBA pixels.
//!
//! The backend is `vello_cpu` (SIMD, sparse strips). Callers give a *view transform* mapping
//! document points to output pixels; the renderer culls by bounds, evaluates appearance stacks
//! (multiple fills/strokes, opacity, blend modes, stroke alignment, dashes), clip groups,
//! gradients, images and text (via `vectorcraft-text` glyph outlines).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod brush_fx;
mod fx;
mod group;
mod live;
mod paint;
mod pattern;
pub mod proof;

use std::collections::HashMap;
use std::sync::Arc;

use vectorcraft_doc::appearance::stroke_paint_bounds;
use vectorcraft_doc::{AppearanceItem, Document, Node, NodeId, NodeKind, StrokeAlign, StrokeLayer, TextObject};
use vectorcraft_geom::{Affine, BezPath, FillRule, Rect, Shape};
use vello_cpu::kurbo;
use vello_cpu::peniko::{self, BlendMode, Compose, Mix};
use vello_cpu::{Pixmap, RenderContext, Resources};

use group::Composite;

pub use effects::stroke::width_outline;
pub use live::expand_live;
pub use pattern::render_pattern_swatch;
pub use vectorcraft_effects as effects;
pub use vello_cpu;

/// Largest raster an export may ask [`Renderer::render_region`] for, per side. The CPU rasteriser
/// addresses at most `u16::MAX` pixels per side (and panics at that edge).
pub const MAX_RASTER_SIDE: u32 = 32_768;
/// Largest raster an export may ask for in total (16384², 1 GiB of RGBA pixels).
pub const MAX_RASTER_PIXELS: u64 = 1 << 28;

/// Pixel size of `region` rendered at `scale` (as [`Renderer::render_region`] rounds it), or an error
/// when it exceeds [`MAX_RASTER_SIDE`] / [`MAX_RASTER_PIXELS`]. Raster exports check this first,
/// because the renderer can only clamp (wrong picture) or fail (abort) at those sizes.
pub fn raster_size(region: Rect, scale: f64) -> Result<(u32, u32), String> {
    if !(scale.is_finite() && scale > 0.0) {
        return Err(format!("invalid scale {scale}"));
    }
    let w = (region.width() * scale).round().max(1.0);
    let h = (region.height() * scale).round().max(1.0);
    let max = f64::from(MAX_RASTER_SIDE);
    if !(w <= max && h <= max && w * h <= MAX_RASTER_PIXELS as f64) {
        return Err(format!(
            "the image would be {w:.0} × {h:.0} pixels; raster exports are limited to {MAX_RASTER_SIDE} pixels per side and {} megapixels: lower the scale or resolution",
            MAX_RASTER_PIXELS / 1_000_000
        ));
    }
    Ok((w as u32, h as u32))
}

/// Rendering options.
#[derive(Clone, Debug)]
pub struct RenderOptions {
    /// Outline (wireframe) view: 1 px black paths, no paint.
    pub outline: bool,
    /// Pasteboard colour behind everything (premultiplied RGBA8); `None` = transparent.
    pub background: Option<[u8; 4]>,
    /// Draw artboards as white rectangles (screen view). Export sets this to false.
    pub artboards: bool,
    /// Objects not to draw (e.g. the object being edited by a live drag preview).
    pub hidden: Vec<NodeId>,
    /// Draw template layers dimmed (50%).
    pub dim_templates: bool,
    /// Soft proof / separations preview (see [`proof`]).
    pub proof: Option<proof::ProofSetup>,
    /// Overprint Preview (see [`proof`] for what overprints).
    pub overprint_preview: bool,
    /// Trim View: clip the artwork to the artboards (nothing on the pasteboard is drawn).
    pub trim: bool,
    /// Leave template layers out (exports and thumbnails: templates are guides, not artwork).
    pub skip_templates: bool,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            outline: false,
            background: None,
            artboards: false,
            hidden: vec![],
            dim_templates: true,
            proof: None,
            overprint_preview: false,
            trim: false,
            skip_templates: false,
        }
    }
}

/// A rendered image (premultiplied RGBA8, row-major).
pub struct Rendered {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Rendered {
    /// Un-premultiplied RGBA8 copy.
    pub fn to_straight(&self) -> Vec<u8> {
        let mut out = self.pixels.clone();
        for px in out.chunks_exact_mut(4) {
            let a = px[3] as u32;
            if a != 0 && a != 255 {
                for c in &mut px[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
        }
        out
    }
    /// The encoders assert on a short buffer, so check it first.
    fn check_size(&self, what: &str) -> Result<(), String> {
        let want = u64::from(self.width) * u64::from(self.height) * 4;
        if self.pixels.len() as u64 == want {
            Ok(())
        } else {
            Err(format!("{what} export: {}×{} pixel buffer has the wrong size", self.width, self.height))
        }
    }
    /// Encode as PNG.
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        self.check_size("PNG")?;
        let mut buf = Vec::new();
        let img = image::RgbaImage::from_raw(self.width, self.height, self.to_straight())
            .ok_or_else(|| format!("PNG export: {}×{} pixel buffer has the wrong size", self.width, self.height))?;
        img.write_to(&mut std::io::Cursor::new(&mut buf), image::ImageFormat::Png).map_err(|e| format!("PNG export: {e}"))?;
        Ok(buf)
    }
    /// Encode as JPEG (flattened on white) at `quality` 1..=100.
    pub fn to_jpeg(&self, quality: u8) -> Result<Vec<u8>, String> {
        self.check_size("JPEG")?;
        let rgba = self.to_straight();
        let rgb: Vec<u8> = rgba
            .chunks_exact(4)
            .flat_map(|p| {
                let a = p[3] as u32;
                let mix = |c: u8| ((c as u32 * a + 255 * (255 - a)) / 255) as u8;
                [mix(p[0]), mix(p[1]), mix(p[2])]
            })
            .collect();
        let mut buf = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality.clamp(1, 100));
        image::ImageEncoder::write_image(enc, &rgb, self.width, self.height, image::ExtendedColorType::Rgb8)
            .map_err(|e| format!("JPEG export: {e}"))?;
        Ok(buf)
    }
    /// Encode as lossless WebP.
    pub fn to_webp(&self) -> Result<Vec<u8>, String> {
        self.check_size("WebP")?;
        let mut buf = Vec::new();
        let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut buf);
        image::ImageEncoder::write_image(enc, &self.to_straight(), self.width, self.height, image::ExtendedColorType::Rgba8)
            .map_err(|e| format!("WebP export: {e}"))?;
        Ok(buf)
    }
    /// Straight-alpha RGBA at (x, y).
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        let p = &self.pixels[i..i + 4];
        let a = p[3] as u32;
        if a == 0 {
            return [0, 0, 0, 0];
        }
        let un = |c: u8| ((c as u32 * 255 + a / 2) / a).min(255) as u8;
        [un(p[0]), un(p[1]), un(p[2]), p[3]]
    }
}

/// Hasher for pointer-keyed caches: a multiply-xorshift (SipHash showed up in frame profiles).
#[derive(Default, Clone, Copy)]
struct PtrHasher(u64);

impl std::hash::Hasher for PtrHasher {
    fn write(&mut self, bytes: &[u8]) {
        for b in bytes {
            self.0 = (self.0 ^ *b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
    fn write_u64(&mut self, n: u64) {
        let x = (self.0 ^ n).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        self.0 = x ^ (x >> 29);
    }
    fn write_usize(&mut self, n: usize) {
        self.write_u64(n as u64);
    }
    fn finish(&self) -> u64 {
        self.0
    }
}

type PtrMap<K, V> = HashMap<K, V, std::hash::BuildHasherDefault<PtrHasher>>;

/// A stroke expanded to a fill outline: (owning node, outline, last frame used).
struct StrokeEntry {
    node: Arc<Node>,
    outline: Arc<BezPath>,
    stamp: u64,
}

/// Cached per-node geometry, keyed by `Arc` identity. Structural sharing means an unchanged node
/// keeps its allocation across edits, so a pointer match (with the Arc kept alive here so the
/// address can't be reused) is an exact cache hit — no invalidation logic needed.
struct GeomEntry {
    node: Arc<Node>,
    bounds: Option<Rect>,
    path: Option<Arc<BezPath>>,
    stamp: u64,
}

/// A clipping path (kept alive so its address can't be reused) and the region it clips to.
type ClipEntry = (Arc<Node>, Option<Arc<(BezPath, FillRule)>>);

/// Reusable renderer (keeps the render context, decoded images and glyph caches between frames).
pub struct Renderer {
    texts: PtrMap<usize, (Arc<Node>, Arc<TextGeom>)>,
    /// Clip regions per clipping path (see [`Self::clip_of`]).
    clips: PtrMap<usize, ClipEntry>,
    /// Context reused when rendering single-threaded (`threads == 0`).
    ctx_st: Option<RenderContext>,
    /// Worker threads for the multithreaded rasterizer (0 = single-threaded).
    pub threads: u16,
    geom: PtrMap<usize, GeomEntry>,
    /// Expanded stroke outlines keyed by (stroke layer address, width bits, tolerance level):
    /// panning re-renders reuse them instead of re-expanding every stroke.
    strokes: PtrMap<(usize, u64, i32), StrokeEntry>,
    /// The node being drawn on the plain-path fast path (owner of cached stroke outlines).
    cur: Option<Arc<Node>>,
    stamp: u64,
    /// Opacity folded into paint alpha for the leaf being drawn (avoids a compositing layer).
    alpha: f32,
    ctx: Option<RenderContext>,
    resources: Resources,
    images: HashMap<String, Arc<Pixmap>>,
    /// Statistics of the last frame.
    pub stats: FrameStats,
    /// Brush art per brushed stroke.
    brushes: brush_fx::BrushCache,
    /// Evaluated blends/envelopes and tessellated meshes.
    live: live::LiveCache,
    /// Blurred, tinted drop shadow / outer glow rasters per object and effect (see `fx`).
    shadows: PtrMap<(usize, usize), fx::ShadowEntry>,
    /// Whether the group being drawn is a knockout group (what its neutral children inherit).
    knockout: bool,
    /// Address of the knockout-group element being drawn as its knockout shape: at full object
    /// opacity and without its opacity mask (see [`Self::draw_knockout`]); 0 = none.
    shape_of: usize,
    /// The art drawn for objects with effects that apply through it (see `fx::has_object_fx`).
    fx_arts: PtrMap<usize, (Arc<Node>, Arc<Node>)>,
    /// Layers open around the drawing point in the current context: a non-isolated group can copy
    /// its backdrop only when there are none (see [`Self::group`]).
    nested: u32,
    /// Copy of the backdrop of the non-isolated group being drawn offscreen, in its context's
    /// pixels: the elements of a non-isolated knockout group composite against it.
    backdrop: Option<Arc<Pixmap>>,
    /// [`Node::blends_through`] per container, keyed like `geom`.
    blends: PtrMap<usize, (Arc<Node>, bool, u64)>,
    /// Clip paths pushed on the current context (path, rule, transform): starting the context
    /// again from a picture of it pushes them again (see [`Self::group`]).
    clip_paths: Vec<(BezPath, FillRule, Affine)>,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct FrameStats {
    pub drawn: usize,
    pub culled: usize,
    pub micros: u64,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy)]
struct Frame<'a> {
    /// Drawing into a multithreaded context: vello filter layers must be rendered offscreen.
    mt: bool,
    doc: &'a Document,
    view: Affine,
    /// Visible region in document coordinates (for culling).
    visible: Rect,
    /// Size of one output pixel in document units.
    px: f64,
    opts: &'a RenderOptions,
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            texts: PtrMap::default(),
            clips: PtrMap::default(),
            ctx_st: None,
            threads: default_threads(),
            geom: PtrMap::default(),
            strokes: PtrMap::default(),
            cur: None,
            stamp: 0,
            alpha: 1.0,
            ctx: None,
            resources: Resources::new(),
            images: HashMap::new(),
            stats: FrameStats::default(),
            brushes: Default::default(),
            shadows: PtrMap::default(),
            live: live::LiveCache::default(),
            knockout: false,
            shape_of: 0,
            fx_arts: PtrMap::default(),
            nested: 0,
            backdrop: None,
            blends: PtrMap::default(),
            clip_paths: vec![],
        }
    }

    /// Render `doc` into a `width`×`height` image using `view` (document → pixel transform).
    pub fn render(&mut self, doc: &Document, width: u32, height: u32, view: Affine, opts: &RenderOptions) -> Rendered {
        let start = now();
        let prepared = proof::prepare(doc, opts);
        let doc: &Document = &prepared;
        let w = width.clamp(1, u16::MAX as u32) as u16;
        let h = height.clamp(1, u16::MAX as u32) as u16;
        // Raster filters (drop shadow, glows, blur) are rendered offscreen on this thread when the
        // main context is multithreaded (see `fx::with_filters`).
        let threads = self.threads;
        let slot = if threads == 0 { self.ctx_st.take() } else { self.ctx.take() };
        let mut ctx = match slot {
            Some(mut c) if c.width() == w && c.height() == h => {
                c.reset();
                c
            }
            _ => RenderContext::new_with(w, h, vello_cpu::RenderSettings { num_threads: threads, ..Default::default() }),
        };
        self.stats = FrameStats::default();
        let inv = view.inverse();
        let visible = inv.transform_rect_bbox(Rect::new(0.0, 0.0, w as f64, h as f64));
        let px = 1.0 / view.determinant().abs().sqrt().max(1e-12);
        let frame = Frame { mt: threads > 0, doc, view, visible, px, opts };

        if let Some(bg) = opts.background {
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_paint(peniko::Color::from_rgba8(bg[0], bg[1], bg[2], bg[3]));
            ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, w as f64, h as f64));
        }
        if opts.artboards && !opts.outline {
            ctx.set_transform(view);
            ctx.set_paint(peniko::Color::WHITE);
            for ab in &doc.artboards {
                ctx.fill_rect(&ab.rect);
            }
        }
        self.stamp += 1;
        (self.nested, self.backdrop) = (0, None);
        self.clip_paths.clear();
        if opts.trim && !doc.artboards.is_empty() {
            let mut clip = BezPath::new();
            for ab in &doc.artboards {
                clip.extend(ab.rect.path_elements(0.1));
            }
            let blends = self.children_blend(&doc.layers);
            let trim = Composite { clip: Some((&clip, FillRule::NonZero)), blends, ..Default::default() };
            self.group(&mut ctx, &frame, trim, &mut |r, c, fr| r.draw_page(c, fr));
        } else {
            self.draw_page(&mut ctx, &frame);
        }
        // Drop cache entries not seen for a few frames.
        let g = self.stamp;
        if self.geom.len() > 1024 {
            self.geom.retain(|_, e| g - e.stamp <= 3);
        }
        if self.strokes.len() > 1024 {
            self.strokes.retain(|_, e| g - e.stamp <= 3);
        }
        if self.shadows.len() > 256 {
            self.shadows.retain(|_, e| g - e.stamp <= 3);
        }
        ctx.flush();
        let mut pm = Pixmap::new(w, h);
        ctx.render(&mut pm, &mut self.resources);
        if threads == 0 {
            self.ctx_st = Some(ctx);
        } else {
            self.ctx = Some(ctx);
        }
        let mut pixels = pm.data_as_u8_slice().to_vec();
        proof::post(&mut pixels, opts);
        self.stats.micros = now().saturating_sub(start);
        Rendered { width: w as u32, height: h as u32, pixels }
    }

    /// The page's art: the layers (or the pattern being edited), inside the page group when the
    /// page has one.
    fn draw_page(&mut self, ctx: &mut RenderContext, f: &Frame) {
        if self.draw_pattern_edit(ctx, f) {
            return;
        }
        let doc = f.doc;
        // While editing an opacity mask its art is seen only through the mask.
        let mask_layer = doc.mask_edit.map(|m| m.layer);
        let layers: Vec<Arc<Node>> = doc.layers.iter().filter(|l| Some(l.id) != mask_layer).cloned().collect();
        // Page Isolated Blending / Page Knockout Group: the page is a group of its own.
        let page_group = !f.opts.outline && (doc.page_isolate || doc.page_knockout);
        let knockout = page_group && doc.page_knockout;
        let mut draw = |r: &mut Self, c: &mut RenderContext, fr: &Frame| {
            r.knockout = knockout;
            r.draw_children(c, fr, &layers, knockout);
            r.knockout = false;
        };
        if page_group {
            let blends = self.children_blend(&layers);
            self.group(ctx, f, Composite { isolated: doc.page_isolate, blends, ..Default::default() }, &mut draw);
        } else {
            draw(self, ctx, f);
        }
    }

    /// Render one artboard (or any document rect) at `scale` pixels per point, transparent or on white,
    /// as exported: template layers are left out.
    pub fn render_region(&mut self, doc: &Document, region: Rect, scale: f64, white: bool) -> Rendered {
        let w = (region.width() * scale).round().max(1.0) as u32;
        let h = (region.height() * scale).round().max(1.0) as u32;
        let view = Affine::scale(scale) * Affine::translate((-region.x0, -region.y0));
        let opts = RenderOptions { background: white.then_some([255, 255, 255, 255]), skip_templates: true, ..Default::default() };
        self.render(doc, w, h, view, &opts)
    }

    /// Render a single node (thumbnails, previews) fitted into `size`×`size` pixels.
    pub fn render_thumbnail(&mut self, doc: &Document, id: NodeId, size: u32) -> Option<Rendered> {
        self.render_node_thumbnail(doc, doc.node(id)?, size, None)
    }

    /// Render any node (also one outside the tree, e.g. opacity-mask art) fitted into
    /// `size`×`size` pixels, optionally over a solid premultiplied background.
    pub fn render_node_thumbnail(&mut self, doc: &Document, n: &Node, size: u32, background: Option<[u8; 4]>) -> Option<Rendered> {
        let b = brush_fx::cull_bounds(n)?;
        let s = (size as f64 - 2.0) / b.width().max(b.height()).max(1e-6);
        let view = Affine::translate((size as f64 / 2.0, size as f64 / 2.0)) * Affine::scale(s) * Affine::translate(-b.center().to_vec2());
        let w = size.clamp(1, u16::MAX as u32) as u16;
        let mut ctx = single_threaded_context(w, w);
        if let Some(bg) = background {
            ctx.set_paint(peniko::Color::from_rgba8(bg[0], bg[1], bg[2], bg[3]));
            ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, w as f64, w as f64));
        }
        let frame = Frame { mt: false, doc, view, visible: b.inflate(1.0, 1.0), px: 1.0 / s, opts: &RenderOptions::default() };
        (self.knockout, self.nested, self.backdrop) = (false, 0, None);
        self.clip_paths.clear();
        self.draw_node(&mut ctx, &frame, n, true);
        ctx.flush();
        let mut pm = Pixmap::new(w, w);
        ctx.render(&mut pm, &mut self.resources);
        Some(Rendered { width: size, height: size, pixels: pm.data_as_u8_slice().to_vec() })
    }

    /// Cached cull bounds of a node (containers union their cached children).
    fn bounds_of(&mut self, a: &Arc<Node>) -> Option<Rect> {
        let key = Arc::as_ptr(a) as usize;
        if let Some(e) = self.geom.get_mut(&key)
            && Arc::ptr_eq(&e.node, a)
        {
            e.stamp = self.stamp;
            return e.bounds;
        }
        let b = match &a.kind {
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } if !fx::has_fx(a) => {
                let mut acc: Option<Rect> = None;
                for c in children {
                    if c.visible {
                        acc = vectorcraft_geom::union_opt(acc, self.bounds_of(c));
                    }
                }
                acc
            }
            _ => brush_fx::cull_bounds(a),
        };
        self.geom.insert(key, GeomEntry { node: a.clone(), bounds: b, path: None, stamp: self.stamp });
        b
    }

    /// Cached region a clip group's clipping path `a` clips to ([`effects::clip_outline`]):
    /// outlining text and uniting shapes is too slow to repeat every frame.
    fn clip_of(&mut self, a: &Arc<Node>) -> Option<Arc<(BezPath, FillRule)>> {
        let key = Arc::as_ptr(a) as usize;
        if let Some((node, region)) = self.clips.get(&key)
            && Arc::ptr_eq(node, a)
        {
            return region.clone();
        }
        let region = effects::clip_outline(a).map(Arc::new);
        if self.clips.len() > 1024 {
            self.clips.clear();
        }
        self.clips.insert(key, (a.clone(), region.clone()));
        region
    }

    /// Cached BezPath of a path node.
    fn path_of(&mut self, a: &Arc<Node>) -> Option<Arc<BezPath>> {
        let key = Arc::as_ptr(a) as usize;
        if let Some(e) = self.geom.get(&key)
            && Arc::ptr_eq(&e.node, a)
            && let Some(p) = &e.path
        {
            return Some(p.clone());
        }
        let p = Arc::new(a.path_data()?.to_bezpath());
        let bounds = self.bounds_of(a);
        self.geom.insert(key, GeomEntry { node: a.clone(), bounds, path: Some(p.clone()), stamp: self.stamp });
        Some(p)
    }

    /// Whether `a` is left out of this frame: hidden, a skipped template, or culled.
    fn skipped(&mut self, f: &Frame, a: &Arc<Node>) -> bool {
        let skipped_template = f.opts.skip_templates && matches!(a.kind, NodeKind::Layer { template: true, .. });
        if !a.visible || skipped_template || f.opts.hidden.contains(&a.id) {
            return true;
        }
        match self.bounds_of(a) {
            Some(b) => {
                let pad = f.px * 2.0;
                // Level of detail: leaves smaller than a quarter pixel are invisible.
                let culled =
                    !rects_overlap(b.inflate(pad, pad), f.visible) || (!a.is_container() && b.width() < f.px * 0.25 && b.height() < f.px * 0.25);
                self.stats.culled += culled as usize;
                culled
            }
            None => !a.is_container(),
        }
    }

    fn draw_arc(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>) {
        if self.skipped(f, a) {
            return;
        }
        if let Some(m) = a.mask.as_deref()
            && !m.disabled
            && !f.opts.outline
            && !self.is_shape(a)
        {
            // A masked object is a non-isolated group of its own.
            let mask = Some(self.opacity_mask(f, m, ctx.width(), ctx.height()));
            let blends = a.blend != vectorcraft_color::BlendMode::Normal || (!a.isolate && self.blends_through(a));
            let bounds = if blends { self.bounds_of(a) } else { None };
            self.group(ctx, f, Composite { mask, blends, bounds, ..Default::default() }, &mut |r, c, fr| r.draw_arc_body(c, fr, a));
            return;
        }
        self.draw_arc_body(ctx, f, a);
    }

    /// Render an opacity mask's art offscreen and turn its luminance into a coverage mask.
    fn opacity_mask(&mut self, f: &Frame, m: &vectorcraft_doc::OpacityMask, w: u16, h: u16) -> vello_cpu::Mask {
        let mut mctx = single_threaded_context(w, h);
        // Mask art is a picture of its own: it takes no part in a knockout group around the object.
        let outer =
            (std::mem::take(&mut self.knockout), std::mem::take(&mut self.nested), self.backdrop.take(), std::mem::take(&mut self.clip_paths));
        self.draw_node(&mut mctx, &Frame { mt: false, ..*f }, &m.art, true);
        (self.knockout, self.nested, self.backdrop, self.clip_paths) = outer;
        mctx.flush();
        let mut pm = Pixmap::new(w, h);
        mctx.render(&mut pm, &mut self.resources);
        let data = pm.data().iter().map(|p| mask_value(p.r, p.g, p.b, p.a, m.clip, m.invert)).collect();
        vello_cpu::Mask::from_parts(data, w, h)
    }

    /// Everything [`Self::draw_arc`] does after culling.
    fn draw_arc_body(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>) {
        // Fast path for plain paths: cached geometry, opacity folded into the paint.
        // A blend mode on a single plain fill applies per draw (exact for one paint operation);
        // a blend layer would composite the whole viewport.
        let blended = a.blend != vectorcraft_color::BlendMode::Normal;
        if let NodeKind::Path { rule, guide: false, .. } = &a.kind
            && !f.opts.outline
            && (!blended || single_plain_fill(a))
            && !a.isolate
            && !fx::has_fx(a)
            && (self.opacity_of(a) >= 1.0 || painted_items(a) == 1)
            && let Some(bp) = self.path_of(a)
        {
            self.alpha = self.opacity_of(a).clamp(0.0, 1.0);
            self.cur = Some(a.clone());
            if blended {
                ctx.set_blend_mode(blend_mode(a.blend));
            }
            self.draw_shape(ctx, f, a, &bp, *rule);
            if blended {
                ctx.set_blend_mode(blend_mode(vectorcraft_color::BlendMode::Normal));
            }
            self.cur = None;
            self.alpha = 1.0;
            self.stats.drawn += 1;
            return;
        }
        if fx::has_object_fx(a) {
            return self.draw_object_fx(ctx, f, a, true);
        }
        if let NodeKind::Text(t) = &a.kind
            && self.opacity_of(a) >= 1.0
            && a.blend == vectorcraft_color::BlendMode::Normal
            && !fx::has_fx(a)
        {
            let g = self.text_geom_of(a, t);
            self.draw_text_geom(ctx, f, a, t, &g);
            self.stats.drawn += 1;
            return;
        }
        if vectorcraft_doc::live::is_live(a) || effects::has_pathfinder(a) {
            return self.draw_live(ctx, f, a);
        }
        self.draw_node(ctx, f, a, true);
    }

    fn draw_node(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, force: bool) {
        if !force && (!n.visible || f.opts.hidden.contains(&n.id)) {
            return;
        }
        if force {
            // Bounds were already checked by draw_arc (or the caller wants it drawn regardless).
        } else if let Some(b) = fx::cull_bounds(n) {
            let pad = f.px * 2.0;
            if !rects_overlap(b.inflate(pad, pad), f.visible) {
                self.stats.culled += n.count();
                return;
            }
        } else if !n.is_container() {
            return;
        }
        if fx::has_object_fx(n) {
            return self.draw_object_fx(ctx, f, &Arc::new(n.clone()), false);
        }
        let outline = f.opts.outline;
        let template_dim = matches!(n.kind, NodeKind::Layer { template: true, .. }) && f.opts.dim_templates;
        let opacity = if template_dim { self.opacity_of(n) * 0.5 } else { self.opacity_of(n) };
        // Whether this container's children knock each other out.
        let knockout = !outline && n.knocks_out(self.knockout);
        let enclosing = std::mem::replace(&mut self.knockout, knockout);
        if !outline && (opacity < 1.0 || n.blend != vectorcraft_color::BlendMode::Normal || n.isolate || knockout) {
            let blends = self.blends_through(n);
            let bounds = if blends { self.node_bounds(n) } else { None };
            let comp = Composite { blend: n.blend, opacity, isolated: n.isolate, blends, bounds, ..Default::default() };
            self.group(ctx, f, comp, &mut |r, c, fr| r.draw_content(c, fr, n, knockout));
        } else {
            self.draw_content(ctx, f, n, knockout);
        }
        self.knockout = enclosing;
        self.stats.drawn += 1;
    }

    /// What `n` draws inside its transparency group (`knockout`: its children knock each other out).
    fn draw_content(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, knockout: bool) {
        match &n.kind {
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
                self.draw_children(ctx, f, children, knockout)
            }
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } if f.opts.outline => {
                for c in children {
                    self.draw_node(ctx, f, c, false);
                }
            }
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                // Nothing to clip by hides the clipped art (as in the SVG and PDF output). A clip
                // group doesn't isolate: blending inside it reaches the art below.
                if let Some((clip, rest)) = children.split_first()
                    && let Some(region) = self.clip_of(clip)
                {
                    let blends = self.blends_through(n);
                    let bounds = if blends { Some(region.0.bounding_box()) } else { None };
                    let comp = Composite { clip: Some((&region.0, region.1)), blends, bounds, ..Default::default() };
                    self.group(ctx, f, comp, &mut |r, c, fr| r.draw_children(c, fr, rest, knockout));
                }
            }
            NodeKind::Path { path, rule, guide, .. } => {
                let bp = path.to_bezpath();
                if *guide {
                    self.hairline(ctx, f, &bp, [0x4a, 0xd8, 0xff, 255]);
                } else {
                    self.draw_shape(ctx, f, n, &bp, *rule);
                }
            }
            NodeKind::Compound { children, rule } => {
                let mut bp = BezPath::new();
                for c in children {
                    if let Some(p) = c.path_data() {
                        bp.extend(p.to_bezpath());
                    }
                }
                self.draw_shape(ctx, f, n, &bp, *rule);
            }
            NodeKind::Text(t) => self.draw_text(ctx, f, n, t),
            NodeKind::Image(im) => self.draw_image(ctx, f, im),
            NodeKind::SymbolInstance { symbol, xf } => {
                if let Some(sym) = f.doc.symbols.iter().find(|s| &s.name == symbol) {
                    let mut art = brush_fx::instance_art(&sym.art, n);
                    art.transform(*xf, false);
                    self.draw_node(ctx, f, &art, true);
                }
            }
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Mesh(_) | NodeKind::Repeat(_) => self.draw_live_node(ctx, f, n),
        }
    }

    /// The children of a group: as the elements of a knockout group when `knockout`.
    fn draw_children(&mut self, ctx: &mut RenderContext, f: &Frame, children: &[Arc<Node>], knockout: bool) {
        if knockout {
            self.draw_knockout(ctx, f, children);
        } else {
            for c in children {
                self.draw_arc(ctx, f, c);
            }
        }
    }

    /// Cull bounds of `n`: cached when it is a node of the tree being drawn.
    fn node_bounds(&mut self, n: &Node) -> Option<Rect> {
        match self.geom.get(&(n as *const Node as usize)) {
            // The entry keeps its node alive, so the address is that node's.
            Some(e) if std::ptr::eq(Arc::as_ptr(&e.node), n) => e.bounds,
            _ => fx::cull_bounds(n),
        }
    }

    /// The children of a knockout group ([`Node::knockout_elements`]). Each element first erases
    /// what the elements below it drew wherever it paints (its knockout shape: its coverage at full
    /// object opacity without its opacity mask, or as drawn when its opacity and mask define the
    /// knockout shape), then adds itself: it composites against the group's backdrop instead of
    /// over the elements below it (the art below the group when the group is non-isolated and
    /// drawn over a copy of it, see [`Self::group`]).
    fn draw_knockout(&mut self, ctx: &mut RenderContext, f: &Frame, children: &[Arc<Node>]) {
        let backdrop = self.backdrop.take();
        for c in Node::knockout_elements(children) {
            if self.skipped(f, c) {
                continue;
            }
            self.knockout_pass(ctx, f, c, Compose::DestOut, None);
            self.knockout_pass(ctx, f, c, Compose::Plus, backdrop.as_ref());
        }
        self.backdrop = backdrop;
    }

    /// Whether `n` is the knockout element being drawn as its shape (see [`Self::draw_knockout`]).
    fn is_shape(&self, n: &Node) -> bool {
        self.shape_of == n as *const Node as usize
    }

    /// The object opacity `n` is drawn with (full while it is drawn as its knockout shape).
    pub(crate) fn opacity_of(&self, n: &Node) -> f32 {
        if self.is_shape(n) { 1.0 } else { n.opacity }
    }

    fn hairline(&mut self, ctx: &mut RenderContext, f: &Frame, bp: &BezPath, rgba: [u8; 4]) {
        ctx.set_transform(Affine::IDENTITY);
        let mut screen = bp.clone();
        screen.apply_affine(f.view);
        ctx.set_stroke(kurbo::Stroke::new(1.0));
        ctx.set_paint(peniko::Color::from_rgba8(rgba[0], rgba[1], rgba[2], rgba[3]));
        ctx.stroke_path(&screen);
    }

    fn draw_shape(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, bp: &BezPath, rule: FillRule) {
        if fx::has_fx(n) {
            return self.draw_shape_fx(ctx, f, n, bp, rule);
        }
        if f.opts.outline {
            self.hairline(ctx, f, bp, [0, 0, 0, 255]);
            return;
        }
        // Bounds only matter to gradients and patterns (their geometry is relative to the object).
        let solid = |p: &vectorcraft_color::Paint| matches!(p, vectorcraft_color::Paint::Solid { .. } | vectorcraft_color::Paint::None);
        let needs_bounds = n.appearance.items.iter().any(|i| match i {
            AppearanceItem::Fill(fl) => !solid(&fl.paint),
            AppearanceItem::Stroke(st) => !solid(&st.paint),
        });
        let bounds = if needs_bounds { bp.bounding_box() } else { Rect::ZERO };
        for item in &n.appearance.items {
            match item {
                AppearanceItem::Fill(fl) => {
                    if !fl.visible || fl.paint.is_none() {
                        continue;
                    }
                    let layered = fl.opacity < 1.0 || fl.blend != vectorcraft_color::BlendMode::Normal;
                    if layered {
                        ctx.set_transform(Affine::IDENTITY);
                        ctx.push_layer(None, Some(blend_mode(fl.blend)), Some(fl.opacity), None, None);
                    }
                    ctx.set_transform(f.view);
                    if paint::set_paint(ctx, &fl.paint, bounds, f.doc) {
                        self.fold_alpha(ctx, &fl.paint);
                        ctx.set_fill_rule(fill_rule(rule));
                        ctx.fill_path(bp);
                    }
                    if layered {
                        ctx.pop_layer();
                    }
                }
                AppearanceItem::Stroke(st) => {
                    if !st.visible || st.paint.is_none() || st.width <= 0.0 {
                        continue;
                    }
                    if st.brush.is_some() && self.draw_brush(ctx, f, n, bp, st) {
                        continue;
                    }
                    self.draw_stroke(ctx, f, bp, rule, st, bounds);
                }
            }
        }
    }

    /// Multiply the folded object opacity into a solid paint.
    fn fold_alpha(&self, ctx: &mut RenderContext, p: &vectorcraft_color::Paint) {
        if self.alpha < 1.0
            && let vectorcraft_color::Paint::Solid { color, .. } = p
        {
            let [r, g, b, a] = color.to_rgba8(self.alpha);
            ctx.set_paint(peniko::Color::from_rgba8(r, g, b, a));
        }
    }

    fn draw_stroke(&mut self, ctx: &mut RenderContext, f: &Frame, bp: &BezPath, rule: FillRule, st: &StrokeLayer, bounds: Rect) {
        let pieces = effects::stroke::stroke_pieces(bp, st);
        // The line and its heads overlap: they take the opacity once, in one layer, instead of
        // each folding it into its paint.
        let folded = if pieces.heads.is_empty() { 1.0 } else { std::mem::replace(&mut self.alpha, 1.0) };
        let opacity = st.opacity * folded;
        let layered = opacity < 1.0 || st.blend != vectorcraft_color::BlendMode::Normal || st.align == StrokeAlign::Outside;
        if layered {
            ctx.set_transform(Affine::IDENTITY);
            ctx.push_layer(None, Some(blend_mode(st.blend)), Some(opacity), None, None);
        }
        ctx.set_transform(f.view);
        let closed = effects::stroke::is_closed(bp);
        // Keep hairlines visible when zoomed far out (at least ~1 device pixel).
        let width = effects::stroke::aligned_width(st, closed).max(f.px * 0.5);
        let inside = st.align == StrokeAlign::Inside && closed;
        if inside {
            ctx.set_fill_rule(fill_rule(rule));
            ctx.push_clip_layer(bp);
        }
        // One paint box for the line and the heads, so a gradient runs on into the heads.
        let paint_bounds = bounds.inflate(st.width / 2.0, st.width / 2.0);
        if paint::set_paint(ctx, &st.paint, paint_bounds, f.doc) {
            self.fold_alpha(ctx, &st.paint);
            ctx.set_fill_rule(peniko::Fill::NonZero);
            match self.cached_stroke(f, &pieces.line, st, width) {
                Some(o) => ctx.fill_path(&o),
                None => ctx.fill_path(&effects::stroke::line_outline(&pieces.line, st, width, f.px * 0.25)),
            }
            for head in &pieces.heads {
                ctx.fill_path(&head.outline);
            }
        }
        if inside {
            ctx.pop_layer();
        }
        if st.align == StrokeAlign::Outside && closed {
            // Punch out the interior.
            ctx.set_blend_mode(BlendMode::new(Mix::Normal, Compose::DestOut));
            ctx.set_paint(peniko::Color::BLACK);
            ctx.set_fill_rule(fill_rule(rule));
            ctx.fill_path(bp);
            ctx.set_blend_mode(BlendMode::default());
        }
        if layered {
            ctx.pop_layer();
        }
        if !pieces.heads.is_empty() {
            self.alpha = folded;
        }
    }

    /// The line part of a stroke of the fast-path node `self.cur` (see
    /// [`effects::stroke::line_outline`]), cached across frames. The tolerance (vello's 0.25
    /// device px) is bucketed to powers of two, never coarser than needed, so zooming reuses
    /// outlines until the level changes.
    fn cached_stroke(&mut self, f: &Frame, line: &BezPath, st: &StrokeLayer, width: f64) -> Option<Arc<BezPath>> {
        let node = self.cur.clone()?;
        let level = (f.px * 0.25).log2().floor() as i32;
        let key = (st as *const StrokeLayer as usize, width.to_bits(), level);
        if let Some(e) = self.strokes.get_mut(&key)
            && Arc::ptr_eq(&e.node, &node)
        {
            e.stamp = self.stamp;
            return Some(e.outline.clone());
        }
        let outline = Arc::new(effects::stroke::line_outline(line, st, width, 2f64.powi(level)));
        self.strokes.insert(key, StrokeEntry { node, outline: outline.clone(), stamp: self.stamp });
        Some(outline)
    }

    fn draw_text(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, t: &TextObject) {
        let g = text_geom(t);
        self.draw_text_geom(ctx, f, n, t, &g);
    }

    /// Cached glyph geometry for a text node (keyed by Arc identity like paths).
    fn text_geom_of(&mut self, a: &Arc<Node>, t: &TextObject) -> Arc<TextGeom> {
        let key = Arc::as_ptr(a) as usize;
        if let Some((node, g)) = self.texts.get(&key)
            && Arc::ptr_eq(node, a)
        {
            return g.clone();
        }
        let g = Arc::new(text_geom(t));
        if self.texts.len() > 4096 {
            self.texts.clear();
        }
        self.texts.insert(key, (a.clone(), g.clone()));
        g
    }

    fn draw_text_geom(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, t: &TextObject, g: &TextGeom) {
        let xf = f.view * t.xf;
        if f.opts.outline {
            let mut p = g.all.clone();
            p.apply_affine(xf);
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_stroke(kurbo::Stroke::new(1.0));
            ctx.set_paint(peniko::Color::BLACK);
            ctx.stroke_path(&p);
            return;
        }
        let tb = t.xf.transform_rect_bbox(g.bounds);
        // Overprinting characters multiply per draw, like overprinting fills and strokes
        // ([`proof`]); the object blend mode is Normal here or applied by an enclosing layer.
        let overprints = proof::overprints(f.opts);
        let overprint = |ctx: &mut RenderContext, on: bool| {
            if overprints {
                ctx.set_blend_mode(if on { blend_mode(vectorcraft_color::BlendMode::Multiply) } else { BlendMode::default() });
            }
        };
        // Object-level appearance fills/strokes apply on top of character fills (like Illustrator).
        for (i, run) in t.runs.iter().enumerate() {
            let Some(path) = g.runs.get(i) else { continue };
            if path.elements().is_empty() {
                continue;
            }
            ctx.set_transform(xf);
            ctx.set_fill_rule(peniko::Fill::NonZero);
            if paint::set_paint(ctx, &run.style.fill, g.bounds, f.doc) {
                self.fold_alpha(ctx, &run.style.fill);
                overprint(ctx, run.style.overprint_fill);
                ctx.fill_path(path);
            }
            if !run.style.stroke.is_none()
                && run.style.stroke_width > 0.0
                && paint::set_paint(ctx, &run.style.stroke, stroke_paint_bounds(g.bounds, run.style.stroke_width), f.doc)
            {
                overprint(ctx, run.style.overprint_stroke);
                ctx.set_stroke(kurbo::Stroke::new(run.style.stroke_width));
                ctx.stroke_path(path);
            }
        }
        overprint(ctx, false);
        if !n.appearance.items.is_empty() {
            let mut all = g.all.clone();
            all.apply_affine(t.xf);
            for item in n.appearance.items.iter().filter(|i| i.visible() && !i.paint().is_none()) {
                // Each item composites with its own opacity and blend mode.
                let layered = item.opacity() < 1.0 || item.blend() != vectorcraft_color::BlendMode::Normal;
                if layered {
                    ctx.set_transform(Affine::IDENTITY);
                    ctx.push_layer(None, Some(blend_mode(item.blend())), Some(item.opacity()), None, None);
                }
                ctx.set_transform(f.view);
                match item {
                    AppearanceItem::Fill(fl) if paint::set_paint(ctx, &fl.paint, tb, f.doc) => {
                        ctx.set_fill_rule(peniko::Fill::NonZero);
                        ctx.fill_path(&all);
                    }
                    AppearanceItem::Stroke(st) if st.width > 0.0 && paint::set_paint(ctx, &st.paint, st.paint_bounds(tb), f.doc) => {
                        ctx.set_stroke(kurbo::Stroke::new(st.width));
                        ctx.stroke_path(&all);
                    }
                    _ => {}
                }
                if layered {
                    ctx.pop_layer();
                }
            }
        }
    }

    fn draw_image(&mut self, ctx: &mut RenderContext, f: &Frame, im: &vectorcraft_doc::ImageObject) {
        let rect = Rect::new(0.0, 0.0, im.width as f64, im.height as f64);
        if f.opts.outline {
            let mut p = rect.to_path(0.1);
            p.apply_affine(im.xf);
            self.hairline(ctx, f, &p, [0, 0, 0, 255]);
            return;
        }
        let pm = match self.images.get(&im.key) {
            Some(p) => p.clone(),
            None => {
                let Some(blob) = f.doc.images.get(&im.key) else { return };
                let Some(pm) = paint::decode_pixmap(&blob.bytes) else { return };
                let pm = Arc::new(pm);
                self.images.insert(im.key.clone(), pm.clone());
                pm
            }
        };
        let sx = im.width as f64 / pm.width().max(1) as f64;
        let sy = im.height as f64 / pm.height().max(1) as f64;
        ctx.set_transform(f.view * im.xf);
        ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(pm), sampler: peniko::ImageSampler::default() });
        ctx.set_paint_transform(Affine::scale_non_uniform(sx, sy));
        ctx.fill_rect(&rect);
        ctx.reset_paint_transform();
    }
}

/// Opacity-mask coverage of one premultiplied pixel: luminance, with the area outside the mask
/// art black (clip) or white (no clip), optionally inverted.
fn mask_value(r: u8, g: u8, b: u8, a: u8, clip: bool, invert: bool) -> u8 {
    let [kr, kg, kb] = vectorcraft_color::blend::MASK_LUM;
    let mut l = (kr * r as f32 + kg * g as f32 + kb * b as f32) / 255.0;
    if !clip {
        l += 1.0 - a as f32 / 255.0;
    }
    if invert {
        l = 1.0 - l;
    }
    (l.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// A render context on the calling thread. vello_cpu's `RenderContext::new` defaults to a
/// multithreaded dispatcher, which panics on filter effects (glows, shadows, blur).
pub(crate) fn single_threaded_context(w: u16, h: u16) -> RenderContext {
    RenderContext::new_with(w, h, vello_cpu::RenderSettings { num_threads: 0, ..Default::default() })
}

/// Exactly one visible painted item, a fill with its own Normal blend and full opacity (one
/// paint operation, so an object blend mode can be applied per draw).
fn single_plain_fill(n: &Node) -> bool {
    let mut painted = n.appearance.items.iter().filter(|i| match i {
        AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
        AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0,
    });
    matches!((painted.next(), painted.next()), (Some(AppearanceItem::Fill(f)), None) if f.blend == vectorcraft_color::BlendMode::Normal && f.opacity >= 1.0 && !matches!(f.paint, vectorcraft_color::Paint::Pattern { .. }))
}

/// Number of visible painted fill/stroke items (opacity folding is exact only for one).
fn painted_items(n: &Node) -> usize {
    n.appearance
        .items
        .iter()
        .filter(|i| match i {
            AppearanceItem::Fill(f) => f.visible && !f.paint.is_none() && matches!(f.paint, vectorcraft_color::Paint::Solid { .. }),
            AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0 && matches!(s.paint, vectorcraft_color::Paint::Solid { .. }) && s.dash.is_none(),
        })
        .count()
        .max(if n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Fill(f) if f.visible && !matches!(f.paint, vectorcraft_color::Paint::Solid { .. } | vectorcraft_color::Paint::None)) || matches!(i, AppearanceItem::Stroke(s) if s.visible && !matches!(s.paint, vectorcraft_color::Paint::Solid { .. } | vectorcraft_color::Paint::None))) { 2 } else { 0 })
}

/// Preferred rasterizer thread count set by the app (Preferences → Performance); negative = automatic.
static THREADS_OVERRIDE: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Override the worker-thread count used by renderers created from now on (`None` = automatic).
pub fn set_default_threads(n: Option<u16>) {
    THREADS_OVERRIDE.store(n.map_or(-1, i32::from), std::sync::atomic::Ordering::Relaxed);
}

/// Rasterizer worker threads: 0 on wasm; otherwise up to 4 (vello's sweet spot), leaving a core free.
pub fn default_threads() -> u16 {
    #[cfg(target_arch = "wasm32")]
    {
        0
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let o = THREADS_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
        if o >= 0 {
            return o.min(64) as u16;
        }
        if std::env::var_os("VECTORCRAFT_RENDER_THREADS").is_some() {
            return std::env::var("VECTORCRAFT_RENDER_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
        }
        std::thread::available_parallelism().map(|n| (n.get().saturating_sub(1)).min(4) as u16).unwrap_or(0)
    }
}

/// Glyph outlines grouped by run, plus the whole text as one path.
struct TextGeom {
    runs: Vec<BezPath>,
    all: BezPath,
    bounds: Rect,
}

fn text_geom(t: &TextObject) -> TextGeom {
    let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    let mut runs = vec![BezPath::new(); t.runs.len()];
    let mut all = BezPath::new();
    for g in &layout.glyphs {
        if let Some(r) = runs.get_mut(g.run) {
            r.extend(g.outline.iter());
        }
        all.extend(g.outline.iter());
    }
    TextGeom { runs, all, bounds: layout.bounds }
}

fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

fn fill_rule(r: FillRule) -> peniko::Fill {
    match r {
        FillRule::NonZero => peniko::Fill::NonZero,
        FillRule::EvenOdd => peniko::Fill::EvenOdd,
    }
}

pub(crate) fn blend_mode(b: vectorcraft_color::BlendMode) -> BlendMode {
    use vectorcraft_color::BlendMode as B;
    let mix = match b {
        B::Normal => Mix::Normal,
        B::Darken => Mix::Darken,
        B::Multiply => Mix::Multiply,
        B::ColorBurn => Mix::ColorBurn,
        B::Lighten => Mix::Lighten,
        B::Screen => Mix::Screen,
        B::ColorDodge => Mix::ColorDodge,
        B::Overlay => Mix::Overlay,
        B::SoftLight => Mix::SoftLight,
        B::HardLight => Mix::HardLight,
        B::Difference => Mix::Difference,
        B::Exclusion => Mix::Exclusion,
        B::Hue => Mix::Hue,
        B::Saturation => Mix::Saturation,
        B::Color => Mix::Color,
        B::Luminosity => Mix::Luminosity,
    };
    BlendMode::new(mix, Compose::SrcOver)
}

#[cfg(not(target_arch = "wasm32"))]
fn now() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_micros() as u64).unwrap_or(0)
}
#[cfg(target_arch = "wasm32")]
fn now() -> u64 {
    0
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_blend;
#[cfg(test)]
mod tests_clip;
#[cfg(test)]
mod tests_isolation;
#[cfg(test)]
mod tests_knockout;
#[cfg(test)]
mod tests_objectfx;
