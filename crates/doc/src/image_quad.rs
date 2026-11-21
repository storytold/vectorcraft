//! Perspective Distort and Free Distort on an image: its pixel rectangle's corners placed anywhere
//! on the page, as long as they make a strictly convex quadrilateral.
//!
//! An image whose corners make a parallelogram is placed by its transform alone
//! ([`crate::ImageObject::xf`]). Other corners are kept in the image's extras
//! ([`crate::NodeExtras::image_quad`]), and the image is then drawn through the projective map
//! that takes its pixel rectangle to them. The renderer, the exporters and printing draw it as a
//! raster mesh warp ([`crate::live::mesh_warp_image`]) made from that map ([`distorted_art`]): no
//! warped copy of the pixels is kept. The quad stays with the image's transform: moving, scaling,
//! rotating or shearing the image maps its corners too ([`crate::Node::transform`]).
//!
//! The convexity rule, the tolerance of the mesh and its cell limit are VectorCraft's own.

use std::sync::Arc;

use vectorcraft_geom::kurbo::BezPath;
use vectorcraft_geom::{Affine, Homography, PathData, Point, Rect, SubPath};

use crate::{Appearance, ImageObject, Node, NodeKind};

/// Most cells along each side of the mesh an image is drawn with ([`cells`]).
pub const MAX_CELLS: usize = 64;

/// The cells along each side an image with fewer pixels than this may still take ([`cells`]).
pub const MIN_CELL_CAP: usize = 8;

/// How far (in points) the mesh an image is drawn with in files and print may stray from the exact
/// projective map ([`Density::PAGE`]).
pub const TOLERANCE: f64 = 0.05;

/// How finely [`distorted_art_at`] cuts a distorted image ([`cells`]), and how far each piece
/// reaches into its neighbors. A pixel inside the image draws opaque only when one piece covers
/// it whole, so pieces much smaller than a pixel would let the page show through.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Density {
    /// How far (in points) the mesh may stray from the exact projective map.
    pub tolerance: f64,
    /// The shortest a cell may be along the longest side of the corners, in points.
    pub min_cell: f64,
    /// How far (in points) each piece reaches past its triangle.
    pub overlap: f64,
}

impl Density {
    /// For files and print, whose viewing size is not known: within [`TOLERANCE`], cells of at
    /// least 2 pt, and pieces that reach half a point past their triangles.
    pub const PAGE: Density = Density { tolerance: TOLERANCE, min_cell: 2.0, overlap: 0.5 };

    /// For drawing at `scale` device pixels a point: within a quarter of a pixel, cells of at least
    /// 4 pixels, and pieces that reach 2 pixels past their triangles.
    pub fn at_scale(scale: f64) -> Density {
        let s = if scale.is_finite() && scale > 1e-6 { scale } else { 1.0 };
        Density { tolerance: 0.25 / s, min_cell: 4.0 / s, overlap: 2.0 / s }
    }

    /// [`Density::PAGE`] for art that is drawn `scale` times its size (a symbol's art in an
    /// instance that scales it): the tolerance, the cell size and the overlap measured on the page.
    pub fn page_at(scale: f64) -> Density {
        let s = if scale.is_finite() && scale > 1e-6 { scale } else { 1.0 };
        let p = Density::PAGE;
        Density { tolerance: p.tolerance / s, min_cell: p.min_cell / s, overlap: p.overlap / s }
    }
}

/// How much `a` stretches its most stretched direction: the larger singular value of its linear
/// part.
pub fn max_stretch(a: Affine) -> f64 {
    let [m0, m1, m2, m3, _, _] = a.as_coeffs();
    let sum = m0 * m0 + m1 * m1 + m2 * m2 + m3 * m3;
    let det = m0 * m3 - m1 * m2;
    ((sum + (sum * sum - 4.0 * det * det).max(0.0).sqrt()) / 2.0).sqrt()
}

/// Why corners cannot be an image's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuadError {
    /// A corner is not a finite point.
    NotFinite,
    /// The corners fold over, meet, or lie on one line: the quadrilateral is not strictly convex.
    NotConvex,
    /// The corners are convex but so close to a line that the perspective map through them cannot
    /// place every corner.
    TooNarrow,
}

