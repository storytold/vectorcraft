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

use std::f64::consts::{FRAC_PI_2, PI};
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{Anchor, AnchorKind, BezPath, Point, SubPath, Vec2};

use crate::draw2::FEEDBACK;
use crate::guides::{DrawSnap, Leave};
use crate::pen::{continue_from, open_end_at};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};

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
/// Screen points the pointer moves after a press before the press drags.
const DRAG_PX: f64 = 3.0;
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
}

/// The press in progress: where it went down, whether it has dragged, and whether it began an undo
/// step (a piece it adds).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    at: Point,
    dragged: bool,
    begun: bool,
}

pub struct QuickPenTool {
    action: Stroke,
    turn: f64,
    drawing: Option<Drawing>,
    press: Option<Press>,
    hover: Option<Point>,
    snap: DrawSnap,
}

impl Default for QuickPenTool {
    fn default() -> Self {
        Self { action: Stroke::Line, turn: DEFAULT_TURN, drawing: None, press: None, hover: None, snap: DrawSnap::default() }
    }
}

/// Is `id` selected on its own, itself or the compound path it is in?
fn selected_alone(cx: &ToolContext, id: NodeId) -> bool {
    let sel = cx.selection.objects.as_slice();
    sel == [id] || cx.doc.parent_of(id).is_some_and(|p| sel == [p] && cx.doc.node(p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. })))
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
        let from = self.drawing.as_ref().and_then(|dr| {
            let sp = dr.before_press.as_ref()?.path_data()?.subpaths.get(dr.si)?;
            sp.anchors.get(dr.kept.last()?.checked_sub(1)?).map(|a| Leave::segment(cx, a.p, false).without_contact())
        });
        let q = self.snap.drag(cx, p, from.as_ref());
        let Some(last) = self.drawing.as_mut().and_then(|dr| dr.pieces.last_mut()) else { return vec![] };
        if last.to == q {
            return vec![];
        }
        last.to = q;
        vec![Action::Preview(COMMAND.into(), self.params(false)), drawn()]
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
    /// an end of an open path, the path goes on).
    fn press_down(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        if self.drawing.is_some() && self.active(cx).is_none() {
            self.drawing = None;
        }
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
            let q = self.snap.press(cx, p, &[id], from.as_ref());
            let (piece, kept) = (self.piece_to(q), sp.anchors.len());
            if let Some(dr) = self.drawing.as_mut() {
                dr.push(piece, kept);
                dr.before_press = shared_node(cx.doc, id).cloned();
            }
            self.press = Some(Press { at: p, dragged: false, begun: true });
            return vec![Action::Begin("Quick Pen".into()), Action::Preview(COMMAND.into(), self.params(false)), drawn()];
        }
        if let Some((id, si, at_first)) = open_end_at(cx, p, tol, None)
            && cx.doc.is_editable(id)
        {
            // From its start, `continue_from` reverses the path first; the tool then sees it.
            self.drawing = Some(Drawing { id: Some(id), si, ..Drawing::default() });
            let mut out = continue_from(cx, id, si, at_first);
            out.push(drawn());
            return out;
        }
        let q = self.snap.press(cx, p, &[], None);
        self.drawing = Some(Drawing { start: Some(q), ..Drawing::default() });
        vec![Action::Exec(COMMAND.into(), self.params(false)), Action::Notify("created".into())]
    }

    /// Ends the path being drawn; a press that added a piece is taken back.
    fn end(&mut self) -> Vec<Action> {
        self.drawing = None;
        self.snap.clear();
        match self.press.take() {
            Some(pr) if pr.begun => vec![Action::Cancel],
            _ => vec![],
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
}

impl Tool for QuickPenTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.press.is_some()
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move => {
                let (on, from) = self.leave(cx).unzip();
                self.hover = Some(self.snap.hover(cx, p, on.as_slice(), from.as_ref()));
                vec![]
            }
            PointerKind::Down => {
                if self.press.is_some() {
                    return vec![];
                }
                self.press_down(cx, p)
            }
            PointerKind::Drag => {
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
            ToolKey::Letter('c') => self.last_sharp_arc(cx).is_some(),
            ToolKey::BracketLeft | ToolKey::BracketRight | ToolKey::AltTap => true,
            ToolKey::Enter => self.active(cx).is_some(),
            _ => false,
        }
    }

    /// A dropped press does not end the path being drawn: the piece it added goes.
    /// The drawing goes on only when the path is back as it was before the press.
    fn drop_press(&mut self, cx: &ToolContext) -> Vec<Action> {
        self.snap.clear();
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
            ToolKey::Escape | ToolKey::Enter => self.end(),
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
                // A drag under way draws its piece with the new action.
                if self.press.is_some_and(|pr| pr.begun) && self.held(cx) {
                    let (action, turn) = (self.action, self.turn);
                    if let Some(last) = self.drawing.as_mut().and_then(|dr| dr.pieces.last_mut()) {
                        (last.stroke, last.turn, last.flip) = (action, turn, false);
                    }
                    return vec![Action::Preview(COMMAND.into(), self.params(false)), drawn()];
                }
                vec![]
            }
            _ => vec![],
        }
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.drawing = None;
        self.snap.clear();
        match self.press.take() {
            Some(pr) if pr.begun => vec![Action::Commit],
            _ => vec![],
        }
    }

    fn chosen_again(&mut self, _cx: &ToolContext) -> Vec<Action> {
        if self.press.is_none() {
            self.drawing = None;
        }
        vec![]
    }

    fn hover_ended(&mut self, _cx: &ToolContext) {
        self.hover = None;
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut o = self.snap.guides().to_vec();
        let Some((id, sp)) = self.active(cx) else { return o };
        if self.press.is_none()
            && let Some(h) = self.hover
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
        if let Some((_, sp)) = self.active(cx) {
            let close = sp.anchors.len() >= 2 && sp.anchors.first().is_some_and(|f| f.p.distance(p) <= tol);
            return if close { Cursor::PenClose } else { Cursor::PenDrawing };
        }
        if open_end_at(cx, p, tol, None).is_some() { Cursor::PenContinue } else { Cursor::Pen }
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

#[cfg(test)]
mod tests;
