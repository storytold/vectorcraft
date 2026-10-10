//! A portable surface-of-revolution evaluator. No windowing, GPU or document dependencies.
//!
//! Open profiles generate uncapped surfaces. Faces are projected and sorted back to front,
//! then flat shaded. This vector preview uses a painter's algorithm, not a depth buffer:
//! intersecting profiles can have ambiguous visibility and are not solid modelling inputs.
#![forbid(unsafe_code)]

mod visibility;
pub use visibility::{VisibleFace, visible_faces};

use vectorcraft_geom::{BezPath, ParamCurve, PathData, Point, Rect};

/// Resource limits shared by desktop, CLI and browser evaluation.
const MAX_PROFILE_POINTS: usize = 1024;
const MAX_FACES: usize = 32768;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct V3(f64, f64, f64);

impl V3 {
    fn sub(self, b: Self) -> Self {
        Self(self.0 - b.0, self.1 - b.1, self.2 - b.2)
    }
    fn cross(self, b: Self) -> Self {
        Self(self.1 * b.2 - self.2 * b.1, self.2 * b.0 - self.0 * b.2, self.0 * b.1 - self.1 * b.0)
    }
    fn dot(self, b: Self) -> f64 {
        self.0 * b.0 + self.1 * b.1 + self.2 * b.2
    }
    fn unit(self) -> Self {
        let l = self.dot(self).sqrt();
        if l < 1e-12 { Self::default() } else { Self(self.0 / l, self.1 / l, self.2 / l) }
    }
}

/// The vertical axis is offset outward from the selected edge of the profile's bounds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Edge {
    #[default]
    Left,
    Right,
}

/// All angles are degrees; offset is in document points, lighting values are 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Revolve {
    pub angle: f64,
    pub offset: f64,
    pub edge: Edge,
    pub rotation_x: f64,
    pub rotation_y: f64,
    pub rotation_z: f64,
    /// Perspective strength, 0 for orthographic projection, up to 1.
    pub perspective: f64,
    /// Radial subdivisions for a full turn, clamped to 8..128.
    pub segments: usize,
    pub light_azimuth: f64,
    pub light_elevation: f64,
    pub light_intensity: f64,
    pub ambient: f64,
    pub shade: bool,
}

impl Default for Revolve {
    fn default() -> Self {
        Self {
            angle: 360.0,
            offset: 0.0,
            edge: Edge::Left,
            rotation_x: 0.0,
            rotation_y: 0.0,
            rotation_z: 0.0,
            perspective: 0.0,
            segments: 64,
            light_azimuth: -45.0,
            light_elevation: 45.0,
            light_intensity: 0.8,
            ambient: 0.25,
            shade: true,
        }
    }
}

fn finite(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    if v.is_finite() { v.clamp(min, max) } else { fallback }
}

impl Revolve {
    /// Unit bases of the rings that adjust the Euler X, Y and Z view parameters.
    /// X turns before Y and Z, Y before Z, and Z turns in the screen plane.
    /// Each pair has a cross product pointing along that parameter's rotation axis.
    pub fn rotation_planes(self) -> [[[f64; 3]; 2]; 3] {
        let o = self.sanitized();
        let x_frame = Self { rotation_x: 0.0, ..o };
        let y_frame = Self { rotation_x: 0.0, rotation_y: 0.0, ..o };
        [
            [x_frame.rotate(V3(0.0, 1.0, 0.0)), x_frame.rotate(V3(0.0, 0.0, 1.0))],
            [y_frame.rotate(V3(1.0, 0.0, 0.0)), y_frame.rotate(V3(0.0, 0.0, -1.0))],
            [V3(1.0, 0.0, 0.0), V3(0.0, 1.0, 0.0)],
        ]
        .map(|plane| plane.map(|v| [v.0, v.1, v.2]))
    }

    /// The fixed centre of view rotation, in document coordinates.
    pub fn rotation_origin(self, bounds: Rect) -> Point {
        let o = self.sanitized();
        Point::new(if o.edge == Edge::Left { bounds.x0 - o.offset } else { bounds.x1 + o.offset }, bounds.center().y)
    }

