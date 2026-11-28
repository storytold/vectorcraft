//! The Quick Pen tool (Alt+P, Option+P on macOS) draws paths of straight lines and circular arcs
//! (`path.quickPen`).
//!
//! A click sets the start point, and each click or drag after it adds a piece from the last point
//! to where the pointer is let go, as the tool's action says: a straight line, a continuous arc
//! (it leaves its start in the direction the path arrives in, so the two join smoothly) or a
//! sharp-corner arc (it turns through the arc size, 180° by default, and meets the path before it
//! at a corner). Alt (Option) pressed and let go on its own switches to the next action. C flips
//! the last sharp-corner arc to the other side of its chord, and [ and ] make the arc size 15°
//! smaller or larger, for the last sharp-corner arc and the ones after it; during a drag they
//! change the arc being dragged. A dragged piece ends where the press is let go. A click on the first
//! point closes the path, a click on an end of an open path goes on drawing from it, and Escape
//! or Enter ends the path. The path's points show as circles, without handles, and a preview
//! shows the piece the next click adds. Points snap to Smart Guides. Each piece is one undo step.
//! While no path is being drawn, the tool edits the selected paths ([`edit`]).
//!
//! A handle stands beyond each end of the path being drawn, or, while none is, of the selected
//! path, where that end is a straight line or a circular arc. Dragging it makes the line or arc
//! longer or shorter, as one undo step (`path.quickPenLength`): a line keeps its direction, and
//! an arc keeps its circle. The dragged end stays on its line or circle and does not snap to Smart
//! Guides. A click on a handle that does not drag is an ordinary click: it adds a point, closes the
//! path or starts one.
//!
//! While an arc is drawn, by a drag or as the piece the next click adds, the tool shows a
//! protractor around the arc's center: a tick across its circle every 15° from where the arc
//! starts, a longer one every 90°, and one where it ends, with a small cross at the center. The
//! piece the next click adds shows its protractor only while the Pen tool's rubber band preference
//! shows that piece. With Smart Guides on, the end of a continuous arc goes onto the nearest tick
//! of that protractor when the tick lies within the snapping tolerance of the pointer, no other
//! Smart Guide snaps the point, and neither the grid nor pixels snap drawn points. A sharp-corner
//! arc, and a continuous arc that has no direction to follow, sweep the arc size, so their ends are
//! not snapped to a tick. After a snap setting changes under a still pointer, the hover's preview,
//! protractor, distance label and Smart Guides are hidden until the pointer moves; when the view,
//! the document or the selection changes, the canvas hovers the still pointer again. Shift does
//! not constrain Quick Pen points, so it does not change the snapping.

mod edit;

use std::f64::consts::{FRAC_PI_2, PI};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, MAX_COORD, Node, NodeId, NodeKind};
use vectorcraft_geom::{Anchor, AnchorKind, BezPath, PathData, Point, SubPath, Vec2};

use crate::draw2::FEEDBACK;
use crate::guides::{DrawSnap, Leave, SnapSettings};
use crate::pen::{continue_from, open_end_at};
use crate::{Action, Cursor, Mods, Overlay, PixelOp, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};
use edit::{Edit, Grab};
use length::EndPiece;

/// The tool's id.
pub const ID: &str = "quickPen";
/// The command that draws the path.
pub const COMMAND: &str = "path.quickPen";
/// A sharp-corner arc's turn (degrees) until [ or ] changes it.
pub const DEFAULT_TURN: f64 = 180.0;
/// The degrees [ and ] take from or add to the arc size.
pub const TURN_STEP: f64 = 15.0;
/// The smallest and largest arc size (degrees).
pub const MIN_TURN: f64 = 1.0;
pub const MAX_TURN: f64 = 359.0;
/// The command that makes the line or arc at an end of a path longer or shorter.
pub const LENGTH_COMMAND: &str = "path.quickPenLength";
/// Degrees between the protractor's ticks, and the steps a continuous arc's sweep snaps to.
pub const TICK_STEP: f64 = 15.0;
/// Half the length (screen points) of a protractor tick; the ticks every 90° are twice as long.
const TICK_PX: f64 = 4.0;
/// Half the length (screen points) of each arm of the cross at an arc's center.
const CROSS_PX: f64 = 4.0;
/// Screen points the pointer moves after a press before the press drags.
const DRAG_PX: f64 = 3.0;
/// Screen points a length handle stands beyond the end of the path, along the way it leaves the end.
pub const LENGTH_HANDLE_PX: f64 = 20.0;
/// A continuous arc whose end lies within this angle (radians) of straight ahead, or of straight
/// behind, is drawn as a line: no circle runs there in the path's direction.
const STRAIGHT: f64 = 1e-6;
/// The notice the tool asks for after each of its own edits, to see the path as the document then
/// has it.
const DRAWN: &str = "drawn";
/// The notice after a press's piece is committed.
const PLACED: &str = "placed";

/// What the next click adds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Stroke {
    #[default]
    Line,
    ContinuousArc,
    SharpArc,
}

impl Stroke {
    pub const ALL: [Stroke; 3] = [Stroke::Line, Stroke::ContinuousArc, Stroke::SharpArc];

    /// The name commands and the tool's `action` option use.
    pub fn name(self) -> &'static str {
        match self {
            Stroke::Line => "line",
            Stroke::ContinuousArc => "continuousArc",
            Stroke::SharpArc => "sharpArc",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == s)
    }

    /// The action after this one: Line, Continuous Arc, Sharp Corner Arc, and round again.
    pub fn next(self) -> Self {
        match self {
            Stroke::Line => Stroke::ContinuousArc,
            Stroke::ContinuousArc => Stroke::SharpArc,
            Stroke::SharpArc => Stroke::Line,
        }
    }
}

/// One piece of a Quick Pen path: from the path's last point to `to`. `turn` (degrees) and `flip`
/// shape a sharp-corner arc, and a continuous arc with no direction to follow.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub to: Point,
    pub stroke: Stroke,
    pub turn: f64,
    pub flip: bool,
}

impl Piece {
    pub fn to_json(&self) -> Value {
        let mut v = json!({"x": self.to.x, "y": self.to.y, "action": self.stroke.name()});
        if self.stroke != Stroke::Line {
            v["turn"] = json!(self.turn);
            v["flip"] = json!(self.flip);
        }
        v
    }
}

/// `v` turned by `a` radians (clockwise on the page, whose y runs down).
fn rot(v: Vec2, a: f64) -> Vec2 {
    let (s, c) = a.sin_cos();
    Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c)
}

fn unit(v: Vec2) -> Option<Vec2> {
    let l = v.hypot();
    (l > 1e-12 && l.is_finite()).then(|| v / l)
}

/// The direction an open subpath runs at its last point: along that point's incoming handle, or
/// else from the segment before it. None for a single point, or a path that comes back on itself.
pub fn arrival(sp: &SubPath) -> Option<Vec2> {
    let n = sp.anchors.len();
    let last = sp.anchors.last()?;
    if let Some(d) = unit(last.p - last.h_in) {
        return Some(d);
    }
    let prev = sp.anchors.get(n.checked_sub(2)?)?;
    unit(last.p - prev.h_out).or_else(|| unit(last.p - prev.p))
}

/// The signed turn (radians) of the arc `piece` adds from `a` to `b` on `sp`, or None for a
/// straight segment. Positive turns bulge to the left of the direction drawn.
fn turn_of(sp: &SubPath, piece: &Piece, a: Point, b: Point) -> Option<f64> {
    let sharp = |p: &Piece| p.turn.to_radians() * if p.flip { -1.0 } else { 1.0 };
    match piece.stroke {
        Stroke::Line => None,
        Stroke::SharpArc => Some(sharp(piece)),
        Stroke::ContinuousArc => match arrival(sp) {
            None => Some(sharp(piece)),
            Some(t) => {
                let c = b - a;
                let phi = t.cross(c).atan2(t.dot(c));
                (phi.abs() > STRAIGHT && phi.abs() < PI - STRAIGHT).then_some(2.0 * phi)
            }
        },
    }
}

