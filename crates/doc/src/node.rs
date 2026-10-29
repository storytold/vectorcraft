//! Document nodes.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_geom::corners::cut_corners;
use vectorcraft_geom::shapes::{self, CornerKind};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Shape};

use crate::appearance::Appearance;
use crate::live::{BlendSpec, EnvelopeKind, GradientMesh, Outliner};
use crate::pattern::RepeatSpec;
use crate::symbols::{SYMBOL_SQUARE, SymbolBoxes};
use crate::text::TextObject;

mod attributes;
mod clip;
mod extras;
mod ids;
mod kind;
mod live_shape;
mod opacity;
mod ops;
mod wire;

pub use attributes::{ForeignAttribute, ImageMap, MAX_SVG_SCRIPTS, ObjectAttributes, SVG_EVENTS};
pub use clip::{ClipPaint, Uniter};
pub use extras::{Extras, ExtrasMut, NodeExtras};
pub use ids::{LAYER_COLORS, LayerColor, NodeId};
pub use kind::{ImageObject, MAX_BUFFERED_VALUES, NodeKind, with_buffered_value_budget};
pub use live_shape::{LiveShape, Scaling};
pub use opacity::{Knockout, OpacityMask, whiten};
use wire::NodeWire;
pub use wire::{MAX_READ_NODES, with_node_budget};

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}

/// One object in the tree. Children are `Arc`s so documents share structure between undo states.
///
/// What most objects have is kept inline; what few have is in [`NodeExtras`], allocated only for
/// the objects that have any of it: every undo step of an edit copies the nodes it changed, so a
/// small node is a cheap step. Files keep the flat field list either way: a node is written field
/// by field from where each lives, and read through a flat `NodeWire`.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(from = "NodeWire")]
pub struct Node {
    pub id: NodeId,
    pub name: Option<String>,
    pub visible: bool,
    pub locked: bool,
    pub opacity: f32,
    pub blend: BlendMode,
    pub isolate: bool,
    /// Knockout Group: whether a container's children knock each other out (see [`Knockout`]).
    pub knockout: Knockout,
    /// Opacity & Mask Define Knockout Shape: inside a knockout group, this object's opacity and
    /// opacity mask scale how much of the objects below it in the group it knocks out.
    pub knockout_shape: bool,
    pub appearance: crate::Shared<Appearance>,
    pub kind: NodeKind,
    /// The angle of the object's own axes, counter-clockwise degrees (0: square to the page): its
    /// bounding box and handles stand at this angle. Transforms turn it with the object (see
    /// [`crate::orient`]); Reset Bounding Box sets it back to 0.
    pub bbox_angle: f64,
    /// The rarely set parts, read with [`Node::extras`] and changed with [`Node::extras_mut`]
    /// (public so that `Node { opacity: 1.0, ..node }` works).
    pub extras: Extras,
}

#[cfg(test)]
mod tests;
