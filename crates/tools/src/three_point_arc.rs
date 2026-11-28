//! The Three-Point Arc tool draws an open circular arc between two ends it is given
//! (`shape.threePointArc`).
//!
//! The ends come first: a drag from one to the other, or a click on each. Shift keeps the second
//! end at 45° steps from the first (from General › Constrain Angle). A third point then shapes the
//! arc, placed by a click or by a press dragged to it: the arc passes through it, or, with the
//! `direction` option on, the arc leaves the first end heading toward it (Alt: the arc arrives at
//! the second end along the line from that end to the point, and Shift keeps the line at 45°
//! steps). `direction` is set in the tool's Tool Options and kept with the preferences; `]` (with
//! or without Shift) switches it while the ends are placed. Every point snaps to Smart Guides.
//! With the `join` option on (Tool Options, or J while the ends are placed), an end placed within
//! reach of an end of an open path lands on it, and the arc is joined to that path. With
//! `tangentSnap` on (the default), in direction mode, when the direction line starts at an end of
//! an open path, the third point snaps onto the line along which the arc carries on the path, the
//! same way or turned back, and the tool shows that line dashed.
//! Until the third point is placed, the tool shows the arc as a dashed outline with its ends, its
//! center as a small cross, the direction line in direction mode and, with Measurement Labels on,
//! the radius by the pointer. Where no arc fits (the third point lies on the line through the
//! ends) or the arc would reach past the canvas, the tool shows "Arc too large" by the pointer
//! (nothing while the pointer is on an end) and a click there draws nothing. The arc is one undo
//! step. Escape, a press dropped part way, and an edit, Undo or Redo made outside the tool drop
//! the ends; a second click on the first end (a double-click) places nothing.

use serde_json::{Value, json};
use vectorcraft_doc::{MAX_COORD, NodeKind};
use vectorcraft_geom::shapes::CircularArc;
use vectorcraft_geom::{PathData, Point, SubPath, Vec2};

use crate::guides::{DrawSnap, Leave};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

pub const ID: &str = "threePointArc";

/// How far (screen points) the pointer travels from a press before the press counts as a drag.
const DRAG_PX: f64 = 2.0;
/// Half the length (screen points) of each arm of the cross at the arc's center.
const CROSS_PX: f64 = 4.0;
/// How near (screen points) the first end a second click counts as a click on it.
const REACH_PX: f64 = 3.0;
/// The label the tool shows by the pointer where no arc fits.
pub const TOO_LARGE: &str = "Arc too large";
/// How near (screen points) an end of an open path an end of the arc counts as on it, the Pen's
/// reach for the path ends it joins.
const END_PX: f64 = 5.0;
/// How near (screen points) the line along which the arc carries on an open path the third point
/// snaps onto it.
const TANGENT_PX: f64 = 5.0;

/// The press in progress.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Press {
    /// The first end, placed at `at` by a press at `from` (before snapping): a release after a
    /// drag places the second end there. The drag is measured from `from`.
    Start { at: Point, from: Point, dragged: bool },
    /// The second end, pressed after a click placed the first: the release places it.
    End,
    /// The third point, pressed once both ends are placed: the release draws the arc.
    Third,
}

/// An end of an open path under an end of the arc: where it is, and the direction a path carrying
/// on from it heads in (none for a path of one anchor).
#[derive(Clone, Copy, Debug, PartialEq)]
struct OpenEnd {
    at: Point,
    out: Option<Vec2>,
}

