use super::cid::{CidText, FontInfo, comparable, font_chars, identify_glyph, matching_face, replacement, unify_radical};
use super::entry::{
    MAX_NESTED, OVERPRINT_UNREAD, OVERPRINT_ZERO_MODE, PATTERN_REUSE, TEXT_OUTLINES, TOO_MANY_OBJECTS, UNMATCHED_GLYPHS_NOTE,
    UNNAMED_CHARACTERS_NOTE, max_objects,
};
use super::state::{Builder, Frame, FrameKind, Saved, bounds, grouped, line_width, mean_scale, paints, round3, stroke_layer};
use super::{
    Affine, Appearance, AppearanceItem, Arc, BezPath, BfString, BlendMode, CacheKey, Color, Colors, Families, FillLayer, FillRule, FunctionBudget,
    FunctionPaint, HashMap, HashSet, ImageBlob, ImageObject, Knockout, Look, MaskSpec, MaskType, Native, Node, NodeId, NodeKind, OpacityMask, Paint,
    PathData, PathDrawMode, Pattern, PatternDef, Placement, Rect, Scan, Shape, SoftMask, StrokeProps, TextAs, TextLine, TextOverprint, TilingPattern,
    Upright, Walked, clipped, contains, fold_stop_opacity, function_paint, gradient_shape, is_rectangle, luminance, mask_spec, shading_gradient,
};

impl<'p> Builder<'p> {
    pub(super) fn new(next: u64, colors: Colors<'p>, text_as: TextAs, route: bool) -> Self {
        Self {
            colors,
            next,
            stack: vec![],
            root_groups: vec![],
            blend: BlendMode::Normal,
            page: Rect::ZERO,
            glyphs: None,
            text: None,
            text_as,
            images: HashMap::new(),
            image_keys: HashMap::new(),
            cmyk_keys: HashSet::new(),
            opaque: HashSet::new(),
            warnings: vec![],
            fonts: HashMap::new(),
            font_names: HashMap::new(),
            families: None,
            missing_fonts: vec![],
            cid_text: HashMap::new(),
            font_chars: HashMap::new(),
            matched: HashMap::new(),
            identified: HashMap::new(),
            versions: HashMap::new(),
            mask: None,
            masked: vec![],
            masks: HashMap::new(),
            patterns: vec![],
            pattern_keys: HashMap::new(),
            function_paints: HashMap::new(),
            function_budget: FunctionBudget::new(),
            objects_left: max_objects(),
            taken: vec![],
            scan: Scan::default(),
            tag_at: 0,
            paint_at: 0,
            aligned: true,
            route,
            unsure: false,
            marked: vec![],
            nested: 0,
            white_images: false,
            last_fill: None,
            lines: HashMap::new(),
        }
    }

    pub(super) fn id(&mut self) -> NodeId {
        let id = NodeId(self.next);
        self.next += 1;
        id
    }

    /// A deep copy of `node` with fresh ids.
    fn reid(&mut self, node: &Node) -> Node {
        // Each copy counts against the import's objects: once they are spent nothing more is
        // drawn through a mask, so the copying stops too.
        self.objects_left = self.objects_left.saturating_sub(1);
        let mut n = node.clone();
        n.id = self.id();
        if let Some(ch) = n.children_mut() {
            let old = std::mem::take(ch);
            *ch = old.iter().map(|c| Arc::new(self.reid(c))).collect();
        }
        n
    }

    /// `m` with fresh ids for its art: a mask read once can mask several objects.
    fn fresh_mask(&mut self, mut m: OpacityMask) -> OpacityMask {
        m.art = Arc::new(self.reid(&m.art));
        m
    }

    pub(super) fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    pub(super) fn begin_page(&mut self, scan: Scan) {
        self.stack = vec![Frame::new(FrameKind::Root)];
        self.root_groups.clear();
        self.blend = BlendMode::Normal;
        self.glyphs = None;
        self.text = None;
        self.mask = None;
        self.masked.clear();
        self.marked.clear();
        (self.scan, self.tag_at, self.aligned, self.paint_at) = (scan, 0, true, 0);
        if self.scan.overprints && self.scan.zero_mode {
            self.warn(OVERPRINT_ZERO_MODE);
        }
    }

    /// Whether the path the interpreter draws now (a fill or a stroke, each counted) overprints. It
    /// takes the path's turn in the page's walk.
    pub(super) fn path_overprint(&mut self) -> bool {
        if self.nested > 0 {
            return false;
        }
        self.paint_at += 1;
        self.scan.overprints && self.scan.paints.get(self.paint_at - 1).copied().unwrap_or(false)
    }

