use super::bounds::{cull_bounds, effected_path};
use super::entries::{Content, Drawn, FX_RASTER_BUDGET, FxGeom, Inputs, Kept, Made, Reads, Shadow, ShadowEntry};
use super::predicates::{evaluates_live, is_container, rasterize_bounds, reads_outside, shifted};
use super::raster::{
    DRAG_PIXEL_BUDGET, MAX_RASTERIZE, PIXEL_BUDGET, ViewCrop, content_changing, doc_raster_ppi, image_size, linear_key, pan_margin, pcolor,
    shade_filter, shadow_filter, silhouette,
};
use super::{
    Affine, Arc, BezPath, FillRule, Frame, GeomContext, Ink, Node, NodeKind, RasterFx, Rect, RenderContext, Renderer, Shape, Vec2, blend_mode,
    effects, fill_rule, peniko,
};

impl Renderer {
    /// Drop shadow / outer glow `fx` (shadow-cache slot `i` of `n`) as a cached raster: the
    /// silhouette `paint` draws is rendered once into a small crop, blurred and tinted on the CPU,
    /// and reused while the object, the zoom/rotation and what the silhouette reads ([`Inputs`]) are
    /// unchanged (pans only move it), and while interacting after the object only moved
    /// ([`Self::move_shadow`]). A raster of the whole reach too large for the budget is cropped to
    /// what can show in the `screen` (device pixels): the view, and around it the blur's spread and
    /// `around` (document units: how far the shadows and glows stacked over this one look), made
    /// coarser to fit the budget where the blur hides the coarser grid. Such a raster, and one
    /// made over it, is reused while a pan keeps the view inside the rect it was made for: the
    /// view, and when the raster kept for the object was made at the same zoom and rotation and no
    /// drag is on, the view with [`pan_margin`] around it, within `over` (the rect, in device
    /// pixels, that the cropped rasters of the shadows and glows it is made over were made for;
    /// `None` when it is made over none). `None` when the effect isn't a shadow/glow or the raster
    /// would still be too large (caller falls back to [`Self::draw_shadow_filter`]).
    /// While dragging on a multithreaded context, a raster that has to be made again counts as too
    /// large when it would hold more than [`DRAG_PIXEL_BUDGET`] pixels.
    #[allow(clippy::too_many_arguments)]
    fn shadow_raster(
        &mut self,
        f: &Frame,
        n: &Node,
        i: usize,
        fx: &RasterFx,
        reach: Rect,
        owner: Option<&Arc<Node>>,
        (screen, around, over): (Rect, f64, Option<Rect>),
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) -> Option<Shadow> {
        let (mode, opacity, dx, dy, blur, color, darkness) = match fx {
            RasterFx::DropShadow { mode, opacity, dx, dy, blur, color, darkness } => (*mode, *opacity, *dx, *dy, *blur, *color, *darkness),
            RasterFx::OuterGlow { mode, opacity, blur, color } => (*mode, *opacity, 0.0, 0.0, *blur, *color, None),
            _ => return None,
        };
        let linear = linear_key(f.view);
        let key = (n as *const Node as usize, i, f.ink);
        let stamp = self.stamp;
        let interacting = f.opts.interacting;
        // What the silhouette reads: as the raster kept for `n` found it, else found now.
        let reads = self.shadows.get(&key).filter(|e| e.node.is(n)).map_or_else(|| Reads::of(n), |e| e.inputs.reads);
        let inputs = self.raster_inputs(f, n, reads);
        let exact = |e: &ShadowEntry| {
            e.linear == linear && e.node.is(n) && e.inputs == inputs && (interacting || !e.moved) && e.view.is_none_or(|v| v.serves(f.view, screen))
        };
        if interacting && !self.shadows.get(&key).is_some_and(exact) {
            self.move_shadow(f, n, key, fx, reach + Vec2::new(dx, dy), owner, &inputs, paint);
        }
        let hit = self.shadows.get_mut(&key).filter(|e| exact(e));
        let (image, x, y, scale, moved, cropped) = match hit {
            Some(e) => {
                e.stamp = stamp;
                self.stats.moved += usize::from(e.moved);
                let p = f.view * e.anchor;
                (e.image.clone(), (p.x + e.rel.0).round(), (p.y + e.rel.1).round(), e.scale, e.moved, e.view.map(|v| v.served(f.view)))
            }
            None => {
                // An Outer Glow's silhouette grows by a quarter of a pixel of the document's raster grid first.
                let grow = if matches!(fx, RasterFx::OuterGlow { .. }) { 18.0 / doc_raster_ppi(f.doc) } else { 0.0 };
                let spread = (blur.max(0.0) * 1.5 + grow) / f.px + 2.0;
                let reach = reach + Vec2::new(dx, dy);
                let whole = f.view.transform_rect_bbox(reach).inflate(spread, spread);
                // The raster's region and scale within `budget` pixels: `None` when over the limits.
                let region = |r: Rect, coarser: bool, budget: f64| {
                    let (x0, y0) = (r.x0.floor(), r.y0.floor());
                    let (w, h) = (r.x1.ceil() - x0, r.y1.ceil() - y0);
                    let scale = if coarser { (w * h / budget).sqrt().ceil().max(1.0) } else { 1.0 };
                    if !scale.is_finite() || scale > 1.0 && blur / 2.0 / f.px / scale < 2.0 {
                        return None;
                    }
                    let (w, h) = ((w / scale).ceil(), (h / scale).ceil());
                    ((1.0..=8192.0).contains(&w) && (1.0..=8192.0).contains(&h) && w * h <= budget).then_some((x0, y0, w, h, scale))
                };
                // A glow too large for the budget is made coarser, where its blur hides the coarser grid: the filter
                // layer the caller falls back to can't make its silhouette solid.
                let full = region(whole, matches!(fx, RasterFx::OuterGlow { .. }), 4.0e6).filter(|_| over.is_none());
                // Else it is cropped to what can show in the view, within the budget of the filter
                // layer it would otherwise be drawn through, and made coarser in the same way. Made
                // again at this zoom and rotation (after a pan), it covers a margin around the view
                // too, so that later pans within the margin draw it.
                let ((x0, y0, w, h, scale), cropped) = match full {
                    Some(r) => (r, None),
                    None => {
                        let m = spread + around / f.px;
                        let again = self.shadows.get(&key).is_some_and(|e| e.linear == linear) && !f.opts.dragging;
                        let e = if again { pan_margin((screen.width(), screen.height()), m, PIXEL_BUDGET, 8192.0) } else { 0.0 };
                        let room = over.map_or(screen.inflate(e, e), |o| screen.inflate(e, e).intersect(o));
                        let r = whole.intersect(room.inflate(m, m));
                        if r.width() <= 0.0 || r.height() <= 0.0 {
                            return None;
                        }
                        (region(r, true, PIXEL_BUDGET)?, Some(room))
                    }
                };
                // During a drag, a larger shadow on a multithreaded context is filtered within the
                // drag's budget (`with_filters`), and no raster is kept.
                if f.opts.dragging && f.mt && w * h > DRAG_PIXEL_BUDGET {
                    return None;
                }
                let (w, h) = (w as u16, h as u16);
                // Silhouette (the object's painted alpha, and its colors under Darkness), offset by
                // the shadow distance.
                let mut off = f.offscreen_context(w, h);
                let view = Affine::scale(1.0 / scale) * Affine::translate((-x0, -y0)) * f.view;
                // Art is culled against the raster's own area: members of a group outside the view cast their shadows in it.
                let visible = view.inverse().transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
                let ink = darkness.map_or(f.ink, |d| f.ink.darkened(d));
                let crop = Frame { mt: false, view, visible, ink, ..*f };
                let before = self.stats.moved;
                self.inside_layer(|r| paint(r, &mut off, &shifted(&crop, dx, dy)));
                // A silhouette drawn over moved rasters makes a moved shadow.
                let moved = self.stats.moved > before;
                off.flush();
                let mut pm = vello_cpu::Pixmap::new(w, h);
                off.render(&mut pm, &mut self.resources);
                // What the raster is made from, for its object once it moves (`move_shadow`).
                let made =
                    Made { pixels: silhouette(&pm, darkness.is_some()), fx: vec![fx.clone()], size: reach.size(), channels: None, outline: None };
                let (wu, hu) = (w as usize, h as usize);
                let sigma = blur / 2.0 / f.px / scale;
                if darkness.is_some() {
                    // The object's own colors, blurred, with the black Darkness adds (premultiplied).
                    let mut planes: [Vec<f32>; 4] = [
                        pm.data().iter().map(|p| p.r as f32).collect(),
                        pm.data().iter().map(|p| p.g as f32).collect(),
                        pm.data().iter().map(|p| p.b as f32).collect(),
                        pm.data().iter().map(|p| p.a as f32).collect(),
                    ];
                    for plane in &mut planes {
                        effects::pixel::blur_plane(plane, wu, hu, sigma);
                    }
                    let [r, g, b, a] = &planes;
                    for (k, px) in pm.data_mut().iter_mut().enumerate() {
                        let al = (a[k] * opacity).clamp(0.0, 255.0).round();
                        let value = |plane: &Vec<f32>| (plane[k] * opacity).round().clamp(0.0, al) as u8;
                        *px = vello_cpu::color::PremulRgba8 { r: value(r), g: value(g), b: value(b), a: al as u8 };
                    }
                } else {
                    // An Outer Glow blurs a solid silhouette: wherever the art has any alpha, as AI files draw a glow round a glow.
                    let solid = matches!(fx, RasterFx::OuterGlow { .. });
                    let mut a: Vec<f32> = pm.data().iter().map(|p| if solid && p.a > 0 { 255.0 } else { p.a as f32 }).collect();
                    effects::pixel::dilate_plane(&mut a, wu, hu, grow / f.px / scale);
                    effects::pixel::blur_plane(&mut a, wu, hu, sigma);
                    let [cr, cg, cb] = f.ink.rgb(&color);
                    let tint = |v: f32, ch: f32| (v * ch).round().clamp(0.0, 255.0) as u8;
                    for (px, av) in pm.data_mut().iter_mut().zip(&a) {
                        let al = (av * opacity).clamp(0.0, 255.0);
                        *px = vello_cpu::color::PremulRgba8 { r: tint(al, cr), g: tint(al, cg), b: tint(al, cb), a: al.round() as u8 };
                    }
                }
                let image = std::sync::Arc::new(pm);
                let anchor = reach.origin();
                let p = f.view * anchor;
                let node = Drawn::of(n, owner);
                let rel = (x0 - p.x, y0 - p.y);
                let view = cropped.map(|room| ViewCrop::of(f.view, room));
                self.shadows.insert(key, ShadowEntry { node, linear, anchor, rel, image: image.clone(), scale, view, made, inputs, moved, stamp });
                self.trim_fx_rasters(FX_RASTER_BUDGET);
                (image, x0, y0, scale, moved, cropped)
            }
        };
        Some(Shadow { image, at: Affine::translate((x, y)) * Affine::scale(scale), mode, moved, cropped })
    }

