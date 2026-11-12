//! Where the points of a planar face's own plane land on the page: the map a graphic drawn on
//! Extrude & Bevel's caps or on a plane goes through, with the same rotation, twist, taper and
//! perspective as the face itself.

use vectorcraft_geom::{Affine, Point, Rect};

use super::build::{Camera, Placing, Turn, check_bounds};
use super::{Extrude, PlanarFace, V3, View};

/// A planar face's map from the art's plane (document coordinates before the 3D effect) to the
/// page: a projective map, affine without perspective.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceMap {
    /// Rows of the map in homogeneous coordinates: a point `p` goes to `origin + (r0 · q, r1 · q) /
    /// (r2 · q)` with `q = (p.x, p.y, 1)`.
    rows: [[f64; 3]; 3],
    origin: Point,
    /// The eye's distance in front of the view's center; `None` without perspective.
    eye: Option<f64>,
    /// The face is turned toward the viewer. A plane, seen from either side, always is.
    pub visible: bool,
}

/// The smallest share of the eye's distance a point of the face may lie in front of the eye at:
/// the same limit [`crate::perspective_scale`] holds parts reaching past the eye to.
const NEAREST: f64 = 1e-3;

impl FaceMap {
    fn new(camera: &Camera, placing: &Placing, visible: bool) -> Self {
        let (x, y, c) = placing.frame();
        let (origin, eye) = camera.center_and_eye();
        let rows = match eye {
            Some(e) => [[e * x.0, e * y.0, e * c.0], [e * x.1, e * y.1, e * c.1], [-x.2, -y.2, e - c.2]],
            None => [[x.0, y.0, c.0], [x.1, y.1, c.1], [0.0, 0.0, 1.0]],
        };
        Self { rows, origin, eye, visible }
    }

    fn homogeneous(&self, p: Point) -> [f64; 3] {
        self.rows.map(|r| r[0] * p.x + r[1] * p.y + r[2])
    }

    /// Where art point `p` lands on the page. `None` when it lies at or behind the eye, or the
    /// result is not finite.
    pub fn to_page(&self, p: Point) -> Option<Point> {
        let [u, v, w] = self.homogeneous(p);
        let limit = self.eye.map_or(0.0, |e| e * NEAREST);
        if w.is_nan() || w <= limit {
            return None;
        }
        let q = Point::new(self.origin.x + u / w, self.origin.y + v / w);
        (q.x.is_finite() && q.y.is_finite()).then_some(q)
    }

    /// The art point that lands on page point `q`. `None` when the face is seen edge-on there or
    /// the point lies beyond the face's horizon.
    pub fn to_art(&self, q: Point) -> Option<Point> {
        let (u, v) = (q.x - self.origin.x, q.y - self.origin.y);
        let [r0, r1, r2] = self.rows;
        let (a, b, c) = (r0[0] - u * r2[0], r0[1] - u * r2[1], -(r0[2] - u * r2[2]));
        let (d, e, f) = (r1[0] - v * r2[0], r1[1] - v * r2[1], -(r1[2] - v * r2[2]));
        let det = a * e - b * d;
        let scale = (a.abs() + b.abs()).max(d.abs() + e.abs());
        if det.is_nan() || det.abs() <= 1e-12 * scale * scale {
            return None;
        }
        let p = Point::new((c * e - b * f) / det, (a * f - c * d) / det);
        // A point of the plane behind the eye projects to the same page point as one in front.
        (p.x.is_finite() && p.y.is_finite() && self.homogeneous(p)[2] > 0.0).then_some(p)
    }

    /// The part of rectangle `r` of the face's plane whose points [`FaceMap::to_page`] maps, kept
    /// back from the eye by twice its nearest distance, as a convex polygon: all of `r` without
    /// perspective. `None` when no part of `r` maps.
    pub fn mapped_part(&self, r: Rect) -> Option<Vec<Point>> {
        let corners = vec![Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
        let Some(e) = self.eye else { return Some(corners) };
        let [a, b, c] = self.rows[2];
        let limit = 2.0 * e * NEAREST;
        let side = |p: Point| a * p.x + b * p.y + c - limit;
        // The polygon cut by the line where `side` is 0 (one step of Sutherland and Hodgman's clip).
        let mut out = vec![];
        for (k, &p) in corners.iter().enumerate() {
            let q = corners[(k + 1) % corners.len()];
            let (sp, sq) = (side(p), side(q));
            if sp >= 0.0 {
                out.push(p);
            }
            if (sp >= 0.0) != (sq >= 0.0) {
                out.push(p.lerp(q, sp / (sp - sq)));
            }
        }
        (out.len() >= 3 && out.iter().all(|p| p.x.is_finite() && p.y.is_finite())).then_some(out)
    }

    /// Without perspective, the map is affine.
    pub fn is_affine(&self) -> bool {
        self.eye.is_none()
    }

    /// The affine map nearest the face's map around art point `p` (exact without perspective):
    /// what a gradient or a stroke on a graphic is drawn with. `None` where [`FaceMap::to_page`]
    /// has no point.
    pub fn local_affine(&self, p: Point) -> Option<Affine> {
        let at = self.to_page(p)?;
        let [u, v, w] = self.homogeneous(p);
        let [r0, r1, r2] = self.rows;
        // The derivatives of u / w and v / w along x and y.
        let d = |r: [f64; 3], n: f64, k: usize| (r[k] * w - n * r2[k]) / (w * w);
        let (a, b, c, e) = (d(r0, u, 0), d(r1, v, 0), d(r0, u, 1), d(r1, v, 1));
        let m = Affine::new([a, b, c, e, at.x - a * p.x - c * p.y, at.y - b * p.x - e * p.y]);
        m.as_coeffs().iter().all(|v| v.is_finite()).then_some(m)
    }
}

/// The map of Extrude & Bevel's cap `face` for a solid built with `bounds`, `options` and `view`,
/// as [`super::extrude`] builds it. With a depth of 0 the art is a plane, whose only face is the
/// front. `None` for a hollow solid (no caps), for the back of a plane, and for invalid bounds.
pub fn extrude_face_map(bounds: Rect, options: Extrude, view: View, face: PlanarFace) -> Option<FaceMap> {
    check_bounds(bounds).ok()?;
    let o = options.sanitized();
    if o.depth <= 0.0 {
        return (face == PlanarFace::Front).then(|| plane_face_map(bounds, view)).flatten();
    }
    if !o.cap {
        return None;
    }
    let camera = Camera::new(view, bounds, o.depth);
    let (t, side) = match face {
        PlanarFace::Front => (0.0, 1.0),
        PlanarFace::Back => (1.0, -1.0),
    };
    let placing = Placing::new(&camera, bounds.center(), Turn::new(bounds, o.twist, o.taper, t), o.depth / 2.0 - o.depth * t);
    let at = placing.place(bounds.center());
    let visible = camera.facing(at, camera.rotate(V3(0.0, 0.0, side))) > 0.0;
    Some(FaceMap::new(&camera, &placing, visible))
}

/// The map of the plane [`super::plane`] turns art with `bounds` into, with `view`.
pub fn plane_face_map(bounds: Rect, view: View) -> Option<FaceMap> {
    check_bounds(bounds).ok()?;
    let camera = Camera::new(view, bounds, 0.0);
    let placing = Placing::new(&camera, bounds.center(), Turn::new(bounds, 0.0, 1.0, 0.0), 0.0);
    Some(FaceMap::new(&camera, &placing, true))
}

#[cfg(test)]
#[path = "tests_face_map.rs"]
mod tests;
