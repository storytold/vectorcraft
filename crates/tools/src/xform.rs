//! Transform and utility tools (rotate/reflect/scale/shear, free transform, eyedropper, gradient,
//! artboard, magic wand, lasso, measure).
//!
//! Like every tool they only emit actions: transforms preview `object.transform` / `object.distort`
//! on the interaction snapshot, the utility tools execute selection / paint / artboard commands.

mod artboard;
mod eyedropper;
mod free;
mod freeform;
mod gradient;
mod measure;
mod transform;
mod wand;

use vectorcraft_doc::{Document, NodeId, NodeKind};
use vectorcraft_geom::{BezPath, Point, Rect, Shape};

use crate::{Overlay, Tool};

pub use artboard::ArtboardTool;
pub use eyedropper::EyedropperTool;
pub use free::{DistortMode, FreeTransformTool};
pub use gradient::GradientTool;
pub use measure::{MeasureTool, measure};
pub use transform::{TransformKind, TransformTool};
pub use wand::{LassoTool, MagicWandTool};

/// Create a transform/utility tool by id (None = not one of ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    Some(match id {
        "rotate" => Box::new(TransformTool::new(TransformKind::Rotate)),
        "reflect" => Box::new(TransformTool::new(TransformKind::Reflect)),
        "scale" => Box::new(TransformTool::new(TransformKind::Scale)),
        "shear" => Box::new(TransformTool::new(TransformKind::Shear)),
        "freeTransform" => Box::new(FreeTransformTool::default()),
        "eyedropper" => Box::new(EyedropperTool::default()),
        "gradient" => Box::new(GradientTool::default()),
        "artboard" => Box::new(ArtboardTool::default()),
        "magicWand" => Box::new(MagicWandTool),
        "lasso" => Box::new(LassoTool::default()),
        "measure" => Box::new(MeasureTool::default()),
        _ => return None,
    })
}

/// Cyan used for the reference point target and tool annotations.
pub const CYAN: [u8; 3] = [0x00, 0xa8, 0xff];
/// Illustrator's selection blue for bounding boxes.
pub const BLUE: [u8; 3] = [0x4a, 0x7c, 0xff];

/// The reference-point target (circle + crosshair) drawn at `p`, `r` in document units.
pub(crate) fn target_overlays(p: Point, r: f64) -> Vec<Overlay> {
    let circle = vectorcraft_geom::kurbo::Circle::new(p, r * 0.6).to_path(0.05);
    vec![
        Overlay::Path { path: circle, color: CYAN, width: 1.0, dashed: false },
        Overlay::Line { a: Point::new(p.x - r, p.y), b: Point::new(p.x + r, p.y), color: CYAN, dashed: false },
        Overlay::Line { a: Point::new(p.x, p.y - r), b: Point::new(p.x, p.y + r), color: CYAN, dashed: false },
    ]
}

/// Closed polygon path through `pts`.
pub(crate) fn polygon(pts: &[Point], closed: bool) -> BezPath {
    let mut bp = BezPath::new();
    for (i, p) in pts.iter().enumerate() {
        if i == 0 {
            bp.move_to(*p);
        } else {
            bp.line_to(*p);
        }
    }
    if closed && pts.len() > 2 {
        bp.close_path();
    }
    bp
}

pub(crate) fn rect_corners(r: Rect) -> [Point; 4] {
    [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]
}

/// The node that owns the appearance for a hit leaf (a compound path's child → the compound).
pub fn paint_owner(doc: &Document, leaf: NodeId) -> NodeId {
    match doc.parent_of(leaf) {
        Some(p) if doc.node(p).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. })) => p,
        _ => leaf,
    }
}
