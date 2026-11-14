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

/// Which overlaps of the pieces of a mesh-warped image ([`Node::warp_piece_image`]) can show
/// through. Where an upper piece overlaps a lower one only over fully opaque pixels of the image,
/// the SVG and PDF writers draw the upper piece over the lower one without a knockout mask for it:
/// a mask there would make the opaque pixels on the upper piece's antialiased edge lighter.
pub struct WarpSeams {
    pieces: Vec<Option<SeamPiece>>,
    alphas: Vec<(u32, u32, Vec<u8>)>,
}

/// A piece of a mesh-warped image whose pixels decide whether its overlaps show through.
struct SeamPiece {
    /// The piece's triangle, in document space.
    triangle: [vectorcraft_geom::Point; 3],
    /// Maps document space to the pixels of its image as decoded.
    to_pixels: vectorcraft_geom::Affine,
    /// The index of its image's alpha in [`WarpSeams::alphas`].
    alpha: usize,
}

/// How far from an overlap, in image pixels, a less than opaque pixel makes the overlap show
/// through: a smoothed image blends each pixel with its neighbors.
const SEAM_MARGIN: f64 = 2.0;

impl WarpSeams {
    /// The overlaps of `pieces` (pieces of a mesh-warped image, in paint order), with the pixels of
    /// their images looked up in `doc`. Each distinct image is decoded once.
    pub fn new(doc: &Document, pieces: &[&Node]) -> Self {
        let mut images: Vec<*const vectorcraft_doc::ImageBlob> = vec![];
        let mut alphas = vec![];
        let pieces = pieces
            .iter()
            .map(|n| {
                let image = n.warp_piece_image()?;
                let NodeKind::Image(im) = &image.kind else { return None };
                let e = image.extras();
                if n.opacity < 1.0 || image.opacity < 1.0 || e.color_mask.is_some() || e.colorize.is_some() {
                    return None;
                }
                let triangle = triangle(n.knockout_outline()?)?;
                let blob = doc.image_blob(im)?;
                let alpha = match images.iter().position(|b| std::ptr::eq(*b, blob)) {
                    Some(i) => i,
                    None => {
                        alphas.push(blob.alpha()?);
                        images.push(std::ptr::from_ref(blob));
                        alphas.len() - 1
                    }
                };
                let (w, h, _) = &alphas[alpha];
                if im.width == 0 || im.height == 0 || im.xf.determinant().abs() < 1e-12 {
                    return None;
                }
                let scale = vectorcraft_geom::Affine::scale_non_uniform(f64::from(*w) / f64::from(im.width), f64::from(*h) / f64::from(im.height));
                Some(SeamPiece { triangle, to_pixels: scale * im.xf.inverse(), alpha })
            })
            .collect();
        WarpSeams { pieces, alphas }
    }

    /// Whether piece `above` (an index into the pieces) drawn over piece `below` can show the
    /// lower piece through where they overlap: either piece or its image is partly transparent, a
    /// color mask or a colorize paint changes its alpha, or a pixel of either image within
    /// [`SEAM_MARGIN`] of the overlap is less than opaque. `true` for a piece that is not a
    /// triangle of an image that can be decoded; `false` when the triangles do not overlap.
    pub fn shows_through(&self, below: usize, above: usize) -> bool {
        let (Some(Some(a)), Some(Some(b))) = (self.pieces.get(below), self.pieces.get(above)) else { return true };
        let overlap = convex_intersection(&a.triangle, &b.triangle);
        overlap.len() >= 3 && [a, b].into_iter().any(|p| self.translucent_near(p, &overlap))
    }

    /// Whether a pixel of piece `p`'s image within [`SEAM_MARGIN`] of the convex polygon `overlap`
    /// (document space) is less than opaque.
    fn translucent_near(&self, p: &SeamPiece, overlap: &[vectorcraft_geom::Point]) -> bool {
        let Some((w, h, alpha)) = self.alphas.get(p.alpha) else { return true };
        let poly: Vec<vectorcraft_geom::Point> = overlap.iter().map(|q| p.to_pixels * *q).collect();
        let (mut x0, mut y0, mut x1, mut y1) = (f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for q in &poly {
            (x0, y0, x1, y1) = (x0.min(q.x), y0.min(q.y), x1.max(q.x), y1.max(q.y));
        }
        let span = |lo: f64, hi: f64, n: u32| {
            let lo = (lo - SEAM_MARGIN).floor().clamp(0.0, f64::from(n)) as u32;
            let hi = (hi + SEAM_MARGIN).ceil().clamp(0.0, f64::from(n)) as u32;
            lo..hi
        };
        let orientation = signed_area(&poly).signum();
        for y in span(y0, y1, *h) {
            for x in span(x0, x1, *w) {
                let c = vectorcraft_geom::Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
                let near = poly.iter().zip(poly.iter().cycle().skip(1)).all(|(s, e)| {
                    let edge = *e - *s;
                    orientation * edge.cross(c - *s) >= -SEAM_MARGIN * edge.hypot()
                });
                let i = y as usize * *w as usize + x as usize;
                if near && alpha.get(i).is_none_or(|a| *a < 255) {
                    return true;
                }
            }
        }
        false
    }
}

/// The three corners of the straight-sided triangle `outline` draws, in document space.
fn triangle(outline: &Node) -> Option<[vectorcraft_geom::Point; 3]> {
    let (bp, _) = clip_outline(outline)?;
    let mut points = vec![];
    for el in bp.elements() {
        match el {
            vectorcraft_geom::PathEl::MoveTo(p) | vectorcraft_geom::PathEl::LineTo(p) if points.is_empty() || points.last() != Some(p) => {
                points.push(*p)
            }
            vectorcraft_geom::PathEl::MoveTo(_) | vectorcraft_geom::PathEl::LineTo(_) | vectorcraft_geom::PathEl::ClosePath => {}
            _ => return None,
        }
    }
    if points.len() == 4 && points.first() == points.last() {
        points.pop();
    }
    let [a, b, c] = points.as_slice() else { return None };
    Some([*a, *b, *c])
}

/// Twice the signed area of polygon `poly`.
fn signed_area(poly: &[vectorcraft_geom::Point]) -> f64 {
    poly.iter().zip(poly.iter().cycle().skip(1)).map(|(a, b)| a.to_vec2().cross(b.to_vec2())).sum()
}

/// The intersection of convex polygons `subject` and `clip`, as a convex polygon (empty when they
/// do not overlap).
fn convex_intersection(subject: &[vectorcraft_geom::Point], clip: &[vectorcraft_geom::Point]) -> Vec<vectorcraft_geom::Point> {
    let orientation = signed_area(clip).signum();
    if orientation == 0.0 {
        return vec![];
    }
    let mut out = subject.to_vec();
    for (s, e) in clip.iter().zip(clip.iter().cycle().skip(1)) {
        let edge = *e - *s;
        let side = |p: vectorcraft_geom::Point| orientation * edge.cross(p - *s);
        let input = std::mem::take(&mut out);
        for (p, q) in input.iter().zip(input.iter().cycle().skip(1)) {
            let (sp, sq) = (side(*p), side(*q));
            if sp >= 0.0 {
                out.push(*p);
            }
            if (sp >= 0.0) != (sq >= 0.0) {
                out.push(*p + (*q - *p) * (sp / (sp - sq)));
            }
        }
        if out.is_empty() {
            break;
        }
    }
    out
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
