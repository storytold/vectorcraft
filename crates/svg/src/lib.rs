//! VectorCraft SVG import and export.
//!
//! * [`export`] / [`export_full`] write a [`Document`] as SVG 1.1 with our own writer, as set by
//!   [`ExportOptions`] (presentation attributes, inline styles, style entities or internal CSS
//!   classes; gradients in `<defs>` with `userSpaceOnUse`; clip groups as `<clipPath>`; images
//!   embedded as `data:` URIs or linked; text as `<text>`/`<tspan>` or outlines; symbols as one
//!   `<symbol>` each with a `<use>` per instance; hidden layers left out, or kept not displayed),
//!   in UTF-8, UTF-16 or ISO 8859-1 ([`Output::bytes`]), as SVG 1.1 or a simplified SVG Tiny 1.2
//!   ([`Profile`]), optionally with the fonts type uses embedded as `@font-face` subsets.
//! * [`import`] / [`import_with_report`] parse SVG with `usvg` and convert its normalized tree into
//!   document nodes with all transforms baked into the geometry.
//! * [`css_rules`] writes the CSS web pages style objects with (the CSS Properties panel): shapes'
//!   paints as backgrounds and borders, type as fonts, shadows as `box-shadow`, each property
//!   written as SVG export writes it in its style sheets.
//!
//! ## Export approximations
//!
//! * **Stroke alignment** is not expressible in SVG 1.1. An *inside* stroke of a closed path is
//!   written as a stroke of double width clipped to the path's own shape (`<clipPath>`); an
//!   *outside* stroke as a stroke of double width masked by a `<mask>` that hides the path's
//!   interior. Both are visually exact but re-import as a group (clip group / masked stroke)
//!   rather than an aligned stroke. Open paths have no inside: their strokes are written centred,
//!   as the canvas draws them.
//! * **Arrowheads, width profiles, dashes fitted to corners and dotted dashes** are written as the
//!   filled outlines the canvas paints (the line and each arrowhead, grouped under the stroke's
//!   opacity); **brushed strokes** as their brush art. Both re-import as filled paths.
//! * **Multiple fills/strokes** (or per-fill/stroke blend modes) are written as a `<g>` holding one
//!   `<path>` per appearance item, in paint order.
//! * Live geometry effects are written as their result and raster effects (shadows, glows, blurs)
//!   as SVG filters.
//! * **Freeform gradients**, which SVG can't express, fill or stroke objects as an `<image>` of
//!   their colour field (sampled as the canvas samples it) clipped to the shape or the stroke's
//!   outline; on characters they are written as linear gradients. Gradients along or across
//!   strokes are written as slices of linear gradients clipped to the stroke.
//! * **Symbol instances** the shared `<symbol>` would paint differently from the canvas (stained,
//!   scaled with strokes, turned or scaled with effects, brushes or live objects, or holding
//!   pattern paints or unlinked masks, which stay on the page) are written as their own art.
//!
//! ## Import approximations
//!
//! * One CSS pixel (user unit) is one point, as on export, and absolute lengths (`in`, `cm`, `mm`,
//!   `pt`, `pc`) keep their physical size (72 pt per inch): `font-size="12pt"` is 12 pt. A root
//!   `width`/`height` in absolute units keeps its physical size, so a 210 mm SVG opens on a 210 mm
//!   artboard, its user units being CSS pixels of it (96 per inch), the other side's too when only
//!   one is absolute. The document's units follow the unit of the root `width` (pixels when it has
//!   none).
//! * `<mask>` imports as a luminance opacity mask; a mask we exported keeps its options and art
//!   (its `data-vectorcraft-mask="noclip invert"` lists the options that differ from clipping and
//!   not inverted). Nested clip paths (`<clipPath clip-path>`) are clip groups in clip groups.
//! * Filters that are a Gaussian blur, a drop shadow (`feDropShadow` or the usual chains of offset,
//!   blur, flood or colour matrix, composite and merge), a glow or a feather (as we export them)
//!   become those live effects; other filters are ignored (reported as warnings).
//! * `<pattern>` becomes a pattern swatch (its content clipped to the tile) painted with the
//!   pattern's placement.
//! * `<symbol>` with `<use>` becomes a symbol and its instances. A `<use>` that shows the symbol
//!   differently (inherited paint, a viewport that cuts it) or that the canvas would paint
//!   differently (see the export's symbol rules) becomes plain art, as do `<use>`s of other
//!   elements.
//! * A `<g id>` layer (top-level, or inside one) that isn't displayed (`display: none`) comes back
//!   as a hidden layer; other undisplayed objects as hidden objects, unless a `<use>` refers to
//!   them.
//! * `spreadMethod` reflect and repeat are expanded into stops over the painted area (at most
//!   64 periods).
//! * `<image>` files are read through [`ImportOptions`]: rasters stay linked, SVG files (and SVG
//!   `data:` URLs) become art (without their text), missing ones a placeholder keeping the link.
//! * Text lines after the first start at the first line's x; text in a clip path is ignored;
//!   absolute positions inside type on a path are ignored; vertical text sets every glyph sideways.
//!
//! ## Import text
//!
//! usvg only keeps `<text>` when fonts are loaded; we don't load a font database (too expensive and
//! unavailable on wasm), so `<text>` elements are read from the XML as live [`TextObject`]s. Each
//! one is swapped for a placeholder before usvg runs, so the text keeps its z-order, parent group,
//! clip, mask, opacity, link, `<use>` instances and gradient or pattern paints. The style cascade
//! supports type, `#id`, `.class` (several), attribute, descendant and child selectors.
//! `<textPath>` becomes type on a path (`startOffset`, `side`), vertical `writing-mode` type on a
//! vertical path; per-character `x`/`y`/`dx`/`dy`/`rotate`, `baseline-shift`, numeric weights,
//! letter and word spacing become character attributes (line breaks, kerning, baseline shift,
//! rotation and style names).
#![forbid(unsafe_code)]

