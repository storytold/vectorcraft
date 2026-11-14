//! Extrude & Bevel, Inflate and Rotate (a flat plane) as projected, shaded vector facets.
//!
//! The 2D art is the front of the solid: its outline is extruded backwards along the view's
//! z-axis, about the centre of its bounds, at the art's own position and size. Planar faces (caps
//! and the rotated plane) are drawn whole, holes included, so no triangulation is needed; curved
//! faces are cut into small facets. Colored Inflate interpolates shared membrane normals;
//! bevels and side walls retain their planar lighting.
//!
//! Visibility is a painter's algorithm, as for Revolve: faces of a closed solid that face away
//! from the viewer are left out, the rest are drawn back to front.

use vectorcraft_geom::{BezPath, FillRule, ParamCurve, PathData, Point, Rect, Vec2};

use crate::{MAX_FACES, MAX_SOLID_FACES, Revolve, V3, eye_distance, perspective_scale};

mod build;
mod face_map;
mod inflate;
mod membrane;
mod outline;
mod rings;

#[cfg(test)]
pub(crate) use build::EXTRUDED;
#[cfg(test)]
use build::{Camera, cuts, edge_distance, newell, section};
pub(crate) use build::{Face3, Keep, extrude_faces, plane_faces};
pub use build::{extrude, plane};
pub use face_map::{FaceMap, extrude_face_map, plane_face_map};
#[cfg(test)]
use inflate::GRID_WORK;
#[cfg(test)]
pub(crate) use inflate::INFLATED;
pub(crate) use inflate::inflate_faces;
pub use inflate::{inflate, inflate_each, inflate_together};
#[cfg(test)]
use membrane::{Edges, GRID_BUDGET, MAX_EDGE_ENTRIES, MAX_GRID_WORK, Membrane, cell_triangles};
pub use rings::TOO_COMPLEX_SURFACE as TOO_COMPLEX_SOLID;
use rings::signed_area;
#[cfg(test)]
use rings::{MAX_SOLID_POINTS, rings};

/// The view shared by every 3D effect: rotation, perspective and directional lighting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub rotation_x: f64,
    pub rotation_y: f64,
    pub rotation_z: f64,
    /// Perspective, 0° (none) to 160°, as Revolve's.
    pub perspective: f64,
    pub light_rotation: f64,
    pub light_height: f64,
    pub light_behind: bool,
    pub light_color: [f64; 3],
    pub light_intensity: f64,
    pub light_softness: f64,
    pub lights: Option<crate::Lights>,
    pub ambient: f64,
    pub shade: bool,
}

impl Default for View {
    fn default() -> Self {
        let r = Revolve::default();
        Self {
            rotation_x: r.rotation_x,
            rotation_y: r.rotation_y,
            rotation_z: r.rotation_z,
            perspective: r.perspective,
            light_rotation: r.light_rotation,
            light_height: r.light_height,
            light_behind: r.light_behind,
            light_color: r.light_color,
            light_intensity: r.light_intensity,
            light_softness: r.light_softness,
            lights: r.lights,
            ambient: r.ambient,
            shade: r.shade,
        }
    }
}

impl View {
    /// Whether the active light setup needs RGB shading rather than neutral brightness.
    pub fn has_colored_lights(self) -> bool {
        let view = self.revolve();
        view.lights.map_or(view.light_color != [1.0; 3], |lights| lights.iter().any(|light| light.color != [1.0; 3]))
    }

    /// The same rotation, projection and lighting maths as Revolve's.
    fn revolve(self) -> Revolve {
        Revolve {
            rotation_x: self.rotation_x,
            rotation_y: self.rotation_y,
            rotation_z: self.rotation_z,
            perspective: self.perspective,
            light_rotation: self.light_rotation,
            light_height: self.light_height,
            light_behind: self.light_behind,
            light_color: self.light_color,
            light_intensity: self.light_intensity,
            light_softness: self.light_softness,
            lights: self.lights,
            ambient: self.ambient,
            shade: self.shade,
            ..Revolve::default()
        }
        .sanitized()
    }
}

/// The shape of a bevel's cross-section, from the cap's edge (0) to the side wall (1).
// A custom profile takes 258 bytes; an extrusion copies its bevel a few times.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BevelShape {
    /// A straight 45° chamfer when width and height are equal.
    #[default]
    Classic,
    /// A convex quarter round.
    Rounded,
    /// A concave quarter round.
    Cove,
    /// Two flat steps.
    Step,
    /// A convex then concave S curve.
    Ogee,
    /// A cross-section of the user's own, drawn as an open path ([`BevelProfile`]).
    Custom(BevelProfile),
}

