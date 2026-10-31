//! Legacy `.ai`, version 7: the PostScript-form Illustrator format, written so that apps that read
//! it get the document's layers, groups, compound paths and clipping groups as editable objects (a
//! PDF-compatible `.ai` without Illustrator's undocumented editing data opens in them as one
//! clipped group per page).
//!
//! The format is the one the published specification documents, and only that: Adobe Illustrator
//! File Format Specification, version 7.0 (23 February 1998), listed by PRONOM at
//! https://www.nationalarchives.gov.uk/PRONOM/fmt/423 (its specification link:
//! http://mercator.elte.hu/~saman/hu/okt/AI7FileFormat.pdf). Section numbers below are its. This
//! writer emits:
//!
//! - Header comments (§2.1): `%%BoundingBox` (the one it requires), `%%CreationDate: (date)
//!   (time)`, custom (spot) colours, `%AI5_FileFormat 3` (version 7.0, Table 2), the artboard as
//!   `%AI5_ArtSize` and an empty `%AI3_TemplateBox` at its centre, which puts the ruler origin at
//!   its bottom-left corner (§2.2), `%AI5_NumLayers`.
//! - Setup (§3): `n Bn`, then each gradient: `%AI5_BeginGradient: (name)`, `(name) type n Bd`,
//!   `[`, its stops (`colorSpec colorStyle midPoint rampPoint %_Bs`), listed from the last for a
//!   linear gradient and listed from the first for a radial one, `BD`, `%AI5_EndGradient`.
//! - Layers (§9): `%AI5_BeginLayer`, `visible preview enabled printing dimmed 0 colorIndex r g b
//!   Lb`, `(name) Ln`, the art, `LB`, `%AI5_EndLayer--`.
//! - Groups `u` … `U`, clipping groups `q` … `Q` with the clipping path painted `h W n` (§5.6),
//!   compound paths `*u` … `*U` (§5.3.5), locked objects `1 A` (§5.1).
//! - Paths (§5.3): `m`, `l`/`L`, `c`/`C` (lower case: the new point is smooth, upper case: a
//!   corner), painted `N n F f S s B b` (lower case closes first); guides `(N) *` before and after
//!   the path (§6); fill rule `XR`; overprint `O`/`R`; stroke `w J j M d`.
//! - Colours (§5.4): `g`/`G` (grey), `k`/`K` (CMYK), `Xa`/`XA` (`r g b`), `x`/`X` (a spot colour
//!   at a tint). Gradient fills (§3.5.3): the path, `Bb`, a radial's highlight `x y angle fraction
//!   Bh` (`0 0 0 0 Bh` at the center, §3.5.5, §3.5.6), `1 (name) x y angle length 1 0 0 1 0 0 Bg`,
//!   `f` (or `F`), and `0 BB`, or `2 BB` (`1 BB` open) to stroke it too.
//! - Images (§8.1, §8.4): `%AI5_File:`, `%AI5_BeginRaster`,
//!   `[ a b c d tx ty ] 0 0 w h w h 8 3 0 0 0 0 XI`, the RGB samples in hex on comment lines of 60
//!   digits, `%AI5_EndRaster`, then a line `F` that §8.4's examples do not show.
//! - Type (§15): point type, area type and type on a path as text objects, `type To`, `a b c d tx
//!   ty startPt Tp` (the text matrix; the frame or path in art space between it and `TP`), each
//!   run's paint style, `Tr` (fill, stroke, both or neither), `/_font size ascent descent Tf`,
//!   `Ts Tz Tt TA Tl Ta Ti` and its characters `(…) Tx` (`\r` ends a paragraph), `TO`. Only the
//!   revisable form is written: readers lay the type out again (§15.1). Fonts are named by their
//!   PostScript names in `%%DocumentFonts` and `%%IncludeFont` (§3.1) and re-encoded to
//!   Windows-1252 (`TE`, then `%AI3_BeginEncoding: _name name`, `TZ`, §3.3), the 8-bit encoding
//!   strings are in.
//!
//! The format has one artboard, no transparency, sublayers, object names, hidden objects (only
//! layers hide), live effects, symbols or appearances with several fills and strokes. Callers write
//! one file per artboard and expand appearances first; what is left is written as the format can
//! hold it, each loss a warning: transparent objects are written opaque, sublayers become groups,
//! live objects and symbols their art, strokes a plain line can't draw their outlines, pattern
//! fills their tiles clipped to the shape, freeform gradients their average colour, elliptical
//! radial gradients circular (the gradient matrix is relative to a canvas the specification
//! doesn't size), transparent image pixels over white. Hidden objects go to a hidden layer of
//! their own above their layer, are left out or show, as [`HiddenObjects`] says (in a hidden
//! layer they stay where they are). Type that 8-bit text can't hold (characters outside
//! Windows-1252, vertical type, ruby, inline graphics, gradient or pattern fills) is written as its
//! outlines, and all type with [`LegacyAiOptions::outline_text`]. Layer, spot color and gradient
//! names and the title are written in Windows-1252 too, and a character outside it is written as
//! `?`. Two spot colors or two gradients whose names become the same that way are numbered (`? 2`).
//!
//! The prolog is a procedure set of the writer's own ([`PROLOG`]) that gives the operators their
//! meaning in PostScript, so the file also prints and previews as a PostScript program (its
//! gradients flat, without its type, which apps that read the format draw).

