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
//! step. A press let go within 2 screen points of where it was pressed is a click, also after a
//! drag. Escape, a press dropped part way, and an edit, Undo or Redo made outside the tool drop
//! the first point. A second press within 3 screen points
//! of the first point, or one that snaps to it, is a press on it: let go there (a double-click) it
//! places nothing, and dragged out it draws the circle on the diameter it drags out. Where no
//! circle passes through the points (two are the same, three lie on one line) or the circle would
//! reach past the canvas, the tool shows none and the release draws nothing; for three different
//! points on one line, or a circle past the canvas, the pointer shows "Circle too large".
//!
//! The tool's options (Circle Tool Options, `tool.setOption`; kept with the preferences):
//! `center`, `radii` and `diameter` show or hide the center cross, the radius lines and the
//! diameter label; `color` is the color of the cross and the lines (one of [`COLORS`]);
//! `standardAnchors` off puts the circle's anchors at the points it was drawn through instead of
//! at 0°, 90°, 180° and 270° (`anchors: "points"`); `dragThird` off places the second point with
//! a click and draws the circle through three points with a third click, and a click on the second
//! point again (as a double-click there does) draws the circle on the two points. Every option is
//! on by default, and `color` is blue.

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

/// The colors the center cross and the radius lines can be drawn in, by option value. Blue is the
/// feedback color the other drawing tools use.
pub const COLORS: &[(&str, [u8; 3])] = &[
    ("red", [0xe5, 0x2d, 0x27]),
    ("blue", crate::draw2::FEEDBACK),
    ("magenta", [0xd6, 0x2c, 0xd6]),
    ("green", [0x1f, 0xb4, 0x4b]),
    ("black", [0x00, 0x00, 0x00]),
    ("gray", [0x80, 0x80, 0x80]),
];

/// The option keys that are true or false.
const SWITCHES: &[&str] = &["center", "radii", "diameter", "standardAnchors", "dragThird"];

/// The tool's options (see the module comment).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Options {
    center: bool,
    radii: bool,
    diameter: bool,
    /// An entry of [`COLORS`].
    color: usize,
    standard_anchors: bool,
    drag_third: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options { center: true, radii: true, diameter: true, color: 1, standard_anchors: true, drag_third: true }
    }
}

impl Options {
    fn switch(&mut self, key: &str) -> Option<&mut bool> {
        Some(match key {
            "center" => &mut self.center,
            "radii" => &mut self.radii,
            "diameter" => &mut self.diameter,
            "standardAnchors" => &mut self.standard_anchors,
            "dragThird" => &mut self.drag_third,
            _ => return None,
        })
    }
}

/// Is every value in `values` one the tool takes? `tool.setOption` checks them all before setting
/// any; [`Tool::set_option`] skips a value it cannot read.
pub fn check_options(values: &Map<String, Value>) -> Result<(), String> {
    for (key, v) in values {
        if SWITCHES.contains(&key.as_str()) {
            if !v.is_boolean() {
                return Err(format!("`{key}` must be true or false"));
            }
        } else if key == "color" {
            if !v.as_str().is_some_and(|s| COLORS.iter().any(|(name, _)| *name == s)) {
                let names: Vec<&str> = COLORS.iter().map(|(name, _)| *name).collect();
                return Err(format!("`{key}` must be one of {}", names.join(", ")));
            }
        } else {
            return Err(format!("unknown Circle tool option `{key}`"));
        }
    }
    Ok(())
}

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
    /// With `dragThird` off, a drag moves the second point and the release places it.
    Second { at: Point, snapped: Point, from: Point, on_first: bool, dragged: bool },
    /// With `dragThird` off, the press after the second point was placed, at `at` (snapped). Let go
    /// on the second point (`on_second`), it draws the circle on the two points; let go on the
    /// first point it places nothing; otherwise it draws the circle through the three points.
    Third { at: Point, from: Point, on_first: bool, on_second: bool, dragged: bool },
}

#[derive(Default)]
pub struct CircleTool {
    snap: DrawSnap,
    options: Options,
    /// The first point, placed by a click (or by the press of a drag in progress).
    first: Option<Point>,
    /// With `dragThird` off, the second point, placed by a click where it is shown (snapped, with
    /// Alt's rounding of the diameter when Alt was held), and where the click let go: a press near
    /// either is a press on the second point, so the second click of a double-click finds it even
    /// where Alt's rounding moved it away from the pointer.
    second: Option<(Point, Point)>,
    press: Option<Press>,
    /// Where the pointer is, snapped: the point the next press or release places.
    pointer: Option<Point>,
}