/// A custom bevel's cross-section: points from the cap's edge, (0, 0), to the side wall, (1, 1),
/// each a (share of the width, share of the height) pair in 0..1, joined by straight lines. The
/// shares are kept to 1/32,768 in a fixed array, so the profile is `Copy` like the built-in shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BevelProfile {
    points: [(u16, u16); BevelProfile::MAX_POINTS],
    len: u8,
}

impl BevelProfile {
    /// The most points a profile holds.
    pub const MAX_POINTS: usize = 64;

    /// A profile of `points`, or `None` unless there are 2 to [`Self::MAX_POINTS`] of them, every
    /// share is a number in 0..1, and they start at (0, 0) and end at (1, 1).
    pub fn new(points: &[(f64, f64)]) -> Option<Self> {
        const END: f64 = 1e-6;
        if !(2..=Self::MAX_POINTS).contains(&points.len()) || points.iter().any(|&(x, y)| !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y)) {
            return None;
        }
        let (first, last) = (points[0], points[points.len() - 1]);
        if first.0 > END || first.1 > END || last.0 < 1.0 - END || last.1 < 1.0 - END {
            return None;
        }
        let mut out = Self { points: [(0, 0); Self::MAX_POINTS], len: points.len() as u8 };
        let share = |v: f64| (v * f64::from(Self::ONE)).round() as u16;
        for (slot, &(x, y)) in out.points.iter_mut().zip(points) {
            *slot = (share(x), share(y));
        }
        out.points[0] = (0, 0);
        out.points[points.len() - 1] = (Self::ONE, Self::ONE);
        Some(out)
    }

    /// A whole share in the stored points: shares are kept to 1/32,768, so halves and quarters
    /// stay exact.
    const ONE: u16 = 1 << 15;

    /// The points, as (share of the width, share of the height).
    pub fn points(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        let share = |v: u16| f64::from(v) / f64::from(Self::ONE);
        self.points[..usize::from(self.len)].iter().map(move |&(x, y)| (share(x), share(y)))
    }

    /// The point at `s` (0..1), the points spread evenly along `s` and joined by straight lines.
    fn at(&self, s: f64) -> (f64, f64) {
        // A profile holds at least two points (`new`).
        let len = usize::from(self.len).max(2);
        let share = |v: u16| f64::from(v) / f64::from(Self::ONE);
        let k = s * (len - 1) as f64;
        let i = (k.floor() as usize).min(len - 2);
        let f = k - i as f64;
        let (a, b) = (self.points[i], self.points[i + 1]);
        let (ax, ay, bx, by) = (share(a.0), share(a.1), share(b.0), share(b.1));
        (ax + (bx - ax) * f, ay + (by - ay) * f)
    }
}

impl BevelShape {
    /// The built-in shapes.
    pub const ALL: [BevelShape; 5] = [Self::Classic, Self::Rounded, Self::Cove, Self::Step, Self::Ogee];

    /// The shape's parameter value; `custom` for a [`BevelShape::Custom`] one.
    pub fn id(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Rounded => "rounded",
            Self::Cove => "cove",
            Self::Step => "step",
            Self::Ogee => "ogee",
            Self::Custom(_) => "custom",
        }
    }

    /// The built-in shape named `s`.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.id() == s)
    }

    /// The cross-section at `s` (0..1): (share of the width, share of the height), both 0..1.
    fn at(self, s: f64) -> (f64, f64) {
        let s = s.clamp(0.0, 1.0);
        let q = s * std::f64::consts::FRAC_PI_2;
        match self {
            Self::Custom(profile) => profile.at(s),
            Self::Classic => (s, s),
            Self::Rounded => (1.0 - q.cos(), q.sin()),
            Self::Cove => (q.sin(), 1.0 - q.cos()),
            Self::Step if s >= 1.0 => (1.0, 1.0),
            Self::Step => {
                // Down, across, down, across.
                let k = s * 4.0;
                let (i, f) = (k.floor(), k.fract());
                match i as u8 {
                    0 => (0.0, f * 0.5),
                    1 => (f * 0.5, 0.5),
                    2 => (0.5, 0.5 + f * 0.5),
                    _ => (0.5 + f * 0.5, 1.0),
                }
            }
            Self::Ogee => {
                let h = (1.0 - (s * std::f64::consts::PI).cos()) / 2.0;
                (s, h)
            }
        }
    }

    /// Samples along the cross-section: the step shape needs its corners exactly.
    fn samples(self) -> Vec<f64> {
        match self {
            Self::Classic => vec![0.0, 1.0],
            Self::Step => vec![0.0, 0.25, 0.5, 0.75, 1.0],
            // Each of its points, where the straight lines between them turn.
            Self::Custom(profile) => {
                let last = (usize::from(profile.len) - 1) as f64;
                (0..usize::from(profile.len)).map(|i| i as f64 / last).collect()
            }
            _ => (0..=8).map(|i| f64::from(i) / 8.0).collect(),
        }
    }
}

