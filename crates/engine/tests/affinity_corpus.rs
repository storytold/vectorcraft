//! Public Affinity documents (`cargo xtask corpus --affinity`, pinned and sha256-verified) open
//! natively, and their art renders like Affinity's own picture of them: every document embeds a
//! thumbnail Affinity rendered when saving it. Each file's mean difference from that thumbnail
//! (0–255 per channel, both flattened on white, at the thumbnail's size) must stay under its
//! ceiling, which sits just above the difference measured when the importer landed.
//!
//! Each artboard is checked. Local runs may skip an absent corpus; CI requires the complete
//! pinned corpus with `AFFINITY_CORPUS_REQUIRED=1`. Tests recheck hashes before using inputs.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

#[path = "../../affinity/tests/support/corpus.rs"]
mod corpus_gate;

/// (file, ceiling). Measured differences when the importer landed are in the comments.
const FILES: &[(&str, f64)] = &[
    ("patchy-artboards.af", 0.25),              // 0.00 across both boards, including overflow clipping
    ("patchy-rgba8.af", 3.0),                   // 2.34: embedded thumbnail versus raster sampling
    ("patchy-rgba16.af", 3.0),                  // 2.34
    ("patchy-dpi300.af", 1.5),                  // 0.87
    ("patchy-group.af", 0.25),                  // 0.00
    ("patchy-transform.af", 1.0),               // 0.19
    ("patchy-vector.af", 1.0),                  // 0.12
    ("patchy-shapes.af", 1.0),                  // 0.24: per-corner radii, a straight corner
    ("patchy-shapes-2.af", 11.5), // 10.51: diamond, double star, crescent, cloud, segment, tear as ellipses (14.75 with the heart, cog and arrow too)
    ("patchy-lazy-placed.af", 1.0), // 0.50: upstream byte-derived lazy-image regression
    ("patchy-embedded-jpeg.af", 1.0), // 0.50
    ("affinity3-mexican-guy.af", 3.0), // 1.80: brush strokes as plain strokes
    ("affinity3-mexican-man.af", 3.5), // 2.24
    ("affinity3-mexican-woman.af", 3.5), // 2.55
    ("affinity3-playing-cards.af", 5.5), // 4.32: layer effects
    ("designer-beer.afdesign", 4.0), // 2.90
    ("designer-cactus.afdesign", 1.5), // 0.33
    ("designer-car.afdesign", 1.5), // 0.40
    ("designer-flowers.afdesign", 3.0), // 2.01
    ("designer-lion-track.afdesign", 5.5), // 4.34
    ("afdesignload-color.afdesign", 1.0), // 0.00
    ("afdesignload-layer_mode.afdesign", 1.0), // 0.00
    ("afdesignload-layer_test.afdesign", 1.0), // 0.00
    ("afdesignload-margins.afdesign", 1.0), // 0.00
    ("afdesignload-raster_test.afdesign", 2.0), // 0.72: pixel layer
    ("afdesignload-revision_test.afdesign", 1.0),
    ("afdesignload-shape_test.afdesign", 2.5), // 1.69: the cloud as an ellipse (3.50 with the heart as one too)
    ("afdesignload-slice_test.afdesign", 1.0),
    ("afdesignload-test_path.afdesign", 1.5), // 0.26
    ("jac21-SimpleLogo.afdesign", 1.5),       // 0.19
    ("jac21-SimpleLogoBanner.afdesign", 5.5), // 4.42: placed images in a Display P3 document
    ("eviltwo-AssetStore.aftemplate", 1.0),   // 0.00: artboards
    ("leakcanary-vector_icon.afdesign", 3.5), // 2.33
];

fn corpus() -> Option<PathBuf> {
    corpus_gate::pinned()
}

/// Mean absolute difference of two RGBA8 images of one size, both composited on white.
fn difference(a: &[u8], b: &[u8]) -> f64 {
    let white = |p: &[u8]| -> [f64; 3] {
        let al = f64::from(p[3]) / 255.0;
        [0, 1, 2].map(|i| f64::from(p[i]) * al + 255.0 * (1.0 - al))
    };
    let (mut sum, mut n) = (0.0, 0usize);
    for (p, q) in a.as_chunks::<4>().0.iter().zip(b.as_chunks::<4>().0.iter()) {
        let (x, y) = (white(p), white(q));
        sum += (0..3).map(|i| (x[i] - y[i]).abs()).sum::<f64>();
        n += 3;
    }
    sum / n.max(1) as f64
}

