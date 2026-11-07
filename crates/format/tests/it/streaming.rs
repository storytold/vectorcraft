//! Saving and opening as streams: the bytes a save writes are those an in-memory save wrote
//! before (the same JSON and data after it, unpacked; the same gzip header), files written the old
//! way still open, entries are read whatever their order, and the limits apply per part of the
//! file.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_format::{BLOB_MAGIC, FormatError, INLINE_MAX, MAX_BLOB, SaveOptions, load, load_file, pdf_content, preview, save_to, save_with};
use vectorcraft_geom::Affine;

/// A document showing images `(key, mime, len)`.
fn doc_with(images: &[(&str, &str, usize)]) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    for (n, &(key, mime, len)) in images.iter().enumerate() {
        let bytes: Vec<u8> = (0..len).map(|i| ((i * 7 + n * 13) % 251) as u8).collect();
        d.images.insert(key.into(), ImageBlob::new(mime, bytes));
        let id = d.alloc_id();
        let im =
            ImageObject { key: key.into(), width: 10, height: 10, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
        d.insert(Some(l), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
    }
    d
}

/// gzip as the app compressed files before saves were streamed (the whole file at once).
fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(bytes).unwrap();
    gz.finish().unwrap()
}

fn gunzip(bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut out).unwrap();
    out
}

fn mixed() -> Document {
    doc_with(&[
        ("photo", "image/jpeg", INLINE_MAX * 3),
        ("scan", "image/tiff", INLINE_MAX * 2),
        ("icon", "image/png", 100),
        ("art", "image/png", INLINE_MAX + 1),
    ])
}

#[test]
fn compressed_files_unpack_to_the_plain_file() {
    let pdf = vec![b'%'; INLINE_MAX * 2];
    let profiles = BTreeMap::from([("Mine".to_string(), vec![7u8; INLINE_MAX + 3])]);
    for d in [Document::new(10.0, 10.0), mixed()] {
        let o = SaveOptions { pdf: Some(pdf.clone()), profiles: profiles.clone(), preview: Some(vec![1, 2, 3]), ..SaveOptions::default() };
        let plain = save_with(&d, &o).unwrap();
        let packed = save_with(&d, &SaveOptions { compress: true, ..o }).unwrap();
        assert_eq!(gunzip(&packed), plain, "the same file, compressed");
        let back = load_file(&packed).unwrap();
        for (k, b) in &d.images {
            assert_eq!(back.doc.images[k].bytes, b.bytes, "{k}");
        }
        assert_eq!(back.profiles, profiles);
        assert_eq!(pdf_content(&packed).unwrap(), pdf);
        assert_eq!(preview(&packed).unwrap(), vec![1, 2, 3]);
    }
}

/// The deflate bytes themselves depend on flate2's backend: zlib-rs (which hayro turns on) writes
/// other blocks for data fed in other pieces than the in-memory save fed it, miniz_oxide didn't.
#[test]
fn without_compressed_images_the_gzip_stream_is_the_old_one() {
    let mut d = doc_with(&[("scan", "image/tiff", INLINE_MAX * 2), ("icon", "image/png", 100)]);
    d.title = "No photos".into();
    for o in [SaveOptions::default(), SaveOptions { pdf: Some(vec![b'%'; INLINE_MAX]), ..SaveOptions::default() }] {
        let plain = save_with(&d, &o).unwrap();
        let (packed, old) = (save_with(&d, &SaveOptions { compress: true, ..o }).unwrap(), gzip(&plain));
        assert_eq!(packed.get(..10), old.get(..10), "the same header");
        assert_eq!(gunzip(&packed), plain);
    }
}

#[test]
fn files_compressed_the_old_way_still_open() {
    let d = mixed();
    let old = gzip(&save_with(&d, &SaveOptions::default()).unwrap());
    let back = load(&old).unwrap();
    assert_eq!(back.images.len(), d.images.len());
    for (k, b) in &d.images {
        assert_eq!(back.images[k].bytes, b.bytes, "{k}");
    }
    assert_eq!(back.node_count(), d.node_count());
}

