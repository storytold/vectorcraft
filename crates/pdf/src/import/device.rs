use super::entry::{INVISIBLE_TEXT_NOTE, MAX_NESTED, OUTLINED_TEXT_NOTE, UNSORTED, WHITE_KEY};
use super::state::{
    Builder, Decoded, Frame, FrameKind, GlyphRun, blend, fill_rule, line_width, mean_scale, resize_alpha, rgba_png, same_outline, stroke_layer,
    stroke_outline,
};
use super::{
    Affine, Appearance, AppearanceItem, Arc, BezPath, BlendMode, ClipPath, Device, FillLayer, FillRule, Glyph, GlyphDrawMode, Image, ImageBlob,
    ImageData, Name, Node, NodeKind, PathData, PathDrawMode, Pattern, Shape, SoftMask, clipped, extend_clip, mesh_shading, tag_key,
};

impl<'a> Device<'a> for Builder<'_> {
    fn set_soft_mask(&mut self, mask: Option<SoftMask<'a>>) {
        let spec = mask.map(|m| self.soft_mask(&m));
        let same = match (&spec, &self.mask) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            self.flush();
            self.mask = spec;
        }
    }

    fn set_blend_mode(&mut self, blend_mode: craft_pdf::interpret::BlendMode) {
        let b = blend(blend_mode);
        if b != self.blend {
            self.flush_glyphs();
            self.flush_text();
            self.blend = b;
        }
    }

    fn draw_path(&mut self, path: &BezPath, transform: Affine, paint: &craft_pdf::interpret::Paint<'a>, draw_mode: &PathDrawMode) {
        self.flush_glyphs();
        let walked = self.walked(false);
        if self.hold_glyph_stroke(path, transform, paint, draw_mode, walked) {
            return;
        }
        self.flush_text();
        let overprint = self.path_overprint();
        let mut bp = path.clone();
        bp.apply_affine(transform);
        if bp.elements().is_empty() {
            return;
        }
        let shading = match paint {
            craft_pdf::interpret::Paint::Pattern(p) => match p.as_ref() {
                Pattern::Shading(sp) => Some(sp),
                Pattern::Tiling(_) => None,
            },
            craft_pdf::interpret::Paint::Color(_) => None,
        };
        // Patch and triangle meshes are gradient meshes; a stroke's, clipped to the stroke's outline
        // (as Illustrator writes a freeform gradient on a stroke).
        if let Some(sp) = shading {
            let stroke = match draw_mode {
                PathDrawMode::Stroke(p) => Some(p),
                PathDrawMode::Fill(_) => None,
            };
            if let Some(meshes) = mesh_shading(sp, &mut self.colors, walked.and_then(|w| self.scan.ink(w, stroke.is_some(), true))) {
                match stroke {
                    None => return self.draw_meshes(meshes, &bp, sp.opacity, false),
                    Some(p) => {
                        if let Some(outline) = stroke_outline(path, transform, p) {
                            return self.draw_meshes(meshes, &outline, sp.opacity, true);
                        }
                    }
                }
            }
        }
        let pd = PathData::from_bezpath(&bp);
        let stroke = matches!(draw_mode, PathDrawMode::Stroke(_));
        let (paint, opacity) = self.paint(paint, stroke, walked);
        // A gradient that doesn't extend past its ends paints only between them.
        let width = match draw_mode {
            PathDrawMode::Stroke(p) => line_width(p, mean_scale(transform)),
            PathDrawMode::Fill(_) => 0.0,
        };
        let band = shading.and_then(|sp| extend_clip(sp, bp.bounding_box().inflate(width, width)));
        let wrap = |b: &mut Self, n: Node| match &band {
            Some(band) => {
                let clip = b.clip_node(band, FillRule::EvenOdd);
                let id = b.id();
                clipped(clip, n, id)
            }
            None => n,
        };
        match draw_mode {
            PathDrawMode::Fill(rule) => {
                let mut f = FillLayer::new(paint);
                (f.opacity, f.overprint) = (opacity, overprint);
                let mut n = Node::path(self.id(), pd, Appearance { items: vec![AppearanceItem::Fill(f)], ..Default::default() });
                if let NodeKind::Path { rule: r, .. } = &mut n.kind {
                    *r = fill_rule(*rule);
                }
                self.last_fill = Some((n.id, bp));
                let n = wrap(self, n);
                self.push_node(n);
            }
            PathDrawMode::Stroke(props) => {
                let mut st = stroke_layer(paint, opacity, props, mean_scale(transform));
                st.overprint = overprint;
                // Fill-then-stroke of the same path (the `B` operator, or a fill and then a stroke
                // of the same outline) becomes one object: a path with a fill and a live stroke.
                let blend = self.blend;
                let fill = self.last_fill.take();
                let last = match self.mask {
                    Some(_) => self.masked.last_mut(),
                    None => self.stack.last_mut().and_then(|f| f.children.last_mut()),
                };
                if band.is_none()
                    && let Some(last) = last
                    && let Some((id, outline)) = fill
                    && last.id == id
                    && last.blend == blend
                    // Opacity or a mask of its own came from a transparency group the fill was
                    // drawn in and this stroke wasn't.
                    && last.opacity == 1.0
                    && last.extras().mask.is_none()
                    && last.appearance.stroke().is_none()
                    && same_outline(&outline, &bp)
                {
                    let last = Arc::make_mut(last);
                    // The stroke's outline: closing it changes the stroke, not the fill.
                    if let Some(p) = last.path_data_mut() {
                        *p = pd;
                    }
                    last.appearance.items.push(AppearanceItem::Stroke(st));
                    return;
                }
                let n = Node::path(self.id(), pd, Appearance { items: vec![AppearanceItem::Stroke(st)], ..Default::default() });
                let n = wrap(self, n);
                self.push_node(n);
            }
        }
    }

    fn push_clip_path(&mut self, clip_path: &ClipPath) {
        self.flush();
        // Clips that contain the whole page do nothing visible; don't create groups for them. Only
        // a polygon that really covers it counts: a diamond touching the page edges has the page
        // as its bounding box but cuts its corners away (#1542).
        let redundant = clip_path.path.elements().len() <= 6
            && self.page.area() > 0.0
            && vectorcraft_geom::hit::polygon_covers_rect(&clip_path.path, self.page, 0.01);
        let kind = if redundant { FrameKind::Skip } else { FrameKind::Clip(Box::new(self.clip_node(&clip_path.path, fill_rule(clip_path.fill)))) };
        self.stack.push(Frame::new(kind));
    }

    fn push_transparency_group(&mut self, opacity: f32, mask: Option<SoftMask<'a>>, blend_mode: craft_pdf::interpret::BlendMode) {
        self.flush();
        let mask = mask.map(|m| self.soft_mask(&m));
        self.stack.push(Frame::new(FrameKind::Group { opacity, blend: blend(blend_mode), mask, isolate: false, knockout: false }));
        // Blend mode applies to the group as a whole, not to its children.
        self.blend = BlendMode::Normal;
    }

    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        transform: Affine,
        glyph_transform: Affine,
        paint: &craft_pdf::interpret::Paint<'a>,
        draw_mode: &GlyphDrawMode,
    ) {
        // Clipping text paints nothing; its outlines arrive separately as a clip path.
        if matches!(draw_mode, GlyphDrawMode::Clip) {
            return;
        }
        let stroke = match draw_mode {
            // Invisible text (a scanned page's recognized text, over the scan) draws nothing but
            // is still text: type with neither fill nor stroke, so it stays searchable.
            GlyphDrawMode::Invisible => {
                let typed = match glyph {
                    Glyph::Outline(o) => self.type_glyph(o, transform * glyph_transform, mean_scale(transform), None, None, false),
                    Glyph::Type3(_) => false,
                };
                if !typed {
                    self.warn(INVISIBLE_TEXT_NOTE);
                }
                return;
            }
            GlyphDrawMode::Fill | GlyphDrawMode::Clip => None,
            GlyphDrawMode::Stroke(p) => Some(p.clone()),
        };
        match glyph {
            Glyph::Outline(o) => {
                let scale = mean_scale(transform);
                let overprint = self.glyph_overprint(stroke.is_some());
                if self.type_glyph(o, transform * glyph_transform, scale, Some(paint), stroke.as_ref(), overprint) {
                    return;
                }
                self.flush_text();
                let mut bp = o.outline();
                bp.apply_affine(transform * glyph_transform);
                let (paint, opacity) = self.paint(paint, stroke.is_some(), self.walked(true));
                let same = self.glyphs.as_ref().is_some_and(|r| {
                    r.paint == paint
                        && r.opacity == opacity
                        && r.overprint == overprint
                        && r.stroke.as_ref().map(|s| s.line_width) == stroke.as_ref().map(|s| s.line_width)
                        && (r.scale - scale).abs() < 1e-9
                });
                if !same {
                    self.flush_glyphs();
                    self.glyphs = Some(GlyphRun { path: BezPath::new(), paint, opacity, stroke, scale, overprint });
                }
                if let Some(r) = &mut self.glyphs {
                    r.path.extend(bp.iter());
                }
                self.warn(OUTLINED_TEXT_NOTE);
            }
            Glyph::Type3(t3) => {
                if self.nested >= MAX_NESTED {
                    return;
                }
                self.flush();
                self.nested += 1;
                t3.interpret(self, transform, glyph_transform, paint);
                self.nested -= 1;
            }
        }
    }

    fn draw_image(&mut self, image: Image<'a, '_>, transform: Affine) {
        self.flush_glyphs();
        self.flush_text();
        match image {
            Image::Raster(r) => {
                let white = self.white_images;
                // Inside an alpha mask the same image is read again, white (its own cache key).
                let key = craft_pdf::interpret::CacheKey::cache_key(&r) ^ if white { WHITE_KEY } else { 0 };
                let st = r.stream();
                let (w, h) = (r.width(), r.height());
                // CMYK images keep their samples (read once per image).
                let known = !white && self.cmyk_keys.contains(&key);
                let cmyk = if known || white {
                    None
                } else {
                    crate::import_image::cmyk(st, w, h).unwrap_or_else(|why| {
                        self.warn(why);
                        None
                    })
                };
                if known || cmyk.is_some() {
                    self.cmyk_keys.insert(key);
                    // Its mask (read by the interpreter, whatever its kind) becomes an opacity
                    // mask, so the inks stay as they are.
                    let mask = if crate::import_image::has_mask(st) { self.alpha_mask(key, &r, w, h, transform) } else { None };
                    return self.add_image_masked(key, || cmyk.map(|blob| (blob, w, h)), transform, mask);
                }
                // JPEG passthrough for plain DeviceRGB/DeviceGray DCT images.
                let dict = st.dict();
                let filters = st.filters();
                let cs_ok = dict.get::<Name<'_>>(b"ColorSpace").is_some_and(|n| matches!(n.as_ref(), b"DeviceRGB" | b"DeviceGray"));
                let jpeg = !white
                    && filters.len() == 1
                    && matches!(filters[0], craft_pdf::syntax::Filter::DctDecode)
                    && cs_ok
                    && !dict.contains_key(b"SMask")
                    && !dict.contains_key(b"Mask")
                    && !dict.contains_key(b"Decode");
                if jpeg {
                    let bytes = st.raw_data().to_vec();
                    self.add_image(key, || Some((ImageBlob::new("image/jpeg", bytes), w, h)), transform);
                    return;
                }
                let mut decoded: Option<Decoded> = None;
                r.with_rgba(
                    |img, alpha| {
                        let (w, h, sf) = (img.width(), img.height(), img.scale_factors());
                        let a = alpha.map(|a| resize_alpha(&a, w, h)).unwrap_or_else(|| vec![255; (w * h) as usize]);
                        let rgba: Vec<u8> = match img {
                            _ if white => a.into_iter().flat_map(|a| [255, 255, 255, a]).collect(),
                            ImageData::Rgb(d) => d.data.as_chunks::<3>().0.iter().zip(a).flat_map(|(c, a)| [c[0], c[1], c[2], a]).collect(),
                            ImageData::Luma(d) => d.data.iter().zip(a).flat_map(|(g, a)| [*g, *g, *g, a]).collect(),
                        };
                        decoded = Some((rgba, w, h, sf));
                    },
                    None,
                );
                let Some((rgba, w, h, sf)) = decoded else {
                    self.warn("an image could not be decoded and was skipped");
                    return;
                };
                let xf = transform * Affine::scale_non_uniform(sf.0 as f64, sf.1 as f64);
                self.add_image(key, || rgba_png(rgba, w, h).map(|png| (ImageBlob::new("image/png", png), w, h)), xf);
            }
            Image::Stencil(s) => {
                let white = self.white_images;
                let key = craft_pdf::interpret::CacheKey::cache_key(&s) ^ if white { WHITE_KEY } else { 0 };
                let mut decoded: Option<Decoded> = None;
                s.with_stencil(
                    |luma, paint| {
                        let rgb = match paint {
                            _ if white => [255, 255, 255, 255],
                            craft_pdf::interpret::Paint::Color(c) => c.to_rgba().to_rgba8(),
                            _ => [0, 0, 0, 255],
                        };
                        let rgba = luma.data.iter().flat_map(|a| [rgb[0], rgb[1], rgb[2], *a]).collect();
                        decoded = Some((rgba, luma.width, luma.height, luma.scale_factors));
                    },
                    None,
                );
                let Some((rgba, w, h, sf)) = decoded else { return };
                let xf = transform * Affine::scale_non_uniform(sf.0 as f64, sf.1 as f64);
                self.add_image(key ^ 0x5_7e9c, || rgba_png(rgba, w, h).map(|png| (ImageBlob::new("image/png", png), w, h)), xf);
            }
        }
    }

    fn pop_clip_path(&mut self) {
        self.flush();
        self.pop_frame();
    }

    // The interpreter tells the flags of a form's group right after pushing it. (It draws a group
    // that adds nothing straight onto its parent, without pushing it.)
    fn set_knockout(&mut self) {
        if let Some(Frame { kind: FrameKind::Group { knockout, .. }, .. }) = self.stack.last_mut() {
            *knockout = true;
        }
    }

    fn set_isolated(&mut self) {
        if let Some(Frame { kind: FrameKind::Group { isolate, .. }, .. }) = self.stack.last_mut() {
            *isolate = true;
        }
    }

    fn pop_transparency_group(&mut self) {
        self.flush();
        self.pop_frame();
    }

    fn begin_marked_content(&mut self, tag: &[u8], _mcid: Option<i32>) {
        if self.nested > 0 {
            return;
        }
        let at = self.tag_at;
        self.tag_at += 1;
        let group = match self.scan.tags.get(at) {
            Some((t, g)) if self.aligned && *t == tag_key(tag) => *g,
            _ => {
                if self.route && self.aligned {
                    self.aligned = false;
                    let went = if self.unsure { "a hidden, non-printing layer of its page" } else { "its page's layer" };
                    self.warn(&format!("some art couldn't be told apart by layer and went to {went}"));
                }
                self.unsure.then_some(UNSORTED)
            }
        }
        .filter(|_| self.route);
        // Only a layer's marked content ends the runs gathered so far: other tags (a span with its
        // actual text, an artifact) leave a line of type whole.
        if group.is_some() {
            self.flush();
        }
        if let Some(g) = group
            && self.stack.len() > 1
            && let Some(f) = self.stack.last_mut()
        {
            f.group.get_or_insert(g);
        }
        self.marked.push(group);
    }

    fn end_marked_content(&mut self) {
        if self.nested > 0 {
            return;
        }
        // The runs gathered within a layer's sequence are its art: finish them before it ends.
        if self.marked.last().copied().flatten().is_some() {
            self.flush();
        }
        self.marked.pop();
    }
}
