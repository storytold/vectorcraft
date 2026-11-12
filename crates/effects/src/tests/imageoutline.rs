//! Outline Object on an image's own fill or stroke: the outline of the image's opaque area, for
//! every pixel format with alpha, and the frame for one without.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, Document, Effect, FillLayer, ImageBlob, ImageObject, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{Affine, Rect};

use crate::*;

fn fx(id: &str) -> Effect {
    Effect { id: id.into(), params: json!({}), visible: true }
}

/// `img` encoded as `format`.
fn encoded(img: image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), format).unwrap();
    out
}

/// A 100×100 RGBA image, transparent but for an opaque disc of radius 30 in its middle.
fn disc() -> image::DynamicImage {
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(100, 100, |x, y| {
        let r = (f64::from(x) + 0.5 - 50.0).hypot(f64::from(y) + 0.5 - 50.0);
        image::Rgba([255, 0, 0, if r <= 30.0 { 255 } else { 0 }])
    }))
}

/// A document holding the encoded image `bytes` (key "img") and an image object of 100×100 pixels
/// placed at (50, 50) with the appearance items `items`, the image's own row at `contents`.
/// → (document, the image's node).
fn image_doc(bytes: Vec<u8>, items: Vec<AppearanceItem>, contents: usize) -> (Document, Node) {
    let mut d = Document::new(200.0, 200.0);
    d.images.insert("img".into(), ImageBlob::new("image/png", bytes));
    let mut n = Node::new(
        NodeId(0),
        NodeKind::Image(Box::new(ImageObject {
            key: "img".into(),
            width: 100,
            height: 100,
            xf: Affine::translate((50.0, 50.0)),
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    n.id = d.alloc_id();
    n.appearance.items = items;
    n.appearance.set_contents_at(contents);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n.clone()).unwrap();
    (d, n)
}

/// A PNG of `w × h` pixels with the color type `color` at `depth` bits, the chunks `extra` before
/// its pixels, and the filtered rows `rows` stored without compression.
fn png(w: u32, h: u32, color: u8, depth: u8, extra: &[(&[u8; 4], Vec<u8>)], rows: &[u8]) -> Vec<u8> {
    fn crc(data: &[u8]) -> u32 {
        let mut c = !0u32;
        for &b in data {
            c ^= u32::from(b);
            for _ in 0..8 {
                c = if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            }
        }
        !c
    }
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc(&body).to_be_bytes());
    };
    let mut ihdr = [w.to_be_bytes(), h.to_be_bytes()].concat();
    ihdr.extend([depth, color, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    for (kind, data) in extra {
        chunk(kind, data);
    }
    // A zlib stream of stored blocks.
    let mut z = vec![0x78, 0x01];
    let blocks: Vec<&[u8]> = if rows.is_empty() { vec![rows] } else { rows.chunks(65_535).collect() };
    for (i, block) in blocks.iter().enumerate() {
        let len = u16::try_from(block.len()).unwrap();
        z.push(u8::from(i + 1 == blocks.len()));
        z.extend(len.to_le_bytes());
        z.extend((!len).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in rows {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend(((b << 16) | a).to_be_bytes());
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    out
}

fn stroke(effects: Vec<Effect>) -> AppearanceItem {
    let mut s = StrokeLayer::new(Paint::solid(Color::BLACK), 4.0);
    s.effects = effects;
    AppearanceItem::Stroke(s)
}

/// The bounds of the paths the image's own fills and strokes paint, below it first.
fn row_bounds(d: &Document, n: &Node) -> Vec<Rect> {
    let art = reshape::reshape(Some(d), n, None).unwrap();
    let mut out = vec![];
    for c in art.children().unwrap() {
        if matches!(c.kind, NodeKind::Path { .. }) {
            out.push(c.geometric_bounds().unwrap());
        }
    }
    out
}

fn near(r: Rect, want: [f64; 4], tol: f64) -> bool {
    [r.x0 - want[0], r.y0 - want[1], r.x1 - want[2], r.y1 - want[3]].iter().all(|d| d.abs() < tol)
}

#[test]
fn outline_object_follows_the_opaque_area_of_an_image_with_alpha() {
    let gray_alpha = image::DynamicImage::ImageLumaA8(disc().to_luma_alpha8());
    let deep = image::DynamicImage::ImageRgba16(disc().to_rgba16());
    for (name, img) in [("rgba", disc()), ("gray and alpha", gray_alpha), ("16 bits a channel", deep)] {
        let (d, n) = image_doc(encoded(img, image::ImageFormat::Png), vec![stroke(vec![fx("path.outlineObject")])], 1);
        let rows = row_bounds(&d, &n);
        assert_eq!(rows.len(), 1, "{name}");
        // The disc runs from 70 to 130; the frame from 50 to 150.
        assert!(near(rows[0], [70.0, 70.0, 130.0, 130.0], 0.5), "{name}: {:?}", rows[0]);
    }
}

#[test]
fn outline_object_follows_the_frame_of_an_image_without_transparency() {
    let opaque = image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(100, 100, image::Rgb([0, 0, 255])));
    let solid = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 255, 200])));
    let formats = [
        ("jpeg", encoded(opaque.clone(), image::ImageFormat::Jpeg)),
        ("rgb png", encoded(opaque, image::ImageFormat::Png)),
        ("rgba png, every pixel more than half opaque", encoded(solid, image::ImageFormat::Png)),
    ];
    for (name, bytes) in formats {
        let (d, n) = image_doc(bytes, vec![stroke(vec![fx("path.outlineObject")])], 1);
        let rows = row_bounds(&d, &n);
        assert!(rows.len() == 1 && near(rows[0], [50.0, 50.0, 150.0, 150.0], 1e-9), "{name}: {rows:?}");
    }
    // Pixels that can't be read keep the frame too.
    let (d, n) = image_doc(b"not an image".to_vec(), vec![stroke(vec![fx("path.outlineObject")])], 1);
    assert!(near(row_bounds(&d, &n)[0], [50.0, 50.0, 150.0, 150.0], 1e-9));
}

#[test]
fn convert_to_shape_keeps_the_frame_beside_outline_object() {
    // Below the image: a fill with Convert to Shape, then a stroke with Outline Object; above it
    // a stroke with Convert to Shape before Outline Object, which starts from the frame.
    let rect = Effect { id: "convertToShape.rectangle".into(), params: json!({"relative": true, "extraW": 0, "extraH": 0}), visible: true };
    let mut fill = FillLayer::new(Paint::solid(Color::rgb(0.0, 1.0, 0.0)));
    fill.effects = vec![rect.clone()];
    let items = vec![AppearanceItem::Fill(fill), stroke(vec![fx("path.outlineObject")]), stroke(vec![rect, fx("path.outlineObject")])];
    let (d, n) = image_doc(encoded(disc(), image::ImageFormat::Png), items, 2);
    let rows = row_bounds(&d, &n);
    assert_eq!(rows.len(), 3, "{rows:?}");
    assert!(near(rows[0], [50.0, 50.0, 150.0, 150.0], 1e-9), "{rows:?}");
    assert!(near(rows[1], [70.0, 70.0, 130.0, 130.0], 0.5), "{rows:?}");
    assert!(near(rows[2], [50.0, 50.0, 150.0, 150.0], 1e-9), "{rows:?}");
    // The image keeps its place between them.
    let art = reshape::reshape(Some(&d), &n, None).unwrap();
    assert!(matches!(art.children().unwrap()[2].kind, NodeKind::Image(_)));
}

#[test]
fn a_transparent_image_gives_outline_object_nothing_to_paint() {
    let clear = image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 100])));
    let (d, n) = image_doc(encoded(clear, image::ImageFormat::Png), vec![stroke(vec![fx("path.outlineObject")])], 1);
    assert!(row_bounds(&d, &n).is_empty());
    let art = reshape::reshape(Some(&d), &n, None).unwrap();
    assert!(matches!(art.children().unwrap().as_slice(), [only] if matches!(only.kind, NodeKind::Image(_))));
}

