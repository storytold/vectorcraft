use super::{
    Affine, Arc, BezPath, ClipEntry, ClipPaintEntry, Compose, Composite, Document, FillRule, Frame, GeomEntry, ISOLATION_DIM, Ink, MAX_GUIDE_DASHES,
    Node, NodeId, NodeKind, Rect, RenderContext, RenderOptions, Renderer, Shape, SymbolBoxes, TraceView, blend_mode, brush_fx, effects, fx, group,
    holds_instance, holds_plugin_effect, images, kurbo, lru, mask_value, painted_items, plain_image, polygon_length, rects_overlap,
    shape_masked_image, single_plain_fill, text_geom_snapped, white_shape_mask,
};

impl Renderer {
    /// Cached cull bounds of a node (containers union their cached children), symbol instances
    /// measured by their art box from `symbols`.
    pub(crate) fn bounds_of(&mut self, symbols: &SymbolBoxes, a: &Arc<Node>) -> Option<Rect> {
        let key = Arc::as_ptr(a) as usize;
        if let Some(e) = self.geom.get_mut(&key)
            && Arc::ptr_eq(&e.node, a)
        {
            e.stamp = self.stamp;
            return e.bounds;
        }
        let b = match &a.kind {
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } if !fx::has_object_fx(a) => {
                let mut acc: Option<Rect> = None;
                // An Image Trace object's hidden source image shows in some of its views.
                let source = a.extras().trace.is_some().then(|| children.first()).flatten();
                for c in children {
                    if c.visible || source.is_some_and(|s| Arc::ptr_eq(s, c)) {
                        acc = vectorcraft_geom::union_opt(acc, self.bounds_of(symbols, c));
                    }
                }
                acc
            }
            _ => brush_fx::cull_bounds(symbols, a),
        };
        self.geom.insert(key, GeomEntry { node: a.clone(), bounds: b, path: None, stamp: self.stamp });
        b
    }

    /// Drop the cached bounds of symbol instances and of the objects that hold them when `doc`'s
    /// symbols are not those of the document last drawn (by name and art): those bounds measured
    /// the instances by their symbol's art.
    pub(crate) fn check_symbols(&mut self, doc: &Document) {
        let same = self.symbols.len() == doc.symbols.len()
            && self.symbols.iter().zip(&doc.symbols).all(|((name, art), s)| *name == s.name && std::ptr::eq(art.as_ptr(), Arc::as_ptr(&s.art)));
        if !same {
            self.geom.retain(|_, e| !holds_instance(&e.node));
            self.symbols = doc.symbols.iter().map(|s| (s.name.clone(), Arc::downgrade(&s.art))).collect();
        }
    }

    /// When a plug-in has been installed or removed since the last check
    /// ([`effects::plugin_revision`]), drop what the caches keep from plug-in effects: the effect
    /// geometry of paths with a plug-in effect, the cull bounds of objects that have one or hold one,
    /// of symbol instances and of the objects that hold them, and the art drawn for the effects of
    /// objects with a plug-in effect. Rasters of raster effects and evaluations of live objects hold
    /// the revision in their keys when what they draw has a plug-in effect (`fx::Inputs`,
    /// `live::resources_key`).
    pub(crate) fn check_plugins(&mut self) {
        let revision = effects::plugin_revision();
        if revision == self.plugin_revision {
            return;
        }
        self.plugin_revision = revision;
        self.fx_geom.retain(|_, e| !effects::has_plugin_effect(&e.node));
        self.geom.retain(|_, e| !holds_plugin_effect(&e.node) && !holds_instance(&e.node));
        self.fx_arts.retain(|(node, ..)| !effects::has_plugin_effect(node));
    }

    /// Cached region a clip group's clipping path `a` clips to ([`effects::clip_outline`]):
    /// outlining text and uniting shapes is too slow to repeat every frame.
    pub(crate) fn clip_of(&mut self, a: &Arc<Node>) -> Option<Arc<(BezPath, FillRule)>> {
        let key = Arc::as_ptr(a) as usize;
        if let Some((node, region)) = self.clips.get(&key, self.stamp)
            && Arc::ptr_eq(node, a)
        {
            return region.clone();
        }
        let region = effects::clip_outline(a).map(Arc::new);
        let bytes = size_of::<ClipEntry>() + region.as_ref().map_or(0, |r| lru::path_bytes(&r.0));
        self.clips.insert(key, (a.clone(), region.clone()), bytes, self.stamp);
        region
    }

    /// Cached fills and strokes of clipping path `a` ([`Node::clip_paint`]): the parts keep their
    /// allocations between frames, so their own geometry stays cached.
    fn clip_paint_of(&mut self, a: &Arc<Node>) -> (Option<Arc<Node>>, Option<Arc<Node>>) {
        let key = Arc::as_ptr(a) as usize;
        if let Some((node, fill, stroke)) = self.clip_paints.get(&key, self.stamp)
            && Arc::ptr_eq(node, a)
        {
            return (fill.clone(), stroke.clone());
        }
        let paint = a.clip_paint();
        let (fill, stroke) = (paint.fill.map(Arc::new), paint.stroke.map(Arc::new));
        let bytes = size_of::<ClipPaintEntry>() + [&fill, &stroke].iter().filter_map(|p| p.as_deref()).map(lru::node_bytes).sum::<usize>();
        self.clip_paints.insert(key, (a.clone(), fill.clone(), stroke.clone()), bytes, self.stamp);
        (fill, stroke)
    }

    /// Cached BezPath of a path or compound path node (its bounds cached with it, see
    /// [`Self::bounds_of`]).
    fn path_of(&mut self, symbols: &SymbolBoxes, a: &Arc<Node>) -> Option<Arc<BezPath>> {
        let key = Arc::as_ptr(a) as usize;
        if let Some(e) = self.geom.get(&key)
            && Arc::ptr_eq(&e.node, a)
            && let Some(p) = &e.path
        {
            return Some(p.clone());
        }
        let p = Arc::new(a.stroke_path()?);
        let bounds = self.bounds_of(symbols, a);
        self.geom.insert(key, GeomEntry { node: a.clone(), bounds, path: Some(p.clone()), stamp: self.stamp });
        Some(p)
    }

    /// The cached path of path or compound path `n` ([`Self::path_of`]) when the frame is in
    /// Outline view and `n` is `fx_owner`, else `None`.
    fn outline_path(&mut self, f: &Frame, n: &Node) -> Option<Arc<BezPath>> {
        let owner = self.fx_owner.as_ref().filter(|o| f.opts.outline && std::ptr::eq(&***o, n)).cloned()?;
        self.path_of(f.symbols, &owner)
    }

    /// Whether `a` is left out of this frame: hidden, a skipped template or guide, or culled.
    fn skipped(&mut self, f: &Frame, a: &Arc<Node>) -> bool {
        let skipped_view_item = (f.opts.skip_templates || f.opts.hide_templates) && matches!(a.kind, NodeKind::Layer { template: true, .. })
            || (f.opts.skip_templates || f.opts.hide_guides) && matches!(a.kind, NodeKind::Path { guide: true, .. });
        if !a.visible || skipped_view_item || f.opts.hidden.contains(&a.id) {
            return true;
        }
        match self.bounds_of(f.symbols, a) {
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

    pub(crate) fn draw_arc(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>) {
        if self.skipped(f, a) {
            return;
        }
        // An image masked by one white shape painted straight into the shape, as an image in a
        // clip group is (envelopes with Preserve Shape Using Transparency cut distorted images into
        // many such pieces).
        if let Some(m) = a.extras().mask.as_deref()
            && !f.opts.outline
            && !self.is_shape(a)
            && let NodeKind::Image(im) = &a.kind
            && white_shape_mask(m)
            && let Some(region) = self.clip_of(&m.art)
            && shape_masked_image(a, im, &region.0)
        {
            self.draw_image_in(ctx, f, im, Some(&region), images::image_tint(a, f.ink), a.extras().pixelated);
            self.stats.drawn += 1;
            return;
        }
        if let Some(m) = a.extras().mask.as_deref()
            && !m.disabled
            && !f.opts.outline
            && !self.is_shape(a)
        {
            // A masked object is a non-isolated group of its own, drawn over its own pixels: the
            // mask is rendered over those only.
            let bounds = self.bounds_of(f.symbols, a);
            let Some((r, sub)) = group::area(ctx, f, bounds) else { return };
            let mask = Some(self.mask_values(&sub, m, r.w, r.h));
            let blends = a.blend != vectorcraft_color::BlendMode::Normal || (!a.isolate && self.blends_through(a));
            self.group(ctx, f, Composite { mask, blends, bounds, ..Default::default() }, &mut |r, c, fr| r.draw_arc_body(c, fr, a));
            return;
        }
        self.draw_arc_body(ctx, f, a);
    }

    /// The coverage of opacity mask `m` per output pixel (row-major): its art rendered offscreen,
    /// as luminance ([`mask_value`]).
    pub(super) fn mask_values(&mut self, f: &Frame, m: &vectorcraft_doc::OpacityMask, w: u16, h: u16) -> Vec<u8> {
        let mut mctx = f.offscreen_context(w, h);
        // Mask art is a picture of its own: it takes no part in a knockout group around the object,
        // and its luminance is that of its screen colours.
        let outer = (
            std::mem::take(&mut self.knockout),
            std::mem::take(&mut self.nested),
            self.backdrop.take(),
            std::mem::take(&mut self.clip_paths),
            self.picture.take(),
        );
        self.draw_kept_node(&mut mctx, &Frame { mt: false, ink: Ink::Display, ..*f }, &m.art);
        let pm = self.finish(&mut mctx, self.raster);
        (self.knockout, self.nested, self.backdrop, self.clip_paths, self.picture) = outer;
        pm.data().iter().map(|p| mask_value(p.r, p.g, p.b, p.a, m.clip, m.invert)).collect()
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
            && let Some(bp) = self.path_of(f.symbols, a)
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
            let t = &*f.doc.inline_resolved(t);
            let (g, kept) = match f.text_snap(t) {
                Some(xf) => (Arc::new(text_geom_snapped(t, Some(xf))), false),
                None => self.text_geom_of(a, t),
            };
            self.draw_text_geom(ctx, f, a, t, &g, kept);
            self.stats.drawn += 1;
            return;
        }
        if vectorcraft_doc::live::is_live(a) || effects::has_pathfinder(a) {
            return self.draw_live(ctx, f, a);
        }
        // Its live-effect geometry depends on the node alone, and for plug-in effects on the
        // plug-ins installed (`check_plugins`), so it can be kept across frames.
        let outer = self.fx_owner.replace(a.clone());
        self.draw_node(ctx, f, a, true);
        self.fx_owner = outer;
    }

    pub(crate) fn draw_node(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, force: bool) {
        if !force && (!n.visible || f.opts.hidden.contains(&n.id)) {
            return;
        }
        // A cancelled export stops drawing; whoever asked for the picture drops it (see `cancel`).
        if vectorcraft_doc::cancel::is_cancelled() {
            return;
        }
        if force {
            // Bounds were already checked by draw_arc (or the caller wants it drawn regardless).
        } else if let Some(b) = fx::cull_bounds(f.symbols, n) {
            let pad = f.px * 2.0;
            if !rects_overlap(b.inflate(pad, pad), f.visible) {
                self.stats.culled += n.count();
                return;
            }
        } else if !n.is_container() {
            return;
        }
        // Layer Options on screen: a layer whose Preview is off draws in outline, and Dim Images
        // fades its images, a template's too (exports and thumbnails, which leave templates out,
        // ignore both).
        if let NodeKind::Layer { preview, dim_images, template, .. } = &n.kind
            && f.opts.dim_templates
            && !f.opts.skip_templates
        {
            // A template layer stays in Preview while the view is in Outline (and whatever its own
            // Preview option says): its picture is there to be traced (#1486).
            if *template && f.opts.outline {
                let opts = RenderOptions { outline: false, ..f.opts.clone() };
                return self.draw_node(ctx, &Frame { opts: &opts, ..*f }, n, force);
            }
            if !preview && !template && !f.opts.outline {
                let opts = RenderOptions { outline: true, ..f.opts.clone() };
                return self.draw_node(ctx, &Frame { opts: &opts, ..*f }, n, force);
            }
            if let Some(p) = *dim_images {
                let outer = self.dim_images;
                let dim = f32::from(p).clamp(0.0, 100.0) / 100.0;
                self.dim_images = Some(outer.map_or(dim, |o| o.min(dim)));
                self.draw_layer_node(ctx, f, n);
                self.dim_images = outer;
                return;
            }
        }
        self.draw_layer_node(ctx, f, n);
    }

    /// [`Self::draw_node`] after culling and Layer Options.
    fn draw_layer_node(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node) {
        // Isolation mode: the art around the isolated container draws dimmed (#833).
        if let Some((isolated, path)) = self.isolation_path(f) {
            // A container on the way down to it draws as usual: its members decide.
            if path.contains(&n.id) && n.id != isolated {
                return self.draw_layer_node_now(ctx, f, n);
            }
            self.isolation_settled = true;
            if n.id == isolated {
                self.draw_layer_node_now(ctx, f, n);
            } else {
                self.dimmed(ctx, f, &mut |r, c, fr| r.draw_layer_node_now(c, fr, n));
            }
            self.isolation_settled = false;
            return;
        }
        self.draw_layer_node_now(ctx, f, n);
    }

    /// In isolation mode, while drawing outside the isolated container and the art dimmed around
    /// it: the isolated container and its layers and groups, from the top layer down to it.
    pub(crate) fn isolation_path(&mut self, f: &Frame) -> Option<(NodeId, Vec<NodeId>)> {
        // Opacity-mask editing isolates the temporary layer for tools; that layer is not painted on
        // the page (the mask applies on the object), so canvas dimming would wash out everything.
        let isolated = f.opts.isolated.filter(|_| f.doc.mask_edit.is_none() && !self.isolation_settled)?;
        let doc = std::ptr::from_ref(f.doc) as usize;
        if self.isolation.as_ref().is_none_or(|(stamp, d, id, _)| (*stamp, *d, *id) != (self.stamp, doc, isolated)) {
            self.isolation = Some((self.stamp, doc, isolated, f.doc.ancestry(isolated).unwrap_or_default()));
        }
        let path = self.isolation.as_ref().map(|(.., path)| path.clone()).filter(|p| !p.is_empty())?;
        Some((isolated, path))
    }

    /// `draw` at [`ISOLATION_DIM`], with nothing inside dimmed again.
    fn dimmed(&mut self, ctx: &mut RenderContext, f: &Frame, draw: &mut group::Content) {
        let settled = std::mem::replace(&mut self.isolation_settled, true);
        let comp = Composite { opacity: ISOLATION_DIM, ..Default::default() };
        self.group(ctx, f, comp, draw);
        self.isolation_settled = settled;
    }

    /// [`Self::draw_layer_node`] past isolation mode's dimming.
    fn draw_layer_node_now(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node) {
        if fx::has_object_fx(n) {
            return self.draw_object_fx(ctx, f, &Arc::new(n.clone()), false);
        }
        let outline = f.opts.outline;
        let opacity = self.opacity_of(n);
        // Whether this container's children knock each other out.
        let knockout = !outline && n.knocks_out(self.knockout);
        let enclosing = std::mem::replace(&mut self.knockout, knockout);
        if !outline && (opacity < 1.0 || n.blend != vectorcraft_color::BlendMode::Normal || n.isolate || knockout) {
            let blends = self.blends_through(n);
            let bounds = if blends { self.node_bounds(f.symbols, n) } else { None };
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
            NodeKind::Group { children, clip: false } if f.opts.trace_views && !f.opts.outline && Self::trace_view(f, n) != TraceView::Result => {
                self.draw_trace_view(ctx, f, children, Self::trace_view(f, n), knockout)
            }
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
                self.draw_children(ctx, f, children, knockout)
            }
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } if f.opts.outline => {
                for c in children {
                    self.draw_outline_member(ctx, f, c);
                }
            }
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                // Nothing to clip by hides the clipped art (as in the SVG and PDF output). A clip
                // group doesn't isolate: blending inside it reaches the art below. The clipping
                // path's fill paints behind the clipped art and its stroke over it, unclipped.
                if let Some((clip, rest)) = children.split_first()
                    && let Some(region) = self.clip_of(clip)
                {
                    let (fill, stroke) = self.clip_paint_of(clip);
                    // An image inside its own frame painted straight into the clipping path, without
                    // a clip layer (envelopes cut distorted images into many such pieces).
                    if let ([image], None, None) = (rest, &fill, &stroke)
                        && let NodeKind::Image(im) = &image.kind
                        && plain_image(image, im, &region.0)
                    {
                        self.draw_image_in(ctx, f, im, Some(&region), images::image_tint(image, f.ink), image.extras().pixelated);
                        self.stats.drawn += 1;
                        return;
                    }
                    let blends = self.blends_through(n);
                    let bounds = if blends { Some(region.0.bounding_box()) } else { None };
                    let comp = Composite { clip: Some((&region.0, region.1)), blends, bounds, ..Default::default() };
                    self.group(ctx, f, comp, &mut |r, c, fr| {
                        if let Some(fill) = &fill {
                            r.draw_arc(c, fr, fill);
                        }
                        r.draw_children(c, fr, rest, knockout)
                    });
                    if let Some(stroke) = &stroke {
                        self.draw_arc(ctx, f, stroke);
                    }
                }
            }
            NodeKind::Path { path, rule, guide, .. } => {
                let bp = self.outline_path(f, n).unwrap_or_else(|| Arc::new(path.to_bezpath()));
                if *guide {
                    let ([r, g, b], dots) = f.opts.guide;
                    let mut screen = BezPath::clone(&bp);
                    screen.apply_affine(f.view);
                    // The rasterizer makes every dash of the whole path before clipping: a guide
                    // running far off screen (from a file) asked for millions of them. Past
                    // [`MAX_GUIDE_DASHES`] (by its control polygon, which is at least as long) it
                    // is drawn solid.
                    let h = self.hairline;
                    if dots && polygon_length(&screen) / (4.0 * h) <= MAX_GUIDE_DASHES {
                        ctx.set_transform(Affine::IDENTITY);
                        ctx.set_stroke(kurbo::Stroke::new(1.5 * h).with_dashes(0.0, [1.5 * h, 2.5 * h]));
                        ctx.set_paint(f.ink.fixed([r, g, b, 255]));
                        ctx.stroke_path(&screen);
                    } else {
                        self.hairline(ctx, f, &bp, [r, g, b, 255]);
                    }
                } else {
                    self.draw_shape(ctx, f, n, &bp, *rule);
                }
            }
            NodeKind::Compound { children, rule } => {
                if let Some(bp) = self.outline_path(f, n) {
                    return self.draw_shape(ctx, f, n, &bp, *rule);
                }
                let mut bp = BezPath::new();
                for c in children {
                    if let Some(p) = c.path_data() {
                        bp.extend(p.to_bezpath());
                    }
                }
                self.draw_shape(ctx, f, n, &bp, *rule);
            }
            NodeKind::Text(t) => self.draw_text(ctx, f, n, t),
            NodeKind::Image(im) => self.draw_image_in(ctx, f, im, None, images::image_tint(n, f.ink), n.extras().pixelated),
            NodeKind::SymbolInstance { symbol, xf } => {
                if let Some(sym) = f.doc.symbols.iter().find(|s| &s.name == symbol)
                    && self.charge_symbol(&sym.art)
                {
                    let (art, kept) = self.placed_art(&sym.art, *xf, false, Some(n));
                    self.draw_placed_art(ctx, f, &art, kept);
                }
            }
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Intertwine { .. } => match self.live_root.clone().filter(|a| std::ptr::eq(&**a, n)) {
                Some(a) => self.draw_live_body(ctx, f, &a, true),
                None => self.draw_live_node(ctx, f, n),
            },
        }
    }

    /// Count symbol art `art`'s objects against the frame's [`vectorcraft_doc::MAX_SYMBOL_NODES`]: `false` (the
    /// instance isn't drawn) once they would pass it.
    fn charge_symbol(&mut self, art: &Node) -> bool {
        let total = self.symbol_nodes.saturating_add(art.count() as u64);
        (total <= vectorcraft_doc::MAX_SYMBOL_NODES).then(|| self.symbol_nodes = total).is_some()
    }

    /// Draw symbol art `art` as [`Self::placed_art`] placed it for an instance or an inline graphic
    /// (`kept`: kept between frames, see [`Self::draw_kept_node`]).
    pub(super) fn draw_placed_art(&mut self, ctx: &mut RenderContext, f: &Frame, art: &Arc<Node>, kept: bool) {
        let outer = std::mem::replace(&mut self.made_again, !kept);
        if kept {
            self.draw_kept_node(ctx, f, art);
        } else {
            self.draw_node(ctx, f, art, true);
        }
        self.made_again = outer;
    }

    /// [`Self::draw_node`] of `a` (drawn whatever its visibility and bounds), whose `Arc` stays the
    /// same from frame to frame while the art does. When `a` is a live object,
    /// [`Self::draw_content`] draws its evaluation from the live cache ([`Self::live_root`]).
    pub(crate) fn draw_kept_node(&mut self, ctx: &mut RenderContext, f: &Frame, a: &Arc<Node>) {
        let live = vectorcraft_doc::live::is_live(a).then(|| a.clone());
        let outer = std::mem::replace(&mut self.live_root, live);
        self.draw_node(ctx, f, a, true);
        self.live_root = outer;
    }

    /// The View `n` draws with on screen: Source Image when it is an Image Trace object listed in
    /// [`RenderOptions::trace_source`], else its own.
    fn trace_view(f: &Frame, n: &Node) -> TraceView {
        match &f.opts.trace_source {
            Some(ids) if n.extras().trace.is_some() && ids.contains(&n.id) => TraceView::Source,
            _ => n.trace_view(),
        }
    }

    /// An Image Trace object's `children` (the hidden source image, then the traced shapes) as
    /// `view` shows them on screen.
    fn draw_trace_view(&mut self, ctx: &mut RenderContext, f: &Frame, children: &[Arc<Node>], view: TraceView, knockout: bool) {
        let Some((source, shapes)) = children.split_first() else { return };
        if view.shows_source() && matches!(source.kind, NodeKind::Image(_)) {
            self.draw_node(ctx, f, source, true);
        }
        if view.shows_result() {
            self.draw_children(ctx, f, shapes, knockout);
        }
        if view.shows_outlines() {
            let opts = RenderOptions { outline: true, ..f.opts.clone() };
            let frame = Frame { opts: &opts, ..*f };
            for c in shapes {
                self.draw_outline_member(ctx, &frame, c);
            }
        }
    }

    /// [`Self::draw_node`] for `c`, a member of a clip group or a shape of an Image Trace object in
    /// Outline view, culled by its cached bounds ([`Self::bounds_of`]) and drawn with `c` as
    /// `fx_owner`, so its path, its live-effect geometry and, through [`Self::draw_kept_node`], a
    /// live object's evaluation come from the caches.
    fn draw_outline_member(&mut self, ctx: &mut RenderContext, f: &Frame, c: &Arc<Node>) {
        if !c.visible || f.opts.hidden.contains(&c.id) || f.opts.hide_guides && matches!(c.kind, NodeKind::Path { guide: true, .. }) {
            return;
        }
        match self.bounds_of(f.symbols, c) {
            Some(b) => {
                let pad = f.px * 2.0;
                if !rects_overlap(b.inflate(pad, pad), f.visible) {
                    self.stats.culled += c.count();
                    return;
                }
            }
            None if !c.is_container() => return,
            None => {}
        }
        let outer = self.fx_owner.replace(c.clone());
        self.draw_kept_node(ctx, f, c);
        self.fx_owner = outer;
    }

    /// The children of a group: as the elements of a knockout group when `knockout`.
    pub(super) fn draw_children(&mut self, ctx: &mut RenderContext, f: &Frame, children: &[Arc<Node>], knockout: bool) {
        if knockout {
            self.draw_knockout(ctx, f, children);
        } else if let Some((_, path)) = self.isolation_path(f) {
            // Isolation mode, in a container on the way down to the isolated one: each run of
            // members around it draws dimmed as one (#833).
            let mut rest = children;
            while let Some(i) = rest.iter().position(|c| path.contains(&c.id)) {
                let (around, from) = rest.split_at(i);
                if !around.is_empty() {
                    self.dimmed(ctx, f, &mut |r, c, fr| around.iter().for_each(|n| r.draw_arc(c, fr, n)));
                }
                let Some((on_path, after)) = from.split_first() else { break };
                self.draw_arc(ctx, f, on_path);
                rest = after;
            }
            if !rest.is_empty() {
                self.dimmed(ctx, f, &mut |r, c, fr| rest.iter().for_each(|n| r.draw_arc(c, fr, n)));
            }
        } else {
            for c in children {
                self.draw_arc(ctx, f, c);
            }
        }
    }

    /// Cull bounds of `n` (symbol instances measured by their art box from `symbols`): cached when
    /// it is a node of the tree being drawn.
    fn node_bounds(&mut self, symbols: &SymbolBoxes, n: &Node) -> Option<Rect> {
        match self.geom.get(&(n as *const Node as usize)) {
            // The entry keeps its node alive, so the address is that node's.
            Some(e) if std::ptr::eq(Arc::as_ptr(&e.node), n) => e.bounds,
            _ => fx::cull_bounds(symbols, n),
        }
    }

    /// The children of a knockout group ([`Node::knockout_elements`]). Each element first erases
    /// what the elements below it drew wherever it paints (its knockout shape: its coverage at full
    /// object opacity without its opacity mask, or as drawn when its opacity and mask define the
    /// knockout shape), then adds itself: it composites against the group's backdrop instead of
    /// over the elements below it (the art below the group when the group is non-isolated and
    /// drawn over a copy of it, see [`Self::group`] and [`Self::knockout_over`]).
    pub(crate) fn draw_knockout(&mut self, ctx: &mut RenderContext, f: &Frame, children: &[Arc<Node>]) {
        let backdrop = self.backdrop.take();
        for c in Node::knockout_elements(children) {
            if self.skipped(f, c) {
                continue;
            }
            self.knockout_pass(ctx, f, c, Compose::DestOut);
            match &backdrop {
                Some(b) => self.knockout_over(ctx, f, c, b),
                None => self.knockout_pass(ctx, f, c, Compose::Plus),
            }
        }
        self.backdrop = backdrop;
    }

    /// Whether `n` is the knockout element being drawn as its shape (see [`Self::draw_knockout`]).
    pub(super) fn is_shape(&self, n: &Node) -> bool {
        self.shape_of == n as *const Node as usize
    }
}
