//! PDF → Document (hayro-interpret device that builds a VectorCraft node tree).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use craft_pdf::interpret::font::Glyph;
use craft_pdf::interpret::hayro_cmap::BfString;
use craft_pdf::interpret::pattern::{Pattern, TilingPattern};
use craft_pdf::interpret::{
    CacheKey, ClipPath, Context, Device, GlyphDrawMode, Image, ImageData, InterpreterCache, InterpreterSettings, InterpreterWarning, LumaData,
    MaskType, PathDrawMode, SoftMask, StrokeProps,
};
use craft_pdf::syntax::object::Name;
use craft_pdf::syntax::page::Page;
use kurbo::{Affine, BezPath, PathEl, Rect, Shape};
use vectorcraft_color::{BlendMode, Color, Paint, Swatch};
use vectorcraft_doc::{
    Appearance, AppearanceItem, Artboard, ColorMode, Dash, Document, FillLayer, ImageBlob, ImageObject, Knockout, LayerColor, LineCap, LineJoin,
    Node, NodeId, NodeKind, OpacityMask, PatternDef, StrokeLayer,
};
use vectorcraft_geom::{FillRule, PathData};

use crate::import_color::{Colors, Native};
use crate::import_mask::{MaskSpec, contains, is_rectangle, luminance, mask_spec, white_cover};
use crate::import_scan::{FontNames, MAX_NESTING, Ocgs, Scan, TextOverprint, Walked, all_on, hides_forms, scan_page, tag_key};
use crate::import_shading::{
    FunctionBudget, FunctionPaint, clipped, extend_clip, fold_stop_opacity, function_paint, gradient_shape, mesh_shading, shading_gradient,
};
use crate::import_text::{Families, LineFacts, Look, Placement, TextLine, Upright};
use crate::{CropTo, ImportOptions, ImportReport, PdfError, TextAs};

mod builder;
mod cid;
mod device;
mod entry;
mod state;

#[cfg(test)]
pub(crate) use cid::{CidText, font_chars, identify_glyph, unify_radical};
pub use entry::{
    INVISIBLE_TEXT_NOTE, OFF_ARTBOARD_NOTE, OUTLINED_TEXT_NOTE, PAST_PAGE_NOTE, UNMATCHED_GLYPHS_NOTE, UNNAMED_CHARACTERS_NOTE, holds_outlines,
    import, import_with_report, is_glyph_outlines, is_outlines_note,
};
#[cfg(test)]
pub(crate) use entry::{import_with_reader_options, read};
pub(crate) use state::{closed_subpaths, round3};
