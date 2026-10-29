// Tests may unwrap and panic (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_format::{FormatError, SaveOptions, VERSION, save_with};
use vectorcraft_geom::{Rect, shapes};

use super::*;

/// A 400 × 200 artboard with an ellipse on it.
fn doc() -> Document {
    let mut d = Document::new(400.0, 200.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    d.insert(Some(l), 0, Node::path(id, shapes::ellipse(Rect::new(20.0, 20.0, 380.0, 180.0)), Appearance::default_art())).unwrap();
    d
}

/// A `w` × `h` PNG of one opaque colour.
fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba([200, 30, 60, 255]));
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn saved(preview: Option<Vec<u8>>, compress: bool) -> Vec<u8> {
    save_with(&doc(), &SaveOptions { preview, compress, ..SaveOptions::default() }).unwrap()
}

#[test]
fn fit_keeps_proportions_within_the_bounds() {
    assert_eq!(fit(256, 128, 512.0, 512.0), Some((512, 256)));
    assert_eq!(fit(128, 256, 512.0, 512.0), Some((256, 512)));
    assert_eq!(fit(256, 128, 64.0, 16.0), Some((32, 16)));
    assert_eq!(fit(256, 1, 16.0, 16.0), Some((16, 1)), "never thinner than a pixel");
    assert_eq!(fit(256, 256, 1e9, 1e9), Some((MAX_THUMBNAIL_SIDE, MAX_THUMBNAIL_SIDE)), "capped");
    for (w, h, mw, mh) in [(0, 1, 9.0, 9.0), (1, 0, 9.0, 9.0), (1, 1, f64::NAN, 9.0), (1, 1, 9.0, f64::INFINITY), (1, 1, 0.5, 9.0), (1, 1, -3.0, 9.0)]
    {
        assert_eq!(fit(w, h, mw, mh), None, "{w}×{h} into {mw}×{mh}");
    }
}

#[test]
fn thumbnails_are_the_embedded_preview_scaled() {
    for compress in [false, true] {
        // A large preview is kept after the JSON (v4) rather than inline.
        for (w, h) in [(256, 128), (2000, 1000)] {
            let t = thumbnail(&saved(Some(png(w, h)), compress), 512.0, 512.0).unwrap();
            assert_eq!((t.width, t.height), (512, 256), "{w}×{h}, compress {compress}");
            assert_eq!(t.rgba.len(), 512 * 256 * 4);
            assert_eq!(t.rgba[..4], [200, 30, 60, 255]);
        }
    }
    let same = thumbnail(&saved(Some(png(256, 128)), false), 256.0, 256.0).unwrap();
    assert_eq!((same.width, same.height), (256, 128));
}

#[test]
fn files_from_before_the_rename_and_older_versions_have_thumbnails() {
    let text = String::from_utf8(saved(Some(png(64, 32)), false)).unwrap();
    let legacy = text.replacen("\"format\":\"vectorcraft\"", "\"format\":\"drawcraft\"", 1);
    assert!(thumbnail(legacy.as_bytes(), 128.0, 128.0).is_ok());
    for version in 1..VERSION {
        let bytes = save_with(&doc(), &SaveOptions { version, preview: Some(png(64, 32)), ..SaveOptions::default() }).unwrap();
        assert!(thumbnail(&bytes, 128.0, 128.0).is_ok(), "version {version}");
    }
}

