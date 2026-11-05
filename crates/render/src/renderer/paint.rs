use super::{
    Affine, AppearanceItem, Arc, BezPath, BlendMode, Compose, FillRule, Frame, Ink, Mix, Node, Rect, RenderContext, Renderer, Shape, SliceEntry,
    Slices, StrokeAlign, StrokeEntry, StrokeLayer, TextGeom, blend_mode, drawable, effects, fill_rule, fx, kurbo, lru, owns_stroke, peniko,
    rects_overlap,
};
use crate::frame::{MovedKey, MovedSliceKey, StrokeKey};
use crate::paint;

impl Renderer {
    /// The object opacity `n` is drawn with (full while it is drawn as its knockout shape).
    pub(crate) fn opacity_of(&self, n: &Node) -> f32 {
        if self.is_shape(n) { 1.0 } else { n.opacity }
    }

    pub(crate) fn hairline(&mut self, ctx: &mut RenderContext, f: &Frame, bp: &BezPath, rgba: [u8; 4]) {
        ctx.set_transform(Affine::IDENTITY);
        let mut screen = bp.clone();
        screen.apply_affine(f.view);
        ctx.set_stroke(kurbo::Stroke::new(self.hairline));
        ctx.set_paint(f.ink.fixed(rgba));
        ctx.stroke_path(&screen);
    }

    pub(super) fn draw_shape(&mut self, ctx: &mut RenderContext, f: &Frame, n: &Node, bp: &BezPath, rule: FillRule) {
        if !drawable(bp, f.view, n) {
            return;
        }
        if fx::has_fx(n) {
            return self.draw_shape_fx(ctx, f, n, bp, rule);
        }
        if f.opts.outline {
            self.hairline(ctx, f, bp, [0, 0, 0, 255]);
            return;
        }
        // Bounds only matter to gradients and patterns (their geometry is relative to the object).
        let solid = |p: &vectorcraft_color::Paint| matches!(p, vectorcraft_color::Paint::Solid { .. } | vectorcraft_color::Paint::None);
        let needs_bounds = n.appearance.items.iter().any(|i| match i {
            AppearanceItem::Fill(fl) => !solid(&fl.paint),
            AppearanceItem::Stroke(st) => !solid(&st.paint),
        });
        let bounds = if needs_bounds { bp.bounding_box() } else { Rect::ZERO };
        for item in &n.appearance.items {
            match item {
                AppearanceItem::Fill(fl) => {
                    if !fl.visible || fl.paint.is_none() {
                        continue;
                    }
                    let layered = fl.opacity < 1.0 || fl.blend != vectorcraft_color::BlendMode::Normal;
                    if layered {
                        ctx.set_transform(Affine::IDENTITY);
                        ctx.push_layer(None, Some(blend_mode(fl.blend)), Some(fl.opacity), None, None);
                    }
                    ctx.set_transform(f.view);
                    if paint::set_fill_paint(ctx, &fl.paint, bounds, f) {
                        self.fold_alpha(ctx, f.ink, &fl.paint);
                        ctx.set_fill_rule(fill_rule(rule));
                        ctx.fill_path(bp);
                    }
                    if layered {
                        ctx.pop_layer();
                    }
                }
                AppearanceItem::Stroke(st) => {
                    if !st.visible || st.paint.is_none() || st.width <= 0.0 {
                        continue;
                    }
                    if st.brush.is_some() && self.draw_brush(ctx, f, n, bp, st) {
                        continue;
                    }
                    self.draw_stroke(ctx, f, Affine::IDENTITY, bp, rule, st, None, bounds);
                }
            }
        }
    }

    /// Multiply the folded object opacity into a solid paint.
    pub(super) fn fold_alpha(&self, ctx: &mut RenderContext, ink: Ink, p: &vectorcraft_color::Paint) {
        if self.alpha < 1.0
            && let vectorcraft_color::Paint::Solid { color, .. } = p
        {
            ctx.set_paint(ink.color(color, self.alpha));
        }
    }

