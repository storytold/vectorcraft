//! The reader: runs the editing data's operators into a document.
//!
//! - Art space has y up; the first artboard's top left corner is the document's origin.
//! - Layers: `%AI5_BeginLayer`, `visible preview unlocked printing dimmed … colorIndex r g b …
//!   dimPercent … Lb` (ten operands in the legacy format, without the one before the colour
//!   index), `(name) Ln`, its art, `LB`.
//! - Groups `u` … `U`, compound paths `*u` … `*U`, clipping groups `q` … `Q` (the path painted
//!   with `W` clips); a group's transparency and name follow its end.
//! - Paths: `m`, `l`/`L`, `c`/`C`, `v`/`V`, `y`/`Y`, painted by `N n F f S s B b` (lower case
//!   closes), `(op) *` for a guide. Stroke `w J j M d`, fill rule `XR`, overprint `O`/`R`.
//! - State: hidden `Xw`, locked `A`, transparency `mode opacity isolate knockout shape Xy`.
//! - Names: an `/ArtDictionary` after an object, its `AI10_ArtUID` the name's XML id.
//! - Artboards: the `ArtboardArray` of the `/Document` dictionary, else `%AI3_Cropmarks`, else
//!   the art size around the template box's centre, else (a file in the legacy format another
//!   app wrote) the bounding box.
//! - Variables: the variable library, XML nodes of the `/Document` dictionary, isn't read; a note
//!   gives how many variables and data sets it has ([`VariableLibrary`]).
//! - Images: `%AI5_BeginRaster`, the colour space `XN`, `[matrix] bounds w h bits type alpha …`
//!   and the samples after `XI`. 8-bit CMYK samples keep their ink amounts in a CMYK TIFF, and
//!   an alpha channel that hides part of such an image becomes its opacity mask; other samples
//!   become an RGBA PNG.
//! - Placed (linked) files, as written in files users sent: `%AI5_BeginPlace`, `[matrix] llx lly urx
//!   ury … (path)` and the operator `` ` ``, the file's preview between `%AI26_BeginPlacedObjectPreview`
//!   and its end (`w h rowBytes 32 n `, then rows of premultiplied ARGB, the top one first), `~`. The
//!   box is as large as the file has pixels; the matrix takes them into art space from the canvas
//!   (y down, its centre the template box's), where they start at the point of art space's origin,
//!   the first row the top one. Each is a linked image showing the preview until its file is found.
//! - Art with an appearance the format can't write as plain art (several fills or strokes,
//!   effects, brushes) comes first as that look, drawn (a group, or an image where an effect
//!   rasterizes it), then the object itself on `%_`
//!   lines and `1 (style) XW`: the drawn look is kept, named after the object.
//! - Patterns: `%AI3_BeginPattern: (name)`, `(name) llx lly urx ury` (the tile, in the pattern's
//!   space, y up), the tile's art (on `%_` lines in newer files), `E`, `%AI3_EndPattern`; each is
//!   read by a reader of its own and becomes a pattern swatch. A fill or stroke with one:
//!   `(name) x y scaleX scaleY angle reflect axis shear shearAxis [matrix] p` (`P`), the matrix
//!   taking the pattern's space to art space (the other operands are what it is made of).
//!
//! - Type in the legacy format (version 7 specification §15): `type To` … `TO`, read by
//!   [`legacy_text`].
//!
//! - Swatches: the palette (`%AI5_BeginPalette`), read by [`super::palette`], replaces the
//!   default swatches.
//!
//! Sections the reader doesn't need (brushes, symbols, styles, filters) are skipped;
//! type, symbols, fills with a pattern it can't read, placed files without a preview and operators
//! it doesn't know are errors.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use kurbo::{Affine, BezPath, PathEl, Point, Rect};
use vectorcraft_color::gradient::Gradient;
use vectorcraft_color::swatch::Swatch;
use vectorcraft_color::{BlendMode, Paint};
use vectorcraft_doc::cmyk::Inks;
use vectorcraft_doc::{
    Appearance, AppearanceItem, Artboard, ColorMode, Dash, Document, FillLayer, ImageBlob, ImageObject, Knockout, LayerColor, LineCap, LineJoin,
    LinkInfo, Node, NodeId, NodeKind, OpacityMask, PatternDef, StrokeLayer, Unit,
};
use vectorcraft_geom::{FillRule, PathData};

use super::super::ate::CANVAS_CENTRE;
use super::lex::{Lexer, Tok};
use super::obj::{self, Obj, V};
use super::paint::{self, GradientDef, Instance, Named};
use crate::ps;

#[path = "legacy_text.rs"]
mod legacy_text;

/// Deepest nesting of layers, groups and dictionaries.
const MAX_DEPTH: usize = 256;
/// Most operands on the stack.
const MAX_STACK: usize = 1 << 20;
/// Most objects read.
const MAX_NODES: usize = 4_000_000;
/// Most points in one path.
const MAX_POINTS: usize = 4_000_000;
/// Largest image read (pixels).
const MAX_PIXELS: u64 = 1 << 25;
/// The note for an image past [`MAX_PIXELS`]: the file then opens from its PDF part, which reads
/// larger images (#1479).
pub const IMAGE_TOO_LARGE: &str = "an image too large to read from the editing data was left out";
/// Most hex digits of an image's samples read from comment lines (CMYK and alpha at the largest size).
const MAX_HEX: usize = (MAX_PIXELS as usize) * 10;
/// Largest side of the document and its artboards (points).
const MAX_SIDE: f64 = 1e6;

/// `%AI5_RulerUnits` → [`Unit`] (Illustrator 7 spec table 3; centimeters is 4; pixels is 6 in later files).
fn ruler_unit(v: u8) -> Option<Unit> {
    Some(match v {
        0 => Unit::Inches,
        1 => Unit::Millimeters,
        2 => Unit::Points,
        3 => Unit::Picas,
        4 => Unit::Centimeters,
        6 => Unit::Pixels,
        _ => return None,
    })
}
/// Art space (y up) → a pattern's space in the document's terms (y down).
const FLIP: Affine = Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0]);
/// Most patterns read (others can't fill art).
const MAX_PATTERNS: usize = 4096;

/// What the editing data holds: the document of its art, and where its art space is.
#[derive(Debug)]
pub struct Structure {
    /// The layers, artboards, swatches and colour mode. A text object is an empty group named
    /// [`TEXT_SLOT`] and its story's number, for the importer to fill.
    pub doc: Document,
    /// Art space (y up) → the document.
    pub to_doc: Affine,
    /// The art's bounding box in art space (`%%HiResBoundingBox`, else `%%BoundingBox`).
    pub bbox: Option<[f64; 4]>,
    /// The first artboard in art space.
    pub artboard: Option<Rect>,
    /// `%AI3_TemplateBox`, whose centre is the canvas's (the text document's origin).
    pub template: Option<[f64; 4]>,
    /// What layers that don't show have that isn't read (operators, objects): left out of them.
    pub hidden_unread: BTreeSet<String>,
    /// The names of text objects, by their slot's id.
    pub slot_names: HashMap<NodeId, String>,
    /// Where the drawn looks lay that objects read with their art style took the place of (the
    /// document's coordinates): the page draws Illustrator's rendering of those styles there.
    pub restyled: Vec<Rect>,
    pub warnings: Vec<String>,
}

/// The name of the empty group that stands for a text object, followed by the number of its
/// story in the text document when the file says.
pub(crate) const TEXT_SLOT: &str = "\u{0}text";

/// Is `name` that of a text slot, and which story is it of?
pub(crate) fn slot_of(name: Option<&str>) -> Option<Option<u32>> {
    name?.strip_prefix(TEXT_SLOT).map(|rest| rest.split(':').next().and_then(|s| s.parse().ok()))
}

/// Which of its story's frames a text slot named `name` is (0 when it doesn't say).
pub(crate) fn slot_frame(name: Option<&str>) -> usize {
    name.and_then(|n| n.strip_prefix(TEXT_SLOT)).and_then(|rest| rest.split_once(':')).and_then(|(_, f)| f.parse().ok()).unwrap_or(0)
}

/// Read the structure of the editing data `data`, applying the file's `%AI24_LargeCanvasScale` only
/// when `large_canvas` is set: an EPS's page is drawn at the full size, but a `.ai`'s PDF part may
/// keep the reduced one, which the PDF import reads as it is (#2031).
pub fn read(data: &[u8], large_canvas: bool) -> Result<Structure, String> {
    let mut r = Reader::new(data);
    r.large_canvas = large_canvas.then_some(1.0);
    r.run()?;
    r.finish()
}

/// The artboards (names, and rects in art space) of the editing data `data` that [`read`] can't
/// read as a whole, as far as it gets: where to put the pages of its PDF part instead, and what to
/// call them (#1068).
pub fn artboards(data: &[u8]) -> Vec<(String, Rect)> {
    let found = artboards_read(data);
    if !found.is_empty() {
        return found;
    }
    // Reading can stop before the artboards (art kept in the document's data comes first): read
    // their array on its own.
    artboard_array(data).map_or(found, |array| {
        let mut alone = b"%_/Document :\n%_/Dictionary :\n".to_vec();
        alone.extend_from_slice(array);
        alone.extend_from_slice(b"\n%_; /NotRecorded ,\n%_;\n");
        artboards_read(&alone)
    })
}

/// The `ArtboardArray` entry of the editing data `data`, from its `/Array :` to its key: found
/// from the key back, counting the dictionaries its artboards open and close.
fn artboard_array(data: &[u8]) -> Option<&[u8]> {
    const KEY: &[u8] = b"%_; (ArtboardArray) ,";
    let end = data.windows(KEY.len()).rposition(|w| w == KEY)? + KEY.len();
    let head = data.get(..end)?;
    let mut depth = 0usize;
    let mut line_end = head.len();
    loop {
        let line_start = head.get(..line_end)?.iter().rposition(|&b| b == b'\n' || b == b'\r').map_or(0, |at| at + 1);
        if let Some(text) = head.get(line_start..line_end)?.strip_prefix(b"%_") {
            if text.starts_with(b";") || text.starts_with(b"/Def ;") {
                depth += 1;
            } else if text.starts_with(b"/") && text.windows(2).any(|w| w == b" :") {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return head.get(line_start..);
                }
            }
        }
        line_end = line_start.checked_sub(1)?;
    }
}

/// [`artboards`] as far as reading `data` from its start gets: those read before an error, and
/// those of a `/Document` still open when it stopped (its art can be what stopped it).
fn artboards_read(data: &[u8]) -> Vec<(String, Rect)> {
    let mut r = Reader::new(data);
    let _ = r.run();
    let open: Vec<Obj> = r.frames.iter().filter_map(|f| if let Kind::Obj(o) = &f.kind { Some((**o).clone()) } else { None }).collect();
    for o in &open {
        if r.artboards.is_empty() {
            r.document(o);
        }
    }
    r.artboards
}

/// How many variables and data sets the variable library of the editing data has: XML nodes
/// (`/XMLNode`) of the `/Document` dictionary, a `variableSets` element holding `variableSet`s, each
/// with its `variables` (`variable` elements) and `sampleDataSets` (`sampleDataSet` elements).
/// VectorCraft doesn't read them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VariableLibrary {
    pub variables: usize,
    pub data_sets: usize,
}

impl VariableLibrary {
    /// The import's note that the library wasn't read, when it has a variable or a data set.
    pub fn note(self) -> Option<String> {
        let Self { variables, data_sets } = self;
        // Counts after a colon need no plural, so every language can translate the note whole.
        (variables + data_sets > 0).then(|| {
            format!("the file's variables and data sets weren't read (variables: {variables}, data sets: {data_sets}): the Variables panel doesn't list them")
        })
    }
}

/// The variable library of the editing data `data` ([`VariableLibrary`]), for a file whose layers
/// [`read`] can't read. Data without a `variableSets` element isn't read.
pub fn variables(data: &[u8]) -> VariableLibrary {
    if super::find(data, b"variableSets").is_none() {
        return VariableLibrary::default();
    }
    let mut r = Reader::new(data);
    let _ = r.run();
    // A `/Document` still open when reading stopped.
    let open: Vec<Obj> = r.frames.iter().filter_map(|f| if let Kind::Obj(o) = &f.kind { Some((**o).clone()) } else { None }).collect();
    for o in open.iter().filter(|o| o.ty == "Document") {
        variable_library(o, 0, &mut r.variables);
    }
    if r.variables != VariableLibrary::default() {
        return r.variables;
    }
    // Reading can stop before the library: read its node on its own.
    variable_sets_node(data).map_or(r.variables, |node| {
        let mut alone = b"%_/Document :\n%_/Dictionary :\n%_/Array :\n".to_vec();
        alone.extend_from_slice(node);
        alone.extend_from_slice(b"\n%_; (xmlnode-children) ,\n%_; /Recorded ,\n%_;\n");
        let mut r = Reader::new(&alone);
        let _ = r.run();
        r.variables
    })
}

/// The `variableSets` XML node of the editing data `data`, from its `/XMLNode :` line to the line
/// that closes it: found from its name back, counting the dictionaries and arrays it holds.
fn variable_sets_node(data: &[u8]) -> Option<&[u8]> {
    const NAME: &[u8] = b"variableSets) /String (xmlnode-nodename) ,";
    let at = data.windows(NAME.len()).position(|w| w == NAME)?;
    let opens = |text: &[u8]| text.starts_with(b"/") && text.windows(2).any(|w| w == b" :");
    let closes = |text: &[u8]| text.starts_with(b";") || text.starts_with(b"/Def ;");
    let mut depth = 0usize;
    let mut line_end = data.get(..at)?.iter().rposition(|&b| b == b'\n' || b == b'\r')?;
    let start = loop {
        let line_start = data.get(..line_end)?.iter().rposition(|&b| b == b'\n' || b == b'\r').map_or(0, |at| at + 1);
        if let Some(text) = data.get(line_start..line_end)?.strip_prefix(b"%_") {
            if closes(text) {
                depth += 1;
            } else if opens(text) {
                match depth.checked_sub(1) {
                    Some(d) => depth = d,
                    None => break line_start,
                }
            }
        }
        line_end = line_start.checked_sub(1)?;
    };
    let mut line_start = start;
    loop {
        let rest = data.get(line_start..)?;
        let line_end = line_start + rest.iter().position(|&b| b == b'\n' || b == b'\r').unwrap_or(rest.len());
        if let Some(text) = data.get(line_start..line_end)?.strip_prefix(b"%_") {
            if closes(text) {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return data.get(start..line_end);
                }
            } else if opens(text) {
                depth += 1;
            }
        }
        if line_end >= data.len() {
            return None;
        }
        line_start = line_end + 1;
    }
}

#[derive(Clone, Debug)]
struct GState {
    fill: Paint,
    stroke: Paint,
    width: f64,
    cap: LineCap,
    join: LineJoin,
    miter: f64,
    dash: Option<Dash>,
    rule: FillRule,
    opacity: f32,
    blend: BlendMode,
    isolate: bool,
    knockout: Knockout,
    knockout_shape: bool,
    overprint_fill: bool,
    overprint_stroke: bool,
    locked: bool,
    hidden: bool,
}