impl std::fmt::Display for QuadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotFinite => "the corners of a distorted image must be finite points",
            Self::NotConvex => "the corners of a distorted image must make a convex shape: an image cannot fold over itself or collapse to a line",
            Self::TooNarrow => "the corners of a distorted image are too close together for a perspective map",
        })
    }
}

/// `im`'s pixel rectangle: (0, 0) to its width and height.
fn pixel_rect(im: &ImageObject) -> Rect {
    Rect::new(0.0, 0.0, f64::from(im.width), f64::from(im.height))
}

/// The corners of `r` in the order a quad keeps them: top left, top right, bottom right, bottom
/// left.
fn rect_corners(r: Rect) -> [Point; 4] {
    [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]
}

/// Whether the corners `q` (in order around the shape) are finite and make a strictly convex
/// quadrilateral, turning either way round. Each turn must be larger than 1e-9 times the product
/// of the two sides it joins, a test that gives the same answer at any scale.
pub fn check(q: &[Point; 4]) -> Result<(), QuadError> {
    if q.iter().any(|p| !p.is_finite()) {
        return Err(QuadError::NotFinite);
    }
    let mut sign = 0.0;
    for i in 0..4 {
        let (a, b, c) = (q[i], q[(i + 1) % 4], q[(i + 2) % 4]);
        let (u, v) = (b - a, c - b);
        let cross = u.cross(v);
        let scale = u.hypot() * v.hypot();
        if !cross.is_finite() || !scale.is_finite() || cross.abs() <= 1e-9 * scale || scale == 0.0 {
            return Err(QuadError::NotConvex);
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return Err(QuadError::NotConvex);
        }
    }
    // Four turns the same way round can still wind twice (a star); a convex quadrilateral's
    // diagonals cross.
    let crosses = |a: Point, b: Point, c: Point, d: Point| (b - a).cross(c - a).signum() != (b - a).cross(d - a).signum();
    if !crosses(q[0], q[2], q[1], q[3]) || !crosses(q[1], q[3], q[0], q[2]) {
        return Err(QuadError::NotConvex);
    }
    Ok(())
}

/// Whether `q` can be image `im`'s corners: [`check`], and either a parallelogram or corners the
/// perspective map from its pixels ([`pixel_map`]) places, each within a millionth of `q`'s
/// shortest side plus the rounding error of coordinates as far from the origin as `q`'s.
pub fn check_for(im: &ImageObject, q: &[Point; 4]) -> Result<(), QuadError> {
    check(q)?;
    if parallelogram(im, q).is_some() {
        return Ok(());
    }
    let tol = placement_tolerance(q);
    let map = pixel_map(im, q);
    let placed = rect_corners(pixel_rect(im)).iter().zip(q).all(|(c, want)| map.apply(*c).is_some_and(|p| p.distance(*want) <= tol));
    if placed { Ok(()) } else { Err(QuadError::TooNarrow) }
}

/// How far a map may place a corner from `q`'s: a millionth of `q`'s shortest side plus the
/// rounding error of coordinates as far from the origin as `q`'s.
fn placement_tolerance(q: &[Point; 4]) -> f64 {
    let side = (0..4).map(|i| q[i].distance(q[(i + 1) % 4])).fold(f64::INFINITY, f64::min);
    let far = q.iter().map(|p| p.x.abs().max(p.y.abs())).fold(0.0, f64::max);
    side * 1e-6 + far * 1e-12
}

/// The affine map from `im`'s pixel space that places its corners at `q`, when `q` is a
/// parallelogram (within 1e-9 of the quad's size): the map puts the fourth corner within
/// [`placement_tolerance`] of `q`'s. `None` otherwise.
fn parallelogram(im: &ImageObject, q: &[Point; 4]) -> Option<Affine> {
    let size = q.iter().fold(0.0_f64, |m, p| m.max((*p - q[0]).hypot()));
    // The affine map through the other three corners misses the bottom right one by this much.
    let gap = (q[0].to_vec2() + q[2].to_vec2() - q[1].to_vec2() - q[3].to_vec2()).hypot();
    if gap > 1e-9 * size || gap > placement_tolerance(q) {
        return None;
    }
    let (w, h) = (f64::from(im.width.max(1)), f64::from(im.height.max(1)));
    let (x, y) = ((q[1] - q[0]) / w, (q[3] - q[0]) / h);
    let a = Affine::new([x.x, x.y, y.x, y.y, q[0].x, q[0].y]);
    a.is_finite().then_some(a)
}

