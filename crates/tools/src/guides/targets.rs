use super::snap::{Leave, meeting, seg_direction, touch_params};
use super::{
    Affine, Arc, BezPath, Document, FontDb, GlyphSnap, GuideStyle, Handle, HashSet, Line, Node, NodeId, NodeKind, OrientedBox, Overlay, ParamCurve,
    ParamCurveNearest, PathSeg, Point, Rect, Ruler, Selection, Shape, SubPath, ToolContext, Vec2, layout_text,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    Anchor,
    Center,
    Edge,
    Artboard,
    Bleed,
    Guide,
    Glyph,
}

impl Kind {
    fn label(self) -> &'static str {
        match self {
            Kind::Anchor => "anchor",
            Kind::Center => "center",
            Kind::Edge => "path",
            Kind::Artboard => "artboard",
            Kind::Bleed => "bleed",
            Kind::Guide => "guide",
            Kind::Glyph => "glyph",
        }
    }
}

/// Snap targets gathered from the document (excluding the objects being edited).
#[derive(Default)]
pub struct Targets {
    pub(super) points: Vec<(Point, Kind)>,
    pub(super) xs: Vec<(f64, Point, Kind)>,
    pub(super) ys: Vec<(f64, Point, Kind)>,
    /// Path segments and their control bounds, which a point lands on ("path"): gathered only for
    /// dragged anchors ([`Self::for_anchor_drag`]).
    pub(super) segments: Vec<(Rect, PathSeg)>,
    /// Ruler guides, which pull a point into line where they run (Snap to Point).
    pub(super) rulers: Vec<Ruler>,
    /// The bounds of the objects a moved selection spaces itself evenly with (Spacing Guides):
    /// gathered only for moves ([`Self::for_move`]).
    pub(super) boxes: Vec<Rect>,
    /// Line Extension: the lines a single dragged anchor keeps to, along the straight segments
    /// it ends, as the end that stays put and the unit direction towards the dragged one
    /// ([`Self::for_anchor_drag`]).
    pub(super) extensions: Vec<(Point, Vec2)>,
    /// Snap to Glyph's guides that run at an angle (in rotated type, or along a glyph's slanted
    /// segments), as a point on each and its unit direction: a drawn point lands on the nearest
    /// ([`Self::for_drawing`]).
    pub(super) angled: Vec<(Point, Vec2)>,
    /// Snap to Tangent and Snap to Perpendicular: a drawn line from an anchor lands where it
    /// grazes a curve, or meets a segment square ([`Self::touching`]).
    pub(super) tangent: bool,
    pub(super) perpendicular: bool,
    /// How the guides these targets produce look.
    pub(super) style: GuideStyle,
}

/// How many anchors, and how many segments, the targets gather at most.
const BUDGET: usize = 20_000;

/// What [`Targets::collect_inner`] gathers from each path besides its anchors and bounds.
#[derive(Clone, Copy, PartialEq)]
enum Gather {
    /// Nothing more: a moved or resized selection lines up with bounds.
    Bounds,
    /// Its segments, which a dragged anchor lands on.
    Segments,
    /// Its segments, and its anchors as lines too: a drawn point lines up with them.
    Drawing,
}

/// The entries of `sorted` (ascending by `key`) whose key is within `tol` of `v`, found by
/// bisection.
fn within<T>(sorted: &[T], key: impl Fn(&T) -> f64, v: f64, tol: f64) -> &[T] {
    let lo = sorted.partition_point(|e| key(e) < v - tol);
    let hi = sorted.partition_point(|e| key(e) <= v + tol);
    sorted.get(lo..hi).unwrap_or_default()
}

/// Visit `nodes` and everything inside them, but the nodes in `skip` and everything inside those:
/// one pass, however many are skipped. A Shaper Group's original shapes, its first child, are
/// not shown and are skipped too.
fn walk_but<'a>(nodes: &'a [Arc<Node>], skip: &HashSet<NodeId>, f: &mut impl FnMut(&'a Node)) {
    for n in nodes.iter().filter(|n| !skip.contains(&n.id)) {
        f(n);
        if let Some(ch) = n.children() {
            walk_but(ch.get(usize::from(n.extras().shaper.is_some())..).unwrap_or_default(), skip, f);
        }
    }
}

/// The lines of `lines` (sorted by where they run) within `tol` of `v`.
pub(super) fn lines_near(lines: &[(f64, Point, Kind)], v: f64, tol: f64) -> &[(f64, Point, Kind)] {
    within(lines, |l| l.0, v, tol)
}

/// `r`'s extent along the x axis (`x`) or the y axis.
pub(super) fn span(r: &Rect, x: bool) -> (f64, f64) {
    if x { (r.x0, r.x1) } else { (r.y0, r.y1) }
}

/// Along axis `x`, does dragging `handle` move the box's minimum edge (true) or maximum (false)?
fn resize_moves_min_edge(handle: Handle, x: bool) -> Option<bool> {
    let (ax, ay) = handle.axes();
    if x && !ax || !x && !ay {
        return None;
    }
    if x {
        Some(matches!(handle, Handle::Left | Handle::TopLeft | Handle::BottomLeft))
    } else {
        Some(matches!(handle, Handle::Top | Handle::TopLeft | Handle::TopRight))
    }
}

/// The objects a moved selection (`moving`) spaces itself evenly with, by their bounds: each
/// visible object on a layer as a whole (a group too), and inside a group holding some of the
/// moving objects its other objects, but a Shaper Group's original shapes. At most [`BUDGET`].
pub(super) fn object_boxes(doc: &Document, moving: &[NodeId]) -> Vec<Rect> {
    fn push(nodes: &[Arc<Node>], moving: &HashSet<NodeId>, holders: &HashSet<NodeId>, out: &mut Vec<Rect>) {
        for n in nodes.iter().filter(|n| n.visible && !moving.contains(&n.id)) {
            if out.len() >= BUDGET {
                return;
            }
            match n.children() {
                Some(ch) if n.is_layer() || holders.contains(&n.id) => {
                    push(ch.get(usize::from(n.extras().shaper.is_some())..).unwrap_or_default(), moving, holders, out)
                }
                _ => out.extend(n.geometric_bounds()),
            }
        }
    }
    let holders: HashSet<NodeId> = moving.iter().filter_map(|id| doc.ancestry(*id)).flatten().collect();
    let mut out = vec![];
    push(&doc.layers, &moving.iter().copied().collect(), &holders, &mut out);
    out
}

impl Targets {
    pub fn collect(doc: &Document, exclude: &[NodeId], visible: Option<Rect>) -> Self {
        Self::collect_inner(doc, exclude, None, visible, Gather::Bounds)
    }

    /// Targets for moving the selection of `cx`: [`Self::collect`]'s but `exclude` (the selection,
    /// or nothing for a copy, which leaves it behind), styled as `cx` asks, and with Smart Guides ›
    /// Spacing Guides the objects it spaces itself evenly with ([`Self::snap_rect`]).
    pub fn for_move(cx: &ToolContext, exclude: &[NodeId]) -> Self {
        let mut t = Self::collect(cx.doc, exclude, None).styled(cx);
        if cx.spacing_guides {
            t.boxes = object_boxes(cx.doc, &cx.selection.objects);
        }
        t
    }

    /// Targets for scaling the selection: the other art, styled as `cx` asks, and with Smart Guides ›
    /// Spacing Guides the neighbours whose gaps the moving edge should match ([`Self::snap_scale`]).
    pub fn for_scale(cx: &ToolContext) -> Self {
        Self::for_move(cx, &cx.selection.objects)
    }

