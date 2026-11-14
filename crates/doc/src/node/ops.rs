use super::extras::Extras;
use super::ids::{LayerColor, NodeId};
use super::kind::NodeKind;
use super::live_shape::{Scaling, moves_only};
use super::opacity::Knockout;
use super::{Affine, Appearance, Arc, BlendMode, EnvelopeKind, FillRule, Node, PathData, Point, Rect, RepeatSpec, SYMBOL_SQUARE, Shape, SymbolBoxes};

impl Node {
    pub fn new(id: NodeId, kind: NodeKind) -> Self {
        Self {
            id,
            name: None,
            visible: true,
            locked: false,
            opacity: 1.0,
            blend: BlendMode::Normal,
            isolate: false,
            knockout: Knockout::Neutral,
            knockout_shape: false,
            appearance: Appearance::default().into(),
            kind,
            bbox_angle: 0.0,
            extras: Extras::default(),
        }
    }

    pub fn path(id: NodeId, path: PathData, appearance: Appearance) -> Self {
        let mut n = Self::new(id, NodeKind::Path { path, rule: FillRule::NonZero, live: None, clipping: false, guide: false });
        n.appearance = appearance.into();
        n
    }

    pub fn group(id: NodeId, children: Vec<Arc<Node>>) -> Self {
        Self::new(id, NodeKind::Group { children, clip: false })
    }

    pub fn layer(id: NodeId, name: &str, color: LayerColor) -> Self {
        let mut n = Self::new(
            id,
            NodeKind::Layer { color, template: false, printable: true, children: vec![], clip: false, preview: true, dim_images: None },
        );
        n.name = Some(name.to_string());
        n
    }

    pub fn children(&self) -> Option<&Vec<Arc<Node>>> {
        match &self.kind {
            NodeKind::Layer { children, .. }
            | NodeKind::Group { children, .. }
            | NodeKind::Compound { children, .. }
            | NodeKind::Blend { children, .. }
            | NodeKind::Envelope { content: children, .. }
            | NodeKind::Repeat(RepeatSpec { source: children, .. })
            | NodeKind::Intertwine { children } => Some(children),
            _ => None,
        }
    }

    pub fn children_mut(&mut self) -> Option<&mut Vec<Arc<Node>>> {
        match &mut self.kind {
            NodeKind::Layer { children, .. }
            | NodeKind::Group { children, .. }
            | NodeKind::Compound { children, .. }
            | NodeKind::Blend { children, .. }
            | NodeKind::Envelope { content: children, .. }
            | NodeKind::Repeat(RepeatSpec { source: children, .. })
            | NodeKind::Intertwine { children } => Some(children),
            _ => None,
        }
    }

    /// Does this object or layer move with its artboard? Locked and hidden ones only with
    /// `locked_and_hidden` (Selection & Anchor Display › Move Locked and Hidden Artwork with
    /// Artboard).
    pub fn rides_with_artboard(&self, locked_and_hidden: bool) -> bool {
        locked_and_hidden || (!self.locked && self.visible)
    }

    pub fn is_layer(&self) -> bool {
        matches!(self.kind, NodeKind::Layer { .. })
    }

    /// A ruler guide ([`NodeKind::RulerGuide`]).
    pub fn is_ruler_guide(&self) -> bool {
        matches!(self.kind, NodeKind::RulerGuide { .. })
    }

    /// A guide of either kind: a ruler guide, or a path made into a guide.
    pub fn is_guide(&self) -> bool {
        matches!(self.kind, NodeKind::RulerGuide { .. } | NodeKind::Path { guide: true, .. })
    }

    /// A template layer (Layer Options → Template).
    pub fn is_template(&self) -> bool {
        matches!(self.kind, NodeKind::Layer { template: true, .. })
    }

    /// The art objects of this container: its children, looking through sublayers, which are not
    /// objects (bottom first). With `editable`, only the visible, unlocked objects in visible,
    /// unlocked, non-template sublayers.
    pub fn layer_art(&self, editable: bool) -> Vec<NodeId> {
        fn visit(n: &Node, editable: bool, out: &mut Vec<NodeId>) {
            for c in n.children().into_iter().flatten() {
                if c.is_ruler_guide() || (editable && (!c.visible || c.locked || c.is_template())) {
                    continue;
                }
                if c.is_layer() {
                    visit(c, editable, out);
                } else {
                    out.push(c.id);
                }
            }
        }
        let mut out = vec![];
        visit(self, editable, &mut out);
        out
    }

    pub fn is_container(&self) -> bool {
        self.children().is_some()
    }

    /// Full opacity, Normal blending, no isolation, knockout or opacity mask (the Layers panel
    /// fills an object's target circle otherwise).
    pub fn has_default_transparency(&self) -> bool {
        self.opacity >= 1.0
            && self.blend == BlendMode::Normal
            && !self.isolate
            && self.knockout == Knockout::Neutral
            && !self.knockout_shape
            && self.extras().mask.is_none()
    }

