//! The Circle tool draws a circle through points it is given (`shape.circleThroughPoints`).
//!
//! Two points are the ends of a diameter: a drag from one to the other, or a click on each. A click
//! on a first point, then a press on a second point dragged to a third, draws the circle through
//! the three. Shift keeps the second point at 45° steps from the first (from General › Constrain
//! Angle), Alt rounds the diameter of a circle drawn on two points to the nearest tick of the
//! rulers at the current zoom, in the General unit, and every point snaps to Smart Guides. Until
//! the release, the tool shows the circle as a dashed outline (solid once it is more than 50,000
//! screen points around) with its points, its center as a small cross, a line from the center to
//! each point and, with Measurement Labels on, the diameter by the pointer. The circle is one undo
//! step. Escape, a press dropped part way, and an edit, Undo or Redo made outside the tool drop
//! the first point. A second press within 3 screen points
//! of the first point, or one that snaps to it, is a press on it: let go there (a double-click) it
//! places nothing, and dragged out it draws the circle on the diameter it drags out. Where no
//! circle passes through the points (two are the same, three lie on one line) or the circle would
//! reach past the canvas, the tool shows none and the release draws nothing; for three different
//! points on one line, or a circle past the canvas, the pointer shows "Circle too large".
//!
//! With the option From Center (`fromCenter`, in the Control bar) on, the first point is the
//! circle's center and the outline passes through the second: a drag from the center, or a click
//! on the center and then on a point of the outline, draws the circle whose radius is the distance
//! between them (`shape.circleThroughPoints` with `center`). The second point snaps to anchors and
//! Smart Guides as the others do, so the outline can pass exactly through another object's anchor.
//! Shift keeps the second point at 45° steps from the center, and Alt rounds the diameter to the
//! rulers' ticks. The option in force when the first point is placed applies to that circle.

use serde_json::{Map, Value, json};
use vectorcraft_doc::MAX_COORD;
use vectorcraft_geom::{Point, Rect, Vec2, shapes};

use crate::guides::{DrawSnap, Leave};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

pub const ID: &str = "circle";

/// How far (screen points) the pointer travels from a press before the press counts as a drag.
const DRAG_PX: f64 = 2.0;
/// Half the length (screen points) of each arm of the cross at the circle's center.
const CROSS_PX: f64 = 4.0;
/// How near (screen points) the first point a second click counts as a click on it.
const REACH_PX: f64 = 3.0;
/// The longest outline (screen points) the preview draws dashed, about 7,000 dashes.
const DASHED_PX: f64 = 50_000.0;

/// The press in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Press {
    /// The first point `at`, pressed with the pointer at `from` (before snapping): a release after
    /// a drag draws the circle on that diameter.
    First { at: Point, from: Point, dragged: bool },
    /// The second press, after a click placed the first point, with the pointer at `from` (before
    /// snapping). `snapped` is the point it snapped to, and `at` that point with Alt's rounding of
    /// the diameter. A release without a drag draws the circle on the diameter to `at`; a drag
    /// places the third point and the circle passes through `snapped`. A press within
    /// [`REACH_PX`] of the first point, or one that snaps to it (`on_first`), is a press on it: let
    /// go there it places nothing, and dragged out it draws the circle on the diameter it drags out.
    Second { at: Point, snapped: Point, from: Point, on_first: bool, dragged: bool },
}

#[derive(Default)]
pub struct CircleTool {
    snap: DrawSnap,
    /// The first point, placed by a click (or by the press of a drag in progress).
    first: Option<Point>,
    press: Option<Press>,
    /// Where the pointer is, snapped: the point the next press or release places.
    pointer: Option<Point>,
    /// The option From Center (`fromCenter`): the first point is the center.
    from_center: bool,
    /// Is the circle being drawn drawn from its center (From Center when its first point was
    /// placed)?
    centered: bool,
}

/// The command the tool runs for `points`: with `centered`, the first point is the center and
/// the second is on the outline.
fn command(points: &[Point], centered: bool) -> (String, Value) {
    let pair = |p: &Point| json!([p.x, p.y]);
    match points {
        [c, p] if centered => ("shape.circleThroughPoints".into(), json!({ "center": pair(c), "points": [pair(p)] })),
        _ => ("shape.circleThroughPoints".into(), json!({ "points": points.iter().map(pair).collect::<Vec<_>>() })),
    }
}

