//! Perspective Grid attachments (Object › Perspective).
//!
//! The grid itself is document data (`Document.unknown["perspectiveGrid"]`, modelled by the tools
//! crate). Which plane an object lies on is kept on the object, so copies, duplicates and pastes
//! stay attached and deleted objects leave nothing stale behind.

use serde::{Deserialize, Serialize};
use vectorcraft_geom::{Homography, Rect};

use crate::{Node, NodeKind};

/// The plane an object in perspective lies on.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PerspectiveAttachment {
    /// `left`, `right` or `ground`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub plane: String,
    /// Where along the plane's normal (points) the parallel plane the object lies on is: 0 is the
    /// grid plane in its original place.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub depth: f64,
    /// Type and symbol instances: the projective map (row-major 3 × 3) that draws their flat art
    /// in perspective. They stay type and symbol instances; the renderer and exporters project
    /// their outlines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projection: Option<[f64; 9]>,
    /// Object › Perspective › Edit Text in progress: the type is drawn flat to be edited (not
    /// saved).
    #[serde(skip)]
    pub editing: bool,
}

impl PerspectiveAttachment {
    pub fn new(plane: &str, depth: f64) -> Self {
        Self { plane: plane.to_string(), depth: if depth.is_finite() { depth } else { 0.0 }, ..Self::default() }
    }

    /// The projection, when it is a valid one.
    pub fn homography(&self) -> Option<Homography> {
        self.projection.and_then(Homography::from_array)
    }
}

impl Node {
    /// The projective map type or a symbol instance in perspective is drawn through (not while
    /// Edit Text shows it flat).
    pub fn projection(&self) -> Option<Homography> {
        let a = self.extras().perspective.as_deref().filter(|a| !a.editing)?;
        matches!(self.kind, NodeKind::Text(_) | NodeKind::SymbolInstance { .. }).then(|| a.homography()).flatten()
    }

    /// `b` (flat bounds) as drawn: the box of its projection for type and symbols in perspective.
    pub(crate) fn projected(&self, b: Option<Rect>) -> Option<Rect> {
        match (self.projection(), b) {
            (Some(h), Some(b)) => h.map_rect_bbox(b).or(Some(b)),
            (_, b) => b,
        }
    }

    /// Keep a projection drawing the same picture moved by `a` (the flat art moves by `a` too). An
    /// invertible `a` that the projection cannot follow removes it, and the art is drawn flat.
    pub(crate) fn transform_projection(&mut self, a: vectorcraft_geom::Affine) {
        if let Some(p) = self.extras_if_any().and_then(|e| e.perspective.as_deref_mut())
            && let Some(h) = p.homography()
            && a.determinant().abs() >= 1e-15
        {
            p.projection = h.conjugated(a).map(|h| h.to_array());
        }
    }
}