/// A bevel along the front (and with `both_sides` the back) edge of an extrusion.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bevel {
    pub shape: BevelShape,
    /// How far the bevel reaches across the face, in points.
    pub width: f64,
    /// How deep it reaches along the extrusion, in points.
    pub height: f64,
    /// How many times the cross-section repeats across the width.
    pub repeat: usize,
    /// The share (0..1) of the width left flat between repeats.
    pub space: f64,
    /// Carved into the shape (true) or added around it (false).
    pub inside: bool,
    pub both_sides: bool,
}

impl Default for Bevel {
    fn default() -> Self {
        Self { shape: BevelShape::Classic, width: 4.0, height: 4.0, repeat: 1, space: 0.0, inside: false, both_sides: false }
    }
}

/// Extrude & Bevel's options. Lengths are points, angles degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Extrude {
    pub depth: f64,
    /// The turn of the back face against the front, about the shape's centre: 0° to 360°.
    pub twist: f64,
    /// The size of the back face, as a share (0.01..1) of the front's.
    pub taper: f64,
    /// Closed ends (a solid) or open ones (a hollow tube).
    pub cap: bool,
    pub bevel: Option<Bevel>,
    /// Keep the faces of a closed solid that are turned away from the viewer (Draw Hidden Faces,
    /// and the classic Wireframe surface), drawn behind the faces turned toward it.
    pub hidden: bool,
}

impl Default for Extrude {
    fn default() -> Self {
        Self { depth: 50.0, twist: 0.0, taper: 1.0, cap: true, bevel: None, hidden: false }
    }
}

/// Inflate's options.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Inflate {
    /// The flat depth between the front and back, in points.
    pub depth: f64,
    /// How far the surface puffs out (0..100 %; 50 % rounds it like a half sphere).
    pub volume: f64,
    /// Puff the back out too; otherwise it stays flat.
    pub both_sides: bool,
    /// The turn of the back against the front, about the shape's centre: 0° to 360°.
    pub twist: f64,
    /// The size of the back, as a share (0.01..1) of the front's.
    pub taper: f64,
    /// A closed back (a solid) or none, so the inflated front is a shell open behind.
    pub cap: bool,
}

impl Default for Inflate {
    fn default() -> Self {
        Self { depth: 0.0, volume: 50.0, both_sides: false, twist: 0.0, taper: 1.0, cap: true }
    }
}

/// A planar face of a solid that a graphic can be mapped onto: Extrude & Bevel's front cap (the
/// art itself, or a plane's only face) or its back cap.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PlanarFace {
    Front,
    Back,
}

impl PlanarFace {
    pub const ALL: [PlanarFace; 2] = [Self::Front, Self::Back];

    pub fn id(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::Back => "back",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.id() == s)
    }
}

/// One projected polygon. Holes are contours of their own (nonzero winding).
#[derive(Clone, Debug, PartialEq)]
pub struct Facet {
    pub contours: Vec<Vec<Point>>,
    pub depth: f64,
    pub brightness: f64,
    pub illumination: [f64; 3],
    /// The face's unit normal in view space (x right, y down, z toward the viewer), turned to the
    /// side the viewer sees.
    pub normal: [f64; 3],
    /// Smooth lighting on an inflated triangle, before clipping individual RGB channels.
    pub rgb_gradient: Option<crate::RgbGradient>,
    /// The facet's outline with the profile's curves kept as cubics, for [`Facet::path`]. The
    /// contours stay the polygon that visibility and depth use.
    pub outline: Option<PathData>,
    /// The facet is a piece of the solid's sides (Extrude & Bevel's walls and bevels, Inflate's
    /// sides) rather than of its ends (the caps, a plane, Inflate's front and back).
    pub side: bool,
    /// The facet is turned away from the viewer: a closed solid keeps it (seen edge-on, or with
    /// Draw Hidden Faces) and draws it behind the facets turned toward the viewer.
    pub away: bool,
    /// The planar end of the solid the facet is: Extrude & Bevel's front or back cap, or a plane
    /// (its front). `None` for every other facet.
    pub end: Option<PlanarFace>,
    /// The face's plane in view space and the projection it was drawn with, for its depth at a
    /// point of the drawing ([`Facet::depth_at`]). `None` for Revolve's faces and for a face whose
    /// corners lie more than 0.001 pt off one plane (a piece of a twisted wall).
    pub plane: Option<DepthPlane>,
    /// The contours in view space, before projection, for tests that measure depths.
    #[cfg(test)]
    view: Vec<Vec<V3>>,
}