/// Adds `piece` at the end of the open subpath `sp`. A piece that ends where the subpath does adds
/// nothing (false). An arc is drawn as cubic segments of at most 90° each, its inner points smooth;
/// the new end is a corner without an outgoing handle. The point the piece leaves from is smooth
/// when a continuous arc goes on from a curve into it, and a corner otherwise.
pub fn append(sp: &mut SubPath, piece: &Piece) -> bool {
    let Some(last) = sp.anchors.last().copied() else { return false };
    let (a, b) = (last.p, piece.to);
    let c = b - a;
    let l = c.hypot();
    if l <= 1e-9 || !l.is_finite() {
        return false;
    }
    let Some(s) = turn_of(sp, piece, a, b) else {
        if let Some(end) = sp.anchors.last_mut() {
            end.h_out = end.p;
            end.kind = AnchorKind::Corner;
        }
        sp.anchors.push(Anchor::corner(b));
        return true;
    };
    let continuous = piece.stroke == Stroke::ContinuousArc && arrival(sp).is_some();
    let n = ((s.abs() / FRAC_PI_2) - 1e-9).ceil().max(1.0) as usize;
    let step = s / n as f64;
    let r = l / (2.0 * (s.abs() / 2.0).sin());
    let k = 4.0 / 3.0 * (step.abs() / 4.0).tan() * r;
    let mut t = rot(c / l, -s / 2.0);
    if let Some(end) = sp.anchors.last_mut() {
        end.h_out = a + t * k;
        end.kind = if continuous && end.h_in != end.p { AnchorKind::Smooth } else { AnchorKind::Corner };
    }
    let mut p = a;
    for i in 1..=n {
        let chord = rot(t, step / 2.0) * (2.0 * r * (step.abs() / 2.0).sin());
        let t2 = rot(t, step);
        let q = if i == n { b } else { p + chord };
        let mut an = Anchor::with_handles(q, q - t2 * k, q);
        if i < n {
            an.h_out = q + t2 * k;
            an.kind = AnchorKind::Smooth;
        }
        sp.anchors.push(an);
        (p, t) = (q, t2);
    }
    true
}

/// The circle an arc piece lies on: its center and radius, the point the arc starts from, and the
/// arc's signed sweep (radians; positive bulges to the left of the direction drawn).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArcCircle {
    pub center: Point,
    pub radius: f64,
    pub start: Point,
    pub sweep: f64,
}

/// The circle of the arc `piece` adds at the end of `sp`, as [`append`] draws it. None for a line,
/// a piece that ends where `sp` does, and a circle whose center or radius is past the document's
/// coordinate limit.
pub fn arc_circle(sp: &SubPath, piece: &Piece) -> Option<ArcCircle> {
    let a = sp.anchors.last()?.p;
    let c = piece.to - a;
    let l = c.hypot();
    if l <= 1e-9 || !l.is_finite() {
        return None;
    }
    let s = turn_of(sp, piece, a, piece.to)?;
    let half = s.abs() / 2.0;
    let radius = l / (2.0 * half.sin());
    // The center lies on the chord's perpendicular bisector, on the side away from the bulge for
    // less than a half circle and on the bulge's side for more.
    let center = a + c / 2.0 + rot(c / l, FRAC_PI_2) * (radius * half.cos() * s.signum());
    (radius <= MAX_COORD && center.x.abs() <= MAX_COORD && center.y.abs() <= MAX_COORD).then_some(ArcCircle { center, radius, start: a, sweep: s })
}

/// The protractor tick nearest the end of a continuous arc ending at `piece.to`: the point on the
/// arc's circle ([`arc_circle`]) where the sweep is a whole number of [`TICK_STEP`]s. None for
/// other pieces, a continuous arc with no direction to follow (it sweeps the arc size, as a
/// sharp-corner arc does), an arc without a circle, a sweep that rounds to nothing or to a whole
/// circle, and a tick past the document's coordinate limit.
pub fn tick_end(sp: &SubPath, piece: &Piece) -> Option<Point> {
    if piece.stroke != Stroke::ContinuousArc {
        return None;
    }
    arrival(sp)?;
    let arc = arc_circle(sp, piece)?;
    let step = TICK_STEP.to_radians();
    let s = (arc.sweep / step).round() * step;
    (s.abs() > step / 2.0 && s.abs() < 2.0 * PI - step / 2.0)
        .then(|| arc.center + rot(arc.start - arc.center, s))
        .filter(|b| b.x.abs() <= MAX_COORD && b.y.abs() <= MAX_COORD)
}

/// The protractor around `arc`'s center: a tick across its circle every [`TICK_STEP`] from where
/// the arc starts, twice as long every 90°, and one where the arc ends.
fn protractor(cx: &ToolContext, arc: &ArcCircle) -> BezPath {
    let step = TICK_STEP.to_radians();
    let out = (arc.start - arc.center) / arc.radius;
    let ticks = (arc.sweep.abs() / step + 1e-9).floor() as usize;
    let mut bp = BezPath::new();
    let mut tick = |angle: f64, long: bool| {
        let d = rot(out, angle);
        let half = cx.tol(if long { 2.0 * TICK_PX } else { TICK_PX });
        bp.move_to(arc.center + d * (arc.radius - half));
        bp.line_to(arc.center + d * (arc.radius + half));
    };
    let per_right_angle = (90.0 / TICK_STEP).round() as usize;
    for k in 0..=ticks {
        tick(arc.sweep.signum() * k as f64 * step, k % per_right_angle == 0);
    }
    if arc.sweep.abs() - ticks as f64 * step > 1e-6 {
        tick(arc.sweep, false);
    }
    bp
}

/// The last two of the first `kept` points of `sp`: where the piece after them starts, and the
/// direction it arrives in.
fn tail(sp: &SubPath, kept: usize) -> Option<SubPath> {
    Some(SubPath::new(sp.anchors.get_range(kept.saturating_sub(2)..kept)?, false))
}

/// Closes `sp`: a last point that lies on the first joins it (the first point takes its incoming
/// handle); otherwise a straight segment closes the path. Fewer than two points stay open.
pub fn close(sp: &mut SubPath) {
    let n = sp.anchors.len();
    if n < 2 {
        return;
    }
    if let (Some(first), Some(last)) = (sp.anchors.first().copied(), sp.anchors.last().copied())
        && n >= 3
        && first.p.distance(last.p) <= 1e-9
    {
        sp.anchors.pop();
        if let Some(f) = sp.anchors.first_mut() {
            f.h_in = last.h_in;
        }
    }
    sp.closed = true;
}

/// `base` with `pieces` added after it, closed when `closed`.
pub fn build(base: &SubPath, pieces: &[Piece], closed: bool) -> SubPath {
    let mut sp = base.clone();
    for piece in pieces {
        append(&mut sp, piece);
    }
    if closed {
        close(&mut sp);
    }
    sp
}

/// The path being drawn: subpath `si` of `id` (none until the engine made a new path, which starts
/// at `start`), and the `pieces` the tool added to it, each after the number of points in `kept`
/// the subpath had before it. `seen` is the path object as the document had it after the tool's
/// last edit, pixel alignment and other changes the engine makes after each edit included, and
/// `before_press` as it was when the press under way began. A piece is drawn again on the points
/// the document holds before it, so the points before it stay as they are.
#[derive(Clone, Debug, Default, PartialEq)]
struct Drawing {
    id: Option<NodeId>,
    si: usize,
    start: Option<Point>,
    pieces: Vec<Piece>,
    kept: Vec<usize>,
    seen: Option<Arc<Node>>,
    before_press: Option<Arc<Node>>,
    /// The document's revision ([`ToolContext::revision`]) after the tool's last edit.
    revision: (u64, u64),
}