/// The homography taking the unit square's corners (0,0), (1,0), (1,1), (0,1) to `q`.
fn square_to(q: &[Point; 4]) -> Homography {
    let [p0, p1, p2, p3] = *q;
    let (sx, sy) = (p0.x - p1.x + p2.x - p3.x, p0.y - p1.y + p2.y - p3.y);
    let (dx1, dx2, dy1, dy2) = (p1.x - p2.x, p3.x - p2.x, p1.y - p2.y, p3.y - p2.y);
    let den = dx1 * dy2 - dx2 * dy1;
    let (g, h) = if den.abs() < 1e-300 { (0.0, 0.0) } else { ((sx * dy2 - dx2 * sy) / den, (dx1 * sy - sx * dy1) / den) };
    Homography([[p1.x - p0.x + g * p1.x, p3.x - p0.x + h * p3.x, p0.x], [p1.y - p0.y + g * p1.y, p3.y - p0.y + h * p3.y, p0.y], [g, h, 1.0]])
}

impl Node {
    /// This image's distortion: where its pixel rectangle's corners stand (top left, top right,
    /// bottom right, bottom left) when they do not make a parallelogram. `None` for an image placed
    /// by its transform alone, for anything that is not an image, and for corners a file holds
    /// that [`check_for`] refuses (the image is then placed by its transform).
    pub fn image_quad(&self) -> Option<[Point; 4]> {
        let NodeKind::Image(im) = &self.kind else { return None };
        let q = **self.extras().image_quad.as_ref()?;
        check_for(im, &q).is_ok().then_some(q)
    }

    /// Whether this is an image with a distortion ([`Self::image_quad`]).
    pub fn is_distorted_image(&self) -> bool {
        self.image_quad().is_some()
    }

    /// Where this image's pixel rectangle's corners stand: its distortion, or its transform's
    /// corners. `None` for anything that is not an image.
    pub fn image_corners(&self) -> Option<[Point; 4]> {
        let NodeKind::Image(im) = &self.kind else { return None };
        Some(self.image_quad().unwrap_or_else(|| rect_corners(pixel_rect(im)).map(|p| im.xf * p)))
    }

    /// Place this image's corners at `q` (top left, top right, bottom right, bottom left): by its
    /// transform when they make a parallelogram (and the distortion is cleared), else as its
    /// distortion. An error, and nothing changes, when `q` fails [`check_for`] or this is not an
    /// image.
    pub fn set_image_corners(&mut self, q: [Point; 4]) -> Result<(), QuadError> {
        let NodeKind::Image(im) = &mut self.kind else { return Err(QuadError::NotConvex) };
        check_for(im, &q)?;
        match parallelogram(im, &q) {
            Some(xf) => {
                im.xf = xf;
                if self.extras().image_quad.is_some() {
                    self.extras_mut().image_quad = None;
                }
            }
            None => self.extras_mut().image_quad = Some(Box::new(q)),
        }
        Ok(())
    }

    /// The outline of this image's frame on the page: its corners joined by straight lines.
    /// `None` for anything that is not an image.
    pub fn image_frame(&self) -> Option<BezPath> {
        let q = self.image_corners()?;
        let mut bp = BezPath::new();
        bp.move_to(q[0]);
        for p in &q[1..] {
            bp.line_to(*p);
        }
        bp.close_path();
        Some(bp)
    }

    /// The map from this image's pixel space to the page: projective for a distorted image, its
    /// transform otherwise. `None` for anything that is not an image.
    pub fn image_map(&self) -> Option<Homography> {
        let NodeKind::Image(im) = &self.kind else { return None };
        Some(match self.image_quad() {
            Some(q) => pixel_map(im, &q),
            None => Homography::from_affine(im.xf),
        })
    }
}