#[test]
fn files_without_anything_to_show_are_errors() {
    assert!(matches!(thumbnail(&saved(None, false), 128.0, 128.0), Err(QuickLookError::NoPreview)));
    let text = String::from_utf8(saved(Some(png(64, 32)), false)).unwrap();
    let newer = text.replacen(&format!("\"version\":{VERSION}"), "\"version\":99", 1);
    let e = thumbnail(newer.as_bytes(), 128.0, 128.0).unwrap_err();
    assert!(matches!(e, QuickLookError::Format(FormatError::TooNew(99))), "{e}");
    assert_eq!(e.code(), 3);
    assert!(matches!(preview(newer.into_bytes(), &Limits::default()), Err(QuickLookError::Format(FormatError::TooNew(99)))), "not rendered either");
    for junk in [&b""[..], b"{}", b"not a document", b"\x1f\x8b\x08\x00junk", b"{\"format\":\"vectorcraft\",\"version\":4,\"preview\":7}"] {
        assert!(thumbnail(junk, 128.0, 128.0).is_err(), "{junk:?}");
        assert!(preview(junk.to_vec(), &Limits::default()).is_err(), "{junk:?}");
    }
    // Bytes that aren't a PNG, and a PNG larger than the extensions decode.
    assert!(matches!(thumbnail(&saved(Some(b"\x89PNG not really".to_vec()), false), 99.0, 99.0), Err(QuickLookError::BadPreview(_))));
    assert!(matches!(thumbnail(&saved(Some(png(MAX_EMBEDDED_SIDE + 1, 1)), false), 99.0, 99.0), Err(QuickLookError::BadPreview(_))));
    assert!(matches!(thumbnail(&saved(Some(png(8, 8)), false), f64::NAN, 99.0), Err(QuickLookError::Empty)));
}

