//! Colorized gray and bilevel image export, checked as pixels as well as markup.

use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::{Document, ImageBlob, ImageObject, LinkInfo, Node, NodeKind};
use vectorcraft_geom::Affine;
use vectorcraft_svg::{ExportOptions, ImageMode, Profile, base64_encode, export_full};

fn bilevel() -> Vec<u8> {
    let entries: [(u16, u16, u32); 9] =
        [(256, 3, 2), (257, 3, 1), (258, 3, 1), (259, 3, 1), (262, 3, 1), (273, 4, 122), (277, 3, 1), (278, 3, 1), (279, 4, 1)];
    let mut out = b"II*\0".to_vec();
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&9u16.to_le_bytes());
    for (tag, kind, value) in entries {
        out.extend_from_slice(&tag.to_le_bytes());
        out.extend_from_slice(&kind.to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.extend_from_slice(&0u32.to_le_bytes());
    out.push(0b0100_0000);
    out
}

fn png(img: image::DynamicImage) -> Vec<u8> {
    let mut out = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

fn document(bytes: Vec<u8>, paint: Option<Paint>) -> Document {
    let mut d = Document::new(80.0, 40.0);
    d.swatches.push(Swatch { name: "Ink".into(), paint: Paint::solid(Color::rgb8(255, 0, 0)), global: true, spot: true });
    d.images.insert("source".into(), ImageBlob::new("image/tiff", bytes));
    let im = ImageObject {
        key: "source".into(),
        width: 2,
        height: 1,
        xf: Affine::translate((10.0, 10.0)) * Affine::scale(20.0),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    n.extras_mut().pixelated = true;
    n.extras_mut().colorize = paint.map(Box::new);
    d.insert(d.default_layer(), 0, n).unwrap();
    d
}

fn rendered(svg: &str) -> ([u8; 4], [u8; 4]) {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).unwrap();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(80, 40).unwrap();
    pixmap.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    let sample = |x| {
        let p = pixmap.pixel(x, 15).unwrap().demultiply();
        [p.red(), p.green(), p.blue(), p.alpha()]
    };
    (sample(15), sample(45))
}

fn samples(svg: &str, points: &[(u32, u32)]) -> Vec<[u8; 4]> {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).unwrap();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(80, 40).unwrap();
    pixmap.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pixmap.as_mut());
    points
        .iter()
        .map(|&(x, y)| {
            let p = pixmap.pixel(x, y).unwrap().demultiply();
            [p.red(), p.green(), p.blue(), p.alpha()]
        })
        .collect()
}

#[test]
fn repeated_colorized_source_shares_coverage_but_keeps_instance_paint() {
    let mut d = document(bilevel(), Some(Paint::solid(Color::rgb8(255, 0, 0))));
    let first_id = d.layers[0].children().unwrap()[0].id;
    if let NodeKind::Image(im) = &mut d.node_mut(first_id).unwrap().kind {
        im.xf = Affine::translate((10.0, 10.0)) * Affine::scale(10.0);
    }
    for (i, color) in [(1, Color::rgb8(0, 0, 255)), (2, Color::rgb8(255, 0, 0))] {
        let mut next = (*d.layers[0].children().unwrap()[0]).clone();
        next.id = d.alloc_id();
        next.extras_mut().colorize = Some(Box::new(Paint::solid(color)));
        if let NodeKind::Image(im) = &mut next.kind {
            im.xf = Affine::translate((10.0 + f64::from(i) * 25.0, 10.0)) * Affine::scale(10.0);
        }
        d.insert(d.default_layer(), i as usize, next).unwrap();
    }
    for profile in [Profile::Svg11, Profile::Tiny12] {
        let svg = export_full(&d, &ExportOptions { profile, artboard: Some(0), ..Default::default() }, None).svg;
        let png_count = svg.matches("data:image/png;base64,").count();
        assert_eq!(png_count, if profile == Profile::Svg11 { 1 } else { 2 }, "{svg}");
        assert_eq!(svg.matches("<use").count(), if profile == Profile::Svg11 { 3 } else { 2 }, "{svg}");
        let actual = samples(&svg, &[(15, 15), (25, 15), (40, 15), (50, 15), (65, 15), (75, 15)]);
        let expected: [[u8; 4]; 6] = [[255, 0, 0, 255], [255; 4], [0, 0, 255, 255], [255; 4], [255, 0, 0, 255], [255; 4]];
        for (got, want) in actual.iter().zip(expected) {
            assert!(got.iter().zip(want).all(|(&a, b)| i16::from(a).abs_diff(i16::from(b)) <= 25), "{actual:?}\n{svg}");
        }
    }
}

