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
//! press on the second segment is dropped. Escape, an edit or Undo outside the tool, or another
//! document forgets the first segment.

use kurbo::{CubicBez, Shape};
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
    /// The segments a circle may touch.
    second: Vec<CubicBez>,
    /// Where the press was.
    at: Point,
    /// Whether Shift was held when the click's circle was last looked for.
    shift: bool,
    /// How far the pointer was from the press last time, and the radius the drag set (none until
    /// the pointer moves far enough).
    reach: f64,
    radius: Option<f64>,
}

pub struct TangentCircleTool {
    radius: f64,
    first: Option<Pick>,
    hover: Option<Pick>,
    press: Option<Press>,
    /// Whether the press in progress picked the first segment.
    picking: bool,
    /// The circle the second segment gives, and whether one was looked for.
    found: Option<SegmentCircle>,
    looked: bool,
    pointer: Point,
    /// The circles the last second segment gave (its path, subpath and segment, and the radius
    /// asked for), so that moving along it only picks among them.
    cache: Option<(CacheKey, Vec<SegmentCircle>)>,
}

type CacheKey = (NodeId, usize, usize, Option<u64>);

impl Default for TangentCircleTool {
    fn default() -> Self {
        Self { radius: RADIUS, first: None, hover: None, press: None, picking: false, found: None, looked: false, pointer: Point::ZERO, cache: None }
    }
}

/// The path segment under `p`, of any editable path.
fn pick_at(cx: &ToolContext, p: Point) -> Option<Pick> {
    let (id, subpath, segment, t) = crate::draw2::hit_segment(cx, p, cx.pick_tol())?;
    let sp = cx.doc.node(id)?.path_data()?.subpaths.get(subpath)?;
    if segment >= sp.segment_count() {
        return None;
    }
    let curve = sp.segment(segment);
    Some(Pick { id, subpath, segment, t, p: curve.eval(t), curve })
}

