//! Smart Guides: snapping to anchors, path segments (for dragged anchors and drawn points), object
//! bounds (edges/centres), artboards, with the magenta construction lines and labels Illustrator
//! users expect. Preferences › Smart Guides sets their colour, which of them show ([`GuideStyle`]),
//! how near a target pulls (`ToolContext::snapping_tolerance`) and the angles of the construction
//! guides drawn points land on (`ToolContext::construction_angles`).
//!
//! The drawing tools (Pen, Curvature, Pencil, the shape tools, Type, Artboard) all snap the points
//! they place through one [`DrawSnap`]: their targets are gathered once per document state, sorted,
//! and each pointer move looks up the few near the pointer by bisection.

use std::collections::HashSet;
use std::sync::Arc;

use vectorcraft_doc::hit::{HitOptions, hit_test};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind, OrientedBox, Selection};
use vectorcraft_geom::kurbo::{ParamCurveDeriv, ParamCurveNearest};
use vectorcraft_geom::{Affine, BezPath, Line, ParamCurve, PathSeg, Point, Rect, Shape, SubPath, Vec2, constrain_angle_from};
use vectorcraft_text::{FontDb, layout as layout_text};

use crate::bbox::Handle;
use crate::{GlyphSnap, Overlay, ScreenFrame, ToolContext};

mod snap;
mod targets;

pub(crate) use snap::hold_on_path;
pub use snap::{
    DEFAULT_CONSTRUCTION_ANGLES, DrawSnap, HandleSnap, Leave, PointSnap, construction_angles, distance_guides, snap_draw, snap_pick, square,
};
use targets::Kind;
pub use targets::Targets;
#[cfg(test)]
use targets::{lines_near, object_boxes};

pub const MAGENTA: [u8; 3] = [0xff, 0x3d, 0xfc];

/// How Smart Guides look (Preferences › Smart Guides › Display Options): their colour, and whether
/// the alignment lines and the anchor/path labels show. Snapping is the same either way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GuideStyle {
    pub color: [u8; 3],
    /// Alignment Guides: the lines along the edges and centres the art lines up with.
    pub lines: bool,
    /// Anchor/Path Labels: "anchor", "center", "path", "align"…
    pub labels: bool,
}

impl Default for GuideStyle {
    fn default() -> Self {
        Self { color: MAGENTA, lines: true, labels: true }
    }
}

impl GuideStyle {
    /// The style the preferences in `cx` ask for.
    pub fn of(cx: &ToolContext) -> Self {
        Self { color: cx.smart_guide_color, lines: cx.alignment_guides, labels: cx.anchor_path_labels }
    }
}

/// A ruler guide as a snap target.
struct Ruler {
    vertical: bool,
    pos: f64,
    /// Where it runs along its line: an artboard guide across its artboard (see
    /// [`vectorcraft_doc::Document::guide_span`]), else (None) the whole canvas.
    span: Option<(f64, f64)>,
}

impl Ruler {
    /// The line it pulls `p` into within `tol` of its ends (its position, where it starts, the
    /// kind), as [`Targets::xs`] holds them: none where it doesn't run.
    fn line_at(&self, p: Point, tol: f64) -> Option<(f64, Point, Kind)> {
        let along = if self.vertical { p.y } else { p.x };
        let start = match self.span {
            Some((a, b)) if along < a - tol || along > b + tol => return None,
            Some((a, _)) => a,
            None => 0.0,
        };
        let from = if self.vertical { Point::new(self.pos, start) } else { Point::new(start, self.pos) };
        Some((self.pos, from, Kind::Guide))
    }
}

#[cfg(test)]
mod tests;
