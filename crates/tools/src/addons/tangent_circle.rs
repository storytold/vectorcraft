//! Tangent Circle tool (Path Precision Tools): click a path segment, then another, selected or
//! not, to draw the circle that is tangent to the first segment where it was clicked and touches the
//! second ([`circles_through`]). With Shift held over the second segment the circle has the tool's
//! `radius` instead, anywhere along the first segment ([`circles_of_radius`]). A drag from the
//! second segment sets the radius to the drag's length, the circle again anywhere along the first
//! segment: Shift steps it by a round number of points about 4 screen pixels long, and Cmd/Ctrl
//! pressed during the drag moves it by a tenth of the pointer's movement (held at the press,
//! Cmd/Ctrl lends the selection tool as it does for every tool). The segments on either side of the
//! second one count with it (the first one excepted), and of the circles found the one touching
//! them nearest the pointer (in a drag, nearest the press) is drawn. While the pointer is over the
//! second segment the circle shows with its center and radii; where there is none, or none that its
//! three points can draw, a note says so. The circle is a new ellipse with the current paint, drawn
//! with `shape.circleThroughPoints` in one undo step, so Align New Objects to Pixel Grid leaves it
//! in place; the first segment stays picked until the circle is drawn, also when drawing fails or the
//! press on the second segment is dropped. Escape, an edit or Undo outside the tool, another
//! document, or a change to the first segment that leaves no undo step (a live preview, or one that
//! was canceled) forgets the first segment. The circles are looked for again whenever either
//! segment's shape differs from the one they were found on. With the `addPoints` option on (Alt at
//! the release turns it the other way, and a dot beside the pointer shows when it is in effect), the
//! segments the circle touches get an anchor point at each touch, which splits them without changing
//! their shape, in the same undo step as the circle; a touch at an existing anchor adds none.
//!
//! The tool's options (its options dialog, `tool.setOption`; kept with the preferences): `radius`;
//! `addPoints`; `center` and `radii` show or hide the circle's center cross and the lines from its center to the
//! two points it touches; `color` is the color of the circle, the cross, those lines and the note
//! (one of [`COLORS`]); `snapTolerance` is how near (screen pixels) the pointer picks a segment
//! with Smart Guides on, where Smart Guides' Snapping Tolerance counts instead when it is larger,
//! or with Smart Guides off while `snapWithoutGuides` is on. Without snapping a segment is picked
//! within the selection tolerance, as a click on a path picks it, and that is also the least reach
//! snapping has. Of the segments within reach the nearest is picked ([`pick_at`]). By default
//! `addPoints` is off, the cross and the lines show, the color is red, the tolerance is 3 px and
//! `snapWithoutGuides` is on.

use kurbo::{CubicBez, ParamCurveNearest, Shape};
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_geom::{BezPath, ParamCurve, Point};
use vectorcraft_pathops::addons::pathtools::{SegmentCircle, circles_of_radius, circles_through};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

/// The tool's id.
pub const ID: &str = "tangentCircle";
/// The radius Shift asks for to start with (pt).
pub const RADIUS: f64 = 36.0;
/// The radius the `radius` option takes (pt).
pub const RADIUS_RANGE: (f64, f64) = (0.01, 1e6);
/// How far (screen pixels) the pointer moves from the second click before a drag sets the radius.
const DRAG_PX: f64 = 3.0;
/// About how long (screen pixels) one step of a Shift drag is.
const STEP_PX: f64 = 4.0;
/// How much of the pointer's movement a Cmd/Ctrl drag gives the radius.
const SLOW: f64 = 0.1;
/// The note shown over a second segment that gives no circle.
pub const NO_CIRCLE: &str = "No tangent circle here";
const BLUE: [u8; 3] = [0x4f, 0x9d, 0xff];
const RED: [u8; 3] = [0xe0, 0x3c, 0x3c];
const GREY: [u8; 3] = [0x80, 0x80, 0x80];
/// The label of the `addPoints` option in the tool's options.
pub const ADD_POINTS: &str = "Add Points to Existing Paths";
/// How far (screen pixels) right of and below the pointer the dot shows while `addPoints` is in
/// effect, and its size.
const DOT_PX: f64 = 10.0;
const DOT_SIZE: f32 = 4.0;
/// The colors the `color` option takes, by option value, with the color the circle, its center
/// cross, its radii and the note are drawn in. Red is the first.
pub const COLORS: &[(&str, [u8; 3])] = &[
    ("red", RED),
    ("blue", crate::draw2::FEEDBACK),
    ("magenta", [0xd6, 0x2c, 0xd6]),
    ("green", [0x1f, 0xb4, 0x4b]),
    ("black", [0x00, 0x00, 0x00]),
    ("gray", GREY),
];
/// The snapping tolerance the tool starts with (screen pixels).
pub const SNAP_PX: f64 = 3.0;
/// The snapping tolerance the `snapTolerance` option takes (screen pixels).
pub const SNAP_RANGE: (f64, f64) = (0.0, 25.0);
/// The option keys that are true or false.
const SWITCHES: &[&str] = &["addPoints", "center", "radii", "snapWithoutGuides"];

/// A segment under the pointer: where on it, and the segment itself.
#[derive(Clone, Debug, PartialEq)]
struct Pick {
    id: NodeId,
    subpath: usize,
    segment: usize,
    t: f64,
    p: Point,
    curve: CubicBez,
}

/// A press on the second segment, kept while the button is down.
#[derive(Clone, Debug, PartialEq)]
struct Press {
    /// The segment pressed on, and the segments a circle may touch.
    pick: Pick,
    second: Vec<CubicBez>,
    /// Where the press was.
    at: Point,
    /// Whether Shift was held when the click's circle was last looked for.
    shift: bool,
    /// The tool's `radius` when the button went down: Shift uses it for the whole press, whatever
    /// `tool.setOption` writes meanwhile.
    shift_radius: f64,
    /// How far the pointer was from the press last time, and the radius the drag set (none until
    /// the pointer moves far enough).
    reach: f64,
    radius: Option<f64>,
}

pub struct TangentCircleTool {
    radius: f64,
    /// Whether drawing a circle adds anchor points where it touches the segments.
    add_points: bool,
    /// The keys held at the last pointer event, and whether the pointer is over the canvas.
    mods: Mods,
    over: bool,
    /// Whether the preview shows the center cross and the radii.
    center: bool,
    radii: bool,
    /// An entry of [`COLORS`].
    color: usize,
    /// The snapping tolerance (screen pixels), and whether the tool snaps with Smart Guides off.
    snap_px: f64,
    snap_without_guides: bool,
    first: Option<Pick>,
    hover: Option<Pick>,
    press: Option<Press>,
    /// Whether the press in progress picked the first segment.
    picking: bool,
    /// The circle the second segment gives, and whether one was looked for.
    found: Option<SegmentCircle>,
    looked: bool,
    pointer: Point,
    /// The circles the last second segment gave (its path, subpath and segment, the radius asked
    /// for, and the shapes of the first segment and of the segments a circle may touch), so that
    /// moving along it only picks among them.
    cache: Option<(CacheKey, Vec<SegmentCircle>)>,
}

type CacheKey = (NodeId, usize, usize, Option<u64>, CubicBez, Vec<CubicBez>);

impl Default for TangentCircleTool {
    fn default() -> Self {
        Self {
            radius: RADIUS,
            add_points: false,
            mods: Mods::default(),
            over: false,
            center: true,
            radii: true,
            color: 0,
            snap_px: SNAP_PX,
            snap_without_guides: true,
            first: None,
            hover: None,
            press: None,
            picking: false,
            found: None,
            looked: false,
            pointer: Point::ZERO,
            cache: None,
        }
    }
}