    /// Rotated unit axes for a view-orientation guide (screen Y points down).
    pub fn view_axes(self) -> [Point; 3] {
        [V3(1.0, 0.0, 0.0), V3(0.0, 1.0, 0.0), V3(0.0, 0.0, 1.0)].map(|v| {
            let p = self.sanitized().rotate(v);
            Point::new(p.0, p.1)
        })
    }

    /// The same projected vertical axis used by the surface evaluator.
    pub fn projected_axis(self, bounds: Rect) -> [Point; 2] {
        let o = self.sanitized();
        let axis = match o.edge {
            Edge::Left => bounds.x0 - o.offset,
            Edge::Right => bounds.x1 + o.offset,
        };
        let center_y = bounds.center().y;
        let radius = (bounds.x0 - axis).abs().max((bounds.x1 - axis).abs());
        let distance = 4.0 * radius.max(bounds.height().abs()).max(1.0);
        [bounds.y0, bounds.y1].map(|y| {
            let v = o.rotate(V3(0.0, y - center_y, 0.0));
            let factor = distance / (distance - v.2 * o.perspective);
            Point::new(axis + v.0 * factor, center_y + v.1 * factor)
        })
    }

    /// Directional light in view space, shared by shading and interactive guides.
    pub fn light_direction(self) -> [f64; 3] {
        let o = self.sanitized();
        let (sl, cl) = o.light_elevation.to_radians().sin_cos();
        let (sa, ca) = o.light_azimuth.to_radians().sin_cos();
        [cl * sa, -sl, cl * ca]
    }

    /// Light-direction orbits as (centre, cosine basis, sine basis), in view space.
    /// The first changes azimuth at the current elevation. The second changes elevation
    /// at the current azimuth. The meridian's valid parameter range is -90..90 degrees.
    pub fn light_orbits(self) -> [[[f64; 3]; 3]; 2] {
        let o = self.sanitized();
        let (se, ce) = o.light_elevation.to_radians().sin_cos();
        let (sa, ca) = o.light_azimuth.to_radians().sin_cos();
        [[[0.0, -se, 0.0], [0.0, 0.0, ce], [ce, 0.0, 0.0]], [[0.0, 0.0, 0.0], [sa, 0.0, ca], [0.0, -1.0, 0.0]]]
    }

    /// Move a light on its current hemisphere, preserving the azimuth at the poles.
    /// A light behind the object stays behind when its projected handle is dragged.
    pub fn light_at_on_side(self, point: Point) -> [f64; 2] {
        let o = self.sanitized();
        let point = Point::new(finite(point.x, 0.0, -1e8, 1e8), finite(point.y, 0.0, -1e8, 1e8));
        let length = point.x.hypot(point.y).max(1.0);
        let (x, y) = (point.x / length, point.y / length);
        let z = (1.0 - x * x - y * y).max(0.0).sqrt().copysign(o.light_direction()[2]);
        let azimuth = if x.hypot(z) < 1e-9 { o.light_azimuth } else { x.atan2(z).to_degrees() };
        [azimuth, (-y).clamp(-1.0, 1.0).asin().to_degrees()]
    }

    /// A dragged light position on the front hemisphere, in normalized view coordinates.
    pub fn light_at(point: Point) -> [f64; 2] {
        let length = point.x.hypot(point.y).max(1.0);
        let (x, y) = (point.x / length, point.y / length);
        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
        [x.atan2(z).to_degrees(), (-y).clamp(-1.0, 1.0).asin().to_degrees()]
    }