    /// Kind name as the Layers panel / Properties panel shows it.
    pub fn kind_label(&self) -> &'static str {
        match &self.kind {
            NodeKind::Layer { .. } => "Layer",
            NodeKind::Group { clip: true, .. } => "Clip Group",
            NodeKind::Group { .. } if self.extras().objects_on_path.is_some() => "Objects on Path",
            NodeKind::Group { .. } if self.extras().compound_shape => "Compound Shape",
            NodeKind::Group { .. } => "Group",
            NodeKind::Path { live: Some(l), .. } => l.label(),
            NodeKind::Path { guide: true, .. } => "Guide",
            NodeKind::Path { .. } => "Path",
            NodeKind::Compound { .. } => "Compound Path",
            NodeKind::Text(_) => "Type",
            NodeKind::Image(_) => "Image",
            NodeKind::SymbolInstance { .. } => "Symbol",
            NodeKind::Blend { .. } => "Blend",
            NodeKind::Envelope { .. } => "Envelope",
            NodeKind::Mesh(_) => "Mesh",
            NodeKind::Repeat(r) => r.kind.label(),
            NodeKind::PlacedDocument(_) => "Placed Document",
            NodeKind::Intertwine { .. } => "Intertwine",
            NodeKind::RulerGuide { .. } => "Guide",
        }
    }

    /// Name shown in the Layers panel: explicit name or `<Kind>`.
    pub fn display_name(&self) -> String {
        if let Some(n) = &self.name {
            return n.clone();
        }
        match &self.kind {
            NodeKind::Text(t) => {
                let s: String = t.plain_text().chars().take(32).collect();
                if s.is_empty() { "<Text>".into() } else { s }
            }
            _ => format!("<{}>", self.kind_label()),
        }
    }

    /// [`Self::display_name`], or its XML id when Preferences › Units › Identify Objects By is XML ID.
    pub fn identify_label(&self, xml_id: bool) -> String {
        let name = self.display_name();
        if xml_id { crate::xml_id::from_object_name(&name) } else { name }
    }

    /// Path data for path nodes.
    pub fn path_data(&self) -> Option<&PathData> {
        match &self.kind {
            NodeKind::Path { path, .. } => Some(path),
            _ => None,
        }
    }

    pub fn path_data_mut(&mut self) -> Option<&mut PathData> {
        match &mut self.kind {
            NodeKind::Path { path, .. } => Some(path),
            _ => None,
        }
    }

    /// The path a path's or compound path's own strokes follow (a compound's members' together).
    pub fn stroke_path(&self) -> Option<vectorcraft_geom::BezPath> {
        match &self.kind {
            NodeKind::Path { path, .. } => Some(path.to_bezpath()),
            NodeKind::Compound { children, .. } => {
                // Sized up front: grown by doubling, the path of a compound path of hundreds of
                // thousands of anchors is copied into ever larger buffers, up to tens of MB, on
                // every call.
                let subpaths = || children.iter().filter_map(|c| c.path_data()).flat_map(|pd| &pd.subpaths);
                let elements = subpaths().filter(|sp| !sp.anchors.is_empty()).map(|sp| 1 + sp.segment_count() + usize::from(sp.closed)).sum();
                let mut bp = vectorcraft_geom::BezPath::from_vec(Vec::with_capacity(elements));
                subpaths().for_each(|sp| sp.to_bezpath_into(&mut bp));
                Some(bp)
            }
            _ => None,
        }
    }

    /// A quick box round everything the object can paint, for culling clicks: paths and compound
    /// paths grow their geometric bounds by the farthest their strokes can reach from any path
    /// ([`Appearance::outset`]) instead of measuring along their shape. An image's box takes in
    /// the shapes its own fills and strokes paint ([`crate::hit::set_image_paint`]). Groups, layers
    /// without a clipping mask and Intertwine groups take in their members' boxes, grown by their
    /// own strokes. Symbol instances are measured as in [`Node::visual_bounds`]. A blend's box
    /// takes in where its spine can put its keys and steps.
    pub fn reach_bounds(&self) -> Option<Rect> {
        self.reach(None)
    }

    /// [`Node::reach_bounds`] with symbol instances measured by their symbol's art box from
    /// `symbols`. An instance that only moves its art is measured as the art's visual box moved,
    /// and any other instance is measured as its geometric box ([`Node::geometric_bounds_with`])
    /// grown by the farthest a stroke in the art can reach. For an instance placed through 9-slice
    /// guides, that box also holds the art box placed by each cell's map
    /// ([`crate::NineSliceMap::placement_bounds`]). An instance whose art holds an instance of a
    /// symbol with 9-slice guides at any depth is measured as its art placed by its transform.
    pub fn reach_bounds_with(&self, symbols: &SymbolBoxes) -> Option<Rect> {
        symbols.quickly(|| self.reach(Some(symbols)))
    }

    /// The box of [`Node::reach_bounds`], or of [`Node::reach_bounds_with`] when `symbols` is given.
    pub(crate) fn reach(&self, symbols: Option<&SymbolBoxes>) -> Option<Rect> {
        match &self.kind {
            NodeKind::Path { .. } | NodeKind::Compound { .. } => {
                let o = self.appearance.outset();
                self.geometric(symbols).map(|b| b.inflate(o, o))
            }
            // The members' reach, grown by the container's own strokes.
            NodeKind::Layer { children, clip: false, .. } | NodeKind::Group { children, clip: false } => {
                let o = self.appearance.outset();
                children
                    .iter()
                    .skip(usize::from(self.extras().shaper.is_some()))
                    .fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.reach(symbols)))
                    .map(|b| b.inflate(o, o))
            }
            NodeKind::Intertwine { children } => children.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.reach(symbols))),
            // The frame and each shape the image's own fills and strokes paint, a stroke's grown by
            // the farthest it can reach from its shape.
            NodeKind::Image(_) => {
                crate::hit::image_paint(symbols.map(SymbolBoxes::document), self).iter().fold(self.geometric(symbols), |acc, (bp, _, item)| {
                    let o = if let crate::AppearanceItem::Stroke(s) = item { s.reach() } else { 0.0 };
                    vectorcraft_geom::union_opt(acc, (!bp.elements().is_empty()).then(|| bp.bounding_box().inflate(o, o)))
                })
            }
            // Keys and steps placed on a spine, turned to it with Align to Path, can reach past the
            // keys and the spine.
            NodeKind::Blend { children, spec } => {
                vectorcraft_geom::union_opt(self.visual(symbols), crate::blend::rail_reach(children, spec, symbols))
            }
            _ => self.visual(symbols),
        }
    }

    /// Geometric bounds (no stroke), recursively. Clip groups are bounded by their clip path. A
    /// symbol instance is measured as its transform applied to the 20 × 20 square around the origin
    /// that symbols made in the app keep their art in ([`Node::geometric_bounds_with`] measures
    /// its art).
    pub fn geometric_bounds(&self) -> Option<Rect> {
        self.geometric(None)
    }

    /// [`Node::geometric_bounds`] with each symbol instance measured as its transform applied to
    /// its symbol's art box from `symbols`. An instance is measured as the square when its symbol
    /// is missing, when its symbol or a symbol nested in its art holds an instance of itself, and
    /// when symbols nest 32 deep in its art.
    pub fn geometric_bounds_with(&self, symbols: &SymbolBoxes) -> Option<Rect> {
        self.geometric(Some(symbols))
    }

    /// Children that contribute to a container's editing bounds and selection highlights. A live trace retains a
    /// hidden bitmap for re-tracing/release; only source-image views measure that bitmap.
    pub fn editing_children(&self) -> &[Arc<Node>] {
        let children = self.children().map(Vec::as_slice).unwrap_or_default();
        let trace_source = self.extras().trace.is_some()
            && !self.trace_view().shows_source()
            && children.first().is_some_and(|n| !n.visible && matches!(n.kind, NodeKind::Image(_)));
        let skip = usize::from(self.extras().shaper.is_some() || trace_source);
        children.get(skip..).unwrap_or_default()
    }

    /// The box of [`Node::geometric_bounds`], or of [`Node::geometric_bounds_with`] when `symbols`
    /// is given.
    pub(crate) fn geometric(&self, symbols: Option<&SymbolBoxes>) -> Option<Rect> {
        let union = |nodes: &[Arc<Node>]| nodes.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.geometric(symbols)));
        match &self.kind {
            NodeKind::Path { path, .. } => path.bounds(),
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                children.first().and_then(|c| c.geometric(symbols))
            }
            NodeKind::Layer { .. } | NodeKind::Group { .. } | NodeKind::Compound { .. } => self
                .editing_children()
                .iter()
                .filter(|c| c.visible || !matches!(self.kind, NodeKind::Layer { .. }))
                .fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.geometric(symbols))),
            NodeKind::Text(t) => self.projected(t.bounds()),
            NodeKind::Image(im) => Some(im.xf.transform_rect_bbox(Rect::new(0.0, 0.0, im.width as f64, im.height as f64))),
            NodeKind::SymbolInstance { symbol, xf } => {
                let art = symbols.and_then(|s| s.art(symbol)).map_or(SYMBOL_SQUARE, |a| a.geometric);
                self.projected(Some(xf.transform_rect_bbox(art)))
            }
            NodeKind::Blend { children, spec } => vectorcraft_geom::union_opt(union(children), spec.spine.as_ref().and_then(|s| s.bounds())),
            NodeKind::Envelope { content, kind, frame, options, .. } => crate::live::envelope_bounds(content, kind, **frame, *options),
            NodeKind::Mesh(m) => m.bounds(),
            // Each copy is measured where the repeat draws it: `RepeatSpec::transforms` lays the
            // copies out by the source's bounds measured without the document.
            NodeKind::Repeat(r) => r.copies_bounds(union(&r.source)?, symbols.map(|s| s.document())),
            NodeKind::PlacedDocument(p) => Some(p.bounds()),
            // The members drawn over the others are drawn where they are.
            NodeKind::Intertwine { children } => union(children),
            // A ruler guide runs across the canvas (or an artboard): it adds nothing to a box.
            NodeKind::RulerGuide { .. } => None,
        }
    }

    /// Visual bounds: what the object paints, its strokes included (paths and compound paths
    /// measure theirs along their own shape, see [`Appearance::stroked_bounds`]). An image is
    /// measured by its frame. A symbol instance is measured as in [`Node::geometric_bounds`],
    /// grown by its own strokes' reach ([`Node::visual_bounds_with`] measures its art).
    pub fn visual_bounds(&self) -> Option<Rect> {
        self.visual(None)
    }

    /// [`Node::visual_bounds`] with symbol instances measured by their symbol's art from
    /// `symbols`, the strokes in it included. An instance that only moves its art is measured as
    /// the art's visual box moved. Any other instance is measured as its art transformed by it with
    /// the strokes at their own weight, as the renderer draws it, and never smaller than its
    /// geometric box ([`Node::geometric_bounds_with`]). Instances inside symbol art are measured as
    /// in [`Node::reach_bounds_with`]. In boxes made by [`SymbolBoxes::with_brush_art`], the box of
    /// an instance whose art holds brushed strokes grows by as far as their brush art can paint. In
    /// boxes made by [`SymbolBoxes::without_art_strokes`], an instance is measured as its art's
    /// geometric box placed by its transform, grown by its own strokes.
    pub fn visual_bounds_with(&self, symbols: &SymbolBoxes) -> Option<Rect> {
        self.visual(Some(symbols))
    }

    /// The visual box of a group or layer without a clipping path, from the visual boxes of its
    /// [`Node::editing_children`]: for a walk that measures the members as well.
    pub fn visual_of_members(&self, members: impl Iterator<Item = Option<Rect>>) -> Option<Rect> {
        // The container's own strokes paint around its members.
        let o = self.appearance.outset();
        members.fold(None, vectorcraft_geom::union_opt).map(|b| b.inflate(o, o))
    }

    /// The box of [`Node::visual_bounds`], or of [`Node::visual_bounds_with`] when `symbols` is
    /// given.
    pub(crate) fn visual(&self, symbols: Option<&SymbolBoxes>) -> Option<Rect> {
        match &self.kind {
            NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
                // A clipping path that paints (see `clip_paint`) strokes over the clip edge.
                let clip = children.first()?;
                if matches!(clip.kind, NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Text(_)) {
                    clip.visual(symbols)
                } else {
                    clip.geometric(symbols)
                }
            }
            NodeKind::Intertwine { children } => children.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.visual(symbols))),
            NodeKind::Layer { .. } | NodeKind::Group { .. } => self.visual_of_members(self.editing_children().iter().map(|c| c.visual(symbols))),
            NodeKind::Blend { children, spec } => {
                let b = children.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.visual(symbols)));
                let o = crate::live::max_outset(children, symbols);
                vectorcraft_geom::union_opt(b, spec.spine.as_ref().and_then(|s| s.bounds()).map(|r| r.inflate(o, o)))
            }
            NodeKind::Envelope { content, .. } | NodeKind::Repeat(RepeatSpec { source: content, .. }) => {
                let o = crate::live::max_outset(content, symbols);
                self.geometric(symbols).map(|b| b.inflate(o, o))
            }
            NodeKind::Path { .. } | NodeKind::Compound { .. } => {
                let b = self.geometric(symbols)?;
                Some(self.appearance.stroked_bounds(b, || self.stroke_path().unwrap_or_default()))
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                let b = match symbols.and_then(|s| Some((s, s.art(symbol)?))) {
                    Some((s, art)) if !s.art_strokes() => xf.transform_rect_bbox(art.geometric),
                    // A symbol drawn at other than its natural size goes through its 9-slice guides.
                    Some((s, art)) if moves_only(*xf) && s.document().symbol_slice_map_of(symbol, art.geometric, *xf).is_none() => {
                        xf.transform_rect_bbox(art.visual)
                    }
                    Some((s, art)) => {
                        let placed = xf.transform_rect_bbox(art.geometric);
                        match s.placed_visual(symbol, *xf).or_else(|| s.sliced_visual(symbol, *xf)) {
                            Some(v) => v.union(placed),
                            // Art placed through 9-slice guides can draw outside `placed`.
                            None => s
                                .document()
                                .symbol_slice_map_of(symbol, art.geometric, *xf)
                                .map_or(placed, |m| m.placement_bounds(art.geometric).union(placed))
                                .inflate(art.reach, art.reach),
                        }
                    }
                    None => xf.transform_rect_bbox(SYMBOL_SQUARE),
                };
                let o = self.appearance.outset() + symbols.map_or(0.0, |s| s.brush_pad(symbol));
                self.projected(Some(b)).map(|b| b.inflate(o, o))
            }
            NodeKind::Text(t) => {
                // The characters' strokes paint around the glyphs.
                let o = self.appearance.outset().max(t.char_stroke_outset());
                self.geometric(symbols).map(|b| b.inflate(o, o))
            }
            // An image is measured by its frame, without its own fills and strokes.
            NodeKind::Image(_) => self.geometric(symbols),
            _ => {
                let o = self.appearance.outset();
                self.geometric(symbols).map(|b| b.inflate(o, o))
            }
        }
    }

    /// Apply an affine transform to the geometry (and gradients) of this node and its descendants.
    /// `scaling` says what else scales by the transform's mean scale (a bool: stroke weights
    /// only, see [`Scaling`]).
    pub fn transform(&mut self, a: Affine, scaling: impl Into<Scaling>) {
        self.transform_scaled(a, &scaling.into());
    }

    /// The path of the child an Objects on Path group's objects are attached to.
    fn on_path_data(&self) -> Option<&PathData> {
        let r = self.extras().objects_on_path.as_deref()?;
        self.children()?.iter().find(|c| c.id == r.path)?.path_data()
    }

    fn transform_scaled(&mut self, a: Affine, sc: &Scaling) {
        // A dimension's measurement moves with it.
        if let Some(e) = self.extras_if_any()
            && let Some(dimension) = &mut e.dimension
        {
            dimension.transform(a);
        }
        // The path the objects are attached to, before it is transformed.
        let on_path_before = self.on_path_data().cloned();
        if !self.is_layer() {
            self.bbox_angle = crate::orient::transformed_angle(self.bbox_angle, a);
        }
        // Type and symbols in perspective keep looking the same, moved by `a`.
        self.transform_projection(a);
        // Refitting an unplaced gradient only reproduces moves and uniform scales.
        if !keeps_gradient_fit(a) && !(sc.keep_fit_when_flat && flattens(a)) {
            self.pin_gradients();
        }
        let k = Scaling::factor(a);
        if let Some(k) = k
            && sc.strokes
        {
            self.appearance.scale_strokes(k);
            if let Some(f) = sc.effects {
                self.appearance.scale_effects(k, f);
            }
        }
        // Only through `DerefMut` when there is something to change: the appearance is shared
        // copy-on-write (`Shared`), and a move must not copy what it leaves as it is.
        if self.appearance.gradients().next().is_some() {
            self.appearance.transform_gradients(a);
        }
        if self.appearance.has_svg_filter() {
            self.appearance.transform_svg_filters(a);
        }
        if sc.patterns {
            crate::pattern::transform_pattern_paints(self, a);
        }
        match &mut self.kind {
            NodeKind::Path { path, live, .. } => {
                path.transform(a);
                if let Some(l) = live {
                    // A rectangle scaled unevenly keeps circular corners: its path is regenerated.
                    let reshaped = l.transform(a);
                    let kept = sc.keep_corners && k.is_some_and(|k| l.keep_corners(k));
                    if reshaped || kept {
                        *path = l.to_path();
                    }
                }
            }
            NodeKind::Layer { children, .. }
            | NodeKind::Group { children, .. }
            | NodeKind::Compound { children, .. }
            | NodeKind::Intertwine { children } => {
                for c in children.iter_mut() {
                    Arc::make_mut(c).transform_scaled(a, sc);
                }
            }
            NodeKind::Text(t) => {
                t.transform(a);
                // Character strokes scale with the type: undo that when strokes keep their weight.
                if sc.keep_type_strokes
                    && let Some(k) = k.filter(|k| *k > 1e-12)
                {
                    t.scale_char_strokes(1.0 / k);
                }
            }
            NodeKind::Image(im) => im.xf = a * im.xf,
            NodeKind::PlacedDocument(p) => p.xf = a * p.xf,
            NodeKind::SymbolInstance { xf, .. } => *xf = a * *xf,
            NodeKind::Blend { children, spec } => {
                for c in children.iter_mut() {
                    Arc::make_mut(c).transform_scaled(a, sc);
                }
                if let Some(s) = &mut spec.spine {
                    s.transform(a);
                }
            }
            NodeKind::Envelope { content, kind, frame, .. } => {
                for c in content.iter_mut() {
                    Arc::make_mut(c).transform_scaled(a, sc);
                }
                **frame = crate::live::envelope_frame(a * **frame);
                match &mut **kind {
                    EnvelopeKind::Mesh { points, handles, .. } => {
                        for p in points.iter_mut() {
                            *p = a * *p;
                        }
                        // Handles are offsets: they take the linear part.
                        let [m0, m1, m2, m3, _, _] = a.as_coeffs();
                        let lin = Affine::new([m0, m1, m2, m3, 0.0, 0.0]);
                        for h in handles.iter_mut().flatten() {
                            *h = (lin * h.to_point()).to_vec2();
                        }
                    }
                    EnvelopeKind::TopObject { path, .. } => path.transform(a),
                    EnvelopeKind::Warp { .. } => {}
                }
            }
            NodeKind::Mesh(m) => m.transform(a),
            NodeKind::Repeat(r) => r.transform(a, *sc),
            // A ruler guide stays where it is when the art around it (its layer) is transformed:
            // the guide commands move it.
            NodeKind::RulerGuide { .. } => {}
        }
        // The frames of the objects attached to a path move with them, and the distances along the
        // path follow the path as it is now (a live shape's path is regenerated).
        let on_path_after = on_path_before.as_ref().and_then(|_| self.on_path_data().cloned());
        if let Some(e) = self.extras_if_any()
            && let Some(on_path) = &mut e.objects_on_path
        {
            on_path.transform(a, on_path_before.as_ref().zip(on_path_after.as_ref()));
        }
        if let Some(m) = self.extras_if_any().and_then(|e| e.mask.as_mut())
            && m.linked
        {
            Arc::make_mut(&mut m.art).transform_scaled(a, sc);
        }
        // The area where an Intertwine member is drawn over the others moves with it.
        if let Some(e) = self.extras_if_any() {
            if let Some(area) = e.intertwine.as_mut() {
                area.transform(a);
            }
            for area in &mut e.intertwine_tiers {
                area.transform(a);
            }
        }
    }

    /// Visit this node and all descendants depth first (paint order).
    pub fn walk<'a>(&'a self, f: &mut impl FnMut(&'a Node)) {
        f(self);
        if let Some(ch) = self.children() {
            for c in ch {
                c.walk(f);
            }
        }
    }

    /// Count of nodes in this subtree, ruler guides left out.
    pub fn count(&self) -> usize {
        let mut n = 0;
        self.walk(&mut |c| n += usize::from(!c.is_ruler_guide()));
        n
    }

    /// [`Self::count`], with the art of the opacity masks in the subtree (and theirs): what copying
    /// and transforming the subtree makes, as [`Self::walk`] doesn't go into masks.
    pub fn count_with_masks(&self) -> usize {
        let mut n = 0;
        self.walk(&mut |c| {
            n += usize::from(!c.is_ruler_guide());
            if let Some(m) = &c.extras().mask {
                n += m.art.count_with_masks();
            }
        });
        n
    }

    /// Fix this node's unplaced gradients to their fit on its current bounds (before a transform
    /// or warp that refitting wouldn't reproduce). Descendants pin their own when transformed.
    pub fn pin_gradients(&mut self) {
        if self.appearance.has_unplaced_gradient()
            && let Some(b) = self.geometric_bounds()
        {
            self.appearance.pin_gradients(b);
        }
    }

    /// The paint behind the Fill (or Stroke) proxy, the map from its space to the document and
    /// the box (in that space) an unplaced gradient fits. `item` (the Appearance panel's active
    /// item) stands in for the topmost fill or stroke when it is of the proxy's kind. Otherwise
    /// type objects paint their runs (the first run speaks for all) in text space, fitted to the
    /// layout bounds; everything else paints its fill or stroke in the document, fitted to the
    /// geometric bounds. Strokes fit the box grown by half their weight. A colorized image shows
    /// its colorize paint as its fill.
    pub fn proxy_paint(&self, stroke: bool, item: Option<usize>) -> Option<(&vectorcraft_color::Paint, Affine, Rect)> {
        let item = self.appearance.item_of_kind(item, !stroke);
        if item.is_none()
            && !stroke
            && let Some(c) = self.extras().colorize.as_deref()
        {
            return Some((c, Affine::IDENTITY, self.geometric_bounds().unwrap_or(Rect::ZERO)));
        }
        if item.is_none()
            && let NodeKind::Text(t) = &self.kind
        {
            let st = &t.runs.first()?.style;
            let b = t.local_bounds();
            return Some(if stroke { (&st.stroke, t.xf, crate::appearance::stroke_paint_bounds(b, st.stroke_width)) } else { (&st.fill, t.xf, b) });
        }
        // An object without bounds (an empty path) still shows its paints, fitted to an empty box.
        let b = self.geometric_bounds().unwrap_or(Rect::ZERO);
        if stroke {
            self.appearance.stroke_at(item).map(|s| (&s.paint, Affine::IDENTITY, s.paint_bounds(b)))
        } else {
            self.appearance.fill_at(item).map(|f| (&f.paint, Affine::IDENTITY, b))
        }
    }

    /// The paint behind the Fill (or Stroke) proxy, the one [`Node::proxy_paint`] gives, without its
    /// map and box: the object is not measured, so a long path costs no more than a short one.
    pub fn proxy_paint_only(&self, stroke: bool, item: Option<usize>) -> Option<&vectorcraft_color::Paint> {
        let item = self.appearance.item_of_kind(item, !stroke);
        if item.is_none()
            && !stroke
            && let Some(c) = self.extras().colorize.as_deref()
        {
            return Some(c);
        }
        if item.is_none()
            && let NodeKind::Text(t) = &self.kind
        {
            let st = &t.runs.first()?.style;
            return Some(if stroke { &st.stroke } else { &st.fill });
        }
        if stroke { self.appearance.stroke_at(item).map(|s| &s.paint) } else { self.appearance.fill_at(item).map(|f| &f.paint) }
    }

    /// The gradient behind the Fill (or Stroke) proxy and its placement in document coordinates
    /// (see [`Node::proxy_paint`]): what the gradient annotator shows and edits.
    pub fn proxy_gradient(&self, stroke: bool, item: Option<usize>) -> Option<(&vectorcraft_color::GradientPaint, vectorcraft_color::GradientGeom)> {
        let (paint, to_doc, b) = self.proxy_paint(stroke, item)?;
        let vectorcraft_color::Paint::Gradient(g) = paint else { return None };
        let mut geom = g.resolve(b);
        if to_doc != Affine::IDENTITY {
            geom.transform(to_doc, g.gradient.kind);
        }
        Some((g, geom))
    }

    /// The freeform gradient behind the Fill (or Stroke) proxy, its points in document coordinates
    /// (the automatic ones while unplaced), and the length its spreads are fractions of there
    /// (see [`Node::proxy_paint`]): what the Gradient tool's freeform annotator shows and edits.
    pub fn proxy_freeform(&self, stroke: bool, item: Option<usize>) -> Option<(vectorcraft_color::Freeform, f64)> {
        let (paint, to_doc, b) = self.proxy_paint(stroke, item)?;
        let vectorcraft_color::Paint::Gradient(g) = paint else { return None };
        if g.gradient.kind != vectorcraft_color::GradientKind::Freeform {
            return None;
        }
        let mut f = g.freeform_on(b).into_owned();
        let mut scale = vectorcraft_color::freeform::spread_scale(b);
        if to_doc != Affine::IDENTITY {
            f.transform(to_doc);
            scale *= to_doc.determinant().abs().sqrt();
        }
        Some((f, scale))
    }
}

