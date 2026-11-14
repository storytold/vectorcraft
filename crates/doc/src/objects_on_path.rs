//! Object › Objects on Path: objects attached to a path and spread along it.
//!
//! An Objects on Path group is an ordinary group ([`NodeKind::Group`]) with an [`ObjectsOnPath`]
//! record in its extras ([`crate::node::NodeExtras::objects_on_path`]). One child is the path, and
//! the other children are the attached objects, kept where they are drawn: the group draws, hits,
//! saves and exports as a group. The record lists the objects in their order along the path, each
//! with the frame it is drawn in and the angle that frame is turned by. [`place`] spreads them
//! along the path ([`stops`]): evenly from its first anchor by default (the first object sits on
//! that anchor, and the last one sits on the end of an open path), or [`ObjectsOnPath::spacing`]
//! apart from [`ObjectsOnPath::start`]. It puts the attachment point of each one
//! ([`ObjectsOnPath::pivot`]) on the path, and turns its frame to the path's direction there,
//! tilted by [`ObjectsOnPath::rotate`].
//!
//! Behavior: <https://helpx.adobe.com/illustrator/using/objects-on-path.html>.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_geom::{Affine, PathData, Point};

use crate::blend::Spine;
use crate::symbols::SymbolBoxes;
use crate::{Document, Node, NodeId, NodeKind};

/// The center of the nine reference points.
pub const CENTER: u8 = 4;

fn center() -> u8 {
    CENTER
}

/// What makes a group an Objects on Path group (see the module docs).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectsOnPath {
    /// The child of the group that is the path.
    pub path: NodeId,
    /// The attached objects, in their order along the path.
    #[serde(default)]
    pub members: Vec<Member>,
    /// Pivot: the reference point of each object that attaches to the path, 0 to 8, row by row
    /// from the top left of the object's bounds in its frame (4: the center).
    #[serde(default = "center")]
    pub pivot: u8,
    /// Rotate: the counter-clockwise degrees each object is turned from upright on the path. An
    /// upright object has its frame's x axis along the path's direction at its attachment point.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub rotate: f64,
    /// Space: the distance along the path (points) from each object's attachment point to the
    /// next one's. `None` (Auto) spreads the objects evenly ([`layout`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spacing: Option<f64>,
    /// The distance along the path (points) from its first anchor to the first object's
    /// attachment point.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub start: f64,
}

/// An object attached to the path of an Objects on Path group.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "MemberWire")]
pub struct Member {
    pub id: NodeId,
    /// The object's frame, mapped to the document: the object's art is drawn in this frame.
    pub frame: Affine,
    /// The angle the frame is turned by (radians, the document's axes): the path's direction at
    /// the object's attachment point plus the tilt from Rotate when it was placed, or the angle of
    /// the frame's x axis before then.
    pub angle: f64,
}

/// A [`Member`] as read from a file: without `angle`, the angle of the frame's x axis.
#[derive(Deserialize)]
struct MemberWire {
    id: NodeId,
    frame: Affine,
    #[serde(default)]
    angle: Option<f64>,
}

impl From<MemberWire> for Member {
    fn from(w: MemberWire) -> Self {
        let angle = w.angle.filter(|a| a.is_finite()).unwrap_or_else(|| x_angle(w.frame));
        Self { id: w.id, frame: w.frame, angle }
    }
}

/// The angle of `frame`'s x axis (radians, the document's axes).
fn x_angle(frame: Affine) -> f64 {
    let [a0, a1, ..] = frame.as_coeffs();
    a1.atan2(a0)
}

impl Member {
    /// `n` in its frame as drawn ([`upright_frame`]).
    pub fn new(n: &Node) -> Self {
        let frame = upright_frame(n);
        Self { id: n.id, frame, angle: x_angle(frame) }
    }
}

