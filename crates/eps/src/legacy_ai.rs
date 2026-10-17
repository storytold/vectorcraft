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
//!   `[`, its stops last first (`colorSpec colorStyle midPoint rampPoint %_Bs`), `BD`,
//!   `%AI5_EndGradient`.
//! - Layers (§9): `%AI5_BeginLayer`, `visible preview enabled printing dimmed 0 colorIndex r g b
//!   Lb`, `(name) Ln`, the art, `LB`, `%AI5_EndLayer--`.
//! - Groups `u` … `U`, clipping groups `q` … `Q` with the clipping path painted `h W n` (§5.6),
//!   compound paths `*u` … `*U` (§5.3.5), locked objects `1 A` (§5.1).
//! - Paths (§5.3): `m`, `l`/`L`, `c`/`C` (lower case: the new point is smooth, upper case: a
//!   corner), painted `N n F f S s B b` (lower case closes first); guides `(N) *` before and after
//!   the path (§6); fill rule `XR`; overprint `O`/`R`; stroke `w J j M d`.
//! - Colours (§5.4): `g`/`G` (grey), `k`/`K` (CMYK), `Xa`/`XA` (`r g b`), `x`/`X` (a spot colour
//!   at a tint). Gradient fills (§3.5.3): the path, `Bb`, `1 (name) x y angle length 1 0 0 1 0 0
//!   Bg`, a radial's highlight `x y angle fraction Bh`, `f` (or `F`), and `0 BB`, or `2 BB` (`1 BB`
//!   open) to stroke it too.
//! - Images (§8): `%AI5_BeginRaster`, `[matrix] 0 0 w h w h 8 3 0 0 0 0 XI`, the RGB samples in
//!   hex on comment lines, `%AI5_EndRaster`.
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
//! Windows-1252, vertical type, inline graphics, gradient or pattern fills) is written as its
//! outlines, and all type with [`LegacyAiOptions::outline_text`].
//!
//! The prolog is a procedure set of the writer's own ([`PROLOG`]) that gives the operators their
//! meaning in PostScript, so the file also prints and previews as a PostScript program (its
//! gradients flat, without its type, which apps that read the format draw).

use std::fmt::Write;
use std::sync::Arc;

use vectorcraft_color::cms::Cms;
use vectorcraft_color::{BlendMode, Color, Gradient, GradientKind, GradientPaint, Paint};
use vectorcraft_doc::{
    AppearanceItem, AreaOptions, CharPosition, CharStyle, Document, FillLayer, ImageObject, Justify, LAYER_COLORS, LayerColor, LineCap, LineJoin,
    Node, NodeKind, ParaDirection, ParaStyle, PathEffect, StrokeLayer, TextKind, TextObject,
};
use vectorcraft_effects::stroke;
use vectorcraft_geom::{Affine, AnchorKind, BezPath, FillRule, PathData, Point, Rect, Shape, SubPath, Vec2};

use crate::{latin, ps};

/// How deep symbols and live objects may nest in one another (deeper art is left out). Groups and
/// layers need no limit of their own: a document is never deeper than
/// [`vectorcraft_doc::MAX_DEPTH`].
const MAX_NEST: u32 = 8;
/// Largest image written (pixels).
const MAX_IMAGE_PIXELS: u64 = 1 << 26;
/// Hex digits on a line of image data.
const HEX_LINE: usize = 64;

pub const SUBLAYERS: &str = "sublayers are written as groups in their top-level layer (legacy .ai files have no sublayers)";
pub const TYPE: &str = "type a legacy .ai can't hold as text (characters outside Windows Latin-1, vertical type, inline graphics, gradient or pattern fills) is written as outlines";
pub const TYPE_ATTRIBUTES: &str = "type attributes legacy .ai files don't have (underline, strikethrough, small caps, superscript and subscript, character rotation, OpenType features, area type options, type on a path effects, space after paragraphs, tabs) are left out";
pub const HIDDEN: &str = "hidden objects are left out (hidden layers are kept, hidden)";
pub const HIDDEN_LAYER: &str = "hidden objects are moved to a hidden layer above their own (legacy .ai files can hide only layers)";
pub const HIDDEN_SHOWN: &str = "hidden objects are written visible (legacy .ai files can hide only layers)";
pub const TRANSPARENCY: &str = "transparency (opacity, blending modes, opacity masks) is written opaque: legacy .ai files have none";
pub const NAMES: &str = "object names are left out (legacy .ai files name only layers)";
const PATTERNS: &str = "pattern fills are written as their tiles, clipped to the shape";
const FREEFORM: &str = "freeform gradients are written as their average colour";
const STROKES: &str =
    "strokes a plain line can't draw (inside or outside the path, variable width, gradients, brushes, arrowheads) are written as filled outlines";