/// The projective map from `im`'s pixel space to the page that puts its corners at `q`.
fn pixel_map(im: &ImageObject, q: &[Point; 4]) -> Homography {
    let (w, h) = (f64::from(im.width.max(1)), f64::from(im.height.max(1)));
    square_to(q).then_after(&Homography::from_affine(Affine::scale_non_uniform(1.0 / w, 1.0 / h)))
}

/// How many cells along each side the mesh of a distorted image takes: the fewest (a power of two
/// up to [`MAX_CELLS`]) for which the mesh's triangles, each mapped by the affine map through its
/// corners, stay within `density`'s tolerance of `map` at the middle of every cell and every cell
/// side. An image takes no more cells than it has pixels along its longer side, or
/// [`MIN_CELL_CAP`] when that is more, and no more than leave each cell `density.min_cell` along
/// the longest side of its corners. A perspective that needs more cells than these limits allow is
/// drawn with the cells they allow.
pub fn cells(map: &Homography, im: &ImageObject, density: Density) -> usize {
    let (w, h) = (f64::from(im.width.max(1)), f64::from(im.height.max(1)));
    let corners = [Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)].map(|p| map.apply(p));
    let longest =
        corners.iter().zip(corners.iter().cycle().skip(1)).filter_map(|(a, b)| Some(a.as_ref()?.distance(*b.as_ref()?))).fold(0.0, f64::max);
    let by_size = longest / density.min_cell;
    let by_size = if by_size.is_finite() { (by_size as usize).max(1) } else { MAX_CELLS };
    let most = MAX_CELLS.min((im.width.max(im.height) as usize).max(MIN_CELL_CAP)).min(by_size);
    let at = |x: f64, y: f64| map.apply(Point::new(x, y));
    let mut k = 1;
    while k < most {
        let fits = (0..k * k).all(|c| {
            let (i, j) = ((c % k) as f64, (c / k) as f64);
            let (x0, y0, x1, y1) = (w * i / k as f64, h * j / k as f64, w * (i + 1.0) / k as f64, h * (j + 1.0) / k as f64);
            let (Some(a), Some(b), Some(cc), Some(d)) = (at(x0, y0), at(x1, y0), at(x1, y1), at(x0, y1)) else { return false };
            let near = |exact: Option<Point>, m: Point| exact.is_some_and(|e| e.distance(m) <= density.tolerance);
            near(at((x0 + x1) / 2.0, (y0 + y1) / 2.0), a.midpoint(cc))
                && near(at((x0 + x1) / 2.0, y0), a.midpoint(b))
                && near(at(x1, (y0 + y1) / 2.0), b.midpoint(cc))
                && near(at((x0 + x1) / 2.0, y1), d.midpoint(cc))
                && near(at(x0, (y0 + y1) / 2.0), a.midpoint(d))
        });
        if fits {
            break;
        }
        k *= 2;
    }
    k.min(most)
}

/// For a format that cannot clip an image into the pieces of [`distorted_art`]: the affine
/// placement nearest distorted image `n`'s projective map, the one that matches it at the middle
/// of its pixels. `None` when `n` is not a distorted image.
pub fn nearest_affine(n: &Node) -> Option<Affine> {
    let NodeKind::Image(im) = &n.kind else { return None };
    let q = n.image_quad()?;
    pixel_map(im, &q).affine_at(pixel_rect(im).center())
}

/// Distorted image `n` as the art that draws it in files and print: [`distorted_art_at`] with
/// [`Density::PAGE`].
pub fn distorted_art(n: &Node) -> Option<Node> {
    distorted_art_at(n, Density::PAGE)
}

