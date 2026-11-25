//! The Curvature Circle tool draws the circle that touches a path at a point and bends as the path
//! bends there (`shape.curvatureCircle`).
//!
//! The pointer snaps to the nearest path within 5 screen points, or within Smart Guides › Snapping
//! Tolerance when that is larger, and the tool shows the circle that matches the path there: a
//! dashed outline (solid once it is more than 50,000 screen points around), its center as a small
//! cross, a line from the center to the point it touches and, with Measurement Labels on, its
//! radius by the pointer, with an asterisk when the segment is the usual cubic of a circular arc
//! of 90° or less and the circle has the arc's own radius. A click draws it, in one undo step, and a
//! press dragged 2 screen points or more, or canceled with Escape, draws nothing. The circle goes on
//! the side the path bends toward; with Shift held at the press, on the other side. On a straight
//! stretch, at a cusp, at a corner, or where the circle would reach past the canvas, the pointer
//! shows "Straight segment", "Cusp", "Corner" or "Circle too large" instead, and a click draws
//! nothing. The pointer passes over the circles the tool drew until the selection holds none of
//! them, another tool is chosen, another document becomes active, or an edit, Undo or Redo is
//! made outside the tool.

use std::collections::HashSet;

use serde_json::json;
use vectorcraft_doc::{MAX_COORD, NodeId};
use vectorcraft_geom::curvature::{CurvatureCircle, NoCircle, curvature_circle_on};
use vectorcraft_geom::{Point, Rect, Vec2, shapes};

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

pub const ID: &str = "curvatureCircle";

/// How near (screen points) a path pulls the pointer, unless the Smart Guides' snapping tolerance
/// is larger.
const SNAP_PX: f64 = 5.0;
/// How far (screen points) the pointer travels from a press before the press counts as a drag.
const DRAG_PX: f64 = 2.0;
/// Half the length (screen points) of each arm of the cross at the circle's center.
const CROSS_PX: f64 = 4.0;
/// The longest outline (screen points) the preview draws dashed, about 7,000 dashes.
const DASHED_PX: f64 = 50_000.0;

/// The point of a path the pointer snapped to: the path, subpath, segment and `t` on it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hit {
    id: NodeId,
    subpath: usize,
    segment: usize,
    t: f64,
}

