use super::targets::{Targets, span};
use super::{
    Affine, GuideStyle, Handle, HitOptions, Line, NodeId, OrientedBox, Overlay, ParamCurve, ParamCurveDeriv, ParamCurveNearest, PathSeg, Point, Rect,
    ScreenFrame, ToolContext, Vec2, constrain_angle_from, hit_test,
};

/// Where curves `a` and `b` cross near `p`: a point projected from one onto the other and back
/// settles where they meet (none when they don't nearby).
pub(super) fn meeting(a: &PathSeg, b: &PathSeg, p: Point) -> Option<Point> {
    let mut q = p;
    for _ in 0..32 {
        let on_a = a.eval(a.nearest(q, 1e-9).t);
        q = b.eval(b.nearest(on_a, 1e-9).t);
        if on_a.distance(q) <= 1e-6 {
            return Some(q);
        }
    }
    None
}

/// The direction of `seg` at `t` (not normalised; zero in a cusp).
pub(super) fn seg_direction(seg: &PathSeg, t: f64) -> Vec2 {
    match seg {
        PathSeg::Line(l) => l.p1 - l.p0,
        PathSeg::Quad(q) => q.deriv().eval(t).to_vec2(),
        PathSeg::Cubic(c) => c.deriv().eval(t).to_vec2(),
    }
}

/// The parameters in (0, 1) where the line from `at` to `seg` is square to it (`perpendicular`)
/// or grazes it, found by sampling the curve for a change of sign and bisecting.
pub(super) fn touch_params(seg: &PathSeg, at: Point, perpendicular: bool) -> Vec<f64> {
    const STEPS: usize = 24;
    let f = |t: f64| {
        let (v, d) = (seg.eval(t) - at, seg_direction(seg, t));
        if perpendicular { v.dot(d) } else { v.cross(d) }
    };
    let mut out = vec![];
    let mut prev = (0.0, f(0.0));
    for i in 1..=STEPS {
        let t = i as f64 / STEPS as f64;
        let cur = (t, f(t));
        if prev.1.is_finite() && cur.1.is_finite() && prev.1 * cur.1 < 0.0 {
            let (mut lo, mut hi) = (prev, cur);
            for _ in 0..40 {
                let mid = 0.5 * (lo.0 + hi.0);
                let m = (mid, f(mid));
                if lo.1 * m.1 <= 0.0 {
                    hi = m;
                } else {
                    lo = m;
                }
            }
            let t = 0.5 * (lo.0 + hi.0);
            // A jump through a cusp changes sign without crossing zero: the line must really
            // run along (or square to) the curve there.
            let (v, d) = (seg.eval(t) - at, seg_direction(seg, t));
            let scale = v.hypot() * d.hypot();
            if scale > 1e-12 && f(t).abs() <= 1e-3 * scale {
                out.push(t);
            }
        }
        prev = cur;
    }
    out
}

/// Where a press at `p` takes hold of `path` by its outline: the point of the path nearest `p`,
/// when it is within `reach`, and on a straight segment exactly on its line. A path dragged by a
/// press on its stroke or beside its edge snaps by that point of the path, so it is the path that
/// lands on a ruler guide or lines up, not the pressed spot a few pixels beside it (#3701).
pub(crate) fn hold_on_path(path: &vectorcraft_geom::PathData, p: Point, reach: f64) -> Option<Point> {
    let (si, seg, _, q, d) = path.nearest(p)?;
    if d > reach {
        return None;
    }
    let sp = path.subpaths.get(si)?;
    if !sp.segment_is_line(seg) {
        return Some(q);
    }
    let c = sp.segment(seg);
    let line = Line::new(c.p0, c.p3);
    Some(line.eval(line.nearest(p, 0.0).t))
}

/// Snap `p` for a drawing tool when smart guides (or grid snapping) are on.
pub fn snap_draw(cx: &ToolContext, p: Point, exclude: &[NodeId]) -> (Point, Vec<Overlay>) {
    snap_with(cx, p, || Targets::collect(cx.doc, exclude, None))
}

/// Snap `p` to pixels or the grid when they are on, else to the smart guide `targets`, else (Snap
/// to Point) to their anchors and the ruler guides within the Snap to Point distance.
fn snap_with(cx: &ToolContext, p: Point, targets: impl FnOnce() -> Targets) -> (Point, Vec<Overlay>) {
    PointSnap::new(cx, targets).snap(cx, p)
}