impl Node {
    /// Whether the children of this group or layer knock each other out, inside a group (or page)
    /// where that is `enclosing`.
    pub fn knocks_out(&self, enclosing: bool) -> bool {
        matches!(self.kind, NodeKind::Group { .. } | NodeKind::Layer { .. }) && self.knockout.resolve(enclosing)
    }

    /// A plain group or layer that is neutral to knockout and has no transparency or appearance
    /// of its own: inside a knockout group its children composite as the group's own children.
    pub fn passes_knockout_through(&self) -> bool {
        matches!(self.kind, NodeKind::Group { clip: false, .. } | NodeKind::Layer { template: false, clip: false, .. })
            && self.has_default_transparency()
            && self.appearance.items.is_empty()
            && self.appearance.effects.is_empty()
    }

    /// The visible elements of a knockout group made of `children` (bottom first): neutral plain
    /// groups ([`Self::passes_knockout_through`]) contribute their children instead of themselves.
    pub fn knockout_elements(children: &[Arc<Node>]) -> Vec<&Arc<Node>> {
        fn push<'a>(children: &'a [Arc<Node>], out: &mut Vec<&'a Arc<Node>>) {
            for c in children.iter().filter(|c| c.visible && !c.is_ruler_guide()) {
                match c.children() {
                    Some(ch) if c.passes_knockout_through() => push(ch, out),
                    _ => out.push(c),
                }
            }
        }
        let mut out = Vec::with_capacity(children.len());
        push(children, &mut out);
        out
    }

    /// As a piece of a mesh-warped image ([`crate::NodeExtras::warp_piece`]), the art whose area is its
    /// knockout shape at full coverage: the art of its opacity mask (on and not inverted), or the
    /// clipping path of its clip group. `None` for other objects, whose knockout shape is what they
    /// paint.
    pub fn knockout_outline(&self) -> Option<&Arc<Node>> {
        let e = self.extras();
        if !e.warp_piece {
            return None;
        }
        if let Some(m) = e.mask.as_deref() {
            return (!m.disabled && !m.invert).then_some(&m.art);
        }
        match &self.kind {
            NodeKind::Group { children, clip: true } => children.first(),
            _ => None,
        }
    }

    /// The image a piece of a mesh-warped image draws ([`Self::knockout_outline`]): the piece itself
    /// when an opacity mask cuts it, else the image its clip group clips. `None` for other objects.
    pub fn warp_piece_image(&self) -> Option<&Node> {
        if !self.extras().warp_piece {
            return None;
        }
        match &self.kind {
            NodeKind::Image(_) => Some(self),
            NodeKind::Group { children, clip: true } => children.get(1).map(|c| &**c).filter(|c| matches!(c.kind, NodeKind::Image(_))),
            _ => None,
        }
    }

    /// Whether this is a clip group or a layer with a clipping mask: its first child clips the rest.
    /// A layer whose first child is a ruler guide holds no art, so a guide never clips it.
    pub fn clips(&self) -> bool {
        match &self.kind {
            NodeKind::Group { clip, .. } => *clip,
            NodeKind::Layer { clip, children, .. } => *clip && children.first().is_none_or(|c| !c.is_ruler_guide()),
            _ => false,
        }
    }

    /// Where the ruler guides at the end of `children` (a layer's) begin: the index past the last
    /// child that is not one. Art goes below this index and ruler guides at or above it.
    pub fn guide_run_start(children: &[Arc<Node>]) -> usize {
        children.iter().rposition(|c| !c.is_ruler_guide()).map_or(0, |i| i + 1)
    }

    /// Remove the ruler guides of this layer and of its sublayers. Other nodes hold none. Only
    /// layers that hold one are copied.
    pub fn strip_ruler_guides(&mut self) {
        if let Some(held) = held_guides(self, &Self::is_ruler_guide) {
            strip_held(self, held);
        }
    }

    /// Call `f` on the position (along the axis it crosses) of each ruler guide of this layer and
    /// of its sublayers, with whether the guide is vertical. Only layers that hold one are copied.
    pub fn map_ruler_guide_positions(&mut self, f: &mut impl FnMut(bool, &mut f64)) {
        if let Some(held) = held_guides(self, &Self::is_ruler_guide) {
            map_held(self, held, f);
        }
    }

    /// Whether this layer or one of its sublayers holds a ruler guide.
    pub fn holds_ruler_guides(&self) -> bool {
        self.is_layer() && self.children().is_some_and(|ch| ch.iter().any(|c| c.is_ruler_guide() || c.holds_ruler_guides()))
    }

    /// The index that an insert into `children` at `index` uses for `node`: art stays below the
    /// layer's ruler guides and a ruler guide goes among them.
    pub fn insert_index(children: &[Arc<Node>], index: usize, node: &Node) -> usize {
        let start = Self::guide_run_start(children);
        let i = index.min(children.len());
        if node.is_ruler_guide() { i.max(start) } else { i.min(start) }
    }

    /// Make this group or layer clip its children by its first child, or stop it.
    pub fn set_clips(&mut self, on: bool) {
        if let NodeKind::Group { clip, .. } | NodeKind::Layer { clip, .. } = &mut self.kind {
            *clip = on;
        }
    }

    /// Whether blending inside this object reaches the art below it unless the object isolates
    /// it: a blend mode other than Normal on a child (or, in a leaf, on a fill or stroke),
    /// directly or through children that don't isolate their own blending.
    pub fn blends_through(&self) -> bool {
        self.blends_through_with(&mut |c| c.blends_through())
    }

    /// [`Self::blends_through`], with `inner` answering it for the children (e.g. from a cache).
    pub fn blends_through_with(&self, inner: &mut dyn FnMut(&Arc<Node>) -> bool) -> bool {
        match self.children() {
            Some(ch) if !matches!(self.kind, NodeKind::Compound { .. }) => Self::children_blend(ch, inner),
            _ => self.appearance.items.iter().any(|i| i.visible() && i.blend() != BlendMode::Normal),
        }
    }

    /// [`Self::blends_through`] of a group made of `children` (`inner` answers it for each child).
    pub fn children_blend(children: &[Arc<Node>], inner: &mut dyn FnMut(&Arc<Node>) -> bool) -> bool {
        children.iter().any(|c| c.visible && (c.blend != BlendMode::Normal || (!c.isolate && inner(c))))
    }
}

