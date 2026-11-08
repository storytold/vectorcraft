//! The editing copy of the art that a PDF-compatible `.ai` file carries in its pages'
//! `PieceInfo` ([`vectorcraft_pdf::PrivateData`]): the document's layers, sublayers, groups, paths,
//! colors, swatches and artboards in the form [`crate::import`]'s `.ai` reader reads, so that apps
//! that open `.ai` files from that copy get them as editable objects.
//!
//! The form is the one the published specification documents (Adobe Illustrator File Format
//! Specification, version 7.0, see [`crate::legacy_ai`]) for paths (§5.3), colors (§5.4),
//! groups and compound paths (§5.3.5), guides (§6) and layers (§9), with the newer forms the reader
//! reads from files users sent:
//!
//! - Header comments: `%%Creator` (naming the format's app, see [`CREATOR_TEXT`]), `%%For` (the
//!   document's author), `%%Canvassize`, `%%BoundingBox`,
//!   `%%HiResBoundingBox`, `%AI5_FileFormat 14.0`, `%AI7_ImageSettings`, the first artboard as
//!   `%AI3_Cropmarks` and `%AI3_TileBox`, the canvas's center as `%AI3_TemplateBox`,
//!   `%AI5_ArtSize 14400 14400`, `%AI5_RulerUnits`, `%AI24_LargeCanvasScale` (1, or 10 when the
//!   artboards together don't fit on the canvas: the art, boxes and artboards are then written at
//!   1/10 of their size), `%AI9_ColorModel` (1 RGB, 2 CMYK),
//!   `%AI5_ArtFlags`, `%AI5_TargetResolution`, `%AI5_NumLayers`, `%AI5_OpenViewLayers` (a digit for
//!   each layer, 7 shown and 6 hidden), `%AI7_GridSettings` (§2.1.1), and the version comment
//!   `%%AI8_CreatorVersion` ([`CREATOR_VERSION`]).
//! - Setup: the gradients the art uses as `n Bn` and a definition of each (§3.5: `%AI5_BeginGradient:
//!   (name)`, `(name) type stops Bd`, `[`, each stop on a `%_` line as `color style opacity 6 midpoint
//!   rampPoint Bs`, `BD`, `%AI5_EndGradient`), those only the swatches use in a non-printing block
//!   (§14: `%AI5_Begin_NonPrinting`, `Np`, `%AI5_End_NonPrinting--`); the patterns the same way
//!   (§3.4: `%AI3_BeginPattern: (name)`, `(name) llx lly urx ury`, the tile's art on `%_` lines, `E`,
//!   `%AI3_EndPattern`); the swatches (`%AI5_BeginPalette`, `0 0 Pb`, the Registration swatch, each
//!   swatch's color operator, gradient instance or pattern operator and `(name) Pc`, a color group as
//!   `0 (name) count Pg` before its swatches, `PB`), then the artboards as the `ArtboardArray` of the
//!   `/Document` dictionary's `/Recorded` part and the canvas size as its `AIDocumentCanvasSize`, on
//!   `%_` lines.
//! - Variables (`writer::variables`): after the canvas size, the variable library as XML nodes
//!   (`/XMLNode`: `xmlnode-nodename`, `xmlnode-nodevalue`, `xmlnode-nodetype`, `xmlnode-children`,
//!   `xmlnode-attributes`) of an XML document `#document`: a `metadata` element holding
//!   `variableSets`, one `variableSet` (`locked`: `none`, or `variableAndTypes` when the variables
//!   are locked), its `variables` (a `variable` element with `varName` and `trait`: `textcontent`,
//!   `visibility`, `fileref` or `graphdata`, none for a variable of unknown kind; a variable bound to
//!   objects declared once for each, its `docRef` the object's XML id, `/XMLUIDREF`) and its
//!   `v:sampleDataSets` (`i:activeDataSet`, the data set shown) of `v:sampleDataSet` elements
//!   (`dataSetName`), each holding an element named after each variable it gives a value: a `p`
//!   element for each line of a text value, a `/Bool` text whether the objects show, a linked file's
//!   `file://` URL or path. An object bound to a variable without a name gets an XML id and an empty
//!   `AIArtName`. Bindings to layers, to type written as outlines, to graphs and to linked images
//!   written as other objects, and graph values, values that don't fit their variable, linked files
//!   that aren't absolute paths, paths from the root that the reader reads as other paths (`/`
//!   alone, `/C:…`), drive and network paths with white space at either end, values of variables
//!   whose names have a colon, and a data set shown that isn't written, are left out, as is what
//!   the reader leaves out
//!   (variables and data sets whose names are empty, repeated, longer than 255 characters or start
//!   or end with white space, variables past 256, data sets past 1,000, bindings past 100,000 or to XML ids longer than
//!   255 characters, names and values past 8,388,608 bytes in all), and values past 200,000 text
//!   lines and values in all.
//! - Layers: `%AI5_BeginLayer`, `visible preview unlocked printing dimmed 0 1 colorIndex r g b 0
//!   dimPercent clipping Lb` (14 operands; a color that isn't one of the preset layer colors has the
//!   index -1; `clipping` is 1 for a clipping layer), `(name) Ln`, the layer's name as an XML id in an
//!   `/ArtDictionary`, its art and sublayers, `LB`, `%AI5_EndLayer--`.
//! - Colors: `g` gray, `k` CMYK, `c m y k r g b Xa` RGB (Lab is written as RGB); a spot color
//!   `c m y k (name) tint x` (CMYK) or `c m y k r g b|L a b (name) tint 1|2 Xx` (RGB or Lab); a
//!   global process color `… (name) tint model Xk`; Registration `1 1 1 1 ([Registration]) tint
//!   Xs`; a pattern `(name) 0 0 1 1 0 0 0 0 0 [matrix] p` (`P`), the matrix taking the pattern's
//!   space (its tile's lower left corner the origin) to art space. Upper case paints the stroke.
//! - Gradients (§3.5.3): around the painting operator of a path (of a compound path's last path),
//!   `Bb` (`1 Bb` for the stroke), a radial gradient's focal point `x y angle fraction Bh`, then
//!   `flag (name) x y angle length a b c d tx ty 1 Bg`, which places the gradient on the path: a
//!   linear one with the identity matrix, `x` the start and `length` the length as fractions of the
//!   path's extent along `angle`; a radial one with the matrix taking the unit circle onto its
//!   extent ellipse, `x` and `y` its center as fractions of the path's bounds there and `length` its
//!   radius as a multiple of the one the bounds give (`writer::paints::placement`). The imaging data
//!   follows (a linear gradient's `a b c d tx ty Xm`, its caps `Bc` and a `Bm` for each pair of
//!   stops; a radial one's `Bm`), then the operator and `0 BB` (`2 BB` or `1 BB` when the path is
//!   stroked too; a fill's instance ends `2 BB` before a stroke's). In a pattern's tile the matrices
//!   are in the canvas's space (y down, from art space's origin there).
//! - State before each object: locked `A`, hidden `Xw` (`1` or `0`). An object's name follows it as
//!   an `/ArtDictionary` whose `AI10_ArtUID` is the name as an XML id. Guides are the path
//!   followed by `(N) *` (`(n) *` when closed), without the `(N) *` that §6 also puts before it.
//! - Transparency: `mode opacity isolate knockout shape Xy` (the blending mode's number, Knockout
//!   Group 0 off, 1 on, 2 neutral), written when it changes: in a path's paint after its colors,
//!   before a guide, and after a group's end (`U` or `Q`) followed by `0 0 Xd` and `6 () XW`; a
//!   group without transparency of its own ends `9 () XW`. A layer's comes after its art: `Xy`,
//!   `0 0 Xd`, `7 () XW`, before `LB`.
//! - Compound paths: `*u`, the subpaths, `*U` (§5.3.5). Clipping groups: `q`, the art, the clipping
//!   path last with `h` (or `H` when open) and `W` before its painting operator, `Q` (§5.6.2); a
//!   clipping layer's clipping path is written the same way, last in the layer.
//! - Opacity masks: an `AI9_Mask` entry in the object's `/ArtDictionary`, a `/Mask` dictionary
//!   holding the mask's art between `X=` and `X+` on `%_` lines, `(Art) ,`, and `Clipping`,
//!   `Inverted`, `Disabled` and `Linked` as `/Bool` entries.
//! - Embedded images (§8): `%AI5_File:`, `%AI5_BeginRaster`, `() 1 XG`, the color space
//!   (`/DeviceGray`, `/DeviceRGB` or `/DeviceCMYK`) with `XN`, `[matrix] w h 0 Xh`, `[matrix] 0 0
//!   w h w h 8 type alpha flags 1 0 4 4 0 0` (type 1 gray, 3 RGB, 4 CMYK; alpha 1 with an alpha
//!   channel; flags 0 gray, 1 RGB, 2 CMYK, 16 more with an alpha channel), `%%BeginData:` counting
//!   `XI`, its line end and the color samples, `XI`, the samples in binary (8 bits, rows from the
//!   top, CMYK images with their ink amounts) followed by the alpha channel, `%%EndData`, `XH`,
//!   `%AI5_EndRaster`, `N`. The matrix (to ten decimals) is an art-space matrix applied to the
//!   pixels with their rows running downwards. In a part on `%_` lines, `XI` and the samples are
//!   not, and `%_%%EndData` ends them.
//! - Linked images (version 7 specification §16, with the newer sections): `%AI5_BeginPlace`,
//!   `%%FileType: 1`, `[matrix] llx lly urx ury 4 4 0` and `(path)` with `` ` ``, `%%IncludeFile:`,
//!   the folder relative to the document's between `%AI28_BeginPlacedRelativePath` and its end,
//!   a preview of at most 256 pixels a side between `%AI26_BeginPlacedObjectPreview` and its end
//!   (`w h rowBytes 32 17 `, then rows of premultiplied ARGB), each in versioned content, `~`,
//!   `%AI5_EndPlace`. The box is the file's pixels at art space's origin on the canvas; the matrix
//!   takes it into art space. In opacity masks, pattern tiles and symbols, linked images are
//!   embedded.
//! - Type: a text object `/AI11Text :` naming its story and frame (`/StoryIndex`, `/FrameIndex`),
//!   for area type and type on a path with its frame's path (`/Art :` … `; /ConfiningPath ,`), and
//!   the text document after the document data (`%AI11_BeginTextDocument`, see
//!   `writer::texts`), in the form the reader's `ate.rs` reads: the characters, the fonts, sizes,
//!   leading, tracking, baseline shift, scales, solid fills and strokes and capitals of each run,
//!   the alignment, indents, spacing, hyphenation and direction of each paragraph, and the frame:
//!   point type's point and matrix, area type's frame, rows, columns, gutters and first baseline,
//!   type on a path's path and span, vertical type, and the frames a threaded story flows through.
//! - Symbols, as the reader reads them ([`crate::import`]'s `.ai` reader, `read.rs`): every symbol
//!   of the document defined in the setup after the patterns, in a non-printing block
//!   (`%AI14_BeginSymbol`, `(name)`, `0 A`, `0 Xw`, `0 Ae`, the art in the symbol's space with y
//!   down, the registration point as `AI13PatternRegistrationType` 1 to 9 from the top left, row by
//!   row (VectorCraft's numbering, in the order the scripting reference lists the points), in the
//!   art's dictionary, `%AI10_EndSymbol`), each after those its art uses; the symbol
//!   list after the palette (`%AI24_BeginSymbolList`, the names in the Symbols panel's order,
//!   `%AI24_EndSymbolList`); each instance as `/SymbolInstance :`, `(name) /SymbolRef ,`, the
//!   matrix taking the art to art space as `a b c d tx ty /RTransformMatrix`, `;`, then its name
//!   and opacity mask and a state block as a group's with its opacity and blending mode (an
//!   instance with isolation, knockout or knockout shape of its own is written as its art). The
//!   definitions are at most the reader's 4,096. In a symbol's art, an instance's matrix and
//!   a gradient's take the art to the canvas's space turned y up about art space's origin there.
//! - Art styles, as the reader reads them (`style.rs`, `read/ops.rs`): a path, compound path or
//!   group with live effects or linked to a graphic style is written as its drawn look (`u`, its
//!   vector art as its geometry effects leave it, without shadows, glows or feathers, `U`, its
//!   transparency, `0 0 Xd`, `6 () XW`), then the object itself on `%_` lines (its points painted
//!   with nothing, or with its gradient fill; a group with its members) with its name and opacity
//!   mask, then `1 (style) XW`. The styles are written in a non-printing block after the symbols
//!   (`%AI9_BeginArtStyles`, each `/KnownStyle :` with its name and a filter tree: a chain of the
//!   stack of fills and strokes with the contents among them, the effects and the blend filter), the
//!   Graphic Styles panel's first; the panel's order follows the palette (`%AI9_BeginArtStyleList`).
//!   A fill filter fills by the non-zero rule (`0 /Bool (UseEvenOdd)`), so a filled object that
//!   fills by the even-odd rule is written as its drawn look. A style is written only when the
//!   reader reads it back as the object's appearance (or the graphic style's).
//!
//! Lines end in CR LF. Strings are written in UTF-8. Art space is ruler space: y up, its origin at
//! the first artboard's top-left corner. On the canvas ([`CANVAS_SIZE`] points square, y down) the
//! artboards together are centered; each artboard's `RulerOrigin` is its top-left corner there, and
//! `%AI3_TemplateBox` holds the canvas's center in art space. What the form above can't hold is written as plain filled and stroked paths, each such
//! loss a warning: type the text document can't hold (in opacity masks, pattern tiles, symbols and
//! art cut to a clipping object, mirrored, turned or unevenly scaled area type and type on a path, inline graphics, ruby,
//! gradient or pattern paint, an appearance of its own) as its outlines and its inline graphics'
//! art, the character and paragraph attributes the text document doesn't hold, freeform gradients, gradients
//! along or across a stroke and gradients without stops, with more than 256 stops or without extent
//! as their average color
//! (gradients that dither or mix perceptually are written as classic ones, and gradient stops linked
//! to global process colors as their colors), pattern fills whose tiles aren't a grid, whose art reaches past
//! the tile or is painted with patterns, and pattern fills in symbols, as their tiles cut
//! to the shape, clipping groups and clipping layers whose clipping object isn't a path,
//! and the clipping groups inside them, as their art cut to that object (its fill cut too, its
//! stroke not; an image in a clipping group of the region it is cut to), images that can't be
//! decoded or have more than 33,554,432 pixels as their outlines filled mid gray, images of 16 bits
//! a channel at 8 bits, linked images in opacity masks, pattern tiles and symbols as embedded images,
//! symbol instances with a fill, appearance changes, isolation, knockout or knockout shape of their
//! own, instances of symbols with 9-slice guides or pattern paint, instances in pattern tiles or in art cut to a
//! clipping object, instances scaled almost flat, instances that turn, mirror or scale symbols holding
//! type, live objects, instances written as their art, shapes with rounded live corners or strokes
//! written as outlines or brush art,
//! instances that scale their symbol more than 100 times, instances on the layers of a large canvas
//! and instances of
//! symbols past 4,096 as their art (symbol art past [`vectorcraft_doc::MAX_SYMBOL_NODES`] objects
//! in all left out), instances of symbols the document doesn't have as empty groups, symbols' 9-slice
//! guides, export types and dynamic type left out, live objects (placed documents included) as their art,
//! strokes a plain line can't draw
//! as their filled outlines, several fills or strokes as a group of a path for each, freeform
//! gradient swatches, gradient swatches of more than 256 stops, swatches of patterns written as
//! tiles and the placement of pattern swatches that move, turn or scale their pattern, the opacity and blending modes of single fills and strokes, opacity masks on
//! layers and opacity masks nested too deep, and, on objects written without an art style, live
//! effects other than geometry effects left out; graphic styles the reader wouldn't read, links to
//! graphic styles on objects not written with them, and every art style on a large canvas, are left
//! out.
//!
//! The data streams hold the program compressed with Zstandard after `%AI24_ZStandard_Data`, with
//! the version numbers of the newest files users sent ([`private_data`]).

