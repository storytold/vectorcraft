//! Width tool (Shift+W).
//!
//! Hovering a stroked path shows a hollow width-point diamond with the stroke's width at that
//! spot. Dragging outward from the path creates (or, on an existing width point's handle end,
//! edits) a width point, scaling both sides together and keeping their ratio. Alt, held at any
//! time during the drag, frees the side under the pointer while the other side holds the width it
//! had when Alt went down; letting Alt go makes the point even again. Dragging a width point's
//! centre slides it along the path; Alt-dragging the centre slides a copy; dropping either onto
//! another width point makes a discontinuous point (the width steps there). Shift-dragging a point
//! moves or scales the whole profile with it: its centre re-spaces every width point along the path
//! in proportion (the ends stay put, so dragging an end point leaves a point where that end was and
//! moves a re-spaced one), its handle scales the width points on the same smooth stretch,
//! up to the nearest sharp corner; Alt alongside Shift scales only that handle's side. Shift-click
//! adds width points to the selection (or takes them out), and dragging one of several selected
//! points moves or widens them all — Alt then changes that one side of every selected point.
//! Double-clicking a stroke opens Width Point Edit (the `widthPoint` dialog), editing a hit point
//! or creating one on confirmation. Delete/Backspace removes the selected width points.
//!
//! Gestures preview `stroke.widthPoint.set {id, t, left, right, index?}` (side widths in points),
//! `stroke.widthPoint.copy {id, index, t}` or, for several points, `stroke.widthProfile.set {ids,
//! points}`, and commit on release; Delete runs `stroke.widthPoint.remove {id, indices}`.
//!
//! Escape deselects the selected width points and sends no command.
//!
//! A compound path's stroke is the compound's: its members are edited through it, and the width
//! points show on every subpath of the stroke (one profile runs along each). The cursor says what a
//! drag does: add a width point on a stroke, or move or widen the one under it.

use serde_json::json;
use vectorcraft_doc::{Document, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{AnchorKind, Point, Vec2};

use super::pathutil::{eval_fraction, fraction_at, left_normal, nearest_fraction};
use super::{BLUE, diamond};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// Width points closer than this (in t) are at the same place (the two sides of a discontinuous
/// point).
const SAME_T: f64 = 1e-6;

/// Where the pointer is relative to a stroked path.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Spot {
    /// The object whose stroke it is (a compound path for its members).
    owner: NodeId,
    /// The path under the pointer.
    id: NodeId,
    sub: usize,
    /// Fraction along the subpath.
    t: f64,
    p: Point,
    /// Left normal.
    n: Vec2,
    /// Side widths (points) at `t`.
    left: f64,
    right: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// Change widths; `Some(left?)` = only one side (Alt).
    Width(Option<bool>),
    /// Slide along the path.
    Move,
    /// Slide a copy along the path (Alt-drag the centre).
    Copy,
}

/// A width point as the tool shows it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct WPoint {
    index: usize,
    t: f64,
    /// On-path point and left normal.
    p: Point,
    n: Vec2,
    /// Side widths (points).
    left: f64,
    right: f64,
}

/// Several selected width points dragged together.
#[derive(Clone, Debug, PartialEq)]
struct Group {
    /// The profile's points when the drag began.
    points: Vec<(f64, f64, f64)>,
    /// Which of them move.
    moving: Vec<usize>,
    /// The stroke's half weight.
    half: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct Drag {
    /// Where the dragged point started (`t`, widths); `p` and `n` follow a slide.
    spot: Spot,
    index: Option<usize>,
    mode: Mode,
    left: f64,
    right: f64,
    t: f64,
    /// Where the pointer went down. A drag has to beat a few pixels before it counts as one, so a
    /// Shift-press that barely moves is still a Shift-click.
    start: Point,
    group: Option<Group>,
    /// The other width points of the subpath when the drag began, which a slid point snaps onto.
    others: Vec<WPoint>,
    moved: bool,
    /// The side the pointer is on (`true` = left).
    side: bool,
    /// The two sides the ratio counts from, which a plain drag scales together (re-based when the
    /// pointer crosses the path).
    base: (f64, f64),
    /// Alt held during the drag: both sides' widths from when Alt went down, so the side the
    /// pointer is not on keeps its own width (crossing the path swaps which side that is).
    held: Option<(f64, f64)>,
    /// Alt during an elastic width drag: scale only the side whose handle the drag began on.
    one_side: bool,
    /// Shift held on an existing point: the whole profile moves (its centre) or scales (its handle)
    /// with this point rather than it alone. A Shift-click instead selects it, decided on release.
    elastic: bool,
    /// The profile's `(t, left, right)` factors when the drag began, which an elastic drag
    /// re-spaces (a move) or scales (a width change). Captured at the start even for a plain drag:
    /// Shift may go down partway through, once the plain drag's preview is already on the document,
    /// so the profile read then is not the one to re-space from.
    profile: Vec<(f64, f64, f64)>,
    /// The smooth run this point sits on as `t` bounds (up to the nearest sharp corner each way):
    /// an elastic width drag scales only the width points inside it.
    run: (f64, f64),
    /// The factor an elastic width drag scales the run by.
    scale: f64,
}

#[derive(Default)]
pub struct WidthTool {
    hover: Option<Spot>,
    drag: Option<Drag>,
    /// Selected width points: the stroke's owner and their positions t.
    selected: Option<(NodeId, Vec<f64>)>,
}

/// The object whose stroke path `id` draws with: its compound path, if it is a member of one.
pub fn stroke_owner(doc: &Document, id: NodeId) -> NodeId {
    doc.parent_of(id).filter(|p| doc.node(*p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. }))).unwrap_or(id)
}

/// The visible weighted stroke of `owner`.
fn stroke_of(doc: &Document, owner: NodeId) -> Option<&StrokeLayer> {
    doc.node(owner)?.appearance.stroke().filter(|s| s.visible && s.width > 0.0 && !s.paint.is_none())
}

/// The paths a stroke owner draws: itself, or a compound path's members.
fn paths_of(doc: &Document, owner: NodeId) -> Vec<NodeId> {
    let mut out = vec![];
    if let Some(n) = doc.node(owner) {
        n.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Path { guide: false, .. }) {
                out.push(c.id);
            }
        });
    }
    out
}

/// Side widths (points) of a stroke at fraction `t`.
pub fn sides_at(st: &StrokeLayer, t: f64) -> (f64, f64) {
    let (l, r) = st.profile.as_ref().map(|p| p.at(t)).unwrap_or((1.0, 1.0));
    (l * st.width / 2.0, r * st.width / 2.0)
}

impl WidthTool {
    /// The stroked path nearest to `p` within reach (stroke half-width + a few pixels).
    fn spot_at(cx: &ToolContext, p: Point) -> Option<Spot> {
        let editable = cx.doc.editable();
        let consider = |id: NodeId| -> Option<(f64, Spot)> {
            if !editable(id) {
                return None;
            }
            let n = cx.doc.node(id)?;
            let NodeKind::Path { path, guide: false, .. } = &n.kind else { return None };
            let owner = stroke_owner(cx.doc, id);
            let st = stroke_of(cx.doc, owner)?;
            let widest = st.profile.as_ref().map_or(1.0, |pr| pr.points.iter().map(|q| q.1.max(q.2)).fold(1.0, f64::max));
            let reach = widest * st.width / 2.0 + cx.tol(4.0);
            if !path.bounds()?.inflate(reach, reach).contains(p) {
                return None;
            }
            let (sub, t, q, tan, d) = nearest_fraction(path, p)?;
            if d > reach {
                return None;
            }
            let (left, right) = sides_at(st, t);
            Some((d, Spot { owner, id, sub, t, p: q, n: left_normal(tan), left, right }))
        };
        let best_of = |ids: &mut dyn Iterator<Item = NodeId>| ids.filter_map(consider).min_by(|a, b| a.0.total_cmp(&b.0));
        // The selected paths first (a selected compound path: its members).
        if let Some(b) = best_of(&mut cx.selection.objects.iter().flat_map(|id| paths_of(cx.doc, *id))) {
            return Some(b.1);
        }
        let mut all = vec![];
        cx.doc.walk(|n| {
            if matches!(n.kind, NodeKind::Path { .. }) {
                all.push(n.id);
            }
        });
        best_of(&mut all.into_iter().rev()).map(|b| b.1)
    }

    /// Width points of `owner`'s stroke on subpath `sub` of path `id`.
    fn points_of(cx: &ToolContext, owner: NodeId, id: NodeId, sub: usize) -> Vec<WPoint> {
        let Some(st) = stroke_of(cx.doc, owner) else { return vec![] };
        let Some(pr) = &st.profile else { return vec![] };
        let Some(NodeKind::Path { path, .. }) = cx.doc.node(id).map(|n| &n.kind) else { return vec![] };
        let Some(sp) = path.subpaths.get(sub) else { return vec![] };
        pr.points
            .iter()
            .enumerate()
            .filter_map(|(index, (t, l, r))| {
                let (p, tan) = eval_fraction(sp, *t)?;
                Some(WPoint { index, t: *t, p, n: left_normal(tan), left: l * st.width / 2.0, right: r * st.width / 2.0 })
            })
            .collect()
    }

    /// Width points of `owner`'s stroke on every subpath it draws.
    fn all_points(cx: &ToolContext, owner: NodeId) -> Vec<WPoint> {
        let subpaths = |id: NodeId| match cx.doc.node(id).map(|n| &n.kind) {
            Some(NodeKind::Path { path, .. }) => path.subpaths.len(),
            _ => 0,
        };
        paths_of(cx.doc, owner).into_iter().flat_map(|id| (0..subpaths(id)).flat_map(move |sub| Self::points_of(cx, owner, id, sub))).collect()
    }

