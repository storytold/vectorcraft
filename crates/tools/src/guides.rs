//! Smart Guides: snapping to anchors, path segments (for dragged anchors and drawn points), object
//! bounds (edges/centres), artboards, with the magenta construction lines and labels Illustrator
//! users expect. Preferences › Smart Guides sets their colour, which of them show ([`GuideStyle`]),
//! how near a target pulls (`ToolContext::snapping_tolerance`) and the angles of the construction
//! guides drawn points land on (`ToolContext::construction_angles`).
//!
//! The drawing tools (Pen, Curvature, Pencil, the shape tools, Type, Artboard) all snap the points
//! they place through one [`DrawSnap`]: their targets are gathered once per document state, sorted,
//! and each pointer move looks up the few near the pointer by bisection.

use std::collections::HashSet;
use std::sync::Arc;

use vectorcraft_doc::hit::{HitOptions, hit_test};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, OrientedBox, Selection};
use vectorcraft_geom::kurbo::ParamCurveNearest;
use vectorcraft_geom::{Affine, Line, ParamCurve, PathSeg, Point, Rect, SubPath, Vec2, constrain_angle_from};

use crate::bbox::Handle;
use crate::{Overlay, ScreenFrame, ToolContext};

pub const MAGENTA: [u8; 3] = [0xff, 0x3d, 0xfc];

/// How Smart Guides look (Preferences › Smart Guides › Display Options): their colour, and whether
/// the alignment lines and the anchor/path labels show. Snapping is the same either way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideStyle {
    pub color: [u8; 3],
    /// Alignment Guides: the lines along the edges and centres the art lines up with.
    pub lines: bool,
    /// Anchor/Path Labels: "anchor", "center", "path", "align"…
    pub labels: bool,
}

impl Default for GuideStyle {
    fn default() -> Self {
        Self { color: MAGENTA, lines: true, labels: true }
    }
}

impl GuideStyle {
    /// The style the preferences in `cx` ask for.
    pub fn of(cx: &ToolContext) -> Self {
        Self { color: cx.smart_guide_color, lines: cx.alignment_guides, labels: cx.anchor_path_labels }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Anchor,
    Center,
    Edge,
    Artboard,
    Bleed,
    Guide,
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
        }
    }
}

/// Snap targets gathered from the document (excluding the objects being edited).
#[derive(Default)]
pub struct Targets {
    points: Vec<(Point, Kind)>,
    xs: Vec<(f64, Point, Kind)>,
    ys: Vec<(f64, Point, Kind)>,
    /// Path segments and their control bounds, which a point lands on ("path"): gathered only for
    /// dragged anchors ([`Self::for_anchor_drag`]).
    segments: Vec<(Rect, PathSeg)>,
    /// Ruler guides, which pull a point into line where they run (Snap to Point).
    rulers: Vec<Ruler>,
    /// The bounds of the objects a moved selection spaces itself evenly with (Spacing Guides):
    /// gathered only for moves ([`Self::for_move`]).
    boxes: Vec<Rect>,
    /// How the guides these targets produce look.
    style: GuideStyle,
}

/// A ruler guide as a snap target.
struct Ruler {
    vertical: bool,
    pos: f64,
    /// Where it runs along its line: an artboard guide across its artboard (see
    /// [`vectorcraft_doc::Document::guide_span`]), else (None) the whole canvas.
    span: Option<(f64, f64)>,
}