/// The segments a circle may touch when `second` is picked: it and the segments on either side of
/// it, without `first`. None when `second` is the first segment again.
fn second_set(cx: &ToolContext, first: &Pick, second: &Pick) -> Vec<CubicBez> {
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
    indices.into_iter().filter(|k| *k < n && !is_first(*k)).map(|k| sp.segment(k)).collect()
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
/// distance from the origin, or too large, in floating point).
fn draw(c: &SegmentCircle, tol: f64) -> Option<Action> {
    let (at, r) = (c.circle.center, c.circle.radius);
    let points: Vec<Point> = [90.0f64, 210.0, 330.0]
        .iter()
        .map(|deg| {
            let (s, k) = deg.to_radians().sin_cos();
            Point::new(at.x + r * k, at.y + r * s)
        })
        .collect();
    let (back, rb) = vectorcraft_geom::shapes::circle_through(points[0], points[1], points[2])?;
    if !(back.distance(at) <= tol && (rb - r).abs() <= tol) {
        return None;
    }
    let points: Vec<[f64; 2]> = points.iter().map(|p| [p.x, p.y]).collect();
    Some(Action::Exec("shape.circleThroughPoints".into(), json!({ "points": points })))
}

fn segment_path(c: &CubicBez) -> BezPath {
    let mut bp = BezPath::new();
    bp.move_to(c.p0);
    bp.curve_to(c.p1, c.p2, c.p3);
    bp
}

impl TangentCircleTool {
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
    /// when the segment or the radius changes.
    fn circles_on(&mut self, cx: &ToolContext, pick: &Pick, radius: Option<f64>) -> Vec<SegmentCircle> {
        let key = (pick.id, pick.subpath, pick.segment, radius.map(f64::to_bits));
        if let Some((k, found)) = &self.cache
            && *k == key
        {
            return found.clone();
        }
        let Some(first) = &self.first else { return vec![] };
        let found = circles(first, &second_set(cx, first, pick), radius);
        self.cache = Some((key, found.clone()));
        found
    }

    /// Look for the circle the pointer at `p` would give over `hover`, before any press.
    fn look(&mut self, cx: &ToolContext, p: Point, mods: Mods) {
        self.found = None;
        self.looked = false;
        let Some(hover) = self.hover.clone().filter(|_| self.first.is_some()) else { return };
        let found = self.circles_on(cx, &hover, mods.shift.then_some(self.radius));
        self.found = nearest_to(found, p, cx.tol(DRAW_PX));
        self.looked = true;
    }

    /// The pointer at `ev` during a press on the second segment: once it is far enough from the
    /// press, the radius follows it (Cmd/Ctrl: a tenth of its movement; Shift: round steps).
    fn drag_to(&mut self, cx: &ToolContext, ev: &PointerEvent) {
        let radius = self.radius;
        let (Some(first), Some(press)) = (&self.first, &mut self.press) else { return };
        let reach = ev.pos.distance(press.at);
        let gear = if ev.mods.cmd { SLOW } else { 1.0 };
        let r = match press.radius {
            None if reach <= cx.tol(DRAG_PX) => {
                // Shift pressed or let go before the drag starts: the click's circle, with or
                // without the tool's radius.
                if ev.mods.shift != press.shift {
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
        match ev.kind {
            PointerKind::Move => {
                self.hover = pick_at(cx, ev.pos);
                self.look(cx, ev.pos, ev.mods);
                vec![]
            }
            PointerKind::Down => {
                let Some(pick) = pick_at(cx, ev.pos) else { return vec![] };
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
                self.press = Some(Press { second, at: ev.pos, shift: ev.mods.shift, reach: 0.0, radius: None });
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
                if self.press.take().is_none() {
                    return vec![];
                }
                // No circle, or one the three points cannot draw: the first segment stays picked,
                // for another try. It also stays when the command fails: only a drawn circle tells
                // the tool (`notify`) to forget it.
                let Some(action) = self.found.take().and_then(|c| draw(&c, cx.tol(DRAW_PX))) else { return vec![] };
                self.looked = false;
                vec![action, Action::Notify(DRAWN.into())]
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
        if let Some(c) = &self.found {
            let (at, r) = (c.circle.center, c.circle.radius);
            // A tolerance of at least a ten-thousandth of the radius keeps a huge circle to a few
            // dozen segments.
            let tol = cx.tol(0.1).max(r * 1e-4);
            out.push(Overlay::Path { path: kurbo::Circle::new(at, r).to_path(tol), color: RED, width: 1.0, dashed: false });
            let arm = cx.tol(4.0);
            out.push(Overlay::Line { a: at - kurbo::Vec2::new(arm, 0.0), b: at + kurbo::Vec2::new(arm, 0.0), color: RED, dashed: false });
            out.push(Overlay::Line { a: at - kurbo::Vec2::new(0.0, arm), b: at + kurbo::Vec2::new(0.0, arm), color: RED, dashed: false });
            out.push(Overlay::Line { a: at, b: c.first, color: RED, dashed: true });
            out.push(Overlay::Line { a: at, b: c.second, color: RED, dashed: true });
            out.push(Overlay::Measure { p: self.pointer, text: format!("R: {}", cx.len(r)) });
        } else if self.looked {
            // A label, which the canvas shows in the interface language.
            out.push(Overlay::Label { p: self.pointer, text: NO_CIRCLE.into(), color: RED });
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Crosshair
    }

    fn options(&self) -> Value {
        json!({ "radius": self.radius })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        if key == "radius"
            && let Some(r) = v.as_f64().filter(|r| (RADIUS_RANGE.0..=RADIUS_RANGE.1).contains(r))
        {
            self.radius = r;
        }
    }

    fn check_option(&self, key: &str, v: &Value) -> Result<(), String> {
        use crate::option_check as c;
        match key {
            "radius" => c::number(key, v, RADIUS_RANGE.0, RADIUS_RANGE.1),
            _ => Err(c::unknown(key)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::{Appearance, Document, Node, Selection};
    use vectorcraft_geom::{PathData, SubPath};

    /// Two open lines, (100, 100)–(300, 100) and (100, 140)–(300, 140), and an L from (100, 300)
    /// over to (300, 300) and down to (300, 400).
    fn doc() -> Document {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let lines: [&[(f64, f64)]; 3] =
            [&[(100.0, 100.0), (300.0, 100.0)], &[(100.0, 140.0), (300.0, 140.0)], &[(100.0, 300.0), (300.0, 300.0), (300.0, 400.0)]];
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
}