/// The node `id` as the document shares it, found through the document's index.
fn shared_node(doc: &Document, id: NodeId) -> Option<&Arc<Node>> {
    let path = doc.index_path(id)?;
    let (first, rest) = path.split_first()?;
    let mut cur = doc.layers.get(*first)?;
    for &i in rest {
        cur = cur.children()?.get(i)?;
    }
    Some(cur)
}

/// Is `now` the node `then` was: the same shared node, or one equal to it?
fn same_node(now: &Node, then: &Arc<Node>) -> bool {
    std::ptr::eq(now, Arc::as_ptr(then)) || *now == **then
}

impl Drawing {
    fn push(&mut self, piece: Piece, kept: usize) {
        self.pieces.push(piece);
        self.kept.push(kept);
    }

    fn pop(&mut self) {
        self.pieces.pop();
        self.kept.pop();
    }

    /// The last piece as `sp`, the subpath being drawn, holds it: the points from where it starts to
    /// the end of the path.
    fn last_tail(&self, sp: &SubPath) -> Option<SubPath> {
        let from = self.kept.last()?.checked_sub(1)?;
        Some(SubPath::new(sp.anchors.get_range(from..)?, false))
    }

    /// After the start of the path changed to a different number of points (`delta` more): keeps
    /// the pieces that start at or after point `fixed` (counted before the change), which now start
    /// `delta` points later, and forgets the others.
    fn after_start_change(&mut self, fixed: usize, delta: isize) {
        let first = self.kept.iter().position(|k| k.saturating_sub(1) >= fixed).unwrap_or(self.kept.len());
        self.pieces.drain(..first);
        self.kept.drain(..first);
        for k in &mut self.kept {
            *k = k.saturating_add_signed(delta).max(1);
        }
    }

    /// Forgets every piece: the path as the document holds it is all the drawing goes on from.
    fn clear_pieces(&mut self) {
        self.pieces.clear();
        self.kept.clear();
    }
}

/// The press in progress: where it went down, whether it has dragged, and whether it began an undo
/// step (a piece it adds). `end` is the pointer for the end of the piece it adds and where the
/// snap put that end, before the protractor's tick.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    at: Point,
    dragged: bool,
    begun: bool,
    end: Option<(Point, Point)>,
}

/// A length handle: the line or arc at one end of subpath `si` of `id`, and where its handle is.
#[derive(Clone, Copy, Debug, PartialEq)]
struct LengthHandle {
    id: NodeId,
    si: usize,
    at_start: bool,
    piece: EndPiece,
    at: Point,
}

/// A length handle being dragged: the handle as the press found it, the length the drag has set,
/// whether the press has moved far enough to drag, and whether the drag began an undo step.
#[derive(Clone, Debug, PartialEq)]
struct LengthDrag {
    handle: LengthHandle,
    press: Point,
    length: f64,
    dragged: bool,
    begun: bool,
    /// The last piece of the path being drawn as it was before the drag, when the handle is at the
    /// end of that piece: the drag changes the piece, so C and [ ] still shape it afterwards.
    last: Option<Piece>,
    /// The subpath as the press found it, and its last piece as that subpath held it.
    before: SubPath,
    tail: Option<SubPath>,
    /// The document's revision ([`ToolContext::revision`]) after the drag's last edit. A press
    /// previews from the path as it found it, so once the revision differs, an edit was made
    /// outside the tool and the drag previews no more.
    revision: (u64, u64),
    /// An edit made outside the tool ended the drag.
    stale: bool,
}

pub struct QuickPenTool {
    action: Stroke,
    turn: f64,
    drawing: Option<Drawing>,
    press: Option<Press>,
    /// A press on an anchor or a segment of a selected path while no path is being drawn.
    edit: Option<Edit>,
    length: Option<LengthDrag>,
    hover: Option<Point>,
    snap: DrawSnap,
    /// The pointer while it hovers, whether it lies on a Smart Guide target ([`Self::landed`]), and
    /// the snap settings then; `hover` is where Smart Guides put it. The protractor's tick is
    /// applied when the point is used ([`Self::hover_end`]), for the action chosen then. A press
    /// clears it.
    pointer: Option<(Point, bool, SnapSettings)>,
}

impl Default for QuickPenTool {
    fn default() -> Self {
        Self {
            action: Stroke::Line,
            turn: DEFAULT_TURN,
            drawing: None,
            press: None,
            edit: None,
            length: None,
            hover: None,
            snap: DrawSnap::default(),
            pointer: None,
        }
    }
}

/// Is `id` selected on its own, itself or the compound path it is in?
fn selected_alone(cx: &ToolContext, id: NodeId) -> bool {
    let sel = cx.selection.objects.as_slice();
    sel == [id] || cx.doc.parent_of(id).is_some_and(|p| sel == [p] && cx.doc.node(p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. })))
}

/// Whether `sp` lies on the canvas: every point and handle does, or else the curve itself (its
/// extrema, not its handles) does.
pub fn on_canvas(sp: &SubPath) -> bool {
    let within = |v: f64| v.abs() <= MAX_COORD;
    sp.anchors.iter().all(|a| [a.p, a.h_in, a.h_out].iter().all(|q| within(q.x) && within(q.y)))
        || PathData::new(vec![sp.clone()]).bounds().is_some_and(|b| [b.x0, b.y0, b.x1, b.y1].into_iter().all(within))
}

fn point_tol(cx: &ToolContext) -> f64 {
    cx.tol(5.0).max(cx.point_tol())
}

/// Asks to see the path once the actions before it are applied.
fn drawn() -> Action {
    Action::Notify(DRAWN.into())
}