    /// What the interpreter draws now, as the page's walk counts it: the path whose turn comes
    /// next, or the `text` before it. `None` in a nested reading, which the walk doesn't follow.
    pub(super) fn walked(&self, text: bool) -> Option<Walked> {
        (self.nested == 0).then_some(if text { Walked::Text(self.paint_at) } else { Walked::Path(self.paint_at) })
    }

    /// Whether the glyph the interpreter draws now (its fill, or its `stroke`) overprints.
    pub(super) fn glyph_overprint(&mut self, stroke: bool) -> bool {
        if self.nested > 0 || !self.scan.overprints {
            return false;
        }
        match self.scan.texts.get(self.paint_at) {
            Some(TextOverprint::All { fill, stroke: s }) => *if stroke { s } else { fill },
            Some(TextOverprint::Mixed) => {
                self.warn("some text is set to overprint in a way that can't be read: it knocks out");
                false
            }
            _ => false,
        }
    }

    /// The page's art, split by optional content group (in the order they first paint).
    pub(super) fn end_page(&mut self) -> Vec<(Option<usize>, Vec<Arc<Node>>)> {
        let mut children = self.close_frames();
        // The interpreter drew another number of paths than the file was read to paint: which
        // of them overprint can't be trusted.
        if self.scan.overprints && self.paint_at != self.scan.paints.len() {
            children.iter_mut().for_each(vectorcraft_doc::overprint::clear_overprints);
            self.warn(OVERPRINT_UNREAD);
        }
        let groups = std::mem::take(&mut self.root_groups);
        let mut parts: Vec<(Option<usize>, Vec<Arc<Node>>)> = vec![];
        // Thousands of groups: find each one's part by index.
        let mut at: HashMap<Option<usize>, usize> = HashMap::new();
        for (n, g) in children.into_iter().zip(groups.into_iter().chain(std::iter::repeat(None))) {
            let i = *at.entry(g).or_insert_with(|| {
                parts.push((g, vec![]));
                parts.len() - 1
            });
            if let Some((_, v)) = parts.get_mut(i) {
                v.push(n);
            }
        }
        parts
    }

    /// Finish the open runs and frames → the root frame's children.
    fn close_frames(&mut self) -> Vec<Arc<Node>> {
        self.flush();
        while self.stack.len() > 1 {
            self.pop_frame();
        }
        self.stack.pop().map(|f| f.children).unwrap_or_default()
    }

    /// The group art drawn now belongs to (the innermost marked as one).
    fn current_group(&self) -> Option<usize> {
        self.marked.iter().rev().find_map(|g| *g)
    }

    /// Add `n` to the open frame; `group`: the optional content group it was drawn in, if it
    /// knows one (else the current one).
    fn emit(&mut self, n: Arc<Node>, group: Option<usize>) {
        let root = self.stack.len() == 1;
        let current = self.current_group();
        let Some(f) = self.stack.last_mut() else { return };
        f.children.push(n);
        // A frame's art goes where its first art was drawn (a clip can end in a later group).
        if root {
            self.root_groups.push(group.or(current));
        } else {
            f.child_groups.push(group.or(current));
            if f.group.is_none() {
                f.group = group.or(current);
            }
        }
    }

    /// Add `n`, drawn now, to the art: left out (with a warning) once the import's objects are
    /// spent ([`TOO_MANY_OBJECTS`]).
    pub(super) fn push_node(&mut self, mut n: Node) {
        let Some(left) = self.objects_left.checked_sub(1) else {
            self.lines.remove(&n.id);
            self.warn(TOO_MANY_OBJECTS);
            return;
        };
        self.objects_left = left;
        if self.blend != BlendMode::Normal && !n.is_container() {
            n.blend = self.blend;
        }
        if self.mask.is_some() {
            self.masked.push(Arc::new(n));
        } else {
            self.emit(Arc::new(n), None);
        }
    }

    /// Finish the open glyph runs and the art drawn through the current soft mask.
    pub(super) fn flush(&mut self) {
        self.flush_glyphs();
        self.flush_text();
        self.flush_masked();
    }

    /// The art drawn through the graphics state's soft mask so far, as one masked object.
    fn flush_masked(&mut self) {
        if self.masked.is_empty() {
            return;
        }
        let art = std::mem::take(&mut self.masked);
        let Some(spec) = self.mask.clone() else {
            art.into_iter().for_each(|n| self.emit(n, None));
            return;
        };
        match spec.as_ref() {
            // Each object is drawn through the mask on its own.
            MaskSpec::Opacity(a) => {
                for mut n in art {
                    Arc::make_mut(&mut n).opacity *= a;
                    self.emit(n, None);
                }
            }
            MaskSpec::Mask(m) => {
                let m = self.fresh_mask(m.clone());
                let node = match <[Arc<Node>; 1]>::try_from(art) {
                    Ok([one]) if one.extras().mask.is_none() => {
                        fold_stop_opacity(&one, &m).unwrap_or_else(|| Arc::unwrap_or_clone(one).with_mask(Some(Box::new(m))))
                    }
                    Ok([one]) => {
                        let mut g = Node::group(self.id(), vec![one]);
                        g.extras_mut().mask = Some(Box::new(m));
                        g
                    }
                    Err(all) => {
                        let mut g = Node::group(self.id(), all);
                        g.extras_mut().mask = Some(Box::new(m));
                        g
                    }
                };
                self.emit(Arc::new(node), None);
            }
        }
    }

