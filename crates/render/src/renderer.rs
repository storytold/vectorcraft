use super::blobs::{blob_key, blob_pixmap, image_mip, lookup, mask_value, mip_level, plain_image};
use super::clock::now;
use super::frame::{
    ClipEntry, ClipPaintEntry, Frame, FrameStats, GeomEntry, MAX_INLINE_DEPTH, MISSING_INLINE_LOGGED, PlacedEntry, PlacedKey, PtrMap, SliceEntry,
    Slices, StrokeEntry,
};
use super::geometry::{MAX_GUIDE_DASHES, polygon_length, rects_overlap, region_pixels};
use super::node_checks::{
    depends_on_document, drawable, holds_instance, holds_plugin_effect, holds_type, owns_stroke, painted_items, place, single_plain_fill, stain_bits,
};
use super::options::{AntiAlias, ISOLATION_DIM, MAX_CONTEXT_SIDE, RenderOptions, Rendered, downsample, supersample_factor};
use super::paint::{SUBSTITUTED, blend_mode, fill_rule};
use super::text_geom::{TextGeom, text_geom, text_geom_snapped};
use super::threads::{default_threads, single_threaded_context};
use super::*;

mod images;
mod nodes;
mod paint;
mod text;

impl Renderer {
    pub fn new() -> Self {
        Self {
            texts: FrameCache::new(lru::TEXT_BUDGET, 1024),
            text_fonts: 0,
            clips: FrameCache::new(lru::CLIP_BUDGET, 256),
            threads: default_threads(),
            geom: PtrMap::default(),
            symbols: vec![],
            plugin_revision: 0,
            strokes: PtrMap::default(),
            char_strokes: FrameCache::new(lru::CHAR_STROKE_BUDGET, 0),
            cur: None,
            made_again: false,
            fx_owner: None,
            live_root: None,
            fx_geom: PtrMap::default(),
            stamp: 0,
            alpha: 1.0,
            contexts: Vec::new(),
            resources: Resources::new(),
            stats: FrameStats::default(),
            brushes: Default::default(),
            shadows: PtrMap::default(),
            pixel_fx: PtrMap::default(),
            doc_inputs: None,
            transparency: None,
            live: live::LiveCache::default(),
            knockout: false,
            shape_of: 0,
            fx_arts: FrameCache::new(lru::FX_ART_BUDGET, 256),
            placed_arts: FrameCache::new(lru::PLACED_ART_BUDGET, 256),
            placed_frame: None,
            nested: 0,
            backdrop: None,
            blends: PtrMap::default(),
            clip_paths: vec![],
            picture: None,
            offscreen_live: 0,
            offscreen_budget: 0,
            symbol_nodes: 0,
            twice_drawn: 0,
            clip_paints: FrameCache::new(lru::CLIP_BUDGET, 256),
            stroke_slices: PtrMap::default(),
            adjusts: HashMap::new(),
            dim_images: None,
            inline_depth: 0,
            raster: vello_cpu::RasterizerSettings::default(),
            isolation: None,
            isolation_settled: false,
            proof_art: None,
            hairline: 1.0,
        }
    }

    /// Render `doc` into a `width`×`height` image using `view` (document → pixel transform).
    pub fn render(&mut self, doc: &Document, width: u32, height: u32, view: Affine, opts: &RenderOptions) -> Rendered {
        self.render_as(doc, width, height, view, opts, false)
    }