#[test]
fn colorize_mask_region_follows_an_image_far_from_the_origin() {
    let mut d = document(bilevel(), Some(Paint::solid(Color::rgb8(255, 0, 0))));
    let id = d.layers[0].children().unwrap()[0].id;
    if let NodeKind::Image(im) = &mut d.node_mut(id).unwrap().kind {
        im.xf = Affine::translate((200_010.0, 10.0)) * Affine::scale(20.0);
    }
    let svg = export_full(&d, &ExportOptions::default(), None).svg;
    let mask = svg.lines().find(|l| l.contains("<mask id=\"colorize-mask")).unwrap();
    assert!(mask.contains("x=\"200009\" y=\"9\" width=\"42\" height=\"22\""), "{mask}");
}

#[test]
fn bilevel_process_and_spot_use_coverage_in_embed_link_and_tiny() {
    let paints = [Paint::solid(Color::rgb8(255, 0, 0)), Paint::Solid { color: Color::rgb8(255, 0, 0), swatch: Some("Ink".into()), tint: 1.0 }];
    for paint in paints {
        let d = document(bilevel(), Some(paint));
        for profile in [Profile::Svg11, Profile::Tiny12] {
            let opts = ExportOptions { profile, artboard: Some(0), ..Default::default() };
            let embedded = export_full(&d, &opts, None);
            assert_eq!(rendered(&embedded.svg), ([255, 0, 0, 255], [255, 255, 255, 255]), "{}", embedded.svg);
            assert!(!embedded.svg.contains("colorize-image"), "single images stay inline");
            assert_eq!(embedded.svg.contains("<mask id=\"colorize-mask"), profile == Profile::Svg11);
            assert!(embedded.svg.contains("image-rendering=\"pixelated\""));
            let linked = export_full(&d, &ExportOptions { images: ImageMode::Link, ..opts }, None);
            assert_eq!(linked.linked.len(), 1);
            let file = &linked.linked[0];
            assert!(linked.svg.contains(&file.name));
            let rgba = image::load_from_memory(&file.bytes).unwrap().to_rgba8();
            assert_eq!((rgba.get_pixel(0, 0).0[3], rgba.get_pixel(1, 0).0[3]), (255, 0));
            let inlined = linked.svg.replace(&file.name, &format!("data:image/png;base64,{}", base64_encode(&file.bytes)));
            assert_eq!(rendered(&inlined), rendered(&embedded.svg));
        }
    }
}

#[test]
fn gray_alpha_and_two_spot_tints_keep_distinct_colors() {
    let bytes = png(image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(2, 1, vec![0, 128, 128, 255]).unwrap()));
    let mut d = document(bytes, Some(Paint::Solid { color: Color::rgb8(255, 0, 0), swatch: Some("Ink".into()), tint: 1.0 }));
    let mut second = (*d.layers[0].children().unwrap()[0]).clone();
    second.id = d.alloc_id();
    second.extras_mut().colorize = Some(Box::new(Paint::Solid { color: Color::rgb8(255, 128, 128), swatch: Some("Ink".into()), tint: 0.5 }));
    if let NodeKind::Image(im) = &mut second.kind {
        im.xf = Affine::translate((10.0, 25.0)) * Affine::scale(10.0);
    }
    d.insert(d.default_layer(), 1, second).unwrap();
    let svg = export_full(&d, &ExportOptions { artboard: Some(0), ..Default::default() }, None).svg;
    assert_eq!(svg.matches("<mask id=\"colorize-mask").count(), 2);
    assert!(svg.contains("#ff0000") && svg.contains("#ff8080"), "{svg}");
    let linked = export_full(&d, &ExportOptions { images: ImageMode::Link, artboard: Some(0), ..Default::default() }, None);
    assert_eq!(linked.linked.len(), 1, "both tints use one coverage PNG");
    assert_eq!(linked.svg.matches(&linked.linked[0].name).count(), 2);
    let (a, b) = rendered(&svg);
    assert!((a[1] as i16 - 127).abs() <= 2 && (b[1] as i16 - 128).abs() <= 2, "{a:?} {b:?}");
}