    /// A clip path node.
    pub(super) fn clip_node(&mut self, path: &BezPath, rule: FillRule) -> Node {
        let mut clip = Node::new(self.id(), NodeKind::Path { path: PathData::from_bezpath(path), rule, live: None, clipping: true, guide: false });
        clip.name = Some("<Clipping Path>".into());
        clip
    }

    pub(super) fn pop_frame(&mut self) {
        if self.stack.len() <= 1 {
            return;
        }
        let Some(f) = self.stack.pop() else { return };
        let group = f.group;
        let node = match f.kind {
            FrameKind::Root => None,
            FrameKind::Skip => {
                grouped(f.children, f.child_groups, group).for_each(|(c, g)| self.emit(c, g));
                None
            }
            FrameKind::Clip(clip) => {
                let noop = clip.path_data().map(|p| p.to_bezpath()).filter(is_rectangle).map(|p| p.bounding_box().inflate(0.01, 0.01));
                if f.children.is_empty() {
                    None
                } else if noop.is_some_and(|r| bounds(&f.children).is_some_and(|b| contains(r, b))) {
                    // A rectangle around all of its art (a form's box) clips nothing.
                    grouped(f.children, f.child_groups, group).for_each(|(c, g)| self.emit(c, g));
                    None
                } else if let Some(shape) = gradient_shape(&clip, &f.children) {
                    // One path, so tools such as the Shape Builder see the shape, not its box (#1304).
                    Some(shape)
                } else {
                    let mut ch = vec![Arc::new(*clip)];
                    ch.extend(f.children);
                    Some(Node::new(self.id(), NodeKind::Group { children: ch, clip: true }))
                }
            }
            FrameKind::Group { opacity, blend, mask, isolate, knockout } => {
                let children = f.children;
                // A constant alpha mask is opacity (of a group that isn't isolated).
                let (opacity, mask) = match mask.as_deref() {
                    Some(MaskSpec::Opacity(a)) => (opacity * a, None),
                    Some(MaskSpec::Mask(m)) => (opacity, Some(m.clone())),
                    None => (opacity, None),
                };
                // Isolation only shows with blending inside; knockout with several objects.
                let isolate = isolate && Node::children_blend(&children, &mut |c| c.blends_through());
                let knockout = knockout && children.len() > 1;
                let plain = opacity >= 0.999 && blend == BlendMode::Normal && mask.is_none() && !isolate && !knockout;
                if children.is_empty() {
                    None
                } else if plain {
                    grouped(children, f.child_groups, group).for_each(|(c, g)| self.emit(c, g));
                    None
                } else if let [only] = children.as_slice()
                    && !only.is_container()
                    && !isolate
                    && !knockout
                    && (mask.is_none() || only.extras().mask.is_none())
                {
                    // A single object in a group: fold opacity, blend and mask into the object.
                    let mut only = (**only).clone();
                    only.opacity *= opacity;
                    if blend != BlendMode::Normal {
                        only.blend = blend;
                    }
                    if let Some(m) = mask {
                        let m = self.fresh_mask(m);
                        only = fold_stop_opacity(&only, &m).unwrap_or_else(|| only.with_mask(Some(Box::new(m))));
                    }
                    Some(only)
                } else {
                    let mut g = Node::group(self.id(), children);
                    (g.opacity, g.blend, g.isolate) = (opacity, blend, isolate);
                    if knockout {
                        g.knockout = Knockout::On;
                    }
                    g.extras_mut().mask = mask.map(|m| Box::new(self.fresh_mask(m)));
                    Some(g)
                }
            }
        };
        if let Some(n) = node {
            self.emit(Arc::new(n), group);
        }
    }

    pub(super) fn flush_glyphs(&mut self) {
        let Some(run) = self.glyphs.take() else { return };
        if run.path.elements().is_empty() {
            return;
        }
        let mut ap = Appearance::default();
        match &run.stroke {
            None => {
                let mut f = FillLayer::new(run.paint);
                (f.opacity, f.overprint) = (run.opacity, run.overprint);
                ap.items.push(AppearanceItem::Fill(f));
            }
            Some(p) => {
                let mut st = stroke_layer(run.paint, run.opacity, p, run.scale);
                st.overprint = run.overprint;
                ap.items.push(AppearanceItem::Stroke(st));
            }
        }
        let id = self.id();
        let mut n = Node::path(id, PathData::from_bezpath(&run.path), ap);
        n.name = Some(TEXT_OUTLINES.into());
        self.push_node(n);
    }

