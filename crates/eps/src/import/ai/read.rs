//! The reader: runs the editing data's operators into a document.
//!
//! - Art space has y up; the first artboard's top left corner is the document's origin.
//! - Layers: `%AI5_BeginLayer`, `visible preview unlocked printing dimmed … colorIndex r g b
//!   template dimPercent … Lb` (ten operands in the legacy format, without the one before the
//!   colour index; see `Reader::layer_attrs`), `(name) Ln`, its art, `LB`.
//! - Multi-layer masks `Mb` <path> `W n` `Md` … `MB`, whose objects can go on past `LB` into the
//!   next layers: a clipping group in each layer, read by [`masks`].
//! - Ruler guides: a straight `(N) *` guide path, directly on a layer, that runs across the whole
//!   canvas (16,383 points around the template box's centre) or from edge to edge of one artboard
//!   is a ruler guide on that layer ([`Document::guides`]), shown or hidden, with the path's name,
//!   lock and visibility; other guide paths stay paths.
//! - Groups `u` … `U`, compound paths `*u` … `*U`, clipping groups `q` … `Q` (the path painted
//!   with `W` clips); a group's transparency and name follow its end. A wraparound group `*w` …
//!   `*W` (version 7 specification §15.4.2) is a group whose objects other than its type wrap
//!   area type, with no offset.
//! - Paths: `m`, `l`/`L`, `c`/`C`, `v`/`V`, `y`/`Y`, painted by `N n F f S s B b` (lower case
//!   closes), `(op) *` for a guide. A segment's operator in lower case makes the anchor it ends on
//!   smooth, in upper case a corner; a closed path's first anchor takes the kind of the segment
//!   back to it. Stroke `w J j M d`, fill rule `XR`, overprint `O`/`R`.
//! - State: hidden `Xw`, locked `A`, transparency `mode opacity isolate knockout shape Xy`.
//! - Names: an `/ArtDictionary` after an object, its `AI10_ArtUID` the name's XML id.
//! - Artboards: the `ArtboardArray` of the `/Document` dictionary, with each one's display
//!   options, else `%AI3_Cropmarks`, else the art size around the template box's centre
//!   (`%AI3_TemplateBox`, `%%TemplateBox` in older files), else (a file in the legacy format
//!   another app wrote) the bounding box.
//! - Document settings: Document Raster Effects Settings from the `/Document` dictionary's
//!   `AI Auto Rasterize`; the author from the header's `%%For`; swatches for the custom colors of
//!   `%%CMYKCustomColor` and `%%RGBCustomColor` that the document has none of.
//! - Variables: the variable library, XML nodes of the `/Document` dictionary: its variables, data
//!   sets, active data set and lock, without bindings; a note gives how many entries were left out
//!   ([`VariableRead`]).
//! - Objects on Path: a group whose objects' dictionaries carry ids, and a record of the
//!   `/Document` dictionary naming them, read by [`on_path`].
//! - Images: `%AI5_BeginRaster`, the colour space `XN`, `[matrix] bounds w h bits type alpha …`
//!   and the samples after `XI`. 8-bit CMYK samples keep their ink amounts in a CMYK TIFF, and
//!   an alpha channel that hides part of such an image becomes its opacity mask; other samples
//!   become an RGBA PNG. Images larger than 2^25 pixels keep their samples' channels (1-bit gray as
//!   a 1-bit PNG), and are averaged down past a target size ([`raster`]); past the hard limits, one
//!   is a box that the page's art replaces when there is a page ([`read_over_page`]).
//! - Linked images in the version 7 form (§8.2, §8.3): `(path) modified XG`, then `XF` with the
//!   operands `XI` takes and no samples. Each is a linked image of that file showing a placeholder
//!   over the image's place until its file is found. An `XF` without a file name before it, or
//!   with operands `XI` couldn't read, is left out with a warning. An `XI` after `XG` (newer files
//!   write `() 1 XG` before their embedded images) stays an embedded image.
//! - Placed (linked) files, as written in files users sent: `%AI5_BeginPlace`, `[matrix] llx lly urx
//!   ury … (path)` and the operator `` ` ``, the file's preview between `%AI26_BeginPlacedObjectPreview`
//!   and its end (`w h rowBytes 32 n `, then rows of premultiplied ARGB, the top one first), `~`. The
//!   box is as large as the file has pixels; the matrix takes them into art space from the canvas
//!   (y down, its centre the template box's), where they start at the point of art space's origin,
//!   the first row the top one. Each is a linked image showing the preview until its file is found.
//!   The dictionary after a placed PDF or `.ai` file has the page shown, from 1, in
//!   `AI11 PlacedPDFPageIndex` and its box in `AI11 PlacedPDFCropTo` (0 Art, 1 Crop, 2 Trim, 3 Bleed,
//!   4 Media, 5 Bounding Box): each is a placed document of that page and box, showing the preview
//!   until its file is found. A value outside those leaves it a linked image.
//!   The version 7 form (§16) has no `%AI5_BeginPlace`: `[matrix] llx lly urx ury (path)` and
//!   `` ` ``, then `%%IncludeFile:` or the file between `%%BeginDocument:` and `%%EndDocument`,
//!   then `~`; its box is the file's `%%BoundingBox`. Newer files have three more numbers before
//!   the path. Operands of any other shape, a second `` ` `` before `~` or an included document
//!   that doesn't end leave the placed file unread.
//! - Art with an appearance the format can't write as plain art (several fills or strokes,
//!   effects, brushes) comes first as that look, drawn (a group, or an image where an effect
//!   rasterizes it), then the object itself on `%_`
//!   lines and `1 (style) XW`: the drawn look is kept, named after the object.
//! - Patterns: `%AI3_BeginPattern: (name)`, `(name) llx lly urx ury` (the tile, in the pattern's
//!   space, y up), the tile's art (on `%_` lines in newer files), `E`, `%AI3_EndPattern`; each is
//!   read by a reader of its own and becomes a pattern swatch. An unpainted path whose dictionary
//!   marks it as the tile's box (`AIPattern_Editor_Backing_Tile_Rect`) is left out of the art. In
//!   the version 7 form (§3.4) a list
//!   of layers follows the tile, `[ (colors) @ (tile) & … ] E`, each layer's tile read from its
//!   string with the colors' state (`_ &`: the tile's box filled). A fill or stroke with one:
//!   `(name) x y scaleX scaleY angle reflect axis shear shearAxis [matrix] p` (`P`), the matrix
//!   taking the pattern's space to art space (the other operands are what it is made of). With a
//!   pattern defined in the version 7 form, the other operands are applied in order (§5.4.1) before
//!   the initial matrix.
//!
//! - Type in the legacy format (version 7 specification §15): `type To` … `TO`, read by
//!   [`legacy_text`].
//!
//! - Graphs (version 7 specification §17): `%_Gs`, the graph's data and settings on `%_` lines,
//!   its drawn art as ordinary groups, paths and type (each object followed by `%_… Go`), `%_GS`.
//!   The drawn art is read as it is. Its group keeps the data and settings when they read whole
//!   ([`graph`]); otherwise they are left out, with a note.
//!
//! - Swatches: the palette (`%AI5_BeginPalette`), read by [`super::palette`], replaces the
//!   default swatches.
//!
//! - Brushes: the brush plug-in objects and their art sections (`%AI8_BeginBrushPattern`), read by
//!   [`super::brush`], replace the default brushes when the file has any.
//!
//! - Graphic styles: the style list (`%AI9_BeginArtStyleList`), one `(name)` per style in the
//!   panel's order, each the name of an art style. The listed styles whose appearance reads
//!   ([`appearance_from_art_style`](ops::appearance_from_art_style)) replace the default graphic
//!   styles, and an object read as itself with a listed style (`1 (name) XW`) that has the
//!   style's appearance and transparency is linked to it. A `1` naming a style the list doesn't
//!   have is the object's own appearance, not a link.
//!
//! - Symbols: `%AI14_BeginSymbol` (or `%AI10_`), `(name)`, `0 A 0 Xw 0 Ae`, the art (in the
//!   symbol's space, y down), a dictionary whose `AI13PatternRegistrationType` is the registration
//!   point (1 to 9, from the top left, row by row), `%AI10_EndSymbol`; each is read by a reader of
//!   its own into the document's symbols. `%AI24_BeginSymbolList` (or `%AI10_`) lists their names
//!   in the Symbols panel's order. An instance: `/SymbolInstance : (name) /SymbolRef , a b c d tx
//!   ty /RTransformMatrix ;`, the matrix taking the symbol's art to art space; in a symbol's art,
//!   it takes it to art space about the canvas point of art space's origin, as a pattern's
//!   gradients are. Its hide and lock come before it (`Xw`, `A`) and its transparency (`Xy`) after
//!   it.
//!
//! Sections the reader doesn't need (styles, filters) are skipped; type, instances of symbols it
//! can't read, fills with a pattern it can't read, placed files without a preview and operators
//! it doesn't know are errors. So is data that ends inside a layer, a group or a compound path.

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
    LinkInfo, Node, NodeId, NodeKind, OpacityMask, PageBox, PatternDef, StrokeLayer, Unit,
};
use vectorcraft_geom::{AnchorKind, FillRule, PathData};