    /// [`Self::render`], or with `inks` the ink amounts in the working CMYK space instead of
    /// screen colours (4 bytes a pixel, see [`Self::render_region_inks`]).
    fn render_as(&mut self, doc: &Document, width: u32, height: u32, view: Affine, opts: &RenderOptions, inks: bool) -> Rendered {
        let start = now();
        if opts.supersample && opts.anti_alias == AntiAlias::Art && opts.mask_view.is_none() {
            let (w, h) = (width.clamp(1, MAX_CONTEXT_SIDE), height.clamp(1, MAX_CONTEXT_SIDE));
            if let Some(k) = supersample_factor(w, h) {
                let hard = RenderOptions { anti_alias: AntiAlias::None, supersample: false, ..opts.clone() };
                let hairline = std::mem::replace(&mut self.hairline, f64::from(k));
                let big = self.render_as(doc, w * k, h * k, Affine::scale(f64::from(k)) * view, &hard, inks);
                self.hairline = hairline;
                let pixels = downsample(&big.pixels, big.width as usize, big.height as usize, k as usize);
                self.stats.micros = now().saturating_sub(start);
                return Rendered { width: w, height: h, pixels };
            }
        }
        self.check_plugins();
        // The document's own symbols: proofing rewrites the colors and blend modes of their art.
        self.check_symbols(doc);
        let prepared = proof::prepare(doc, opts, &mut self.proof_art);
        let doc: &Document = &prepared;
        let w = width.clamp(1, MAX_CONTEXT_SIDE) as u16;
        let h = height.clamp(1, MAX_CONTEXT_SIDE) as u16;
        // Raster filters (drop shadow, glows, blur) are rendered offscreen on this thread when the
        // main context is multithreaded (see `fx::with_filters`).
        let threads = self.threads;
        self.stats = FrameStats::default();
        let inv = view.inverse();
        let visible = inv.transform_rect_bbox(Rect::new(0.0, 0.0, w as f64, h as f64));
        let px = 1.0 / view.determinant().abs().sqrt().max(1e-12);
        let screen = if opts.rich_black { Ink::RichBlack } else { Ink::Display };
        let symbols = doc.symbol_boxes().quick();
        let frame = Frame { mt: threads > 0, doc, symbols: &symbols, view, visible, px, opts, ink: screen };
        self.begin_frame();
        // View Opacity Mask: the mask's coverage in greyscale instead of the artwork.
        if let Some(m) = opts.mask_view.and_then(|id| doc.node(id)).and_then(|n| n.extras().mask.as_deref()) {
            let pixels = self.mask_values(&frame, m, w, h).into_iter().flat_map(|v| [v, v, v, 255]).collect();
            self.end_frame();
            self.stats.micros = now().saturating_sub(start);
            return Rendered { width: w as u32, height: h as u32, pixels };
        }
        let mut pixels = if inks || self.blends_in_cmyk(doc, opts) {
            // Blending in CMYK: one frame per ink plane, then their inks shown on screen.
            let cmy = self.draw_frame(&Frame { ink: Ink::Cmy, ..frame }, w, h);
            let stats = self.stats;
            let k = self.draw_frame(&Frame { ink: Ink::K, ..frame }, w, h);
            self.stats = stats;
            if inks {
                ink::amounts(&cmy, &k)
            } else {
                let (pixels, composed) = ink::compose(&cmy, &k, opts, threads);
                self.stats.ink_threads = composed;
                pixels
            }
        } else {
            self.draw_frame(&frame, w, h)
        };
        self.end_frame();
        if !inks {
            proof::post(&mut pixels, opts);
        }
        self.stats.micros = now().saturating_sub(start);
        Rendered { width: w as u32, height: h as u32, pixels }
    }

    /// Start a frame: cache entries used from now on are marked with its stamp.
    fn begin_frame(&mut self) {
        self.stamp += 1;
        self.inline_depth = 0;
        // Nothing is drawn offscreen yet (even if an earlier frame stopped part way).
        (self.offscreen_live, self.offscreen_budget) = (0, 0);
        self.symbol_nodes = 0;
        self.twice_drawn = 0;
        self.placed_frame = None;
    }

    /// End a frame: drop cache entries not used for a few frames (every render, thumbnails too,
    /// so the nodes they hold go with them).
    fn end_frame(&mut self) {
        let g = self.stamp;
        if self.geom.len() > 1024 {
            self.geom.retain(|_, e| g - e.stamp <= 3);
        }
        if self.strokes.len() > 1024 {
            self.strokes.retain(|_, e| g - e.stamp <= 3);
        }
        if self.stroke_slices.len() > 256 {
            self.stroke_slices.retain(|_, e| g - e.stamp <= 3);
        }
        if self.shadows.len() > 256 {
            self.shadows.retain(|_, e| g - e.stamp <= 3);
        }
        if self.fx_geom.len() > 256 {
            self.fx_geom.retain(|_, e| g - e.stamp <= 3);
        }
        // Filtered rasters are large: only those of the last frame stay.
        self.pixel_fx.retain(|_, e| e.stamp == g);
        self.texts.sweep(g);
        self.char_strokes.sweep(g);
        self.clips.sweep(g);
        self.clip_paints.sweep(g);
        self.fx_arts.sweep(g);
        self.placed_arts.sweep(g);
        self.brushes.entries.sweep(g);
        self.live.sweep(g);
    }