    /// Draw shadow raster `s` through `to`, from the view the raster was made for to the one `ctx`
    /// draws in. The blend mode applies per draw: a blend layer would be a full-viewport compositing
    /// pass per shadow. A full-size raster drawn as made sits on whole pixels, so the cheapest
    /// sampler is exact.
    fn draw_shadow(&mut self, ctx: &mut RenderContext, s: &Shadow, to: Affine) {
        let (w, h) = (s.image.width() as f64, s.image.height() as f64);
        let at = to * s.at;
        ctx.set_transform(at);
        ctx.set_blend_mode(blend_mode(s.mode));
        let [a, b, c, d, _, _] = at.as_coeffs();
        let exact = (a, b, c, d) == (1.0, 0.0, 0.0, 1.0) && at.translation().x.fract() == 0.0 && at.translation().y.fract() == 0.0;
        let quality = if exact { peniko::ImageQuality::Low } else { peniko::ImageQuality::Medium };
        let sampler = peniko::ImageSampler { quality, ..Default::default() };
        ctx.reset_paint_transform();
        ctx.set_paint(vello_cpu::Image { image: vello_cpu::ImageSource::Pixmap(s.image.clone()), sampler });
        ctx.fill_rect(&Rect::new(0.0, 0.0, w, h));
        ctx.set_blend_mode(blend_mode(vectorcraft_doc::color::BlendMode::Normal));
        ctx.set_transform(Affine::IDENTITY);
    }