use super::super::ate::CANVAS_CENTRE;
use super::super::graphics::HAIRLINE;
use super::lex::{Lexer, Tok};
use super::obj::{self, NameEncoding, Obj, V};
use super::paint::{self, Global, GradientDef, Instance, Named, ShownNames};
use crate::ps;

mod art;
mod containers;
mod directives;
mod document;
mod graph;
mod masks;
mod on_path;
mod ops;
mod raster;
mod types;

pub(crate) use art::join_and;
use art::preview_png;
pub use document::{
    BindingRef, Brushes, PaletteSwatches, Symbols, VariableRead, active_artboard, artboards, bindings_note, brushes, graphic_styles, palette,
    symbols, variables,
};
pub(crate) use document::{slot_frame, slot_of};
use graph::Graph;
pub(crate) use ops::{read_art_style, read_style_list, style_moves_with};
#[cfg(test)]
pub(crate) use raster::with_image_limits;
pub(crate) use types::MAX_NODES;
pub use types::PlacedPage;
use types::{BLEND_BUDGET, BrushArtColors, FLIP, Frame, GState, Kind, PatternHead, PendingEnvelope, Place, PlacedTail, Raster, SymbolHead};
#[cfg(test)]
pub use types::{GRAPH_PLAIN, GRAPH_UNENDED, LINKED_PLACEHOLDERS, NON_NATIVE_ART, NON_NATIVE_ART_UNREAD, VERSIONED_UNENDED};
pub(super) use types::{blend_mode, blend_name};