use std::fmt::Write;
use std::sync::Arc;

use vectorcraft_color::cms::Cms;
use vectorcraft_color::{BlendMode, Color, Gradient, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{
    AppearanceItem, AreaOptions, CharPosition, CharStyle, Document, FillLayer, ImageBlob, Justify, LAYER_COLORS, LayerColor, LineCap, LineJoin, Node,
    NodeKind, ParaDirection, ParaStyle, PathEffect, StrokeLayer, TextKind, TextObject,
};
use vectorcraft_effects::stroke;
use vectorcraft_geom::{Affine, AnchorKind, BezPath, FillRule, PathData, Point, Rect, Shape, SubPath, Vec2};

use crate::{latin, ps};

mod encoding;
mod prolog;
mod writer;

#[cfg(test)]
use encoding::writable;
use encoding::{gradient_def, latin_string};
pub(crate) use prolog::PROCSET;
use prolog::{PROLOG, PS_HEADER, fonts};
use writer::Writer;

/// How deep symbols and live objects may nest in one another (deeper art is left out). Groups and
/// layers need no limit of their own: a document is never deeper than
/// [`vectorcraft_doc::MAX_DEPTH`].
const MAX_NEST: u32 = 8;
/// Largest image written (pixels).
const MAX_IMAGE_PIXELS: u64 = 1 << 26;
/// Hex digits on a line of image data (§8.4).
const HEX_LINE: usize = 60;

pub const SUBLAYERS: &str = "sublayers are written as groups in their top-level layer (legacy .ai files have no sublayers)";
pub const TYPE: &str = "type a legacy .ai can't hold as text (characters outside Windows Latin-1, vertical type, ruby, inline graphics, gradient or pattern fills) is written as outlines";
pub const TYPE_ATTRIBUTES: &str = "type attributes legacy .ai files don't have (underline, strikethrough, small caps, superscript and subscript, character rotation, OpenType features, Variable Font settings, area type options, type on a path effects, space after paragraphs, tabs, bulleted and numbered lists) are left out";
pub const HIDDEN: &str = "hidden objects are left out (hidden layers are kept, hidden)";
pub const HIDDEN_LAYER: &str = "hidden objects are moved to a hidden layer above their own (legacy .ai files can hide only layers)";
pub const HIDDEN_SHOWN: &str = "hidden objects are written visible (legacy .ai files can hide only layers)";
pub const TRANSPARENCY: &str = "transparency (opacity, blending modes, opacity masks) is written opaque: legacy .ai files have none";
pub const LAYER_NAME: &str = "characters outside Windows Latin-1 in layer names are written as ? (legacy .ai files can't hold them)";
pub const TITLE_AND_COLOR_NAMES: &str =
    "characters outside Windows Latin-1 in the title and in spot color and gradient names are written as ? (legacy .ai files can't hold them)";
pub const NAMES: &str = "object names are left out (legacy .ai files name only layers)";
const PATTERNS: &str = "pattern fills are written as their tiles, clipped to the shape";
const FREEFORM: &str = "freeform gradients are written as their average colour";
const STROKES: &str =
    "strokes a plain line can't draw (inside or outside the path, variable width, gradients, brushes, arrowheads) are written as filled outlines";
const IMAGE_ALPHA: &str = "transparent image pixels are written over white";
const LIVE: &str = "symbols, blends, envelopes, meshes, repeats, placed documents and intertwines are written as their art";
/// The warning for an image left out because it can't be decoded: the note raster exports give
/// (`vectorcraft-engine`'s `rasterfx::undecoded_warning`).
const UNDECODED: &str = "an image couldn't be decoded and is missing from the picture";
const LARGE_IMAGE: &str = "an image that is empty or too large for legacy .ai was left out";
const ELLIPTICAL: &str = "elliptical radial gradients are written circular";

/// What to write in the header, and how to write what the format can't hold.
#[derive(Clone, Debug, Default)]
pub struct LegacyAiOptions {
    /// `%%Title`.
    pub title: String,
    /// `%%CreationDate`, Unix seconds.
    pub created: Option<i64>,
    /// Where hidden objects go (the format hides only layers).
    pub hidden: HiddenObjects,
    /// Type as its glyphs' outlines, not text objects.
    pub outline_text: bool,
}

/// What a legacy `.ai` does with hidden objects, which the format can't mark hidden (only layers).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HiddenObjects {
    /// Each layer's hidden objects in a hidden layer of their own, just above it (they leave
    /// their groups).
    #[default]
    Layer,
    /// Left out.
    Omit,
    /// Written where they are, visible.
    Visible,
}

/// The file and what it couldn't hold.
#[derive(Clone, Debug, Default)]
pub struct LegacyAiOutput {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
}

