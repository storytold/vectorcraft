use super::{
    Affine, AppearanceItem, Arc, BezPath, BlendMode, FillRule, Frame, MAX_INLINE_DEPTH, MISSING_INLINE_LOGGED, Node, NodeKind, PlacedEntry,
    PlacedKey, Rect, RenderContext, Renderer, SUBSTITUTED, Shape, TextGeom, TextObject, blend_mode, blob_pixmap, depends_on_document, fx, holds_type,
    image_mip, ink, kurbo, lru, peniko, place, proof, stain_bits, text_geom, text_geom_snapped, vello_cpu,
};
use super::{AntiAlias, Pixmap, single_threaded_context};
use crate::paint;
use crate::{MAX_RASTER_SIDE, TypeAntiAlias, TypeEdgeEntry};

impl Renderer {
    pub(super) fn draw_text(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, t: &TextObject) {
        let t = &*f.doc.inline_resolved(t);
        // Type drawn as `fx_owner` (the node `draw_arc` is drawing, or type art that `fx_arts`
        // keeps, see `draw_fx_art`) takes its glyph geometry from `texts`, as on the fast path,
        // except in frames that snap type to pixels.
        let owner = self.fx_owner.as_ref().filter(|o| std::ptr::eq(&***o, n)).cloned();
        let (g, kept) = match (f.text_snap(t), owner) {
            (None, Some(a)) => self.text_geom_of(&a, t),
            (snap, _) => (Arc::new(text_geom_snapped(t, snap)), false),
        };
        self.draw_text_geom(ctx, f, n, t, &g, kept);
    }

