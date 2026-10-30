//! Local, original CorelDRAW-authored oracles. No proprietary artwork, fonts or profiles are
//! committed. Set CDR_FIXTURE_DIR as described in docs/cdr-import.md to run these checks.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::Path;

use vectorcraft_engine::cmd::fileio;
use vectorcraft_render::Renderer;

/// Give corresponding paths distinct RGB fills to isolate geometry and stacking
/// from profile-dependent paint. Compound members inherit their parent's paint.
fn geometry_colours(nodes: &mut [std::sync::Arc<vectorcraft_doc::Node>], index: &mut u32) {
    for node in nodes {
        let node = std::sync::Arc::make_mut(node);
        node.opacity = 1.0;
        let compound = matches!(node.kind, vectorcraft_doc::NodeKind::Compound { .. });
        if node.path_data().is_some() || compound {
            let n = *index;
            *index += 1;
            let color = vectorcraft_color::Color::rgb8((30 + n * 53 % 200) as u8, (30 + n * 97 % 200) as u8, (30 + n * 131 % 200) as u8);
            node.appearance = vectorcraft_doc::Appearance {
                items: vec![vectorcraft_doc::AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(vectorcraft_color::Paint::solid(color)))],
                ..Default::default()
            }
            .into();
        }
        if !compound && let Some(children) = node.children_mut() {
            geometry_colours(children, index);
        }
    }
}

