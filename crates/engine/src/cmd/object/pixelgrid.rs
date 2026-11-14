//! Transform panel › Align to Pixel Grid: an object's pixel-aligned property, and the snapping that
//! keeps the horizontal and vertical segments of a pixel-aligned object on the pixel grid
//! (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/pixel-preview-mode-for-web-graphics.html>).

use vectorcraft_doc::{AppearanceItem, LiveShape, StrokeAlign};

use super::{
    Affine, Arc, Document, EngineError, Node, NodeId, NodeKind, Paint, PathData, Rect, Result, Scaling, Session, Value, Vec2, bad, json,
    selected_roots,
};
use crate::cmd::{given_objects, switch_or};

const C: &str = "object.setPixelAligned";

/// How far apart two coordinates may be and still count as one, in points: segments this close to
/// horizontal or vertical count as horizontal or vertical.
const EPS: f64 = 1e-6;

/// Whether `n` can be aligned to the pixel grid: a path that is not a guide, or a compound path.
/// Type, images, groups and other objects cannot.
pub(crate) fn can_align(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. })
}

/// For an object being drawn into `d`: when the document's Align New Objects to Pixel Grid is on
/// and [`can_align`] accepts `n`, turn `n`'s pixel-aligned property on. The edit that adds it snaps
/// it to the pixel grid ([`refresh`]). Pasted, duplicated, placed and imported objects do not come
/// through here.
pub(crate) fn align_new_object(d: &Document, n: &mut Node) {
    if d.setup.align_new_objects_to_pixel_grid && can_align(n) {
        n.extras_mut().pixel_aligned = true;
    }
}

/// `object.setPixelAligned`: turn the pixel-aligned property of the objects given (or the selected
/// objects) on or off. Turning it on snaps them to the pixel grid at once. Every object must be
/// one [`can_align`] accepts, else nothing changes. When every object already has the value, the
/// document and its undo history stay as they are.
pub(super) fn set_pixel_aligned(s: &mut Session, p: &Value) -> Result<Value> {
    let on = switch_or(p, "on", true, C)?;
    let ids = match given_objects(s, p, C)? {
        Some(ids) => ids,
        None => selected_roots(s)?,
    };
    let d = &s.doc()?.doc;
    let mut changed: Vec<NodeId> = vec![];
    for id in &ids {
        let n = d.node(*id).ok_or(EngineError::NoNode(*id))?;
        // A path inside a compound path snaps with the compound path, whose property it takes.
        let in_compound = d.parent_of(*id).and_then(|p| d.node(p)).is_some_and(|p| matches!(p.kind, NodeKind::Compound { .. }));
        if !can_align(n) || in_compound {
            return Err(bad(C, "only paths and compound paths can be aligned to the pixel grid"));
        }
        if n.extras().pixel_aligned != on && !changed.contains(id) {
            changed.push(*id);
        }
    }
    if changed.is_empty() {
        return Ok(json!({ "ids": [] }));
    }
    s.edit("Align to Pixel Grid", |d, _| {
        for id in &changed {
            let n = d.node_mut(*id).ok_or(EngineError::NoNode(*id))?;
            n.extras_mut().pixel_aligned = on;
            if on {
                snap_object(n, Scaling::default());
            }
        }
        Ok(())
    })?;
    Ok(json!({ "ids": changed.iter().map(|i| i.0).collect::<Vec<_>>() }))
}

/// After a transform of `n` with `sc`: snap `n` and every object in it that is pixel-aligned to the
/// pixel grid, the art of linked opacity masks included. Objects that are not pixel-aligned stay as
/// they are.
pub(crate) fn keep_pixel_aligned(n: &mut Node, sc: Scaling) {
    let mut found = vec![];
    aligned_within(n, &mut vec![], &mut found);
    for at in found {
        if let Some(a) = node_at(n, &at) {
            snap_object(a, sc);
        }
    }
}

/// One step from an object down to an object it holds: its child at an index, or the art of its
/// linked opacity mask.
#[derive(Clone, Copy)]
enum Step {
    Child(usize),
    Mask,
}

/// Add to `out` the steps from the object [`keep_pixel_aligned`] started at to `n` (`at`) and to
/// every pixel-aligned object in `n`, in the art of linked opacity masks included, `n` first. One
/// walk visits each object once, so nothing is copied that does not snap.
fn aligned_within(n: &Node, at: &mut Vec<Step>, out: &mut Vec<Vec<Step>>) {
    if n.extras().pixel_aligned {
        out.push(at.clone());
    }
    // The paths of a pixel-aligned compound path snap with it, but their opacity masks still need a visit.
    for (i, c) in n.children().map_or(&[][..], Vec::as_slice).iter().enumerate() {
        at.push(Step::Child(i));
        aligned_within(c, at, out);
        at.pop();
    }
    if let Some(m) = n.extras().mask.as_ref()
        && m.linked
    {
        at.push(Step::Mask);
        aligned_within(&m.art, at, out);
        at.pop();
    }
}