#[test]
fn the_outline_follows_the_images_own_pixels_and_placement() {
    // Pixels kept on the image itself (art made outside a document), placed at half size.
    let (_, mut n) = image_doc(vec![], vec![stroke(vec![fx("path.outlineObject")])], 1);
    if let NodeKind::Image(im) = &mut n.kind {
        im.pixels = Some(Arc::new(ImageBlob::new("image/png", encoded(disc(), image::ImageFormat::Png))));
        im.xf = Affine::translate((10.0, 20.0)) * Affine::scale(0.5);
    }
    let art = reshape::reshape(None, &n, None).unwrap();
    let b = art.children().unwrap()[0].geometric_bounds().unwrap();
    // The disc runs from 20 to 80 pixels: half that, moved by (10, 20).
    assert!(near(b, [20.0, 30.0, 50.0, 60.0], 0.3), "{b:?}");
}

#[test]
fn an_off_center_outline_turns_and_reflects_with_a_smaller_copy_of_the_pixels() {
    // A 120×60 image whose opaque area is the rectangle from (12, 6) to (40, 24), its pixels a
    // copy at half size (as a linked image's preview is), turned a quarter and reflected.
    let half = image::DynamicImage::ImageRgba8(image::RgbaImage::from_fn(60, 30, |x, y| {
        image::Rgba([255, 0, 0, if (6..20).contains(&x) && (3..12).contains(&y) { 255 } else { 0 }])
    }));
    let (d, mut n) = image_doc(encoded(half, image::ImageFormat::Png), vec![stroke(vec![fx("path.outlineObject")])], 1);
    let xf = Affine::translate((100.0, 40.0)) * Affine::rotate(std::f64::consts::FRAC_PI_2) * Affine::scale_non_uniform(-1.0, 1.0);
    if let NodeKind::Image(im) = &mut n.kind {
        (im.width, im.height, im.xf) = (120, 60, xf);
    }
    let rows = row_bounds(&d, &n);
    let want = xf.transform_rect_bbox(Rect::new(12.0, 6.0, 40.0, 24.0));
    // The rectangle runs from (76, 0) to (94, 28); a flip in either direction, or the copy's
    // width and height swapped, would move it.
    assert!(near(want, [76.0, 0.0, 94.0, 28.0], 1e-9), "{want:?}");
    assert!(rows.len() == 1 && near(rows[0], [want.x0, want.y0, want.x1, want.y1], 0.5), "{rows:?}");
}

