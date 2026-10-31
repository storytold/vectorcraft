//! The graphics state and the operators that draw: coordinates, paths, colours, painting,
//! clipping and shadings. Painted paths become document objects as they are drawn ([`Out`]).

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use kurbo::{PathEl, Shape};
use vectorcraft_color::swatch::REGISTRATION;
use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::clipnest::{Clip, Drawn, MAX_NEST, deep_clips};
use vectorcraft_doc::pattern::PatternDef;
use vectorcraft_doc::{Appearance, AppearanceItem, Dash, Document, FillLayer, LineCap, LineJoin, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, Vec2};

use super::interp::{Interp, MAX_GSAVE, matrix_obj};
use super::obj::{DictRef, Key, Obj, Op, PsError, Res, Shared, clamp_between, ps_err};

mod ops;
mod paint;
mod path;
mod state;

pub(crate) use paint::collapse;
pub(crate) use state::{GState, Out, Space, process};

/// Most objects one file makes.
const MAX_NODES: usize = 1 << 20;
/// Objects reaching further than this from the page (points) come from damaged coordinates and
/// are left out.
const LIMIT: f64 = 1e6;
/// Width of a zero-width line (the thinnest a device draws), points.
const HAIRLINE: f64 = 0.25;
/// Most inks a `DeviceN` space has (PostScript's own limit).
const MAX_INKS: usize = 32;
/// Most dashes `strokepath` makes of a path (more and it outlines the path undashed).
const MAX_DASHES: f64 = 1e5;
/// Most entries a dash pattern has: the most elements an array has in a typical PostScript
/// implementation (PLRM 3rd ed., Table B.1).
const MAX_DASH_ENTRIES: usize = 65_535;
/// Most entries an indexed colour space's table has.
const MAX_HIVAL: usize = 4095;
/// Most elements the current path may have.
const MAX_PATH: usize = 1 << 20;
/// Most path elements drawn in all (each object's clips and groups counting as elements too).
const MAX_DRAWN: usize = 1 << 24;
/// Samples taken of a shading function that isn't a plain interpolation.
const SHADING_SAMPLES: usize = 32;
/// Most bounds of a stitched linear function that become a gradient's stops (two where the pieces
/// don't meet): past it, the function is sampled like any other (each stop evaluates the whole
/// function).
const MAX_STITCH_STOPS: usize = 1024;
/// Most functions one evaluation of a shading's function visits: the function itself, each entry
/// of an array of functions and, in each stitching function, the piece that applies to the input.
/// For every shading type, the `Function` entry is one function dictionary or an array of one for
/// each color component (PLRM 3rd ed., Tables 4.12 to 4.17; a type 7 shading has the entries of
/// type 6). The reader allows at most [`MAX_INKS`] components, and the depth limit allows each
/// entry a chain of at most 8 functions.
const MAX_FUNCTION_VISITS: usize = 1 + MAX_INKS * 8;
/// Work one function costs against the operation budget each time it is evaluated: about as much
/// as 8 operations (measured).
const FUNCTION_OPS: u64 = 8;

pub(super) const FAR_AWAY: &str = "objects far outside the page were left out";
pub(super) const TOO_MUCH: &str = "the file draws more than VectorCraft reads: the rest was left out";
const NESTED_PATTERNS: &str = "patterns nested too deeply are filled with mid-grey";
/// Deepest patterns and glyph procedures drawn inside each other's art.
pub(crate) const MAX_APART: u32 = 8;
const UNKNOWN_FUNCTIONS: &str = "shadings whose functions are of an unknown type are filled with their middle colour";
const UNKNOWN_SHADINGS: &str = "shadings of an unknown type were left out";