mod css;
mod export;
mod import;

pub use css::{CssOptions, CssRule, CssSheet, CssUnits, css_rules};

use serde::{Deserialize, Serialize};

pub use vectorcraft_doc::Document;
pub use vectorcraft_doc::TextObject;

/// How style properties are written (SVG Options → Styling).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Styling {
    /// `fill="#ff0000"` attributes.
    #[default]
    #[serde(rename = "presentation")]
    PresentationAttributes,
    /// `style="fill:#ff0000"`.
    #[serde(rename = "style")]
    InlineStyle,
    /// `style="&st1;"`, each distinct declaration block an XML entity declared in the DOCTYPE.
    #[serde(rename = "entities")]
    StyleEntities,
    /// `class="cls-1"` with a `<style>` element in `<defs>`.
    #[serde(rename = "css")]
    InternalCss,
}

/// Which `id` attributes objects get (SVG Options → Object IDs). Definitions other elements
/// reference (gradients, clip paths, masks, patterns, filters) always have ids.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ObjectIds {
    /// Readable ids from layer and object names.
    #[default]
    LayerNames,
    /// Only the ids that are referenced.
    Minimal,
    /// Every id carries a prefix derived from the content, so SVGs inlined in one page never
    /// share an id.
    Unique,
}

/// How raster images are written (SVG Options → Images).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ImageMode {
    /// `data:` URIs inside the SVG.
    #[default]
    Embed,
    /// `href` to an image file: a linked image's own file, an embedded one written next to the
    /// SVG (see [`Output::linked`]).
    Link,
}

/// The character encoding of an exported file (SVG Options → Encoding).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Encoding {
    #[default]
    #[serde(rename = "utf8", alias = "utf-8")]
    Utf8,
    /// UTF-16, big-endian, after a byte order mark.
    #[serde(rename = "utf16", alias = "utf-16")]
    Utf16,
    /// ISO 8859-1 (Latin-1): other characters are written as character references (`&#x4E2D;`).
    #[serde(rename = "latin1", alias = "iso-8859-1")]
    Latin1,
}

impl Encoding {
    /// The name the XML declaration gives it.
    pub fn xml_name(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf16 => "UTF-16",
            Self::Latin1 => "ISO-8859-1",
        }
    }
}

/// The SVG profile written (SVG Options → SVG Profile).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    /// SVG 1.1 (full).
    #[default]
    #[serde(rename = "svg11", alias = "1.1")]
    Svg11,
    /// SVG Tiny 1.2, simplified: `version="1.2" baseProfile="tiny"`, presentation attributes only
    /// (no style sheets, style attributes, blend modes or CSS-only properties), no filters, masks
    /// or `<symbol>`s and no embedded fonts. Raster effects and opacity masks are left out,
    /// knockout groups written as plain groups, symbol instances as their own art and strokes
    /// aligned outside clipped by the area outside the shape. Clip paths, patterns and type on a
    /// path are written as in SVG 1.1.
    #[serde(rename = "tiny12", alias = "tiny1.2")]
    Tiny12,
}

