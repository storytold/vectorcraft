//! Drawing and path-editing tools (pencil, curvature, anchor tools, scissors, knife, line family…).
//!
//! Like every tool these only emit [`crate::Action`]s; the geometry work happens in the engine's
//! `path.*` commands (`crates/engine/src/cmd/draw2.rs`), so gestures replay through the journal.

mod anchor;
mod curvature;
mod family;
mod gesture;

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::{Anchor, AnchorKind, BezPath, Point, SubPath};

use crate::{Action, Tool, ToolContext};

pub use anchor::AnchorTool;
pub use curvature::{CurvatureTool, EndCurve, curvature_close, curvature_extend, curvature_insert, curvature_move};
pub use family::FamilyTool;
pub use gesture::GestureTool;

/// Colour of tool feedback (Illustrator's selection blue).
pub const FEEDBACK: [u8; 3] = [0x4f, 0x9d, 0xff];

/// Create a drawing tool by id (None = not one of ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    Some(match id {
        "pencil" | "paintbrush" | "blobBrush" | "eraser" | "knife" | "smooth" | "pathEraser" | "join" => Box::new(GestureTool::new(id)),
        "curvature" => Box::new(CurvatureTool::default()),
        "addAnchor" | "deleteAnchor" | "anchorPoint" | "scissors" => Box::new(AnchorTool::new(id)),
        "arc" | "spiral" | "rectangularGrid" | "polarGrid" => Box::new(FamilyTool::new(id)),
        _ => return None,
    })
}

/// A spline through `points` (Catmull-Rom converted to cubic Béziers). Points flagged `true` are
/// corners (no handles); open ends have no handles either. Used by the Curvature tool and by the
/// engine's freehand fitting.
pub fn catmull_rom(points: &[(Point, bool)], closed: bool) -> SubPath {
    let n = points.len();
    let mut anchors = Vec::with_capacity(n);
    for i in 0..n {
        let (p, corner) = points[i];
        let ends = !closed && (i == 0 || i + 1 == n);
        if corner || ends || n < 3 {
            anchors.push(Anchor::corner(p));
            continue;
        }
        let prev = points[(i + n - 1) % n].0;
        let next = points[(i + 1) % n].0;
        let t = (next - prev) / 6.0;
        if t.hypot() < 1e-9 {
            anchors.push(Anchor::corner(p));
            continue;
        }
        anchors.push(Anchor { p, h_in: p - t, h_out: p + t, kind: AnchorKind::Smooth });
    }
    SubPath::new(anchors, closed && n > 2)
}

/// `[[x, y], …]` for command params.
pub fn points_json(pts: &[Point]) -> Value {
    Value::Array(pts.iter().map(|p| json!([p.x, p.y])).collect())
}

/// A polyline preview path.
pub fn polyline(pts: &[Point]) -> BezPath {
    let mut bp = BezPath::new();
    if let Some(f) = pts.first() {
        bp.move_to(*f);
        for p in &pts[1..] {
            bp.line_to(*p);
        }
    }
    bp
}

/// Editable path nodes, selected ones first, then the rest top-most first.
fn candidate_paths(cx: &ToolContext) -> Vec<NodeId> {
    let mut v: Vec<NodeId> = cx.selection.objects.iter().copied().filter(|id| cx.doc.node(*id).is_some_and(|n| n.path_data().is_some())).collect();
    let mut rest = vec![];
    cx.doc.walk(|n| {
        if let NodeKind::Path { guide: false, .. } = &n.kind {
            rest.push(n.id);
        }
    });
    rest.reverse();
    for id in rest {
        if !v.contains(&id) {
            v.push(id);
        }
    }
    v.retain(|id| cx.doc.is_editable(*id));
    v
}

