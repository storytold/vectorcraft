//! Portable 3D effect geometry: Revolve, Extrude & Bevel, Inflate and Rotate. No windowing, GPU or
//! document dependencies.
//!
//! Open profiles optionally close their circular ends with caps. Faces are projected and sorted back to front,
//! then flat or smoothly shaded with vector gradients. This preview uses a painter's algorithm, not a depth buffer:
//! intersecting profiles can have ambiguous visibility and are not solid modelling inputs.
#![forbid(unsafe_code)]

mod solid;
mod visibility;
pub use solid::{Bevel, BevelShape, Extrude, Facet, Inflate, View, extrude, inflate, inflate_together, plane};
pub use visibility::{VisibleFace, visible_faces};
mod shading;
pub use shading::{LightGradient, RgbGradient, ShadeGradient};
mod lights;
pub use lights::{Light, Lights, MAX_LIGHTS};
mod caps;

use vectorcraft_geom::{BezPath, ParamCurve, PathData, Point, Rect, Vec2, kurbo::ParamCurveDeriv};

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

/// All angles are degrees; offset is in document points, lighting values are 0..1 (ambient 0..2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Revolve {
    pub angle: f64,
    pub offset: f64,
    pub edge: Edge,
    pub rotation_x: f64,
    pub rotation_y: f64,
    pub rotation_z: f64,
    /// Perspective, 0° (none) to 160°: see [`eye_distance`].
    pub perspective: f64,
    /// Radial subdivisions for a full turn, clamped to 8..128.
    pub segments: usize,
    /// The light's Rotation, −180° to 180°: round the object on the page, 0° on its right and
    /// counterclockwise.
    pub light_rotation: f64,
    /// The light's Height, 0° to 90°: from beside the object (0°) to straight in front of it.
    pub light_height: f64,
    /// The light is behind the object (Move light to the back of object) rather than in front.
    pub light_behind: bool,
    /// Opaque RGB light color, each channel 0..1. Ambient light stays neutral.
    pub light_color: [f64; 3],
    pub light_intensity: f64,
    /// Softness, 0 (a sharp fall-off) to 1: how far round the object the light reaches.
    pub light_softness: f64,
    /// Explicit light list. None retains the earlier single-light parameters exactly.
    pub lights: Option<Lights>,
    /// Ambient Light, 0 to 2.
    pub ambient: f64,
    pub shade: bool,
    /// Interpolate profile normals using vector gradients. Enabled by default.
    pub smooth_shading: bool,
    /// Close circular endpoint rings of open profiles. Partial sweeps get sectors;
    /// the angular cut sides are left open. Enabled by default.
    pub cap_ends: bool,
}

/// The default light's Rotation and Height: the light of the earlier default direction (45° to the
/// left of the eye and 45° up), to the left of and above the object, 30° in front of it.
pub const DEFAULT_LIGHT: (f64, f64) = (125.264_389_682_754_65, 30.0);

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
            light_rotation: DEFAULT_LIGHT.0,
            light_height: DEFAULT_LIGHT.1,
            light_behind: false,
            light_color: [1.0; 3],
            light_intensity: 0.8,
            light_softness: 0.0,
            lights: None,
            ambient: 0.25,
            shade: true,
            smooth_shading: true,
            cap_ends: true,
        }
    }
}

/// The widest documented Perspective.
pub const MAX_PERSPECTIVE: f64 = 160.0;

/// How far the eye is from the centre of an object of half size `size`, for a Perspective of
/// `angle` degrees: from where a sphere of √3 × `size` about the centre (the box around the object
/// fits inside it) fills that angle of the view. 0° is no perspective (`None`, the eye infinitely
/// far); at 160° the eye is just outside the sphere, so never inside the object.
fn eye_distance(angle: f64, size: f64) -> Option<f64> {
    let angle = finite(angle, 0.0, 0.0, MAX_PERSPECTIVE);
    (angle > 0.0).then(|| 3f64.sqrt() * size / (angle / 2.0).to_radians().sin())
}

/// The scale of a view-space point `z` in front of the object's centre plane, for an eye at
/// `eye`: 1 without perspective. Parts reaching past the eye's sphere (a wide outward bevel at a
/// wide angle) are held just in front of the eye rather than turned inside out.
fn perspective_scale(eye: Option<f64>, z: f64) -> f64 {
    eye.map_or(1.0, |e| e / (e - z).max(e * 1e-3))
}

