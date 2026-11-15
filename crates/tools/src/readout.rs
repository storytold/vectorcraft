//! What the tool in use gives the Info panel to show ([`crate::Tool::readout`]).

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Vec2};

/// The Info panel's values for the tool in use. Lengths and points are in document units, and
/// angles are in degrees as the matching command takes them: counter-clockwise for
/// `object.rotate`'s `angle` and `object.reflect`'s `axis`, and `object.shear`'s `angle`.
#[derive(Clone, Debug, PartialEq)]
pub enum Readout {
    /// A drag that moves the selection or draws a gradient vector: how far it has gone in x and y.
    Drag { dx: f64, dy: f64 },
    /// A Scale tool drag: the horizontal and vertical scale (1 leaves the size as it is).
    Scale { sx: f64, sy: f64 },
    /// A Rotate tool drag: where the center of the selection's bounding box is, and the angle
    /// turned.
    Rotate { center: Point, angle: f64 },
    /// A Reflect tool drag: where the center of the selection's bounding box is, and the angle of
    /// the axis.
    Reflect { center: Point, angle: f64 },
    /// A Shear tool drag: where the center of the selection's bounding box is, the angle of the
    /// shear axis (0 horizontal, 90 vertical) and the shear angle.
    Shear { center: Point, axis: f64, angle: f64 },
    /// The Paintbrush: the brush it paints with, if any.
    Brush { name: Option<String> },
}

impl Readout {
    /// The readout of a drag by `d`.
    pub fn drag(d: Vec2) -> Self {
        Readout::Drag { dx: d.x, dy: d.y }
    }

    /// The length of a [`Readout::Drag`] and its angle, as the Measure tool gives them.
    pub fn distance_angle(&self) -> Option<(f64, f64)> {
        match *self {
            Readout::Drag { dx, dy } => {
                let (dist, angle, ..) = crate::xform::measure(Point::ORIGIN, Point::new(dx, dy));
                Some((dist, angle))
            }
            _ => None,
        }
    }

    /// The readout as `info.readout` reports it.
    pub fn to_json(&self) -> Value {
        let at = |p: Point| json!([p.x, p.y]);
        match self {
            Readout::Drag { dx, dy } => {
                let (distance, angle) = self.distance_angle().unwrap_or_default();
                json!({ "kind": "drag", "dx": dx, "dy": dy, "distance": distance, "angle": angle })
            }
            Readout::Scale { sx, sy } => json!({ "kind": "scale", "width": sx * 100.0, "height": sy * 100.0 }),
            Readout::Rotate { center, angle } => json!({ "kind": "rotate", "center": at(*center), "angle": angle }),
            Readout::Reflect { center, angle } => json!({ "kind": "reflect", "center": at(*center), "angle": angle }),
            Readout::Shear { center, axis, angle } => json!({ "kind": "shear", "center": at(*center), "axis": axis, "angle": angle }),
            Readout::Brush { name } => json!({ "kind": "brush", "brush": name }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_drag_gives_its_length_and_its_angle_up_the_page() {
        let r = Readout::drag(Vec2::new(30.0, -40.0));
        let (d, a) = r.distance_angle().unwrap();
        assert!((d - 50.0).abs() < 1e-9 && (a - 53.13010235415598).abs() < 1e-9, "{d} {a}");
        assert_eq!(r.to_json()["dx"], 30.0);
        assert_eq!(Readout::Scale { sx: 1.5, sy: 0.5 }.to_json(), json!({"kind": "scale", "width": 150.0, "height": 50.0}));
        assert_eq!(Readout::Scale { sx: 1.0, sy: 1.0 }.distance_angle(), None);
        assert_eq!(Readout::Brush { name: None }.to_json(), json!({"kind": "brush", "brush": null}));
    }
}
