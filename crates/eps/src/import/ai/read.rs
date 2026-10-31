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

mod art;
mod containers;
mod directives;
mod document;
mod ops;
mod types;

use art::preview_png;
pub use document::{PaletteSwatches, VariableLibrary, artboards, palette, variables};
pub(crate) use document::{slot_frame, slot_of};
pub use types::IMAGE_TOO_LARGE;
use types::{BLEND_BUDGET, FLIP, Frame, GState, Kind, PatternHead, Place, Raster};
#[cfg(test)]
pub use types::{NON_NATIVE_ART, NON_NATIVE_ART_UNREAD, VERSIONED_UNENDED};
pub(super) use types::{blend_mode, blend_name};

#[path = "legacy_text.rs"]
mod legacy_text;

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
    /// Placed files whose pixels the file puts at another point too: their image's id and the
    /// move, in its pixels, that places them there (the corner of the box they are written with,
    /// in place of the canvas point of art space's origin).
    pub placed_elsewhere: Vec<(NodeId, Affine)>,
    pub warnings: Vec<String>,
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
    /// The blend part each object's dictionary names (`ShapeBlendPathType`, `ShapeBlendPathID`).
    blend_roles: super::blend::Roles,
    /// A blend's record being read from the `%` lines after its `XP`: its length and hex digits.
    blend_record: Option<(usize, Vec<u8>)>,
    /// How many more objects blends may lay out in this import ([`super::blend::live_blend`]).
    blend_budget: usize,
    /// A group just ended, its state (`Xy`, `Xd`, `XW`) not yet read.
    group_ended: bool,
    /// A gradient mesh just ended (`/End X!`), its own state (`Xy`) not yet read: also after its
    /// dictionary (its name, its opacity mask and the mask's art).
    mesh_ended: bool,
    /// A layer's own state (`Xy` after its art, before `LB`), until something says the art moves on.
    layer_state: Option<(f32, BlendMode, bool, Knockout)>,
    /// The state a mesh or blend had before an `Xy` right after its end, which is the layer's
    /// instead when the layer's `LB` follows rather than their own `6 () XW`.
    mesh_undo: Option<(f32, BlendMode, bool, Knockout)>,
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
    /// See [`Structure::placed_elsewhere`].
    placed_elsewhere: Vec<(NodeId, Affine)>,
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
            blend_roles: HashMap::new(),
            blend_record: None,
            blend_budget: BLEND_BUDGET,
            clip_fills: HashSet::new(),
            looks: HashSet::new(),
            compound_groups: HashSet::new(),
            text_looks: HashSet::new(),
            group_ended: false,
            mesh_ended: false,
            layer_state: None,
            mesh_undo: None,
            styles: HashMap::new(),
            palette: super::palette::Palette::default(),
            restyled: vec![],
            placed_elsewhere: vec![],
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
}