    /// Targets for a drawn point: the art in `visible` (all of it for None) but `exclude`, its
    /// anchors, centres, edges and path segments, its anchors in line too. The anchors and
    /// segments of `exclude` (the path being drawn, whose bounds grow with it) stay targets. With
    /// `glyphs` (Snap to Glyph on), live text gives the guides those options ask for.
    pub fn for_drawing(doc: &Document, exclude: &[NodeId], visible: Option<Rect>, glyphs: Option<GlyphSnap>) -> Self {
        let mut t = Self::collect_inner(doc, exclude, None, visible, Gather::Drawing);
        t.push_paths(doc, exclude, true);
        if let Some(o) = glyphs {
            t.push_glyphs(doc, exclude, visible, o);
        }
        t.index()
    }

    /// Targets for a dragged direction handle of path `id`: the other art, and the anchors of `id`
    /// (its bounds and the segments through the handle move with it, so they would chase it).
    pub fn for_handle(doc: &Document, id: NodeId) -> Self {
        let mut t = Self::collect_inner(doc, &[id], None, None, Gather::Bounds);
        t.push_paths(doc, &[id], false);
        t.index()
    }

    /// The anchors of the paths `ids` as targets, on them and in line, and their segments too
    /// when `segments`: at most [`BUDGET`] of each.
    fn push_paths(&mut self, doc: &Document, ids: &[NodeId], segments: bool) {
        let (mut anchors, mut segment_budget) = (BUDGET, if segments { BUDGET } else { 0 });
        for path in ids.iter().filter_map(|id| doc.node(*id)?.path_data()) {
            for sp in &path.subpaths {
                for a in sp.anchors.iter().take(anchors) {
                    self.push_anchor(a.p);
                    anchors -= 1;
                }
                self.push_segments(sp, |_| false, &mut segment_budget);
            }
        }
    }

    /// Sort the targets so that [`within`] finds those near a point by bisection: the anchors
    /// and centres by x, the lines by where they run. Every constructor ends with it (the sort
    /// is stable: of targets alike, the first gathered still wins).
    fn index(mut self) -> Self {
        self.points.sort_by(|a, b| a.0.x.total_cmp(&b.0.x));
        self.xs.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.ys.sort_by(|a, b| a.0.total_cmp(&b.0));
        self
    }

    /// Anchor `a` as a target: a point lands on it and lines up with it.
    fn push_anchor(&mut self, a: Point) {
        self.points.push((a, Kind::Anchor));
        self.xs.push((a.x, a, Kind::Anchor));
        self.ys.push((a.y, a, Kind::Anchor));
    }

    /// A glyph guide through `at` along `dir` (on the page): a line the art lines up with when it
    /// runs across or down the page, else (with `angular`, Angular Guides) an angled line a point
    /// lands on.
    fn push_glyph_line(&mut self, at: Point, dir: Vec2, angular: bool) {
        let len = dir.hypot();
        if !(len.is_finite() && len > 1e-12 && at.is_finite()) {
            return;
        }
        let d = dir / len;
        if d.y.abs() <= 1e-9 {
            self.ys.push((at.y, at, Kind::Glyph));
        } else if d.x.abs() <= 1e-9 {
            self.xs.push((at.x, at, Kind::Glyph));
        } else if angular {
            self.angled.push((at, d));
        }
    }

