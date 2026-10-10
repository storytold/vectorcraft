use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Node, NodeKind, TextObject};
use vectorcraft_geom::{Point, shapes};

use super::png::{PngOptions, encode, pixels_per_metre};
use super::*;

/// A chunk's data by type (first match).
fn chunk_data<'a>(file: &'a [u8], ty: &[u8; 4]) -> Option<&'a [u8]> {
    let mut i = 8;
    while i + 8 <= file.len() {
        let len = u32::from_be_bytes(file[i..i + 4].try_into().unwrap()) as usize;
        if &file[i + 4..i + 8] == ty {
            return Some(&file[i + 8..i + 8 + len]);
        }
        i += 12 + len;
    }
    None
}

/// Deterministic straight-alpha noise with some fully transparent and some opaque pixels.
fn noise(w: u32, h: u32, opaque: bool) -> Vec<u8> {
    let mut x: u32 = 0x1234_5678;
    (0..w * h * 4)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            if i % 4 == 3 && opaque { 255 } else { (x >> 24) as u8 }
        })
        .collect()
}

fn decode(png: &[u8]) -> Vec<u8> {
    image::load_from_memory_with_format(png, image::ImageFormat::Png).expect("decodes").to_rgba8().into_raw()
}

#[test]
fn png_round_trips_plain_and_interlaced_at_odd_sizes() {
    for (w, h) in [(1, 1), (3, 5), (9, 9), (17, 2), (8, 8), (33, 31)] {
        for opaque in [false, true] {
            let px = noise(w, h, opaque);
            for interlaced in [false, true] {
                let file = encode(&px, w, h, &PngOptions { ppi: None, interlaced }).unwrap();
                assert_eq!(&file[..8], b"\x89PNG\r\n\x1a\n");
                assert_eq!(file[28], u8::from(interlaced), "IHDR interlace method");
                assert_eq!(file[25], if opaque { 2 } else { 6 }, "opaque images are written as RGB");
                assert_eq!(decode(&file), px, "{w}×{h} opaque={opaque} interlaced={interlaced}");
                assert!(chunk_data(&file, b"pHYs").is_none());
            }
        }
    }
}

#[test]
fn phys_holds_pixels_per_metre() {
    let file = encode(&noise(2, 2, true), 2, 2, &PngOptions { ppi: Some(300.0), interlaced: false }).unwrap();
    let phys = chunk_data(&file, b"pHYs").expect("pHYs");
    let ppm = (300.0f64 / 0.0254).round() as u32;
    assert_eq!(ppm, pixels_per_metre(300.0));
    assert_eq!(phys, [ppm.to_be_bytes().as_slice(), ppm.to_be_bytes().as_slice(), &[1]].concat());
}

#[test]
fn png_refuses_a_buffer_of_the_wrong_size() {
    assert!(encode(&[0; 12], 2, 2, &PngOptions::default()).is_err());
    assert!(encode(&[], 0, 0, &PngOptions::default()).is_err());
}