/// SVG export options (SVG Options). Deserializes from camelCase JSON with every field optional;
/// unknown fields are rejected.
///
/// These defaults are the library's and the engine's (`document.export` without options):
/// presentation attributes and a fixed size, the most portable output. The SVG Options dialog
/// starts from Internal CSS and Responsive instead, like the reference app's Export As.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportOptions {
    /// Artboard index to export (sets the viewBox); `None` = the bounds of all art. Callers pick
    /// it (the engine's `artboard`/`range`/`useArtboards` params), so it is not deserialized.
    #[serde(skip)]
    pub artboard: Option<usize>,
    pub styling: Styling,
    /// Decimal places for coordinates (1–7 in the dialog; default 3).
    pub decimals: u8,
    pub object_ids: ObjectIds,
    pub images: ImageMode,
    /// No indentation or newlines, no XML declaration.
    pub minify: bool,
    /// Omit `width`/`height` so the SVG scales to its container.
    pub responsive: bool,
    /// Fonts → Convert to Outlines: text becomes paths (portable, no font needed to view it).
    pub outline_text: bool,
    /// Embed the native document (passed to [`export_full`]) in `<metadata>` so VectorCraft
    /// reopens the SVG with nothing lost (see [`editing`]).
    pub preserve_editing: bool,
    /// Write `<metadata>` with the document's title, format and File Info (Dublin Core).
    pub metadata: bool,
    /// One positioned `<tspan>` per line of type instead of one per style run, tab stop and
    /// justified word (smaller; viewers then space the line with their own font metrics).
    pub fewer_tspans: bool,
    /// Keep hidden layers and objects, written hidden (`display="none"`), as Save does; exports
    /// leave them out unless asked.
    pub hidden_layers: bool,
    /// The file's character encoding (see [`Output::bytes`]).
    pub encoding: Encoding,
    pub profile: Profile,
    /// Embed the fonts type uses as `@font-face` rules, each subset to the characters used where
    /// its licence allows (whole where it forbids subsetting, left out where it forbids
    /// embedding). Not with outlined text or the Tiny profile.
    pub embed_fonts: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            artboard: Some(0),
            styling: Styling::PresentationAttributes,
            decimals: 3,
            object_ids: ObjectIds::LayerNames,
            images: ImageMode::Embed,
            minify: false,
            responsive: false,
            outline_text: false,
            preserve_editing: false,
            metadata: false,
            fewer_tspans: false,
            hidden_layers: false,
            encoding: Encoding::Utf8,
            profile: Profile::Svg11,
            embed_fonts: false,
        }
    }
}

/// Decimal places the options accept.
pub const DECIMALS: std::ops::RangeInclusive<u8> = 1..=7;

impl ExportOptions {
    /// Values outside what the options allow (the decimals range).
    pub fn check(&self) -> Result<(), String> {
        if DECIMALS.contains(&self.decimals) {
            Ok(())
        } else {
            Err(format!("decimals must be {}–{} (got {})", DECIMALS.start(), DECIMALS.end(), self.decimals))
        }
    }
}

/// An export: the SVG text and the image files it links to.
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub svg: String,
    /// [`ImageMode::Link`]: embedded images written as files next to the SVG, named after their
    /// content (`img….png`), so exports of different documents into one folder never clash.
    pub linked: Vec<LinkedImage>,
    /// Features the export approximates.
    pub warnings: Vec<String>,
    /// The encoding [`Self::bytes`] writes.
    pub encoding: Encoding,
}

impl Output {
    /// The file: the SVG in its encoding (UTF-16 after a byte order mark).
    pub fn bytes(&self) -> Vec<u8> {
        match self.encoding {
            Encoding::Utf8 => self.svg.as_bytes().to_vec(),
            e => e.encode_str(&self.svg),
        }
    }

    /// [`Self::bytes`], taking the SVG text (UTF-8 files are the text itself, not a copy).
    pub fn take_bytes(&mut self) -> Vec<u8> {
        let svg = std::mem::take(&mut self.svg);
        match self.encoding {
            Encoding::Utf8 => svg.into_bytes(),
            e => e.encode_str(&svg),
        }
    }
}