#[test]
/// Compare every page of optional original CorelDRAW fixtures with independently exported SVGs.
fn original_corel_files_match_their_independent_svg_exports() {
    let Ok(folder) = std::env::var("CDR_FIXTURE_DIR") else { return };
    let folder = Path::new(&folder);
    let mut cases = Vec::new();
    for stem in ["rectangle", "geometry"] {
        for version in [15, 16, 27] {
            cases.push((stem, version, if stem == "rectangle" { 1 } else { 2 }));
        }
    }
    for stem in ["gradient", "group-transform"] {
        if folder.join(format!("{stem}-v27.cdr")).exists() {
            cases.push((stem, 27, 1));
        }
    }
    for version in [15, 16, 27] {
        for stem in ["bitmap", "powerclip", "corners", "corner-types", "polygons", "fill-rules", "bspline"] {
            if folder.join(format!("{stem}-v{version}.cdr")).exists() {
                cases.push((stem, version, 1));
            }
        }
    }
    for (stem, version, expected_pages) in cases {
        let name = format!("{stem}-v{version}.cdr");
        let bytes = std::fs::read(folder.join(&name)).unwrap();
        let loaded = fileio::load(&name, &bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(loaded.format.id, "cdr");
        assert!(!loaded.preview_only);
        if stem == "bitmap" {
            assert_eq!(loaded.doc.images.len(), 2, "both embedded artwork images must be retained");
        } else {
            assert!(loaded.doc.images.is_empty());
        }
        assert_eq!(loaded.doc.artboards.len(), expected_pages, "{name}");
        for (page, ab) in loaded.doc.artboards.iter().enumerate() {
            let oracle_name = format!("{stem}-page{}.svg", page + 1);
            let oracle = std::fs::read_to_string(folder.join(&oracle_name)).unwrap();
            let reference = vectorcraft_svg::import(&oracle).unwrap();
            let frame = reference.artboards[0].rect;
            assert!((ab.rect.width() - frame.width()).abs() < 0.01, "{name} page width");
            assert!((ab.rect.height() - frame.height()).abs() < 0.01, "{name} page height");
            let actual = Renderer::new().render_region(&loaded.doc, ab.rect, 1.0, stem != "bitmap");
            let expected = Renderer::new().render_region(&reference, frame, 1.0, stem != "bitmap");
            assert_eq!((actual.width, actual.height), (expected.width, expected.height));
            let mean = actual.pixels.iter().zip(&expected.pixels).map(|(a, b)| a.abs_diff(*b) as f64).sum::<f64>() / actual.pixels.len() as f64;
            eprintln!("{name} page {}: mean byte difference {mean:.6}; warnings={:?}", page + 1, loaded.warnings);
            if let Ok(output) = std::env::var("CDR_RENDER_DIR") {
                std::fs::create_dir_all(&output).unwrap();
                std::fs::write(Path::new(&output).join(format!("{stem}-v{version}-page{}.png", page + 1)), actual.to_png().unwrap()).unwrap();
                std::fs::write(Path::new(&output).join(format!("{stem}-reference-page{}.png", page + 1)), expected.to_png().unwrap()).unwrap();
            }
            if stem == "bitmap" {
                // Corel applies its CMYK profile when exporting PNG. Import keeps continuous
                // alpha and numeric profile-free colour, so compare geometry/alpha separately.
                let alpha_mean = actual
                    .pixels
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .zip(expected.pixels.as_chunks::<4>().0.iter())
                    .map(|(a, b)| a[3].abs_diff(b[3]) as f64)
                    .sum::<f64>()
                    / (actual.pixels.len() / 4) as f64;
                assert!(alpha_mean < 0.1, "{name}: bitmap placement/alpha differs from Corel: {alpha_mean}");
                let actual_images: Vec<_> = loaded.doc.images.values().map(|b| image::load_from_memory(&b.bytes).unwrap().to_rgba8()).collect();
                let expected_images: Vec<_> = reference.images.values().map(|b| image::load_from_memory(&b.bytes).unwrap().to_rgba8()).collect();
                let original_pixels = image::open(folder.join("bitmap-original-checker.png")).unwrap().to_rgba8();
                assert!(actual_images.contains(&original_pixels), "native RGB/alpha must preserve the original generated PNG exactly");
                assert!(
                    actual_images.iter().any(|a| {
                        expected_images.iter().any(|b| {
                            a.dimensions() == b.dimensions()
                                && a.pixels().zip(b.pixels()).all(|(a, b)| {
                                    // Corel rounds straight RGB when exporting partial alpha and
                                    // replaces invisible RGB at alpha zero. Coverage is exact;
                                    // opaque colour is exact; premultiplied partial colour is
                                    // allowed one byte of export quantization.
                                    a[3] == b[3]
                                        && (0..3).all(|c| {
                                            if a[3] == 255 {
                                                a[c] == b[c]
                                            } else {
                                                let alpha = u32::from(a[3]);
                                                let pa = (u32::from(a[c]) * alpha + 127) / 255;
                                                let pb = (u32::from(b[c]) * alpha + 127) / 255;
                                                pa.abs_diff(pb) <= 1
                                            }
                                        })
                                })
                        })
                    }),
                    "RGB bitmap pixels/alpha must match the independent SVG export within export quantization"
                );
                assert!(loaded.warnings.iter().any(|w| w.contains("CMYK bitmap")));
            } else {
                assert!(mean < 0.1, "{name} page {} differs from the CorelDRAW SVG oracle: {mean:.6}", page + 1);
            }
        }
    }
}

#[test]
/// A locally supplied zero-page drawing retains every curve and matches its separate Corel SVG.
/// The user file and derived artwork remain local; only generated fixtures are committed.
fn private_zero_page_drawing_matches_its_local_vector_oracle() {
    let Ok(folder) = std::env::var("CDR_ZERO_PAGE_FIXTURE_DIR") else { return };
    let folder = Path::new(&folder);
    let loaded = fileio::load("zero-page.cdr", &std::fs::read(folder.join("input.cdr")).unwrap()).unwrap();
    let reference = vectorcraft_svg::import(&std::fs::read_to_string(folder.join("corel-artwork.svg")).unwrap()).unwrap();
    /// Count the native editable subpaths without treating group wrappers as art objects.
    fn counts(nodes: &[std::sync::Arc<vectorcraft_doc::Node>]) -> (usize, usize) {
        let mut result = (0, 0);
        for node in nodes {
            if let Some(path) = node.path_data() {
                result.0 += 1;
                result.1 += path.subpaths.len();
            }
            if let Some(children) = node.children() {
                let child = counts(children);
                result.0 += child.0;
                result.1 += child.1;
            }
        }
        result
    }
    assert_eq!(counts(&loaded.doc.layers), (180, 424));
    assert!(loaded.doc.images.is_empty() && !loaded.preview_only);
    assert!(loaded.warnings.iter().any(|warning| warning.contains("artboard was fitted")));
    let frame = loaded.doc.artboards[0].rect;
    let expected_frame = reference.artboards[0].rect;
    assert!((frame.width() - expected_frame.width()).abs() < 0.01);
    assert!((frame.height() - expected_frame.height()).abs() < 0.01);
    let actual = Renderer::new().render_region(&loaded.doc, frame, 2.0, true);
    let expected = Renderer::new().render_region(&reference, expected_frame, 2.0, true);
    assert_eq!((actual.width, actual.height), (expected.width, expected.height));
    let mean = actual.pixels.iter().zip(&expected.pixels).map(|(a, b)| a.abs_diff(*b) as f64).sum::<f64>() / actual.pixels.len() as f64;
    // Check geometry/stacking independently as well as the original colours:
    // corresponding paths get distinct RGB paints so equal-coloured overlapping
    // objects cannot hide an internal boundary or paint-order regression.
    let mut actual_geometry = loaded.doc.clone();
    let mut expected_geometry = reference.clone();
    let (mut actual_count, mut expected_count) = (0, 0);
    geometry_colours(&mut actual_geometry.layers, &mut actual_count);
    geometry_colours(&mut expected_geometry.layers, &mut expected_count);
    assert_eq!((actual_count, expected_count), (180, 180));
    let a = Renderer::new().render_region(&actual_geometry, frame, 2.0, true);
    let b = Renderer::new().render_region(&expected_geometry, expected_frame, 2.0, true);
    let geometry_mean = a.pixels.iter().zip(&b.pixels).map(|(a, b)| a.abs_diff(*b) as f64).sum::<f64>() / a.pixels.len() as f64;
    eprintln!(
        "private zero-page CDR: 180 curves,424 subpaths; geometry mean {geometry_mean:.6}; colour mean {mean:.6}; warnings={:?}",
        loaded.warnings
    );
    if let Ok(output) = std::env::var("CDR_RENDER_DIR") {
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(Path::new(&output).join("zero-page-native.png"), actual.to_png().unwrap()).unwrap();
        std::fs::write(Path::new(&output).join("zero-page-reference.png"), expected.to_png().unwrap()).unwrap();
    }
    assert!(geometry_mean < 0.1, "native zero-page geometry/stacking differs from its Corel SVG oracle: {geometry_mean}");
    assert!(mean < 0.1, "native zero-page colours differ from this drawing's Corel SVG oracle: {mean}");
}

#[test]
/// A control-heavy drawing retains native topology and matches Corel's independently rendered alpha.
/// The incomplete SVG is not a coverage oracle; CMYK colours remain a separate measurement.
fn private_control_heavy_drawing_retains_topology_and_matches_corel_coverage() {
    let Ok(folder) = std::env::var("CDR_LARGE_CURVE_FIXTURE_DIR") else { return };
    let folder = Path::new(&folder);
    let loaded = fileio::load("large-curves.cdr", &std::fs::read(folder.join("input.cdr")).unwrap()).unwrap();
    assert!(loaded.doc.images.is_empty() && !loaded.preview_only);
    let (mut paths, mut subpaths, mut anchors, mut segments, mut point_subpaths) = (0, 0, 0, 0, 0);
    loaded.doc.walk(|node| {
        if let Some(path) = node.path_data() {
            paths += 1;
            subpaths += path.subpaths.len();
            anchors += path.anchor_count();
            segments += path.subpaths.iter().map(|subpath| subpath.segment_count()).sum::<usize>();
            point_subpaths += path.subpaths.iter().filter(|subpath| subpath.closed && subpath.anchors.len() == 1).count();
        }
    });
    assert_eq!((paths, subpaths, anchors), (1897, 4343, 468205));
    // Three closed one-anchor subpaths retain their points but have no drawable
    // segment in PathData; Corel's collection counts each as a degenerate segment.
    assert_eq!(point_subpaths, 3);
    assert_eq!(segments + point_subpaths, 468205);
    let board = loaded.doc.artboards[0].rect;
    assert!((board.width() - 210.0 * 72.0 / 25.4).abs() < 0.01);
    assert!((board.height() - 297.0 * 72.0 / 25.4).abs() < 0.01);
    let bounds = loaded.doc.layers.iter().filter_map(|layer| layer.visual_bounds()).reduce(|bounds, next| bounds.union(next)).unwrap();
    assert!((bounds.width() - 195.2517 * 72.0 / 25.4).abs() < 0.02);
    assert!((bounds.height() - 194.612 * 72.0 / 25.4).abs() < 0.02);
    // Corel's SVG truncates one 91621-coordinate curve to its first 26085
    // coordinates. This separate PNG is rendered from the original native
    // document, with the complete off-page artwork selected.
    let expected = image::open(folder.join("corel-artwork-rgba-20261022.png")).unwrap().to_rgba8();
    let scale = expected.width() as f64 / bounds.width();
    let actual = Renderer::new().render_region(&loaded.doc, bounds, scale, false);
    assert_eq!((actual.width, actual.height), expected.dimensions());
    let colour_mean = actual
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(expected.pixels())
        .map(|(a, b)| {
            (0..3)
                .map(|channel| {
                    let premultiplied = (u32::from(b[channel]) * u32::from(b[3]) + 127) / 255;
                    u32::from(a[channel]).abs_diff(premultiplied) as f64
                })
                .sum::<f64>()
        })
        .sum::<f64>()
        / (expected.pixels().len() * 3) as f64;
    let alpha_mean = actual.pixels.as_chunks::<4>().0.iter().zip(expected.pixels()).map(|(a, b)| a[3].abs_diff(b[3]) as f64).sum::<f64>()
        / expected.pixels().len() as f64;
    eprintln!(
        "private control-heavy CDR: {paths} paths,{subpaths} subpaths,{anchors} anchors,{segments} segments; Corel-native alpha mean {alpha_mean:.6}; premultiplied colour mean {colour_mean:.6}; warnings={:?}",
        loaded.warnings
    );
    if let Ok(output) = std::env::var("CDR_RENDER_DIR") {
        std::fs::create_dir_all(&output).unwrap();
        std::fs::write(Path::new(&output).join("large-curves-native-complete.png"), actual.to_png().unwrap()).unwrap();
    }
    assert!(actual.pixels.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 0));
    // This is a different rasterizer, not the shared-renderer SVG oracle used by
    // the original suite. Keep exact topology and compare half-covered masks:
    // all binary disagreement must lie within one output pixel of the other
    // renderer's boundary, rather than widening the SVG suite's 0.1-byte gate.
    let actual_mask: Vec<bool> = actual.pixels.as_chunks::<4>().0.iter().map(|pixel| pixel[3] >= 128).collect();
    let expected_mask: Vec<bool> = expected.pixels().map(|pixel| pixel[3] >= 128).collect();
    let (width, height) = (actual.width as usize, actual.height as usize);
    let mut mismatches = 0;
    let mut outside_boundary = 0;
    for y in 0..height {
        for x in 0..width {
            let at = y * width + x;
            if actual_mask[at] == expected_mask[at] {
                continue;
            }
            mismatches += 1;
            let (foreground, other) = if actual_mask[at] { (&actual_mask, &expected_mask) } else { (&expected_mask, &actual_mask) };
            assert!(foreground[at]);
            let has_neighbour = (y.saturating_sub(1)..=(y + 1).min(height - 1))
                .any(|row| (x.saturating_sub(1)..=(x + 1).min(width - 1)).any(|col| other[row * width + col]));
            if !has_neighbour {
                outside_boundary += 1;
            }
        }
    }
    eprintln!("Corel/native half-coverage disagreements: {mismatches}; outside one-pixel boundary: {outside_boundary}");
    assert_eq!(outside_boundary, 0, "native/Corel coverage differs beyond the explicit one-pixel raster boundary");
    assert!(loaded.warnings.iter().any(|warning| warning.contains("CMYK colours retain numeric values")));
}