use std::fmt::Write;
use std::sync::Arc;

use vectorcraft_doc::{Document, Node, Unit};
use vectorcraft_geom::{Affine, Rect};
use vectorcraft_pdf::PrivateData;

use crate::ps;

mod writer;

use writer::Writer;

/// The version comment's value: the creator version of the newest files users sent.
pub const CREATOR_VERSION: &str = "30.7.0";
/// The `PieceInfo` numbers that go with Zstandard-compressed data: container 12, creator 30,
/// round trip 24, stream type 2.
const CONTAINER_VERSION: u32 = 12;
const CREATOR: u32 = 30;
const ROUNDTRIP_VERSION: u32 = 24;
const ROUNDTRIP_STREAM_TYPE: u32 = 2;
/// What starts the data streams of Zstandard-compressed data.
const ZSTD_MARKER: &[u8] = b"%AI24_ZStandard_Data";
/// `%%Canvassize`: the side of the canvas in points, as the files users sent have it.
const CANVAS_SIZE: u32 = 16383;
/// The scale of a large canvas: its sides are ten times the canvas's, as the format's large canvas
/// documents have them (2270 inches square).
const LARGE_CANVAS_SCALE: f64 = 10.0;
/// `%AI5_ArtSize`, as the files users sent have it.
const ART_SIZE: u32 = 14400;
/// `%AI5_ArtFlags` (§2.1.1, Table 4), as the files users sent have them: page setup, print tiles,
/// placed images, preview patterns, split long paths, tile full pages, printer's default
/// screens, auto default screens, compatible gradient printing.
const ART_FLAGS: &str = "0 0 0 1 0 0 1 0 0";
/// `%AI5_TargetResolution` (§2.1.1).
const TARGET_RESOLUTION: u32 = 800;
/// The gridlines' gray: VectorCraft's default grid color (200 of 255).
const GRID_GRAY: f64 = 200.0 / 255.0;