    pub(super) fn flush_text(&mut self) {
        let Some(mut line) = self.text.take() else { return };
        let held = std::mem::take(&mut line.held);
        if let Some((t, opacity, facts)) = line.finish() {
            let mut n = Node::new(self.id(), NodeKind::Text(Box::new(t)));
            n.opacity = opacity;
            self.lines.insert(n.id, facts);
            self.push_node(n);
        }
        // The strokes of its glyphs' outlines, right after it ([`import_lines`] makes them its stroke).
        for n in held {
            self.push_node(n);
        }
    }

    /// A stroke of the outline of the glyph just drawn, in the glyph's own paint, while every glyph
    /// before it on the open line was stroked so too, is held with the line instead of ending it
    /// (an editor writes type on a path glyph by glyph, each followed by the stroke added to the
    /// type). `false`: it isn't one. `walked`: the path in the page's walk.
    pub(super) fn hold_glyph_stroke(
        &mut self,
        path: &BezPath,
        transform: Affine,
        paint: &craft_pdf::interpret::Paint<'_>,
        draw_mode: &PathDrawMode,
        walked: Option<Walked>,
    ) -> bool {
        let PathDrawMode::Stroke(props) = draw_mode else { return false };
        if self.mask.is_some() || self.blend != BlendMode::Normal || matches!(paint, craft_pdf::interpret::Paint::Pattern(_)) {
            return false;
        }
        let (p, opacity) = self.paint(paint, true, walked);
        let Some(line) = &self.text else { return false };
        let Some(ink) = line.ink.last() else { return false };
        if line.held.len() + 1 != line.ink.len() || opacity < 0.999 || !line.last_fill().is_some_and(|f| *f == p) {
            return false;
        }
        let mut bp = path.clone();
        bp.apply_affine(transform);
        let b = bp.bounding_box();
        let tol = 0.01 * (ink.width() + ink.height()) + 0.01;
        if ![(b.x0, ink.x0), (b.y0, ink.y0), (b.x1, ink.x1), (b.y1, ink.y1)].iter().all(|(a, c)| (a - c).abs() <= tol) {
            return false;
        }
        let mut st = stroke_layer(p, opacity, props, mean_scale(transform));
        // Every painted path takes its turn in the scan's overprint list, a held one too.
        st.overprint = self.path_overprint();
        let n = Node::path(self.id(), PathData::from_bezpath(&bp), Appearance { items: vec![AppearanceItem::Stroke(st)], ..Default::default() });
        if let Some(line) = &mut self.text {
            line.held.push(n);
        }
        true
    }

    /// Convert a hayro paint to a VectorCraft paint and opacity (`stroke`: for a stroke; `walked`:
    /// what it paints in the page's walk, which names its ink).
    pub(super) fn paint(&mut self, p: &craft_pdf::interpret::Paint<'_>, stroke: bool, walked: Option<Walked>) -> (Paint, f32) {
        let pattern = matches!(p, craft_pdf::interpret::Paint::Pattern(_));
        let inks = walked.and_then(|w| self.scan.ink(w, stroke, pattern));
        match p {
            craft_pdf::interpret::Paint::Color(c) => {
                let [r, g, b, a] = c.to_rgba().components();
                let paint = self.colors.solid(c, inks).map_or_else(|| Paint::solid(Color::rgb(round3(r), round3(g), round3(b))), Native::paint);
                (paint, a)
            }
            craft_pdf::interpret::Paint::Pattern(pat) => match pat.as_ref() {
                Pattern::Shading(sp) => {
                    // The ink names outlive the walk's borrow of `self`.
                    let inks = inks.map(<[String]>::to_vec);
                    match self.function_shading(sp, inks.as_deref()) {
                        Some(paint) => (paint, sp.opacity),
                        None => match shading_gradient(sp, &mut self.colors, inks.as_deref()) {
                            Some((g, _)) => (Paint::Gradient(Box::new(g)), sp.opacity),
                            None => {
                                self.warn("shadings that can't be read as a gradient or gradient mesh (such as very large meshes) are imported as a flat grey");
                                (Paint::solid(Color::rgb(0.5, 0.5, 0.5)), sp.opacity)
                            }
                        },
                    }
                }
                Pattern::Tiling(t) => match self.tiling(t, stroke) {
                    Some(paint) => (paint, 1.0),
                    None => {
                        self.warn("tiling patterns nested too deeply are imported as a flat grey");
                        (Paint::solid(Color::rgb(0.5, 0.5, 0.5)), 1.0)
                    }
                },
            },
        }
    }