#[test]
fn public_affinity_documents_render_like_their_affinity_thumbnails() {
    let Some(dir) = corpus() else {
        eprintln!("corpus/affinity is absent: run `cargo xtask corpus --affinity`");
        return;
    };
    let mut failures = Vec::new();
    for (name, ceiling) in FILES {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let loaded = vectorcraft_engine::cmd::fileio::load(name, &bytes).unwrap();
        assert!(!loaded.preview_only, "{name} opened as its preview: {:?}", loaded.warnings);
        let d = &loaded.doc;
        let thumb = image::load_from_memory(vectorcraft_affinity::preview(&bytes).unwrap().png).unwrap().to_rgba8();
        // The embedded thumbnail shows the union of the artboards. Check every board,
        // including small ones that the previous largest-board-only check omitted.
        let union = d.artboards.iter().map(|a| a.rect).reduce(|a, b| a.union(b)).unwrap();
        assert!(union.width().is_finite() && union.width() > 0.0);
        let scale = f64::from(thumb.width()) / union.width();
        for (index, artboard) in d.artboards.iter().enumerate() {
            let board = artboard.rect;
            let crop = |v: f64| (v * scale).round().max(0.0) as u32;
            let (x, y) = (crop(board.x0 - union.x0), crop(board.y0 - union.y0));
            assert!(x < thumb.width() && y < thumb.height(), "{name} board {index}: thumbnail crop outside image");
            let (w, h) = (crop(board.width()).min(thumb.width() - x), crop(board.height()).min(thumb.height() - y));
            assert!(w > 0 && h > 0, "{name} board {index}: empty thumbnail crop");
            let expected = image::imageops::crop_imm(&thumb, x, y, w, h).to_image();
            let rendered = vectorcraft_render::Renderer::new().render_region(d, board, scale, false);
            let got = image::RgbaImage::from_raw(rendered.width, rendered.height, rendered.to_straight()).unwrap();
            let got = image::imageops::resize(&got, w, h, image::imageops::FilterType::Triangle);
            let diff = difference(expected.as_raw(), got.as_raw());
            eprintln!("{name} board {index} ({}): {diff:.2} (ceiling {ceiling})", artboard.name);
            if diff > *ceiling {
                failures.push(format!("{name} board {index}: {diff:.2} > {ceiling}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every actual .af fixture must open as editable art, retain its complete native document,
/// and export every artboard. Reimporting our SVG/PDF and comparing our PSD merged pixels
/// checks our export paths; it is not an independent Affinity-versus-PSD fidelity oracle.
#[test]
fn affinity3_all_artboards_survive_native_save_and_export() {
    let Some(dir) = corpus() else { return };
    let names: Vec<_> = include_str!("../../../xtask/affinity-corpus.sha256")
        .lines()
        .filter_map(|l| l.split_once("  "))
        .map(|(_, name)| name)
        .filter(|name| name.ends_with(".af"))
        .collect();
    for name in names {
        let bytes = std::fs::read(dir.join(name)).unwrap();
        let loaded = vectorcraft_engine::cmd::fileio::load(name, &bytes).unwrap();
        assert!(!loaded.preview_only, "{name}: preview-only import: {:?}", loaded.warnings);
        let doc = &loaded.doc;
        assert!(!doc.artboards.is_empty(), "{name}: no artboards");
        let native = vectorcraft_format::save(doc, false);
        let reloaded = vectorcraft_format::load(&native).unwrap();
        assert_eq!(serde_json::to_value(&reloaded).unwrap(), serde_json::to_value(doc).unwrap(), "{name}: native save/reload lost document data");
        if doc.artboards.len() > 1 {
            let params = serde_json::json!({"range": "all", "useArtboards": true, "preserveEditing": false});
            let svg = vectorcraft_engine::cmd::fileio::encode_all(doc, "svg", &params).unwrap();
            assert_eq!(svg.files.len(), doc.artboards.len(), "{name}: SVG must write every artboard");
            assert_eq!(
                svg.files.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
                (0..doc.artboards.len()).map(Some).collect::<Vec<_>>(),
                "{name}: SVG artboard order"
            );
            let pdf = vectorcraft_engine::cmd::fileio::encode(doc, "pdf", &params).unwrap();
            assert_eq!(
                vectorcraft_pdf::info(&pdf, None).unwrap().pages.len(),
                doc.artboards.len(),
                "{name}: PDF must write every artboard as a page"
            );
        }
        for (index, board) in doc.artboards.iter().enumerate() {
            assert!(
                board.rect.width().is_finite() && board.rect.width() > 0.0 && board.rect.height().is_finite() && board.rect.height() > 0.0,
                "{name} board {index}: invalid bounds"
            );
            let scale = 128.0 / board.rect.width().max(board.rect.height());
            let params =
                serde_json::json!({"artboard": index, "useArtboards": true, "scale": scale, "background": "transparent", "preserveEditing": false});
            let svg = vectorcraft_engine::cmd::fileio::encode(doc, "svg", &params).unwrap();
            let svg = vectorcraft_svg::import(std::str::from_utf8(&svg).unwrap()).unwrap();
            assert_eq!(svg.artboards.len(), 1, "{name} board {index}: SVG page count");
            assert!((svg.artboards[0].rect.width() - board.rect.width()).abs() < 0.01, "{name} board {index}: SVG width");
            assert!((svg.artboards[0].rect.height() - board.rect.height()).abs() < 0.01, "{name} board {index}: SVG height");
            let pdf = vectorcraft_engine::cmd::fileio::encode(doc, "pdf", &params).unwrap();
            let info = vectorcraft_pdf::info(&pdf, None).unwrap();
            assert_eq!(info.pages.len(), 1, "{name} board {index}: PDF page count");
            assert!((info.pages[0].width - board.rect.width()).abs() < 0.01, "{name} board {index}: PDF width");
            assert!((info.pages[0].height - board.rect.height()).abs() < 0.01, "{name} board {index}: PDF height");
            assert_eq!(vectorcraft_pdf::import(&pdf).unwrap().artboards.len(), 1, "{name} board {index}: PDF reimport");
            let psd = vectorcraft_engine::cmd::fileio::encode(doc, "psd", &params).unwrap();
            let png = vectorcraft_engine::cmd::fileio::encode(doc, "png", &params).unwrap();
            let png = image::load_from_memory(&png).unwrap().to_rgba8();
            let (merged, has_alpha) = psd_merged(&psd);
            assert_eq!(merged.dimensions(), png.dimensions(), "{name} board {index}: PSD image dimensions");
            // PSD merged RGB is stored on a white matte for readers that ignore alpha;
            // the separate alpha plane, when present, keeps the original coverage.
            for (pixel, (p, q)) in png.pixels().zip(merged.pixels()).enumerate() {
                if has_alpha {
                    assert_eq!(p[3], q[3], "{name} board {index} pixel {pixel}: PSD alpha");
                }
                let alpha = u32::from(p[3]);
                for channel in 0..3 {
                    let expected = ((u32::from(p[channel]) * alpha + 255 * (255 - alpha) + 127) / 255) as u8;
                    assert!(
                        q[channel].abs_diff(expected) <= 1,
                        "{name} board {index} pixel {pixel} channel {channel}: PSD white-matted RGB {} differs from PNG on white {expected}",
                        q[channel]
                    );
                }
            }
            eprintln!("{name} board {index}: native/SVG/PDF/PSD verified (import warnings: {:?})", loaded.warnings);
        }
    }
}

/// Minimal test-only decoder written from the public PSD file-format layout:
/// https://www.adobe.com/devnet-apps/photoshop/fileformatashtml/
/// It independently reads the merged RGB planes and PackBits rows, skipping metadata/layers.
fn psd_merged(bytes: &[u8]) -> (image::RgbaImage, bool) {
    let be16 = |at: usize| u16::from_be_bytes(bytes[at..at + 2].try_into().unwrap());
    let be32 = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
    assert_eq!(&bytes[..4], b"8BPS");
    assert_eq!(be16(4), 1);
    assert_eq!(&bytes[6..12], &[0; 6]);
    let channels = usize::from(be16(12));
    assert!((3..=4).contains(&channels));
    let (height, width) = (be32(14), be32(18));
    assert_eq!(be16(22), 8);
    assert_eq!(be16(24), 3);
    let mut at = 26;
    for _ in 0..3 {
        at += 4 + be32(at);
    }
    let compression = be16(at);
    at += 2;
    let mut planes = vec![vec![0; width * height]; channels];
    match compression {
        0 => {
            for plane in &mut planes {
                plane.copy_from_slice(&bytes[at..at + width * height]);
                at += width * height;
            }
        }
        1 => {
            let row_lengths: Vec<_> = (0..channels * height).map(|i| usize::from(be16(at + i * 2))).collect();
            at += channels * height * 2;
            for (channel, plane) in planes.iter_mut().enumerate() {
                for row in 0..height {
                    let end = at + row_lengths[channel * height + row];
                    let mut decoded = Vec::new();
                    while at < end {
                        let control = bytes[at] as i8;
                        at += 1;
                        match control {
                            0..=127 => {
                                let n = control as usize + 1;
                                assert!(at + n <= end);
                                decoded.extend_from_slice(&bytes[at..at + n]);
                                at += n;
                            }
                            -127..=-1 => {
                                assert!(at < end);
                                decoded.extend(std::iter::repeat_n(bytes[at], (1i16 - i16::from(control)) as usize));
                                at += 1;
                            }
                            -128 => {}
                        }
                    }
                    assert_eq!(decoded.len(), width);
                    plane[row * width..(row + 1) * width].copy_from_slice(&decoded);
                }
            }
        }
        _ => panic!("unexpected PSD compression {compression}"),
    }
    assert_eq!(at, bytes.len(), "PSD merged planes must consume the complete image section");
    let image = image::RgbaImage::from_fn(width as u32, height as u32, |x, y| {
        let i = y as usize * width + x as usize;
        image::Rgba([planes[0][i], planes[1][i], planes[2][i], planes.get(3).map_or(255, |p| p[i])])
    });
    (image, channels == 4)
}
