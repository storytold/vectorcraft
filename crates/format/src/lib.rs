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

use std::borrow::Cow;
use std::collections::{BTreeMap, HashSet};
use std::io::{Read as _, Write as _};

use serde::{Deserialize, Serialize};
use vectorcraft_doc::{Document, ImageBlob, Node, NodeKind};

#[cfg(not(target_arch = "wasm32"))]
pub use atomic::{write_atomic, write_atomic_with};

/// v5: compound shapes (v4: large data after the JSON; v1 to v4 files still load).
pub const VERSION: u32 = 5;
/// The first version whose readers know compound shapes.
pub const COMPOUND_SHAPES_SINCE: u32 = 5;
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
/// Most bytes a compressed file may unpack to (a guard against decompression bombs).
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
    #[error("the compressed file unpacks to more than {} MB", MAX_UNPACKED >> 20)]
    TooBig,
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
    /// Its bytes: the base64 `data`, or its part of `tail` (the data after the JSON).
    fn bytes(&self, tail: &[u8]) -> Option<Vec<u8>> {
        match self.blob {
            Some([offset, len]) => {
                let start = usize::try_from(offset).ok()?;
                let end = start.checked_add(usize::try_from(len).ok()?)?;
                tail.get(start..end).map(<[u8]>::to_vec)
            }
            None => base64_decode(&self.data),
        }
    }
}

/// Entries written for one file: large bytes go to the data after the JSON when the version keeps
/// them there.
struct Blobs {
    tail: Vec<u8>,
    outside: bool,
}

impl Blobs {
    fn image(&mut self, mime: &str, bytes: &[u8], proxy: bool) -> Image {
        if self.outside && bytes.len() >= INLINE_MAX {
            let at = self.tail.len() as u64;
            self.tail.extend_from_slice(bytes);
            return Image { mime: mime.into(), data: String::new(), blob: Some([at, bytes.len() as u64]), proxy };
        }
        Image { mime: mime.into(), data: base64_encode(bytes), blob: None, proxy }
    }
}

/// The MIME type of a preview's bytes (JPEG or PNG).
fn preview_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0xff, 0xd8]) { "image/jpeg" } else { vectorcraft_doc::links::PROXY_MIME }
}

/// The JSON of a file and the data after it (v4), split at [`BLOB_MAGIC`] after the first JSON
/// value.
fn split_tail(text: &[u8]) -> (&[u8], &[u8]) {
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

    /// The defaults for saving `doc` to disk: pretty when small (see [`PRETTY_MAX_OBJECTS`]).
    pub fn for_doc(doc: &Document) -> Self {
        let objects: usize = doc.layers.iter().map(|l| l.count()).sum();
        Self { pretty: objects <= PRETTY_MAX_OBJECTS, ..Self::default() }
    }
}

/// Serialize a document (pretty = human-diffable) in the current version. Editing-mode working
/// copies are left out ([`Document::without_edit_modes`]).
pub fn save(doc: &Document, pretty: bool) -> Vec<u8> {
    save_with(doc, &SaveOptions { pretty, ..SaveOptions::default() }).unwrap_or_default()
}

/// Documents up to this many objects are saved pretty-printed (readable, diffable); larger ones
/// compact, where indentation would multiply the file size and slow opening down.
pub const PRETTY_MAX_OBJECTS: usize = 2000;

/// Serialize for saving to disk: pretty when small, compact when large.
pub fn save_file(doc: &Document) -> Vec<u8> {
    save_with(doc, &SaveOptions::for_doc(doc)).unwrap_or_default()
}