/// The ruler guides a layer holds: its children that are one, by index in order (each with
/// nothing inside), and its sublayers holding one (with what they hold).
#[derive(Debug, Default)]
pub(crate) struct HeldGuides(Vec<(usize, HeldGuides)>);

/// The ruler guides, by `is_guide`, that `n` holds if it is a layer, or `None` when it holds
/// none. `is_guide` looks at each child of `n` and of its sublayers once.
pub(crate) fn held_guides(n: &Node, is_guide: &impl Fn(&Node) -> bool) -> Option<HeldGuides> {
    if !n.is_layer() {
        return None;
    }
    let held: Vec<_> = n
        .children()
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(i, c)| if is_guide(c) { Some((i, HeldGuides::default())) } else { held_guides(c, is_guide).map(|h| (i, h)) })
        .collect();
    (!held.is_empty()).then_some(HeldGuides(held))
}

/// Remove the ruler guides of `layer` and of its sublayers that [`held_guides`] found (`held`).
/// Only the sublayers holding one are copied, and only those are visited.
pub(crate) fn strip_held(layer: &mut Node, held: HeldGuides) {
    let Some(ch) = layer.children_mut() else { return };
    let mut held = held.0.into_iter().peekable();
    let mut next = 0;
    ch.retain_mut(|c| {
        let at = next;
        next += 1;
        let Some((_, inner)) = held.next_if(|(i, _)| *i == at) else { return true };
        // A sublayer holds something; a ruler guide has nothing inside.
        if inner.0.is_empty() {
            return false;
        }
        strip_held(Arc::make_mut(c), inner);
        true
    });
}