/// A 40×30 pt document with a black circle off the pixel grid.
fn circle_doc() -> Document {
    let mut d = Document::new(40.0, 30.0);
    let id = d.alloc_id();
    let n = Node::path(id, shapes::ellipse(Rect::new(5.3, 4.7, 31.1, 26.2)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn export(d: &Document, format: RasterFormat, o: &RasterExportOptions) -> image::RgbaImage {
    let bytes = Renderer::new().export_region(d, d.artboards[0].rect, format, o).unwrap();
    image::load_from_memory(&bytes).unwrap().to_rgba8()
}

#[test]
fn resolution_sets_the_pixel_size_and_is_stored() {
    let d = circle_doc();
    assert_eq!(export(&d, RasterFormat::Png, &RasterExportOptions::default()).dimensions(), (40, 30));
    let o = RasterExportOptions { ppi: 144.0, ..Default::default() };
    assert_eq!(export(&d, RasterFormat::Png, &o).dimensions(), (80, 60), "144 ppi doubles the size");
    let png = Renderer::new().export_region(&d, d.artboards[0].rect, RasterFormat::Png, &o).unwrap();
    assert_eq!(&chunk_data(&png, b"pHYs").unwrap()[..4], pixels_per_metre(144.0).to_be_bytes());
    let jpg = Renderer::new().export_region(&d, d.artboards[0].rect, RasterFormat::Jpeg, &o).unwrap();
    // JFIF APP0: units 1 (dots per inch), then the x and y density.
    let app0 = jpg.windows(5).position(|w| w == b"JFIF\0").unwrap();
    assert_eq!(&jpg[app0 + 7..app0 + 12], [1, 0, 144, 0, 144]);
}

#[test]
fn backgrounds_are_opaque_or_transparent() {
    let d = circle_doc();
    let transparent = export(&d, RasterFormat::Png, &RasterExportOptions::default());
    assert_eq!(transparent.get_pixel(0, 0)[3], 0);
    for (bg, want) in [([255, 255, 255], [255, 255, 255, 255]), ([0, 0, 0], [0, 0, 0, 255]), ([30, 120, 200], [30, 120, 200, 255])] {
        let img = export(&d, RasterFormat::Png, &RasterExportOptions { background: Some(bg), ..Default::default() });
        assert!(img.pixels().all(|p| p[3] == 255), "no alpha below 255 on {bg:?}");
        assert_eq!(img.get_pixel(0, 0).0, want);
    }
    let webp = export(&d, RasterFormat::WebP, &RasterExportOptions { background: Some([0, 0, 0]), ..Default::default() });
    assert_eq!(webp.get_pixel(0, 0).0, [0, 0, 0, 255]);
}

/// #787: translucent art over opaque art (or a background) stays exactly opaque, its anti-aliased
/// edges too, so the file is written as RGB.
#[test]
fn translucent_edges_over_opaque_art_stay_opaque() {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let under = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    let mut over =
        Node::path(d.alloc_id(), shapes::ellipse(Rect::new(10.0, 10.0, 90.0, 90.0)), Appearance::basic(Paint::solid(Color::WHITE), Paint::None, 0.0));
    over.opacity = 0.5;
    let only_over = {
        let mut d = d.clone();
        d.insert(Some(l), 0, over.clone()).unwrap();
        d
    };
    d.insert(Some(l), 0, under).unwrap();
    d.insert(Some(l), 1, over).unwrap();
    // Over a black background at 72 ppi, and over an opaque rectangle at a scale that puts the
    // edges on fractions of a pixel.
    for (doc, o) in [
        (&only_over, RasterExportOptions { background: Some([0, 0, 0]), ..Default::default() }),
        (&d, RasterExportOptions { ppi: 72.0 * 0.3515625, ..Default::default() }),
    ] {
        let png = Renderer::new().export_region(doc, doc.artboards[0].rect, RasterFormat::Png, &o).unwrap();
        let img = image::load_from_memory(&png).unwrap().to_rgba8();
        let below: Vec<_> = img.pixels().filter(|p| p[3] < 255).collect();
        assert!(below.is_empty(), "ppi {}: {} pixels below alpha 255, e.g. {:?}", o.ppi, below.len(), below.first());
        assert_eq!(png[25], 2, "IHDR colour type: RGB");
    }
    // The inside is half white over black.
    let img = export(&only_over, RasterFormat::Png, &RasterExportOptions { background: Some([0, 0, 0]), ..Default::default() });
    assert!(img.get_pixel(50, 50).0[..3].iter().all(|c| (127..=128).contains(c)), "{:?}", img.get_pixel(50, 50));
}

#[test]
fn interlaced_export_decodes_to_the_same_pixels() {
    let d = circle_doc();
    let plain = Renderer::new().export_region(&d, d.artboards[0].rect, RasterFormat::Png, &RasterExportOptions::default()).unwrap();
    let o = RasterExportOptions { interlaced: true, ..Default::default() };
    let laced = Renderer::new().export_region(&d, d.artboards[0].rect, RasterFormat::Png, &o).unwrap();
    assert_eq!((plain[28], laced[28]), (0, 1));
    assert_eq!(decode(&plain), decode(&laced));
}

#[test]
fn anti_alias_none_paints_whole_pixels() {
    let d = circle_doc();
    let soft = export(&d, RasterFormat::Png, &RasterExportOptions::default());
    assert!(soft.pixels().any(|p| p[3] != 0 && p[3] != 255), "art anti-aliasing has partial edge pixels");
    let hard = export(&d, RasterFormat::Png, &RasterExportOptions { anti_alias: AntiAlias::None, ..Default::default() });
    assert!(hard.pixels().all(|p| p[3] == 0 || p[3] == 255), "no anti-aliasing: every alpha is 0 or 255");
    let covered = |img: &image::RgbaImage| img.pixels().filter(|p| p[3] >= 128).count() as i64;
    assert!((covered(&hard) - covered(&soft)).abs() <= 8, "the same shape, only harder edges");
}

#[test]
fn type_anti_aliasing_snaps_text_to_pixels() {
    let mut d = Document::new(80.0, 30.0);
    let id = d.alloc_id();
    let t = TextObject::point(Point::new(4.37, 18.61), "Hill", CharStyle { size: 12.0, ..CharStyle::default() });
    let n = Node::new(id, NodeKind::Text(Box::new(t)));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let art = export(&d, RasterFormat::Png, &RasterExportOptions::default());
    let typ = export(&d, RasterFormat::Png, &RasterExportOptions { anti_alias: AntiAlias::Type, ..Default::default() });
    assert!(art.pixels().any(|p| p[3] > 0), "the text is drawn");
    assert_ne!(art, typ, "type moves to whole pixels");
    // Snapping moves glyphs by under a pixel: the ink stays where it was.
    let ink = |img: &image::RgbaImage| img.pixels().map(|p| p[3] as u64).sum::<u64>() as f64;
    assert!((ink(&typ) / ink(&art) - 1.0).abs() < 0.1);
}

#[test]
fn anti_alias_none_is_the_same_on_every_thread_count() {
    // Glows are filtered offscreen when the renderer is multithreaded: their clip must be hard too.
    let mut d = Document::new(80.0, 80.0);
    let id = d.alloc_id();
    let mut n = Node::path(id, shapes::rectangle(Rect::new(20.5, 20.3, 60.4, 59.6)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let glow = serde_json::json!({"color": "#ffffff", "mode": "normal", "opacity": 100, "blur": 4});
    n.appearance.effects = vec![vectorcraft_doc::Effect { id: "stylize.innerGlow".into(), params: glow, visible: true }];
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let opts = RenderOptions { anti_alias: AntiAlias::None, ..Default::default() };
    let render = |threads: u16| {
        let mut r = Renderer::new();
        r.threads = threads;
        r.render_region_with(&d, d.artboards[0].rect, 1.0, &opts)
    };
    let (st, mt) = (render(0), render(3));
    for img in [&st, &mt] {
        assert!(img.to_straight().as_chunks::<4>().0.iter().all(|p| p[3] == 0 || p[3] == 255), "hard edges");
    }
    let worst = st.pixels.iter().zip(&mt.pixels).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(worst <= 4, "max channel difference {worst}");
}

#[test]
fn art_bounds_cover_visible_art_only() {
    let mut d = circle_doc();
    let l = d.layers[0].id;
    let hidden = {
        let id = d.alloc_id();
        let mut n =
            Node::path(id, shapes::rectangle(Rect::new(100.0, 100.0, 120.0, 120.0)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
        n.visible = false;
        n
    };
    d.insert(Some(l), 1, hidden).unwrap();
    let b = art_bounds(&d).unwrap();
    assert!((b.x0 - 5.3).abs() < 1e-6 && (b.x1 - 31.1).abs() < 1e-6 && (b.y1 - 26.2).abs() < 1e-6, "{b:?}");
    let empty = Document::new(10.0, 10.0);
    assert_eq!(art_bounds(&empty), None);
}

/// A black path round `r` in the first layer of `d`, made a guide when `guide`.
fn add_rect(d: &mut Document, r: Rect, guide: bool) {
    let id = d.alloc_id();
    let mut n = Node::path(id, shapes::rectangle(r), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    if let NodeKind::Path { guide: g, .. } = &mut n.kind {
        *g = guide;
    }
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
}

#[test]
fn guides_dont_count_in_the_arts_bounds() {
    let mut d = circle_doc();
    add_rect(&mut d, Rect::new(0.0, 15.0, 400.0, 15.0), true);
    let b = art_bounds(&d).unwrap();
    assert!((b.x0 - 5.3).abs() < 1e-6 && (b.x1 - 31.1).abs() < 1e-6, "the guide doesn't count: {b:?}");
}

#[test]
fn art_bounds_end_at_a_clipping_layers_clip() {
    let mut d = Document::new(40.0, 30.0);
    // The layer clips its art to its first object (added last): what lies beyond isn't drawn.
    add_rect(&mut d, Rect::new(-100.0, -100.0, 200.0, 200.0), false);
    add_rect(&mut d, Rect::new(2.0, 3.0, 20.0, 10.0), false);
    let layer = std::sync::Arc::make_mut(&mut d.layers[0]);
    if let NodeKind::Layer { clip, .. } = &mut layer.kind {
        *clip = true;
    }
    assert_eq!(art_bounds(&d), Some(Rect::new(2.0, 3.0, 20.0, 10.0)));
}
