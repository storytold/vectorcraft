//! Object › Objects on Path: objects attached to a path and spread along it.
//!
//! An Objects on Path group is an ordinary group ([`NodeKind::Group`]) with an [`ObjectsOnPath`]
//! record in its extras ([`crate::node::NodeExtras::objects_on_path`]). One child is the path, and
//! the other children are the attached objects, kept where they are drawn: the group draws, hits,
//! saves and exports as a group. The record lists the objects in their order along the path, each
//! with the frame it is drawn in and the angle that frame is turned by. [`place`] spreads them
//! evenly along the path from its first anchor (the first object sits on that anchor, and the last
//! one sits on the end of an open path), puts the attachment point of each one
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
        Self { path, members, pivot: pivot.min(8), rotate: if rotate.is_finite() { rotate } else { 0.0 } }
    }
    /// The frames go with the group, and each frame's angle becomes the transformed path's
    /// direction at the object plus the tilt, so that [`place`] keeps each object's scale, shear
    /// and reflection when it spreads the objects along the transformed path again. A reflection
    /// also reverses Rotate: the objects keep the mirrored tilt to the mirrored path.
    pub fn transform(&mut self, a: Affine) {
        let before = tilt(self.rotate);
        if a.determinant() < 0.0 && self.rotate != 0.0 {
            self.rotate = -self.rotate;
        }
        let after = tilt(self.rotate);
        let [a0, a1, a2, a3, _, _] = a.as_coeffs();
        for m in &mut self.members {
            m.frame = a * m.frame;
            let (sin, cos) = (m.angle - before).sin_cos();
            let (x, y) = (a0 * cos + a2 * sin, a1 * cos + a3 * sin);
            if x.is_finite() && y.is_finite() && (x != 0.0 || y != 0.0) {
                m.angle = y.atan2(x) + after;
            }
        }
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
/// (not a guide) whose first subpath has some length.
pub fn attachable_path(n: &Node) -> Option<&PathData> {
    match &n.kind {
        NodeKind::Path { path, guide: false, .. } if Spine::new(path).is_some_and(|s| s.length() > 1e-9) => Some(path),
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

/// Where `n` objects attach along `path` (its first subpath): the point and the direction there
/// (radians, as `atan2` gives in the document's axes). They are spread evenly by length from the
/// first anchor: on an open path the last one is at its end, on a closed path the gaps go all the way
/// around. `None` for a path without length.
pub fn stops(path: &PathData, n: usize) -> Option<Vec<(Point, f64)>> {
    let spine = Spine::new(path)?;
    let len = spine.length();
    if !len.is_finite() || len <= 1e-9 {
        return None;
    }
    let closed = path.subpaths.first().is_some_and(|s| s.closed);
    let gaps = if closed { n } else { n.saturating_sub(1) }.max(1);
    Some((0..n).map(|i| spine.at_length(len * i as f64 / gaps as f64)).collect())
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
    let Some(path) = children.iter().find(|c| c.id == path_id).filter(|c| matches!(c.kind, NodeKind::Path { .. })) else {
        g.extras_mut().objects_on_path = None;
        return true;
    };
    let path = attachable_path(path).cloned();
    let at: HashMap<NodeId, usize> = children.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
    rec.members.retain(|m| m.id != path_id && at.contains_key(&m.id));
    let mut listed: HashSet<NodeId> = rec.members.iter().map(|m| m.id).collect();
    for c in children.iter() {
        if c.id != path_id && listed.insert(c.id) {
            rec.members.push(Member::new(c));
        }
    }
    let mut moved = false;
    if let Some(stops) = path.and_then(|p| stops(&p, rec.members.len())) {
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
        let s = stops(&open, 4).unwrap();
        let xs: Vec<f64> = s.iter().map(|(p, _)| p.x.round()).collect();
        assert_eq!(xs, [0.0, 100.0, 200.0, 300.0], "first at the start, last at the end of an open path");
        let closed = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
        let s = stops(&closed, 4).unwrap();
        let mut pts: Vec<(i64, i64)> = s.iter().map(|(p, _)| (p.x.round() as i64, p.y.round() as i64)).collect();
        pts.sort();
        assert_eq!(pts, [(0, 0), (0, 100), (100, 0), (100, 100)], "a closed path spreads them all the way round");
        assert!(stops(&shapes::line(Point::ZERO, Point::ZERO), 2).is_none());
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
        d.symbols.push(crate::Symbol { name: "Bar".into(), art: Arc::new(rect(9, Rect::new(0.0, 0.0, 40.0, 10.0))) });
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