/// The nearest path segment within `reach` of `p`, of any editable path; of segments at the same
/// distance, one of a selected path, then the one on top.
fn pick_at(cx: &ToolContext, p: Point, reach: f64) -> Option<Pick> {
    let mut best: Option<((NodeId, usize, usize, f64), f64)> = None;
    for id in crate::draw2::candidate_paths(cx) {
        let Some(pd) = cx.doc.node(id).and_then(|n| n.path_data()) else { continue };
        // A path whose control points' box, with its edges, lies out of reach holds no point within
        // reach.
        let near = |b: vectorcraft_geom::Rect| p.x >= b.x0 - reach && p.x <= b.x1 + reach && p.y >= b.y0 - reach && p.y <= b.y1 + reach;
        if pd.control_bounds().is_none_or(|b| !near(b)) {
            continue;
        }
        if let Some((subpath, segment, t, _, d)) = pd.nearest(p)
            && d <= reach
            && best.is_none_or(|(_, b)| d < b)
        {
            best = Some(((id, subpath, segment, t), d));
        }
    }
    let ((id, subpath, segment, t), _) = best?;
    let sp = cx.doc.node(id)?.path_data()?.subpaths.get(subpath)?;
    if segment >= sp.segment_count() {
        return None;
    }
    let curve = sp.segment(segment);
    Some(Pick { id, subpath, segment, t, p: curve.eval(t), curve })
}

/// The document's segment at `pick` as it is now: none when its path, subpath or segment is gone.
fn segment_now(cx: &ToolContext, pick: &Pick) -> Option<CubicBez> {
    let sp = cx.doc.node(pick.id)?.path_data()?.subpaths.get(pick.subpath)?;
    (pick.segment < sp.segment_count()).then(|| sp.segment(pick.segment))
}

/// Whether the document's segment at `pick` still has the shape it had when it was picked.
fn unchanged(cx: &ToolContext, pick: &Pick) -> bool {
    segment_now(cx, pick) == Some(pick.curve)
}

/// The segments a circle may touch when `second` is picked: it and the segments on either side of
/// it, without `first`. None when `second` is the first segment again.
fn second_set(cx: &ToolContext, first: &Pick, second: &Pick) -> Vec<CubicBez> {
    second_segments(cx, first, second).into_iter().map(|(_, c)| c).collect()
}

/// [`second_set`] with each segment's index on the second pick's subpath.
fn second_segments(cx: &ToolContext, first: &Pick, second: &Pick) -> Vec<(usize, CubicBez)> {
    if (first.id, first.subpath, first.segment) == (second.id, second.subpath, second.segment) {
        return vec![];
    }
    let Some(sp) = cx.doc.node(second.id).and_then(|n| n.path_data()).and_then(|p| p.subpaths.get(second.subpath)) else { return vec![] };
    let n = sp.segment_count();
    let i = second.segment;
    let mut indices = vec![i];
    if i > 0 || sp.closed {
        indices.push((i + n - 1) % n);
    }
    if i + 1 < n || sp.closed {
        indices.push((i + 1) % n);
    }
    indices.sort_unstable();
    indices.dedup();
    let is_first = |k: usize| first.id == second.id && first.subpath == second.subpath && first.segment == k;
    indices.into_iter().filter(|k| *k < n && !is_first(*k)).map(|k| (k, sp.segment(k))).collect()
}

/// Where on `curve` the point `p` lies: its parameter, or none when `p` is within `tol` of either
/// end, where the segment already has an anchor point.
fn split_at(curve: &CubicBez, p: Point, tol: f64) -> Option<f64> {
    if p.distance(curve.p0) <= tol || p.distance(curve.p3) <= tol {
        return None;
    }
    let t = curve.nearest(p, 1e-12).t;
    (t > 0.0 && t < 1.0).then_some(t)
}

/// The `path.insertAnchor` steps that add an anchor point where circle `c` touches the first segment
/// and the segment of `second` (the segments a circle may touch, with their indices, on the subpath
/// of `pick`) nearest its second touch. On one subpath the later segment comes first, so that the
/// anchor added before it does not move it along.
fn anchor_steps(first: &Pick, pick: &Pick, second: &[(usize, CubicBez)], c: &SegmentCircle, tol: f64) -> Vec<Value> {
    let mut at: Vec<(NodeId, usize, usize, f64)> = vec![];
    if let Some(t) = split_at(&first.curve, c.first, tol) {
        at.push((first.id, first.subpath, first.segment, t));
    }
    let nearest = second.iter().min_by(|(_, a), (_, b)| a.nearest(c.second, 1e-12).distance_sq.total_cmp(&b.nearest(c.second, 1e-12).distance_sq));
    if let Some((k, curve)) = nearest
        && let Some(t) = split_at(curve, c.second, tol)
    {
        at.push((pick.id, pick.subpath, *k, t));
    }
    at.sort_by_key(|a| std::cmp::Reverse((a.0, a.1, a.2)));
    at.into_iter().map(|(id, subpath, segment, t)| json!({ "command": "path.insertAnchor", "params": { "id": id.0, "subpath": subpath, "segment": segment, "t": t } })).collect()
}

/// Of `circles`, the one touching the second segments nearest `at`, leaving out those [`draw`] cannot
/// draw within `tol`.
fn nearest_to(circles: Vec<SegmentCircle>, at: Point, tol: f64) -> Option<SegmentCircle> {
    circles.into_iter().filter(|c| draw(c, tol).is_some()).min_by(|a, b| (a.second - at).hypot().total_cmp(&(b.second - at).hypot()))
}

/// How far (screen pixels) the circle three points give back may be from the circle found.
const DRAW_PX: f64 = 0.05;
/// What the tool is told once its circle is drawn: then it forgets the first segment.
const DRAWN: &str = "tangentCircle.drawn";

/// The circles on `first` and `second`: tangent at the first pick, or of `radius`.
fn circles(first: &Pick, second: &[CubicBez], radius: Option<f64>) -> Vec<SegmentCircle> {
    match radius {
        Some(r) => circles_of_radius(first.curve, second, r),
        None => circles_through(first.curve, first.t, second),
    }
}

/// The step a Shift drag rounds the radius to: 1, 2 or 5 times a power of 10, at least `least`.
fn round_step(least: f64) -> f64 {
    if !(least.is_finite() && least > 0.0) {
        return 1.0;
    }
    let e = 10f64.powf(least.log10().floor());
    [1.0, 2.0, 5.0, 10.0].into_iter().map(|m| m * e).find(|s| *s >= least).unwrap_or(10.0 * e)
}

/// Draw circle `c`: through three points spread around it, with `shape.circleThroughPoints`, which
/// leaves it where it is with Align New Objects to Pixel Grid on, so that it stays tangent. None
/// when the three points do not give the circle back to within `tol` (a circle too small for its
/// distance from the origin, or too large, in floating point), or when the circle would reach past
/// the canvas ([`crate::circle::circle_of`]), which the command refuses.
fn draw(c: &SegmentCircle, tol: f64) -> Option<Action> {
    circle_params(c, tol).map(|p| Action::Exec(CIRCLE.into(), p))
}

/// The command [`draw`] runs.
const CIRCLE: &str = "shape.circleThroughPoints";