impl Default for GState {
    fn default() -> Self {
        let black = Paint::solid(vectorcraft_color::Color::BLACK);
        Self {
            fill: black.clone(),
            stroke: black,
            width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter: 10.0,
            dash: None,
            rule: FillRule::NonZero,
            opacity: 1.0,
            blend: BlendMode::Normal,
            isolate: false,
            knockout: Knockout::Off,
            knockout_shape: false,
            overprint_fill: false,
            overprint_stroke: false,
            locked: false,
            hidden: false,
        }
    }
}

/// The blend modes by their number in `Xy`.
const BLENDS: [BlendMode; 16] = [
    BlendMode::Normal,
    BlendMode::Multiply,
    BlendMode::Screen,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::ColorDodge,
    BlendMode::ColorBurn,
    BlendMode::Darken,
    BlendMode::Lighten,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
];

/// The name effects take for blend mode `n` of `Xy` (and of effects' `blnd`).
pub(super) fn blend_name(n: usize) -> String {
    format!("{:?}", blend_mode(n))
}

/// Blend mode `n` of `Xy` (Normal when it is out of range).
pub(super) fn blend_mode(n: usize) -> BlendMode {
    BLENDS.get(n).copied().unwrap_or(BlendMode::Normal)
}

#[derive(Debug, Default)]
struct LayerAttrs {
    name: String,
    visible: bool,
    preview: bool,
    locked: bool,
    printable: bool,
    dim: Option<u8>,
    color: Option<LayerColor>,
}

#[derive(Debug)]
enum Kind {
    Layer(Box<LayerAttrs>),
    Group,
    Compound,
    Clip,
    Mask,
    Obj(Box<Obj>),
}

#[derive(Debug)]
struct Frame {
    kind: Kind,
    children: Vec<Node>,
    /// Opened on a `%_` line.
    hidden: bool,
    /// [`Reader::after_object`] when it opened.
    after: bool,
    /// The state when it opened (a container's lock and visibility; a dictionary's state to go
    /// back to).
    gs: GState,
    /// The operand stack's height when it opened.
    base: usize,
    /// The clipping paths' indexes among the children.
    clips: Vec<usize>,
    /// A compound member group explicitly declares its subpaths' winding with `D`.
    compound_members: bool,
}

/// What a reader of a pattern's definition has of its head, `(name) llx lly urx ury`: the operands
/// before its first operator.
#[derive(Debug, Default)]
struct PatternHead {
    taken: bool,
    name: String,
    /// The tile in the pattern's space (y up).
    tile: Option<Rect>,
}

/// An image's operands, from `XN` and before `XI`.
#[derive(Debug, Default)]
struct Raster {
    space: Option<String>,
    /// `XI`'s operands when its samples follow as hex in comment lines (`%FF00…`) up to
    /// `%AI5_EndRaster`, the form the version 7 specification documents (§8).
    operands: Option<Vec<V>>,
    /// Those hex digits.
    hex: Vec<u8>,
}

/// A placed file's operands, from `` ` `` and its preview, until `~`.
#[derive(Debug, Default)]
struct Place {
    /// The canvas, where the file's pixels are → art space.
    matrix: Option<Affine>,
    /// The file's box (`llx lly urx ury`): its size is the file's, in pixels.
    bounds: Option<Rect>,
    path: String,
    /// The preview as a PNG, and its size in pixels.
    preview: Option<(Vec<u8>, u32, u32)>,
}

/// What the open result says of placed files: they show the preview the file keeps of them.
const LINKED_PREVIEWS: &str =
    "linked pictures show the low-resolution preview saved in the file until their files are found: relink them in the Links panel";

struct Reader<'a> {
    lex: Lexer<'a>,
    stack: Vec<V>,
    /// Where the `[` marks are on the stack, innermost last (some may have gone with operands).
    marks: Vec<usize>,
    gs: GState,
    frames: Vec<Frame>,
    path: BezPath,
    points: usize,
    /// The path is a clipping path (`W`).
    clip_next: bool,
    doc: Document,
    to_doc: Option<Affine>,
    cropmarks: Option<Rect>,
    artboards: Vec<(String, Rect)>,
    /// The variables and data sets of the `/Document` dictionary's variable library.
    variables: VariableLibrary,
    gradients: HashMap<String, Gradient>,
    gradient_def: Option<GradientDef>,
    instance: Option<Instance>,
    pending_fill: Option<Paint>,
    pending_stroke: Option<Paint>,
    /// The named colours the colour operators define (`x`, `Xx`, `Xk`…).
    spots: Vec<Named>,
    /// The named colours gradient stops name. A stop doesn't say whether its colour is an ink or
    /// a global process colour, so these make swatches only for names no colour operator defines.
    stop_spots: Vec<Named>,
    raster: Option<Raster>,
    /// The placed file being read (`%AI5_BeginPlace` … `~`).
    place: Option<Place>,
    /// The last object read is the last child of the innermost frame, and nothing started since.
    after_object: bool,
    nodes: usize,
    /// What layers that show have that this reader doesn't read (errors).
    unsupported: BTreeSet<String>,
    /// Operators the reader doesn't know on layers that show.
    unknown: BTreeSet<String>,
    /// What layers that don't show have that this reader doesn't read.
    hidden_unread: BTreeSet<String>,
    /// The header's bounding boxes and template box.
    bbox: Option<[f64; 4]>,
    hires: Option<[f64; 4]>,
    template: Option<[f64; 4]>,
    /// Colours set in each model, when the header doesn't say the document's.
    cmyk_colors: usize,
    rgb_colors: usize,
    color_model: Option<bool>,
    warnings: Vec<String>,
    /// Objects read as their drawn look (several fills or strokes, effects, brushes).
    drawn_looks: usize,
    /// Nodes read from `%_` lines.
    hidden_ids: HashSet<NodeId>,
    /// A group just ended, its state (`Xy`, `Xd`, `XW`) not yet read.
    group_ended: bool,
    /// A layer's own state (`Xy` after its art, before `LB`), until something says the art moves on.
    layer_state: Option<(f32, BlendMode, bool, Knockout)>,
    /// Painted copies of clipping paths that have a fill (drawn under what they clip).
    clip_fills: HashSet<NodeId>,
    /// Groups closed as an object's drawn look (`9 () XW`).
    looks: HashSet<NodeId>,
    /// Legacy compound member groups, even when followed by the generic `9 () XW` marker.
    compound_groups: HashSet<NodeId>,
    /// Groups of a text object and its look (see [`Self::style_marker`]), which aren't a look.
    text_looks: HashSet<NodeId>,
    /// The gradient mesh being read (`/Mesh X!` … `/End X!`), and whether it started on a `%_` line.
    mesh: Option<(super::mesh::MeshReader, bool)>,
    /// The art styles (`%AI9_BeginArtStyles`), by name.
    styles: HashMap<String, super::style::ArtStyle>,
    /// The swatches (`%AI5_BeginPalette`).
    palette: super::palette::Palette,
    /// See [`Structure::restyled`].
    restyled: Vec<Rect>,
    /// CMYK samples converted, by value.
    cmyk_cache: HashMap<[u8; 4], [u8; 3]>,
    /// A `/ForeignObject`'s `/Data`: its PDF as ASCII85 in comment lines, until the object closes.
    foreign_data: Option<Vec<u8>>,
    /// The page ended: what follows isn't art.
    done: bool,
    /// The dictionary closed last was opened on a `%_` line.
    obj_hidden: bool,
    /// The names of text objects, by their slot's id.
    slot_names: HashMap<NodeId, String>,
    /// The artboard's size and centre in files without artboards or crop marks.
    art_size: Option<(f64, f64)>,
    template_center: Option<(f64, f64)>,
    /// `%AI24_LargeCanvasScale`: a Large Canvas document's editing data is written at 1/N scale
    /// (see [`Reader::ensure_space`]); 1 (no scaling) when the file doesn't declare one, `None`
    /// when the declared factor is not applied ([`read`]).
    large_canvas: Option<f64>,
    /// `%AI5_RulerUnits` from the header (Illustrator 7 spec table 3, plus centimeters and pixels).
    ruler_units: Option<u8>,
    /// This reader reads a pattern's definition (see [`Reader::define_pattern`]).
    pattern: Option<PatternHead>,
    /// The tile (y down) of each pattern read, by name.
    pattern_tiles: HashMap<String, Rect>,
    /// The legacy text object being read (`To` … `TO`), and the modal type attributes.
    text: Option<legacy_text::LegacyText>,
    type_state: legacy_text::TypeState,
}

/// Operators that change nothing the reader keeps.
const IGNORED: &[&str] = &[
    "Ae",
    "AE",
    "Ap",
    "As",
    "Xd",
    "Xr",
    "XG",
    "Xh",
    "XH",
    "XF",
    "Bc",
    "XP",
    "Np",
    // Flatness (`i`): how finely curves print.
    "i",
    // Path resolution (`Ar`, §12): how finely curves print.
    "Ar",
    "TZ",
    "Xs",
    "Xt",
    // A brushed path's width at each anchor (`(…) Xt n XD`), on its hidden source path.
    "XD",
    "Xi",
    "XI",
    "`",
    "LB2",
    "Lc",
    "Xv",
    "XV",
    "Xq",
    "XQ",
    "Xg",
    "Xn",
    "Bn",
    "gsave",
    "grestore",
    "showpage",
    "annotatepage",
];

/// Sections skipped whole, by their markers without the `AI<version>_` prefix: begin → end.
const SKIPPED: &[(&str, &str)] = &[
    ("BeginPattern", "EndPattern"),
    ("BeginBrushPattern", "EndBrushPattern"),
    ("BeginSVGFilter", "EndSVGFilter"),
    ("BeginSymbol", "EndSymbol"),
    ("BeginPluginObject", "EndPluginObject"),
    ("BeginArtStyles", "EndArtStyles"),
    ("BeginArtStyleList", "EndArtStyleList"),
    ("BeginSymbolList", "EndSymbolList"),
    ("BeginTextDocument", "EndTextDocument"),
    ("BeginEncoding", "EndEncoding"),
    ("Alternate_Content", "End_Versioned_Content"),
];

/// The note for a `/ForeignObject` (non-native art, the content of a placed PDF, which the editor
/// shows but doesn't edit) whose PDF can't be read.
pub const NON_NATIVE_ART_UNREAD: &str = "non-native art (a placed PDF's content) that VectorCraft can't read is left out";
/// The name of the group a `/ForeignObject`'s art comes in as.
pub const NON_NATIVE_ART: &str = "Non-native art";