/// The end of an open subpath of an editable path (not a guide) within reach of `p`.
fn open_end(cx: &ToolContext, p: Point) -> Option<OpenEnd> {
    let tol = cx.tol(END_PX).max(cx.point_tol());
    let editable = cx.doc.editable();
    // The nearest end; among ends equally near, as `shape.threePointArc`'s `join` picks among the
    // ends at a point: one in the selection first, then the last in paint order, and of one
    // subpath's two ends its last anchor.
    let mut best: Option<(OpenEnd, f64, bool)> = None;
    cx.doc.walk(|n| {
        let NodeKind::Path { path, guide: false, .. } = &n.kind else { return };
        if !editable(n.id) {
            return;
        }
        // In isolation mode, only art inside the isolated node.
        let chain = cx.doc.ancestry(n.id).unwrap_or_default();
        if cx.isolation.is_some_and(|s| !chain.contains(&s)) {
            return;
        }
        let chosen = chain.iter().any(|id| cx.selection.objects.contains(id));
        for sp in path.subpaths.iter().filter(|sp| !sp.closed) {
            let ends = [false, true].map(|at_first| end_of(sp, at_first));
            let near = match ends {
                [Some(last), Some(first)] => Some(if last.at.distance(p) <= first.at.distance(p) { last } else { first }),
                [last, first] => last.or(first),
            };
            let Some(e) = near else { continue };
            let d = e.at.distance(p);
            if d <= tol && best.is_none_or(|(_, bd, was)| d < bd || (d == bd && (chosen || !was))) {
                best = Some((e, d, chosen));
            }
        }
    });
    best.map(|(e, _, _)| e)
}

/// The end of the open subpath `sp` (its first anchor when `at_first`), with the direction a path
/// carrying on from it heads in.
fn end_of(sp: &SubPath, at_first: bool) -> Option<OpenEnd> {
    let n = sp.anchors.len();
    let (end, next) =
        if at_first { (sp.anchors.first()?, sp.anchors.get(1)) } else { (sp.anchors.last()?, n.checked_sub(2).and_then(|i| sp.anchors.get(i))) };
    // The path reaches the end along the first of its control points that is off the end.
    let mut back = vec![if at_first { end.h_out } else { end.h_in }];
    if let Some(q) = next {
        back.extend([if at_first { q.h_in } else { q.h_out }, q.p]);
    }
    let out = back.into_iter().map(|q| end.p - q).find(|v| v.hypot() > 0.0).map(|v| v / v.hypot());
    Some(OpenEnd { at: end.p, out })
}

pub struct ThreePointArcTool {
    snap: DrawSnap,
    /// The third point gives the arc's direction at an end instead of a point it passes through.
    direction: bool,
    /// An end of the arc on an end of an open path joins the arc to that path.
    join: bool,
    /// In direction mode, the third point snaps to where the arc carries on the open path its
    /// direction line starts at, the same way or turned back.
    tangent_snap: bool,
    start: Option<Point>,
    end: Option<Point>,
    /// The open path ends found under the first and the second end as each was placed.
    open: [Option<OpenEnd>; 2],
    press: Option<Press>,
    /// Where the pointer is, snapped: the point the next press or release places.
    pointer: Option<Point>,
    /// The line the third point snapped to: the end it runs through and its direction.
    tangent: Option<(Point, Vec2)>,
    /// Alt was held at the last pointer event: in direction mode, the direction is at the second end.
    at_end: bool,
    /// Where the pointer was at its last event, pressed or hovering, before snapping, and the keys
    /// held then.
    raw: Option<(Point, Mods)>,
}

impl Default for ThreePointArcTool {
    fn default() -> Self {
        Self {
            snap: DrawSnap::default(),
            direction: false,
            join: false,
            tangent_snap: true,
            start: None,
            end: None,
            open: [None; 2],
            press: None,
            pointer: None,
            tangent: None,
            at_end: false,
            raw: None,
        }
    }
}

/// The arc from `start` to `end` shaped by `third`, as `shape.threePointArc` draws it: through
/// `third`, or with `direction`, heading toward `third` from `start` (or, with `at_end`, arriving
/// at `end` along the line from `end` to `third`). None where no arc fits or the arc would reach
/// past the canvas. With it, the path the command draws.
pub fn arc_of(start: Point, end: Point, third: Point, direction: bool, at_end: bool) -> Option<(CircularArc, PathData)> {
    let (arc, path) = match (direction, at_end) {
        (false, _) => CircularArc::through(start, end, third).map(|arc| (arc, arc.to_path_via(third))),
        (true, false) => CircularArc::leaving(start, third - start, end).map(|arc| (arc, arc.to_path())),
        (true, true) => CircularArc::arriving(start, end, end - third).map(|arc| (arc, arc.to_path())),
    }?;
    // The curve is measured, not its handles, which can reach past the canvas while the curve stays
    // inside, as the document's own range check measures a path.
    let inside = |x: f64, y: f64| x.abs() <= MAX_COORD && y.abs() <= MAX_COORD;
    let on_canvas = path.bounds().is_some_and(|r| inside(r.x0, r.y0) && inside(r.x1, r.y1));
    on_canvas.then_some((arc, path))
}

