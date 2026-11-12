//! The native `.vectorcraft` format.
//!
//! A `.vectorcraft` file is UTF-8 JSON, gzip-compressed when saved with Use Compression (told apart
//! by its magic bytes on open, whatever the version):
//! ```json
//! { "format": "vectorcraft", "version": 3, "generator": "VectorCraft 0.1.0",
//!   "preview": { "mime": "image/png", "data": "<base64>" },
//!   "profiles": { "<profile name>": { "mime": "application/vnd.iccprofile", "data": "<base64>" } },
//!   "pdf": { "mime": "application/pdf", "data": "<base64>" },
//!   "document": { …vectorcraft_doc::Document… },
//!   "images": { "<key>": { "mime": "image/png", "data": "<base64>" } } }
//! ```
//! It is lossless for everything in the document model; foreign data lives in `document.unknown`
//! and document keys a newer version added are kept (`Document::extra`). Only the images the
//! document uses are written. `preview` (optional) is a PNG of the first artboard, at most
//! [`PREVIEW_MAX`] pixels on its longer side, for file browsers. `profiles` (optional, Embed ICC
//! Profiles) holds the ICC files of colour profiles the document is tagged with, for machines that
//! don't have them ([`load_file`] returns them). `pdf` (optional, Create PDF-Compatible File) is a
//! PDF of every artboard for apps that read PDF ([`pdf_content`]). An image only linked images
//! show is saved as its low-resolution preview (`"proxy": true`), unless saved with Include Linked
//! Files; the engine reads the linked file again when the document opens.
//!
//! Versions: v1 wrote anchors as `{p: {x, y}, in: {x, y}, out: {x, y}, kind}`; v2 as
//! `{p: [x, y], in?, out?, kind?}` with default-valued fields left out; v3 keeps the assigned colour
//! profiles in `document.color_profiles` (before: `document.unknown.colorProfiles`) and may be
//! compressed; v4 may keep large data (images, placed documents, the PDF, profiles of at least
//! [`INLINE_MAX`] bytes) after the JSON instead of in it: the JSON is followed by [`BLOB_MAGIC`] and
//! the bytes, and such an entry says where they are (`"blob": [offset, length]`) instead of giving
//! `data`. Every version loads, and [`save_with`] writes any of them for older apps (under the
//! format name `drawcraft`, which every version reads). Readers reject files whose `version` is
//! newer than they support.
//!
//! The document has a version of its own (`document.version`, [`vectorcraft_doc::FORMAT_VERSION`]),
//! which readers do not check. A version 2 document's Twist effects turn clockwise for a positive
//! angle. A version 1 document's Twist angles are negated when it opens, unless the file is of
//! [`VERSION`] and its `generator` is VectorCraft 0.21.0 or later, which turned them clockwise
//! already; [`save_with`] writes a version 1 document with them negated back for an older file
//! version ([`vectorcraft_doc::twist`]).
#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod atomic;
mod legacy;
mod stream;
mod unpack;

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::io::Write;
use std::sync::Arc;

use craft_vectorcraft_format::{Contents, Head, Image, ReadError, Tail};
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::{Deserializer, Serialize};
use vectorcraft_doc::{Document, ImageBlob};

#[cfg(not(target_arch = "wasm32"))]
pub use atomic::{sync_file, write_atomic, write_atomic_with, write_new_with};
pub use craft_vectorcraft_format::base64::{decode as base64_decode, decoded_len as base64_decoded_len, encode as base64_encode};
pub use craft_vectorcraft_format::{
    BLOB_MAGIC, COMPRESSED_SINCE, EXTENSION, ICC_MIME, INLINE_MAX, LEGACY_EXTENSION, MIN_VERSION, PREVIEW_MAX, VERSION, is_compressed, json_part,
    sniff,
};
use stream::{Gz, Indent, KeyScan, Out};

/// Most bytes the JSON of a compressed file may unpack to (a guard against decompression bombs,
/// as are [`MAX_BLOB`] and [`MAX_BLOBS`]). A compressed save refuses a document past any of them,
/// so every file VectorCraft writes reads back. On wasm32 the cap is a quarter of the native
/// one: the JSON is deserialized into a document several times its size, and the 4 GiB address
/// space also holds the file, the JSON, the document and the app (#3587).
pub const MAX_JSON: u64 = if cfg!(target_arch = "wasm32") { 256 << 20 } else { 1 << 30 };
/// Most bytes one entry of the data after the JSON (an image, a placed document, the PDF, a
/// profile) may have in a compressed file.
pub const MAX_BLOB: u64 = craft_vectorcraft_format::MAX_ENTRY;
/// Most bytes the entries after the JSON may have together in a compressed file: 1 GiB in the web
/// build, whose 4 GiB address space also holds the JSON, the document and the app, so 4 GiB of
/// entries failed to allocate rather than being refused.
pub const MAX_BLOBS: u64 = if cfg!(target_arch = "wasm32") { 1 << 30 } else { 4 << 30 };
/// Most bytes a compressed file whose JSON isn't followed by the marker where VectorCraft puts it
/// (hand-made or damaged files) may unpack to: it is unpacked whole.
const MAX_UNPACKED: u64 = 512 << 20;

/// How much reading a native file may unpack: a reader that needs less than [`load`] (Finder's
/// Quick Look extensions) gives smaller caps, so a file past them is refused
/// ([`FormatError::TooBig`]) before that much is unpacked or read. A cap larger than its default
/// ([`MAX_JSON`], [`MAX_BLOBS`]) counts as the default. The default caps apply to compressed files
/// only (an uncompressed file is in memory already); tighter ones apply to both.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReadCaps {
    /// Most bytes the JSON may unpack to.
    pub json: u64,
    /// Most bytes the entries after the JSON (images, profiles, the PDF) may have together; each
    /// one is also at most [`MAX_BLOB`].
    pub blobs: u64,
}

impl Default for ReadCaps {
    fn default() -> Self {
        Self { json: MAX_JSON, blobs: MAX_BLOBS }
    }
}

impl ReadCaps {
    /// Each cap at most its default.
    fn within_defaults(self) -> Self {
        Self { json: self.json.min(MAX_JSON), blobs: self.blobs.min(MAX_BLOBS) }
    }
}

/// Is `ext` (without the dot, any case) a native document extension?
pub fn is_native_ext(ext: &str) -> bool {
    ext.eq_ignore_ascii_case(EXTENSION) || ext.eq_ignore_ascii_case(LEGACY_EXTENSION)
}

/// Does the file name or path end in a native document extension?
pub fn is_native_name(name: &str) -> bool {
    std::path::Path::new(name).extension().and_then(|e| e.to_str()).is_some_and(is_native_ext)
}

#[derive(Debug, thiserror::Error)]
pub enum FormatError {
    #[error("not a VectorCraft file: {0}")]
    NotVectorcraft(String),
    #[error("file version {0} is newer than this VectorCraft supports ({VERSION})")]
    TooNew(u32),
    #[error("invalid image data for `{0}`")]
    BadImage(String),
    #[error("can't save version {0}: VectorCraft writes versions {MIN_VERSION} to {VERSION}")]
    BadVersion(u32),
    #[error("version {0} files can't be compressed: only version {COMPRESSED_SINCE} and later open compressed files")]
    CompressedTooOld(u32),
    #[error("the compressed file unpacks to more than VectorCraft reads ({} MB of document, {} MB per image)", MAX_JSON >> 20, MAX_BLOB >> 20)]
    TooBig,
    #[error("the document is too large to save compressed ({0}): save it with Use Compression off")]
    TooBigToCompress(String),
    #[error("can't encode the document: {0}")]
    Encode(String),
}

impl From<craft_vectorcraft_format::Error> for FormatError {
    fn from(e: craft_vectorcraft_format::Error) -> Self {
        match e {
            craft_vectorcraft_format::Error::Format(format) => FormatError::NotVectorcraft(format!("format is `{format}`")),
            craft_vectorcraft_format::Error::TooNew(version) => FormatError::TooNew(version),
            // The crate's other errors (`TooBig`: entries past the data after the JSON) keep their message.
            e => FormatError::NotVectorcraft(e.to_string()),
        }
    }
}

/// Is data of this MIME type compressed already (no use compressing it again)?
fn is_packed(mime: &str) -> bool {
    matches!(mime, "image/jpeg" | "image/png" | "image/gif" | "image/webp")
}

/// The MIME type of a preview's bytes (JPEG or PNG).
fn preview_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xff, 0xd8]) { "image/jpeg" } else { vectorcraft_doc::links::PROXY_MIME }
}

/// How [`save_with`] writes a document.
#[derive(Clone, Debug)]
pub struct SaveOptions {
    /// Indented JSON (readable, diffable); compact otherwise. Compressed files are always compact.
    pub pretty: bool,
    /// gzip the file (Use Compression), for version [`COMPRESSED_SINCE`] and later.
    pub compress: bool,
    /// The version to write ([`MIN_VERSION`]`..=`[`VERSION`]): older ones for older apps.
    pub version: u32,
    /// A PNG preview to embed (at most [`PREVIEW_MAX`] pixels on its longer side).
    pub preview: Option<Vec<u8>>,
    /// Include Linked Files: linked images keep the file's own bytes, not just their preview.
    pub include_linked: bool,
    /// Embed ICC Profiles: ICC files by profile name.
    pub profiles: BTreeMap<String, Vec<u8>>,
    /// Create PDF-Compatible File: a PDF of the document to carry.
    pub pdf: Option<Vec<u8>>,
    /// The application's PDF save preference, retained even when there are no artboards to draw.
    /// `None` omits this metadata, preserving the lower-level writer's existing output.
    pub pdf_compatible: Option<bool>,
    /// Decimal places to round path anchors and handles to ([`Document::round_geometry`]), at
    /// most [`MAX_PRECISION`] (a larger value rounds to that many); `None` writes them exactly.
    pub precision: Option<u8>,
}

/// The most decimal places [`SaveOptions::precision`] rounds path geometry to.
pub const MAX_PRECISION: u8 = 9;

impl Default for SaveOptions {
    fn default() -> Self {
        Self {
            pretty: false,
            compress: false,
            version: VERSION,
            preview: None,
            include_linked: false,
            profiles: BTreeMap::new(),
            pdf: None,
            pdf_compatible: None,
            precision: None,
        }
    }
}