/// The object that `steps` lead to from `n`, made unique for editing.
fn node_at<'a>(n: &'a mut Node, steps: &[Step]) -> Option<&'a mut Node> {
    let mut cur = n;
    for s in steps {
        cur = match s {
            Step::Child(i) => Arc::make_mut(cur.children_mut()?.get_mut(*i)?),
            Step::Mask => Arc::make_mut(&mut cur.extras_if_any()?.mask.as_mut()?.art),
        };
    }
    Some(cur)
}

/// After a transform of object `id` with `sc`: when `id` is a path of a pixel-aligned compound path
/// (moved on its own, as with Group Selection), snap that compound path to the pixel grid again.
/// The compound path holds the property, so [`keep_pixel_aligned`] on the path alone does nothing.
pub(crate) fn keep_compound_aligned(d: &mut Document, id: NodeId, sc: Scaling) {
    let Some(parent) = d.parent_of(id) else { return };
    if let Some(n) = d.node_mut(parent)
        && matches!(n.kind, NodeKind::Compound { .. })
        && n.extras().pixel_aligned
    {
        snap_object(n, sc);
    }
}

/// After any edit ([`Session::edit`]): snap each pixel-aligned object that the edit changed or added
/// (its node in `doc` is not the one in `before`), with the pixel-aligned art of its linked opacity
/// mask, to the pixel grid. Objects the edit did not touch are not visited.
pub(crate) fn refresh(before: &Document, doc: &mut Document) {
    let mut ids = vec![];
    changed_aligned(&before.layers, &doc.layers, &mut ids);
    for id in ids {
        if let Some(n) = doc.node_mut(id) {
            keep_pixel_aligned(n, Scaling::default());
        }
    }
}

/// After a pass that moved what objects `ids` hold (Objects on Path placing its members): snap the
/// pixel-aligned objects in them to the pixel grid again.
pub(crate) fn keep_aligned_in(doc: &mut Document, ids: &[NodeId]) {
    for id in ids {
        if doc.node(*id).is_some_and(holds_aligned)
            && let Some(n) = doc.node_mut(*id)
        {
            keep_pixel_aligned(n, Scaling::default());
        }
    }
}

/// The objects of `after` that differ from their counterparts in `before` (or are new) and are
/// pixel-aligned or hold pixel-aligned art in a linked opacity mask. Inside such an object the
/// walk stops: [`keep_pixel_aligned`] visits what it holds.
fn changed_aligned(before: &[Arc<Node>], after: &[Arc<Node>], out: &mut Vec<NodeId>) {
    let mut by_id: Option<std::collections::HashMap<NodeId, &Arc<Node>>> = None;
    for (i, a) in after.iter().enumerate() {
        let b = match before.get(i) {
            Some(b) if b.id == a.id => Some(b),
            _ => by_id.get_or_insert_with(|| before.iter().map(|b| (b.id, b)).collect()).get(&a.id).copied(),
        };
        if b.is_some_and(|b| Arc::ptr_eq(b, a)) {
            continue;
        }
        if a.extras().pixel_aligned || a.extras().mask.as_ref().is_some_and(|m| m.linked && holds_aligned(&m.art)) {
            out.push(a.id);
        } else if let Some(children) = a.children() {
            changed_aligned(b.and_then(|b| b.children()).map_or(&[], Vec::as_slice), children, out);
        }
    }
}

/// After an edit that replaces object `id`'s geometry: snap it, or the pixel-aligned compound path
/// that holds it, to the pixel grid again when it is pixel-aligned.
pub(crate) fn resnap(d: &mut Document, id: NodeId) {
    if let Some(n) = d.node_mut(id) {
        snap_if_aligned(n);
    }
    keep_compound_aligned(d, id, Scaling::default());
}

/// After an edit of `n`'s strokes: when `n` is pixel-aligned, round its stroke weights and snap it to
/// the pixel grid again, since a new weight or alignment can move where the stroke's edges fall.
pub(crate) fn snap_if_aligned(n: &mut Node) {
    if n.extras().pixel_aligned {
        snap_object(n, Scaling::default());
    }
}

/// Whether `n` or an object in it, or in the art of a linked opacity mask in it, is pixel-aligned.
/// It goes only where a transform of `n` goes: not into unlinked masks.
fn holds_aligned(n: &Node) -> bool {
    n.extras().pixel_aligned
        || n.extras().mask.as_ref().is_some_and(|m| m.linked && holds_aligned(&m.art))
        || n.children().is_some_and(|ch| ch.iter().any(|c| holds_aligned(c)))
}

