//! The region a clip group clips to ([`Node::clip_outline`]) with text outlined by the font engine
//! and several shapes united, so the renderer and the SVG and PDF writers clip alike.

use vectorcraft_doc::{Appearance, Node, NodeKind};
use vectorcraft_geom::{BezPath, FillRule, PathData};

/// The clipping path `clip` of a clip group as one path and fill rule: compound paths keep their
/// holes, even-odd paths their rule, text clips by its glyph outlines and a group by the union of
/// its members. `None` when there is nothing to clip by (the clipped art is then hidden).
pub fn clip_outline(clip: &Node) -> Option<(BezPath, FillRule)> {
    clip.clip_outline(Some(&outline_text), &unite)
}

/// Text as one path of its glyph outlines, in document space; a compound shape as its outline.
fn outline_text(n: &Node) -> Option<Node> {
    if let Some(p) = crate::compound_shape_path(n) {
        return Some(Node::path(n.id, p, Appearance::default()));
    }
    let NodeKind::Text(t) = &n.kind else { return None };
    let mut bp = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).to_bezpath();
    bp.apply_affine(t.xf);
    Some(Node::path(n.id, PathData::from_bezpath(&bp), Appearance::default()))
}

fn unite(shapes: &[(BezPath, FillRule)]) -> BezPath {
    let paths: Vec<(PathData, FillRule)> = shapes.iter().map(|(bp, r)| (PathData::from_bezpath(bp), *r)).collect();
    let refs: Vec<(&PathData, FillRule)> = paths.iter().map(|(p, r)| (p, *r)).collect();
    vectorcraft_pathops::unite_all(&refs).to_bezpath()
}