/// The params of the [`CIRCLE`] command that [`draw`] runs.
fn circle_params(c: &SegmentCircle, tol: f64) -> Option<Value> {
    let (at, r) = (c.circle.center, c.circle.radius);
    let points: Vec<Point> = [90.0f64, 210.0, 330.0]
        .iter()
        .map(|deg| {
            let (s, k) = deg.to_radians().sin_cos();
            Point::new(at.x + r * k, at.y + r * s)
        })
        .collect();
    let (back, rb) = crate::circle::circle_of(&points)?;
    if !(back.distance(at) <= tol && (rb - r).abs() <= tol) {
        return None;
    }
    let points: Vec<[f64; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
    Some(json!({ "points": points }))
}

fn segment_path(c: &CubicBez) -> BezPath {
    let mut bp = BezPath::new();
    bp.move_to(c.p0);
    bp.curve_to(c.p1, c.p2, c.p3);
    bp
}

impl TangentCircleTool {
    /// How near (document units) the pointer picks a segment: with Smart Guides on, the snapping
    /// tolerance or Smart Guides' Snapping Tolerance, whichever is larger; with them off, the
    /// snapping tolerance while `snapWithoutGuides` is on, and otherwise none. Never less than the
    /// selection tolerance.
    fn reach(&self, cx: &ToolContext) -> f64 {
        let pick = cx.pick_tol();
        if cx.smart_guides {
            pick.max(cx.tol(self.snap_px)).max(cx.snap_tol())
        } else if self.snap_without_guides {
            pick.max(cx.tol(self.snap_px))
        } else {
            pick
        }
    }

    /// The color the preview is drawn in.
    fn preview_color(&self) -> [u8; 3] {
        COLORS.get(self.color).map_or(RED, |(_, rgb)| *rgb)
    }

    /// The radius a drag gives, with Shift in round steps.
    fn shown_radius(cx: &ToolContext, r: f64, shift: bool) -> f64 {
        if !shift {
            return r;
        }
        let step = round_step(cx.tol(STEP_PX));
        // Less than half a step rounds to 0: no circle, as without Shift.
        (r / step).round() * step
    }

    /// The circles `pick` gives as the second segment, with Shift's `radius`: worked out again only
    /// when the segment, the radius or the shape of either segment changes.
    fn circles_on(&mut self, cx: &ToolContext, pick: &Pick, radius: Option<f64>) -> Vec<SegmentCircle> {
        let Some(first) = &self.first else { return vec![] };
        let second = second_set(cx, first, pick);
        let key = (pick.id, pick.subpath, pick.segment, radius.map(f64::to_bits), first.curve, second);
        if let Some((k, found)) = &self.cache
            && *k == key
        {
            return found.clone();
        }
        let found = circles(first, &key.5, radius);
        self.cache = Some((key, found.clone()));
        found
    }

    /// Forget the first segment when the document's no longer has the shape it was picked with: a
    /// live preview, or one that was canceled, changes it without an undo step, so the tool is not
    /// told ([`Tool::history_moved`]).
    fn forget_if_changed(&mut self, cx: &ToolContext) {
        if self.first.as_ref().is_some_and(|f| !unchanged(cx, f)) {
            self.forget();
        }
    }

    /// Look for the circle the pointer at `p` would give over `hover`, before any press.
    fn look(&mut self, cx: &ToolContext, p: Point, mods: Mods) {
        self.found = None;
        self.looked = false;
        self.forget_if_changed(cx);
        let Some(hover) = self.hover.clone().filter(|_| self.first.is_some()) else { return };
        let found = self.circles_on(cx, &hover, mods.shift.then_some(self.radius));
        self.found = nearest_to(found, p, cx.tol(DRAW_PX));
        self.looked = true;
    }

    /// The pointer at `ev` during a press on the second segment: once it is far enough from the
    /// press, the radius follows it (Cmd/Ctrl: a tenth of its movement; Shift: round steps).
    fn drag_to(&mut self, cx: &ToolContext, ev: &PointerEvent) {
        if self.press.is_none() {
            return;
        }
        // A change without an undo step since the press: a changed first segment is forgotten (the
        // press then draws nothing), and the circle follows the segments it may touch as they are now.
        self.forget_if_changed(cx);
        let (Some(first), Some(press)) = (&self.first, &mut self.press) else { return };
        let radius = press.shift_radius;
        // A pressed segment that is gone leaves nothing to touch.
        let second = if segment_now(cx, &press.pick).is_some() { second_set(cx, first, &press.pick) } else { vec![] };
        let moved = second != press.second;
        press.second = second;
        let reach = ev.pos.distance(press.at);
        let gear = if ev.mods.cmd { SLOW } else { 1.0 };
        let r = match press.radius {
            None if reach <= cx.tol(DRAG_PX) => {
                // Shift pressed or let go before the drag starts, or the segments changed: the
                // click's circle, with or without the tool's radius.
                if moved || ev.mods.shift != press.shift {
                    press.shift = ev.mods.shift;
                    let found = circles(first, &press.second, press.shift.then_some(radius));
                    self.found = nearest_to(found, press.at, cx.tol(DRAW_PX));
                }
                return;
            }
            None => reach * gear,
            Some(r) => (r + (reach - press.reach) * gear).max(0.0),
        };
        press.reach = reach;
        press.radius = Some(r);
        let shown = Self::shown_radius(cx, r, ev.mods.shift);
        // Dragged back to the press: a radius of 0, and no circle.
        self.found = if shown > 0.0 { nearest_to(circles(first, &press.second, Some(shown)), press.at, cx.tol(DRAW_PX)) } else { None };
    }

    fn forget(&mut self) {
        self.first = None;
        self.press = None;
        self.picking = false;
        self.found = None;
        self.looked = false;
        self.cache = None;
    }
}

impl Tool for TangentCircleTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.press.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        self.pointer = ev.pos;
        self.mods = ev.mods;
        self.over = true;
        match ev.kind {
            PointerKind::Move => {
                self.hover = pick_at(cx, ev.pos, self.reach(cx));
                self.look(cx, ev.pos, ev.mods);
                vec![]
            }
            PointerKind::Down => {
                let Some(pick) = pick_at(cx, ev.pos, self.reach(cx)) else { return vec![] };
                self.forget_if_changed(cx);
                let Some(first) = self.first.clone() else {
                    self.first = Some(pick);
                    self.picking = true;
                    self.hover = None;
                    self.found = None;
                    self.looked = false;
                    return vec![];
                };
                let second = second_set(cx, &first, &pick);
                let found = self.circles_on(cx, &pick, ev.mods.shift.then_some(self.radius));
                self.found = nearest_to(found, ev.pos, cx.tol(DRAW_PX));
                self.looked = true;
                self.hover = None;
                self.press = Some(Press { pick, second, at: ev.pos, shift: ev.mods.shift, shift_radius: self.radius, reach: 0.0, radius: None });
                vec![]
            }
            PointerKind::Drag => {
                self.drag_to(cx, ev);
                vec![]
            }
            PointerKind::Up => {
                // The release's own position and keys set the radius of a drag.
                self.drag_to(cx, ev);
                self.picking = false;
                let Some(press) = self.press.take() else { return vec![] };
                // No circle, or one the three points cannot draw: the first segment stays picked,
                // for another try. It also stays when the command fails: only a drawn circle tells
                // the tool (`notify`) to forget it.
                let Some(c) = self.found.take() else { return vec![] };
                let Some(params) = circle_params(&c, cx.tol(DRAW_PX)) else { return vec![] };
                self.looked = false;
                // Alt at the release turns `addPoints` the other way.
                let steps = match &self.first {
                    Some(first) if self.add_points != ev.mods.alt => {
                        let second = if segment_now(cx, &press.pick).is_some() { second_segments(cx, first, &press.pick) } else { vec![] };
                        anchor_steps(first, &press.pick, &second, &c, cx.tol(DRAW_PX))
                    }
                    _ => vec![],
                };
                if steps.is_empty() {
                    return vec![Action::Exec(CIRCLE.into(), params), Action::Notify(DRAWN.into())];
                }
                let mut commands = steps;
                // A path that was not selected becomes the selection when it gets an anchor; the
                // selection is put back before the circle, which Draw Behind places behind it.
                if commands.iter().any(|s| s["params"]["id"].as_u64().is_some_and(|id| !cx.selection.contains(NodeId(id)))) {
                    let ids: Vec<u64> = cx.selection.objects.iter().map(|id| id.0).collect();
                    commands.push(json!({ "command": "select.set", "params": { "ids": ids } }));
                }
                commands.push(json!({ "command": CIRCLE, "params": params }));
                // One undo step for the anchors and the circle, and none of them when a step fails.
                vec![Action::Exec("command.batch".into(), json!({ "label": "Ellipse", "commands": commands })), Action::Notify(DRAWN.into())]
            }
            // The canvas sends a double-click after the second click's `Up`, which drew the circle.
            PointerKind::DoubleClick | PointerKind::TripleClick => vec![],
        }
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.first.is_some() {
            self.forget();
        }
        vec![]
    }

    /// A dropped press (a canceled touch, a second finger) undoes only what that press did: a press
    /// on the second segment draws nothing and the first segment stays picked, and a press that
    /// picked the first segment leaves none picked.
    fn drop_press(&mut self, _cx: &ToolContext) -> Vec<Action> {
        if self.press.take().is_some() {
            self.found = None;
            self.looked = false;
        } else if std::mem::take(&mut self.picking) {
            self.forget();
        }
        vec![]
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        key == ToolKey::Escape && self.first.is_some()
    }

    fn notify(&mut self, _cx: &ToolContext, what: &str) {
        if what == DRAWN {
            self.forget();
        }
    }

    /// An edit, Undo or Redo outside the tool, or another document: the first segment may be gone.
    fn history_moved(&mut self) {
        self.forget();
        self.hover = None;
    }

    /// The segment and the circle shown depend on what lies under the pointer, which panning,
    /// zooming or an edit changes.
    fn wants_rehover(&self) -> bool {
        true
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = None;
        self.over = false;
        if self.press.is_none() {
            self.found = None;
            self.looked = false;
        }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        if let Some(first) = &self.first {
            out.push(Overlay::Path { path: segment_path(&first.curve), color: BLUE, width: 2.0, dashed: false });
            out.push(Overlay::Anchor { p: first.p, color: BLUE, filled: true, size: 7.0 });
        }
        if let Some(hover) = &self.hover {
            out.push(Overlay::Path { path: segment_path(&hover.curve), color: BLUE, width: 1.0, dashed: false });
            out.push(Overlay::Anchor { p: hover.p, color: BLUE, filled: false, size: 7.0 });
        }
        if let Some(press) = &self.press
            && press.radius.is_some()
        {
            out.push(Overlay::Line { a: press.at, b: self.pointer, color: GREY, dashed: false });
        }
        let color = self.preview_color();
        if let Some(c) = &self.found {
            let (at, r) = (c.circle.center, c.circle.radius);
            // A tolerance of at least a ten-thousandth of the radius keeps a huge circle to a few
            // dozen segments.
            let tol = cx.tol(0.1).max(r * 1e-4);
            out.push(Overlay::Path { path: kurbo::Circle::new(at, r).to_path(tol), color, width: 1.0, dashed: false });
            if self.center {
                let arm = cx.tol(4.0);
                out.push(Overlay::Line { a: at - kurbo::Vec2::new(arm, 0.0), b: at + kurbo::Vec2::new(arm, 0.0), color, dashed: false });
                out.push(Overlay::Line { a: at - kurbo::Vec2::new(0.0, arm), b: at + kurbo::Vec2::new(0.0, arm), color, dashed: false });
            }
            if self.radii {
                out.push(Overlay::Line { a: at, b: c.first, color, dashed: true });
                out.push(Overlay::Line { a: at, b: c.second, color, dashed: true });
            }
            out.push(Overlay::Measure { p: self.pointer, text: format!("R: {}", cx.len(r)) });
        } else if self.looked {
            // A label, which the canvas shows in the interface language.
            out.push(Overlay::Label { p: self.pointer, text: NO_CIRCLE.into(), color });
        }
        // `addPoints` in effect, with Alt as it is now.
        if (self.over || self.press.is_some()) && self.add_points != self.mods.alt {
            let off = cx.tol(DOT_PX);
            out.push(Overlay::CurvePoint { p: self.pointer + kurbo::Vec2::new(off, off), color: BLUE, filled: true, size: DOT_SIZE });
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Crosshair
    }

    fn options(&self) -> Value {
        json!({
            "radius": self.radius,
            "addPoints": self.add_points,
            "center": self.center,
            "radii": self.radii,
            "color": COLORS.get(self.color).map_or("red", |(name, _)| *name),
            "snapTolerance": self.snap_px,
            "snapWithoutGuides": self.snap_without_guides,
        })
    }

    /// Sets one option; a value the option does not take leaves it as it was.
    fn set_option(&mut self, key: &str, v: &Value) {
        if self.check_option(key, v).is_err() {
            return;
        }
        match key {
            "radius" => self.radius = v.as_f64().unwrap_or(self.radius),
            "addPoints" => self.add_points = v.as_bool().unwrap_or(self.add_points),
            "center" => self.center = v.as_bool().unwrap_or(self.center),
            "radii" => self.radii = v.as_bool().unwrap_or(self.radii),
            "color" => self.color = v.as_str().and_then(|s| COLORS.iter().position(|(name, _)| *name == s)).unwrap_or(self.color),
            "snapTolerance" => self.snap_px = v.as_f64().unwrap_or(self.snap_px),
            "snapWithoutGuides" => self.snap_without_guides = v.as_bool().unwrap_or(self.snap_without_guides),
            _ => {}
        }
    }

    fn check_option(&self, key: &str, v: &Value) -> Result<(), String> {
        use crate::option_check as c;
        match key {
            "radius" => c::number(key, v, RADIUS_RANGE.0, RADIUS_RANGE.1),
            "snapTolerance" => c::number(key, v, SNAP_RANGE.0, SNAP_RANGE.1),
            "color" => c::one_of(key, v, &COLORS.iter().map(|(name, _)| *name).collect::<Vec<_>>()),
            k if SWITCHES.contains(&k) => c::boolean(key, v),
            _ => Err(c::unknown(key)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::{Appearance, Document, MAX_COORD, Node, Selection};
    use vectorcraft_geom::{PathData, SubPath};

    /// Two open lines, (100, 100)–(300, 100) and (100, 140)–(300, 140), and an L from (100, 300)
    /// over to (300, 300) and down to (300, 400).
    fn doc() -> Document {
        doc_at(100.0, 140.0)
    }

    /// [`doc`] with the two open lines at heights `a` and `b`, with the same ids.
    fn doc_at(a: f64, b: f64) -> Document {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let lines: [&[(f64, f64)]; 3] = [&[(100.0, a), (300.0, a)], &[(100.0, b), (300.0, b)], &[(100.0, 300.0), (300.0, 300.0), (300.0, 400.0)]];
        for pts in lines {
            let id = d.alloc_id();
            let pts: Vec<Point> = pts.iter().map(|(x, y)| Point::new(*x, *y)).collect();
            d.insert(Some(l), 0, Node::path(id, PathData::single(SubPath::polyline(&pts, false)), Appearance::default_art())).unwrap();
        }
        d
    }

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    fn shifted(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        let mut e = ev(kind, x, y);
        e.mods.shift = true;
        e
    }

    /// A release, with the tool told what its actions tell it, as the session does once the command
    /// before the `Notify` has run.
    fn up(t: &mut TangentCircleTool, cx: &ToolContext, e: &PointerEvent) -> Vec<Action> {
        let acts = t.pointer(cx, e);
        for a in &acts {
            if let Action::Notify(what) = a {
                t.notify(cx, what);
            }
        }
        acts
    }

    /// The circle an action list draws, as the box around it: (x, y, width, height).
    fn drawn(actions: &[Action]) -> (f64, f64, f64, f64) {
        let [Action::Exec(cmd, p), Action::Notify(what)] = actions else { panic!("{actions:?}") };
        assert_eq!((cmd.as_str(), what.as_str()), ("shape.circleThroughPoints", DRAWN));
        let pts: Vec<Point> = p["points"].as_array().unwrap().iter().map(|q| Point::new(q[0].as_f64().unwrap(), q[1].as_f64().unwrap())).collect();
        let (at, r) = vectorcraft_geom::shapes::circle_through(pts[0], pts[1], pts[2]).unwrap();
        (at.x - r, at.y - r, 2.0 * r, 2.0 * r)
    }

    fn close(a: (f64, f64, f64, f64), b: (f64, f64, f64, f64)) -> bool {
        [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)].iter().all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn two_clicks_draw_the_circle_tangent_at_the_first_click() {
        let d = doc();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TangentCircleTool::default();
        assert_eq!(t.cursor(&cx, Point::ZERO, Mods::default()), Cursor::Crosshair);
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 150.0, 101.0)).is_empty(), "the first segment");
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 150.0, 101.0)).is_empty());
        // Over the second line: the circle through (150, 100) touching it shows, 20 pt in radius.
        t.pointer(&cx, &ev(PointerKind::Move, 200.0, 139.0));
        let ov = t.overlays(&cx);
        assert!(ov.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text.starts_with("R: 20.00"))), "{ov:?}");
        assert!(ov.iter().filter(|o| matches!(o, Overlay::Line { dashed: true, .. })).count() == 2, "the radii: {ov:?}");
        // Moving along the same segment picks among the circles already found.
        let kept = t.cache.clone();
        assert!(kept.as_ref().is_some_and(|(_, c)| c.len() == 1));
        t.pointer(&cx, &ev(PointerKind::Move, 250.0, 139.0));
        assert_eq!(t.cache, kept);
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 200.0, 139.0)).is_empty());
        let released = t.pointer(&cx, &ev(PointerKind::Up, 200.0, 139.0));
        assert!(close(drawn(&released), (130.0, 100.0, 40.0, 40.0)), "{released:?}");
        // Until the command has run and the tool is told, the first segment stays: a command that
        // fails leaves it picked.
        assert!(t.first.is_some() && !t.busy());
        t.notify(&cx, DRAWN);
        assert!(t.first.is_none() && t.overlays(&cx).is_empty(), "ready for the next circle");
        // A quick second click on the second segment: the canvas sends its `Up` and then a
        // double-click, which draws nothing more.
        for (kind, x, y) in [(PointerKind::Down, 150.0, 101.0), (PointerKind::Up, 150.0, 101.0), (PointerKind::Down, 200.0, 139.0)] {
            assert!(t.pointer(&cx, &ev(kind, x, y)).is_empty());
        }
        let released = up(&mut t, &cx, &ev(PointerKind::Up, 200.0, 139.0));
        assert!(close(drawn(&released), (130.0, 100.0, 40.0, 40.0)), "{released:?}");
        assert!(t.pointer(&cx, &ev(PointerKind::DoubleClick, 200.0, 139.0)).is_empty());
        assert!(t.first.is_none() && !t.busy());
    }

    #[test]
    fn shift_uses_the_radius_and_a_corner_takes_its_fillet() {
        let d = doc();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TangentCircleTool::default();
        t.set_option("radius", &json!(10));
        assert_eq!(t.options()["radius"], json!(10.0));
        // `tool.setOption` refuses a value outside 0.01 to 1000000, or one that is not a number,
        // and any other option; set anyway, such a value leaves the radius as it is.
        for good in [json!(0.01), json!(36), json!(1e6)] {
            assert_eq!(t.check_option("radius", &good), Ok(()), "{good}");
        }
        for bad in [json!(0), json!(0.005), json!(2e6), json!(-5), json!("12"), Value::Null] {
            assert_eq!(t.check_option("radius", &bad), Err("`radius` must be a number from 0.01 to 1000000".into()), "{bad}");
            t.set_option("radius", &bad);
            assert_eq!(t.options()["radius"], json!(10.0), "{bad}");
        }
        assert!(t.check_option("size", &json!(10)).is_err());
        // The L's two segments: the circle of radius 10 in its corner.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 300.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 300.0));
        t.pointer(&cx, &shifted(PointerKind::Down, 300.0, 350.0));
        let released = up(&mut t, &cx, &shifted(PointerKind::Up, 300.0, 350.0));
        assert!(close(drawn(&released), (280.0, 300.0, 20.0, 20.0)), "{released:?}");
        // Shift pressed or let go during a click counts at the release: with a radius of 5 no
        // circle reaches the line 40 pt away, and without Shift the click's circle is 20 pt.
        let mut t = TangentCircleTool::default();
        t.set_option("radius", &json!(5));
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 139.0));
        t.pointer(&cx, &shifted(PointerKind::Drag, 200.0, 139.0));
        assert!(t.pointer(&cx, &shifted(PointerKind::Up, 200.0, 139.0)).is_empty(), "nothing drawn");
        assert!(t.first.is_some());
        t.pointer(&cx, &shifted(PointerKind::Down, 200.0, 139.0));
        let released = t.pointer(&cx, &ev(PointerKind::Up, 200.0, 139.0));
        assert!(close(drawn(&released), (130.0, 100.0, 40.0, 40.0)), "{released:?}");
    }

    #[test]
    fn a_press_keeps_the_radius_it_started_with() {
        let d = doc();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TangentCircleTool::default();
        t.set_option("radius", &json!(20));
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 101.0));
        t.pointer(&cx, &shifted(PointerKind::Down, 200.0, 139.0));
        // A radius written during the press does not reach the circle, however Shift changes.
        t.set_option("radius", &json!(5));
        t.pointer(&cx, &ev(PointerKind::Drag, 200.0, 139.0));
        t.pointer(&cx, &shifted(PointerKind::Drag, 200.0, 139.0));
        let released = up(&mut t, &cx, &shifted(PointerKind::Up, 200.0, 139.0));
        assert!(close(drawn(&released), (180.0, 100.0, 40.0, 40.0)), "{released:?}");
    }

    #[test]
    fn the_preview_options_set_its_cross_radii_and_color() {
        let d = doc();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let green = COLORS.iter().find(|(name, _)| *name == "green").map(|(_, rgb)| *rgb);
        let lines =
            |ov: &[Overlay], dashed_lines: bool| ov.iter().filter(|o| matches!(o, Overlay::Line { dashed, .. } if *dashed == dashed_lines)).count();
        let mut t = TangentCircleTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Move, 200.0, 139.0));
        // By default: the red circle, its center cross (two lines) and its two radii.
        let ov = t.overlays(&cx);
        assert_eq!((lines(&ov, false), lines(&ov, true)), (2, 2), "{ov:?}");
        assert!(ov.iter().any(|o| matches!(o, Overlay::Path { color, width, .. } if *color == RED && *width == 1.0)), "{ov:?}");
        t.set_option("center", &json!(false));
        let ov = t.overlays(&cx);
        assert_eq!((lines(&ov, false), lines(&ov, true)), (0, 2), "no cross: {ov:?}");
        t.set_option("radii", &json!(false));
        t.set_option("color", &json!("green"));
        let ov = t.overlays(&cx);
        assert_eq!((lines(&ov, false), lines(&ov, true)), (0, 0), "no radii: {ov:?}");
        assert!(ov.iter().any(|o| matches!(o, Overlay::Path { color, .. } if Some(*color) == green)), "{ov:?}");
        assert!(ov.iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text.starts_with("R: 20.00"))), "the radius still shows: {ov:?}");
        // The note takes the color too.
        t.set_option("radius", &json!(5));
        t.pointer(&cx, &shifted(PointerKind::Move, 200.0, 140.0));
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Label { text, color, .. } if text == NO_CIRCLE && Some(*color) == green)));
    }

    #[test]
    fn the_preview_and_snap_options_take_only_their_values() {
        let mut t = TangentCircleTool::default();
        let o = t.options();
        assert_eq!(
            (&o["center"], &o["radii"], &o["color"], &o["snapTolerance"], &o["snapWithoutGuides"]),
            (&json!(true), &json!(true), &json!("red"), &json!(3.0), &json!(true))
        );
        for (key, good) in [
            ("center", json!(false)),
            ("radii", json!(false)),
            ("snapWithoutGuides", json!(false)),
            ("color", json!("gray")),
            ("snapTolerance", json!(0)),
            ("snapTolerance", json!(25)),
            ("snapTolerance", json!(7.5)),
        ] {
            assert_eq!(t.check_option(key, &good), Ok(()), "{key} {good}");
            t.set_option(key, &good);
            let now = t.options()[key].clone();
            assert!(now == good || (good.is_number() && now.as_f64() == good.as_f64()), "{key}: {now} for {good}");
        }
        let colors = "`color` must be one of `red`, `blue`, `magenta`, `green`, `black`, `gray`";
        for (key, bad, error) in [
            ("center", json!(1), "`center` must be true or false"),
            ("radii", json!("no"), "`radii` must be true or false"),
            ("snapWithoutGuides", Value::Null, "`snapWithoutGuides` must be true or false"),
            ("color", json!("orange"), colors),
            ("color", json!("Red"), colors),
            ("color", json!(0), colors),
            ("snapTolerance", json!(-1), "`snapTolerance` must be a number from 0 to 25"),
            ("snapTolerance", json!(26), "`snapTolerance` must be a number from 0 to 25"),
            ("snapTolerance", json!("3"), "`snapTolerance` must be a number from 0 to 25"),
        ] {
            assert_eq!(t.check_option(key, &bad), Err(error.into()), "{key} {bad}");
            let before = t.options();
            t.set_option(key, &bad);
            assert_eq!(t.options(), before, "{key} {bad}");
        }
    }

    #[test]
    fn the_snap_options_set_how_near_a_segment_is_picked() {
        let d = doc();
        let (sel, p) = (Selection::default(), paint());
        // At 100%, Selection Tolerance 3 px; the first line is at y = 100.
        let picks = |t: &mut TangentCircleTool, cx: &ToolContext, y: f64| {
            t.forget();
            t.pointer(cx, &ev(PointerKind::Down, 150.0, y));
            t.pointer(cx, &ev(PointerKind::Up, 150.0, y));
            t.first.is_some()
        };
        let guides = |tolerance: f64| ToolContext { smart_guides: true, snapping_tolerance: tolerance, ..cx(&d, &sel, &p) };
        let no_guides = ToolContext { smart_guides: false, ..cx(&d, &sel, &p) };
        let mut t = TangentCircleTool::default();
        // Smart Guides on: their Snapping Tolerance when it is larger than the tool's.
        assert!(picks(&mut t, &guides(10.0), 109.0));
        assert!(!picks(&mut t, &guides(10.0), 111.0));
        t.set_option("snapTolerance", &json!(12));
        assert!(picks(&mut t, &guides(4.0), 111.0), "the tool's when it is larger");
        assert!(!picks(&mut t, &guides(4.0), 113.0));
        // Smart Guides off: the tool's tolerance while it snaps without them.
        t.set_option("snapTolerance", &json!(8));
        assert!(picks(&mut t, &no_guides, 107.0));
        assert!(!picks(&mut t, &no_guides, 109.0));
        // Without snapping, only the Selection Tolerance; a tolerance below it does not shrink it.
        t.set_option("snapWithoutGuides", &json!(false));
        assert!(!picks(&mut t, &no_guides, 105.0));
        assert!(picks(&mut t, &no_guides, 102.5));
        t.set_option("snapWithoutGuides", &json!(true));
        t.set_option("snapTolerance", &json!(0));
        assert!(picks(&mut t, &no_guides, 102.5));
        // Over the gap between the two lines, the nearer one is picked, though both are in reach.
        t.set_option("snapTolerance", &json!(25));
        t.forget();
        t.pointer(&no_guides, &ev(PointerKind::Move, 150.0, 122.0));
        assert!(t.hover.as_ref().is_some_and(|h| (h.p.y - 140.0).abs() < 1e-9), "{:?}", t.hover);
        t.pointer(&no_guides, &ev(PointerKind::Move, 150.0, 118.0));
        assert!(t.hover.as_ref().is_some_and(|h| (h.p.y - 100.0).abs() < 1e-9), "{:?}", t.hover);
        // A segment exactly at the reach is in reach, on either side of it.
        t.pointer(&no_guides, &ev(PointerKind::Move, 150.0, 75.0));
        assert!(t.hover.as_ref().is_some_and(|h| (h.p.y - 100.0).abs() < 1e-9), "{:?}", t.hover);
        t.pointer(&no_guides, &ev(PointerKind::Move, 150.0, 165.0));
        assert!(t.hover.as_ref().is_some_and(|h| (h.p.y - 140.0).abs() < 1e-9), "{:?}", t.hover);
    }

    #[test]
    fn a_drag_sets_the_radius_with_shift_steps_and_a_slow_drag() {
        let d = doc();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TangentCircleTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 300.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 300.0));
        t.pointer(&cx, &ev(PointerKind::Down, 300.0, 350.0));
        assert!(t.busy());
        // A drag 12.3 pt long asks for that radius; Shift rounds it (4 px at 100%: steps of 5).
        assert!(t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 362.3)).is_empty());
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Line { dashed: false, color, .. } if *color == GREY)));
        assert!((t.found.unwrap().circle.radius - 12.3).abs() < 1e-6);
        t.pointer(&cx, &shifted(PointerKind::Drag, 300.0, 362.3));
        assert!((t.found.unwrap().circle.radius - 10.0).abs() < 1e-9);
        // Cmd/Ctrl: 10 pt more of drag adds 1 pt.
        let mut slow = ev(PointerKind::Drag, 300.0, 372.3);
        slow.mods.cmd = true;
        t.pointer(&cx, &slow);
        assert!((t.found.unwrap().circle.radius - 13.3).abs() < 1e-6, "{:?}", t.found);
        let released = up(&mut t, &cx, &ev(PointerKind::Up, 300.0, 372.3));
        assert!(close(drawn(&released), (286.7 - 13.3, 300.0, 26.6, 26.6)), "{released:?}");
        // A drag back past its press asks for no radius: no circle, not one through the first click.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 300.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 300.0));
        t.pointer(&cx, &ev(PointerKind::Down, 300.0, 350.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 360.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 350.0));
        assert!(t.found.is_none(), "{:?}", t.found);
        t.pointer(&cx, &shifted(PointerKind::Drag, 300.0, 350.0));
        assert!(t.found.is_none(), "with Shift too: {:?}", t.found);
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 300.0, 350.0)).is_empty());
        // A release away from the last drag event: the release sets the radius.
        t.pointer(&cx, &ev(PointerKind::Down, 300.0, 350.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 300.0, 360.0));
        let released = up(&mut t, &cx, &ev(PointerKind::Up, 300.0, 365.0));
        assert!(close(drawn(&released), (300.0 - 30.0, 300.0, 30.0, 30.0)), "{released:?}");
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 300.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 300.0));
        // Cmd/Ctrl held from the start: a 20 pt drag gives 2 pt.
        t.pointer(&cx, &ev(PointerKind::Down, 300.0, 350.0));
        let mut slow = ev(PointerKind::Drag, 300.0, 370.0);
        slow.mods.cmd = true;
        t.pointer(&cx, &slow);
        assert!((t.found.unwrap().circle.radius - 2.0).abs() < 1e-6, "{:?}", t.found);
        assert_eq!(round_step(2.0), 2.0);
        assert_eq!(round_step(0.3), 0.5);
        assert_eq!(round_step(7.0), 10.0);
    }

    #[test]
    fn no_circle_says_so_and_escape_forgets_the_first_segment() {
        let d = doc();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TangentCircleTool::default();
        // The first segment again (on the L, whose other segment is beside it): no circle.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 300.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 300.0));
        t.pointer(&cx, &shifted(PointerKind::Move, 200.0, 300.0));
        assert!(t.found.is_none() && t.looked, "{:?}", t.found);
        t.key(&cx, ToolKey::Escape, Mods::default());
        // Nothing under the pointer: nothing picked.
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0)).is_empty());
        assert!(t.first.is_none());
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 100.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 100.0));
        // Shift with a radius of 5: no circle reaches the line 40 pt away.
        t.set_option("radius", &json!(5));
        t.pointer(&cx, &shifted(PointerKind::Move, 200.0, 140.0));
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == NO_CIRCLE)));
        assert!(t.wants_rehover(), "the note follows what lies under a still pointer");
        t.pointer(&cx, &shifted(PointerKind::Down, 200.0, 140.0));
        assert!(t.pointer(&cx, &shifted(PointerKind::Up, 200.0, 140.0)).is_empty(), "nothing drawn");
        assert!(t.first.is_some(), "the first segment stays for another try");
        assert!(t.claims_key(&cx, ToolKey::Escape));
        t.key(&cx, ToolKey::Escape, Mods::default());
        assert!(t.first.is_none() && !t.claims_key(&cx, ToolKey::Escape));
        // An edit the tool did not make (or Undo, or another document) forgets it too.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 100.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 100.0));
        assert!(t.first.is_some());
        t.history_moved();
        assert!(t.first.is_none() && t.overlays(&cx).is_empty());
    }

    #[test]
    fn a_dropped_press_undoes_only_what_it_did() {
        let d = doc();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TangentCircleTool::default();
        // A press on the second segment, dropped: nothing drawn, and the first segment stays.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 139.0));
        assert!(t.busy());
        assert!(t.drop_press(&cx).is_empty());
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 200.0, 139.0)).is_empty(), "the release that follows draws nothing");
        assert!(t.first.is_some() && !t.busy());
        // The next click there draws the circle.
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 139.0));
        let released = t.pointer(&cx, &ev(PointerKind::Up, 200.0, 139.0));
        assert!(close(drawn(&released), (130.0, 100.0, 40.0, 40.0)), "{released:?}");
        t.notify(&cx, DRAWN);
        // A press that picked the first segment, dropped: no segment stays picked.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 101.0));
        assert!(t.drop_press(&cx).is_empty());
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 101.0));
        assert!(t.first.is_none());
        // A dropped press that picked nothing leaves the first segment picked.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Down, 50.0, 50.0));
        assert!(t.drop_press(&cx).is_empty());
        t.pointer(&cx, &ev(PointerKind::Up, 50.0, 50.0));
        assert!(t.first.is_some());
    }

    #[test]
    fn a_change_without_an_undo_step_is_not_drawn_from_the_old_shapes() {
        let (sel, p) = (Selection::default(), paint());
        let (d, previewed, first_previewed) = (doc(), doc_at(100.0, 180.0), doc_at(80.0, 140.0));
        let (cx, during, first_during) = (cx(&d, &sel, &p), cx(&previewed, &sel, &p), cx(&first_previewed, &sel, &p));
        let mut t = TangentCircleTool::default();
        // A live preview moves the second line down to 180, and the pointer over it shows a 40 pt
        // circle. The preview is canceled, which leaves no undo step: the click draws the 20 pt circle
        // touching the line where it is again, not the one found during the preview.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 101.0));
        t.pointer(&during, &ev(PointerKind::Move, 200.0, 179.0));
        assert!(t.overlays(&during).iter().any(|o| matches!(o, Overlay::Measure { text, .. } if text.starts_with("R: 40.00"))));
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 139.0));
        let released = up(&mut t, &cx, &ev(PointerKind::Up, 200.0, 139.0));
        assert!(close(drawn(&released), (130.0, 100.0, 40.0, 40.0)), "{released:?}");
        // The same for a press made during the preview and released after it was canceled.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 101.0));
        t.pointer(&during, &ev(PointerKind::Down, 200.0, 179.0));
        let released = up(&mut t, &cx, &ev(PointerKind::Up, 200.0, 179.0));
        assert!(close(drawn(&released), (130.0, 100.0, 40.0, 40.0)), "{released:?}");
        // A first segment picked during a preview that moved it, canceled since: forgotten, with no
        // circle shown from where it was.
        t.pointer(&first_during, &ev(PointerKind::Down, 150.0, 81.0));
        t.pointer(&first_during, &ev(PointerKind::Up, 150.0, 81.0));
        assert!(t.first.is_some());
        t.pointer(&cx, &ev(PointerKind::Move, 200.0, 139.0));
        assert!(t.first.is_none() && t.found.is_none() && !t.looked);
        // A press on a first segment that the preview changed since the press draws nothing.
        t.pointer(&first_during, &ev(PointerKind::Down, 150.0, 81.0));
        t.pointer(&first_during, &ev(PointerKind::Up, 150.0, 81.0));
        t.pointer(&first_during, &ev(PointerKind::Down, 200.0, 139.0));
        assert!(t.busy());
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 200.0, 139.0)).is_empty());
        assert!(t.first.is_none() && !t.busy());
        // A press on the L's second segment, which a preview then removes: nothing to touch, so the
        // release draws no circle against the segment left beside it.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Down, 300.0, 350.0));
        let l = t.press.as_ref().map(|p| (p.pick.id, p.pick.segment));
        assert!(matches!(l, Some((_, 1))), "{l:?}");
        let mut cut = doc();
        if let Some(path) = l.and_then(|(id, _)| cut.node_mut(id)).and_then(|n| n.path_data_mut()) {
            path.subpaths[0] = SubPath::polyline(&[Point::new(100.0, 300.0), Point::new(300.0, 300.0)], false);
        }
        let cut_cx = crate::testutil::cx(&cut, &sel, &p);
        assert!(t.pointer(&cut_cx, &ev(PointerKind::Up, 300.0, 350.0)).is_empty(), "nothing drawn");
        assert!(t.first.is_some(), "the first segment stays for another try");
    }

    #[test]
    fn a_circle_the_three_points_cannot_draw_keeps_the_first_segment() {
        // A circle the geometry found, moved to (x, 0) and given radius `r`.
        let line = |y: f64| CubicBez::new(Point::new(0.0, y), Point::new(30.0, y), Point::new(60.0, y), Point::new(90.0, y));
        let found = circles_of_radius(line(0.0), &[line(40.0)], 20.0);
        let circle = |x: f64, r: f64| {
            let mut c = found[0];
            (c.circle.center, c.circle.radius, c.first, c.second) = (Point::new(x, 0.0), r, Point::new(x, -r), Point::new(x, r));
            c
        };
        assert!(draw(&circle(100.0, 20.0), 1e-3).is_some());
        // At 10⁹ pt from the origin, where coordinates step by about 1.2e-7 pt, a radius of 5e-8 pt
        // rounds the three points onto one line.
        assert!(draw(&circle(1e9, 5e-8), 1e-3).is_none());
        // The preview leaves such a circle out, even nearest the pointer: no red circle for a click
        // that draws nothing.
        let at = Point::new(1e9, 0.0);
        assert_eq!(nearest_to(vec![circle(1e9, 5e-8), circle(100.0, 20.0)], at, 1e-3), Some(circle(100.0, 20.0)));
        assert_eq!(nearest_to(vec![circle(1e9, 5e-8)], at, 1e-3), None);
        let d = doc();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TangentCircleTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 139.0));
        t.found = Some(circle(1e9, 5e-8));
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 200.0, 139.0)).is_empty(), "nothing drawn");
        assert!(t.first.is_some(), "the first segment stays for another try");
        // A huge circle (segments 10³⁰ pt apart) shows with a few dozen segments, not millions.
        t.found = Some(circle(0.0, 1e30));
        let segments: Vec<usize> = t
            .overlays(&cx)
            .iter()
            .filter_map(|o| match o {
                Overlay::Path { path, color, .. } if *color == RED => Some(path.elements().len()),
                _ => None,
            })
            .collect();
        assert!(matches!(segments[..], [n] if n < 100), "{segments:?}");
    }

    /// The steps of the `command.batch` an action list runs: (command, params) each.
    fn batch_steps(actions: &[Action]) -> Vec<(String, Value)> {
        let [Action::Exec(cmd, p), Action::Notify(what)] = actions else { panic!("{actions:?}") };
        assert_eq!((cmd.as_str(), what.as_str()), ("command.batch", DRAWN));
        p["commands"].as_array().unwrap().iter().map(|s| (s["command"].as_str().unwrap().to_string(), s["params"].clone())).collect()
    }

    /// An inserted anchor's place: (path, subpath, segment) and the point at its `t` on that segment.
    fn inserted(cx: &ToolContext, params: &Value) -> (u64, u64, u64, Point) {
        let (id, sub, seg) = (params["id"].as_u64().unwrap(), params["subpath"].as_u64().unwrap(), params["segment"].as_u64().unwrap());
        let sp = &cx.doc.node(NodeId(id)).unwrap().path_data().unwrap().subpaths[sub as usize];
        (id, sub, seg, sp.segment(seg as usize).eval(params["t"].as_f64().unwrap()))
    }

    fn alt(mut e: PointerEvent) -> PointerEvent {
        e.mods.alt = true;
        e
    }

    #[test]
    fn add_points_adds_an_anchor_where_the_circle_touches_each_segment() {
        let d = doc();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let line_at = |y: f64| pick_at(&cx, Point::new(150.0, y), cx.pick_tol()).map(|p| p.id.0).unwrap();
        let (top, below) = (line_at(100.0), line_at(140.0));
        let mut t = TangentCircleTool::default();
        t.set_option("addPoints", &json!(true));
        assert_eq!(t.options()["addPoints"], json!(true));
        // The circle through (150, 100) touching the line below at (150, 140): an anchor a quarter
        // along each line, then the circle, in one batch.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Down, 200.0, 139.0));
        let steps = batch_steps(&up(&mut t, &cx, &ev(PointerKind::Up, 200.0, 139.0)));
        assert_eq!(steps.len(), 4, "{steps:?}");
        assert!(steps[..2].iter().all(|(c, _)| c == "path.insertAnchor"), "{steps:?}");
        let mut anchors: Vec<(u64, u64, u64, Point)> = steps[..2].iter().map(|(_, p)| inserted(&cx, p)).collect();
        anchors.sort_by_key(|a| a.0);
        let mut want = [(top, 0, 0, Point::new(150.0, 100.0)), (below, 0, 0, Point::new(150.0, 140.0))];
        want.sort_by_key(|a| a.0);
        assert!(top != below && anchors.iter().zip(want).all(|(a, b)| (a.0, a.1, a.2) == (b.0, b.1, b.2) && a.3.distance(b.3) < 1e-6), "{anchors:?}");
        // The lines were not selected, so the selection (none) is put back before the circle.
        assert_eq!(steps[2], ("select.set".to_string(), json!({ "ids": [] })), "{steps:?}");
        let (cmd, params) = &steps[3];
        let circle = drawn(&[Action::Exec(cmd.clone(), params.clone()), Action::Notify(DRAWN.into())]);
        assert!(close(circle, (130.0, 100.0, 40.0, 40.0)), "{steps:?}");
        assert!(t.first.is_none(), "ready for the next circle");
        // With both lines selected, the anchors leave the selection as it is: no step puts it back.
        let mut both = Selection::default();
        both.add(NodeId(top));
        both.add(NodeId(below));
        let selected = self::cx(&d, &both, &p);
        t.pointer(&selected, &ev(PointerKind::Down, 150.0, 101.0));
        t.pointer(&selected, &ev(PointerKind::Up, 150.0, 101.0));
        t.pointer(&selected, &ev(PointerKind::Down, 200.0, 139.0));
        let steps = batch_steps(&up(&mut t, &selected, &ev(PointerKind::Up, 200.0, 139.0)));
        assert!(steps.len() == 3 && steps[2].0 == CIRCLE, "{steps:?}");
        // Alt at the release turns it off for that circle, and with the option off turns it on.
        for (on, alt_up, adds) in [(true, true, false), (false, true, true), (false, false, false)] {
            t.set_option("addPoints", &json!(on));
            t.pointer(&cx, &ev(PointerKind::Down, 150.0, 101.0));
            t.pointer(&cx, &ev(PointerKind::Up, 150.0, 101.0));
            t.pointer(&cx, &ev(PointerKind::Down, 200.0, 139.0));
            t.pointer(&cx, &alt(ev(PointerKind::Drag, 200.0, 139.0)));
            let release = if alt_up { alt(ev(PointerKind::Up, 200.0, 139.0)) } else { ev(PointerKind::Up, 200.0, 139.0) };
            let released = up(&mut t, &cx, &release);
            if adds {
                assert_eq!(batch_steps(&released).len(), 4, "on {on}, Alt {alt_up}: {released:?}");
            } else {
                assert!(close(drawn(&released), (130.0, 100.0, 40.0, 40.0)), "on {on}, Alt {alt_up}: {released:?}");
            }
        }
    }

    #[test]
    fn add_points_on_one_path_adds_the_later_anchor_first_and_none_at_an_anchor() {
        let d = doc();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TangentCircleTool::default();
        t.set_option("addPoints", &json!(true));
        t.set_option("radius", &json!(10));
        // The L's corner circle of radius 10 touches (290, 300) on its first segment and (300, 310)
        // on its second: the second segment's anchor comes first, so the first one's index holds.
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 300.0));
        t.pointer(&cx, &ev(PointerKind::Up, 150.0, 300.0));
        t.pointer(&cx, &shifted(PointerKind::Down, 300.0, 350.0));
        let steps = batch_steps(&up(&mut t, &cx, &shifted(PointerKind::Up, 300.0, 350.0)));
        assert_eq!(steps.len(), 4, "{steps:?}");
        let l = pick_at(&cx, Point::new(300.0, 350.0), cx.pick_tol()).map(|p| p.id.0);
        let anchors: Vec<(u64, u64, u64, Point)> = steps[..2].iter().map(|(_, p)| inserted(&cx, p)).collect();
        let want = [(1, Point::new(300.0, 310.0)), (0, Point::new(290.0, 300.0))];
        assert!(anchors.iter().zip(want).all(|(a, (seg, at))| Some(a.0) == l && a.1 == 0 && a.2 == seg && a.3.distance(at) < 1e-6), "{anchors:?}");
        assert_eq!((steps[2].0.as_str(), steps[3].0.as_str()), ("select.set", CIRCLE));
        // A circle touching both lines at their right ends: no anchor is added on top of an anchor,
        // so only the circle is drawn, as without the option.
        t.set_option("radius", &json!(36));
        t.pointer(&cx, &ev(PointerKind::Down, 300.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Up, 300.0, 101.0));
        t.pointer(&cx, &ev(PointerKind::Down, 299.0, 139.0));
        let released = up(&mut t, &cx, &ev(PointerKind::Up, 299.0, 139.0));
        assert!(close(drawn(&released), (280.0, 100.0, 40.0, 40.0)), "{released:?}");
    }

    #[test]
    fn add_points_shows_a_dot_and_is_an_option_that_refuses_other_values() {
        let d = doc();
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TangentCircleTool::default();
        assert_eq!(t.options()["addPoints"], json!(false), "off to start");
        let dot =
            |t: &TangentCircleTool| t.overlays(&cx).iter().any(|o| matches!(o, Overlay::CurvePoint { filled: true, color, .. } if *color == BLUE));
        t.pointer(&cx, &ev(PointerKind::Move, 50.0, 50.0));
        assert!(!dot(&t));
        // Alt turns it on while held, and back off with the option on.
        t.pointer(&cx, &alt(ev(PointerKind::Move, 50.0, 50.0)));
        assert!(dot(&t));
        t.set_option("addPoints", &json!(true));
        assert!(!dot(&t));
        t.pointer(&cx, &ev(PointerKind::Move, 50.0, 50.0));
        assert!(dot(&t));
        let Some(Overlay::CurvePoint { p, .. }) = t.overlays(&cx).into_iter().find(|o| matches!(o, Overlay::CurvePoint { .. })) else {
            panic!("no dot")
        };
        assert!(p.x > 50.0 && p.y > 50.0, "beside the pointer: {p:?}");
        // The pointer off the canvas: no dot left behind.
        t.hover_ended(&cx);
        assert!(!dot(&t));
        for bad in [json!(1), json!("true"), Value::Null] {
            assert_eq!(t.check_option("addPoints", &bad), Err("`addPoints` must be true or false".into()), "{bad}");
            t.set_option("addPoints", &bad);
            assert_eq!(t.options()["addPoints"], json!(true), "{bad}");
        }
        assert_eq!(t.check_option("addPoints", &json!(false)), Ok(()));
    }

    #[test]
    fn a_circle_that_would_reach_past_the_canvas_is_not_offered() {
        // Two lines 40 pt apart ending at the canvas edge, x = 4,000,000: a click at x the first gives
        // the circle of radius 20 around (x, 120), whose right side is at x + 20.
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        for y in [100.0, 140.0] {
            let id = d.alloc_id();
            let pts = [Point::new(MAX_COORD - 100.0, y), Point::new(MAX_COORD, y)];
            d.insert(Some(l), 0, Node::path(id, PathData::single(SubPath::polyline(&pts, false)), Appearance::default_art())).unwrap();
        }
        let (sel, p) = (Selection::default(), paint());
        let cx = cx(&d, &sel, &p);
        let mut t = TangentCircleTool::default();
        // Past the edge by 5 pt: the note shows, and a click draws nothing.
        let x = MAX_COORD - 15.0;
        t.pointer(&cx, &ev(PointerKind::Down, x, 100.0));
        t.pointer(&cx, &ev(PointerKind::Up, x, 100.0));
        t.pointer(&cx, &ev(PointerKind::Move, x, 140.0));
        assert!(t.found.is_none() && t.looked, "{:?}", t.found);
        assert!(t.overlays(&cx).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == NO_CIRCLE)));
        t.pointer(&cx, &ev(PointerKind::Down, x, 140.0));
        assert!(t.pointer(&cx, &ev(PointerKind::Up, x, 140.0)).is_empty(), "nothing drawn");
        assert!(t.first.is_some(), "the first segment stays for another try");
        // Its right side 1 pt inside the edge: drawn.
        let x = MAX_COORD - 21.0;
        t.key(&cx, ToolKey::Escape, Mods::default());
        t.pointer(&cx, &ev(PointerKind::Down, x, 100.0));
        t.pointer(&cx, &ev(PointerKind::Up, x, 100.0));
        t.pointer(&cx, &ev(PointerKind::Down, x, 140.0));
        let released = up(&mut t, &cx, &ev(PointerKind::Up, x, 140.0));
        let (bx, by, w, h) = drawn(&released);
        assert!((bx - (x - 20.0)).abs() < 1e-3 && (by - 100.0).abs() < 1e-3 && (w - 40.0).abs() < 1e-3 && (h - 40.0).abs() < 1e-3, "{released:?}");
        assert!(bx + w <= MAX_COORD, "{released:?}");
    }
}
