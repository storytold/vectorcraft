//! Curved projected rings shared by a cap and its adjoining body faces.

use vectorcraft_geom::{CubicBez, ParamCurve, Shape};

use super::{BezPath, Face, PathData, Point, Rect, Revolve, V3, Vec2, eye_distance, perspective_scale, project_face};

const CURVE_TOLERANCE: f64 = 0.001;
const MAX_RING_CURVES: usize = 32_768;

pub(super) struct RingArc {
    curves: Vec<CubicBez>,
    points: Vec<Point>,
}

pub(super) struct Ring {
    pub arcs: Vec<RingArc>,
    center: V3,
    seed: [V3; 3],
    near: f64,
    far: f64,
}

fn project(v: V3, origin: Point, eye: Option<f64>) -> Point {
    let factor = perspective_scale(eye, v.2);
    Point::new(origin.x + v.0 * factor, origin.y + v.1 * factor)
}

fn fit_arc(
    sample: &impl Fn(f64) -> (Point, Vec2),
    a: f64,
    b: f64,
    depth: usize,
    curves: &mut Vec<CubicBez>,
    budget: &mut usize,
) -> Result<(), String> {
    let (p, dp) = sample(a);
    let (q, dq) = sample(b);
    let curve = CubicBez::new(p, p + dp * ((b - a) / 3.0), q - dq * ((b - a) / 3.0), q);
    let error = [0.25, 0.5, 0.75].into_iter().map(|t| curve.eval(t).distance(sample(a + (b - a) * t).0)).fold(0.0, f64::max);
    if error > CURVE_TOLERANCE && depth < 10 {
        let mid = (a + b) / 2.0;
        fit_arc(sample, a, mid, depth + 1, curves, budget)?;
        fit_arc(sample, mid, b, depth + 1, curves, budget)?;
    } else {
        *budget += 1;
        if *budget > MAX_RING_CURVES {
            return Err("Revolve surface is too complex; simplify the profile".into());
        }
        curves.push(curve);
    }
    Ok(())
}

impl Ring {
    pub fn new(end: Point, bounds: Rect, o: Revolve, steps: usize, budget: &mut usize) -> Result<Self, String> {
        let origin = o.rotation_origin(bounds);
        let radius = end.x - origin.x;
        let eye = eye_distance(o.perspective, (bounds.x0 - origin.x).abs().max((bounds.x1 - origin.x).abs()).max(bounds.height().abs()).max(1.0));
        let sweep = o.angle.to_radians();
        let vertex = |t: f64| o.rotate(V3(radius * t.cos(), end.y - origin.y, radius * t.sin()));
        let sample = |t: f64| {
            let v = vertex(t);
            let d = o.rotate(V3(-radius * t.sin(), 0.0, radius * t.cos()));
            let f = perspective_scale(eye, v.2);
            let df = eye.map_or(0.0, |e| if e - v.2 > e * 1e-3 { f * f * d.2 / e } else { 0.0 });
            (project(v, origin, eye), Vec2::new(d.0 * f + v.0 * df, d.1 * f + v.1 * df))
        };
        // Visibility gets a bounded dense polygon; output keeps the cubic outline.
        // Every body edge uses these same samples and curves, preventing rim gaps.
        let samples = (512.0 * o.angle / 360.0 / steps as f64).ceil().max(1.0) as usize;
        let center = o.rotate(V3(0.0, end.y - origin.y, 0.0));
        let (mut near, mut far) = (center.2, center.2);
        let mut arcs = vec![];
        for j in 0..steps {
            let (a, b) = (sweep * j as f64 / steps as f64, sweep * (j + 1) as f64 / steps as f64);
            let mut curves = vec![];
            fit_arc(&sample, a, b, 0, &mut curves, budget)?;
            let points = (0..=samples)
                .map(|k| {
                    let v = vertex(a + (b - a) * k as f64 / samples as f64);
                    near = near.max(v.2);
                    far = far.min(v.2);
                    project(v, origin, eye)
                })
                .collect();
            arcs.push(RingArc { curves, points });
        }
        Ok(Self { arcs, center, seed: [center, vertex(0.0), vertex(sweep / steps as f64)], near, far })
    }