    /// Paint stroke `st` of the shape `bp` (in the space `xf` maps to the document; `bounds`: its
    /// geometric bounds there) with its alignment, dashes, profile, arrowheads, opacity and blend.
    /// `chars` is the glyph geometry kept in `texts` and the run that `st` is the character stroke
    /// of, when it is one.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_stroke(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        xf: Affine,
        bp: &BezPath,
        rule: FillRule,
        st: &StrokeLayer,
        chars: Option<(&Arc<TextGeom>, usize)>,
        bounds: Rect,
    ) {
        // A stroke drawn without its dashes is a copy, not the document's stroke: its outline is
        // made here and not kept with the document's outlines.
        let drawn = f.stroke(st);
        let (st, chars) = match &drawn {
            std::borrow::Cow::Owned(solid) => (solid, None),
            std::borrow::Cow::Borrowed(st) => (*st, chars),
        };
        let pieces = effects::stroke::stroke_pieces(bp, st);
        // The line and its heads overlap: they take the opacity once, in one layer, instead of
        // each folding it into its paint.
        let folded = if pieces.heads.is_empty() { 1.0 } else { std::mem::replace(&mut self.alpha, 1.0) };
        let opacity = st.opacity * folded;
        let layered = opacity < 1.0 || st.blend != vectorcraft_color::BlendMode::Normal || st.align == StrokeAlign::Outside;
        if layered {
            ctx.set_transform(Affine::IDENTITY);
            ctx.push_layer(None, Some(blend_mode(st.blend)), Some(opacity), None, None);
        }
        ctx.set_transform(f.view * xf);
        let closed = effects::stroke::is_closed(bp);
        // Keep hairlines visible when zoomed far out (at least ~1 device pixel). A width profile
        // multiplies the weight, so it is the profile's widest point that is kept visible: raising
        // the weight further would swell the whole stroke while the view zooms out (#995).
        let widest = st.profile.as_ref().map_or(1.0, |p| p.widest().max(1e-3));
        let width = effects::stroke::aligned_width(st, closed).max(f.px * 0.5 / xf.determinant().abs().sqrt().max(1e-9) / widest);
        let inside = st.align == StrokeAlign::Inside && closed;
        if inside {
            ctx.set_fill_rule(fill_rule(rule));
            ctx.push_clip_layer(bp);
        }
        // One paint box for the line and the heads, so a gradient runs on into the heads.
        let paint_bounds = bounds.inflate(st.width / 2.0, st.width / 2.0);
        if let Some(g) = st.path_gradient() {
            self.fill_path_gradient(ctx, f, bp, rule, st, chars, &pieces, width, &g.gradient);
        } else if paint::set_paint(ctx, &st.paint, paint_bounds, f) {
            self.fold_alpha(ctx, f.ink, &st.paint);
            ctx.set_fill_rule(peniko::Fill::NonZero);
            match self.cached_stroke(f, chars, bp, &pieces.line, st, width) {
                Some(o) => ctx.fill_path(&o),
                None => ctx.fill_path(&effects::stroke::line_outline(&pieces.line, st, width, f.px * 0.25)),
            }
            for head in &pieces.heads {
                ctx.fill_path(&head.outline);
            }
        }
        if inside {
            ctx.pop_layer();
        }
        if st.align == StrokeAlign::Outside && closed {
            // Punch out the interior.
            ctx.set_blend_mode(BlendMode::new(Mix::Normal, Compose::DestOut));
            ctx.set_paint(peniko::Color::BLACK);
            ctx.set_fill_rule(fill_rule(rule));
            ctx.fill_path(bp);
            ctx.set_blend_mode(BlendMode::default());
        }
        if layered {
            ctx.pop_layer();
        }
        if !pieces.heads.is_empty() {
            self.alpha = folded;
        }
    }

    /// Paint the line and arrowheads (`pieces`) of stroke `st` of `bp` (filled with `rule`) with
    /// its gradient `g` laid along or across the path ([`effects::stroke::gradient_slices`]). Each
    /// piece clips the slices, grown a pixel into each other and each replacing what it covers, so
    /// neighbours meet without seams and translucent colours paint once. The slices of a stroke
    /// of `self.cur` are kept in `stroke_slices`, and a moved copy of the object draws the slices
    /// made before it moved, moved by the same offset ([`Self::moved_slices`]).
    #[allow(clippy::too_many_arguments)]
    fn fill_path_gradient(
        &mut self,
        ctx: &mut RenderContext,
        f: &Frame,
        bp: &BezPath,
        rule: FillRule,
        st: &StrokeLayer,
        chars: Option<(&Arc<TextGeom>, usize)>,
        pieces: &effects::stroke::StrokePieces,
        width: f64,
        g: &vectorcraft_color::Gradient,
    ) {
        let level = (f.px * 0.25).log2().floor() as i32;
        let make = || -> Slices {
            #[cfg(test)]
            SLICES_MADE.with(|n| n.set(n.get() + 1));
            let tol = 2f64.powi(level);
            // Reach a pixel past the stroke, so its antialiased edge has colour under it.
            let slices = effects::stroke::gradient_slices(bp, rule, st, tol, f.px + tol, f.px);
            Arc::new(
                slices
                    .into_iter()
                    .map(|s| {
                        let (b, paint) = (s.shape.bounding_box(), s.paint(g));
                        (s.shape, paint, b)
                    })
                    .collect(),
            )
        };
        let (slices, by) = match self.cur.clone().filter(|n| owns_stroke(n, st)) {
            Some(node) => {
                let key = (self.stroke_owner(Arc::as_ptr(&node) as usize), st as *const StrokeLayer as usize, level);
                match self.stroke_slices.get_mut(&key) {
                    Some(e) if Arc::ptr_eq(&e.node, &node) => {
                        e.stamp = self.stamp;
                        (e.slices.clone(), e.by)
                    }
                    _ => {
                        // Under its object's id, the stroke's place and the tolerance level, the key
                        // of the stroke's latest slices is kept, except for art made again in every
                        // frame.
                        let place = node.appearance.items.iter().position(|i| matches!(i, AppearanceItem::Stroke(s) if std::ptr::eq(s, st)));
                        let moved = place.filter(|_| !self.made_again).map(|place| (node.id.0, place, level));
                        let kept = moved.and_then(|mk| self.moved_slices(mk, &node, st, rule, bp));
                        let (made, slices, by, own) = match kept {
                            Some((made, slices, by)) => (made, slices, by, true),
                            None => {
                                let own = moved.is_some() && self.own_path(&node, bp);
                                (node.clone(), make(), kurbo::Vec2::ZERO, own)
                            }
                        };
                        self.stroke_slices.insert(key, SliceEntry { node, made, slices: slices.clone(), by, rule, own, stamp: self.stamp });
                        if let Some(mk) = moved {
                            self.moved_slices.insert(mk, key);
                        }
                        (slices, by)
                    }
                }
            }
            None => (make(), kurbo::Vec2::ZERO),
        };
        let line = match self.cached_stroke(f, chars, bp, &pieces.line, st, width) {
            Some(o) => o,
            None => Arc::new(effects::stroke::line_outline(&pieces.line, st, width, f.px * 0.25)),
        };
        ctx.set_fill_rule(peniko::Fill::NonZero);
        for clip in std::iter::once(&*line).chain(pieces.heads.iter().map(|h| &h.outline)) {
            let cb = clip.bounding_box();
            if clip.elements().is_empty() || !cb.is_finite() {
                continue;
            }
            ctx.push_clip_layer(clip);
            ctx.set_blend_mode(BlendMode::new(Mix::Normal, Compose::Copy));
            for (shape, paint, b) in slices.iter() {
                let b = *b + by;
                if !rects_overlap(b, cb) {
                    continue;
                }
                if by == kurbo::Vec2::ZERO {
                    if paint::set_paint(ctx, paint, b, f) {
                        ctx.fill_path(shape);
                    }
                } else if paint::set_paint(ctx, &moved_paint(paint, by), b, f) {
                    let mut shape = shape.clone();
                    shape.apply_affine(Affine::translate(by));
                    ctx.fill_path(&shape);
                }
            }
            ctx.set_blend_mode(BlendMode::default());
            ctx.pop_layer();
        }
    }

    /// The line part of a stroke (see [`effects::stroke::line_outline`]), cached across frames: a
    /// character stroke of run `chars.1` of kept glyph geometry `chars.0` in `char_strokes`, a
    /// stroke of `self.cur` in `strokes` (under [`Self::stroke_owner`] and the stroke layer). The
    /// tolerance (vello's 0.25 device px) is bucketed to powers of two, never coarser than needed,
    /// so zooming reuses outlines until the level changes. Other strokes aren't cached: for them
    /// this returns `None`. `line` is the line of `st` along `path` ([`effects::stroke::stroke_pieces`]).
    fn cached_stroke(
        &mut self,
        f: &Frame,
        chars: Option<(&Arc<TextGeom>, usize)>,
        path: &BezPath,
        line: &BezPath,
        st: &StrokeLayer,
        width: f64,
    ) -> Option<Arc<BezPath>> {
        let level = (f.px * 0.25).log2().floor() as i32;
        let expand = || Arc::new(effects::stroke::line_outline(line, st, width, 2f64.powi(level)));
        if let Some((g, run)) = chars {
            let key = (self.stroke_owner(Arc::as_ptr(g) as usize), run, width.to_bits(), level);
            if let Some((kept, outline)) = self.char_strokes.get(&key, self.stamp)
                && Arc::ptr_eq(kept, g)
            {
                return Some(outline.clone());
            }
            let outline = expand();
            self.char_strokes.insert(key, (g.clone(), outline.clone()), lru::path_bytes(&outline), self.stamp);
            return Some(outline);
        }
        let node = self.cur.clone().filter(|n| owns_stroke(n, st))?;
        let key = (self.stroke_owner(Arc::as_ptr(&node) as usize), st as *const StrokeLayer as usize, width.to_bits(), level);
        let v = f.visible;
        let holds = |crop: Option<Rect>| crop.is_none_or(|r| r.x0 <= v.x0 && r.y0 <= v.y0 && r.x1 >= v.x1 && r.y1 >= v.y1);
        if let Some(e) = self.strokes.get_mut(&key)
            && Arc::ptr_eq(&e.node, &node)
            && holds(e.crop)
        {
            e.stamp = self.stamp;
            return Some(e.outline.clone());
        }
        // Under its object's id, the stroke's place and the outline's width and tolerance, the key
        // of the stroke's latest outline is kept, except for art made again in every frame.
        let place = node.appearance.items.iter().position(|i| matches!(i, AppearanceItem::Stroke(s) if std::ptr::eq(s, st)));
        let moved = place.filter(|_| !self.made_again).map(|place| (node.id.0, place, width.to_bits(), level));
        let own = moved.is_some() && self.own_path(&node, path);
        if let Some(mk) = moved
            && let Some((outline, crop)) = self.moved_outline(mk, &node, st, line, v)
        {
            self.keep_stroke(key, StrokeEntry { node, outline: outline.clone(), stamp: self.stamp, crop, own }, moved);
            return Some(outline);
        }
        #[cfg(test)]
        STROKES_OUTLINED.with(|n| n.set(n.get() + 1));
        // A long stroke without a width profile is outlined only in and around the view, with a
        // margin of the view's own size on each side ([`crop_dashed`] for a dashed one,
        // [`crop_line`] otherwise).
        let region = v.inflate(v.width(), v.height());
        let tol = 2f64.powi(level);
        if crops()
            && line.elements().len() > CROP_ELEMENTS
            && st.dash.is_some()
            && st.profile.is_none()
            && let Some(outline) = crop_dashed(line, st, width, region, crop_reach(st, width, f.px, tol), tol)
        {
            let outline = Arc::new(outline);
            self.keep_stroke(key, StrokeEntry { node, outline: outline.clone(), stamp: self.stamp, crop: Some(region), own }, moved);
            return Some(outline);
        }
        let cropped = (crops() && line.elements().len() > CROP_ELEMENTS && st.dash.is_none() && st.profile.is_none())
            .then(|| crop_line(line, region, crop_reach(st, width, f.px, tol), width))
            .flatten();
        let (outline, crop) = match cropped {
            Some(part) => (Arc::new(effects::stroke::line_outline(&part, st, width, tol)), Some(region)),
            None => (expand(), None),
        };
        self.keep_stroke(key, StrokeEntry { node, outline: outline.clone(), stamp: self.stamp, crop, own }, moved);
        Some(outline)
    }

    /// Whether `path` is the path that `geom` keeps for `node` ([`Self::path_of`]), the path
    /// [`Node::stroke_path`] makes from its anchors.
    fn own_path(&self, node: &Arc<Node>, path: &BezPath) -> bool {
        self.geom
            .get(&(Arc::as_ptr(node) as usize))
            .is_some_and(|g| Arc::ptr_eq(&g.node, node) && g.path.as_deref().is_some_and(|p| std::ptr::eq(p, path)))
    }

    /// Keeps outline `e` in `strokes` under `key`, and lists `key` as the latest outline under
    /// `moved` ([`MovedKey`]).
    fn keep_stroke(&mut self, key: StrokeKey, e: StrokeEntry, moved: Option<MovedKey>) {
        self.strokes.insert(key, e);
        if let Some(mk) = moved {
            self.moved_strokes.insert(mk, key);
        }
    }

    /// The outline of stroke `st` of `node` along `line`, made from the latest outline kept under
    /// `mk` ([`MovedKey`]) when that outline was made along a line of its object's own path, its
    /// object is `node` changed, its stroke differs from `st` in its paint at most, and its line is
    /// `line` moved (to within rounding): that outline moved by the same offset, with the region it
    /// was cropped to moved too. The anchors of the two objects are compared first
    /// ([`anchors_moved`]), and the earlier line is made again from its object and stroke only when
    /// they match. `None` when there is no such outline, or when its region does not hold the view
    /// `v`.
    fn moved_outline(&self, mk: MovedKey, node: &Node, st: &StrokeLayer, line: &BezPath, v: Rect) -> Option<(Arc<BezPath>, Option<Rect>)> {
        let e = self.strokes.get(self.moved_strokes.get(&mk)?)?;
        if !e.own || e.node.id != node.id {
            return None;
        }
        let Some(AppearanceItem::Stroke(old)) = e.node.appearance.items.get(mk.1) else {
            return None;
        };
        let unpainted = |s: &StrokeLayer| StrokeLayer { paint: vectorcraft_color::Paint::None, ..s.clone() };
        if !std::ptr::eq(old, st) && unpainted(old) != unpainted(st) {
            return None;
        }
        if !anchors_moved(&e.node, node) {
            return None;
        }
        let path = e.node.stroke_path()?;
        let by = offset_between(&effects::stroke::stroke_pieces(&path, old).line, line)?;
        let crop = e.crop.map(|r| r + by);
        if crop.is_some_and(|r| r.x0 > v.x0 || r.y0 > v.y0 || r.x1 < v.x1 || r.y1 < v.y1) {
            return None;
        }
        if by == kurbo::Vec2::ZERO {
            return Some((e.outline.clone(), crop));
        }
        let mut outline = (*e.outline).clone();
        outline.apply_affine(Affine::translate(by));
        Some((Arc::new(outline), crop))
    }

    /// The slices of stroke `st` of `node` along `bp` (filled with `rule`), made from the latest
    /// slices kept under `mk` ([`MovedSliceKey`]) when those were made along their object's own
    /// path, their object is `node` changed, its stroke there differs from `st` at most in where
    /// its gradient is placed, its fill rule is `rule`, and `bp` is its path moved (to within
    /// rounding): the node they were made from, those slices, and the offset from that node to
    /// `bp`, which the slices are drawn moved by. The anchors of the two objects are compared first
    /// ([`anchors_moved`]), and the earlier path is made again from its object only when they
    /// match. `None` when there are no such slices.
    fn moved_slices(
        &self,
        mk: MovedSliceKey,
        node: &Node,
        st: &StrokeLayer,
        rule: FillRule,
        bp: &BezPath,
    ) -> Option<(Arc<Node>, Slices, kurbo::Vec2)> {
        let e = self.stroke_slices.get(self.moved_slices.get(&mk)?)?;
        if !e.own || e.rule != rule || e.made.id != node.id {
            return None;
        }
        let Some(AppearanceItem::Stroke(old)) = e.made.appearance.items.get(mk.1) else {
            return None;
        };
        // The slices take only the gradient's stops and kind, not where it is placed, which a move
        // moves too.
        let unplaced = |s: &StrokeLayer| {
            let mut s = s.clone();
            if let vectorcraft_color::Paint::Gradient(g) = &mut s.paint {
                g.geom = None;
                g.angle = 0.0;
            }
            s
        };
        if !std::ptr::eq(old, st) && unplaced(old) != unplaced(st) {
            return None;
        }
        if !anchors_moved(&e.made, node) {
            return None;
        }
        let by = offset_between(&e.made.stroke_path()?, bp)?;
        Some((e.made.clone(), e.slices.clone(), by))
    }

    /// What the `strokes`, `char_strokes` and `stroke_slices` keys hold for a stroke owner at
    /// `address` (`self.cur` or kept glyph geometry): the address, or 0 while art made again in
    /// every frame is drawn ([`Self::made_again`]).
    fn stroke_owner(&self, address: usize) -> usize {
        if self.made_again { 0 } else { address }
    }
}