/// Is the circle at `c` with radius `r` inside the canvas, with its opposite edges on different
/// numbers (as `shape.circleThroughPoints` requires)?
fn on_canvas(c: Point, r: f64) -> bool {
    [c.x - r, c.y - r, c.x + r, c.y + r].iter().all(|v| v.abs() <= MAX_COORD) && c.x - r < c.x + r && c.y - r < c.y + r
}

/// The circle through two points (the ends of a diameter) or three, as `shape.circleThroughPoints`
/// draws it: none where no circle passes through them or it would reach past the canvas.
pub fn circle_of(points: &[Point]) -> Option<(Point, f64)> {
    shapes::circle_through_points(points).filter(|(c, r)| on_canvas(*c, *r))
}

/// The circle `points` give: with `centered`, the circle at the first point whose outline passes
/// through the second, as `shape.circleThroughPoints` with `center` draws it (none for a second
/// point on the center or a circle past the canvas); otherwise [`circle_of`].
fn circle_for(points: &[Point], centered: bool) -> Option<(Point, f64)> {
    match *points {
        [c, p] if centered => {
            let r = c.distance(p);
            (c.is_finite() && r.is_finite() && r > 0.0 && on_canvas(c, r)).then_some((c, r))
        }
        _ if centered => None,
        _ => circle_of(points),
    }
}

/// With Alt, `p` moved along the line from `first` so that the diameter (their distance, or twice
/// it from a center) is on the nearest ruler tick at the current zoom, in the General unit; `p`
/// itself without Alt, and when that rounding leaves no distance.
fn round_diameter(cx: &ToolContext, first: Point, p: Point, mods: Mods, centered: bool) -> Point {
    let per = if centered { 2.0 } else { 1.0 };
    let d = first.distance(p) * per;
    let rounded = cx.unit.snap_to_ruler_tick(d, cx.zoom);
    if mods.alt && d > 0.0 && rounded > 0.0 && rounded.is_finite() { first + (p - first) * (rounded / d) } else { p }
}

/// Are the points two or more different points through which no circle is drawn: three on one
/// line, or a circle that would reach past the canvas?
fn too_large(points: &[Point], centered: bool) -> bool {
    let distinct = points.iter().enumerate().all(|(i, p)| !points.iter().take(i).any(|q| q == p));
    points.len() >= 2 && distinct && circle_for(points, centered).is_none()
}

/// Checks the options `tool.setOption` sets on the Circle tool: `fromCenter` is true or false, and
/// no other key is known.
pub fn check_options(values: &Map<String, Value>) -> Result<(), String> {
    for (key, v) in values {
        match key.as_str() {
            "fromCenter" if v.is_boolean() => {}
            "fromCenter" => return Err(format!("`{key}` must be true or false")),
            _ => return Err(format!("unknown Circle tool option `{key}`")),
        }
    }
    Ok(())
}

impl CircleTool {
    fn clear(&mut self) {
        self.first = None;
        self.press = None;
        self.pointer = None;
        self.centered = false;
        self.snap.clear();
    }

    /// [`round_diameter`] for the circle being drawn.
    fn round(&self, cx: &ToolContext, first: Point, p: Point, mods: Mods) -> Point {
        round_diameter(cx, first, p, mods, self.centered)
    }

    /// How the second point keeps to the first: Shift's 45° steps and the construction guides.
    fn leave(&self, cx: &ToolContext, mods: Mods) -> Option<Leave> {
        self.first.map(|a| Leave::segment(cx, a, mods.shift))
    }

    /// The points the circle passes through as the tool shows it now.
    fn points(&self) -> Vec<Point> {
        let mut out: Vec<Point> = self.first.into_iter().collect();
        match self.press {
            Some(Press::First { .. }) | None => out.extend(self.pointer.filter(|_| self.first.is_some())),
            // A press on the first point adds no point: a drag from it drags out a diameter.
            Some(Press::Second { on_first: true, dragged, .. }) => out.extend(self.pointer.filter(|_| dragged)),
            // From a center, the second point follows the pointer.
            Some(Press::Second { .. }) if self.centered => out.extend(self.pointer),
            Some(Press::Second { at, dragged: false, .. }) => out.push(at),
            Some(Press::Second { snapped, dragged: true, .. }) => {
                out.push(snapped);
                out.extend(self.pointer);
            }
        }
        out
    }