#[test]
fn gray_midtone_uses_process_cmyk_display_color() {
    let color = Color::cmyk(0.2, 0.7, 0.1, 0.1);
    let bytes = png(image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(2, 1, vec![128, 255]).unwrap()));
    let d = document(bytes, Some(Paint::solid(color)));
    let out = export_full(&d, &ExportOptions { artboard: Some(0), ..Default::default() }, None);
    let (actual, white) = rendered(&out.svg);
    let rgb = color.to_rgb();
    let expected = rgb.map(|v| (255.0 * (1.0 - 127.0 / 255.0 + v.clamp(0.0, 1.0) * 127.0 / 255.0)).round() as u8);
    for i in 0..3 {
        assert!((i16::from(actual[i]) - i16::from(expected[i])).abs() <= 2, "{actual:?}, expected {expected:?}");
    }
    assert_eq!(white, [255; 4]);
    assert!(out.warnings.is_empty());
}

#[test]
fn uncolorized_and_color_image_controls_keep_original_paths() {
    let gray = document(bilevel(), None);
    let svg = export_full(&gray, &ExportOptions { artboard: Some(0), ..Default::default() }, None).svg;
    assert!(!svg.contains("colorize-mask") && svg.contains("<image"));
    let patterned = document(bilevel(), Some(Paint::Pattern { pattern: "missing".into(), xf: Affine::IDENTITY }));
    assert_eq!(export_full(&patterned, &ExportOptions { artboard: Some(0), ..Default::default() }, None).svg, svg);
    let color = png(image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(2, 1, vec![0, 0, 255, 0, 255, 0]).unwrap()));
    let mut d = document(color, None);
    d.images.get_mut("source").unwrap().mime = "image/png".into();
    let before = export_full(&d, &ExportOptions { artboard: Some(0), ..Default::default() }, None).svg;
    let id = d.layers[0].children().unwrap()[0].id;
    d.node_mut(id).unwrap().extras_mut().colorize = Some(Box::new(Paint::solid(Color::rgb8(255, 0, 0))));
    assert_eq!(export_full(&d, &ExportOptions { artboard: Some(0), ..Default::default() }, None).svg, before);
    // Pixels that aren't grayscale (an embedded SVG, unknown bytes) keep the uncolorized path.
    let svg_source = br#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="1"><rect width="2" height="1" fill="blue"/></svg>"#.to_vec();
    for (bytes, mime) in [(svg_source, "image/svg+xml"), (vec![1, 2, 3], "image/tiff")] {
        let mut d = document(bytes, None);
        d.images.get_mut("source").unwrap().mime = mime.into();
        let before = export_full(&d, &ExportOptions { artboard: Some(0), ..Default::default() }, None);
        let id = d.layers[0].children().unwrap()[0].id;
        d.node_mut(id).unwrap().extras_mut().colorize = Some(Box::new(Paint::solid(Color::rgb8(255, 0, 0))));
        let after = export_full(&d, &ExportOptions { artboard: Some(0), ..Default::default() }, None);
        assert!(after.svg.contains("<image"), "{mime}");
        assert_eq!((after.svg, after.warnings), (before.svg, before.warnings), "{mime}");
    }

    let mut truncated = bilevel();
    truncated.pop(); // Keep the TIFF header, but remove its pixel data.
    let damaged = document(truncated, Some(Paint::solid(Color::rgb8(255, 0, 0))));
    assert!(damaged.images["source"].gray_source().is_some());
    let out = export_full(&damaged, &ExportOptions { artboard: Some(0), ..Default::default() }, None);
    assert!(!out.svg.contains("<image"));
    assert_eq!(out.warnings, ["an image couldn't be decoded and is missing from the picture"]);

    let mut missing = document(bilevel(), Some(Paint::solid(Color::rgb8(255, 0, 0))));
    missing.images.remove("source");
    let id = missing.layers[0].children().unwrap()[0].id;
    if let NodeKind::Image(im) = &mut missing.node_mut(id).unwrap().kind {
        im.link = Some(LinkInfo::new("/original/gray.tiff"));
    }
    let out = export_full(&missing, &ExportOptions { artboard: Some(0), images: ImageMode::Link, ..Default::default() }, None);
    assert!(!out.svg.contains("/original/gray.tiff") && !out.svg.contains("<image"));
    assert!(out.linked.is_empty());
    assert_eq!(out.warnings, ["an image couldn't be decoded and is missing from the picture"]);

    missing.images.insert("source".into(), ImageBlob::new("image/tiff", vec![]));
    let out = export_full(&missing, &ExportOptions { artboard: Some(0), ..Default::default() }, None);
    assert!(!out.svg.contains("/original/gray.tiff") && !out.svg.contains("<image"));
    assert_eq!(out.warnings, ["an image couldn't be decoded and is missing from the picture"]);
}