impl ObjectsOnPath {
    /// A record for the objects `members` (in their order along the path) attached to `path`, each
    /// in its frame as drawn ([`upright_frame`]).
    pub fn new(path: NodeId, members: &[&Node], pivot: u8, rotate: f64) -> Self {
        let members = members.iter().map(|n| Member::new(n)).collect();
        Self { path, members, pivot: pivot.min(8), rotate: if rotate.is_finite() { rotate } else { 0.0 }, spacing: None, start: 0.0 }
    }
    /// The frames go with the group, and each frame's angle becomes the transformed path's
    /// direction at the object plus the tilt, so that [`place`] keeps each object's scale, shear
    /// and reflection when it spreads the objects along the transformed path again. A reflection
    /// also reverses Rotate: the objects keep the mirrored tilt to the mirrored path. With `paths`,
    /// the group's path before the transform and as it is after it, the start and a Space value
    /// keep the first and last objects where the transform takes them on the path; a value that
    /// cannot be followed that way scales by how much the path's length changed. Without `paths`,
    /// they scale by the transform's average scale (the square root of its area scale). A
    /// value that would not be finite once scaled stays as it is.
    pub fn transform(&mut self, a: Affine, paths: Option<(&PathData, &PathData)>) {
        let before = tilt(self.rotate);
        let [a0, a1, a2, a3, _, _] = a.as_coeffs();
        if a.determinant() < 0.0 && self.rotate != 0.0 {
            self.rotate = -self.rotate;
        }
        // A move leaves the distances along the path as they are.
        let moves = a0 == 1.0 && a1 == 0.0 && a2 == 0.0 && a3 == 1.0;
        if !moves {
            let measured = paths.and_then(|(old, new)| Some((Measured::new(old)?, Measured::new(new)?)));
            let along = measured.as_ref().and_then(|(was, is)| {
                let s = is.len / was.len;
                (s.is_finite() && s > 0.0).then_some(s)
            });
            let scale = along.unwrap_or_else(|| a.determinant().abs().sqrt());
            if scale.is_finite() && scale > 0.0 {
                let scaled = |v: f64| Some(v * scale).filter(|s| s.is_finite()).unwrap_or(v);
                let moved = measured.as_ref().and_then(|(was, is)| self.moved_along(a, was, is, scale));
                let (spacing, start) = moved.unwrap_or((None, None));
                self.spacing = self.spacing.map(|g| spacing.unwrap_or_else(|| scaled(g)));
                self.start = start.unwrap_or_else(|| scaled(self.start));
            }
        }
        let after = tilt(self.rotate);
        for m in &mut self.members {
            m.frame = a * m.frame;
            let (sin, cos) = (m.angle - before).sin_cos();
            let (x, y) = (a0 * cos + a2 * sin, a1 * cos + a3 * sin);
            if x.is_finite() && y.is_finite() && (x != 0.0 || y != 0.0) {
                m.angle = y.atan2(x) + after;
            }
        }
    }
    /// The start and Space value on `new`, the path `old` after the transform `a` (regenerated, for
    /// a live shape), that keep the first object and the last one where `a` takes them: each one's
    /// point on `old`, mapped by `a`, measured along `new` near `scale` times its distance along
    /// `old`. Either one is `None` when it cannot be followed that way: a start before or beyond the
    /// ends of an open path, or a Space value of 0, one the path has no room for, or one with fewer
    /// than two objects. `None` for a start that is not finite.
    fn moved_along(&self, a: Affine, old: &Measured, new: &Measured, scale: f64) -> Option<(Option<f64>, Option<f64>)> {
        let (was, is) = (&old.spine, &new.spine);
        let (len, closed, new_len) = (old.len, old.closed, new.len);
        if !self.start.is_finite() {
            return None;
        }
        let n = self.members.len();
        let (s0, gap) = layout(len, closed, n, self.spacing, self.start);
        let follow = |s: f64| -> Option<f64> {
            let (p, _) = was.at_length(s);
            let t = is.nearest(a * p, Some(s * scale))?.1;
            t.is_finite().then_some(if closed { wrap(t, new_len) } else { t.clamp(0.0, new_len) })
        };
        let t0 = follow(s0)?;
        // On a closed path the start keeps the number of times it goes round, when that stays finite.
        let start =
            if closed { Some(t0 + ((self.start - s0) / len).round() * new_len).filter(|s| s.is_finite()) } else { (s0 == self.start).then_some(t0) };
        let spacing = self.spacing.filter(|g| n > 1 && g.is_finite() && *g > 0.0 && *g <= gap).and_then(|g| {
            let t1 = follow(nth_length(len, closed, s0, gap, n - 1))?;
            let steps = (n - 1) as f64;
            // Round a closed path, the span nearest the scaled one.
            let expected = steps * g * scale;
            let span = if closed { wrap(t1 - t0 - expected + new_len / 2.0, new_len) - new_len / 2.0 + expected } else { t1 - t0 };
            let g = span / steps;
            (g.is_finite() && g >= 0.0).then_some(g)
        });
        Some((spacing, start))
    }
    /// Move the object `member` to the place on the path of the object `target`: the objects
    /// between them each move one place toward where `member` was, except the objects for which
    /// `fixed` is true, which keep their places and are passed over. Only the order changes, not the
    /// Space or the start. Returns `member`'s new index, or `None` when either one is not an object
    /// of the group.
    pub fn move_member(&mut self, member: NodeId, target: NodeId, fixed: impl Fn(NodeId) -> bool) -> Option<usize> {
        let moves = |id: NodeId| id == member || id == target || !fixed(id);
        let slots: Vec<usize> = (0..self.members.len()).filter(|&i| moves(self.members[i].id)).collect();
        let from = slots.iter().position(|&i| self.members[i].id == member)?;
        let to = slots.iter().position(|&i| self.members[i].id == target)?;
        let mut moving: Vec<_> = slots.iter().map(|&i| self.members[i]).collect();
        let m = moving.remove(from);
        moving.insert(to, m);
        for (&i, m) in slots.iter().zip(moving) {
            self.members[i] = m;
        }
        Some(slots[to])
    }
    /// The children given new ids (a copy of the group): the path and the objects by their new ids.
    pub fn rename(&mut self, ids: &HashMap<NodeId, NodeId>) {
        let to = |id: NodeId| ids.get(&id).copied().unwrap_or(id);
        self.path = to(self.path);
        for m in &mut self.members {
            m.id = to(m.id);
        }
    }
    /// The group's children changed from `before` to `after`: a new child at the index of a member
    /// that is gone replaces it (Break Link to Symbol, for one), keeping its place on the path.
    pub fn follow_replacements(&mut self, before: &[Arc<Node>], after: &[Arc<Node>]) {
        let now: HashSet<NodeId> = after.iter().map(|c| c.id).collect();
        let then: HashSet<NodeId> = before.iter().map(|c| c.id).collect();
        let listed: HashSet<NodeId> = self.members.iter().map(|m| m.id).collect();
        let mut ids = HashMap::new();
        for (old, new) in before.iter().zip(after) {
            if listed.contains(&old.id) && !now.contains(&old.id) && !then.contains(&new.id) && new.id != self.path {
                ids.insert(old.id, new.id);
            }
        }
        if !ids.is_empty() {
            self.rename(&ids);
        }
    }
}

/// Is `n` an Objects on Path group?
pub fn is_objects_on_path(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { .. }) && n.extras().objects_on_path.is_some()
}

/// The path of `n` if it is a path object an Objects on Path group can attach objects to: a path
/// (not a guide) whose first subpath has a length ([`path_length`]).
pub fn attachable_path(n: &Node) -> Option<&PathData> {
    match &n.kind {
        NodeKind::Path { path, guide: false, .. } if path_length(path).is_some() => Some(path),
        _ => None,
    }
}

/// What keeps an object from going into an Objects on Path group, as its path or as one of its
/// objects ([`held`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Held {
    /// It is inside a compound path, blend, envelope, repeat or Intertwine group, as part of its art.
    Live,
    /// It is the clipping path of a clip group or of a layer with a clipping mask, or inside one.
    Clipping,
}