    /// The width point part nearest to `p` within a few pixels: a centre (`None`) or the end of a
    /// left (`Some(true)`) or right (`Some(false)`) handle.
    fn hit(cx: &ToolContext, spot: &Spot, p: Point) -> Option<(WPoint, Option<bool>)> {
        let tol = cx.tol(5.0);
        Self::points_of(cx, spot.owner, spot.id, spot.sub)
            .into_iter()
            .flat_map(|w| [(w, None, w.p), (w, Some(true), w.p + w.n * w.left), (w, Some(false), w.p - w.n * w.right)])
            .map(|(w, part, q)| (q.distance(p), w, part))
            .filter(|h| h.0 <= tol)
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, w, part)| (w, part))
    }

    fn is_selected(&self, id: NodeId, t: f64) -> bool {
        self.selected.as_ref().is_some_and(|(sid, ts)| *sid == id && ts.iter().any(|s| (s - t).abs() <= SAME_T))
    }

    /// Shift-click on the width point at `t` of `id`: add it to the selection or take it out.
    /// → whether it is selected now.
    fn toggle(&mut self, id: NodeId, t: f64) -> bool {
        match &mut self.selected {
            Some((sid, ts)) if *sid == id => match ts.iter().position(|s| (s - t).abs() <= SAME_T) {
                Some(i) => {
                    ts.remove(i);
                    false
                }
                None => {
                    ts.push(t);
                    true
                }
            },
            _ => {
                self.selected = Some((id, vec![t]));
                true
            }
        }
    }

    /// The indices of the selected width points of `id` (both sides of a discontinuous point).
    fn selected_indices(&self, cx: &ToolContext, id: NodeId) -> Vec<usize> {
        let Some(pr) = stroke_of(cx.doc, id).and_then(|s| s.profile.as_ref()) else { return vec![] };
        (0..pr.points.len()).filter(|i| self.is_selected(id, pr.points[*i].0)).collect()
    }

    /// A drag of `id`'s selected points together: a move when more than one profile point is
    /// selected (a discontinuous point moves as one), a width change when several are.
    fn group(&self, cx: &ToolContext, id: NodeId, mode: Mode) -> Option<Group> {
        let st = stroke_of(cx.doc, id)?;
        let moving = self.selected_indices(cx, id);
        let several = match mode {
            Mode::Move => moving.len() > 1,
            Mode::Width(_) => self.selected.as_ref().is_some_and(|(_, ts)| ts.len() > 1),
            Mode::Copy => false,
        };
        several.then(|| Group { points: st.profile.as_ref().map(|p| p.points.clone()).unwrap_or_default(), moving, half: st.width / 2.0 })
    }

    /// The `t` bounds of the smooth run a width point at `t` sits on: the nearest sharp corner
    /// each way, as fractions along the subpath. Wider than the path when there is none.
    fn smooth_run(cx: &ToolContext, id: NodeId, sub: usize, t: f64) -> (f64, f64) {
        let Some(NodeKind::Path { path, .. }) = cx.doc.node(id).map(|n| &n.kind) else { return (0.0, 1.0) };
        let Some(sp) = path.subpaths.get(sub) else { return (0.0, 1.0) };
        let segs = sp.segment_count();
        let fraction = |i: usize| {
            if segs == 0 {
                0.0
            } else if i >= segs {
                fraction_at(sp, segs - 1, 1.0)
            } else {
                fraction_at(sp, i, 0.0)
            }
        };
        let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
        let (mut first, mut last) = (f64::INFINITY, f64::NEG_INFINITY);
        let n = sp.anchors.len();
        for (i, a) in sp.anchors.iter().enumerate() {
            // An open path's ends are where the stroke stops, not sharp corners, so they do not
            // bound a run: a width point at either end scales with it.
            if a.kind != AnchorKind::Corner || (!sp.closed && (i == 0 || i + 1 == n)) {
                continue;
            }
            let f = fraction(i);
            first = first.min(f);
            last = last.max(f);
            // A corner at `t` (within the same rounding tolerance used for width points) bounds
            // the run on both sides: the point sits on the corner itself, not on a smooth
            // stretch, so only that point scales. Comparing `f` and `t` exactly can make a tiny
            // rounding difference leave one side unbounded.
            let on_corner = (f - t).abs() <= SAME_T;
            if on_corner || f < t {
                lo = lo.max(f);
            }
            if on_corner || f > t {
                hi = hi.min(f);
            }
        }
        // A closed path has no ends: a run with no corner on one side carries on across the seam,
        // from the last corner round to the first, so its bound lies outside 0..1.
        if sp.closed && first <= last {
            if lo == f64::NEG_INFINITY {
                lo = last - 1.0;
            }
            if hi == f64::INFINITY {
                hi = first + 1.0;
            }
        }
        (lo, hi)
    }

    /// The command previewing drag `d`.
    fn preview(d: &Drag) -> Action {
        let id = d.spot.owner.0;
        if d.elastic {
            return Action::Preview("stroke.widthProfile.set".into(), json!({"ids": [id], "points": Self::elastic_points(d)}));
        }
        if let Some(g) = &d.group {
            let (dt, dl, dr) = (d.t - d.spot.t, (d.left - d.spot.left) / g.half, (d.right - d.spot.right) / g.half);
            let mut pts: Vec<[f64; 3]> = g
                .points
                .iter()
                .enumerate()
                .map(|(i, &(t, l, r))| match (g.moving.contains(&i), d.mode) {
                    (true, Mode::Move) => [(t + dt).clamp(0.0, 1.0), l, r],
                    (true, _) => [t, (l + dl).max(0.0), (r + dr).max(0.0)],
                    (false, _) => [t, l, r],
                })
                .collect();
            pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
            return Action::Preview("stroke.widthProfile.set".into(), json!({"ids": [id], "points": pts}));
        }
        match (d.mode, d.index) {
            (Mode::Copy, Some(i)) => Action::Preview("stroke.widthPoint.copy".into(), json!({"id": id, "index": i, "t": d.t})),
            (_, index) => {
                let mut v = json!({"id": id, "t": d.t, "left": d.left, "right": d.right});
                if let Some(i) = index {
                    v["index"] = json!(i);
                }
                Action::Preview("stroke.widthPoint.set".into(), v)
            }
        }
    }

    /// The profile an elastic drag previews. A move re-spaces every point along the path, so the
    /// gaps either side of the dragged point stretch or squeeze in proportion and the ends stay
    /// put — dragging an end point keeps a point there and moves the re-spaced one; a width change
    /// scales the points on the dragged point's smooth run by the same factor (the width the
    /// pointer pulls its handle to over the width it started at).
    fn elastic_points(d: &Drag) -> Vec<[f64; 3]> {
        match d.mode {
            Mode::Move => {
                let (t0, t1) = (d.spot.t, d.t);
                let lo = d.profile.first().map_or(0.0, |q| q.0);
                let hi = d.profile.last().map_or(1.0, |q| q.0);
                let mut pts: Vec<[f64; 3]> = d
                    .profile
                    .iter()
                    .map(|&(t, l, r)| {
                        // Anchored at the profile's ends: everything before the dragged point
                        // scales by (t1 - lo)/(t0 - lo), everything after by (hi - t1)/(hi - t0).
                        let t = if t <= t0 {
                            let span = t0 - lo;
                            if span <= 1e-12 { t1 } else { lo + (t - lo) * (t1 - lo) / span }
                        } else {
                            let span = t0 - hi;
                            if span >= -1e-12 { t1 } else { hi + (t - hi) * (t1 - hi) / span }
                        };
                        [t.clamp(0.0, 1.0), l, r]
                    })
                    .collect();
                // The ends are the anchors, so dragging one of them would slide it off the end and
                // leave the path's start or finish without a width point. Keep a point where that
                // end was, with the same widths, and let the re-spaced point carry the drag.
                let kept = if (t0 - lo).abs() <= SAME_T {
                    d.profile.first().copied()
                } else if (t0 - hi).abs() <= SAME_T {
                    d.profile.last().copied()
                } else {
                    None
                };
                if let Some((t, l, r)) = kept {
                    pts.push([t, l, r]);
                }
                pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
                pts
            }
            _ => {
                let (lo, hi) = d.run;
                // A closed run that wraps past the seam has a bound outside 0..1, so a width point
                // near the other end of the path belongs to it only when shifted across the seam.
                let wrapped = (lo.is_finite() && lo < 0.0) || (hi.is_finite() && hi > 1.0);
                let in_run = |t: f64| {
                    if wrapped { [t - 1.0, t, t + 1.0].into_iter().any(|q| q > lo && q < hi) } else { t > lo && t < hi }
                };
                d.profile
                    .iter()
                    .enumerate()
                    .map(|(i, &(t, l, r))| {
                        if Some(i) != d.index && !in_run(t) {
                            return [t, l, r];
                        }
                        // Alt scales the side under the handle only; the other one holds.
                        match (d.one_side, d.side) {
                            (true, true) => [t, l * d.scale, r],
                            (true, false) => [t, l, r * d.scale],
                            (false, _) => [t, l * d.scale, r * d.scale],
                        }
                    })
                    .collect()
            }
        }
    }
}