    /// The second point as the tool shows it: none for a press on the first point, nor for a
    /// circle drawn from its center, whose second point follows the pointer.
    fn second(&self) -> Option<Point> {
        if self.centered {
            return None;
        }
        match self.press {
            Some(Press::Second { on_first: false, dragged: false, at, .. }) => Some(at),
            Some(Press::Second { on_first: false, dragged: true, snapped, .. }) => Some(snapped),
            _ => None,
        }
    }

    /// Runs the circle through `points` and starts afresh; where no circle is drawn through them
    /// (the tool showed none), it only starts afresh.
    fn draw(&mut self, points: &[Point]) -> Vec<Action> {
        let centered = self.centered;
        self.clear();
        if circle_for(points, centered).is_none() {
            return vec![];
        }
        let (c, v) = command(points, centered);
        vec![Action::Exec(c, v)]
    }

    /// Is the press a drag at `ev`: dragged already, or `ev` is [`DRAG_PX`] or more from the press?
    fn drags(&self, cx: &ToolContext, ev: &PointerEvent) -> bool {
        match self.press {
            Some(Press::First { from, dragged, .. } | Press::Second { from, dragged, .. }) => dragged || ev.pos.distance(from) >= cx.tol(DRAG_PX),
            None => false,
        }
    }
}