/// Distorted image `n` as the art that draws it: a clip group, clipped by its corners, around the
/// raster mesh warp ([`crate::live::mesh_warp_image`]) of its pixels through its projective map,
/// with [`cells`] cells along each side for `density` and each piece reaching `density.overlap`
/// past its triangle. The art has `n`'s id but not its name and transparency (which the caller
/// keeps on the object around the art). `None` when `n` is not a distorted image.
pub fn distorted_art_at(n: &Node, density: Density) -> Option<Node> {
    let NodeKind::Image(im) = &n.kind else { return None };
    let q = n.image_quad()?;
    let map = pixel_map(im, &q);
    let k = cells(&map, im, density);
    let mut plain = n.clone();
    // Whether the image shows is the image's to decide: a hidden source image that a view draws
    // anyway (Image Trace's Source Image) draws its art.
    plain.visible = true;
    // The image's own fills and strokes paint by the image's rules, not on the mesh's group.
    plain.appearance = Default::default();
    plain.name = None;
    plain.opacity = 1.0;
    plain.blend = Default::default();
    plain.isolate = false;
    plain.knockout = Default::default();
    plain.knockout_shape = false;
    if let Some(e) = plain.extras_if_any() {
        e.mask = None;
    }
    let mut art = crate::live::mesh_warp_image_with(&plain, im, &|p| p, k, n.extras().pixelated, false, Some(density.overlap));
    // The pieces along the image's edges reach past its corners too: the corners clip them.
    let mut edge = Node::path(n.id, PathData::single(SubPath::polyline(&q, true)), Appearance::default());
    if let NodeKind::Path { clipping, .. } = &mut edge.kind {
        *clipping = true;
    }
    let pieces = std::mem::replace(&mut art.kind, NodeKind::Group { children: vec![], clip: true });
    let mut mesh = art.clone();
    mesh.kind = pieces;
    art.knockout = Default::default();
    art.kind = NodeKind::Group { children: vec![Arc::new(edge), Arc::new(mesh)], clip: true };
    Some(art)
}