impl SaveOptions {
    /// Can a file be written this way (a version [`save_with`] writes, compressed only for readers
    /// that open compressed files)?
    pub fn check(&self) -> Result<(), FormatError> {
        if !(MIN_VERSION..=VERSION).contains(&self.version) {
            return Err(FormatError::BadVersion(self.version));
        }
        if self.compress && self.version < COMPRESSED_SINCE {
            return Err(FormatError::CompressedTooOld(self.version));
        }
        Ok(())
    }

    /// The defaults for saving `doc` to disk: pretty when small (see [`Self::pretty_fits`]).
    pub fn for_doc(doc: &Document) -> Self {
        let o = Self::default();
        Self { pretty: o.pretty_fits(doc), ..o }
    }

    /// Is `doc` small enough to save pretty-printed with these options: at most
    /// [`PRETTY_MAX_OBJECTS`] objects, and at most [`PRETTY_MAX_BYTES`] of pretty-printed JSON
    /// as [`save_to`] writes the document with them (in their version)?
    pub fn pretty_fits(&self, doc: &Document) -> bool {
        let objects: usize = doc.layers.iter().map(|l| l.count()).sum();
        objects <= PRETTY_MAX_OBJECTS && body(doc, self).is_ok_and(|b| pretty_size_at_most(&b, PRETTY_MAX_BYTES))
    }
}

/// Is the document in `b` at most `limit` bytes as pretty-printed JSON, indented as a pretty file
/// holds it? Serializing stops as soon as it is past the limit, so a large document costs no more
/// than `limit` bytes of work.
fn pretty_size_at_most(b: &Body, limit: usize) -> bool {
    struct Capped {
        left: usize,
    }
    impl Write for Capped {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.left = self.left.checked_sub(buf.len()).ok_or_else(|| std::io::Error::other("over the limit"))?;
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    // The document is a member of the file's top-level object, indented one level more.
    write_body(Indent { inner: Capped { left: limit }, on: true }, b.v1.as_ref(), &b.d, true).is_ok()
}

/// The document as [`save_to`] writes it, with the images it lists outside the document.
struct Body {
    d: Document,
    /// The version 1 form of `d`, written instead of it.
    v1: Option<serde_json::Value>,
    blobs: BTreeMap<String, ImageBlob>,
    /// Images only linked images show (see [`Document::linked_only_images`]).
    linked: std::collections::BTreeSet<String>,
}

/// `doc` as [`save_to`] writes it with `o`: without editing-mode working copies, with placed
/// documents as their art for older versions, path geometry rounded to `o.precision`, and its
/// images taken out.
fn body(doc: &Document, o: &SaveOptions) -> Result<Body, FormatError> {
    let version = o.version;
    let mut d = doc.without_edit_modes().into_owned();
    // Ruler guides are saved in the document's `guides` list, as before they were nodes, so every
    // version reads them.
    d.guides_to_saved();
    // Older apps don't know placed documents: they get the art the objects show, with its
    // resources. Otherwise resources only exporting adds are never saved.
    if version < VERSION && d.has_placed() {
        d = d.with_placed_art().into_owned();
        d.placed_as_groups();
    } else {
        d.drop_placed_resources();
    }
    // The art placed documents became for older versions is rounded too.
    if let Some(decimals) = o.precision {
        d.round_geometry(decimals.min(MAX_PRECISION));
    }
    let linked = d.linked_only_images();
    // The blobs go in the file's `images` (only those the document uses).
    let blobs = std::mem::take(&mut d.images);
    if version < 3 {
        d.store_legacy_color_profiles();
    }
    // Apps before this one's version take Color Halftone's Max Radius in points (#3260), and
    // predate 0.21.0, turning Twist the other way. The radii first: their conversion runs only on
    // a document of version 3 or later, and the Twist one then marks it version 1.
    if version < VERSION {
        d.halftone_radii_to_points();
        d.store_legacy_twist_angles();
    }
    let v1 = if version == 1 {
        let mut v = serde_json::to_value(&d).map_err(encode_err)?;
        legacy::anchors_to_v1(&mut v);
        Some(v)
    } else {
        None
    };
    Ok(Body { d, v1, blobs, linked })
}

/// Serialize a document (pretty = human-diffable) in the current version. Editing-mode working
/// copies are left out ([`Document::without_edit_modes`]).
pub fn save(doc: &Document, pretty: bool) -> Vec<u8> {
    save_with(doc, &SaveOptions { pretty, ..SaveOptions::default() }).unwrap_or_default()
}

/// Documents up to this many objects are saved pretty-printed (readable, diffable); larger ones
/// compact, where indentation would multiply the file size and slow opening down.
pub const PRETTY_MAX_OBJECTS: usize = 2000;

/// Documents whose pretty-printed JSON is larger than this are saved compact even with few
/// objects: pretty-printing spreads every path anchor over several lines, so traced or generated
/// art (in layers, symbols or patterns alike) grows about fivefold.
pub const PRETTY_MAX_BYTES: usize = 1 << 20;

/// Serialize for saving to disk: pretty when small, compact when large.
pub fn save_file(doc: &Document) -> Vec<u8> {
    save_with(doc, &SaveOptions::for_doc(doc)).unwrap_or_default()
}

/// Serialize a document as `o` says: the version, compression, layout and preview. Editing-mode
/// working copies and images nothing uses are left out.
pub fn save_with(doc: &Document, o: &SaveOptions) -> Result<Vec<u8>, FormatError> {
    let mut out = Vec::new();
    save_to(doc, o, &mut out)?;
    Ok(out)
}

/// Bytes of JSON serialized at a time on their way to the output.
const JSON_CHUNK: usize = 64 << 10;

/// [`save_with`], written to `w` as it is made: the JSON is serialized straight into it (through
/// the compressor when compressing) and the images are copied in from the document, so the file
/// is never held in memory as a whole. Give it a buffered writer. On an error `w` has part of a
/// file: write to a temporary file ([`write_atomic_with`]).
pub fn save_to(doc: &Document, o: &SaveOptions, w: impl Write) -> Result<(), FormatError> {
    o.check()?;
    let pretty = o.pretty && !o.compress;
    let Body { d, v1, blobs, linked } = body(doc, o)?;
    let entries = Entries { blobs: &blobs, linked: &linked, include_linked: o.include_linked };
    let mut out_blobs = Tail::new(o.version);
    // Images are only referenced by image objects' `key` (wherever the objects are: layers,
    // symbols, patterns, masks, or foreign data). The data after the JSON has the images first,
    // then the profiles and the PDF, which the file lists before the document: with those, the
    // images in use are looked for before the document is written (it is serialized twice).
    let mut images = None;
    if !blobs.is_empty() && (!o.profiles.is_empty() || o.pdf.is_some()) {
        let mut scan = KeyScan::new(std::io::sink());
        write_body(std::io::BufWriter::with_capacity(JSON_CHUNK, &mut scan), v1.as_ref(), &d, pretty)?;
        images = Some(entries.images(&scan.keys, &mut out_blobs));
    }
    let profiles: BTreeMap<&str, Image> = o.profiles.iter().map(|(name, icc)| (name.as_str(), out_blobs.image(ICC_MIME, icc, false))).collect();
    let pdf = o.pdf.as_deref().map(|p| out_blobs.image("application/pdf", p, false));
    let mut out = if o.compress { Out::Gz(Box::new(Gz::new(w).map_err(encode_err)?)) } else { Out::Plain(w) };
    let mut env = Envelope { pretty, first: true };
    // Older apps only know the name from before the rename.
    env.field(&mut out, "format", &if o.version < 3 { LEGACY_EXTENSION } else { EXTENSION })?;
    env.field(&mut out, "version", &o.version)?;
    env.field(&mut out, "generator", &format!("VectorCraft {}", env!("CARGO_PKG_VERSION")))?;
    if let Some(compatible) = o.pdf_compatible {
        env.field(&mut out, "pdfCompatible", &compatible)?;
    }
    if let Some(png) = &o.preview {
        env.field(&mut out, "preview", &Image::inline("image/png", png))?;
    }
    if !profiles.is_empty() {
        env.field(&mut out, "profiles", &profiles)?;
    }
    if let Some(pdf) = &pdf {
        env.field(&mut out, "pdf", pdf)?;
    }
    env.key(&mut out, "document")?;
    let mut scan = KeyScan::new(Indent { inner: &mut out, on: pretty });
    // Without blobs no key matters.
    scan.active = images.is_none() && !blobs.is_empty();
    let mut buffered = std::io::BufWriter::with_capacity(JSON_CHUNK, &mut scan);
    write_body(&mut buffered, v1.as_ref(), &d, pretty)?;
    buffered.flush().map_err(encode_err)?;
    drop(buffered);
    let used = std::mem::take(&mut scan.keys);
    let images = images.unwrap_or_else(|| entries.images(&used, &mut out_blobs));
    env.field(&mut out, "images", &images)?;
    env.finish(&mut out)?;
    if o.compress {
        let json = out.written().unwrap_or(0);
        let tail: Vec<u64> = out_blobs.parts().iter().map(|p| p.bytes.len() as u64).collect();
        if json > MAX_JSON {
            return Err(FormatError::TooBigToCompress(format!("{} MB of document, at most {} MB", json >> 20, MAX_JSON >> 20)));
        }
        if let Some(big) = tail.iter().find(|&&n| n > MAX_BLOB) {
            return Err(FormatError::TooBigToCompress(format!("an image of {} MB, at most {} MB", big >> 20, MAX_BLOB >> 20)));
        }
        if out_blobs.len() > MAX_BLOBS {
            return Err(FormatError::TooBigToCompress(format!("{} MB of images, at most {} MB", out_blobs.len() >> 20, MAX_BLOBS >> 20)));
        }
    }
    if !out_blobs.is_empty() {
        out.write_all(BLOB_MAGIC).map_err(encode_err)?;
        for part in out_blobs.parts() {
            out.packed(is_packed(&part.mime)).map_err(encode_err)?;
            out.write_all(part.bytes).map_err(encode_err)?;
        }
    }
    out.finish().map_err(encode_err)
}

/// The images a file lists, from the document's blobs.
struct Entries<'a> {
    blobs: &'a BTreeMap<String, ImageBlob>,
    /// Images only linked images show (see [`Document::linked_only_images`]).
    linked: &'a std::collections::BTreeSet<String>,
    include_linked: bool,
}