/// Call `f` (whether it is vertical, its position) on each ruler guide of `layer` and of its
/// sublayers that [`held_guides`] found (`held`), in the order they lie in. Only the sublayers
/// holding one, and the guides, are copied, and only those sublayers are visited.
pub(crate) fn map_held(layer: &mut Node, held: HeldGuides, f: &mut impl FnMut(bool, &mut f64)) {
    let Some(ch) = layer.children_mut() else { return };
    for (i, inner) in held.0 {
        let Some(c) = ch.get_mut(i) else { continue };
        if !inner.0.is_empty() {
            map_held(Arc::make_mut(c), inner, f);
        } else if let NodeKind::RulerGuide { vertical, pos, .. } = &mut Arc::make_mut(c).kind {
            f(*vertical, pos);
        }
    }
}

impl Node {
    /// The Appearance panel's row for what this object's own fills and strokes paint around:
    /// "Contents" for a group or layer (its members) and for an image (its pixels), "Characters"
    /// for type; `None` for objects whose fills and strokes paint their own geometry.
    pub fn contents_label(&self) -> Option<&'static str> {
        match self.kind {
            NodeKind::Group { .. } | NodeKind::Layer { .. } | NodeKind::Image(_) => Some("Contents"),
            NodeKind::Text(_) => Some("Characters"),
            _ => None,
        }
    }
}