/// Snapping for a dragged point, its targets gathered once (when the drag begins): what
/// [`snap_draw`] does for a point at a time.
pub struct PointSnap {
    /// The smart guide targets, or with Smart Guides off those of Snap to Point, and how near (in
    /// screen pixels) they pull; none with pixels or the grid snapping instead, or nothing on.
    targets: Option<(Targets, f64)>,
}

impl PointSnap {
    pub fn new(cx: &ToolContext, targets: impl FnOnce() -> Targets) -> Self {
        let targets = if cx.snap_to_pixel || cx.snap_to_grid {
            None
        } else if cx.smart_guides {
            // Snap to Point keeps pulling points onto the ruler guides with Smart Guides on (#812).
            let mut t = targets().styled(cx);
            (t.tangent, t.perpendicular) = (cx.snap_tangent, cx.snap_perpendicular);
            Some((if cx.snap_to_point { t.with_rulers(cx) } else { t }, cx.snapping_tolerance))
        } else if cx.snap_to_point {
            Some((targets().for_snap_to_point(cx), cx.snap_tolerance))
        } else {
            None
        };
        Self { targets }
    }

    /// Where `p` goes, and the guides showing why.
    pub fn snap(&self, cx: &ToolContext, p: Point) -> (Point, Vec<Overlay>) {
        self.snap_from(cx, p, None)
    }

    /// Where a drawn point `p` leaving `from` goes ([`Targets::snap_from`]), and the guides
    /// showing why. Pixels and the grid round it before Shift turns it onto its angle.
    pub fn snap_from(&self, cx: &ToolContext, p: Point, from: Option<&Leave>) -> (Point, Vec<Overlay>) {
        let keep = |q: Point| from.and_then(|f| f.constrained(q)).unwrap_or(q);
        if cx.snap_to_pixel {
            return (keep(Point::new(p.x.round(), p.y.round())), vec![]);
        }
        if cx.snap_to_grid {
            return (keep(vectorcraft_geom::snap::snap_point_to_grid(p, cx.grid_step())), vec![]);
        }
        match &self.targets {
            Some((t, tol)) => t.snap_from(p, from, cx.tol(*tol)),
            None => (keep(p), vec![]),
        }
    }

    /// Snap a bounding-box resize ([`Targets::snap_scale`]) as [`Self::snap`] snaps a point. With
    /// Snap to Pixel on, the corner or side the handle moves lines up with whole pixels, and with
    /// Snap to Grid on it lines up with the nearest gridline, however far that is; neither shows
    /// guides. Otherwise the handle snaps to the targets [`Self::new`] gathered.
    pub fn snap_scale(
        &self,
        cx: &ToolContext,
        bx: &OrientedBox,
        handle: Handle,
        a: Affine,
        proportional: bool,
        from_center: bool,
    ) -> (Affine, Vec<Overlay>) {
        let step = if cx.snap_to_pixel {
            Some(1.0)
        } else if cx.snap_to_grid {
            Some(cx.grid_step()).filter(|s| s.is_finite() && *s > 0.0)
        } else {
            None
        };
        if let Some(step) = step {
            let moved = Handle::ALL.map(|h| bx.to_doc() * (a * h.pos(bx.rect)));
            // Every point the handle moves is within half a step of a gridline, so a tolerance of
            // one step always finds one.
            return (Targets::grid_lines(moved, step).snap_scale(bx, handle, a, proportional, from_center, step).0, vec![]);
        }
        match &self.targets {
            Some((t, tol)) => t.snap_scale(bx, handle, a, proportional, from_center, cx.tol(*tol)),
            None => (a, vec![]),
        }
    }
}

/// The Construction Guides angle set new preferences start with.
pub const DEFAULT_CONSTRUCTION_ANGLES: &str = "90° & 45° Angles";