const IMAGE_ALPHA: &str = "transparent image pixels are written over white";
const LIVE: &str = "symbols, blends, envelopes, meshes, repeats, placed documents and intertwines are written as their art";
const UNREADABLE_IMAGE: &str = "an image that can't be decoded or is too large was left out";
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
    let mut s = String::new();
    s.push_str(PS_HEADER);
    s.push('\n');
    let _ = writeln!(s, "%%Creator: VectorCraft {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(s, "%%Title: {}", ps::string(&o.title));
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
        let _ = writeln!(s, "{} {}", if i == 0 { "%%DocumentCustomColors:" } else { "%%+" }, ps::string(name));
    }
    for (i, (name, cmyk)) in w.spots.iter().enumerate() {
        s.push_str(if i == 0 { "%%CMYKCustomColor: " } else { "%%+ " });
        ps::push_nums(&mut s, &cmyk.map(f64::from));
        let _ = writeln!(s, "{}", ps::string(name));
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

/// The setup's fonts (§3.1, §3.3): `%%IncludeFont` for each, the encoding strings are in (`TE`),
/// and each font re-encoded to it as `_name` (`TZ`; a TrueType font takes the encoding as its own,
/// not the standard one, as the specification says of fonts that aren't Type 1).
fn fonts(s: &mut String, fonts: &[(String, bool)]) {
    if fonts.is_empty() {
        return;
    }
    for (name, _) in fonts {
        let _ = writeln!(s, "%%IncludeFont: {name}");
    }
    let te = latin::te();
    s.push_str(&te);
    // The encoding pairs, without `[` and `TE`.
    let pairs = te.trim_start_matches('[').trim_end().trim_end_matches("TE").trim_end();
    for (name, truetype) in fonts {
        let _ = writeln!(s, "%AI3_BeginEncoding: _{name} {name}");
        if *truetype {
            let _ = writeln!(s, "[{pairs}\n/_{name}/{name} 0 0 0 TZ\n%AI3_EndEncoding TrueType");
        } else {
            let _ = writeln!(s, "[/_{name}/{name} 0 0 1 TZ\n%AI3_EndEncoding AdobeType");
        }
    }
}

/// The prolog's procedure set, as `%%DocumentSuppliedResources` names it: a file that has it is
/// read from its program when its layers are asked for ([`crate::import_with`]).
pub(crate) const PROCSET: &str = "VectorCraft_Legacy 1 0";

/// The prolog: a procedure set of this writer's own (`VectorCraft_Legacy 1 0`), in a dictionary
/// (`VCLegacy`) the setup begins and the trailer ends (§1.2: the prolog holds definitions only,
/// the setup activates them, the trailer deactivates them). It gives each operator the script
/// uses its meaning in PostScript, so that the file prints and previews as a PostScript program,
/// written from the PostScript Language Reference and this format's specification (no other
/// application's procedure set): paths and their painting with the fill and stroke colours,
/// compound paths painted as one path, clipping groups, guides that don't print, layers that are
/// hidden or don't print marking nothing, and images drawn from the samples on the comment lines
/// that follow `XI`. Gradients fill with the colour set before them (the writer sets the first
/// stop's), and type marks nothing: apps that read the format draw both. The page trailer's
/// `annotatepage` (§3, Script Setup: `gsave annotatepage grestore showpage`) marks nothing.
const PROLOG: &str = "%%BeginResource: procset VectorCraft_Legacy 1 0
/VCLegacy 96 dict def
VCLegacy begin
/bd {bind def} bind def
% State: the fill and stroke colours (procedures that set them), their overprints, the fill rule,
% a clip to make at the next painting operator, a compound path being built, a guide being
% drawn, whether the layer prints, and what the compound path is painted with.
/_fc {0 setgray} def /_sc {0 setgray} def
/_fo false def /_so false def /_eo false def /_w false def /_cp false def /_g false def
/_on true def /_rf false def /_rs false def
% Painting: a fill and a stroke that keep the path; the path painted as _rf and _rs say (after
% the clip W asked for), then ended.
/_fill {gsave _fc _fo setoverprint _eo {eofill} {fill} ifelse grestore} bd
/_stroke {gsave _sc _so setoverprint stroke grestore} bd
/_draw {_w {_eo {eoclip} {clip} ifelse /_w false def} if
 _on _g not and {_rf {_fill} if _rs {_stroke} if} if newpath} bd
% close? fill? stroke? _r: paints now, or at the end of the compound path being built.
/_r {/_rs exch def /_rf exch def {closepath} if _cp not {_draw} if} bd
% Path construction (section 5.3.2) and painting (5.3.3).
/m {moveto} bd /l {lineto} bd /L {lineto} bd /c {curveto} bd /C {curveto} bd
/N {false false false _r} bd /n {true false false _r} bd
/F {false true false _r} bd /f {true true false _r} bd
/S {false false true _r} bd /s {true false true _r} bd
/B {false true true _r} bd /b {true true true _r} bd
/XR {1 eq /_eo exch def} bd
% Compound paths (5.3.5): one path, painted at its end.
/*u {/_cp true def} bd /*U {/_cp false def _draw} bd
% Groups, clipping groups and masks (5.6), locking (5.1), guides (6.1).
/u {} bd /U {} bd /q {gsave} bd /Q {grestore} bd
/H {} bd /h {closepath} bd /W {/_w true def} bd
/A {pop} bd
/* {pop _g {newpath /_g false def} {/_g true def} ifelse} bd
% Colours (5.4): fill and stroke colours, overprints; a custom colour as its CMYK equivalent at
% its tint (tint 0 is the full colour).
/g {/setgray load 2 array astore cvx /_fc exch def} bd
/G {/setgray load 2 array astore cvx /_sc exch def} bd
/k {/setcmykcolor load 5 array astore cvx /_fc exch def} bd
/K {/setcmykcolor load 5 array astore cvx /_sc exch def} bd
/Xa {/setrgbcolor load 4 array astore cvx /_fc exch def} bd
/XA {/setrgbcolor load 4 array astore cvx /_sc exch def} bd
/_tint {1 exch sub /_t exch def pop 4 {_t mul 4 1 roll} repeat} bd
/x {_tint k} bd /X {_tint K} bd
/O {1 eq /_fo exch def} bd /R {1 eq /_so exch def} bd
/w {setlinewidth} bd /J {setlinecap} bd /j {setlinejoin} bd /M {setmiterlimit} bd /d {setdash} bd
% Gradients (3.5): definitions are skipped; an instance fills with the colour set before it
% (the writer sets its first stop's), and BB strokes the path as its flag says.
/Bn {pop} bd /Bd {pop pop pop} bd /BD {cleartomark} bd
/Bb {gsave} bd /Bg {12 {pop} repeat} bd /Bh {pop pop pop pop} bd
/BB {grestore _cp {0 gt {/_rs true def} if}
 {dup 2 eq {closepath} if 0 gt _on and {_stroke} if newpath} ifelse} bd
% Layers (9): a layer that is hidden or doesn't print marks nothing.
/Lb {6 {pop} repeat exch pop exch pop 1 eq exch 1 eq and /_on exch def} bd
/Ln {pop} bd /LB {/_on true def} bd
% Images (8): the samples, in hex on the comment lines that follow XI, read a line at a time.
/_l 256 string def /_s 128 string def
/_row {{currentfile _l readline not {exit} if dup length 1 gt {exit} if pop} loop
 dup length 1 gt {dup length 1 sub 1 exch getinterval /ASCIIHexDecode filter _s readstring pop} {pop ()} ifelse} bd
/XI {pop pop pop pop /_ic exch def /_ib exch def /_ih exch def /_iw exch def pop pop pop pop
 [1 0 0 -1 0 0] exch matrix concatmatrix
 _on {gsave concat _iw _ih _ib [1 0 0 1 0 0] /_row load false _ic 1 eq {1} {_ic} ifelse colorimage grestore} {pop} ifelse} bd
% Type (15) is set by the app that reads the file: its operators take their operands and mark
% nothing; encodings (3.3) are skipped.
/TE {cleartomark} bd /TZ {cleartomark} bd
/To {pop} bd /Tp {7 {pop} repeat} bd /TP {} bd /TO {} bd
/Tr {pop} bd /Tf {pop pop pop pop} bd /Ts {pop} bd /Tz {pop pop} bd /Tt {pop} bd /TA {pop} bd
/Tl {pop pop} bd /Ta {pop} bd /Ti {pop pop pop} bd /Tx {pop} bd
% The page trailer (3): annotatepage marks nothing.
/annotatepage {} bd
end
%%EndResource
";

/// The first line of a PostScript file (the DSC version comment).
const PS_HEADER: &str = "%!PS-Adobe-3.0"; // brand-ok: the PostScript header comment

/// A gradient definition: `%AI5_BeginGradient: (name)`, `(name) type stops Bd`, `[`, the stops
/// last first (`colorSpec colorStyle midPoint rampPoint %_Bs`), `BD`, `%AI5_EndGradient`.
fn gradient_def(s: &mut String, w: &Writer, name: &str, g: &Gradient) {
    let name = ps::string(name);
    let kind = if g.kind == GradientKind::Radial { 1 } else { 0 };
    let _ = writeln!(s, "%AI5_BeginGradient: {name}\n{name} {kind} {} Bd\n[", g.stops.len());
    for stop in g.stops.iter().rev() {
        let paint = Paint::Solid { color: stop.color, swatch: stop.swatch.clone(), tint: stop.tint };
        match w.ink(&paint) {
            Ink::Gray(v) => {
                ps::push_num(s, f64::from(v));
                s.push_str(" 0 ");
            }
            Ink::Cmyk(c) => {
                ps::push_nums(s, &c.map(f64::from));
                s.push_str("1 ");
            }
            Ink::Rgb(c, rgb) => {
                ps::push_nums(s, &c.map(f64::from));
                ps::push_nums(s, &rgb.map(f64::from));
                s.push_str("2 ");
            }
            Ink::Spot { cmyk, name, tint } => {
                ps::push_nums(s, &cmyk.map(f64::from));
                let _ = write!(s, "{} ", ps::string(&name));
                ps::push_num(s, f64::from(1.0 - tint));
                s.push_str(" 3 ");
            }
        }
        ps::push_nums(s, &[f64::from(stop.midpoint) * 100.0, f64::from(stop.offset) * 100.0]);
        s.push_str("%_Bs\n");
    }
    s.push_str("BD\n%AI5_EndGradient\n");
}

/// A colour as the format writes it.
#[derive(Clone, Debug, PartialEq)]
enum Ink {
    /// Lightness (1 is white).
    Gray(f32),
    Cmyk([f32; 4]),
    /// RGB, with its CMYK equivalent.
    Rgb([f32; 4], [f32; 3]),
    /// A spot colour's full CMYK equivalent, name and tint (1: full).
    Spot {
        cmyk: [f32; 4],
        name: String,
        tint: f32,
    },
}

fn unit(v: f32) -> f32 {
    if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
}

/// Can `a` be drawn through (a finite, non-flat map)?
fn invertible(a: Affine) -> bool {
    let det = a.determinant();
    det.is_finite() && det.abs() > 1e-12
}

struct Writer<'a> {
    doc: &'a Document,
    /// The document → art space.
    to_art: Affine,
    cms: Arc<Cms>,
    body: String,
    /// Gradient definitions, by the name objects use.
    gradients: Vec<(String, Gradient)>,
    /// Spot colours used: name and full CMYK equivalent.
    spots: Vec<(String, [f32; 4])>,
    warnings: Vec<String>,
    /// The lock state the last `A` set.
    locked: bool,
    /// Symbols and live objects being written inside one another.
    nest: u32,
    /// What to do with hidden objects.
    hidden: HiddenObjects,
    /// Type as outlines.
    outline_text: bool,
    /// The layer being written's hidden objects for its hidden layer, and whether each is locked.
    hidden_art: Vec<(Node, bool)>,
    /// Writing a layer that is hidden (or the hidden layer of one's hidden objects): hidden objects
    /// are written where they are.
    in_hidden: bool,
    /// The fonts type is set in: PostScript name, and whether it's TrueType.
    fonts: Vec<(String, bool)>,
}

impl<'a> Writer<'a> {
    fn new(doc: &'a Document, to_art: Affine, o: &LegacyAiOptions) -> Self {
        Self {
            doc,
            to_art,
            cms: vectorcraft_color::cms::active(),
            body: String::new(),
            gradients: vec![],
            spots: vec![],
            warnings: vec![],
            locked: false,
            nest: 0,
            hidden: o.hidden,
            outline_text: o.outline_text,
            hidden_art: vec![],
            in_hidden: false,
            fonts: vec![],
        }
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn line(&mut self, s: &str) {
        self.body.push_str(s);
        self.body.push('\n');
    }

    // ---------- layers and objects ----------

    /// A top-level layer. False when it isn't a layer.
    fn layer(&mut self, n: &Node) -> bool {
        let NodeKind::Layer { color, template, printable, children, clip, preview, dim_images } = &n.kind else { return false };
        let [r, g, b] = match color {
            LayerColor::Preset(i) => LAYER_COLORS.get(usize::from(*i)).map_or([79, 128, 255], |c| c.1),
            LayerColor::Custom(rgb) => *rgb,
        };
        let index = match color {
            LayerColor::Preset(i) if usize::from(*i) < LAYER_COLORS.len() => i64::from(*i),
            _ => -1,
        };
        let flag = |b: bool| u8::from(b);
        // A template layer doesn't print and shows dimmed.
        let _ = writeln!(
            self.body,
            "%AI5_BeginLayer\n{} {} {} {} {} 0 {index} {r} {g} {b} Lb\n{} Ln",
            flag(n.visible),
            flag(*preview),
            flag(!n.locked),
            flag(*printable && !*template),
            flag(dim_images.is_some() || *template),
            ps::string(n.name.as_deref().unwrap_or("Layer")),
        );
        self.locked = false;
        self.line("0 A");
        self.in_hidden = !n.visible;
        self.contents(children, *clip, false);
        self.in_hidden = false;
        self.line("LB\n%AI5_EndLayer--");
        true
    }

    /// The hidden layer of top-level layer `n`'s hidden objects, just above it (named after it,
    /// with its colour, lock and printing). False when it has none.
    fn hidden_layer(&mut self, n: &Node) -> bool {
        if self.hidden_art.is_empty() {
            return false;
        }
        let art = std::mem::take(&mut self.hidden_art);
        let NodeKind::Layer { color, template, printable, .. } = &n.kind else { return false };
        let [r, g, b] = match color {
            LayerColor::Preset(i) => LAYER_COLORS.get(usize::from(*i)).map_or([79, 128, 255], |c| c.1),
            LayerColor::Custom(rgb) => *rgb,
        };
        let index = match color {
            LayerColor::Preset(i) if usize::from(*i) < LAYER_COLORS.len() => i64::from(*i),
            _ => -1,
        };
        let name = format!("{} (hidden)", n.name.as_deref().unwrap_or("Layer"));
        let _ = writeln!(
            self.body,
            "%AI5_BeginLayer\n0 1 {} {} {} 0 {index} {r} {g} {b} Lb\n{} Ln",
            u8::from(!n.locked),
            u8::from(*printable && !*template),
            u8::from(*template),
            ps::string(&name),
        );
        self.locked = false;
        self.line("0 A");
        self.in_hidden = true;
        for (node, locked) in &art {
            self.node(node, *locked);
        }
        self.in_hidden = false;
        self.line("LB\n%AI5_EndLayer--");
        true
    }

    /// A container's members, the first clipping the others when `clip`; `locked`: the container is.
    fn contents(&mut self, children: &[Arc<Node>], clip: bool, locked: bool) {
        if clip {
            self.clip_group(children, locked);
        } else {
            for c in children {
                self.node(c, locked);
            }
        }
    }

    /// Set the lock state of the objects that follow.
    fn lock(&mut self, locked: bool) {
        if locked != self.locked {
            self.locked = locked;
            self.line(if locked { "1 A" } else { "0 A" });
        }
    }

    /// One object (inside a container locked when `locked`).
    fn node(&mut self, n: &Node, locked: bool) {
        if !n.visible && !self.in_hidden {
            match self.hidden {
                HiddenObjects::Omit => {
                    self.warn(HIDDEN);
                    return;
                }
                HiddenObjects::Layer => {
                    self.warn(HIDDEN_LAYER);
                    self.hidden_art.push((n.clone(), locked || n.locked));
                    return;
                }
                HiddenObjects::Visible => self.warn(HIDDEN_SHOWN),
            }
        }
        if n.opacity < 1.0 || n.blend != BlendMode::Normal || n.extras().mask.as_ref().is_some_and(|m| !m.disabled) {
            self.warn(TRANSPARENCY);
        }
        if n.name.as_deref().is_some_and(|s| !s.is_empty()) && !n.is_layer() {
            self.warn(NAMES);
        }
        let locked = locked || n.locked;
        match &n.kind {
            NodeKind::Layer { children, clip, .. } => {
                self.warn(SUBLAYERS);
                self.group(children, *clip, locked);
            }
            NodeKind::Group { children, clip } => self.group(children, *clip, locked),
            NodeKind::Path { path, rule, guide, .. } => {
                self.lock(locked);
                if *guide {
                    self.guide(path);
                } else {
                    self.shape(n, path, *rule);
                }
            }
            NodeKind::Compound { children, rule } => {
                self.lock(locked);
                let mut subpaths = vec![];
                for p in children.iter().filter_map(|c| c.path_data()) {
                    subpaths.extend(p.subpaths.iter().cloned());
                }
                self.shape(n, &PathData::new(subpaths), *rule);
            }
            NodeKind::Text(t) => {
                self.lock(locked);
                self.text(t);
            }
            NodeKind::Image(im) => {
                self.lock(locked);
                self.image(im);
            }
            NodeKind::SymbolInstance { symbol, xf } => {
                self.warn(LIVE);
                if let Some(sym) = self.doc.symbols.iter().find(|s| &s.name == symbol)
                    && self.nest < MAX_NEST
                {
                    let mut art = (*sym.art).clone();
                    art.transform(*xf, false);
                    art.visible = true;
                    self.nest += 1;
                    self.node(&art, locked);
                    self.nest -= 1;
                }
            }
            NodeKind::Blend { .. }
            | NodeKind::Envelope { .. }
            | NodeKind::Mesh(_)
            | NodeKind::Repeat(_)
            | NodeKind::PlacedDocument(_)
            | NodeKind::Intertwine { .. } => {
                self.warn(LIVE);
                if self.nest < MAX_NEST {
                    let g = vectorcraft_effects::expand_live_deep(Some(self.doc), n);
                    self.nest += 1;
                    if let Some(children) = g.children() {
                        self.group(children, false, locked);
                    }
                    self.nest -= 1;
                }
            }
        }
    }

    /// A group (`u` … `U`), or a clipping group (`q` … `Q`).
    fn group(&mut self, children: &[Arc<Node>], clip: bool, locked: bool) {
        if children.is_empty() {
            return;
        }
        self.lock(locked);
        if clip {
            self.clip_group(children, locked);
        } else {
            self.line("u");
            self.contents(children, false, locked);
            self.lock(locked);
            self.line("U");
        }
    }

    /// A clipping group: its first member's outline clips the others (`h W n`). That member's fills
    /// paint behind the art, inside the clip, and its strokes over it, outside.
    fn clip_group(&mut self, children: &[Arc<Node>], locked: bool) {
        let Some((clip, rest)) = children.split_first() else { return };
        // Nothing to clip by hides the clipped art.
        let Some((outline, rule)) = vectorcraft_effects::clip_outline(clip) else { return };
        let paint = clip.clip_paint();
        self.lock(locked);
        self.line("q");
        let path = PathData::from_bezpath(&outline);
        self.fill_rule(rule);
        let several = path.subpaths.len() > 1;
        if several {
            self.line("*u");
        }
        for sp in path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()) {
            self.mask(sp);
        }
        if several {
            self.line("*U");
        }
        if let Some(fill) = &paint.fill {
            self.node(fill, locked);
        }
        for c in rest {
            self.node(c, locked);
        }
        self.lock(locked);
        self.line("Q");
        if let Some(stroke) = &paint.stroke {
            self.node(stroke, locked);
        }
    }

    // ---------- paths ----------

    /// A clipping path (§5.6.2): its points, `h` (closed) or `H`, `W`, then painted with nothing.
    fn mask(&mut self, sp: &SubPath) {
        self.subpath(sp);
        self.line(if sp.closed { "h\nW\nn" } else { "H\nW\nN" });
    }

    /// A guide (§6): `(N) *`, its path, `(N) *`.
    fn guide(&mut self, path: &PathData) {
        for sp in path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()) {
            self.line("(N) *");
            self.subpath(sp);
            self.line("(N) *");
        }
    }

    /// A path's fills and strokes: one object with one fill and a plain stroke over it where it
    /// can be, else a group of an object for each (in paint order).
    fn shape(&mut self, n: &Node, path: &PathData, rule: FillRule) {
        if path.is_empty() {
            return;
        }
        let items: Vec<&AppearanceItem> = n
            .appearance
            .items
            .iter()
            .filter(|i| match i {
                AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
                AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0 && s.width.is_finite(),
            })
            .collect();
        for i in &items {
            if i.opacity() < 1.0 || i.blend() != BlendMode::Normal {
                self.warn(TRANSPARENCY);
            }
        }
        match items.as_slice() {
            [] => self.path_object(path, rule, None, None),
            [AppearanceItem::Fill(f)] if direct_fill(&f.paint) => self.path_object(path, rule, Some(f), None),
            [AppearanceItem::Stroke(s)] if direct_stroke(s) => self.path_object(path, rule, None, Some(s)),
            [AppearanceItem::Fill(f), AppearanceItem::Stroke(s)] if direct_fill(&f.paint) && direct_stroke(s) => {
                self.path_object(path, rule, Some(f), Some(s))
            }
            [one] => self.item(path, rule, one),
            items => {
                self.line("u");
                for i in items {
                    self.item(path, rule, i);
                }
                self.line("U");
            }
        }
    }

    /// One fill or stroke of a path as an object of its own.
    fn item(&mut self, path: &PathData, rule: FillRule, item: &AppearanceItem) {
        match item {
            AppearanceItem::Fill(f) => self.fill_object(path, rule, f),
            AppearanceItem::Stroke(s) if direct_stroke(s) => self.path_object(path, rule, None, Some(s)),
            AppearanceItem::Stroke(s) => {
                self.warn(STROKES);
                let outline = stroke::outline_region(path, rule, s);
                let fill = FillLayer { overprint: s.overprint, ..FillLayer::new(s.paint.clone()) };
                self.fill_object(&outline, FillRule::NonZero, &fill);
            }
        }
    }

    /// A fill as an object of its own: patterns as their tiles clipped to the path.
    fn fill_object(&mut self, path: &PathData, rule: FillRule, f: &FillLayer) {
        if direct_fill(&f.paint) {
            return self.path_object(path, rule, Some(f), None);
        }
        let Paint::Pattern { pattern, xf } = &f.paint else {
            // A freeform gradient: its average colour.
            self.warn(FREEFORM);
            let color = match &f.paint {
                Paint::Gradient(g) => average(&g.gradient),
                _ => Color::gray(0.5),
            };
            let flat = FillLayer { paint: Paint::solid(color), ..f.clone() };
            return self.path_object(path, rule, Some(&flat), None);
        };
        self.warn(PATTERNS);
        let doc = self.doc;
        let Some(def) = doc.pattern(pattern) else {
            let flat = FillLayer { paint: Paint::solid(Color::gray(0.5)), ..f.clone() };
            return self.path_object(path, rule, Some(&flat), None);
        };
        let area = path.to_bezpath().bounding_box();
        let tiles = def.instances_in(*xf, area);
        self.line("q");
        self.fill_rule(rule);
        let several = path.subpaths.len() > 1;
        if several {
            self.line("*u");
        }
        for sp in path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()) {
            self.mask(sp);
        }
        if several {
            self.line("*U");
        }
        let locked = self.locked;
        for t in &tiles {
            self.node(t, locked);
        }
        self.lock(locked);
        self.line("Q");
    }

    /// A path object painted with `fill` (a colour or a linear or radial gradient) and `stroke`
    /// (a plain line of a colour) over it; several subpaths make a compound path.
    fn path_object(&mut self, path: &PathData, rule: FillRule, fill: Option<&FillLayer>, stroke: Option<&StrokeLayer>) {
        let subpaths: Vec<&SubPath> = path.subpaths.iter().filter(|sp| !sp.anchors.is_empty()).collect();
        if subpaths.is_empty() {
            return;
        }
        let gradient = match fill.map(|f| &f.paint) {
            Some(Paint::Gradient(g)) => Some(self.gradient(g, path.to_bezpath().bounding_box())),
            _ => None,
        };
        if let Some(f) = fill {
            let paint = match &f.paint {
                // The colour under the gradient: its first stop's.
                Paint::Gradient(g) => g.gradient.stops.first().map_or(Paint::None, |s| Paint::solid(s.color)),
                p => p.clone(),
            };
            self.color(&paint, false);
            self.line(if f.overprint { "1 O" } else { "0 O" });
        }
        if let Some(s) = stroke {
            self.color(&s.paint, true);
            self.line(if s.overprint { "1 R" } else { "0 R" });
            self.line_style(s);
        }
        self.fill_rule(rule);
        let op = match (fill.is_some(), stroke.is_some()) {
            (true, true) => 'b',
            (true, false) => 'f',
            (false, true) => 's',
            (false, false) => 'n',
        };
        let several = subpaths.len() > 1;
        if several {
            self.line("*u");
        }
        for sp in subpaths {
            self.subpath(sp);
            if let Some(g) = &gradient {
                // The gradient fills between `Bb` and `BB`, whose flag strokes the path too (§3.5.3).
                self.body.push_str(g);
                self.line(if sp.closed { "f" } else { "F" });
                self.line(match (stroke.is_some(), sp.closed) {
                    (false, _) => "0 BB",
                    (true, false) => "1 BB",
                    (true, true) => "2 BB",
                });
            } else {
                self.line(&if sp.closed { op.to_string() } else { op.to_ascii_uppercase().to_string() });
            }
        }
        if several {
            self.line("*U");
        }
    }

    /// `rule` as `XR` (1: even-odd).
    fn fill_rule(&mut self, rule: FillRule) {
        self.line(if rule == FillRule::EvenOdd { "1 XR" } else { "0 XR" });
    }

    /// One subpath's points, in art space: `m`, then a line or curve to each point (lower case: the
    /// point is smooth, upper case: a corner, §5.3.2), and for a closed one the segment back to the
    /// start, which says whether the start is smooth or a corner (as the specification's examples
    /// close paths: the start point again, then the painting operator).
    fn subpath(&mut self, sp: &SubPath) {
        let Some(first) = sp.anchors.first() else { return };
        let xf = self.to_art;
        let s = &mut self.body;
        let pt = |s: &mut String, p: Point| {
            let p = xf * p;
            ps::push_nums(s, &[p.x, p.y]);
        };
        pt(s, first.p);
        s.push_str("m\n");
        let segment = |s: &mut String, a: &vectorcraft_geom::Anchor, b: &vectorcraft_geom::Anchor| {
            let straight = a.h_out == a.p && b.h_in == b.p;
            let smooth = b.kind == AnchorKind::Smooth;
            if straight {
                pt(s, b.p);
                s.push_str(if smooth { "l\n" } else { "L\n" });
            } else {
                pt(s, a.h_out);
                pt(s, b.h_in);
                pt(s, b.p);
                s.push_str(if smooth { "c\n" } else { "C\n" });
            }
        };
        for pair in sp.anchors.windows(2) {
            if let [a, b] = pair {
                segment(s, a, b);
            }
        }
        if sp.closed
            && sp.anchors.len() > 1
            && let Some(last) = sp.anchors.last()
        {
            segment(s, last, first);
        }
    }

    /// The stroke's width, cap, join, miter limit and dashes: `w J j M d`.
    fn line_style(&mut self, st: &StrokeLayer) {
        let cap = match st.cap {
            LineCap::Butt => 0,
            LineCap::Round => 1,
            LineCap::Square => 2,
        };
        let join = match st.join {
            LineJoin::Miter => 0,
            LineJoin::Round => 1,
            LineJoin::Bevel => 2,
        };
        let s = &mut self.body;
        ps::push_num(s, st.width);
        let _ = write!(s, " w {cap} J {join} j ");
        ps::push_num(s, if st.miter_limit.is_finite() { st.miter_limit.max(1.0) } else { 10.0 });
        s.push_str(" M [");
        let dash = st.dash.as_ref().filter(|d| d.is_dashed());
        if let Some(d) = dash {
            ps::push_nums(s, &d.pattern);
            s.pop();
        }
        s.push_str("] ");
        ps::push_num(s, dash.map_or(0.0, |d| d.offset));
        s.push_str(" d\n");
    }

    // ---------- colour ----------

    /// How `p` (a solid paint) is written.
    fn ink(&self, p: &Paint) -> Ink {
        let Paint::Solid { color, swatch, tint } = p else { return Ink::Gray(0.0) };
        let doc = self.doc;
        let intent = self.cms.settings().intent;
        if let Some(name) = swatch
            && let Some(sw) = doc.swatch(name).filter(|s| s.spot)
            && let Some(full) = sw.paint.color()
        {
            let full = doc.linked_color(full, true);
            return Ink::Spot { cmyk: self.cms.to_cmyk(&full, intent).map(unit), name: name.clone(), tint: unit(*tint) };
        }
        match *color {
            // Grey is ink coverage; the format's grey is lightness.
            Color::Gray { k } => Ink::Gray(unit(1.0 - k)),
            Color::Cmyk { c, m, y, k } => Ink::Cmyk([c, m, y, k].map(unit)),
            Color::Rgb { r, g, b } => Ink::Rgb(self.cms.to_cmyk(color, intent).map(unit), [r, g, b].map(unit)),
            Color::Lab { .. } => {
                let rgb = color.to_rgb().map(unit);
                Ink::Rgb(self.cms.to_cmyk(color, intent).map(unit), rgb)
            }
        }
    }

    /// Set the fill (or with `stroke`, the stroke) colour to solid paint `p`.
    fn color(&mut self, p: &Paint, stroke: bool) {
        let ink = self.ink(p);
        let op = |fill: &'static str, line: &'static str| if stroke { line } else { fill };
        let s = &mut self.body;
        match ink {
            Ink::Gray(v) => {
                ps::push_num(s, f64::from(v));
                let _ = writeln!(s, " {}", op("g", "G"));
            }
            Ink::Cmyk(c) => {
                ps::push_nums(s, &c.map(f64::from));
                let _ = writeln!(s, "{}", op("k", "K"));
            }
            Ink::Rgb(_, rgb) => {
                ps::push_nums(s, &rgb.map(f64::from));
                let _ = writeln!(s, "{}", op("Xa", "XA"));
            }
            Ink::Spot { cmyk, name, tint } => {
                ps::push_nums(s, &cmyk.map(f64::from));
                let _ = write!(s, "{} ", ps::string(&name));
                // The format's tint is how much lighter than the full colour (0: full).
                ps::push_num(s, f64::from(1.0 - tint));
                let _ = writeln!(s, " {}", op("x", "X"));
                if !self.spots.iter().any(|(n, _)| *n == name) {
                    self.spots.push((name, cmyk));
                }
            }
        }
    }

    // ---------- gradients ----------

    /// The definition gradient `g` uses (defined once for each distinct gradient), and its
    /// instance on a path of bounds `bounds`: `Bb`, `1 (name) x y angle length 1 0 0 1 0 0 Bg`, a
    /// radial's highlight `x y angle fraction Bh` (its distance a fraction of the radius, §3.5.5).
    fn gradient(&mut self, g: &GradientPaint, bounds: Rect) -> String {
        let name = self.define(&g.gradient, g.swatch.as_deref());
        for stop in &g.gradient.stops {
            if stop.opacity < 1.0 {
                self.warn(TRANSPARENCY);
            }
            if let Some(sw) = &stop.swatch {
                // Spot colours in stops are custom colours too.
                let p = Paint::Solid { color: stop.color, swatch: Some(sw.clone()), tint: stop.tint };
                if let Ink::Spot { cmyk, name, .. } = self.ink(&p)
                    && !self.spots.iter().any(|(n, _)| *n == name)
                {
                    self.spots.push((name, cmyk));
                }
            }
        }
        let geom = g.resolve(bounds);
        let (start, end) = (self.to_art * geom.start, self.to_art * geom.end);
        let d = end - start;
        let (angle, length) = (d.y.atan2(d.x).to_degrees(), d.hypot());
        // The matrix is relative to the origin of a canvas whose size the specification doesn't
        // give (§3.5.4), so it stays the identity: an elliptical radial is written circular.
        if g.gradient.kind == GradientKind::Radial && (geom.aspect - 1.0).abs() > 1e-6 {
            self.warn(ELLIPTICAL);
        }
        let mut s = String::from("Bb\n");
        if g.gradient.kind == GradientKind::Radial
            && let Some(f) = geom.focal
        {
            let off: Vec2 = self.to_art * f - start;
            if off.hypot() > 1e-9 && length > 1e-9 {
                let fraction = (off.hypot() / length).min(1.0);
                ps::push_nums(&mut s, &[off.x, off.y, off.y.atan2(off.x).to_degrees(), fraction]);
                s.push_str("Bh\n");
            }
        }
        s.push_str("1 ");
        s.push_str(&ps::string(&name));
        s.push(' ');
        ps::push_nums(&mut s, &[start.x, start.y, angle, length]);
        s.push_str("1 0 0 1 0 0 Bg\n");
        s
    }

    /// The name of the definition of `g` (its swatch's name when it has one), defined the first
    /// time.
    fn define(&mut self, g: &Gradient, swatch: Option<&str>) -> String {
        if let Some((name, _)) = self.gradients.iter().find(|(_, d)| d == g) {
            return name.clone();
        }
        let taken = |n: &str, defs: &[(String, Gradient)]| defs.iter().any(|(m, _)| m == n);
        let base = swatch.filter(|s| !s.is_empty()).unwrap_or("Gradient").to_string();
        let mut name = base.clone();
        let mut i = 1;
        while taken(&name, &self.gradients) {
            i += 1;
            name = format!("{base} {i}");
        }
        self.gradients.push((name.clone(), g.clone()));
        name
    }

    // ---------- type ----------

    /// Type: a text object when the format can hold it as text, else its outlines.
    fn text(&mut self, t: &TextObject) {
        if !invertible(t.xf) {
            return;
        }
        if !self.outline_text {
            if writable(t) {
                return self.text_object(t);
            }
            self.warn(TYPE);
        }
        self.outlines(t);
    }

    /// A text object (§15): `type To`, the text path (`a b c d tx ty startPt Tp`, an area's frame
    /// or the path type runs along, `TP`), then each run's style and characters, `TO`. The matrix
    /// takes text space (y up, the first baseline's start for point type) to art space; the frame
    /// and path are in art space, as the specification's examples have them (§15.5).
    fn text_object(&mut self, t: &TextObject) {
        if lost_attributes(t) {
            self.warn(TYPE_ATTRIBUTES);
        }
        // Area type that Shrink Text to Fit scales down is written in the styles it draws in.
        let fitted = matches!((&t.kind, t.area.fit), (TextKind::Area { .. }, vectorcraft_doc::AreaFit::ShrinkText { .. }))
            .then(|| vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).fit_scale)
            .filter(|f| *f != 1.0)
            .map(|f| vectorcraft_text::scaled(t, f));
        let t = fitted.as_ref().unwrap_or(t);
        let m = self.to_art * t.xf * Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);
        let (kind, path, start) = match &t.kind {
            TextKind::Point => (0, None, 0.0),
            TextKind::Area { frame } => (1, frame.subpaths.iter().find(|sp| !sp.anchors.is_empty()).cloned(), 0.0),
            TextKind::OnPath { path, .. } => {
                let sp = path.subpaths.iter().find(|sp| !sp.anchors.is_empty()).cloned();
                let fraction = t.kind.path_span().map_or(0.0, |(s, _)| s);
                let start = sp
                    .as_ref()
                    .map_or(0.0, |sp| segment_point(&PathData::new(vec![sp.clone()]).transformed(self.to_art * t.xf).to_bezpath(), fraction));
                (2, sp, start)
            }
        };
        let _ = writeln!(self.body, "{kind} To");
        let [a, b, c, d, e, f] = m.as_coeffs();
        ps::push_nums(&mut self.body, &[a, b, c, d, e, f, start]);
        self.line("Tp");
        if let Some(sp) = path {
            let placed = SubPath::new(sp.anchors.iter().map(|a| a.transform(t.xf)).collect(), sp.closed);
            self.subpath(&placed);
            self.line(if sp.closed { "n" } else { "N" });
        }
        self.line("TP");
        let mut para = 0usize;
        let mut para_start = true;
        for run in &t.runs {
            let text = if run.style.all_caps { run.text.to_uppercase() } else { run.text.clone() };
            let mut pieces = text.split('\n').peekable();
            while let Some(piece) = pieces.next() {
                let ends_paragraph = pieces.peek().is_some();
                if piece.is_empty() && !ends_paragraph {
                    continue;
                }
                if para_start {
                    self.paragraph(t.para_at(para));
                    para_start = false;
                }
                self.char_style(&run.style, t.para_at(para));
                let mut bytes: Vec<u8> = piece.chars().filter_map(latin::encode).collect();
                if ends_paragraph {
                    bytes.push(b'\r');
                    para += 1;
                    para_start = true;
                }
                self.body.push_str(&ps::bytes(&bytes));
                self.line(" Tx");
            }
        }
        self.line("TO");
    }

    /// A paragraph's attributes: alignment `Ta` and indents `Ti` (first line's, left, right).
    fn paragraph(&mut self, p: &ParaStyle) {
        let align = match p.justify {
            Justify::Auto if p.direction == Some(ParaDirection::RightToLeft) => 2,
            Justify::Auto | Justify::Left => 0,
            Justify::Center => 1,
            Justify::Right => 2,
            Justify::JustifyLeft | Justify::JustifyCenter | Justify::JustifyRight => 3,
            Justify::JustifyAll => 4,
        };
        let _ = writeln!(self.body, "{align} Ta");
        ps::push_nums(&mut self.body, &[p.first_line_indent, p.left_indent, p.right_indent]);
        self.line("Ti");
    }

    /// A run's character attributes and paint style: `Tr`, its fill and stroke, `Tf`, `Ts`, `Tz`,
    /// `Tt`, `TA`, and the leading with the space before its paragraph, `Tl`.
    fn char_style(&mut self, st: &CharStyle, p: &ParaStyle) {
        let fill = !st.fill.is_none();
        let stroke = st.has_stroke();
        let render = match (fill, stroke) {
            (true, false) => 0,
            (false, true) => 1,
            (true, true) => 2,
            (false, false) => 3,
        };
        let _ = writeln!(self.body, "{render} Tr");
        if fill {
            self.color(&st.fill, false);
            self.line(if st.overprint_fill { "1 O" } else { "0 O" });
        }
        if stroke {
            self.color(&st.stroke, true);
            self.line(if st.overprint_stroke { "1 R" } else { "0 R" });
            self.line_style(&st.stroke_layer());
        }
        let (name, truetype, ascent, descent) = font_of(st);
        if !self.fonts.iter().any(|(n, _)| *n == name) {
            self.fonts.push((name.clone(), truetype));
        }
        let size = finite(st.size, 12.0);
        let _ = write!(self.body, "/_{name} ");
        ps::push_nums(&mut self.body, &[size, ascent * size, descent * size]);
        self.line("Tf");
        ps::push_num(&mut self.body, finite(st.baseline_shift, 0.0));
        self.line(" Ts");
        ps::push_nums(&mut self.body, &[finite(st.h_scale, 100.0), finite(st.v_scale, 100.0)]);
        self.line("Tz");
        ps::push_num(&mut self.body, finite(st.tracking, 0.0));
        self.line(" Tt");
        self.line(if st.kerning.is_none() { "1 TA" } else { "0 TA" });
        ps::push_nums(&mut self.body, &[finite(st.effective_leading(), size * 1.2), finite(p.space_before, 0.0)]);
        self.line("Tl");
    }

    /// Type as its glyphs' outlines: a group of a compound path for each run, with the run's fill
    /// and stroke.
    fn outlines(&mut self, t: &TextObject) {
        let layout = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
        let scale = t.xf.determinant().abs().sqrt();
        self.line("u");
        for (i, run) in t.runs.iter().enumerate() {
            let mut bp = BezPath::new();
            for g in layout.glyphs.iter().filter(|g| g.run == i) {
                bp.extend(g.outline.iter());
            }
            if bp.elements().is_empty() {
                continue;
            }
            bp.apply_affine(t.xf);
            let path = PathData::from_bezpath(&bp);
            let fill =
                (!run.style.fill.is_none()).then(|| FillLayer { overprint: run.style.overprint_fill, ..FillLayer::new(run.style.fill.clone()) });
            let stroke = run.style.has_stroke().then(|| {
                let mut s = run.style.stroke_layer();
                s.width *= scale;
                s
            });
            let items: Vec<AppearanceItem> = fill.map(AppearanceItem::Fill).into_iter().chain(stroke.map(AppearanceItem::Stroke)).collect();
            let mut n = Node::path(vectorcraft_doc::NodeId(0), path.clone(), vectorcraft_doc::Appearance::default());
            n.appearance = vectorcraft_doc::Appearance { items, ..vectorcraft_doc::Appearance::default() }.into();
            self.shape(&n, &path, FillRule::NonZero);
        }
        self.line("U");
    }

    // ---------- images ----------

    /// An image (§8): `%AI5_BeginRaster`, its matrix (an art-space matrix applied to the pixels
    /// with their rows running downwards, `(u, -v)`) and operands with `XI`, its RGB samples in hex
    /// on comment lines, `%AI5_EndRaster`.
    fn image(&mut self, im: &ImageObject) {
        let flip = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);
        let m = self.to_art * im.xf * flip;
        if !invertible(m) {
            return;
        }
        let Some(img) = self.doc.images.get(&im.key).and_then(|b| image::load_from_memory(&b.bytes).ok()) else {
            self.warn(UNREADABLE_IMAGE);
            return;
        };
        let (w, h) = (img.width(), img.height());
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > MAX_IMAGE_PIXELS {
            self.warn(UNREADABLE_IMAGE);
            return;
        }
        // The object places the image's own pixel grid (its width × height) wherever the file's is.
        let fit = Affine::scale_non_uniform(f64::from(im.width.max(1)) / f64::from(w), f64::from(im.height.max(1)) / f64::from(h));
        let m = self.to_art * im.xf * fit * flip;
        let rgba = img.to_rgba8();
        let mut hex = String::with_capacity(rgba.len() / 4 * 7);
        hex.push('%');
        let mut on_line = 0;
        let mut see_through = false;
        for px in rgba.pixels() {
            let [r, g, b, a] = px.0;
            if a < 255 {
                see_through = true;
            }
            // Over white.
            let over = |c: u8| ((u32::from(c) * u32::from(a) + 255 * (255 - u32::from(a)) + 127) / 255) as u8;
            for c in [over(r), over(g), over(b)] {
                let _ = write!(hex, "{c:02X}");
                on_line += 2;
                if on_line >= HEX_LINE {
                    hex.push_str("\n%");
                    on_line = 0;
                }
            }
        }
        if hex.ends_with('%') {
            hex.pop();
        } else {
            hex.push('\n');
        }
        if see_through {
            self.warn(IMAGE_ALPHA);
        }
        let [a, b, c, d, e, f] = m.as_coeffs();
        let s = &mut self.body;
        s.push_str("%AI5_File:\n%AI5_BeginRaster\n[");
        ps::push_nums(s, &[a, b, c, d, e, f]);
        s.pop();
        let _ = writeln!(s, "] 0 0 {w} {h} {w} {h} 8 3 0 0 0 0 XI");
        s.push_str(&hex);
        s.push_str("%AI5_EndRaster\n");
    }
}

/// Can type `t` be written as text: horizontal, its runs plain characters (no inline graphics)
/// that 8-bit text can hold, filled and stroked with colours?
fn writable(t: &TextObject) -> bool {
    let solid = |p: &Paint| matches!(p, Paint::Solid { .. } | Paint::None);
    !t.vertical
        && t.runs.iter().any(|r| !r.text.is_empty())
        && t.runs.iter().all(|r| {
            r.inline.is_none()
                && solid(&r.style.fill)
                && solid(&r.style.stroke)
                && (if r.style.all_caps { r.text.to_uppercase() } else { r.text.clone() }).chars().all(|c| latin::encode(c).is_some())
        })
}

/// Has type `t` attributes a legacy `.ai` text object can't hold (lost when it's written as text)?
fn lost_attributes(t: &TextObject) -> bool {
    let run_lost = |s: &CharStyle| {
        s.underline || s.strikethrough || s.small_caps.is_some() || s.position != CharPosition::Normal || s.rotation != 0.0 || !s.features.is_empty()
    };
    let para_lost = |p: &ParaStyle| p.space_after != 0.0 || !p.tabs.is_empty();
    t.runs.iter().any(|r| run_lost(&r.style))
        || t.paragraph_styles().iter().any(para_lost)
        || (matches!(t.kind, TextKind::Area { .. }) && t.area != AreaOptions::default())
        || (matches!(t.kind, TextKind::OnPath { .. }) && (t.path_effect != PathEffect::default() || t.path_spacing != 0.0))
}

/// The font type in style `st` is set in: its PostScript name, whether it's TrueType, and its
/// ascent and descent (ems). A font that isn't installed is named after its family and style.
fn font_of(st: &CharStyle) -> (String, bool, f64, f64) {
    let db = vectorcraft_text::FontDb::global();
    match db.face_version(&st.font_family, &st.font_style, st.font_version.as_deref()) {
        Some(face) => {
            let (ascent, descent) = face.vertical_metrics();
            let em = face.units_per_em().max(1.0);
            let name = face.postscript_name();
            (name, face.is_truetype(), ascent / em, descent / em)
        }
        None => {
            let squeeze = |s: &str| -> String { s.chars().filter(|c| c.is_ascii_alphanumeric()).collect() };
            let style = squeeze(&st.font_style);
            let name =
                if style.is_empty() || style == "Regular" { squeeze(&st.font_family) } else { format!("{}-{style}", squeeze(&st.font_family)) };
            (if name.is_empty() { "Helvetica".into() } else { name }, false, 0.75, 0.25)
        }
    }
}

/// `v`, or `default` when it isn't finite.
fn finite(v: f64, default: f64) -> f64 {
    if v.is_finite() { v } else { default }
}

/// Where a fraction of `path`'s length is, as the start of type on a path gives it (§15.5.2): the
/// index of the segment it's on, plus how far along that segment (its curve parameter).
fn segment_point(path: &BezPath, fraction: f64) -> f64 {
    use kurbo::ParamCurveArclen;
    const ACCURACY: f64 = 1e-3;
    let segments: Vec<kurbo::PathSeg> = path.segments().collect();
    let lengths: Vec<f64> = segments.iter().map(|s| s.arclen(ACCURACY)).collect();
    let total: f64 = lengths.iter().sum();
    if !(total.is_finite() && total > 0.0) {
        return 0.0;
    }
    let mut left = fraction.clamp(0.0, 1.0) * total;
    for (i, (seg, len)) in segments.iter().zip(&lengths).enumerate() {
        if left <= *len || i + 1 == segments.len() {
            let t = if *len > 0.0 { seg.inv_arclen(left.min(*len), ACCURACY) } else { 0.0 };
            return i as f64 + t.clamp(0.0, 0.9999);
        }
        left -= len;
    }
    0.0
}

/// Can `p` paint a path object's fill directly (a colour or a linear or radial gradient)?
fn direct_fill(p: &Paint) -> bool {
    match p {
        Paint::Solid { .. } => true,
        Paint::Gradient(g) => g.gradient.kind != GradientKind::Freeform && !g.gradient.stops.is_empty(),
        _ => false,
    }
}

/// Can stroke `s` be written as the stroke of a path object (a plain line of a colour)?
fn direct_stroke(s: &StrokeLayer) -> bool {
    matches!(s.paint, Paint::Solid { .. }) && stroke::is_plain(s)
}

/// The average colour of a gradient's stops (in RGB).
fn average(g: &Gradient) -> Color {
    let n = g.stops.len().max(1) as f32;
    let sum = g.stops.iter().map(|s| s.color.to_rgb()).fold([0.0f32; 3], |a, c| [a[0] + c[0], a[1] + c[1], a[2] + c[2]]);
    Color::rgb(unit(sum[0] / n), unit(sum[1] / n), unit(sum[2] / n))
}

#[cfg(test)]
#[path = "tests_legacy_ai.rs"]
mod tests;