/// Snap pixel-aligned object `n` to the pixel grid: its stroke weights become whole points (at
/// least 1), and its horizontal and vertical straight segments move to whole points, or to half
/// points when a centered stroke has an odd weight, so that the stroke's edges fall on the grid.
/// A compound path snaps each of its paths. Placed gradients and linked opacity masks of an object
/// whose anchors moved follow it ([`follow_snap`]).
fn snap_object(n: &mut Node, sc: Scaling) {
    let half = round_strokes(n);
    let before = n.geometric_bounds();
    let mut moved = false;
    if let NodeKind::Compound { children, .. } = &mut n.kind {
        // A path is copied for editing only when the snap moves it.
        for c in children.iter_mut() {
            if path_snaps(c, half) {
                let was = c.geometric_bounds();
                let anchors = snaps_anchors(c);
                let snapped = Arc::make_mut(c);
                snap_path_node(snapped, half, sc);
                if anchors {
                    follow_snap(snapped, was, sc);
                }
                moved = true;
            }
        }
    } else if snaps_anchors(n) {
        moved = snap_path_node(n, half, sc);
    } else {
        // A live shape is moved with `Node::transform`, which carries its gradients and mask along.
        snap_path_node(n, half, sc);
    }
    if moved {
        follow_snap(n, before, sc);
    }
}

/// Whether a snap moves the anchors of path `n` itself (a plain path, or one with Live Corners)
/// rather than transforming it as a whole (another live shape).
fn snaps_anchors(n: &Node) -> bool {
    matches!(&n.kind, NodeKind::Path { live, .. } if live.as_deref().is_none_or(|l| matches!(l, LiveShape::Path { .. })))
}

/// After the anchors of `n` moved in a snap, from bounds `before`: its placed gradients are fitted
/// to its new bounds, its patterns move and scale with those bounds when `sc` transforms patterns,
/// and the art of its linked opacity mask moves and scales with them (and snaps when it is
/// pixel-aligned itself).
fn follow_snap(n: &mut Node, before: Option<Rect>, sc: Scaling) {
    let (Some(f), Some(t)) = (before, n.geometric_bounds()) else { return };
    if f == t {
        return;
    }
    n.appearance.rebase_gradients(f, t);
    if sc.patterns {
        vectorcraft_doc::pattern::transform_pattern_paints(n, rect_to_rect(f, t));
    }
    if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut())
        && m.linked
    {
        let art = Arc::make_mut(&mut m.art);
        art.transform(rect_to_rect(f, t), sc);
        keep_pixel_aligned(art, sc);
    }
}

/// Round the stroke weights of `n` to whole points, at least 1 (strokes without weight stay
/// without). Returns whether a visible, painted centered stroke has an odd weight.
fn round_strokes(n: &mut Node) -> bool {
    let rounds = |w: f64| if w > 0.0 { w.round().max(1.0) } else { w };
    let needs = n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(st) if rounds(st.width) != st.width));
    if needs {
        for item in &mut n.appearance.items {
            if let AppearanceItem::Stroke(st) = item {
                st.width = rounds(st.width);
            }
        }
    }
    n.appearance.items.iter().any(|i| {
        matches!(i, AppearanceItem::Stroke(st) if st.visible
            && !matches!(st.paint, Paint::None)
            && st.width > 0.0
            && st.width % 2.0 == 1.0
            && st.align == StrokeAlign::Center)
    })
}

/// Snap path object `n`'s horizontal and vertical straight segments ([`snap_segments`]). A path
/// with Live Corners snaps the outline its corners are cut from. Another live shape is moved and
/// scaled as a whole instead, from its path's bounds to the bounds of the snapped path, so that it
/// stays a live shape. Returns whether it changed.
fn snap_path_node(n: &mut Node, half: bool, sc: Scaling) -> bool {
    let NodeKind::Path { path, live, .. } = &mut n.kind else { return false };
    let Some(shape) = live.as_deref_mut() else { return snap_segments(path, half) };
    if let LiveShape::Path { base, .. } = shape {
        if !snap_segments(base, half) {
            return false;
        }
        *path = shape.to_path();
        return true;
    }
    let mut snapped = path.clone();
    if !snap_segments(&mut snapped, half) {
        return false;
    }
    let (Some(from), Some(to)) = (path.bounds(), snapped.bounds()) else { return false };
    n.transform(rect_to_rect(from, to), Scaling { keep_corners: true, patterns: sc.patterns, ..Scaling::default() });
    true
}

/// Whether [`snap_path_node`] would change path object `n`: whether an anchor of the path it snaps
/// moves. Another kind of object is not changed.
fn path_snaps(n: &Node, half: bool) -> bool {
    let NodeKind::Path { path, live, .. } = &n.kind else { return false };
    match live.as_deref() {
        Some(LiveShape::Path { base, .. }) => segment_moves(base, half).is_some(),
        _ => segment_moves(path, half).is_some(),
    }
}