/// What keeps the object `id` of `d` where it is ([`Held`]), if anything: in an Objects on Path
/// group it would no longer be a part of the compound path, blend, envelope, repeat or Intertwine
/// group it is in, or of the clipping path it is or is in. The innermost reason counts. `None` for
/// an object that is neither a clipping path nor inside one and is in a layer directly or through
/// plain groups and clip groups, and for an id not in `d`.
pub fn held(d: &Document, id: NodeId) -> Option<Held> {
    let path = d.index_path(id)?;
    let mut chain: Vec<&Node> = Vec::with_capacity(path.len());
    let mut nodes = &d.layers;
    for &i in &path {
        let n: &Node = nodes.get(i)?;
        chain.push(n);
        if let Some(children) = n.children() {
            nodes = children;
        }
    }
    for (depth, n) in chain.iter().enumerate().rev() {
        let Some(parent) = depth.checked_sub(1).and_then(|up| chain.get(up)) else { break };
        if matches!(n.kind, NodeKind::Path { clipping: true, .. }) || (parent.clips() && path.get(depth) == Some(&0)) {
            return Some(Held::Clipping);
        }
        if matches!(
            parent.kind,
            NodeKind::Compound { .. } | NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Repeat(_) | NodeKind::Intertwine { .. }
        ) {
            return Some(Held::Live);
        }
    }
    None
}

/// The frame `n` is drawn in when it is attached: the page's axes turned by its rotation
/// ([`Node::bbox_angle`]) less the nearest multiple of 90°, about the center of its bounds. Placed in
/// that frame, an object stands square to the path or across it.
pub fn upright_frame(n: &Node) -> Affine {
    let a = n.bbox_angle;
    let off = if a.is_finite() { a - (a / 90.0).round() * 90.0 } else { 0.0 };
    match n.geometric_bounds() {
        Some(b) if off.abs() > 1e-9 => Affine::rotate_about(-off.to_radians(), b.center()),
        _ => Affine::IDENTITY,
    }
}

/// The length of `path`'s first subpath and whether it is closed. `None` for a path without
/// length, or with a length too large to be finite.
pub fn path_length(path: &PathData) -> Option<(f64, bool)> {
    Measured::new(path).map(|m| (m.len, m.closed))
}

/// A path's first subpath flattened ([`Spine`]), with its length and whether it is closed, so that
/// one flattening serves both measuring the path and finding points along it.
struct Measured {
    spine: Spine,
    len: f64,
    closed: bool,
}

impl Measured {
    /// `None` for a path without length, or with a length too large to be finite.
    fn new(path: &PathData) -> Option<Self> {
        let spine = Spine::new(path)?;
        let len = spine.length();
        let closed = path.subpaths.first().is_some_and(|s| s.closed);
        (len.is_finite() && len > 1e-9).then_some(Self { spine, len, closed })
    }
}

/// Where the first of `n` objects attaches along a path of length `len` (closed or not), and the
/// distance from each object's attachment point to the next one's, both along the path (points),
/// for Space `spacing` (`None`: Auto) from `start`:
/// - The first object is `start` from the first anchor: on an open path no further than its end,
///   and on a closed path less than once around.
/// - Auto spreads the objects evenly from there: on an open path the last one is at its end, and on
///   a closed path the gaps go all the way around.
/// - A Space value is kept to what the path has room for, so a shorter path draws the objects
///   closer while the value stays: on an open path the last object reaches its end at most, and on
///   a closed path the last one stops at least a gap short of the first.
///
/// A Space value that is negative or not finite counts as Auto, and a `start` that is not finite as 0.
pub fn layout(len: f64, closed: bool, n: usize, spacing: Option<f64>, start: f64) -> (f64, f64) {
    let start = match (start.is_finite(), closed) {
        (false, _) => 0.0,
        (true, true) => wrap(start, len),
        (true, false) => start.clamp(0.0, len.max(0.0)),
    };
    let room = if closed {
        len / n.max(1) as f64
    } else if n > 1 {
        (len - start) / (n - 1) as f64
    } else {
        0.0
    };
    let room = if room.is_finite() { room.max(0.0) } else { 0.0 };
    match spacing.filter(|g| g.is_finite() && *g >= 0.0) {
        Some(g) => (start, g.min(room)),
        None => (start, room),
    }
}

/// `s` taken round a closed path of length `len`: 0 or more, and less than `len`.
fn wrap(s: f64, len: f64) -> f64 {
    let w = s.rem_euclid(len);
    if w.is_finite() && w < len { w } else { 0.0 }
}

/// Where `n` objects attach along `path` (its first subpath): the point and the direction there
/// (radians, as `atan2` gives in the document's axes). They go along it as [`layout`] spaces them
/// for Space `spacing` (`None`: Auto) from `start`. `None` for a path without length.
pub fn stops(path: &PathData, n: usize, spacing: Option<f64>, start: f64) -> Option<Vec<(Point, f64)>> {
    let Measured { spine, len, closed } = Measured::new(path)?;
    let (start, gap) = layout(len, closed, n, spacing, start);
    Some((0..n).map(|i| spine.at_length(nth_length(len, closed, start, gap, i))).collect())
}

/// Where object `i` of `n` attaches along `path`, as [`stops`] places it, without the others.
/// `None` for an `i` of `n` or more, without measuring the path.
pub fn stop(path: &PathData, n: usize, spacing: Option<f64>, start: f64, i: usize) -> Option<(Point, f64)> {
    if i >= n {
        return None;
    }
    let Measured { spine, len, closed } = Measured::new(path)?;
    let (start, gap) = layout(len, closed, n, spacing, start);
    Some(spine.at_length(nth_length(len, closed, start, gap, i)))
}

/// The distance along a path of length `len` (closed or not) to object `i`, from `start` and
/// `gap` apart as [`layout`] gives them.
pub fn nth_length(len: f64, closed: bool, start: f64, gap: f64, i: usize) -> f64 {
    let s = start + gap * i as f64;
    if closed { wrap(s, len) } else { s.min(len) }
}