impl Tool for WidthTool {
    fn id(&self) -> &'static str {
        "width"
    }
    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move => {
                self.hover = Self::spot_at(cx, p);
                vec![]
            }
            PointerKind::DoubleClick => {
                let Some(spot) = Self::spot_at(cx, p) else { return vec![] };
                let Some((w, _)) = Self::hit(cx, &spot, p) else {
                    self.selected = Some((spot.owner, vec![spot.t]));
                    return vec![Action::Dialog("widthPoint".into(), json!({"id": spot.owner.0, "t": spot.t}))];
                };
                self.selected = Some((spot.owner, vec![w.t]));
                vec![Action::Dialog("widthPoint".into(), json!({"id": spot.owner.0, "index": w.index, "adjustAdjoining": ev.mods.shift}))]
            }
            PointerKind::Down => {
                let Some(spot) = Self::spot_at(cx, p) else {
                    if !ev.mods.shift {
                        self.selected = None;
                    }
                    return vec![];
                };
                let drag = match Self::hit(cx, &spot, p) {
                    Some((w, part)) => {
                        let mut mode = match part {
                            None if ev.mods.alt => Mode::Copy,
                            None => Mode::Move,
                            Some(left) => Mode::Width(ev.mods.alt.then_some(left)),
                        };
                        // Shift on an existing point drags the whole profile with it. A Shift-click
                        // (released without moving) selects instead, so the selection waits. Alt
                        // alongside Shift limits an elastic width drag to the handle's own side.
                        let elastic = ev.mods.shift && mode != Mode::Copy;
                        if !elastic {
                            if ev.mods.shift {
                                if !self.toggle(spot.owner, w.t) {
                                    return vec![];
                                }
                            } else if mode == Mode::Copy || !self.is_selected(spot.owner, w.t) {
                                self.selected = Some((spot.owner, vec![w.t]));
                            }
                        }
                        let others = Self::points_of(cx, spot.owner, spot.id, spot.sub).into_iter().filter(|o| o.index != w.index).collect();
                        let group = if elastic { None } else { self.group(cx, spot.owner, mode) };
                        // Several selected points take Alt live, like one does: the side is decided
                        // while the drag runs rather than at the press.
                        if group.is_some() && matches!(mode, Mode::Width(Some(_))) {
                            mode = Mode::Width(None);
                        }
                        // Capture the profile as it is now: the drag's own previews go onto the
                        // document, so this is the last point at which it is the profile the drag
                        // began with, which an elastic move or width change spaces from.
                        let profile = stroke_of(cx.doc, spot.owner).and_then(|s| s.profile.as_ref()).map(|pr| pr.points.clone()).unwrap_or_default();
                        let run = if elastic && matches!(mode, Mode::Width(_)) { Self::smooth_run(cx, spot.id, spot.sub, w.t) } else { (0.0, 1.0) };
                        let spot = Spot { t: w.t, p: w.p, n: w.n, left: w.left, right: w.right, ..spot };
                        Drag {
                            spot,
                            index: Some(w.index),
                            mode,
                            left: w.left,
                            right: w.right,
                            t: w.t,
                            start: p,
                            group,
                            others,
                            moved: false,
                            side: part.unwrap_or(true),
                            base: (w.left, w.right),
                            held: None,
                            one_side: false,
                            elastic,
                            profile,
                            run,
                            scale: 1.0,
                        }
                    }
                    None => {
                        let side = (p - spot.p).dot(spot.n) >= 0.0;
                        self.selected = Some((spot.owner, vec![spot.t]));
                        let mode = Mode::Width(ev.mods.alt.then_some(side));
                        Drag {
                            spot,
                            index: None,
                            mode,
                            left: spot.left,
                            right: spot.right,
                            t: spot.t,
                            start: p,
                            group: None,
                            others: vec![],
                            moved: false,
                            side,
                            base: (spot.left, spot.right),
                            held: None,
                            one_side: false,
                            elastic: false,
                            profile: vec![],
                            run: (0.0, 1.0),
                            scale: 1.0,
                        }
                    }
                };
                let label = if drag.mode == Mode::Copy {
                    "Copy Width Point"
                } else if drag.elastic {
                    "Width Profile"
                } else {
                    "Width Point"
                };
                self.drag = Some(drag);
                vec![Action::Begin(label.into())]
            }
            PointerKind::Drag => {
                let Some(d) = &mut self.drag else { return vec![] };
                // A few pixels of jitter must not turn a Shift-click into a drag: the click is
                // decided on release, so the press only counts as a drag once it has moved.
                d.moved |= p.distance(d.start) > cx.tol(2.0);
                // Shift may go down or up partway through a drag: match the live modifier so a plain
                // drag turns elastic (and back) without letting go. Alt alongside it (see below)
                // narrows an elastic width drag to one side; on its own it keeps its own path.
                // Only an existing point can be elastic: a drag creating one has no profile to
                // re-space, so Shift leaves it alone. Entering captures the smooth run, which the
                // drag's own previews don't change (the profile it spaces from was taken at Down).
                let elastic_now = ev.mods.shift && d.mode != Mode::Copy && d.index.is_some();
                if elastic_now != d.elastic {
                    if elastic_now {
                        d.run = if matches!(d.mode, Mode::Width(_)) { Self::smooth_run(cx, d.spot.id, d.spot.sub, d.spot.t) } else { (0.0, 1.0) };
                    }
                    d.elastic = elastic_now;
                }
                if d.elastic {
                    match d.mode {
                        // The point follows the pointer along the path; the preview re-spaces the
                        // rest of the profile around it.
                        Mode::Move => {
                            let Some(NodeKind::Path { path, .. }) = cx.doc.node(d.spot.id).map(|n| &n.kind) else { return vec![] };
                            let Some(sp) = path.subpaths.get(d.spot.sub) else { return vec![] };
                            let Some((_, t, q, tan, _)) = nearest_fraction(&vectorcraft_geom::PathData::single(sp.clone()), p) else { return vec![] };
                            // The profile's ends stay put, so the dragged point stops at the first or
                            // last of them rather than overtaking one. Re-evaluate the spot at the
                            // clamped fraction so the overlay and the committed selection agree.
                            let lo = d.profile.first().map_or(0.0, |q| q.0);
                            let hi = d.profile.last().map_or(1.0, |q| q.0);
                            let t = t.max(lo).min(hi);
                            let (q, tan) = eval_fraction(sp, t).unwrap_or((q, tan));
                            (d.t, d.spot.p, d.spot.n) = (t, q, left_normal(tan));
                        }
                        // The handle follows the pointer; the preview scales that point's smooth run
                        // by the same factor. Alt scales the handle's own side only, so the other
                        // side of every point in the run keeps its width.
                        _ => {
                            let off = (p - d.spot.p).dot(d.spot.n);
                            d.one_side = ev.mods.alt;
                            let base = if d.side { d.spot.left } else { d.spot.right };
                            d.scale = if base > 1e-9 { off.abs() / base } else { 1.0 };
                            (d.left, d.right) = if d.one_side {
                                if d.side { (d.spot.left * d.scale, d.spot.right) } else { (d.spot.left, d.spot.right * d.scale) }
                            } else {
                                (d.spot.left * d.scale, d.spot.right * d.scale)
                            };
                        }
                    }
                    return vec![Self::preview(d)];
                }
                match d.mode {
                    // An existing point (or several) takes Alt live; a new one is created at the
                    // pointer's width, or on one side of it when Alt is already down.
                    Mode::Width(_) if d.index.is_some() => {
                        // The pointer along the left normal: + on the left side, - on the right.
                        let off = (p - d.spot.p).dot(d.spot.n);
                        let mag = off.abs();
                        let side_now = off >= 0.0;
                        // Crossing the path hands the drag to the other side; the ratio counts from
                        // there, so the change is smooth. With both sides at zero there is no ratio
                        // to count from, so keep the previous baseline (rather than wiping it) and
                        // preserve the ratio for dragging the opposite side back out.
                        if side_now != d.side {
                            let (l, r) = (d.left.max(0.0), d.right.max(0.0));
                            if l > 0.0 || r > 0.0 {
                                d.base = (l, r);
                            }
                            d.side = side_now;
                        }
                        if ev.mods.alt {
                            // Alt: the side under the pointer follows it, the other holds the width
                            // it had when Alt went down. Both sides are captured, so crossing the
                            // path hands the drag over without swapping their widths.
                            let (l0, r0) = *d.held.get_or_insert((d.left, d.right));
                            if d.side {
                                (d.left, d.right) = (mag, r0);
                            } else {
                                (d.right, d.left) = (mag, l0);
                            }
                        } else {
                            // Letting Alt go makes the point even again, at the pointer's width.
                            if d.held.take().is_some() {
                                d.base = (mag, mag);
                            }
                            // The side under the pointer follows it; the other keeps their ratio.
                            let (base, other) = if d.side { (d.base.0, d.base.1) } else { (d.base.1, d.base.0) };
                            let other = (other * if base > 1e-9 { mag / base } else { 1.0 }).max(0.0);
                            if d.side {
                                (d.left, d.right) = (mag, other);
                            } else {
                                (d.right, d.left) = (mag, other);
                            }
                        }
                    }
                    Mode::Width(side) => {
                        let off = (p - d.spot.p).dot(d.spot.n);
                        match side {
                            None => {
                                d.left = off.abs();
                                d.right = off.abs();
                            }
                            Some(true) => d.left = off.max(0.0),
                            Some(false) => d.right = (-off).max(0.0),
                        }
                    }
                    Mode::Move | Mode::Copy => {
                        let Some(NodeKind::Path { path, .. }) = cx.doc.node(d.spot.id).map(|n| &n.kind) else { return vec![] };
                        let Some(sp) = path.subpaths.get(d.spot.sub) else { return vec![] };
                        let Some((_, t, q, tan, _)) = nearest_fraction(&vectorcraft_geom::PathData::single(sp.clone()), p) else { return vec![] };
                        (d.t, d.spot.p, d.spot.n) = (t, q, left_normal(tan));
                        // Dropped onto another width point (a single point only): it lands exactly
                        // there, making a discontinuous point.
                        let tol = cx.tol(5.0);
                        if d.group.is_none()
                            && let Some(o) = d.others.iter().find(|o| o.p.distance(q) <= tol)
                        {
                            (d.t, d.spot.p, d.spot.n) = (o.t, o.p, o.n);
                        }
                    }
                }
                vec![Self::preview(d)]
            }
            PointerKind::Up => match self.drag.take() {
                Some(mut d) => {
                    // A Shift-click selects (or deselects) the point: nothing was dragged, so the
                    // interaction it began is cancelled rather than committed.
                    if d.elastic && !d.moved {
                        self.toggle(d.spot.owner, d.spot.t);
                        return vec![Action::Cancel];
                    }
                    let mut acts = vec![];
                    // Alt let go with no move since (the canvas only sends a drag when the pointer
                    // moves): the point still evens off, at the pointer's width, before committing.
                    if !ev.mods.alt && d.held.take().is_some() {
                        let mag = (p - d.spot.p).dot(d.spot.n).abs();
                        (d.left, d.right) = (mag, mag);
                        acts.push(Self::preview(&d));
                    }
                    let dt = d.t - d.spot.t;
                    // An elastic move re-spaces the profile, so the group's uniform `dt` would leave
                    // the selection's `t` values off the points it committed; keep only the dragged
                    // point selected then, and leave grouped Width drags to move the group.
                    let grouped = d.group.is_some() && !(d.elastic && d.mode == Mode::Move);
                    self.selected = match (grouped, d.moved, self.selected.take()) {
                        // A click on one of several selected points selects just that one.
                        (true, false, _) if !ev.mods.shift => Some((d.spot.owner, vec![d.spot.t])),
                        // Points moved together stay selected.
                        (true, _, Some((id, ts))) if d.mode == Mode::Move => Some((id, ts.into_iter().map(|t| (t + dt).clamp(0.0, 1.0)).collect())),
                        (true, _, sel) => sel,
                        (false, _, _) => Some((d.spot.owner, vec![d.t])),
                    };
                    acts.push(Action::Commit);
                    acts
                }
                None => vec![],
            },
        }
    }
    /// Delete/Backspace remove the selected width points and Escape deselects them; with none
    /// selected the keys do what they do with other tools (Clear deletes the selected objects, and
    /// Escape leaves isolation mode, for one).
    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::Delete | ToolKey::Backspace | ToolKey::Escape)
            && self.drag.is_none()
            && self.selected.as_ref().is_some_and(|(id, _)| !self.selected_indices(cx, *id).is_empty())
    }
    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.drag.is_none() {
            self.selected = None;
            return vec![];
        }
        if !matches!(key, ToolKey::Delete | ToolKey::Backspace) {
            return vec![];
        }
        let Some(id) = self.selected.as_ref().map(|s| s.0) else { return vec![] };
        let indices = self.selected_indices(cx, id);
        if indices.is_empty() {
            return vec![];
        }
        self.selected = None;
        vec![Action::Exec("stroke.widthPoint.remove".into(), json!({"id": id.0, "indices": indices}))]
    }
    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        let r = cx.tol(4.0);
        let focus = self.drag.as_ref().map(|d| d.spot).or(self.hover);
        // Every subpath's width points of the stroke under the pointer, and of the selected ones'.
        let show = |owner: NodeId, out: &mut Vec<Overlay>| {
            for w in Self::all_points(cx, owner) {
                let sel = self.is_selected(owner, w.t);
                out.push(Overlay::Line { a: w.p + w.n * w.left, b: w.p - w.n * w.right, color: BLUE, dashed: false });
                out.push(Overlay::Path { path: diamond(w.p, r), color: BLUE, width: if sel { 2.5 } else { 1.0 }, dashed: false });
                out.push(Overlay::Handle { p: w.p + w.n * w.left, color: BLUE });
                out.push(Overlay::Handle { p: w.p - w.n * w.right, color: BLUE });
            }
        };
        if let Some(s) = focus {
            show(s.owner, &mut out);
        }
        if let Some((owner, _)) = &self.selected
            && focus.is_none_or(|s| s.owner != *owner)
        {
            show(*owner, &mut out);
        }
        if let Some(d) = &self.drag {
            let q = d.spot.p;
            out.push(Overlay::Line { a: q + d.spot.n * d.left, b: q - d.spot.n * d.right, color: BLUE, dashed: false });
            out.push(Overlay::Path { path: diamond(q, r), color: BLUE, width: 2.0, dashed: false });
            if cx.measurement_labels {
                out.push(Overlay::Measure { p: q + d.spot.n * (d.left + r * 3.0), text: format!("W: {}", cx.stroke_unit.readout(d.left + d.right)) });
            }
        } else if let Some(h) = &self.hover {
            out.push(Overlay::Line { a: h.p + h.n * h.left, b: h.p - h.n * h.right, color: BLUE, dashed: true });
            out.push(Overlay::Path { path: diamond(h.p, r), color: BLUE, width: 1.0, dashed: false });
        }
        out
    }
    fn cursor(&self, cx: &ToolContext, p: Point, _mods: Mods) -> Cursor {
        if self.drag.is_some() {
            return Cursor::WidthPoint;
        }
        // The stroke under the pointer, as the last move found it (no search every frame).
        match self.hover {
            Some(spot) if Self::hit(cx, &spot, p).is_some() => Cursor::WidthPoint,
            Some(_) => Cursor::WidthAdd,
            None => Cursor::Width,
        }
    }
    fn busy(&self) -> bool {
        self.drag.is_some()
    }
    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.hover = None;
        if self.drag.take().is_some() { vec![Action::Commit] } else { vec![] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::{Appearance, Node, Selection, WidthProfile};
    use vectorcraft_geom::Point;
    use vectorcraft_geom::{PathData, SubPath};

    fn doc_line() -> (Document, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut ap = Appearance::default_art();
        ap.stroke_mut().unwrap().width = 10.0;
        let path = PathData::single(SubPath::polyline(&[Point::new(100.0, 200.0), Point::new(300.0, 200.0)], false));
        d.insert(Some(l), 0, Node::path(id, path, ap)).unwrap();
        (d, id)
    }

    /// The line with width points at t = 0.25, 0.5 and 0.75 (x = 150, 200, 250; 5 pt either side,
    /// the stroke's own width).
    fn doc_three_points() -> (Document, NodeId) {
        let (mut d, id) = doc_line();
        let points = vec![(0.0, 1.0, 1.0), (0.25, 1.0, 1.0), (0.5, 1.0, 1.0), (0.75, 1.0, 1.0), (1.0, 1.0, 1.0)];
        d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().profile = Some(WidthProfile { points });
        (d, id)
    }

    /// An L-shaped polyline (100, 200) → (200, 200) → (200, 300): a sharp corner at t = 0.5, with
    /// width points on the first leg at t = 0.1 and 0.25 and past the corner at 0.5 and 0.75.
    fn doc_corner() -> (Document, NodeId) {
        let (mut d, id) = doc_line();
        let points = vec![(0.1, 1.0, 1.0), (0.25, 2.0, 2.0), (0.5, 1.0, 1.0), (0.75, 1.0, 1.0)];
        d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().profile = Some(WidthProfile { points });
        let n = d.node_mut(id).unwrap();
        let NodeKind::Path { path, .. } = &mut n.kind else { panic!("a path") };
        path.subpaths = vec![SubPath::polyline(&[Point::new(100.0, 200.0), Point::new(200.0, 200.0), Point::new(200.0, 300.0)], false)];
        (d, id)
    }

    fn ev(kind: PointerKind, x: f64, y: f64, mods: Mods) -> PointerEvent {
        PointerEvent::new(kind, x, y).with_mods(mods)
    }

    fn preview(acts: &[Action]) -> (&str, &serde_json::Value) {
        match acts {
            [Action::Preview(cmd, v)] => (cmd.as_str(), v),
            _ => panic!("{acts:?}"),
        }
    }

    #[test]
    fn drag_outward_creates_symmetric_width_point() {
        let (d, id) = doc_line();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Down, 150.0, 201.0)), vec![Action::Begin("Width Point".into())]);
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 150.0, 220.0));
        let (cmd, v) = preview(&acts);
        assert_eq!(cmd, "stroke.widthPoint.set");
        assert_eq!(v["id"], json!(id.0));
        assert!((v["t"].as_f64().unwrap() - 0.25).abs() < 1e-3);
        assert!((v["left"].as_f64().unwrap() - 20.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 20.0).abs() < 1e-6);
        assert_eq!(t.pointer(&c, &PointerEvent::new(PointerKind::Up, 150.0, 220.0)), vec![Action::Commit]);
    }

    /// A drag that creates a width point has no profile to re-space, so holding Shift partway
    /// through must leave it alone rather than preview a profile set with nothing in it (which
    /// would clear the stroke's width points).
    #[test]
    fn shift_mid_drag_while_creating_a_point_keeps_creating_it() {
        let (d, _) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let (none, shift) = (Mods::default(), Mods { shift: true, ..Default::default() });
        // Drag outward from a bare stretch of the path (between the points at t = 0.25 and 0.5).
        t.pointer(&c, &ev(PointerKind::Down, 175.0, 201.0, none));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 175.0, 215.0, none));
        let (cmd, _) = preview(&acts);
        assert_eq!(cmd, "stroke.widthPoint.set");
        // Holding Shift keeps creating that point, at the pointer's width.
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 175.0, 215.0, shift));
        let (cmd, v) = preview(&acts);
        assert_eq!(cmd, "stroke.widthPoint.set", "{v}");
        assert!((v["left"].as_f64().unwrap() - 15.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 15.0).abs() < 1e-6, "{v}");
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 175.0, 215.0, shift)), vec![Action::Commit]);
    }

    #[test]
    fn alt_drag_changes_one_side_and_move_slides() {
        let (mut d, id) = doc_line();
        let n = d.node_mut(id).unwrap();
        n.appearance.stroke_mut().unwrap().profile = Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 2.0, 2.0), (1.0, 1.0, 1.0)] });
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        // Alt-drag the left handle end (y = 200 - 10) further up.
        let alt = Mods { alt: true, ..Default::default() };
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 200.0, 190.0).with_mods(alt));
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 200.0, 170.0).with_mods(alt));
        let (_, v) = preview(&acts);
        assert_eq!(v["index"], json!(1));
        assert!((v["left"].as_f64().unwrap() - 30.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 10.0).abs() < 1e-6);
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 200.0, 170.0));
        let del = t.key(&c, ToolKey::Delete, Mods::default());
        assert_eq!(del, vec![Action::Exec("stroke.widthPoint.remove".into(), json!({"id": id.0, "indices": [1]}))]);
        // Drag the centre along the path.
        t.pointer(&c, &PointerEvent::new(PointerKind::Down, 200.0, 200.0));
        let acts = t.pointer(&c, &PointerEvent::new(PointerKind::Drag, 250.0, 205.0));
        let (_, v) = preview(&acts);
        assert!((v["t"].as_f64().unwrap() - 0.75).abs() < 1e-3);
        t.pointer(&c, &PointerEvent::new(PointerKind::Up, 250.0, 205.0));
        assert!(t.selected.as_ref().is_some_and(|(_, ts)| (ts[0] - 0.75).abs() < 1e-3), "the moved point stays selected");
    }

    #[test]
    fn live_alt_frees_one_side_mid_drag_and_returns_to_even() {
        let (mut d, id) = doc_line();
        d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().profile =
            Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 2.0, 2.0), (1.0, 1.0, 1.0)] });
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let alt = Mods { alt: true, ..Default::default() };
        // A plain drag out of the left handle end widens both sides evenly.
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 190.0, Mods::default()));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 200.0, 170.0, Mods::default()));
        let (_, v) = preview(&acts);
        assert!((v["left"].as_f64().unwrap() - 30.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 30.0).abs() < 1e-6, "{v}");
        // Holding Alt partway through freezes the other side at the width it had.
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 200.0, 160.0, alt));
        let (_, v) = preview(&acts);
        assert!((v["left"].as_f64().unwrap() - 40.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 30.0).abs() < 1e-6, "{v}");
        // Letting Alt go makes the point even again, at the pointer's width.
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 200.0, 150.0, Mods::default()));
        let (_, v) = preview(&acts);
        assert!((v["left"].as_f64().unwrap() - 50.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 50.0).abs() < 1e-6, "{v}");
        t.pointer(&c, &ev(PointerKind::Up, 200.0, 150.0, Mods::default()));
    }

    #[test]
    fn releasing_alt_without_a_move_still_evens_the_point() {
        let (mut d, id) = doc_line();
        d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().profile =
            Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 2.0, 2.0), (1.0, 1.0, 1.0)] });
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let alt = Mods { alt: true, ..Default::default() };
        // Widen both sides evenly, then Alt-drag the left one out: the right side freezes at 30.
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 190.0, Mods::default()));
        t.pointer(&c, &ev(PointerKind::Drag, 200.0, 170.0, Mods::default()));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 200.0, 160.0, alt));
        let (_, v) = preview(&acts);
        assert!((v["left"].as_f64().unwrap() - 40.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 30.0).abs() < 1e-6, "{v}");
        // Alt and the mouse go up together, with no drag in between: the point still evens off.
        let acts = t.pointer(&c, &ev(PointerKind::Up, 200.0, 160.0, Mods::default()));
        assert_eq!(acts.len(), 2, "{acts:?}");
        assert!(
            matches!(&acts[0], Action::Preview(_, v) if (v["left"].as_f64().unwrap() - 40.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 40.0).abs() < 1e-6),
            "{acts:?}"
        );
        assert_eq!(acts[1], Action::Commit);
    }

    #[test]
    fn alt_drag_across_the_path_holds_each_side_at_its_own_width() {
        let (mut d, id) = doc_line();
        d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().profile =
            Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 1.0, 3.0), (1.0, 1.0, 1.0)] });
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let alt = Mods { alt: true, ..Default::default() };
        // The point is 5 pt left, 15 pt right; a plain drag out to 10 scales both (10 and 30).
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 195.0, Mods::default()));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 200.0, 190.0, Mods::default()));
        let (_, v) = preview(&acts);
        assert!((v["left"].as_f64().unwrap() - 10.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 30.0).abs() < 1e-6, "{v}");
        // Alt: the left goes out to 15, the right holds the 30 it had.
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 200.0, 185.0, alt));
        let (_, v) = preview(&acts);
        assert!((v["left"].as_f64().unwrap() - 15.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 30.0).abs() < 1e-6, "{v}");
        // Crossing the path drags the right side; the left goes back to the 10 it had when Alt
        // went down (rather than taking the right's 30, as a single held value would).
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 200.0, 205.0, alt));
        let (_, v) = preview(&acts);
        assert!((v["right"].as_f64().unwrap() - 5.0).abs() < 1e-6 && (v["left"].as_f64().unwrap() - 10.0).abs() < 1e-6, "{v}");
        t.pointer(&c, &ev(PointerKind::Up, 200.0, 205.0, alt));
    }

    #[test]
    fn a_plain_drag_scales_both_sides_keeping_their_ratio() {
        let (mut d, id) = doc_line();
        d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().profile =
            Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 1.0, 3.0), (1.0, 1.0, 1.0)] });
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        // The point at t = 0.5 is 5 pt one side and 15 the other; dragging its left handle end out
        // to 15 keeps the ratio, so the other side grows to 45 rather than resetting even.
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 195.0, Mods::default()));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 200.0, 185.0, Mods::default()));
        let (_, v) = preview(&acts);
        assert!((v["left"].as_f64().unwrap() - 15.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 45.0).abs() < 1e-6, "{v}");
        t.pointer(&c, &ev(PointerKind::Up, 200.0, 185.0, Mods::default()));
    }

    #[test]
    fn crossing_the_path_with_both_sides_at_zero_keeps_their_ratio() {
        let (mut d, id) = doc_line();
        d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().profile =
            Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 1.0, 3.0), (1.0, 1.0, 1.0)] });
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        // Drag the left handle (5 pt) in to the path: both sides pinch to zero.
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 195.0, Mods::default()));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 200.0, 200.0, Mods::default()));
        let (_, v) = preview(&acts);
        assert!(v["left"].as_f64().unwrap().abs() < 1e-6 && v["right"].as_f64().unwrap().abs() < 1e-6, "{v}");
        // Crossing to the right with both at zero keeps the 1:3 baseline rather than wiping it, so
        // dragging the right side out gives a right width of 1 and a left of a third, not zero.
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 200.0, 201.0, Mods::default()));
        let (_, v) = preview(&acts);
        assert!((v["right"].as_f64().unwrap() - 1.0).abs() < 1e-6 && (v["left"].as_f64().unwrap() - 1.0 / 3.0).abs() < 1e-6, "{v}");
        t.pointer(&c, &ev(PointerKind::Up, 200.0, 201.0, Mods::default()));
    }

    #[test]
    fn shift_click_selects_several_points_that_drag_together() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let (none, shift) = (Mods::default(), Mods { shift: true, ..Default::default() });
        // Click the point at t = 0.25, Shift-click the one at 0.75.
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 200.0, none));
        t.pointer(&c, &ev(PointerKind::Up, 150.0, 200.0, none));
        assert_eq!(t.pointer(&c, &ev(PointerKind::Down, 250.0, 200.0, shift)), vec![Action::Begin("Width Profile".into())]);
        t.pointer(&c, &ev(PointerKind::Up, 250.0, 200.0, shift));
        assert_eq!(t.selected_indices(&c, id), vec![1, 3]);
        // Dragging one of them 20 pt along moves both by a tenth of the path, as one command.
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 200.0, none));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 170.0, 203.0, none));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        let ts: Vec<f64> = v["points"].as_array().unwrap().iter().map(|q| (q[0].as_f64().unwrap() * 1000.0).round() / 1000.0).collect();
        assert_eq!(ts, vec![0.0, 0.35, 0.5, 0.85, 1.0]);
        t.pointer(&c, &ev(PointerKind::Up, 170.0, 203.0, none));
        let (_, sel) = t.selected.clone().unwrap();
        assert!((sel[0] - 0.35).abs() < 1e-3 && (sel[1] - 0.85).abs() < 1e-3, "the selection moves with them: {sel:?}");
        // Widening one handle widens every selected point by as much.
        let mut t = WidthTool { selected: Some((id, vec![0.25, 0.75])), ..Default::default() };
        t.pointer(&c, &ev(PointerKind::Down, 250.0, 195.0, none));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 250.0, 190.0, none));
        let pts = preview(&acts).1["points"].as_array().unwrap().clone();
        let w = |i: usize, side: usize| (pts[i][side].as_f64().unwrap() * 1e6).round() / 1e6;
        assert_eq!((w(1, 1), w(3, 2), w(2, 1)), (2.0, 2.0, 1.0));
        t.pointer(&c, &ev(PointerKind::Up, 250.0, 190.0, none));
        // Shift-clicking a selected point takes it out again (on release, and drags nothing).
        assert_eq!(t.pointer(&c, &ev(PointerKind::Down, 150.0, 200.0, shift)), vec![Action::Begin("Width Profile".into())]);
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 150.0, 200.0, shift)), vec![Action::Cancel]);
        assert_eq!(t.selected_indices(&c, id), vec![3]);
        // A plain click on one of several selected points selects just that one.
        let mut t = WidthTool { selected: Some((id, vec![0.25, 0.75])), ..Default::default() };
        t.pointer(&c, &ev(PointerKind::Down, 250.0, 200.0, none));
        t.pointer(&c, &ev(PointerKind::Up, 250.0, 200.0, none));
        assert_eq!(t.selected_indices(&c, id), vec![3]);
    }

    /// A Shift-press that barely moves is still a Shift-click: a pixel or two of jitter must not
    /// commit an elastic drag instead of toggling the point's selection.
    #[test]
    fn a_tiny_nudge_keeps_a_shift_click_selecting() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let shift = Mods { shift: true, ..Default::default() };
        let mut t = WidthTool { selected: Some((id, vec![0.25])), ..Default::default() };
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0, shift));
        t.pointer(&c, &ev(PointerKind::Drag, 201.0, 200.0, shift));
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 201.0, 200.0, shift)), vec![Action::Cancel]);
        assert_eq!(t.selected_indices(&c, id), vec![1, 2], "the point is added to the selection, not dragged");
        // Past the dead zone it drags the profile instead.
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0, shift));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 210.0, 200.0, shift));
        assert_eq!(preview(&acts).0, "stroke.widthProfile.set");
        assert_eq!(t.pointer(&c, &ev(PointerKind::Up, 210.0, 200.0, shift)), vec![Action::Commit]);
    }

    /// Alt changes one side of every selected point, not just of the one under the pointer, and it
    /// works from the press or partway through the drag.
    #[test]
    fn alt_drag_changes_one_side_of_every_selected_point() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let alt = Mods { alt: true, ..Default::default() };
        // The points at t = 0.25 and 0.5 selected; Alt-drag the left handle of the first one out.
        let mut t = WidthTool { selected: Some((id, vec![0.25, 0.5])), ..Default::default() };
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 195.0, alt));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 150.0, 190.0, alt));
        let pts = preview(&acts).1["points"].as_array().unwrap().clone();
        let w = |i: usize, side: usize| (pts[i][side].as_f64().unwrap() * 1e6).round() / 1e6;
        assert_eq!((w(1, 1), w(1, 2), w(2, 1), w(2, 2)), (2.0, 1.0, 2.0, 1.0), "both selected points widen on the left only");
        // Further out, still one side.
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 150.0, 185.0, alt));
        let pts = preview(&acts).1["points"].as_array().unwrap().clone();
        let w = |i: usize, side: usize| (pts[i][side].as_f64().unwrap() * 1e6).round() / 1e6;
        assert_eq!((w(1, 1), w(1, 2), w(2, 1), w(2, 2)), (3.0, 1.0, 3.0, 1.0));
        t.pointer(&c, &ev(PointerKind::Up, 150.0, 185.0, alt));
        // Pressed partway through a plain drag, Alt takes the group's other side back where it
        // started and keeps it there.
        let mut t = WidthTool { selected: Some((id, vec![0.25, 0.5])), ..Default::default() };
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 195.0, Mods::default()));
        t.pointer(&c, &ev(PointerKind::Drag, 150.0, 190.0, Mods::default()));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 150.0, 185.0, alt));
        let pts = preview(&acts).1["points"].as_array().unwrap().clone();
        let w = |i: usize, side: usize| (pts[i][side].as_f64().unwrap() * 1e6).round() / 1e6;
        assert_eq!((w(1, 1), w(1, 2)), (3.0, 2.0), "the side under the pointer follows it, the other holds where Alt went down");
        assert_eq!((w(2, 1), w(2, 2)), (3.0, 2.0));
        t.pointer(&c, &ev(PointerKind::Up, 150.0, 185.0, alt));
    }

    #[test]
    fn shift_drag_of_a_point_respaces_every_width_point() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let shift = Mods { shift: true, ..Default::default() };
        // Shift-drag the point at t = 0.5 a tenth of the way along: everything before it spreads to
        // fill the first half and everything after squeezes into the second; the ends stay put.
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0, shift));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 210.0, 200.0, shift));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        let ts: Vec<f64> = v["points"].as_array().unwrap().iter().map(|q| (q[0].as_f64().unwrap() * 1000.0).round() / 1000.0).collect();
        assert_eq!(ts, vec![0.0, 0.275, 0.55, 0.775, 1.0]);
        t.pointer(&c, &ev(PointerKind::Up, 210.0, 200.0, shift));
        assert!(t.selected.as_ref().is_some_and(|(_, ts)| (ts[0] - 0.55).abs() < 1e-3), "the dragged point stays selected where it landed");
    }

    /// A grouped Move that turns elastic partway through re-spaces the profile, so the group's
    /// uniform shift no longer lands the selection on the points it committed: only the dragged
    /// point stays selected.
    #[test]
    fn an_elastic_grouped_move_selects_the_dragged_point() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool { selected: Some((id, vec![0.25, 0.5])), ..Default::default() };
        let (none, shift) = (Mods::default(), Mods { shift: true, ..Default::default() });
        // Plain-drag the pair (a grouped Move), then hold Shift: the drag re-spaces the profile.
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0, none));
        t.pointer(&c, &ev(PointerKind::Drag, 205.0, 200.0, none));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 210.0, 200.0, shift));
        assert_eq!(preview(&acts).0, "stroke.widthProfile.set");
        t.pointer(&c, &ev(PointerKind::Up, 210.0, 200.0, shift));
        let (_, sel) = t.selected.clone().unwrap();
        assert_eq!(sel.len(), 1, "only the dragged point stays selected: {sel:?}");
        assert!((sel[0] - 0.55).abs() < 1e-3, "and it is where the drag landed: {sel:?}");
    }

    #[test]
    fn a_shift_move_stops_at_the_profile_ends() {
        let (d, id) = doc_corner();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let shift = Mods { shift: true, ..Default::default() };
        // Shift-drag the point at t = 0.25 past the end of the path: it stops at the profile's last
        // point (t = 0.75) instead of overtaking it, and the spot is re-evaluated at the clamp.
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 200.0, shift));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 200.0, 350.0, shift));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        let ts: Vec<f64> = v["points"].as_array().unwrap().iter().map(|q| (q[0].as_f64().unwrap() * 1000.0).round() / 1000.0).collect();
        assert_eq!(ts, vec![0.1, 0.75, 0.75, 0.75]);
        let d = t.drag.as_ref().unwrap();
        assert!((d.t - 0.75).abs() < 1e-6, "the moved point is clamped to the profile's last t: {}", d.t);
        assert!((d.spot.p - Point::new(200.0, 250.0)).hypot() < 1e-3, "the spot is re-evaluated at the clamped t: {:?}", d.spot.p);
        t.pointer(&c, &ev(PointerKind::Up, 200.0, 350.0, shift));
    }

    /// Shift-dragging the profile's first or last point must not slide it off the end: a point
    /// stays where that end was and the re-spaced one carries the drag.
    #[test]
    fn shift_drag_of_an_end_point_keeps_a_point_at_that_end() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let shift = Mods { shift: true, ..Default::default() };
        let ts = |v: &serde_json::Value| -> Vec<f64> {
            v["points"].as_array().unwrap().iter().map(|q| (q[0].as_f64().unwrap() * 10000.0).round() / 10000.0).collect()
        };
        // The first point (t = 0) in to t = 0.05.
        t.pointer(&c, &ev(PointerKind::Down, 100.0, 200.0, shift));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 110.0, 200.0, shift));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        assert_eq!(ts(v), vec![0.0, 0.05, 0.2875, 0.525, 0.7625, 1.0]);
        t.pointer(&c, &ev(PointerKind::Up, 110.0, 200.0, shift));
        // The last point (t = 1) in to t = 0.95, its own end kept.
        t.pointer(&c, &ev(PointerKind::Down, 300.0, 200.0, shift));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 290.0, 200.0, shift));
        let (_, v) = preview(&acts);
        assert_eq!(ts(v), vec![0.0, 0.2375, 0.475, 0.7125, 0.95, 1.0]);
        t.pointer(&c, &ev(PointerKind::Up, 290.0, 200.0, shift));
    }

    #[test]
    fn shift_pressed_mid_drag_turns_it_elastic_and_back() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let shift = Mods { shift: true, ..Default::default() };
        let none = Mods::default();
        // Begin a plain drag of the point at t = 0.5: only that point moves.
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0, none));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 210.0, 200.0, none));
        let (cmd, v) = preview(&acts);
        assert_eq!(cmd, "stroke.widthPoint.set");
        assert!((v["t"].as_f64().unwrap() - 0.55).abs() < 1e-3, "{v}");
        // Holding Shift partway through re-spaces the whole profile instead.
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 210.0, 200.0, shift));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        let ts: Vec<f64> = v["points"].as_array().unwrap().iter().map(|q| (q[0].as_f64().unwrap() * 1000.0).round() / 1000.0).collect();
        assert_eq!(ts, vec![0.0, 0.275, 0.55, 0.775, 1.0]);
        // Letting Shift go turns it back into a plain drag of the one point.
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 210.0, 200.0, none));
        let (cmd, v) = preview(&acts);
        assert_eq!(cmd, "stroke.widthPoint.set");
        assert!((v["t"].as_f64().unwrap() - 0.55).abs() < 1e-3, "{v}");
        t.pointer(&c, &ev(PointerKind::Up, 210.0, 200.0, none));
    }

    /// The app puts a drag's preview onto the document as it goes, so a profile read partway
    /// through a drag already carries that drag's own move. Turning Shift on then must re-space
    /// from the profile the drag began with — otherwise the dragged point is spaced as one of the
    /// others and every point (it included) leaves the cursor.
    #[test]
    fn shift_mid_drag_spaces_from_the_profile_the_drag_began_with() {
        let (mut d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        let none = Mods::default();
        let mut t = WidthTool::default();
        // Plain-drag the point at t = 0.5 to t = 0.55 and let the document take the preview, as the
        // app would.
        {
            let c = cx(&d, &s, &p);
            t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0, none));
            t.pointer(&c, &ev(PointerKind::Drag, 210.0, 200.0, none));
        }
        d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().profile =
            Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.25, 1.0, 1.0), (0.55, 1.0, 1.0), (0.75, 1.0, 1.0), (1.0, 1.0, 1.0)] });
        // Holding Shift now re-spaces from t = 0.5 → 0.55: the dragged point stays under the cursor
        // and the ends stay put, exactly as a drag begun with Shift down.
        let c = cx(&d, &s, &p);
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 210.0, 200.0, shift));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        let ts: Vec<f64> = v["points"].as_array().unwrap().iter().map(|q| (q[0].as_f64().unwrap() * 1000.0).round() / 1000.0).collect();
        assert_eq!(ts, vec![0.0, 0.275, 0.55, 0.775, 1.0]);
        t.pointer(&c, &ev(PointerKind::Up, 210.0, 200.0, shift));
    }

    /// The width twin of the test above: a profile read mid-drag already carries the plain drag's
    /// own width change, so an elastic width change must scale the run from the drag's start.
    #[test]
    fn shift_mid_drag_scales_the_smooth_run_from_the_profile_the_drag_began_with() {
        let (mut d, id) = doc_corner();
        let (s, p) = (Selection::default(), paint());
        let shift = Mods { shift: true, ..Default::default() };
        let none = Mods::default();
        let mut t = WidthTool::default();
        // Plain-drag the left handle of the point at t = 0.25 (width factor 2) to a factor of 4.
        {
            let c = cx(&d, &s, &p);
            t.pointer(&c, &ev(PointerKind::Down, 150.0, 190.0, none));
            let acts = t.pointer(&c, &ev(PointerKind::Drag, 150.0, 180.0, none));
            let (_, v) = preview(&acts);
            assert!((v["left"].as_f64().unwrap() - 20.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 20.0).abs() < 1e-6, "{v}");
        }
        d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().profile =
            Some(WidthProfile { points: vec![(0.1, 1.0, 1.0), (0.25, 4.0, 4.0), (0.5, 1.0, 1.0), (0.75, 1.0, 1.0)] });
        // Holding Shift now scales the run (t = 0.1, 0.25) by the pointer's factor (2) from the
        // drag's start, rather than compounding it on the already-widened profile.
        let c = cx(&d, &s, &p);
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 150.0, 180.0, shift));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        let pts: Vec<[f64; 3]> =
            v["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap(), q[2].as_f64().unwrap()]).collect();
        assert!((pts[0][1] - 2.0).abs() < 1e-6 && (pts[1][1] - 4.0).abs() < 1e-6, "the run doubles from the start: {pts:?}");
        t.pointer(&c, &ev(PointerKind::Up, 150.0, 180.0, shift));
    }

    #[test]
    fn shift_pressed_mid_drag_scales_the_smooth_run() {
        let (d, id) = doc_corner();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let shift = Mods { shift: true, ..Default::default() };
        let none = Mods::default();
        // Begin a plain drag of the left handle of the point at t = 0.25 (width 2), which scales
        // both sides evenly.
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 190.0, none));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 150.0, 180.0, none));
        let (cmd, v) = preview(&acts);
        assert_eq!(cmd, "stroke.widthPoint.set");
        assert!((v["left"].as_f64().unwrap() - 20.0).abs() < 1e-6 && (v["right"].as_f64().unwrap() - 20.0).abs() < 1e-6, "{v}");
        // Holding Shift partway through scales the whole smooth run (t = 0.1 and 0.25) instead.
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 150.0, 180.0, shift));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        let pts: Vec<[f64; 3]> =
            v["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap(), q[2].as_f64().unwrap()]).collect();
        assert!((pts[0][1] - 2.0).abs() < 1e-6 && (pts[1][1] - 4.0).abs() < 1e-6, "the run doubles: {pts:?}");
        assert!((pts[2][1] - 1.0).abs() < 1e-6 && (pts[3][1] - 1.0).abs() < 1e-6, "past the corner holds: {pts:?}");
        t.pointer(&c, &ev(PointerKind::Up, 150.0, 180.0, shift));
    }

    #[test]
    fn shift_drag_of_a_handle_scales_only_its_smooth_run() {
        let (d, id) = doc_corner();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let shift = Mods { shift: true, ..Default::default() };
        // Shift-drag the left handle of the point at t = 0.25 (x = 150) out to twice its width: the
        // other point on the first leg (t = 0.1) scales with it, the sharp corner at t = 0.5 and the
        // point past it (t = 0.75) keep their widths.
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 190.0, shift));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 150.0, 180.0, shift));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        let pts: Vec<[f64; 3]> =
            v["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap(), q[2].as_f64().unwrap()]).collect();
        assert!((pts[0][0] - 0.1).abs() < 1e-6 && (pts[0][1] - 2.0).abs() < 1e-6, "{pts:?}");
        assert!((pts[1][0] - 0.25).abs() < 1e-6 && (pts[1][1] - 4.0).abs() < 1e-6 && (pts[1][2] - 4.0).abs() < 1e-6, "{pts:?}");
        assert!((pts[2][1] - 1.0).abs() < 1e-6 && (pts[3][1] - 1.0).abs() < 1e-6, "{pts:?}");
        t.pointer(&c, &ev(PointerKind::Up, 150.0, 180.0, shift));
    }

    /// An open path's ends are where the stroke stops, not sharp corners, so a Shift-drag of a
    /// handle scales the width points at t = 0 and t = 1 too rather than tapering the stroke.
    #[test]
    fn shift_drag_of_a_handle_scales_an_open_paths_end_points() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let shift = Mods { shift: true, ..Default::default() };
        // Shift-drag the left handle of the middle point (x = 200, width 1) out to twice its width:
        // the whole path is one smooth run, so every point scales, ends included.
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 195.0, shift));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 200.0, 190.0, shift));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        let pts: Vec<[f64; 3]> =
            v["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap(), q[2].as_f64().unwrap()]).collect();
        assert!((pts[0][0] - 0.0).abs() < 1e-6 && (pts[0][1] - 2.0).abs() < 1e-6, "the t = 0 end scales: {pts:?}");
        assert!((pts[2][1] - 2.0).abs() < 1e-6, "the dragged point scales: {pts:?}");
        assert!((pts[4][0] - 1.0).abs() < 1e-6 && (pts[4][1] - 2.0).abs() < 1e-6, "the t = 1 end scales: {pts:?}");
        t.pointer(&c, &ev(PointerKind::Up, 200.0, 190.0, shift));
    }

    /// The stored width `t` and the corner's arc-length fraction can differ by rounding. A corner
    /// within `SAME_T` of the point must still bound both sides of the run, or the point would
    /// pick up everything on one side of it.
    #[test]
    fn a_corner_rounding_difference_still_bounds_both_sides() {
        let (d, id) = doc_corner();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        for t in [0.5 - SAME_T / 2.0, 0.5 + SAME_T / 2.0] {
            let (lo, hi) = WidthTool::smooth_run(&c, id, 0, t);
            assert!((lo - 0.5).abs() <= SAME_T, "{t}: lo {lo}");
            assert!((hi - 0.5).abs() <= SAME_T, "{t}: hi {hi}");
        }
    }

    /// A width point can sit exactly on a sharp corner. Its smooth run is the point itself, not
    /// "everything on the far side of the corner", so only that point scales.
    #[test]
    fn shift_drag_of_a_handle_on_a_corner_scales_only_that_point() {
        let (d, id) = doc_corner();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let shift = Mods { shift: true, ..Default::default() };
        // The corner point (t = 0.5, x = 200, width 1) out to twice its width.
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 195.0, shift));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 200.0, 190.0, shift));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        let pts: Vec<[f64; 3]> =
            v["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap(), q[2].as_f64().unwrap()]).collect();
        assert!((pts[2][0] - 0.5).abs() < 1e-6 && (pts[2][1] - 2.0).abs() < 1e-6, "the corner point scales: {pts:?}");
        assert!((pts[0][1] - 1.0).abs() < 1e-6, "the point before the corner holds: {pts:?}");
        assert!((pts[1][1] - 2.0).abs() < 1e-6, "the other point on the first leg holds: {pts:?}");
        assert!((pts[3][1] - 1.0).abs() < 1e-6, "the point past the corner holds: {pts:?}");
        t.pointer(&c, &ev(PointerKind::Up, 200.0, 190.0, shift));
    }

    /// Shift+Alt scales the handle's own side of the smooth run and leaves the other side of those
    /// points where it was.
    #[test]
    fn alt_shift_drag_scales_one_side_of_the_run() {
        let (d, id) = doc_corner();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let both = Mods { shift: true, alt: true, ..Default::default() };
        // The left handle of the point at t = 0.25, out to twice its width.
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 190.0, both));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 150.0, 180.0, both));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        let pts: Vec<[f64; 3]> =
            v["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap(), q[2].as_f64().unwrap()]).collect();
        assert!((pts[0][1] - 2.0).abs() < 1e-6 && (pts[0][2] - 1.0).abs() < 1e-6, "the run's left side scales, its right holds: {pts:?}");
        assert!((pts[1][1] - 4.0).abs() < 1e-6 && (pts[1][2] - 2.0).abs() < 1e-6, "the dragged point: {pts:?}");
        assert!((pts[2][1] - 1.0).abs() < 1e-6 && (pts[3][1] - 1.0).abs() < 1e-6, "past the corner holds: {pts:?}");
        t.pointer(&c, &ev(PointerKind::Up, 150.0, 180.0, both));
    }

    #[test]
    fn a_closed_paths_smooth_run_wraps_across_the_seam() {
        let (mut d, id) = doc_line();
        let n = d.node_mut(id).unwrap();
        // A closed square with corners at t = 0.25 and 0.75; the seam (t = 0) sits in the smooth run
        // that wraps from the last corner round to the first.
        n.appearance.stroke_mut().unwrap().profile =
            Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.1, 1.0, 1.0), (0.5, 1.0, 1.0), (0.9, 1.0, 1.0)] });
        let NodeKind::Path { path, .. } = &mut n.kind else { panic!("a path") };
        let mut sp =
            SubPath::polyline(&[Point::new(100.0, 100.0), Point::new(200.0, 100.0), Point::new(200.0, 200.0), Point::new(100.0, 200.0)], true);
        sp.anchors[0].kind = AnchorKind::Smooth;
        sp.anchors[2].kind = AnchorKind::Smooth;
        path.subpaths = vec![sp];
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let shift = Mods { shift: true, ..Default::default() };
        // Shift-drag the left handle of the seam point (t = 0) to twice its width: the point just
        // before the seam (t = 0.9) is on the same run and scales too, while the other smooth run
        // (t = 0.5) keeps its width.
        t.pointer(&c, &ev(PointerKind::Down, 100.0, 95.0, shift));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 100.0, 90.0, shift));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["ids"]), ("stroke.widthProfile.set", &json!([id.0])));
        let pts: Vec<[f64; 3]> =
            v["points"].as_array().unwrap().iter().map(|q| [q[0].as_f64().unwrap(), q[1].as_f64().unwrap(), q[2].as_f64().unwrap()]).collect();
        assert!((pts[0][1] - 2.0).abs() < 1e-6 && (pts[1][1] - 2.0).abs() < 1e-6, "{pts:?}");
        assert!((pts[2][1] - 1.0).abs() < 1e-6, "the other smooth run keeps its width: {pts:?}");
        assert!((pts[3][1] - 2.0).abs() < 1e-6, "the point before the seam scales across it: {pts:?}");
        t.pointer(&c, &ev(PointerKind::Up, 100.0, 90.0, shift));
    }

    #[test]
    fn alt_drag_copies_and_dropping_onto_a_point_snaps_to_it() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let alt = Mods { alt: true, ..Default::default() };
        assert_eq!(t.pointer(&c, &ev(PointerKind::Down, 150.0, 200.0, alt)), vec![Action::Begin("Copy Width Point".into())]);
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 180.0, 200.0, alt));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["id"], &v["index"]), ("stroke.widthPoint.copy", &json!(id.0), &json!(1)));
        assert!((v["t"].as_f64().unwrap() - 0.4).abs() < 1e-3);
        t.pointer(&c, &ev(PointerKind::Up, 180.0, 200.0, alt));
        // Sliding the point at 0.25 to within a few pixels of the one at 0.5 lands exactly on it.
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 200.0, Mods::default()));
        let acts = t.pointer(&c, &ev(PointerKind::Drag, 197.0, 200.0, Mods::default()));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, v["t"].as_f64(), v["index"].as_u64()), ("stroke.widthPoint.set", Some(0.5), Some(1)));
    }

    /// A compound path of two 200 pt lines (y = 200 and 300) with a 10 pt stroke on the compound
    /// (its members carry none), a width point at t = 0.5 → (doc, compound, second member).
    fn doc_compound() -> (Document, NodeId, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let (c, a, b) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let line = |id, y| {
            let path = PathData::single(SubPath::polyline(&[Point::new(100.0, y), Point::new(300.0, y)], false));
            std::sync::Arc::new(Node::path(id, path, Appearance::basic(vectorcraft_color::Paint::None, vectorcraft_color::Paint::None, 0.0)))
        };
        let mut comp = Node::new(c, NodeKind::Compound { children: vec![line(a, 200.0), line(b, 300.0)], rule: Default::default() });
        comp.appearance = Appearance::default_art().into();
        let st = comp.appearance.stroke_mut().unwrap();
        st.width = 10.0;
        st.profile = Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 1.0, 1.0), (1.0, 1.0, 1.0)] });
        d.insert(Some(l), 0, comp).unwrap();
        (d, c, b)
    }

    #[test]
    fn compound_path_strokes_are_edited_through_the_compound() {
        let (d, c, b) = doc_compound();
        assert_eq!(stroke_owner(&d, b), c);
        let (s, p) = (Selection { objects: vec![c], ..Default::default() }, paint());
        let cx = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        // Drag out from the second member: the compound's stroke gets the width point.
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 150.0, 301.0, Mods::default())), vec![Action::Begin("Width Point".into())]);
        let acts = t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 320.0, Mods::default()));
        let (cmd, v) = preview(&acts);
        assert_eq!((cmd, &v["id"]), ("stroke.widthPoint.set", &json!(c.0)));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 320.0, Mods::default()));
        // The width point in the middle of the first member selects (and deletes) the compound's.
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 200.0, Mods::default()));
        t.pointer(&cx, &ev(PointerKind::Up, 200.0, 200.0, Mods::default()));
        assert!(t.claims_key(&cx, ToolKey::Delete));
        assert_eq!(
            t.key(&cx, ToolKey::Delete, Mods::default()),
            vec![Action::Exec("stroke.widthPoint.remove".into(), json!({"id": c.0, "indices": [1]}))]
        );
    }

    #[test]
    fn escape_deselects_the_width_points() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let (none, shift) = (Mods::default(), Mods { shift: true, ..Default::default() });
        // With nothing selected the tool leaves Escape to the app.
        assert!(!t.claims_key(&c, ToolKey::Escape));
        // Select the points at t = 0.25 and 0.75: Escape is the tool's and deselects both, sending
        // nothing to the document.
        t.pointer(&c, &ev(PointerKind::Down, 150.0, 200.0, none));
        t.pointer(&c, &ev(PointerKind::Up, 150.0, 200.0, none));
        t.pointer(&c, &ev(PointerKind::Down, 250.0, 200.0, shift));
        t.pointer(&c, &ev(PointerKind::Up, 250.0, 200.0, shift));
        assert_eq!(t.selected_indices(&c, id), vec![1, 3]);
        t.pointer(&c, &ev(PointerKind::Move, 200.0, 202.0, none));
        let thick = |t: &WidthTool| t.overlays(&c).iter().filter(|o| matches!(o, Overlay::Path { width, .. } if *width > 2.0)).count();
        assert_eq!(thick(&t), 2, "the two selected diamonds are drawn selected");
        assert!(t.claims_key(&c, ToolKey::Escape));
        assert!(t.key(&c, ToolKey::Escape, none).is_empty());
        assert!(t.selected_indices(&c, id).is_empty());
        assert_eq!(thick(&t), 0, "no diamond is drawn selected");
        // Delete then removes nothing, and Escape is the app's again.
        assert!(!t.claims_key(&c, ToolKey::Delete) && !t.claims_key(&c, ToolKey::Escape));
        assert!(t.key(&c, ToolKey::Delete, none).is_empty());
        // During a drag the dragged point stays selected.
        t.pointer(&c, &ev(PointerKind::Down, 200.0, 200.0, none));
        t.pointer(&c, &ev(PointerKind::Drag, 210.0, 200.0, none));
        assert!(!t.claims_key(&c, ToolKey::Escape));
        t.key(&c, ToolKey::Escape, none);
        assert!(t.selected.is_some());
    }

    #[test]
    fn width_points_show_on_every_subpath() {
        let (d, c, _) = doc_compound();
        let (s, p) = (Selection { objects: vec![c], ..Default::default() }, paint());
        let cx = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        let handles = |t: &WidthTool| t.overlays(&cx).iter().filter(|o| matches!(o, Overlay::Handle { .. })).count();
        // Hovering one member shows the 3 width points on both (two handle ends each).
        t.pointer(&cx, &ev(PointerKind::Move, 150.0, 302.0, Mods::default()));
        assert_eq!(handles(&t), 3 * 2 * 2);
        // A selected point shows them on every subpath when the pointer is elsewhere.
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 300.0, Mods::default()));
        t.pointer(&cx, &ev(PointerKind::Up, 200.0, 300.0, Mods::default()));
        t.pointer(&cx, &ev(PointerKind::Move, 450.0, 450.0, Mods::default()));
        assert_eq!(handles(&t), 3 * 2 * 2);
        // One path of two subpaths: both.
        let (mut d2, id) = doc_line();
        let n = d2.node_mut(id).unwrap();
        if let NodeKind::Path { path, .. } = &mut n.kind {
            path.subpaths.push(SubPath::polyline(&[Point::new(100.0, 260.0), Point::new(300.0, 260.0)], false));
        }
        n.appearance.stroke_mut().unwrap().profile = Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 2.0, 2.0), (1.0, 1.0, 1.0)] });
        let s2 = Selection { objects: vec![id], ..Default::default() };
        let cx2 = crate::testutil::cx(&d2, &s2, &p);
        let mut t = WidthTool::default();
        t.pointer(&cx2, &ev(PointerKind::Down, 200.0, 260.0, Mods::default()));
        t.pointer(&cx2, &ev(PointerKind::Up, 200.0, 260.0, Mods::default()));
        t.pointer(&cx2, &ev(PointerKind::Move, 450.0, 450.0, Mods::default()));
        let ends: Vec<Point> = t.overlays(&cx2).iter().filter_map(|o| if let Overlay::Handle { p, .. } = o { Some(*p) } else { None }).collect();
        assert!(ends.iter().any(|q| q.y < 230.0) && ends.iter().any(|q| q.y > 230.0), "both subpaths: {ends:?}");
    }

    #[test]
    fn the_cursor_says_what_a_drag_does() {
        let (d, _) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        let mut t = WidthTool::default();
        assert_eq!(t.cursor(&c, Point::new(400.0, 400.0), Mods::default()), Cursor::Width);
        t.pointer(&c, &ev(PointerKind::Move, 170.0, 201.0, Mods::default()));
        assert_eq!(t.cursor(&c, Point::new(170.0, 201.0), Mods::default()), Cursor::WidthAdd, "on the stroke: adds a point");
        t.pointer(&c, &ev(PointerKind::Move, 150.0, 200.0, Mods::default()));
        assert_eq!(t.cursor(&c, Point::new(150.0, 200.0), Mods::default()), Cursor::WidthPoint, "on a width point");
        assert_eq!(t.cursor(&c, Point::new(150.0, 195.0), Mods::default()), Cursor::WidthPoint, "on a handle end");
    }

    #[test]
    fn double_click_on_a_width_point_opens_width_point_edit() {
        let (d, id) = doc_three_points();
        let (s, p) = (Selection::default(), paint());
        let c = cx(&d, &s, &p);
        for shift in [false, true] {
            let mut t = WidthTool::default();
            let mods = Mods { shift, ..Default::default() };
            let acts = t.pointer(&c, &ev(PointerKind::DoubleClick, 200.0, 195.0, mods));
            let [Action::Dialog(kind, p)] = acts.as_slice() else { panic!("expected Width Point Edit: {acts:?}") };
            assert_eq!(kind, "widthPoint");
            assert_eq!((&p["id"], &p["index"]), (&json!(id.0), &json!(2)));
            assert_eq!(p.get("adjustAdjoining").and_then(serde_json::Value::as_bool).unwrap_or(false), shift);
            assert!(t.pointer(&c, &ev(PointerKind::DoubleClick, 120.0, 250.0, mods)).is_empty(), "not on a stroked path");
        }
    }
}
