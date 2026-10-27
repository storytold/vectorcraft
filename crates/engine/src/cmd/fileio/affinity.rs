//! Affinity documents (`.af`, `.afdesign`, `.afphoto`, `.afpub`): the native layers, curves,
//! shapes, artboards, text, tables, master pages, layer effects and images that
//! `vectorcraft-affinity` reads, mapped to editable VectorCraft art. Whatever the reader could not map is listed in the warnings. A file whose
//! native document can't be read opens as its embedded preview instead, with a warning that says
//! why, and can't be placed.

use std::io::Cursor;
use std::sync::Arc;

use kurbo::{Affine, BezPath, Point, Rect, Vec2};
use serde_json::json;
use vectorcraft_affinity::model::{self as af, Align as AfAlign, Blend, Kind, Pixels};
use vectorcraft_affinity::paint::{self as ap, Cap, GradientKind as AfKind, Join};
use vectorcraft_color::{BlendMode, Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::appearance::{Appearance, AppearanceItem, Dash, Effect, FillLayer, LineCap, LineJoin, StrokeAlign, StrokeLayer};
use vectorcraft_doc::geom::{FillRule, PathData, shapes};
use vectorcraft_doc::text::{CharStyle, FirstBaseline, Justify, TextKind, TextObject, TextRun, VerticalAlign};
use vectorcraft_doc::{Artboard, Document, ImageObject, LayerColor, Node, NodeKind, OpacityMask};

use super::load::{err, raster_doc};
use crate::Result;

mod document;
mod import;
mod layout;
mod master_copies;
mod nodes;
mod paint;
mod text;

pub(super) use import::import;
use master_copies::{CopyCost, MasterPart};

struct Builder {
    doc: Document,
    warnings: Vec<String>,
    /// Points per document pixel.
    k: f64,
    /// Layers made so far (their colours cycle).
    layer: u8,
    /// How deep inside embedded Affinity documents the node being mapped is: their artboards
    /// are only clip groups, never artboards of this document.
    embedded: usize,
    /// Each master spread's art as shown on pages, in the order the masters are first used.
    masters: Vec<(usize, Vec<Arc<Node>>)>,
    /// The document's CMYK profile, until [`Builder::document_cmyk`] makes its table.
    cmyk_profile: Option<Vec<u8>>,
    /// That table, once made (`Some(None)`: there is none).
    cmyk_lut: Option<Option<Arc<vectorcraft_color::cms::CmykLut>>>,
    /// The working CMYK profile's table, once a CMYK pixel layer needs it.
    working_cmyk: Option<Arc<vectorcraft_color::cms::CmykLut>>,
    /// What baking colour and gradient overlays may still take ([`overlay::MAX_WORK`]).
    overlay_work: u64,
    /// What copying master pages onto the pages that show them may still take.
    copy_left: CopyCost,
    /// Each master spread's objects as its pages show them, made the first time one does.
    master_parts: std::collections::HashMap<usize, Arc<[MasterPart]>>,
}