/// The tilt Rotate gives (radians, the document's axes, where y points down): `rotate` degrees
/// counter-clockwise on the page.
fn tilt(rotate: f64) -> f64 {
    -rotate.to_radians()
}

/// The frame that puts the attachment point `pivot` of `n` (drawn in `m`'s frame) at `at`, turned
/// by `angle` (radians, the document's axes) instead of `m`'s angle: the frame's own scale, shear
/// and reflection are kept. Symbol instances are measured by their art (`symbols`).
fn placed_frame(n: &Node, symbols: &SymbolBoxes, m: &Member, pivot: u8, at: Point, angle: f64) -> Option<Affine> {
    let det = m.frame.determinant();
    if !det.is_finite() || det.abs() < 1e-12 || !at.x.is_finite() || !at.y.is_finite() || !angle.is_finite() || !m.angle.is_finite() {
        return None;
    }
    let mut local = n.clone();
    local.transform(m.frame.inverse(), false);
    let q = vectorcraft_geom::reference_point(local.geometric_bounds_with(symbols)?, usize::from(pivot.min(8)));
    let [a0, a1, a2, a3, _, _] = m.frame.as_coeffs();
    let shape = Affine::rotate(-m.angle) * Affine::new([a0, a1, a2, a3, 0.0, 0.0]);
    Some(Affine::translate(at.to_vec2()) * Affine::rotate(angle) * shape * Affine::translate(-q.to_vec2()))
}

/// Is `a` the identity, give or take rounding?
fn negligible(a: Affine) -> bool {
    let [a0, a1, a2, a3, a4, a5] = a.as_coeffs();
    (a0 - 1.0).abs() < 1e-9 && a1.abs() < 1e-9 && a2.abs() < 1e-9 && (a3 - 1.0).abs() < 1e-9 && a4.abs() < 1e-6 && a5.abs() < 1e-6
}

