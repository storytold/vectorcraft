//! `.vectorcraft` v4: large data after the JSON (images, the PDF, profiles), small files still plain
//! JSON, older versions inline, damaged offsets refused.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_format::{BLOB_MAGIC, FormatError, INLINE_MAX, SaveOptions, VERSION, load, load_file, pdf_content, save_with};
use vectorcraft_geom::Affine;

/// A document showing image `key` of `len` bytes.
fn doc_with_image(key: &str, len: usize) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let bytes: Vec<u8> = (0..len).map(|i| (i * 7 % 251) as u8).collect();
    d.images.insert(key.into(), ImageBlob::new("image/png", bytes));
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let im = ImageObject { key: key.into(), width: 10, height: 10, xf: Affine::IDENTITY, link: None, placement: Default::default() };
    d.insert(Some(l), 0, Node::new(id, NodeKind::Image(im))).unwrap();
    d
}

fn has_tail(bytes: &[u8]) -> bool {
    bytes.windows(BLOB_MAGIC.len()).any(|w| w == BLOB_MAGIC)
}

#[test]
fn large_data_goes_after_the_json_and_reads_back() {
    let d = doc_with_image("big", INLINE_MAX + 10);
    let pdf = vec![b'%'; INLINE_MAX * 2];
    for compress in [false, true] {
        let o = SaveOptions { compress, pdf: Some(pdf.clone()), ..SaveOptions::default() };
        let bytes = save_with(&d, &o).unwrap();
        if !compress {
            assert!(has_tail(&bytes), "a binary section");
            // The JSON before it holds no base64 copy of the image.
            assert!(bytes.len() < INLINE_MAX + 10 + pdf.len() + (32 << 10), "{} bytes", bytes.len());
        }
        let back = load(&bytes).unwrap();
        assert_eq!(back.images["big"].bytes, d.images["big"].bytes, "compress {compress}");
        assert_eq!(pdf_content(&bytes).unwrap(), pdf);
    }
}

#[test]
fn small_files_stay_plain_json_and_older_versions_inline() {
    let small = save_with(&doc_with_image("small", 100), &SaveOptions::default()).unwrap();
    assert!(!has_tail(&small));
    let v: Value = serde_json::from_slice(&small).unwrap();
    assert_eq!(v["version"], VERSION);
    let big = doc_with_image("big", INLINE_MAX + 10);
    let v3 = save_with(&big, &SaveOptions { version: 3, ..SaveOptions::default() }).unwrap();
    assert!(!has_tail(&v3), "version 3 keeps everything in the JSON");
    let v: Value = serde_json::from_slice(&v3).unwrap();
    assert_eq!((v["format"].as_str(), v["version"].as_u64()), (Some("vectorcraft"), Some(3)));
    assert_eq!(load(&v3).unwrap().images["big"].bytes, big.images["big"].bytes);
}

#[test]
fn damaged_offsets_are_refused_not_a_crash() {
    let d = doc_with_image("big", INLINE_MAX + 10);
    let bytes = save_with(&d, &SaveOptions::default()).unwrap();
    // The data cut short.
    let cut = &bytes[..bytes.len() - 100];
    assert!(matches!(load_file(cut), Err(FormatError::BadImage(_))));
    // An offset past the end.
    let text = String::from_utf8_lossy(&bytes);
    let at = text.find("\"blob\":[").unwrap();
    let mut damaged = bytes.clone();
    damaged.splice(at..at + 8, b"\"blob\":[9".iter().copied());
    assert!(load_file(&damaged).is_err());
    // A missing section.
    let json_only = &bytes[..bytes.windows(BLOB_MAGIC.len()).position(|w| w == BLOB_MAGIC).unwrap()];
    assert!(load_file(json_only).is_err());
    let _ = Arc::new(());
}
