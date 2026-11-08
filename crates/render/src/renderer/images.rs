use super::{
    Affine, Arc, BezPath, Document, FillRule, Frame, HashMap, Ink, Node, NodeKind, Pixmap, Rect, RenderContext, Renderer, Shape, blob_key,
    blob_pixmap, fill_rule, image_mip, ink, lookup, mip_level, peniko, proof, vello_cpu,
};
use crate::images;

/// The color image node `n` of `doc` is colorized with ([`vectorcraft_doc::NodeExtras::colorize`]),
/// if it is and its pixels take it ([`vectorcraft_doc::ImageBlob::takes_colorize`]), as `ink`
/// paints it: the display color, or on an ink plane the color's own inks, as a fill of that color
/// paints them.
pub(super) fn image_tint(doc: &Document, n: &Node, ink: Ink) -> Option<[u8; 3]> {
    let c = n.extras().colorize.as_deref()?.color()?;
    if let NodeKind::Image(im) = &n.kind
        && doc.image_blob(im).is_some_and(|b| !b.takes_colorize())
    {
        return None;
    }
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
            kept.map(Shown::whole)
        } else if let Some(pm) = mips.then(|| self.kept_mip(f, &im.key, im.pixels.as_deref(), outline, tint, unit)).flatten() {
            Some(Shown::whole(pm))
        } else {
            self.image_pixmap(f.doc, &im.key, im.pixels.as_deref(), outline, !mips).map(|(key, pm)| {
                // Shrunk first, so what is made of the pixels below is made of the ones drawn: a
                // 141-megapixel photo is separated at the size it shows, not at its own (#4155).
                let (key, pm) = if mips { image_mip(key, pm, unit) } else { (key, pm) };
                let blob = f.doc.image_blob(im);
                // Zoomed in on it, only the tiles in view.
                if let Some(window) = in_view(f, im, &key, &pm, outline, tint) {
                    return window_pixmap(f, &key, &pm, window, outline, tint, blob);
                }
                Shown::whole(shown_pixmap(f, key, pm, outline, tint, blob))
            })
        };
        if let Some(Shown { pm, level: (lw, lh), at: (ax, ay) }) = shown {
            // The pixels drawn are `pm`, from (`ax`, `ay`) of the `lw`×`lh` pixels of the level.
            let sx = im.width as f64 / f64::from(lw.max(1));
            let sy = im.height as f64 / f64::from(lh.max(1));
            let drawn = Affine::scale_non_uniform(sx, sy) * Affine::translate((f64::from(ax), f64::from(ay)));
            let part = Rect::new(
                f64::from(ax) * sx,
                f64::from(ay) * sy,
                f64::from(ax + u32::from(pm.width())) * sx,
                f64::from(ay + u32::from(pm.height())) * sy,
            )
            .intersect(rect);
            let quality = if f.opts.smooth_images && !pixelated { peniko::ImageQuality::Medium } else { peniko::ImageQuality::Low };
            let sampler = peniko::ImageSampler { quality, ..Default::default() };
            ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(pm), sampler });
            match area {
                Some((bp, rule)) => {
                    ctx.set_transform(f.view);
                    ctx.set_paint_transform(im.xf * drawn);
                    ctx.set_fill_rule(fill_rule(*rule));
                    ctx.fill_path(bp);
                }
                None => {
                    ctx.set_transform(f.view * im.xf);
                    ctx.set_paint_transform(drawn);
                    ctx.fill_rect(&part);
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
        } else if !pending && !outline {
            self.image_left_out = self.image_left_out.map(|_| true);
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
        let key = shown_key(f, key, grey, tint);
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
        let key = shown_key(f, key, grey, tint);
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

/// Image pixels [`Renderer::draw_image_in`] draws: `pm`, from (`at`) of the `level` pixels of the
/// mip drawn (all of them, or the tiles in view).
struct Shown {
    pm: Arc<Pixmap>,
    level: (u32, u32),
    at: (u32, u32),
}

impl Shown {
    /// All of the pixels of a level.
    fn whole(pm: Arc<Pixmap>) -> Self {
        let level = (u32::from(pm.width()), u32::from(pm.height()));
        Self { pm, level, at: (0, 0) }
    }
}

/// Image pixels `pm` (kept as `key`) as [`Renderer::draw_image_in`] draws them: in greyscale with
/// `grey` (the outline view's) or colorized with `tint` as they are, else painted with the frame's
/// ink and shown through its Separations Preview plates. Kept with the images under
/// [`shown_key`], and marked used there.
fn shown_pixmap(
    f: &Frame,
    key: images::ImageKey,
    pm: Arc<Pixmap>,
    grey: bool,
    tint: Option<[u8; 3]>,
    blob: Option<&vectorcraft_doc::ImageBlob>,
) -> Arc<Pixmap> {
    let (key, pm) = match tint {
        Some(rgb) => colorized_pixmap(key, &pm, rgb),
        None => (key, pm),
    };
    // A colorized image is already in the frame's ink ([`image_tint`]), and with Separations
    // Preview its color is already the visible plates' (`proof::prepare`).
    if grey || tint.is_some() {
        return pm;
    }
    let (key, pm) = ink::ink_image(&key, &pm, f.ink, blob, true);
    match f.plates {
        Some(lut) => proof::plate_image(lut, &key, &pm, blob, true).1,
        None => pm,
    }
}

/// The tiles of image pixels `pm` (kept as `key`, the level of image `im` drawn) in the frame's
/// view, as a range of columns and rows, when the pixels' colours are made again
/// ([`shown_pixmap`]: colorized, separated or proofed), more than one tile of them is made, and
/// some are out of view. `None` otherwise: all the pixels are made, as they are for a CMYK image's
/// own inks (made from the whole image's).
fn in_view(
    f: &Frame,
    im: &vectorcraft_doc::ImageObject,
    key: &images::ImageKey,
    pm: &Pixmap,
    grey: bool,
    tint: Option<[u8; 3]>,
) -> Option<([u32; 2], [u32; 2])> {
    let made = !grey && (tint.is_some() || ink::ink_key(key, f.ink) != *key || f.plates.is_some());
    let (w, h) = (u32::from(pm.width()), u32::from(pm.height()));
    if !made || (w <= images::TILE && h <= images::TILE) || ink::shows_own_inks(key) {
        return None;
    }
    // The view in the level's pixels, a couple wider for smoothing at its edge.
    let to_level = (im.xf * Affine::scale_non_uniform(im.width as f64 / f64::from(w), im.height as f64 / f64::from(h))).inverse();
    let v = f.visible;
    let corners = [(v.x0, v.y0), (v.x1, v.y0), (v.x1, v.y1), (v.x0, v.y1)].map(|c| to_level * vectorcraft_geom::Point::from(c));
    let (x0, x1) = corners.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let (y0, y1) = corners.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p.y), b.max(p.y)));
    if ![x0, x1, y0, y1].iter().all(|v| v.is_finite()) {
        return None;
    }
    let tiles = |lo: f64, hi: f64, n: u32| {
        let last = n.saturating_sub(1) / images::TILE;
        let at = |v: f64| ((v.clamp(0.0, f64::from(n.saturating_sub(1))) as u32) / images::TILE).min(last);
        ([at(lo - 2.0), at(hi + 2.0)], last)
    };
    let ((cols, last_col), (rows, last_row)) = (tiles(x0, x1, w), tiles(y0, y1, h));
    // All of them in view: made whole, as without tiles.
    (cols != [0, last_col] || rows != [0, last_row]).then_some((cols, rows))
}