#[test]
fn an_image_in_a_group_exports_with_its_outline() {
    let (mut d, n) = image_doc(encoded(disc(), image::ImageFormat::Png), vec![stroke(vec![fx("path.outlineObject")])], 1);
    let l = d.layers[0].id;
    d.remove(n.id).unwrap();
    let g = d.alloc_id();
    d.insert(Some(l), 0, Node::group(g, vec![Arc::new(n)])).unwrap();
    let baked = bake_document(&d).unwrap();
    let mut paths = vec![];
    for layer in &baked.layers {
        layer.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Path { .. }) && (c.appearance.stroke_width() - 4.0).abs() < 1e-9 {
                paths.push(c.geometric_bounds().unwrap());
            }
        });
    }
    assert!(matches!(paths.as_slice(), [b] if near(*b, [70.0, 70.0, 130.0, 130.0], 0.5)), "{paths:?}");
}

/// The bounds of the paths stroked 4 pt in `d` with its effects baked, as exports write it.
fn baked_strokes(d: &Document) -> Vec<Rect> {
    let baked = bake_document(d).unwrap();
    let mut paths = vec![];
    for layer in &baked.layers {
        layer.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Path { .. }) && (c.appearance.stroke_width() - 4.0).abs() < 1e-9 {
                paths.push(c.geometric_bounds().unwrap());
            }
        });
    }
    paths
}

#[test]
fn an_image_under_a_clipping_mask_or_with_an_opacity_mask_exports_with_its_outline() {
    let disc_outline = |paths: &[Rect]| matches!(paths, [b] if near(*b, [70.0, 70.0, 130.0, 130.0], 0.5));
    // Under a clipping mask that covers the whole image.
    let (mut d, n) = image_doc(encoded(disc(), image::ImageFormat::Png), vec![stroke(vec![fx("path.outlineObject")])], 1);
    let l = d.layers[0].id;
    d.remove(n.id).unwrap();
    let clip = Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)), Default::default());
    let g = d.alloc_id();
    d.insert(Some(l), 0, Node::new(g, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(n)], clip: true })).unwrap();
    let paths = baked_strokes(&d);
    assert!(disc_outline(&paths), "{paths:?}");
    // With an opacity mask.
    let (mut d, mut n) = image_doc(encoded(disc(), image::ImageFormat::Png), vec![stroke(vec![fx("path.outlineObject")])], 1);
    let mut white = Node::path(NodeId(999), vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 200.0, 200.0)), Default::default());
    white.appearance.set_fill(Paint::solid(Color::WHITE));
    n.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(Node::group(NodeId(998), vec![Arc::new(white)]), true)));
    let id = n.id;
    *d.node_mut(id).unwrap() = n;
    let paths = baked_strokes(&d);
    assert!(disc_outline(&paths), "{paths:?}");
}