/// `%%Creator`: the creator line VectorCraft writes in the header of the editing data. It names
/// VectorCraft and the format's app, and holds no version number; the version is written on its
/// own in `%%AI8_CreatorVersion` ([`CREATOR_VERSION`]).
const CREATOR_TEXT: &str = "VectorCraft, editing data for Illustrator"; // brand-ok: the creator line VectorCraft writes

/// The first line of a PostScript file (the DSC version comment).
const PS_HEADER: &str = "%!PS-Adobe-3.0"; // brand-ok: the PostScript header comment

/// What to write in the header.
#[derive(Clone, Debug, Default)]
pub struct AiEditingOptions {
    /// `%%Title`.
    pub title: String,
    /// `%%CreationDate`, Unix seconds.
    pub created: Option<i64>,
}

/// The editing data and what it couldn't hold.
#[derive(Clone, Debug, Default)]
pub struct AiEditingOutput {
    /// The program: header comments, setup, layers, trailer.
    pub program: Vec<u8>,
    /// Its header comments, up to `%%EndComments`.
    pub header: Vec<u8>,
    pub warnings: Vec<String>,
}

/// Write `doc` as the editing data of a `.ai` file: every artboard and layer.
pub fn export_ai_editing(doc: &Document, o: &AiEditingOptions) -> Result<AiEditingOutput, String> {
    let board = doc.artboards.first().ok_or("the document has no artboard")?.rect;
    if !(board.is_finite() && board.width() > 0.0 && board.height() > 0.0) {
        return Err("the document has no artboard".into());
    }
    // Artboards that don't fit on the canvas are written on a large canvas, at 1/scale.
    let scale = canvas_scale(doc);
    // The objects that may keep their effects in an art style, as they are before baking (not on
    // a large canvas, whose scale the styles' strokes and effects would need).
    let styled = writer::styled_objects(doc);
    let own_styles = doc.graphic_styles != Document::new(1.0, 1.0).graphic_styles;
    let links = writer::style_links(doc);
    // Live geometry effects as their result, as the PDF part draws them.
    let baked = baked(doc);
    let doc = baked.as_ref().unwrap_or(doc);
    let reduced = (scale != 1.0).then(|| {
        let mut d = doc.clone();
        crate::import::scale_document(&mut d, 1.0 / scale, true);
        // Symbol and pattern art is placed without scaling its strokes, so its strokes and effects
        // are scaled here.
        for s in d.symbols.iter_mut() {
            scale_strokes(Arc::make_mut(&mut s.art), 1.0 / scale);
        }
        for p in d.patterns.iter_mut() {
            for a in &mut p.art {
                scale_strokes(Arc::make_mut(a), 1.0 / scale);
            }
        }
        d
    });
    // Symbols are defined at their own size, whatever the canvas.
    let symbol_doc = doc;
    let doc = reduced.as_ref().unwrap_or(doc);
    let board = doc.artboards.first().map_or(board, |a| a.rect);
    // The document is y down; art space is ruler space: y up, its origin the first artboard's
    // top-left corner.
    let to_art = Affine::new([1.0, 0.0, 0.0, -1.0, -board.x0, board.y0]);
    let origin = ruler_origin(doc, to_art);
    let mut w = Writer::new(doc, to_art, origin);
    // On a large canvas the layers' symbol instances are written as their art.
    w.symbols_live = scale == 1.0;
    w.symbol_doc = symbol_doc;
    w.reserve_swatch_gradients();
    if scale == 1.0 {
        w.styled = styled;
        w.panel_styles();
    } else if !styled.is_empty() || own_styles || links > 0 {
        w.warnings.push(writer::LARGE_CANVAS_STYLES.to_string());
    }
    for layer in &doc.layers {
        w.layer(layer);
    }
    if scale == 1.0 {
        w.lost_links(links);
    }
    // The symbols no instance uses too; their art adds definitions.
    w.symbols();
    // The palette first: its gradient and pattern swatches add definitions.
    let mut palette = String::new();
    w.palette(&mut palette);
    let library = w.variable_library();
    let first = to_art.transform_rect_bbox(board);
    let art = doc
        .layers
        .iter()
        .filter(|l| l.visible)
        .filter_map(|l| l.visual_bounds())
        .map(|b| to_art.transform_rect_bbox(b))
        .filter(|b| b.is_finite())
        .reduce(|a, b| a.union(b))
        .unwrap_or(first);
    let header = header(doc, o, &w, art, first, origin, scale);
    let mut s = header.clone();
    s.push_str("%%BeginProlog\n%%EndProlog\n%%BeginSetup\n");
    w.definitions(&mut s);
    w.symbol_definitions(&mut s);
    w.art_styles(&mut s);
    s.push_str(&palette);
    w.art_style_list(&mut s);
    w.symbol_list(&mut s);
    artboards(&mut s, doc, to_art, origin, &library);
    if !w.texts.is_empty() {
        s.push_str(&w.texts.section(doc));
    }
    s.push_str("%%EndSetup\n");
    s.push_str(&w.body);
    s.push_str("%%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\n%%EOF\n");
    // Lines end in CR LF, as in the files users sent; line breaks inside strings are escaped.
    let program = splice(&crlf(&s), &w.payloads);
    Ok(AiEditingOutput { program, header: crlf(&header), warnings: w.warnings })
}