impl Ruler {
    /// The line it pulls `p` into within `tol` of its ends (its position, where it starts, the
    /// kind), as [`Targets::xs`] holds them: none where it doesn't run.
    fn line_at(&self, p: Point, tol: f64) -> Option<(f64, Point, Kind)> {
        let along = if self.vertical { p.y } else { p.x };
        let start = match self.span {
            Some((a, b)) if along < a - tol || along > b + tol => return None,
            Some((a, _)) => a,
            None => 0.0,
        };
        let from = if self.vertical { Point::new(self.pos, start) } else { Point::new(start, self.pos) };
        Some((self.pos, from, Kind::Guide))
    }
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
/// one pass, however many are skipped.
fn walk_but<'a>(nodes: &'a [Arc<Node>], skip: &HashSet<NodeId>, f: &mut impl FnMut(&'a Node)) {
    for n in nodes.iter().filter(|n| !skip.contains(&n.id)) {
        f(n);
        if let Some(ch) = n.children() {
            walk_but(ch, skip, f);
        }
    }
}

/// The lines of `lines` (sorted by where they run) within `tol` of `v`.
fn lines_near(lines: &[(f64, Point, Kind)], v: f64, tol: f64) -> &[(f64, Point, Kind)] {
    within(lines, |l| l.0, v, tol)
}

/// `r`'s extent along the x axis (`x`) or the y axis.
fn span(r: &Rect, x: bool) -> (f64, f64) {
    if x { (r.x0, r.x1) } else { (r.y0, r.y1) }
}

/// The objects a moved selection (`moving`) spaces itself evenly with, by their bounds: each
/// visible object on a layer as a whole (a group too), and inside a group holding some of the
/// moving objects its other objects. At most [`BUDGET`].
fn object_boxes(doc: &Document, moving: &[NodeId]) -> Vec<Rect> {
    fn push(nodes: &[Arc<Node>], moving: &HashSet<NodeId>, holders: &HashSet<NodeId>, out: &mut Vec<Rect>) {
        for n in nodes.iter().filter(|n| n.visible && !moving.contains(&n.id)) {
            if out.len() >= BUDGET {
                return;
            }
            match n.children() {
                Some(ch) if n.is_layer() || holders.contains(&n.id) => push(ch, moving, holders, out),
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

    /// Targets for moving the selection of `cx`: [`Self::collect`]'s, styled as `cx` asks, and with
    /// Smart Guides › Spacing Guides the objects it spaces itself evenly with ([`Self::snap_rect`]).
    pub fn for_move(cx: &ToolContext) -> Self {
        let mut t = Self::collect(cx.doc, &cx.selection.objects, None).styled(cx);
        if cx.spacing_guides {
            t.boxes = object_boxes(cx.doc, &cx.selection.objects);
        }
        t
    }

    /// Targets for a drawn point: the art in `visible` (all of it for None) but `exclude`, its
    /// anchors, centres, edges and path segments, its anchors in line too. The anchors and
    /// segments of `exclude` (the path being drawn, whose bounds grow with it) stay targets.
    pub fn for_drawing(doc: &Document, exclude: &[NodeId], visible: Option<Rect>) -> Self {
        let mut t = Self::collect_inner(doc, exclude, None, visible, Gather::Drawing);
        t.push_paths(doc, exclude, true);
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
    /// the paths being reshaped (their bounds move along, so they are no targets).
    pub fn for_anchor_drag(doc: &Document, sel: &Selection) -> Self {
        let mut t = Self::collect_inner(doc, &sel.objects, None, None, Gather::Segments);
        let (mut anchors, mut segments) = (BUDGET, BUDGET);
        for (id, set) in &sel.anchors {
            let Some(path) = doc.node(*id).and_then(|n| n.path_data()) else { continue };
            for (si, sp) in path.subpaths.iter().enumerate() {
                let moving = |ai: usize| set.contains(&(si, ai));
                for a in sp.anchors.iter().enumerate().filter(|(ai, _)| !moving(*ai)).map(|(_, a)| a.p).take(anchors) {
                    t.push_anchor(a);
                    anchors -= 1;
                }
                t.push_segments(sp, moving, &mut segments);
            }
        }
        t.index()
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
        self
    }

    /// View → Snap to Point: only these targets' anchors pull, and the ruler guides (shown and
    /// unlocked, as the selection tools pick them) pull the pointer into line where they run.
    fn for_snap_to_point(self, cx: &ToolContext) -> Self {
        let mut t = self.anchors_only();
        if cx.guides {
            t.rulers = cx.doc.guides.iter().map(|g| Ruler { vertical: g.vertical, pos: g.pos, span: cx.doc.guide_span(g) }).collect();
        }
        t
    }

    /// What a dragged selection's grabbed point snaps to with View → Snap to Point and Smart
    /// Guides off: the anchors of the art but `exclude` and the ruler guides.
    pub fn snap_to_point(cx: &ToolContext, exclude: &[NodeId]) -> Option<Self> {
        (cx.snap_to_point && !cx.smart_guides).then(|| Self::collect(cx.doc, exclude, None).for_snap_to_point(cx))
    }

    /// Anchors and centres of the visible leaves under `roots` (the roots included) whose bounds
    /// reach into `near`.
    fn points_near(doc: &Document, roots: &[NodeId], near: Rect) -> Self {
        let mut t = Targets::default();
        for n in roots.iter().filter_map(|id| doc.node(*id)) {
            n.walk(&mut |c| {
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
        if let Some(q) = self.on_segment(p, tol) {
            return (q, self.label(q, Kind::Edge.label()).into_iter().collect());
        }
        if let Some(f) = from
            && let Some((dir, q)) = f.construction(p, tol)
        {
            let construction = |q: Point| Overlay::Line { a: f.at, b: q, color: self.style.color, dashed: false };
            return match self.slide(f.at, dir, q, tol, false) {
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

    /// The anchor or centre nearest `p`, within `tol`.
    fn nearest_point(&self, p: Point, tol: f64) -> Option<(Point, Kind)> {
        within(&self.points, |q| q.0.x, p.x, tol)
            .iter()
            .copied()
            .filter(|(q, _)| q.distance(p) <= tol)
            .min_by(|a, b| a.0.distance(p).total_cmp(&b.0.distance(p)))
    }

    /// Slide `q`, on the line through `at` along `dir` (beyond `at` only, for a `ray`), to where
    /// the nearest target line crosses it, at most twice `tol` away: where it lands and the
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
            // A line that far across the way crosses it at least that far along.
            for (t, from, _) in lines_near(lines, v, reach) {
                let s = (t - a) / d;
                // Exactly on the line, whatever the rounding along the way.
                let hit = if vertical { Point::new(*t, at.y + dir.y * s) } else { Point::new(at.x + dir.x * s, *t) };
                let moved = hit.distance(q);
                if moved <= reach && (!ray || s > 0.0) && best.is_none_or(|b| moved < b.0) {
                    best = Some((moved, hit, *from));
                }
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
    /// the nearest target ("align"), whatever the box's angle. A handle with one way to go (a side,
    /// or a proportional corner) slides along it until one of them is on a target.
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
        let nearest = |v: f64, ts: &[(f64, Point, Kind)]| -> Hit {
            lines_near(ts, v, tol)
                .iter()
                .map(|(t, from, _)| (*t, *from))
                .filter(|(t, _)| (t - v).abs() <= tol)
                .min_by(|p, q| (p.0 - v).abs().total_cmp(&(q.0 - v).abs()))
        };
        let (mut nx, mut ny) = (sx, sy);
        let mut snap: Snap = (None, None, None);
        if handle.is_corner() && !proportional {
            // The corner goes anywhere: onto an anchor or centre, else it takes the target's x, its
            // y, or both.
            let p = to_doc * (a * handle.pos(r));
            let (hx, hy) = (nearest(p.x, &self.xs), nearest(p.y, &self.ys));
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
                for (is_x, at, b, d, ts) in [(true, p.x, base.x, dir.x, &self.xs), (false, p.y, base.y, dir.y, &self.ys)] {
                    if d.abs() <= 1e-9 {
                        continue;
                    }
                    let Some(hit) = nearest(at, ts) else { continue };
                    consider((hit.0 - b) / d, true, if is_x { (Some(hit), None, None) } else { (None, Some(hit), None) });
                }
            }
            if let Some((s, _, _, found)) = best {
                (nx, ny, snap) = (c.0 + s * e.0, c.1 + s * e.1, found);
            }
        }
        let out = Affine::translate(origin.to_vec2()) * Affine::scale_non_uniform(nx, ny) * Affine::translate(-origin.to_vec2());
        let (hit_x, hit_y, on) = snap;
        if let Some((q, k)) = on {
            return (out, self.label(q, k.label()).into_iter().collect());
        }
        // The lines run from the target's own point across the resized box, on the page.
        let nr = (to_doc * out).transform_rect_bbox(r);
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
        (out, ov)
    }
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
            Some((targets().styled(cx), cx.snapping_tolerance))
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
}

impl Leave {
    /// A segment from `at` (Pen, Curvature, Line Segment): Shift keeps it at 45° steps from
    /// General › Constrain Angle; with Smart Guides on it lands on the construction guides.
    pub fn segment(cx: &ToolContext, at: Point, shift: bool) -> Self {
        Self {
            at,
            shift: shift.then_some(Keep::Angle { step: 45.0, base: cx.constrain_angle }),
            angles: if cx.smart_guides { cx.construction_angles } else { &[] },
        }
    }

    /// A box dragged from its corner or centre `at` (Rectangle, Ellipse, Arc, the grids,
    /// Artboard): Shift keeps the dragged corner on a diagonal, the box a square.
    pub fn diagonal(at: Point, shift: bool) -> Self {
        Self { at, shift: shift.then_some(Keep::Square), angles: &[] }
    }

    /// An anchor dragged from `at` (Direct Selection): Shift keeps the move at 45° steps round
    /// it, sliding along that way into line with the nearest target (#886). No construction
    /// guides through it, and the base matches [`crate::bbox::move_delta`] (0°) so the
    /// constrained direction is unchanged.
    pub fn anchor_drag(at: Point) -> Self {
        Self { at, shift: Some(Keep::Angle { step: 45.0, base: 0.0 }), angles: &[] }
    }

    /// Where Shift puts `p`: none without it.
    fn constrained(&self, p: Point) -> Option<Point> {
        let v = p - self.at;
        Some(match self.shift? {
            Keep::Angle { step, base } => self.at + constrain_angle_from(v, step, base),
            Keep::Square => self.at + square(v),
        })
    }

    /// The construction guide through `at` nearest `p`, within `tol` of it (and `p` farther than
    /// that from `at`, where every guide meets): its direction and where `p` lands on it.
    fn construction(&self, p: Point, tol: f64) -> Option<(Vec2, Point)> {
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
    view: [bool; 5],
    style: GuideStyle,
    tolerances: [f64; 2],
}

impl SnapKey {
    fn of(cx: &ToolContext, exclude: &[NodeId]) -> Self {
        Self {
            revision: cx.revision,
            exclude: exclude.to_vec(),
            screen: cx.screen,
            view: [cx.smart_guides, cx.snap_to_point, cx.snap_to_pixel, cx.snap_to_grid, cx.guides],
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
        let snap = PointSnap::new(cx, || Targets::for_drawing(cx.doc, &key.exclude, visible(cx)));
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
pub struct HandleSnap(DrawSnap);

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
    roots.extend(hit_test(cx.doc, p, HitOptions { tol, ..cx.hit_options() }).map(|h| h.leaf));
    let near = Rect::new(p.x - tol, p.y - tol, p.x + tol, p.y + tol);
    Targets::points_near(cx.doc, &roots, near).styled(cx).snap_point(p, tol)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;

    #[test]
    fn artboard_bleed_is_a_target() {
        let (mut d, _) = doc_with_rect();
        let t = Targets::collect(&d, &[], None);
        assert_eq!(t.snap_point(Point::new(512.0, 300.0), 4.0).0.x, 512.0, "no bleed, nothing near");
        d.setup.bleed = [10.0; 4];
        let t = Targets::collect(&d, &[], None);
        assert_eq!(t.snap_point(Point::new(512.0, 300.0), 4.0).0.x, 510.0);
    }

    #[test]
    fn snaps_to_anchor_and_alignment() {
        let (d, _) = doc_with_rect();
        let t = Targets::collect(&d, &[], None);
        let (p, ov) = t.snap_point(Point::new(102.0, 99.0), 4.0);
        assert_eq!(p, Point::new(100.0, 100.0));
        assert!(matches!(&ov[0], Overlay::Label { text, .. } if text == "anchor"));
        let (p, _) = t.snap_point(Point::new(301.0, 199.0), 4.0);
        assert_eq!(p.y, 200.0);
    }

    #[test]
    fn rect_snap_aligns_edges() {
        let (d, id) = doc_with_rect();
        let t = Targets::collect(&d, &[id], None);
        // Artboard is 0..500; a rect at x0=2 snaps to the artboard's left edge.
        let (dv, ov) = t.snap_rect(Rect::new(2.0, 50.0, 52.0, 90.0), 4.0);
        assert_eq!(dv.x, -2.0);
        assert!(!ov.is_empty());
    }

    #[test]
    fn scale_snap_lands_the_moved_edges_on_targets() {
        use crate::bbox::scale_for_drag;
        let (d, _) = doc_with_rect();
        // The document's rect is 100..200: a box beside it is resized against it.
        let t = Targets::collect(&d, &[], None);
        let r = Rect::new(250.0, 100.0, 300.0, 150.0);
        let snap = |h: Handle, p: Point, shift: bool, alt: bool| {
            let (a, ov) = t.snap_scale(&OrientedBox::aligned(r), h, scale_for_drag(r, h, p, shift, alt), shift, alt, 4.0);
            (a.transform_rect_bbox(r), ov)
        };
        // The bottom edge lands on the neighbour's bottom: same height.
        let lines = |ov: &[Overlay]| ov.iter().filter(|o| matches!(o, Overlay::Line { .. })).count();
        let align = |ov: &[Overlay]| ov.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "align"));
        let (nr, ov) = snap(Handle::Bottom, Point::new(275.0, 197.0), false, false);
        assert_eq!(nr, Rect::new(250.0, 100.0, 300.0, 200.0));
        assert_eq!(lines(&ov), 1);
        assert!(align(&ov), "{ov:?}");
        // A corner snaps each axis on its own: y to the neighbour, x stays free.
        let (nr, ov) = snap(Handle::BottomRight, Point::new(330.0, 202.0), false, false);
        assert_eq!(nr, Rect::new(250.0, 100.0, 330.0, 200.0));
        assert_eq!(lines(&ov), 1);
        // Proportional: the snapped axis carries the other one.
        let (nr, _) = snap(Handle::BottomRight, Point::new(340.0, 198.0), true, false);
        assert_eq!(nr, Rect::new(250.0, 100.0, 350.0, 200.0));
        // From the centre the dragged edge still lands on the target.
        let (nr, _) = snap(Handle::Bottom, Point::new(275.0, 198.0), false, true);
        assert_eq!(nr, Rect::new(250.0, 50.0, 300.0, 200.0));
        // Out of reach, and the fixed edge's own target: untouched.
        let (nr, ov) = snap(Handle::Bottom, Point::new(275.0, 180.0), false, false);
        assert_eq!(nr, Rect::new(250.0, 100.0, 300.0, 180.0));
        assert!(ov.is_empty());
        let (nr, ov) = snap(Handle::Bottom, Point::new(275.0, 102.0), false, false);
        assert_eq!(nr, Rect::new(250.0, 100.0, 300.0, 102.0));
        assert!(ov.is_empty());
    }

    /// A resize handle lands on another object's anchor point (#482), not only in line with its
    /// edges and centre, and says so; a side handle stops where its way passes the anchor.
    #[test]
    fn scale_snap_lands_the_handle_on_anchors() {
        use crate::bbox::scale_for_drag;
        use vectorcraft_doc::{Appearance, Node};
        let (mut d, _) = doc_with_rect();
        let l = d.layers[0].id;
        let id = d.alloc_id();
        // A triangle whose apex (320, 400) lines up with none of the edges or centres on x.
        let tri = vectorcraft_geom::PathData::single(SubPath::polyline(
            &[Point::new(300.0, 300.0), Point::new(380.0, 300.0), Point::new(320.0, 400.0)],
            true,
        ));
        d.insert(Some(l), 1, Node::path(id, tri, Appearance::default_art())).unwrap();
        let t = Targets::collect(&d, &[], None);
        let snap = |r: Rect, h: Handle, p: Point, shift: bool| {
            let (a, ov) = t.snap_scale(&OrientedBox::aligned(r), h, scale_for_drag(r, h, p, shift, false), shift, false, 4.0);
            (a.transform_rect_bbox(r), ov)
        };
        let anchor = |ov: &[Overlay]| matches!(ov, [Overlay::Label { text, p, .. }] if text == "anchor" && *p == Point::new(320.0, 400.0));
        // A free corner takes the anchor's x and y.
        let (nr, ov) = snap(Rect::new(250.0, 350.0, 300.0, 390.0), Handle::BottomRight, Point::new(322.0, 398.0), false);
        assert_eq!(nr, Rect::new(250.0, 350.0, 320.0, 400.0));
        assert!(anchor(&ov), "{ov:?}");
        // The right side, at the apex's height: its x lands on the anchor.
        let (nr, ov) = snap(Rect::new(250.0, 380.0, 300.0, 420.0), Handle::Right, Point::new(318.0, 401.0), false);
        assert_eq!(nr, Rect::new(250.0, 380.0, 320.0, 420.0));
        assert!(anchor(&ov), "{ov:?}");
        // A proportional corner slides along its diagonal onto it.
        let (nr, ov) = snap(Rect::new(240.0, 320.0, 300.0, 380.0), Handle::BottomRight, Point::new(318.0, 398.0), true);
        assert_eq!(nr, Rect::new(240.0, 320.0, 320.0, 400.0));
        assert!(anchor(&ov), "{ov:?}");
    }

    /// Smart Guides › Spacing Guides (#394): a moved selection lands as far from its neighbour as
    /// two other objects of its row are apart, or halfway between its two neighbours, and the
    /// equal gaps are marked; up and down the same in a column. Off, it stays where it is dragged.
    #[test]
    fn spacing_guides_space_a_moved_selection_evenly() {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        // A and B 20 apart in a row at y 300–340, C and D 30 apart in a column at x 400–440.
        for r in [
            Rect::new(20.0, 300.0, 60.0, 340.0),
            Rect::new(80.0, 300.0, 120.0, 340.0),
            Rect::new(400.0, 20.0, 440.0, 50.0),
            Rect::new(400.0, 80.0, 440.0, 110.0),
        ] {
            let id = d.alloc_id();
            d.insert(Some(l), 0, Node::path(id, vectorcraft_geom::shapes::rectangle(r), Default::default())).unwrap();
        }
        let (s, p) = (Selection::default(), paint());
        let lines = |ov: &[Overlay]| ov.iter().filter(|o| matches!(o, Overlay::Line { color, .. } if *color == MAGENTA)).count();
        let t = Targets::for_move(&cx(&d, &s, &p));
        // 21 after B: 20, as A and B, with both gaps marked (a line and two ticks each).
        let (dv, ov) = t.snap_rect(Rect::new(141.0, 305.0, 181.0, 345.0), 4.0);
        assert_eq!(dv, Vec2::new(-1.0, 0.0));
        assert_eq!(lines(&ov), 6, "{ov:?}");
        assert!(ov.iter().any(|o| matches!(o, Overlay::Line { a, b, .. } if (a.x, b.x) == (120.0, 140.0) && a.y == b.y)), "{ov:?}");
        assert!(ov.iter().any(|o| matches!(o, Overlay::Line { a, b, .. } if (a.x, b.x) == (60.0, 80.0) && a.y == b.y)), "{ov:?}");
        // Halfway between B and a box at 220–260: 50 either side.
        let mut wide = d.clone();
        let id = wide.alloc_id();
        wide.insert(Some(l), 0, Node::path(id, vectorcraft_geom::shapes::rectangle(Rect::new(220.0, 300.0, 260.0, 340.0)), Default::default()))
            .unwrap();
        let (dv, ov) = Targets::for_move(&cx(&wide, &s, &p)).snap_rect(Rect::new(148.0, 305.0, 188.0, 345.0), 4.0);
        assert_eq!(dv, Vec2::new(2.0, 0.0));
        assert!(lines(&ov) >= 6, "{ov:?}");
        // A column: 28 under D, as C and D are 30 apart.
        let (dv, ov) = t.snap_rect(Rect::new(405.0, 138.0, 445.0, 168.0), 4.0);
        assert_eq!(dv, Vec2::new(0.0, 2.0));
        assert!(ov.iter().any(|o| matches!(o, Overlay::Line { a, b, .. } if (a.y, b.y) == (110.0, 140.0) && a.x == b.x)), "{ov:?}");
        // Above C, 32 over it: 30.
        let (dv, _) = t.snap_rect(Rect::new(405.0, -42.0, 445.0, -12.0), 4.0);
        assert_eq!(dv, Vec2::new(0.0, 2.0));
        // Off: nowhere to go, and no marks.
        let off = ToolContext { spacing_guides: false, ..cx(&d, &s, &p) };
        let (dv, ov) = Targets::for_move(&off).snap_rect(Rect::new(141.0, 305.0, 181.0, 345.0), 4.0);
        assert_eq!((dv, lines(&ov)), (Vec2::ZERO, 0));
    }

    /// Preferences › Smart Guides › Display Options (#394): the colour, and Alignment Guides and
    /// Anchor/Path Labels off, change what shows and never where a point lands.
    #[test]
    fn display_options_colour_and_hide_the_guides_but_keep_the_snap() {
        let (d, _) = doc_with_rect();
        let s = Selection::default();
        let p = paint();
        let green = [0x00, 0xff, 0x00];
        let c = ToolContext { smart_guide_color: green, ..cx(&d, &s, &p) };
        let t = Targets::collect(&d, &[], None).styled(&c);
        let (q, ov) = t.snap_point(Point::new(102.0, 99.0), 4.0);
        assert_eq!(q, Point::new(100.0, 100.0));
        assert!(matches!(&ov[0], Overlay::Label { text, color, .. } if text == "anchor" && *color == green), "{ov:?}");
        let (q, ov) = t.snap_point(Point::new(301.0, 199.0), 4.0);
        assert_eq!(q.y, 200.0);
        assert!(ov.iter().all(|o| matches!(o, Overlay::Line { color, .. } | Overlay::Label { color, .. } if *color == green)), "{ov:?}");
        assert!(
            ov.iter().any(|o| matches!(o, Overlay::Line { .. })) && ov.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "align"))
        );
        // Alignment Guides off: the point still lines up and says so, without the line.
        let c = ToolContext { alignment_guides: false, ..cx(&d, &s, &p) };
        let t = Targets::collect(&d, &[], None).styled(&c);
        let (q, ov) = t.snap_point(Point::new(301.0, 199.0), 4.0);
        assert_eq!(q.y, 200.0);
        assert!(!ov.iter().any(|o| matches!(o, Overlay::Line { .. })), "{ov:?}");
        assert!(ov.iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "align")), "{ov:?}");
        let (dv, ov) = t.snap_rect(Rect::new(2.0, 50.0, 52.0, 90.0), 4.0);
        assert_eq!((dv.x, ov.len()), (-2.0, 0));
        // A handle already in line moves nowhere, yet is still "align".
        let (q, ov) = t.snap_point_with(Point::new(400.0, 200.0), &[], 4.0);
        assert_eq!(q, Point::new(400.0, 200.0));
        assert!(matches!(&ov[..], [Overlay::Label { text, .. }] if text == "align"), "{ov:?}");
        // Anchor/Path Labels off: on the anchor, without saying so.
        let c = ToolContext { anchor_path_labels: false, ..cx(&d, &s, &p) };
        let t = Targets::collect(&d, &[], None).styled(&c);
        let (q, ov) = t.snap_point(Point::new(102.0, 99.0), 4.0);
        assert_eq!((q, ov.len()), (Point::new(100.0, 100.0), 0));
        let (q, ov) = t.snap_point(Point::new(301.0, 199.0), 4.0);
        assert_eq!(q.y, 200.0);
        assert!(ov.iter().all(|o| matches!(o, Overlay::Line { .. })) && !ov.is_empty(), "{ov:?}");
        assert_eq!(t.snap_guide(false, 199.0, 4.0), (200.0, vec![]));
    }

    /// Dragging an anchor of a path: its other anchors and the segments not touching it stay
    /// targets, but neither it nor the path's bounds (which move) are.
    #[test]
    fn anchor_drag_targets_leave_out_what_moves() {
        let (d, id) = doc_with_rect();
        let mut s = Selection::default();
        s.set([id]);
        s.anchors.insert(id, [(0, 0)].into_iter().collect());
        let t = Targets::for_anchor_drag(&d, &s);
        let anchors: Vec<Point> = t.points.iter().filter(|(_, k)| *k == Kind::Anchor).map(|(p, _)| *p).collect();
        assert_eq!(anchors, [Point::new(100.0, 200.0), Point::new(200.0, 100.0), Point::new(200.0, 200.0)], "by x");
        assert_eq!(t.segments.len(), 2);
        assert!(!t.points.iter().any(|(p, k)| *k == Kind::Center && *p == Point::new(150.0, 150.0)), "the rect's centre moves");
        assert_eq!(t.snap_point(Point::new(101.0, 99.0), 4.0).0, Point::new(100.0, 100.0), "in line with its neighbours, not on itself");
    }

    fn labels(ov: &[Overlay]) -> Vec<&str> {
        ov.iter().filter_map(|o| if let Overlay::Label { text, .. } = o { Some(text.as_str()) } else { None }).collect()
    }

    fn lines(ov: &[Overlay]) -> Vec<(Point, Point)> {
        ov.iter().filter_map(|o| if let Overlay::Line { a, b, .. } = o { Some((*a, *b)) } else { None }).collect()
    }

    /// A drawn point (#506): on another object's anchors, in line with them too (not only with its
    /// bounds), onto its paths; Shift slides it along its 45° step into line; near a construction
    /// guide through the point it leaves from it lands on the guide, where a target's line crosses
    /// it if one does nearby.
    #[test]
    fn drawn_points_land_on_anchors_paths_alignments_and_construction_guides() {
        let (d, _) = doc_with_rect();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let t = Targets::for_drawing(&d, &[], None).styled(&c);
        // The rect is 100..200: its corner, a point on its side, and in line with an anchor.
        let (q, ov) = t.snap_from(Point::new(202.0, 198.0), None, 4.0);
        assert_eq!((q, labels(&ov)), (Point::new(200.0, 200.0), vec!["anchor"]));
        let (q, ov) = t.snap_from(Point::new(202.0, 160.0), None, 4.0);
        assert_eq!((q, labels(&ov)), (Point::new(200.0, 160.0), vec!["path"]));
        let (q, ov) = t.snap_from(Point::new(330.0, 102.0), None, 4.0);
        assert_eq!((q, labels(&ov)), (Point::new(330.0, 100.0), vec!["align"]));
        // Shift from (300, 300): the pointer at 2° keeps to 0°, then slides onto x = 200.
        let shift = Leave::segment(&c, Point::new(300.0, 300.0), true);
        let (q, ov) = t.snap_from(Point::new(203.0, 303.0), Some(&shift), 4.0);
        assert!((q - Point::new(200.0, 300.0)).hypot() < 1e-9, "{q:?}");
        assert_eq!(labels(&ov), ["align"]);
        assert_eq!(lines(&ov).len(), 1, "{ov:?}");
        // Nothing in reach: on its angle, as far as the pointer.
        let (q, ov) = t.snap_from(Point::new(300.0, 380.0), Some(&shift), 4.0);
        assert_eq!((q, ov.len()), (Point::new(300.0, 380.0), 0));
        // Construction guides through (300, 320): onto the 135° one…
        let from = Leave::segment(&c, Point::new(300.0, 320.0), false);
        let (q, ov) = t.snap_from(Point::new(352.0, 373.0), Some(&from), 4.0);
        assert!((q - Point::new(352.5, 372.5)).hypot() < 1e-9, "{q:?}");
        assert_eq!(lines(&ov), vec![(Point::new(300.0, 320.0), q)]);
        // …where x = 200 crosses it.
        let (q, ov) = t.snap_from(Point::new(203.0, 222.5), Some(&from), 4.0);
        assert!((q - Point::new(200.0, 220.0)).hypot() < 1e-9, "{q:?}");
        assert_eq!((lines(&ov).len(), labels(&ov)), (2, vec!["align"]));
        // Construction Guides off (or Smart Guides off): in line, not on a guide.
        let off = ToolContext { construction_angles: &[], ..cx(&d, &s, &p) };
        let (q, _) = t.snap_from(Point::new(352.0, 373.0), Some(&Leave::segment(&off, Point::new(300.0, 320.0), false)), 4.0);
        assert_eq!(q, Point::new(352.0, 373.0));
        // A box's corner keeps to a diagonal with Shift, the farther axis giving its size.
        let (q, _) = t.snap_from(Point::new(340.0, 310.0), Some(&Leave::diagonal(Point::new(300.0, 300.0), true)), 4.0);
        assert_eq!(q, Point::new(340.0, 340.0));
    }

    /// The angle sets of Preferences › Smart Guides › Construction Guides.
    #[test]
    fn construction_angle_sets() {
        assert_eq!(construction_angles(DEFAULT_CONSTRUCTION_ANGLES), [0.0, 45.0, 90.0, 135.0]);
        assert_eq!(construction_angles("90° Angles"), [0.0, 90.0]);
        assert_eq!(construction_angles("30° Angles"), [30.0, 60.0, 120.0, 150.0]);
        assert_eq!(construction_angles("90° & 45° & 30° Angles").len(), 8);
        assert_eq!(construction_angles("nonsense"), construction_angles(DEFAULT_CONSTRUCTION_ANGLES));
    }

    /// The sorted targets find by bisection just what a scan of all of them finds, whatever
    /// gathered them.
    #[test]
    fn bisection_finds_what_a_scan_finds() {
        use vectorcraft_doc::{Appearance, Node};
        let (mut d, id) = doc_with_rect();
        let l = d.layers[0].id;
        for i in 0..60 {
            let (x, y) = ((i * 37 % 450) as f64, (i * 53 % 430) as f64);
            let tri = vectorcraft_geom::PathData::single(SubPath::polyline(
                &[Point::new(x, y), Point::new(x + 13.0, y + 3.0), Point::new(x + 5.0, y + 17.0)],
                true,
            ));
            let n = d.alloc_id();
            d.insert(Some(l), 1, Node::path(n, tri, Appearance::default_art())).unwrap();
        }
        let mut s = Selection::default();
        s.set([id]);
        s.anchors.insert(id, [(0, 1)].into_iter().collect());
        for t in [Targets::collect(&d, &[], None), Targets::for_drawing(&d, &[id], None), Targets::for_anchor_drag(&d, &s)] {
            for q in (0..40).flat_map(|i| (0..40).map(move |j| Point::new(i as f64 * 12.7, j as f64 * 11.3))) {
                let scan = t.points.iter().copied().filter(|(a, _)| a.distance(q) <= 6.0).min_by(|a, b| a.0.distance(q).total_cmp(&b.0.distance(q)));
                assert_eq!(t.nearest_point(q, 6.0).map(|h| h.0.distance(q)), scan.map(|h| h.0.distance(q)), "{q:?}");
                for (lines, v) in [(&t.xs, q.x), (&t.ys, q.y)] {
                    let mut near: Vec<f64> = lines_near(lines, v, 6.0).iter().map(|l| l.0).collect();
                    let mut all: Vec<f64> = lines.iter().map(|l| l.0).filter(|w| (w - v).abs() <= 6.0).collect();
                    near.sort_by(f64::total_cmp);
                    all.sort_by(f64::total_cmp);
                    assert_eq!(near, all);
                }
            }
        }
    }

    /// A drawing tool's snapper reuses its targets while hovering over the same document state, and
    /// gathers them again when it changes, and on every press.
    #[test]
    fn draw_snap_gathers_once_per_document_state() {
        let (d, _) = doc_with_rect();
        let empty = vectorcraft_doc::Document::new(500.0, 500.0);
        let (s, p) = (Selection::default(), paint());
        let at = Point::new(202.0, 198.0);
        let mut snap = DrawSnap::default();
        let c = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
        assert_eq!(snap.hover(&c, at, &[], None), Point::new(200.0, 200.0));
        assert_eq!(labels(snap.guides()), ["anchor"]);
        // The same revision: the targets are the ones gathered (the document isn't read again).
        let c = ToolContext { revision: (1, 1), ..cx(&empty, &s, &p) };
        assert_eq!(snap.hover(&c, at, &[], None), Point::new(200.0, 200.0));
        // A new revision, or a press, reads it.
        let c = ToolContext { revision: (1, 2), ..cx(&empty, &s, &p) };
        assert_eq!(snap.hover(&c, at, &[], None), at);
        assert!(snap.guides().is_empty());
        let c = ToolContext { revision: (1, 2), ..cx(&d, &s, &p) };
        assert_eq!(snap.press(&c, at, &[], None), Point::new(200.0, 200.0));
        // The drag keeps what the press gathered, though the document now shows its preview.
        let c = ToolContext { revision: (1, 3), ..cx(&empty, &s, &p) };
        assert_eq!(snap.drag(&c, at, None), Point::new(200.0, 200.0));
        snap.clear();
        assert!(snap.guides().is_empty());
        // Snap to Grid wins over Smart Guides, Shift still keeping the angle.
        let c = ToolContext { snap_to_grid: true, ..cx(&d, &s, &p) };
        let from = Leave::segment(&c, Point::ZERO, true);
        let q = snap.press(&c, Point::new(31.0, 2.0), &[], Some(&from));
        assert!((q.y - 0.0).abs() < 1e-9, "{q:?}");
    }
}
