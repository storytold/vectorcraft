use super::{
    Affine, AppearanceItem, Arc, BezPath, BlendMode, FillRule, Frame, MAX_INLINE_DEPTH, MISSING_INLINE_LOGGED, Node, NodeKind, PlacedEntry,
    PlacedKey, Rect, RenderContext, Renderer, SUBSTITUTED, Shape, TextGeom, TextObject, blend_mode, blob_pixmap, depends_on_document, fx, holds_type,
    image_mip, ink, kurbo, lru, peniko, place, proof, stain_bits, text_geom, text_geom_snapped, vello_cpu,
};
use crate::paint;

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