/// The move and scale that takes `from` onto `to` (a side without size is only moved).
fn rect_to_rect(from: Rect, to: Rect) -> Affine {
    let sx = if from.width() > EPS { to.width() / from.width() } else { 1.0 };
    let sy = if from.height() > EPS { to.height() / from.height() } else { 1.0 };
    Affine::translate((to.x0, to.y0)) * Affine::scale_non_uniform(sx, sy) * Affine::translate((-from.x0, -from.y0))
}

/// Move the anchors of `path`'s horizontal straight segments to whole points vertically and the
/// anchors of its vertical ones to whole points horizontally (`half`: to half points), each anchor
/// with its handles, so curves and diagonal segments keep their shape apart from the anchors they
/// share with those segments. Coordinates that differ stay at least 1 point apart within a subpath,
/// so no side collapses. Returns whether an anchor moved.
pub(crate) fn snap_segments(path: &mut PathData, half: bool) -> bool {
    let Some(moves) = segment_moves(path, half) else { return false };
    for (sp, m) in path.subpaths.iter_mut().zip(moves) {
        for (a, d) in sp.anchors.iter_mut().zip(m) {
            if d.x != 0.0 || d.y != 0.0 {
                a.translate(d);
            }
        }
    }
    true
}

/// How far [`snap_segments`] moves each anchor of each subpath of `path`, or `None` when it moves
/// no anchor.
fn segment_moves(path: &PathData, half: bool) -> Option<Vec<Vec<Vec2>>> {
    let mut changed = false;
    let mut moves = Vec::with_capacity(path.subpaths.len());
    for sp in &path.subpaths {
        let n = sp.anchors.len();
        let (mut snap_x, mut snap_y) = (vec![false; n], vec![false; n]);
        for i in 0..sp.segment_count() {
            if !sp.segment_is_line(i) {
                continue;
            }
            let (a, b) = (i % n, (i + 1) % n);
            let (p, q) = (sp.anchors[a].p, sp.anchors[b].p);
            let (dx, dy) = ((q.x - p.x).abs(), (q.y - p.y).abs());
            if dy <= EPS && dx > EPS {
                (snap_y[a], snap_y[b]) = (true, true);
            } else if dx <= EPS && dy > EPS {
                (snap_x[a], snap_x[b]) = (true, true);
            }
        }
        let xs = snapped_values(sp.anchors.iter().zip(&snap_x).filter(|(_, s)| **s).map(|(a, _)| a.p.x), half);
        let ys = snapped_values(sp.anchors.iter().zip(&snap_y).filter(|(_, s)| **s).map(|(a, _)| a.p.y), half);
        let mut m = Vec::with_capacity(n);
        for (i, a) in sp.anchors.iter().enumerate() {
            let dx = if snap_x[i] { snapped(&xs, a.p.x) - a.p.x } else { 0.0 };
            let dy = if snap_y[i] { snapped(&ys, a.p.y) - a.p.y } else { 0.0 };
            changed |= dx != 0.0 || dy != 0.0;
            m.push(Vec2::new(dx, dy));
        }
        moves.push(m);
    }
    changed.then_some(moves)
}

/// The coordinates `values` and where each snaps to, in increasing order: the nearest whole point
/// (`half`: half point), and at least 1 point past where the coordinate before it snaps when the
/// two differ.
fn snapped_values(values: impl Iterator<Item = f64>, half: bool) -> Vec<(f64, f64)> {
    let mut sorted: Vec<f64> = values.filter(|v| v.is_finite()).collect();
    sorted.sort_by(f64::total_cmp);
    let mut out: Vec<(f64, f64)> = vec![];
    for v in sorted {
        match out.last() {
            Some(&(last, _)) if v - last <= EPS => {}
            Some(&(_, at)) => out.push((v, nearest(v, half).max(at + 1.0))),
            None => out.push((v, nearest(v, half))),
        }
    }
    out
}

/// The whole point (`half`: half point) nearest `v`. A whole point halfway between two half points
/// goes to the half point above it, and a half point halfway between two whole points goes to the
/// whole point below it, so that switching between the two returns a coordinate to where it was.
fn nearest(v: f64, half: bool) -> f64 {
    if half { (v + EPS).floor() + 0.5 } else { (v - 0.5 - EPS).ceil() }
}

/// Where coordinate `v` snaps to, from [`snapped_values`] (`v` itself when it is not listed). A
/// coordinate merged into one listed is at most [`EPS`] past it.
fn snapped(values: &[(f64, f64)], v: f64) -> f64 {
    let i = values.partition_point(|(x, _)| *x < v - EPS);
    values.get(i).filter(|(x, _)| (x - v).abs() <= EPS).map_or(v, |(_, to)| *to)
}