impl<'a> Entries<'a> {
    /// The entries of the images named in `used`, in the order of their names.
    fn images(&self, used: &HashSet<String>, out: &mut Tail<'a>) -> BTreeMap<&'a str, Image> {
        self.blobs
            .iter()
            .filter(|(k, _)| used.contains(k.as_str()))
            .map(|(k, b)| {
                // Include Linked Files keeps the file's bytes whenever they are loaded.
                let preview_only = self.linked.contains(k) && (!self.include_linked || b.is_proxy());
                let image = match b.proxy.as_ref().filter(|_| preview_only) {
                    Some(p) => out.image(preview_mime(p), p, true),
                    None => out.image(&b.mime, &b.bytes, false),
                };
                (k.as_str(), image)
            })
            .collect()
    }
}

fn encode_err(e: impl std::fmt::Display) -> FormatError {
    FormatError::Encode(e.to_string())
}

fn to_json(v: &impl Serialize, pretty: bool) -> Result<Vec<u8>, FormatError> {
    if pretty { serde_json::to_vec_pretty(v) } else { serde_json::to_vec(v) }.map_err(encode_err)
}

/// The document's JSON: its version 1 form `v1`, else `d` (generic, not `dyn Write`: serde_json
/// writes token by token).
fn write_body(w: impl Write, v1: Option<&serde_json::Value>, d: &Document, pretty: bool) -> Result<(), FormatError> {
    match v1 {
        Some(v) => write_json(w, v, pretty),
        None => write_json(w, d, pretty),
    }
}

fn write_json(w: impl Write, v: &impl Serialize, pretty: bool) -> Result<(), FormatError> {
    if pretty { serde_json::to_writer_pretty(w, v) } else { serde_json::to_writer(w, v) }.map_err(encode_err)
}

/// The file's top-level object, written member by member (the document is streamed in).
struct Envelope {
    pretty: bool,
    first: bool,
}

impl Envelope {
    fn field(&mut self, out: &mut impl Write, key: &str, value: &impl Serialize) -> Result<(), FormatError> {
        let json = to_json(value, self.pretty)?;
        self.key(out, key)?;
        Indent { inner: out, on: self.pretty }.write_all(&json).map_err(encode_err)
    }

    /// The start of member `key` (a plain name): its value follows, indented one level when
    /// pretty (as serde_json would: see [`Indent`]).
    fn key(&mut self, out: &mut impl Write, key: &str) -> Result<(), FormatError> {
        let sep: &[u8] = if std::mem::take(&mut self.first) { b"{" } else { b"," };
        let indent: &[u8] = if self.pretty { b"\n  " } else { b"" };
        let colon: &[u8] = if self.pretty { b"\": " } else { b"\":" };
        [sep, indent, b"\"", key.as_bytes(), colon].iter().try_for_each(|part| out.write_all(part)).map_err(encode_err)
    }

    fn finish(self, out: &mut impl Write) -> Result<(), FormatError> {
        out.write_all(if self.pretty { b"\n}" } else { b"}" }).map_err(encode_err)
    }
}

pub fn load(bytes: &[u8]) -> Result<Document, FormatError> {
    load_info(bytes).map(|(doc, _)| doc)
}

/// A native file read: its document, what it says about itself and the colour profiles it carries.
#[derive(Debug)]
pub struct NativeFile {
    pub doc: Document,
    /// What was done to bring the document within the nesting caps
    /// ([`Document::limit_depth_noting`]: a file saved before them, or a damaged one) and within
    /// the canvas ([`Document::drop_beyond_canvas`]: a damaged or hand-made file).
    pub depth_notes: Vec<String>,
    pub info: FileInfo,
    /// ICC files by profile name (saved with Embed ICC Profiles; undecodable ones are left out).
    pub profiles: BTreeMap<String, Vec<u8>>,
}

/// What a native file says about itself besides its document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileInfo {
    /// The file's format version (see [`VERSION`]).
    pub version: u32,
    /// Written under the project's former name (`"format": "drawcraft"`).
    pub legacy: bool,
    /// The saved `pdfCompatible` preference, or the presence of a non-null `pdf` member in
    /// files without a boolean preference. The PDF payload is not read or validated.
    pub pdf_compatible: bool,
}

impl FileInfo {
    /// Written by an older version or under the former name: saving rewrites it in today's format.
    pub fn is_old(&self) -> bool {
        self.legacy || self.version < VERSION
    }
}

/// [`load`], also telling the file's version and name.
pub fn load_info(bytes: &[u8]) -> Result<(Document, FileInfo), FormatError> {
    load_file(bytes).map(|f| (f.doc, f.info))
}

/// [`load`], with everything else the file carries.
pub fn load_file(bytes: &[u8]) -> Result<NativeFile, FormatError> {
    load_file_within(bytes, ReadCaps::default())
}

/// [`load`] unpacking no more than `caps` allow ([`FormatError::TooBig`] past them).
pub fn load_within(bytes: &[u8], caps: ReadCaps) -> Result<Document, FormatError> {
    load_file_within(bytes, caps).map(|f| f.doc)
}

/// [`load_file`] unpacking no more than `caps` allow ([`FormatError::TooBig`] past them).
pub fn load_file_within(bytes: &[u8], caps: ReadCaps) -> Result<NativeFile, FormatError> {
    load_file_budgeted(bytes, caps, vectorcraft_doc::node::MAX_READ_NODES, vectorcraft_doc::node::MAX_BUFFERED_VALUES)
}

/// [`load_file_within`] reading at most `max_nodes` objects and buffering at most `max_values`
/// values before objects' types, so a test can exercise the budgets without documents that large
/// ([`vectorcraft_doc::node::MAX_READ_NODES`], [`vectorcraft_doc::node::MAX_BUFFERED_VALUES`]).
fn load_file_budgeted(bytes: &[u8], caps: ReadCaps, max_nodes: usize, max_values: usize) -> Result<NativeFile, FormatError> {
    let file = unpack::Unpacked::open(bytes, caps)?;
    // The JSON reader stops at 128 levels (a document's objects use three each), the paths read
    // at `MAX_READ_ANCHORS` anchors together, the document at `MAX_READ_NODES` objects, and the
    // fields read before an object's type at `MAX_BUFFERED_VALUES` values. The preview and the
    // PDF are skipped without being kept.
    fn within_budgets<T>(max_nodes: usize, max_values: usize, read: impl FnOnce() -> T) -> T {
        #[cfg(test)]
        DOCUMENT_READS.with(|n| n.set(n.get() + 1));
        vectorcraft_doc::geom::path::with_anchor_budget(vectorcraft_doc::geom::path::MAX_READ_ANCHORS, || {
            vectorcraft_doc::node::with_node_budget(max_nodes, || vectorcraft_doc::node::with_buffered_value_budget(max_values, read))
        })
    }
    let read_error = |e| match e {
        ReadError::Json(e) => {
            let why = if e.to_string().contains("recursion limit") { format!("its objects are nested too deep ({e})") } else { e.to_string() };
            FormatError::NotVectorcraft(why)
        }
        ReadError::Envelope(e) => e.into(),
        e => FormatError::NotVectorcraft(e.to_string()),
    };
    // The document and the saved PDF preference come from one pass over the JSON.
    let (mut f, pdf_compatible) = match within_budgets(max_nodes, max_values, || Contents::<Document>::read_with_pdf_preference(file.json())) {
        Ok(read) => read,
        // That pass refuses a `pdfCompatible` that is not a boolean, and any other data error.
        // When it stopped at the preference, the file is read again without it, and only then is
        // its JSON scanned for the preference a second time, so such a file opens as before. Any
        // other error, such as a document past a budget, is returned without a second read.
        Err(ReadError::Json(e)) if e.is_data() && refused_preference_at(file.json(), &e) => {
            let f = within_budgets(max_nodes, max_values, || Contents::<Document>::read(file.json())).map_err(read_error)?;
            let pdf_compatible = pdf_member(file.json()).map_err(|e| FormatError::NotVectorcraft(e.to_string()))?;
            (f, pdf_compatible)
        }
        Err(e) => return Err(read_error(e)),
    };
    let info = FileInfo { version: f.version, legacy: f.format == LEGACY_EXTENSION, pdf_compatible };
    // A version 1 document saved before VectorCraft 0.21.0, or for an app before it (an older file
    // version): its Twist angles turned the other way. The app that wrote it is read from the JSON
    // before the data after it is taken; the walk's depth is bounded by the JSON reader's nesting
    // limit.
    let current_file = f.version >= VERSION;
    f.document.migrate_twist_angles(|| current_file && vectorcraft_doc::twist::written_clockwise(generator(file.json()).as_deref()));
    // The bytes after the JSON, read in one pass: the images' first, then the profiles'.
    let entries: Vec<[u64; 2]> = f.images.values().chain(f.profiles.values()).filter_map(|i| i.blob).collect();
    let tail = file.entries(&entries)?;
    // Entries naming the same bytes share them; a profile's own copy of shared bytes counts
    // against the data's size, so copies never add up to more than the file has (#3574).
    let mut distinct = HashSet::new();
    let mut spare: u64 = tail.iter().flatten().filter(|b| distinct.insert(Arc::as_ptr(b))).map(|b| b.len() as u64).sum();
    let mut tail = tail.into_iter();
    let mut bytes_of = |img: &Image| match img.blob {
        Some(_) => tail.next().flatten(),
        None => base64_decode(&img.data).map(Arc::new),
    };
    let mut doc = f.document;
    for (k, img) in f.images {
        let bytes = bytes_of(&img).ok_or_else(|| FormatError::BadImage(k.clone()))?;
        let mut blob = ImageBlob { mime: img.mime, bytes, proxy: None };
        // Until the linked file is read, the preview stands in for it.
        if img.proxy {
            blob.proxy = Some(blob.bytes.clone());
        }
        doc.images.insert(k, blob);
    }
    // Saved mid-edit by an older version: drop the opacity-mask editing layer. Those versions
    // removed it only as a top-level layer, so an id naming anything else is ignored.
    if doc.mask_edit.is_some_and(|me| !doc.layers.iter().any(|l| l.id == me.layer)) {
        doc.mask_edit = None;
    }
    doc.drop_edit_modes();
    // Saved before per-fill/stroke overprint: the Overprint Black list becomes overprint flags.
    doc.migrate_overprint_black();
    // Saved before v3: the assigned profiles move out of `unknown`.
    doc.migrate_color_profiles();
    // A document before version 3: Color Halftone's Max Radius in points becomes pixels (#3260).
    doc.migrate_halftone_radii();
    doc.fix_next_id();
    // Untrusted: the bleed and type proportions, within the ranges Document Setup takes.
    doc.setup.keep_in_range();
    // Untrusted: capped, named and pointing at objects the file has.
    doc.tidy_saved_selections();
    // Untrusted: capped, named and with a zoom and centre the canvas can show.
    doc.tidy_views();
    doc.tidy_graph_designs();
    doc.raster_effects.add_around = doc.raster_effects.add_around.clamp(0.0, vectorcraft_doc::rastersettings::MAX_ADD_AROUND);
    let mut depth_notes = vec![];
    doc.limit_depth_noting(&mut depth_notes);
    // Untrusted: ruler guide nodes only directly in layers, above their art (after the depth limit:
    // the walk recurses; before the canvas check, which takes a layer's first member as its
    // clipping path).
    doc.tidy_ruler_guides();
    // Untrusted: nothing beyond the canvas, where no edit may leave anything (after the depth
    // limit: the walk recurses). A file whose artboards all lie there has no page to show (#3575).
    let had_artboards = !doc.artboards.is_empty();
    let beyond = doc.drop_beyond_canvas();
    if had_artboards && doc.artboards.is_empty() {
        return Err(FormatError::NotVectorcraft(format!(
            "its artboards all lie beyond the canvas (more than {} pt from the origin)",
            vectorcraft_doc::MAX_COORD
        )));
    }
    depth_notes.extend(beyond.note());
    // The saved ruler guides become nodes on their layers (after the guides beyond the canvas are
    // dropped, and after `fix_next_id`: new ids start above the file's).
    doc.guides_into_nodes();
    // Untrusted: the type's sizes, leading, baseline shifts and scales, its indents and paragraph
    // spacing, and its tab stops, each within its range (after the depth limit: the walk recurses).
    doc.keep_type_values_in_range();
    // Untrusted: ruby ranges over the type's text, clamped to it (after the depth limit too).
    doc.fix_ruby();
    // Untrusted: the raster effects resolution, within 1 to 2400 ppi (72 when it isn't finite).
    doc.raster_effects_ppi = Some(doc.raster_effects_ppi).filter(|v| v.is_finite()).unwrap_or(72.0).clamp(1.0, 2400.0);
    // Untrusted: the superscript, subscript and small caps proportions of the type, the graph
    // labels and the character styles, within the ranges `document.setup` takes (after the depth
    // limit: the walk recurses).
    doc.keep_scripts_in_range();
    // The conversions above bring an older document to today's version.
    doc.version = doc.version.max(vectorcraft_doc::FORMAT_VERSION);
    let profiles = f
        .profiles
        .into_iter()
        .filter_map(|(name, icc)| Some((name, bytes_of(&icc)?)))
        .map(|(name, icc)| match Arc::try_unwrap(icc) {
            Ok(icc) => Ok((name, icc)),
            Err(icc) => {
                spare = spare.checked_sub(icc.len() as u64).ok_or(FormatError::TooBig)?;
                Ok((name, icc.to_vec()))
            }
        })
        .collect::<Result<_, FormatError>>()?;
    Ok(NativeFile { doc, depth_notes, info, profiles })
}