/// What the tool shows at a hit.
enum Found {
    Circle(CurvatureCircle),
    /// The pointer's text, and where the path is.
    Refused(&'static str, Point),
}

#[derive(Default)]
pub struct CurvatureCircleTool {
    /// The point under the pointer, and whether Shift was held.
    hover: Option<(Hit, bool)>,
    /// The press in progress, where the pointer was pressed: what a release draws.
    press: Option<(Hit, bool)>,
    from: Point,
    /// The circles the tool drew since the history last moved outside it or the selection last
    /// held none of them.
    drawn: HashSet<NodeId>,
}

impl CurvatureCircleTool {
    /// After Deselect or a selection made outside the tool, the selection holds none of the
    /// circles the tool drew, and the pointer reaches them again.
    /// Returns whether it forgot any.
    fn forget_unselected(&mut self, cx: &ToolContext) -> bool {
        if self.drawn.is_empty() || cx.selection.objects.iter().any(|id| self.drawn.contains(id)) {
            return false;
        }
        self.drawn.clear();
        true
    }
}

/// The nearest point within reach of `p` on the paths a click can pick, guides and the paths in
/// `skip` left out; of paths at the same distance, a selected one, then the one on top.
fn hit_at(cx: &ToolContext, p: Point, skip: &HashSet<NodeId>) -> Option<Hit> {
    let tol = cx.tol(SNAP_PX).max(cx.snap_tol());
    let mut best: Option<(Hit, f64)> = None;
    for id in crate::draw2::candidate_paths(cx) {
        if skip.contains(&id) {
            continue;
        }
        let Some(pd) = cx.doc.node(id).filter(|n| !n.is_guide()).and_then(|n| n.path_data()) else { continue };
        // A path whose control points' box lies out of reach holds no point within reach.
        if pd.control_bounds().is_none_or(|b| !b.inflate(tol, tol).contains(p)) {
            continue;
        }
        if let Some((subpath, segment, t, _, d)) = pd.nearest(p)
            && d <= tol
            && best.is_none_or(|(_, b)| d < b)
        {
            best = Some((Hit { id, subpath, segment, t }, d));
        }
    }
    best.map(|(hit, _)| hit)
}

/// The circle at `hit` as `shape.curvatureCircle` draws it, or why it draws none; `None` when the
/// path or segment is gone.
fn found(cx: &ToolContext, hit: Hit, convex: bool) -> Option<Found> {
    let sp = cx.doc.node(hit.id)?.path_data()?.subpaths.get(hit.subpath)?;
    if hit.segment >= sp.segment_count() {
        return None;
    }
    let at = vectorcraft_geom::ParamCurve::eval(&sp.segment(hit.segment), hit.t);
    Some(match curvature_circle_on(sp, hit.segment, hit.t, convex) {
        Ok(c) if on_canvas(&c) => Found::Circle(c),
        Ok(_) => Found::Refused("Circle too large", at),
        Err(NoCircle::Straight) => Found::Refused("Straight segment", at),
        Err(NoCircle::Cusp) => Found::Refused("Cusp", at),
        Err(NoCircle::Corner) => Found::Refused("Corner", at),
    })
}

/// Does the circle stay on the canvas?
fn on_canvas(c: &CurvatureCircle) -> bool {
    let (p, r) = (c.center, c.radius);
    [p.x - r, p.y - r, p.x + r, p.y + r].iter().all(|v| v.abs() <= MAX_COORD)
}

impl Tool for CurvatureCircleTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        if !ev.pos.is_finite() {
            return vec![];
        }
        self.forget_unselected(cx);
        match ev.kind {
            PointerKind::Move => {
                self.hover = hit_at(cx, ev.pos, &self.drawn).map(|h| (h, ev.mods.shift));
                vec![]
            }
            PointerKind::Down => {
                self.press = hit_at(cx, ev.pos, &self.drawn).map(|h| (h, ev.mods.shift));
                self.hover = self.press;
                self.from = ev.pos;
                vec![]
            }
            // A drag is not a click: the release draws nothing.
            PointerKind::Drag => {
                if ev.pos.distance(self.from) >= cx.tol(DRAG_PX) {
                    self.press = None;
                    self.hover = None;
                }
                vec![]
            }
            PointerKind::Up => {
                let press = self.press.take();
                self.hover = hit_at(cx, ev.pos, &self.drawn).map(|h| (h, ev.mods.shift));
                let Some((hit, convex)) = press else { return vec![] };
                if ev.pos.distance(self.from) >= cx.tol(DRAG_PX) || !matches!(found(cx, hit, convex), Some(Found::Circle(_))) {
                    return vec![];
                }
                let p = json!({"id": hit.id.0, "subpath": hit.subpath, "segment": hit.segment, "t": hit.t, "convex": convex});
                vec![Action::Exec("shape.curvatureCircle".into(), p), Action::Notify("created".into())]
            }
            PointerKind::DoubleClick | PointerKind::TripleClick => vec![],
        }
    }

    /// Escape during a press cancels it: the release draws nothing.
    fn key(&mut self, _cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if key == ToolKey::Escape && self.press.take().is_some() {
            self.hover = None;
        }
        vec![]
    }

    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        key == ToolKey::Escape && self.press.is_some()
    }

    /// A dropped press (a second finger, a canceled touch) draws nothing.
    fn drop_press(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.press = None;
        vec![]
    }

    /// The circle shown depends on the path under the pointer, which an edit, an undo or a zoom
    /// (the reach is in screen points) changes.
    fn wants_rehover(&self) -> bool {
        true
    }

    /// The command selects the circle it drew; the tool adds it to the circles the pointer passes
    /// over.
    fn notify(&mut self, cx: &ToolContext, what: &str) {
        if what == "created"
            && let [id] = cx.selection.objects[..]
        {
            self.drawn.insert(id);
        }
    }

    /// A command from outside the tool (Deselect, a pick in the Layers panel) may leave none of the
    /// circles the tool drew selected. The circle shown under the pointer then waits for the next
    /// hover.
    fn after_command(&mut self, cx: &ToolContext) -> Vec<Action> {
        if self.forget_unselected(cx) {
            self.hover = None;
        }
        vec![]
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.hover = None;
        self.press = None;
        self.drawn.clear();
        vec![]
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        if self.press.is_none() {
            self.hover = None;
        }
    }

    /// An edit, Undo or Redo made outside the tool, or another document, may move or remove the
    /// path under the pointer, and the pointer reaches the circles the tool drew again.
    fn history_moved(&mut self) {
        self.hover = None;
        self.press = None;
        self.drawn.clear();
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some((hit, convex)) = self.hover else { return vec![] };
        let color = crate::draw2::FEEDBACK;
        let mut o = vec![];
        match found(cx, hit, convex) {
            Some(Found::Circle(c)) => {
                let r = c.radius;
                let path = shapes::ellipse(Rect::new(c.center.x - r, c.center.y - r, c.center.x + r, c.center.y + r)).to_bezpath();
                // The outline is drawn solid when it is longer than DASHED_PX on screen.
                let dashed = std::f64::consts::TAU * r <= cx.tol(DASHED_PX);
                o.push(Overlay::Path { path, color, width: 1.0, dashed });
                o.push(Overlay::Line { a: c.center, b: c.at, color, dashed: false });
                let arm = cx.tol(CROSS_PX);
                o.push(Overlay::Line { a: c.center - Vec2::new(arm, 0.0), b: c.center + Vec2::new(arm, 0.0), color, dashed: false });
                o.push(Overlay::Line { a: c.center - Vec2::new(0.0, arm), b: c.center + Vec2::new(0.0, arm), color, dashed: false });
                o.push(Overlay::Anchor { p: c.at, color, filled: true, size: 6.0 });
                if cx.measurement_labels {
                    let star = if c.nominal { "*" } else { "" };
                    o.push(Overlay::Measure { p: c.at + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: format!("R: {}{star}", cx.len(r)) });
                }
            }
            Some(Found::Refused(text, at)) => o.push(Overlay::Label { p: at, text: text.into(), color }),
            None => {}
        }
        o
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _mods: Mods) -> Cursor {
        Cursor::Crosshair
    }
}

#[cfg(test)]
mod tests;
