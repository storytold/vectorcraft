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

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::io::{Read as _, Write};

use serde::{Deserialize, Serialize};
use vectorcraft_doc::{Document, ImageBlob};

#[cfg(not(target_arch = "wasm32"))]
pub use atomic::{sync_file, write_atomic, write_atomic_with, write_new_with};
use stream::{Gz, Indent, KeyScan, Out};

/// v4: large data after the JSON (v1 to v3 files still load).
pub const VERSION: u32 = 4;
/// Data this large or larger goes after the JSON (v4), not into it as base64.
pub const INLINE_MAX: usize = 8 << 10;
/// What separates the JSON from the data after it (v4).
pub const BLOB_MAGIC: &[u8] = b"\n\0VCBLOBS\0";
/// The oldest version [`save_with`] writes.
pub const MIN_VERSION: u32 = 1;
/// The first version whose readers open compressed files.
pub const COMPRESSED_SINCE: u32 = 3;
/// Most pixels on the longer side of an embedded preview.
pub const PREVIEW_MAX: u32 = 256;
pub const EXTENSION: &str = "vectorcraft";
/// Extension and format name from before the project was renamed (DrawCraft): still opened.
pub const LEGACY_EXTENSION: &str = "drawcraft";
/// Most bytes the JSON of a compressed file may unpack to (a guard against decompression bombs,
/// as are [`MAX_BLOB`] and [`MAX_BLOBS`]). A compressed save refuses a document past any of them,
/// so every file VectorCraft writes reads back.
pub const MAX_JSON: u64 = 1 << 30;
/// Most bytes one entry of the data after the JSON (an image, a placed document, the PDF, a
/// profile) may have in a compressed file.
pub const MAX_BLOB: u64 = 1 << 30;
/// Most bytes the entries after the JSON may have together in a compressed file.
pub const MAX_BLOBS: u64 = 4 << 30;
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

#[derive(Serialize, Deserialize)]
struct Image {
    mime: String,
    /// The bytes as base64, unless they come after the JSON (`blob`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    data: String,
    /// Where the bytes are in the data after the JSON (v4): `[offset, length]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob: Option<[u64; 2]>,
    /// `data` is a linked image's preview ([`ImageBlob::proxy`]), not the file's bytes.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    proxy: bool,
}

impl Image {
    /// Its bytes when they are in the JSON (base64 `data`); `None` when they come after it
    /// ([`Image::blob`]) or don't decode.
    fn inline(&self) -> Option<Vec<u8>> {
        match self.blob {
            Some(_) => None,
            None => base64_decode(&self.data),
        }
    }
}

/// Entries written for one file: large bytes go to the data after the JSON when the version keeps
/// them there (in the order they are added, each at the offset it is given).
struct Blobs<'a> {
    /// The bytes, and whether they are compressed already ([`is_packed`]).
    tail: Vec<(&'a [u8], bool)>,
    at: u64,
    outside: bool,
}