/// A section comment's marker without its `AI<version>_` prefix and value (`AI14_BeginSymbol` →
/// `BeginSymbol`); DSC comments (`%%BeginProlog`) keep their `%`.
fn marker(c: &str) -> &str {
    let word = c.split(|ch: char| ch == ':' || ch.is_whitespace()).next().unwrap_or(c);
    word.strip_prefix("AI").map(|r| r.trim_start_matches(|ch: char| ch.is_ascii_digit())).and_then(|r| r.strip_prefix('_')).unwrap_or(word)
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        let mut doc = Document::new(1.0, 1.0);
        doc.layers.clear();
        Self {
            lex: Lexer::new(data),
            stack: Vec::new(),
            marks: Vec::new(),
            gs: GState::default(),
            frames: Vec::new(),
            path: BezPath::new(),
            points: 0,
            clip_next: false,
            doc,
            to_doc: None,
            cropmarks: None,
            artboards: Vec::new(),
            variables: VariableLibrary::default(),
            gradients: HashMap::new(),
            gradient_def: None,
            instance: None,
            pending_fill: None,
            pending_stroke: None,
            spots: Vec::new(),
            stop_spots: Vec::new(),
            raster: None,
            place: None,
            foreign_data: None,
            after_object: false,
            nodes: 0,
            unsupported: BTreeSet::new(),
            unknown: BTreeSet::new(),
            hidden_unread: BTreeSet::new(),
            bbox: None,
            hires: None,
            template: None,
            cmyk_colors: 0,
            rgb_colors: 0,
            color_model: None,
            warnings: Vec::new(),
            drawn_looks: 0,
            hidden_ids: HashSet::new(),
            clip_fills: HashSet::new(),
            looks: HashSet::new(),
            compound_groups: HashSet::new(),
            text_looks: HashSet::new(),
            group_ended: false,
            layer_state: None,
            styles: HashMap::new(),
            palette: super::palette::Palette::default(),
            restyled: vec![],
            mesh: None,
            cmyk_cache: HashMap::new(),
            done: false,
            obj_hidden: false,
            slot_names: HashMap::new(),
            art_size: None,
            template_center: None,
            large_canvas: None,
            ruler_units: None,
            pattern: None,
            pattern_tiles: HashMap::new(),
            text: None,
            type_state: legacy_text::TypeState::default(),
        }
    }

    fn run(&mut self) -> Result<(), String> {
        while let Some(t) = self.lex.next_token() {
            if self.done {
                break;
            }
            let hidden = t.hidden;
            match t.tok {
                Tok::Num(v) => self.push(V::Num(v))?,
                Tok::Str(s) | Tok::Hex(s) => self.push(V::Str(s))?,
                Tok::Name(n) => self.push(V::Name(obj::text(n)))?,
                Tok::Word(w) => {
                    // Operators are ASCII; others are unknown words all the same.
                    let w = std::str::from_utf8(w).unwrap_or("?");
                    self.op(w, hidden)?;
                }
                Tok::Comment(c) => self.comment(c)?,
                Tok::Data(d) => self.image(d)?,
                Tok::Preview(d) => {
                    if let Some(p) = self.place.as_mut() {
                        p.preview = preview_png(d);
                    }
                }
            }
        }
        if self.frames.iter().any(|f| matches!(f.kind, Kind::Layer(_))) {
            return Err("it ends inside a layer".into());
        }
        Ok(())
    }

    fn push(&mut self, v: V) -> Result<(), String> {
        if self.stack.len() >= MAX_STACK {
            return Err("it has too many values in a row".into());
        }
        self.stack.push(v);
        Ok(())
    }

    /// The operands of the operator being run (above the innermost dictionary's), taken off.
    fn operands(&mut self) -> Vec<V> {
        let base = self.frames.last().map_or(0, |f| f.base).min(self.stack.len());
        self.stack.split_off(base)
    }

    fn nums(vals: &[V]) -> Vec<f64> {
        vals.iter().filter_map(V::num).collect()
    }

    // ---- comments --------------------------------------------------------------------------

    fn comment(&mut self, c: &[u8]) -> Result<(), String> {
        let line = String::from_utf8_lossy(c);
        let line = line.trim_end();
        if let Some(r) = self.raster.as_mut()
            && r.operands.is_some()
            && !line.is_empty()
            && line.bytes().all(|b| b.is_ascii_hexdigit())
        {
            if r.hex.len() < MAX_HEX {
                r.hex.extend_from_slice(line.as_bytes());
            }
            return Ok(());
        }
        let m = marker(line);
        // A pattern's definition (one inside another's isn't read).
        if m == "BeginPattern" && self.pattern.is_none() {
            let section = self.section("EndPattern")?;
            self.define_pattern(section);
            return Ok(());
        }
        if m == "BeginArtStyles" {
            let section = self.section("EndArtStyles")?;
            self.styles = super::style::art_styles(section);
            return Ok(());
        }
        if m == "BeginPalette" {
            let section = self.section("EndPalette")?;
            self.palette = super::palette::palette(section);
            return Ok(());
        }
        if m == "BeginEncoding" {
            self.type_state.encoding_comment(line);
        }
        if let Some((_, end)) = SKIPPED.iter().find(|(b, _)| *b == m) {
            return self.skip_to(end);
        }
        match m {
            "%BeginProlog" => return self.skip_to("%EndProlog"),
            "%BeginResource" => return self.skip_to("%EndResource"),
            // The art ends with the page.
            "%PageTrailer" | "%Trailer" => {
                self.done = true;
                return Ok(());
            }
            _ => {}
        }
        if line.starts_with("AI5_BeginLayer") {
            return self.open(Kind::Layer(Box::default()), false);
        }
        if line.starts_with("AI5_BeginRaster") {
            self.raster = Some(Raster::default());
        } else if line.starts_with("AI5_EndRaster") {
            if let Some(Raster { operands: Some(vals), hex, .. }) = self.raster.take() {
                return self.image_from(vals, &hex);
            }
        } else if line.starts_with("AI5_BeginPlace") {
            self.place = Some(Place::default());
        } else if line.starts_with("AI5_EndPlace") {
            // One that `~` didn't end isn't read.
            if self.place.take().is_some() {
                self.unreadable("placed files");
            }
        } else if self.frames.is_empty()
            && let Some(v) = line.strip_prefix("%HiResBoundingBox:")
        {
            self.hires = four(v).or(self.hires);
        } else if self.frames.is_empty()
            && let Some(v) = line.strip_prefix("%BoundingBox:")
        {
            self.bbox = four(v).or(self.bbox);
        } else if let Some(v) = line.strip_prefix("AI3_Cropmarks:") {
            let n: Vec<f64> = v.split_whitespace().filter_map(|w| w.parse().ok()).collect();
            if let [x0, y0, x1, y1] = n.as_slice() {
                let r = Rect::new(*x0, *y0, *x1, *y1);
                if sane(r) {
                    self.cropmarks = Some(r);
                }
            }
        } else if self.frames.is_empty()
            && let Some(v) = line.strip_prefix("AI5_RulerUnits:")
            && let Ok(n) = v.trim().parse::<u8>()
        {
            self.ruler_units = Some(n);
        } else if let Some(v) = line.strip_prefix("AI5_ArtSize:") {
            self.art_size = two(v);
        } else if let Some(v) = line.strip_prefix("AI3_TemplateBox:") {
            self.template_center = two(v);
            self.template = four(v).or(self.template);
        } else if let Some(v) = line.strip_prefix("AI24_LargeCanvasScale:") {
            // Illustrator's Large Canvas writes the editing data (art and boxes) at 1/N scale, N
            // being this; the printed page applies the same factor, so the art reaches the page's
            // size once it is applied to the art space ([`Self::ensure_space`]). A value outside
            // the range Illustrator writes is left alone.
            if let (Some(scale), Ok(n)) = (&mut self.large_canvas, v.trim().parse::<f64>())
                && (1.0..=100.0).contains(&n)
            {
                *scale = n;
            }
        } else if let Some(v) = line.strip_prefix("AI9_ColorModel:") {
            self.color_model = Some(v.trim() == "2");
        }
        Ok(())
    }

    /// Skip to the comment with marker `end` (sections of the same kind nested in it count, and
    /// versioned content in an alternate).
    fn skip_to(&mut self, end: &str) -> Result<(), String> {
        self.section(end).map(|_| ())
    }

    /// [`Self::skip_to`] → what was skipped, up to the comment that ends it.
    fn section(&mut self, end: &str) -> Result<&'a [u8], String> {
        let begin = SKIPPED.iter().find(|(_, e)| *e == end).map_or("", |(b, _)| *b);
        let begin = if end == "End_Versioned_Content" { "Begin_Content_if_version_gt" } else { begin };
        let start = self.lex.offset();
        let mut depth = 0usize;
        loop {
            let before = self.lex.offset();
            let Some(t) = self.lex.next_token() else { break };
            if let Tok::Comment(c) = t.tok {
                let line = String::from_utf8_lossy(c);
                let m = marker(&line);
                if !begin.is_empty() && m == begin {
                    depth += 1;
                } else if m == end {
                    if depth == 0 {
                        return Ok(self.lex.slice(start, before));
                    }
                    depth -= 1;
                }
            }
        }
        Err(format!("its section ending with `%{end}` doesn't end"))
    }

    /// A pattern's definition (`section`: what follows `%AI3_BeginPattern`), read by a reader of its
    /// own into a pattern swatch, its art in the pattern's space (y down). A definition it can't
    /// read whole (an operator it doesn't know, type, unbalanced groups, no tile) isn't kept: art
    /// filled with it can't be read.
    fn define_pattern(&mut self, section: &[u8]) {
        if self.pattern_tiles.len() >= MAX_PATTERNS {
            return;
        }
        let mut sub = Reader::new(section);
        sub.pattern = Some(PatternHead::default());
        // The gradients defined so far, lent (and those it defines kept).
        sub.gradients = std::mem::take(&mut self.gradients);
        sub.nodes = self.nodes;
        sub.template = self.template;
        let read = sub.open(Kind::Group, false).and_then(|()| sub.run());
        self.nodes = sub.nodes;
        self.gradients = std::mem::take(&mut sub.gradients);
        let whole = read.is_ok() && sub.unsupported.is_empty() && sub.unknown.is_empty() && sub.frames.len() == 1;
        let (Some(head), Some(frame)) = (sub.pattern.take(), sub.frames.pop()) else { return };
        let Some(tile) = head.tile.filter(|t| whole && sane(*t) && t.width() > 0.0 && t.height() > 0.0 && !frame.children.is_empty()) else {
            return;
        };
        let art: Vec<Arc<Node>> = frame.children.iter().map(|n| Arc::new(self.doc.reid(n))).collect();
        let mut def = PatternDef::new(&head.name, art);
        def.tile = FLIP.transform_rect_bbox(tile);
        if self.pattern_tiles.insert(def.name.clone(), def.tile).is_some() {
            self.doc.patterns.retain(|p| p.name != def.name);
        }
        self.doc.patterns.push(def);
        for (key, blob) in sub.doc.images {
            self.doc.images.entry(key).or_insert(blob);
        }
        self.spots.extend(sub.spots);
        self.stop_spots.extend(sub.stop_spots);
        self.cmyk_colors += sub.cmyk_colors;
        self.rgb_colors += sub.rgb_colors;
        for w in &sub.warnings {
            self.warn(w);
        }
    }

    /// In a pattern's definition, the operands of its first operator start with its head,
    /// `(name) llx lly urx ury`: taken off them.
    fn take_head(&mut self, mut vals: Vec<V>) -> Vec<V> {
        let Some(head) = self.pattern.as_mut().filter(|h| !h.taken) else { return vals };
        head.taken = true;
        let Some(at) = vals.iter().position(|v| matches!(v, V::Str(_))) else { return vals };
        let tile = Self::nums(vals.get(at + 1..at + 5).unwrap_or_default());
        let ([x0, y0, x1, y1], Some(name)) = (tile.as_slice(), vals.get(at).and_then(V::text)) else { return vals };
        head.name = name;
        head.tile = Some(Rect::new(*x0, *y0, *x1, *y1));
        vals.split_off(at + 5)
    }

    // ---- frames ----------------------------------------------------------------------------

    fn open(&mut self, kind: Kind, hidden: bool) -> Result<(), String> {
        if self.frames.len() >= MAX_DEPTH {
            return Err("its objects are nested too deeply".into());
        }
        if matches!(kind, Kind::Layer(_)) {
            self.ensure_space();
        }
        let base = if matches!(kind, Kind::Obj(_)) { self.stack.len() } else { self.frames.last().map_or(0, |f| f.base) };
        self.frames.push(Frame {
            kind,
            children: Vec::new(),
            hidden,
            after: self.after_object,
            gs: self.gs.clone(),
            base,
            clips: Vec::new(),
            compound_members: false,
        });
        self.after_object = false;
        Ok(())
    }

    /// Close the innermost frame, which must be of the kind `is` accepts.
    fn close(&mut self, what: &str, is: impl Fn(&Kind) -> bool) -> Result<Frame, String> {
        match self.frames.last() {
            Some(f) if is(&f.kind) => self.frames.pop().ok_or_else(|| format!("`{what}` closes nothing")),
            _ => Err(format!("`{what}` closes something it didn't open")),
        }
    }

    /// Add a node to the innermost container.
    fn add(&mut self, node: Node, hidden: bool) -> Result<(), String> {
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err(format!("it has more than {MAX_NODES} objects"));
        }
        if hidden {
            self.hidden_ids.insert(node.id);
        }
        match self.frames.last_mut() {
            Some(f) => f.children.push(node),
            None => return Err("it has art outside its layers".into()),
        }
        self.after_object = true;
        Ok(())
    }

    /// The last object read, while nothing else started.
    fn last_object(&mut self) -> Option<&mut Node> {
        if !self.after_object {
            return None;
        }
        self.frames.last_mut()?.children.last_mut()
    }

    fn new_node(&mut self, kind: NodeKind, gs: &GState) -> Node {
        let mut n = Node::new(self.doc.alloc_id(), kind);
        n.visible = !gs.hidden;
        n.locked = gs.locked;
        n
    }

    fn end_container(&mut self, what: &str, hidden: bool) -> Result<(), String> {
        let f = match what {
            "U" => self.close(what, |k| matches!(k, Kind::Group))?,
            "*U" => self.close(what, |k| matches!(k, Kind::Compound))?,
            _ => self.close(what, |k| matches!(k, Kind::Clip))?,
        };
        let Frame { kind, children: kids, gs, clips, hidden: opened_hidden, compound_members, .. } = f;
        // A group holding only clipping paths (`q … u <path> W n U … Q`) doesn't clip on its own: `u`/`U` don't save
        // the graphics state as `q`/`Q` do, so its paths clip the clipping group around it, as the file's PDF part draws it.
        if matches!(kind, Kind::Group)
            && !clips.is_empty()
            && clips.len() == kids.len()
            && matches!(self.frames.last(), Some(Frame { kind: Kind::Clip, .. }))
        {
            for kid in kids {
                if let Some(parent) = self.frames.last_mut() {
                    parent.clips.push(parent.children.len());
                }
                self.add(kid, opened_hidden || hidden)?;
            }
            return Ok(());
        }
        // A compound path of clipping paths clips its clipping group.
        let clipping = matches!(kind, Kind::Compound) && !clips.is_empty();
        self.group_ended = !matches!(kind, Kind::Compound);
        let mut painted = None;
        let node = match kind {
            // A compound path holding a group (its drawn look) is that look: its paths paint apart.
            // Some files mark every group so, the plain one holding the subpaths too (#980): one
            // paint for all its paths keeps it one compound path, its holes cut.
            Kind::Compound if kids.iter().any(|k| self.looks.contains(&k.id) && paints_apart(k)) => {
                let (children, clip) = self.clip_first(kids, &clips);
                self.new_node(NodeKind::Group { children, clip }, &gs)
            }
            Kind::Compound => {
                // A group in a compound path (`*u u … U *U`) only holds its paths. Clipping subpaths'
                // painted copies make a compound of their own, under what it clips.
                let (fills, kids): (Vec<Node>, Vec<Node>) = ungrouped(kids).into_iter().partition(|n| self.clip_fills.contains(&n.id));
                painted = self.compound(fills, &gs);
                let Some(n) = self.compound(kids, &gs) else { return Ok(()) };
                n
            }
            // A group with a clipping path (`q` … `Q`, or a `W` path in a group) is a clipping group.
            _ => {
                let (children, clip) = self.clip_first(kids, &clips);
                self.new_node(NodeKind::Group { children, clip }, &gs)
            }
        };
        if clipping && let Some(f) = self.frames.last_mut() {
            f.clips.push(f.children.len());
        }
        if compound_members {
            self.compound_groups.insert(node.id);
        }
        self.add(node, opened_hidden || hidden)?;
        if let Some(p) = painted {
            self.clip_fills.insert(p.id);
            self.add(p, opened_hidden || hidden)?;
        }
        Ok(())
    }

    /// The compound path of `kids`, painted and made transparent as its subpaths say.
    fn compound(&mut self, mut kids: Vec<Node>, gs: &GState) -> Option<Node> {
        // The compound path's paint is its last path's (a gradient's instance wraps only
        // that one); the last painted one, should the last have none.
        let shown = kids.iter().rev().find(|k| !k.appearance.items.is_empty()).or(kids.last())?;
        let (appearance, opacity, blend, rule) = (shown.appearance.clone(), shown.opacity, shown.blend, path_rule(shown));
        for k in &mut kids {
            k.opacity = 1.0;
            k.blend = BlendMode::Normal;
            k.visible = true;
            k.locked = false;
        }
        let mut n = self.new_node(NodeKind::Compound { children: kids.into_iter().map(Arc::new).collect(), rule }, gs);
        n.appearance = appearance;
        n.opacity = opacity;
        n.blend = blend;
        Some(n)
    }

    /// `kids` with the clipping paths among them (indexes `clips`) first, several as one compound
    /// path → the children and whether they clip.
    fn clip_first(&mut self, kids: Vec<Node>, clips: &[usize]) -> (Vec<Arc<Node>>, bool) {
        let (mut clip, rest): (Vec<Indexed>, Vec<Indexed>) = kids.into_iter().enumerate().partition(|(i, _)| clips.contains(i));
        let clip_node = match clip.len() {
            0 => None,
            1 => clip.pop().map(|(_, n)| n),
            _ => {
                let rule = clip.first().map_or(FillRule::NonZero, |(_, n)| path_rule(n));
                let children = clip.into_iter().map(|(_, n)| Arc::new(n)).collect();
                Some(Node::new(self.doc.alloc_id(), NodeKind::Compound { children, rule }))
            }
        };
        let clips = clip_node.is_some();
        // A clipping path's own fill paints under the art it clips.
        let (fills, rest): (Vec<Indexed>, Vec<Indexed>) = rest.into_iter().partition(|(_, n)| self.clip_fills.contains(&n.id));
        let children = clip_node.into_iter().chain(fills.into_iter().chain(rest).map(|(_, n)| n)).map(Arc::new).collect();
        (children, clips)
    }

    fn end_layer(&mut self) -> Result<(), String> {
        let f = self.close("LB", |k| matches!(k, Kind::Layer(_)))?;
        let Kind::Layer(a) = f.kind else { return Ok(()) };
        let (children, clip) = self.clip_first(f.children, &f.clips);
        let color = a.color.unwrap_or(LayerColor::Preset(0));
        let mut layer = Node::new(
            self.doc.alloc_id(),
            NodeKind::Layer { color, template: false, printable: a.printable, children, clip, preview: a.preview, dim_images: a.dim },
        );
        layer.name = Some(a.name.clone());
        layer.visible = a.visible;
        layer.locked = a.locked;
        // The layer's own opacity, blend and knockout, written after its art (#2003).
        if let Some((opacity, blend, isolate, knockout)) = self.layer_state.take() {
            (layer.opacity, layer.blend, layer.isolate, layer.knockout) = (opacity, blend, isolate, knockout);
        }
        match self.frames.last_mut() {
            Some(parent) => {
                parent.children.push(layer);
                self.after_object = true;
            }
            None => {
                self.doc.layers.push(Arc::new(layer));
                self.after_object = false;
            }
        }
        Ok(())
    }

    // ---- operators -------------------------------------------------------------------------

    fn op(&mut self, w: &str, hidden: bool) -> Result<(), String> {
        match w {
            "[" => {
                self.marks.push(self.stack.len());
                return self.push(V::Mark);
            }
            "]" => {
                // The innermost `[` still on the stack (the stack is cut back by operators).
                let mut at = self.stack.len();
                while let Some(m) = self.marks.pop() {
                    if matches!(self.stack.get(m), Some(V::Mark)) {
                        at = m;
                        break;
                    }
                }
                let items: Vec<V> = self.stack.split_off(at).into_iter().skip(1).collect();
                return self.push(V::Arr(items));
            }
            ":" => return self.begin_obj(hidden),
            "," => {
                if let Some(Frame { kind: Kind::Obj(o), base, .. }) = self.frames.last_mut() {
                    obj::add_entry(o, &mut self.stack, *base);
                    // Binary data in ASCII85 follows, up to its `~>`: a `/Binary` dictionary's
                    // (`/ASCII85Decode ,`) or a non-native art object's (`/ForeignObject`'s
                    // `/Data ,`, the PDF it keeps), in comment lines. ASCII85 has `_`, so a line
                    // of it may start with `%_` and read as hidden tokens: a `(` there would open
                    // a string that swallows the rest of the layer.
                    let key = o.entries.last().and_then(|(k, _)| k.as_deref());
                    if o.ty == "Binary" && key == Some("ASCII85Decode") {
                        self.lex.skip_past(b"~>");
                    } else if o.ty == "ForeignObject" && key == Some("Data") {
                        self.foreign_data = Some(self.lex.skip_past(b"~>").to_vec());
                    }
                }
                return Ok(());
            }
            ";" => return self.end_obj(),
            "X=" => {
                if !matches!(self.frames.last(), Some(Frame { kind: Kind::Obj(o), .. }) if o.ty == "Mask") {
                    // `X=` also introduces legacy text frame art, read by `text_frames`.
                    return Ok(());
                }
                self.end_path_quietly();
                return self.open(Kind::Mask, hidden);
            }
            "X+" if self.frames.iter().any(|f| matches!(f.kind, Kind::Mask)) => return self.end_mask(),
            "X+" => return Ok(()),
            _ => {}
        }
        // What follows an object (its name, transparency and style) ends with anything else. The
        // state block of a layer whose art ended (`Xy`, `Xd`, `XW`, then `LB`) is read at the `LB`.
        if !matches!(w, "Xy" | "Xd" | "XW" | "BB" | "LB") {
            self.after_object = false;
            self.layer_state = None;
        }
        let group_ended = std::mem::take(&mut self.group_ended);
        let vals = self.operands();
        let vals = self.take_head(vals);
        if let Some(d) = self.gradient_def.as_mut() {
            match w {
                "Bs" => d.stop(&vals, &mut self.stop_spots),
                "BD" => {
                    if let Some((name, g)) = self.gradient_def.take().and_then(GradientDef::finish) {
                        self.gradients.insert(name, g);
                    }
                }
                _ => {}
            }
            return Ok(());
        }
        if w == "X!" || self.mesh.is_some() {
            let name = vals.iter().rev().find_map(|v| if let V::Name(n) = v { Some(n.as_str()) } else { None });
            match (w, name) {
                ("X!", Some("Mesh")) => {
                    self.end_path_quietly();
                    self.mesh = Some((super::mesh::MeshReader::default(), hidden));
                    return Ok(());
                }
                ("X!", Some("End")) if self.mesh.is_some() => return self.end_mesh(),
                ("X#", _) => {
                    let to_doc = self.space();
                    if let Some((m, _)) = self.mesh.as_mut() {
                        m.entry(&vals, to_doc);
                    }
                    return Ok(());
                }
                _ if self.mesh.is_some() => return Ok(()),
                _ => {}
            }
        }
        match w {
            // Containers.
            "u" => {
                self.end_path_quietly();
                self.open(Kind::Group, hidden)?
            }
            "*u" => {
                self.end_path_quietly();
                self.open(Kind::Compound, hidden)?
            }
            "q" => {
                self.end_path_quietly();
                self.open(Kind::Clip, hidden)?
            }
            "U" | "*U" | "Q" => self.end_container(w, hidden)?,
            "Lb" => self.layer_attrs(&vals),
            "Ln" => {
                if let Some(Frame { kind: Kind::Layer(a), .. }) = self.frames.last_mut() {
                    a.name = vals.iter().rev().find_map(V::text).unwrap_or_default();
                }
            }
            "LB" => self.end_layer()?,
            // Paths.
            "m" | "l" | "L" | "c" | "C" | "v" | "V" | "y" | "Y" => self.path_op(w, &vals)?,
            "h" => close(&mut self.path),
            // Ends a path that stays open.
            "H" => {}
            "W" => self.clip_next = true,
            // A text object's frame or path.
            "N" | "n" | "F" | "f" | "S" | "s" | "B" | "b" if self.in_text_path() => self.text_path(),
            "N" | "n" | "F" | "f" | "S" | "s" | "B" | "b" => self.paint(w, hidden, false)?,
            "*" => {
                let op = vals.iter().rev().find_map(V::text).unwrap_or_else(|| "N".into());
                self.paint(&op, hidden, true)?;
            }
            // Colours.
            "g" | "G" | "k" | "K" | "x" | "X" | "Xa" | "XA" | "Xx" | "XX" | "Xk" | "XK" | "Xs" | "XS" | "Xz" | "XZ" => self.color(w, &vals),
            "p" | "P" => self.pattern_paint(w, &vals),
            // A pattern's definition ends.
            "E" if self.pattern.is_some() => self.done = true,
            "O" => self.gs.overprint_fill = Self::nums(&vals).last() == Some(&1.0),
            "R" => self.gs.overprint_stroke = Self::nums(&vals).last() == Some(&1.0),
            "XR" => self.gs.rule = if Self::nums(&vals).last() == Some(&1.0) { FillRule::EvenOdd } else { FillRule::NonZero },
            "w" => {
                if let Some(v) = Self::nums(&vals).last().filter(|v| v.is_finite() && **v >= 0.0) {
                    self.gs.width = *v;
                }
            }
            "J" => {
                self.gs.cap = match Self::nums(&vals).last().map(|v| *v as i64) {
                    Some(1) => LineCap::Round,
                    Some(2) => LineCap::Square,
                    _ => LineCap::Butt,
                }
            }
            "j" => {
                self.gs.join = match Self::nums(&vals).last().map(|v| *v as i64) {
                    Some(1) => LineJoin::Round,
                    Some(2) => LineJoin::Bevel,
                    _ => LineJoin::Miter,
                }
            }
            "M" => {
                if let Some(v) = Self::nums(&vals).last().filter(|v| v.is_finite() && **v >= 1.0) {
                    self.gs.miter = v.min(500.0);
                }
            }
            "d" => self.dash(&vals),
            "D" => {
                // The published Adobe Illustrator File Format Specification (version 3.0), §6.8,
                // permits `u` groups of compound members.
                // Their explicit winding declarations describe subpaths, not separate paints;
                // modern streams can also end these groups with `9 () XW`.
                let n = self.frames.len();
                if n >= 2
                    && self.frames.get(n - 2).is_some_and(|f| matches!(f.kind, Kind::Compound))
                    && let Some(f) = self.frames.last_mut().filter(|f| matches!(f.kind, Kind::Group))
                {
                    f.compound_members = true;
                }
            }
            // Transparency and state.
            "Xy" => self.transparency(&vals),
            // A group's own state (`Xd`, `6 () XW`) without an `Xy` keeps the current transparency, as
            // the file's PDF part draws it. A look's group (`9 () XW`) has no state of its own.
            "Xd" if group_ended => {
                let gs = self.gs.clone();
                if let Some(n) = self.last_object().filter(|n| matches!(n.kind, NodeKind::Group { .. })) {
                    (n.opacity, n.blend, n.isolate, n.knockout) = (gs.opacity, gs.blend, gs.isolate, gs.knockout);
                }
            }
            "Xw" => self.gs.hidden = Self::nums(&vals).last() == Some(&1.0),
            "A" => self.gs.locked = Self::nums(&vals).last() == Some(&1.0),
            "XW" => self.style_marker(&vals, hidden),
            // Gradients.
            "Bd" => self.gradient_def = GradientDef::begin(&vals),
            "Bs" | "BD" => {}
            "Bb" => self.instance = Some(Instance::begin(&vals)),
            "Bg" => {
                if let Some(i) = self.instance.as_mut() {
                    i.bg(&vals);
                }
            }
            "Bm" => {
                if let Some(i) = self.instance.as_mut() {
                    i.bm(&vals);
                }
            }
            "Xm" => {
                if let Some(i) = self.instance.as_mut() {
                    i.xm(&vals);
                }
            }
            "Bh" => {
                if let Some(i) = self.instance.as_mut() {
                    i.bh(&vals);
                }
            }
            "BB" => {
                // An instance that closes with no path to paint is an object's own note of its
                // gradient, written after it: it paints nothing (not the next path either).
                if let Some(i) = self.instance.take()
                    && !self.path.elements().is_empty()
                {
                    self.set_instance(&i);
                }
                // `1 BB` strokes the path the gradient just filled, `2 BB` closes it first (§3.5.3).
                if matches!(Self::nums(&vals).last(), Some(f) if *f == 1.0 || *f == 2.0) {
                    self.stroke_last();
                }
            }
            // Images.
            "XN" => {
                if let Some(r) = self.raster.as_mut() {
                    r.space = vals.iter().rev().find_map(V::text);
                }
            }
            // Placed files.
            "`" if self.place.is_some() => {
                let matrix = vals.iter().find_map(|v| if let V::Arr(a) = v { Some(Self::nums(a)) } else { None });
                let path = vals.iter().rev().find_map(V::text).unwrap_or_default();
                if let Some(p) = self.place.as_mut() {
                    p.matrix = matrix.and_then(|m| <[f64; 6]>::try_from(m).ok()).map(Affine::new);
                    p.bounds =
                        Self::nums(&vals).get(..4).and_then(|b| <[f64; 4]>::try_from(b).ok()).map(|[x0, y0, x1, y1]| Rect::new(x0, y0, x1, y1));
                    p.path = path;
                }
            }
            "~" if self.place.is_some() => self.end_place(hidden)?,
            // Type in the legacy format.
            "To" => self.begin_text(&vals, hidden),
            "Tp" => self.begin_text_path(&vals),
            "TP" => self.end_text_path(),
            "TO" => self.end_text(hidden)?,
            "Tx" | "Tj" | "TX" => self.text_body(w, &vals),
            "Tr" | "Tf" | "Ts" | "Tz" | "Tt" | "TA" | "Tl" | "Ta" | "Ti" => self.text_attr(w, &vals),
            // Attributes the reader doesn't keep (kerning pairs, spacing, hanging punctuation,
            // Far-Eastern type) and final-form positions (§15.3).
            "Tk" | "TK" | "TC" | "TW" | "Tq" | "Tc" | "Tw" | "Tm" | "Td" | "T*" | "TR" | "T+" | "T-" | "TV" | "Tv" | "Ty" | "TY" | "TG" | "Tg"
            | "Tu" | "Xu" => {}
            // Hyphenation and tab-ruler attributes the reader doesn't keep either (§22): `Th`
            // hyphen control, `XL` hyphenation language, `Xb`/`XB` a tab definition (and the `Xe`
            // tab descriptions between them).
            "Th" | "XL" | "Xb" | "XB" | "Xe" => {}
            "TE" => self.te(&vals),
            // An image whose samples follow in comment lines (read at `%AI5_EndRaster`).
            "XI" if self.raster.is_some() => {
                if let Some(r) = self.raster.as_mut() {
                    r.operands = Some(vals);
                }
            }
            _ if IGNORED.contains(&w) => {}
            // Before the art, the setup runs procedures of the printing format.
            _ if self.frames.is_empty() => {}
            _ => {
                let op: String = w.chars().take(24).collect();
                if !self.shown() {
                    if self.hidden_unread.len() < 16 {
                        self.hidden_unread.insert(format!("`{op}`"));
                    }
                } else if self.unknown.len() < 16 {
                    self.unknown.insert(op);
                }
            }
        }
        Ok(())
    }

    /// `:` after `/Type`: a dictionary opens.
    fn begin_obj(&mut self, hidden: bool) -> Result<(), String> {
        let ty = match self.stack.pop() {
            Some(V::Name(n)) => n,
            Some(other) => {
                self.stack.push(other);
                String::new()
            }
            None => String::new(),
        };
        self.open(Kind::Obj(Box::new(Obj { ty, ..Obj::default() })), hidden)?;
        // A dictionary's state starts afresh (styles set theirs inside).
        self.gs = GState::default();
        Ok(())
    }

    /// Capture mask art in document coordinates, without adding it to the owner's siblings.
    fn end_mask(&mut self) -> Result<(), String> {
        self.end_path_quietly();
        let f = self.close("X+", |k| matches!(k, Kind::Mask))?;
        let (children, clip) = self.clip_first(f.children, &f.clips);
        let art = self.new_node(NodeKind::Group { children, clip }, &f.gs);
        self.gs = f.gs;
        self.after_object = f.after;
        let Some(Frame { kind: Kind::Obj(o), .. }) = self.frames.last_mut() else {
            return Err("mask art ends outside a mask dictionary".into());
        };
        if o.art.replace(Arc::new(art)).is_some() {
            return Err("a mask has more than one art block".into());
        }
        Ok(())
    }

    /// `;`: the innermost dictionary closes.
    fn end_obj(&mut self) -> Result<(), String> {
        if !matches!(self.frames.last(), Some(Frame { kind: Kind::Obj(_), .. })) {
            return Ok(());
        }
        let Some(f) = self.frames.pop() else { return Ok(()) };
        let Frame { kind, children, base, hidden, gs, after, .. } = f;
        let Kind::Obj(mut o) = kind else { return Ok(()) };
        obj::close(&mut o, &mut self.stack, base);
        self.obj_hidden = hidden;
        // Art made inside a dictionary (type's paths) isn't kept.
        self.gs = gs;
        self.after_object = after;
        if matches!(self.frames.last(), Some(Frame { kind: Kind::Obj(_), .. })) {
            return self.push(V::Obj(Rc::new(*o)));
        }
        match o.ty.as_str() {
            "ArtDictionary" => {
                let envelope = o.get("AIDeformOptionsKey").is_some();
                self.art_dictionary(&o);
                if envelope {
                    let mut slots = vec![];
                    for child in &children {
                        envelope_text_slots(child, &mut slots);
                    }
                    for mut slot in slots {
                        // The editing data keeps the source text inside the envelope's options;
                        // its visible result is the outlined art the PDF page carries.
                        slot.visible = false;
                        self.add(slot, false)?;
                    }
                }
            }
            "Document" => self.document(&o),
            "AI11Text" => self.text_slot(&o)?,
            "SymbolInstance" => self.unreadable("symbols"),
            "ForeignObject" => self.foreign_art(&o)?,
            _ => {}
        }
        Ok(())
    }

    /// A text object (`/AI11Text`): an empty group standing for it, named after its story.
    fn text_slot(&mut self, o: &Obj) -> Result<(), String> {
        if self.pattern.is_some() {
            self.unreadable("type in a pattern");
        }
        // `1 /FreeUndo`: the type in the drawn look of a text object that has an appearance. Its
        // story is one of another text document (`/AI11UndoFreeTextDocument`), which counts its
        // stories on its own; the object itself follows the look, on `%_` lines, and is the type.
        if o.nums("FreeUndo").first() == Some(&1.0) {
            return Ok(());
        }
        let index = |key: &str| o.nums(key).first().copied().filter(|v| (0.0..f64::from(u32::MAX)).contains(v)).map(|v| v as u32);
        let (story, frame) = (index("StoryIndex"), index("FrameIndex").unwrap_or(0));
        let gs = self.gs.clone();
        let mut n = self.new_node(NodeKind::Group { children: vec![], clip: false }, &gs);
        n.name = Some(story.map_or_else(|| TEXT_SLOT.to_string(), |s| format!("{TEXT_SLOT}{s}:{frame}")));
        // The type takes the state's transparency, as a path does; an `Xy` after it sets the slot's own.
        n.opacity = gs.opacity;
        n.blend = gs.blend;
        let hidden = self.obj_hidden;
        self.add(n, hidden)
    }

    /// Non-native art (`/ForeignObject`): the content of a placed PDF, which Illustrator shows but
    /// doesn't edit. Its `/Data` is that PDF (ASCII85 in comment lines, kept by the `,` handler),
    /// `/Bounds` the art's box in a space measured from `/Origin` with y down, and `/RTransform`
    /// maps that space onto the art (both read off files: the box lands on the clipping paths
    /// around it). The PDF importer reads the PDF, and its art is fitted to the box, as one
    /// group. Without data, or with a PDF that can't be read, nothing is drawn.
    fn foreign_art(&mut self, o: &Obj) -> Result<(), String> {
        let Some(text) = self.foreign_data.take() else { return Ok(()) };
        // One `%` starts each line; the rest, `_` included, is data.
        let mut encoded = String::with_capacity(text.len());
        for line in text.split(|b| matches!(b, b'\r' | b'\n')) {
            encoded.push_str(&String::from_utf8_lossy(line.strip_prefix(b"%").unwrap_or(line)));
        }
        let Some(pdf) = ps::ascii85_decode(&encoded) else {
            self.warn(NON_NATIVE_ART_UNREAD);
            return Ok(());
        };
        let (rt, origin, bounds) = (o.get("RTransform").map(Self::nums), o.get("Origin").map(Self::nums), o.get("Bounds").map(Self::nums));
        let (Some([a, b, c, d, tx, ty]), Some([ox, oy]), Some([x0, y0, x1, y1])) = (rt.as_deref(), origin.as_deref(), bounds.as_deref()) else {
            self.warn(NON_NATIVE_ART_UNREAD);
            return Ok(());
        };
        let place = Affine::new([*a, *b, *c, *d, *tx, *ty]) * Affine::new([1.0, 0.0, 0.0, -1.0, *ox, *oy]);
        let art_box = place.transform_rect_bbox(Rect::new(*x0, *y0, *x1, *y1));
        if !sane(art_box) || art_box.width() <= 0.0 || art_box.height() <= 0.0 {
            self.warn(NON_NATIVE_ART_UNREAD);
            return Ok(());
        }
        let target = self.space().transform_rect_bbox(art_box);
        let opts = vectorcraft_pdf::ImportOptions { layers: false, ..Default::default() };
        let read = match vectorcraft_pdf::import_with_report(&pdf, &opts) {
            Ok(r) => r,
            Err(e) => {
                self.warn(&format!("{NON_NATIVE_ART_UNREAD} ({e})"));
                return Ok(());
            }
        };
        let mut art: Vec<Node> = read.document.layers.iter().filter_map(|l| l.children()).flatten().map(|n| n.as_ref().clone()).collect();
        let Some(bbox) = art.iter().fold(None, |acc, n| vectorcraft_geom::union_opt(acc, n.geometric_bounds())) else { return Ok(()) };
        if bbox.width() <= 0.0 || bbox.height() <= 0.0 {
            return Ok(());
        }
        // The art's own box onto the object's box (its measure of the same art). The PDF importer
        // hands the art back as the page shows it, and it keeps that orientation unless its
        // placement mirrors it (a negative determinant): the usual `/RTransform` `1 0 0 -1 …` with
        // the origin's flip doesn't, and the art then shows as the file's own PDF part draws it.
        let fit = if place.determinant() < 0.0 {
            Affine::translate((target.x0, target.y1)) * Affine::scale_non_uniform(target.width() / bbox.width(), -target.height() / bbox.height())
        } else {
            Affine::translate((target.x0, target.y0)) * Affine::scale_non_uniform(target.width() / bbox.width(), target.height() / bbox.height())
        } * Affine::translate((-bbox.x0, -bbox.y0));
        for n in &mut art {
            n.transform(fit, true);
            // The PDF's swatches stay with it: its colours keep their values, unlinked.
            self.reid(n);
        }
        for (key, blob) in read.document.images {
            self.doc.images.entry(key).or_insert(blob);
        }
        let gs = self.gs.clone();
        let mut group = self.new_node(NodeKind::Group { children: art.into_iter().map(Arc::new).collect(), clip: false }, &gs);
        group.name = Some(NON_NATIVE_ART.into());
        self.add(group, self.obj_hidden)
    }

    /// Give `n` and its descendants ids of this document, and unlink their swatches.
    fn reid(&mut self, n: &mut Node) {
        n.id = self.doc.alloc_id();
        for it in &mut n.appearance.items {
            let p = match it {
                AppearanceItem::Fill(f) => &mut f.paint,
                AppearanceItem::Stroke(s) => &mut s.paint,
            };
            if let Paint::Gradient(g) = p {
                g.swatch = None;
            }
            p.map_links(&mut |_, link, _| {
                *link = None;
                true
            });
        }
        if let Some(m) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
            self.reid(Arc::make_mut(&mut m.art));
        }
        if let Some(children) = n.children_mut() {
            for c in children {
                self.reid(Arc::make_mut(c));
            }
        }
    }

    /// Note something the data has that isn't read: an error on a layer that shows, left out of a
    /// layer that doesn't.
    fn unreadable(&mut self, what: &str) {
        if self.shown() {
            self.unsupported.insert(what.into());
        } else {
            self.hidden_unread.insert(what.into());
        }
    }

    /// Drop the object just read when it is an envelope's own mesh: a group on `%_` lines holding
    /// only gradient meshes. Whether it was dropped.
    fn drop_envelope_mesh(&mut self) -> bool {
        fn only_meshes(n: &Node) -> bool {
            match &n.kind {
                NodeKind::Mesh(_) => true,
                NodeKind::Group { children, .. } => !children.is_empty() && children.iter().all(|c| only_meshes(c)),
                _ => false,
            }
        }
        let Some(n) = self.last_object() else { return false };
        let id = n.id;
        if !matches!(n.kind, NodeKind::Group { .. }) || !only_meshes(n) || !self.hidden_ids.contains(&id) {
            return false;
        }
        if let Some(f) = self.frames.last_mut() {
            f.children.pop();
        }
        self.hidden_ids.remove(&id);
        self.after_object = false;
        true
    }

    /// Do the layers being read show?
    fn shown(&self) -> bool {
        self.frames.iter().all(|f| !matches!(&f.kind, Kind::Layer(a) if !a.visible))
    }

    /// An object's dictionary: its name (`AIArtName`, or the XML id `AI10_ArtUID`), and the
    /// opacity mask it may carry.
    fn art_dictionary(&mut self, o: &Obj) {
        // An envelope (a distortion, `AIDeformStyle` / `DeformValue`) is written on `%_` lines as
        // a group holding the mesh that shapes it; the distorted art the file's PDF part draws
        // follows on plain lines. That mesh isn't art, so it goes (#2405).
        if (o.get("AIDeformStyle").is_some() || o.get("DeformValue").is_some()) && self.drop_envelope_mesh() {
            return;
        }
        if let Some(mask) = o.obj("AI9_Mask").or_else(|| o.obj("Mask")) {
            if let Some(art) = &mask.art {
                let on = |key: &str| mask.nums(key).first().copied() == Some(1.0);
                let value =
                    OpacityMask { art: art.clone(), clip: on("Clipping"), invert: on("Inverted"), disabled: on("Disabled"), linked: on("Linked") };
                if let Some(n) = self.last_object().filter(|n| !matches!(n.kind, NodeKind::Layer { .. })) {
                    n.extras_mut().mask = Some(Box::new(value));
                } else if !on("Disabled") {
                    self.unreadable("opacity masks without an owning object");
                }
            } else if mask.nums("Disabled").first().copied() != Some(1.0) {
                self.unreadable("opacity masks without readable art");
            }
        }
        let name = match o.text("AIArtName") {
            Some(name) => name,
            None => match o.obj("AI10_ArtUID").and_then(Obj::content_text) {
                Some(id) => obj::xml_name(&id),
                None => return,
            },
        };
        let Some(n) = self.last_object().filter(|n| !matches!(n.kind, NodeKind::Layer { .. }) && !name.is_empty()) else { return };
        if slot_of(n.name.as_deref()).is_some() {
            let id = n.id;
            self.slot_names.insert(id, name);
        } else {
            n.name = Some(name);
        }
    }

    /// The `/Document` dictionary: the artboards, and how many variables and data sets its variable
    /// library has.
    fn document(&mut self, o: &Obj) {
        variable_library(o, 0, &mut self.variables);
        let Some(list) = find_entry(o, "ArtboardArray", 0) else { return };
        for (i, ab) in list.items().enumerate().take(1000) {
            let (p1, p2) = (ab.nums("PositionPoint1"), ab.nums("PositionPoint2"));
            let ([x0, y0, ..], [x1, y1, ..]) = (p1.as_slice(), p2.as_slice()) else { continue };
            let r = Rect::new(*x0, *y0, *x1, *y1).abs();
            if !sane(r) || r.area() <= 0.0 {
                continue;
            }
            let name = ab.text("Name").unwrap_or_else(|| format!("Artboard {}", i + 1));
            self.artboards.push((name, r));
        }
    }

    /// `… Lb`: a layer's options. The legacy format's has ten operands, its colour index one place
    /// earlier (`visible preview unlocked printing dimmed masks colorIndex r g b`).
    fn layer_attrs(&mut self, vals: &[V]) {
        let n = Self::nums(vals);
        let Some(Frame { kind: Kind::Layer(a), .. }) = self.frames.last_mut() else { return };
        let flag = |i: usize, default: bool| n.get(i).map_or(default, |v| *v != 0.0);
        a.visible = flag(0, true);
        a.preview = flag(1, true);
        a.locked = !flag(2, true);
        a.printable = flag(3, true);
        let dim_pct = n.get(12).copied().filter(|v| v.is_finite()).unwrap_or(50.0).clamp(0.0, 100.0);
        a.dim = flag(4, false).then_some(dim_pct.round() as u8);
        let at = if n.len() <= 10 { 6 } else { 7 };
        let index = n.get(at).copied().unwrap_or(0.0);
        let rgb = |i: usize| n.get(at + i).map_or(0, |v| v.clamp(0.0, 255.0) as u8);
        a.color = Some(if (0.0..vectorcraft_doc::LAYER_COLORS.len() as f64).contains(&index) && index.fract() == 0.0 {
            LayerColor::Preset(index as u8)
        } else {
            LayerColor::Custom([rgb(1), rgb(2), rgb(3)])
        });
    }

    /// `(name) x y scaleX scaleY angle reflect axis shear shearAxis [matrix] p` (`P`: the stroke's):
    /// the pattern swatch `name`, its space taken to art space by the matrix. A pattern this hasn't
    /// read can't be drawn.
    fn pattern_paint(&mut self, w: &str, vals: &[V]) {
        // In the setup it paints nothing (and the art's space isn't fixed yet).
        if self.frames.is_empty() {
            return;
        }
        let name = vals.iter().find_map(|v| if let V::Str(_) = v { v.text() } else { None });
        let Some(tile) = name.as_deref().and_then(|n| self.pattern_tiles.get(n)).copied() else {
            self.unreadable("pattern fills");
            return;
        };
        let matrix = vals.iter().rev().find_map(|v| if let V::Arr(a) = v { paint::affine(&Self::nums(a)) } else { None });
        let Some(xf) = pattern_xf(self.space(), matrix, tile) else {
            self.unreadable("pattern fills");
            return;
        };
        let p = Paint::Pattern { pattern: name.unwrap_or_default(), xf };
        if w == "p" {
            self.gs.fill = p;
        } else {
            self.gs.stroke = p;
        }
    }

    fn color(&mut self, w: &str, vals: &[V]) {
        let Some((p, named)) = paint::color_op(w, vals) else { return };
        // Registration (`Xs`, `Xz`) says nothing of the document's colour model.
        let registration = matches!(w, "Xs" | "XS" | "Xz" | "XZ");
        match p.color().filter(|_| !registration) {
            Some(vectorcraft_color::Color::Rgb { .. }) => self.rgb_colors += 1,
            Some(vectorcraft_color::Color::Cmyk { .. }) => self.cmyk_colors += 1,
            _ => {}
        }
        if let Some(n) = named {
            self.spots.push(n);
        }
        // A colour set after a gradient instance that painted nothing (`Bb` … `0 BB` with no
        // path) is the next object's paint, not the gradient.
        if w.ends_with(|c: char| c.is_ascii_lowercase()) {
            self.gs.fill = p;
            self.pending_fill = None;
        } else {
            self.gs.stroke = p;
            self.pending_stroke = None;
        }
    }

    /// `[dashes] phase d`.
    fn dash(&mut self, vals: &[V]) {
        let pattern: Vec<f64> = vals.iter().rev().find_map(|v| if let V::Arr(a) = v { Some(Self::nums(a)) } else { None }).unwrap_or_default();
        let offset = Self::nums(vals).last().copied().unwrap_or(0.0);
        self.gs.dash = (!pattern.is_empty() && pattern.len() <= 64 && pattern.iter().all(|v| v.is_finite() && *v >= 0.0)).then(|| Dash {
            pattern,
            offset: if offset.is_finite() { offset } else { 0.0 },
            align_corners: false,
        });
    }

    /// `mode opacity isolate knockout shape Xy`; after a group's end, the group's too.
    fn transparency(&mut self, vals: &[V]) {
        let n = Self::nums(vals);
        let [mode, opacity, rest @ ..] = n.as_slice() else { return };
        let gs = &mut self.gs;
        gs.blend = BLENDS.get(*mode as usize).copied().filter(|_| *mode >= 0.0).unwrap_or(BlendMode::Normal);
        gs.opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) as f32 } else { 1.0 };
        gs.isolate = rest.first().is_some_and(|v| *v != 0.0);
        gs.knockout = match rest.get(1).map(|v| *v as i64) {
            Some(1) => Knockout::On,
            Some(2) => Knockout::Neutral,
            _ => Knockout::Off,
        };
        gs.knockout_shape = rest.get(2).is_some_and(|v| *v != 0.0);
        let gs = self.gs.clone();
        if self.last_object().is_some_and(|n| matches!(n.kind, NodeKind::Group { .. } | NodeKind::Compound { .. })) {
            if let Some(n) = self.last_object() {
                n.opacity = gs.opacity;
                n.blend = gs.blend;
                n.isolate = gs.isolate;
                n.knockout = gs.knockout;
            }
            self.layer_state = None;
        } else {
            // After a layer's art and before its `LB`: the layer's own state (#2003). Kept until
            // an operator that isn't part of the state block (or the `LB` that takes it) says the
            // art moves on, so a path's state before its paint isn't read as its layer's.
            self.layer_state =
                matches!(self.frames.last(), Some(Frame { kind: Kind::Layer(_), .. })).then_some((gs.opacity, gs.blend, gs.isolate, gs.knockout));
        }
    }

    /// `n (style) XW` after an object: `1 (style)` after the object written on `%_` lines, its
    /// drawn look before it (all on `%_` lines inside another such object). Other markers end what
    /// follows an object.
    fn style_marker(&mut self, vals: &[V], hidden: bool) {
        let code = Self::nums(vals).last().copied().unwrap_or(0.0);
        let style = vals.iter().rev().find_map(V::text).unwrap_or_default();
        let after = std::mem::take(&mut self.after_object);
        if code == 9.0
            && let Some(look) = self
                .frames
                .last()
                .and_then(|f| f.children.last())
                .filter(|n| matches!(n.kind, NodeKind::Group { .. }) && !self.compound_groups.contains(&n.id))
        {
            self.looks.insert(look.id);
        }
        if code != 1.0 || style.is_empty() || !after {
            return;
        }
        let space = self.space();
        let Some(f) = self.frames.last_mut() else { return };
        let n = f.children.len();
        let (Some(object), Some(look)) = (n.checked_sub(1).and_then(|i| f.children.get(i)), n.checked_sub(2).and_then(|i| f.children.get(i))) else {
            return;
        };
        // The look is a group of paths, or one image where an effect (a blur, a shadow) rasterizes it.
        if !self.hidden_ids.contains(&object.id)
            || self.hidden_ids.contains(&look.id) != hidden
            || self.text_looks.contains(&look.id)
            || !matches!(look.kind, NodeKind::Group { clip: false, .. } | NodeKind::Image(_))
        {
            return;
        }
        // A style read here: the object, painted as it says.
        if let Some(appearance) = self
            .styles
            .get(&style)
            .and_then(|st| styled(object, look, st, &StylePaints { gradients: &self.gradients, patterns: &self.pattern_tiles, space }))
        {
            let Some(mut object) = f.children.pop() else { return };
            let Some(look) = f.children.pop() else { return };
            if let Some(b) = look.visual_bounds().filter(|_| !hidden && object.visible && look.visible) {
                self.restyled.push(b);
            }
            object.appearance = appearance.into();
            // The object's transparency is written on its look, after the look's group ends.
            (object.opacity, object.blend, object.isolate, object.knockout) = (look.opacity, look.blend, look.isolate, look.knockout);
            // Now the object itself, not a copy an enclosing look's `strip` would remove.
            self.hidden_ids.remove(&object.id);
            f.children.push(object);
            return;
        }
        let is_text = slot_of(object.name.as_deref()).is_some();
        let Some(mut object) = f.children.pop() else { return };
        let Some(look) = f.children.last_mut() else { return };
        // Inside a copy on `%_` lines everything is, so nothing tells the look's own copies apart.
        if !hidden {
            strip(look, &self.hidden_ids);
        }
        if is_text {
            // A text object stays the type it is: over what else its look draws, or in place of a
            // look that was only its type. Its transparency is written on its look.
            let empty = look.children().is_some_and(|c| c.is_empty());
            if empty || (look.opacity >= 1.0 && look.blend == BlendMode::Normal && !look.isolate) {
                (object.opacity, object.blend, object.isolate, object.knockout) = (look.opacity, look.blend, look.isolate, look.knockout);
                if empty {
                    *look = object;
                } else {
                    f.children.push(object);
                }
                return;
            }
            // The look and the type go into a group that has the transparency.
            let Some(mut look) = f.children.pop() else { return };
            let state = (look.opacity, look.blend, look.isolate, look.knockout);
            (look.opacity, look.blend, look.isolate, look.knockout) = (1.0, BlendMode::Normal, false, Knockout::Neutral);
            (object.opacity, object.blend, object.isolate) = (1.0, BlendMode::Normal, false);
            let mut group = Node::group(self.doc.alloc_id(), vec![Arc::new(look), Arc::new(object)]);
            (group.opacity, group.blend, group.isolate, group.knockout) = state;
            self.text_looks.insert(group.id);
            if let Some(f) = self.frames.last_mut() {
                f.children.push(group);
            }
            return;
        }
        look.name = object.name.clone().or(look.name.take());
        look.visible = object.visible;
        look.locked = object.locked;
        if !hidden {
            self.drawn_looks += 1;
        }
    }

    // ---- paths -----------------------------------------------------------------------------

    fn path_op(&mut self, w: &str, vals: &[V]) -> Result<(), String> {
        let n = Self::nums(vals);
        let p = |i: usize| -> Option<Point> {
            let at = n.len().checked_sub(i)?;
            Some(Point::new(*n.get(at)?, *n.get(at + 1)?))
        };
        self.points += 1;
        if self.points > MAX_POINTS {
            return Err("a path has too many points".into());
        }
        let current = self.path.elements().last().and_then(|e| e.end_point());
        match w {
            "m" => {
                let Some(a) = p(2) else { return Ok(()) };
                self.path.move_to(a);
            }
            _ if current.is_none() => {}
            "l" | "L" => {
                if let Some(a) = p(2) {
                    self.path.line_to(a);
                }
            }
            "c" | "C" => {
                if let (Some(a), Some(b), Some(c)) = (p(6), p(4), p(2)) {
                    self.path.curve_to(a, b, c);
                }
            }
            "v" | "V" => {
                if let (Some(cur), Some(b), Some(c)) = (current, p(4), p(2)) {
                    self.path.curve_to(cur, b, c);
                }
            }
            _ => {
                if let (Some(a), Some(c)) = (p(4), p(2)) {
                    self.path.curve_to(a, c, c);
                }
            }
        }
        Ok(())
    }

    /// A path left unpainted before a container opens.
    fn end_path_quietly(&mut self) {
        self.path = BezPath::new();
        self.points = 0;
        self.clip_next = false;
    }

    /// Paint the current path (`op`), as a guide when `guide`.
    fn paint(&mut self, op: &str, hidden: bool, guide: bool) -> Result<(), String> {
        let mut bp = std::mem::take(&mut self.path);
        self.points = 0;
        let clip = std::mem::take(&mut self.clip_next);
        if let Some(i) = self.instance.take() {
            self.set_instance(&i);
        }
        let (fill_paint, stroke_paint) = (self.pending_fill.take(), self.pending_stroke.take());
        if bp.elements().is_empty() {
            return Ok(());
        }
        if op.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
            close(&mut bp);
        }
        let to_doc = self.space();
        let path = PathData::from_bezpath(&(to_doc * bp));
        let gs = self.gs.clone();
        let (fill, stroke) = match op.to_ascii_lowercase().as_str() {
            "f" => (true, false),
            "s" => (false, true),
            "b" => (true, true),
            _ => (false, false),
        };
        let mut items = Vec::new();
        if fill && !clip {
            items.push(AppearanceItem::Fill(FillLayer {
                overprint: gs.overprint_fill,
                ..FillLayer::new(fill_paint.clone().unwrap_or(gs.fill.clone()))
            }));
        }
        if stroke && !clip {
            let mut st = StrokeLayer::new(stroke_paint.unwrap_or(gs.stroke.clone()), gs.width);
            st.cap = gs.cap;
            st.join = gs.join;
            st.miter_limit = gs.miter;
            st.dash = gs.dash.clone();
            st.overprint = gs.overprint_stroke;
            items.push(AppearanceItem::Stroke(st));
        }
        let kind = NodeKind::Path { path, rule: gs.rule, live: None, clipping: clip, guide };
        let mut node = self.new_node(kind, &gs);
        node.appearance = Appearance { items, ..Appearance::default() }.into();
        node.opacity = gs.opacity;
        node.blend = gs.blend;
        node.knockout_shape = gs.knockout_shape;
        if clip && let Some(f) = self.frames.last_mut() {
            f.clips.push(f.children.len());
        }
        // A clipping path's fill is painted too, under the art it clips, as the file's PDF part draws it.
        let painted = clip && fill && !matches!(gs.fill, Paint::None);
        let copy = painted.then(|| {
            let mut copy = node.clone();
            copy.id = self.doc.alloc_id();
            if let NodeKind::Path { clipping, .. } = &mut copy.kind {
                *clipping = false;
            }
            let paint = fill_paint.clone().unwrap_or_else(|| gs.fill.clone());
            copy.appearance = Appearance {
                items: vec![AppearanceItem::Fill(FillLayer { overprint: gs.overprint_fill, ..FillLayer::new(paint) })],
                ..Appearance::default()
            }
            .into();
            copy
        });
        self.add(node, hidden)?;
        if let Some(copy) = copy {
            self.clip_fills.insert(copy.id);
            self.add(copy, hidden)?;
        }
        Ok(())
    }

    /// `/End X!`: the gradient mesh read, as one object.
    fn end_mesh(&mut self) -> Result<(), String> {
        let Some((m, hidden)) = self.mesh.take() else { return Ok(()) };
        if m.unread || m.patches.is_empty() {
            self.unreadable("gradient meshes it can't read");
            return Ok(());
        }
        let gs = self.gs.clone();
        // One mesh where the patches join up into the grid they say they make; else each patch is
        // a mesh of its own, in a group.
        match m.one_mesh() {
            Ok(one) => {
                let mut node = self.new_node(NodeKind::Mesh(one), &gs);
                node.opacity = gs.opacity;
                node.blend = gs.blend;
                return self.add(node, hidden);
            }
            Err(super::mesh::NotOne::Unjoined) => self.warn("a gradient mesh whose patches don't join up comes in as a group of its patches"),
            Err(super::mesh::NotOne::Unplaced) => {}
        }
        let mut patches: Vec<Node> = m.patches.into_iter().map(|p| self.new_node(NodeKind::Mesh(p), &gs)).collect();
        let mut node = match patches.len() {
            1 => patches.pop().unwrap_or_else(|| self.new_node(NodeKind::Group { children: vec![], clip: false }, &gs)),
            _ => self.new_node(NodeKind::Group { children: patches.into_iter().map(Arc::new).collect(), clip: false }, &gs),
        };
        node.opacity = gs.opacity;
        node.blend = gs.blend;
        self.add(node, hidden)
    }

    /// Stroke the path just painted (a gradient-filled one, `1 BB` or `2 BB`) with the stroke's state.
    fn stroke_last(&mut self) {
        let gs = self.gs.clone();
        let Some(n) = self.last_object() else { return };
        if !matches!(n.kind, NodeKind::Path { clipping: false, .. }) || n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(_))) {
            return;
        }
        let mut st = StrokeLayer::new(gs.stroke.clone(), gs.width);
        st.cap = gs.cap;
        st.join = gs.join;
        st.miter_limit = gs.miter;
        st.dash = gs.dash.clone();
        st.overprint = gs.overprint_stroke;
        let mut a = (*n.appearance).clone();
        a.items.push(AppearanceItem::Stroke(st));
        n.appearance = a.into();
    }

    fn set_instance(&mut self, i: &Instance) {
        let mut to_doc = self.space();
        // In a pattern's definition a gradient's `Xm` and `Bm` are written y down about the canvas point of art space's
        // origin, as the file's PDF part draws them.
        if self.pattern.is_some()
            && i.has_matrix()
            && let Some(t) = self.template
        {
            let (cx, cy) = ((t[0] + t[2]) / 2.0, (t[1] + t[3]) / 2.0);
            to_doc *= Affine::new([1.0, 0.0, 0.0, -1.0, CANVAS_CENTRE - cx, CANVAS_CENTRE + cy]);
        }
        match i.paint(&self.gradients, to_doc) {
            Some(p) if i.stroke => self.pending_stroke = Some(p),
            Some(p) => self.pending_fill = Some(p),
            None => {
                let name = i.name.clone().unwrap_or_default();
                self.warn(&format!("gradient “{name}” isn't defined in the file: a plain colour stands in for it"));
            }
        }
    }

    // ---- images ----------------------------------------------------------------------------

    /// The samples after `XI`, its operands on the stack.
    fn image(&mut self, data: &[u8]) -> Result<(), String> {
        let vals = self.operands();
        // Operands that came before an `XI` on their own line.
        let vals = match self.raster.as_mut().and_then(|r| r.operands.take()) {
            Some(before) if !vals.iter().any(|v| matches!(v, V::Arr(_))) => before,
            _ => vals,
        };
        self.image_from(vals, data)
    }

    /// An image of operands `vals` and samples `data`.
    fn image_from(&mut self, vals: Vec<V>, data: &[u8]) -> Result<(), String> {
        let space = self.raster.as_ref().and_then(|r| r.space.clone()).unwrap_or_default();
        let Some(m) = vals.iter().find_map(|v| if let V::Arr(a) = v { Some(Self::nums(a)) } else { None }) else {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let n = Self::nums(&vals);
        let [_, _, _, _, w, h, bits, kind, alpha, _, binary, ..] = n.as_slice() else {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let (w, h) = (*w as u64, *h as u64);
        if w.saturating_mul(h) > MAX_PIXELS {
            self.warn(IMAGE_TOO_LARGE);
            return Ok(());
        }
        if w == 0 || h == 0 || ![1.0, 8.0].contains(bits) {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        }
        let channels: usize = match space.as_str() {
            "DeviceGray" => 1,
            "DeviceCMYK" => 4,
            "DeviceRGB" => 3,
            _ => match *kind as i64 {
                1 => 1,
                4 => 4,
                _ => 3,
            },
        };
        let decoded;
        let data = if *binary == 0.0 {
            let digits: Vec<u8> = data.iter().copied().filter(|b| b.is_ascii_hexdigit()).collect();
            decoded = super::super::lex::hex_decode(&digits).unwrap_or_default();
            decoded.as_slice()
        } else {
            data
        };
        let row = if *bits == 1.0 { (w as usize * channels).div_ceil(8) } else { w as usize * channels };
        let need = row.saturating_mul(h as usize);
        // An alpha channel follows the colour samples, one byte a pixel.
        let alpha_len = if *alpha > 0.0 { (w * h) as usize } else { 0 };
        let (Some(samples), opacity) = (data.get(..need), data.get(need..need.saturating_add(alpha_len))) else {
            self.warn("an image whose data is cut short was left out");
            return Ok(());
        };
        let opacity = opacity.filter(|_| alpha_len > 0);
        // CMYK samples keep their ink amounts as a CMYK TIFF, as the PDF importer keeps a CMYK
        // image's, and an alpha channel that hides part of the image becomes its opacity mask.
        if channels == 4
            && *bits == 8.0
            && let Some(blob) = Inks::new(w as u32, h as u32, samples.to_vec()).and_then(|i| ImageBlob::cmyk_tiff(&i))
        {
            return self.add_raster(blob, opacity, &m, (w as u32, h as u32));
        }
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h as usize {
            let line = samples.get(y * row..(y + 1) * row).unwrap_or_default();
            for x in 0..w as usize {
                // A 1-bit row holds each pixel's channels in turn, a bit each, padded to a whole
                // byte (`row`); a set bit is the channel at full strength (255).
                let s = |c: usize| {
                    let i = x * channels + c;
                    if *bits == 1.0 { line.get(i / 8).map_or(0, |b| ((b >> (7 - i % 8)) & 1) * 255) } else { line.get(i).copied().unwrap_or(0) }
                };
                let px = match channels {
                    1 => [s(0); 3],
                    4 => {
                        let key = [s(0), s(1), s(2), s(3)];
                        if self.cmyk_cache.len() > 1 << 16 {
                            self.cmyk_cache.clear();
                        }
                        *self.cmyk_cache.entry(key).or_insert_with(|| cmyk_rgb(key))
                    }
                    _ => [s(0), s(1), s(2)],
                };
                let a = opacity.and_then(|o| o.get(y * w as usize + x)).copied().unwrap_or(255);
                rgba.extend_from_slice(&[px[0], px[1], px[2], a]);
            }
        }
        let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, rgba) else {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let mut png = vec![];
        if img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        }
        self.add_raster(ImageBlob::new("image/png", png), None, &m, (w as u32, h as u32))
    }

    /// Add the image `blob` (`w` × `h` pixels), which the matrix `m` maps into art space, with
    /// `alpha` (its alpha channel, one byte a pixel) as its opacity mask when it hides part of the
    /// image.
    fn add_raster(&mut self, blob: ImageBlob, alpha: Option<&[u8]>, m: &[f64], (w, h): (u32, u32)) -> Result<(), String> {
        // The matrix is an art-space matrix (y up) applied to the pixels with their rows running
        // downwards, (u, -v): pixels → (u, -v) → art space. Flipping the matrix's result instead
        // gives the same place only when the image isn't rotated (b = c = 0).
        let [a, b, c, d, tx, ty] = m else {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        };
        let to_doc = self.space();
        let xf = to_doc * Affine::new([*a, *b, *c, *d, *tx, *ty]) * FLIP;
        if !xf.as_coeffs().iter().all(|v| v.is_finite()) || xf.determinant().abs() < 1e-12 {
            self.warn("an image VectorCraft can't read was left out");
            return Ok(());
        }
        let mask = alpha.and_then(|a| self.alpha_mask(a, w, h, xf));
        let key = blob.content_key();
        self.doc.images.entry(key.clone()).or_insert(blob);
        self.add_image_masked(ImageObject { key, width: w, height: h, xf, link: None, placement: Default::default(), pixels: None }, mask, false)
    }

    /// An image's alpha channel (`w` × `h` bytes) as a grayscale image over the image (`xf`): its
    /// opacity mask, as in PDF import. `None` when it hides nothing.
    fn alpha_mask(&mut self, alpha: &[u8], w: u32, h: u32, xf: Affine) -> Option<OpacityMask> {
        if alpha.iter().all(|v| *v == 255) {
            return None;
        }
        let mut png = Vec::new();
        image::GrayImage::from_raw(w, h, alpha.to_vec())?.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
        let blob = ImageBlob::new("image/png", png);
        let key = blob.content_key();
        self.doc.images.entry(key.clone()).or_insert(blob);
        let art = Node::new(
            self.doc.alloc_id(),
            NodeKind::Image(Box::new(ImageObject { key, width: w, height: h, xf, link: None, placement: Default::default(), pixels: None })),
        );
        Some(OpacityMask::new(art, true))
    }

    /// Add an image object with the state's transparency.
    fn add_image(&mut self, im: ImageObject, hidden: bool) -> Result<(), String> {
        self.add_image_masked(im, None, hidden)
    }

    /// Add an image object with the state's transparency and `mask` as its opacity mask.
    fn add_image_masked(&mut self, im: ImageObject, mask: Option<OpacityMask>, hidden: bool) -> Result<(), String> {
        let gs = self.gs.clone();
        let mut node = self.new_node(NodeKind::Image(Box::new(im)), &gs);
        node.opacity = gs.opacity;
        node.blend = gs.blend;
        node.extras_mut().mask = mask.map(Box::new);
        self.add(node, hidden)
    }

    /// `~`: the placed file read since `%AI5_BeginPlace` becomes a linked image in its place that
    /// shows the preview the data keeps of it, as a linked image whose file is missing does. One
    /// without a preview, a path or a matrix isn't read.
    fn end_place(&mut self, hidden: bool) -> Result<(), String> {
        let Some(Place { matrix: Some(m), bounds: Some(b), path, preview: Some((png, w, h)) }) = self.place.take().filter(|p| !p.path.is_empty())
        else {
            self.unreadable("placed files");
            return Ok(());
        };
        // The file's pixels (y down) start at the canvas point of art space's origin (the box's own
        // corner when the file doesn't say where the canvas is); the preview's are spread over them.
        let centre = self.template.map(|t| ((t[0] + t[2]) / 2.0, (t[1] + t[3]) / 2.0));
        let origin = centre.map_or(b.origin(), |(x, y)| Point::new(CANVAS_CENTRE - x, CANVAS_CENTRE + y));
        let over_file = Affine::translate(origin.to_vec2()) * Affine::scale_non_uniform(b.width() / f64::from(w), b.height() / f64::from(h));
        let xf = self.space() * m * over_file;
        if !xf.as_coeffs().iter().all(|v| v.is_finite()) || xf.determinant().abs() < 1e-12 {
            self.unreadable("placed files");
            return Ok(());
        }
        // The preview is all the document has of the file: what a missing link shows.
        let bytes = Arc::new(png);
        let blob = ImageBlob { mime: "image/png".into(), bytes: bytes.clone(), proxy: Some(bytes) };
        let key = blob.content_key();
        self.doc.images.entry(key.clone()).or_insert(blob);
        self.warn(LINKED_PREVIEWS);
        self.add_image(
            ImageObject { key, width: w, height: h, xf, link: Some(LinkInfo::new(path)), placement: Default::default(), pixels: None },
            hidden,
        )
    }

    // ---- the document ----------------------------------------------------------------------

    /// The artboard of a file without artboards or crop marks: its size around the template
    /// box's centre (half a point off the grid).
    fn sized_board(&self) -> Option<Rect> {
        let ((w, h), (cx, cy)) = (self.art_size?, self.template_center?);
        // Rounded out, but not off a whole point the decimals in the file only just miss.
        let near = |v: f64| (v - v.round()).abs() < 1e-6;
        let (left, top) = (cx - w / 2.0, cy + h / 2.0);
        let x0 = if near(left) { left.round() } else { left.floor() };
        let y1 = if near(top) { top.round() } else { top.ceil() };
        let r = Rect::new(x0, y1 - h, x0 + w, y1);
        (sane(r) && w > 0.0 && h > 0.0).then_some(r)
    }

    /// The first artboard: the `/Document`'s first, else the crop marks, else [`Self::sized_board`].
    fn first_board(&self) -> Option<Rect> {
        self.artboards.first().map(|(_, r)| *r).or(self.cropmarks).or_else(|| self.sized_board())
    }

    /// The page of a file that says nothing of its artboard (the legacy format as other apps write
    /// it): its art's bounding box.
    fn bbox_board(&self) -> Option<Rect> {
        let [x0, y0, x1, y1] = self.hires.or(self.bbox)?;
        Some(Rect::new(x0, y0, x1, y1)).filter(|r| r.width() > 0.0 && r.height() > 0.0)
    }

    /// Art space → the document (fixed when the art starts, from the artboards read by then). A
    /// Large Canvas editing copy is written at 1/N scale ([`Self::large_canvas`]): the factor makes
    /// its art and boxes reach the page's size, which is what the page it is opened beside holds.
    fn ensure_space(&mut self) -> Affine {
        if let Some(m) = self.to_doc {
            return m;
        }
        let first = self.first_board().or_else(|| self.bbox_board());
        let top_left = first.map_or(Point::new(0.0, 792.0), |r| Point::new(r.x0, r.y1));
        let s = self.large_canvas.unwrap_or(1.0);
        let m = Affine::new([s, 0.0, 0.0, -s, -s * top_left.x, s * top_left.y]);
        self.to_doc = Some(m);
        m
    }

    /// Art space → where the art being read goes: the document, or while a pattern's definition is
    /// read, the pattern's space (y down).
    fn space(&mut self) -> Affine {
        if self.pattern.is_some() { FLIP } else { self.ensure_space() }
    }

    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) && self.warnings.len() < 64 {
            self.warnings.push(w.to_string());
        }
    }

    fn finish(mut self) -> Result<Structure, String> {
        if !self.unknown.is_empty() {
            let ops: Vec<String> = self.unknown.iter().map(|o| format!("`{o}`")).collect();
            self.unsupported.insert(format!("operators VectorCraft doesn't know ({})", ops.join(", ")));
        }
        if !self.unsupported.is_empty() {
            let list: Vec<&str> = self.unsupported.iter().map(String::as_str).collect();
            return Err(format!("it has {}, which VectorCraft doesn't read from it yet", join_and(&list)));
        }
        if self.doc.layers.is_empty() {
            return Err("it has no layers".into());
        }
        let to_doc = self.ensure_space();
        let artboard = self.first_board();
        drop_commented_copies(&mut self.doc.layers, &self.hidden_ids);
        let mut doc = std::mem::replace(&mut self.doc, Document::new(1.0, 1.0));
        let boards: Vec<(String, Rect)> = if self.artboards.is_empty() {
            artboard.or_else(|| self.bbox_board()).map(|r| vec![("Artboard 1".to_string(), r)]).unwrap_or_default()
        } else {
            self.artboards.clone()
        };
        doc.artboards = boards
            .into_iter()
            .enumerate()
            .map(|(i, (name, r))| Artboard {
                id: i as u32 + 1,
                name,
                rect: to_doc.transform_rect_bbox(r),
                locked: false,
                show_center_mark: false,
                show_cross_hairs: false,
                show_video_safe_areas: false,
                video_ruler_pixel_aspect_ratio: 1.0,
                color: None,
            })
            .collect();
        if doc.artboards.is_empty() {
            let rect = doc.art_bounds().filter(|r| sane(*r) && r.area() > 0.0).unwrap_or(Rect::new(0.0, 0.0, 612.0, 792.0));
            doc.artboards = vec![Artboard {
                id: 1,
                name: "Artboard 1".into(),
                rect,
                locked: false,
                show_center_mark: false,
                show_cross_hairs: false,
                show_video_safe_areas: false,
                video_ruler_pixel_aspect_ratio: 1.0,
                color: None,
            }];
        }
        if self.color_model.unwrap_or(self.cmyk_colors > self.rgb_colors) {
            doc.color_mode = ColorMode::Cmyk;
            let (swatches, groups) = vectorcraft_color::default_swatches(ColorMode::Cmyk.model());
            (doc.swatches, doc.swatch_groups) = (swatches.into(), groups.into());
        }
        if let Some(u) = self.ruler_units.and_then(ruler_unit) {
            doc.units = u;
        }
        // The file's own swatches in place of the defaults, in its order and colour groups.
        if !self.palette.is_empty() {
            let palette = std::mem::take(&mut self.palette);
            let swatch = |e: super::palette::Entry| -> Option<Swatch> {
                use super::palette::Cell;
                let (paint, global, spot) = match e.cell {
                    Cell::Color { color, global, spot } => (Paint::solid(color), global, spot),
                    Cell::Gradient(g) => {
                        let mut gp = vectorcraft_color::gradient::GradientPaint::new(self.gradients.get(&g)?.clone());
                        gp.swatch = Some(e.name.clone());
                        (Paint::Gradient(Box::new(gp)), false, false)
                    }
                    Cell::Pattern(p) => {
                        doc.patterns.iter().find(|d| d.name == p)?;
                        (Paint::Pattern { pattern: p, xf: Affine::IDENTITY }, false, false)
                    }
                };
                Some(Swatch { name: e.name, paint, global, spot })
            };
            let mut swatches = vec![Swatch { name: "[None]".into(), paint: Paint::None, global: false, spot: false }];
            swatches.extend(palette.swatches.into_iter().filter_map(swatch));
            let groups: Vec<_> = palette
                .groups
                .into_iter()
                .map(|(name, s)| vectorcraft_color::swatch::SwatchGroup { name, swatches: s.into_iter().filter_map(swatch).collect() })
                .collect();
            (doc.swatches, doc.swatch_groups) = (swatches.into(), groups.into());
        }
        // The colour operators' definitions first: a gradient, defined before the art, names a
        // global process colour as a stop and would otherwise make it an ink (#1771).
        for n in self.spots.iter().chain(&self.stop_spots) {
            if doc.swatch(&n.name).is_none() {
                doc.swatches.push(Swatch { name: n.name.clone(), paint: Paint::solid(n.color), global: true, spot: n.spot });
            }
        }
        for name in doc.patterns.iter().map(|p| p.name.clone()).collect::<Vec<_>>() {
            if doc.swatch(&name).is_none() {
                doc.swatches.push(Swatch { paint: Paint::Pattern { pattern: name.clone(), xf: Affine::IDENTITY }, name, global: false, spot: false });
            }
        }
        let mut warnings = self.warnings;
        if self.drawn_looks > 0 {
            warnings.push(format!(
                "objects with several fills or strokes, effects or brushes came in as their drawn look ({}): their appearance isn't editable yet",
                self.drawn_looks
            ));
        }
        warnings.extend(self.variables.note());
        Ok(Structure {
            doc,
            to_doc,
            bbox: self.hires.or(self.bbox),
            artboard,
            template: self.template,
            hidden_unread: self.hidden_unread,
            slot_names: self.slot_names,
            restyled: self.restyled,
            warnings,
        })
    }
}