/// Write `doc`'s first artboard and every layer (all its art, on the artboard or not) as a
/// legacy `.ai` file of version 7. Art space is y up, its origin at the artboard's left edge and
/// just under its bottom edge: the artboard's top is on a whole point, where readers that place
/// the artboard by rounding the template box out to whole points put it too.
pub fn export_legacy_ai(doc: &Document, o: &LegacyAiOptions) -> Result<LegacyAiOutput, String> {
    let board = doc.artboards.first().ok_or("the document has no artboard")?.rect;
    if !(board.is_finite() && board.width() > 0.0 && board.height() > 0.0) {
        return Err("the document has no artboard".into());
    }
    // The document is y down; art space y up from the artboard's left edge, its top on a whole point.
    let (width, height) = (board.width(), board.height());
    let top = if (height - height.round()).abs() < 1e-6 { height.round() } else { height.ceil() };
    let below = top - height;
    let to_art = Affine::new([1.0, 0.0, 0.0, -1.0, -board.x0, board.y1 + below]);
    let mut w = Writer::new(doc, to_art, o);
    let mut layers = 0usize;
    for layer in &doc.layers {
        if w.layer(layer) {
            layers += 1;
        }
        if w.hidden_layer(layer) {
            layers += 1;
        }
    }
    let art = doc
        .layers
        .iter()
        .filter(|l| l.visible)
        .filter_map(|l| l.visual_bounds())
        .map(|b| to_art.transform_rect_bbox(b))
        .filter(|b| b.is_finite())
        .reduce(|a, b| a.union(b))
        .unwrap_or(Rect::new(0.0, below, width, top));
    let title = w.latin_name(&o.title, TITLE_AND_COLOR_NAMES);
    let mut s = String::new();
    s.push_str(PS_HEADER);
    s.push('\n');
    let _ = writeln!(s, "%%Creator: VectorCraft {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(s, "%%Title: {}", latin_string(&title));
    if let Some(t) = o.created {
        let [y, mo, d, h, mi, _] = vectorcraft_doc::metadata::civil(t);
        let _ = writeln!(s, "%%CreationDate: ({mo}/{d}/{y:04}) ({h:02}:{mi:02})");
    }
    let _ = writeln!(s, "%%BoundingBox: {} {} {} {}", art.x0.floor() as i64, art.y0.floor() as i64, art.x1.ceil() as i64, art.y1.ceil() as i64);
    s.push_str("%%HiResBoundingBox: ");
    ps::push_nums(&mut s, &[art.x0, art.y0, art.x1, art.y1]);
    s.pop();
    s.push('\n');
    s.push_str("%%DocumentProcessColors: Cyan Magenta Yellow Black\n");
    for (i, (name, _)) in w.spots.iter().enumerate() {
        let _ = writeln!(s, "{} {}", if i == 0 { "%%DocumentCustomColors:" } else { "%%+" }, latin_string(name));
    }
    for (i, (name, cmyk)) in w.spots.iter().enumerate() {
        s.push_str(if i == 0 { "%%CMYKCustomColor: " } else { "%%+ " });
        ps::push_nums(&mut s, &cmyk.map(f64::from));
        let _ = writeln!(s, "{}", latin_string(name));
    }
    for (i, (name, _)) in w.fonts.iter().enumerate() {
        let _ = writeln!(s, "{} {name}", if i == 0 { "%%DocumentFonts:" } else { "%%+" });
    }
    s.push_str("%AI5_FileFormat 3\n%AI3_ColorUsage: Color\n");
    // An empty template box at the artboard's centre sets the ruler origin (§2.2).
    let (cx, cy) = (ps::num(width / 2.0), ps::num(below + height / 2.0));
    let _ = writeln!(s, "%AI3_TemplateBox: {cx} {cy} {cx} {cy}");
    let _ = writeln!(s, "%AI3_TileBox: 0 {} {} {}", ps::num(below), ps::num(width), ps::num(top));
    s.push_str("%AI3_DocumentPreview: None\n");
    let _ = writeln!(s, "%AI5_ArtSize: {} {}", ps::num(width), ps::num(height));
    s.push_str("%AI5_RulerUnits: 2\n");
    let _ = writeln!(s, "%AI5_NumLayers: {layers}");
    let _ = writeln!(s, "%%DocumentSuppliedResources: procset {PROCSET}");
    s.push_str("%%EndComments\n%%BeginProlog\n");
    s.push_str(PROLOG);
    s.push_str("%%EndProlog\n%%BeginSetup\n");
    s.push_str("VCLegacy begin\n");
    fonts(&mut s, &w.fonts);
    if !w.gradients.is_empty() {
        let _ = writeln!(s, "{} Bn", w.gradients.len());
        for (name, g) in &w.gradients {
            gradient_def(&mut s, &w, name, g);
        }
    }
    s.push_str("%%EndSetup\n");
    s.push_str(&w.body);
    // The page trailer as the specification's script has it (§3); the document trailer takes the
    // prolog's dictionary off the dictionary stack (its procedure set termination, §3).
    s.push_str("%%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\nend\n%%EOF\n");
    Ok(LegacyAiOutput { bytes: s.into_bytes(), warnings: w.warnings })
}

#[cfg(test)]
mod tests;