/// Serialize a document as `o` says: the version, compression, layout and preview. Editing-mode
/// working copies and images nothing uses are left out.
pub fn save_with(doc: &Document, o: &SaveOptions) -> Result<Vec<u8>, FormatError> {
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
    // Older apps don't know compound shapes: callers that can evaluate them pass their outlines
    // ([`Document::compound_shapes_as`]); any left become groups of their members.
    if o.version < COMPOUND_SHAPES_SINCE && d.has_compound_shapes() {
        d.compound_shapes_as(&mut |n| {
            let children = n.children().cloned().unwrap_or_default();
            let mut g = Node::new(n.id, NodeKind::Group { children, clip: false });
            (g.name, g.visible, g.locked, g.opacity, g.blend, g.mask) = (n.name.clone(), n.visible, n.locked, n.opacity, n.blend, n.mask.clone());
            g
        });
    }
    let linked = d.linked_only_images();
    // The blobs go in the file's `images` (only those the document uses).
    let blobs = std::mem::take(&mut d.images);
    if o.version < 3 {
        d.store_legacy_color_profiles();
    }
    let body = if o.version == 1 {
        let mut v = serde_json::to_value(&d).map_err(encode_err)?;
        legacy::anchors_to_v1(&mut v);
        to_json(&v, pretty)?
    } else {
        to_json(&d, pretty)?
    };
    // Images are only referenced by image objects' `key` (wherever the objects are: layers,
    // symbols, patterns, masks, or foreign data).
    let used = if blobs.is_empty() { HashSet::new() } else { string_members(&body, "key") };
    let mut out_blobs = Blobs { tail: vec![], outside: o.version >= 4 };
    let images: BTreeMap<&str, Image> = blobs
        .iter()
        .filter(|(k, _)| used.contains(k.as_str()))
        .map(|(k, b)| {
            // Include Linked Files keeps the file's bytes whenever they are loaded.
            let preview_only = linked.contains(k) && (!o.include_linked || b.is_proxy());
            let image = match b.proxy.as_ref().filter(|_| preview_only) {
                Some(p) => out_blobs.image(preview_mime(p), p, true),
                None => out_blobs.image(&b.mime, &b.bytes, false),
            };
            (k.as_str(), image)
        })
        .collect();
    let profiles: BTreeMap<&str, Image> = o.profiles.iter().map(|(name, icc)| (name.as_str(), out_blobs.image(ICC_MIME, icc, false))).collect();
    let pdf = o.pdf.as_deref().map(|p| out_blobs.image("application/pdf", p, false));
    let blob = |mime: &str, bytes: &[u8]| Image { mime: mime.into(), data: base64_encode(bytes), blob: None, proxy: false };
    let carried = profiles.values().chain(&pdf).map(|i| i.data.len()).sum::<usize>();
    let size = body.len() + images.values().map(|i| i.data.len()).sum::<usize>() + carried + 256;
    let mut w = Envelope { out: Vec::with_capacity(size), pretty };
    // Older apps only know the name from before the rename.
    w.field("format", &if o.version < 3 { LEGACY_EXTENSION } else { EXTENSION })?;
    w.field("version", &o.version)?;
    w.field("generator", &format!("VectorCraft {}", env!("CARGO_PKG_VERSION")))?;
    if let Some(png) = &o.preview {
        w.field("preview", &blob("image/png", png))?;
    }
    if !profiles.is_empty() {
        w.field("profiles", &profiles)?;
    }
    if let Some(pdf) = &pdf {
        w.field("pdf", pdf)?;
    }
    w.raw("document", &body);
    w.field("images", &images)?;
    let mut out = w.finish();
    if !out_blobs.tail.is_empty() {
        out.extend_from_slice(BLOB_MAGIC);
        out.extend_from_slice(&out_blobs.tail);
    }
    if !o.compress {
        return Ok(out);
    }
    let mut gz = flate2::write::GzEncoder::new(Vec::with_capacity(out.len() / 4), flate2::Compression::default());
    gz.write_all(&out).map_err(encode_err)?;
    gz.finish().map_err(encode_err)
}

fn encode_err(e: impl std::fmt::Display) -> FormatError {
    FormatError::Encode(e.to_string())
}

fn to_json(v: &impl Serialize, pretty: bool) -> Result<Vec<u8>, FormatError> {
    if pretty { serde_json::to_vec_pretty(v) } else { serde_json::to_vec(v) }.map_err(encode_err)
}

/// The file's top-level object, written member by member (the document goes in pre-serialized).
struct Envelope {
    out: Vec<u8>,
    pretty: bool,
}

impl Envelope {
    fn field(&mut self, key: &str, value: &impl Serialize) -> Result<(), FormatError> {
        let json = to_json(value, self.pretty)?;
        self.raw(key, &json);
        Ok(())
    }

