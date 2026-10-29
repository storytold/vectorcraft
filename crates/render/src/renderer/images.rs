use super::{
    Affine, Arc, BezPath, Document, FillRule, Frame, HashMap, Ink, Node, Pixmap, Rect, RenderContext, Renderer, Shape, blob_key, blob_pixmap,
    fill_rule, image_mip, ink, lookup, mip_level, peniko, vello_cpu,
};
use crate::images;

/// The color image node `n` is colorized with ([`vectorcraft_doc::NodeExtras::colorize`]), if it
/// is, as `ink` paints it: the display color, or on an ink plane the color's own inks, as a fill
/// of that color paints them.
pub(super) fn image_tint(n: &Node, ink: Ink) -> Option<[u8; 3]> {
    let c = n.extras().colorize.as_deref()?.color()?;
    Some(ink.rgb(&c).map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
}

impl Renderer {
    /// Draw an image, colorized with `tint` when given ([`image_tint`]).
    pub(crate) fn draw_image(&mut self, ctx: &mut RenderContext, f: &Frame, im: &vectorcraft_doc::ImageObject, tint: Option<[u8; 3]>) {
        self.draw_image_in(ctx, f, im, None, tint, false);
    }

    /// Draw an image, or with `area` (a region in the document inside the image's frame, see
    /// [`plain_image`]) the image painted into that region; colorized with `tint` when given; with
    /// `pixelated` ([`vectorcraft_doc::NodeExtras::pixelated`]) without smoothing.
    pub(super) fn draw_image_in(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        im: &vectorcraft_doc::ImageObject,
        area: Option<&(BezPath, FillRule)>,
        tint: Option<[u8; 3]>,
        pixelated: bool,
    ) {
        let rect = Rect::new(0.0, 0.0, im.width as f64, im.height as f64);
        // Outline mode draws the image's frame, and with Document Setup → Show Images in Outline
        // Mode its pixels in greyscale under the frame.
        let outline = f.opts.outline;
        // With mips the full pixels are only looked at (not marked used) until they're drawn, and
        // a mip the store holds is drawn without them: they may have been dropped (#2820). A
        // pixelated image is drawn from its full pixels: a mip averages them.
        let mips = f.opts.image_mips && !pixelated;
        let unit = f.view * im.xf * Affine::scale_non_uniform(im.width as f64, im.height as f64);
        // The outline view shows the image's own pixels.
        let tint = tint.filter(|_| !outline);
        // With `kept_images_only`, an image still to be decoded: it draws as a light box, not as
        // an undecoded image.
        let mut pending = false;
        let shown = if outline && !f.doc.setup.outline_images {
            None
        } else if f.opts.kept_images_only {
            let kept = self.nearest_kept(f, &im.key, im.pixels.as_deref(), outline, tint, unit);
            pending = kept.is_none() && !self.noted_undecodable(f.doc, &im.key, im.pixels.as_deref());
            if pending {
                // Not decoded: a light box where the image will be, as a placed document's
                // bitmap shows while it is made (`placed_document`).
                ctx.set_paint(f.ink.fixed([128, 128, 128, 40]));
                match area {
                    Some((bp, rule)) => {
                        ctx.set_transform(f.view);
                        ctx.set_fill_rule(fill_rule(*rule));
                        ctx.fill_path(bp);
                    }
                    None => {
                        ctx.set_transform(f.view * im.xf);
                        ctx.fill_rect(&rect);
                    }
                }
            }
            kept
        } else if let Some(pm) = mips.then(|| self.kept_mip(f, &im.key, im.pixels.as_deref(), outline, tint, unit)).flatten() {
            Some(pm)
        } else {
            self.image_pixmap(f.doc, &im.key, im.pixels.as_deref(), outline, !mips).map(|(key, pm)| {
                let (key, pm) = match tint {
                    Some(rgb) => colorized_pixmap(key, &pm, rgb, !mips),
                    None => (key, pm),
                };
                // A colorized image is already in the frame's ink ([`image_tint`]).
                let (key, pm) = if outline || tint.is_some() { (key, pm) } else { ink::ink_image(&key, &pm, f.ink, f.doc.image_blob(im), !mips) };
                if mips { image_mip(&key, pm, unit) } else { pm }
            })
        };
        if let Some(pm) = shown {
            let sx = im.width as f64 / pm.width().max(1) as f64;
            let sy = im.height as f64 / pm.height().max(1) as f64;
            let quality = if f.opts.smooth_images && !pixelated { peniko::ImageQuality::Medium } else { peniko::ImageQuality::Low };
            let sampler = peniko::ImageSampler { quality, ..Default::default() };
            ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(pm), sampler });
            match area {
                Some((bp, rule)) => {
                    ctx.set_transform(f.view);
                    ctx.set_paint_transform(im.xf * Affine::scale_non_uniform(sx, sy));
                    ctx.set_fill_rule(fill_rule(*rule));
                    ctx.fill_path(bp);
                }
                None => {
                    ctx.set_transform(f.view * im.xf);
                    ctx.set_paint_transform(Affine::scale_non_uniform(sx, sy));
                    ctx.fill_rect(&rect);
                }
            }
            ctx.reset_paint_transform();
            // A dimmed layer's images: white over them, so they show at that opacity on paper.
            if let Some(dim) = self.dim_images.filter(|d| *d < 1.0 && !outline) {
                ctx.set_paint(f.ink.fixed([255, 255, 255, ((1.0 - dim) * 255.0).round() as u8]));
                match area {
                    Some((bp, rule)) => {
                        ctx.set_transform(f.view);
                        ctx.set_fill_rule(fill_rule(*rule));
                        ctx.fill_path(bp);
                    }
                    None => {
                        ctx.set_transform(f.view * im.xf);
                        ctx.fill_rect(&rect);
                    }
                }
            }
        } else if f.opts.mark_undecoded_images && !pending && (!outline || f.doc.setup.outline_images) {
            self.draw_undecoded_mark(ctx, f, im, area);
        }
        if outline {
            let mut p = rect.to_path(0.1);
            p.apply_affine(im.xf);
            self.hairline(ctx, f, &p, [0, 0, 0, 255]);
        }
    }

    /// Draw the mark of image `im` whose pixels can't be had ([`RenderOptions::mark_undecoded_images`]):
    /// a light gray box over its frame, crossed from corner to corner with a gray hairline, and
    /// with `area` (see [`Self::draw_image_in`]) only inside that region.
    fn draw_undecoded_mark(&mut self, ctx: &mut RenderContext, f: &Frame, im: &vectorcraft_doc::ImageObject, area: Option<&(BezPath, FillRule)>) {
        let (w, h) = (im.width as f64, im.height as f64);
        if let Some((bp, rule)) = area {
            ctx.set_transform(f.view);
            ctx.set_fill_rule(fill_rule(*rule));
            ctx.push_clip_layer(bp);
        }
        let rect = Rect::new(0.0, 0.0, w, h);
        ctx.set_transform(f.view * im.xf);
        ctx.set_paint(f.ink.fixed([128, 128, 128, 40]));
        ctx.fill_rect(&rect);
        let mut cross = rect.to_path(0.1);
        cross.move_to((0.0, 0.0));
        cross.line_to((w, h));
        cross.move_to((w, 0.0));
        cross.line_to((0.0, h));
        cross.apply_affine(im.xf);
        self.hairline(ctx, f, &cross, [128, 128, 128, 255]);
        if area.is_some() {
            ctx.pop_layer();
        }
    }

    /// Name image `key` recoloured by `map` (its hash `hash`) as `derived` (the key the adjusted
    /// art's image takes): drawing `derived` draws the recoloured pixels, made when first drawn and
    /// kept with the decoded images. Names past a limit drop the art made with them too, so it is
    /// made again.
    pub(crate) fn adjusted_image(&mut self, key: &str, derived: String, hash: u64, map: &vectorcraft_effects::ColorMap) {
        if self.adjusts.contains_key(&derived) {
            return;
        }
        if self.adjusts.len() >= 1024 {
            self.adjusts.clear();
            self.fx_arts.clear();
        }
        self.adjusts.insert(derived, (key.to_string(), hash, map.clone()));
    }

    /// The pixels of image `key` (`own`, the pixels an image holds itself, a blob of `doc`, or a
    /// recoloured one named by [`Self::adjusted_image`]), or a greyscale copy of them, with their
    /// key in the shared store. `touch` marks them used there.
    pub(crate) fn image_pixmap(
        &mut self,
        doc: &Document,
        key: &str,
        own: Option<&vectorcraft_doc::ImageBlob>,
        grey: bool,
        touch: bool,
    ) -> Option<(images::ImageKey, Arc<Pixmap>)> {
        let (key, colour) = match own.or_else(|| doc.images.get(key)) {
            Some(blob) => blob_pixmap(blob, touch)?,
            None => {
                let (src, hash, map) = self.adjusts.get(key)?.clone();
                let (src_key, src) = self.image_pixmap(doc, &src, None, false, touch)?;
                let key = images::ImageKey { adjust: Some(hash), ..src_key };
                match lookup(&key, touch) {
                    Some(pm) => (key, pm),
                    None => {
                        let mut pm = (*src).clone();
                        let mut memo: HashMap<[u8; 4], [u8; 3]> = HashMap::new();
                        for px in pm.data_mut().iter_mut().filter(|p| p.a > 0) {
                            let a = px.a as u32;
                            let [r, g, b] = *memo.entry([px.r, px.g, px.b, px.a]).or_insert_with(|| {
                                // Premultiplied to straight, adjusted, then premultiplied again.
                                let straight = [px.r, px.g, px.b].map(|v| ((v as u32 * 255 + a / 2) / a).min(255) as u8);
                                map.apply_rgb8(straight).map(|v| ((v as u32 * a + 127) / 255) as u8)
                            });
                            (px.r, px.g, px.b) = (r, g, b);
                        }
                        let pm = images::store().insert(key.clone(), Arc::new(pm));
                        (key, pm)
                    }
                }
            }
        };
        if !grey {
            return Some((key, colour));
        }
        let key = images::ImageKey { grey: true, ..key };
        if let Some(p) = lookup(&key, touch) {
            return Some((key, p));
        }
        let mut pm = (*colour).clone();
        for px in pm.data_mut() {
            // The luma of premultiplied components is the premultiplied luma.
            let l = (0.2126 * px.r as f32 + 0.7152 * px.g as f32 + 0.0722 * px.b as f32 + 0.5) as u8;
            (px.r, px.g, px.b) = (l, l, l);
        }
        let pm = images::store().insert(key.clone(), Arc::new(pm));
        Some((key, pm))
    }

    /// The key [`Self::image_pixmap`] gives the pixels of image `key` (or `own`) and their size, without
    /// making or looking at them: `None` until the image has been decoded.
    fn image_key(&self, doc: &Document, key: &str, own: Option<&vectorcraft_doc::ImageBlob>, grey: bool) -> Option<(images::ImageKey, (u16, u16))> {
        let (key, size) = match own.or_else(|| doc.images.get(key)) {
            Some(blob) => {
                let (key, _, size) = blob_key(blob);
                (key, size?)
            }
            None => {
                let (src, hash, _) = self.adjusts.get(key)?;
                let (src_key, size) = self.image_key(doc, src, None, false)?;
                (images::ImageKey { adjust: Some(*hash), ..src_key }, size)
            }
        };
        Some((if grey { images::ImageKey { grey: true, ..key } } else { key }, size))
    }

    /// The mip [`image_mip`] draws image `key` (or `own`) with through `m`, in greyscale with
    /// `grey` (the outline view's) or colorized with `tint`, when the store holds it, marked used
    /// there; `None` when it draws the full pixels or the mip is yet to be made.
    fn kept_mip(
        &self,
        f: &Frame,
        key: &str,
        own: Option<&vectorcraft_doc::ImageBlob>,
        grey: bool,
        tint: Option<[u8; 3]>,
        m: Affine,
    ) -> Option<Arc<Pixmap>> {
        let (key, (w, h)) = self.image_key(f.doc, key, own, grey)?;
        let key = images::ImageKey { tint, ..key };
        let key = if grey || tint.is_some() { key } else { ink::ink_key(&key, f.ink) };
        let level = mip_level(m, w, h);
        if level == 0 {
            return None;
        }
        images::store().get(&images::ImageKey { mip: level, ..key })
    }

    /// The pixels of image `key` (or `own`) that [`RenderOptions::kept_images_only`] draws through
    /// `m`, in greyscale with `grey` (the outline view's) or colorized with `tint`: of the mips the
    /// store holds, the one [`image_mip`] would draw, else the nearest finer one (the full pixels
    /// last), else the nearest coarser one. `None` when the store holds none. Nothing is marked used.
    fn nearest_kept(
        &self,
        f: &Frame,
        key: &str,
        own: Option<&vectorcraft_doc::ImageBlob>,
        grey: bool,
        tint: Option<[u8; 3]>,
        m: Affine,
    ) -> Option<Arc<Pixmap>> {
        let (key, (w, h)) = self.image_key(f.doc, key, own, grey)?;
        let key = images::ImageKey { tint, ..key };
        let key = if grey || tint.is_some() { key } else { ink::ink_key(&key, f.ink) };
        let level = mip_level(m, w, h);
        let s = images::store();
        let at = |l: u8| s.peek(&images::ImageKey { mip: l, ..key.clone() });
        (0..=level).rev().find_map(at).or_else(|| (level + 1..=16).find_map(at))
    }

    /// Whether the shared store notes image `key` (or `own`), or the image an adjusted image is
    /// made from, as one whose pixels can't be decoded. Nothing is decoded.
    fn noted_undecodable(&self, doc: &Document, key: &str, own: Option<&vectorcraft_doc::ImageBlob>) -> bool {
        match own.or_else(|| doc.images.get(key)) {
            Some(blob) => {
                let content = blob_key(blob).0.content;
                images::store().is_undecodable(&content)
            }
            None => self.adjusts.get(key).is_some_and(|(src, ..)| self.noted_undecodable(doc, src, None)),
        }
    }
}

/// Image pixels `pm` (kept as `key`) colorized with `rgb` ([`images::colorized`]), kept with the
/// images, and their key. `touch` marks them used there.
fn colorized_pixmap(key: images::ImageKey, pm: &Pixmap, rgb: [u8; 3], touch: bool) -> (images::ImageKey, Arc<Pixmap>) {
    let key = images::ImageKey { tint: Some(rgb), ..key };
    if let Some(p) = lookup(&key, touch) {
        return (key, p);
    }
    let pm = images::store().insert(key.clone(), Arc::new(images::colorized(pm, rgb)));
    (key, pm)
}