    /// Read a soft mask, pattern cell or type 3 glyph (`read`) on its own: its art, in the
    /// coordinates `page` has. `None` when nested too deeply.
    fn nested(&mut self, page: Rect, read: impl FnOnce(&mut Self)) -> Option<Vec<Arc<Node>>> {
        if self.nested >= MAX_NESTED {
            return None;
        }
        self.flush();
        let saved = Saved {
            stack: std::mem::replace(&mut self.stack, vec![Frame::new(FrameKind::Root)]),
            root_groups: std::mem::take(&mut self.root_groups),
            blend: std::mem::replace(&mut self.blend, BlendMode::Normal),
            mask: self.mask.take(),
            page: std::mem::replace(&mut self.page, page),
        };
        self.nested += 1;
        read(self);
        let art = self.close_frames();
        self.nested -= 1;
        (self.stack, self.root_groups, self.blend, self.mask, self.page) = (saved.stack, saved.root_groups, saved.blend, saved.mask, saved.page);
        Some(art)
    }

    /// What soft mask `m` does (read once per mask and placement).
    pub(super) fn soft_mask(&mut self, m: &SoftMask<'_>) -> Arc<MaskSpec> {
        let key = m.cache_key();
        if let Some(s) = self.masks.get(&key) {
            return s.clone();
        }
        let page = self.page;
        let saved = std::mem::replace(&mut self.white_images, m.mask_type() == MaskType::Alpha);
        let art = self.nested(page, |b| m.interpret(b));
        self.white_images = saved;
        let spec = match art {
            Some(art) => {
                let [r, g, b, _] = m.background_color().to_rgba().components();
                let backdrop = luminance(&Color::rgb(r, g, b));
                let inverted = m.transfer_function().is_some_and(|tf| {
                    let mut ends = [0, 255];
                    tf.apply_to(&mut ends);
                    ends[0] > ends[1]
                });
                let id = self.id();
                mask_spec(art, m.mask_type() == MaskType::Alpha, backdrop, inverted, page, |all| Node::group(id, all))
            }
            None => {
                self.warn("soft masks nested too deeply were left out (content drawn unmasked)");
                MaskSpec::Opacity(1.0)
            }
        };
        let spec = Arc::new(spec);
        self.masks.insert(key, spec.clone());
        spec
    }

    /// Function-based shading `sp` (painting with `inks`) as a paint, read the first time it is
    /// used and within the import's budget of function evaluations; `None` for other shadings.
    fn function_shading(&mut self, sp: &craft_pdf::interpret::pattern::ShadingPattern, inks: Option<&[String]>) -> Option<Paint> {
        let key = (sp.cache_key(), inks.map(<[String]>::to_vec));
        if let Some(paint) = self.function_paints.get(&key) {
            return Some(paint.clone());
        }
        let paint = match function_paint(sp, &mut self.colors, inks, &self.function_budget)? {
            FunctionPaint::Gradient(g) => Paint::Gradient(g),
            FunctionPaint::Average(c) => {
                self.warn(
                    "function-based shadings that aren't read as a gradient (their colour changes in two directions, they need too many stops, or the file has too many to read) are imported as their average colour",
                );
                Paint::solid(c)
            }
            FunctionPaint::Unread => {
                self.warn("function-based shadings that can't be read (their function or domain is invalid, or the file has too many to read) are imported as a flat grey");
                Paint::solid(Color::rgb(0.5, 0.5, 0.5))
            }
        };
        self.function_paints.insert(key, paint.clone());
        Some(paint)
    }