#[path = "legacy_text.rs"]
mod legacy_text;

// The `.ai` writer defines symbols as this reader keeps them.
pub(crate) use directives::symbol_art_member;
pub(crate) use types::MAX_SYMBOLS;

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
    /// Top-level object ids and the unread kinds within them.
    pub unread: Vec<(NodeId, Vec<String>)>,
    /// Other bounds for previewless placed files whose page art uses the file box's corner.
    pub unread_elsewhere: Vec<(NodeId, Rect)>,
    /// The names of text objects, by their slot's id.
    pub slot_names: HashMap<NodeId, String>,
    /// Where the drawn looks lay that objects read with their art style took the place of (the
    /// document's coordinates): the page draws Illustrator's rendering of those styles there.
    pub restyled: Vec<Rect>,
    /// Where linked images whose pixels the data doesn't keep (`XF`) show a placeholder (the
    /// document's coordinates). A PostScript page doesn't draw them there; a PDF part draws the
    /// image's pixels.
    pub placeholders: Vec<Rect>,
    /// Placed files whose pixels the file puts at another point too: their image's id and the
    /// move, in its pixels, that places them there (the corner of the box they are written with,
    /// in place of the canvas point of art space's origin).
    pub placed_elsewhere: Vec<(NodeId, Affine)>,
    /// The placed files that are pages of PDF or `.ai` files.
    pub placed_pages: Vec<PlacedPage>,
    /// The live envelopes read from the editing data, by id, with the drawn art each took the place
    /// of: the importer puts that art back when the layers with the envelopes don't draw as the page.
    pub envelopes_drawn: Vec<(NodeId, Node)>,
    /// The warped text objects, checked against their drawn looks once their type is laid out.
    pub warped_type: super::envelope::WarpedType,
    /// The document's graphic styles are those of the file's style list, in place of the defaults.
    pub styles_read: bool,
    /// The patterns that paint the text document's characters, placed in the document, by the
    /// name of the art style that gives each: the styles that are one paint, a pattern this read
    /// (see `ate::Texts::with_patterns`).
    pub text_patterns: HashMap<String, Paint>,
    /// The file's `%AI24_LargeCanvasScale` (1 when it declares none), applied or not.
    pub canvas_scale: f64,
    /// The objects by their XML id (`AI10_ArtUID`), a text object by its slot's id: `None` for an
    /// id two objects have.
    pub xml_ids: HashMap<String, Option<NodeId>>,
    /// The objects the variable library binds to its variables, by their XML ids: the importer
    /// binds those it keeps.
    pub bindings: Vec<BindingRef>,
    /// How many objects the library binds that aren't in `bindings` ([`VariableRead::bindings_skipped`]).
    pub bindings_skipped: usize,
    pub warnings: Vec<String>,
}

/// The most XML ids of objects ([`Structure::xml_ids`]) kept.
const MAX_XML_IDS: usize = 1_000_000;

/// The most bytes of XML ids ([`Structure::xml_ids`]) kept, all ids together.
pub(super) const MAX_XML_ID_BYTES: usize = 16 << 20;

/// Record that the object `id` has the XML id `uid` in `xml_ids`, whose ids take `bytes` bytes. Two
/// objects with one id are neither of them that id's. A new id is left out when `xml_ids` holds
/// [`MAX_XML_IDS`] ids or its bytes would pass [`MAX_XML_ID_BYTES`].
pub(super) fn remember_xml_id(xml_ids: &mut HashMap<String, Option<NodeId>>, bytes: &mut usize, uid: String, id: NodeId) {
    if let Some(seen) = xml_ids.get_mut(&uid) {
        if *seen != Some(id) {
            *seen = None;
        }
    } else if xml_ids.len() < MAX_XML_IDS && bytes.saturating_add(uid.len()) <= MAX_XML_ID_BYTES {
        *bytes += uid.len();
        xml_ids.insert(uid, Some(id));
    }
}

/// [`read_in`], names that aren't UTF-8 read as Windows-1252.
#[cfg(test)]
pub fn read(data: &[u8], large_canvas: bool) -> Result<Structure, String> {
    read_in(data, large_canvas, NameEncoding::default())
}

/// Read the structure of the editing data `data`, applying the file's `%AI24_LargeCanvasScale` only
/// when `large_canvas` is set: an EPS's page is drawn at the full size, but a `.ai`'s PDF part may
/// keep the reduced one, which the PDF import reads as it is (#2031). Names that aren't UTF-8
/// (layers, objects, artboards, colors, gradients, patterns, graphic styles, swatches, brushes,
/// symbols, variables and data sets) are read in `names`, each with a warning.
pub fn read_in(data: &[u8], large_canvas: bool, names: NameEncoding) -> Result<Structure, String> {
    read_with(data, large_canvas, false, names)
}

/// [`read_in`] for a file whose page art replaces the objects the reader doesn't read (see
/// [`Reader::unread_box`]): an image past the hard limits ([`raster::Limits`]) is then such an
/// object, where [`read_in`] leaves it out with a note.
pub(crate) fn read_over_page(data: &[u8], large_canvas: bool, names: NameEncoding) -> Result<Structure, String> {
    read_with(data, large_canvas, true, names)
}