/// The source text inside an envelope options dictionary, carried through its containing metadata.
fn envelope_text_slots(node: &Node, out: &mut Vec<Node>) {
    if slot_of(node.name.as_deref()).is_some() {
        out.push(node.clone());
    } else if let Some(children) = node.children() {
        for child in children {
            envelope_text_slots(child, out);
        }
    }
}

/// The text objects written twice, plainly and on `%_` lines: the commented copy (`hidden`) of a
/// story that also has a plain one goes.
fn drop_commented_copies(layers: &mut [Arc<Node>], hidden: &HashSet<NodeId>) {
    fn plain(nodes: &[Arc<Node>], hidden: &HashSet<NodeId>, out: &mut HashSet<u32>) {
        for n in nodes {
            match slot_of(n.name.as_deref()) {
                Some(Some(story)) if !hidden.contains(&n.id) => {
                    out.insert(story);
                }
                _ => {
                    if let Some(c) = n.children() {
                        plain(c, hidden, out);
                    }
                }
            }
        }
    }
    fn drop(nodes: &mut Vec<Arc<Node>>, hidden: &HashSet<NodeId>, plain: &HashSet<u32>) {
        nodes.retain(|n| !(hidden.contains(&n.id) && slot_of(n.name.as_deref()).flatten().is_some_and(|s| plain.contains(&s))));
        for n in nodes.iter_mut() {
            if n.children().is_some_and(|c| !c.is_empty())
                && let Some(c) = Arc::make_mut(n).children_mut()
            {
                drop(c, hidden, plain);
            }
        }
    }
    let mut stories = HashSet::new();
    plain(layers, hidden, &mut stories);
    if stories.is_empty() {
        return;
    }
    for l in layers.iter_mut() {
        if let Some(c) = Arc::make_mut(l).children_mut() {
            drop(c, hidden, &stories);
        }
    }
}