/// The Perspective angle that shows exactly what the earlier 0–1 perspective strength did (files
/// from before the angle). The strength put the eye at 4 × the object's half size / strength, so
/// sin(angle / 2) = √3 × strength / 4: full strength is about 51°.
pub fn perspective_from_strength(strength: f64) -> f64 {
    let s = finite(strength, 0.0, 0.0, 1.0);
    2.0 * (3f64.sqrt() * s / 4.0).asin().to_degrees()
}

/// The Rotation, Height and side of the light an earlier effect set by its azimuth (degrees round
/// the vertical axis, 0° at the eye) and elevation (degrees up): the same direction, so it lights
/// the object exactly as before.
pub fn light_from_azimuth_elevation(azimuth: f64, elevation: f64) -> (f64, f64, bool) {
    let (azimuth, elevation) = (finite(azimuth, -45.0, -360.0, 360.0), finite(elevation, 45.0, -90.0, 90.0));
    let (sl, cl) = elevation.to_radians().sin_cos();
    let (sa, ca) = azimuth.to_radians().sin_cos();
    let [x, y, z] = [cl * sa, -sl, cl * ca];
    // Straight in front or behind, any Rotation is the same light.
    let rotation = if x.hypot(y) < 1e-12 { 0.0 } else { (-y).atan2(x).to_degrees() + 0.0 };
    (rotation, z.abs().min(1.0).asin().to_degrees(), z < 0.0)
}

/// How bright a face with unit normal `n` is under the light `l` (both in view space): the ambient
/// light plus the light's intensity where it reaches. Without softness the light falls off with
/// the cosine to nothing at 90°; softness wraps it further round the object, at 100 % to nothing
/// only straight away from it. Never brighter than the art's own colour.
fn brightness(o: &Revolve, n: V3, l: V3) -> f64 {
    if !o.shade {
        return 1.0;
    }
    if o.lights.is_some() {
        let rgb = illumination(o, n, l);
        return (rgb[0] + rgb[1] + rgb[2]) / 3.0;
    }
    let s = o.light_softness;
    let lit = ((n.dot(l) + s) / (1.0 + s)).max(0.0);
    (o.ambient + o.light_intensity * lit).clamp(0.0, 1.0)
}

fn direct_light(o: &Revolve, n: V3, l: V3) -> f64 {
    o.light_intensity * ((n.dot(l) + o.light_softness) / (1.0 + o.light_softness)).max(0.0)
}

fn illumination(o: &Revolve, n: V3, l: V3) -> [f64; 3] {
    if !o.shade {
        return [1.0; 3];
    }
    if o.lights.is_some() {
        return unclipped_illumination(o, n, l).map(|v| v.clamp(0.0, 1.0));
    }
    let direct = direct_light(o, n, l);
    o.light_color.map(|c| (o.ambient + direct * c).clamp(0.0, 1.0))
}