    fn sanitized(self) -> Self {
        Self {
            angle: finite(self.angle, 360.0, 0.0, 360.0),
            offset: finite(self.offset, 0.0, 0.0, 1e5),
            rotation_x: finite(self.rotation_x, 0.0, -360.0, 360.0),
            rotation_y: finite(self.rotation_y, 0.0, -360.0, 360.0),
            rotation_z: finite(self.rotation_z, 0.0, -360.0, 360.0),
            perspective: finite(self.perspective, 0.0, 0.0, 1.0),
            segments: self.segments.clamp(8, 128),
            light_azimuth: finite(self.light_azimuth, -45.0, -360.0, 360.0),
            light_elevation: finite(self.light_elevation, 45.0, -90.0, 90.0),
            light_intensity: finite(self.light_intensity, 0.8, 0.0, 1.0),
            ambient: finite(self.ambient, 0.25, 0.0, 1.0),
            ..self
        }
    }
    fn rotate(self, v: V3) -> V3 {
        let (sx, cx) = self.rotation_x.to_radians().sin_cos();
        let (sy, cy) = self.rotation_y.to_radians().sin_cos();
        let (sz, cz) = self.rotation_z.to_radians().sin_cos();
        let v = V3(v.0, cx * v.1 - sx * v.2, sx * v.1 + cx * v.2);
        let v = V3(cy * v.0 + sy * v.2, v.1, -sy * v.0 + cy * v.2);
        V3(cz * v.0 - sz * v.1, sz * v.0 + cz * v.1, v.2)
    }
}

/// One opaque vector polygon, already projected. Brightness multiplies the source colour.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub points: Vec<Point>,
    pub depth: f64,
    pub brightness: f64,
}

impl Face {
    pub fn path(&self) -> PathData {
        let mut bp = BezPath::new();
        if let Some(first) = self.points.first() {
            bp.move_to(*first);
            for p in self.points.iter().skip(1) {
                bp.line_to(*p);
            }
            bp.close_path();
        }
        PathData::from_bezpath(&bp)
    }
}