/// The tiles `cols` × `rows` of image pixels `pm` (kept as `key`) as [`shown_pixmap`] makes them,
/// each made and kept on its own ([`images::ImageKey::tile`]), in one pixmap.
fn window_pixmap(
    f: &Frame,
    key: &images::ImageKey,
    pm: &Pixmap,
    (cols, rows): ([u32; 2], [u32; 2]),
    grey: bool,
    tint: Option<[u8; 3]>,
    blob: Option<&vectorcraft_doc::ImageBlob>,
) -> Shown {
    let (w, h) = (u32::from(pm.width()), u32::from(pm.height()));
    let t = images::TILE;
    let at = (cols[0] * t, rows[0] * t);
    let size = (((cols[1] + 1) * t).min(w) - at.0, ((rows[1] + 1) * t).min(h) - at.1);
    let mut window = Pixmap::new(size.0 as u16, size.1 as u16);
    for (col, row) in (rows[0]..=rows[1]).flat_map(|r| (cols[0]..=cols[1]).map(move |c| (c, r))) {
        let tile_key = images::ImageKey { tile: Some((col as u16, row as u16)), ..key.clone() };
        let (x, y) = (col * t, row * t);
        let made = lookup(&shown_key(f, tile_key.clone(), grey, tint), true).or_else(|| {
            let part = images::crop(pm, x, y, t.min(w - x), t.min(h - y))?;
            Some(shown_pixmap(f, tile_key, Arc::new(part), grey, tint, blob))
        });
        if let Some(made) = made {
            images::paste(&mut window, &made, (x - at.0) as usize, (y - at.1) as usize);
        }
    }
    Shown { pm: Arc::new(window), level: (w, h), at }
}

/// The key [`Renderer::draw_image_in`] gives the pixels it draws of image pixels `key`: in
/// greyscale with `grey` (the outline view's) or colorized with `tint` as they are, else painted
/// with the frame's ink and shown through its Separations Preview plates.
fn shown_key(f: &Frame, key: images::ImageKey, grey: bool, tint: Option<[u8; 3]>) -> images::ImageKey {
    let key = images::ImageKey { tint, ..key };
    if grey || tint.is_some() {
        return key;
    }
    let key = ink::ink_key(&key, f.ink);
    match f.plates {
        Some(lut) => images::ImageKey { plates: lut.generation, ..key },
        None => key,
    }
}

/// Image pixels `pm` (kept as `key`) colorized with `rgb` ([`images::colorized`]), kept with the
/// images and marked used there, and their key.
fn colorized_pixmap(key: images::ImageKey, pm: &Pixmap, rgb: [u8; 3]) -> (images::ImageKey, Arc<Pixmap>) {
    let key = images::ImageKey { tint: Some(rgb), ..key };
    if let Some(p) = lookup(&key, true) {
        return (key, p);
    }
    let pm = images::store().insert(key.clone(), Arc::new(images::colorized(pm, rgb)));
    (key, pm)
}