impl Encoding {
    /// `svg` written in this encoding.
    fn encode_str(self, svg: &str) -> Vec<u8> {
        match self {
            Encoding::Utf8 => svg.as_bytes().to_vec(),
            Encoding::Utf16 => [0xfe, 0xff].into_iter().chain(svg.encode_utf16().flat_map(u16::to_be_bytes)).collect(),
            // The writer left only Latin-1 characters (the others are character references).
            Encoding::Latin1 => svg.chars().map(|c| u8::try_from(c).unwrap_or(b'?')).collect(),
        }
    }
}

/// An image file an exported SVG links to by `name`.
#[derive(Clone, Debug)]
pub struct LinkedImage {
    pub name: String,
    pub bytes: std::sync::Arc<Vec<u8>>,
}

#[derive(Debug, thiserror::Error)]
pub enum SvgError {
    #[error("SVG parse error: {0}")]
    Parse(String),
    /// Its elements nest (references followed) deeper than the reader goes.
    #[error("the SVG's elements are nested more than {0} deep")]
    TooDeep(usize),
}

/// Export a document as an SVG string (linked images are not returned: see [`export_full`]).
pub fn export(doc: &Document, opts: &ExportOptions) -> String {
    export_full(doc, opts, None).svg
}

/// Export a document as an SVG string, also returning warnings about approximated features.
pub fn export_with_report(doc: &Document, opts: &ExportOptions) -> (String, Vec<String>) {
    let o = export_full(doc, opts, None);
    (o.svg, o.warnings)
}

/// Export a document as SVG plus the image files it links to. `native`: the document in the
/// native format, embedded when [`ExportOptions::preserve_editing`] is on.
pub fn export_full(doc: &Document, opts: &ExportOptions, native: Option<&[u8]>) -> Output {
    export::export(doc, opts, native)
}

/// The XML namespace of the editing data [`ExportOptions::preserve_editing`] embeds.
pub const EDITING_NS: &str = "urn:vectorcraft:editing";

/// The native document an SVG carries ([`ExportOptions::preserve_editing`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Editing {
    /// The native document, base64.
    pub data: String,
    /// The SVG around it is still the one written with it: the hash it keeps of the markup
    /// outside its `<metadata>` matches (whitespace between tags and line endings aside). False
    /// once another app edited the SVG; true for editing data written without a hash.
    pub intact: bool,
}

/// The native document an SVG carries, if any, and whether the SVG was edited since.
pub fn editing(svg: &str) -> Option<Editing> {
    // (An SVG nested too deep for the XML reader carries none: it isn't one of ours.)
    if !svg.contains(EDITING_NS) || import::xml_too_deep(svg) {
        return None;
    }
    let opts = usvg::roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() };
    let xml = usvg::roxmltree::Document::parse_with_options(svg, opts).ok()?;
    let el = xml.descendants().find(|n| n.tag_name().namespace() == Some(EDITING_NS) && n.tag_name().name() == "document")?;
    let data: String = el.children().filter_map(|c| c.text()).flat_map(|t| t.chars().filter(|c| !c.is_whitespace())).collect();
    if data.is_empty() {
        return None;
    }
    // The hash covers everything but the `<metadata>` element holding the data.
    let intact = match (el.attribute("hash"), el.parent_element()) {
        (None, _) => true,
        (Some(h), Some(meta)) => {
            let r = meta.range();
            let (head, tail) = (svg.get(..r.start).unwrap_or(""), svg.get(r.end..).unwrap_or(""));
            body_hash(&[head, tail]) == h
        }
        (Some(_), None) => false,
    };
    Some(Editing { data, intact })
}

/// The native document an SVG carries (base64), if any, edited since or not (see [`editing`]).
pub fn editing_data(svg: &str) -> Option<String> {
    editing(svg).map(|e| e.data)
}

/// The hash [`ExportOptions::preserve_editing`] keeps of an SVG's markup outside its
/// `<metadata>` (`parts`, in order): FNV-1a of the text with whitespace between tags dropped and
/// other whitespace runs read as one space, so re-indenting or other line endings don't count as
/// edits.
pub(crate) fn body_hash(parts: &[&str]) -> String {
    let mut h = FNV_SEED;
    // The last byte hashed and whether whitespace came after it.
    let (mut last, mut gap) = (None, false);
    for b in parts.iter().flat_map(|p| p.bytes()) {
        if b.is_ascii_whitespace() {
            gap = true;
            continue;
        }
        if gap && last.is_some_and(|l| l != b'>') && b != b'<' {
            h = fnv1a_from(h, b" ");
        }
        gap = false;
        last = Some(b);
        h = fnv1a_from(h, &[b]);
    }
    format!("{h:016x}")
}