    /// Tiling pattern `t` as a pattern swatch's paint (the swatch made the first time).
    fn tiling(&mut self, t: &TilingPattern<'_>, stroke: bool) -> Option<Paint> {
        let key = (t.cache_key(), stroke);
        let tile = Rect::from_origin_size(t.bbox.origin(), (t.x_step.abs().max(1e-3) as f64, t.y_step.abs().max(1e-3) as f64));
        let xf = t.matrix * Affine::translate(tile.origin().to_vec2());
        // An uncoloured pattern draws in the current colour: read it again unless it always
        // drew the same.
        if let Some((n, seen)) = self.pattern_keys.get(&key)
            && *n >= PATTERN_REUSE
            && let [(_, name)] = seen.as_slice()
        {
            return Some(Paint::Pattern { pattern: name.clone(), xf });
        }
        // A swatch is shared with uses outside an alpha mask, and pattern paints keep their
        // colours there: its images are read as they are.
        let white = std::mem::take(&mut self.white_images);
        let art = self.nested(Rect::ZERO, |b| {
            // A cell whose content can't be decoded is empty.
            let _ = t.interpret(b, Affine::IDENTITY, stroke);
        });
        self.white_images = white;
        let art = art?;
        let sig = paints(&art);
        let entry = self.pattern_keys.entry(key).or_default();
        entry.0 += 1;
        let name = match entry.1.iter().find(|(s, _)| *s == sig) {
            Some((_, name)) => name.clone(),
            None => {
                let name = (self.patterns.len() + 1..).map(|i| format!("Pattern {i}")).find(|n| !self.taken.contains(n)).unwrap_or_default();
                self.taken.push(name.clone());
                let mut def = PatternDef::new(&name, art);
                def.tile = tile;
                self.patterns.push(def);
                if let Some(e) = self.pattern_keys.get_mut(&key) {
                    e.1.push((sig, name.clone()));
                }
                name
            }
        };
        Some(Paint::Pattern { pattern: name, xf })
    }

    pub(super) fn add_image(&mut self, key: u128, make: impl FnOnce() -> Option<(ImageBlob, u32, u32)>, xf: Affine) {
        self.add_image_masked(key, make, xf, None);
    }

    /// An image's alpha (its soft mask, stencil mask or colour key, at its own resolution) as a
    /// greyscale image over the image's `w` × `h` pixels (`transform`): its opacity mask. `None`
    /// when it masks nothing.
    pub(super) fn alpha_mask(
        &mut self,
        key: u128,
        r: &craft_pdf::interpret::RasterImage<'_>,
        w: u32,
        h: u32,
        transform: Affine,
    ) -> Option<OpacityMask> {
        let mkey = key ^ 0x5_3a5c;
        if self.opaque.contains(&mkey) {
            return None;
        }
        let (k, mw, mh) = if let Some(v) = self.image_keys.get(&mkey).cloned() {
            v
        } else {
            let mut alpha = None;
            r.with_rgba(|_, a| alpha = a, None);
            let Some(a) = alpha.filter(|a| a.data.iter().any(|v| *v < 255)) else {
                self.opaque.insert(mkey);
                return None;
            };
            let mut png = Vec::new();
            image::GrayImage::from_raw(a.width, a.height, a.data)?.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
            let blob = ImageBlob::new("image/png", png);
            let k = blob.content_key();
            self.images.insert(k.clone(), blob);
            self.image_keys.insert(mkey, (k.clone(), a.width, a.height));
            (k, a.width, a.height)
        };
        let xf = transform * Affine::scale_non_uniform(f64::from(w) / f64::from(mw.max(1)), f64::from(h) / f64::from(mh.max(1)));
        let art = Node::new(
            self.id(),
            NodeKind::Image(Box::new(ImageObject { key: k, width: mw, height: mh, xf, link: None, placement: Default::default(), pixels: None })),
        );
        Some(OpacityMask::new(art, true))
    }

    pub(super) fn add_image_masked(
        &mut self,
        key: u128,
        make: impl FnOnce() -> Option<(ImageBlob, u32, u32)>,
        xf: Affine,
        mask: Option<OpacityMask>,
    ) {
        let (k, w, h) = if let Some(v) = self.image_keys.get(&key).cloned() {
            v
        } else {
            let Some((blob, w, h)) = make() else {
                self.warn("an image could not be decoded and was skipped");
                return;
            };
            // Content keys: placing this page into another document merges its images safely.
            let k = blob.content_key();
            self.images.insert(k.clone(), blob);
            self.image_keys.insert(key, (k.clone(), w, h));
            (k, w, h)
        };
        let id = self.id();
        let mut n = Node::new(
            id,
            NodeKind::Image(Box::new(ImageObject { key: k, width: w, height: h, xf, link: None, placement: Default::default(), pixels: None })),
        );
        n.extras_mut().mask = mask.map(Box::new);
        self.push_node(n);
    }

    /// Mesh shadings filling `region` (document space): gradient meshes, clipped to the region
    /// when they reach past it, or always (`clip`: a stroke's outline, which a mesh can lie
    /// within the box of without filling it).
    pub(super) fn draw_meshes(&mut self, meshes: Vec<vectorcraft_doc::live::GradientMesh>, region: &BezPath, opacity: f32, clip: bool) {
        let nodes: Vec<Arc<Node>> = meshes.into_iter().map(|m| Arc::new(Node::new(self.id(), NodeKind::Mesh(m)))).collect();
        let Some(art) = bounds(&nodes) else { return };
        let mut content = match <[Arc<Node>; 1]>::try_from(nodes) {
            Ok([one]) => Arc::unwrap_or_clone(one),
            Err(all) => Node::group(self.id(), all),
        };
        content.opacity *= opacity;
        if clip || !contains(region.bounding_box().inflate(0.5, 0.5), art) {
            let clip = self.clip_node(region, FillRule::NonZero);
            content = clipped(clip, content, self.id());
        }
        self.push_node(content);
    }

