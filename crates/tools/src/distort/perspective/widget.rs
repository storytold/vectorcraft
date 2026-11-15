//! The Plane Switching Widget: a small cube in a corner of the document window whose faces pick
//! the active plane (left, horizontal, right) and whose dot picks none. It stays put on screen
//! when the view pans or zooms ([`ScreenFrame`]); without a window (headless) it sits in the first
//! artboard's corner. Perspective Grid Options (double-click the tool) hide it or move it to
//! another corner ([`WidgetOptions`], kept with the preferences).

use serde::{Deserialize, Serialize};
use vectorcraft_doc::Document;
use vectorcraft_geom::{Point, Vec2};

use super::{PerspectiveGrid, Plane, WidgetGeom, in_quad};
use crate::{ScreenFrame, ToolContext};

/// The widget's size and its margin from the window's edges, in screen pixels.
const SIZE: f64 = 40.0;
const MARGIN: f64 = 10.0;

/// Where the widget sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum WidgetCorner {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl WidgetCorner {
    pub const ALL: [Self; 4] = [Self::TopLeft, Self::TopRight, Self::BottomLeft, Self::BottomRight];
    pub fn id(self) -> &'static str {
        match self {
            Self::TopLeft => "topLeft",
            Self::TopRight => "topRight",
            Self::BottomLeft => "bottomLeft",
            Self::BottomRight => "bottomRight",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::TopLeft => "Top Left",
            Self::TopRight => "Top Right",
            Self::BottomLeft => "Bottom Left",
            Self::BottomRight => "Bottom Right",
        }
    }
    /// By id or label, any case.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id().eq_ignore_ascii_case(s.trim()) || c.label().eq_ignore_ascii_case(s.trim()))
    }
}

/// Perspective Grid Options: whether the Plane Switching Widget shows, and where, and what
/// Automatic Plane Positioning takes a point from ([`super::infer`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WidgetOptions {
    pub show: bool,
    pub position: WidgetCorner,
    /// Automatic Plane Positioning › Anchor Point of Perspective Artwork.
    pub anchor_point: bool,
    /// Automatic Plane Positioning › Intersection of Gridlines.
    pub gridline_intersection: bool,
}

impl Default for WidgetOptions {
    fn default() -> Self {
        Self { show: true, position: WidgetCorner::TopLeft, anchor_point: true, gridline_intersection: true }
    }
}

impl WidgetOptions {
    /// What Automatic Plane Positioning takes a point from.
    pub fn targets(self) -> PlaneTargets {
        PlaneTargets { anchors: self.anchor_point, gridlines: self.gridline_intersection }
    }
}

/// What Automatic Plane Positioning takes a point from: the anchor points of art in perspective,
/// the gridline intersections of the planes that show.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlaneTargets {
    pub anchors: bool,
    pub gridlines: bool,
}

impl PlaneTargets {
    pub const ALL: Self = Self { anchors: true, gridlines: true };
    pub fn any(self) -> bool {
        self.anchors || self.gridlines
    }
}

/// Where the widget is: the window it stays put in (none headless) and its corner.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WidgetPlace<'a> {
    pub screen: Option<&'a ScreenFrame>,
    pub corner: WidgetCorner,
}