/// The command the tool runs for the arc from `start` to `end` shaped by `third`, joined to the
/// open paths at its ends with `join`.
fn command(start: Point, end: Point, third: Point, direction: bool, at_end: bool, join: bool) -> (String, Value) {
    let pair = |p: Point| json!([p.x, p.y]);
    let mut params = if direction {
        json!({ "start": pair(start), "end": pair(end), "toward": pair(third), "at": if at_end { "end" } else { "start" } })
    } else {
        json!({ "start": pair(start), "end": pair(end), "through": pair(third) })
    };
    if join {
        params["join"] = json!(true);
    }
    ("shape.threePointArc".into(), params)
}

impl ThreePointArcTool {
    fn clear(&mut self) {
        self.start = None;
        self.end = None;
        self.open = [None; 2];
        self.press = None;
        self.pointer = None;
        self.tangent = None;
        self.snap.clear();
    }

    /// The end `which` (0 the first, 1 the second) placed at `p`: the open path end under it is
    /// noted, and with `join` on the end lands on it.
    fn place(&mut self, cx: &ToolContext, which: usize, p: Point) -> Point {
        let found = open_end(cx, p);
        if let Some(slot) = self.open.get_mut(which) {
            *slot = found;
        }
        match found {
            Some(e) if self.join => e.at,
            _ => p,
        }
    }

    /// The second end let go at `b`, with the first at `a`: placed, unless landing on an open path
    /// end puts it on the first end, which leaves it to another try.
    fn place_end(&mut self, cx: &ToolContext, a: Point, b: Point) {
        let at = self.place(cx, 1, b);
        if at.distance(a) > cx.tol(REACH_PX) {
            self.end = Some(at);
        } else {
            self.open[1] = None;
        }
    }

    /// How the point being placed keeps to the points before it: the second end to the first, and
    /// in direction mode the third point to the end its direction line starts from.
    fn leave(&self, cx: &ToolContext, mods: Mods) -> Option<Leave> {
        match (self.start, self.end) {
            (Some(a), None) => Some(Leave::segment(cx, a, mods.shift)),
            (Some(a), Some(b)) if self.direction => Some(Leave::segment(cx, if mods.alt { b } else { a }, mods.shift)),
            _ => None,
        }
    }

    /// The third point at `q`, snapped: in direction mode with `tangentSnap` on, near the line
    /// along which the arc carries on the open path its direction line starts at, it moves onto
    /// that line. Shift keeps the line at 45° steps instead.
    fn third(&mut self, cx: &ToolContext, q: Point, mods: Mods) -> Point {
        self.tangent = None;
        let (Some(a), Some(b)) = (self.start, self.end) else { return q };
        if !self.direction || !self.tangent_snap || mods.shift {
            return q;
        }
        let (o, end) = if self.at_end { (b, self.open[1]) } else { (a, self.open[0]) };
        // The direction line starts on the path's end itself, so the arc carries the path on.
        let Some(OpenEnd { at, out: Some(t) }) = end.filter(|e| e.at == o) else { return q };
        let k = (q - at).dot(t);
        let foot = at + t * k;
        if k != 0.0 && q.distance(foot) <= cx.tol(TANGENT_PX) {
            self.tangent = Some((at, t));
            foot
        } else {
            q
        }
    }

    /// The pressed pointer moved to `ev`, or Shift or Alt changed: the point being placed follows
    /// it. A change of keys alone does not make the press a drag.
    fn follow(&mut self, cx: &ToolContext, ev: &PointerEvent) {
        let moved = self.raw.is_none_or(|(p, _)| p != ev.pos);
        self.raw = Some((ev.pos, ev.mods));
        match &mut self.press {
            Some(Press::Start { at, from, dragged }) => {
                *dragged |= moved && ev.pos.distance(*from) >= cx.tol(DRAG_PX);
                let leave = Leave::segment(cx, *at, ev.mods.shift);
                self.pointer = Some(self.snap.drag(cx, ev.pos, Some(&leave)));
            }
            Some(Press::End) => {
                let leave = self.leave(cx, ev.mods);
                self.pointer = Some(self.snap.drag(cx, ev.pos, leave.as_ref()));
            }
            Some(Press::Third) => {
                let leave = self.leave(cx, ev.mods);
                let q = self.snap.drag(cx, ev.pos, leave.as_ref());
                self.pointer = Some(self.third(cx, q, ev.mods));
            }
            None => {}
        }
    }

