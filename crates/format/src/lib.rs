//! The native `.vectorcraft` format.
//!
//! A `.vectorcraft` file is UTF-8 JSON:
//! ```json
//! { "format": "vectorcraft", "version": 1, "generator": "VectorCraft 0.1.0",
//!   "document": { …vectorcraft_doc::Document… },
//!   "images": { "<key>": { "mime": "image/png", "data": "<base64>" } } }
//! ```
//! It is lossless for everything in the document model and preserves unknown fields under
//! `document.unknown`. Readers must reject files whose `version` is newer than they support.
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_doc::{Document, ImageBlob};

/// v2: anchors as `{p: [x, y], in?, out?, kind?}` and default-valued fields omitted (v1 files still load).
pub const VERSION: u32 = 2;
pub const EXTENSION: &str = "vectorcraft";
/// Extension and format name from before the project was renamed (DrawCraft): still opened.
pub const LEGACY_EXTENSION: &str = "drawcraft";

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
}

#[derive(Serialize, Deserialize)]
struct Image {
    mime: String,
    data: String,
}

#[derive(Serialize, Deserialize)]
struct File {
    format: String,
    version: u32,
    #[serde(default)]
    generator: String,
    document: Document,
    #[serde(default)]
    images: BTreeMap<String, Image>,
}

/// Serialize a document (pretty = human-diffable). Editing-mode working copies are left out
/// ([`Document::without_edit_modes`]).
pub fn save(doc: &Document, pretty: bool) -> Vec<u8> {
    let doc = doc.without_edit_modes();
    let images = doc.images.iter().map(|(k, b)| (k.clone(), Image { mime: b.mime.clone(), data: base64_encode(&b.bytes) })).collect();
    let f = File {
        format: "vectorcraft".into(),
        version: VERSION,
        generator: format!("VectorCraft {}", env!("CARGO_PKG_VERSION")),
        document: doc.into_owned(),
        images,
    };
    if pretty { serde_json::to_vec_pretty(&f).unwrap_or_default() } else { serde_json::to_vec(&f).unwrap_or_default() }
}

/// Documents up to this many objects are saved pretty-printed (readable, diffable); larger ones
/// compact, where indentation would multiply the file size and slow opening down.
pub const PRETTY_MAX_OBJECTS: usize = 2000;

/// Serialize for saving to disk: pretty when small, compact when large.
pub fn save_file(doc: &Document) -> Vec<u8> {
    let objects: usize = doc.layers.iter().map(|l| l.count()).sum();
    save(doc, objects <= PRETTY_MAX_OBJECTS)
}

pub fn load(bytes: &[u8]) -> Result<Document, FormatError> {
    let f: File = serde_json::from_slice(bytes).map_err(|e| FormatError::NotVectorcraft(e.to_string()))?;
    if f.format != EXTENSION && f.format != LEGACY_EXTENSION {
        return Err(FormatError::NotVectorcraft(format!("format is `{}`", f.format)));
    }
    if f.version > VERSION {
        return Err(FormatError::TooNew(f.version));
    }
    let mut doc = f.document;
    for (k, img) in f.images {
        let bytes = base64_decode(&img.data).ok_or_else(|| FormatError::BadImage(k.clone()))?;
        doc.images.insert(k, ImageBlob { mime: img.mime, bytes: Arc::new(bytes) });
    }
    // Saved mid-edit by an older version: drop the opacity-mask editing layer.
    doc.drop_edit_modes();
    // Saved before per-fill/stroke overprint: the Overprint Black list becomes overprint flags.
    doc.migrate_overprint_black();
    doc.fix_next_id();
    Ok(doc)
}

/// Does this look like a `.vectorcraft` file?
pub fn sniff(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(256)];
    std::str::from_utf8(head).is_ok_and(|s| s.trim_start().starts_with('{') && (s.contains("\"vectorcraft\"") || s.contains("\"drawcraft\"")))
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
    use vectorcraft_doc::{Appearance, Node};
    use vectorcraft_geom::{Rect, shapes};

    #[test]
    fn roundtrip() {
        let mut d = Document::new(100.0, 100.0);
        let l = d.layers[0].id;
        let id = d.alloc_id();
        d.insert(Some(l), 0, Node::path(id, shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art())).unwrap();
        d.images.insert("k".into(), ImageBlob { mime: "image/png".into(), bytes: Arc::new(vec![1, 2, 3, 250]) });
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
