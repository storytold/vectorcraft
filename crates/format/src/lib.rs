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
#![forbid(unsafe_code)]

#[cfg(not(target_arch = "wasm32"))]
mod atomic;
mod legacy;
mod stream;
mod unpack;

use std::collections::{BTreeMap, HashSet};
use std::io::Write;
use std::sync::Arc;

use craft_vectorcraft_format::{Contents, Head, Image, ReadError, Tail};
use serde::Serialize;
use vectorcraft_doc::{Document, ImageBlob};

#[cfg(not(target_arch = "wasm32"))]
pub use atomic::{sync_file, write_atomic, write_atomic_with, write_new_with};
pub use craft_vectorcraft_format::base64::{decode as base64_decode, encode as base64_encode};
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
}

impl Default for SaveOptions {
    fn default() -> Self {
        Self { pretty: false, compress: false, version: VERSION, preview: None, include_linked: false, profiles: BTreeMap::new(), pdf: None }
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
/// documents as their art for older versions, and its images taken out.
fn body(doc: &Document, o: &SaveOptions) -> Result<Body, FormatError> {
    let version = o.version;
    let mut d = doc.without_edit_modes().into_owned();
    // Older apps don't know placed documents: they get the art the objects show, with its
    // resources. Otherwise resources only exporting adds are never saved.
    if version < VERSION && d.has_placed() {
        d = d.with_placed_art().into_owned();
        d.placed_as_groups();
    } else {
        d.drop_placed_resources();
    }
    let linked = d.linked_only_images();
    // The blobs go in the file's `images` (only those the document uses).
    let blobs = std::mem::take(&mut d.images);
    if version < 3 {
        d.store_legacy_color_profiles();
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
    load_file_within(bytes, vectorcraft_doc::node::MAX_READ_NODES, vectorcraft_doc::node::MAX_BUFFERED_VALUES)
}

/// [`load_file`] reading at most `max_nodes` objects and buffering at most `max_values` values
/// before objects' types, so a test can exercise the budgets without documents that large
/// ([`vectorcraft_doc::node::MAX_READ_NODES`], [`vectorcraft_doc::node::MAX_BUFFERED_VALUES`]).
fn load_file_within(bytes: &[u8], max_nodes: usize, max_values: usize) -> Result<NativeFile, FormatError> {
    let file = unpack::Unpacked::new(bytes)?;
    // The JSON reader stops at 128 levels (a document's objects use three each), the paths read
    // at `MAX_READ_ANCHORS` anchors together, the document at `MAX_READ_NODES` objects, and the
    // fields read before an object's type at `MAX_BUFFERED_VALUES` values. The preview and the
    // PDF are skipped without being kept.
    let read = || {
        vectorcraft_doc::node::with_node_budget(max_nodes, || {
            vectorcraft_doc::node::with_buffered_value_budget(max_values, || Contents::<Document>::read(file.json()))
        })
    };
    let f = vectorcraft_doc::geom::path::with_anchor_budget(vectorcraft_doc::geom::path::MAX_READ_ANCHORS, read).map_err(|e| match e {
        ReadError::Json(e) => {
            let why = if e.to_string().contains("recursion limit") { format!("its objects are nested too deep ({e})") } else { e.to_string() };
            FormatError::NotVectorcraft(why)
        }
        ReadError::Envelope(e) => e.into(),
        e => FormatError::NotVectorcraft(e.to_string()),
    })?;
    let info = FileInfo { version: f.version, legacy: f.format == LEGACY_EXTENSION };
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
    // Saved mid-edit by an older version: drop the opacity-mask editing layer.
    doc.drop_edit_modes();
    // Saved before per-fill/stroke overprint: the Overprint Black list becomes overprint flags.
    doc.migrate_overprint_black();
    // Saved before v3: the assigned profiles move out of `unknown`.
    doc.migrate_color_profiles();
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

/// The bytes of the member of the file's head `pick` picks (only that entry of the data after
/// the JSON is kept).
fn head_member(bytes: &[u8], pick: impl FnOnce(Head) -> Option<Image>) -> Option<Vec<u8>> {
    let file = unpack::Unpacked::new(bytes).ok()?;
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
    fn file_info_tells_old_files() {
        let d = Document::new(10.0, 10.0);
        let current = save(&d, false);
        assert_eq!(load_info(&current).unwrap().1, FileInfo { version: VERSION, legacy: false });
        assert!(!load_info(&current).unwrap().1.is_old());
        let text = String::from_utf8(current).unwrap();
        let legacy = text.replacen("\"format\":\"vectorcraft\"", "\"format\":\"drawcraft\"", 1);
        assert!(load_info(legacy.as_bytes()).unwrap().1.is_old());
        let v1 = text.replacen(&format!("\"version\":{VERSION}"), "\"version\":1", 1);
        assert_eq!(load_info(v1.as_bytes()).unwrap().1, FileInfo { version: 1, legacy: false });
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
            load_file_within(&bytes, n, vectorcraft_doc::node::MAX_BUFFERED_VALUES).unwrap().doc.node_count(),
            n,
            "a document of exactly the budget opens"
        );
        let Err(FormatError::NotVectorcraft(why)) = load_file_within(&bytes, n - 1, vectorcraft_doc::node::MAX_BUFFERED_VALUES) else {
            panic!("one object past the budget must be refused")
        };
        assert!(why.contains(&format!("more than {} objects", n - 1)), "{why}");
        assert_eq!(load(&bytes).unwrap().node_count(), n, "the real budget opens ordinary documents");
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
        let Err(FormatError::NotVectorcraft(why)) = load_file_within(&bomb, vectorcraft_doc::node::MAX_READ_NODES, 100) else {
            panic!("past the budget must be refused")
        };
        assert!(why.contains("more than 100 values before its type"), "{why}");
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
}