/// `s` with each line ending in CR LF.
fn crlf(s: &str) -> Vec<u8> {
    s.replace('\n', "\r\n").into_bytes()
}

/// `text` with each payload's mark (`\u{1}`, its index in `payloads`, `\u{1}`) replaced by its
/// bytes. Strings are written with that byte escaped, so a mark is never part of one.
fn splice(text: &[u8], payloads: &[Vec<u8>]) -> Vec<u8> {
    let mark = writer::PAYLOAD as u8;
    if payloads.is_empty() {
        return text.to_vec();
    }
    let mut out = Vec::with_capacity(text.len() + payloads.iter().map(Vec::len).sum::<usize>());
    let mut parts = text.split(|b| *b == mark);
    if let Some(first) = parts.next() {
        out.extend_from_slice(first);
    }
    // The parts alternate: an index, then the text up to the next mark.
    while let (Some(index), Some(rest)) = (parts.next(), parts.next()) {
        let payload = std::str::from_utf8(index).ok().and_then(|i| i.parse::<usize>().ok()).and_then(|i| payloads.get(i));
        if let Some(p) = payload {
            out.extend_from_slice(p);
        }
        out.extend_from_slice(rest);
    }
    out
}

/// `doc` with its geometry effects and group paint baked ([`vectorcraft_effects::bake_document`]),
/// in its symbols' art and its patterns' tile art too (which instances and pattern fills are
/// written as); `None` when nothing changes.
fn baked(doc: &Document) -> Option<Document> {
    let baked = vectorcraft_effects::bake_document(doc);
    let d = baked.as_ref().unwrap_or(doc);
    let needs = vectorcraft_effects::needs_bake;
    if !d.symbols.iter().any(|s| needs(&s.art)) && !d.patterns.iter().any(|p| p.art.iter().any(|a| needs(a))) {
        return baked;
    }
    // The symbols' art, then each pattern's art, baked as the layers of a copy of the document.
    let mut art = d.clone();
    let symbols = d.symbols.iter().map(|s| Arc::new(Node::group(s.art.id, vec![s.art.clone()])));
    let patterns = d.patterns.iter().map(|p| Arc::new(Node::group(vectorcraft_doc::NodeId(0), p.art.clone())));
    art.layers = symbols.chain(patterns).collect();
    // The copy keeps what baking adds to the document (the images color adjustments make).
    let Some(mut out) = vectorcraft_effects::bake_document(&art) else { return baked };
    let layers = std::mem::replace(&mut out.layers, d.layers.clone());
    let mut members = layers.iter().map(|l| l.children().cloned().unwrap_or_default());
    for s in out.symbols.iter_mut() {
        if let Some(a) = members.next().and_then(|m| m.first().cloned()) {
            s.art = a;
        }
    }
    for p in out.patterns.iter_mut() {
        if let Some(m) = members.next() {
            p.art = m;
        }
    }
    Some(out)
}