fn unclipped_illumination(o: &Revolve, n: V3, legacy_light: V3) -> [f64; 3] {
    let mut rgb = [o.ambient; 3];
    if let Some(lights) = o.lights {
        for light in lights.iter() {
            let direct = light.direct(n);
            for (out, color) in rgb.iter_mut().zip(light.color) {
                *out += direct * color;
            }
        }
    } else {
        let direct = direct_light(o, n, legacy_light);
        for (out, color) in rgb.iter_mut().zip(o.light_color) {
            *out += direct * color;
        }
    }
    rgb
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
        let eye = eye_distance(o.perspective, radius.max(bounds.height().abs()).max(1.0));
        [bounds.y0, bounds.y1].map(|y| {
            let v = o.rotate(V3(0.0, y - center_y, 0.0));
            let factor = perspective_scale(eye, v.2);
            Point::new(axis + v.0 * factor, center_y + v.1 * factor)
        })
    }

    /// Directional light in view space, shared by shading and interactive guides.
    pub fn light_direction(self) -> [f64; 3] {
        let o = self.sanitized();
        let (sr, cr) = o.light_rotation.to_radians().sin_cos();
        let (sh, ch) = o.light_height.to_radians().sin_cos();
        [ch * cr, -ch * sr, if o.light_behind { -sh } else { sh }]
    }

    /// Light-direction orbits as (centre, cosine basis, sine basis), in view space.
    /// The first changes the Rotation at the current Height: a circle on the page. The second
    /// changes the Height at the current Rotation; its parameter runs 0..90 degrees.
    pub fn light_orbits(self) -> [[[f64; 3]; 3]; 2] {
        let o = self.sanitized();
        let (sr, cr) = o.light_rotation.to_radians().sin_cos();
        let (sh, ch) = o.light_height.to_radians().sin_cos();
        let side = if o.light_behind { -1.0 } else { 1.0 };
        [[[0.0, 0.0, side * sh], [ch, 0.0, 0.0], [0.0, -ch, 0.0]], [[0.0, 0.0, 0.0], [cr, -sr, 0.0], [0.0, 0.0, side]]]
    }

    /// The Rotation and Height of a light dragged to `point` on the page (in units of the light
    /// guide's radius, y down), on the side it is already on: the further out, the lower. At the
    /// centre the Rotation is kept.
    pub fn light_at_on_side(self, point: Point) -> [f64; 2] {
        let o = self.sanitized();
        let point = Point::new(finite(point.x, 0.0, -1e8, 1e8), finite(point.y, 0.0, -1e8, 1e8));
        let r = point.x.hypot(point.y);
        let rotation = if r < 1e-9 { o.light_rotation } else { (-point.y).atan2(point.x).to_degrees() };
        [rotation, r.min(1.0).acos().to_degrees()]
    }

    fn sanitized(self) -> Self {
        Self {
            angle: finite(self.angle, 360.0, 0.0, 360.0),
            offset: finite(self.offset, 0.0, 0.0, 1e5),
            rotation_x: finite(self.rotation_x, 0.0, -360.0, 360.0),
            rotation_y: finite(self.rotation_y, 0.0, -360.0, 360.0),
            rotation_z: finite(self.rotation_z, 0.0, -360.0, 360.0),
            perspective: finite(self.perspective, 0.0, 0.0, MAX_PERSPECTIVE),
            segments: self.segments.clamp(8, 128),
            light_rotation: (finite(self.light_rotation, DEFAULT_LIGHT.0, -1e6, 1e6) + 180.0).rem_euclid(360.0) - 180.0,
            light_height: finite(self.light_height, DEFAULT_LIGHT.1, 0.0, 90.0),
            light_intensity: finite(self.light_intensity, 0.8, 0.0, 1.0),
            light_softness: finite(self.light_softness, 0.0, 0.0, 1.0),
            light_color: self.light_color.map(|c| finite(c, 1.0, 0.0, 1.0)),
            ambient: finite(self.ambient, 0.25, 0.0, 2.0),
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

/// One projected vector face. Brightness multiplies the source colour; an optional
/// gradient replaces that constant brightness for a smoothly shaded triangle.
#[derive(Clone, Debug, PartialEq)]
pub struct Face {
    pub points: Vec<Point>,
    pub depth: f64,
    pub brightness: f64,
    pub gradient: Option<ShadeGradient>,
    /// RGB illumination and its unclipped smooth ramp, independent of the legacy white ramp.
    pub illumination: [f64; 3],
    pub light_gradient: Option<LightGradient>,
    pub rgb_gradient: Option<RgbGradient>,
    /// Curved vector boundary; points remain a bounded visibility approximation.
    pub outline: Option<PathData>,
    /// A single endpoint disk or sector, with its axis centre first in points.
    pub cap: bool,
    /// Bounded radial strips for a body face joining two curved endpoint rings.
    pub visibility_pieces: Option<Vec<Vec<Point>>>,
}

impl Face {
    pub fn path(&self) -> PathData {
        if let Some(path) = &self.outline {
            return path.clone();
        }
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

/// Body faces and a cap's planar seed use identical two-sided shading and projection.
fn project_face(vertices: &[V3], normals: Option<&[V3]>, o: Revolve, origin: Point, eye: Option<f64>, light: V3) -> Result<Option<Face>, String> {
    let Some(first) = vertices.first() else { return Ok(None) };
    // A triangle fan also handles a quad whose first edge lies on the axis.
    let mut normal = vertices
        .get(1..)
        .unwrap_or_default()
        .windows(2)
        .filter_map(|pair| {
            let [a, b] = pair else { return None };
            Some(a.sub(*first).cross(b.sub(*first)).unit())
        })
        .find(|n| n.dot(*n) >= 0.5)
        .unwrap_or_default();
    if normal.dot(normal) < 0.5 {
        return Ok(None);
    }
    // Profiles may run up or down. Shade the visible side of this two-sided surface.
    if normal.2 < 0.0 {
        normal = V3(-normal.0, -normal.1, -normal.2);
    }
    let flat_brightness = brightness(&o, normal, light);
    let points: Vec<_> = vertices
        .iter()
        .map(|v| {
            let factor = perspective_scale(eye, v.2);
            Point::new(origin.x + v.0 * factor, origin.y + v.1 * factor)
        })
        .collect();
    if points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return Err("Revolve projection is not finite".into());
    }
    let area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(a, b)| (a.x - origin.x) * (b.y - origin.y) - (b.x - origin.x) * (a.y - origin.y))
        .sum::<f64>();
    if area.abs() < 1e-10 {
        return Ok(None);
    }
    let (brightness, gradient) = if let Some(normals) = normals {
        let values: Vec<_> = normals
            .iter()
            .map(|n| {
                let n = if n.dot(normal) < 0.0 { V3(-n.0, -n.1, -n.2) } else { *n };
                brightness(&o, n, light)
            })
            .collect();
        let brightness = if values.is_empty() { flat_brightness } else { values.iter().sum::<f64>() / values.len() as f64 };
        (brightness, ShadeGradient::triangle(&points, &values))
    } else {
        (flat_brightness, None)
    };
    let (illumination, light_gradient) = if let Some(normals) = normals {
        let normals: Vec<_> = normals.iter().map(|n| if n.dot(normal) < 0.0 { V3(-n.0, -n.1, -n.2) } else { *n }).collect();
        let mut rgb = [0.0; 3];
        for n in &normals {
            for (out, value) in rgb.iter_mut().zip(illumination(&o, *n, light)) {
                *out += value;
            }
        }
        let rgb = if normals.is_empty() { illumination(&o, normal, light) } else { rgb.map(|v| v / normals.len() as f64) };
        let ramp = if o.shade && o.lights.is_none() && o.light_color != [1.0; 3] {
            let values: Vec<_> = normals.iter().map(|n| direct_light(&o, *n, light)).collect();
            ShadeGradient::triangle(&points, &values).map(|diffuse| LightGradient { diffuse, ambient: o.ambient, color: o.light_color })
        } else {
            None
        };
        (rgb, ramp)
    } else {
        (illumination(&o, normal, light), None)
    };
    let rgb_gradient = normals.filter(|_| o.shade && o.lights.is_some()).and_then(|normals| {
        let values: Vec<_> =
            normals.iter().map(|n| unclipped_illumination(&o, if n.dot(normal) < 0.0 { V3(-n.0, -n.1, -n.2) } else { *n }, light)).collect();
        RgbGradient::triangle(&points, &values)
    });
    Ok(Some(Face {
        points,
        depth: vertices.iter().map(|v| v.2).sum::<f64>() / vertices.len() as f64,
        brightness,
        gradient,
        illumination,
        light_gradient,
        rgb_gradient,
        outline: None,
        cap: false,
        visibility_pieces: None,
    }))
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
    let eye = eye_distance(o.perspective, radius.max(bounds.height().abs()).max(1.0));
    let steps = ((o.segments as f64 * o.angle / 360.0).ceil() as usize).max(1);
    let sweep = o.angle.to_radians();
    let smooth = o.shade && o.smooth_shading;
    let [lx, ly, lz] = o.light_direction();
    let light = V3(lx, ly, lz);
    let mut faces = Vec::new();
    let vertex = |p: Point, step: usize| {
        let (s, c) = (sweep * step as f64 / steps as f64).sin_cos();
        o.rotate(V3((p.x - axis) * c, p.y - center_y, (p.x - axis) * s))
    };
    let normal = |tangent: Vec2, step: usize| {
        let (s, c) = (sweep * step as f64 / steps as f64).sin_cos();
        o.rotate(V3(tangent.y * c, -tangent.x, tangent.y * s).unit())
    };
    let mut mesh_cost = 0;
    let mut emit = |face: Option<Face>, cost: usize| -> Result<(), String> {
        if let Some(face) = face {
            faces.push(face);
            mesh_cost += cost;
            if mesh_cost > MAX_FACES {
                return Err("Revolve surface is too complex; reduce Segments or simplify the profile".into());
            }
        }
        Ok(())
    };
    let mut count = 0;
    let mut curve_budget = 0;
    for sp in &path.subpaths {
        let mut profile = Vec::new();
        let mut tangents = Vec::new();
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
                let t0 = (k - 1) as f64 / samples as f64;
                let t1 = k as f64 / samples as f64;
                let b = curve.eval(t1);
                let tangent = |t: f64| {
                    if !smooth {
                        return Vec2::ZERO;
                    }
                    let a = curve.eval(t0);
                    if samples == 1 {
                        return b - a;
                    }
                    let d = curve.deriv();
                    let v = d.eval(t).to_vec2();
                    if v.hypot2() > 1e-20 {
                        return v;
                    }
                    // Repeated endpoint handles still have a one-sided curve tangent.
                    let v = d.eval(t.clamp(1e-7, 1.0 - 1e-7)).to_vec2();
                    if v.hypot2() > 1e-20 { v } else { b - a }
                };
                tangents.push([tangent(t0), tangent(t1)]);
                profile.push(b);
                count += 1;
                if count > MAX_PROFILE_POINTS {
                    return Err("Revolve profile is too complex".into());
                }
            }
        }
        let ends = o.cap_ends && !sp.closed && profile.len() >= 2 && profile.first().zip(profile.last()).is_some_and(|(a, b)| a.distance(*b) > 1e-10);
        let mut rings = [None, None];
        if ends {
            for (ring, end) in rings.iter_mut().zip([profile.first(), profile.last()]) {
                if let Some(end) = end
                    && (end.x - axis).abs() > 1e-10
                {
                    *ring = Some(caps::Ring::new(*end, bounds, o, steps, &mut curve_budget)?);
                }
            }
        }
        for (i, (pair, [ta, tb])) in profile.windows(2).zip(tangents).enumerate() {
            let [a, b] = pair else { continue };
            if ![a.x, a.y, b.x, b.y].iter().all(|v| v.is_finite() && v.abs() <= 1e8) {
                return Err("Revolve profile contains invalid coordinates".into());
            }
            for j in 0..steps {
                let [v0, v1, v2, v3] = [vertex(*a, j), vertex(*b, j), vertex(*b, j + 1), vertex(*a, j + 1)];
                let first = (i == 0).then(|| rings.first().and_then(Option::as_ref).and_then(|r| r.arcs.get(j))).flatten();
                let last = (i + 2 == profile.len()).then(|| rings.last().and_then(Option::as_ref).and_then(|r| r.arcs.get(j))).flatten();
                let projected = |vertices: &[V3], normals: Option<&[V3]>, edges: &[(usize, &caps::RingArc, bool)]| {
                    let mut face = project_face(vertices, normals, o, Point::new(axis, center_y), eye, light)?;
                    if let Some(face) = &mut face {
                        caps::curve_edges(face, edges);
                    }
                    Ok::<_, String>(face)
                };
                if smooth {
                    let [n0, n1, n2, n3] = [normal(ta, j), normal(tb, j), normal(tb, j + 1), normal(ta, j + 1)];
                    emit(projected(&[v0, v1, v2], Some(&[n0, n1, n2]), &last.map(|arc| (1, arc, false)).into_iter().collect::<Vec<_>>())?, 1)?;
                    emit(projected(&[v0, v2, v3], Some(&[n0, n2, n3]), &first.map(|arc| (2, arc, true)).into_iter().collect::<Vec<_>>())?, 1)?;
                } else {
                    let edges: Vec<_> = last.map(|arc| (1, arc, false)).into_iter().chain(first.map(|arc| (3, arc, true))).collect();
                    emit(projected(&[v0, v1, v2, v3], None, &edges)?, 1)?;
                }
            }
        }
        for (index, ring) in rings.iter().enumerate().filter_map(|(i, r)| r.as_ref().map(|r| (i, r))) {
            let neighbours =
                if index == 0 { profile.first().zip(profile.get(1)) } else { profile.last().zip(profile.get(profile.len().saturating_sub(2))) };
            if let Some((end, neighbour)) = neighbours {
                let dy = end.y - neighbour.y;
                let outward = if dy.abs() > 1e-10 {
                    dy.signum()
                } else if end.y < center_y {
                    -1.0
                } else {
                    1.0
                };
                // Keep the original tessellation work budget, even though each cap
                // now produces only one curved vector face.
                emit(ring.face(bounds, o, light, outward)?, steps)?;
            }
        }
    }
    faces.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    Ok(faces)
}

#[cfg(test)]
mod tests;