#[cfg(test)]
thread_local! {
    /// Tests: how many times a file's JSON was scanned again for its PDF preference on this thread.
    static PDF_MEMBER_SCANS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    /// Tests: how many times a file's document was read on this thread.
    static DOCUMENT_READS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Whether reading `json`'s `pdfCompatible` as a boolean or null fails with a data error at the
/// line and column of `e`, that is, whether the read that gave `e` stopped at the preference. Only
/// the bytes up to that position, and the one there, are read, and they are skipped without being
/// kept.
fn refused_preference_at(json: &[u8], e: &serde_json::Error) -> bool {
    // serde_json gives a 1-based line and the bytes before the position on that line.
    let Some(line_start) = (match e.line() {
        0 => None,
        1 => Some(0),
        n => json.iter().enumerate().filter(|(_, b)| **b == b'\n').nth(n - 2).map(|(i, _)| i + 1),
    }) else {
        return false;
    };
    let end = line_start.saturating_add(e.column()).saturating_add(1).min(json.len());
    struct StrictPreference;
    impl<'de> Visitor<'de> for StrictPreference {
        type Value = ();

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a native file envelope")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
            while let Some(key) = map.next_key::<String>()? {
                if key == "pdfCompatible" {
                    map.next_value::<Option<bool>>()?;
                } else {
                    map.next_value::<IgnoredAny>()?;
                }
            }
            Ok(())
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(())
        }
    }
    serde_json::Deserializer::from_slice(&json[..end])
        .deserialize_any(StrictPreference)
        .is_err_and(|s| s.is_data() && s.line() == e.line() && s.column() == e.column())
}

/// The saved PDF preference, falling back to the presence of its payload without retaining or
/// decoding it. The array form has neither member. Repeated members use their last value.
fn pdf_member(json: &[u8]) -> Result<bool, serde_json::Error> {
    #[cfg(test)]
    PDF_MEMBER_SCANS.with(|n| n.set(n.get() + 1));
    struct PdfMember;
    impl<'de> Visitor<'de> for PdfMember {
        type Value = bool;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a native file envelope")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<bool, A::Error> {
            let mut present = false;
            let mut preference = None;
            while let Some(key) = map.next_key::<String>()? {
                match key.as_str() {
                    "pdfCompatible" => preference = map.next_value_seed(Preference)?,
                    "pdf" => present = map.next_value::<Option<IgnoredAny>>()?.is_some(),
                    _ => {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
            }
            Ok(preference.unwrap_or(present))
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<bool, A::Error> {
            while seq.next_element::<IgnoredAny>()?.is_some() {}
            Ok(false)
        }
    }
    serde_json::Deserializer::from_slice(json).deserialize_any(PdfMember)
}

/// A `pdfCompatible` value: `Some` for a boolean, `None` for null or any other value, which is
/// skipped without being kept, as an unknown member is.
struct Preference;

impl<'de> DeserializeSeed<'de> for Preference {
    type Value = Option<bool>;

    fn deserialize<D: Deserializer<'de>>(self, d: D) -> Result<Option<bool>, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Preference {
    type Value = Option<bool>;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("any value")
    }

    fn visit_bool<E>(self, v: bool) -> Result<Option<bool>, E> {
        Ok(Some(v))
    }

    fn visit_i64<E>(self, _: i64) -> Result<Option<bool>, E> {
        Ok(None)
    }

    fn visit_u64<E>(self, _: u64) -> Result<Option<bool>, E> {
        Ok(None)
    }

    fn visit_f64<E>(self, _: f64) -> Result<Option<bool>, E> {
        Ok(None)
    }

    fn visit_str<E>(self, _: &str) -> Result<Option<bool>, E> {
        Ok(None)
    }

    fn visit_unit<E>(self) -> Result<Option<bool>, E> {
        Ok(None)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Option<bool>, A::Error> {
        IgnoredAny.visit_seq(seq).map(|_| None)
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Option<bool>, A::Error> {
        IgnoredAny.visit_map(map).map(|_| None)
    }
}

/// The file's first `generator` member, the app that wrote it (`None`: it has none, it is not a
/// string, or the file is not an object of members). The other members, and any further
/// `generator`, are skipped without being kept, as reading the file skips them.
fn generator(json: &[u8]) -> Option<String> {
    struct Generator(Option<String>);
    struct Members;
    impl<'de> serde::de::Visitor<'de> for Members {
        type Value = Generator;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a file's members")
        }
        fn visit_map<A: serde::de::MapAccess<'de>>(self, mut m: A) -> Result<Generator, A::Error> {
            let mut first: Option<serde_json::Value> = None;
            while let Some(key) = m.next_key::<std::borrow::Cow<'de, str>>()? {
                if key == "generator" && first.is_none() {
                    first = Some(m.next_value()?);
                } else {
                    m.next_value::<serde::de::IgnoredAny>()?;
                }
            }
            Ok(Generator(first.and_then(|v| v.as_str().map(str::to_owned))))
        }
    }
    impl<'de> serde::Deserialize<'de> for Generator {
        fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
            d.deserialize_map(Members)
        }
    }
    serde_json::from_slice::<Generator>(json).ok()?.0
}

/// The bytes of the member of the file's head `pick` picks (only that entry of the data after
/// the JSON is kept).
fn head_member(bytes: &[u8], pick: impl FnOnce(Head) -> Option<Image>) -> Option<Vec<u8>> {
    let file = unpack::Unpacked::open(bytes, ReadCaps::default()).ok()?;
    let head: Head = serde_json::from_slice(file.json()).ok()?;
    let img = pick(head)?;
    match img.blob {
        Some(at) => file.entries(&[at]).ok()?.pop().flatten().map(Arc::unwrap_or_clone),
        None => base64_decode(&img.data),
    }
}

/// The preview PNG embedded in a native file (`None`: it has none, or isn't a native file).
pub fn preview(bytes: &[u8]) -> Option<Vec<u8>> {
    head_member(bytes, |h| h.preview)
}

/// The preview PNG embedded in a native file, after the checks [`load`] makes on the file itself:
/// `Ok(None)` when a file this version reads has no preview; an error when the bytes aren't a
/// native file, were written by a newer version ([`FormatError::TooNew`]), unpack past
/// [`MAX_JSON`], [`MAX_BLOB`] or [`MAX_BLOBS`] ([`FormatError::TooBig`]), or the preview's bytes
/// are missing. The document itself is not read (Finder's thumbnails, #663).
pub fn preview_checked(bytes: &[u8]) -> Result<Option<Vec<u8>>, FormatError> {
    preview_checked_within(bytes, ReadCaps::default())
}