/// Put the objects of Objects on Path group `g` on its path (see the module docs). Before that, the
/// record lets go of the objects that are no longer in the group, and children it does not list
/// (other than the path) join it at the end of the path. A group whose path is gone, or is no
/// longer a path object, becomes a plain group. Symbol instances are measured by their art
/// (`symbols`, from the document the group is in). Returns whether `g` changed.
pub fn place(g: &mut Node, symbols: &SymbolBoxes) -> bool {
    let Some(mut rec) = g.extras().objects_on_path.as_deref().cloned() else { return false };
    let old = rec.clone();
    let NodeKind::Group { children, .. } = &mut g.kind else { return false };
    let path_id = rec.path;
    let Some(path) = children.iter().find(|c| c.id == path_id).filter(|c| matches!(c.kind, NodeKind::Path { .. })).cloned() else {
        g.extras_mut().objects_on_path = None;
        return true;
    };
    // `stops` measures the path: on a path without length no object moves.
    let path = match &path.kind {
        NodeKind::Path { path, guide: false, .. } => Some(path),
        _ => None,
    };
    let at: HashMap<NodeId, usize> = children.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
    rec.members.retain(|m| m.id != path_id && at.contains_key(&m.id));
    let mut listed: HashSet<NodeId> = rec.members.iter().map(|m| m.id).collect();
    for c in children.iter() {
        if c.id != path_id && listed.insert(c.id) {
            rec.members.push(Member::new(c));
        }
    }
    let mut moved = false;
    if let Some(stops) = path.and_then(|p| stops(p, rec.members.len(), rec.spacing, rec.start)) {
        let turn = tilt(rec.rotate);
        for (m, (p, dir)) in rec.members.iter_mut().zip(stops) {
            let Some(c) = at.get(&m.id).and_then(|i| children.get_mut(*i)) else { continue };
            let Some(next) = placed_frame(c, symbols, m, rec.pivot, p, dir + turn) else { continue };
            let delta = next * m.frame.inverse();
            if !negligible(delta) {
                Arc::make_mut(c).transform(delta, false);
                *m = Member { id: m.id, frame: next, angle: dir + turn };
                moved = true;
            }
        }
    }
    if rec != old {
        g.extras_mut().objects_on_path = Some(Box::new(rec));
        return true;
    }
    moved
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_geom::{Rect, shapes};

    fn rect(id: u64, r: Rect) -> Node {
        Node::path(NodeId(id), shapes::rectangle(r), Default::default())
    }

    #[test]
    fn stops_spread_from_the_first_anchor() {
        let open = shapes::line(Point::new(0.0, 0.0), Point::new(300.0, 0.0));
        let s = stops(&open, 4, None, 0.0).unwrap();
        let xs: Vec<f64> = s.iter().map(|(p, _)| p.x.round()).collect();
        assert_eq!(xs, [0.0, 100.0, 200.0, 300.0], "first at the start, last at the end of an open path");
        let closed = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
        let s = stops(&closed, 4, None, 0.0).unwrap();
        let mut pts: Vec<(i64, i64)> = s.iter().map(|(p, _)| (p.x.round() as i64, p.y.round() as i64)).collect();
        pts.sort();
        assert_eq!(pts, [(0, 0), (0, 100), (100, 0), (100, 100)], "a closed path spreads them all the way round");
        assert!(stops(&shapes::line(Point::ZERO, Point::ZERO), 2, None, 0.0).is_none());
    }

    /// The x of each stop on the `len` pt line from (0, 0) to the right, to 1/1000 pt.
    fn xs_on_line(len: f64, n: usize, spacing: Option<f64>, start: f64) -> Vec<f64> {
        let open = shapes::line(Point::new(0.0, 0.0), Point::new(len, 0.0));
        stops(&open, n, spacing, start).unwrap().iter().map(|(p, _)| (p.x * 1000.0).round() / 1000.0).collect()
    }

    #[test]
    fn space_sets_the_distance_between_the_stops() {
        assert_eq!(xs_on_line(300.0, 4, Some(60.0), 0.0), [0.0, 60.0, 120.0, 180.0], "the first stays on the first anchor");
        assert_eq!(xs_on_line(300.0, 4, Some(60.0), 60.0), [60.0, 120.0, 180.0, 240.0], "centered on the path");
        assert_eq!(xs_on_line(300.0, 4, Some(0.0), 0.0), [0.0; 4], "no space stacks them");
        // More than the path has room for: the last one reaches the end.
        assert_eq!(xs_on_line(300.0, 4, Some(500.0), 0.0), [0.0, 100.0, 200.0, 300.0]);
        assert_eq!(xs_on_line(300.0, 4, Some(500.0), 150.0), [150.0, 200.0, 250.0, 300.0]);
        // A shorter path draws them closer, while the Space value stays to come back with the length.
        assert_eq!(xs_on_line(90.0, 4, Some(60.0), 0.0), [0.0, 30.0, 60.0, 90.0]);
        assert_eq!(layout(90.0, false, 4, Some(60.0), 0.0), (0.0, 30.0));
        assert_eq!(layout(300.0, false, 4, Some(60.0), 0.0), (0.0, 60.0));
        // Values that are not usable count as Auto, and a start beyond the end stops at it.
        assert_eq!(xs_on_line(300.0, 4, Some(f64::NAN), f64::INFINITY), [0.0, 100.0, 200.0, 300.0]);
        assert_eq!(xs_on_line(300.0, 4, Some(-5.0), 0.0), [0.0, 100.0, 200.0, 300.0]);
        assert_eq!(xs_on_line(300.0, 2, Some(10.0), 400.0), [300.0, 300.0]);
        assert_eq!(xs_on_line(300.0, 1, Some(10.0), 20.0), [20.0], "one object has no space to set");
        assert_eq!(xs_on_line(300.0, 0, Some(10.0), 0.0), Vec::<f64>::new());
        // One stop on its own is where the stops put it.
        let open = shapes::line(Point::new(0.0, 0.0), Point::new(300.0, 0.0));
        let all = stops(&open, 4, Some(60.0), 60.0).unwrap();
        for (i, s) in all.iter().enumerate() {
            assert_eq!(stop(&open, 4, Some(60.0), 60.0, i), Some(*s));
        }
        assert_eq!(stop(&open, 4, Some(60.0), 60.0, 4), None);
    }

    #[test]
    fn space_on_a_closed_path_wraps_at_the_first_anchor_and_stops_short_of_the_first_object() {
        // The square's outline is 400 pt round, from (0, 0) to the right and on clockwise.
        let closed = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
        let pts = |spacing, start| -> Vec<(i64, i64)> {
            stops(&closed, 4, spacing, start).unwrap().iter().map(|(p, _)| (p.x.round() as i64, p.y.round() as i64)).collect()
        };
        assert_eq!(pts(Some(50.0), 0.0), [(0, 0), (50, 0), (100, 0), (100, 50)]);
        // Centered on the first anchor: the span goes back past it.
        assert_eq!(pts(Some(50.0), -75.0), [(0, 75), (0, 25), (25, 0), (75, 0)]);
        // At most a quarter of the way round each: no two objects meet at the first anchor.
        assert_eq!(pts(Some(300.0), 0.0), [(0, 0), (100, 0), (100, 100), (0, 100)]);
        assert_eq!(layout(400.0, true, 4, Some(300.0), 400.0), (0.0, 100.0));
    }

    #[test]
    fn moving_an_object_shifts_the_ones_between_it_and_its_new_place() {
        let (a, b, c) =
            (rect(2, Rect::new(0.0, 0.0, 10.0, 10.0)), rect(3, Rect::new(20.0, 0.0, 30.0, 10.0)), rect(4, Rect::new(40.0, 0.0, 50.0, 10.0)));
        let rec = ObjectsOnPath { spacing: Some(30.0), start: 5.0, ..ObjectsOnPath::new(NodeId(1), &[&a, &b, &c], CENTER, 15.0) };
        let order = |r: &ObjectsOnPath| r.members.iter().map(|m| m.id.0).collect::<Vec<_>>();
        let free = |_: NodeId| false;
        let mut fwd = rec.clone();
        assert_eq!(fwd.move_member(NodeId(2), NodeId(4), free), Some(2));
        assert_eq!(order(&fwd), [3, 4, 2], "the first to the last place");
        assert_eq!((fwd.spacing, fwd.start, fwd.pivot, fwd.rotate), (rec.spacing, rec.start, rec.pivot, rec.rotate));
        let mut back = rec.clone();
        assert_eq!(back.move_member(NodeId(4), NodeId(2), free), Some(0));
        assert_eq!(order(&back), [4, 2, 3], "the last to the first place");
        let mut same = rec.clone();
        assert_eq!(same.move_member(NodeId(3), NodeId(3), free), Some(1));
        assert_eq!(same, rec, "onto its own place");
        let mut other = rec.clone();
        assert_eq!(other.move_member(NodeId(1), NodeId(3), free), None, "the path is not one of the objects");
        assert_eq!(other.move_member(NodeId(3), NodeId(9), free), None);
        assert_eq!(other, rec);
        // A fixed object between them keeps its place; the others pass over it.
        let mut over = rec.clone();
        assert_eq!(over.move_member(NodeId(2), NodeId(4), |id| id == NodeId(3)), Some(2));
        assert_eq!(order(&over), [4, 3, 2], "the first and last trade places round the fixed one");
        let mut back_over = rec.clone();
        assert_eq!(back_over.move_member(NodeId(4), NodeId(2), |id| id == NodeId(3)), Some(0));
        assert_eq!(order(&back_over), [4, 3, 2]);
    }

    #[test]
    fn a_group_saved_without_space_reads_as_auto() {
        let a = rect(2, Rect::new(0.0, 0.0, 10.0, 10.0));
        let rec = ObjectsOnPath::new(NodeId(1), &[&a], CENTER, 0.0);
        let v = serde_json::to_value(&rec).unwrap();
        assert!(v.get("spacing").is_none() && v.get("start").is_none(), "Auto is not written: {v}");
        let back: ObjectsOnPath = serde_json::from_value(v).unwrap();
        assert_eq!((back.spacing, back.start), (None, 0.0));
        let set = ObjectsOnPath { spacing: Some(60.0), start: 12.5, ..rec };
        let back: ObjectsOnPath = serde_json::from_value(serde_json::to_value(&set).unwrap()).unwrap();
        assert_eq!(back, set);
    }

    #[test]
    fn transforming_the_group_scales_its_space() {
        let a = rect(2, Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut rec = ObjectsOnPath { spacing: Some(60.0), start: 12.5, ..ObjectsOnPath::new(NodeId(1), &[&a], CENTER, 30.0) };
        rec.transform(Affine::new([-2.0, 0.0, 0.0, 2.0, 5.0, 5.0]), None);
        assert_eq!((rec.spacing, rec.start, rec.rotate), (Some(120.0), 25.0, -30.0));
        rec.transform(Affine::translate((40.0, 0.0)) * Affine::rotate(0.7), None);
        assert!((rec.spacing.unwrap() - 120.0).abs() < 1e-9 && (rec.start - 25.0).abs() < 1e-9, "{rec:?}");
        // Twice as wide and no higher: a level path is stretched twice along its length.
        let level = shapes::line(Point::new(0.0, 0.0), Point::new(300.0, 0.0));
        let stretch = Affine::new([2.0, 0.0, 0.0, 1.0, 0.0, 0.0]);
        rec.transform(stretch, Some((&level, &level.transformed(stretch))));
        assert!((rec.spacing.unwrap() - 240.0).abs() < 1e-9 && (rec.start - 50.0).abs() < 1e-9, "{rec:?}");
        let mut auto = ObjectsOnPath::new(NodeId(1), &[&a], CENTER, 0.0);
        auto.transform(Affine::scale(3.0), None);
        assert_eq!((auto.spacing, auto.start), (None, 0.0), "Auto stays Auto");
        let mut huge = ObjectsOnPath { spacing: Some(1e308), start: -1e308, ..auto };
        huge.transform(Affine::scale(2.0), None);
        assert_eq!((huge.spacing, huge.start), (Some(1e308), -1e308), "kept finite");
    }

    /// Stretching a path that turns a corner keeps the first and last objects where the stretch
    /// takes them, not where the whole path's change in length would.
    #[test]
    fn a_stretch_keeps_the_first_and_last_objects_where_it_takes_them() {
        // 100 pt to the right, then 100 pt down; twice as wide, the first part is 200 pt long.
        let points = [Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 100.0)];
        let corner = PathData::single(vectorcraft_geom::SubPath::polyline(&points, false));
        let stretch = Affine::scale_non_uniform(2.0, 1.0);
        let wide = corner.transformed(stretch);
        let (a, b) = (rect(2, Rect::new(0.0, 0.0, 10.0, 10.0)), rect(3, Rect::new(20.0, 0.0, 30.0, 10.0)));
        let mut rec = ObjectsOnPath { spacing: Some(40.0), start: 50.0, ..ObjectsOnPath::new(NodeId(1), &[&a, &b], CENTER, 0.0) };
        rec.transform(stretch, Some((&corner, &wide)));
        assert!((rec.start - 100.0).abs() < 1e-9 && (rec.spacing.unwrap() - 80.0).abs() < 1e-9, "{rec:?}");
        // Round a closed path, from before its first anchor: the start goes round as many times.
        let square = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
        let wide = square.transformed(stretch);
        let mut rec = ObjectsOnPath { spacing: Some(50.0), start: -75.0, ..ObjectsOnPath::new(NodeId(1), &[&a, &b], CENTER, 0.0) };
        let before = stops(&square, 2, rec.spacing, rec.start).unwrap();
        rec.transform(stretch, Some((&square, &wide)));
        let after = stops(&wide, 2, rec.spacing, rec.start).unwrap();
        for ((p, _), (q, _)) in before.iter().zip(&after) {
            assert!((stretch * *p).distance(*q) < 1e-9, "{p:?} went to {q:?}: {rec:?}");
        }
        assert!(rec.start < 0.0, "{rec:?}");
    }

    /// A start round a closed path so many times that scaling it would not be finite stays as it is.
    #[test]
    fn a_closed_path_start_too_large_to_scale_stays_finite() {
        let square = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
        let grow = Affine::scale(2.0);
        let a = rect(2, Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut rec = ObjectsOnPath { start: 1e308, ..ObjectsOnPath::new(NodeId(1), &[&a], CENTER, 0.0) };
        rec.transform(grow, Some((&square, &square.transformed(grow))));
        assert_eq!(rec.start, 1e308, "{rec:?}");
    }

    /// Stretching the group across its level path only keeps the objects where the stretch takes
    /// them: twice as far apart.
    #[test]
    fn a_group_stretched_along_its_path_keeps_the_objects_in_proportion() {
        let line = Node::path(NodeId(1), shapes::line(Point::new(0.0, 100.0), Point::new(300.0, 100.0)), Default::default());
        let (a, b) = (rect(2, Rect::new(0.0, 0.0, 10.0, 10.0)), rect(3, Rect::new(20.0, 0.0, 30.0, 10.0)));
        let mut g = Node::new(NodeId(4), NodeKind::Group { children: vec![Arc::new(line), Arc::new(a.clone()), Arc::new(b.clone())], clip: false });
        let rec = ObjectsOnPath { spacing: Some(20.0), start: 10.0, ..ObjectsOnPath::new(NodeId(1), &[&a, &b], CENTER, 0.0) };
        g.extras_mut().objects_on_path = Some(Box::new(rec));
        let empty = crate::Document::new(1.0, 1.0);
        let symbols = empty.symbol_boxes();
        place(&mut g, &symbols);
        g.transform(Affine::new([2.0, 0.0, 0.0, 1.0, 0.0, 0.0]), false);
        let rec = g.extras().objects_on_path.as_deref().cloned().unwrap();
        // The spine measures arc length along the curve, so within rounding of the exact values.
        assert!(rec.spacing.is_some_and(|d| (d - 40.0).abs() < 1e-9) && (rec.start - 20.0).abs() < 1e-9, "{rec:?}");
        place(&mut g, &symbols);
        let x = |id| g.children().and_then(|c| c.iter().find(|c| c.id == id).cloned()).unwrap().geometric_bounds().unwrap().center().x;
        assert!((x(NodeId(2)) - 20.0).abs() < 1e-9 && (x(NodeId(3)) - 60.0).abs() < 1e-9, "{} {}", x(NodeId(2)), x(NodeId(3)));
    }

    #[test]
    fn length_near_measures_along_the_path_and_past_its_ends() {
        // An L: 400 pt to the right, then 300 pt up.
        let points = [Point::new(0.0, 300.0), Point::new(400.0, 300.0), Point::new(400.0, 0.0)];
        let l = PathData::single(vectorcraft_geom::SubPath::polyline(&points, false));
        let spine = Spine::new(&l).unwrap();
        assert!((spine.length_near(Point::new(300.0, 320.0), None) - 300.0).abs() < 1e-9);
        assert!((spine.length_near(Point::new(420.0, 100.0), None) - 600.0).abs() < 1e-9);
        assert!((spine.length_near(Point::new(400.0, -26.0), None) - 726.0).abs() < 1e-9, "beyond the end");
        assert!((spine.length_near(Point::new(-50.0, 300.0), None) + 50.0).abs() < 1e-9, "before the start");
        // A path that turns back near its start: beyond its end, the pointer is nearer the first
        // segment than the last one, but on the line of the last one.
        let hook = [Point::new(0.0, 0.0), Point::new(200.0, 0.0), Point::new(200.0, 10.0), Point::new(100.0, 10.0)];
        let hook = Spine::new(&PathData::single(vectorcraft_geom::SubPath::polyline(&hook, false))).unwrap();
        assert!((hook.length_near(Point::new(80.0, 10.0), None) - 330.0).abs() < 1e-9);
        assert!((hook.length_near(Point::new(50.0, 10.0), None) - 360.0).abs() < 1e-9);
        let closed = Spine::new(&shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0))).unwrap();
        assert!((closed.length_near(Point::new(-30.0, -30.0), None)).abs() < 1e-9, "a closed path has no ends");
    }

    #[test]
    fn length_near_a_point_followed_stays_on_that_part_of_the_path() {
        // A narrow U, open on the left: its ends are 20 pt apart, 620 pt from each other along it.
        let u = [Point::new(0.0, 0.0), Point::new(300.0, 0.0), Point::new(300.0, 20.0), Point::new(0.0, 20.0)];
        let u = Spine::new(&PathData::single(vectorcraft_geom::SubPath::polyline(&u, false))).unwrap();
        assert!((u.length_near(Point::new(-26.0, 20.0), None) - 646.0).abs() < 1e-9, "beyond the end");
        // Nearer the first segment's line, the pointer stays beyond the end it was following.
        assert!((u.length_near(Point::new(-26.0, 8.0), None) + 26.0).abs() < 1e-9);
        assert!((u.length_near(Point::new(-26.0, 8.0), Some(646.0)) - 646.0).abs() < 1e-9);
        // Following the path round, it goes on from where it was.
        assert!((u.length_near(Point::new(305.0, 10.0), Some(312.0)) - 310.0).abs() < 1e-9);
        // Round a closed path's first anchor: from 10 pt short of it to 20 pt past it.
        let square = Spine::new(&shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0))).unwrap();
        let (at, _) = square.at_length(390.0);
        assert!((square.length_near(at, None) - 390.0).abs() < 1e-9);
        assert!((square.length_near(Point::new(20.0, -5.0), Some(390.0)) - 20.0).abs() < 1e-9);
    }

    #[test]
    fn upright_frames_round_the_rotation_to_a_right_angle() {
        let mut n = rect(1, Rect::new(0.0, 0.0, 10.0, 10.0));
        assert_eq!(upright_frame(&n), Affine::IDENTITY);
        n.bbox_angle = 90.0;
        assert_eq!(upright_frame(&n), Affine::IDENTITY);
        n.bbox_angle = 30.0;
        let f = upright_frame(&n);
        let [a0, a1, ..] = f.as_coeffs();
        assert!((a1.atan2(a0).to_degrees() + 30.0).abs() < 1e-9, "a frame turned 30° counter-clockwise");
    }

    #[test]
    fn a_symbol_instance_is_placed_by_its_art() {
        // The art is a 40 × 10 box to the right of and below the origin, not the 20 × 20 square
        // around it that instances are measured by without the symbols.
        let mut d = crate::Document::new(400.0, 400.0);
        d.symbols.push(crate::Symbol { dynamic: false, name: "Bar".into(), art: Arc::new(rect(9, Rect::new(0.0, 0.0, 40.0, 10.0))) });
        let line = Node::path(NodeId(1), shapes::line(Point::new(100.0, 300.0), Point::new(300.0, 300.0)), Default::default());
        let instance = Node::new(NodeId(2), NodeKind::SymbolInstance { symbol: "Bar".into(), xf: Affine::IDENTITY });
        let other = rect(3, Rect::new(0.0, 0.0, 10.0, 10.0));
        let members = [&instance, &other];
        let mut g = Node::new(
            NodeId(4),
            NodeKind::Group { children: vec![Arc::new(line), Arc::new(instance.clone()), Arc::new(other.clone())], clip: false },
        );
        g.extras_mut().objects_on_path = Some(Box::new(ObjectsOnPath::new(NodeId(1), &members, 7, 0.0)));
        let symbols = d.symbol_boxes();
        assert!(place(&mut g, &symbols));
        let placed = g.children().and_then(|c| c.iter().find(|c| c.id == NodeId(2)).cloned()).unwrap();
        let b = placed.geometric_bounds_with(&symbols).unwrap();
        assert!((b.y1 - 300.0).abs() < 1e-9 && (b.center().x - 100.0).abs() < 1e-9, "the art's bottom middle on the start: {b:?}");
    }

    #[test]
    fn a_member_saved_without_its_angle_reads_the_angle_of_its_frame() {
        let m = Member { id: NodeId(2), frame: Affine::rotate(0.5), angle: 0.5 };
        let mut v = serde_json::to_value(m).unwrap();
        assert_eq!(serde_json::from_value::<Member>(v.clone()).unwrap(), m);
        v.as_object_mut().unwrap().remove("angle");
        let back: Member = serde_json::from_value(v).unwrap();
        assert!((back.angle - 0.5).abs() < 1e-12, "{back:?}");
    }

    /// What `f` returns, and how many times it flattened a path ([`Spine::new`]).
    fn flattenings<T>(f: impl FnOnce() -> T) -> (T, usize) {
        crate::blend::SPINES_BUILT.with(|n| n.set(0));
        let out = f();
        (out, crate::blend::SPINES_BUILT.with(|n| n.get()))
    }

    #[test]
    fn stops_and_stop_flatten_the_path_once() {
        let open = shapes::line(Point::new(0.0, 0.0), Point::new(300.0, 0.0));
        let closed = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
        for path in [&open, &closed] {
            let (all, count) = flattenings(|| stops(path, 4, Some(40.0), 25.0));
            let all = all.unwrap();
            assert_eq!((all.len(), count), (4, 1));
            for (i, s) in all.iter().enumerate() {
                assert_eq!(flattenings(|| stop(path, 4, Some(40.0), 25.0, i)), (Some(*s), 1), "object {i}");
            }
            assert_eq!(flattenings(|| stop(path, 4, Some(40.0), 25.0, 4)), (None, 0), "an index past the last object");
            assert_eq!(flattenings(|| stop(path, 0, None, 0.0, 0)), (None, 0), "no objects");
        }
        let point = shapes::line(Point::ZERO, Point::ZERO);
        assert!(stops(&point, 2, None, 0.0).is_none() && stop(&point, 2, None, 0.0, 0).is_none());
    }

    #[test]
    fn transforming_the_group_flattens_each_path_once() {
        let a = rect(2, Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = rect(3, Rect::new(20.0, 0.0, 30.0, 10.0));
        let level = shapes::line(Point::new(0.0, 0.0), Point::new(300.0, 0.0));
        let stretch = Affine::scale_non_uniform(2.0, 1.0);
        let wide = level.transformed(stretch);
        let mut rec = ObjectsOnPath { spacing: Some(40.0), start: 10.0, ..ObjectsOnPath::new(NodeId(1), &[&a, &b], CENTER, 0.0) };
        let ((), count) = flattenings(|| rec.transform(stretch, Some((&level, &wide))));
        assert_eq!(count, 2, "the path before the transform and after it");
        assert!((rec.start - 20.0).abs() < 1e-9 && (rec.spacing.unwrap() - 80.0).abs() < 1e-9, "{rec:?}");
        let moved = level.transformed(Affine::translate((5.0, 5.0)));
        let ((), count) = flattenings(|| rec.transform(Affine::translate((5.0, 5.0)), Some((&level, &moved))));
        assert_eq!(count, 0, "a move leaves the distances along the path as they are");
    }

    #[test]
    fn placing_the_objects_flattens_the_path_once() {
        let line = Node::path(NodeId(1), shapes::line(Point::new(0.0, 100.0), Point::new(300.0, 100.0)), Default::default());
        let (a, b) = (rect(2, Rect::new(0.0, 0.0, 10.0, 10.0)), rect(3, Rect::new(20.0, 0.0, 30.0, 10.0)));
        let mut g = Node::new(NodeId(4), NodeKind::Group { children: vec![Arc::new(line), Arc::new(a.clone()), Arc::new(b.clone())], clip: false });
        g.extras_mut().objects_on_path = Some(Box::new(ObjectsOnPath::new(NodeId(1), &[&a, &b], CENTER, 0.0)));
        let empty = crate::Document::new(1.0, 1.0);
        let symbols = empty.symbol_boxes();
        assert_eq!(flattenings(|| place(&mut g, &symbols)), (true, 1));
        let x = |id| g.children().and_then(|c| c.iter().find(|c| c.id == id).cloned()).unwrap().geometric_bounds().unwrap().center().x;
        assert!((x(NodeId(2))).abs() < 1e-9 && (x(NodeId(3)) - 300.0).abs() < 1e-9, "{} {}", x(NodeId(2)), x(NodeId(3)));
    }

    /// A path whose length adds up past the largest finite number cannot be measured, so nothing
    /// attaches to it.
    #[test]
    fn a_path_too_long_to_measure_is_not_attachable() {
        let huge = Node::path(NodeId(1), shapes::line(Point::new(-1e308, 0.0), Point::new(1e308, 0.0)), Default::default());
        let NodeKind::Path { path, .. } = &huge.kind else { panic!("a path") };
        assert!(path_length(path).is_none() && stops(path, 2, None, 0.0).is_none());
        assert!(attachable_path(&huge).is_none());
        let square = rect(2, Rect::new(0.0, 0.0, 10.0, 10.0));
        assert!(attachable_path(&square).is_some());
    }

    #[test]
    fn a_group_without_its_path_is_a_plain_group() {
        let a = rect(2, Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut g = Node::new(NodeId(3), NodeKind::Group { children: vec![Arc::new(a.clone())], clip: false });
        g.extras_mut().objects_on_path = Some(Box::new(ObjectsOnPath::new(NodeId(1), &[&a], CENTER, 0.0)));
        let d = crate::Document::new(100.0, 100.0);
        assert!(place(&mut g, &d.symbol_boxes()));
        assert!(!is_objects_on_path(&g));
    }
}