fn read_with(data: &[u8], large_canvas: bool, page_art: bool, names: NameEncoding) -> Result<Structure, String> {
    let mut r = Reader::with_names(data, names);
    r.large_canvas = large_canvas.then_some(1.0);
    r.page_art = page_art;
    r.run()?;
    r.finish()
}

/// How many objects [`read`] counts against [`MAX_NODES`] in the editing data `data`.
#[cfg(test)]
pub(crate) fn objects_read(data: &[u8]) -> Result<usize, String> {
    let mut r = Reader::with_names(data, NameEncoding::default());
    r.run()?;
    Ok(r.nodes)
}

/// [`read`] for editing data kept apart from the program `ps` it came with (an EPS's private
/// data): the program's header gives the author and the custom colors the editing data's header
/// doesn't. `page_art`: as [`read_over_page`] reads a file. `names`: the encoding of names that
/// aren't UTF-8.
pub fn read_beside(data: &[u8], large_canvas: bool, ps: &[u8], page_art: bool, names: NameEncoding) -> Result<Structure, String> {
    let header = Reader::header_of(ps, names);
    let mut r = Reader::with_names(data, names);
    r.large_canvas = large_canvas.then_some(1.0);
    r.page_art = page_art;
    // The program's custom colors have swatches of their names, which no gradient or pattern takes.
    for n in &header.custom_colors {
        r.global_names.reserve(&n.name);
    }
    r.run()?;
    r.program_header(header);
    r.finish()
}

/// The names the gradients and patterns of the editing data `data` get in the document
/// ([`ShownNames`]), with the names its other swatches can have reserved first: Registration, the
/// default swatches of both color modes when the palette the reader keeps is missing or empty, the
/// names that palette gives, the named colors, and the names without a newline of the gradients
/// and patterns. Names come only from what the reader reads: not from the alternates of versioned
/// content, the prolog, resources, sections it skips (such as symbols and the text document) or
/// what follows the trailer. The palette the reader keeps is the last one outside those and the
/// sections it reads on their own. Names that aren't UTF-8 are read in `encoding`.
fn swatch_names(data: &[u8], encoding: NameEncoding) -> (ShownNames, Option<&[u8]>) {
    /// The sections the reader reads on their own whose definitions and colors it keeps.
    const READ_APART: &[&str] = &["BeginPattern", "BeginBrushPattern", "BeginPluginObject", "BeginArtStyles", "BeginArtStyleList"];
    /// Whether the marker `m` ends what began with `begin` and ends with `end`, `depth` of its kind
    /// open inside it.
    fn closes(begin: &str, end: &str, depth: &mut usize, m: &str) -> bool {
        if !begin.is_empty() && m == begin {
            *depth += 1;
        } else if m == end {
            match depth.checked_sub(1) {
                Some(d) => *depth = d,
                None => return true,
            }
        }
        false
    }
    let mut names = ShownNames::default();
    names.reserve(vectorcraft_color::swatch::REGISTRATION);
    // The reader reads these names again, and keeps the notes on them.
    let mut decoder = obj::Names::new(encoding);
    let mut colors = paint::ColorNames::new(encoding);
    let mut lex = Lexer::new(data);
    // What the reader skips wherever it is (the marker of one of its kind nested in it, its end
    // marker, how many are open inside it, and where reading goes on when it doesn't end), the
    // top-level section being passed over (the first three, and whether the reader reads it on its
    // own), and where the palette being found starts.
    let mut skip: Option<(&str, &str, usize, Option<usize>)> = None;
    let mut section: Option<(&str, &str, usize, bool)> = None;
    let (mut palette_at, mut palette) = (None, None);
    // Where the first art styles section starts, and that section.
    let (mut styles_at, mut styles) = (None, None);
    // A pattern's name is the first string of its definition.
    let mut head = false;
    // ASCII85 data in comment lines is skipped past, as the reader skips it.
    let mut dictionaries = ops::Dictionaries::default();
    // The scan's state where the versioned content being skipped starts, to go back to.
    let mut versioned = None;
    loop {
        let before = lex.offset();
        let Some(t) = lex.next_token() else {
            // Versioned content that doesn't end is read whole, as the reader reads it.
            match skip.take() {
                Some((.., Some(at))) => {
                    lex.seek(at);
                    dictionaries = versioned.take().unwrap_or_default();
                    continue;
                }
                _ => break,
            }
        };
        let markers = skip.map(|s| (s.0, s.1)).or(palette_at.map(|_| ("", "EndPalette"))).or(section.map(|s| (s.0, s.1)));
        dictionaries.pass(&t.tok, &mut lex, markers);
        match &t.tok {
            Tok::Comment(c) => {
                let line = String::from_utf8_lossy(c);
                let m = marker(&line);
                if let Some((begin, end, depth, _)) = skip.as_mut() {
                    if closes(begin, end, depth, m) {
                        skip = None;
                    }
                } else if let Some(at) = palette_at {
                    if m == "EndPalette" {
                        palette = Some(lex.slice(at, before));
                        palette_at = None;
                    }
                } else if m.starts_with("Alternate_Content") && section.is_none_or(|s| s.3) {
                    skip = Some(("Begin_Content_if_version_gt", "End_Versioned_Content", 0, Some(lex.offset())));
                    versioned = Some(dictionaries.clone());
                } else if m == "%BeginProlog" {
                    skip = Some(("", "%EndProlog", 0, None));
                } else if m == "%BeginResource" {
                    skip = Some(("", "%EndResource", 0, None));
                } else if let Some((begin, end, depth, _)) = section.as_mut() {
                    if closes(begin, end, depth, m) {
                        if let Some(at) = styles_at.take().filter(|_| *begin == "BeginArtStyles") {
                            styles = Some(lex.slice(at, before));
                        }
                        section = None;
                    }
                } else if m == "BeginPalette" {
                    palette_at = Some(lex.offset());
                } else if let Some((begin, end)) = types::SKIPPED.iter().find(|(b, _)| *b == m) {
                    section = Some((begin, end, 0, READ_APART.contains(begin)));
                    head = m == "BeginPattern";
                    if m == "BeginArtStyles" && styles.is_none() {
                        styles_at = Some(lex.offset());
                    }
                } else if matches!(m, "%PageTrailer" | "%Trailer") {
                    break;
                }
                continue;
            }
            // A palette's names are those of the palette kept, as the palette reads them.
            _ if skip.is_some() || palette_at.is_some() || section.is_some_and(|s| !s.3) => continue,
            Tok::Str(s) | Tok::Hex(s) if std::mem::take(&mut head) => {
                let text = decoder.bytes(s);
                if !text.contains('\n') {
                    names.reserve(&text);
                }
            }
            _ => {}
        }
        colors.token(t.tok);
    }
    colors.gradients.iter().filter(|n| !n.contains('\n')).for_each(|n| names.reserve(n));
    let palette = palette.map(|p| super::palette::palette(p, &mut decoder)).unwrap_or_default();
    if palette.is_empty() {
        for mode in [ColorMode::Rgb, ColorMode::Cmyk] {
            let (swatches, groups) = vectorcraft_color::default_swatches(mode.model());
            for g in &groups {
                names.reserve(&g.name);
                g.swatches.iter().for_each(|s| names.reserve(&s.name));
            }
            swatches.iter().for_each(|s| names.reserve(&s.name));
        }
    }
    palette.own_names().for_each(|n| names.reserve(n));
    colors.names.iter().for_each(|n| names.reserve(n));
    (names, styles)
}