impl Tool for CircleTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if !ev.pos.is_finite() {
            return vec![];
        }
        // A drag ends where it is let go: the release moves the points as a drag to it would. A
        // click, or a release within DRAG_PX of the press, keeps the points of the press.
        if ev.kind == PointerKind::Up && self.drags(cx, ev) {
            self.pointer(cx, &PointerEvent { kind: PointerKind::Drag, ..*ev });
        }
        match ev.kind {
            PointerKind::Move => {
                let leave = self.leave(cx, ev.mods);
                let p = self.snap.hover(cx, ev.pos, &[], leave.as_ref());
                self.pointer = Some(self.first.map_or(p, |a| self.round(cx, a, p, ev.mods)));
                vec![]
            }
            PointerKind::Down => {
                match self.first {
                    None => {
                        let at = self.snap.press(cx, ev.pos, &[], None);
                        self.first = Some(at);
                        self.centered = self.from_center;
                        self.press = Some(Press::First { at, from: ev.pos, dragged: false });
                        self.pointer = Some(at);
                    }
                    Some(a) => {
                        let leave = self.leave(cx, ev.mods);
                        let snapped = self.snap.press(cx, ev.pos, &[], leave.as_ref());
                        let at = self.round(cx, a, snapped, ev.mods);
                        let on_first = snapped == a || ev.pos.distance(a) <= cx.tol(REACH_PX);
                        self.press = Some(Press::Second { at, snapped, from: ev.pos, on_first, dragged: false });
                        self.pointer = Some(at);
                    }
                }
                vec![]
            }
            PointerKind::Drag => {
                let centered = self.centered;
                match &mut self.press {
                    Some(Press::First { at, from, dragged }) => {
                        *dragged |= ev.pos.distance(*from) >= cx.tol(DRAG_PX);
                        let leave = Leave::segment(cx, *at, ev.mods.shift);
                        self.pointer = Some(round_diameter(cx, *at, self.snap.drag(cx, ev.pos, Some(&leave)), ev.mods, centered));
                    }
                    // Dragged from the first point, the pointer is the other end of a diameter, as
                    // for a drag from a first press; from a center, the second point is always the
                    // pointer.
                    Some(Press::Second { from, dragged, on_first, .. }) if *on_first || centered => {
                        *dragged |= ev.pos.distance(*from) >= cx.tol(DRAG_PX);
                        let leave = self.first.map(|a| Leave::segment(cx, a, ev.mods.shift));
                        let p = self.snap.drag(cx, ev.pos, leave.as_ref());
                        self.pointer = Some(self.first.map_or(p, |a| round_diameter(cx, a, p, ev.mods, centered)));
                    }
                    Some(Press::Second { from, dragged, .. }) => {
                        *dragged |= ev.pos.distance(*from) >= cx.tol(DRAG_PX);
                        self.pointer = Some(self.snap.drag(cx, ev.pos, None));
                    }
                    None => {}
                }
                vec![]
            }
            PointerKind::Up => match (self.press.take(), self.first, self.pointer) {
                // Once the press has moved DRAG_PX, the release draws the circle the tool shows, also
                // where the pointer came back nearer the press; where it shows none, the release
                // draws nothing and starts afresh.
                (Some(Press::First { at, dragged: true, .. }), _, Some(b))
                    if b.distance(at) >= cx.tol(DRAG_PX) || circle_for(&[at, b], self.centered).is_some() =>
                {
                    self.draw(&[at, b])
                }
                // A click, or a drag back onto the press, places the first point; the guides of the
                // press go.
                (Some(Press::First { .. }), ..) => {
                    self.snap.clear();
                    vec![]
                }
                // The second click of a double-click lands on the first point and places nothing;
                // a drag from there draws the circle on the diameter it drags out.
                (Some(Press::Second { on_first: true, dragged: false, .. }), ..) => vec![],
                (Some(Press::Second { on_first: true, dragged: true, .. }), Some(a), Some(c)) => self.draw(&[a, c]),
                // From a center: the outline passes through the point the tool shows it on, where
                // the drag ends or, within DRAG_PX of the press, where the pointer last was.
                (Some(Press::Second { .. }), Some(a), Some(c)) if self.centered => self.draw(&[a, c]),
                (Some(Press::Second { at, dragged: false, .. }), Some(a), _) => self.draw(&[a, at]),
                // Three points: Alt's rounding of the diameter does not apply.
                (Some(Press::Second { snapped, dragged: true, .. }), Some(a), Some(c)) => self.draw(&[a, snapped, c]),
                _ => vec![],
            },
            PointerKind::DoubleClick | PointerKind::TripleClick => vec![],
        }
    }

    /// `{fromCenter}`.
    fn options(&self) -> Value {
        json!({ "fromCenter": self.from_center })
    }

    /// `fromCenter` (true or false, [`check_options`]); a circle being drawn keeps the mode it
    /// started with.
    fn set_option(&mut self, key: &str, value: &Value) {
        if let ("fromCenter", Some(on)) = (key, value.as_bool()) {
            self.from_center = on;
        }
    }

    /// [`check_options`] for one option.
    fn check_option(&self, key: &str, value: &Value) -> Result<(), String> {
        check_options(&Map::from_iter([(key.to_string(), value.clone())]))
    }

    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape {
            self.clear();
        }
        vec![]
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        key == ToolKey::Escape && self.first.is_some()
    }

    /// A first point placed: Escape drops it.
    fn ends_on_escape(&self, _cx: &ToolContext) -> bool {
        self.first.is_some()
    }

    fn busy(&self) -> bool {
        self.first.is_some()
    }

    /// A dropped press (a second finger, a canceled touch) drops the points placed so far.
    fn drop_press(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.clear();
        vec![]
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.clear();
        vec![]
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        if self.press.is_none() {
            self.pointer = None;
            self.snap.clear();
        }
    }

    /// An edit, Undo or Redo made outside the tool, or another document, drops the first point.
    fn history_moved(&mut self) {
        self.clear();
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        let points = self.points();
        let color = crate::draw2::FEEDBACK;
        if let Some((c, r)) = circle_for(&points, self.centered) {
            let path = shapes::ellipse(Rect::new(c.x - r, c.y - r, c.x + r, c.y + r)).to_bezpath();
            // The canvas draws a dashed outline dash by dash along its whole length: a circle longer
            // than DASHED_PX on screen is drawn solid instead.
            let dashed = std::f64::consts::TAU * r <= cx.tol(DASHED_PX);
            o.push(Overlay::Path { path, color, width: 1.0, dashed });
            // A circle drawn from its center has one radius, to the point on its outline.
            for p in points.iter().filter(|p| **p != c) {
                o.push(Overlay::Line { a: c, b: *p, color, dashed: false });
            }
            let arm = cx.tol(CROSS_PX);
            o.push(Overlay::Line { a: c - Vec2::new(arm, 0.0), b: c + Vec2::new(arm, 0.0), color, dashed: false });
            o.push(Overlay::Line { a: c - Vec2::new(0.0, arm), b: c + Vec2::new(0.0, arm), color, dashed: false });
            if let Some(p) = self.pointer.filter(|_| cx.measurement_labels) {
                o.push(Overlay::Measure { p: p + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: format!("D: {}", cx.len(2.0 * r)) });
            }
        } else if let Some(p) = self.pointer.filter(|_| too_large(&points, self.centered)) {
            o.push(Overlay::Label { p, text: "Circle too large".into(), color });
        }
        for p in self.first.into_iter().chain(self.second()) {
            o.push(Overlay::Anchor { p, color, filled: true, size: 6.0 });
        }
        o
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Crosshair
    }
}

#[cfg(test)]
mod tests;
