//! The Quick Pen tool (Alt+P, Option+P on macOS) draws paths of straight lines and circular arcs
//! (`path.quickPen`).
//!
//! A click sets the start point, and each click or drag after it adds a piece from the last point
//! to where the pointer is let go, as the tool's action says: a straight line, a continuous arc
//! (it leaves its start in the direction the path arrives in, so the two join smoothly) or a
//! sharp-corner arc (it turns through the arc size, 180° by default, and meets the path before it
//! at a corner). Alt (Option) pressed and let go on its own switches to the next action. C flips
//! the last sharp-corner arc to the other side of its chord, and [ and ] make the arc size 15°
//! smaller or larger, for the last sharp-corner arc and the ones after it. A click on the first
//! point closes the path, a click on an end of an open path goes on drawing from it, and Escape
//! or Enter ends the path. The path's points show as circles, without handles, and a preview
//! shows the piece the next click adds. Points snap to Smart Guides. Each piece is one undo step.
//!
//! A handle stands beyond each end of the path being drawn, or, while none is, of the selected
//! path, where that end is a straight line or a circular arc. Dragging it makes the line or arc
//! longer or shorter, as one undo step (`path.quickPenLength`): a line keeps its direction, and
//! an arc keeps its circle. The dragged end stays on its line or circle and does not snap to Smart
//! Guides. A click on a handle that does not drag is an ordinary click: it adds a point, closes the
//! path or starts one.

use std::f64::consts::{FRAC_PI_2, PI};

use serde_json::{Value, json};
use vectorcraft_doc::{MAX_COORD, NodeId, NodeKind};
use vectorcraft_geom::{Anchor, AnchorKind, BezPath, Point, SubPath, Vec2};

use crate::draw2::FEEDBACK;
use crate::guides::{DrawSnap, Leave};
use crate::pen::{continue_from, open_end_at};
use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey};
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
/// Screen points the pointer moves after a press before the press drags.
const DRAG_PX: f64 = 3.0;
/// Screen points a length handle stands beyond the end of the path, along the way it leaves the end.
pub const LENGTH_HANDLE_PX: f64 = 20.0;
/// A continuous arc whose end lies within this angle (radians) of straight ahead, or of straight
/// behind, is drawn as a line: no circle runs there in the path's direction.
const STRAIGHT: f64 = 1e-6;

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

/// The path being drawn: subpath `si` of `id`, whose points were `base` before the tool drew on
/// (both none until the engine made a new path), then `pieces`. A new path starts at `start`.
#[derive(Clone, Debug, PartialEq)]
struct Drawing {
    id: Option<NodeId>,
    si: usize,
    base: Option<SubPath>,
    start: Option<Point>,
    pieces: Vec<Piece>,
}

impl Drawing {
    /// The subpath as the tool drew it.
    fn drawn(&self) -> Option<SubPath> {
        Some(build(self.base.as_ref()?, &self.pieces, false))
    }

    /// The last piece as drawn in `drawn` (the path [`Self::drawn`] builds): the points from where
    /// it starts to the end of the path.
    fn last_tail(&self, drawn: &SubPath) -> Option<SubPath> {
        let (_, rest) = self.pieces.split_last()?;
        let from = build(self.base.as_ref()?, rest, false).anchors.len().checked_sub(1)?;
        Some(SubPath::new(drawn.anchors.get_range(from..)?, false))
    }

