//! `.vectorcraft` v4: large data after the JSON (images, the PDF, profiles), small files still plain
//! JSON, older versions inline, damaged offsets refused.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_format::{BLOB_MAGIC, FormatError, INLINE_MAX, SaveOptions, load, load_file, pdf_content, save_with};
use vectorcraft_geom::Affine;

/// A document showing image `key` of `len` bytes.
fn doc_with_image(key: &str, len: usize) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let bytes: Vec<u8> = (0..len).map(|i| (i * 7 % 251) as u8).collect();
    d.images.insert(key.into(), ImageBlob::new("image/png", bytes));
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let im = ImageObject { key: key.into(), width: 10, height: 10, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
    d.insert(Some(l), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
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
    assert_eq!(v["version"], 4);
    let big = doc_with_image("big", INLINE_MAX + 10);
    let v3 = save_with(&big, &SaveOptions { version: 3, ..SaveOptions::default() }).unwrap();
    assert!(!has_tail(&v3), "version 3 keeps everything in the JSON");
    let v: Value = serde_json::from_slice(&v3).unwrap();
    assert_eq!((v["format"].as_str(), v["version"].as_u64()), (Some("vectorcraft"), Some(3)));
    assert_eq!(load(&v3).unwrap().images["big"].bytes, big.images["big"].bytes);
}

/// `json_part` (what exporter plug-ins are given): the whole JSON value, without the data after
/// it; a file without such data is its JSON already.
#[test]
fn json_part_is_the_json_without_the_data_after_it() {
    let big = save_with(&doc_with_image("big", INLINE_MAX + 10), &SaveOptions::default()).unwrap();
    let json = vectorcraft_format::json_part(&big);
    assert!(json.len() < big.len() - INLINE_MAX && !has_tail(json));
    let v: Value = serde_json::from_slice(json).unwrap();
    assert!(v["images"]["big"].get("blob").is_some() && v["images"]["big"].get("data").is_none(), "{}", v["images"]);
    let small = save_with(&doc_with_image("small", 100), &SaveOptions::default()).unwrap();
    assert_eq!(vectorcraft_format::json_part(&small), &small[..]);
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

/// A `preview` of the wrong shape reads as missing without hiding the file's PDF.
#[test]
fn a_damaged_preview_leaves_the_pdf_readable() {
    let pdf = b"%PDF-1.7 small".to_vec();
    let o = SaveOptions { version: 3, preview: Some(vec![5; 30]), pdf: Some(pdf.clone()), ..SaveOptions::default() };
    let mut v: Value = serde_json::from_slice(&save_with(&doc_with_image("small", 100), &o).unwrap()).unwrap();
    v["preview"] = Value::from(5);
    let damaged = serde_json::to_vec(&v).unwrap();
    assert_eq!(vectorcraft_format::preview(&damaged), None);
    assert_eq!(pdf_content(&damaged), Some(pdf));
    assert!(load_file(&damaged).is_ok());
}

/// A repeated `generator`, `preview` or `pdf` opens, of any shape; any other repeated member is
/// refused. The crate's `Contents` and `File` accept and refuse the same files.
#[test]
fn repeated_members_open_only_when_skipped() {
    let o = SaveOptions { version: 3, ..SaveOptions::default() };
    let text = String::from_utf8(save_with(&doc_with_image("small", 100), &o).unwrap()).unwrap();
    let document = serde_json::from_str::<Value>(&text).unwrap()["document"].to_string();
    let with = |members: &str| text.replacen('{', &format!("{{{members},"), 1);
    let contents_reads = |s: &str| {
        let contents = craft_vectorcraft_format::Contents::<serde::de::IgnoredAny>::read(s.as_bytes()).is_ok();
        assert_eq!(craft_vectorcraft_format::File::<serde::de::IgnoredAny>::read(s.as_bytes()).is_ok(), contents, "{s}");
        contents
    };
    let skipped = with(r#""preview":5,"preview":{"mime":"image/png","data":"AA=="},"generator":[],"generator":"x","pdf":{},"pdf":null"#);
    let back = load_file(skipped.as_bytes()).unwrap();
    assert_eq!(back.doc.images["small"].bytes.len(), 100);
    assert!(contents_reads(&skipped));
    let document = format!(r#""document":{document}"#);
    for member in [document.as_str(), r#""format":"vectorcraft""#, r#""version":3"#, r#""images":{}"#, r#""profiles":{}"#] {
        let repeated = with(&format!("{member},{member}"));
        let err = load_file(repeated.as_bytes()).err().unwrap_or_else(|| panic!("{member}: a repeated member opened"));
        assert!(err.to_string().contains("duplicate field"), "{member}: {err}");
        assert!(!contents_reads(&repeated), "{member}: the crate's Contents read a repeated member");
    }
}

/// A file read back and saved again with the same options gives the same bytes, in every version,
/// compressed or not, with a preview, profiles and a PDF inline and after the JSON.
#[test]
fn a_file_read_back_saves_to_the_same_bytes() {
    let mut d = doc_with_image("big", INLINE_MAX + 10);
    d.images.insert("small".into(), ImageBlob::new("image/png", vec![1, 2, 3]));
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let im = ImageObject { key: "small".into(), width: 1, height: 1, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
    d.insert(Some(l), 1, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
    let profiles: BTreeMap<String, Vec<u8>> = [("Small".to_string(), vec![7; 40]), ("Large".to_string(), vec![9; INLINE_MAX])].into_iter().collect();
    for version in 1..=4 {
        for compress in [false, true].into_iter().filter(|&c| !c || version >= 3) {
            for pretty in [false, true] {
                let o = SaveOptions {
                    version,
                    compress,
                    pretty,
                    preview: Some(vec![5; 300]),
                    profiles: profiles.clone(),
                    pdf: Some(vec![b'%'; INLINE_MAX * 2]),
                    ..SaveOptions::default()
                };
                let first = save_with(&d, &o).unwrap();
                if !compress {
                    let json = vectorcraft_format::json_part(&first);
                    let env = craft_vectorcraft_format::File::<serde::de::IgnoredAny>::read(json).unwrap();
                    assert_eq!((env.version, env.images.len(), env.profiles.len()), (version, 2, 2));
                    assert!(env.preview.is_some() && env.pdf.is_some());
                }
                let back = load_file(&first).unwrap();
                assert_eq!(back.profiles, profiles, "v{version} compress {compress} pretty {pretty}");
                let again = save_with(&back.doc, &SaveOptions { profiles: back.profiles, ..o }).unwrap();
                assert!(first == again, "v{version} compress {compress} pretty {pretty}: the second save differs");
            }
        }
    }
}