/// Does `bytes` start like gzip data (an SVGZ file)?
pub fn is_svgz(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1f, 0x8b])
}

/// Compress SVG text as SVGZ (gzip).
pub fn compress(svg: &str) -> Vec<u8> {
    compress_bytes(svg.as_bytes())
}

/// Compress an SVG file (in any encoding, see [`Output::bytes`]) as SVGZ (gzip).
pub fn compress_bytes(svg: &[u8]) -> Vec<u8> {
    use std::io::Write as _;
    let mut gz = flate2::write::GzEncoder::new(Vec::with_capacity(svg.len() / 4), flate2::Compression::default());
    // Writing to a Vec cannot fail.
    let _ = gz.write_all(svg);
    gz.finish().unwrap_or_default()
}

/// Most bytes an SVGZ file may unpack to (a guard against decompression bombs).
const MAX_SVGZ: u64 = 512 << 20;

/// SVG text from SVG or SVGZ bytes: UTF-8, UTF-16 (after a byte order mark) or, when the XML
/// declaration says so, ISO 8859-1.
pub fn text_of(bytes: &[u8]) -> Result<std::borrow::Cow<'_, str>, SvgError> {
    use std::io::Read as _;
    if !is_svgz(bytes) {
        return match decode(bytes)? {
            Decoded::Utf8(skip) => std::str::from_utf8(bytes.get(skip..).unwrap_or_default()).map(Into::into).map_err(|_| not_utf8()),
            Decoded::Text(t) => Ok(t.into()),
        };
    }
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(MAX_SVGZ + 1)
        .read_to_end(&mut raw)
        .map_err(|e| SvgError::Parse(format!("not a valid SVGZ file: {e}")))?;
    if raw.len() as u64 > MAX_SVGZ {
        return Err(SvgError::Parse(format!("the SVGZ file unpacks to more than {} MB", MAX_SVGZ >> 20)));
    }
    match decode(&raw)? {
        Decoded::Utf8(skip) => {
            raw.drain(..skip);
            String::from_utf8(raw).map(Into::into).map_err(|_| not_utf8())
        }
        Decoded::Text(t) => Ok(t.into()),
    }
}

fn not_utf8() -> SvgError {
    SvgError::Parse("SVG is not UTF-8, UTF-16 or ISO 8859-1".into())
}

/// How SVG bytes are read.
enum Decoded {
    /// UTF-8 after this many bytes (a byte order mark).
    Utf8(usize),
    /// Text decoded from another encoding.
    Text(String),
}

/// The encoding of SVG bytes: a byte order mark (UTF-8 or UTF-16), else the XML declaration's
/// (ISO 8859-1 by its names; anything else is read as UTF-8).
fn decode(bytes: &[u8]) -> Result<Decoded, SvgError> {
    let utf16 = |rest: &[u8], unit: fn([u8; 2]) -> u16| {
        let (units, odd) = rest.as_chunks::<2>();
        if !odd.is_empty() {
            return Err(not_utf8());
        }
        char::decode_utf16(units.iter().map(|u| unit(*u))).collect::<Result<String, _>>().map(Decoded::Text).map_err(|_| not_utf8())
    };
    match bytes {
        [0xef, 0xbb, 0xbf, ..] => Ok(Decoded::Utf8(3)),
        [0xfe, 0xff, rest @ ..] => utf16(rest, u16::from_be_bytes),
        [0xff, 0xfe, rest @ ..] => utf16(rest, u16::from_le_bytes),
        _ if declared_latin1(bytes) => Ok(Decoded::Text(bytes.iter().map(|b| char::from(*b)).collect())),
        _ => Ok(Decoded::Utf8(0)),
    }
}

/// Does the XML declaration at the start of `bytes` name ISO 8859-1?
fn declared_latin1(bytes: &[u8]) -> bool {
    let head = bytes.get(..bytes.len().min(200)).unwrap_or_default();
    let Some(decl) = head.strip_prefix(b"<?xml").and_then(|d| d.split(|b| *b == b'>').next()) else { return false };
    let decl = String::from_utf8_lossy(decl).to_ascii_lowercase();
    let Some(at) = decl.find("encoding") else { return false };
    let value = decl.get(at + "encoding".len()..).unwrap_or("").trim_start().trim_start_matches('=').trim_start();
    let name = value.trim_start_matches(['"', '\'']).split(['"', '\'', ' ', '?']).next().unwrap_or("");
    matches!(name, "iso-8859-1" | "iso8859-1" | "iso_8859-1" | "latin1" | "latin-1" | "l1")
}