    /// Draw the inline graphics of type `t` (laid out in `g`): each symbol's art through the
    /// normal node path, placed by the layout (inside the text's own transparency group, so its
    /// opacity and blend mode apply). A missing symbol draws nothing.
    fn draw_inlines(&mut self, ctx: &mut RenderContext, f: &Frame, t: &TextObject, g: &TextGeom) {
        if g.inlines.is_empty() && !t.runs.iter().any(|r| r.inline.is_some()) {
            return;
        }
        if self.inline_depth >= MAX_INLINE_DEPTH {
            return;
        }
        for r in t.runs.iter().filter_map(|r| r.inline.as_ref()) {
            if !f.doc.symbols.iter().any(|s| s.name == r.symbol) && !MISSING_INLINE_LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
                log::warn!("inline graphic shows symbol {:?}, which the document doesn't have: it draws nothing", r.symbol);
            }
        }
        self.inline_depth += 1;
        for ig in &g.inlines {
            let Some(art) = t.runs.get(ig.run).and_then(|r| r.inline.as_ref()) else { continue };
            let Some(sym) = f.doc.symbols.iter().find(|s| s.name == art.symbol) else { continue };
            // Strokes scale with the art (it is sized to the type, as in the SVG `<use>`).
            let (node, kept) = self.placed_art(&sym.art, t.xf * ig.xf * f.doc.symbol_natural_xf(&art.symbol), true, None);
            self.draw_placed_art(ctx, f, &node, kept);
        }
        self.inline_depth -= 1;
    }

    /// Symbol art `art` as an instance or an inline graphic draws it ([`place`]), and whether it is
    /// kept. Art that is a group, a layer or a live object without an appearance of its own is kept
    /// between frames under the art's `Arc`, `xf`, `scale_strokes` and the stain of instance
    /// `inst`: it and its members keep their `Arc`s from frame to frame, and the caches keyed by a
    /// node's address (geometry, stroke outlines, glyphs, live-effect geometry, evaluated live
    /// objects) hit for them. Other art is placed again in every frame, and so is art that holds
    /// symbol instances, type with inline graphics, images, placed documents or raster effects
    /// ([`depends_on_document`]), and art that the cache has no room for. Art that holds type is
    /// placed again when the fonts change.
    pub(super) fn placed_art(&mut self, art: &Arc<Node>, xf: Affine, scale_strokes: bool, inst: Option<&Node>) -> (Arc<Node>, bool) {
        let kept_kind = matches!(art.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) || vectorcraft_doc::live::is_live(art);
        if !kept_kind || fx::has_object_fx(art) {
            return (place(art, xf, scale_strokes, inst), false);
        }
        let stain = stain_bits(inst.and_then(vectorcraft_brush::stain));
        let key = (Arc::as_ptr(art) as usize, xf.as_coeffs().map(f64::to_bits), scale_strokes, stain);
        if let Some(e) = self.placed_arts.get(&key, self.stamp)
            && std::ptr::eq(e.art.as_ptr(), Arc::as_ptr(art))
            && e.fonts.is_none_or(|g| g == vectorcraft_text::FontDb::global().generation())
        {
            return match &e.placed {
                Some(placed) => (placed.clone(), true),
                None => (place(art, xf, scale_strokes, inst), false),
            };
        }
        let placed = place(art, xf, scale_strokes, inst);
        let kept = (!depends_on_document(art)).then(|| placed.clone());
        let keeps = kept.is_some();
        let fonts = (keeps && holds_type(art)).then(|| vectorcraft_text::FontDb::global().generation());
        // The bytes count the key and the entry, the art's allocation that the weak reference
        // keeps, and the kept art.
        let bytes = size_of::<(PlacedKey, PlacedEntry, Node)>() + kept.as_deref().map_or(0, lru::node_bytes);
        let inserted = self.placed_arts.insert(key, PlacedEntry { art: Arc::downgrade(art), placed: kept, fonts }, bytes, self.stamp);
        (placed, keeps && inserted)
    }

    /// Cached glyph geometry for a text node (keyed by Arc identity like paths).
    /// Text `t` that is not the node's own, such as the copy whose inline graphics
    /// `Document::inline_resolved` sized again, is laid out without the cache. Also returns
    /// whether the geometry is kept in the cache.
    pub(super) fn text_geom_of(&mut self, a: &Arc<Node>, t: &TextObject) -> (Arc<TextGeom>, bool) {
        if !matches!(&a.kind, NodeKind::Text(own) if std::ptr::eq(&**own, t)) {
            return (Arc::new(text_geom(t)), false);
        }
        let fonts = vectorcraft_text::FontDb::global().generation();
        if fonts != self.text_fonts {
            self.texts.clear();
            self.char_strokes.clear();
            self.text_fonts = fonts;
        }
        let key = Arc::as_ptr(a) as usize;
        if let Some((node, g)) = self.texts.get(&key, self.stamp)
            && Arc::ptr_eq(node, a)
        {
            return (g.clone(), true);
        }
        let g = Arc::new(text_geom(t));
        let kept = self.texts.insert(key, (a.clone(), g.clone()), g.bytes(), self.stamp);
        (g, kept)
    }

    /// Draw type `n` (`t`, laid out in `g`). The outlines of its character strokes are cached
    /// when `g` is kept in `texts` (`kept`).
    pub(super) fn draw_text_geom(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, t: &TextObject, g: &Arc<TextGeom>, kept: bool) {
        let xf = f.view * t.xf;
        if f.opts.outline {
            let mut p = g.all.clone();
            p.apply_affine(xf);
            ctx.set_transform(Affine::IDENTITY);
            ctx.set_stroke(kurbo::Stroke::new(self.hairline));
            ctx.set_paint(peniko::Color::BLACK);
            ctx.stroke_path(&p);
            // Colour bitmap glyphs: their boxes, as images show in outline mode.
            for (_, image, to_text) in &g.images {
                let mut b = image.rect().to_path(0.1);
                b.apply_affine(t.xf * *to_text);
                self.hairline(ctx, f, &b, [0, 0, 0, 255]);
            }
            // COLR glyphs: their layers' outlines.
            for (_, art) in &g.colours {
                self.draw_colour_glyph(ctx, f, art, t.xf);
            }
            self.draw_inlines(ctx, f, t, g);
            return;
        }
        if t.anti_alias != TypeAntiAlias::Sharp && self.draw_type_edges(ctx, f, n, t, g, kept) {
            return;
        }
        let tb = t.xf.transform_rect_bbox(g.bounds);
        // Overprinting characters darken per draw, like overprinting fills and strokes
        // ([`proof`]); the object blend mode is Normal here or applied by an enclosing layer.
        let overprints = proof::overprints(f.opts);
        let overprint = |ctx: &mut RenderContext, on: bool| {
            if overprints {
                ctx.set_blend_mode(if on { blend_mode(vectorcraft_color::BlendMode::Darken) } else { BlendMode::default() });
            }
        };
        // The object's own fills and strokes paint the whole outline: those below the Characters
        // row under the characters' own fill and stroke, the others over them.
        let (below, above) = n.appearance.split_contents();
        let all = (!n.appearance.items.is_empty()).then(|| {
            let mut all = g.all.clone();
            all.apply_affine(t.xf);
            all
        });
        if let Some(all) = &all {
            self.draw_text_items(ctx, f, n, below, all, tb);
        }
        if f.opts.highlight_substitutions {
            for (on, path) in [
                (f.opts.highlight_substituted_fonts, Some(&g.substituted_fonts)),
                (f.doc.setup.highlight_substituted_glyphs, Some(&g.substituted_glyphs)),
                // Measured only when shown: it lays the text out again.
                (f.opts.highlight_alternate_glyphs, f.opts.highlight_alternate_glyphs.then(|| g.alternate_glyphs(t))),
            ] {
                let Some(path) = path else { continue };
                if on && !path.elements().is_empty() {
                    ctx.set_transform(xf);
                    ctx.set_fill_rule(peniko::Fill::NonZero);
                    ctx.set_paint(SUBSTITUTED);
                    ctx.fill_path(path);
                }
            }
        }
        for (i, run) in t.runs.iter().enumerate() {
            // Colour bitmap glyphs paint themselves: the run's fill and stroke don't.
            for (_, image, to_text) in g.images.iter().filter(|(r, ..)| *r == i) {
                overprint(ctx, false);
                self.draw_glyph_image(ctx, f, image, t.xf * *to_text);
            }
            for (_, art) in g.colours.iter().filter(|(r, _)| *r == i) {
                self.draw_colour_glyph(ctx, f, art, t.xf);
            }
            let path = g.run(i);
            let path = path.as_ref();
            if path.elements().is_empty() {
                continue;
            }
            ctx.set_transform(xf);
            ctx.set_fill_rule(peniko::Fill::NonZero);
            if paint::set_fill_paint(ctx, &run.style.fill, g.bounds, f) {
                self.fold_alpha(ctx, f.ink, &run.style.fill);
                overprint(ctx, run.style.overprint_fill);
                ctx.fill_path(path);
            }
            if run.style.has_stroke() {
                // Character strokes are drawn in text space, with their cap, join and dashes.
                overprint(ctx, run.style.overprint_stroke);
                let chars = kept.then_some((g, i));
                self.draw_stroke(ctx, f, t.xf, path, FillRule::NonZero, &run.style.stroke_layer(), chars, g.bounds);
            }
        }
        overprint(ctx, false);
        self.draw_inlines(ctx, f, t, g);
        if let Some(all) = &all {
            self.draw_text_items(ctx, f, n, above, all, tb);
        }
    }

    /// Draw type `t` of node `n` (laid out in `g`) with its anti-aliasing method, None, Crisp or
    /// Strong, on the pixels of the output ([`Frame::output`]): the type is drawn offscreen on those
    /// pixels and placed back pixel for pixel. With None it is drawn with hard edges, so a pixel
    /// takes the type's color where the type covers at least half of it. With Crisp and Strong it
    /// is drawn as Sharp draws it, and again with the characters and the object's own fills and
    /// strokes in opaque black, which gives the share `c` of each pixel they cover; the pixel's
    /// color and opacity are then scaled by [`edge_coverage`] of `c` divided by `c`, so the opacity
    /// of the paints is kept. Where the output's pixels would number more than the frame's there
    /// and more than [`fx::PIXEL_BUDGET`], or be more than [`MAX_RASTER_SIDE`] a side (a frame
    /// drawn coarser than the output), the frame's own pixels are used.
    ///
    /// The pixels are kept (`type_edges`). In a later frame they are placed without drawing the
    /// type offscreen while the object, the output's zoom and rotation, the method and edge
    /// smoothing and what the type reads ([`Self::raster_inputs`]) are unchanged, the output has
    /// moved by whole pixels, and the kept pixels hold every pixel the frame needs. After a pan,
    /// the kept pixels are placed moved by it; after a pan that brings more of the type into view,
    /// the type is drawn offscreen again. The outlines of its character strokes are cached when
    /// `g` is kept in `texts` (`geom_kept`).
    ///
    /// Returns false, having drawn nothing, for type that is drawn as Sharp draws it: type with
    /// inline graphics, type with color glyphs (bitmap or COLR), type whose own fills or strokes
    /// include a brush stroke or a blend mode other than Normal, type that overprints while
    /// Overprint Preview is on, and type that shows a substitution highlight on screen. Drawn
    /// offscreen, those would blend with nothing or take the method on art that is not the
    /// characters'. It also returns false for type whose offscreen drawing would be more than
    /// [`MAX_RASTER_SIDE`] pixels a side.
    fn draw_type_edges(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, t: &TextObject, g: &Arc<TextGeom>, geom_kept: bool) -> bool {
        if proof::overprints(f.opts) && t.runs.iter().any(|r| r.style.overprint_fill || r.style.overprint_stroke) {
            return false;
        }
        let normal = vectorcraft_color::BlendMode::Normal;
        let brush = |i: &AppearanceItem| matches!(i, AppearanceItem::Stroke(st) if st.brush.is_some());
        if t.runs.iter().any(|r| r.inline.is_some()) || n.appearance.items.iter().any(|i| i.visible() && (i.blend() != normal || brush(i))) {
            return false;
        }
        // Color glyphs paint their own pixels and layers, whose opacity is not the characters' coverage.
        if !g.images.is_empty() || !g.colours.is_empty() {
            return false;
        }
        if f.opts.highlight_substitutions {
            let shows = |on: bool, path: &BezPath| on && !path.elements().is_empty();
            if shows(f.opts.highlight_substituted_fonts, &g.substituted_fonts)
                || shows(f.doc.setup.highlight_substituted_glyphs, &g.substituted_glyphs)
                // Measured only when shown: it lays the text out again.
                || (f.opts.highlight_alternate_glyphs && !g.alternate_glyphs(t).elements().is_empty())
            {
                return false;
            }
        }
        // Where the type draws: its glyphs, grown by the farthest its strokes reach, besides its
        // bounds (an area type frame's, or a point type estimate before it is laid out).
        let glyphs = type_reach(n, t, g);
        let reach = fx::cull_bounds(f.symbols, n).map_or(glyphs, |b| b.union(glyphs));
        // This frame's pixels to the output's: a scale and an offset (a supersampled export's
        // factor, an offscreen layer's crop).
        let grid = f.output * f.view.inverse();
        let [sx, _, _, sy, _, _] = grid.as_coeffs();
        let reach = reach.inflate(2.0 * f.px / sx.abs().min(1.0), 2.0 * f.px / sy.abs().min(1.0));
        let on = f.view.transform_rect_bbox(reach).intersect(Rect::new(0.0, 0.0, f64::from(ctx.width()), f64::from(ctx.height())));
        if !on.is_finite() {
            return false;
        }
        if on.width() <= 0.0 || on.height() <= 0.0 {
            return true;
        }
        let grid = edge_grid(on, grid);
        let output = grid * f.view;
        let [sx, _, _, sy, _, _] = grid.as_coeffs();
        let on = grid.transform_rect_bbox(on);
        // Whole pixels of the output.
        let (x0, y0) = (on.x0.floor(), on.y0.floor());
        let (Ok(w), Ok(h)) = (u16::try_from((on.x1 - x0).ceil() as u64), u16::try_from((on.y1 - y0).ceil() as u64)) else { return false };
        // (vello_cpu panics drawing near `u16::MAX` pixels a side.)
        if u32::from(w) > MAX_RASTER_SIDE || u32::from(h) > MAX_RASTER_SIDE {
            return false;
        }
        let view = Affine::translate((-x0, -y0)) * output;
        let visible = view.inverse().transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        let px = 1.0 / view.determinant().abs().sqrt().max(1e-12);
        let frame = Frame { mt: false, view, visible, px, output: view, ..*f };
        let threshold = match t.anti_alias {
            TypeAntiAlias::None => AntiAlias::None.threshold(),
            // The frame's pixels are smaller than the output's in a supersampled export, whose art
            // is smooth on the output's pixels (Art Optimized).
            _ if sx.abs() < 0.999 || sy.abs() < 0.999 => None,
            _ => f.opts.anti_alias.threshold(),
        };
        // Pixels kept from an earlier frame, for the type of the node itself (not a copy of its
        // text): drawn on output pixels of the same size, rotation and phase, holding those needed.
        let own = matches!(&n.kind, NodeKind::Text(o) if std::ptr::eq(&**o, t));
        let key = (std::ptr::from_ref(n) as usize, f.ink);
        let linear = fx::linear_key(output);
        let shift = output.translation();
        let settings = (t.anti_alias, threshold, f.opts.anti_alias, f.opts.precise, self.alpha.to_bits());
        let inputs = own.then(|| self.raster_inputs(f, n, fx::Reads::of(n)));
        let whole = |v: f64| (v - v.round()).abs() < 1e-6;
        let kept = inputs.as_ref().and_then(|inputs| {
            let e = self.type_edges.get(&key, self.stamp)?;
            let d = shift - e.shift;
            let (ox, oy) = (e.origin.0 + d.x.round(), e.origin.1 + d.y.round());
            let (iw, ih) = (f64::from(e.image.width()), f64::from(e.image.height()));
            let holds = ox <= x0 && oy <= y0 && ox + iw >= x0 + f64::from(w) && oy + ih >= y0 + f64::from(h);
            let fits = e.linear == linear && e.settings == settings && whole(d.x) && whole(d.y) && holds && e.inputs == *inputs && e.node == *n;
            fits.then(|| (e.image.clone(), ox, oy))
        });
        let (image, ox, oy) = match kept {
            Some(kept) => kept,
            None => {
                let pm = self.draw_type_pixels(&frame, n, t, g, geom_kept, (w, h), threshold);
                self.stats.type_edges += 1;
                let image = Arc::new(pm);
                if let Some(inputs) = inputs {
                    let bytes = usize::from(w) * usize::from(h) * 4 + lru::node_bytes(n);
                    let e = TypeEdgeEntry { node: n.clone(), linear, settings, inputs, shift, origin: (x0, y0), image: image.clone() };
                    self.type_edges.insert(key, e, bytes, self.stamp);
                }
                (image, x0, y0)
            }
        };
        ctx.set_transform(grid.inverse() * Affine::translate((ox, oy)));
        // The pixmap maps onto the rect pixel for pixel, each one on a pixel of the output.
        ctx.reset_paint_transform();
        let sampler = peniko::ImageSampler { quality: peniko::ImageQuality::Low, ..Default::default() };
        let (iw, ih) = (f64::from(image.width()), f64::from(image.height()));
        ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(image), sampler });
        ctx.fill_rect(&Rect::new(0.0, 0.0, iw, ih));
        ctx.set_transform(Affine::IDENTITY);
        true
    }

    /// The pixels of type `t` of node `n` (laid out in `g`) with its anti-aliasing method, drawn
    /// offscreen in `frame` on `w` × `h` pixels with edge threshold `threshold`
    /// ([`Self::draw_type_edges`]). The outlines of its character strokes are cached when `g` is
    /// kept in `texts` (`kept`).
    #[allow(clippy::too_many_arguments)]
    fn draw_type_pixels(
        &mut self,
        frame: &Frame,
        n: &Node,
        t: &TextObject,
        g: &Arc<TextGeom>,
        kept: bool,
        (w, h): (u16, u16),
        threshold: Option<u8>,
    ) -> Pixmap {
        let draw = |r: &mut Self, frame: &Frame, n: &Node, t: &TextObject| {
            let mut off = single_threaded_context(w, h);
            off.set_aliasing_threshold(threshold);
            r.inside_layer(|r| r.draw_text_geom(&mut off, frame, n, t, g, kept));
            off.flush();
            let mut pm = Pixmap::new(w, h);
            off.render_with(&mut pm, &mut r.resources, r.raster);
            pm
        };
        let sharp = TextObject { anti_alias: TypeAntiAlias::Sharp, ..t.clone() };
        let mut pm = draw(self, frame, n, &sharp);
        if t.anti_alias != TypeAntiAlias::None {
            let black = |p: &mut vectorcraft_color::Paint| {
                if !p.is_none() {
                    *p = vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK);
                }
            };
            let mut ink = sharp;
            for r in &mut ink.runs {
                black(&mut r.style.fill);
                black(&mut r.style.stroke);
            }
            let mut inked = n.clone();
            for item in &mut inked.appearance.items {
                black(item.paint_mut());
                match item {
                    AppearanceItem::Fill(fl) => fl.opacity = 1.0,
                    AppearanceItem::Stroke(st) => st.opacity = 1.0,
                }
            }
            let covered = draw(self, frame, &inked, &ink);
            for (p, c) in pm.data_mut().iter_mut().zip(covered.data()) {
                *p = edge_pixel(*p, c.a, t.anti_alias);
            }
        }
        pm
    }

    /// Paint some of type `n`'s own fills and strokes (`items`) on its glyph outlines `all`
    /// (document space; `tb` their bounds). Fills composite with their own opacity and blend mode;
    /// strokes take every stroke option, as on paths.
    fn draw_text_items(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, items: &[AppearanceItem], all: &BezPath, tb: Rect) {
        for item in items.iter().filter(|i| i.visible() && !i.paint().is_none()) {
            match item {
                AppearanceItem::Fill(fl) => {
                    let layered = fl.opacity < 1.0 || fl.blend != vectorcraft_color::BlendMode::Normal;
                    if layered {
                        ctx.set_transform(Affine::IDENTITY);
                        ctx.push_layer(None, Some(blend_mode(fl.blend)), Some(fl.opacity), None, None);
                    }
                    ctx.set_transform(f.view);
                    if paint::set_fill_paint(ctx, &fl.paint, tb, f) {
                        ctx.set_fill_rule(peniko::Fill::NonZero);
                        ctx.fill_path(all);
                    }
                    if layered {
                        ctx.pop_layer();
                    }
                }
                AppearanceItem::Stroke(st) if st.width > 0.0 => {
                    if st.brush.is_none() || !self.draw_brush(ctx, f, n, all, st) {
                        self.draw_stroke(ctx, f, Affine::IDENTITY, all, FillRule::NonZero, st, None, tb);
                    }
                }
                AppearanceItem::Stroke(_) => {}
            }
        }
    }

    /// Draw COLR glyph `art` (its layers in text space) through `xf` (text space → document), as
    /// art: kept between frames as symbol art is ([`Self::placed_art`]).
    fn draw_colour_glyph(&mut self, ctx: &mut RenderContext, f: &Frame, art: &Arc<Node>, xf: Affine) {
        let (node, kept) = self.placed_art(art, xf, true, None);
        let outer = std::mem::replace(&mut self.made_again, !kept);
        self.draw_node(ctx, f, &node, true);
        self.made_again = outer;
    }

    /// Draw colour bitmap glyph `image` through `xf` (its pixels → document space), its pixels
    /// decoded once and kept with the images' pixels.
    fn draw_glyph_image(&mut self, ctx: &mut RenderContext, f: &Frame, image: &vectorcraft_text::GlyphImage, xf: Affine) {
        let Some((key, pm)) = blob_pixmap(&image.blob, true) else { return };
        let (key, pm) = ink::ink_image(&key, &pm, f.ink, Some(&image.blob), true);
        let (key, pm) = match f.plates {
            Some(lut) => proof::plate_image(lut, &key, &pm, Some(&image.blob), true),
            None => (key, pm),
        };
        let unit = f.view * xf * Affine::scale_non_uniform(image.width as f64, image.height as f64);
        let pm = image_mip(&key, pm, unit);
        let sx = image.width as f64 / pm.width().max(1) as f64;
        let sy = image.height as f64 / pm.height().max(1) as f64;
        let quality = if f.opts.smooth_images { peniko::ImageQuality::Medium } else { peniko::ImageQuality::Low };
        ctx.set_paint(vello_cpu::Image {
            image: vello_cpu::ImageSource::Pixmap(pm),
            sampler: peniko::ImageSampler { quality, ..Default::default() },
        });
        ctx.set_transform(f.view * xf);
        ctx.set_paint_transform(Affine::scale_non_uniform(sx, sy));
        ctx.fill_rect(&image.rect());
        ctx.reset_paint_transform();
    }
}

