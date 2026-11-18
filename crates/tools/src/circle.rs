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

use serde_json::{Value, json};
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
}

/// The command the tool runs for `points`.
fn command(points: &[Point]) -> (String, Value) {
    let pairs: Vec<Value> = points.iter().map(|p| json!([p.x, p.y])).collect();
    ("shape.circleThroughPoints".into(), json!({ "points": pairs }))
}

/// The circle through two points (the ends of a diameter) or three, as `shape.circleThroughPoints`
/// draws it: none where no circle passes through them or it would reach past the canvas.
pub fn circle_of(points: &[Point]) -> Option<(Point, f64)> {
    let on_canvas = |(c, r): &(Point, f64)| [c.x - r, c.y - r, c.x + r, c.y + r].iter().all(|v| v.abs() <= MAX_COORD);
    shapes::circle_through_points(points).filter(on_canvas)
}

/// With Alt, `p` moved along the line from `first` so that their distance (the diameter) is on
/// the nearest ruler tick at the current zoom, in the General unit; `p` itself without Alt, and
/// when that rounding leaves no distance.
fn round_diameter(cx: &ToolContext, first: Point, p: Point, mods: Mods) -> Point {
    let d = first.distance(p);
    let rounded = cx.unit.snap_to_ruler_tick(d, cx.zoom);
    if mods.alt && d > 0.0 && rounded > 0.0 && rounded.is_finite() { first + (p - first) * (rounded / d) } else { p }
}

/// Are the points two or more different points through which no circle is drawn: three on one
/// line, or a circle that would reach past the canvas?
fn too_large(points: &[Point]) -> bool {
    let distinct = points.iter().enumerate().all(|(i, p)| !points.iter().take(i).any(|q| q == p));
    points.len() >= 2 && distinct && circle_of(points).is_none()
}

impl CircleTool {
    fn clear(&mut self) {
        self.first = None;
        self.press = None;
        self.pointer = None;
        self.snap.clear();
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
            Some(Press::Second { at, dragged: false, .. }) => out.push(at),
            Some(Press::Second { snapped, dragged: true, .. }) => {
                out.push(snapped);
                out.extend(self.pointer);
            }
        }
        out
    }

    /// The second point as the tool shows it: none for a press on the first point.
    fn second(&self) -> Option<Point> {
        match self.press {
            Some(Press::Second { on_first: false, dragged: false, at, .. }) => Some(at),
            Some(Press::Second { on_first: false, dragged: true, snapped, .. }) => Some(snapped),
            _ => None,
        }
    }

    /// Runs the circle through `points` and starts afresh; where no circle is drawn through them
    /// (the tool showed none), it only starts afresh.
    fn draw(&mut self, points: &[Point]) -> Vec<Action> {
        self.clear();
        if circle_of(points).is_none() {
            return vec![];
        }
        let (c, v) = command(points);
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
                self.pointer = Some(self.first.map_or(p, |a| round_diameter(cx, a, p, ev.mods)));
                vec![]
            }
            PointerKind::Down => {
                match self.first {
                    None => {
                        let at = self.snap.press(cx, ev.pos, &[], None);
                        self.first = Some(at);
                        self.press = Some(Press::First { at, from: ev.pos, dragged: false });
                        self.pointer = Some(at);
                    }
                    Some(a) => {
                        let leave = self.leave(cx, ev.mods);
                        let snapped = self.snap.press(cx, ev.pos, &[], leave.as_ref());
                        let at = round_diameter(cx, a, snapped, ev.mods);
                        let on_first = snapped == a || ev.pos.distance(a) <= cx.tol(REACH_PX);
                        self.press = Some(Press::Second { at, snapped, from: ev.pos, on_first, dragged: false });
                        self.pointer = Some(at);
                    }
                }
                vec![]
            }
            PointerKind::Drag => {
                match &mut self.press {
                    Some(Press::First { at, from, dragged }) => {
                        *dragged |= ev.pos.distance(*from) >= cx.tol(DRAG_PX);
                        let leave = Leave::segment(cx, *at, ev.mods.shift);
                        self.pointer = Some(round_diameter(cx, *at, self.snap.drag(cx, ev.pos, Some(&leave)), ev.mods));
                    }
                    // Dragged from the first point, the pointer is the other end of a diameter, as
                    // for a drag from a first press.
                    Some(Press::Second { from, on_first: true, dragged, .. }) => {
                        *dragged |= ev.pos.distance(*from) >= cx.tol(DRAG_PX);
                        let leave = self.first.map(|a| Leave::segment(cx, a, ev.mods.shift));
                        let p = self.snap.drag(cx, ev.pos, leave.as_ref());
                        self.pointer = Some(self.first.map_or(p, |a| round_diameter(cx, a, p, ev.mods)));
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
                (Some(Press::First { at, dragged: true, .. }), _, Some(b)) if b.distance(at) >= cx.tol(DRAG_PX) => self.draw(&[at, b]),
                // A click places the first point; the guides of the press go.
                (Some(Press::First { .. }), ..) => {
                    self.snap.clear();
                    vec![]
                }
                // The second click of a double-click lands on the first point and places nothing;
                // a drag from there draws the circle on the diameter it drags out.
                (Some(Press::Second { on_first: true, dragged: false, .. }), ..) => vec![],
                (Some(Press::Second { on_first: true, dragged: true, .. }), Some(a), Some(c)) => self.draw(&[a, c]),
                (Some(Press::Second { at, dragged: false, .. }), Some(a), _) => self.draw(&[a, at]),
                // Three points: Alt's rounding of the diameter does not apply.
                (Some(Press::Second { snapped, dragged: true, .. }), Some(a), Some(c)) => self.draw(&[a, snapped, c]),
                _ => vec![],
            },
            PointerKind::DoubleClick | PointerKind::TripleClick => vec![],
        }
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
        if let Some((c, r)) = circle_of(&points) {
            let path = shapes::ellipse(Rect::new(c.x - r, c.y - r, c.x + r, c.y + r)).to_bezpath();
            // The canvas draws a dashed outline dash by dash along its whole length: a circle longer
            // than DASHED_PX on screen is drawn solid instead.
            let dashed = std::f64::consts::TAU * r <= cx.tol(DASHED_PX);
            o.push(Overlay::Path { path, color, width: 1.0, dashed });
            for p in &points {
                o.push(Overlay::Line { a: c, b: *p, color, dashed: false });
            }
            let arm = cx.tol(CROSS_PX);
            o.push(Overlay::Line { a: c - Vec2::new(arm, 0.0), b: c + Vec2::new(arm, 0.0), color, dashed: false });
            o.push(Overlay::Line { a: c - Vec2::new(0.0, arm), b: c + Vec2::new(0.0, arm), color, dashed: false });
            if let Some(p) = self.pointer.filter(|_| cx.measurement_labels) {
                o.push(Overlay::Measure { p: p + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: format!("D: {}", cx.len(2.0 * r)) });
            }
        } else if let Some(p) = self.pointer.filter(|_| too_large(&points)) {
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