    /// Splits `changed`, the drawn path with its start changed, into a base and the most pieces at
    /// its end that build it again point for point, so the pieces a drag did not touch stay pieces
    /// (C and `[` `]` go on changing the last one). None when no piece builds it again.
    fn rebase(&self, changed: &SubPath) -> Option<(SubPath, Vec<Piece>)> {
        let base = self.base.as_ref()?;
        let m = changed.anchors.len();
        let total = build(base, &self.pieces, false).anchors.len();
        for k in 0..self.pieces.len() {
            let (before, kept) = self.pieces.split_at(k);
            let added = total.checked_sub(build(base, before, false).anchors.len())?;
            let Some(keep) = m.checked_sub(added) else { continue };
            if keep == 0 {
                continue;
            }
            let new_base = SubPath::new(changed.anchors.get_range(..keep)?, false);
            if build(&new_base, kept, false).anchors == changed.anchors {
                return Some((new_base, kept.to_vec()));
            }
        }
        None
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
#[derive(Clone, Copy, Debug, PartialEq)]
struct LengthDrag {
    handle: LengthHandle,
    press: Point,
    length: f64,
    dragged: bool,
    begun: bool,
    /// The last piece of the path being drawn as it was before the drag, when the handle is at the
    /// end of that piece: the drag changes the piece, so C and [ ] still shape it afterwards.
    last: Option<Piece>,
}

pub struct QuickPenTool {
    action: Stroke,
    turn: f64,
    drawing: Option<Drawing>,
    press: Option<Press>,
    length: Option<LengthDrag>,
    hover: Option<Point>,
    snap: DrawSnap,
}

impl Default for QuickPenTool {
    fn default() -> Self {
        Self { action: Stroke::Line, turn: DEFAULT_TURN, drawing: None, press: None, length: None, hover: None, snap: DrawSnap::default() }
    }
}

/// Is `id` selected on its own, itself or the compound path it is in?
fn selected_alone(cx: &ToolContext, id: NodeId) -> bool {
    let sel = cx.selection.objects.as_slice();
    sel == [id] || cx.doc.parent_of(id).is_some_and(|p| sel == [p] && cx.doc.node(p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. })))
}

/// Whether every point and handle of `sp` lies on the canvas.
fn on_canvas(sp: &SubPath) -> bool {
    sp.anchors.iter().all(|a| [a.p, a.h_in, a.h_out].iter().all(|q| q.x.abs() <= MAX_COORD && q.y.abs() <= MAX_COORD))
}

fn point_tol(cx: &ToolContext) -> f64 {
    cx.tol(5.0).max(cx.point_tol())
}