    /// Drop shadow / outer glow `fx` of what `paint` draws (bounded by `reach`) through a vello
    /// filter layer, when there is no raster for it ([`Self::shadow_raster`]). vello's shadow filter
    /// blurs the soft alpha, so an Outer Glow drawn here lacks the raster's solid silhouette.
    fn draw_shadow_filter(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        fx: &RasterFx,
        reach: Rect,
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) {
        let (mode, opacity, dx, dy, blur, color, darkness) = match fx {
            RasterFx::DropShadow { mode, opacity, dx, dy, blur, color, darkness } => (*mode, *opacity, *dx, *dy, *blur, *color, *darkness),
            RasterFx::OuterGlow { mode, opacity, blur, color } => (*mode, *opacity, 0.0, 0.0, *blur, *color, None),
            _ => return,
        };
        let filter = match darkness {
            Some(_) => shade_filter(blur),
            None => shadow_filter(0.0, 0.0, blur, pcolor(f.ink, &color)),
        };
        // The offset moves the content rather than the filter: vello_cpu drops layer content
        // that lies entirely outside the viewport before filtering.
        let reach = reach + Vec2::new(dx, dy);
        self.with_filters(ctx, f, reach, blur, Some((mode, opacity)), |r, c, fr, comp| {
            c.set_transform(fr.view);
            c.push_layer(None, comp.map(|m| blend_mode(m.0)), comp.map(|m| m.1), None, Some(filter));
            let ink = darkness.map_or(fr.ink, |d| fr.ink.darkened(d));
            let source = Frame { ink, ..*fr };
            paint(r, c, &shifted(&source, dx, dy));
            c.pop_layer();
        });
    }