/// A node and its index among its siblings.
type Indexed = (usize, Node);

/// Close the subpath `bp` ends with, if it has one open.
fn close(bp: &mut BezPath) {
    if bp.elements().last().is_some_and(|e| !matches!(e, PathEl::ClosePath)) {
        bp.close_path();
    }
}

/// The fill rule of a path or compound node.
/// `kids` with the members of each plain group among them in the group's place, at any depth.
fn ungrouped(kids: Vec<Node>) -> Vec<Node> {
    let mut out = Vec::with_capacity(kids.len());
    for k in kids {
        match k.kind {
            NodeKind::Group { children, clip: false } => out.extend(ungrouped(children.into_iter().map(Arc::unwrap_or_clone).collect())),
            _ => out.push(k),
        }
    }
    out
}

/// Whether `look`'s paths don't all paint the same.
fn paints_apart(look: &Node) -> bool {
    fn leaves<'a>(n: &'a Node, out: &mut Vec<&'a Node>) {
        match &n.kind {
            NodeKind::Group { children, .. } => children.iter().for_each(|c| leaves(c, out)),
            _ => out.push(n),
        }
    }
    let mut paths = vec![];
    leaves(look, &mut paths);
    paths.windows(2).any(|w| matches!(w, [a, b] if a.appearance != b.appearance))
}