/// The editing data of `doc` as a `.ai` file's pages carry it: the program compressed with
/// Zstandard after `%AI24_ZStandard_Data`, its header as the `AIMetaData`, and the version numbers
/// that go with that form; and what it couldn't hold.
pub fn private_data(doc: &Document, o: &AiEditingOptions) -> Result<(PrivateData, Vec<String>), String> {
    let out = export_ai_editing(doc, o)?;
    let mut data = ZSTD_MARKER.to_vec();
    data.extend(ruzstd::encoding::compress_to_vec(out.program.as_slice(), ruzstd::encoding::CompressionLevel::Fastest));
    let p = PrivateData {
        data,
        metadata: out.header,
        container_version: CONTAINER_VERSION,
        creator_version: CREATOR,
        roundtrip_version: ROUNDTRIP_VERSION,
        roundtrip_stream_type: ROUNDTRIP_STREAM_TYPE,
    };
    Ok((p, out.warnings))
}

/// Where art space's origin lies on the canvas (y down, [`CANVAS_SIZE`] points square): the
/// artboards together centered on the canvas's center, on whole points, as the files users sent
/// place them.
fn ruler_origin(doc: &Document, to_art: Affine) -> (f64, f64) {
    let all = doc.artboards.iter().map(|ab| to_art.transform_rect_bbox(ab.rect)).filter(|r| r.is_finite()).reduce(|a, b| a.union(b));
    let c = all.map_or((0.0, 0.0), |r| (r.center().x, r.center().y));
    let mid = canvas_center();
    ((mid - c.0).floor(), (mid + c.1).floor())
}