impl<'a> Blobs<'a> {
    fn image(&mut self, mime: &str, bytes: &'a [u8], proxy: bool) -> Image {
        if self.outside && bytes.len() >= INLINE_MAX {
            let len = bytes.len() as u64;
            let at = self.at;
            self.at += len;
            self.tail.push((bytes, is_packed(mime)));
            return Image { mime: mime.into(), data: String::new(), blob: Some([at, len]), proxy };
        }
        Image { mime: mime.into(), data: base64_encode(bytes), blob: None, proxy }
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

/// The JSON of a file and the data after it (v4), split at [`BLOB_MAGIC`] after the first JSON
/// value.
fn split_tail(text: &[u8]) -> (&[u8], &[u8]) {
    // JSON holds no raw NUL, so the marker (which has one) starts at the first NUL, if the file
    // has one. The writer puts it right after the JSON's last byte; found there, it is where the
    // first JSON value ends, without parsing the JSON twice. Anything else takes the parse.
    let Some(nul) = text.iter().position(|&b| b == 0) else {
        // No NUL, so no marker: the parse below would find none either.
        return (text, &[]);
    };
    if let Some(at) = nul.checked_sub(1)
        && text.get(at..).is_some_and(|r| r.starts_with(BLOB_MAGIC))
        && at.checked_sub(1).and_then(|k| text.get(k)).is_some_and(|b| matches!(b, b'}' | b']' | b'"'))
    {
        return (text.get(..at).unwrap_or(text), text.get(at + BLOB_MAGIC.len()..).unwrap_or_default());
    }
    let mut values = serde_json::Deserializer::from_slice(text).into_iter::<serde::de::IgnoredAny>();
    if values.next().is_some_and(|v| v.is_ok()) {
        let end = values.byte_offset();
        if let Some(rest) = text.get(end..)
            && rest.starts_with(BLOB_MAGIC)
        {
            return (text.get(..end).unwrap_or(text), rest.get(BLOB_MAGIC.len()..).unwrap_or_default());
        }
    }
    (text, &[])
}

#[derive(Deserialize)]
struct File {
    format: String,
    version: u32,
    document: Document,
    #[serde(default)]
    images: BTreeMap<String, Image>,
    #[serde(default)]
    profiles: BTreeMap<String, Image>,
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

/// The MIME type of embedded ICC profiles.
pub const ICC_MIME: &str = "application/vnd.iccprofile";

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

    /// The defaults for saving `doc` to disk: pretty when small (see [`PRETTY_MAX_OBJECTS`] and
    /// [`PRETTY_MAX_BYTES`]).
    pub fn for_doc(doc: &Document) -> Self {
        let objects: usize = doc.layers.iter().map(|l| l.count()).sum();
        Self { pretty: objects <= PRETTY_MAX_OBJECTS && compact_size_at_most(doc, PRETTY_MAX_BYTES), ..Self::default() }
    }
}

/// Is `doc` at most `limit` bytes as compact JSON? Serializing stops as soon as it is past the
/// limit, so a large document costs no more than `limit` bytes of work.
fn compact_size_at_most(doc: &Document, limit: usize) -> bool {
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
    serde_json::to_writer(Capped { left: limit }, doc).is_ok()
}

/// Serialize a document (pretty = human-diffable) in the current version. Editing-mode working
/// copies are left out ([`Document::without_edit_modes`]).
pub fn save(doc: &Document, pretty: bool) -> Vec<u8> {
    save_with(doc, &SaveOptions { pretty, ..SaveOptions::default() }).unwrap_or_default()
}

/// Documents up to this many objects are saved pretty-printed (readable, diffable); larger ones
/// compact, where indentation would multiply the file size and slow opening down.
pub const PRETTY_MAX_OBJECTS: usize = 2000;

/// Documents whose compact JSON is larger than this are saved compact even with few objects:
/// pretty-printing spreads every path anchor over several lines, so traced or generated art (in
/// layers, symbols or patterns alike) would grow about tenfold.
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
    let mut d = doc.without_edit_modes().into_owned();
    // Older apps don't know placed documents: they get the art the objects show, with its
    // resources. Otherwise resources only exporting adds are never saved.
    if o.version < VERSION && d.has_placed() {
        d = d.with_placed_art().into_owned();
        d.placed_as_groups();
    } else {
        d.drop_placed_resources();
    }
    let linked = d.linked_only_images();
    // The blobs go in the file's `images` (only those the document uses).
    let blobs = std::mem::take(&mut d.images);
    if o.version < 3 {
        d.store_legacy_color_profiles();
    }
    let v1 = if o.version == 1 {
        let mut v = serde_json::to_value(&d).map_err(encode_err)?;
        legacy::anchors_to_v1(&mut v);
        Some(v)
    } else {
        None
    };
    let entries = Entries { blobs: &blobs, linked: &linked, include_linked: o.include_linked };
    let mut out_blobs = Blobs { tail: vec![], at: 0, outside: o.version >= 4 };
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
    let blob = |mime: &str, bytes: &[u8]| Image { mime: mime.into(), data: base64_encode(bytes), blob: None, proxy: false };
    let mut out = if o.compress { Out::Gz(Box::new(Gz::new(w).map_err(encode_err)?)) } else { Out::Plain(w) };
    let mut env = Envelope { pretty, first: true };
    // Older apps only know the name from before the rename.
    env.field(&mut out, "format", &if o.version < 3 { LEGACY_EXTENSION } else { EXTENSION })?;
    env.field(&mut out, "version", &o.version)?;
    env.field(&mut out, "generator", &format!("VectorCraft {}", env!("CARGO_PKG_VERSION")))?;
    if let Some(png) = &o.preview {
        env.field(&mut out, "preview", &blob("image/png", png))?;
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
        let tail: Vec<u64> = out_blobs.tail.iter().map(|(b, _)| b.len() as u64).collect();
        if json > MAX_JSON {
            return Err(FormatError::TooBigToCompress(format!("{} MB of document, at most {} MB", json >> 20, MAX_JSON >> 20)));
        }
        if let Some(big) = tail.iter().find(|&&n| n > MAX_BLOB) {
            return Err(FormatError::TooBigToCompress(format!("an image of {} MB, at most {} MB", big >> 20, MAX_BLOB >> 20)));
        }
        if out_blobs.at > MAX_BLOBS {
            return Err(FormatError::TooBigToCompress(format!("{} MB of images, at most {} MB", out_blobs.at >> 20, MAX_BLOBS >> 20)));
        }
    }
    if !out_blobs.tail.is_empty() {
        out.write_all(BLOB_MAGIC).map_err(encode_err)?;
        for &(bytes, packed) in &out_blobs.tail {
            out.packed(packed).map_err(encode_err)?;
            out.write_all(bytes).map_err(encode_err)?;
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
    fn images(&self, used: &HashSet<String>, out: &mut Blobs<'a>) -> BTreeMap<&'a str, Image> {
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

/// Is this gzip data (a compressed file)?
pub fn is_compressed(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1f, 0x8b])
}

pub fn load(bytes: &[u8]) -> Result<Document, FormatError> {
    load_info(bytes).map(|(doc, _)| doc)
}

/// A native file read: its document, what it says about itself and the colour profiles it carries.
#[derive(Debug)]
pub struct NativeFile {
    pub doc: Document,
    /// What was done to bring the document within the nesting caps
    /// ([`Document::limit_depth_noting`]: a file saved before them, or a damaged one).
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
    let file = unpack::Unpacked::new(bytes)?;
    // The JSON reader stops at 128 levels (a document's objects use three each).
    let f: File = serde_json::from_slice(file.json()).map_err(|e| {
        let why = if e.to_string().contains("recursion limit") { format!("its objects are nested too deep ({e})") } else { e.to_string() };
        FormatError::NotVectorcraft(why)
    })?;
    if f.format != EXTENSION && f.format != LEGACY_EXTENSION {
        return Err(FormatError::NotVectorcraft(format!("format is `{}`", f.format)));
    }
    if f.version > VERSION {
        return Err(FormatError::TooNew(f.version));
    }
    // The bytes after the JSON, read in one pass: the images' first, then the profiles'.
    let entries: Vec<[u64; 2]> = f.images.values().chain(f.profiles.values()).filter_map(|i| i.blob).collect();
    let mut tail = file.entries(&entries)?.into_iter();
    let mut bytes_of = |img: &Image| match img.blob {
        Some(_) => tail.next().flatten(),
        None => img.inline(),
    };
    let mut doc = f.document;
    for (k, img) in f.images {
        let bytes = bytes_of(&img).ok_or_else(|| FormatError::BadImage(k.clone()))?;
        let mut blob = ImageBlob::new(img.mime, bytes);
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
    doc.tidy_graph_designs();
    doc.raster_effects.add_around = doc.raster_effects.add_around.clamp(0.0, vectorcraft_doc::rastersettings::MAX_ADD_AROUND);
    let mut depth_notes = vec![];
    doc.limit_depth_noting(&mut depth_notes);
    // Untrusted: the type's sizes, leading, baseline shifts and scales, its indents and paragraph
    // spacing, and its tab stops, each within its range (after the depth limit: the walk recurses).
    doc.keep_type_values_in_range();
    // Untrusted: the raster effects resolution, within 1 to 2400 ppi (72 when it isn't finite).
    doc.raster_effects_ppi = Some(doc.raster_effects_ppi).filter(|v| v.is_finite()).unwrap_or(72.0).clamp(1.0, 2400.0);
    let profiles = f.profiles.into_iter().filter_map(|(name, icc)| Some((name, bytes_of(&icc)?))).collect();
    Ok(NativeFile { doc, depth_notes, info: FileInfo { version: f.version, legacy: f.format == LEGACY_EXTENSION }, profiles })
}

/// The members of a native file besides its document (which is skipped, not decoded).
#[derive(Deserialize)]
struct Head {
    preview: Option<Image>,
    pdf: Option<Image>,
}

/// The bytes of the member of the file's head `pick` picks (only that entry of the data after
/// the JSON is kept).
fn head_member(bytes: &[u8], pick: impl FnOnce(Head) -> Option<Image>) -> Option<Vec<u8>> {
    let file = unpack::Unpacked::new(bytes).ok()?;
    let head: Head = serde_json::from_slice(file.json()).ok()?;
    let img = pick(head)?;
    match img.blob {
        Some(at) => file.entries(&[at]).ok()?.pop().flatten(),
        None => img.inline(),
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

/// Does this look like a `.vectorcraft` file (compressed or not)?
pub fn sniff(bytes: &[u8]) -> bool {
    const HEAD: usize = 256;
    let head: Cow<'_, [u8]> = if is_compressed(bytes) {
        let mut head = Vec::with_capacity(HEAD);
        // A damaged stream may still unpack its start: judge what came out.
        let _ = flate2::read::GzDecoder::new(bytes).take(HEAD as u64).read_to_end(&mut head);
        Cow::Owned(head)
    } else {
        Cow::Borrowed(bytes.get(..HEAD).unwrap_or(bytes))
    };
    // The cut may split a character.
    let s = String::from_utf8_lossy(&head);
    s.trim_start().starts_with('{') && (s.contains("\"vectorcraft\"") || s.contains("\"drawcraft\""))
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(B64[(n >> 18) as usize & 63] as char);
        out.push(B64[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { B64[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { B64[n as usize & 63] as char } else { '=' });
    }
    out
}

pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| B64.iter().position(|&b| b == c).map(|p| p as u32);
    let clean: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !clean.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for c in clean.chunks(4) {
        let mut n = 0u32;
        let mut pad = 0;
        for &b in c {
            n <<= 6;
            if b == b'=' {
                pad += 1;
            } else {
                n |= val(b)?;
            }
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
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
        let im = ImageObject { key: "k".into(), width: 1, height: 1, xf: Affine::IDENTITY, link: None, placement: Default::default() };
        d.insert(Some(l), 1, Node::new(img, NodeKind::Image(Box::new(im)))).unwrap();
        let bytes = save(&d, true);
        assert!(sniff(&bytes));
        let back = load(&bytes).unwrap();
        assert_eq!(back.node_count(), d.node_count());
        assert_eq!(back.images["k"].bytes.as_slice(), &[1, 2, 3, 250]);
        assert_eq!(back.node(id).unwrap().path_data(), d.node(id).unwrap().path_data());
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
}