    /// The family and style of font `key` (drawing glyph `o`): the installed face of its PostScript
    /// name, else the available family its name reads as; whether it is available.
    /// The character of glyph `glyph` of font `key` that no mapping names, found by its outline in
    /// the installed face of the font's name ([`identify_glyph`]); cached per glyph.
    fn identified_char(&mut self, key: u128, glyph: u32, o: &craft_pdf::interpret::font::OutlineGlyph) -> Option<char> {
        if let Some(c) = self.identified.get(&(key, glyph)) {
            return *c;
        }
        let c = self.font_name(key, o).face.and_then(|f| identify_glyph(&f, &o.outline()));
        self.identified.insert((key, glyph), c);
        c
    }

    fn font_name(&mut self, key: u128, o: &craft_pdf::interpret::font::OutlineGlyph) -> FontInfo {
        if let Some(n) = self.font_names.get(&key) {
            return n.clone();
        }
        let data = o.font_data();
        let name = self.fonts.get(&key).map(|f| f.name.clone()).or_else(|| data.as_ref().and_then(|d| d.postscript_name.clone()));
        let (weight, italic) = data.as_ref().map_or((None, false), |d| (d.weight, d.is_italic));
        // A subset's six-letter tag.
        let ps = name.as_deref().map(|n| match n.split_once('+') {
            Some((tag, rest)) if tag.len() == 6 && tag.chars().all(|c| c.is_ascii_uppercase()) => rest.to_string(),
            _ => n.to_string(),
        });
        let installed = ps.as_deref().and_then(|ps| vectorcraft_text::FontDb::global().find_postscript(ps));
        let n = match (installed, name) {
            (Some(face), _) => FontInfo { family: face.family.clone(), style: face.style.clone(), face: Some(face) },
            (None, Some(name)) => {
                let families = self.families.get_or_insert_with(Families::available);
                let (family, style, found) = families.resolve(&name, weight, italic);
                if !found && !family.is_empty() && !self.missing_fonts.contains(&family) {
                    self.missing_fonts.push(family.clone());
                }
                FontInfo { family, style, face: None }
            }
            (None, None) => {
                let d = vectorcraft_doc::CharStyle::default();
                FontInfo { family: d.font_family, style: d.font_style, face: None }
            }
        };
        self.font_names.insert(key, n.clone());
        n
    }

