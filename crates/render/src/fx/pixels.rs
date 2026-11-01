use super::bounds::{apply, item_effects};
use super::entries::{Content, DocInputs, Drawn, FX_RASTER_BUDGET, Inputs, KeptImage, Made, Reads, feather_outline};
#[cfg(test)]
use super::raster::RASTERIZED;
use super::raster::{
    DRAG_PIXEL_BUDGET, PIXEL_BUDGET, PixelEntry, ViewCrop, content_changing, doc_raster_ppi, drag_scale, draw_placed, draw_scaled, filter_region,
    filter_region_within, hard_edged, image_size, linear_key, looks_around, pan_margin,
};
use super::{
    Affine, AppearanceItem, Arc, BezPath, Channels, DefaultHasher, Document, FillRule, Frame, GeomContext, Hash, Hasher, Ink, Node, PixelFx,
    RasterFx, Rect, RenderContext, Renderer, Shape, blend_mode, effects, fill_rule, paint, peniko,
};

impl Renderer {
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
    /// lower-resolution raster is cached while the object, the zoom and what the content reads stay
    /// the same and a pan keeps the view inside the rect it was made for ([`PixelEntry::view`]): the
    /// view, and for a raster cropped to it that is made again at the same zoom and rotation, outside
    /// a drag and for content without a [`Kept`] image, the view with [`pan_margin`] around it. A
    /// lower-resolution one is kept resampled to the view's pixels ([`Self::resampled`]).
    /// While dragging, content without such a raster to draw renders within [`DRAG_PIXEL_BUDGET`]
    /// pixels and is not cached.
    pub(super) fn pixel_content(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        content: &Content,
        rfx: &[RasterFx],
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) {
        let chain: Vec<&RasterFx> = rfx.iter().filter(content_changing).collect();
        // When the first draw of this content kept its raster, that raster is drawn.
        if let Some(KeptImage::Raster(image, place, failed, image_left_out)) = content.kept.and_then(|k| k.image.borrow().clone()) {
            crate::paint::add_decode_failures(failed);
            if image_left_out {
                self.image_left_out = self.image_left_out.map(|_| true);
            }
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
        let linear = linear_key(f.view);
        let key = content.cache.map(|slot| (content.node as *const Node as usize, slot, f.ink));
        // Made again at this zoom and rotation (after a pan), a raster cropped to the view covers a
        // margin around the view too, so that later pans within the margin draw it.
        let again = key.and_then(|key| self.pixel_fx.get(&key)).is_some_and(|e| e.linear == linear) && content.kept.is_none() && !f.opts.dragging;
        let room = match margin {
            Some(m) if !fits && again => {
                let e = pan_margin((screen.width(), screen.height()), m, PIXEL_BUDGET, f64::from(crate::MAX_RASTER_SIDE));
                screen.inflate(e, e)
            }
            _ => screen,
        };
        let region = match margin {
            Some(m) if !fits => full.intersect(room.inflate(m, m)),
            _ => full,
        };
        let (x0, y0) = (region.x0.floor(), region.y0.floor());
        let (rw, rh) = ((region.x1 - x0).max(1.0), (region.y1 - y0).max(1.0));
        // (vello_cpu panics drawing near `u16::MAX` pixels a side.)
        let side = f64::from(crate::MAX_RASTER_SIDE);
        let k = (PIXEL_BUDGET / (rw * rh)).sqrt().min(side / rw).min(side / rh).min(1.0);
        let (w, h) = ((rw * k).ceil().clamp(1.0, side), (rh * k).ceil().clamp(1.0, side));
        // The cached raster: of the whole content at full size at this zoom, else of this view (and
        // the margin around it).
        let whole = fits && k >= 1.0;
        let in_view = (!whole).then(|| ViewCrop::of(f.view, room));
        let stamp = self.stamp;
        let channels = match f.ink.base() {
            Ink::Cmy => effects::pixel::Channels::CmyPlane,
            Ink::K => effects::pixel::Channels::KPlane,
            Ink::Display | Ink::RichBlack if f.doc.color_mode == vectorcraft_doc::ColorMode::Cmyk => effects::pixel::Channels::Cmyk,
            _ => effects::pixel::Channels::Rgb,
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
            if e.image_left_out {
                self.image_left_out = self.image_left_out.map(|_| true);
            }
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
        let outer_image_left_out = self.image_left_out.replace(false);
        let pattern_images_left_out = crate::pattern::images_left_out();
        // A Rasterize first in the chain draws the content on its image's own grid instead, unless
        // that image is too large: the content is drawn once.
        let mut rasterized = match chain.first() {
            Some(RasterFx::Rasterize(r)) => {
                let rect = r.image(content.bounds.unwrap_or(content.reach), line);
                let kept = content.kept.and_then(|k| match &*k.image.borrow() {
                    Some(KeptImage::Grid(image, failed, image_left_out)) => {
                        crate::paint::add_decode_failures(*failed);
                        if *image_left_out {
                            self.image_left_out = self.image_left_out.map(|_| true);
                        }
                        Some(image.clone())
                    }
                    _ => None,
                });
                let image = kept.or_else(|| {
                    let failures = crate::paint::decode_failures();
                    let outer_image_left_out = self.image_left_out.replace(false);
                    let pattern_images_left_out = crate::pattern::images_left_out();
                    let image = self.rasterize_image(f, r, rect, paint).map(std::rc::Rc::new);
                    let left_out = self.image_left_out.unwrap_or(false) || crate::pattern::images_left_out() > pattern_images_left_out;
                    self.image_left_out = outer_image_left_out.map(|outer| outer || left_out);
                    if let (Some(k), Some(image)) = (content.kept, &image) {
                        *k.image.borrow_mut() = Some(KeptImage::Grid(image.clone(), crate::paint::decode_failures() - failures, left_out));
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
        let image_left_out = self.image_left_out.unwrap_or(false) || crate::pattern::images_left_out() > pattern_images_left_out;
        self.image_left_out = outer_image_left_out.map(|outer| outer || image_left_out);
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
            let entry =
                PixelEntry { node, linear, anchor, rel, k, image: image.clone(), view: in_view, made, inputs, moved, failed, image_left_out, stamp };
            self.pixel_fx.insert(key, entry);
            self.trim_fx_rasters(FX_RASTER_BUDGET);
        }
        // An image over the limits was drawn at a frame's resolution: its raster is kept.
        match content.kept.filter(|kept| kept.image.borrow().is_none()) {
            Some(kept) => {
                let place = f.view.inverse() * Affine::translate((x0, y0)) * Affine::scale(1.0 / k);
                *kept.image.borrow_mut() = Some(KeptImage::Raster(image.clone(), place, failed, image_left_out));
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
    /// layer's Dim Images and the options they are drawn with (with
    /// [`crate::RenderOptions::kept_images_only`], also which pixels and undecodable notes the
    /// image store holds); for type the fonts, whether overprinting shows
    /// ([`crate::proof::overprints`]) and Document Setup's substitution highlights; and the
    /// plug-ins installed, when an object in it has a plug-in effect, and when it shows symbol or
    /// pattern art and an object in the document's symbol or pattern art has one.
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
            o.mark_undecoded_images.hash(&mut h);
            // With only kept pixels drawn, a raster is made again once the store holds more pixels
            // or notes an image as undecodable.
            o.kept_images_only.then(|| crate::images::store().added()).hash(&mut h);
        }
        if reads.brushes {
            self.brushes.library_key(f.doc, self.stamp).hash(&mut h);
        }
        if reads.text {
            let o = f.opts;
            let highlights = o.highlight_substitutions.then_some((
                o.highlight_substituted_fonts,
                f.doc.setup.highlight_substituted_glyphs,
                o.highlight_alternate_glyphs,
            ));
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
    pub(super) fn trim_fx_rasters(&mut self, budget: usize) {
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
    pub(super) fn with_filters(
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
    pub(super) fn paint_items(
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