/// How much of a pixel (out of 255) type that covers `c` of it shows with anti-aliasing method
/// `m`: `c` with Sharp; all of it from half coverage (127) up, else none, with None; `2c - 0.5`
/// with Crisp; and `2c` with Strong; each kept between 0 and all.
pub(crate) fn edge_coverage(m: TypeAntiAlias, c: u8) -> u8 {
    let c = i32::from(c);
    let shown = match m {
        TypeAntiAlias::None => {
            if c >= 127 {
                255
            } else {
                0
            }
        }
        TypeAntiAlias::Sharp => c,
        TypeAntiAlias::Crisp => 2 * c - 128,
        TypeAntiAlias::Strong => 2 * c,
    };
    shown.clamp(0, 255) as u8
}

/// Where (document space) type `t` of node `n`, laid out in `g`, paints: its glyphs grown by the
/// farthest the characters' strokes reach in text space, transformed as the type is, then grown by
/// the farthest the object's own strokes reach in document space. A stroke's reach takes in its
/// miter spikes and square caps ([`StrokeLayer::reach`]).
pub(crate) fn type_reach(n: &Node, t: &TextObject, g: &TextGeom) -> Rect {
    let chars = t.runs.iter().filter(|r| r.style.has_stroke()).map(|r| r.style.stroke_layer().reach()).fold(0.0, f64::max);
    let own = n.appearance.outset();
    t.xf.transform_rect_bbox(g.bounds.inflate(chars, chars)).inflate(own, own)
}