    /// While interacting, give `n` (shadow-cache key `key`) the shadow or glow raster an earlier
    /// frame made for its object (the same id, effect slot and ink, at this zoom and rotation, with
    /// the same `inputs`) when only the silhouette's place changed after that frame: made by the
    /// same effect `fx` from a silhouette whose bounds have the size of `reach` (the content's
    /// bounds moved by the shadow's offset), and whose alpha (and colors, under Darkness) are those
    /// of `n`'s silhouette drawn into a raster placed as that one was against its anchor
    /// ([`silhouette`]). The raster is kept as `n`'s, marked
    /// moved, and is drawn at the nearest whole pixel.
    #[allow(clippy::too_many_arguments)]
    fn move_shadow(
        &mut self,
        f: &Frame,
        n: &Node,
        key: (usize, usize, Ink),
        fx: &RasterFx,
        reach: Rect,
        owner: Option<&Arc<Node>>,
        inputs: &Inputs,
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) {
        let (dx, dy, colored) = match fx {
            RasterFx::DropShadow { dx, dy, darkness, .. } => (*dx, *dy, darkness.is_some()),
            _ => (0.0, 0.0, false),
        };
        let (linear, stamp, size) = (linear_key(f.view), self.stamp, reach.size());
        let fits = |k: &(usize, usize, Ink), e: &ShadowEntry| {
            (k.1, k.2) == (key.1, key.2)
                && e.stamp < stamp
                && e.linear == linear
                && e.view.is_none()
                && e.node.id() == n.id
                && e.inputs == *inputs
                && e.made.fits(&[fx], size, None, None)
        };
        let Some((old, rel, w, h, scale)) =
            self.shadows.iter().find(|(k, e)| fits(k, e)).map(|(k, e)| (*k, e.rel, e.image.width(), e.image.height(), e.scale))
        else {
            return;
        };
        // The silhouette drawn where the old one lies in its raster.
        let anchor = reach.origin();
        let p = f.view * anchor;
        let mut off = f.offscreen_context(w, h);
        let view = Affine::scale(1.0 / scale) * Affine::translate((-(p.x + rel.0), -(p.y + rel.1))) * f.view;
        // Culled against the raster's area, as `shadow_raster` draws the silhouette.
        let visible = view.inverse().transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(w), f64::from(h)));
        let ink = match fx {
            RasterFx::DropShadow { darkness: Some(d), .. } => f.ink.darkened(*d),
            _ => f.ink,
        };
        let crop = Frame { mt: false, view, visible, ink, ..*f };
        self.inside_layer(|r| paint(r, &mut off, &shifted(&crop, dx, dy)));
        off.flush();
        let mut pm = vello_cpu::Pixmap::new(w, h);
        off.render(&mut pm, &mut self.resources);
        if self.shadows.get(&old).is_none_or(|e| silhouette(&pm, colored) != e.made.pixels) {
            return;
        }
        if let Some(mut e) = self.shadows.remove(&old) {
            (e.node, e.anchor, e.moved) = (Drawn::of(n, owner), anchor, true);
            self.shadows.insert(key, e);
        }
    }

    /// `draw_shape` for nodes with live effects.
    pub(crate) fn draw_shape_fx(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, bp: &BezPath, rule: FillRule) {
        // The node `draw_arc` is drawing gets its geometry from the cache: `bp` is its own path,
        // made from the node alone, so the effects give the same result every frame while the
        // plug-ins installed stay the same (`check_plugins`).
        let owner = self.fx_owner.as_ref().filter(|o| std::ptr::eq(&***o, n)).cloned();
        let mut cached = owner.map(|o| {
            let key = Arc::as_ptr(&o) as usize;
            let stamp = self.stamp;
            let e = match self.fx_geom.remove(&key) {
                Some(e) if Arc::ptr_eq(&e.node, &o) => e,
                _ => FxGeom { g: Arc::new(effected_path(n, bp)), items: vec![None; n.appearance.items.len()], node: o, stamp },
            };
            (key, FxGeom { stamp, ..e })
        });
        let g = match &cached {
            Some((_, e)) => e.g.clone(),
            None => Arc::new(effected_path(n, bp)),
        };
        self.draw_shape_fx_geom(ctx, f, n, &g, rule, cached.as_mut().map(|(_, e)| &mut e.items));
        if let Some((key, e)) = cached {
            self.fx_geom.insert(key, e);
        }
    }

    /// [`Self::draw_shape_fx`] on the effected geometry `g`, with the items' cached geometry.
    fn draw_shape_fx_geom(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        n: &Node,
        g: &BezPath,
        rule: FillRule,
        items: Option<&mut Vec<Option<Arc<BezPath>>>>,
    ) {
        if g.elements().is_empty() {
            return;
        }
        if f.opts.outline {
            self.hairline(ctx, f, g, [0, 0, 0, 255]);
            return;
        }
        let gctx = GeomContext::of(n);
        let rfx = effects::raster_effects(&n.appearance.effects);
        // Document-space reach of the painted geometry (strokes, arrowheads…).
        let outset = n.appearance.outset();
        let reach = g.bounding_box().inflate(outset, outset);
        let bounds = rasterize_bounds(f.symbols, n, &rfx);
        // Under Rasterize the content reaches as far as its fills' and strokes' raster effects paint.
        let reach = bounds.map_or(reach, |b| reach.union(b));
        let content = Content { node: n, owner: None, cache: Some(0), reach, outline: Some((g, rule)), bounds, kept: None };
        let mut items = items;
        self.raster_fx(ctx, f, &content, &rfx, &mut |r, c, fr| r.paint_items(c, fr, n, g, rule, &gctx, items.as_deref_mut()));
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
            let bounds = if blends { self.bounds_of(f.symbols, a) } else { None };
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
        // Art the cache did not keep is made again in the next frame.
        let kept = cache && self.fx_arts.get(&(Arc::as_ptr(a) as usize), self.stamp).is_some_and(|e| Arc::ptr_eq(&e.1, &art));
        let outer = self.made_again;
        self.made_again = outer || !kept;
        // With Crop Marks, the object inside the art paints its own raster effects.
        let rfx = if effects::has_crop_marks(a) { vec![] } else { effects::raster_effects(&a.appearance.effects) };
        match cull_bounds(f.symbols, &art) {
            Some(reach) if !f.opts.outline && !rfx.is_empty() => {
                // Feather, Inner Glow and Rasterize with Create Clipping Mask clip to the art's outline.
                let clipped = rfx.iter().any(|x| {
                    matches!(x, RasterFx::Feather { .. } | RasterFx::InnerGlow { .. }) || matches!(x, RasterFx::Rasterize(r) if r.clipping_mask)
                });
                let region = if clipped { self.clip_of(&art) } else { None };
                // A symbol's art can change while the instance doesn't: no cached shadows.
                let cache = (cache && !matches!(a.kind, NodeKind::SymbolInstance { .. })).then_some(0);
                let bounds = rasterize_bounds(f.symbols, a, &rfx);
                let reach = bounds.map_or(reach, |b| reach.union(b));
                let content = Content { node: a, owner: Some(a), cache, reach, outline: region.as_deref().map(|r| (&r.0, r.1)), bounds, kept: None };
                self.raster_fx(ctx, f, &content, &rfx, &mut |r, c, fr| r.draw_fx_art(c, fr, a, &art));
                ctx.set_transform(Affine::IDENTITY);
            }
            _ => self.draw_fx_art(ctx, f, a, &art),
        }
        self.made_again = outer;
    }

    /// The art [`Self::draw_object_fx`] draws for `a` (cached by `Arc` identity when `cache`):
    /// reshaped by its geometry effects, else type, images and paths as themselves and the others
    /// as their evaluated art (groups and layers: [`effects::evaluate_container`], keeping their
    /// knockout setting), recoloured by the colour adjustments in it ([`Self::adjusted_art`]);
    /// without the object's transparency and effects (a path keeps its geometry effects). Art that
    /// is made from live objects it evaluated ([`evaluates_live`]), or from type, symbol art or
    /// colors shaded through the color settings ([`reads_outside`]), is kept only while what it
    /// reads from outside `a` stays the same ([`Self::live_found`]).
    pub(super) fn fx_art(&mut self, doc: &vectorcraft_doc::Document, a: &Arc<Node>, cache: bool) -> Arc<Node> {
        let key = Arc::as_ptr(a) as usize;
        let mut kept = None;
        if cache
            && let Some((node, art, found)) = self.fx_arts.get(&key, self.stamp)
            && Arc::ptr_eq(node, a)
        {
            match found {
                None => return art.clone(),
                Some(was) => kept = Some((art.clone(), was.0)),
            }
        }
        let found = (kept.is_some() || (cache && (evaluates_live(a) || reads_outside(a)))).then(|| self.live_found(doc, a));
        if let Some((art, was)) = kept
            && found.as_ref().is_some_and(|f| f.0 == was)
        {
            return art;
        }
        let symbol = match &a.kind {
            NodeKind::SymbolInstance { symbol, .. } => {
                doc.symbols.iter().find(|s| s.name == *symbol).map(|s| crate::brush_fx::instance_art(&s.art, a))
            }
            _ => None,
        };
        let container = is_container(a);
        let art = match effects::crop_marks_art(a)
            .or_else(|| effects::revolve_art(a))
            .or_else(|| effects::solid_art(a))
            .or_else(|| effects::reshape(Some(doc), a, symbol.as_ref()))
        {
            Some(r) => r,
            None if container => effects::evaluate_container(Some(doc), a).unwrap_or_else(|| (**a).clone()),
            None if matches!(a.kind, NodeKind::Text(_) | NodeKind::Image(_) | NodeKind::Path { .. } | NodeKind::Compound { .. }) => (**a).clone(),
            None => effects::outline_art(Some(doc), a, symbol.as_ref()).unwrap_or_else(|| Node::group(a.id, vec![])),
        };
        let art = if effects::has_revolve(a) || effects::has_solid(a) { effects::evaluate_container(Some(doc), &art).unwrap_or(art) } else { art };
        let mut art = self.adjusted_art(art, effects::color_map(&a.appearance.effects).as_ref());
        // A path's geometry effects still apply when it is drawn.
        let path = matches!(art.kind, NodeKind::Path { .. } | NodeKind::Compound { .. });
        art.appearance.effects.retain(|e| path && effects::is_geometry(&e.id));
        let mut art = art.with_mask(None);
        art.opacity = 1.0;
        art.blend = Default::default();
        art.isolate = false;
        // One element: its pieces never knock each other out (a group's members still do as its
        // knockout setting says).
        art.knockout = if container { a.knockout } else { vectorcraft_doc::Knockout::Off };
        let art = Arc::new(art);
        if cache {
            let bytes = crate::lru::node_bytes(&art);
            self.fx_arts.insert(key, (a.clone(), art.clone(), found), bytes, self.stamp);
        }
        art
    }

    /// Draw `art`, the art [`Self::fx_art`] made for `a`. Type art that `fx_arts` keeps for `a` is
    /// drawn as `fx_owner`, so [`Self::draw_text`] takes its glyph geometry from `texts`.
    fn draw_fx_art(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>, art: &Arc<Node>) {
        let kept =
            matches!(art.kind, NodeKind::Text(_)) && self.fx_arts.get(&(Arc::as_ptr(a) as usize), self.stamp).is_some_and(|e| Arc::ptr_eq(&e.1, art));
        if !kept {
            return self.draw_node(ctx, f, art, true);
        }
        let outer = self.fx_owner.replace(art.clone());
        self.draw_node(ctx, f, art, true);
        self.fx_owner = outer;
    }

    /// `art` with the colour adjustments inside it applied, then `outer` (its object's own) on
    /// everything ([`effects::adjust`]). Embedded images become recoloured copies, cached with the
    /// decoded images under a key naming the adjustment.
    fn adjusted_art(&mut self, mut art: Node, outer: Option<&effects::ColorMap>) -> Node {
        art.appearance.effects.retain(|e| !effects::is_adjustment(&e.id));
        let mut wanted: Vec<(String, String, u64, effects::ColorMap)> = vec![];
        let adjusted = effects::adjust(&art, outer, &mut |key, map| {
            let mut h = std::hash::DefaultHasher::new();
            std::hash::Hash::hash(&format!("{map:?}"), &mut h);
            let hash = std::hash::Hasher::finish(&h);
            let derived = format!("{key}\u{0}adjust\u{0}{hash:016x}");
            wanted.push((key.to_string(), derived.clone(), hash, map.clone()));
            Some(derived)
        });
        for (key, derived, hash, map) in wanted {
            self.adjusted_image(&key, derived, hash, &map);
        }
        adjusted.unwrap_or(art)
    }

    /// Paint `content` with raster effects `rfx` (see the module docs): shadows and outer glows
    /// below it, the content blurred or feathered, inner glows above it (clipped to its outline).
    /// `paint` draws the content in the frame it is given.
    pub(super) fn raster_fx(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        content: &Content,
        rfx: &[RasterFx],
        paint: &mut dyn FnMut(&mut Self, &mut RenderContext, &Frame),
    ) {
        // Only the last MAX_RASTERIZE Rasterize effects are drawn.
        let count = rfx.iter().filter(|fx| matches!(fx, RasterFx::Rasterize(_))).count();
        let capped: Vec<RasterFx>;
        let rfx = if count > MAX_RASTERIZE {
            let mut left_out = count - MAX_RASTERIZE;
            capped = rfx
                .iter()
                .filter(|fx| {
                    let out = left_out > 0 && matches!(fx, RasterFx::Rasterize(_));
                    left_out -= usize::from(out);
                    !out
                })
                .cloned()
                .collect();
            &capped[..]
        } else {
            rfx
        };
        // Rasterize at the document's raster effects resolution takes it here.
        let resolved: Vec<RasterFx>;
        let rfx = if rfx.iter().any(|fx| matches!(fx, RasterFx::Rasterize(r) if r.ppi.is_none())) {
            let ppi = doc_raster_ppi(f.doc);
            resolved = rfx.iter().map(|fx| fx.at_document_ppi(ppi)).collect();
            &resolved[..]
        } else {
            rfx
        };
        // Rasterize makes an image of everything the effects before it paint, shadows and glows
        // included, and the shadows and glows after it paint around that image. When some come
        // after the last Rasterize, the stack splits after it, else when any effect comes before
        // it, the stack splits before it; the effects after the split paint around the content
        // drawn with those before it. Rasterize then comes first in its part.
        if let Some(at) = rfx.iter().rposition(|fx| matches!(fx, RasterFx::Rasterize(_))) {
            let paints_around = |fx: &[RasterFx]| fx.iter().any(|fx| !content_changing(&fx));
            let (_, rest) = rfx.split_at(at + 1);
            let split = if paints_around(rest) { Some(at + 1) } else { (at > 0).then_some(at) };
            if let Some(split) = split {
                let (before, after) = rfx.split_at(split);
                let line = 72.0 / doc_raster_ppi(f.doc);
                let (mut reach, mut spread) = (content.reach, content.bounds);
                // After a Rasterize on white without a clipping mask, the silhouette is its image,
                // and it stays the silhouette through the effects after it (as in `pixel_content`).
                let mut image = None;
                // Whether the image of a Rasterize before the split is over `rasterize_image`'s
                // limits. Such an image is kept only as far as the view needs it, so the effects
                // after the split are not cached.
                let (mut over, mut last_over) = (false, false);
                for fx in before {
                    if let RasterFx::Rasterize(r) = fx {
                        last_over = image_size(r, r.image(spread.unwrap_or(reach), line), line).is_none();
                        over |= last_over;
                    }
                    let o = fx.outset(reach);
                    reach = reach.inflate(o, o);
                    spread = spread.map(|b| fx.spread(b, line));
                    if let RasterFx::Rasterize(r) = fx
                        && r.white
                        && !r.clipping_mask
                    {
                        image = Some(spread.unwrap_or(reach).to_path(0.1));
                    }
                }
                // When the split comes after the Rasterize, the shadows and glows after it and the
                // content each draw the content before the split. The first draw makes the image,
                // and the others draw the one it keeps.
                let window = after.iter().fold(f.visible, |w, fx| {
                    let o = fx.outset(w);
                    w.inflate(o, o)
                });
                let frame = last_over.then_some((f.view, f.px));
                let kept = (split > at).then(|| Kept { window, frame, image: std::cell::RefCell::new(None) });
                let inner = Content {
                    node: content.node,
                    owner: content.owner,
                    cache: None,
                    reach: content.reach,
                    outline: content.outline,
                    bounds: content.bounds,
                    kept: kept.as_ref(),
                };
                let outline = image.as_ref().map(|g| (g, FillRule::NonZero)).or(content.outline);
                // When the split comes before the Rasterize, the part from it on draws the content
                // once, and its image is the one kept for a split above.
                let outer = Content {
                    node: content.node,
                    owner: content.owner,
                    cache: content.cache.filter(|_| !over),
                    reach,
                    outline,
                    bounds: spread,
                    kept: content.kept.filter(|_| split == at),
                };
                self.raster_fx(ctx, f, &outer, after, &mut |r, c, fr| r.raster_fx(c, fr, &inner, before, paint));
                return;
            }
        }
        let reach = content.reach;
        // The `Arc` drawn: the caller's, or the node `draw_arc` is drawing.
        let owner = content.owner.cloned().or_else(|| self.fx_owner.clone().filter(|o| std::ptr::eq(&**o, content.node)));
        // Below the content: shadows and outer glows, each of the content with the ones before it in the stack, as AI
        // files draw a glow round a glow and the SVG export nests them. Their rasters are made first to last, each once
        // and drawn into the later ones' silhouettes as an image (so n of them cost n rasters, not 2^n draws), then
        // drawn with the last lowest. One without a raster (an object drawn uncached, a glow past the budget) is drawn
        // through a filter layer from the content alone, and goes into the later silhouettes that way.
        let mut made: Vec<(&RasterFx, Option<Shadow>)> = vec![];
        let mut stacked = reach;
        let screen = Rect::new(0.0, 0.0, f64::from(ctx.width()), f64::from(ctx.height()));
        // How far (document units) a shadow or glow looks from a pixel to the silhouette under it.
        let looks = |fx: &RasterFx| match fx {
            RasterFx::DropShadow { dx, dy, blur, .. } => blur.max(0.0) * 1.5 + dx.abs() + dy.abs(),
            RasterFx::OuterGlow { blur, .. } => blur.max(0.0) * 1.5,
            _ => 0.0,
        };
        for (i, fx) in rfx.iter().enumerate().filter(|(_, x)| x.is_below()) {
            // A raster cropped to the view covers what the shadows and glows stacked over it read.
            let around: f64 = rfx.iter().skip(i + 1).filter(|x| x.is_below()).map(looks).sum();
            // The rect that the cropped rasters of the earlier ones (drawn into this one's silhouette) were all made for.
            let over = made.iter().filter_map(|(_, s)| s.as_ref()?.cropped).reduce(|a, b| a.intersect(b));
            let shadow = {
                let earlier = &made;
                let view = f.view;
                let mut art = |r: &mut Self, c: &mut RenderContext, fr: &Frame| {
                    let to = fr.view * view.inverse();
                    for (fx, s) in earlier.iter().rev() {
                        match s {
                            Some(s) => {
                                r.stats.moved += usize::from(s.moved);
                                r.draw_shadow(c, s, to);
                            }
                            None => r.draw_shadow_filter(c, fr, fx, reach, paint),
                        }
                    }
                    paint(r, c, fr);
                };
                let view = (screen, around, over);
                content.cache.and_then(|slot| self.shadow_raster(f, content.node, slot + i, fx, stacked, owner.as_ref(), view, &mut art))
            };
            stacked = match fx {
                RasterFx::DropShadow { dx, dy, blur, .. } => stacked.union((stacked + Vec2::new(*dx, *dy)).inflate(blur * 1.5, blur * 1.5)),
                _ => stacked.inflate(fx.outset(stacked), fx.outset(stacked)),
            };
            made.push((fx, shadow));
        }
        for (fx, s) in made.iter().rev() {
            match s {
                Some(s) => self.draw_shadow(ctx, s, Affine::IDENTITY),
                None => self.draw_shadow_filter(ctx, f, fx, reach, paint),
            }
        }
        // The content itself (blurred / feathered / filtered), on its pixels: a filter layer's blur
        // works on a coarser grid tied to the canvas, so the blurred art shifted by up to a pixel
        // and looked blocky as the view moved by parts of a pixel (#755).
        let blurs = rfx.iter().any(|fx| matches!(fx, RasterFx::Feather { radius } | RasterFx::GaussianBlur { radius } if *radius > 0.0));
        if blurs || rfx.iter().any(|fx| matches!(fx, RasterFx::Pixel(_) | RasterFx::Rasterize(_))) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RenderOptions;
    use vectorcraft_color::Color;
    use vectorcraft_doc::{Document, color::BlendMode};

    #[test]
    fn uncached_darkness_shadow_subtracts_before_separate_opacity() {
        let doc = Document::new(100.0, 100.0);
        let symbols = doc.symbol_boxes().quick();
        let opts = RenderOptions::default();
        let frame = Frame {
            mt: false,
            doc: &doc,
            symbols: &symbols,
            view: Affine::IDENTITY,
            visible: Rect::new(0.0, 0.0, 100.0, 100.0),
            px: 1.0,
            opts: &opts,
            ink: Ink::Display,
        };
        let mut ctx = frame.offscreen_context(100, 100);
        let mut r = Renderer::new();
        let fx =
            RasterFx::DropShadow { mode: BlendMode::Normal, opacity: 0.5, dx: 40.0, dy: 0.0, blur: 0.0, color: Color::BLACK, darkness: Some(0.25) };
        let source = Rect::new(10.0, 10.0, 30.0, 30.0);
        r.draw_shadow_filter(&mut ctx, &frame, &fx, source, &mut |_, c, f| {
            c.set_transform(f.view);
            c.set_paint(f.ink.color(&Color::rgb(0.8, 0.4, 0.2), 1.0));
            c.fill_rect(&source);
        });
        ctx.flush();
        let mut pm = vello_cpu::Pixmap::new(100, 100);
        ctx.render(&mut pm, &mut r.resources);
        let px = pm.data()[20 * 100 + 60];
        assert!(px.r.abs_diff(70) <= 1 && px.g.abs_diff(19) <= 1 && px.b == 0 && px.a.abs_diff(128) <= 1, "{px:?}");
        assert!(r.shadows.is_empty());
    }
}
