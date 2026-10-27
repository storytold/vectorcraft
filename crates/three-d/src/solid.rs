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

use crate::{MAX_FACES, Revolve, V3, eye_distance, perspective_scale};

mod build;
mod inflate;
mod membrane;
mod rings;

#[cfg(test)]
use build::{Camera, cuts, edge_distance, newell, section};
pub use build::{extrude, plane};
pub use inflate::{inflate, inflate_together};
#[cfg(test)]
use membrane::{Edges, Membrane, cell_triangles};
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
}

impl BevelShape {
    pub const ALL: [BevelShape; 5] = [Self::Classic, Self::Rounded, Self::Cove, Self::Step, Self::Ogee];

    pub fn id(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Rounded => "rounded",
            Self::Cove => "cove",
            Self::Step => "step",
            Self::Ogee => "ogee",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.id() == s)
    }

    /// The cross-section at `s` (0..1): (share of the width, share of the height), both 0..1.
    fn at(self, s: f64) -> (f64, f64) {
        let s = s.clamp(0.0, 1.0);
        let q = s * std::f64::consts::FRAC_PI_2;
        match self {
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
}

impl Default for Extrude {
    fn default() -> Self {
        Self { depth: 50.0, twist: 0.0, taper: 1.0, cap: true, bevel: None }
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

/// One projected polygon. Holes are contours of their own (nonzero winding).
#[derive(Clone, Debug, PartialEq)]
pub struct Facet {
    pub contours: Vec<Vec<Point>>,
    pub depth: f64,
    pub brightness: f64,
    pub illumination: [f64; 3],
    /// Smooth lighting on an inflated triangle, before clipping individual RGB channels.
    pub rgb_gradient: Option<crate::RgbGradient>,
    /// The contours in view space, before projection, for tests that measure depths.
    #[cfg(test)]
    view: Vec<Vec<V3>>,
}

impl Facet {
    /// About how thick the facet is across: twice its area over its perimeter (a w × l rectangle
    /// long against its width gives about w).
    pub fn thickness(&self) -> f64 {
        let area: f64 = self.contours.iter().map(|c| signed_area(c)).sum::<f64>().abs();
        let perimeter: f64 =
            self.contours.iter().map(|c| c.iter().zip(c.iter().cycle().skip(1)).take(c.len()).map(|(a, b)| a.distance(*b)).sum::<f64>()).sum();
        if perimeter > 0.0 { 2.0 * area / perimeter } else { 0.0 }
    }

    pub fn path(&self) -> PathData {
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