/// The map from a frame's pixels to the pixels [`Renderer::draw_type_edges`] draws type on, for
/// the region `on` of the frame's pixels: `grid`, which maps them to the output's, unless the
/// output's pixels there would number more than the frame's and more than [`fx::PIXEL_BUDGET`],
/// or be more than [`MAX_RASTER_SIDE`] a side, and then the frame's own pixels (the identity).
pub(crate) fn edge_grid(on: Rect, grid: Affine) -> Affine {
    let area = |r: Rect| r.width() * r.height();
    let (out, side) = (grid.transform_rect_bbox(on), f64::from(MAX_RASTER_SIDE));
    if area(out) > area(on).max(fx::PIXEL_BUDGET) || out.width() > side || out.height() > side { Affine::IDENTITY } else { grid }
}

/// Pixel `p` of type the anti-aliasing method `m` draws, where the type covers `c` (out of 255) of
/// the pixel: its color and opacity scaled by [`edge_coverage`] of `c` divided by `c`. A pixel the
/// type covers wholly or not at all is kept.
fn edge_pixel(p: vello_cpu::color::PremulRgba8, c: u8, m: TypeAntiAlias) -> vello_cpu::color::PremulRgba8 {
    if c == 0 || c == 255 {
        return p;
    }
    let (to, c) = (u32::from(edge_coverage(m, c)), u32::from(c));
    let scale = |v: u8| ((u32::from(v) * to + c / 2) / c).min(255) as u8;
    let a = scale(p.a);
    vello_cpu::color::PremulRgba8 { r: scale(p.r).min(a), g: scale(p.g).min(a), b: scale(p.b).min(a), a }
}
