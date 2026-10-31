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
//!   `%AI5_ArtSize 14400 14400`, `%AI5_RulerUnits`, `%AI9_ColorModel` (1 RGB, 2 CMYK),
//!   `%AI5_ArtFlags`, `%AI5_TargetResolution`, `%AI5_NumLayers`, `%AI5_OpenViewLayers` (a digit for
//!   each layer, 7 shown and 6 hidden), `%AI7_GridSettings` (§2.1.1), and the version comment
//!   `%%AI8_CreatorVersion` ([`CREATOR_VERSION`]).
//! - Setup: `0 Bn` (no gradient definitions, §3), the color swatches (`%AI5_BeginPalette`,
//!   `0 0 Pb`, the Registration swatch, each swatch's color operator and `(name) Pc`, a color group as `0 (name) count Pg`
//!   before its swatches, `PB`), then the artboards as the `ArtboardArray` of the `/Document`
//!   dictionary's `/Recorded` part and the canvas size as its `AIDocumentCanvasSize`, on `%_`
//!   lines.
//! - Layers: `%AI5_BeginLayer`, `visible preview unlocked printing dimmed 0 1 colorIndex r g b 0
//!   dimPercent 0 Lb` (14 operands; a color that isn't one of the preset layer colors has the
//!   index -1), `(name) Ln`, the layer's name as an XML id in an `/ArtDictionary`, its art and
//!   sublayers, `LB`, `%AI5_EndLayer--`.
//! - Colors: `g` gray, `k` CMYK, `c m y k r g b Xa` RGB (Lab is written as RGB); a spot color
//!   `c m y k (name) tint x` (CMYK) or `c m y k r g b|L a b (name) tint 1|2 Xx` (RGB or Lab); a
//!   global process color `… (name) tint model Xk`; Registration `1 1 1 1 ([Registration]) tint
//!   Xs`. Upper case paints the stroke.
//! - State before each object: locked `A`, hidden `Xw` (`1` or `0`). An object's name follows it as
//!   an `/ArtDictionary` whose `AI10_ArtUID` is the name as an XML id. Guides are `(N) *`, the
//!   path, `(N) *` (`(n) *` when closed).
//!
//! Lines end in CR LF. Strings are written in UTF-8. Art space is ruler space: y up, its origin at
//! the first artboard's top-left corner. On the canvas ([`CANVAS_SIZE`] points square, y down) the
//! artboards together are centered; each artboard's `RulerOrigin` is its top-left corner there, and
//! `%AI3_TemplateBox` holds the canvas's center in art space. What the form above can't hold is written as plain filled and stroked paths, each such
//! loss a warning: type as its outlines and its inline graphics' art, gradients as their average color, pattern fills as their
//! tiles cut to the shape, clipping groups and clipping layers as their art cut to the clipping
//! path (the clipping path's fill cut too, its stroke not), images as their outlines filled with
//! their average color, symbols and live objects as their art (symbol art past
//! [`vectorcraft_doc::MAX_SYMBOL_NODES`] objects in all left out), strokes a plain line can't draw
//! as their filled outlines, several fills or strokes as a group of a path for each, gradient and
//! pattern swatches, transparency and live effects other than geometry effects left out.
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

/// `%%Creator`. Apps that read the editing data take the file's version from this line while they
/// read the header, and only a creator that names the format's app gets the version that measures
/// `%AI3_TemplateBox` on a canvas of [`CANVAS_SIZE`] points; any other creator places the art
/// 3,871.5 points off its artboard. The text holds no version number (a number in it would be
/// read as the format's version).
const CREATOR_TEXT: &str = "VectorCraft, editing data for Illustrator"; // brand-ok: the creator the format's version is read from

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
    // Live geometry effects as their result, as the PDF part draws them.
    let baked = baked(doc);
    let doc = baked.as_ref().unwrap_or(doc);
    // The document is y down; art space is ruler space: y up, its origin the first artboard's
    // top-left corner.
    let to_art = Affine::new([1.0, 0.0, 0.0, -1.0, -board.x0, board.y0]);
    let mut w = Writer::new(doc, to_art);
    for layer in &doc.layers {
        w.layer(layer);
    }
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
    let origin = ruler_origin(doc, to_art);
    let header = header(doc, o, &w, art, first, origin);
    let mut s = header.clone();
    // No gradient definitions: gradients are written as their average color.
    s.push_str("%%BeginProlog\n%%EndProlog\n%%BeginSetup\n0 Bn\n");
    w.palette(&mut s);
    artboards(&mut s, doc, to_art, origin);
    s.push_str("%%EndSetup\n");
    s.push_str(&w.body);
    s.push_str("%%PageTrailer\ngsave annotatepage grestore showpage\n%%Trailer\n%%EOF\n");
    // Lines end in CR LF, as in the files users sent; line breaks inside strings are escaped.
    Ok(AiEditingOutput { program: crlf(&s), header: crlf(&header), warnings: w.warnings })
}

/// `s` with each line ending in CR LF.
fn crlf(s: &str) -> Vec<u8> {
    s.replace('\n', "\r\n").into_bytes()
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

/// The canvas's center, on its own axes and in both directions.
fn canvas_center() -> f64 {
    f64::from(CANVAS_SIZE) / 2.0
}

/// The header comments, through `%%EndComments`. `art`: the visible art's bounds, `first` the first
/// artboard, in art space; `origin` art space's origin on the canvas ([`ruler_origin`]).
fn header(doc: &Document, o: &AiEditingOptions, w: &Writer, art: Rect, first: Rect, origin: (f64, f64)) -> String {
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
    // Linked images aren't embedded (the data holds no images).
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
    s.push_str("%AI24_LargeCanvasScale: 1\n");
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
        Unit::Points | Unit::FeetInches | Unit::Meters | Unit::Yards | Unit::Feet => 2,
    }
}

/// The artboards as the `ArtboardArray` of the `/Document` dictionary: each one's top-left and
/// bottom-right corners in art space (`PositionPoint1`, `PositionPoint2`), its top-left corner on
/// the canvas (`RulerOrigin`, `origin` being art space's origin there), its pixel aspect ratio
/// (`PAR`), `DisplayMark` 0 (as in the files users sent), whether it is the selected one (the
/// first) and its name; then the canvas's size. Apps that read the editing data drop a record that
/// lacks any of the first five. Both sit in the dictionary the `/Document` marks `/Recorded`,
/// followed by an empty `/NotRecorded` one, as in the files users sent: apps that read the editing
/// data do not read artboards from the `/NotRecorded` dictionary, and then place the art one
/// artboard's height above a default artboard.
fn artboards(s: &mut String, doc: &Document, to_art: Affine, origin: (f64, f64)) {
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
        let _ = writeln!(s, " /Real (PAR) ,\n%_0 /Int (DisplayMark) ,\n%_{} /Bool (IsArtboardSelected) ,", u8::from(i == 0));
        let _ = writeln!(s, "%_{} /UnicodeString (Name) ,\n%_; ,", ps::bytes(ab.name.as_bytes()));
    }
    s.push_str("%_; (ArtboardArray) ,\n");
    let _ = writeln!(s, "%_{CANVAS_SIZE} /Int (AIDocumentCanvasSize) ,");
    s.push_str("%_; /Recorded ,\n%_/Dictionary : /NotRecorded ,\n%_; /NotRecorded ,\n%_;\n%AI9_EndDocumentData\n");
}

#[cfg(test)]
mod tests;