impl QuickPenTool {
    /// The subpath being drawn, while it is selected, editable, open and its path still exactly as
    /// the tool's last edit left it: an edit made to the path outside the tool (a point moved, its
    /// look changed, the path it went on from changed), also during a press, ends the drawing, so
    /// the next piece does not undo it.
    fn active<'a>(&self, cx: &'a ToolContext) -> Option<(NodeId, &'a SubPath)> {
        let dr = self.drawing.as_ref()?;
        let id = dr.id?;
        if !selected_alone(cx, id) || !cx.doc.is_editable(id) {
            return None;
        }
        let node = cx.doc.node(id)?;
        if !same_node(node, dr.seen.as_ref()?) {
            return None;
        }
        let sp = node.path_data()?.subpaths.get(dr.si)?;
        (!sp.closed).then_some((id, sp))
    }

    /// `path.quickPen`'s params for the path being drawn: its last piece on the points before it,
    /// closed when `closed`.
    fn params(&self, closed: bool) -> Value {
        let Some(dr) = &self.drawing else { return json!({}) };
        let pieces: Vec<Value> = dr.pieces.last().map(Piece::to_json).into_iter().collect();
        let mut v = json!({"pieces": pieces});
        match (dr.id, dr.start) {
            (Some(id), _) => {
                v["id"] = json!(id.0);
                v["subpath"] = json!(dr.si);
                v["keep"] = json!(dr.kept.last().copied().unwrap_or(1).max(1));
            }
            (None, Some(s)) => v["start"] = json!([s.x, s.y]),
            (None, None) => {}
        }
        if closed {
            v["closed"] = json!(true);
        }
        v
    }

    /// The last piece, while it is a sharp-corner arc: the one drawn last while no press is under
    /// way, or the one a press is drawing.
    fn last_sharp_arc(&self, cx: &ToolContext) -> Option<Piece> {
        match self.press {
            Some(pr) if !pr.begun => return None,
            Some(_) => self.held(cx).then_some(())?,
            None => self.active(cx).map(|_| ())?,
        }
        self.drawing.as_ref()?.pieces.last().copied().filter(|p| p.stroke == Stroke::SharpArc)
    }

    /// During a press: is the path being drawn still active, and the document as the tool's last
    /// preview left it? A preview starts again from the document as the press found it, so after
    /// an edit made outside the tool the press previews no more.
    fn held(&self, cx: &ToolContext) -> bool {
        self.active(cx).is_some() && self.drawing.as_ref().is_some_and(|dr| dr.revision == cx.revision)
    }

    /// Moves the end of the piece a press draws to `p` (snapped) and previews it. An edit made
    /// outside the tool since the last preview ends the drawing instead: the press then commits
    /// the document as it is.
    fn drag_to(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        if !self.held(cx) {
            self.drawing = None;
            self.snap.clear();
            return vec![];
        }
        // The snapping leaves from the point before the piece being dragged, as the press found it.
        let before = self.before_piece();
        let from = before.as_ref().and_then(|sp| sp.anchors.last()).map(|a| Leave::segment(cx, a.p, false).without_contact());
        let q = self.snap.drag(cx, p, from.as_ref());
        if let Some(pr) = self.press.as_mut() {
            pr.end = Some((p, q));
        }
        let q = match self.drawing.as_ref().and_then(|dr| dr.pieces.last()) {
            Some(piece) => self.press_end(cx, piece, p, q),
            None => q,
        };
        let Some(last) = self.drawing.as_mut().and_then(|dr| dr.pieces.last_mut()) else { return vec![] };
        if last.to == q {
            return vec![];
        }
        last.to = q;
        vec![Action::Preview(COMMAND.into(), self.params(false)), drawn()]
    }

    /// Where `piece`, which a press draws, ends with the pointer at `p` that the snap put at `q`:
    /// on the protractor's tick when [`Self::on_tick`] puts it there for the piece's action.
    fn press_end(&self, cx: &ToolContext, piece: &Piece, p: Point, q: Point) -> Point {
        let Some(sp) = self.before_piece() else { return q };
        let from = sp.anchors.last().map(|a| Leave::segment(cx, a.p, false).without_contact());
        let landed = piece.stroke == Stroke::ContinuousArc && self.landed(cx, p, q, from.as_ref());
        self.on_tick(cx, &sp, *piece, p, q, landed)
    }

    /// `q`, where the last snap put the end of `piece` drawn after `sp` (the pointer being at `p`),
    /// moved onto the protractor's nearest tick ([`tick_end`]) when Smart Guides are on, the tick
    /// lies within the snapping tolerance of the pointer, and nothing else snapped the point: the
    /// snap left it where the pointer is, the pointer is on no Smart Guide target (shown or not,
    /// `landed`), and neither the grid nor pixels snap drawn points.
    fn on_tick(&self, cx: &ToolContext, sp: &SubPath, piece: Piece, p: Point, q: Point, landed: bool) -> Point {
        if landed || !cx.smart_guides || q != p || cx.snap_to_grid || cx.snaps_to_pixel(PixelOp::Drawing) {
            return q;
        }
        tick_end(sp, &Piece { to: q, ..piece }).filter(|b| b.distance(q) <= cx.tol(cx.snapping_tolerance)).unwrap_or(q)
    }

    /// Whether the pointer at `p`, which the last snap left in place at `q`, lies on a Smart Guide
    /// target leaving `from`, shown or not ([`DrawSnap::lands_on_target`]). A point the snap moved
    /// is not asked about, since a tick applies only to a point left in place.
    fn landed(&self, cx: &ToolContext, p: Point, q: Point, from: Option<&Leave>) -> bool {
        q == p && self.snap.lands_on_target(cx, p, from)
    }

    /// Where the next click's piece after `sp` ends while the pointer hovers: the hover's snapped
    /// point, on the protractor's tick when [`Self::on_tick`] puts it there. None before the
    /// pointer moves after a press, and while the hover is stale ([`Self::hover_stale`]).
    fn hover_end(&self, cx: &ToolContext, sp: &SubPath) -> Option<Point> {
        let (&(p, landed, _), q) = (self.pointer.as_ref()?, self.hover?);
        if self.hover_stale(cx) {
            return None;
        }
        Some(self.on_tick(cx, sp, self.piece_to(q), p, q, landed))
    }

    /// Whether the snap settings changed since the pointer last moved: the hover's snapped point
    /// and its tick may then not be where a click puts the point, so the hover shows nothing until
    /// the pointer moves again.
    fn hover_stale(&self, cx: &ToolContext) -> bool {
        self.pointer.as_ref().is_some_and(|(_, _, at)| *at != SnapSettings::of(cx))
    }

    /// During a press that adds a piece: the points before that piece, as the press found them
    /// (the last two, see [`tail`]).
    fn before_piece(&self) -> Option<SubPath> {
        let dr = self.drawing.as_ref()?;
        tail(dr.before_press.as_ref()?.path_data()?.subpaths.get(dr.si)?, *dr.kept.last()?)
    }

    /// Draws the path again after its last piece changed: a preview while a press draws that
    /// piece, else a step of its own.
    fn redraw(&self) -> Vec<Action> {
        if self.press.is_some_and(|pr| pr.begun) {
            vec![Action::Preview(COMMAND.into(), self.params(false)), drawn()]
        } else {
            vec![Action::Exec(COMMAND.into(), self.params(false)), drawn()]
        }
    }

    fn piece_to(&self, to: Point) -> Piece {
        Piece { to, stroke: self.action, turn: self.turn, flip: false }
    }

    /// The segment from the path's last point the snapping leaves from.
    fn leave(&self, cx: &ToolContext) -> Option<(NodeId, Leave)> {
        let (id, sp) = self.active(cx)?;
        sp.anchors.last().map(|a| (id, Leave::segment(cx, a.p, false).without_contact()))
    }

    /// What a press draws: a piece on the path being drawn, its closing piece, or a new path (from
    /// an end of an open path, the path goes on). With no path being drawn, a press on an anchor
    /// or a segment of a selected path waits to see whether it drags ([`edit`]).
    fn press_down(&mut self, cx: &ToolContext, p: Point, shift: bool) -> Vec<Action> {
        if self.drawing.is_some() && self.active(cx).is_none() {
            self.drawing = None;
        }
        // A press on a length handle waits to see whether it drags; one let go without dragging is
        // the click `click` makes.
        if let Some(handle) = self.length_handle_at(cx, p)
            && let Some(before) = cx.doc.node(handle.id).and_then(|n| n.path_data()).and_then(|pd| pd.subpaths.get(handle.si)).cloned()
        {
            let last = self.drawn_last_piece(cx, &handle);
            let tail = last.and(self.drawing.as_ref()).and_then(|dr| dr.last_tail(&before));
            if let Some(dr) = self.drawing.as_mut().filter(|dr| dr.id == Some(handle.id)) {
                dr.before_press = shared_node(cx.doc, handle.id).cloned();
            }
            let drag = LengthDrag {
                handle,
                press: p,
                length: handle.piece.length,
                dragged: false,
                begun: false,
                last,
                before,
                tail,
                revision: cx.revision,
                stale: false,
            };
            // The hover's Smart Guides do not follow a length drag.
            self.snap.clear();
            self.length = Some(drag);
            return vec![];
        }
        self.click(cx, p, shift)
    }

    /// What a press away from a length handle, or a click on one, draws.
    fn click(&mut self, cx: &ToolContext, p: Point, shift: bool) -> Vec<Action> {
        let tol = point_tol(cx);
        if let Some((id, sp)) = self.active(cx) {
            let (first, last) = match (sp.anchors.first(), sp.anchors.last()) {
                (Some(f), Some(l)) => (f.p, l.p),
                _ => return vec![],
            };
            if sp.anchors.len() >= 2 && p.distance(first) <= tol && p.distance(first) <= p.distance(last) {
                let (piece, kept) = (self.piece_to(first), sp.anchors.len());
                if let Some(dr) = self.drawing.as_mut() {
                    dr.push(piece, kept);
                }
                let out = vec![Action::Exec(COMMAND.into(), self.params(true))];
                self.drawing = None;
                return out;
            }
            if p.distance(last) <= tol {
                return vec![];
            }
            let from = self.leave(cx).map(|l| l.1);
            let snapped = self.snap.press(cx, p, &[id], from.as_ref());
            let landed = self.action == Stroke::ContinuousArc && self.landed(cx, p, snapped, from.as_ref());
            let q = self.on_tick(cx, sp, self.piece_to(snapped), p, snapped, landed);
            let (piece, kept) = (self.piece_to(q), sp.anchors.len());
            if let Some(dr) = self.drawing.as_mut() {
                dr.push(piece, kept);
                dr.before_press = shared_node(cx.doc, id).cloned();
            }
            self.press = Some(Press { at: p, dragged: false, begun: true, end: Some((p, snapped)) });
            return vec![Action::Begin("Quick Pen".into()), Action::Preview(COMMAND.into(), self.params(false)), drawn()];
        }
        // An anchor of a selected path comes first, then an open end, then a selected path's
        // segment.
        let grab = edit::grab_at(cx, p, tol);
        if let Some(grab @ Grab::Anchor { .. }) = grab {
            self.edit = Some(Edit::new(cx, grab, p, shift));
            return vec![];
        }
        if let Some((id, si, at_first)) = open_end_at(cx, p, tol, None)
            && cx.doc.is_editable(id)
        {
            return self.continue_path(cx, id, si, at_first);
        }
        if let Some(grab) = grab {
            self.edit = Some(Edit::new(cx, grab, p, shift));
            return vec![];
        }
        self.start_path(cx, p)
    }

    /// Goes on drawing from an end of subpath `si` of path `id`.
    fn continue_path(&mut self, cx: &ToolContext, id: NodeId, si: usize, at_first: bool) -> Vec<Action> {
        // From its start, `continue_from` reverses the path first; the tool then sees it.
        self.drawing = Some(Drawing { id: Some(id), si, ..Drawing::default() });
        let mut out = vec![];
        // Anchors selected by an earlier click or drag: the path is selected whole again.
        if !cx.selection.anchors.is_empty() || !cx.selection.segments.is_empty() {
            let top = cx.doc.parent_of(id).filter(|p| cx.doc.node(*p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. }))).unwrap_or(id);
            out.push(Action::Exec("select.set".into(), json!({"ids": [top.0]})));
        }
        out.extend(continue_from(cx, id, si, at_first));
        out.push(drawn());
        out
    }

    /// Starts a new path at `p`.
    fn start_path(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        let q = self.snap.press(cx, p, &[], None);
        self.drawing = Some(Drawing { start: Some(q), ..Drawing::default() });
        vec![Action::Exec(COMMAND.into(), self.params(false)), Action::Notify("created".into())]
    }

    /// Ends the path being drawn; a press that added a piece, or moved part of a path, is taken
    /// back. A move that another command's edit interrupted is kept with that edit.
    fn end(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.drawing = None;
        self.pointer = None;
        self.snap.clear();
        let moved = self.edit.take().filter(|e| e.began).map(|mut e| if e.open(cx) { Action::Cancel } else { Action::Commit });
        match self.press.take() {
            Some(pr) if pr.begun => vec![Action::Cancel],
            _ => moved.into_iter().collect(),
        }
    }

    /// The release of a press on an anchor or a segment of a selected path: a drag, also one let
    /// go past the drag distance after shorter drag events, moves what it holds to `p` and ends
    /// its undo step. A click on an end of an open path goes on drawing from it, a click on another
    /// anchor selects it alone (with Shift, adds it to the selected anchors or takes it out), and a
    /// click on a segment starts a new path there. A click whose path or selection another command
    /// changed while the press was down does nothing.
    fn release_edit(&mut self, cx: &ToolContext, mut e: Edit, p: Point) -> Vec<Action> {
        if e.began || p.distance(e.start) >= cx.tol(DRAG_PX) {
            let mut out = e.drag(cx, p);
            // A drag that could not begin (its path or the selection changed under the press) has
            // no step to end.
            out.extend(e.finish(cx));
            return out;
        }
        if !e.unchanged(cx) {
            return vec![];
        }
        match e.grab {
            Grab::Anchor { id, si, ai, .. } if e.shift => {
                // `select.anchors` toggles from every anchor of a path selected whole on its own,
                // so the first Shift-click there selects the anchor alone.
                let whole = cx.selection.anchors.is_empty() && cx.selection.objects.len() <= 1;
                let mode = if whole { "set" } else { "toggle" };
                vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[si, ai]], "mode": mode}))]
            }
            Grab::Anchor { id, si, ai, end: true, .. } => self.continue_path(cx, id, si, ai == 0),
            Grab::Anchor { id, si, ai, .. } => {
                vec![Action::Exec("select.anchors".into(), json!({"id": id.0, "anchors": [[si, ai]], "mode": "set"}))]
            }
            Grab::Segment { .. } => self.start_path(cx, e.start),
        }
    }

    /// The piece the next click adds, from the end of `sp` to `h`, as a path to preview.
    fn rubber_band(&self, sp: &SubPath, h: Point) -> Option<BezPath> {
        let n = sp.anchors.len();
        let mut tail = SubPath::new(sp.anchors.get_range(n.saturating_sub(2)..)?, false);
        let from = tail.anchors.len() - 1;
        if !append(&mut tail, &self.piece_to(h)) {
            return None;
        }
        let mut added = tail.anchors.get_range(from..)?;
        if let Some(a) = added.first_mut() {
            a.h_in = a.p;
        }
        let mut bp = BezPath::new();
        SubPath::new(added, false).to_bezpath_into(&mut bp);
        Some(bp)
    }

    /// The length handles: at both ends of the path being drawn, or, while none is, of each open
    /// subpath of the one editable path selected, wherever the end is a line or an arc.
    fn length_handles(&self, cx: &ToolContext) -> Vec<LengthHandle> {
        // At the end of the path being drawn, the handle changes the last piece drawn alone, even
        // where the piece before it goes on around the same circle.
        let mut tail = None;
        let subpaths: Vec<(NodeId, usize, &SubPath)> = if let Some((id, sp)) = self.active(cx) {
            tail = self.drawing.as_ref().and_then(|dr| dr.last_tail(sp));
            self.drawing.as_ref().map(|dr| (id, dr.si, sp)).into_iter().collect()
        } else if let [id] = cx.selection.objects.as_slice()
            && cx.doc.is_editable(*id)
            && let Some(node) = cx.doc.node(*id)
        {
            // A compound path shows the handles of the paths it holds.
            let members: Vec<&Node> = if matches!(node.kind, NodeKind::Compound { .. }) {
                let editable = cx.doc.editable();
                node.editing_children().iter().map(|c| c.as_ref()).filter(|c| editable(c.id)).collect()
            } else {
                vec![node]
            };
            members
                .into_iter()
                .filter_map(|n| Some((n.id, n.path_data()?)))
                .flat_map(|(id, pd)| pd.subpaths.iter().enumerate().filter(|(_, sp)| !sp.closed).map(move |(si, sp)| (id, si, sp)))
                .collect()
        } else {
            vec![]
        };
        let off = cx.tol(LENGTH_HANDLE_PX);
        let mut out = vec![];
        for (id, si, sp) in subpaths {
            for at_start in [false, true] {
                let piece = match &tail {
                    Some(t) if !at_start => length::end_piece(t, false),
                    _ => length::end_piece(sp, at_start),
                };
                if let Some(piece) = piece {
                    out.push(LengthHandle { id, si, at_start, piece, at: piece.end + piece.out * off });
                }
            }
        }
        out
    }

    /// The length handle under `p`, the nearest when several are.
    fn length_handle_at(&self, cx: &ToolContext, p: Point) -> Option<LengthHandle> {
        let tol = point_tol(cx);
        self.length_handles(cx).into_iter().filter(|h| h.at.distance(p) <= tol).min_by(|a, b| a.at.distance(p).total_cmp(&b.at.distance(p)))
    }

    /// The last piece of the path being drawn, when `handle` is at its end and the piece is the
    /// line or arc there.
    fn drawn_last_piece(&self, cx: &ToolContext, handle: &LengthHandle) -> Option<Piece> {
        let (id, sp) = self.active(cx)?;
        let dr = self.drawing.as_ref()?;
        if handle.at_start || id != handle.id || dr.si != handle.si {
            return None;
        }
        let last = dr.pieces.last()?;
        let from = sp.anchors.get(dr.kept.last()?.checked_sub(1)?)?.p;
        let same = match handle.piece.shape {
            length::EndShape::Line => last.stroke == Stroke::Line,
            length::EndShape::Arc { .. } => last.stroke != Stroke::Line,
        };
        (same && from.distance(handle.piece.fixed) <= 1e-9).then_some(*last)
    }

    /// Puts back the last piece a length drag changed, when the drag is taken back.
    fn restore_last_piece(&mut self, drag: &LengthDrag) {
        if let (Some(last), Some(l)) = (drag.last, self.drawing.as_mut().and_then(|dr| dr.pieces.last_mut())) {
            *l = last;
        }
    }

    /// `path.quickPenLength`'s params for the length drag.
    fn length_params(drag: &LengthDrag) -> Value {
        let h = &drag.handle;
        json!({"id": h.id.0, "subpath": h.si, "end": if h.at_start { "start" } else { "end" }, "length": drag.length})
    }

    /// Moves the end of the line or arc a length drag changes to follow the pointer at `p` and
    /// previews it. An edit made outside the tool since the last preview ends the drag instead: the
    /// press then commits the document as it is.
    fn length_drag_to(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        let Some(mut drag) = self.length.clone() else { return vec![] };
        if drag.stale {
            return vec![];
        }
        if drag.revision != cx.revision {
            drag.stale = true;
            self.length = Some(drag);
            return vec![];
        }
        if !drag.dragged && p.distance(drag.press) < cx.tol(DRAG_PX) {
            return vec![];
        }
        drag.dragged = true;
        self.length = Some(drag.clone());
        // The end follows the pointer, as far behind it as the press was.
        let piece = drag.handle.piece;
        let mut length = piece.length_dragged(drag.press, p, drag.length);
        // The last piece drawn goes on being a piece of the drawing, drawn again to its new
        // end; an arc piece keeps an arc size of at least 1°.
        let mut redrawn = None;
        let changed = if let (Some(last), Some(tail)) = (drag.last, drag.tail.as_ref()) {
            if let length::EndShape::Arc { radius, .. } = piece.shape {
                length = length.max(MIN_TURN.to_radians() * radius);
            }
            let changed = length::with_length(tail, false, length);
            redrawn = changed.as_ref().and_then(|sp| sp.anchors.last().map(|a| a.p)).map(|to| {
                let turn = match piece.shape {
                    length::EndShape::Arc { radius, .. } => (length / radius).to_degrees().clamp(MIN_TURN, MAX_TURN),
                    length::EndShape::Line => last.turn,
                };
                Piece { to, turn, ..last }
            });
            changed
        } else {
            length::with_length(&drag.before, drag.handle.at_start, length)
        };
        // A length that would take the path past the canvas is not taken: the end stays
        // where the drag last put it.
        if !changed.is_some_and(|sp| on_canvas(&sp)) {
            return vec![];
        }
        // A length the end already has changes nothing: no preview, and no undo step before the
        // first change.
        if (length - drag.length).abs() <= 1e-12 * length.max(1.0) {
            return vec![];
        }
        let mut out = vec![];
        if !drag.begun {
            drag.begun = true;
            out.push(Action::Begin("Quick Pen".into()));
        }
        drag.length = length;
        match redrawn {
            Some(pc) => {
                if let Some(l) = self.drawing.as_mut().and_then(|dr| dr.pieces.last_mut()) {
                    *l = pc;
                }
                out.push(Action::Preview(COMMAND.into(), self.params(false)));
            }
            None => out.push(Action::Preview(LENGTH_COMMAND.into(), Self::length_params(&drag))),
        }
        // The drawing sees the path as the preview left it.
        out.push(drawn());
        self.length = Some(drag);
        out
    }

    /// A length drag let go: once it changed the path being drawn, the drawing goes on from the
    /// changed path, which `path.quickPenLength` made from the same points. A press on a handle let
    /// go without dragging is a click there. After an edit made outside the tool, the path being
    /// drawn ends and the document stays as that edit left it.
    fn end_length_drag(&mut self, cx: &ToolContext) -> Vec<Action> {
        let Some(drag) = self.length.take() else { return vec![] };
        if drag.stale || drag.revision != cx.revision {
            self.drawing = None;
            self.snap.clear();
            return if drag.begun { vec![Action::Commit] } else { vec![] };
        }
        if !drag.dragged {
            let mut out = self.click(cx, drag.press, false);
            // A handle over a segment of the selected path: the press has ended, so the click
            // there is released at once.
            if let Some(e) = self.edit.take() {
                out.extend(self.release_edit(cx, e, drag.press));
            }
            if self.press.take().is_some_and(|pr| pr.begun) {
                out.extend([Action::Commit, Action::Notify(PLACED.into())]);
            }
            if let Some(dr) = self.drawing.as_mut() {
                dr.before_press = None;
            }
            self.snap.clear();
            return out;
        }
        if let Some(dr) = self.drawing.as_mut() {
            dr.before_press = None;
        }
        if !drag.begun {
            return vec![];
        }
        // A drag that ends at the length the piece started with is taken back, as Escape takes it
        // back: it records no undo step.
        let changed = (drag.length - drag.handle.piece.length).abs() > 1e-9 * drag.length.max(1.0);
        if !changed {
            self.restore_last_piece(&drag);
            return vec![Action::Cancel, drawn()];
        }
        // A drag that drew the last piece again has already changed the drawing.
        if drag.last.is_none()
            && let Some(dr) = self.drawing.as_mut()
            && dr.id == Some(drag.handle.id)
            && dr.si == drag.handle.si
        {
            // The pieces that start at or after the fixed end of a changed start still draw as they
            // did, on the points the document holds before them; every other piece is forgotten.
            let new_len = length::with_length(&drag.before, drag.handle.at_start, drag.length).map(|sp| sp.anchors.len());
            let fixed = drag.before.anchors.iter().position(|a| a.p.distance(drag.handle.piece.fixed) <= 1e-9);
            match (drag.handle.at_start, new_len, fixed) {
                (true, Some(new_len), Some(fixed)) => dr.after_start_change(fixed, new_len as isize - drag.before.anchors.len() as isize),
                (_, None, _) => self.drawing = None,
                _ => dr.clear_pieces(),
            }
        }
        vec![Action::Commit, drawn()]
    }
}