/// The angles (degrees) of the construction guides of a Smart Guides › Angles set: "90°" gives the
/// horizontal and the vertical, "45°" the diagonals, "60°" and "30°" their multiples between those.
pub fn construction_angles(set: &str) -> &'static [f64] {
    match set {
        "30° Angles" => &[30.0, 60.0, 120.0, 150.0],
        "45° Angles" => &[45.0, 135.0],
        "60° Angles" => &[60.0, 120.0],
        "90° Angles" => &[0.0, 90.0],
        "90° & 45° & 30° Angles" => &[0.0, 30.0, 45.0, 60.0, 90.0, 120.0, 135.0, 150.0],
        _ => &[0.0, 45.0, 90.0, 135.0],
    }
}

/// How Shift keeps a drawn point to the point it leaves from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Keep {
    /// At steps of `step` degrees counted from `base` (a segment).
    Angle { step: f64, base: f64 },
    /// On a diagonal, as far along as the pointer's farther axis (a square's corner).
    Square,
}

/// The point a drawn point leaves from (the anchor before it, a shape's start) and how it keeps
/// to it: with Shift held at fixed angles round it, and onto the construction guides through it
/// (Smart Guides › Construction Guides).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Leave {
    pub at: Point,
    /// Shift held: how the point keeps to `at`.
    shift: Option<Keep>,
    /// The angles of the construction guides through `at` (none with them off).
    angles: &'static [f64],
    /// A drawn segment: it may land square to a segment or grazing a curve (a box does not).
    pub(super) contact: bool,
}

impl Leave {
    /// A segment from `at` (Pen, Curvature, Line Segment): Shift keeps it at 45° steps from
    /// General › Constrain Angle; with Smart Guides on it lands on the construction guides.
    pub fn segment(cx: &ToolContext, at: Point, shift: bool) -> Self {
        Self {
            at,
            shift: shift.then_some(Keep::Angle { step: 45.0, base: cx.constrain_angle }),
            angles: if cx.smart_guides { cx.construction_angles } else { &[] },
            contact: true,
        }
    }

    /// A curved source cannot make a line-to-line perpendicular or line-to-curve tangent contact.
    pub fn without_contact(mut self) -> Self {
        self.contact = false;
        self
    }

    /// A box dragged from its corner or centre `at` (Rectangle, Ellipse, Arc, the grids, and
    /// Artboard, which passes no Shift): Shift keeps the dragged corner on a diagonal, the box a
    /// square.
    pub fn diagonal(at: Point, shift: bool) -> Self {
        Self { at, shift: shift.then_some(Keep::Square), angles: &[], contact: false }
    }

    /// Where Shift puts `p`: none without it.
    pub(super) fn constrained(&self, p: Point) -> Option<Point> {
        let v = p - self.at;
        Some(match self.shift? {
            Keep::Angle { step, base } => self.at + constrain_angle_from(v, step, base),
            Keep::Square => self.at + square(v),
        })
    }

    /// The construction guide through `at` nearest `p`, within `tol` of it (and `p` farther than
    /// that from `at`, where every guide meets): its direction and where `p` lands on it.
    pub(super) fn construction(&self, p: Point, tol: f64) -> Option<(Vec2, Point)> {
        let v = p - self.at;
        if v.hypot() <= tol {
            return None;
        }
        self.angles
            .iter()
            // Counter-clockwise as seen on the y-down page.
            .map(|a| Vec2::new(a.to_radians().cos(), -a.to_radians().sin()))
            .map(|d| (d, self.at + d * v.dot(d)))
            .map(|(d, q)| (d, q, q.distance(p)))
            .filter(|(.., dist)| *dist <= tol)
            .min_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(d, q, _)| (d, q))
    }
}

/// `d` made square: as long on both axes as on its longer one, each keeping its sign.
pub fn square(d: Vec2) -> Vec2 {
    let s = d.x.abs().max(d.y.abs());
    Vec2::new(s * d.x.signum(), s * d.y.signum())
}

/// Smart Guides for the points a drawing tool places: the Pen's and Curvature tool's anchors,
/// the Pencil's ends, the shape tools' start and dragged corner, the Type tool's click and the
/// Artboard tool's new artboard. Each such tool keeps one and asks it for every point, hovering
/// too, so they all snap alike, and shows its [`Self::guides`].
///
/// The targets ([`Targets::for_drawing`], the art in the window) are gathered once per document
/// state: hovering gathers them again only after the document, the selection, what is left out,
/// the window or the snapping options changed; a press always gathers them afresh, and the drag
/// it starts keeps them, the document then showing the drag's own previews.
#[derive(Default)]
pub struct DrawSnap {
    /// The targets, and what they were gathered for (none: for one drag only).
    targets: Option<(Option<SnapKey>, PointSnap)>,
    guides: Vec<Overlay>,
}