/// A facet's plane in view space (x right, y down, z toward the viewer): the points `v` with
/// `normal · v = offset`, drawn by a projection about `center` with the eye `eye` in front of the
/// object's center plane (`None` without perspective).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DepthPlane {
    pub normal: [f64; 3],
    pub offset: f64,
    pub center: Point,
    pub eye: Option<f64>,
}

impl DepthPlane {
    /// The view-space depth (larger is nearer the viewer) of the point of the plane that projects
    /// to `p`, a point of the drawing. `None` when the plane is seen edge-on there or the point is
    /// not in front of the eye.
    pub fn depth_at(&self, p: Point) -> Option<f64> {
        let [nx, ny, nz] = self.normal;
        let (x, y) = (p.x - self.center.x, p.y - self.center.y);
        let z = match self.eye {
            None => (nz.abs() > 1e-9).then(|| (self.offset - nx * x - ny * y) / nz)?,
            Some(e) => {
                // On the ray through `p`, the point at depth z is at (x, y) × (e - z) / e.
                let a = (nx * x + ny * y) / e;
                let den = nz - a;
                let z = (den.abs() > 1e-9).then(|| (self.offset - a * e) / den)?;
                (z < e).then_some(z)?
            }
        };
        z.is_finite().then_some(z)
    }
}

impl Facet {
    /// The view-space depth of the facet at `p`, a point of the drawing ([`DepthPlane::depth_at`]).
    pub fn depth_at(&self, p: Point) -> Option<f64> {
        self.plane.as_ref()?.depth_at(p)
    }

    /// About how thick the facet is across: twice its area over its perimeter (a w × l rectangle
    /// long against its width gives about w).
    pub fn thickness(&self) -> f64 {
        let area: f64 = self.contours.iter().map(|c| signed_area(c)).sum::<f64>().abs();
        let perimeter: f64 =
            self.contours.iter().map(|c| c.iter().zip(c.iter().cycle().skip(1)).take(c.len()).map(|(a, b)| a.distance(*b)).sum::<f64>()).sum();
        if perimeter > 0.0 { 2.0 * area / perimeter } else { 0.0 }
    }

    /// The facet's outline: its curved outline when it has one, else its contours.
    pub fn path(&self) -> PathData {
        if let Some(path) = &self.outline {
            return path.clone();
        }
        let mut bp = BezPath::new();
        for c in &self.contours {
            let Some(first) = c.first() else { continue };
            bp.move_to(*first);
            for p in c.iter().skip(1) {
                bp.line_to(*p);
            }
            bp.close_path();
        }
        PathData::from_bezpath(&bp)
    }

    /// Revolve's face `face` as a facet: its polygon, curved outline, depth, normal and lighting.
    /// A face that joins the profile's ends round the axis is a side; a cap is an end.
    pub(crate) fn of_face(face: crate::Face) -> Self {
        Facet {
            contours: vec![face.points],
            depth: face.depth,
            brightness: face.brightness,
            illumination: face.illumination,
            normal: face.normal,
            rgb_gradient: None,
            outline: face.outline,
            side: !face.cap,
            // A revolved cap is not one of the planar faces graphics map onto.
            end: None,
            away: false,
            plane: None,
            #[cfg(test)]
            view: vec![],
        }
    }
}

fn finite(v: f64, fallback: f64, min: f64, max: f64) -> f64 {
    if v.is_finite() { v.clamp(min, max) } else { fallback }
}

impl Extrude {
    fn sanitized(self) -> Self {
        Self {
            depth: finite(self.depth, 50.0, 0.0, 2000.0),
            twist: finite(self.twist, 0.0, 0.0, 360.0),
            taper: finite(self.taper, 1.0, 0.01, 1.0),
            bevel: self.bevel.map(|b| Bevel {
                width: finite(b.width, 4.0, 0.0, 1000.0),
                height: finite(b.height, 4.0, 0.0, 1000.0),
                repeat: b.repeat.clamp(1, 32),
                space: finite(b.space, 0.0, 0.0, 0.9),
                ..b
            }),
            ..self
        }
    }
}

impl Inflate {
    fn sanitized(self) -> Self {
        Self {
            depth: finite(self.depth, 0.0, 0.0, 2000.0),
            volume: finite(self.volume, 50.0, 0.0, 100.0),
            twist: finite(self.twist, 0.0, 0.0, 360.0),
            taper: finite(self.taper, 1.0, 0.01, 1.0),
            ..self
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_inflate_shading;

#[cfg(test)]
mod tests_limits;