#[test]
fn outline_object_on_a_fill_and_on_an_indexed_image_follows_the_opaque_area() {
    let mut fill = FillLayer::new(Paint::solid(Color::rgb(0.0, 1.0, 0.0)));
    fill.effects = vec![fx("path.outlineObject")];
    let (d, n) = image_doc(encoded(disc(), image::ImageFormat::Png), vec![AppearanceItem::Fill(fill)], 1);
    let rows = row_bounds(&d, &n);
    assert!(matches!(rows.as_slice(), [b] if near(*b, [70.0, 70.0, 130.0, 130.0], 0.5)), "{rows:?}");
    // A palette image whose first color is transparent (a tRNS chunk).
    let mut rows = vec![];
    for y in 0..100 {
        rows.push(0);
        for x in 0..100 {
            let r = (f64::from(x) + 0.5 - 50.0).hypot(f64::from(y) + 0.5 - 50.0);
            rows.push(u8::from(r <= 30.0));
        }
    }
    let palette = png(100, 100, 3, 8, &[(b"PLTE", vec![0, 0, 0, 255, 0, 0]), (b"tRNS", vec![0, 255])], &rows);
    let (d, n) = image_doc(palette, vec![stroke(vec![fx("path.outlineObject")])], 1);
    let rows = row_bounds(&d, &n);
    assert!(matches!(rows.as_slice(), [b] if near(*b, [70.0, 70.0, 130.0, 130.0], 0.5)), "{rows:?}");
}

#[test]
fn an_image_too_large_or_too_complex_to_outline_keeps_its_frame() {
    // 72 megapixels at 8 bits a channel, and 42 at 16: refused before their pixels are read.
    for (w, h, depth) in [(9000, 8000, 8), (7000, 6000, 16)] {
        let bytes = png(w, h, 6, depth, &[], &[]);
        assert!(matches!(vectorcraft_trace::decode_alpha(&bytes), Err(vectorcraft_trace::TraceError::TooLarge { .. })), "{w}×{h}");
        let (d, n) = image_doc(bytes, vec![stroke(vec![fx("path.outlineObject")])], 1);
        assert!(matches!(row_bounds(&d, &n).as_slice(), [b] if near(*b, [50.0, 50.0, 150.0, 150.0], 1e-9)), "{w}×{h}");
    }
    // A checkerboard of 2 px squares, whose outline would need more than 100,000 anchor points.
    let checker = image::RgbaImage::from_fn(640, 640, |x, y| image::Rgba([0, 0, 0, if (x / 2 + y / 2) % 2 == 0 { 255 } else { 0 }]));
    let (d, n) =
        image_doc(encoded(image::DynamicImage::ImageRgba8(checker), image::ImageFormat::Png), vec![stroke(vec![fx("path.outlineObject")])], 1);
    assert!(matches!(row_bounds(&d, &n).as_slice(), [b] if near(*b, [50.0, 50.0, 150.0, 150.0], 1e-9)));
}

#[test]
fn an_outline_is_traced_once_for_its_pixels_and_dropped_with_them() {
    let traces = || reshape::TRACES.with(std::cell::Cell::get);
    let (d, n) = image_doc(encoded(disc(), image::ImageFormat::Png), vec![stroke(vec![fx("path.outlineObject")])], 1);
    let NodeKind::Image(im) = &n.kind else { panic!("not an image") };
    let before = traces();
    let first = reshape::opaque_area(&[Some(&d)], im).unwrap();
    assert_eq!(traces(), before + 1);
    for _ in 0..3 {
        row_bounds(&d, &n);
    }
    assert_eq!(reshape::opaque_area(&[Some(&d)], im).unwrap(), first);
    assert_eq!(traces(), before + 1);
    let pixels = Arc::downgrade(&d.images.get(&im.key).unwrap().bytes);
    assert!(reshape::keeps_outline(&pixels));
    // Once the document and its pixels are gone, the next trace drops their entry.
    drop(d);
    assert!(pixels.upgrade().is_none());
    let (other, _) = image_doc(encoded(disc(), image::ImageFormat::Png), vec![], 1);
    reshape::opaque_area(&[Some(&other)], im).unwrap();
    assert_eq!(traces(), before + 2);
    assert!(!reshape::keeps_outline(&pixels));
}