fn path_rule(n: &Node) -> FillRule {
    match &n.kind {
        NodeKind::Path { rule, .. } | NodeKind::Compound { rule, .. } => *rule,
        _ => FillRule::NonZero,
    }
}

/// The tile's art (y down from the tile's top left) → the pattern's space (`matrix`) → art space
/// → the document (`space`). The tile's lower left is the pattern's origin wherever the
/// definition drew it, as the file's PDF part tiles it. `None` when it can't be inverted.
fn pattern_xf(space: Affine, matrix: Option<Affine>, tile: Rect) -> Option<Affine> {
    let xf = space * matrix.unwrap_or(Affine::IDENTITY) * FLIP * Affine::translate((0.0, -tile.height()));
    (xf.determinant().abs() >= 1e-12 && xf.as_coeffs().iter().all(|v| v.is_finite())).then_some(xf)
}

/// What a style's fills paint with: the file's gradients and pattern tiles, and art space → the document.
struct StylePaints<'a> {
    gradients: &'a HashMap<String, Gradient>,
    patterns: &'a HashMap<String, Rect>,
    space: Affine,
}

/// The gradients and patterns `look` paints with, placed as it draws them, by their swatch's name
/// (the first of each; a pattern's name after `PATTERN_KEY`).
fn placed_paints(look: &Node, out: &mut HashMap<String, Paint>, depth: usize) {
    if depth > MAX_DEPTH {
        return;
    }
    for item in &look.appearance.items {
        let key = match item.paint() {
            Paint::Gradient(g) => g.swatch.clone(),
            Paint::Pattern { pattern, .. } => Some(format!("{PATTERN_KEY}{pattern}")),
            _ => None,
        };
        if let Some(key) = key {
            out.entry(key).or_insert_with(|| item.paint().clone());
        }
    }
    for c in look.children().into_iter().flatten() {
        placed_paints(c, out, depth + 1);
    }
}