/// The selected paths the Pen and the Curvature tool edit (not guides, nor locked or hidden ones).
pub(crate) fn editable_paths<'a>(cx: &'a ToolContext) -> impl Iterator<Item = NodeId> + 'a {
    cx.selection
        .objects
        .iter()
        .copied()
        .filter(|&id| cx.doc.is_editable(id) && cx.doc.node(id).is_some_and(|n| matches!(n.kind, NodeKind::Path { guide: false, .. })))
}

/// Nearest anchor within `tol`: (path, subpath, anchor).
pub(crate) fn hit_anchor(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, usize)> {
    anchor_in(cx, candidate_paths(cx), p, tol)
}

/// Nearest anchor within `tol` on the first of `ids` that has one: (path, subpath, anchor).
pub(crate) fn anchor_in(cx: &ToolContext, ids: impl IntoIterator<Item = NodeId>, p: Point, tol: f64) -> Option<(NodeId, usize, usize)> {
    let mut best: Option<((NodeId, usize, usize), f64)> = None;
    for id in ids {
        let Some(pd) = cx.doc.node(id).and_then(|n| n.path_data()) else { continue };
        for (si, ai, a) in pd.anchors() {
            let d = a.p.distance(p);
            if d <= tol && best.is_none_or(|b| d < b.1 - 1e-9) {
                best = Some(((id, si, ai), d));
            }
        }
        if best.is_some() {
            break;
        }
    }
    best.map(|b| b.0)
}

/// Nearest point on a path segment within `tol`: (path, subpath, segment, t).
pub(crate) fn hit_segment(cx: &ToolContext, p: Point, tol: f64) -> Option<(NodeId, usize, usize, f64)> {
    segment_in(cx, candidate_paths(cx), p, tol)
}

/// Nearest point within `tol` on a segment of the first of `ids` that passes there: (path,
/// subpath, segment, t).
pub(crate) fn segment_in(cx: &ToolContext, ids: impl IntoIterator<Item = NodeId>, p: Point, tol: f64) -> Option<(NodeId, usize, usize, f64)> {
    for id in ids {
        let Some(pd) = cx.doc.node(id).and_then(|n| n.path_data()) else { continue };
        if let Some((si, seg, t, _, d)) = pd.nearest(p)
            && d <= tol
        {
            return Some((id, si, seg, t));
        }
    }
    None
}

/// Add an anchor where [`hit_segment`] or [`segment_in`] found a segment.
pub(crate) fn insert_anchor((id, si, seg, t): (NodeId, usize, usize, f64)) -> Action {
    Action::Exec("path.insertAnchor".into(), json!({"id": id.0, "subpath": si, "segment": seg, "t": t}))
}

/// Delete the anchor [`hit_anchor`] or [`anchor_in`] found.
pub(crate) fn remove_anchor((id, si, ai): (NodeId, usize, usize)) -> Action {
    Action::Exec("path.removeAnchor".into(), json!({"id": id.0, "subpath": si, "anchor": ai}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catmull_rom_passes_through_points() {
        let pts = [(Point::new(0.0, 0.0), false), (Point::new(50.0, 50.0), false), (Point::new(100.0, 0.0), false)];
        let sp = catmull_rom(&pts, false);
        assert_eq!(sp.anchors.len(), 3);
        assert_eq!(sp.anchors[1].p, Point::new(50.0, 50.0));
        assert_eq!(sp.anchors[1].kind, AnchorKind::Smooth);
        // The middle tangent is horizontal (parallel to first → last).
        assert!((sp.anchors[1].h_out.y - 50.0).abs() < 1e-9);
        assert!(!sp.anchors[0].has_out());
    }

    #[test]
    fn all_tools_exist() {
        for id in [
            "pencil",
            "paintbrush",
            "blobBrush",
            "eraser",
            "knife",
            "smooth",
            "pathEraser",
            "join",
            "curvature",
            "addAnchor",
            "deleteAnchor",
            "anchorPoint",
            "scissors",
            "arc",
            "spiral",
            "rectangularGrid",
            "polarGrid",
        ] {
            assert_eq!(create(id).unwrap().id(), id);
        }
        assert!(create("selection").is_none());
    }
}