/// What a [`DrawSnap`]'s targets were gathered for: the document state, the art left out, the
/// window and every option [`PointSnap::new`] reads.
#[derive(PartialEq)]
struct SnapKey {
    revision: (u64, u64),
    exclude: Vec<NodeId>,
    screen: Option<ScreenFrame>,
    view: [bool; 7],
    snap_to_glyph: bool,
    style: GuideStyle,
    tolerances: [f64; 2],
}

impl SnapKey {
    fn of(cx: &ToolContext, exclude: &[NodeId]) -> Self {
        Self {
            revision: cx.revision,
            exclude: exclude.to_vec(),
            screen: cx.screen,
            view: [cx.smart_guides, cx.snap_to_point, cx.snap_to_pixel, cx.snap_to_grid, cx.guides, cx.snap_tangent, cx.snap_perpendicular],
            snap_to_glyph: cx.snap_to_glyph && cx.smart_guides && cx.alignment_guides,
            style: GuideStyle::of(cx),
            tolerances: [cx.snapping_tolerance, cx.snap_tolerance],
        }
    }
}

/// The part of the document the window shows (none headless: all of it).
fn visible(cx: &ToolContext) -> Option<Rect> {
    let s = cx.screen?;
    let (w, h) = s.size;
    Some(Rect::from_points(s.at(0.0, 0.0), s.at(w, h)).union_pt(s.at(w, 0.0)).union_pt(s.at(0.0, h)))
}

impl DrawSnap {
    /// Snap the pointer hovering at `p` (the art but `exclude` as targets, the anchors of
    /// `exclude` still pulling), reusing the targets while nothing they depend on changed.
    pub fn hover(&mut self, cx: &ToolContext, p: Point, exclude: &[NodeId], from: Option<&Leave>) -> Point {
        let key = SnapKey::of(cx, exclude);
        if self.targets.as_ref().is_none_or(|(k, _)| k.as_ref() != Some(&key)) {
            self.gather(cx, key);
        }
        self.snap(cx, p, from)
    }

    /// Snap the point pressed at `p`, the targets gathered afresh: the drag that follows keeps
    /// them ([`Self::drag`]).
    pub fn press(&mut self, cx: &ToolContext, p: Point, exclude: &[NodeId], from: Option<&Leave>) -> Point {
        self.gather(cx, SnapKey::of(cx, exclude));
        self.snap(cx, p, from)
    }

    /// Gather `targets` other than the art for the drag a press starts (a point being dragged
    /// leaves itself out); the next hover gathers its own again.
    pub fn hold(&mut self, cx: &ToolContext, targets: impl FnOnce() -> Targets) {
        self.targets = Some((None, PointSnap::new(cx, targets)));
        self.guides.clear();
    }

    /// Snap `p`, dragged since the press, with the targets the press gathered.
    pub fn drag(&mut self, cx: &ToolContext, p: Point, from: Option<&Leave>) -> Point {
        if self.targets.is_none() {
            self.gather(cx, SnapKey::of(cx, &[]));
        }
        self.snap(cx, p, from)
    }

    /// The guides of the last snap.
    pub fn guides(&self) -> &[Overlay] {
        &self.guides
    }

    /// Hide the guides (the gesture ended).
    pub fn clear(&mut self) {
        self.guides.clear();
    }

    fn gather(&mut self, cx: &ToolContext, key: SnapKey) {
        let snap = PointSnap::new(cx, || Targets::for_drawing(cx.doc, &key.exclude, visible(cx), key.snap_to_glyph));
        self.targets = Some((Some(key), snap));
    }

    fn snap(&mut self, cx: &ToolContext, p: Point, from: Option<&Leave>) -> Point {
        let (q, guides) = match &self.targets {
            Some((_, t)) => t.snap_from(cx, p, from),
            None => (p, vec![]),
        };
        self.guides = guides;
        q
    }
}

/// Snapping for a dragged direction handle: Shift keeps it at 45° steps round its anchor, else it
/// snaps as a drawn point does ([`DrawSnap`]). The targets ([`Targets::for_handle`]) are gathered
/// on the first move that needs them and kept for the rest of the drag.
#[derive(Default)]
pub struct HandleSnap(pub(crate) DrawSnap);

