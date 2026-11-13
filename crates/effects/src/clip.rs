//! The region a clip group clips to ([`Node::clip_outline`]) with text outlined by the font engine
//! and several shapes united, so the renderer and the SVG and PDF writers clip alike.

use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeKind};
use vectorcraft_geom::{BezPath, FillRule, PathData};

/// The clipping path `clip` of a clip group as one path and fill rule: compound paths keep their
/// holes, even-odd paths their rule, text clips by its glyph outlines and a group by the union of
/// its members. `None` when there is nothing to clip by (the clipped art is then hidden).
pub fn clip_outline(clip: &Node) -> Option<(BezPath, FillRule)> {
    clip.clip_outline(Some(&outline_text), &unite)
}

/// The knockout shape of a piece of a mesh-warped image ([`Node::knockout_outline`]) painted opaque
/// in `color`, which the SVG and PDF writers put in their knockout masks: for a clip group, a
/// rectangle clipped by the same clipping path (so a viewer covers the same pixels of its edges as
/// when it clips the piece), else the path of its opacity mask's art, filled. `None` for other
/// objects and for a piece with nothing to clip by.
pub fn knockout_silhouette(n: &Node, color: Color) -> Option<Node> {
    let outline = n.knockout_outline()?;
    let (bp, rule) = clip_outline(outline)?;
    let paint = Appearance::basic(Paint::solid(color), Paint::None, 0.0);
    if n.clips() && n.extras().mask.is_none() {
        let cover = Node::path(n.id, vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Shape::bounding_box(&bp).inflate(1.0, 1.0)), paint);
        let children = vec![outline.clone(), std::sync::Arc::new(cover)];
        return Some(Node::new(n.id, NodeKind::Group { children, clip: true }));
    }
    let mut p = Node::path(n.id, PathData::from_bezpath(&bp), paint);
    if let NodeKind::Path { rule: r, .. } = &mut p.kind {
        *r = rule;
    }
    Some(p)
}

/// Whether any of `pieces`, pieces of a mesh-warped image ([`Node::warp_piece_image`]), paints less
/// than opaque anywhere inside its triangle: it or its image is partly transparent, a color mask
/// clears some of its colors, a colorize paint draws its light pixels partly transparent, or the
/// pixels of its image (looked up in `doc`) are. Each distinct
/// image's pixels are scanned once. Pieces of an opaque image overlap without showing it, so the SVG
/// and PDF writers draw them one over another without masks.
pub fn warp_pieces_translucent(doc: &Document, pieces: &[&Node]) -> bool {
    let mut scanned: Vec<*const vectorcraft_doc::ImageBlob> = vec![];
    for n in pieces {
        let Some(image) = n.warp_piece_image() else { continue };
        let NodeKind::Image(im) = &image.kind else { continue };
        let e = image.extras();
        if n.opacity < 1.0 || image.opacity < 1.0 || e.color_mask.is_some() || e.colorize.is_some() {
            return true;
        }
        let Some(blob) = doc.image_blob(im) else { continue };
        if scanned.contains(&std::ptr::from_ref(blob)) {
            continue;
        }
        scanned.push(std::ptr::from_ref(blob));
        if blob.has_transparency() {
            return true;
        }
    }
    false
}

/// Text as one path of its glyph outlines, in document space.
fn outline_text(n: &Node) -> Option<Node> {
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