/// What [`placed_paints`] keys a pattern's name with, apart from the gradients' names.
const PATTERN_KEY: &str = "\u{1}pattern ";

/// `object`'s appearance with style `st`, its drawn look `look`: `None` when the style isn't one
/// this reads, or asks a group for fills or strokes.
fn styled(object: &Node, look: &Node, st: &super::style::ArtStyle, paints: &StylePaints) -> Option<Appearance> {
    use super::style::StyleItem;
    // A text object's slot (an empty group standing for it) takes the style as the type will.
    let slot = slot_of(object.name.as_deref()).is_some();
    if !st.plain
        || (!slot && !st.simple)
        || (object.is_container() && !slot && !matches!(object.kind, NodeKind::Compound { .. }) && !st.items.is_empty())
    {
        return None;
    }
    // A style with one gradient fill: the object's own gradient, placed as the object's fill is.
    let gradients = st.items.iter().filter(|i| matches!(i, StyleItem::Gradient { .. })).count();
    let own_gradient = object.appearance.fill().filter(|f| matches!(f.paint, Paint::Gradient(_)));
    // A style's gradients are written in a space of the object's bounds that isn't documented; its
    // drawn look has each where the app placed it.
    let mut placed = HashMap::new();
    placed_paints(look, &mut placed, 0);
    let mut items = vec![];
    for (n, item) in st.items.iter().enumerate() {
        let mut layer = match item {
            StyleItem::Fill(paint) => AppearanceItem::Fill(FillLayer::new(paint.clone())),
            StyleItem::Gradient { name, angle } => {
                let paint = match (name.as_ref().and_then(|n| placed.get(n)), gradients, own_gradient) {
                    // Other objects than type: the object's own, as they always were.
                    (_, 1, Some(own)) if !slot => own.paint.clone(),
                    (Some(p), ..) => p.clone(),
                    (None, 1, Some(own)) => own.paint.clone(),
                    _ => {
                        let gradient = name.as_ref().and_then(|n| paints.gradients.get(n))?;
                        let mut g = vectorcraft_color::gradient::GradientPaint::new(gradient.clone());
                        // Fitted to the object's bounds at its angle (art space turns counter-clockwise, as the document's angles do).
                        g.angle = *angle;
                        Paint::Gradient(Box::new(g))
                    }
                };
                AppearanceItem::Fill(FillLayer::new(paint))
            }
            StyleItem::Pattern { name, matrix } => match placed.get(&format!("{PATTERN_KEY}{name}")) {
                Some(p) => AppearanceItem::Fill(FillLayer::new(p.clone())),
                None => {
                    let tile = paints.patterns.get(name).copied()?;
                    let xf = pattern_xf(paints.space, matrix.and_then(|m| paint::affine(&m)), tile)?;
                    AppearanceItem::Fill(FillLayer::new(Paint::Pattern { pattern: name.clone(), xf }))
                }
            },
            StyleItem::Stroke { paint, width, cap, join, miter, dash, profile } => {
                let mut layer = StrokeLayer::new(paint.clone(), *width);
                (layer.cap, layer.join, layer.miter_limit, layer.dash, layer.profile) = (*cap, *join, *miter, dash.clone(), profile.clone());
                AppearanceItem::Stroke(layer)
            }
        };
        if let Some(look) = st.looks.get(n) {
            match &mut layer {
                AppearanceItem::Fill(f) => {
                    (f.visible, f.opacity, f.blend, f.effects) = (look.visible, look.opacity, look.blend, look.effects.clone())
                }
                AppearanceItem::Stroke(s) => {
                    (s.visible, s.opacity, s.blend, s.effects) = (look.visible, look.opacity, look.blend, look.effects.clone())
                }
            }
        }
        items.push(layer);
    }
    let contents_index = if object.is_container() || matches!(object.kind, NodeKind::Text(_) | NodeKind::Image(_)) {
        st.contents_index
    } else {
        object.appearance.contents_index
    };
    Some(Appearance { items, effects: st.effects.clone(), contents_index })
}