    /// The arc as the tool shows it now, and its path: both ends placed and the pointer on a point
    /// that fits.
    fn arc(&self) -> Option<(CircularArc, PathData)> {
        arc_of(self.start?, self.end?, self.pointer?, self.direction, self.at_end)
    }
}

impl Tool for ThreePointArcTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if !ev.pos.is_finite() {
            return vec![];
        }
        if self.end.is_some() {
            self.at_end = ev.mods.alt;
        }
        match ev.kind {
            PointerKind::Move => {
                self.raw = Some((ev.pos, ev.mods));
                let leave = self.leave(cx, ev.mods);
                let q = self.snap.hover(cx, ev.pos, &[], leave.as_ref());
                self.pointer = Some(self.third(cx, q, ev.mods));
                vec![]
            }
            PointerKind::Down => {
                self.raw = Some((ev.pos, ev.mods));
                let leave = self.leave(cx, ev.mods);
                match (self.start, self.end) {
                    (None, _) => {
                        let snapped = self.snap.press(cx, ev.pos, &[], None);
                        let at = self.place(cx, 0, snapped);
                        self.start = Some(at);
                        self.press = Some(Press::Start { at, from: ev.pos, dragged: false });
                        self.pointer = Some(at);
                    }
                    (Some(a), None) => {
                        let at = self.snap.press(cx, ev.pos, &[], leave.as_ref());
                        // The second click of a double-click lands on the first end.
                        if at.distance(a) <= cx.tol(REACH_PX) {
                            self.press = None;
                            return vec![];
                        }
                        self.press = Some(Press::End);
                        self.pointer = Some(at);
                    }
                    (Some(_), Some(_)) => {
                        let q = self.snap.press(cx, ev.pos, &[], leave.as_ref());
                        self.pointer = Some(self.third(cx, q, ev.mods));
                        self.press = Some(Press::Third);
                    }
                }
                vec![]
            }
            PointerKind::Drag => {
                self.follow(cx, ev);
                vec![]
            }
            PointerKind::Up => {
                // A release away from the last drag event places the point where it is let go, and
                // a release with Shift or Alt changed places it as those keys keep it.
                let same = self.raw.is_some_and(|(p, m)| p == ev.pos && (m.shift, m.alt) == (ev.mods.shift, ev.mods.alt));
                if self.press.is_some() && !same {
                    self.follow(cx, ev);
                }
                match (self.press.take(), self.start, self.pointer) {
                    (Some(Press::Start { at, dragged: true, .. }), _, Some(b)) if b.distance(at) >= cx.tol(DRAG_PX) => self.place_end(cx, at, b),
                    (Some(Press::End), Some(a), Some(b)) if b.distance(a) > cx.tol(REACH_PX) => self.place_end(cx, a, b),
                    (Some(Press::Third), Some(a), Some(c)) => {
                        let Some(b) = self.end else { return vec![] };
                        // Where no arc fits, nothing is drawn and the ends stay for another try.
                        if self.arc().is_none() {
                            self.snap.clear();
                            return vec![];
                        }
                        let (name, params) = command(a, b, c, self.direction, self.at_end, self.join);
                        self.clear();
                        return vec![Action::Exec(name, params)];
                    }
                    _ => {}
                }
                // A placed end: the guides of the press go.
                self.snap.clear();
                vec![]
            }
            PointerKind::DoubleClick | PointerKind::TripleClick => vec![],
        }
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            ToolKey::Escape => self.clear(),
            ToolKey::BracketRight if self.start.is_some() => {
                self.direction = !self.direction;
                // A press in progress: the point being placed keeps to the new mode.
                // A hovering pointer: the third point it shows keeps to the new mode too.
                match (self.press, self.raw) {
                    (Some(_), Some((p, mods))) => self.follow(cx, &PointerEvent::new(PointerKind::Drag, p.x, p.y).with_mods(mods)),
                    (None, Some((p, mods))) if self.end.is_some() && self.pointer.is_some() => {
                        let leave = self.leave(cx, mods);
                        let q = self.snap.hover(cx, p, &[], leave.as_ref());
                        self.pointer = Some(self.third(cx, q, mods));
                    }
                    _ => {}
                }
            }
            ToolKey::Letter('j') if self.start.is_some() => {
                self.join = !self.join;
                // Switched on: the ends placed on open path ends land on them, unless that would
                // put one end on the other.
                if self.join {
                    let reach = cx.tol(REACH_PX);
                    if let Some(e) = self.open[0]
                        && self.end.is_none_or(|b| b.distance(e.at) > reach)
                    {
                        self.start = Some(e.at);
                        if let Some(Press::Start { at, .. }) = &mut self.press {
                            *at = e.at;
                        }
                    }
                    if let (Some(a), Some(_), Some(e)) = (self.start, self.end, self.open[1])
                        && a.distance(e.at) > reach
                    {
                        self.end = Some(e.at);
                    }
                }
            }
            _ => {}
        }
        vec![]
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        matches!(key, ToolKey::Escape | ToolKey::BracketRight | ToolKey::Letter('j')) && self.start.is_some()
    }

    /// An end placed: Escape drops it.
    fn ends_on_escape(&self, _cx: &ToolContext) -> bool {
        self.start.is_some()
    }

    fn busy(&self) -> bool {
        self.start.is_some()
    }

    /// A dropped press (a second finger, a canceled touch) drops the ends placed so far.
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
            self.tangent = None;
            self.snap.clear();
        }
    }

    /// An edit, Undo or Redo made outside the tool, or another document, drops the ends.
    fn history_moved(&mut self) {
        self.clear();
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        let color = crate::draw2::FEEDBACK;
        match (self.start, self.end, self.pointer) {
            // Placing the second end: the chord so far.
            (Some(a), None, Some(b)) if b != a => o.push(Overlay::Line { a, b, color, dashed: true }),
            (Some(a), Some(b), Some(p)) => {
                if self.direction {
                    o.push(Overlay::Line { a: if self.at_end { b } else { a }, b: p, color, dashed: false });
                }
                // The line the third point snapped to, as far past its end as the pointer is.
                if let Some((at, t)) = self.tangent {
                    let reach = t * p.distance(at);
                    o.push(Overlay::Line { a: at - reach, b: at + reach, color, dashed: true });
                }
                match self.arc() {
                    Some((arc, path)) => {
                        o.push(Overlay::Path { path: path.to_bezpath(), color, width: 1.0, dashed: true });
                        let (c, arm) = (arc.center, cx.tol(CROSS_PX));
                        o.push(Overlay::Line { a: c - Vec2::new(arm, 0.0), b: c + Vec2::new(arm, 0.0), color, dashed: false });
                        o.push(Overlay::Line { a: c - Vec2::new(0.0, arm), b: c + Vec2::new(0.0, arm), color, dashed: false });
                        if cx.measurement_labels {
                            o.push(Overlay::Measure { p, text: format!("R: {}", cx.len(arc.radius)) });
                        }
                    }
                    // A pointer on an end gives no third point yet, so no label.
                    None if p.distance(a) <= cx.tol(REACH_PX) || p.distance(b) <= cx.tol(REACH_PX) => {}
                    None => o.push(Overlay::Label { p, text: TOO_LARGE.into(), color }),
                }
            }
            _ => {}
        }
        for p in self.start.iter().chain(self.end.iter()) {
            o.push(Overlay::Anchor { p: *p, color, filled: true, size: 6.0 });
        }
        o
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Crosshair
    }

    fn options(&self) -> Value {
        json!({ "direction": self.direction, "join": self.join, "tangentSnap": self.tangent_snap })
    }

    fn set_option(&mut self, key: &str, value: &Value) {
        let Some(on) = value.as_bool() else { return };
        match key {
            "direction" => self.direction = on,
            "join" => self.join = on,
            "tangentSnap" => self.tangent_snap = on,
            _ => {}
        }
    }

    fn check_option(&self, key: &str, v: &Value) -> Result<(), String> {
        use crate::option_check as c;
        match key {
            "direction" | "join" | "tangentSnap" => c::boolean(key, v),
            _ => Err(c::unknown(key)),
        }
    }
}

#[cfg(test)]
mod tests;