/// Evaluate a profile in document space. All subpaths share one axis from `bounds`.
/// Invalid or excessively complex inputs return an error, never a partial surface.
pub fn revolve(path: &PathData, bounds: Rect, options: Revolve) -> Result<Vec<Face>, String> {
    let o = options.sanitized();
    if ![bounds.x0, bounds.y0, bounds.x1, bounds.y1].iter().all(|v| v.is_finite() && v.abs() <= 1e8) {
        return Err("Revolve requires finite profile coordinates within 100 million points".into());
    }
    let axis = match o.edge {
        Edge::Left => bounds.x0 - o.offset,
        Edge::Right => bounds.x1 + o.offset,
    };
    let center_y = (bounds.y0 + bounds.y1) * 0.5;
    let radius = (bounds.x0 - axis).abs().max((bounds.x1 - axis).abs());
    let distance = 4.0 * radius.max(bounds.height().abs()).max(1.0);
    let steps = ((o.segments as f64 * o.angle / 360.0).ceil() as usize).max(1);
    let sweep = o.angle.to_radians();
    let [lx, ly, lz] = o.light_direction();
    let light = V3(lx, ly, lz);
    let mut faces = Vec::new();
    let mut count = 0;
    for sp in &path.subpaths {
        let mut profile = Vec::new();
        if let Some(first) = sp.anchors.first() {
            profile.push(first.p);
            count += 1;
            if count > MAX_PROFILE_POINTS {
                return Err("Revolve profile is too complex".into());
            }
        }
        let segments = if sp.closed { sp.anchors.len() } else { sp.anchors.len().saturating_sub(1) };
        // Bound work before sampling any data. Curves get sixteen samples, straight edges one.
        if segments > MAX_PROFILE_POINTS {
            return Err("Revolve profile is too complex".into());
        }
        for i in 0..segments {
            let curve = sp.segment(i);
            let samples = if sp.segment_is_line(i) { 1 } else { 16 };
            for k in 1..=samples {
                profile.push(curve.eval(k as f64 / samples as f64));
                count += 1;
                if count > MAX_PROFILE_POINTS {
                    return Err("Revolve profile is too complex".into());
                }
            }
        }
        for pair in profile.windows(2) {
            let [a, b] = pair else { continue };
            if ![a.x, a.y, b.x, b.y].iter().all(|v| v.is_finite() && v.abs() <= 1e8) {
                return Err("Revolve profile contains invalid coordinates".into());
            }
            for j in 0..steps {
                let vertex = |p: Point, step: usize| {
                    let (s, c) = (sweep * step as f64 / steps as f64).sin_cos();
                    o.rotate(V3((p.x - axis) * c, p.y - center_y, (p.x - axis) * s))
                };
                let vertices = [vertex(*a, j), vertex(*b, j), vertex(*b, j + 1), vertex(*a, j + 1)];
                // The diagonal gives a nonzero normal even when one profile endpoint is on-axis.
                let mut normal = vertices[1].sub(vertices[0]).cross(vertices[2].sub(vertices[0])).unit();
                if normal.dot(normal) < 0.5 {
                    normal = vertices[2].sub(vertices[0]).cross(vertices[3].sub(vertices[0])).unit();
                }
                if normal.dot(normal) < 0.5 {
                    continue;
                }
                // Profiles may run up or down. Shade the visible side of this two-sided surface.
                if normal.2 < 0.0 {
                    normal = V3(-normal.0, -normal.1, -normal.2);
                }
                let brightness = if o.shade { (o.ambient + o.light_intensity * normal.dot(light).max(0.0)).clamp(0.0, 1.0) } else { 1.0 };
                let points: Vec<_> = vertices
                    .iter()
                    .map(|v| {
                        let factor = distance / (distance - v.2 * o.perspective);
                        Point::new(axis + v.0 * factor, center_y + v.1 * factor)
                    })
                    .collect();
                if points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
                    return Err("Revolve projection is not finite".into());
                }
                let area = points
                    .iter()
                    .zip(points.iter().cycle().skip(1))
                    .take(4)
                    .map(|(a, b)| (a.x - axis) * (b.y - center_y) - (b.x - axis) * (a.y - center_y))
                    .sum::<f64>();
                if area.abs() < 1e-10 {
                    continue;
                }
                faces.push(Face { points, depth: vertices.iter().map(|v| v.2).sum::<f64>() / 4.0, brightness });
                if faces.len() > MAX_FACES {
                    return Err("Revolve surface is too complex; reduce Segments or simplify the profile".into());
                }
            }
        }
    }
    faces.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    Ok(faces)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn light_orbits_and_handle_projection_match_the_shading_direction_on_both_sides() {
        for azimuth in [-170.0f64, -45.0, 0.0, 83.0, 167.0] {
            for elevation in [-85.0f64, -30.0, 0.0, 45.0, 90.0] {
                let o = Revolve { light_azimuth: azimuth, light_elevation: elevation, ..Default::default() };
                let direction = o.light_direction();
                for ([center, u, v], angle) in o.light_orbits().into_iter().zip([azimuth, elevation]) {
                    let (s, c) = angle.to_radians().sin_cos();
                    for i in 0..3 {
                        assert!((center[i] + u[i] * c + v[i] * s - direction[i]).abs() < 1e-12);
                    }
                }
                let [a, e] = o.light_at_on_side(Point::new(direction[0], direction[1]));
                assert!((a - azimuth).abs() < 1e-9 && (e - elevation).abs() < 1e-9, "{azimuth}, {elevation} -> {a}, {e}");
            }
        }
        let o = Revolve { light_azimuth: 135.0, ..Default::default() };
        let [azimuth, elevation] = o.light_at_on_side(Point::new(0.3, -0.4));
        let [x, y, z] = Revolve { light_azimuth: azimuth, light_elevation: elevation, ..o }.light_direction();
        assert!((x - 0.3).abs() < 1e-12 && (y + 0.4).abs() < 1e-12 && z < 0.0);
        let [a, e] = o.light_at_on_side(Point::new(f64::NAN, f64::INFINITY));
        assert!(a.is_finite() && e.is_finite());
    }

    #[test]
    fn ring_axes_match_euler_parameter_rotation_and_origin_stays_fixed() {
        let o = Revolve { rotation_x: 27.0, rotation_y: -39.0, rotation_z: 53.0, ..Default::default() };
        let seed = V3(0.3, -0.5, 0.7);
        let before = o.rotate(seed);
        let planes = o.rotation_planes();
        for (axis, [u, v]) in planes.iter().enumerate() {
            let normal = V3(u[0], u[1], u[2]).cross(V3(v[0], v[1], v[2]));
            assert!((normal.dot(normal) - 1.0).abs() < 1e-12);
            let mut changed = o;
            match axis {
                0 => changed.rotation_x += 0.0001,
                1 => changed.rotation_y += 0.0001,
                _ => changed.rotation_z += 0.0001,
            }
            let actual = changed.rotate(seed).sub(before);
            let expected = normal.cross(before);
            let step = 0.0001f64.to_radians();
            for (a, b) in [(actual.0 / step, expected.0), (actual.1 / step, expected.1), (actual.2 / step, expected.2)] {
                assert!((a - b).abs() < 1e-5);
            }
        }
        assert_eq!(Revolve { edge: Edge::Right, offset: 12.0, ..o }.rotation_origin(Rect::new(20.0, 30.0, 80.0, 100.0)), Point::new(92.0, 65.0));
    }
    #[test]
    fn guides_match_projection_and_dragged_light_matches_shading_direction() {
        let o = Revolve { rotation_x: 0.0, rotation_y: 0.0, rotation_z: 0.0, offset: 10.0, ..Default::default() };
        assert_eq!(o.view_axes(), [Point::new(1.0, 0.0), Point::new(0.0, 1.0), Point::new(0.0, 0.0)]);
        assert_eq!(o.projected_axis(Rect::new(20.0, 30.0, 80.0, 100.0)), [Point::new(10.0, 30.0), Point::new(10.0, 100.0)]);
        let [azimuth, elevation] = Revolve::light_at(Point::new(0.3, -0.4));
        let [x, y, z] = Revolve { light_azimuth: azimuth, light_elevation: elevation, ..o }.light_direction();
        assert!((x - 0.3).abs() < 1e-9 && (y + 0.4).abs() < 1e-9 && z > 0.0);
        let [azimuth, elevation] = Revolve::light_at(Point::new(2.0, -2.0));
        assert!(azimuth.is_finite() && elevation.is_finite() && elevation.abs() <= 90.0);
    }
    fn cylinder() -> (PathData, Rect) {
        let mut bp = BezPath::new();
        bp.move_to((40.0, 10.0));
        bp.line_to((40.0, 90.0));
        (PathData::from_bezpath(&bp), Rect::new(0.0, 10.0, 40.0, 90.0))
    }
    #[test]
    fn cylinder_has_expected_silhouette_depth_order_and_shading() {
        let (p, b) = cylinder();
        let faces = revolve(&p, b, Revolve { rotation_x: 0.0, rotation_y: 0.0, ..Default::default() }).unwrap();
        let points: Vec<_> = faces.iter().flat_map(|f| &f.points).collect();
        assert!((points.iter().map(|p| p.x).fold(f64::INFINITY, f64::min) + 40.0).abs() < 1e-6);
        assert!((points.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max) - 40.0).abs() < 1e-6);
        assert!(faces.windows(2).all(|w| w[0].depth <= w[1].depth));
        assert!(faces.iter().any(|f| f.brightness > 0.8) && faces.iter().any(|f| f.brightness < 0.4));
    }
    #[test]
    fn edge_offset_sweep_and_rotation_change_the_surface() {
        let (p, b) = cylinder();
        let base = revolve(&p, b, Revolve::default()).unwrap();
        for o in [
            Revolve { edge: Edge::Right, offset: 20.0, ..Default::default() },
            Revolve { angle: 120.0, ..Default::default() },
            Revolve { rotation_z: 60.0, perspective: 0.8, ..Default::default() },
        ] {
            assert_ne!(base, revolve(&p, b, o).unwrap());
        }
        assert!(revolve(&p, b, Revolve { angle: 0.0, ..Default::default() }).unwrap().is_empty());
    }
    #[test]
    fn malformed_and_degenerate_inputs_are_bounded() {
        let (p, b) = cylinder();
        let f = revolve(&p, b, Revolve { segments: usize::MAX, angle: f64::NAN, offset: f64::INFINITY, ..Default::default() }).unwrap();
        assert!(f.len() <= 128);
        assert!(revolve(&p, Rect::new(f64::NAN, 0.0, 1.0, 1.0), Revolve::default()).is_err());
        assert!(revolve(&PathData::default(), b, Revolve::default()).unwrap().is_empty());
    }
}