#[test]
fn save_to_writes_what_save_with_returns() {
    let d = mixed();
    for compress in [false, true] {
        let o = SaveOptions { compress, ..SaveOptions::default() };
        let mut streamed = vec![];
        save_to(&d, &o, &mut streamed).unwrap();
        assert_eq!(streamed, save_with(&d, &o).unwrap());
        let back = load(&streamed).unwrap();
        assert_eq!(back.images["photo"].bytes, d.images["photo"].bytes);
    }
    // A failing writer fails the save.
    struct Full;
    impl Write for Full {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    assert!(save_to(&d, &SaveOptions::default(), Full).is_err());
    assert!(save_to(&d, &SaveOptions { compress: true, ..SaveOptions::default() }, Full).is_err());
}

/// `plain` with its JSON changed by `edit` (the data after it kept).
fn with_json(plain: &[u8], edit: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
    let at = plain.windows(BLOB_MAGIC.len()).position(|w| w == BLOB_MAGIC).unwrap();
    let mut v: serde_json::Value = serde_json::from_slice(&plain[..at]).unwrap();
    edit(&mut v);
    let mut out = serde_json::to_vec(&v).unwrap();
    out.extend_from_slice(&plain[at..]);
    out
}

#[test]
fn entries_in_any_order_or_shared_read_back() {
    let d = doc_with(&[("a", "image/png", INLINE_MAX), ("b", "image/png", INLINE_MAX + 5)]);
    let plain = save_with(&d, &SaveOptions::default()).unwrap();
    // Swap the entries' places (b's bytes listed first), and point a third image at a's bytes.
    let file = with_json(&plain, |v| {
        let (a, b) = (v["images"]["a"]["blob"].clone(), v["images"]["b"]["blob"].clone());
        v["images"]["a"]["blob"] = b.clone();
        v["images"]["b"]["blob"] = a.clone();
        v["images"]["c"] = serde_json::json!({"mime": "image/png", "blob": b});
    });
    for bytes in [file.clone(), gzip(&file)] {
        let back = load(&bytes).unwrap();
        assert_eq!(back.images["a"].bytes, d.images["b"].bytes);
        assert_eq!(back.images["b"].bytes, d.images["a"].bytes);
        assert_eq!(back.images["c"].bytes, d.images["b"].bytes);
    }
}

#[test]
fn limits_apply_per_entry_and_to_all_entries() {
    let d = doc_with(&[("a", "image/png", INLINE_MAX), ("b", "image/png", INLINE_MAX)]);
    let plain = save_with(&d, &SaveOptions::default()).unwrap();
    // One entry past the limit: refused before anything is allocated.
    let huge = with_json(&plain, |v| v["images"]["a"]["blob"] = serde_json::json!([0, MAX_BLOB + 1]));
    assert!(matches!(load_file(&gzip(&huge)), Err(FormatError::TooBig)));
    // Each within it, together past the limit for all.
    let many = with_json(&plain, |v| {
        for i in 0..5 {
            v["images"][format!("x{i}")] = serde_json::json!({"mime": "image/png", "blob": [0, MAX_BLOB]});
        }
    });
    assert!(matches!(load_file(&gzip(&many)), Err(FormatError::TooBig)));
    // A length the data doesn't have: the image is damaged, nothing more.
    let short = with_json(&plain, |v| v["images"]["a"]["blob"] = serde_json::json!([0, 1u64 << 20]));
    assert!(matches!(load_file(&gzip(&short)), Err(FormatError::BadImage(_))));
    // Uncompressed files have no limits to apply (the file is all there is).
    assert!(matches!(load_file(&huge), Err(FormatError::BadImage(_))));
}

#[test]
fn a_damaged_compressed_file_is_refused() {
    let bytes = save_with(&mixed(), &SaveOptions { compress: true, ..SaveOptions::default() }).unwrap();
    // The data after the JSON cut short, then its checksum wrong.
    assert!(load_file(&bytes[..bytes.len() - 4000]).is_err());
    let mut bad = bytes.clone();
    let n = bad.len();
    bad[n - 6] ^= 0xff;
    assert!(load_file(&bad).is_err(), "the checksum is checked to the end");
}

#[test]
fn documents_read_whatever_their_key_order_and_keep_keys_they_dont_know() {
    let d = mixed();
    let plain = save_with(&d, &SaveOptions::default()).unwrap();
    let file = with_json(&plain, |v| {
        // serde_json's map keeps keys sorted: `type` is no longer first in the node kinds (a
        // layer's `children`, `clip` and `color` come before it), and the document's fields come
        // in another order.
        v["document"]["aaa_from_a_newer_version"] = serde_json::json!({"x": [1, 2]});
        v["document"]["zzz_too"] = serde_json::json!("kept");
    });
    let text = String::from_utf8_lossy(&file);
    assert!(!text.contains(r#"{"type":"layer""#) && text.contains(r#""type":"layer""#), "`type` isn't first");
    for bytes in [file.clone(), gzip(&file)] {
        let back = load(&bytes).unwrap();
        assert_eq!(back.layers, load(&plain).unwrap().layers);
        assert_eq!(back.extra["aaa_from_a_newer_version"]["x"], serde_json::json!([1, 2]));
        assert_eq!(back.extra["zzz_too"], "kept");
        // Saved again, they are kept.
        assert_eq!(load(&save_with(&back, &SaveOptions::default()).unwrap()).unwrap().extra["zzz_too"], "kept");
    }
}