/// `%AI24_LargeCanvasScale`: 1 when the artboards together fit on the canvas, with a point to
/// spare on each side for the rounding of [`ruler_origin`], else [`LARGE_CANVAS_SCALE`]: then the
/// art, boxes and artboards are written at 1/[`LARGE_CANVAS_SCALE`] of their size.
fn canvas_scale(doc: &Document) -> f64 {
    let all = doc.artboards.iter().map(|ab| ab.rect).filter(|r| r.is_finite()).reduce(|a, b| a.union(b));
    let room = f64::from(CANVAS_SIZE) - 2.0;
    if all.is_some_and(|r| r.width() > room || r.height() > room) { LARGE_CANVAS_SCALE } else { 1.0 }
}

/// The strokes and effects of `n` and of what it holds, `k` times as large.
fn scale_strokes(n: &mut Node, k: f64) {
    n.appearance.scale_strokes(k);
    n.appearance.scale_effects(k, vectorcraft_effects::scale_effect);
    if let Some(mask) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
        scale_strokes(Arc::make_mut(&mut mask.art), k);
    }
    if let Some(children) = n.children_mut() {
        for c in children {
            scale_strokes(Arc::make_mut(c), k);
        }
    }
}

/// The canvas's center, on its own axes and in both directions.
fn canvas_center() -> f64 {
    f64::from(CANVAS_SIZE) / 2.0
}