impl Tool for QuickPenTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.press.is_some() || self.edit.is_some() || self.length.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move => {
                let (on, from) = self.leave(cx).unzip();
                let q = self.snap.hover(cx, p, on.as_slice(), from.as_ref());
                self.hover = Some(q);
                self.pointer = Some((p, on.is_some() && self.landed(cx, p, q, from.as_ref()), SnapSettings::of(cx)));
                vec![]
            }
            PointerKind::Down => {
                if self.press.is_some() || self.edit.is_some() || self.length.is_some() {
                    return vec![];
                }
                // The hover shows again when the pointer moves after the press: the point the press
                // places can lie on a tick away from it. A stale hover's guides stay hidden, also
                // when the press snaps nothing.
                if self.hover_stale(cx) {
                    self.snap.clear();
                }
                self.hover = None;
                self.pointer = None;
                self.press_down(cx, p, ev.mods.shift)
            }
            PointerKind::Drag if self.length.is_some() => self.length_drag_to(cx, p),
            PointerKind::Drag => {
                if let Some(e) = self.edit.as_mut() {
                    if !e.began && p.distance(e.start) < cx.tol(DRAG_PX) {
                        return vec![];
                    }
                    return e.drag(cx, p);
                }
                let Some(mut pr) = self.press else { return vec![] };
                if !pr.dragged && p.distance(pr.at) < cx.tol(DRAG_PX) {
                    return vec![];
                }
                pr.dragged = true;
                self.press = Some(pr);
                if !pr.begun {
                    return vec![];
                }
                self.drag_to(cx, p)
            }
            PointerKind::Up => {
                if let Some(e) = self.edit.take() {
                    return self.release_edit(cx, e, p);
                }
                if self.length.is_some() {
                    self.snap.clear();
                    // The end goes where the press is let go, also past the last drag.
                    let mut out = self.length_drag_to(cx, p);
                    out.extend(self.end_length_drag(cx));
                    return out;
                }
                let out = match self.press {
                    // The piece ends where the press is let go, also past the last drag.
                    Some(pr) if pr.begun => {
                        let mut out = vec![];
                        if pr.dragged || p.distance(pr.at) >= cx.tol(DRAG_PX) {
                            out = self.drag_to(cx, p);
                        } else if self.active(cx).is_none() {
                            // An edit made outside the tool during the press ends the path.
                            self.drawing = None;
                        }
                        // With Real-time Drawing off, the commit applies the piece.
                        out.extend([Action::Commit, Action::Notify(PLACED.into())]);
                        out
                    }
                    _ => vec![],
                };
                if let Some(dr) = self.drawing.as_mut() {
                    dr.before_press = None;
                }
                self.press = None;
                self.snap.clear();
                out
            }
            PointerKind::DoubleClick | PointerKind::TripleClick => vec![],
        }
    }

    fn notify(&mut self, cx: &ToolContext, what: &str) {
        if what == edit::MOVED {
            if let Some(e) = self.edit.as_mut() {
                e.revision = Some(cx.revision);
            }
            return;
        }
        if what == DRAWN
            && let Some(drag) = self.length.as_mut()
        {
            drag.revision = cx.revision;
        }
        let Some(dr) = self.drawing.as_mut() else { return };
        if what == "created"
            && let [id] = cx.selection.objects.as_slice()
            && let Some(node) = shared_node(cx.doc, *id).filter(|n| n.path_data().is_some_and(|pd| !pd.subpaths.is_empty()))
            && dr.id.is_none()
        {
            (dr.id, dr.si, dr.start, dr.seen) = (Some(*id), 0, None, Some(node.clone()));
        } else if (what == DRAWN || what == PLACED)
            && let Some(id) = dr.id
        {
            dr.seen = shared_node(cx.doc, id).cloned();
            dr.revision = cx.revision;
            // Once committed, a piece that added no point to the path (the command refused it) is
            // not kept for C and [ ] to change.
            let points = dr.seen.as_ref().and_then(|n| n.path_data()).and_then(|pd| pd.subpaths.get(dr.si)).map(|sp| sp.anchors.len());
            if what == PLACED
                && let (Some(points), Some(&kept)) = (points, dr.kept.last())
                && points <= kept
            {
                dr.pop();
            }
        }
    }

    /// A path being drawn: Escape ends it.
    fn ends_on_escape(&self, cx: &ToolContext) -> bool {
        self.active(cx).is_some()
    }

    fn drawing_end(&self, cx: &ToolContext) -> Option<(Point, Option<Vec2>)> {
        let (_, sp) = self.active(cx)?;
        Some((sp.anchors.last()?.p, arrival(sp)))
    }

    fn claims_key(&self, cx: &ToolContext, key: ToolKey) -> bool {
        match key {
            ToolKey::Letter('c') => self.length.is_some() || self.last_sharp_arc(cx).is_some(),
            ToolKey::BracketLeft | ToolKey::BracketRight | ToolKey::AltTap => true,
            ToolKey::Enter => self.active(cx).is_some(),
            _ => false,
        }
    }

    /// A dropped press does not end the path being drawn: the piece it added goes.
    /// The drawing goes on only when the path is back as it was before the press.
    fn drop_press(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.snap.clear();
        // The session takes back what an edit moved; the release that follows does nothing.
        self.edit = None;
        if let Some(drag) = self.length.take() {
            self.restore_last_piece(&drag);
            if let Some(dr) = self.drawing.as_mut() {
                let before = dr.before_press.take();
                if drag.begun {
                    match (dr.id.and_then(|id| cx.doc.node(id)), before) {
                        (Some(now), Some(before)) if same_node(now, &before) => dr.seen = Some(before),
                        _ => self.drawing = None,
                    }
                }
            }
        }
        if self.press.take().is_some_and(|pr| pr.begun)
            && let Some(dr) = self.drawing.as_mut()
        {
            dr.pop();
            let now = dr.id.and_then(|id| cx.doc.node(id));
            match (now, dr.before_press.take()) {
                (Some(now), Some(before)) if same_node(now, &before) => dr.seen = Some(before),
                _ => self.drawing = None,
            }
        }
        vec![]
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            // Escape during a length drag takes the drag back and leaves the path being drawn; Enter
            // does nothing until the drag ends.
            ToolKey::Escape if self.length.is_some() => match self.length.take() {
                Some(drag) => {
                    if let Some(dr) = self.drawing.as_mut() {
                        dr.before_press = None;
                    }
                    if !drag.begun {
                        return vec![];
                    }
                    // An edit made outside the tool since the last preview stays: the cancel would
                    // take it back with the drag.
                    if drag.stale || drag.revision != cx.revision {
                        self.drawing = None;
                        return vec![Action::Commit];
                    }
                    self.restore_last_piece(&drag);
                    // The drawing sees the path as the cancel leaves it.
                    vec![Action::Cancel, drawn()]
                }
                None => vec![],
            },
            ToolKey::Enter if self.length.is_some() => vec![],
            // C and [ ] change the last piece once the drag has ended.
            ToolKey::Letter('c') | ToolKey::BracketLeft | ToolKey::BracketRight if self.length.is_some() => vec![],
            ToolKey::Escape | ToolKey::Enter => self.end(cx),
            ToolKey::Letter('c') => {
                if self.last_sharp_arc(cx).is_none() {
                    return vec![];
                }
                if let Some(last) = self.drawing.as_mut().and_then(|dr| dr.pieces.last_mut()) {
                    last.flip = !last.flip;
                }
                self.redraw()
            }
            ToolKey::BracketLeft | ToolKey::BracketRight => {
                let by = if key == ToolKey::BracketLeft { -TURN_STEP } else { TURN_STEP };
                let arc = self.last_sharp_arc(cx);
                let turn = arc.map_or(self.turn, |a| a.turn) + by;
                if !(MIN_TURN..=MAX_TURN).contains(&turn) {
                    return vec![];
                }
                self.turn = turn;
                if arc.is_none() {
                    return vec![];
                }
                if let Some(last) = self.drawing.as_mut().and_then(|dr| dr.pieces.last_mut()) {
                    last.turn = turn;
                }
                self.redraw()
            }
            ToolKey::AltTap => {
                self.action = self.action.next();
                // A drag under way draws its piece with the new action. Its end goes through the
                // protractor's tick again for that action: a continuous arc's end on a tick goes
                // back to where the snap put the pointer for another action, and the other way.
                if self.press.is_some_and(|pr| pr.begun) && self.held(cx) {
                    let (action, turn, end) = (self.action, self.turn, self.press.and_then(|pr| pr.end));
                    let piece = self.drawing.as_ref().and_then(|dr| dr.pieces.last()).map(|&pc| {
                        let pc = Piece { stroke: action, turn, flip: false, ..pc };
                        end.map_or(pc, |(p, q)| Piece { to: self.press_end(cx, &pc, p, q), ..pc })
                    });
                    if let (Some(piece), Some(last)) = (piece, self.drawing.as_mut().and_then(|dr| dr.pieces.last_mut())) {
                        *last = piece;
                    }
                    return vec![Action::Preview(COMMAND.into(), self.params(false)), drawn()];
                }
                vec![]
            }
            _ => vec![],
        }
    }

    fn deactivate(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.drawing = None;
        self.snap.clear();
        let moved = self.edit.take().and_then(|mut e| e.finish(cx));
        if self.length.take().is_some_and(|drag| drag.begun) {
            return vec![Action::Commit];
        }
        match self.press.take() {
            Some(pr) if pr.begun => vec![Action::Commit],
            _ => moved.into_iter().collect(),
        }
    }

    fn chosen_again(&mut self, _cx: &ToolContext) -> Vec<Action> {
        if self.press.is_none() && self.length.is_none() {
            self.drawing = None;
        }
        vec![]
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = None;
        self.pointer = None;
    }

    /// A hover's snapped point and its tick depend on the art and the view under a still pointer.
    fn wants_rehover(&self) -> bool {
        self.pointer.is_some()
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        // A stale hover's Smart Guides go with its preview.
        let mut o = if self.hover_stale(cx) { vec![] } else { self.snap.guides().to_vec() };
        if let Some(e) = &self.edit {
            o.extend(e.guides.iter().cloned());
        }
        // The length handles: a line from each end to a handle circle beyond it; during a drag, the
        // one being dragged, where the changed end is.
        let handles: Vec<(Point, Point)> = match &self.length {
            Some(drag) => {
                let h = &drag.handle;
                let sp = cx.doc.node(h.id).and_then(|n| n.path_data()).and_then(|pd| pd.subpaths.get(h.si));
                let piece = sp.and_then(|sp| length::end_piece(sp, h.at_start));
                piece.map(|pc| (pc.end, pc.end + pc.out * cx.tol(LENGTH_HANDLE_PX))).into_iter().collect()
            }
            None if self.press.is_none() && self.edit.is_none() => self.length_handles(cx).iter().map(|h| (h.piece.end, h.at)).collect(),
            None => vec![],
        };
        for &(a, b) in &handles {
            o.push(Overlay::Line { a, b, color: FEEDBACK, dashed: false });
            o.push(Overlay::Handle { p: b, color: FEEDBACK });
        }
        let Some((id, sp)) = self.active(cx) else { return o };
        let end = if self.press.is_none() { self.hover_end(cx, sp) } else { None };
        // The arc being drawn: the piece a press drags, or the piece the next click adds while the
        // rubber band shows it.
        let arc = match self.press {
            Some(pr) if pr.begun => {
                self.before_piece().zip(self.drawing.as_ref().and_then(|dr| dr.pieces.last())).and_then(|(sp, piece)| arc_circle(&sp, piece))
            }
            Some(_) => None,
            None => end.filter(|_| cx.pen_rubber_band).and_then(|h| arc_circle(sp, &self.piece_to(h))),
        };
        if let Some(arc) = arc {
            o.push(Overlay::Path { path: protractor(cx, &arc), color: FEEDBACK, width: 1.0, dashed: false });
            let (c, arm) = (arc.center, cx.tol(CROSS_PX));
            o.push(Overlay::Line { a: c - Vec2::new(arm, 0.0), b: c + Vec2::new(arm, 0.0), color: FEEDBACK, dashed: false });
            o.push(Overlay::Line { a: c - Vec2::new(0.0, arm), b: c + Vec2::new(0.0, arm), color: FEEDBACK, dashed: false });
        }
        let on_handle = self.hover.is_some_and(|h| handles.iter().any(|&(_, at)| at.distance(h) <= point_tol(cx)));
        if self.press.is_none()
            && self.length.is_none()
            && !on_handle
            && !self.hover_stale(cx)
            && let Some(h) = end.or(self.hover)
        {
            if cx.pen_rubber_band
                && let Some(path) = self.rubber_band(sp, h)
            {
                o.push(Overlay::Path { path, color: cx.doc.layer_color(id), width: 1.0, dashed: false });
            }
            if let Some(last) = sp.anchors.last().map(|a| a.p)
                && cx.measurement_labels
                && last.distance(h) > 1e-9
            {
                o.push(Overlay::Measure { p: h + Vec2::new(cx.tol(12.0), cx.tol(12.0)), text: cx.segment_measure_label(last, h) });
            }
        }
        let n = sp.anchors.len();
        for (i, a) in sp.anchors.iter().enumerate() {
            o.push(Overlay::CurvePoint { p: a.p, color: FEEDBACK, filled: i + 1 == n, size: 6.0 });
        }
        o
    }

    fn cursor(&self, cx: &ToolContext, p: Point, _m: Mods) -> Cursor {
        let tol = point_tol(cx);
        if self.length.is_some() || self.length_handle_at(cx, p).is_some() {
            return Cursor::Move;
        }
        if let Some((_, sp)) = self.active(cx) {
            let close = sp.anchors.len() >= 2 && sp.anchors.first().is_some_and(|f| f.p.distance(p) <= tol);
            return if close { Cursor::PenClose } else { Cursor::PenDrawing };
        }
        if let Some(e) = &self.edit {
            return if e.began { Cursor::Move } else { edit::cursor(cx, e.grab) };
        }
        let grab = edit::grab_at(cx, p, tol);
        if let Some(grab @ Grab::Anchor { .. }) = grab {
            return edit::cursor(cx, grab);
        }
        if open_end_at(cx, p, tol, None).is_some() {
            return Cursor::PenContinue;
        }
        grab.map_or(Cursor::Pen, |grab| edit::cursor(cx, grab))
    }

    fn options(&self) -> Value {
        json!({ "action": self.action.name(), "turn": self.turn })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "action" => {
                if let Some(a) = v.as_str().and_then(Stroke::parse) {
                    self.action = a;
                }
            }
            "turn" => {
                if let Some(t) = v.as_f64().filter(|t| (MIN_TURN..=MAX_TURN).contains(t)) {
                    self.turn = t;
                }
            }
            _ => {}
        }
    }

    fn check_option(&self, key: &str, v: &Value) -> Result<(), String> {
        match key {
            "action" => crate::option_check::one_of(key, v, &Stroke::ALL.map(Stroke::name)),
            "turn" => crate::option_check::number(key, v, MIN_TURN, MAX_TURN),
            _ => Err(crate::option_check::unknown(key)),
        }
    }
}

pub mod length;

#[cfg(test)]
mod tests;