#[test]
fn reading_stops_at_the_cap() {
    let dir = std::env::temp_dir().join(format!("vc-quicklook-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("doc.vectorcraft");
    std::fs::write(&path, saved(None, true)).unwrap();
    let len = std::fs::metadata(&path).unwrap().len();
    assert_eq!(read_file(&path, len).unwrap().len() as u64, len);
    assert!(matches!(read_file(&path, len - 1), Err(QuickLookError::TooLarge(n)) if n == len));
    assert!(matches!(read_file(&dir.join("missing.vectorcraft"), MAX_FILE), Err(QuickLookError::Read(_))));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The pixels per metre a PNG's pHYs chunk gives.
fn per_metre(png: &[u8]) -> Option<u32> {
    let d = png::Decoder::new(std::io::Cursor::new(png));
    let r = d.read_info().unwrap();
    r.info().pixel_dims.map(|p| p.xppu)
}

#[test]
fn previews_render_the_first_artboard_within_the_limits() {
    let p = preview(saved(None, true), &Limits::default()).unwrap();
    assert!(p.rendered);
    assert_eq!((p.width, p.height), (2048, 1024), "the longer side at max_side");
    assert_eq!(p.points, (400.0, 200.0), "shown at the artboard's size");
    // 2048 px for 400 pt: 368.64 pixels per inch.
    assert_eq!(per_metre(&p.png), Some((72.0 * 2048.0 / 400.0 / 0.0254_f64).round() as u32));
    let img = image::load_from_memory(&p.png).unwrap().into_rgba8();
    assert_eq!(img.get_pixel(2, 2).0, [255, 255, 255, 255], "the artboard is white");
    assert!(img.pixels().any(|p| p.0 != [255, 255, 255, 255]), "the ellipse's outline is drawn");

    let small = Limits { max_side: 4096, max_pixels: 20_000, ..Limits::default() };
    let p = preview(saved(None, false), &small).unwrap();
    assert!(u64::from(p.width) * u64::from(p.height) <= 20_400, "{}×{}", p.width, p.height);
}

#[test]
fn previews_fall_back_to_the_embedded_one() {
    let embedded = png(64, 32);
    let no_time = Limits { time: Duration::ZERO, ..Limits::default() };
    let p = preview(saved(Some(embedded.clone()), true), &no_time).unwrap();
    assert!(!p.rendered && p.png == embedded && p.points == (64.0, 32.0), "out of time");
    // Fresh limits: the render above may still be running, and would make this one wait its turn.
    let no_time = Limits { time: Duration::ZERO, ..Limits::default() };
    assert!(matches!(preview(saved(None, true), &no_time), Err(QuickLookError::TimedOut(_))));
    let tiny_file = Limits { max_render_file: 10, ..Limits::default() };
    let p = preview(saved(Some(embedded.clone()), false), &tiny_file).unwrap();
    assert!(!p.rendered, "too large to render");
    // Nothing to render: no artboard and no art.
    let mut empty = Document::new(10.0, 10.0);
    empty.artboards.clear();
    let bytes = save_with(&empty, &SaveOptions::default()).unwrap();
    assert!(matches!(preview(bytes, &Limits::default()), Err(QuickLookError::Empty)));
    let bytes = save_with(&empty, &SaveOptions { preview: Some(embedded.clone()), ..SaveOptions::default() }).unwrap();
    assert!(!preview(bytes, &Limits::default()).unwrap().rendered);
}

#[test]
fn previews_of_documents_past_the_render_caps_show_the_embedded_one() {
    let embedded = png(64, 32);
    let bytes = saved(Some(embedded.clone()), true);
    let tight = Limits { caps: vectorcraft_format::ReadCaps { json: 100, blobs: 0 }, ..Limits::default() };
    let p = preview(bytes.clone(), &tight).unwrap();
    assert!(!p.rendered && p.png == embedded, "past the caps: not rendered");
    assert!(matches!(render(&bytes, &tight), Err(QuickLookError::Format(FormatError::TooBig))));
    assert!(matches!(preview(saved(None, true), &tight), Err(QuickLookError::Format(FormatError::TooBig))), "nothing else to show");
    assert!(preview(bytes, &Limits::default()).unwrap().rendered);
    // Uncompressed files too.
    let plain = saved(Some(embedded.clone()), false);
    let p = preview(plain.clone(), &tight).unwrap();
    assert!(!p.rendered && p.png == embedded, "uncompressed, past the caps: not rendered");
    assert!(matches!(render(&plain, &tight), Err(QuickLookError::Format(FormatError::TooBig))));
    assert!(preview(plain, &Limits::default()).unwrap().rendered);
}

#[test]
fn renders_that_ran_out_of_time_are_counted_until_they_end() {
    let limits = Limits { time: Duration::ZERO, ..Limits::default() };
    // While one is still running, no other render starts.
    limits.late.store(1, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(render_in_time(saved(None, false), &limits), Err(QuickLookError::Busy)));
    let embedded = png(64, 32);
    let p = preview(saved(Some(embedded.clone()), false), &limits).unwrap();
    assert!(!p.rendered && p.png == embedded, "busy: the embedded preview");
    assert_eq!(QuickLookError::Busy.code(), 11);
    // A render given up on counts until its thread ends, then no longer.
    limits.late.store(0, std::sync::atomic::Ordering::SeqCst);
    assert!(matches!(render_in_time(saved(None, false), &limits), Err(QuickLookError::TimedOut(_))));
    let start = std::time::Instant::now();
    while limits.late.load(std::sync::atomic::Ordering::SeqCst) != 0 {
        assert!(start.elapsed() < Duration::from_secs(30), "the late render never stopped counting");
        std::thread::sleep(Duration::from_millis(5));
    }
    let roomy = Limits { late: limits.late.clone(), ..Limits::default() };
    assert!(preview(saved(None, false), &roomy).unwrap().rendered, "renders start again");
}

#[test]
fn preview_scale_is_capped() {
    let l = Limits::default();
    assert_eq!(preview_scale(400.0, 200.0, &l), Some(2048.0 / 400.0));
    assert_eq!(preview_scale(10.0, 10.0, &l), Some(8.0), "a tiny artboard isn't blown up past 8×");
    let s = preview_scale(10_000.0, 10_000.0, &Limits { max_pixels: 1_000_000, ..l.clone() }).unwrap();
    assert!((s * 10_000.0 - 1000.0).abs() < 1e-6);
    for (w, h) in [(0.0, 1.0), (1.0, -1.0), (f64::NAN, 1.0), (f64::INFINITY, 1.0)] {
        assert_eq!(preview_scale(w, h, &l), None);
    }
}