/// The header comments, through `%%EndComments`. `art`: the visible art's bounds, `first` the first
/// artboard, in art space; `origin` art space's origin on the canvas ([`ruler_origin`]); `scale`
/// the large canvas scale ([`canvas_scale`]).
fn header(doc: &Document, o: &AiEditingOptions, w: &Writer, art: Rect, first: Rect, origin: (f64, f64), scale: f64) -> String {
    let mut s = String::new();
    s.push_str(PS_HEADER);
    s.push('\n');
    let _ = writeln!(s, "%%Creator: {CREATOR_TEXT}");
    let _ = writeln!(s, "%%AI8_CreatorVersion: {CREATOR_VERSION}");
    // The user and the organization (§2.1.1).
    let _ = writeln!(s, "%%For: {} ()", ps::bytes(doc.metadata.author.as_bytes()));
    let _ = writeln!(s, "%%Title: {}", ps::bytes(o.title.as_bytes()));
    if let Some(t) = o.created {
        let [y, mo, d, h, mi, _] = vectorcraft_doc::metadata::civil(t);
        let _ = writeln!(s, "%%CreationDate: ({mo}/{d}/{y:04}) ({h:02}:{mi:02})");
    }
    let _ = writeln!(s, "%%Canvassize: {CANVAS_SIZE}");
    let _ = writeln!(s, "%%BoundingBox: {} {} {} {}", art.x0.floor() as i64, art.y0.floor() as i64, art.x1.ceil() as i64, art.y1.ceil() as i64);
    s.push_str("%%HiResBoundingBox: ");
    ps::push_nums(&mut s, &[art.x0, art.y0, art.x1, art.y1]);
    s.pop();
    s.push('\n');
    s.push_str("%%DocumentProcessColors: Cyan Magenta Yellow Black\n");
    s.push_str("%AI5_FileFormat 14.0\n%AI3_ColorUsage: Color\n");
    // Linked files aren't included (their previews are).
    s.push_str("%AI7_ImageSettings: 0\n");
    let spots = w.spot_swatches();
    for (i, (name, _)) in spots.iter().enumerate() {
        let _ = writeln!(s, "{} {}", if i == 0 { "%%DocumentCustomColors:" } else { "%%+" }, ps::bytes(name.as_bytes()));
    }
    for (i, (name, cmyk)) in spots.iter().enumerate() {
        s.push_str(if i == 0 { "%%CMYKCustomColor: " } else { "%%+ " });
        ps::push_nums(&mut s, &cmyk.map(f64::from));
        let _ = writeln!(s, "{}", ps::bytes(name.as_bytes()));
    }
    // The first artboard, left, bottom, right, top.
    let mut board = String::new();
    ps::push_nums(&mut board, &[first.x0, first.y0, first.x1, first.y1]);
    board.pop();
    let _ = writeln!(s, "%AI3_Cropmarks: {board}");
    // The canvas's center in art space.
    let (cx, cy) = (ps::num(canvas_center() - origin.0), ps::num(origin.1 - canvas_center()));
    let _ = writeln!(s, "%AI3_TemplateBox: {cx} {cy} {cx} {cy}");
    let _ = writeln!(s, "%AI3_TileBox: {board}");
    s.push_str("%AI3_DocumentPreview: None\n");
    let _ = writeln!(s, "%AI5_ArtSize: {ART_SIZE} {ART_SIZE}");
    let _ = writeln!(s, "%AI5_RulerUnits: {}", ruler_units(doc.units));
    let _ = writeln!(s, "%AI24_LargeCanvasScale: {}", ps::num(scale));
    let model = if doc.color_mode == vectorcraft_doc::ColorMode::Cmyk { 2 } else { 1 };
    let _ = writeln!(s, "%AI9_ColorModel: {model}");
    let _ = writeln!(s, "%AI5_ArtFlags: {ART_FLAGS}");
    let _ = writeln!(s, "%AI5_TargetResolution: {TARGET_RESOLUTION}");
    let _ = writeln!(s, "%AI5_NumLayers: {}", doc.layers.len());
    // A digit for each layer: 7 shown, 6 hidden.
    let views: String = doc.layers.iter().map(|l| if l.visible { '7' } else { '6' }).collect();
    if !views.is_empty() {
        let _ = writeln!(s, "%AI5_OpenViewLayers: {views}");
    }
    // Gridlines and subdivisions (§2.1.1, Table 6): in back, lines, in VectorCraft's grid gray.
    let (space, subdiv) = (ps::num(doc.grid.spacing), doc.grid.subdivisions);
    let gray = ps::num(GRID_GRAY);
    let _ = writeln!(s, "%AI7_GridSettings: {space} {subdiv} {space} {subdiv} 1 0 {gray} {gray} {gray} {gray} {gray} {gray}");
    s.push_str("%%EndComments\n");
    s
}