    /// Glyph `o` as type: added to the line being gathered (or starting one). `false`: it keeps
    /// its outline (no Unicode text, mirrored, or not type mode). No `paint`: invisible text,
    /// type with neither fill nor stroke.
    pub(super) fn type_glyph(
        &mut self,
        o: &craft_pdf::interpret::font::OutlineGlyph,
        m: Affine,
        scale: f64,
        paint: Option<&craft_pdf::interpret::Paint<'_>>,
        stroke: Option<&StrokeProps>,
        overprint: bool,
    ) -> bool {
        if self.text_as != TextAs::Text || self.nested > 0 {
            return false;
        }
        let key = o.font_cache_key();
        let glyph = o.glyph_id().to_u32();
        let (unicode, from_cid) = match o.as_unicode().filter(|u| !replacement(u)) {
            Some(u) => (Some(u), false),
            // No mapping, or one to U+FFFD (#708): the font program may still name the character.
            None => {
                let cid = self.cid_text.entry(key).or_insert_with(|| o.font_data().and_then(|d| CidText::of(d.data.as_ref().as_ref())).map(Arc::new));
                match cid.as_ref().and_then(|c| c.unicode(glyph)) {
                    Some(u) => (Some(u), true),
                    None => {
                        let chars = self
                            .font_chars
                            .entry(key)
                            .or_insert_with(|| o.font_data().and_then(|d| font_chars(d.data.as_ref().as_ref())).map(Arc::new));
                        let named = chars.as_ref().and_then(|m| m.get(&glyph)).copied();
                        // Still unnamed: the installed font may draw the same outline (#811).
                        (named.or_else(|| self.identified_char(key, glyph, o)).map(BfString::Char), false)
                    }
                }
            }
        };
        let text: String = match unicode {
            Some(BfString::Char(c)) => c.to_string(),
            Some(BfString::String(s)) => s,
            None => {
                // Invisible text isn't kept as outlines: the caller says it was left out.
                if paint.is_some() && o.as_unicode().is_some_and(|u| replacement(&u)) {
                    let name = self.font_name(key, o).family;
                    self.warn(&format!("text in {name} {UNNAMED_CHARACTERS_NOTE}"));
                }
                return false;
            }
        };
        // Adobe's CID tables map some ideographs to their look-alike Kangxi radicals.
        let text: String = if from_cid { text.chars().map(unify_radical).collect() } else { text };
        let Some(at) = Placement::of(m) else { return false };
        if text.is_empty() || text.chars().any(|c| c.is_control()) {
            return false;
        }
        let width = o.advance_width().filter(|w| w.is_finite());
        // Set vertically (WMode 1): no horizontal advance.
        let upright = width.is_some_and(|w| w.abs() <= 1.0);
        let mut info = self.font_name(key, o);
        // A font the file doesn't carry is drawn with a stand-in: its glyphs can't tell which
        // style of the installed family it is, so the name decides.
        let embedded = self.fonts.get(&key).is_none_or(|f| f.embedded);
        if let Some(face) = &info.face
            && embedded
        {
            let decided = match self.matched.get(&key) {
                Some(m) => m.clone(),
                // A glyph that can't be compared (several characters, a vertical form) or that
                // shows nothing (invisible text) decides nothing: it takes the face of its name
                // until one that can be compared does.
                None if paint.is_none() || comparable(&text, upright).is_none() => Some(face.clone()),
                None => {
                    let m = matching_face(face, &text, o, upright);
                    self.matched.insert(key, m.clone());
                    m
                }
            };
            // Invisible text shows no glyphs that could differ: it keeps the face of its name.
            let Some(f) = decided.or_else(|| paint.is_none().then(|| face.clone())) else {
                self.warn(UNMATCHED_GLYPHS_NOTE);
                return false;
            };
            info = FontInfo { family: f.family.clone(), style: f.style.clone(), face: Some(f) };
        }
        // A version of the family and style that the two alone don't resolve to: the type names it.
        // Kept per font once its face is decided (until then each glyph asks).
        let named = |f: &Option<Arc<vectorcraft_text::FontFace>>| {
            let f = f.as_ref()?;
            let default = vectorcraft_text::FontDb::global().face(&f.family, &f.style)?;
            (default.version != f.version).then(|| f.version.clone())
        };
        let version = match self.versions.get(&key) {
            Some(v) => v.clone(),
            None if self.matched.contains_key(&key) => self.versions.entry(key).or_insert_with(|| named(&info.face)).clone(),
            None => named(&info.face),
        };
        let (paint, opacity) = match paint {
            Some(p) => {
                let (paint, opacity) = self.paint(p, stroke.is_some(), self.walked(true));
                (Some(paint), opacity)
            }
            None => (None, 1.0),
        };
        let stroke = stroke.zip(paint.clone()).map(|(p, paint)| (paint, line_width(p, scale)));
        let look = Look {
            font: key,
            family: info.family,
            style: info.style,
            version,
            size: at.size,
            h_scale: at.h_scale,
            overprint_fill: stroke.is_none() && overprint,
            overprint_stroke: stroke.is_some() && overprint,
            fill: if stroke.is_none() { paint } else { None },
            stroke,
        };
        let advance = width.map_or(at.size * 0.5, |w| w as f64 / 1000.0 * at.size * at.h_scale / 100.0);
        // A glyph set vertically is drawn from its origin, which the file puts the position
        // vector's vx (from the font's `W2`, else half its width from `W` or `DW`) to the left of
        // the column's centre (ISO 32000-1 §9.7.4.3); vertical type centres each glyph on the
        // column. Half an em only for a full-width glyph (#722).
        let top = upright.then(|| {
            let vx = self.fonts.get(&key).and_then(|f| f.vertical.as_ref()).map_or(500.0, |v| v.centre(glyph));
            m * kurbo::Point::new(vx, 880.0)
        });
        let place = |line: &mut TextLine| match top {
            Some(top) => line.push_upright(&look, at, opacity, Upright { top }, &text),
            None => line.push(&look, at, opacity, advance, &text),
        };
        self.flush_glyphs();
        // A stroke over the glyph just filled (fill and stroke rendering).
        if let (Some(s), Some(line)) = (&look.stroke, &mut self.text)
            && look.fill.is_none()
            && line.stroke_last(key, at, s.clone(), overprint)
        {
            return true;
        }
        if !self.text.as_mut().is_some_and(place) {
            self.flush_text();
            let mut line = TextLine::new(at, opacity);
            place(&mut line);
            self.text = Some(line);
        }
        // Its ink: a stroke the file draws as outlines of the glyphs is told by it.
        let mut ink = o.outline();
        if let Some(line) = &mut self.text
            && !ink.elements().is_empty()
        {
            ink.apply_affine(m);
            line.ink.push(ink.bounding_box());
        }
        true
    }
}