impl Node {
    /// Whether this object shows any transparency, itself or anything inside it: object, fill or
    /// stroke opacity below 100%, a blend mode, an opacity mask, effects, transparent gradient
    /// stops (characters' fills and strokes included) or a colorized image. Hidden objects show
    /// none.
    pub fn shows_transparency(&self) -> bool {
        self.shows_transparency_with(&|_| true)
    }

    /// [`Self::shows_transparency`], counting a colorized image by itself only when `colorize`
    /// accepts it, for files that write some colorized images in their colorize paint (EPS).
    pub fn shows_transparency_with(&self, colorize: &dyn Fn(&Node) -> bool) -> bool {
        self.visible && (self.own_transparency(colorize) || self.children().is_some_and(|ch| ch.iter().any(|c| c.shows_transparency_with(colorize))))
    }

    /// [`Self::shows_transparency`] of the object alone, what it holds left out.
    pub fn shows_own_transparency(&self) -> bool {
        self.visible && self.own_transparency(&|_| true)
    }

    /// The object's own transparency, as [`Self::shows_transparency_with`] counts it.
    fn own_transparency(&self, colorize: &dyn Fn(&Node) -> bool) -> bool {
        use vectorcraft_color::Paint;
        let see_through = |p: &Paint| matches!(p, Paint::Gradient(g) if g.gradient.stops.iter().any(|s| s.opacity < 1.0));
        let effects = |fx: &[crate::Effect]| fx.iter().any(|e| e.visible);
        self.opacity < 1.0
            || self.blend != BlendMode::Normal
            || self.extras().mask.as_ref().is_some_and(|m| !m.disabled)
            || (matches!(self.kind, NodeKind::Image(_)) && self.extras().colorize.is_some() && colorize(self))
            || effects(&self.appearance.effects)
            || self
                .appearance
                .items
                .iter()
                .any(|i| i.visible() && (i.opacity() < 1.0 || i.blend() != BlendMode::Normal || see_through(i.paint()) || effects(i.effects())))
            // Characters' own fills and strokes, which draw with the object.
            || matches!(&self.kind, NodeKind::Text(t) if t.runs.iter().any(|r| see_through(&r.style.fill) || (r.style.stroke_width > 0.0 && see_through(&r.style.stroke))))
    }
}

