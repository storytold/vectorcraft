//! Object menu: transforms, arrange, group, lock/hide, compound paths, clipping masks, isolation,
//! align & distribute, object properties.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::corners::{convert_radii_relative, set_corners};
use vectorcraft_doc::{Appearance, Document, Knockout, LiveCorners, LiveShape, Node, NodeId, NodeKind, OrientedBox, Scaling};
use vectorcraft_geom::shapes::CornerKind;
use vectorcraft_geom::{Affine, FillRule, PathData, Point, Rect, Vec2};
use vectorcraft_render::effects;

use super::edit::{duplicate_in, outermost, selected_roots, selected_roots_to_transform};
use super::opacitymask::percent;
use super::*;
use crate::{EngineError, LastTransform};

mod align;
mod arrange;
mod bounds;
mod commands;
mod compound;
mod props;
mod shape;
mod spans;
mod transform;

pub(crate) use align::choice;
pub(crate) use arrange::group_nodes;
pub(crate) use bounds::bounds_rewrite;
pub use commands::specs;
pub(super) use compound::{as_clipping_path, make_clipping_path, release_clip};
pub(crate) use transform::{apply_transform, preview_copies, scaling, transform_node, transform_patterns, transforms_objects};