impl QuickPenTool {
    /// The subpath being drawn, while it is selected, editable, open and still exactly what the tool
    /// drew: an edit made outside the tool (a point moved, the path it went on from changed) ends
    /// the drawing, so the next piece does not undo it.
    fn active<'a>(&self, cx: &'a ToolContext) -> Option<(NodeId, &'a SubPath)> {
        let dr = self.drawing.as_ref()?;
        let id = dr.id?;
        if !selected_alone(cx, id) || !cx.doc.is_editable(id) {
            return None;
        }
        let sp = cx.doc.node(id)?.path_data()?.subpaths.get(dr.si)?;
        (!sp.closed && dr.drawn()?.anchors == sp.anchors).then_some((id, sp))
    }

    /// `path.quickPen`'s params for the path being drawn, closed when `closed`.
    fn params(&self, closed: bool) -> Value {
        let Some(dr) = &self.drawing else { return json!({}) };
        let pieces: Vec<Value> = dr.pieces.iter().map(Piece::to_json).collect();
        let mut v = json!({"pieces": pieces});
        match (dr.id, dr.start) {
            (Some(id), _) => {
                v["id"] = json!(id.0);
                v["subpath"] = json!(dr.si);
                v["keep"] = json!(dr.base.as_ref().map_or(1, |b| b.anchors.len().max(1)));
            }
            (None, Some(s)) => v["start"] = json!([s.x, s.y]),
            (None, None) => {}
        }
        if closed {
            v["closed"] = json!(true);
        }
        v
    }

    /// The last piece, while it is a sharp-corner arc and no press is under way.
    fn last_sharp_arc(&self, cx: &ToolContext) -> Option<Piece> {
        if self.press.is_some() {
            return None;
        }
        self.active(cx)?;
        self.drawing.as_ref()?.pieces.last().copied().filter(|p| p.stroke == Stroke::SharpArc)
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
        // A press on a length handle waits to see whether it drags; one let go without dragging is
        // the click `click` makes.
        if let Some(handle) = self.length_handle_at(cx, p) {
            let last = self.drawn_last_piece(cx, &handle);
            self.length = Some(LengthDrag { handle, press: p, length: handle.piece.length, dragged: false, begun: false, last });
            return vec![];
        }
        self.click(cx, p)
    }

    /// What a press away from a length handle, or a click on one, draws.
    fn click(&mut self, cx: &ToolContext, p: Point) -> Vec<Action> {
        let tol = point_tol(cx);
        if let Some((id, sp)) = self.active(cx) {
            let (first, last) = match (sp.anchors.first(), sp.anchors.last()) {
                (Some(f), Some(l)) => (f.p, l.p),
                _ => return vec![],
            };
            if sp.anchors.len() >= 2 && p.distance(first) <= tol && p.distance(first) <= p.distance(last) {
                let piece = self.piece_to(first);
                if let Some(dr) = self.drawing.as_mut() {
                    dr.pieces.push(piece);
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
            let piece = self.piece_to(q);
            if let Some(dr) = self.drawing.as_mut() {
                dr.pieces.push(piece);
            }
            self.press = Some(Press { at: p, dragged: false, begun: true });
            return vec![Action::Begin("Quick Pen".into()), Action::Preview(COMMAND.into(), self.params(false))];
        }
        if let Some((id, si, at_first)) = open_end_at(cx, p, tol, None)
            && cx.doc.is_editable(id)
            && let Some(sp) = cx.doc.node(id).and_then(|n| n.path_data()).and_then(|pd| pd.subpaths.get(si))
        {
            // From its start, `continue_from` reverses the path first.
            let mut base = sp.clone();
            if at_first {
                base.reverse();
            }
            self.drawing = Some(Drawing { id: Some(id), si, base: Some(base), start: None, pieces: vec![] });
            return continue_from(cx, id, si, at_first);
        }
        let q = self.snap.press(cx, p, &[], None);
        self.drawing = Some(Drawing { id: None, si: 0, base: None, start: Some(q), pieces: vec![] });
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
            && let Some(pd) = cx.doc.node(*id).and_then(|n| n.path_data())
        {
            pd.subpaths.iter().enumerate().filter(|(_, sp)| !sp.closed).map(|(si, sp)| (*id, si, sp)).collect()
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
        let (id, _) = self.active(cx)?;
        let dr = self.drawing.as_ref()?;
        if handle.at_start || id != handle.id || dr.si != handle.si {
            return None;
        }
        let (last, rest) = dr.pieces.split_last()?;
        let from = build(dr.base.as_ref()?, rest, false).anchors.last()?.p;
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

    /// A length drag let go: once it changed the path being drawn, the drawing goes on from the
    /// changed path, which `path.quickPenLength` made from the same points. A press on a handle let
    /// go without dragging is a click there.
    fn end_length_drag(&mut self, cx: &ToolContext) -> Vec<Action> {
        let Some(drag) = self.length.take() else { return vec![] };
        if !drag.dragged {
            let mut out = self.click(cx, drag.press);
            if self.press.take().is_some_and(|pr| pr.begun) {
                out.push(Action::Commit);
            }
            self.snap.clear();
            return out;
        }
        if !drag.begun {
            return vec![];
        }
        // The command leaves a piece that already has the length unchanged; a drag that drew the
        // last piece again has already changed the drawing.
        let changed = (drag.length - drag.handle.piece.length).abs() > 1e-9 * drag.length.max(1.0);
        if changed
            && drag.last.is_none()
            && let Some(dr) = self.drawing.as_mut()
            && dr.id == Some(drag.handle.id)
            && dr.si == drag.handle.si
            && let Some(drawn) = dr.drawn()
        {
            match length::with_length(&drawn, drag.handle.at_start, drag.length) {
                Some(changed) => {
                    let (base, pieces) = dr.rebase(&changed).unwrap_or((changed, vec![]));
                    (dr.base, dr.pieces) = (Some(base), pieces);
                }
                None => self.drawing = None,
            }
        }
        vec![Action::Commit]
    }
}

impl Tool for QuickPenTool {
    fn id(&self) -> &'static str {
        ID
    }

    fn busy(&self) -> bool {
        self.press.is_some() || self.length.is_some()
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
                if self.press.is_some() || self.length.is_some() {
                    return vec![];
                }
                self.press_down(cx, p)
            }
            PointerKind::Drag if self.length.is_some() => {
                let Some(mut drag) = self.length else { return vec![] };
                if !drag.dragged && p.distance(drag.press) < cx.tol(DRAG_PX) {
                    return vec![];
                }
                drag.dragged = true;
                self.length = Some(drag);
                // The end follows the pointer, as far behind it as the press was.
                let piece = drag.handle.piece;
                let mut length = piece.length_dragged(drag.press, p, drag.length);
                // The last piece drawn goes on being a piece of the drawing, drawn again to its new
                // end; an arc piece keeps an arc size of at least 1°.
                let mut redrawn = None;
                let before = self.drawing.as_ref().and_then(|dr| {
                    let mut dr = dr.clone();
                    *dr.pieces.last_mut()? = drag.last?;
                    dr.last_tail(&dr.drawn()?)
                });
                let changed = if let (Some(last), Some(drawn)) = (drag.last, before) {
                    if let length::EndShape::Arc { radius, .. } = piece.shape {
                        length = length.max(MIN_TURN.to_radians() * radius);
                    }
                    let changed = length::with_length(&drawn, false, length);
                    redrawn = changed.as_ref().and_then(|sp| sp.anchors.last().map(|a| a.p)).map(|to| {
                        let turn = match piece.shape {
                            length::EndShape::Arc { radius, .. } => (length / radius).to_degrees().clamp(MIN_TURN, MAX_TURN),
                            length::EndShape::Line => last.turn,
                        };
                        Piece { to, turn, ..last }
                    });
                    changed
                } else {
                    let h = &drag.handle;
                    let sp = cx.doc.node(h.id).and_then(|n| n.path_data()).and_then(|pd| pd.subpaths.get(h.si));
                    sp.and_then(|sp| length::with_length(sp, h.at_start, length))
                };
                // A length that would take the path past the canvas is not taken: the end stays
                // where the drag last put it.
                if !changed.is_some_and(|sp| on_canvas(&sp)) {
                    return vec![];
                }
                let mut out = vec![];
                if !drag.begun {
                    drag.begun = true;
                    out.push(Action::Begin("Quick Pen".into()));
                }
                drag.length = length;
                self.length = Some(drag);
                match redrawn {
                    Some(pc) => {
                        if let Some(l) = self.drawing.as_mut().and_then(|dr| dr.pieces.last_mut()) {
                            *l = pc;
                        }
                        out.push(Action::Preview(COMMAND.into(), self.params(false)));
                    }
                    None => out.push(Action::Preview(LENGTH_COMMAND.into(), Self::length_params(&drag))),
                }
                out
            }
            PointerKind::Drag => {
                let Some(mut pr) = self.press else { return vec![] };
                if !pr.dragged && p.distance(pr.at) < cx.tol(DRAG_PX) {
                    return vec![];
                }
                pr.dragged = true;
                self.press = Some(pr);
                // The snapping leaves from the point before the piece being dragged.
                let from = self.drawing.as_ref().and_then(|dr| {
                    let sp = build(dr.base.as_ref()?, dr.pieces.split_last().map_or(&[][..], |(_, rest)| rest), false);
                    sp.anchors.last().map(|a| Leave::segment(cx, a.p, false).without_contact())
                });
                let q = self.snap.drag(cx, p, from.as_ref());
                if let Some(last) = self.drawing.as_mut().and_then(|dr| dr.pieces.last_mut()) {
                    last.to = q;
                }
                vec![Action::Preview(COMMAND.into(), self.params(false))]
            }
            PointerKind::Up => {
                self.snap.clear();
                if self.length.is_some() {
                    return self.end_length_drag(cx);
                }
                match self.press.take() {
                    Some(pr) if pr.begun => vec![Action::Commit],
                    _ => vec![],
                }
            }
            PointerKind::DoubleClick | PointerKind::TripleClick => vec![],
        }
    }

    fn notify(&mut self, cx: &ToolContext, what: &str) {
        if what == "created"
            && let [id] = cx.selection.objects.as_slice()
            && let Some(base) = cx.doc.node(*id).and_then(|n| n.path_data()).and_then(|pd| pd.subpaths.first())
            && let Some(dr) = self.drawing.as_mut()
            && dr.id.is_none()
        {
            (dr.id, dr.si, dr.base, dr.start) = (Some(*id), 0, Some(base.clone()), None);
        }
    }

    /// A path being drawn: Escape ends it.
    fn ends_on_escape(&self, cx: &ToolContext) -> bool {
        self.active(cx).is_some()
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
    fn drop_press(&mut self, _cx: &ToolContext) -> Vec<Action> {
        if let Some(drag) = self.length.take() {
            self.restore_last_piece(&drag);
        }
        if self.press.take().is_some_and(|pr| pr.begun)
            && let Some(dr) = self.drawing.as_mut()
        {
            dr.pieces.pop();
        }
        self.snap.clear();
        vec![]
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        match key {
            // Escape during a length drag takes the drag back and leaves the path being drawn; Enter
            // does nothing until the drag ends.
            ToolKey::Escape if self.length.is_some() => match self.length.take() {
                Some(drag) if drag.begun => {
                    self.restore_last_piece(&drag);
                    vec![Action::Cancel]
                }
                _ => vec![],
            },
            ToolKey::Enter if self.length.is_some() => vec![],
            ToolKey::Escape | ToolKey::Enter => self.end(),
            ToolKey::Letter('c') => {
                if self.last_sharp_arc(cx).is_none() {
                    return vec![];
                }
                if let Some(last) = self.drawing.as_mut().and_then(|dr| dr.pieces.last_mut()) {
                    last.flip = !last.flip;
                }
                vec![Action::Exec(COMMAND.into(), self.params(false))]
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
                vec![Action::Exec(COMMAND.into(), self.params(false))]
            }
            ToolKey::AltTap => {
                self.action = self.action.next();
                // A drag under way draws its piece with the new action.
                if self.press.is_some_and(|pr| pr.begun) {
                    let (action, turn) = (self.action, self.turn);
                    if let Some(last) = self.drawing.as_mut().and_then(|dr| dr.pieces.last_mut()) {
                        (last.stroke, last.turn, last.flip) = (action, turn, false);
                    }
                    return vec![Action::Preview(COMMAND.into(), self.params(false))];
                }
                vec![]
            }
            _ => vec![],
        }
    }

    fn deactivate(&mut self, _cx: &ToolContext) -> Vec<Action> {
        self.drawing = None;
        self.snap.clear();
        if self.length.take().is_some_and(|drag| drag.begun) {
            return vec![Action::Commit];
        }
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
        // The length handles: a line from each end to a handle circle beyond it; during a drag, the
        // one being dragged, where the changed end is.
        let handles: Vec<(Point, Point)> = match &self.length {
            Some(drag) => {
                let h = &drag.handle;
                let sp = cx.doc.node(h.id).and_then(|n| n.path_data()).and_then(|pd| pd.subpaths.get(h.si));
                let piece = sp.and_then(|sp| length::end_piece(sp, h.at_start));
                piece.map(|pc| (pc.end, pc.end + pc.out * cx.tol(LENGTH_HANDLE_PX))).into_iter().collect()
            }
            None if self.press.is_none() => self.length_handles(cx).iter().map(|h| (h.piece.end, h.at)).collect(),
            None => vec![],
        };
        for &(a, b) in &handles {
            o.push(Overlay::Line { a, b, color: FEEDBACK, dashed: false });
            o.push(Overlay::Handle { p: b, color: FEEDBACK });
        }
        let Some((id, sp)) = self.active(cx) else { return o };
        let on_handle = self.hover.is_some_and(|h| handles.iter().any(|&(_, at)| at.distance(h) <= point_tol(cx)));
        if self.press.is_none()
            && self.length.is_none()
            && !on_handle
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
        if self.length.is_some() || self.length_handle_at(cx, p).is_some() {
            return Cursor::Move;
        }
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

pub mod length;

#[cfg(test)]
mod tests;
