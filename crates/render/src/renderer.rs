use super::blobs::{blob_key, blob_pixmap, image_mip, lookup, mask_value, mip_level, plain_image, shape_masked_image, white_shape_mask};
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
use super::{
    Affine, AppearanceItem, Arc, BezPath, BlendMode, Compose, Composite, Document, FillRule, FrameCache, HashMap, Ink, Mix, Node, NodeId, NodeKind,
    Pixmap, Rect, RenderContext, Renderer, Resources, Shape, StrokeAlign, StrokeLayer, SymbolBoxes, TextObject, TraceView, brush_fx, effects, fx,
    group, ink, kurbo, live, lru, peniko, proof, vello_cpu,
};

#[cfg(test)]
pub(crate) use paint::{SLICES_MADE, STROKES_OUTLINED, anchors_moved, offset_between};

mod images;
mod nodes;
mod paint;
mod text;

#[cfg(test)]
pub(crate) use paint::{CROP_STROKES, crop_line};
#[cfg(test)]
pub(crate) use text::{edge_coverage, edge_grid, type_reach};

impl Renderer {
    pub fn new() -> Self {
        // Envelopes map type from its glyphs here too, without an engine session.
        vectorcraft_text::register_envelope_ink();
        Self {
            texts: FrameCache::new(lru::TEXT_BUDGET, 1024),
            text_ids: PtrMap::default(),
            text_fonts: 0,
            clips: FrameCache::new(lru::CLIP_BUDGET, 256),
            threads: default_threads(),
            geom: PtrMap::default(),
            kept: PtrMap::default(),
            symbols: vec![],
            plugin_revision: 0,
            brush_library: None,
            painting_plugins: false,
            paint_names: 0,
            strokes: PtrMap::default(),
            moved_strokes: PtrMap::default(),
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
            type_edges: FrameCache::new(lru::TYPE_EDGE_BUDGET, 0),
            doc_inputs: None,
            transparency: None,
            live: live::LiveCache::default(),
            knockout: false,
            shape_of: 0,
            fx_arts: FrameCache::new(lru::FX_ART_BUDGET, 256),
            measured: PtrMap::default(),
            measured_bytes: 0,
            measured_budget: lru::MEASURED_BUDGET,
            pass_bounds: PtrMap::default(),
            three_d_bounds: PtrMap::default(),
            three_d_kept: 0,
            #[cfg(test)]
            three_d_sweeps: 0,
            placed_arts: FrameCache::new(lru::PLACED_ART_BUDGET, 256),
            placed_frame: None,
            nested: 0,
            backdrop: None,
            blends: PtrMap::default(),
            clip_paths: vec![],
            picture: None,
            image_left_out: None,
            offscreen_live: 0,
            offscreen_budget: 0,
            symbol_nodes: 0,
            twice_drawn: 0,
            clip_paints: FrameCache::new(lru::CLIP_BUDGET, 256),
            stroke_slices: PtrMap::default(),
            moved_slices: PtrMap::default(),
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

    /// Render `doc` into a `width`×`height` image using `view` (document → pixel transform). Type
    /// set in one of the document's composite fonts draws in the document's definition, also when
    /// no session has registered it (a document that is not in front, or read from a file).
    pub fn render(&mut self, doc: &Document, width: u32, height: u32, view: Affine, opts: &RenderOptions) -> Rendered {
        let _lent = vectorcraft_text::FontDb::global().lend_composite_fonts(&doc.composite_fonts);
        let _frame = vectorcraft_text::KeptLayoutsFrame::begin();
        self.render_as(doc, width, height, view, opts, false, 1)
    }

    /// [`Self::render`], or with `inks` the ink amounts in the working CMYK space instead of
    /// screen colours (4 bytes a pixel, see [`Self::render_region_inks`]). `cell` is the side of a
    /// pixel of the output in pixels of this render: the supersampling factor while an export
    /// draws its art at several times its size, else 1 ([`Frame::output`]).
    #[allow(clippy::too_many_arguments)]
    fn render_as(&mut self, doc: &Document, width: u32, height: u32, view: Affine, opts: &RenderOptions, inks: bool, cell: u32) -> Rendered {
        let start = now();
        if opts.supersample && opts.anti_alias == AntiAlias::Art && opts.mask_view.is_none() {
            let (w, h) = (width.clamp(1, MAX_CONTEXT_SIDE), height.clamp(1, MAX_CONTEXT_SIDE));
            if let Some(k) = supersample_factor(w, h) {
                let hard = RenderOptions { anti_alias: AntiAlias::None, supersample: false, ..opts.clone() };
                let hairline = std::mem::replace(&mut self.hairline, f64::from(k));
                let big = self.render_as(doc, w * k, h * k, Affine::scale(f64::from(k)) * view, &hard, inks, k);
                self.hairline = hairline;
                let pixels = downsample(&big.pixels, big.width as usize, big.height as usize, k as usize);
                self.stats.micros = now().saturating_sub(start);
                return Rendered { width: w, height: h, pixels };
            }
        }
        self.check_plugins(doc);
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
        let output = Affine::scale(1.0 / f64::from(cell.max(1))) * view;
        let plates = proof::plate_lut(opts);
        let frame = Frame { mt: threads > 0, doc, symbols: &symbols, view, visible, px, opts, ink: screen, output, plates: plates.as_deref() };
        self.begin_frame();
        // View Opacity Mask: the mask's coverage in greyscale instead of the artwork.
        if let Some(m) = opts.mask_view.and_then(|id| doc.node(id)).and_then(|n| n.extras().mask.as_deref()) {
            let pixels = self
                .mask_values(&frame, m, w, h)
                .unwrap_or_else(|| vec![0; w as usize * h as usize])
                .into_iter()
                .flat_map(|v| [v, v, v, 255])
                .collect();
            self.end_frame();
            self.stats.symbol_art = symbols.art_measured();
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
        self.stats.symbol_art = symbols.art_measured();
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
        // Swept however few the entries: each keeps its node alive, and an object dragged or
        // edited makes a new node (with its layer) in every frame. Kept until there were 1,024 of
        // them, a drag left up to that many old copies of the object alive, each with all its
        // members (and, for a compound path, its assembled path).
        self.geom.retain(|_, e| g - e.stamp <= 3);
        // A kept path stays while its object is in `geom`, which keeps the objects still drawn.
        let geom = &self.geom;
        self.kept.retain(|_, e| g - e.stamp <= 3 || geom.get(&(Arc::as_ptr(&e.node) as usize)).is_some_and(|k| Arc::ptr_eq(&k.node, &e.node)));
        if self.strokes.len() > 1024 {
            self.strokes.retain(|_, e| g - e.stamp <= 3);
            let strokes = &self.strokes;
            self.moved_strokes.retain(|_, k| strokes.contains_key(k));
        }
        if self.stroke_slices.len() > 256 {
            self.stroke_slices.retain(|_, e| g - e.stamp <= 3);
            let slices = &self.stroke_slices;
            self.moved_slices.retain(|_, k| slices.contains_key(k));
        }
        if self.shadows.len() > 256 {
            self.shadows.retain(|_, e| g - e.stamp <= 3);
        }
        if self.fx_geom.len() > 256 {
            self.fx_geom.retain(|_, e| g - e.stamp <= 3);
        }
        // Filtered rasters are large: only those of the last frame stay.
        self.pixel_fx.retain(|_, e| e.stamp == g);
        self.type_edges.sweep(g);
        self.texts.sweep(g);
        self.text_ids.retain(|_, key| self.texts.contains_key(key));
        self.char_strokes.sweep(g);
        self.clips.sweep(g);
        self.clip_paints.sweep(g);
        self.fx_arts.sweep(g);
        // Only the 3D art still kept (in `fx_arts`) keeps its bounds.
        self.let_go_of_measured();
        self.placed_arts.sweep(g);
        self.brushes.entries.sweep(g);
        self.live.sweep(g);
    }

    /// Draw frame `f` (`w`×`h` pixels): the artboards and the page's art, then the background
    /// under them. The art blends only with itself, as it does on the canvas, where the paper is
    /// drawn apart from it: a Screen glow on the bare page stays visible on a white background (#6125).
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
        self.raster = raster_settings(opts);
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
        let mut pixels = pm.data_as_u8_slice().to_vec();
        if let Some(bg) = opts.background {
            over_background(&mut pixels, f.ink.fixed8(bg));
        }
        pixels
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
        let _lent = vectorcraft_text::FontDb::global().lend_composite_fonts(&doc.composite_fonts);
        let _frame = vectorcraft_text::KeptLayoutsFrame::begin();
        self.render_as(doc, w, h, view, opts, true, 1).pixels
    }

    /// Render a single node (thumbnails, previews) fitted into `size`×`size` pixels.
    pub fn render_thumbnail(&mut self, doc: &Document, id: NodeId, size: u32) -> Option<Rendered> {
        self.render_node_thumbnail(doc, doc.node(id)?, size, None)
    }

    /// Render node `n` of `doc` (its resources) into `w`×`h` transparent pixels through `view`.
    pub fn render_node(&mut self, doc: &Document, n: &Arc<Node>, w: u16, h: u16, view: Affine) -> Option<Rendered> {
        self.render_node_with(doc, n, w, h, view, &RenderOptions::default())
    }

    /// [`Self::render_node`] with `opts` (e.g. [`RenderOptions::mark_undecoded_images`]).
    pub fn render_node_with(&mut self, doc: &Document, n: &Arc<Node>, w: u16, h: u16, view: Affine, opts: &RenderOptions) -> Option<Rendered> {
        let _lent = vectorcraft_text::FontDb::global().lend_composite_fonts(&doc.composite_fonts);
        self.check_plugins(doc);
        self.check_symbols(doc);
        let visible = view.inverse().transform_rect_bbox(Rect::new(0.0, 0.0, w as f64, h as f64));
        let pixels = self.node_pixels(doc, n, (w, h), view, visible, opts, false, 1);
        Some(Rendered { width: w as u32, height: h as u32, pixels })
    }

    /// Node `n` drawn into `w`×`h` transparent pixels through `view`, culled to `visible` (document
    /// space); `forced` draws it whatever its visibility and bounds. With [`RenderOptions::supersample`]
    /// and Art Optimized edges the art is drawn with hard edges at several times the size and
    /// averaged down, and [`RenderOptions::precise`] picks the quality render mode, as in a page
    /// render ([`Self::render_as`], [`Self::draw_frame`]). `cell` is as in [`Self::render_as`].
    #[allow(clippy::too_many_arguments)]
    fn node_pixels(
        &mut self,
        doc: &Document,
        n: &Arc<Node>,
        (w, h): (u16, u16),
        view: Affine,
        visible: Rect,
        opts: &RenderOptions,
        forced: bool,
        cell: u32,
    ) -> Vec<u8> {
        if opts.supersample
            && opts.anti_alias == AntiAlias::Art
            && let Some(k) = supersample_factor(u32::from(w), u32::from(h))
        {
            let hard = RenderOptions { anti_alias: AntiAlias::None, supersample: false, ..opts.clone() };
            // `supersample_factor` keeps each side within MAX_CONTEXT_SIDE, so it fits in a u16.
            let (bw, bh) = (u32::from(w) * k, u32::from(h) * k);
            let hairline = std::mem::replace(&mut self.hairline, f64::from(k));
            let big = self.node_pixels(doc, n, (bw as u16, bh as u16), Affine::scale(f64::from(k)) * view, visible, &hard, forced, k);
            self.hairline = hairline;
            return downsample(&big, bw as usize, bh as usize, k as usize);
        }
        let px = 1.0 / view.determinant().abs().sqrt().max(1e-12);
        let mut ctx = single_threaded_context(w, h);
        ctx.set_aliasing_threshold(opts.anti_alias.threshold());
        let symbols = doc.symbol_boxes().quick();
        let output = Affine::scale(1.0 / f64::from(cell.max(1))) * view;
        let frame = Frame { mt: false, doc, symbols: &symbols, view, visible, px, opts, ink: Ink::Display, output, plates: None };
        let raster = std::mem::replace(&mut self.raster, raster_settings(opts));
        self.begin_frame();
        (self.knockout, self.nested, self.backdrop, self.picture) = (false, 0, None, None);
        self.clip_paths.clear();
        if forced {
            self.draw_forced_arc(&mut ctx, &frame, n);
        } else {
            self.draw_arc(&mut ctx, &frame, n);
        }
        let pm = self.finish(&mut ctx, self.raster);
        self.end_frame();
        self.raster = raster;
        pm.data_as_u8_slice().to_vec()
    }

    /// Render any node (also one outside the tree, e.g. opacity-mask art) fitted into
    /// `size`×`size` pixels, optionally over a solid straight-alpha background. A symbol instance
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
        let _lent = vectorcraft_text::FontDb::global().lend_composite_fonts(&doc.composite_fonts);
        self.check_plugins(doc);
        self.check_symbols(doc);
        let b = brush_fx::cull_bounds(&doc.symbol_boxes(), n)?;
        let w = size.clamp(1, MAX_CONTEXT_SIDE) as u16;
        // A 1-pixel margin on each side, unless the thumbnail is too small to keep one.
        let s = (w as f64 - 2.0).max(1.0) / b.width().max(b.height()).max(1e-6);
        let view = Affine::translate((w as f64 / 2.0, w as f64 / 2.0)) * Affine::scale(s) * Affine::translate(-b.center().to_vec2());
        let mut pixels = self.node_pixels(doc, &Arc::new(n.clone()), (w, w), view, b.inflate(1.0, 1.0), opts, true, 1);
        if let Some(bg) = background {
            over_background(&mut pixels, bg);
        }
        Some(Rendered { width: w as u32, height: w as u32, pixels })
    }
}

/// The rasterizer settings for `opts`: the quality render mode when [`RenderOptions::precise`] is set.
fn raster_settings(opts: &RenderOptions) -> vello_cpu::RasterizerSettings {
    let render_mode = if opts.precise { vello_cpu::RenderMode::OptimizeQuality } else { vello_cpu::RenderMode::OptimizeSpeed };
    vello_cpu::RasterizerSettings { render_mode, ..Default::default() }
}

/// Premultiplied RGBA `pixels` drawn over the solid color `background` (`[r, g, b, a]`, straight).
pub(crate) fn over_background(pixels: &mut [u8], [r, g, b, a]: [u8; 4]) {
    let alpha = u32::from(a);
    let under = [u32::from(r) * alpha / 255, u32::from(g) * alpha / 255, u32::from(b) * alpha / 255, alpha];
    for p in pixels.as_chunks_mut::<4>().0 {
        let rest = 255 - u32::from(p[3]);
        for (v, bg) in p.iter_mut().zip(under) {
            *v = (u32::from(*v) + (bg * rest + 127) / 255).min(255) as u8;
        }
    }
}