    pub fn face(&self, bounds: Rect, o: Revolve, light: V3, outward: f64) -> Result<Option<Face>, String> {
        let origin = o.rotation_origin(bounds);
        let eye = eye_distance(o.perspective, (bounds.x0 - origin.x).abs().max((bounds.x1 - origin.x).abs()).max(bounds.height().abs()).max(1.0));
        let Some(mut face) = project_face(&self.seed, None, o, origin, eye, light)? else { return Ok(None) };
        let Some(first) = self.arcs.first().and_then(|a| a.points.first()).copied() else { return Ok(None) };
        let center = project(self.center, origin, eye);
        let mut path = BezPath::new();
        path.move_to(if o.angle == 360.0 { first } else { center });
        if o.angle < 360.0 {
            path.line_to(first);
        }
        face.points = vec![center, first];
        for arc in &self.arcs {
            arc.append(&mut path, &mut face.points, false);
        }
        path.close_path();
        if path.area().abs() < 1e-10 {
            return Ok(None);
        }
        face.outline = Some(PathData::from_bezpath(&path));
        face.cap = true;
        // An outward-facing disk covers its rim's seam strokes. An inward-facing
        // disk stays behind the body, including when the view turns upside down.
        face.depth = if o.rotate(V3(0.0, outward, 0.0)).2 >= 0.0 { self.near } else { self.far };
        Ok(Some(face))
    }
}

impl RingArc {
    fn append(&self, path: &mut BezPath, points: &mut Vec<Point>, reverse: bool) {
        if reverse {
            for c in self.curves.iter().rev() {
                path.curve_to(c.p2, c.p1, c.p0);
            }
            points.extend(self.points.iter().rev().skip(1));
        } else {
            for c in &self.curves {
                path.curve_to(c.p1, c.p2, c.p3);
            }
            points.extend(self.points.iter().skip(1));
        }
    }
}

pub(super) fn curve_edges(face: &mut Face, edges: &[(usize, &RingArc, bool)]) {
    if edges.is_empty() {
        return;
    }
    if face.points.len() == 4
        && let (Some((_, bottom, _)), Some((_, top, _))) =
            (edges.iter().find(|(edge, _, _)| *edge == 1), edges.iter().find(|(edge, _, _)| *edge == 3))
    {
        // Connecting equal angular samples avoids the long, narrow triangles
        // produced by ear clipping a cylinder's concave projected boundary.
        let strips: Vec<_> = top
            .points
            .windows(2)
            .zip(bottom.points.windows(2))
            .filter_map(|(a, b)| {
                let ([a0, a1], [b0, b1]) = (a, b) else { return None };
                Some(vec![*a0, *b0, *b1, *a1])
            })
            .collect();
        let areas: Vec<_> = strips
            .iter()
            .map(|p| {
                let origin = p.first().copied().unwrap_or_default();
                p.iter().zip(p.iter().cycle().skip(1)).take(p.len()).map(|(a, b)| (*a - origin).cross(*b - origin)).sum::<f64>()
            })
            .collect();
        // A grazing projection can fold a radial strip back over itself. Keep
        // the original boundary's winding there rather than unioning its folds.
        if !(areas.iter().any(|a| *a > 1e-10) && areas.iter().any(|a| *a < -1e-10)) {
            face.visibility_pieces = Some(strips);
        }
    }
    let Some(first) = face.points.first().copied() else { return };
    let mut path = BezPath::new();
    path.move_to(first);
    let mut points = vec![first];
    for (i, next) in face.points.iter().cycle().skip(1).take(face.points.len()).enumerate() {
        if let Some((_, arc, reverse)) = edges.iter().find(|(edge, _, _)| *edge == i) {
            arc.append(&mut path, &mut points, *reverse);
        } else {
            path.line_to(*next);
            points.push(*next);
        }
    }
    path.close_path();
    points.pop(); // The final closing edge already repeated the first point.
    face.points = points;
    face.outline = Some(PathData::from_bezpath(&path));
}