/// What the brushes and brush art read so far give a brushed stroke.
#[derive(Default)]
struct BrushesKept {
    /// The counts of brushes and art sections when this was worked out.
    read: (usize, usize),
    /// Which of [`Reader::brush_drafts`] are kept as brushes.
    kept: Vec<bool>,
    /// The art an Art brush draws as the file does.
    bending: HashSet<String>,
}

struct Reader<'a> {
    lex: Lexer<'a>,
    stack: Vec<V>,
    /// Where the `[` marks are on the stack, innermost last (some may have gone with operands).
    marks: Vec<usize>,
    gs: GState,
    frames: Vec<Frame>,
    path: BezPath,
    /// The kind of the anchor each segment of `path` ends on, in order: smooth for a lower-case
    /// segment operator, corner for an upper-case one (version 7 specification §5.3.2).
    path_kinds: Vec<AnchorKind>,
    points: usize,
    /// The path is a clipping path (`W`).
    clip_next: bool,
    doc: Document,
    to_doc: Option<Affine>,
    cropmarks: Option<Rect>,
    artboards: Vec<document::AiArtboard>,
    /// The index in `artboards` of the active artboard (`CropAreaActive`).
    active_board: Option<usize>,
    /// The user the header's `%%For` names.
    author: Option<String>,
    /// The custom colors the header's `%%CMYKCustomColor` and `%%RGBCustomColor` define.
    custom_colors: Vec<Named>,
    /// The last header comment defined custom colors (CMYK when true), which a `%%+` line continues.
    custom_more: Option<bool>,
    /// The header ended (`%%EndComments`).
    header_done: bool,
    /// The `/Document` dictionary's variable library.
    variables: VariableRead,
    /// The Objects on Path records of the `/Document` dictionary and the ids of the objects.
    on_path: on_path::OnPath,
    gradients: HashMap<String, Gradient>,
    /// The names the gradients and patterns get in the document, by their names in the file.
    global_names: ShownNames,
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
    /// The samples of the image being read are on `%_` lines (`%_%%BeginData`): it is the copy of
    /// a styled object there, as the other objects on those lines are.
    image_hidden: bool,
    /// The file `XG` names for the image after it (`XF`), until an image is read.
    image_link: Option<String>,
    /// The placed file being read (`%AI5_BeginPlace` … `~`).
    place: Option<Place>,
    /// The placed files just read, until the dictionary after each: at most one for each depth of
    /// frames, the innermost last.
    placed_tails: Vec<PlacedTail>,
    /// The last object read is the last child of the innermost frame, and nothing started since.
    after_object: bool,
    /// The last object read is an image, and only the operators that end its raster (`XH`, `Xr`,
    /// `N`) came since: its name and opacity mask still follow.
    image_tail: bool,
    nodes: usize,
    /// What layers that show have that this reader doesn't read (errors).
    unsupported: BTreeSet<String>,
    /// Operators the reader doesn't know on layers that show.
    unknown: BTreeSet<String>,
    /// What layers that don't show have that this reader doesn't read.
    hidden_unread: BTreeSet<String>,
    unread: Vec<(NodeId, Vec<String>)>,
    /// The art boxes (symbol space) of symbols whose definition can be measured but not read
    /// whole, by their name in the file: their instances come in as the page draws them.
    symbol_bounds: HashMap<String, Rect>,
    /// The symbols read, by their name in the file → their name in the document.
    symbol_names: HashMap<String, String>,
    /// The names the symbols read have in the document, and for each name read more than once,
    /// the number its next copy is tried with.
    symbol_taken: HashSet<String>,
    symbol_suffix: HashMap<String, usize>,
    /// The art boxes (symbol space) of symbols read, by their name in the file, once measured for
    /// an instance that comes in as the page draws it.
    symbol_art_bounds: HashMap<String, Option<Rect>>,
    /// The symbol list's names, in the Symbols panel's order (`%AI24_BeginSymbolList`).
    symbol_list: Option<Vec<String>>,
    /// The symbols read were checked for instances of symbols the file doesn't have.
    symbols_checked: bool,
    /// This reader reads a symbol's definition (see [`Reader::define_symbol`]).
    symbol: Option<SymbolHead>,
    /// Depth shared by pattern and symbol readers.
    subreader_depth: usize,
    pending_unread_fill: bool,
    pending_unread_stroke: bool,
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
    /// The objects read as their drawn look in the art of [`Reader::brush_art`], by name: counted in
    /// [`Reader::drawn_looks`] once a brush that is kept uses the art.
    brush_art_looks: HashMap<String, usize>,
    /// The objects read as their drawn look in the art of the patterns kept, by the name the document
    /// has the pattern by: a pattern defined again counts only its last definition.
    pattern_looks: HashMap<String, usize>,
    /// Nodes read from `%_` lines.
    hidden_ids: HashSet<NodeId>,
    /// The blend part each object's dictionary names (`ShapeBlendPathType`, `ShapeBlendPathID`).
    blend_roles: super::blend::Roles,
    /// A blend's record being read from the `%` lines after its `XP`: its length and hex digits.
    blend_record: Option<(usize, Vec<u8>)>,
    /// How many more objects blends may lay out in this import ([`super::blend::live_blend`]).
    blend_budget: usize,
    /// The group on `%_` lines an envelope's dictionary just took (see [`Reader::drop_envelope_art`]).
    envelope_art: Option<Node>,
    /// Envelopes waiting for their drawn art, innermost last.
    envelopes: Vec<PendingEnvelope>,
    /// How many more envelopes and warped text objects this import may draw to check
    /// ([`super::envelope::drawn_alike`]).
    envelope_checks: usize,
    /// How far the live envelopes read so far draw from their drawn art ([`super::envelope::Spent`]).
    envelope_spent: super::envelope::Spent,
    /// The text objects read with a Warp ([`super::envelope::WarpedType`]).
    warped_type: Vec<super::envelope::WarpedLook>,
    /// The envelopes of type read with their drawn art, settled once the type is laid out
    /// ([`super::envelope::TypedEnvelope`]).
    typed_envelopes: Vec<super::envelope::TypedEnvelope>,
    /// The live envelopes made so far and the drawn art each took the place of
    /// ([`Structure::envelopes_drawn`]).
    envelopes_drawn: Vec<(NodeId, Node)>,
    /// A group just ended, its state (`Xy`, `Xd`, `XW`) not yet read.
    group_ended: bool,
    /// A gradient mesh just ended (`/End X!`), its own state (`Xy`) not yet read: also after its
    /// dictionary (its name, its opacity mask and the mask's art).
    mesh_ended: bool,
    /// A layer's own state (`Xy` after its art, before `LB`), until something says the art moves on.
    layer_state: Option<(f32, BlendMode, bool, Knockout, bool)>,
    /// The state a mesh or blend had before an `Xy` right after its end, which is the layer's
    /// instead when the layer's `LB` follows rather than their own `6 () XW`.
    mesh_undo: Option<(f32, BlendMode, bool, Knockout, bool)>,
    /// The state a layer's last group or compound path had before an `Xy` right after its end,
    /// which is the layer's instead when the layer's own `7 () XW` follows.
    container_undo: Option<(f32, BlendMode, bool, Knockout, bool)>,
    /// The fills of compound clipping paths whose subpaths paint differently, as compound paths of their own (drawn
    /// under what they clip).
    clip_fills: HashSet<NodeId>,
    /// Groups closed as an object's drawn look (`9 () XW`).
    looks: HashSet<NodeId>,
    /// Straight `(N) *` guide paths painted directly on a layer, by id: their two points in art
    /// space. [`Reader::finish`] makes the ruler guides among them guides of their layer.
    ruler_guides: HashMap<NodeId, (Point, Point)>,
    /// Legacy compound member groups, even when followed by the generic `9 () XW` marker.
    compound_groups: HashSet<NodeId>,
    /// Groups of a text object and its look (see [`Self::style_marker`]), which aren't a look.
    text_looks: HashSet<NodeId>,
    /// The gradient mesh being read (`/Mesh X!` … `/End X!`), and whether it started on a `%_` line.
    mesh: Option<(super::mesh::MeshReader, bool)>,
    /// The art styles (`%AI9_BeginArtStyles`), by name.
    styles: HashMap<String, super::style::ArtStyle>,
    /// The style list (`%AI9_BeginArtStyleList`): the Graphic Styles panel's names, in its order.
    style_list: Vec<Vec<u8>>,
    /// The objects read as themselves with the style their `1 (style) XW` names: their id and the
    /// style's name, as written.
    style_links: Vec<(NodeId, String)>,
    /// The swatches (`%AI5_BeginPalette`).
    palette: super::palette::Palette,
    /// See [`Structure::restyled`].
    restyled: Vec<Rect>,
    /// See [`Structure::placeholders`].
    placeholders: Vec<Rect>,
    /// See [`Structure::placed_elsewhere`].
    placed_elsewhere: Vec<(NodeId, Affine)>,
    /// See [`Structure::placed_pages`].
    placed_pages: Vec<PlacedPage>,
    unread_elsewhere: Vec<(NodeId, Rect)>,
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
    /// [`Structure::xml_ids`].
    xml_ids: HashMap<String, Option<NodeId>>,
    /// The bytes of the ids in `xml_ids`.
    xml_id_bytes: usize,
    /// The artboard's size and centre in files without artboards or crop marks.
    art_size: Option<(f64, f64)>,
    template_center: Option<(f64, f64)>,
    /// `%AI24_LargeCanvasScale`: a Large Canvas document's editing data is written at 1/N scale
    /// (see [`Reader::ensure_space`]); 1 (no scaling) when the file doesn't declare one, `None`
    /// when the declared factor is not applied ([`read`]).
    large_canvas: Option<f64>,
    /// The file has page art to replace the objects that aren't read ([`read_over_page`]).
    page_art: bool,
    /// The declared `%AI24_LargeCanvasScale` (1 without one), applied or not.
    canvas_scale: f64,
    /// `%AI5_RulerUnits` from the header (Illustrator 7 spec table 3, plus centimeters and pixels).
    ruler_units: Option<u8>,
    /// This reader reads a pattern's definition (see [`Reader::define_pattern`]).
    pattern: Option<PatternHead>,
    /// The tile (y down) of each pattern read, by name.
    pattern_tiles: HashMap<String, Rect>,
    /// The patterns read from a definition in the version 7 form, whose fills apply their other
    /// operands before the matrix (see [`Reader::pattern_paint`]).
    v7_patterns: HashSet<String>,
    /// The legacy text object being read (`To` … `TO`), and the modal type attributes.
    text: Option<legacy_text::LegacyText>,
    type_state: legacy_text::TypeState,
    /// How names that aren't UTF-8 are read, and the notes on them.
    names: obj::Names,
    /// The file has brush objects (the brush manager's order or a brush).
    brush_objects: bool,
    /// The brush manager's order.
    brush_order: Option<Vec<(super::brush::Tool, String)>>,
    /// The brushes read, in the order of their objects.
    brush_drafts: Vec<super::brush::BrushDraft>,
    /// The art of the brushes (`%AI8_BeginBrushPattern`), by name.
    brush_art: HashMap<String, Node>,
    /// The colors of the art in [`Reader::brush_art`], by name.
    brush_art_colors: HashMap<String, BrushArtColors>,
    /// The brush art sections read so far, repeated names included.
    brush_art_sections: usize,
    /// What the brushes and brush art read so far give a brushed stroke.
    brushes_kept: BrushesKept,
    /// Brush objects that weren't read.
    brushes_unread: usize,
    /// The graph being read (see [`graph`]).
    graph: Option<Graph>,
    /// The multi-layer mask being read (`Mb` … `MB`), which goes on past a layer's end.
    multi_mask: Option<masks::MultiMask>,
}