/// The command the tool runs for `points`; with `standard_anchors` off, the circle's anchors are
/// at the points.
fn command(points: &[Point], standard_anchors: bool) -> (String, Value) {
    let pairs: Vec<Value> = points.iter().map(|p| json!([p.x, p.y])).collect();
    let params = if standard_anchors { json!({ "points": pairs }) } else { json!({ "points": pairs, "anchors": "points" }) };
    ("shape.circleThroughPoints".into(), params)
}

/// The circle through two points (the ends of a diameter) or three, as `shape.circleThroughPoints`
/// draws it: none where no circle passes through them or it would reach past the canvas.
pub fn circle_of(points: &[Point]) -> Option<(Point, f64)> {
    let on_canvas = |(c, r): &(Point, f64)| [c.x - r, c.y - r, c.x + r, c.y + r].iter().all(|v| v.abs() <= MAX_COORD);
    shapes::circle_through_points(points).filter(on_canvas)
}

/// [`circle_of`], as the tool draws it: with `standard_anchors` off, also none where the path with
/// an anchor at each point ([`shapes::circle_with_anchors_at`]) reaches past the canvas, since its
/// curves can reach slightly outside the circle.
fn drawn_circle(points: &[Point], standard_anchors: bool) -> Option<(Point, f64)> {
    let fits = |(c, r): &(Point, f64)| {
        let b = vectorcraft_geom::Shape::bounding_box(&shapes::circle_with_anchors_at(*c, *r, points).to_bezpath());
        [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD)
    };
    circle_of(points).filter(|c| standard_anchors || fits(c))
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
fn too_large(points: &[Point], standard_anchors: bool) -> bool {
    let distinct = points.iter().enumerate().all(|(i, p)| !points.iter().take(i).any(|q| q == p));
    points.len() >= 2 && distinct && drawn_circle(points, standard_anchors).is_none()
}

impl CircleTool {
    fn clear(&mut self) {
        self.first = None;
        self.second = None;
        self.press = None;
        self.pointer = None;
        self.snap.clear();
    }

    /// How the second point keeps to the first: Shift's 45° steps and the construction guides. A
    /// third point keeps to neither.
    fn leave(&self, cx: &ToolContext, mods: Mods) -> Option<Leave> {
        self.first.filter(|_| self.second.is_none()).map(|a| Leave::segment(cx, a, mods.shift))
    }

    /// The points the circle passes through as the tool shows it now.
    fn points(&self) -> Vec<Point> {
        let mut out: Vec<Point> = self.first.into_iter().collect();
        if let Some((b, _)) = self.second {
            match self.press {
                // Let go where it was pressed, a press on the first point draws nothing and one on
                // the second point draws the circle on the two.
                Some(Press::Third { on_first: true, dragged: false, .. }) => {}
                Some(Press::Third { on_second: true, dragged: false, .. }) => out.push(b),
                _ => {
                    out.push(b);
                    out.extend(self.pointer);
                }
            }
            return out;
        }
        match self.press {
            Some(Press::First { .. }) | None => out.extend(self.pointer.filter(|_| self.first.is_some())),
            // A press on the first point adds no point: a drag from it drags out a diameter.
            Some(Press::Second { on_first: true, dragged, .. }) => out.extend(self.pointer.filter(|_| dragged)),
            Some(Press::Second { at, dragged: false, .. }) => out.push(at),
            // With `dragThird` off, a drag moves the second point.
            Some(Press::Second { at, .. }) if !self.options.drag_third => out.push(at),
            Some(Press::Second { snapped, dragged: true, .. }) => {
                out.push(snapped);
                out.extend(self.pointer);
            }
            Some(Press::Third { .. }) => {}
        }
        out
    }

    /// The second point as the tool shows it: none for a press on the first point.
    fn second(&self) -> Option<Point> {
        if let Some((b, _)) = self.second {
            return Some(b);
        }
        match self.press {
            Some(Press::Second { on_first: false, at, .. }) if !self.options.drag_third => Some(at),
            Some(Press::Second { on_first: false, dragged: false, at, .. }) => Some(at),
            Some(Press::Second { on_first: false, dragged: true, snapped, .. }) => Some(snapped),
            _ => None,
        }
    }

    /// Runs the circle through `points` and starts afresh; where no circle is drawn through them
    /// (the tool showed none), it only starts afresh.
    fn draw(&mut self, points: &[Point]) -> Vec<Action> {
        self.clear();
        if drawn_circle(points, self.options.standard_anchors).is_none() {
            return vec![];
        }
        let (c, v) = command(points, self.options.standard_anchors);
        vec![Action::Exec(c, v)]
    }

    /// With `dragThird` off, the release of a press after the second point was placed: on the
    /// second point it draws the circle on the two points, on the first it places nothing, and
    /// elsewhere it draws the circle through the three, or keeps the points where none passes
    /// through them.
    fn third_up(&mut self, at: Point, on_first: bool, on_second: bool, dragged: bool) -> Vec<Action> {
        let (Some(a), Some((b, _))) = (self.first, self.second) else { return vec![] };
        if on_second && !dragged {
            return self.draw(&[a, b]);
        }
        if (on_first && !dragged) || drawn_circle(&[a, b, at], self.options.standard_anchors).is_none() {
            self.snap.clear();
            return vec![];
        }
        self.draw(&[a, b, at])
    }

    /// Is the press let go at `ev` a drag: is `ev` [`DRAG_PX`] or more from the press? A release
    /// nearer the press is a click, also after a drag that came back to it.
    fn drags(&self, cx: &ToolContext, ev: &PointerEvent) -> bool {
        match self.press {
            Some(Press::First { from, .. } | Press::Second { from, .. } | Press::Third { from, .. }) => ev.pos.distance(from) >= cx.tol(DRAG_PX),
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
        // click, or a release within DRAG_PX of the press (also after a drag), keeps the points of
        // the press.
        if ev.kind == PointerKind::Up {
            if self.drags(cx, ev) {
                self.pointer(cx, &PointerEvent { kind: PointerKind::Drag, ..*ev });
            } else if let Some(press) = &mut self.press {
                let (at, on_first) = match press {
                    Press::First { at, dragged, .. } => {
                        *dragged = false;
                        (*at, false)
                    }
                    Press::Second { at, on_first, dragged, .. } => {
                        *dragged = false;
                        (*at, *on_first)
                    }
                    // With `dragThird` off, a click on the third point's press: the point goes back
                    // to where it was pressed.
                    Press::Third { at, from, dragged, .. } => {
                        *dragged = false;
                        *at = self.snap.press(cx, *from, &[], None);
                        (*at, false)
                    }
                };
                // A click on the first point places nothing: the pointer goes back to that point, so
                // the tool shows no circle.
                self.pointer = if on_first { self.first } else { Some(at) };
                self.snap.clear();
            }
        }
        match ev.kind {
            PointerKind::Move => {
                let leave = self.leave(cx, ev.mods);
                let p = self.snap.hover(cx, ev.pos, &[], leave.as_ref());
                // Alt's rounding of the diameter is for the second point only.
                let rounds = self.first.filter(|_| self.second.is_none());
                self.pointer = Some(rounds.map_or(p, |a| round_diameter(cx, a, p, ev.mods)));
                vec![]
            }
            PointerKind::Down => {
                if let (Some(a), Some((b, clicked))) = (self.first, self.second) {
                    let at = self.snap.press(cx, ev.pos, &[], None);
                    let reach = cx.tol(REACH_PX);
                    let to_second = ev.pos.distance(b).min(ev.pos.distance(clicked));
                    let (mut on_first, mut on_second) = (at == a || ev.pos.distance(a) <= reach, at == b || to_second <= reach);
                    // A press in reach of both points is on the one it snapped to, otherwise on the
                    // nearer one.
                    if on_first && on_second {
                        let first = if (at == a) != (at == b) { at == a } else { ev.pos.distance(a) < to_second };
                        (on_first, on_second) = (first, !first);
                    }
                    self.press = Some(Press::Third { at, from: ev.pos, on_first, on_second, dragged: false });
                    self.pointer = Some(at);
                    return vec![];
                }
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
                let drag_third = self.options.drag_third;
                let first = self.first;
                match &mut self.press {
                    // With `dragThird` off, the second point follows the pointer as it does before
                    // the press.
                    // The point stays where it was pressed until the press is a drag.
                    Some(Press::Second { at, snapped, from, on_first: false, dragged }) if !drag_third => {
                        *dragged |= ev.pos.distance(*from) >= cx.tol(DRAG_PX);
                        if *dragged {
                            let leave = first.map(|a| Leave::segment(cx, a, ev.mods.shift));
                            *snapped = self.snap.drag(cx, ev.pos, leave.as_ref());
                            *at = first.map_or(*snapped, |a| round_diameter(cx, a, *snapped, ev.mods));
                            self.pointer = Some(*at);
                        }
                    }
                    Some(Press::Third { at, from, dragged, .. }) => {
                        *dragged |= ev.pos.distance(*from) >= cx.tol(DRAG_PX);
                        if *dragged {
                            *at = self.snap.drag(cx, ev.pos, None);
                            self.pointer = Some(*at);
                        }
                    }
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
                (Some(Press::Third { at, on_first, on_second, dragged, .. }), ..) => self.third_up(at, on_first, on_second, dragged),
                // With `dragThird` off, the release places the second point; let go on the first
                // point, as a press there is, it places nothing.
                (Some(Press::Second { at, snapped, on_first: false, .. }), Some(a), _) if !self.options.drag_third => {
                    if !(at == a || snapped == a || ev.pos.distance(a) <= cx.tol(REACH_PX)) {
                        self.second = Some((at, ev.pos));
                    }
                    self.snap.clear();
                    vec![]
                }
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
        if let Some((c, r)) = drawn_circle(&points, self.options.standard_anchors) {
            // The path the release draws: with `standardAnchors` off, the one with an anchor at each point.
            let path = if self.options.standard_anchors {
                shapes::ellipse(Rect::new(c.x - r, c.y - r, c.x + r, c.y + r)).to_bezpath()
            } else {
                shapes::circle_with_anchors_at(c, r, &points).to_bezpath()
            };
            // The canvas draws a dashed outline dash by dash along its whole length: a circle longer
            // than DASHED_PX on screen is drawn solid instead.
            let dashed = std::f64::consts::TAU * r <= cx.tol(DASHED_PX);
            o.push(Overlay::Path { path, color, width: 1.0, dashed });
            let opts = self.options;
            let lines = COLORS.get(opts.color).map_or(color, |(_, rgb)| *rgb);
            if opts.radii {
                for p in &points {
                    o.push(Overlay::Line { a: c, b: *p, color: lines, dashed: false });
                }
            }
            if opts.center {
                let arm = cx.tol(CROSS_PX);
                o.push(Overlay::Line { a: c - Vec2::new(arm, 0.0), b: c + Vec2::new(arm, 0.0), color: lines, dashed: false });
                o.push(Overlay::Line { a: c - Vec2::new(0.0, arm), b: c + Vec2::new(0.0, arm), color: lines, dashed: false });
            }
            if let Some(p) = self.pointer.filter(|_| cx.measurement_labels && opts.diameter) {
                o.push(Overlay::Measure { p: p + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: format!("D: {}", cx.len(2.0 * r)) });
            }
        } else if let Some(p) = self.pointer.filter(|_| too_large(&points, self.options.standard_anchors)) {
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

    fn options(&self) -> Value {
        let o = self.options;
        let color = COLORS.get(o.color).map_or("blue", |(name, _)| *name);
        json!({
            "center": o.center,
            "radii": o.radii,
            "diameter": o.diameter,
            "color": color,
            "standardAnchors": o.standard_anchors,
            "dragThird": o.drag_third,
        })
    }

    /// Takes the values [`check_options`] takes.
    fn check_option(&self, key: &str, value: &Value) -> Result<(), String> {
        check_options(&Map::from_iter([(key.to_string(), value.clone())]))
    }

    /// Sets one option; a value the option does not take leaves it as it was. Switching
    /// `dragThird` drops the points placed so far.
    fn set_option(&mut self, key: &str, value: &Value) {
        if key == "color" {
            if let Some(i) = value.as_str().and_then(|s| COLORS.iter().position(|(name, _)| *name == s)) {
                self.options.color = i;
            }
            return;
        }
        let Some(on) = value.as_bool() else { return };
        let was = self.options.drag_third;
        if let Some(switch) = self.options.switch(key) {
            *switch = on;
        }
        if self.options.drag_third != was {
            self.clear();
        }
    }
}

#[cfg(test)]
mod tests;