/// The pieces of [`distorted_art_at`]'s art: the children of its mesh group.
pub fn distorted_pieces(art: &Node) -> &[Arc<Node>] {
    art.children().and_then(|c| c.get(1)).and_then(|m| m.children()).map_or(&[], Vec::as_slice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeId;

    fn image(w: u32, h: u32) -> Node {
        let im = ImageObject { key: "k".into(), width: w, height: h, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
        Node::new(NodeId(1), NodeKind::Image(Box::new(im)))
    }

    fn pts(v: [(f64, f64); 4]) -> [Point; 4] {
        v.map(|(x, y)| Point::new(x, y))
    }

    #[test]
    fn a_convex_quad_either_way_round_passes_and_a_fold_or_line_does_not() {
        assert_eq!(check(&pts([(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)])), Ok(()));
        assert_eq!(check(&pts([(0.0, 0.0), (0.0, 10.0), (10.0, 10.0), (10.0, 0.0)])), Ok(()));
        // Two corners swapped: the sides cross.
        assert_eq!(check(&pts([(0.0, 0.0), (10.0, 0.0), (0.0, 10.0), (10.0, 10.0)])), Err(QuadError::NotConvex));
        // A dent.
        assert_eq!(check(&pts([(0.0, 0.0), (10.0, 0.0), (2.0, 2.0), (0.0, 10.0)])), Err(QuadError::NotConvex));
        // Three on a line, and two corners at one point.
        assert_eq!(check(&pts([(0.0, 0.0), (5.0, 0.0), (10.0, 0.0), (0.0, 10.0)])), Err(QuadError::NotConvex));
        assert_eq!(check(&pts([(0.0, 0.0), (0.0, 0.0), (10.0, 10.0), (0.0, 10.0)])), Err(QuadError::NotConvex));
        assert_eq!(check(&pts([(f64::NAN, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)])), Err(QuadError::NotFinite));
    }

    /// A convex quad with one edge so short that the perspective map through it cannot place the
    /// other corners: refused when set, and a file's copy of it leaves the image undistorted.
    #[test]
    fn corners_the_perspective_map_cannot_place_are_refused() {
        let q = pts([(50.0, 0.0), (50.000000005, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        assert_eq!(check(&q), Ok(()));
        let mut n = image(100, 100);
        assert_eq!(n.set_image_corners(q), Err(QuadError::TooNarrow));
        assert!(n.extras().image_quad.is_none());
        n.extras_mut().image_quad = Some(Box::new(q));
        assert_eq!(n.image_quad(), None);
        assert!(distorted_art(&n).is_none());
        assert_eq!(n.image_corners(), Some(rect_corners(Rect::new(0.0, 0.0, 100.0, 100.0))));
        // Reading the file drops it, so a later transform can't make it a distortion.
        let mut back: Node = serde_json::from_value(serde_json::to_value(&n).unwrap()).unwrap();
        assert!(back.extras().image_quad.is_none());
        back.transform(Affine::scale_non_uniform(1e6, 1.0), false);
        assert!(!back.is_distorted_image());
    }

    #[test]
    fn a_parallelogram_goes_into_the_transform_and_other_corners_into_the_quad() {
        let mut n = image(100, 50);
        n.set_image_corners(pts([(10.0, 0.0), (110.0, 10.0), (100.0, 60.0), (0.0, 50.0)])).unwrap();
        assert_eq!(n.image_quad(), None);
        let NodeKind::Image(im) = &n.kind else { panic!("an image") };
        assert!((im.xf * Point::new(100.0, 50.0)).distance(Point::new(100.0, 60.0)) < 1e-9);
        let q = pts([(40.0, 0.0), (60.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        n.set_image_corners(q).unwrap();
        assert_eq!(n.image_quad(), Some(q));
        assert_eq!(n.image_corners(), Some(q));
        let map = n.image_map().unwrap();
        for (c, want) in rect_corners(Rect::new(0.0, 0.0, 100.0, 50.0)).iter().zip(q) {
            assert!(map.apply(*c).unwrap().distance(want) < 1e-9, "{c:?}");
        }
        // Back to a rectangle: the quad is cleared.
        n.set_image_corners(pts([(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)])).unwrap();
        assert!(n.extras().image_quad.is_none());
    }

    /// Corners whose gap from a parallelogram is tiny next to the quad's size but not next to its
    /// shortest side stay a distortion: the affine map would put the bottom right corner twice as
    /// low as asked.
    #[test]
    fn corners_the_affine_map_misses_stay_a_distortion() {
        let mut n = image(100, 100);
        let q = pts([(0.0, 0.0), (100000.0, 0.0), (100000.0, 0.00005), (0.0, 0.0001)]);
        n.set_image_corners(q).unwrap();
        assert_eq!(n.image_quad(), Some(q));
        let map = n.image_map().unwrap();
        for (c, want) in rect_corners(Rect::new(0.0, 0.0, 100.0, 100.0)).iter().zip(q) {
            assert!(map.apply(*c).unwrap().distance(want) < 1e-9, "{c:?}");
        }
    }

    /// A scale that squeezes valid corners past what a perspective map can place leaves the image
    /// at the placement nearest them, scaled, not at the rectangle it had before.
    #[test]
    fn a_scale_past_the_perspective_limit_keeps_the_nearest_placement() {
        let mut n = image(100, 100);
        n.set_image_corners(pts([(0.0, 0.0), (100.0, 0.0), (200.0, 2e-7), (0.0, 2e-7)])).unwrap();
        assert!(n.is_distorted_image());
        n.transform(Affine::scale_non_uniform(1.0, 0.25), false);
        assert!(n.extras().image_quad.is_none());
        let b = n.geometric_bounds().unwrap();
        assert!(b.height() < 1e-6 && (b.width() - 150.0).abs() < 60.0, "{b:?}");
    }

    /// A file's corners that the image cannot take are dropped as it is read, so a later scale
    /// that would make them valid leaves the image placed by its transform; a list of three
    /// points is dropped too, and the rest of the image is read.
    #[test]
    fn corners_a_file_holds_that_are_refused_or_not_four_are_dropped_as_it_is_read() {
        let mut n = image(100, 100);
        n.extras_mut().image_quad = Some(Box::new(pts([(0.0, 0.0), (100.0, 0.0), (200.0, 5e-8), (0.0, 5e-8)])));
        assert_eq!(n.image_quad(), None);
        let mut read: Node = serde_json::from_value(serde_json::to_value(&n).unwrap()).unwrap();
        assert!(read.extras().image_quad.is_none());
        read.transform(Affine::scale_non_uniform(1.0, 4.0), false);
        assert!(!read.is_distorted_image());
        assert_eq!(read.geometric_bounds(), Some(Rect::new(0.0, 0.0, 100.0, 400.0)));
        let mut v = serde_json::to_value(&n).unwrap();
        v["image_quad"] = serde_json::json!([[0.0, 0.0], [10.0, 0.0], [10.0, 10.0]]);
        let read: Node = serde_json::from_value(v).unwrap();
        assert!(read.extras().image_quad.is_none());
        assert!(matches!(read.kind, NodeKind::Image(_)));
        // Past a fourth point the rest is skipped without being read as points.
        for extra in [serde_json::json!([5.0, 5.0]), serde_json::json!("not a point")] {
            let mut v = serde_json::to_value(&n).unwrap();
            v["image_quad"] = serde_json::json!([[40.0, 0.0], [60.0, 0.0], [100.0, 100.0], [0.0, 100.0], extra, {"x": 1}]);
            let read: Node = serde_json::from_value(v).unwrap();
            assert!(read.extras().image_quad.is_none());
        }
        // Valid corners read back.
        let q = pts([(40.0, 0.0), (60.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        n.set_image_corners(q).unwrap();
        let read: Node = serde_json::from_value(serde_json::to_value(&n).unwrap()).unwrap();
        assert_eq!(read.image_quad(), Some(q));
    }

    #[test]
    fn refused_corners_change_nothing_and_a_bad_quad_in_a_file_is_ignored() {
        let mut n = image(10, 10);
        let before = n.clone();
        assert!(n.set_image_corners(pts([(0.0, 0.0), (10.0, 0.0), (0.0, 10.0), (10.0, 10.0)])).is_err());
        assert_eq!(n, before);
        n.extras_mut().image_quad = Some(Box::new(pts([(0.0, 0.0), (10.0, 0.0), (0.0, 10.0), (10.0, 10.0)])));
        assert_eq!(n.image_quad(), None);
        assert_eq!(n.image_corners(), Some(pts([(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)])));
    }

    #[test]
    fn the_mesh_is_as_fine_as_the_perspective_needs() {
        let mut n = image(400, 400);
        n.set_image_corners(pts([(10.0, 0.0), (400.0, 5.0), (390.0, 400.0), (0.0, 395.0)])).unwrap();
        let NodeKind::Image(im) = n.kind.clone() else { panic!("an image") };
        let gentle = cells(&n.image_map().unwrap(), &im, Density::PAGE);
        n.set_image_corners(pts([(160.0, 0.0), (240.0, 0.0), (400.0, 400.0), (0.0, 400.0)])).unwrap();
        let strong = cells(&n.image_map().unwrap(), &im, Density::PAGE);
        assert!(gentle < strong && strong <= MAX_CELLS, "{gentle} {strong}");
        // Three by two pixels drawn as large still bend, in up to MIN_CELL_CAP cells.
        let mut tiny = image(3, 2);
        tiny.set_image_corners(pts([(160.0, 0.0), (240.0, 0.0), (400.0, 400.0), (0.0, 400.0)])).unwrap();
        let NodeKind::Image(tim) = &tiny.kind else { panic!("an image") };
        assert_eq!(cells(&tiny.image_map().unwrap(), tim, Density::PAGE), MIN_CELL_CAP.min(strong));
        // Drawn small, the same corners take only cells of at least 4 pixels: their longest side,
        // about 431 pt, is about 17 pixels at 1/25 px a point, room for 4 cells.
        let map = n.image_map().unwrap();
        assert!(cells(&map, &im, Density::at_scale(1.0 / 25.0)) <= 4);
        // Drawn large, a quarter of a pixel is finer than 0.05 pt, so no fewer cells than on the page.
        assert!(cells(&map, &im, Density::at_scale(8.0)) >= strong);
        // Each cell of the page's mesh is at least 2 pt along the longest side: about 32 pt takes
        // 16 at most.
        let mut small = image(400, 400);
        small.set_image_corners(pts([(12.0, 0.0), (18.0, 0.0), (30.0, 30.0), (0.0, 30.0)])).unwrap();
        assert!(cells(&small.image_map().unwrap(), &im, Density::PAGE) <= 16);
    }

    /// Each piece of the art reaches the density's overlap past its triangle, and the corners clip
    /// the art, so nothing draws outside them.
    #[test]
    fn the_pieces_overlap_and_the_corners_clip_them() {
        let mut n = image(100, 100);
        let q = pts([(40.0, 0.0), (60.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        n.set_image_corners(q).unwrap();
        let art = distorted_art_at(&n, Density { tolerance: 1.0, min_cell: 1.0, overlap: 3.0 }).unwrap();
        let Some([edge, _]) = art.children().map(Vec::as_slice) else { panic!("an edge and a mesh") };
        assert!(matches!(edge.kind, NodeKind::Path { clipping: true, .. }));
        assert_eq!(edge.geometric_bounds(), Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
        let pieces = distorted_pieces(&art);
        assert!(pieces.len() >= 2);
        // The first piece's clip triangle reaches 3 pt past the image's top left corner.
        let Some([clip, _]) = pieces[0].children().map(Vec::as_slice) else { panic!("a clip and an image") };
        let b = clip.geometric_bounds().unwrap();
        assert!(b.y0 < -2.0, "{b:?}");
    }

    #[test]
    fn the_art_puts_the_pixel_corners_on_the_quad() {
        let mut n = image(100, 100);
        n.opacity = 0.5;
        let q = pts([(40.0, 0.0), (60.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        n.set_image_corners(q).unwrap();
        let art = distorted_art(&n).unwrap();
        assert_eq!(art.opacity, 1.0);
        let mut corners_seen = [false; 4];
        art.walk(&mut |c| {
            if let NodeKind::Image(im) = &c.kind {
                for (i, (p, want)) in rect_corners(Rect::new(0.0, 0.0, 100.0, 100.0)).iter().zip(q).enumerate() {
                    if (im.xf * *p).distance(want) < 1e-9 {
                        corners_seen[i] = true;
                    }
                }
            }
        });
        assert_eq!(corners_seen, [true; 4]);
        assert!(distorted_art(&image(4, 4)).is_none());
        // A hidden image that a view draws anyway gives art that draws.
        n.visible = false;
        assert!(distorted_art(&n).unwrap().visible);
        // The image's own fills and strokes stay off the mesh's group.
        let stroke = vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK);
        n.appearance = crate::Appearance::basic(vectorcraft_color::Paint::None, stroke, 1.0).into();
        assert!(distorted_art(&n).unwrap().appearance.items.is_empty());
    }

    #[test]
    fn deep_expansion_reaches_a_distorted_image_inside_a_group() {
        let mut n = image(100, 100);
        n.set_image_corners(pts([(40.0, 0.0), (60.0, 0.0), (100.0, 100.0), (0.0, 100.0)])).unwrap();
        let inner = Node::new(NodeId(2), NodeKind::Group { children: vec![std::sync::Arc::new(n)], clip: false });
        let outer = Node::new(NodeId(3), NodeKind::Group { children: vec![std::sync::Arc::new(inner)], clip: false });
        let out = crate::live::expand_deep(&outer, None);
        let mut distorted = 0;
        let mut pieces = 0;
        out.walk(&mut |c| {
            distorted += usize::from(c.is_distorted_image());
            pieces += usize::from(matches!(c.kind, NodeKind::Image(_)));
        });
        assert_eq!(distorted, 0);
        assert!(pieces > 1, "{pieces}");
    }

    /// A distorted image with an effect of its own stays an image with its appearance when the
    /// art around it is expanded, so the effect still applies where the art is drawn.
    #[test]
    fn deep_expansion_keeps_a_distorted_image_with_its_own_appearance() {
        let mut n = image(100, 100);
        n.set_image_corners(pts([(40.0, 0.0), (60.0, 0.0), (100.0, 100.0), (0.0, 100.0)])).unwrap();
        let effect = crate::Effect { id: "distort.transform".into(), params: serde_json::json!({"moveH": 40}), visible: true };
        n.appearance = crate::Appearance { effects: vec![effect], ..Default::default() }.into();
        let inner = Node::new(NodeId(2), NodeKind::Group { children: vec![std::sync::Arc::new(n.clone())], clip: false });
        let outer = Node::new(NodeId(3), NodeKind::Group { children: vec![std::sync::Arc::new(inner)], clip: false });
        let out = crate::live::expand_deep(&outer, None);
        let mut kept = vec![];
        out.walk(&mut |c| {
            if c.is_distorted_image() {
                kept.push(c.appearance.clone());
            }
        });
        assert_eq!(kept, vec![n.appearance]);
    }
}