/// A section comment's marker without its `AI<version>_` prefix and value (`AI14_BeginSymbol` →
/// `BeginSymbol`); DSC comments (`%%BeginProlog`) keep their `%`.
fn marker(c: &str) -> &str {
    let word = c.split(|ch: char| ch == ':' || ch.is_whitespace()).next().unwrap_or(c);
    word.strip_prefix("AI").map(|r| r.trim_start_matches(|ch: char| ch.is_ascii_digit())).and_then(|r| r.strip_prefix('_')).unwrap_or(word)
}

/// Is `name` that of a text slot for type that a look draws ([`document::LOOK_SLOT`])?
pub(crate) fn is_look_slot(name: Option<&str>) -> bool {
    name == Some(document::LOOK_SLOT)
}

impl<'a> Reader<'a> {
    /// A reader of the editing data `data` as a whole, with the names its gradients and patterns
    /// get in the document and its art styles found first ([`swatch_names`]): the symbols defined
    /// before the art styles section use them too. Names that aren't UTF-8 are read in `encoding`.
    fn with_names(data: &'a [u8], encoding: NameEncoding) -> Self {
        let mut r = Self::new(data);
        r.names = obj::Names::new(encoding);
        let (names, styles) = swatch_names(data, encoding);
        r.global_names = names;
        if let Some(section) = styles {
            r.styles = super::style::art_styles_in(section, &mut r.names);
        }
        r
    }

