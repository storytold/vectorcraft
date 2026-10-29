//! Object → Flatten Transparency: transparent art becomes opaque art that looks the same.
//!
//! The targets (`ids` or the selection) split into groups of overlapping objects. Groups without
//! transparency stay as they are (their type and strokes are outlined on request). The others are
//! cut into atomic regions: the faces of the planar arrangement of every fill, outlined stroke and
//! clipping path in them ([`po::regions`]). The same paints cover each whole region, so its colour
//! is exact: those paints composited as the renderer composites them ([`Composer`], blend modes by
//! [`composite`]), over white (or over nothing with Preserve Alpha); in CMYK documents their inks
//! composite, plane by plane ([`cmyk_planes`]), and the colour is CMYK. Each region becomes a path
//! filled with that flat colour. Where gradients, patterns, images, opacity masks or raster
//! effects reach there is no single colour: those regions are rendered into one image per group,
//! clipped to them with Clip Complex Regions (else the image is a rectangle that takes in the
//! regions it overlaps). The raster/vector balance rasterizes whole groups that split into more
//! regions than it allows (all of them at 0). For files without transparency (PDF/X-1a, PDF/X-3)
//! a CMYK document's images are opaque CMYK: its colours keep their inks there too.
//!
//! Edit → Transparency Flattener Presets: the built-in presets plus the user's, saved with the
//! preferences ([`crate::Prefs::flattener_presets`]); a `preset` named anywhere options are taken
//! finds both.
//!
//! Window → Flattener Preview: the same plan, worked out without rendering, reports what flattening
//! the document would touch ([`FlattenReport`], `flattener.preview`).

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::blend::{cmyk_planes, composite, planes_cmyk};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::appearance::{AppearanceItem, FillLayer};
use vectorcraft_doc::cmyk::Inks;
use vectorcraft_doc::{Appearance, ColorMode, Document, Effect, ImageBlob, ImageObject, Knockout, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, FillRule, PathData, Rect, shapes};
use vectorcraft_pathops as po;
use vectorcraft_render::effects;

use super::edit::roots_of;
use super::menucmds::{MAX_PIXELS, isolated_doc, unique_key};
use super::pathops::{node_path, outline_strokes, outline_strokes_under, separate, shape_node};
use super::{CommandSpec, EngineError, Result, Session, always, bad, bool_or, cmd, has_doc, str_param, targets, unique_name};

mod analysis;
mod apply;
mod commands;
mod compose;
mod options;
mod presets;
mod report;

pub(crate) use analysis::see_through_images;
pub(crate) use apply::flatten_document;
pub use commands::specs;
pub use options::FlattenOptions;
pub use presets::{FlattenerPreset, PRESET_EXTS, PRESET_FORMAT};
pub use report::{FlattenReport, Highlight, Overprints, report};