/// [`preview_checked`] unpacking no more than `caps` allow ([`FormatError::TooBig`] past them):
/// the JSON within `caps.json`, and the data after it (read only when the preview is kept there)
/// within `caps.blobs`.
pub fn preview_checked_within(bytes: &[u8], caps: ReadCaps) -> Result<Option<Vec<u8>>, FormatError> {
    #[derive(serde::Deserialize)]
    struct Checked {
        format: String,
        version: u32,
        preview: Option<Image>,
    }
    let file = unpack::Unpacked::open(bytes, caps)?;
    let head: Checked = serde_json::from_slice(file.json()).map_err(|e| FormatError::NotVectorcraft(e.to_string()))?;
    craft_vectorcraft_format::check(&head.format, head.version)?;
    let Some(img) = head.preview else { return Ok(None) };
    let png = match img.blob {
        Some(at) => file.entries(&[at])?.pop().flatten().map(Arc::unwrap_or_clone),
        None => base64_decode(&img.data),
    };
    png.map(Some).ok_or_else(|| FormatError::BadImage("preview".into()))
}

/// The PDF a native file saved with Create PDF-Compatible File carries (`None`: it has none, or
/// isn't a native file).
pub fn pdf_content(bytes: &[u8]) -> Option<Vec<u8>> {
    head_member(bytes, |h| h.pdf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_from_before_the_rename_still_open() {
        let d = Document::new(100.0, 50.0);
        let legacy = String::from_utf8(save(&d, false)).unwrap().replacen("\"format\":\"vectorcraft\"", "\"format\":\"drawcraft\"", 1);
        assert!(legacy.contains("\"drawcraft\""));
        assert!(sniff(legacy.as_bytes()));
        assert_eq!(load(legacy.as_bytes()).unwrap().artboards[0].rect, d.artboards[0].rect);
        assert!(is_native_name("old/Poster.DrawCraft") && is_native_name("new.vectorcraft") && !is_native_name("x.svg"));
        assert!(load(br#"{"format":"other","version":1,"document":{}}"#).is_err());
    }

    #[test]
    fn preview_checked_reads_only_files_this_version_reads() {
        let d = Document::new(10.0, 10.0);
        let small = vec![7u8; 100];
        // Past INLINE_MAX (the writer keeps a preview in the JSON whatever its size).
        let big: Vec<u8> = (0..INLINE_MAX + 100).map(|i| (i % 251) as u8).collect();
        for compress in [false, true] {
            for png in [&small, &big] {
                let o = SaveOptions { compress, preview: Some(png.clone()), ..SaveOptions::default() };
                let bytes = save_with(&d, &o).unwrap();
                assert_eq!(preview_checked(&bytes).unwrap().as_ref(), Some(png), "compress {compress}, {} bytes", png.len());
                assert_eq!(preview(&bytes).as_ref(), Some(png));
            }
        }
        let plain = save(&d, false);
        assert!(preview_checked(&plain).unwrap().is_none(), "no preview is no error");
        let text = String::from_utf8(save_with(&d, &SaveOptions { preview: Some(small.clone()), ..SaveOptions::default() }).unwrap()).unwrap();
        let legacy = text.replacen("\"format\":\"vectorcraft\"", "\"format\":\"drawcraft\"", 1);
        assert_eq!(preview_checked(legacy.as_bytes()).unwrap(), Some(small.clone()), "the former name");
        let newer = text.replacen(&format!("\"version\":{VERSION}"), "\"version\":99", 1);
        assert!(matches!(preview_checked(newer.as_bytes()), Err(FormatError::TooNew(99))));
        assert!(preview(newer.as_bytes()).is_some(), "the unchecked read doesn't look");
        let other = text.replacen("\"format\":\"vectorcraft\"", "\"format\":\"other\"", 1);
        assert!(matches!(preview_checked(other.as_bytes()), Err(FormatError::NotVectorcraft(_))));
        for junk in [&b""[..], b"{", b"\x1f\x8b\x08garbage", b"{\"format\":\"vectorcraft\"}", b"[1,2]"] {
            assert!(preview_checked(junk).is_err(), "{junk:?}");
        }
        // A cut file.
        let o = SaveOptions { preview: Some(big.clone()), ..SaveOptions::default() };
        let whole = save_with(&d, &o).unwrap();
        let cut = &whole[..whole.len() - 50];
        assert!(preview_checked(cut).is_err());
    }

    /// A v4 file whose preview is kept after the JSON (as another writer may keep it): `d` saved
    /// without one, with `preview` naming `png` placed after `before` bytes of other data.
    fn with_preview_after_the_json(d: &Document, before: usize, png: &[u8], compress: bool) -> Vec<u8> {
        let text = String::from_utf8(save(d, false)).unwrap();
        let body = text.strip_prefix('{').unwrap();
        let json = format!("{{\"preview\":{{\"mime\":\"image/png\",\"blob\":[{before},{}]}},{body}", png.len());
        let mut file = json.into_bytes();
        file.extend_from_slice(BLOB_MAGIC);
        file.extend(std::iter::repeat_n(0x55u8, before));
        file.extend_from_slice(png);
        if !compress {
            return file;
        }
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        std::io::Write::write_all(&mut gz, &file).unwrap();
        gz.finish().unwrap()
    }

    #[test]
    fn previews_after_the_json_are_read_within_the_caps() {
        use std::io::Read as _;
        let d = Document::new(10.0, 10.0);
        let png: Vec<u8> = (0..INLINE_MAX + 100).map(|i| (i % 251) as u8).collect();
        for compress in [false, true] {
            let file = with_preview_after_the_json(&d, 1000, &png, compress);
            assert_eq!(preview_checked(&file).unwrap(), Some(png.clone()), "compress {compress}");
            assert_eq!(preview(&file), Some(png.clone()));
            assert!(load(&file).is_ok(), "the document reads too");
            // Past the end of the data.
            let cut = &file[..file.len() - 50];
            assert!(preview_checked(cut).is_err(), "cut, compress {compress}");
        }
        // Compressed, the caps apply as the file is unpacked.
        let file = with_preview_after_the_json(&d, 1 << 20, &png, true);
        let json_len = {
            let mut all = Vec::new();
            flate2::read::GzDecoder::new(file.as_slice()).read_to_end(&mut all).unwrap();
            all.windows(BLOB_MAGIC.len()).position(|w| w == BLOB_MAGIC).unwrap() as u64
        };
        let roomy = ReadCaps { json: json_len, blobs: (1 << 20) + png.len() as u64 };
        assert_eq!(preview_checked_within(&file, roomy).unwrap(), Some(png.clone()));
        let short_json = ReadCaps { json: json_len - 1, ..roomy };
        assert!(matches!(preview_checked_within(&file, short_json), Err(FormatError::TooBig)));
        // Room for the preview itself, not for the data before it.
        let short_blobs = ReadCaps { blobs: png.len() as u64, ..roomy };
        assert!(matches!(preview_checked_within(&file, short_blobs), Err(FormatError::TooBig)));
        assert!(matches!(load_within(&file, short_json), Err(FormatError::TooBig)));
        // A tighter JSON cap applies to an uncompressed file too; the default one doesn't.
        let plain = with_preview_after_the_json(&d, 1000, &png, false);
        let plain_json = plain.windows(BLOB_MAGIC.len()).position(|w| w == BLOB_MAGIC).unwrap() as u64;
        let fits = ReadCaps { json: plain_json, ..ReadCaps::default() };
        assert_eq!(preview_checked_within(&plain, fits).unwrap(), Some(png.clone()));
        let tight = ReadCaps { json: plain_json - 1, ..ReadCaps::default() };
        assert!(matches!(preview_checked_within(&plain, tight), Err(FormatError::TooBig)));
        assert!(matches!(load_within(&plain, tight), Err(FormatError::TooBig)));
        // So does a tighter cap on the data after the JSON.
        let room = ReadCaps { blobs: png.len() as u64, ..ReadCaps::default() };
        assert_eq!(preview_checked_within(&plain, room).unwrap(), Some(png.clone()));
        let short = ReadCaps { blobs: png.len() as u64 - 1, ..ReadCaps::default() };
        assert!(matches!(preview_checked_within(&plain, short), Err(FormatError::TooBig)));
        // Caps past the defaults are the defaults.
        let huge = ReadCaps { json: u64::MAX, blobs: u64::MAX };
        assert_eq!(huge.within_defaults(), ReadCaps::default());
    }

    #[test]
    fn file_info_keeps_pdf_presence_without_reading_its_payload() {
        let d = Document::new(10.0, 10.0);
        for version in [3, 4] {
            for compress in [false, true] {
                for present in [false, true] {
                    let options = SaveOptions { version, compress, pdf: present.then(|| vec![7; 128 * 1024]), ..SaveOptions::default() };
                    let bytes = save_with(&d, &options).unwrap();
                    assert_eq!(load_info(&bytes).unwrap().1.pdf_compatible, present);
                }
            }
        }
        let mut value: serde_json::Value = serde_json::from_slice(&save(&d, false)).unwrap();
        let array = serde_json::json!([EXTENSION, VERSION, value["document"], value["images"], {}]);
        assert!(!load_info(&serde_json::to_vec(&array).unwrap()).unwrap().1.pdf_compatible);
        value["pdf"] = serde_json::json!({"mime":"application/pdf", "data":"not base64"});
        assert!(load_info(&serde_json::to_vec(&value).unwrap()).unwrap().1.pdf_compatible);
        value["pdf"] = serde_json::Value::Null;
        value["unrecognized"] = serde_json::json!({"pdf":{"data":"nested"}});
        assert!(!load_info(&serde_json::to_vec(&value).unwrap()).unwrap().1.pdf_compatible);
        // A preference that is not a boolean is skipped like an unknown member: the file opens,
        // and the presence of the PDF decides.
        for odd in [serde_json::json!(1), serde_json::json!("yes"), serde_json::json!([true]), serde_json::json!({"on": true})] {
            for (pdf, present) in [(serde_json::Value::Null, false), (serde_json::json!({"mime":"application/pdf"}), true)] {
                value["pdf"] = pdf;
                value["pdfCompatible"] = odd.clone();
                assert_eq!(load_info(&serde_json::to_vec(&value).unwrap()).unwrap().1.pdf_compatible, present, "{odd}");
            }
        }
    }

    #[test]
    fn only_a_preference_that_is_not_a_boolean_scans_the_json_again() {
        let scans_for = |bytes: &[u8]| {
            PDF_MEMBER_SCANS.with(|n| n.set(0));
            let compatible = load_info(bytes).unwrap().1.pdf_compatible;
            (compatible, PDF_MEMBER_SCANS.with(|n| n.get()))
        };
        let d = Document::new(10.0, 10.0);
        assert_eq!(scans_for(&save(&d, false)), (false, 0));
        assert_eq!(scans_for(&save(&d, true)), (false, 0));
        for compatible in [false, true] {
            let bytes = save_with(&d, &SaveOptions { pdf_compatible: Some(compatible), ..SaveOptions::default() }).unwrap();
            assert_eq!(scans_for(&bytes), (compatible, 0));
        }
        let with_pdf = save_with(&d, &SaveOptions { pdf: Some(vec![7; 1024]), ..SaveOptions::default() }).unwrap();
        assert_eq!(scans_for(&with_pdf), (true, 0));
        let mut value: serde_json::Value = serde_json::from_slice(&save(&d, false)).unwrap();
        for odd in [serde_json::json!("yes"), serde_json::json!(1), serde_json::json!([true])] {
            value["pdfCompatible"] = odd.clone();
            assert_eq!(scans_for(&serde_json::to_vec(&value).unwrap()), (false, 1), "{odd}");
        }
        // A document that can't be read still fails. Read before the preference, it is not read
        // a second time.
        value["document"] = serde_json::json!("not a document");
        DOCUMENT_READS.with(|n| n.set(0));
        assert!(load_info(&serde_json::to_vec(&value).unwrap()).is_err());
        assert_eq!(DOCUMENT_READS.with(|n| n.get()), 1);
        // Read after a preference that is not a boolean, it fails on the second read, which is
        // the first to reach it.
        let mut first = serde_json::to_vec(&value).unwrap();
        assert!(first.starts_with(br#"{"document""#));
        first.splice(1..1, br#""pdfCompatible":"yes","#.iter().copied());
        DOCUMENT_READS.with(|n| n.set(0));
        assert!(load_info(&first).is_err());
        assert_eq!(DOCUMENT_READS.with(|n| n.get()), 2);
    }

    /// A document past a read budget is refused after one read, also in a file whose preference
    /// is not a boolean.
    #[test]
    fn a_document_past_a_budget_is_read_once() {
        let mut d = Document::new(100.0, 100.0);
        let l = d.add_layer(None);
        for i in 0..20 {
            let id = d.alloc_id();
            d.insert(Some(l), i, Node::path(id, shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art())).unwrap();
        }
        let n = d.node_count();
        let mut value: serde_json::Value = serde_json::from_slice(&save(&d, false)).unwrap();
        for preference in [serde_json::Value::Null, serde_json::json!(true), serde_json::json!("yes")] {
            value["pdfCompatible"] = preference.clone();
            let bytes = serde_json::to_vec(&value).unwrap();
            DOCUMENT_READS.with(|r| r.set(0));
            let Err(FormatError::NotVectorcraft(why)) =
                load_file_budgeted(&bytes, ReadCaps::default(), n - 1, vectorcraft_doc::node::MAX_BUFFERED_VALUES)
            else {
                panic!("one object past the budget must be refused")
            };
            assert!(why.contains(&format!("more than {} objects", n - 1)), "{why}");
            assert_eq!(DOCUMENT_READS.with(|r| r.get()), 1, "{preference}");
        }
    }

    #[test]
    fn file_info_keeps_the_pdf_preference_without_artboards() {
        let mut d = Document::new(10.0, 10.0);
        d.artboards.clear();
        for compatible in [false, true] {
            let bytes = save_with(&d, &SaveOptions { pdf_compatible: Some(compatible), ..SaveOptions::default() }).unwrap();
            assert_eq!(load_info(&bytes).unwrap().1.pdf_compatible, compatible);
            assert!(pdf_content(&bytes).is_none());
        }
    }

    #[test]
    fn file_info_tells_old_files() {
        let d = Document::new(10.0, 10.0);
        let current = save(&d, false);
        assert_eq!(load_info(&current).unwrap().1, FileInfo { version: VERSION, legacy: false, pdf_compatible: false });
        assert!(!load_info(&current).unwrap().1.is_old());
        let text = String::from_utf8(current).unwrap();
        let legacy = text.replacen("\"format\":\"vectorcraft\"", "\"format\":\"drawcraft\"", 1);
        assert!(load_info(legacy.as_bytes()).unwrap().1.is_old());
        let v1 = text.replacen(&format!("\"version\":{VERSION}"), "\"version\":1", 1);
        assert_eq!(load_info(v1.as_bytes()).unwrap().1, FileInfo { version: 1, legacy: false, pdf_compatible: false });
        assert!(load_info(v1.as_bytes()).unwrap().1.is_old());
    }

    /// A file is native when its top-level `format` member names VectorCraft, read as JSON reads
    /// it; the word elsewhere in the head does not count.
    #[test]
    fn sniff_reads_the_format_member() {
        // `vectorcraft` with its second `c` written as a JSON escape.
        let escaped = [br#"{"format":"vector\"#.as_slice(), br#"u0063raft","version":4,"document":{}}"#].concat();
        assert!(sniff(&escaped));
        assert!(!sniff(br#"{"format":"other","generator":"vectorcraft"}"#));
        assert!(!sniff(br#"{"names":["vectorcraft"],"format":"svg"}"#));
    }

    /// Every layout VectorCraft has written puts `format` first: the version 1 and 2 files from
    /// before the rename (a struct of `format`, `version`, `generator`, `document` and `images`,
    /// compact or pretty), and each version `save_with` writes, compact, pretty and compressed.
    #[test]
    fn sniff_takes_every_layout_written() {
        let before_rename = [
            r#"{"format":"drawcraft","version":1,"generator":"DrawCraft 0.1.0","document":{},"images":{}}"#,
            "{\n  \"format\": \"drawcraft\",\n  \"version\": 2,\n  \"generator\": \"DrawCraft 0.1.0\",\n  \"document\": {},\n  \"images\": {}\n}",
        ];
        for text in before_rename {
            assert!(sniff(text.as_bytes()), "{text}");
        }
        let d = Document::new(10.0, 10.0);
        for version in MIN_VERSION..=VERSION {
            for (pretty, compress) in [(false, false), (true, false), (false, true)] {
                let o = SaveOptions { pretty, compress, version, ..SaveOptions::default() };
                if o.check().is_err() {
                    continue;
                }
                let bytes = save_with(&d, &o).unwrap();
                assert!(sniff(&bytes), "version {version}, pretty {pretty}, compress {compress}");
            }
        }
    }

    /// An envelope error other than the format and version keeps the crate's message.
    #[test]
    fn envelope_errors_keep_their_message() {
        let e = FormatError::from(craft_vectorcraft_format::Error::TooBig);
        assert!(matches!(e, FormatError::NotVectorcraft(_)), "{e}");
        assert!(e.to_string().contains(&craft_vectorcraft_format::Error::TooBig.to_string()), "{e}");
        let e = FormatError::from(craft_vectorcraft_format::Error::Format("x".into()));
        assert_eq!(e.to_string(), "not a VectorCraft file: format is `x`");
        assert!(matches!(FormatError::from(craft_vectorcraft_format::Error::TooNew(9)), FormatError::TooNew(9)));
    }

    /// `save_to` writes the file member by member and streams the document in. For every version
    /// it writes, compact, pretty and compressed, its bytes are those of the crate's `File`
    /// holding the same members, serialized with `serde_json::to_vec` or `to_vec_pretty` and
    /// followed by the crate's `Tail`.
    #[test]
    fn save_to_writes_the_bytes_of_the_crates_file() {
        use std::io::Read;

        /// The document as `save_to` writes it: its version 1 form, or the document.
        struct Written<'a>(&'a Body);
        impl Serialize for Written<'_> {
            fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                match &self.0.v1 {
                    Some(v) => v.serialize(s),
                    None => self.0.d.serialize(s),
                }
            }
        }
        let mut d = Document::new(100.0, 100.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::path(id, shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art())).unwrap();
        // An image large enough to go after the JSON from version 4, a small one, and one that
        // nothing uses.
        let big: Vec<u8> = (0..INLINE_MAX as u32 + 5).map(|i| (i % 251) as u8).collect();
        for (key, bytes) in [("big", big), ("small", vec![1, 2, 3, 250]), ("unused", vec![9])] {
            d.images.insert(key.into(), ImageBlob::new("image/png", bytes));
        }
        for (i, key) in ["big", "small"].into_iter().enumerate() {
            let img = d.alloc_id();
            let im =
                ImageObject { key: key.into(), width: 1, height: 1, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
            d.insert(Some(l), i + 1, Node::new(img, NodeKind::Image(Box::new(im)))).unwrap();
        }
        let png = vec![0x89, b'P', b'N', b'G'];
        let icc = vec![7u8; INLINE_MAX];
        let mut pdf = b"%PDF-1.7\n".to_vec();
        pdf.resize(INLINE_MAX + 1, b' ');
        let full = SaveOptions {
            preview: Some(png.clone()),
            profiles: [("Gray".to_string(), icc.clone())].into(),
            pdf: Some(pdf.clone()),
            ..SaveOptions::default()
        };
        let mut checked = 0;
        for version in MIN_VERSION..=VERSION {
            for members in [&full, &SaveOptions::default()] {
                for pretty in [false, true] {
                    let o = SaveOptions { version, pretty, ..members.clone() };
                    let b = body(&d, &o).unwrap();
                    let mut file = craft_vectorcraft_format::File::new(Written(&b));
                    file.format = if version < 3 { LEGACY_EXTENSION } else { EXTENSION }.into();
                    file.version = version;
                    file.generator = Some(format!("VectorCraft {}", env!("CARGO_PKG_VERSION")));
                    file.preview = o.preview.as_deref().map(|p| Image::inline("image/png", p));
                    // The images come first in the data after the JSON, then the profiles and the PDF.
                    let mut tail = file.tail();
                    for key in ["big", "small"] {
                        file.images.insert(key.into(), tail.image("image/png", b.blobs[key].bytes.as_slice(), false));
                    }
                    for (name, icc) in &o.profiles {
                        file.profiles.insert(name.clone(), tail.image(ICC_MIME, icc, false));
                    }
                    file.pdf = o.pdf.as_deref().map(|p| tail.image("application/pdf", p, false));
                    let mut expected = if pretty { serde_json::to_vec_pretty(&file) } else { serde_json::to_vec(&file) }.unwrap();
                    tail.write_to(&mut expected).unwrap();
                    assert_eq!(tail.is_empty(), version < 4, "version {version}");
                    assert!(save_with(&d, &o).unwrap() == expected, "version {version}, pretty {pretty}, {} profiles", o.profiles.len());
                    checked += 1;
                    if version >= COMPRESSED_SINCE && !pretty {
                        let gz = save_with(&d, &SaveOptions { compress: true, ..o.clone() }).unwrap();
                        let mut unpacked = Vec::new();
                        flate2::read::GzDecoder::new(gz.as_slice()).read_to_end(&mut unpacked).unwrap();
                        assert!(unpacked == expected, "version {version}, compressed, {} profiles", o.profiles.len());
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 20);
    }

    /// Files are read through the crate's `Contents` with the messages VectorCraft gave before,
    /// and a top-level array holds the format, version, document, images and profiles in that
    /// order.
    #[test]
    fn files_read_through_the_crates_contents_as_before() {
        let message = |json: &str| load(json.as_bytes()).err().map(|e| e.to_string());
        assert_eq!(
            message(r#""x""#).as_deref(),
            Some(r#"not a VectorCraft file: invalid type: string "x", expected struct File at line 1 column 3"#)
        );
        assert!(
            message(r#"{"format":"vectorcraft","version":4}"#).is_some_and(|m| m.starts_with("not a VectorCraft file: missing field `document`"))
        );
        let v: serde_json::Value = serde_json::from_slice(&save(&Document::new(10.0, 10.0), false)).unwrap();
        let doc = v["document"].to_string();
        let file = |format: &str, version: u32| format!(r#"{{"format":"{format}","version":{version},"document":{doc}}}"#);
        assert_eq!(message(&file("svg", 4)).as_deref(), Some("not a VectorCraft file: format is `svg`"));
        assert_eq!(message(&file("vectorcraft", 9)).as_deref(), Some("file version 9 is newer than this VectorCraft supports (4)"));
        // A document that does not read is reported first, as before.
        assert!(
            message(r#"{"format":"vectorcraft","version":9,"document":7}"#).is_some_and(|m| m.starts_with("not a VectorCraft file: invalid type"))
        );
        for json in [format!(r#"["vectorcraft",3,{doc},{{}},{{}}]"#), format!(r#"["vectorcraft",3,{doc}]"#)] {
            assert_eq!(load(json.as_bytes()).map(|d| d.artboards.len()).ok(), Some(1), "{json}");
        }
        assert!(load(format!(r#"["vectorcraft",3,{{}},{doc},{{}}]"#).as_bytes()).is_err());
    }
    use vectorcraft_doc::{Appearance, ImageObject, Node, NodeKind};
    use vectorcraft_geom::{Affine, Rect, shapes};

    /// Color Halftone's Max Radius is in pixels of the raster effects resolution since document
    /// version 3 (#3260): a file from before (version 1, or version 2 from 0.21.0 on) opens with
    /// its radius in points converted, one saved since opens as it was. At 72 ppi a point is a
    /// pixel, so the number stays.
    #[test]
    fn halftone_radii_of_older_documents_open_in_pixels() {
        for ppi in [72.0, 300.0] {
            let mut d = Document::new(100.0, 100.0);
            d.raster_effects_ppi = ppi;
            let (l, id) = (d.layers[0].id, d.alloc_id());
            let mut look = Appearance::default_art();
            look.effects.push(vectorcraft_doc::Effect {
                id: "pixelate.colorHalftone".into(),
                params: serde_json::json!({"maxRadius": 8.0}),
                visible: true,
            });
            d.insert(Some(l), 0, Node::path(id, shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0)), look)).unwrap();
            let radius = |d: &Document| d.node(id).unwrap().appearance.effects[0].params["maxRadius"].as_f64().unwrap();
            assert_eq!(radius(&load(&save(&d, false)).unwrap()), 8.0, "{ppi} ppi: saved in pixels, opened in pixels");
            // Version 2 (0.21.0's Twist) still held the radius in points.
            d.version = 2;
            let v2 = load(&save(&d, false)).unwrap();
            assert!((radius(&v2) - 8.0 * ppi / 72.0).abs() < 1e-9, "version 2, 8 pt at {ppi} ppi: {}", radius(&v2));
            d.version = 1;
            let old = load(&save(&d, false)).unwrap();
            assert_eq!(old.version, vectorcraft_doc::FORMAT_VERSION);
            assert!((radius(&old) - 8.0 * ppi / 72.0).abs() < 1e-9, "8 pt at {ppi} ppi: {}", radius(&old));
            // Saved for an older app, in every older version: the radius in points, which opens
            // here in pixels again.
            let px = radius(&old);
            for version in MIN_VERSION..VERSION {
                let bytes = save_with(&old, &SaveOptions { version, ..SaveOptions::default() }).unwrap();
                let json = String::from_utf8(unpack::Unpacked::new(&bytes).unwrap().json().to_vec()).unwrap();
                let at = json.find("\"maxRadius\":").unwrap() + "\"maxRadius\":".len();
                let saved: f64 = json[at..].split([',', '}']).next().unwrap().trim().parse().unwrap();
                assert!((saved - 8.0).abs() < 1e-9, "{ppi} ppi, version {version} holds 8 pt: {saved}");
                let back = load(&bytes).unwrap();
                assert!((radius(&back) - px).abs() < 1e-9, "{ppi} ppi, version {version}: {}", radius(&back));
            }
            assert_eq!(radius(&old), px, "{ppi} ppi: saving leaves the open document as it is");
        }
    }

    #[test]
    fn roundtrip() {
        let mut d = Document::new(100.0, 100.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::path(id, shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art())).unwrap();
        d.images.insert("k".into(), ImageBlob::new("image/png", vec![1, 2, 3, 250]));
        // Only images in use are saved.
        let img = d.alloc_id();
        let im = ImageObject { key: "k".into(), width: 1, height: 1, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
        d.insert(Some(l), 1, Node::new(img, NodeKind::Image(Box::new(im)))).unwrap();
        let bytes = save(&d, true);
        assert!(sniff(&bytes));
        let back = load(&bytes).unwrap();
        assert_eq!(back.node_count(), d.node_count());
        assert_eq!(back.images["k"].bytes.as_slice(), &[1, 2, 3, 250]);
        assert_eq!(back.node(id).unwrap().path_data(), d.node(id).unwrap().path_data());
    }

    /// #3587: objects are read within a budget. A compressed file of many small objects stays
    /// small on disk while they take far more memory once read, so past the budget it is refused
    /// with a message naming it, before the rest of the document is read.
    #[test]
    fn a_documents_objects_are_capped() {
        assert_eq!(vectorcraft_doc::node::MAX_READ_NODES, if cfg!(target_arch = "wasm32") { 512 << 10 } else { 4 << 20 });
        let mut d = Document::new(100.0, 100.0);
        let l = d.add_layer(None);
        for i in 0..200 {
            let id = d.alloc_id();
            d.insert(Some(l), i, Node::path(id, shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art())).unwrap();
        }
        let n = d.node_count();
        let bytes = save_with(&d, &SaveOptions { compress: true, ..SaveOptions::default() }).unwrap();
        assert!(is_compressed(&bytes));
        assert_eq!(
            load_file_budgeted(&bytes, ReadCaps::default(), n, vectorcraft_doc::node::MAX_BUFFERED_VALUES).unwrap().doc.node_count(),
            n,
            "a document of exactly the budget opens"
        );
        let Err(FormatError::NotVectorcraft(why)) =
            load_file_budgeted(&bytes, ReadCaps::default(), n - 1, vectorcraft_doc::node::MAX_BUFFERED_VALUES)
        else {
            panic!("one object past the budget must be refused")
        };
        assert!(why.contains(&format!("more than {} objects", n - 1)), "{why}");
        assert_eq!(load(&bytes).unwrap().node_count(), n, "the real budget opens ordinary documents");
    }

    /// The saved ruler guides become objects when read, so each counts against the same budget.
    #[test]
    fn saved_ruler_guides_count_against_the_object_budget() {
        let mut d = Document::new(100.0, 100.0);
        let l = d.add_layer(None);
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::path(id, shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art())).unwrap();
        for i in 0..50 {
            d.add_ruler_guide(&vectorcraft_doc::Guide::new(i % 2 == 0, f64::from(i)));
        }
        let n = d.node_count() + d.ruler_guides().len();
        let bytes = save_with(&d, &SaveOptions { compress: true, ..SaveOptions::default() }).unwrap();
        let opened = load_file_budgeted(&bytes, ReadCaps::default(), n, vectorcraft_doc::node::MAX_BUFFERED_VALUES).unwrap().doc;
        assert_eq!(opened.ruler_guides().len(), 50, "a document of exactly the budget opens with its guides");
        let Err(FormatError::NotVectorcraft(why)) =
            load_file_budgeted(&bytes, ReadCaps::default(), n - 1, vectorcraft_doc::node::MAX_BUFFERED_VALUES)
        else {
            panic!("one guide past the budget must be refused")
        };
        assert!(why.contains(&format!("more than {} objects", n - 1)), "{why}");
    }

    /// Ruler guide nodes a file holds out of place are put right as it is read: one below a layer's
    /// art moves up to the layer's guides, and one in a group, beside the layers or in a symbol is
    /// removed. Saving again writes no ruler guide node, which builds from before ruler guides
    /// were nodes could not read.
    #[test]
    fn ruler_guide_nodes_out_of_place_in_a_file_are_put_right() {
        let mut d = Document::new(100.0, 100.0);
        let l = d.add_layer(None);
        let (a, b, group) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let path = |id| Node::path(id, shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
        d.insert(Some(l), 0, path(a)).unwrap();
        d.insert(Some(l), usize::MAX, Node::group(group, vec![Arc::new(path(b))])).unwrap();
        let li = d.layers.iter().position(|x| x.id == l).unwrap();
        let mut v: serde_json::Value = serde_json::from_slice(&save(&d, false)).unwrap();
        let guide = |id: u64| serde_json::json!({"id": id, "kind": {"type": "rulerguide", "vertical": true, "pos": 5.0}});
        let kids = &mut v["document"]["layers"][li]["kind"]["children"];
        kids.as_array_mut().unwrap().insert(0, guide(50));
        kids[2]["kind"]["children"].as_array_mut().unwrap().push(guide(51));
        v["document"]["layers"].as_array_mut().unwrap().push(guide(52));
        let opened = load(&serde_json::to_vec(&v).unwrap()).unwrap();
        let listed: Vec<u64> = opened.ruler_guides().iter().filter_map(|g| g.id).map(|id| id.0).collect();
        assert_eq!(listed, [50]);
        let layer = opened.layers.iter().find(|x| x.id == l).unwrap();
        assert_eq!(layer.children().unwrap().iter().map(|c| c.id.0).collect::<Vec<_>>(), [a.0, group.0, 50], "art first, then the guide");
        assert_eq!(opened.node(group).unwrap().children().unwrap().len(), 1, "the group keeps only its path");
        assert!(opened.layers.iter().all(|x| !x.is_ruler_guide()));
        assert!(!String::from_utf8(save(&opened, false)).unwrap().contains("rulerguide"));

        // A ruler guide in a symbol's art is removed too.
        let mut d = Document::new(100.0, 100.0);
        let sym = d.alloc_id();
        let art = Node::group(sym, vec![Arc::new(path(d.alloc_id()))]);
        d.symbols.push(vectorcraft_doc::Symbol { name: "S".into(), art: Arc::new(art), dynamic: false });
        let mut v: serde_json::Value = serde_json::from_slice(&save(&d, false)).unwrap();
        v["document"]["symbols"][0]["art"]["kind"]["children"].as_array_mut().unwrap().push(guide(60));
        let opened = load(&serde_json::to_vec(&v).unwrap()).unwrap();
        assert_eq!(opened.symbols[0].art.children().unwrap().len(), 1);
        assert!(!opened.symbols[0].art.children().unwrap().iter().any(|c| c.is_ruler_guide()));
    }

    /// #3587: a reordered file (its objects' fields before their `type`, as a JSON tool that
    /// sorts keys writes them) still opens; one whose fields before a `type` hold more values than
    /// the budget, such as a long array of numbers, is refused as they are read.
    #[test]
    fn values_buffered_before_an_objects_type_are_capped() {
        let mut d = Document::new(100.0, 100.0);
        let l = d.add_layer(None);
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::path(id, shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art())).unwrap();
        // Through `serde_json::Value`, whose objects sort their keys: `children` before `type`.
        let sorted = serde_json::to_vec(&serde_json::from_slice::<serde_json::Value>(&save(&d, false)).unwrap()).unwrap();
        assert_eq!(load(&sorted).unwrap().node_count(), d.node_count());
        let mut v: serde_json::Value = serde_json::from_slice(&save(&d, false)).unwrap();
        let zeros = vec![0; 1000];
        v["document"]["layers"][0]["kind"]["children"] = serde_json::json!([{"id": 90, "kind": {"children": zeros, "type": "group"}}]);
        let bomb = serde_json::to_vec(&v).unwrap();
        let Err(FormatError::NotVectorcraft(why)) = load_file_budgeted(&bomb, ReadCaps::default(), vectorcraft_doc::node::MAX_READ_NODES, 100) else {
            panic!("past the budget must be refused")
        };
        assert!(why.contains("more than 100 values before its type"), "{why}");
    }

    /// A version 1 document written before VectorCraft 0.21.0 (or by an app the file does not
    /// name) has its Twist angles negated when it opens; one written by 0.21.0 or later, and a
    /// later document, keep them. Each opens as the current version. A save for an older file
    /// version negates them back.
    #[test]
    fn twist_angles_follow_the_document_version_and_generator() {
        use vectorcraft_doc::Effect;
        let mut d = Document::new(100.0, 100.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let mut a = Appearance::default_art();
        a.effects.push(Effect { id: "distort.twist".into(), params: serde_json::json!({"angle": 30.0}), visible: true });
        d.insert(Some(l), 0, Node::path(id, shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0)), a)).unwrap();
        let angle = |d: &Document| d.node(id).and_then(|n| n.appearance.effects[0].params["angle"].as_f64());
        let now = vectorcraft_doc::FORMAT_VERSION;
        let current = save(&d, false);
        let back = load(&current).unwrap();
        assert_eq!((back.version, angle(&back)), (now, Some(30.0)), "a current document is read as saved");
        let mut v2: serde_json::Value = serde_json::from_slice(&current).unwrap();
        v2["document"]["version"] = 2.into();
        let back = load(&serde_json::to_vec(&v2).unwrap()).unwrap();
        assert_eq!((back.version, angle(&back)), (now, Some(30.0)), "a version 2 document keeps its angles");
        let opened = |generator: Option<&str>| {
            let mut v: serde_json::Value = serde_json::from_slice(&current).unwrap();
            v["document"]["version"] = 1.into();
            match generator {
                Some(g) => v["generator"] = g.into(),
                None => {
                    v.as_object_mut().unwrap().remove("generator");
                }
            }
            let d = load(&serde_json::to_vec(&v).unwrap()).unwrap();
            (d.version, angle(&d))
        };
        for g in [Some("VectorCraft 0.20.0"), Some("VectorCraft 0.9.12"), Some("DrawCraft 0.1.0"), Some("VectorCraft"), None] {
            assert_eq!(opened(g), (now, Some(-30.0)), "{g:?}");
        }
        for g in ["VectorCraft 0.21.0", "VectorCraft 0.21.3", "VectorCraft 1.0.0"] {
            assert_eq!(opened(Some(g)), (now, Some(30.0)), "{g}");
        }
        // A file may repeat `generator`: the first one names the app.
        let mut v: serde_json::Value = serde_json::from_slice(&current).unwrap();
        v["document"]["version"] = 1.into();
        v["generator"] = "VectorCraft 0.21.0".into();
        let text = serde_json::to_string(&v).unwrap();
        let first = r#""generator":"VectorCraft 0.21.0""#;
        let twice = text.replacen(first, &format!(r#"{first},"generator":"DrawCraft 0.1.0""#), 1);
        assert_ne!(twice, text);
        assert_eq!(angle(&load(twice.as_bytes()).unwrap()), Some(30.0));
        v["generator"] = "VectorCraft 0.20.0".into();
        let old = load(&serde_json::to_vec(&v).unwrap()).unwrap();
        let again = load(&save(&old, false)).unwrap();
        assert_eq!((again.version, angle(&again)), (now, Some(-30.0)), "saved again, it is the current version");
        // Saved for an older file version (apps before 0.21.0): version 1 with the angle negated
        // back, whatever the generator, and it reopens as it was.
        for version in MIN_VERSION..VERSION {
            let bytes = save_with(&d, &SaveOptions { version, ..SaveOptions::default() }).unwrap();
            let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(v["document"]["version"], 1, "file version {version}");
            assert!(String::from_utf8_lossy(&bytes).contains(r#""angle":-30.0"#), "file version {version}");
            let back = load(&bytes).unwrap();
            assert_eq!((back.version, angle(&back)), (now, Some(30.0)), "file version {version}");
        }
    }

    #[test]
    fn rejects_foreign_and_future() {
        assert!(load(b"{}").is_err());
        let d = Document::new(1.0, 1.0);
        let mut v: serde_json::Value = serde_json::from_slice(&save(&d, false)).unwrap();
        v["version"] = 99.into();
        assert!(matches!(load(&serde_json::to_vec(&v).unwrap()), Err(FormatError::TooNew(99))));
    }

    #[test]
    fn base64() {
        for s in [&b""[..], b"f", b"fo", b"foo", b"foob", b"fooba", b"foobar"] {
            assert_eq!(base64_decode(&base64_encode(s)).unwrap(), s);
        }
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_rejects_misplaced_padding() {
        for s in ["====", "A===", "AA=A", "=AAA", "QQ==QUJD", "QUI=QUJD"] {
            assert_eq!(base64_decode(s), None, "{s}");
        }
        assert_eq!(base64_decode("QQ=="), Some(vec![0x41]));
        assert_eq!(base64_decode("QUI="), Some(vec![0x41, 0x42]));
        assert_eq!(base64_decode("QUJD"), Some(vec![0x41, 0x42, 0x43]));
    }

    /// A layer clipping by a path beyond the canvas is emptied on open and stops clipping, also
    /// when the file puts a ruler guide before the clipping path: no other object becomes it.
    #[test]
    fn a_guide_before_a_clipping_path_beyond_the_canvas_does_not_change_the_clipping_path() {
        let opened = |guide_first: bool| {
            let mut d = Document::new(100.0, 100.0);
            let rect = |d: &mut Document, y: f64| {
                Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, y, 10.0, y + 10.0)), Appearance::default_art()))
            };
            let (far, near) = (rect(&mut d, 1e17), rect(&mut d, 0.0));
            if let NodeKind::Layer { children, clip, .. } = &mut Arc::make_mut(&mut d.layers[0]).kind {
                *children = vec![far, near];
                *clip = true;
            }
            // Saving writes ruler guides to the saved list: the guide node is put in the file.
            let mut v: serde_json::Value = serde_json::from_slice(&save(&d, false)).unwrap();
            let guide = serde_json::json!({"id": 50, "kind": {"type": "rulerguide", "vertical": true, "pos": 5.0}});
            let kids = v["document"]["layers"][0]["kind"]["children"].as_array_mut().unwrap();
            if guide_first {
                kids.insert(0, guide)
            } else {
                kids.push(guide)
            }
            let d = load(&serde_json::to_vec(&v).unwrap()).unwrap();
            let NodeKind::Layer { children, clip, .. } = &d.layers[0].kind else { panic!("a layer") };
            (*clip, children.iter().map(|c| c.is_ruler_guide()).collect::<Vec<_>>())
        };
        assert_eq!(opened(false), (false, vec![true]), "the clipping path and what it clipped go; the guide stays");
        assert_eq!(opened(true), opened(false));
    }
}