impl PerspectiveGrid {
    /// The widget's geometry (`tol` = document units per screen pixel) at `place`.
    pub fn widget_at(&self, doc: &Document, tol: f64, place: WidgetPlace) -> WidgetGeom {
        let right = matches!(place.corner, WidgetCorner::TopRight | WidgetCorner::BottomRight);
        let bottom = matches!(place.corner, WidgetCorner::BottomLeft | WidgetCorner::BottomRight);
        let inset = MARGIN + SIZE / 2.0;
        // Headless, the first artboard stands in for the window.
        let frame = place.screen.copied().unwrap_or_else(|| {
            let ab = super::define::first_artboard(doc);
            let tol = tol.max(1e-9);
            ScreenFrame {
                origin: Point::new(ab.x0, ab.y0),
                right: Vec2::new(tol, 0.0),
                down: Vec2::new(0.0, tol),
                size: (ab.width() / tol, ab.height() / tol),
            }
        });
        let (w, h) = frame.size;
        let c = Vec2::new(if right { w - inset } else { inset }, if bottom { h - inset } else { inset });
        let p = |dx: f64, dy: f64| frame.at(c.x + dx * SIZE, c.y + dy * SIZE);
        let faces = vec![
            (Plane::Left, [p(-0.5, -0.25), p(0.0, 0.0), p(0.0, 0.5), p(-0.5, 0.25)]),
            (Plane::Right, [p(0.0, 0.0), p(0.5, -0.25), p(0.5, 0.25), p(0.0, 0.5)]),
            (Plane::Ground, [p(0.0, -0.5), p(0.5, -0.25), p(0.0, 0.0), p(-0.5, -0.25)]),
        ];
        // The dot sits off the cube's corner toward the window's inside.
        let (sx, sy) = (if right { -0.62 } else { 0.62 }, if bottom { -0.62 } else { 0.62 });
        (faces, (p(sx, sy), 0.12 * SIZE * tol))
    }

    /// Which widget control is at `p` (see [`Self::widget_at`]).
    pub fn widget_hit_at(&self, doc: &Document, tol: f64, place: WidgetPlace, p: Point) -> Option<Plane> {
        let (faces, (c, r)) = self.widget_at(doc, tol, place);
        if p.distance(c) <= r * 1.5 {
            return Some(Plane::None);
        }
        faces.into_iter().find(|(_, q)| in_quad(q, p)).map(|(pl, _)| pl)
    }

    /// Which widget control the tool context's pointer `p` is on (none while the widget is hidden).
    pub fn widget_hit_cx(&self, cx: &ToolContext, p: Point) -> Option<Plane> {
        let corner = cx.plane_widget?;
        self.widget_hit_at(cx.doc, cx.tol(1.0), WidgetPlace { screen: cx.screen.as_ref(), corner }, p)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_geom::Rect;

    fn frame(origin: Point, zoom: f64) -> ScreenFrame {
        ScreenFrame { origin, right: Vec2::new(1.0 / zoom, 0.0), down: Vec2::new(0.0, 1.0 / zoom), size: (600.0, 400.0) }
    }

    #[test]
    fn the_widget_stays_put_on_screen() {
        let d = Document::new(800.0, 600.0);
        let g = PerspectiveGrid::normal(2, Rect::new(0.0, 0.0, 800.0, 600.0));
        for corner in WidgetCorner::ALL {
            // Panned and zoomed, the widget is at the same screen pixels.
            let (a, b) = (frame(Point::new(0.0, 0.0), 1.0), frame(Point::new(250.0, -40.0), 4.0));
            let (fa, _) = g.widget_at(&d, 1.0, WidgetPlace { screen: Some(&a), corner });
            let (fb, _) = g.widget_at(&d, 0.25, WidgetPlace { screen: Some(&b), corner });
            for ((_, qa), (_, qb)) in fa.iter().zip(&fb) {
                for (pa, pb) in qa.iter().zip(qb) {
                    let px = (*pb - b.origin) * 4.0;
                    assert!((px - pa.to_vec2()).hypot() < 1e-9, "{corner:?}");
                }
            }
            // Inside the window, near the chosen corner.
            let x = fa[0].1[0].x;
            assert!(if matches!(corner, WidgetCorner::TopRight | WidgetCorner::BottomRight) { x > 500.0 } else { x < 100.0 }, "{corner:?}");
            let centre = |q: &[Point; 4]| Point::new(q.iter().map(|p| p.x).sum::<f64>() / 4.0, q.iter().map(|p| p.y).sum::<f64>() / 4.0);
            assert_eq!(g.widget_hit_at(&d, 0.25, WidgetPlace { screen: Some(&b), corner }, centre(&fb[1].1)), Some(Plane::Right));
        }
        // Headless, the top-left one is in the first artboard's corner, as before.
        let (faces, _) = g.widget(&d, 1.0);
        assert_eq!(faces[0].1[0], Point::new(10.0, 20.0));
        assert_eq!(WidgetCorner::parse("Bottom Left"), Some(WidgetCorner::BottomLeft));
        assert_eq!(WidgetCorner::parse("middle"), None);
    }
}