/// Import an SVG document.
pub fn import(svg: &str) -> Result<Document, SvgError> {
    import_with_report(svg).map(|(d, _)| d)
}

/// Import an SVG document, also returning warnings about unsupported or approximated features.
/// Linked files (`<image href="photo.png">`) aren't read: see [`import_with`].
pub fn import_with_report(svg: &str) -> Result<(Document, Vec<String>), SvgError> {
    import_with(svg, &ImportOptions::default())
}

pub use import::MAX_ELEMENT_DEPTH;

/// Import an SVG document reading the files it links to as `opts` says, also returning warnings.
pub fn import_with(svg: &str, opts: &ImportOptions) -> Result<(Document, Vec<String>), SvgError> {
    // usvg converts the tree by recursion, in large stack frames: the nesting it is given is
    // capped (`import::depth`), and it gets room for that cap whatever thread asks.
    vectorcraft_doc::depth::with_deep_stack(|| import::import(svg, opts))
}

/// Reads a linked file: its bytes and the link to it (absolute path, size, time and hash), or
/// `None` when there is no readable file at the (absolute) path.
pub type ReadFile<'a> = &'a (dyn Fn(&str) -> Option<(Vec<u8>, vectorcraft_doc::LinkInfo)> + Sync);

/// How [`import_with`] reads the files an SVG's `<image>` elements link to.
///
/// A raster file stays linked ([`vectorcraft_doc::ImageObject::link`]); an SVG file imports as
/// vector art. A file that can't be read imports as a placeholder that keeps the link (and a
/// warning), so it can be relinked.
#[derive(Clone, Copy, Default)]
pub struct ImportOptions<'a> {
    /// The folder relative links are found in: the SVG file's own (taken from the working
    /// directory when it isn't absolute). Without it they can't be read.
    pub folder: Option<&'a str>,
    /// Reads a linked file. Without it no file is read.
    pub read: Option<ReadFile<'a>>,
}

/// FNV-1a: a stable content hash (image keys, unique id prefixes).
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    fnv1a_from(FNV_SEED, bytes)
}

const FNV_SEED: u64 = 0xcbf29ce484222325;

/// FNV-1a of `bytes` continued from hash `h`.
fn fnv1a_from(h: u64, bytes: &[u8]) -> u64 {
    bytes.iter().fold(h, |h, b| (h ^ *b as u64).wrapping_mul(0x100000001b3))
}

/// Standard base64 (RFC 4648, with padding).
pub fn base64_encode(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Escape text for XML content and attribute values.
pub fn xml_escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            '\'' => o.push_str("&apos;"),
            _ => o.push(c),
        }
    }
    o
}

/// Format a number with at most `decimals` places, trimming trailing zeros.
pub fn fmt_num(v: f64, decimals: u8) -> String {
    let mut s = String::new();
    push_num(&mut s, v, decimals);
    s
}

/// [`fmt_num`] appended to `out`, without allocating.
pub fn push_num(out: &mut String, v: f64, decimals: u8) {
    use std::fmt::Write as _;
    let v = if v.is_finite() { v } else { 0.0 };
    let start = out.len();
    let _ = write!(out, "{:.*}", decimals as usize, v); // writing to a String can't fail
    let s = out.get(start..).unwrap_or_default();
    if s.contains('.') {
        let keep = s.trim_end_matches('0').trim_end_matches('.').len();
        out.truncate(start + keep);
    }
    if matches!(out.get(start..), Some("-0" | "")) {
        out.truncate(start);
        out.push('0');
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn numbers() {
        assert_eq!(fmt_num(1.0, 3), "1");
        assert_eq!(fmt_num(1.23456, 3), "1.235");
        assert_eq!(fmt_num(-0.0001, 3), "0");
        assert_eq!(fmt_num(10.5, 1), "10.5");
        assert_eq!(fmt_num(100.0, 0), "100");
    }

    #[test]
    fn escaping() {
        assert_eq!(xml_escape("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&apos;");
    }
}
