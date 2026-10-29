//! Live Blends (Object → Blend), Envelope Distort (Object → Envelope Distort) and Gradient Mesh
//! (Object → Create Gradient Mesh, Mesh tool edits).
//!
//! The objects are live document nodes (`NodeKind::Blend`, `NodeKind::Envelope`,
//! `NodeKind::Mesh`); evaluation lives in `vectorcraft_doc::live` and the renderer. **Expand**
//! replaces a live object by its evaluated geometry (`vectorcraft_render::expand_live`).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::live::{
    self, BlendDefaults, BlendEase, BlendEaseType, BlendOrientation, BlendSpacing, BlendSpec, EnvelopeKind, EnvelopeMap, EnvelopeOptions,
    GradientMesh, MeshAppearance, PreserveShape, Spine,
};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, Selection};
use vectorcraft_geom::{Affine, Anchor, PathData, Point, Vec2};

use super::edit::selected_roots;
use super::expand::gradient_sampler;
use super::pathops::Beside;
use super::{
    CommandSpec, EngineError, Result, Session, always, bad, check_numbers, check_switches, cmd, color_value, f64_or, f64_req, finite_numbers,
    has_doc, has_multi, has_selection, id_param, ids_param, number_or, ok, str_param, switch_or,
};

mod blend;
mod commands;
mod common;
mod envelope;
mod mesh;

pub(crate) use blend::expand_blends;
pub use commands::specs;
pub(super) use common::fix_ids;
pub(super) use envelope::expand_envelopes_under;