/// `%AI5_RulerUnits` of `u` (as the reader reads it; units it has no number for are points).
fn ruler_units(u: Unit) -> u8 {
    match u {
        Unit::Inches => 0,
        Unit::Millimeters => 1,
        Unit::Picas => 3,
        Unit::Centimeters => 4,
        Unit::Pixels => 6,
        Unit::Points | Unit::FeetInches | Unit::Meters | Unit::Yards | Unit::Feet | Unit::Q | Unit::Ha => 2,
    }
}

/// The artboards as the `ArtboardArray` of the `/Document` dictionary: each one's top-left and
/// bottom-right corners in art space (`PositionPoint1`, `PositionPoint2`), its top-left corner on
/// the canvas (`RulerOrigin`, `origin` being art space's origin there), its pixel aspect ratio
/// (`PAR`), `DisplayMark` 0 (as in the files users sent), whether it is the selected one (the
/// active one) and its name; then the index of the active artboard (`CropAreaActive`: the one
/// [`Document::opened_artboard`] names, else the first) and the canvas's size. Every record holds
/// all of these entries. The artboards, the active artboard and the canvas size are written in the
/// `/Recorded` part of the `/Document` dictionary, followed by an empty `/NotRecorded` part.
fn artboards(s: &mut String, doc: &Document, to_art: Affine, origin: (f64, f64), library: &str) {
    let active = doc.opened_artboard.and_then(|id| doc.artboards.iter().position(|a| a.id == id)).unwrap_or(0);
    s.push_str("%AI9_BeginDocumentData\n%_/Document :\n%_/Dictionary :\n%_/Array :\n");
    for (i, ab) in doc.artboards.iter().enumerate() {
        let r = to_art.transform_rect_bbox(ab.rect);
        s.push_str("%_/Dictionary :\n%_");
        ps::push_nums(s, &[r.x0, r.y1]);
        s.push_str("/RealPointRelToROrigin\n%_ (PositionPoint1) ,\n%_");
        ps::push_nums(s, &[r.x1, r.y0]);
        s.push_str("/RealPointRelToROrigin\n%_ (PositionPoint2) ,\n%_");
        ps::push_nums(s, &[origin.0 + r.x0, origin.1 - r.y1]);
        s.push_str("/RealPoint\n%_ (RulerOrigin) ,\n%_");
        let par = ab.video_ruler_pixel_aspect_ratio;
        ps::push_num(s, if par.is_finite() && par > 0.0 { par } else { 1.0 });
        // Display marks: 1 the center mark, 2 cross hairs, 4 video safe areas.
        let mark = u8::from(ab.show_center_mark) | u8::from(ab.show_cross_hairs) << 1 | u8::from(ab.show_video_safe_areas) << 2;
        let _ = writeln!(s, " /Real (PAR) ,\n%_{mark} /Int (DisplayMark) ,\n%_{} /Bool (IsArtboardSelected) ,", u8::from(i == active));
        let _ = writeln!(s, "%_{} /UnicodeString (Name) ,\n%_; ,", ps::bytes(ab.name.as_bytes()));
    }
    s.push_str("%_; (ArtboardArray) ,\n");
    let _ = writeln!(s, "%_{active} /Int (CropAreaActive) ,");
    let _ = writeln!(s, "%_{CANVAS_SIZE} /Int (AIDocumentCanvasSize) ,");
    // The variable library, after the canvas size.
    s.push_str(library);
    s.push_str("%_; /Recorded ,\n%_/Dictionary : /NotRecorded ,\n%_; /NotRecorded ,\n%_;\n%AI9_EndDocumentData\n");
}

#[cfg(test)]
mod tests;