/// Is `a` a move plus a positive uniform scale (after which a refit gradient still matches)? The
/// terms are compared within 1e-12 of the scale (at least of 1). A scale of 0 or less, a turn, a
/// reflection and an uneven scale give false, and so does a map that flattens the object
/// ([`flattens`]), which callers that keep a fitted gradient there check apart.
pub fn keeps_gradient_fit(a: Affine) -> bool {
    let [m0, m1, m2, m3, _, _] = a.as_coeffs();
    let eps = 1e-12 * m0.abs().max(1.0);
    m0 > 0.0 && m1.abs() <= eps && m2.abs() <= eps && (m0 - m3).abs() <= eps
}

/// Does `a` flatten what it maps to a line or a point (its determinant 0, within 1e-12 of the
/// squares of its terms), or is it not finite?
pub fn flattens(a: Affine) -> bool {
    let [m0, m1, m2, m3, _, _] = a.as_coeffs();
    let norm = m0 * m0 + m1 * m1 + m2 * m2 + m3 * m3;
    !a.is_finite() || a.determinant().abs() <= 1e-12 * norm
}

impl Node {
    /// Whether a document point lies in this object's filled regions ([`Node::clip_shapes`]; true
    /// everywhere for objects without them, such as type, whose boxes stand in).
    pub fn contains_fn(&self) -> impl Fn(Point) -> bool + use<> {
        let shapes = self.clip_shapes(None);
        move |p| shapes.is_empty() || shapes.iter().any(|(bp, rule)| vectorcraft_geom::hit::fill_contains(bp, *rule, p))
    }
}