impl HandleSnap {
    /// Where the handle of anchor `ai` of subpath `si` of path `id` goes with the pointer at `p`,
    /// and the guides showing why.
    pub fn snap(&mut self, cx: &ToolContext, (id, si, ai): (NodeId, usize, usize), p: Point, shift: bool) -> (Point, Vec<Overlay>) {
        let path = cx.doc.node(id).and_then(|n| n.path_data());
        if shift && let Some(a) = path.and_then(|pd| pd.subpaths.get(si)?.anchors.get(ai)) {
            return (a.p + constrain_angle_from(p - a.p, 45.0, cx.constrain_angle), vec![]);
        }
        if self.0.targets.is_none() {
            self.0.hold(cx, || Targets::for_handle(cx.doc, id));
        }
        let q = self.0.drag(cx, p, None);
        (q, self.0.guides().to_vec())
    }
}

/// Snap a picked point (a transform tool's reference point) to the nearest anchor or centre of the
/// selection or of the object under the pointer, when Snap to Point or Smart Guides is on.
pub fn snap_pick(cx: &ToolContext, p: Point) -> (Point, Vec<Overlay>) {
    if !(cx.snap_to_point || cx.smart_guides) {
        return (p, vec![]);
    }
    let tol = if cx.smart_guides { cx.snap_tol() } else { cx.tol(cx.snap_tolerance) };
    let mut roots = cx.selection.objects.clone();
    roots.extend(hit_test(cx.doc, p, HitOptions { tol, skip_guides: false, ..cx.hit_options() }).map(|h| h.leaf));
    let near = Rect::new(p.x - tol, p.y - tol, p.x + tol, p.y + tol);
    Targets::points_near(cx.doc, &roots, near).styled(cx).snap_point(p, tol)
}

/// Distance Guides (Smart Guides → Invoke Distance Guides): how far the selection's bounds `from`
/// lie from `to` along each axis, as measured lines ([`Overlay::Distance`]) labelled with their
/// lengths in the General unit. Along an axis on which the two lie apart, one line spans the gap
/// between their facing edges; along one on which they overlap, or one holds the other, one line
/// runs between their left (top) edges and one between their right (bottom) edges. An artboard
/// (`artboard`) is always measured edge to edge: the selection's distance from its left, right,
/// top and bottom edges. Each line runs across the middle of where the two overlap on the other
/// axis, else across the selection's middle, with a dashed line from `to`'s edge out to it when it
/// passes beside `to`. Lines of no length are left out.
pub fn distance_guides(cx: &ToolContext, from: Rect, to: Rect, artboard: bool) -> Vec<Overlay> {
    let finite = |r: &Rect| [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite());
    if !finite(&from) || !finite(&to) {
        return vec![];
    }
    let color = cx.smart_guide_color;
    let mut out = Vec::new();
    for x in [true, false] {
        let ((f0, f1), (t0, t1)) = (span(&from, x), span(&to, x));
        let ((g0, g1), (u0, u1)) = (span(&from, !x), span(&to, !x));
        let (lo, hi) = (g0.max(u0), g1.min(u1));
        let across = if lo <= hi { (lo + hi) / 2.0 } else { (g0 + g1) / 2.0 };
        let pt = |u: f64, c: f64| if x { Point::new(u, c) } else { Point::new(c, u) };
        // Each measured line: from the selection's edge to `to`'s.
        let lines: Vec<(f64, f64)> = match (artboard, f1 <= t0, t1 <= f0) {
            (false, true, _) => vec![(f1, t0)],
            (false, _, true) => vec![(f0, t1)],
            _ => vec![(f0, t0), (f1, t1)],
        };
        for (fu, tu) in lines {
            let length = (tu - fu).abs();
            if length <= 1e-9 {
                continue;
            }
            out.push(Overlay::Distance { a: pt(fu, across), b: pt(tu, across), text: cx.len(length), color });
            let on_edge = across.clamp(u0, u1);
            if on_edge != across {
                out.push(Overlay::Line { a: pt(tu, on_edge), b: pt(tu, across), color, dashed: true });
            }
        }
    }
    out
}