    fn new(data: &'a [u8]) -> Self {
        // An envelope's saved margins and its check against its drawn art measure type by its
        // glyphs, as the app maps it.
        vectorcraft_text::register_envelope_ink();
        let mut doc = Document::new(1.0, 1.0);
        doc.layers.clear();
        Self {
            lex: Lexer::new(data),
            stack: Vec::new(),
            marks: Vec::new(),
            gs: GState::default(),
            frames: Vec::new(),
            path: BezPath::new(),
            path_kinds: Vec::new(),
            points: 0,
            clip_next: false,
            doc,
            to_doc: None,
            cropmarks: None,
            artboards: Vec::new(),
            active_board: None,
            author: None,
            custom_colors: Vec::new(),
            custom_more: None,
            header_done: false,
            variables: VariableRead::default(),
            on_path: on_path::OnPath::default(),
            gradients: HashMap::new(),
            global_names: ShownNames::default(),
            gradient_def: None,
            instance: None,
            pending_fill: None,
            pending_stroke: None,
            spots: Vec::new(),
            stop_spots: Vec::new(),
            raster: None,
            image_hidden: false,
            image_link: None,
            place: None,
            foreign_data: None,
            after_object: false,
            image_tail: false,
            nodes: 0,
            unsupported: BTreeSet::new(),
            unknown: BTreeSet::new(),
            hidden_unread: BTreeSet::new(),
            unread: Vec::new(),
            symbol_bounds: HashMap::new(),
            symbol_names: HashMap::new(),
            symbol_taken: HashSet::new(),
            symbol_suffix: HashMap::new(),
            symbol_art_bounds: HashMap::new(),
            symbol_list: None,
            symbols_checked: false,
            symbol: None,
            subreader_depth: 0,
            pending_unread_fill: false,
            pending_unread_stroke: false,
            bbox: None,
            hires: None,
            template: None,
            cmyk_colors: 0,
            rgb_colors: 0,
            color_model: None,
            warnings: Vec::new(),
            drawn_looks: 0,
            brush_art_looks: HashMap::new(),
            pattern_looks: HashMap::new(),
            hidden_ids: HashSet::new(),
            blend_roles: HashMap::new(),
            blend_record: None,
            blend_budget: BLEND_BUDGET,
            envelope_art: None,
            envelopes: Vec::new(),
            envelope_checks: super::envelope::MAX_CHECKED,
            envelope_spent: super::envelope::Spent::default(),
            warped_type: Vec::new(),
            typed_envelopes: Vec::new(),
            envelopes_drawn: Vec::new(),
            clip_fills: HashSet::new(),
            looks: HashSet::new(),
            ruler_guides: HashMap::new(),
            compound_groups: HashSet::new(),
            text_looks: HashSet::new(),
            group_ended: false,
            mesh_ended: false,
            layer_state: None,
            mesh_undo: None,
            container_undo: None,
            styles: HashMap::new(),
            style_list: vec![],
            style_links: vec![],
            palette: super::palette::Palette::default(),
            restyled: vec![],
            placeholders: vec![],
            placed_elsewhere: vec![],
            placed_pages: vec![],
            placed_tails: vec![],
            unread_elsewhere: vec![],
            mesh: None,
            cmyk_cache: HashMap::new(),
            done: false,
            obj_hidden: false,
            slot_names: HashMap::new(),
            xml_ids: HashMap::new(),
            xml_id_bytes: 0,
            art_size: None,
            template_center: None,
            large_canvas: None,
            page_art: false,
            canvas_scale: 1.0,
            ruler_units: None,
            pattern: None,
            pattern_tiles: HashMap::new(),
            v7_patterns: HashSet::new(),
            text: None,
            type_state: legacy_text::TypeState::default(),
            names: obj::Names::default(),
            brush_objects: false,
            brush_order: None,
            brush_drafts: Vec::new(),
            brush_art: HashMap::new(),
            brush_art_colors: HashMap::new(),
            brush_art_sections: 0,
            brushes_kept: BrushesKept::default(),
            brushes_unread: 0,
            graph: None,
            multi_mask: None,
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
                Tok::Comment(c) => self.comment(c, hidden)?,
                Tok::Data(d) => self.image(d, hidden)?,
                Tok::Preview(d) => {
                    if let Some(p) = self.place.as_mut() {
                        p.preview = preview_png(d);
                    }
                }
                // A placed file's document, included whole: its art isn't read here. When no
                // `%%EndDocument` ends it, the placed file isn't read.
                Tok::Document(d) => {
                    if let Some(p) = self.place.as_mut() {
                        p.broken |= d.is_none();
                    }
                }
            }
        }
        // A pattern's or symbol's reader opens a group of its own and checks it is the only frame
        // left. In the document, a container still open after the last layer holds art no layer has.
        let in_layer = self.frames.iter().any(|f| matches!(f.kind, Kind::Layer(_)));
        if self.subreader_depth == 0 && !in_layer && self.frames.iter().any(|f| !matches!(f.kind, Kind::Obj(_) | Kind::Mask)) {
            return Err("it ends inside a group or compound path".into());
        }
        if self.frames.iter().any(|f| matches!(f.kind, Kind::Layer(_))) {
            return Err("it ends inside a layer".into());
        }
        if self.multi_mask.is_some() {
            return Err("it ends inside a multi-layer mask".into());
        }
        // A placed file that no `~` ends isn't read.
        if let Some(open) = self.place.take() {
            self.unreadable("placed files", open.hidden);
        }
        Ok(())
    }
}