    /// Member `key` (a plain name) with the JSON text `json`, indented one level when pretty (as
    /// serde_json would).
    fn raw(&mut self, key: &str, json: &[u8]) {
        let out = &mut self.out;
        out.push(if out.is_empty() { b'{' } else { b',' });
        if self.pretty {
            out.extend_from_slice(b"\n  ");
        }
        out.push(b'"');
        out.extend_from_slice(key.as_bytes());
        out.extend_from_slice(if self.pretty { b"\": " } else { b"\":" });
        // Newlines only occur between tokens (inside strings they are escaped).
        let mut lines = json.split(|&b| b == b'\n');
        out.extend_from_slice(lines.next().unwrap_or_default());
        for line in lines {
            out.extend_from_slice(b"\n  ");
            out.extend_from_slice(line);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        self.out.extend_from_slice(if self.pretty { b"\n}" } else { b"}" });
        self.out
    }
}

/// The string values of every `"<name>": "…"` member at any depth of JSON text. Quotes inside JSON
/// strings are escaped, so every match is a real member.
fn string_members(json: &[u8], name: &str) -> HashSet<String> {
    let tag = format!("\"{name}\":");
    let tag = tag.as_bytes();
    let mut out = HashSet::new();
    let mut rest = json;
    while let Some(at) = rest.windows(tag.len()).position(|w| w == tag) {
        rest = rest.get(at + tag.len()..).unwrap_or_default();
        let value = rest.trim_ascii_start();
        if value.first() == Some(&b'"')
            && let Some(Ok(s)) = serde_json::Deserializer::from_slice(value).into_iter::<String>().next()
        {
            out.insert(s);
        }
    }
    out
}

/// Is this gzip data (a compressed file)?
pub fn is_compressed(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1f, 0x8b])
}

/// The JSON text of a file, unpacked when compressed.
fn unpack(bytes: &[u8]) -> Result<Cow<'_, [u8]>, FormatError> {
    if !is_compressed(bytes) {
        return Ok(Cow::Borrowed(bytes));
    }
    let mut raw = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .take(MAX_UNPACKED + 1)
        .read_to_end(&mut raw)
        .map_err(|e| FormatError::NotVectorcraft(format!("damaged compressed data: {e}")))?;
    if raw.len() as u64 > MAX_UNPACKED {
        return Err(FormatError::TooBig);
    }
    Ok(Cow::Owned(raw))
}

pub fn load(bytes: &[u8]) -> Result<Document, FormatError> {
    load_info(bytes).map(|(doc, _)| doc)
}

/// A native file read: its document, what it says about itself and the colour profiles it carries.
#[derive(Debug)]
pub struct NativeFile {
    pub doc: Document,
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
    let text = unpack(bytes)?;
    let (json, tail) = split_tail(&text);
    let f: File = serde_json::from_slice(json).map_err(|e| FormatError::NotVectorcraft(e.to_string()))?;
    if f.format != EXTENSION && f.format != LEGACY_EXTENSION {
        return Err(FormatError::NotVectorcraft(format!("format is `{}`", f.format)));
    }
    if f.version > VERSION {
        return Err(FormatError::TooNew(f.version));
    }
    let mut doc = f.document;
    for (k, img) in f.images {
        let bytes = img.bytes(tail).ok_or_else(|| FormatError::BadImage(k.clone()))?;
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
    // Untrusted: capped, named and pointing at objects the file has.
    doc.tidy_saved_selections();
    let profiles = f.profiles.into_iter().filter_map(|(name, icc)| Some((name, icc.bytes(tail)?))).collect();
    Ok(NativeFile { doc, info: FileInfo { version: f.version, legacy: f.format == LEGACY_EXTENSION }, profiles })
}

/// The members of a native file besides its document (which is skipped, not decoded).
#[derive(Deserialize)]
struct Head {
    preview: Option<Image>,
    pdf: Option<Image>,
}

/// The file's head and the data after its JSON.
fn head(bytes: &[u8]) -> Option<(Head, Vec<u8>)> {
    let text = unpack(bytes).ok()?;
    let (json, tail) = split_tail(&text);
    Some((serde_json::from_slice(json).ok()?, tail.to_vec()))
}

/// The preview PNG embedded in a native file (`None`: it has none, or isn't a native file).
pub fn preview(bytes: &[u8]) -> Option<Vec<u8>> {
    let (h, tail) = head(bytes)?;
    h.preview?.bytes(&tail)
}

/// The PDF a native file saved with Create PDF-Compatible File carries (`None`: it has none, or
/// isn't a native file).
pub fn pdf_content(bytes: &[u8]) -> Option<Vec<u8>> {
    let (h, tail) = head(bytes)?;
    h.pdf?.bytes(&tail)
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
        d.insert(Some(l), 1, Node::new(img, NodeKind::Image(im))).unwrap();
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