/// The stroke lines with more path elements than this are outlined in and around the view only
/// ([`crop_line`]).
pub(crate) const CROP_ELEMENTS: usize = 1024;

#[cfg(test)]
thread_local! {
    /// Whether long strokes are outlined near the view only on this thread ([`crop_line`]).
    pub(crate) static CROP_STROKES: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
    /// How many stroke outlines of nodes [`Renderer::cached_stroke`] has made on this thread.
    pub(crate) static STROKES_OUTLINED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// How many sets of gradient stroke slices [`Renderer::fill_path_gradient`] has made on this
    /// thread.
    pub(crate) static SLICES_MADE: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Paint `p` of a slice ([`Slices`]) moved by `by`: a gradient's start, end and focal point moved.
fn moved_paint(p: &vectorcraft_color::Paint, by: kurbo::Vec2) -> vectorcraft_color::Paint {
    let mut p = p.clone();
    if let vectorcraft_color::Paint::Gradient(g) = &mut p
        && let Some(geom) = &mut g.geom
    {
        geom.start += by;
        geom.end += by;
        if let Some(focal) = &mut geom.focal {
            *focal += by;
        }
    }
    p
}

/// The offset that moves path `was` onto path `now`, when `now` is `was` moved (each point within
/// the rounding that [`Offset::holds`] allows). `None` otherwise, and for an empty path.
pub(crate) fn offset_between(was: &BezPath, now: &BezPath) -> Option<kurbo::Vec2> {
    if !matches!(was.elements().first(), Some(kurbo::PathEl::MoveTo(_))) {
        return None;
    }
    let mut offset = Offset::new();
    (path_pairs(was, now, |a, b| offset.takes(a, b)) && path_pairs(was, now, |a, b| offset.holds(a, b))).then(|| offset.by())
}

/// Whether `was` and `now` have the same elements, and `f` holds for each pair of their points.
/// Stops at the first pair for which `f` does not hold.
fn path_pairs(was: &BezPath, now: &BezPath, mut f: impl FnMut(kurbo::Point, kurbo::Point) -> bool) -> bool {
    use kurbo::PathEl;
    let (was, now) = (was.elements(), now.elements());
    was.len() == now.len()
        && was.iter().zip(now).all(|pair| match pair {
            (PathEl::MoveTo(a), PathEl::MoveTo(b)) | (PathEl::LineTo(a), PathEl::LineTo(b)) => f(*a, *b),
            (PathEl::QuadTo(a0, a1), PathEl::QuadTo(b0, b1)) => f(*a0, *b0) && f(*a1, *b1),
            (PathEl::CurveTo(a0, a1, a2), PathEl::CurveTo(b0, b1, b2)) => f(*a0, *b0) && f(*a1, *b1) && f(*a2, *b2),
            (PathEl::ClosePath, PathEl::ClosePath) => true,
            _ => false,
        })
}

/// One offset between pairs of points. Pairs are read once with [`Offset::takes`], which measures
/// the offset on each axis at the pair with the smallest coordinates on that axis, and once more
/// with [`Offset::holds`], which checks every pair against that offset.
struct Offset {
    by: [f64; 2],
    size: [f64; 2],
}

impl Offset {
    fn new() -> Self {
        Self { by: [0.0; 2], size: [f64::INFINITY; 2] }
    }

    /// Whether `b` is `a` moved by the offset measured so far, on each axis within 3 units in the
    /// last place of the larger of the pair's coordinates and those of the pair the offset was
    /// measured at. A pair with smaller coordinates than any before it on an axis then measures the
    /// offset on that axis.
    fn takes(&mut self, a: kurbo::Point, b: kurbo::Point) -> bool {
        [(a.x, b.x), (a.y, b.y)].into_iter().zip(self.by.iter_mut().zip(&mut self.size)).all(|((a, b), (by, size))| {
            let at = a.abs().max(b.abs());
            if at.is_nan() || at >= *size {
                return (b - a - *by).abs() <= 3.0 * ulp(at);
            }
            let near = size.is_infinite() || (b - a - *by).abs() <= 3.0 * ulp(*size);
            (*by, *size) = (b - a, at);
            near && by.is_finite()
        })
    }

    /// Whether `b` is `a` moved by the offset, on each axis within 3 units in the last place of the
    /// larger of the pair's coordinates: the most that rounding `b`, the pair the offset was measured
    /// at (whose coordinates are no larger), and the two offsets can make together.
    fn holds(&self, a: kurbo::Point, b: kurbo::Point) -> bool {
        let near = |a: f64, b: f64, by: f64| (b - a - by).abs() <= 3.0 * ulp(a.abs().max(b.abs()));
        near(a.x, b.x, self.by[0]) && near(a.y, b.y, self.by[1])
    }

    fn by(&self) -> kurbo::Vec2 {
        kurbo::Vec2::new(self.by[0], self.by[1])
    }
}

/// The distance from `x`, which is 0 or more, to the next larger `f64`.
fn ulp(x: f64) -> f64 {
    f64::from_bits(x.to_bits() + 1) - x
}

/// Whether the anchors and handles of path `now` are those of path `was` moved by one offset,
/// measured as [`offset_between`] measures it, with the same subpaths, anchor counts and closed
/// subpaths. It reads the anchors without making either path ([`Node::stroke_path`]), and stops at
/// the first anchor that differs. `false` for objects other than paths.
pub(crate) fn anchors_moved(was: &Node, now: &Node) -> bool {
    let (vectorcraft_doc::NodeKind::Path { path: was, .. }, vectorcraft_doc::NodeKind::Path { path: now, .. }) = (&was.kind, &now.kind) else {
        return false;
    };
    let mut offset = Offset::new();
    anchor_pairs(was, now, |a, b| offset.takes(a, b)) && anchor_pairs(was, now, |a, b| offset.holds(a, b))
}

/// Whether `was` and `now` have the same subpaths, anchor counts and closed subpaths, and `f` holds
/// for each pair of their anchors and handles. Stops at the first pair for which `f` does not hold.
fn anchor_pairs(was: &vectorcraft_geom::PathData, now: &vectorcraft_geom::PathData, mut f: impl FnMut(kurbo::Point, kurbo::Point) -> bool) -> bool {
    was.subpaths.len() == now.subpaths.len()
        && was.subpaths.iter().zip(&now.subpaths).all(|(a, b)| {
            a.closed == b.closed
                && a.anchors.len() == b.anchors.len()
                && a.anchors.iter().zip(&b.anchors).all(|(a, b)| f(a.p, b.p) && f(a.h_in, b.h_in) && f(a.h_out, b.h_out))
        })
}

fn crops() -> bool {
    #[cfg(test)]
    return CROP_STROKES.with(std::cell::Cell::get);
    #[cfg(not(test))]
    true
}

/// How far stroke `st` at `width` can paint from its path, at a join or a cap, with two output
/// pixels (`px`) and the outline's tolerance `tol` added.
pub(crate) fn crop_reach(st: &StrokeLayer, width: f64, px: f64, tol: f64) -> f64 {
    let corner = if st.join == vectorcraft_doc::LineJoin::Miter { st.miter_limit.max(std::f64::consts::SQRT_2) } else { std::f64::consts::SQRT_2 };
    width / 2.0 * corner + 2.0 * px + tol
}

/// Whether the box of segment `s`'s control points meets `near`. A coordinate that is not a number
/// counts as near.
fn is_near(s: &kurbo::PathSeg, near: Rect) -> bool {
    use kurbo::PathSeg;
    let pts: &[kurbo::Point] = match s {
        PathSeg::Line(l) => &[l.p0, l.p1],
        PathSeg::Quad(q) => &[q.p0, q.p1, q.p2],
        PathSeg::Cubic(c) => &[c.p0, c.p1, c.p2, c.p3],
    };
    !(pts.iter().all(|p| p.x < near.x0)
        || pts.iter().all(|p| p.x > near.x1)
        || pts.iter().all(|p| p.y < near.y0)
        || pts.iter().all(|p| p.y > near.y1))
}

/// The outline of the line `line` of dashed stroke `st` (without a width profile) at `width`,
/// flattened to `tol`, made from the segments within `reach` of `region` only, for drawing in
/// `region`. The dashes and dots on those segments sit where the whole line puts them
/// ([`effects::stroke::dash_kept`]); a pattern that draws the stroke solid is outlined as a solid
/// line along [`crop_line`]. `None` when every segment is kept.
fn crop_dashed(line: &BezPath, st: &StrokeLayer, width: f64, region: Rect, reach: f64, tol: f64) -> Option<BezPath> {
    let near = region.inflate(reach, reach);
    if kurbo::segments(line.elements().iter().copied()).all(|s| is_near(&s, near)) {
        return None;
    }
    if let Some(outline) = effects::stroke::dashed_line_outline_kept(line, st, width, tol, &|s| is_near(s, near)) {
        return Some(outline);
    }
    let solid = StrokeLayer { dash: None, ..st.clone() };
    crop_line(line, region, reach, width).map(|part| effects::stroke::line_outline(&part, &solid, width, tol))
}

/// The stroke line `line` without the segments whose control points' box lies more than `reach`
/// from `region`, for outlining a stroke of `width` that is drawn only in `region`. A subpath that
/// loses some of its segments is replaced by its runs of kept segments, each an open subpath; a
/// closed one is walked from a removed segment, so that a run passes through the point where it
/// starts and ends. A closed subpath that a stroke of `width` can cross over (`width` at least the
/// smaller side of its bounds) keeps all its segments, and so does a subpath with no segments.
/// `None` when every segment is kept.
pub(crate) fn crop_line(line: &BezPath, region: Rect, reach: f64, width: f64) -> Option<BezPath> {
    use kurbo::{PathEl, PathSeg};
    let near = region.inflate(reach, reach);
    let keep = |s: &PathSeg| is_near(s, near);
    let els = line.elements();
    let mut starts: Vec<usize> = els.iter().enumerate().filter(|(_, e)| matches!(e, PathEl::MoveTo(_))).map(|(i, _)| i).collect();
    if starts.first() != Some(&0) {
        starts.insert(0, 0);
    }
    let mut out = BezPath::new();
    let mut cut = false;
    for (k, &start) in starts.iter().enumerate() {
        let Some(sub) = els.get(start..starts.get(k + 1).copied().unwrap_or(els.len())) else { continue };
        let segs = || kurbo::segments(sub.iter().copied());
        let (total, kept) = segs().fold((0, 0), |(t, n), s| (t + 1, n + usize::from(keep(&s))));
        let closed = sub.iter().any(|e| matches!(e, PathEl::ClosePath));
        let crosses = || {
            let b = BezPath::from_vec(sub.to_vec()).bounding_box();
            width >= b.width().min(b.height())
        };
        if kept == total || closed && crosses() {
            out.extend(sub.iter().copied());
            continue;
        }
        cut = true;
        let first = if closed { segs().position(|s| !keep(&s)).map_or(0, |i| i + 1) } else { 0 };
        let mut open = false;
        for s in segs().skip(first).chain(segs().take(first)) {
            if !keep(&s) {
                open = false;
                continue;
            }
            if !open {
                out.move_to(match s {
                    PathSeg::Line(l) => l.p0,
                    PathSeg::Quad(q) => q.p0,
                    PathSeg::Cubic(c) => c.p0,
                });
                open = true;
            }
            match s {
                PathSeg::Line(l) => out.line_to(l.p1),
                PathSeg::Quad(q) => out.quad_to(q.p1, q.p2),
                PathSeg::Cubic(c) => out.curve_to(c.p1, c.p2, c.p3),
            }
        }
    }
    cut.then_some(out)
}