/// Remove from `n` the children read from `%_` lines (`hidden`: the object itself, not its look).
fn strip(n: &mut Node, hidden: &HashSet<NodeId>) {
    if let Some(children) = n.children_mut() {
        children.retain(|c| !hidden.contains(&c.id));
        for c in children.iter_mut() {
            strip(Arc::make_mut(c), hidden);
        }
    }
}

/// The dictionary of entry `key` in `o` or the dictionaries it holds.
fn find_entry<'o>(o: &'o Obj, key: &str, depth: usize) -> Option<&'o Obj> {
    if depth > 8 {
        return None;
    }
    o.obj(key).or_else(|| o.entries.iter().flat_map(|(_, v)| v.iter()).filter_map(V::obj).find_map(|c| find_entry(c, key, depth + 1)))
}

/// Add the variables and data sets of the variable libraries in `o` or the dictionaries it holds
/// (see [`VariableLibrary`]) to `found`.
fn variable_library(o: &Obj, depth: usize, found: &mut VariableLibrary) {
    if depth > MAX_DEPTH {
        return;
    }
    if o.ty == "XMLNode" && element_name(o).as_deref() == Some("variableSets") {
        for set in elements(o, "variableSet") {
            found.variables += elements(set, "variables").flat_map(|v| elements(v, "variable")).count();
            found.data_sets += elements(set, "sampleDataSets").flat_map(|d| elements(d, "sampleDataSet")).count();
        }
        return;
    }
    for c in o.items() {
        variable_library(c, depth + 1, found);
    }
}

/// The name of the XML node `o` (its `xmlnode-nodename`) without a namespace prefix (`v:`).
fn element_name(o: &Obj) -> Option<String> {
    let name = o.text("xmlnode-nodename")?;
    Some(name.rsplit(':').next().unwrap_or(&name).to_string())
}

/// The child nodes named `name` (see [`element_name`]) of the XML node `o`.
fn elements<'o>(o: &'o Obj, name: &'o str) -> impl Iterator<Item = &'o Obj> {
    o.obj("xmlnode-children").into_iter().flat_map(Obj::items).filter(move |c| c.ty == "XMLNode" && element_name(c).as_deref() == Some(name))
}

/// A placed file's preview as a PNG, with its size: `w h rowBytes 32 n ` (each number followed by
/// a space), then `h` rows of `rowBytes` bytes, premultiplied ARGB. `None` when it isn't that, is
/// larger than [`MAX_PIXELS`] or is cut short.
fn preview_png(data: &[u8]) -> Option<(Vec<u8>, u32, u32)> {
    let mut rest = data.trim_ascii_start();
    let mut head = [0usize; 5];
    for v in &mut head {
        let n = rest.iter().position(|b| *b == b' ')?;
        *v = std::str::from_utf8(rest.get(..n)?).ok()?.parse().ok()?;
        rest = rest.get(n + 1..)?;
    }
    let [w, h, row, bits, _] = head;
    let (width, height) = (u32::try_from(w).ok()?, u32::try_from(h).ok()?);
    if w == 0 || h == 0 || u64::from(width) * u64::from(height) > MAX_PIXELS || bits != 32 || row < w * 4 {
        return None;
    }
    // Every row is there before anything is allocated for them.
    if rest.len() < (h - 1).checked_mul(row)?.checked_add(w * 4)? {
        return None;
    }
    let mut rgba = Vec::with_capacity(w * h * 4);
    for line in rest.chunks(row).take(h) {
        for &[a, r, g, b] in line.get(..w * 4)?.as_chunks::<4>().0 {
            let straight = |c: u8| if a == 0 { 0 } else { (u32::from(c) * 255 / u32::from(a)).min(255) as u8 };
            rgba.extend_from_slice(&[straight(r), straight(g), straight(b), a]);
        }
    }
    let mut png = vec![];
    image::RgbaImage::from_raw(width, height, rgba)?.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some((png, width, height))
}

fn sane(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite() && v.abs() <= MAX_SIDE)
}

/// The four numbers of a comment's value (a box that may be a point).
fn four(v: &str) -> Option<[f64; 4]> {
    let n: Vec<f64> = v.split_whitespace().map_while(|w| w.parse::<f64>().ok()).collect();
    let [a, b, c, d] = n.as_slice() else { return None };
    [a, b, c, d].iter().all(|v| v.is_finite() && v.abs() <= MAX_SIDE).then_some([*a, *b, *c, *d])
}

/// The first two numbers of a comment's value.
fn two(v: &str) -> Option<(f64, f64)> {
    let n: Vec<f64> = v.split_whitespace().filter_map(|w| w.parse().ok()).filter(|v: &f64| v.is_finite()).collect();
    match n.as_slice() {
        [a, b, ..] => Some((*a, *b)),
        _ => None,
    }
}

/// A CMYK sample in RGB.
fn cmyk_rgb([c, m, y, k]: [u8; 4]) -> [u8; 3] {
    let f = |v: u8| f32::from(v) / 255.0;
    let rgb = vectorcraft_color::Color::cmyk(f(c), f(m), f(y), f(k)).to_rgba8(1.0);
    [rgb[0], rgb[1], rgb[2]]
}

/// `a`, `a and b`, `a, b and c`.
fn join_and(items: &[&str]) -> String {
    match items {
        [] => String::new(),
        [one] => (*one).to_string(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}