    /// Draw frame `f` (`w`×`h` pixels): the background, the artboards and the page's art.
    fn draw_frame(&mut self, f: &Frame, w: u16, h: u16) -> Vec<u8> {
        let (threads, opts) = (self.threads, f.opts);
        let kept = self.contexts.iter().position(|(t, c)| *t == threads && c.width() == w && c.height() == h);
        let mut ctx = match kept.map(|i| self.contexts.remove(i)) {
            Some((_, mut c)) => {
                c.reset();
                c
            }
            None => RenderContext::new_with(w, h, vello_cpu::RenderSettings { num_threads: threads, ..Default::default() }),
        };
        // `reset` keeps the threshold of the previous render: set it every time.
        ctx.set_aliasing_threshold(opts.anti_alias.threshold());
        let render_mode = if opts.precise { vello_cpu::RenderMode::OptimizeQuality } else { vello_cpu::RenderMode::OptimizeSpeed };
        self.raster = vello_cpu::RasterizerSettings { render_mode, ..Default::default() };
        if let Some(bg) = opts.background {
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_paint(f.ink.fixed(bg));
            ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, w as f64, h as f64));
        }
        if opts.artboards && !opts.outline {
            // Paper: white on screen, no ink on the ink planes.
            ctx.set_transform(f.view);
            ctx.set_paint(peniko::Color::WHITE);
            for ab in &f.doc.artboards {
                ctx.fill_rect(&ab.rect);
            }
        }
        (self.nested, self.backdrop, self.picture) = (0, None, None);
        self.clip_paths.clear();
        if opts.trim && !f.doc.artboards.is_empty() {
            let mut clip = BezPath::new();
            for ab in &f.doc.artboards {
                clip.extend(ab.rect.path_elements(0.1));
            }
            let blends = self.children_blend(&f.doc.layers);
            let trim = Composite { clip: Some((&clip, FillRule::NonZero)), blends, ..Default::default() };
            self.group(&mut ctx, f, trim, &mut |r, c, fr| r.draw_page(c, fr));
        } else {
            self.draw_page(&mut ctx, f);
        }
        let pm = self.finish(&mut ctx, self.raster);
        self.contexts.push((threads, ctx));
        if self.contexts.len() > 2 {
            self.contexts.remove(0);
        }
        pm.data_as_u8_slice().to_vec()
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
    /// as exported: template layers and guides are left out.
    pub fn render_region(&mut self, doc: &Document, region: Rect, scale: f64, white: bool) -> Rendered {
        let opts = RenderOptions { background: white.then_some([255, 255, 255, 255]), skip_templates: true, ..Default::default() };
        self.render_region_with(doc, region, scale, &opts)
    }

    /// Render a document rect at `scale` pixels per point with `opts`.
    pub fn render_region_with(&mut self, doc: &Document, region: Rect, scale: f64, opts: &RenderOptions) -> Rendered {
        let (w, h) = region_pixels(region, scale);
        let view = Affine::scale(scale) * Affine::translate((-region.x0, -region.y0));
        self.render(doc, w, h, view, opts)
    }

    /// `region` of `doc` as ink amounts in the working CMYK space, 4 bytes a pixel (0 = no ink):
    /// drawn as the two ink planes of [`ink`], so CMYK colours keep their own inks (black type
    /// stays black ink alone) and RGB ones are separated with the colour settings. Where nothing
    /// is drawn (no background) there is no ink. CMYK exports draw this.
    pub fn render_region_inks(&mut self, doc: &Document, region: Rect, scale: f64, opts: &RenderOptions) -> Vec<u8> {
        let (w, h) = region_pixels(region, scale);
        let view = Affine::scale(scale) * Affine::translate((-region.x0, -region.y0));
        self.render_as(doc, w, h, view, opts, true).pixels
    }

    /// Render a single node (thumbnails, previews) fitted into `size`×`size` pixels.
    pub fn render_thumbnail(&mut self, doc: &Document, id: NodeId, size: u32) -> Option<Rendered> {
        self.render_node_thumbnail(doc, doc.node(id)?, size, None)
    }

    /// Render node `n` of `doc` (its resources) into `w`×`h` transparent pixels through `view`.
    pub fn render_node(&mut self, doc: &Document, n: &Arc<Node>, w: u16, h: u16, view: Affine) -> Option<Rendered> {
        self.check_plugins();
        self.check_symbols(doc);
        let inv = view.inverse();
        let visible = inv.transform_rect_bbox(Rect::new(0.0, 0.0, w as f64, h as f64));
        let px = 1.0 / view.determinant().abs().sqrt().max(1e-12);
        let mut ctx = single_threaded_context(w, h);
        let opts = RenderOptions::default();
        let symbols = doc.symbol_boxes().quick();
        let frame = Frame { mt: false, doc, symbols: &symbols, view, visible, px, opts: &opts, ink: Ink::Display };
        self.begin_frame();
        (self.knockout, self.nested, self.backdrop, self.picture) = (false, 0, None, None);
        self.clip_paths.clear();
        self.draw_arc(&mut ctx, &frame, n);
        let pm = self.finish(&mut ctx, vello_cpu::RasterizerSettings::default());
        self.end_frame();
        Some(Rendered { width: w as u32, height: h as u32, pixels: pm.data_as_u8_slice().to_vec() })
    }

    /// Render any node (also one outside the tree, e.g. opacity-mask art) fitted into
    /// `size`×`size` pixels, optionally over a solid premultiplied background. A symbol instance
    /// is fitted by its symbol's art in `doc`.
    pub fn render_node_thumbnail(&mut self, doc: &Document, n: &Node, size: u32, background: Option<[u8; 4]>) -> Option<Rendered> {
        self.render_node_thumbnail_with(doc, n, size, background, &RenderOptions::default())
    }

    /// [`Self::render_thumbnail`] with `opts` (e.g. [`RenderOptions::image_mips`]).
    pub fn render_thumbnail_with(&mut self, doc: &Document, id: NodeId, size: u32, opts: &RenderOptions) -> Option<Rendered> {
        self.render_node_thumbnail_with(doc, doc.node(id)?, size, None, opts)
    }

    /// [`Self::render_node_thumbnail`] with `opts` (e.g. [`RenderOptions::image_mips`]).
    pub fn render_node_thumbnail_with(
        &mut self,
        doc: &Document,
        n: &Node,
        size: u32,
        background: Option<[u8; 4]>,
        opts: &RenderOptions,
    ) -> Option<Rendered> {
        self.check_plugins();
        self.check_symbols(doc);
        let b = brush_fx::cull_bounds(&doc.symbol_boxes(), n)?;
        let s = (size as f64 - 2.0) / b.width().max(b.height()).max(1e-6);
        let view = Affine::translate((size as f64 / 2.0, size as f64 / 2.0)) * Affine::scale(s) * Affine::translate(-b.center().to_vec2());
        let w = size.clamp(1, MAX_CONTEXT_SIDE) as u16;
        let mut ctx = single_threaded_context(w, w);
        if let Some(bg) = background {
            ctx.set_paint(peniko::Color::from_rgba8(bg[0], bg[1], bg[2], bg[3]));
            ctx.fill_rect(&kurbo::Rect::new(0.0, 0.0, w as f64, w as f64));
        }
        let symbols = doc.symbol_boxes().quick();
        let frame = Frame { mt: false, doc, symbols: &symbols, view, visible: b.inflate(1.0, 1.0), px: 1.0 / s, opts, ink: Ink::Display };
        self.begin_frame();
        (self.knockout, self.nested, self.backdrop, self.picture) = (false, 0, None, None);
        self.clip_paths.clear();
        self.draw_node(&mut ctx, &frame, n, true);
        let pm = self.finish(&mut ctx, vello_cpu::RasterizerSettings::default());
        self.end_frame();
        Some(Rendered { width: size, height: size, pixels: pm.data_as_u8_slice().to_vec() })
    }
}