    /// Live text as View › Snap to Glyph shows it: the glyph bounds of every text object, and for
    /// a single line of type also its baseline and x-height, the proximity guides at the edges of
    /// each character's advance width, and its glyphs' anchor points. In rotated type these
    /// guides run at its angle (angular guides), and type of one glyph also has angular guides
    /// along the glyph's slanted straight segments. Each kind of guide is there only while its
    /// option `o` is on.
    fn push_glyphs(&mut self, doc: &Document, exclude: &[NodeId], visible: Option<Rect>, o: GlyphSnap) {
        let skip: HashSet<NodeId> = exclude.iter().copied().collect();
        let db = FontDb::global();
        let kind = Kind::Glyph;
        let mut budget = BUDGET / 4;
        let (mut anchors, mut angular) = (BUDGET, BUDGET / 4);
        walk_but(&doc.layers, &skip, &mut |n| {
            if budget == 0 || !n.visible {
                return;
            }
            let NodeKind::Text(t) = &n.kind else { return };
            let lay = layout_text(db, t);
            let ink = t.xf.transform_rect_bbox(lay.bounds);
            if !ink.is_finite() || ink.width() <= 0.0 || ink.height() <= 0.0 {
                return;
            }
            if let Some(v) = visible
                && ink.intersect(v).area() <= 0.0
                && !v.contains(ink.center())
            {
                return;
            }
            // A paragraph shows glyph bounds only.
            let single = lay.lines.len() == 1;
            // Angular guides along a glyph's slanted segments: type of one glyph only.
            let one_glyph = lay.glyphs.iter().filter(|g| !g.outline.elements().is_empty()).count() == 1;
            // Text space → the page, and a direction in text space → one on the page.
            let to_doc = |p: Point| t.xf * p;
            let along = |m: Affine, v: Vec2| (m * v.to_point()) - (m * Point::ZERO);
            if single && o.baseline {
                let line_doc = t.xf * lay.line_xf;
                for line in &lay.lines {
                    self.push_glyph_line(line_doc * Point::new(line.x0, line.baseline), along(line_doc, Vec2::new(1.0, 0.0)), o.angular);
                }
            }
            // The last x-height pushed, to push each run of glyphs alike once.
            let mut last_x_height: Option<(Point, Vec2)> = None;
            for g in &lay.glyphs {
                if budget == 0 {
                    break;
                }
                // Glyphs that draw nothing (spaces) give no guides.
                let bb = g.outline.bounding_box();
                if !(bb.width() > 0.0 && bb.height() > 0.0) {
                    continue;
                }
                budget -= 1;
                // Horizontal type and type on a path: the baseline runs from where the pen stood
                // along the glyph's angle (not the glyph's own drawing, which Character Rotation
                // turns), up being to its left.
                if single
                    && !lay.vertical
                    && let Some(face) = db.face_by_id(g.font_id)
                {
                    let ahead = Vec2::new(g.angle.cos(), g.angle.sin());
                    let up = Vec2::new(ahead.y, -ahead.x);
                    let dir = along(t.xf, ahead);
                    // The x-height of the glyph as placed without its Character Rotation (raised or
                    // lowered by its baseline shift), measured across the baseline.
                    let top = g.xf * g.unturned * Point::new(0.0, -face.x_height());
                    let at = to_doc(g.origin + up * (top - g.origin).dot(up));
                    let same = last_x_height.is_some_and(|(a, d)| {
                        let (l, k) = (d.hypot(), dir.hypot());
                        l > 1e-12 && k > 1e-12 && (d / l - dir / k).hypot() <= 1e-9 && ((at - a).cross(d / l)).abs() <= 1e-6
                    });
                    if o.x_height && face.x_height() > 0.0 && !same {
                        self.push_glyph_line(at, dir, o.angular);
                        last_x_height = Some((at, dir));
                    }
                    // Proximity guides: across the baseline at both edges of the width the layout
                    // gives the character (its advance with tracking and kerning).
                    if o.proximity {
                        for e in [g.origin, g.origin + ahead * g.advance] {
                            self.push_glyph_line(to_doc(e), along(t.xf, up), o.angular);
                        }
                    }
                }
                if single {
                    for seg in g.outline.segments() {
                        if o.anchors && anchors > 0 {
                            self.points.push((to_doc(seg.start()), Kind::Anchor));
                            anchors -= 1;
                        }
                        if o.angular
                            && one_glyph
                            && angular > 0
                            && let PathSeg::Line(l) = seg
                        {
                            let (p, q) = (to_doc(l.p0), to_doc(l.p1));
                            let d = q - p;
                            let len = d.hypot();
                            if len > 1e-6 && d.x.abs() > 1e-9 * len && d.y.abs() > 1e-9 * len {
                                self.push_glyph_line(p, d, true);
                                angular -= 1;
                            }
                        }
                    }
                }
                if o.bounds {
                    let r = t.xf.transform_rect_bbox(bb);
                    for p in [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)] {
                        self.xs.push((p.x, p, kind));
                        self.ys.push((p.y, p, kind));
                    }
                }
            }
        });
    }

    /// The same targets drawing their guides as the Smart Guides preferences in `cx` ask
    /// ([`GuideStyle::of`]).
    pub fn styled(mut self, cx: &ToolContext) -> Self {
        self.style = GuideStyle::of(cx);
        self
    }

    /// An alignment line from `a` to `b`, unless Alignment Guides are off.
    fn line(&self, a: Point, b: Point) -> Option<Overlay> {
        self.style.lines.then_some(Overlay::Line { a, b, color: self.style.color, dashed: false })
    }

    /// A label `text` at `p`, unless Anchor/Path Labels are off.
    fn label(&self, p: Point, text: &str) -> Option<Overlay> {
        self.style.labels.then(|| Overlay::Label { p, text: text.into(), color: self.style.color })
    }

    /// Targets for dragging artboard `index`: everything except that artboard and `exclude` (the
    /// art that moves along with it).
    pub fn for_artboard(doc: &Document, index: usize, exclude: &[NodeId]) -> Self {
        Self::collect_inner(doc, exclude, Some(index), None, Gather::Bounds)
    }

    /// Targets for dragging the direct-selected anchors of `sel` (and its objects selected as a
    /// whole): the other art, its path segments too, and the anchors and segments that stay put on
    /// the paths being reshaped (their bounds move along, so they are no targets). A single anchor
    /// also keeps to the extensions of the straight segments it ends (#2120).
    pub fn for_anchor_drag(doc: &Document, sel: &Selection) -> Self {
        let mut t = Self::collect_inner(doc, &sel.objects, None, None, Gather::Segments);
        let (mut anchors, mut segments) = (BUDGET, BUDGET);
        // With several anchors moving, the one under the pointer is not known here, and the
        // others' extensions don't run through it.
        let single = sel.objects.len() <= 1 && sel.anchors.values().map(|s| s.len()).sum::<usize>() == 1;
        // A path inside another selected object, such as a path in a selected group, moves whole
        // with that object, so none of its anchors or segments stay put.
        let selected: HashSet<NodeId> = sel.objects.iter().copied().collect();
        let inside_selected = |id: NodeId| doc.ancestry(id).is_some_and(|a| a.iter().rev().skip(1).any(|p| selected.contains(p)));
        for (id, set) in sel.anchors.iter().filter(|(id, _)| !inside_selected(**id)) {
            let Some(path) = doc.node(*id).and_then(|n| n.path_data()) else { continue };
            for (si, sp) in path.subpaths.iter().enumerate() {
                let moving = |ai: usize| set.contains(&(si, ai));
                for a in sp.anchors.iter().enumerate().filter(|(ai, _)| !moving(*ai)).map(|(_, a)| a.p).take(anchors) {
                    t.push_anchor(a);
                    anchors -= 1;
                }
                t.push_segments(sp, moving, &mut segments);
                if single {
                    t.push_extensions(sp, moving);
                }
            }
        }
        t.index()
    }

    /// The extensions of the straight segments of `sp` with one end among the anchors `moving`
    /// accepts and the other staying put. Horizontal and vertical ones are left out: lining up
    /// with the other end already keeps the point on them, and says "align".
    fn push_extensions(&mut self, sp: &SubPath, moving: impl Fn(usize) -> bool) {
        let n = sp.anchors.len();
        for seg in (0..sp.segment_count()).filter(|&seg| moving(seg) != moving((seg + 1) % n) && sp.segment_is_line(seg)) {
            let c = sp.segment(seg);
            let (fixed, dragged) = if moving(seg) { (c.p3, c.p0) } else { (c.p0, c.p3) };
            let d = dragged - fixed;
            let len = d.hypot();
            if len.is_finite() && d.x.abs() > 1e-9 * len && d.y.abs() > 1e-9 * len {
                self.extensions.push((fixed, d / len));
            }
        }
    }

    /// The segments of `sp` that stay put while the anchors `moving` accepts move, as targets, at
    /// most `budget` of them (counted down).
    fn push_segments(&mut self, sp: &SubPath, moving: impl Fn(usize) -> bool, budget: &mut usize) {
        let n = sp.anchors.len();
        for seg in 0..sp.segment_count() {
            if *budget == 0 {
                return;
            }
            if moving(seg) || moving((seg + 1) % n) {
                continue;
            }
            let c = sp.segment(seg);
            let bounds = Rect::from_points(c.p0, c.p1).union_pt(c.p2).union_pt(c.p3);
            // A straight one as a line, so the point lands on it exactly.
            self.segments.push((bounds, if sp.segment_is_line(seg) { PathSeg::Line(Line::new(c.p0, c.p3)) } else { PathSeg::Cubic(c) }));
            *budget -= 1;
        }
    }

    fn collect_inner(doc: &Document, exclude: &[NodeId], skip_artboard: Option<usize>, visible: Option<Rect>, gather: Gather) -> Self {
        let mut t = Targets::default();
        let add_rect = |t: &mut Targets, r: Rect, kind: Kind| {
            let c = r.center();
            for p in [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)] {
                t.xs.push((p.x, p, kind));
                t.ys.push((p.y, p, kind));
            }
            // A bleed shares its artboard's centre.
            if kind != Kind::Bleed {
                t.points.push((c, Kind::Center));
                t.xs.push((c.x, c, Kind::Center));
                t.ys.push((c.y, c, Kind::Center));
            }
        };
        for (_, ab) in doc.artboards.iter().enumerate().filter(|(i, _)| Some(*i) != skip_artboard) {
            add_rect(&mut t, ab.rect, Kind::Artboard);
            if doc.setup.has_bleed() {
                add_rect(&mut t, doc.setup.bleed_rect(ab.rect), Kind::Bleed);
            }
        }
        let (mut budget, mut segment_budget) = (BUDGET, BUDGET);
        walk_but(&doc.layers, &exclude.iter().copied().collect(), &mut |n| {
            if budget == 0 || n.is_container() || !n.visible {
                return;
            }
            let Some(b) = n.geometric_bounds() else { return };
            if let Some(v) = visible
                && b.intersect(v).area() <= 0.0
                && !v.contains(b.center())
            {
                return;
            }
            if let NodeKind::Path { path, .. } = &n.kind {
                for (_, _, a) in path.anchors() {
                    if gather == Gather::Drawing {
                        t.push_anchor(a.p);
                    } else {
                        t.points.push((a.p, Kind::Anchor));
                    }
                    budget = budget.saturating_sub(1);
                }
                if gather != Gather::Bounds {
                    for sp in &path.subpaths {
                        t.push_segments(sp, |_| false, &mut segment_budget);
                    }
                }
            }
            add_rect(&mut t, b, Kind::Edge);
        });
        t.index()
    }

    /// Only these targets' anchors pull (View → Snap to Point).
    pub(crate) fn anchors_only(mut self) -> Self {
        self.points.retain(|(_, k)| *k == Kind::Anchor);
        self.xs.clear();
        self.ys.clear();
        self.segments.clear();
        self.extensions.clear();
        self.angled.clear();
        self
    }

    /// View → Snap to Point: only these targets' anchors pull, and the ruler guides (shown and
    /// unlocked, as the selection tools pick them) pull the pointer into line where they run.
    pub(super) fn for_snap_to_point(self, cx: &ToolContext) -> Self {
        self.anchors_only().with_rulers(cx)
    }

    /// These targets and the ruler guides, when they are shown (and so unlocked to the selection
    /// tools): they pull a point into line where they run.
    pub(super) fn with_rulers(mut self, cx: &ToolContext) -> Self {
        if cx.guides {
            self.rulers = cx
                .doc
                .guides
                .iter()
                .filter(|g| cx.doc.guide_shown(g))
                .map(|g| Ruler { vertical: g.vertical, pos: g.pos, span: cx.doc.guide_span(g) })
                .collect();
        }
        self
    }

    /// The ruler guides alone ([`Self::with_rulers`]): what a dragged selection's grabbed point
    /// still snaps to with View → Snap to Point when Smart Guides line the selection up.
    pub(crate) fn rulers(cx: &ToolContext) -> Self {
        Self::default().styled(cx).with_rulers(cx)
    }

    /// What a dragged selection's grabbed point snaps to with View → Snap to Point and Smart
    /// Guides off: the anchors of the art but `exclude` and the ruler guides.
    pub fn snap_to_point(cx: &ToolContext, exclude: &[NodeId]) -> Option<Self> {
        (cx.snap_to_point && !cx.smart_guides).then(|| Self::collect(cx.doc, exclude, None).for_snap_to_point(cx))
    }

    /// The gridlines (one every `step` from the page's origin, as
    /// [`vectorcraft_geom::snap::snap_to_grid`] places them) at and either side of the nearest one
    /// to each of `points`, as lines to line up with.
    pub(super) fn grid_lines(points: impl IntoIterator<Item = Point>, step: f64) -> Self {
        let mut t = Targets::default();
        let line = |v: f64, k: i32| vectorcraft_geom::snap::snap_to_grid(v, step, 0.0) + f64::from(k) * step;
        for p in points {
            for k in -1..=1 {
                let (x, y) = (line(p.x, k), line(p.y, k));
                t.xs.push((x, Point::new(x, p.y), Kind::Guide));
                t.ys.push((y, Point::new(p.x, y), Kind::Guide));
            }
        }
        t.index()
    }

    /// Anchors and centres of the visible leaves under `roots` (the roots included) whose bounds
    /// reach into `near`.
    pub(super) fn points_near(doc: &Document, roots: &[NodeId], near: Rect) -> Self {
        let mut t = Targets::default();
        for n in roots.iter().filter_map(|id| doc.node(*id)) {
            crate::draw2::walk_shown(n, &mut |c| {
                if c.is_container() || !c.visible {
                    return;
                }
                let Some(b) = c.geometric_bounds() else { return };
                if b.x0 > near.x1 || b.x1 < near.x0 || b.y0 > near.y1 || b.y1 < near.y0 {
                    return;
                }
                if let NodeKind::Path { path, .. } = &c.kind {
                    t.points.extend(path.anchors().map(|(_, _, a)| (a.p, Kind::Anchor)));
                }
                t.points.push((b.center(), Kind::Center));
            });
        }
        t.index()
    }

    /// Snap a single point: onto the nearest anchor or centre, else onto the nearest segment, else
    /// into line with targets. Returns the snapped point and guide overlays.
    pub fn snap_point(&self, p: Point, tol: f64) -> (Point, Vec<Overlay>) {
        self.snap_from(p, None, tol)
    }

    /// Snap a drawn point `p` as [`Self::snap_point`] does, and when it leaves `from` (the anchor
    /// before it, a shape's start): with Shift held it keeps to its angles round it, sliding along
    /// that way into line with the nearest target; else, before lining up, it lands on a
    /// construction guide through it, where a target's line crosses the guide if one does nearby.
    pub fn snap_from(&self, p: Point, from: Option<&Leave>, tol: f64) -> (Point, Vec<Overlay>) {
        if let Some(f) = from
            && let Some(q) = f.constrained(p)
        {
            return match self.slide(f.at, q - f.at, q, tol, true) {
                Some((q, target)) => (q, self.lined_up(q, target)),
                None => (q, vec![]),
            };
        }
        if let Some((q, k)) = self.nearest_point(p, tol) {
            return (q, self.label(q, k.label()).into_iter().collect());
        }
        if let Some(q) = self.crossing(p, tol) {
            return (q, self.label(q, "intersect").into_iter().collect());
        }
        if let Some(f) = from.filter(|f| f.contact)
            && let Some((q, perpendicular, dir)) = self.touching(f.at, p, tol)
        {
            return (q, self.touch_guides(f.at, q, perpendicular, dir, tol));
        }
        if let Some(q) = self.on_segment(p, tol) {
            return (q, self.label(q, Kind::Edge.label()).into_iter().collect());
        }
        // A construction guide through the point it leaves, else a dragged segment's extension:
        // it lands on the guide, sliding into line with a target crossing it nearby.
        if let Some((at, dir, q)) = from
            .and_then(|f| f.construction(p, tol).map(|(dir, q)| (f.at, dir, q)))
            .or_else(|| self.extension(p, tol))
            .or_else(|| self.on_angled(p, tol))
        {
            let construction = |q: Point| Overlay::Line { a: at, b: q, color: self.style.color, dashed: false };
            return match self.slide(at, dir, q, tol, false) {
                Some((q, target)) => (q, std::iter::once(construction(q)).chain(self.lined_up(q, target)).collect()),
                None => (q, vec![construction(q)]),
            };
        }
        let mut out = p;
        let mut ov = vec![];
        // The targets' lines along one axis and the ruler guides running past `p`, the nearest
        // within reach of `v`.
        let nearest = |lines: &[(f64, Point, Kind)], vertical: bool, v: f64| {
            let rulers = self.rulers.iter().filter(|r| r.vertical == vertical).filter_map(|r| r.line_at(p, tol));
            lines_near(lines, v, tol)
                .iter()
                .copied()
                .chain(rulers)
                .filter(|(t, ..)| (t - v).abs() <= tol)
                .min_by(|a, b| (a.0 - v).abs().total_cmp(&(b.0 - v).abs()))
        };
        let mut aligned = false;
        if let Some((x, from, _)) = nearest(&self.xs, true, p.x) {
            out.x = x;
            aligned = true;
            ov.extend(self.line(from, Point::new(x, p.y)));
        }
        if let Some((y, from, _)) = nearest(&self.ys, false, p.y) {
            out.y = y;
            aligned = true;
            ov.extend(self.line(from, Point::new(p.x, y)));
        }
        if aligned {
            ov.extend(self.label(out, "align"));
        }
        (out, ov)
    }

    /// Where a line from `at` through a point near `p` touches a target segment: square to it
    /// (`perpendicular`, the foot of the perpendicular from `at`) or just grazing a curve
    /// (`tangent`), within `tol` of `p`. The point, whether it is the perpendicular one, and the
    /// segment's direction there. Targets gather segments for drawn points and dragged anchors only.
    pub(super) fn touching(&self, at: Point, p: Point, tol: f64) -> Option<(Point, bool, Vec2)> {
        if !(self.tangent || self.perpendicular) || ![at.x, at.y, p.x, p.y, tol].iter().all(|v| v.is_finite()) {
            return None;
        }
        let mut best: Option<(f64, Point, bool, Vec2)> = None;
        // Every segment in reach is tried: their order is the document's, not the distance.
        for (_, seg) in self.segments.iter().filter(|(b, _)| b.inflate(tol, tol).contains(p)) {
            let curved = !matches!(seg, PathSeg::Line(_));
            for (perpendicular, wanted) in [(true, self.perpendicular && !curved), (false, self.tangent && curved)] {
                if !wanted {
                    continue;
                }
                for t in touch_params(seg, at, perpendicular) {
                    let (q, d) = (seg.eval(t), seg_direction(seg, t));
                    let dist = q.distance(p);
                    if dist <= tol && q.distance(at) > tol && best.is_none_or(|b| dist < b.0) {
                        best = Some((dist, q, perpendicular, d));
                    }
                }
            }
        }
        best.map(|(_, q, perpendicular, d)| (q, perpendicular, d))
    }

    /// The guides of a point landed where the line from `at` touches a segment at `q` running
    /// along `dir`: the line itself, a square corner mark for the perpendicular, and the label.
    fn touch_guides(&self, at: Point, q: Point, perpendicular: bool, dir: Vec2, tol: f64) -> Vec<Overlay> {
        let mut out: Vec<Overlay> = self.line(at, q).into_iter().collect();
        let (u, v) = (at - q, dir);
        if perpendicular && self.style.lines && u.hypot2() > 1e-18 && v.hypot2() > 1e-18 {
            let (u, v) = (u / u.hypot(), v / v.hypot());
            let s = tol * 1.5;
            let mut mark = BezPath::new();
            mark.move_to(q + u * s);
            mark.line_to(q + u * s + v * s);
            mark.line_to(q + v * s);
            out.push(Overlay::Path { path: mark, color: self.style.color, width: 1.0, dashed: false });
        }
        out.extend(self.label(q, if perpendicular { "perpendicular" } else { "tangent" }));
        out
    }

    /// The segment extension nearest `p`, within `tol` of it (and `p` farther than that from the
    /// end it starts at): that end, its direction and where `p` lands on it. Where `p` is nearer
    /// the horizontal or the vertical through that end there is none: lining up with it ("align")
    /// is what straightens a segment that is nearly level or upright.
    fn extension(&self, p: Point, tol: f64) -> Option<(Point, Vec2, Point)> {
        self.extensions
            .iter()
            .filter(|(at, _)| at.distance(p) > tol)
            .map(|&(at, d)| (at, d, at + d * (p - at).dot(d)))
            .map(|(at, d, q)| (at, d, q, q.distance(p)))
            .filter(|(at, .., dist)| *dist <= tol && (p.x - at.x).abs().min((p.y - at.y).abs()) > *dist)
            .min_by(|a, b| a.3.total_cmp(&b.3))
            .map(|(at, d, q, _)| (at, d, q))
    }

    /// The angled glyph guide ([`Self::angled`]) nearest `p`, within `tol` of it: a point on it, its
    /// direction and where `p` lands on it.
    fn on_angled(&self, p: Point, tol: f64) -> Option<(Point, Vec2, Point)> {
        self.angled
            .iter()
            .map(|&(at, d)| (at, d, at + d * (p - at).dot(d)))
            .map(|(at, d, q)| (at, d, q, q.distance(p)))
            .filter(|(.., dist)| *dist <= tol)
            .min_by(|a, b| a.3.total_cmp(&b.3))
            .map(|(at, d, q, _)| (at, d, q))
    }

    /// The anchor or centre nearest `p`, within `tol`.
    /// Is a target point (an anchor, a centre) within `tol` of `p`?
    pub(crate) fn has_point_near(&self, p: Point, tol: f64) -> bool {
        self.nearest_point(p, tol).is_some()
    }

    pub(super) fn nearest_point(&self, p: Point, tol: f64) -> Option<(Point, Kind)> {
        within(&self.points, |q| q.0.x, p.x, tol)
            .iter()
            .copied()
            .filter(|(q, _)| q.distance(p) <= tol)
            .min_by(|a, b| a.0.distance(p).total_cmp(&b.0.distance(p)))
    }

    /// Slide `q`, on the line through `at` along `dir` (beyond `at` only, for a `ray`), to where
    /// the nearest target line or angled glyph guide crosses it, at most twice `tol` away: where it lands and the
    /// target's own point (where its line starts).
    fn slide(&self, at: Point, dir: Vec2, q: Point, tol: f64, ray: bool) -> Option<(Point, Point)> {
        if !dir.hypot2().is_finite() || dir.hypot2() <= 1e-18 {
            return None;
        }
        let reach = 2.0 * tol;
        let mut best: Option<(f64, Point, Point)> = None;
        for (vertical, lines, d, a, v) in [(true, &self.xs, dir.x, at.x, q.x), (false, &self.ys, dir.y, at.y, q.y)] {
            if d.abs() <= 1e-9 {
                continue;
            }
            // The ruler guides that way count as lines too (#812).
            let rulers: Vec<(f64, Point, Kind)> = self.rulers.iter().filter(|r| r.vertical == vertical).filter_map(|r| r.line_at(q, reach)).collect();
            // A line that far across the way crosses it at least that far along.
            for (t, from, _) in lines_near(lines, v, reach).iter().chain(&rulers) {
                let s = (t - a) / d;
                // Exactly on the line, whatever the rounding along the way.
                let hit = if vertical { Point::new(*t, at.y + dir.y * s) } else { Point::new(at.x + dir.x * s, *t) };
                let moved = hit.distance(q);
                if moved <= reach && (!ray || s > 0.0) && best.is_none_or(|b| moved < b.0) {
                    best = Some((moved, hit, *from));
                }
            }
        }
        // The angled glyph guides it crosses.
        for &(on, e) in &self.angled {
            let across = dir.cross(e);
            if across.abs() <= 1e-9 * dir.hypot() {
                continue;
            }
            let s = (on - at).cross(e) / across;
            let hit = at + dir * s;
            let moved = hit.distance(q);
            if moved <= reach && (!ray || s > 0.0) && best.is_none_or(|b| moved < b.0) {
                best = Some((moved, hit, on));
            }
        }
        best.map(|(_, hit, from)| (hit, from))
    }

    /// The guides of a point `q` slid into line with the target at `from`: the line from it and
    /// "align", or the target's own label when `q` sits right on it.
    fn lined_up(&self, q: Point, from: Point) -> Vec<Overlay> {
        if let Some((on, k)) = self.nearest_point(q, 1e-9) {
            return self.label(on, k.label()).into_iter().collect();
        }
        self.line(from, q).into_iter().chain(self.label(q, "align")).collect()
    }

    /// The nearest point within `tol` of `p` on a target segment.
    fn on_segment(&self, p: Point, tol: f64) -> Option<Point> {
        self.segments
            .iter()
            .filter(|(b, _)| b.inflate(tol, tol).contains(p))
            .map(|(_, c)| (c, c.nearest(p, 1e-6)))
            .filter(|(_, n)| n.distance_sq <= tol * tol)
            .min_by(|a, b| a.1.distance_sq.total_cmp(&b.1.distance_sq))
            .map(|(c, n)| c.eval(n.t))
    }

    /// Where two target paths, a path and a ruler guide, or two ruler guides cross, the crossing
    /// nearest `p` within `tol` ("intersect", #1602).
    fn crossing(&self, p: Point, tol: f64) -> Option<Point> {
        // A ruler guide running past, as a short line through the reach of `p`.
        let ruler = |r: &Ruler| {
            let (pos, ..) = r.line_at(p, tol)?;
            let (across, along) = if r.vertical { (p.x, p.y) } else { (p.y, p.x) };
            let (a, b) = (along - tol, along + tol);
            ((pos - across).abs() <= tol)
                .then(|| PathSeg::Line(if r.vertical { Line::new((pos, a), (pos, b)) } else { Line::new((a, pos), (b, pos)) }))
        };
        let near: Vec<PathSeg> = self
            .segments
            .iter()
            .filter(|(b, c)| b.inflate(tol, tol).contains(p) && c.nearest(p, 1e-6).distance_sq <= tol * tol)
            .map(|(_, c)| *c)
            .chain(self.rulers.iter().filter_map(ruler))
            // shortcut: the first 32 lines in reach are paired, raise it if busier crossings matter
            .take(32)
            .collect();
        let mut best: Option<Point> = None;
        for (i, a) in near.iter().enumerate() {
            for b in near.iter().skip(i + 1) {
                let hits: Vec<Point> = match (a, b) {
                    (PathSeg::Line(l), c) | (c, PathSeg::Line(l)) => c.intersect_line(*l).iter().map(|x| l.eval(x.line_t)).collect(),
                    _ => meeting(a, b, p).into_iter().collect(),
                };
                for q in hits {
                    if q.distance(p) <= tol && best.is_none_or(|b| q.distance(p) < b.distance(p)) {
                        best = Some(q);
                    }
                }
            }
        }
        best
    }

    /// Snap a ruler guide at `v` (the x of a vertical one, the y of a horizontal one) into line
    /// with the nearest edge, centre or anchor within `tol`: where it goes, and a label there.
    pub fn snap_guide(&self, vertical: bool, v: f64, tol: f64) -> (f64, Vec<Overlay>) {
        let along = |q: &Point| if vertical { q.x } else { q.y };
        let lines = if vertical { &self.xs } else { &self.ys };
        let best = lines_near(lines, v, tol)
            .iter()
            .map(|(t, from, k)| (*t, *from, *k))
            .chain(self.points.iter().map(|(q, k)| (along(q), *q, *k)))
            .filter(|(t, ..)| (t - v).abs() <= tol)
            .min_by(|a, b| (a.0 - v).abs().total_cmp(&(b.0 - v).abs()));
        match best {
            Some((t, from, k)) => (t, self.label(from, k.label()).into_iter().collect()),
            None => (v, vec![]),
        }
    }

    /// Snap a dragged point that carries others along at `offsets` (a resize handle and the
    /// bleed edge beyond it): onto an anchor or centre near the point itself, otherwise into line
    /// with targets, each axis on whichever of them comes nearest.
    pub fn snap_point_with(&self, p: Point, offsets: &[Vec2], tol: f64) -> (Point, Vec<Overlay>) {
        if self.nearest_point(p, tol).is_some() {
            return self.snap_point(p, tol);
        }
        let rects: Vec<Rect> = std::iter::once(Vec2::ZERO).chain(offsets.iter().copied()).map(|o| Rect::from_points(p + o, p + o)).collect();
        let (adj, mut ov, aligned) = self.align_rects(&rects, tol);
        let out = p + adj;
        if aligned.contains(&true) {
            ov.extend(self.label(out, "align"));
        }
        (out, ov)
    }

    /// Snap a moving rectangle (selection bounds after a move by `d`): tries its corners/edges/centre.
    pub fn snap_rect(&self, r: Rect, tol: f64) -> (Vec2, Vec<Overlay>) {
        self.snap_rects(&[r], tol)
    }

    /// Snap a point that may only move along `dir` (the grabbed point of a move Shift keeps to its
    /// line): onto the point of that line nearest an anchor or center within `tol` of `p`, labeled
    /// at the anchor or center, else to where the nearest target line or ruler guide crosses the
    /// line, at most twice `tol` away ([`Self::slide`]). Returns where it goes and the guides.
    pub(crate) fn snap_point_along(&self, p: Point, dir: Vec2, tol: f64) -> (Point, Vec<Overlay>) {
        let l2 = dir.hypot2();
        if !l2.is_finite() || l2 <= 1e-18 {
            return (p, vec![]);
        }
        if let Some((q, k)) = self.nearest_point(p, tol) {
            return (p + dir * ((q - p).dot(dir) / l2), self.label(q, k.label()).into_iter().collect());
        }
        match self.slide(p, dir, p, tol, false) {
            Some((q, from)) => (q, self.lined_up(q, from)),
            None => (p, vec![]),
        }
    }

    /// [`Self::snap_rects`] for rectangles that may only move along `dir` (a move Shift keeps to
    /// its line): they slide along it until the edge or center of one of them is in line with a
    /// target, within `tol` of it on that axis, the target that needs the shortest way along the
    /// line winning. Lining up with nothing, a single rectangle with Spacing Guides slides to where
    /// it is spaced evenly along an axis the line runs along. Returns the shift (along `dir`) and
    /// the guides.
    pub(crate) fn snap_rects_along(&self, rects: &[Rect], dir: Vec2, tol: f64) -> (Vec2, Vec<Overlay>) {
        let len = dir.hypot();
        if !len.is_finite() || len <= 1e-9 {
            return (Vec2::ZERO, vec![]);
        }
        let u = dir / len;
        // The axes the line runs along: the x axis (`true`) and the y axis, and how far a step of
        // one along the line goes on that axis.
        let axes = [(true, u.x), (false, u.y)].into_iter().filter(|(_, c)| c.abs() > 1e-9);
        // The way along the line, the target line it lines up with, where that line starts, and
        // its axis.
        let mut best: Option<(f64, f64, Point, bool)> = None;
        for (x, c) in axes.clone() {
            let lines = if x { &self.xs } else { &self.ys };
            for r in rects {
                let vs = if x { [r.x0, r.center().x, r.x1] } else { [r.y0, r.center().y, r.y1] };
                for v in vs {
                    for (t, from, _) in lines_near(lines, v, tol).iter().filter(|(t, ..)| (t - v).abs() <= tol) {
                        let s = (t - v) / c;
                        if best.is_none_or(|b| s.abs() < b.0.abs()) {
                            best = Some((s, *t, *from, x));
                        }
                    }
                }
            }
        }
        if let Some((s, t, from, x)) = best {
            let ov = rects.iter().map(|r| *r + u * s).reduce(|a, b| a.union(b)).and_then(|m| {
                if x {
                    self.line(Point::new(t, from.y.min(m.y0)), Point::new(t, from.y.max(m.y1)))
                } else {
                    self.line(Point::new(from.x.min(m.x0), t), Point::new(from.x.max(m.x1), t))
                }
            });
            return (u * s, ov.into_iter().collect());
        }
        let [r] = rects else { return (Vec2::ZERO, vec![]) };
        if self.boxes.is_empty() {
            return (Vec2::ZERO, vec![]);
        }
        let spaced =
            axes.filter_map(|(x, c)| self.even_spacing(*r, x, tol).map(|(shift, _)| (shift / c, x))).min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()));
        let Some((s, x)) = spaced else { return (Vec2::ZERO, vec![]) };
        let gaps = self.even_spacing(*r + u * s, x, tol).map(|(_, gaps)| gaps).unwrap_or_default();
        (u * s, gaps.into_iter().flat_map(|g| self.gap_marks(g, x, tol)).collect())
    }

    /// Snap rectangles that move together (an artboard and its bleed): the edge or centre of any
    /// of them nearest a target, per axis. A single moved selection with Spacing Guides
    /// ([`Self::for_move`]) is spaced evenly along an axis that lines up with nothing
    /// ([`Self::even_spacing`]). Returns the shift and the guides.
    pub fn snap_rects(&self, rects: &[Rect], tol: f64) -> (Vec2, Vec<Overlay>) {
        let (mut d, mut ov, aligned) = self.align_rects(rects, tol);
        if let [r] = rects
            && !self.boxes.is_empty()
        {
            let moved = *r + d;
            let mut spaced = false;
            for (x, lined_up) in [true, false].into_iter().zip(aligned) {
                if let Some((shift, _)) = self.even_spacing(moved, x, tol).filter(|_| !lined_up) {
                    *(if x { &mut d.x } else { &mut d.y }) += shift;
                    spaced = true;
                }
            }
            // The equal gaps, marked where the rectangle ends up.
            if spaced {
                let done = *r + d;
                for (x, lined_up) in [true, false].into_iter().zip(aligned) {
                    let gaps = if lined_up { None } else { self.even_spacing(done, x, tol) };
                    ov.extend(gaps.into_iter().flat_map(|(_, gaps)| gaps).flat_map(|g| self.gap_marks(g, x, tol)));
                }
            }
        }
        (d, ov)
    }

    /// Spacing Guides along the x axis (`x`) or the y axis for the moved rectangle `r`, among the
    /// objects of its row (column): the shift (within `tol`) that leaves it as far from its
    /// neighbour before or after it as two other objects of the row are apart, or as far from both
    /// neighbours; and the equal gaps then, as (start, end, where across) along that axis.
    fn even_spacing(&self, r: Rect, x: bool, tol: f64) -> Option<(f64, Vec<[f64; 3]>)> {
        let (r0, r1) = span(&r, x);
        let len = r1 - r0;
        let (c0, c1) = span(&r, !x);
        // The objects beside it, before or after, overlapping it across, by where they start.
        let mut row: Vec<(Rect, f64, f64)> = self
            .boxes
            .iter()
            .map(|b| (*b, span(b, x).0, span(b, x).1))
            .filter(|(b, a0, a1)| span(b, !x).0 < c1 && span(b, !x).1 > c0 && (*a1 <= r0 + tol || *a0 >= r1 - tol))
            .collect();
        row.sort_by(|a, b| a.1.total_cmp(&b.1));
        // Where two boxes overlap across: the middle of that (or of the space between them).
        let mid = |a: &Rect, b: &Rect| (span(a, !x).0.max(span(b, !x).0) + span(a, !x).1.min(span(b, !x).1)) / 2.0;
        // Each object and the nearest one after it, unless the moved rectangle sits between them.
        let pairs: Vec<(f64, [f64; 3])> = row
            .iter()
            .filter_map(|(p, _, p1)| {
                let next = row.partition_point(|(_, a0, _)| *a0 < *p1);
                let (q, q0, _) = row.get(next)?;
                let straddles = *p1 <= r0 + tol && *q0 >= r1 - tol;
                (*q0 > *p1 && !straddles).then(|| (q0 - p1, [*p1, *q0, mid(p, q)]))
            })
            .collect();
        let before = row.iter().filter(|(_, _, a1)| *a1 <= r0 + tol).max_by(|a, b| a.2.total_cmp(&b.2));
        let after = row.iter().filter(|(_, a0, _)| *a0 >= r1 - tol).min_by(|a, b| a.1.total_cmp(&b.1));
        // Where it could start, and the gap it repeats (None: halfway between its neighbours).
        let mut starts: Vec<(f64, Option<f64>)> = vec![];
        if let (Some(a), Some(b)) = (before, after)
            && b.1 - a.2 > len
        {
            starts.push(((a.2 + b.1 - len) / 2.0, None));
        }
        for (g, _) in &pairs {
            starts.extend(before.map(|a| (a.2 + g, Some(*g))));
            starts.extend(after.map(|b| (b.1 - g - len, Some(*g))));
        }
        let (start, repeats) =
            starts.into_iter().filter(|(t, _)| (t - r0).abs() <= tol).min_by(|a, b| (a.0 - r0).abs().total_cmp(&(b.0 - r0).abs()))?;
        let at = r + if x { Vec2::new(start - r0, 0.0) } else { Vec2::new(0.0, start - r0) };
        let (end, mut gaps) = (start + len, vec![]);
        if let Some((a, _, a1)) = before
            && repeats.is_none_or(|g| (start - a1 - g).abs() <= 1e-9)
        {
            gaps.push([*a1, start, mid(a, &at)]);
        }
        if let Some((b, b0, _)) = after
            && repeats.is_none_or(|g| (b0 - end - g).abs() <= 1e-9)
        {
            gaps.push([end, *b0, mid(b, &at)]);
        }
        if let Some(g) = repeats {
            gaps.extend(pairs.iter().filter(|(pg, _)| (pg - g).abs() <= 1e-9).map(|(_, gap)| *gap));
        }
        Some((start - r0, gaps))
    }

    /// A Spacing Guides mark: a line across the gap (start, end, where across) along the x axis
    /// (`x`) or the y axis, with a tick `tick` long either side of it at each end.
    fn gap_marks(&self, [u0, u1, c]: [f64; 3], x: bool, tick: f64) -> [Overlay; 3] {
        let pt = |u: f64, c: f64| if x { Point::new(u, c) } else { Point::new(c, u) };
        let line = |a: Point, b: Point| Overlay::Line { a, b, color: self.style.color, dashed: false };
        [line(pt(u0, c), pt(u1, c)), line(pt(u0, c - tick), pt(u0, c + tick)), line(pt(u1, c - tick), pt(u1, c + tick))]
    }

    /// [`Self::snap_rects`] before Spacing Guides, also saying whether each axis (x, y) lined up
    /// (even with Alignment Guides off, or already in line, when there is no line or shift to
    /// tell).
    fn align_rects(&self, rects: &[Rect], tol: f64) -> (Vec2, Vec<Overlay>, [bool; 2]) {
        let nearest = |targets: &[(f64, Point, Kind)], along: fn(&Rect) -> [f64; 3]| {
            rects
                .iter()
                .flat_map(|r| along(r).into_iter().map(move |v| (v, *r)))
                .flat_map(|(v, r)| lines_near(targets, v, tol).iter().map(move |(t, from, _)| (t - v, *from, v, r)))
                .filter(|(d, ..)| d.abs() <= tol)
                .min_by(|a, b| a.0.abs().total_cmp(&b.0.abs()))
        };
        let best_x = nearest(&self.xs, |r| [r.x0, r.center().x, r.x1]);
        let best_y = nearest(&self.ys, |r| [r.y0, r.center().y, r.y1]);
        let mut d = Vec2::ZERO;
        let mut ov = vec![];
        if let Some((dx, from, x, r)) = best_x {
            d.x = dx;
            let x = x + dx;
            let (y0, y1) = (from.y.min(r.y0), from.y.max(r.y1));
            ov.extend(self.line(Point::new(x, y0), Point::new(x, y1)));
        }
        if let Some((dy, from, y, r)) = best_y {
            d.y = dy;
            let y = y + dy;
            let (x0, x1) = (from.x.min(r.x0), from.x.max(r.x1));
            ov.extend(self.line(Point::new(x0, y), Point::new(x1, y)));
        }
        (d, ov, [best_x.is_some(), best_y.is_some()])
    }

    /// Snap a bounding-box resize. `a` is the scale [`crate::bbox::scale_for_drag`] gave for
    /// dragging `handle` of the box `bx` (about the centre when `from_center`), in the box's own
    /// frame, and so is the result. The handle lands on an anchor or centre within `tol` on the page
    /// (labelled as [`Self::snap_point`] does), else the corners and the side it moves line up with
    /// the nearest target or ruler guide ("align"), whatever the box's angle. A handle with one way
    /// to go (a side, or a proportional corner) slides along it until one of them is on a target.
    pub fn snap_scale(&self, bx: &OrientedBox, handle: Handle, a: Affine, proportional: bool, from_center: bool, tol: f64) -> (Affine, Vec<Overlay>) {
        type Hit = Option<(f64, Point)>;
        // Where a snap lands: in line with a target's x and/or y, or on an anchor or centre.
        type Snap = (Hit, Hit, Option<(Point, Kind)>);
        let r = bx.rect;
        let origin = if from_center { r.center() } else { handle.opposite().pos(r) };
        let (d0, [sx, _, _, sy, _, _]) = (handle.pos(r) - origin, a.as_coeffs());
        let (to_doc, to_local) = (bx.to_doc(), bx.to_doc().inverse());
        // A scale that collapses or flips the box is no snap.
        let valid = |n: f64, s: f64| n.is_finite() && n * s > 0.0;
        // The target line along the x axis (`x`: a vertical line) or the y axis nearest `p`, the
        // ruler guides running past it included.
        let nearest = |p: Point, x: bool| -> Hit {
            let (v, ts) = if x { (p.x, &self.xs) } else { (p.y, &self.ys) };
            let rulers = self.rulers.iter().filter(|r| r.vertical == x).filter_map(|r| r.line_at(p, tol));
            lines_near(ts, v, tol)
                .iter()
                .copied()
                .chain(rulers)
                .map(|(t, from, _)| (t, from))
                .filter(|(t, _)| (t - v).abs() <= tol)
                .min_by(|p, q| (p.0 - v).abs().total_cmp(&(q.0 - v).abs()))
        };
        let (mut nx, mut ny) = (sx, sy);
        let mut snap: Snap = (None, None, None);
        if handle.is_corner() && !proportional {
            // The corner goes anywhere: onto an anchor or centre, else it takes the target's x, its
            // y, or both.
            let p = to_doc * (a * handle.pos(r));
            let (hx, hy) = (nearest(p, true), nearest(p, false));
            let on = self.nearest_point(p, tol).map(|(q, k)| (Some((q.x, q)), Some((q.y, q)), Some((q, k))));
            for (tx, ty, at) in on.into_iter().chain([(hx, hy, None), (hx, None, None), (None, hy, None)]) {
                if tx.is_none() && ty.is_none() {
                    continue;
                }
                let q = to_local * Point::new(tx.map_or(p.x, |h| h.0), ty.map_or(p.y, |h| h.0));
                let (mut cx, mut cy) = ((q.x - origin.x) / d0.x, (q.y - origin.y) / d0.y);
                // Square to the page, the axis without a target stays exactly where it was.
                if bx.angle == 0.0 {
                    (cx, cy) = (if tx.is_some() { cx } else { sx }, if ty.is_some() { cy } else { sy });
                }
                if valid(cx, sx) && valid(cy, sy) {
                    (nx, ny, snap) = (cx, cy, (tx, ty, at));
                    break;
                }
            }
        } else {
            // One parameter `s`: the scale is `c + s·e` on each axis.
            let (ax, _) = handle.axes();
            let (e, c, s0) = match (handle.is_corner(), ax, proportional) {
                (true, _, _) => ((sx.signum(), sy.signum()), (0.0, 0.0), sx.abs()),
                (false, true, true) => ((1.0, sx.signum()), (0.0, 0.0), sx),
                (false, true, false) => ((1.0, 0.0), (0.0, 1.0), sx),
                (false, false, true) => ((sy.signum(), 1.0), (0.0, 0.0), sy),
                (false, false, false) => ((0.0, 1.0), (1.0, 0.0), sy),
            };
            // The points that may land on a target: the handle, and the two ends of its side.
            let i = handle as usize;
            let ends = if handle.is_corner() { [None, None] } else { [Handle::ALL.get((i + 1) % 8), Handle::ALL.get((i + 7) % 8)] };
            // How far the handle travels per unit of `s`: a snap may not drag it far from the
            // pointer, as it would for a target the side runs almost along.
            let travel = (e.0 * d0.x).hypot(e.1 * d0.y);
            // The snap so far: its `s`, whether it only lines up (an anchor or centre comes first),
            // how far it moves the handle, and where it lands.
            let mut best: Option<(f64, bool, f64, Snap)> = None;
            let mut consider = |s: f64, in_line: bool, found: Snap| {
                let moved = (s - s0).abs() * travel;
                if valid(s, s0) && moved <= 2.0 * tol && best.as_ref().is_none_or(|k| (in_line, moved) < (k.1, k.2)) {
                    best = Some((s, in_line, moved, found));
                }
            };
            for h in std::iter::once(handle).chain(ends.into_iter().flatten().copied()) {
                let v = h.pos(r) - origin;
                let base = to_doc * (origin + Vec2::new(c.0 * v.x, c.1 * v.y));
                let dir = (to_doc * Point::new(e.0 * v.x, e.1 * v.y)).to_vec2();
                let p = base + dir * s0;
                // An anchor or centre beside its way: it stops at the nearest point of the way.
                let l2 = dir.hypot2();
                if l2 > 1e-18
                    && let Some((q, k)) = self.nearest_point(p, tol)
                {
                    consider((q - base).dot(dir) / l2, false, (None, None, Some((q, k))));
                }
                for (is_x, b, d) in [(true, base.x, dir.x), (false, base.y, dir.y)] {
                    if d.abs() <= 1e-9 {
                        continue;
                    }
                    let Some(hit) = nearest(p, is_x) else { continue };
                    consider((hit.0 - b) / d, true, if is_x { (Some(hit), None, None) } else { (None, Some(hit), None) });
                }
            }
            if let Some((s, _, _, found)) = best {
                (nx, ny, snap) = (c.0 + s * e.0, c.1 + s * e.1, found);
            }
        }
        let mut out = Affine::translate(origin.to_vec2()) * Affine::scale_non_uniform(nx, ny) * Affine::translate(-origin.to_vec2());
        let (hit_x, hit_y, on) = snap;
        if let Some((q, k)) = on {
            return (out, self.label(q, k.label()).into_iter().collect());
        }
        // The lines run from the target's own point across the resized box, on the page.
        let mut nr = (to_doc * out).transform_rect_bbox(r);
        let mut ov = vec![];
        if let Some((x, from)) = hit_x {
            ov.extend(self.line(Point::new(x, from.y.min(nr.y0)), Point::new(x, from.y.max(nr.y1))));
        }
        if let Some((y, from)) = hit_y {
            ov.extend(self.line(Point::new(from.x.min(nr.x0), y), Point::new(from.x.max(nr.x1), y)));
        }
        if hit_x.is_some() || hit_y.is_some() {
            ov.extend(self.label(to_doc * (out * handle.pos(r)), "align"));
        }
        if !self.boxes.is_empty() && !from_center {
            let aligned = [hit_x.is_some(), hit_y.is_some()];
            for (is_x, lined_up) in [(true, aligned[0]), (false, aligned[1])] {
                if lined_up {
                    continue;
                }
                let Some(moves_min) = resize_moves_min_edge(handle, is_x) else { continue };
                if let Some((shift, gaps)) = self.even_spacing(nr, is_x, tol) {
                    let (lo, hi) = span(&nr, is_x);
                    let len = hi - lo;
                    let new_len = if moves_min { hi - (lo + shift) } else { hi + shift - lo };
                    if len > 1e-9 && new_len > 1e-9 {
                        let f = new_len / len;
                        let (ax, ay) = handle.axes();
                        if is_x {
                            nx *= f;
                            if proportional && ay {
                                ny *= f;
                            }
                        } else {
                            ny *= f;
                            if proportional && ax {
                                nx *= f;
                            }
                        }
                        out = Affine::translate(origin.to_vec2()) * Affine::scale_non_uniform(nx, ny) * Affine::translate(-origin.to_vec2());
                        nr = (to_doc * out).transform_rect_bbox(r);
                        ov.extend(gaps.into_iter().flat_map(|g| self.gap_marks(g, is_x, tol)));
                    }
                }
            }
        }
        (out, ov)
    }
}
