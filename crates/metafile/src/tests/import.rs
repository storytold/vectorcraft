//! Metafiles read back: VectorCraft's own EMF and WMF round-trip, hand-built records of other
//! writers play as they draw, and damaged files fail or import partly without a panic.

use vectorcraft_color::{Gradient, GradientPaint, Paint};
use vectorcraft_doc::{AppearanceItem, Dash, Document, Node, NodeKind, TextObject};
use vectorcraft_geom::{Affine, Rect, shapes};

use crate::bytes::Out;
use crate::tests::*;

/// Every object of the imported document, depth first.
fn all(d: &Document) -> Vec<Node> {
    let mut v = vec![];
    for l in &d.layers {
        for c in l.children().into_iter().flatten() {
            c.walk(&mut |n| v.push(n.clone()));
        }
    }
    v
}

fn paths(d: &Document) -> Vec<Node> {
    all(d).into_iter().filter(|n| matches!(n.kind, NodeKind::Path { clipping: false, .. })).collect()
}

fn near(a: Rect, b: Rect, tol: f64) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() <= tol)
}

fn fill_rgb(n: &Node) -> Option<[u8; 3]> {
    n.appearance.items.iter().find_map(|i| match i {
        AppearanceItem::Fill(f) => f.paint.color().map(|c| {
            let [r, g, b, _] = c.to_rgba8(1.0);
            [r, g, b]
        }),
        _ => None,
    })
}

fn sample() -> Document {
    let mut d = doc_with(200.0, 100.0, vec![]);
    add(&mut d, path(shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 60.0)), red(), blue(), 4.0));
    add(&mut d, path(shapes::ellipse(Rect::new(100.0, 20.0, 180.0, 80.0)), blue(), Paint::None, 0.0));
    d
}

#[test]
fn emf_output_round_trips() {
    let d = sample();
    let out = emf(&d);
    let back = import(&out.bytes).unwrap();
    assert_eq!(back.kind, Kind::Emf);
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    let ab = back.document.artboards[0].rect;
    assert!(near(ab, Rect::new(0.0, 0.0, 200.0, 100.0), 0.02), "{ab:?}");
    let p = paths(&back.document);
    // The rectangle's fill (its stroke is drawn after it, as another path) and the ellipse.
    let rect: Vec<&Node> = p.iter().filter(|n| fill_rgb(n) == Some([255, 0, 0])).collect();
    assert_eq!(rect.len(), 1);
    assert!(near(rect[0].geometric_bounds().unwrap(), Rect::new(10.0, 10.0, 90.0, 60.0), 0.03));
    let stroke = p.iter().find_map(|n| n.appearance.stroke().cloned()).unwrap();
    assert!((stroke.width - 4.0).abs() < 0.02, "{}", stroke.width);
    assert_eq!(stroke.paint.color().map(|c| c.to_rgba8(1.0)), Some([0, 0, 255, 255]));
    let ellipse = p.iter().find(|n| fill_rgb(n) == Some([0, 0, 255])).unwrap();
    assert!(near(ellipse.geometric_bounds().unwrap(), Rect::new(100.0, 20.0, 180.0, 80.0), 0.03));
    assert!(
        matches!(&ellipse.kind, NodeKind::Path { path, .. } if path.to_bezpath().elements().iter().any(|e| matches!(e, kurbo::PathEl::CurveTo(..))))
    );
}

#[test]
fn wmf_output_round_trips() {
    let d = sample();
    let back = import(&wmf(&d).bytes).unwrap();
    assert_eq!(back.kind, Kind::Wmf);
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert!(near(back.document.artboards[0].rect, Rect::new(0.0, 0.0, 200.0, 100.0), 0.05));
    let p = paths(&back.document);
    let rect = p.iter().find(|n| fill_rgb(n) == Some([255, 0, 0])).unwrap();
    // 1440 units an inch: within a twentieth of a point.
    assert!(near(rect.geometric_bounds().unwrap(), Rect::new(10.0, 10.0, 90.0, 60.0), 0.05));
    let stroke = p.iter().find_map(|n| n.appearance.stroke().cloned()).unwrap();
    assert!((stroke.width - 4.0).abs() < 0.05);
    let ellipse = p.iter().find(|n| fill_rgb(n) == Some([0, 0, 255])).unwrap();
    assert!(near(ellipse.geometric_bounds().unwrap(), Rect::new(100.0, 20.0, 180.0, 80.0), 0.1));
}

/// A symbol's square that a Transform effect moves 100 pt right is written where the canvas draws
/// it, in EMF and in WMF, read back: art whose own place is off the artboard is written where the
/// effect moves it onto it, and an instance that doubles the art's width places it first, so the
/// effect still moves it 100 pt (to x = 120, not 220). A stained instance keeps its stain.
#[test]
fn symbol_art_effects_are_written_where_they_draw() {
    let moved = |rect: Rect| {
        let mut art = path(shapes::rectangle(rect), red(), Paint::None, 0.0);
        art.appearance.effects.push(vectorcraft_doc::Effect {
            id: "distort.transform".into(),
            params: [("moveH", 100.0)].into_iter().collect(),
            visible: true,
        });
        art
    };
    // The art's square, the instance's transform and fill, and the square and color it draws.
    let cases = [
        (Rect::new(-90.0, 10.0, -80.0, 20.0), Affine::IDENTITY, None, Rect::new(10.0, 10.0, 20.0, 20.0), [255, 0, 0]),
        (Rect::new(10.0, 30.0, 20.0, 40.0), Affine::scale_non_uniform(2.0, 1.0), Some(blue()), Rect::new(120.0, 30.0, 140.0, 40.0), [0, 0, 255]),
    ];
    for (square, xf, stain, want, rgb) in cases {
        let mut inst = Node::new(vectorcraft_doc::NodeId(0), NodeKind::SymbolInstance { symbol: "Moved".into(), xf });
        if let Some(p) = stain {
            inst.appearance = vectorcraft_doc::Appearance::basic(p, Paint::None, 0.0).into();
        }
        let mut d = doc_with(200.0, 100.0, vec![inst]);
        d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: "Moved".into(), art: std::sync::Arc::new(moved(square)) });
        for kind in [Kind::Emf, Kind::Wmf] {
            let back = import(&export(&d, d.artboards[0].rect, kind).unwrap().bytes).unwrap();
            let drawn: Vec<(Option<Rect>, Option<[u8; 3]>)> = paths(&back.document).iter().map(|n| (n.geometric_bounds(), fill_rgb(n))).collect();
            assert_eq!(drawn.len(), 1, "{kind:?} {xf:?}: {drawn:?}");
            assert!(drawn[0].0.is_some_and(|b| near(b, want, 0.1)), "{kind:?} {xf:?}: {drawn:?}");
            assert_eq!(drawn[0].1, Some(rgb), "{kind:?} {xf:?}");
        }
    }
}

#[test]
fn freeform_points_that_share_a_model_are_written_as_their_mix() {
    use vectorcraft_color::{Color, Freeform, FreeformPoint, GradientKind};
    let mid = |a: Color, b: Color| {
        let p = |x, c| FreeformPoint::new(Point::new(x, 50.0), c);
        let mut g = GradientPaint::new(Gradient { kind: GradientKind::Freeform, ..Default::default() });
        g.freeform = Some(Freeform { points: vec![p(20.0, a), p(80.0, b)], ..Default::default() });
        let d =
            doc_with(100.0, 100.0, vec![path(shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Paint::Gradient(Box::new(g)), Paint::None, 0.0)]);
        let back = import(&emf(&d).bytes).unwrap();
        let im = all(&back.document)
            .iter()
            .find_map(|n| match &n.kind {
                NodeKind::Image(im) => Some(im.clone()),
                _ => None,
            })
            .unwrap();
        let decoded = craft_image_codecs::decode(&back.document.images[&im.key].bytes).unwrap();
        let px = image::RgbaImage::from_raw(decoded.width(), decoded.height(), decoded.to_rgba8()).unwrap();
        px.get_pixel(px.width() / 2, px.height() / 2).0
    };
    let near = |got: [u8; 4], want: [u8; 4]| got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 6);
    let got = mid(Color::cmyk(1.0, 0.0, 0.0, 0.0), Color::cmyk(0.0, 0.0, 0.0, 1.0));
    let want = Color::cmyk(0.5, 0.0, 0.0, 0.5).to_rgba8(1.0);
    assert!(near(got, want), "CMYK: {got:?} vs {want:?}");
    let got = mid(Color::lab(90.0, -60.0, 40.0), Color::lab(30.0, 60.0, -40.0));
    let want = Color::lab(60.0, 0.0, 0.0).to_rgba8(1.0);
    assert!(near(got, want), "Lab: {got:?} vs {want:?}");
}

#[test]
fn dashes_gradients_and_images_round_trip_through_emf() {
    let mut line = path(shapes::line((10.0, 90.0).into(), (190.0, 90.0).into()), Paint::None, red(), 2.0);
    line.appearance.stroke_mut().unwrap().dash = Some(Dash { pattern: vec![6.0, 3.0], offset: 0.0, align_corners: false });
    let g = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
    let mut d = doc_with(200.0, 100.0, vec![line, path(shapes::rectangle(Rect::new(10.0, 10.0, 60.0, 30.0)), g, Paint::None, 0.0)]);
    let img = image_node(&mut d, png(4, 2, [0, 200, 0, 255]), 4, 2, Affine::translate((100.0, 10.0)) * Affine::scale(5.0));
    add(&mut d, img);
    let back = import(&emf(&d).bytes).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    let nodes = all(&back.document);
    let dash = nodes.iter().find_map(|n| n.appearance.stroke().and_then(|s| s.dash.clone())).unwrap();
    assert!((dash.pattern[0] - 6.0).abs() < 0.02 && (dash.pattern[1] - 3.0).abs() < 0.02, "{dash:?}");
    // The gradient: an image clipped to its rectangle (the picture's own clip is dropped); the
    // image: its pixels where they were.
    let groups: Vec<&Node> = nodes.iter().filter(|n| matches!(n.kind, NodeKind::Group { clip: true, .. })).collect();
    assert_eq!(groups.len(), 1);
    let clipped = groups[0].children().unwrap();
    assert!(near(clipped[0].geometric_bounds().unwrap(), Rect::new(10.0, 10.0, 60.0, 30.0), 0.03));
    assert!(matches!(clipped[1].kind, NodeKind::Image(_)));
    let im = nodes
        .iter()
        .find_map(|n| match &n.kind {
            NodeKind::Image(im) if im.width == 4 => Some(im.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(im.height, 2);
    let placed = im.xf.transform_rect_bbox(Rect::new(0.0, 0.0, 4.0, 2.0));
    assert!(near(placed, Rect::new(100.0, 10.0, 120.0, 20.0), 0.05), "{placed:?}");
    let decoded = craft_image_codecs::decode(&back.document.images[&im.key].bytes).unwrap();
    let px = image::RgbaImage::from_raw(decoded.width(), decoded.height(), decoded.to_rgba8()).unwrap();
    assert_eq!(px.get_pixel(0, 0).0, [0, 200, 0, 255]);
}

// ---------- hand-built records ----------

/// An EMF of `records` (type, body) with a frame of `w` × `h` hundredths of a millimetre on a
/// device of 100 pixels a millimetre. Bodies are padded to a multiple of 4 bytes.
fn hand_emf(w: i32, h: i32, records: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut o = Out::default();
    let rec = |o: &mut Out, kind: u32, body: &[u8]| {
        o.u32(kind);
        o.u32(8 + body.len().next_multiple_of(4) as u32);
        o.bytes(body);
        o.pad(4);
    };
    let mut head = Out::default();
    head.bytes(&[0; 16]);
    for v in [0, 0, w, h] {
        head.i32(v);
    }
    head.u32(0x464D_4520);
    head.u32(0x10000);
    head.bytes(&[0; 8]);
    head.u16(1);
    head.u16(0);
    head.bytes(&[0; 12]);
    for v in [10000, 10000, 100, 100] {
        head.i32(v);
    }
    rec(&mut o, 1, &head.0);
    for (k, body) in records {
        rec(&mut o, *k, body);
    }
    rec(&mut o, 14, &[0, 0, 0, 0, 16, 0, 0, 0, 20, 0, 0, 0]);
    o.0
}

fn le(values: &[i32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[test]
fn hand_built_emf_plays_like_gdi() {
    let mut text = vec![0; 16];
    text.extend(le(&[1, 0, 0]));
    // At (1000, 1500), 2 characters at offset 76 (from the record start).
    text.extend(le(&[1000, 1500, 2, 76, 0, 0, 0, 0, 0, 0]));
    text.extend("Hi".encode_utf16().flat_map(u16::to_le_bytes));
    let mut font = le(&[2, -1000, 0, 0, 0, 700]);
    font.extend([0u8; 8]);
    font.extend("Arial".encode_utf16().chain(std::iter::repeat_n(0, 27)).flat_map(u16::to_le_bytes));
    let records = vec![
        // A green brush in slot 1, selected; a null pen.
        (39, le(&[1, 0, 0x00_ff_00, 0])),
        (37, le(&[1])),
        (37, le(&[0x8000_0008u32 as i32])),
        // A rectangle of 10 × 5 mm at (10, 10) mm.
        (43, le(&[1000, 1000, 2000, 1500])),
        // A record nobody knows, and one of a kind not read.
        (9999, le(&[1, 2])),
        (41, le(&[0, 0, 10, 0, 0])),
        // Clip to a rectangle; a black pen draws two lines one after the other.
        (30, le(&[0, 0, 1500, 1500])),
        (37, le(&[0x8000_0007u32 as i32])),
        (27, le(&[0, 0])),
        (54, le(&[500, 0])),
        (54, le(&[500, 500])),
        // A 10 mm bold font and red text.
        (82, font),
        (37, le(&[2])),
        (24, le(&[0xff])),
        (84, text),
    ];
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    let mm = 72.0 / 25.4;
    assert!(near(back.document.artboards[0].rect, Rect::new(0.0, 0.0, 50.0 * mm, 30.0 * mm), 0.01));
    assert_eq!(back.warnings, vec!["2 records of kinds VectorCraft doesn't read were skipped".to_string()]);
    let layer = back.document.layers[0].children().unwrap().clone();
    // The rectangle outside the clip; the lines and the text in a clipping group.
    assert_eq!(layer.len(), 2);
    assert_eq!(fill_rgb(&layer[0]), Some([0, 255, 0]));
    assert!(near(layer[0].geometric_bounds().unwrap(), Rect::new(10.0 * mm, 10.0 * mm, 20.0 * mm, 15.0 * mm), 0.01));
    assert!(layer[0].appearance.stroke().is_none(), "the null pen");
    let NodeKind::Group { children, clip: true } = &layer[1].kind else { panic!("a clipping group") };
    assert!(matches!(children[0].kind, NodeKind::Path { clipping: true, .. }));
    assert!(near(children[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 15.0 * mm, 15.0 * mm), 0.01));
    // The two lines drawn one after the other are one path.
    let lines = &children[1];
    assert_eq!(lines.appearance.items.len(), 1);
    assert!(near(lines.geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 5.0 * mm, 5.0 * mm), 0.01));
    let NodeKind::Text(t) = &children[2].kind else { panic!("text") };
    assert_eq!(t.plain_text(), "Hi");
    let st = &t.runs[0].style;
    assert_eq!((st.font_family.as_str(), st.font_style.as_str()), ("Arial", "Bold"));
    assert!((st.size - 10.0 * mm).abs() < 0.01);
    assert_eq!(st.fill.color().map(|c| c.to_rgba8(1.0)), Some([255, 0, 0, 255]));
}

#[test]
fn emf_mapping_modes_and_world_transforms() {
    let records = vec![
        // MM_ANISOTROPIC: a 100 × 100 window on a 1000 × 2000 viewport.
        (17, le(&[8])),
        (9, le(&[100, 100])),
        (11, le(&[1000, 2000])),
        (43, le(&[0, 0, 10, 10])),
        // Then a world transform moving by (5, 0) logical units.
        (35, [1.0f32, 0.0, 0.0, 1.0, 5.0, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect()),
        (43, le(&[0, 0, 10, 10])),
    ];
    let back = import(&hand_emf(10000, 10000, &records)).unwrap();
    let p = paths(&back.document);
    let mm = 72.0 / 25.4;
    // 10 units → 100 × 200 pixels → 1 × 2 mm.
    assert!(near(p[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, mm, 2.0 * mm), 0.01));
    assert!(near(p[1].geometric_bounds().unwrap(), Rect::new(0.5 * mm, 0.0, 1.5 * mm, 2.0 * mm), 0.01));
}

/// A placeable WMF of `records` (function, parameters) in a `w` × `h` box at 1440 units an inch.
fn hand_wmf(w: i16, h: i16, records: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut o = Out::default();
    o.u32(0x9AC6_CDD7);
    o.u16(0);
    for v in [0, 0, w, h] {
        o.i16(v);
    }
    o.u16(1440);
    o.u32(0);
    let sum = crate::wmf::checksum(&o.0);
    o.u16(sum);
    o.bytes(&[1, 0, 9, 0, 0, 3]);
    o.bytes(&[0; 12]);
    for (f, body) in records.iter().chain(&[(0, vec![])]) {
        o.u32(3 + body.len() as u32 / 2);
        o.u16(*f);
        o.bytes(body);
    }
    o.0
}

fn le16(values: &[i16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[test]
fn hand_built_wmf_plays_like_gdi() {
    let records = vec![
        // A blue brush (slot 0), a 20-unit red pen with square caps (slot 1), selected.
        (0x02FC, [le16(&[0]), 0x00ff_0000u32.to_le_bytes().to_vec(), le16(&[0])].concat()),
        (0x02FA, [le16(&[0x100, 20, 0]), 0x0000_00ffu32.to_le_bytes().to_vec()].concat()),
        (0x012D, le16(&[0])),
        (0x012D, le16(&[1])),
        // A rectangle (bottom, right, top, left): 1 × 0.5 inches at the origin.
        (0x041B, le16(&[720, 1440, 0, 0])),
        (0x0999, le16(&[1, 2])),
        // Delete the brush: the next object takes slot 0 again.
        (0x01F0, le16(&[0])),
        (0x02FC, [le16(&[0]), 0x0000_ff00u32.to_le_bytes().to_vec(), le16(&[0])].concat()),
        (0x012D, le16(&[0])),
        (0x0418, le16(&[1440, 1440, 720, 0])),
    ];
    let back = import(&hand_wmf(1440, 1440, &records)).unwrap();
    assert!(near(back.document.artboards[0].rect, Rect::new(0.0, 0.0, 72.0, 72.0), 1e-9));
    assert_eq!(back.warnings, vec!["1 record of kinds VectorCraft doesn't read was skipped".to_string()]);
    let p = paths(&back.document);
    assert_eq!(p.len(), 2);
    assert_eq!(fill_rgb(&p[0]), Some([0, 0, 255]));
    assert!(near(p[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 72.0, 36.0), 1e-9));
    let st = p[0].appearance.stroke().unwrap();
    assert!((st.width - 1.0).abs() < 1e-9 && st.cap == vectorcraft_doc::LineCap::Square);
    assert_eq!(fill_rgb(&p[1]), Some([0, 255, 0]), "the new brush took the freed slot");
}

#[test]
fn wmf_window_is_mapped_onto_the_placeable_box() {
    // The box is 1000 × 500 units at 1440 an inch: 50 × 25 pt.
    let want = Rect::new(12.5, 6.25, 37.5, 18.75);
    // Window 0,0 – 4000 × 2000: the rectangle 1000,500 – 3000,1500 is the middle half.
    let ext = vec![(0x020C, le16(&[2000, 4000])), (0x041B, le16(&[1500, 3000, 500, 1000]))];
    // Window 10000,20000 – 1000 × 500 (the box): the rectangle 10250,20125 – 10750,20375.
    let org = vec![(0x020B, le16(&[20000, 10000])), (0x020C, le16(&[500, 1000])), (0x041B, le16(&[20375, 10750, 20125, 10250]))];
    // Window 0,2000 – 4000 × -2000 (y up): the same rectangle, flipped onto the same place.
    let flip = vec![(0x020B, le16(&[2000, 0])), (0x020C, le16(&[-2000, 4000])), (0x041B, le16(&[1500, 3000, 500, 1000]))];
    for records in [ext, org, flip] {
        let back = import(&hand_wmf(1000, 500, &records)).unwrap();
        assert!(near(back.document.artboards[0].rect, Rect::new(0.0, 0.0, 50.0, 25.0), 1e-9));
        let p = paths(&back.document);
        assert_eq!(p.len(), 1);
        assert!(near(p[0].geometric_bounds().unwrap(), want, 1e-6), "{:?}", p[0].geometric_bounds());
    }
}

#[test]
fn wmf_without_a_placeable_header_is_read_at_1440_units_an_inch() {
    // The window is 2880 × 1440 units: 2 × 1 inches. The rectangle is its left half.
    let records = vec![(0x020B, le16(&[0, 0])), (0x020C, le16(&[1440, 2880])), (0x041B, le16(&[1440, 1440, 0, 0]))];
    let plain = hand_wmf(2880, 1440, &records)[22..].to_vec();
    let back = import(&plain).unwrap();
    assert!(near(back.document.artboards[0].rect, Rect::new(0.0, 0.0, 144.0, 72.0), 1e-6), "{:?}", back.document.artboards[0].rect);
    let p = paths(&back.document);
    assert_eq!(p.len(), 1);
    assert!(near(p[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 72.0, 72.0), 1e-6), "{:?}", p[0].geometric_bounds());
}

#[test]
fn wmf_without_a_placeable_box_in_a_metric_map_mode_is_read_at_96_pixels_an_inch() {
    // MM_TWIPS (y up) and a window of 1440 twips square: one inch. The rectangle is the whole window.
    let records = vec![(0x0103, le16(&[6])), (0x020B, le16(&[0, 0])), (0x020C, le16(&[1440, 1440])), (0x041B, le16(&[1440, 1440, 0, 0]))];
    let boxed = hand_wmf(1440, 1440, &records);
    let plain = boxed[22..].to_vec();
    // The box runs from right to left, so it gives no size.
    let mut inverted = boxed;
    inverted[6..8].copy_from_slice(&1440i16.to_le_bytes());
    inverted[10..12].copy_from_slice(&0i16.to_le_bytes());
    for bytes in [plain, inverted] {
        let back = import(&bytes).unwrap();
        assert!(near(back.document.artboards[0].rect, Rect::new(0.0, 0.0, 72.0, 72.0), 1e-4), "{:?}", back.document.artboards[0].rect);
        let p = paths(&back.document);
        assert_eq!(p.len(), 1);
        assert!(near(p[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 72.0, 72.0), 1e-4), "{:?}", p[0].geometric_bounds());
    }
}

#[test]
fn wmf_whose_metric_map_mode_is_restored_away_is_read_at_1440_units_an_inch() {
    // SAVEDC, MM_TWIPS, then RESTOREDC back to MM_TEXT, by a count of levels and by a level; the
    // drawing is then the one in `wmf_without_a_placeable_header_is_read_at_1440_units_an_inch`.
    for restore in [-1i16, 1] {
        let records = vec![
            (0x001E, Vec::new()),
            (0x0103, le16(&[6])),
            (0x0127, le16(&[restore])),
            (0x020B, le16(&[0, 0])),
            (0x020C, le16(&[1440, 2880])),
            (0x041B, le16(&[1440, 1440, 0, 0])),
        ];
        let plain = hand_wmf(2880, 1440, &records)[22..].to_vec();
        let back = import(&plain).unwrap();
        assert!(near(back.document.artboards[0].rect, Rect::new(0.0, 0.0, 144.0, 72.0), 1e-6), "{restore}: {:?}", back.document.artboards[0].rect);
        let p = paths(&back.document);
        assert_eq!(p.len(), 1);
        assert!(near(p[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 72.0, 72.0), 1e-6), "{restore}: {:?}", p[0].geometric_bounds());
    }
}

#[test]
fn wmf_that_draws_in_a_metric_map_mode_and_restores_is_read_at_96_pixels_an_inch() {
    // SAVEDC, MM_TWIPS, a one-inch rectangle, then RESTOREDC back to MM_TEXT with nothing more drawn.
    let records = vec![
        (0x001E, Vec::new()),
        (0x0103, le16(&[6])),
        (0x020B, le16(&[0, 0])),
        (0x020C, le16(&[1440, 1440])),
        (0x041B, le16(&[1440, 1440, 0, 0])),
        (0x0127, le16(&[-1])),
    ];
    let plain = hand_wmf(1440, 1440, &records)[22..].to_vec();
    let back = import(&plain).unwrap();
    let p = paths(&back.document);
    assert_eq!(p.len(), 1);
    let b = p[0].geometric_bounds().unwrap();
    assert!((b.width() - 72.0).abs() < 1e-4 && (b.height() - 72.0).abs() < 1e-4, "{b:?}");
}

#[test]
fn wmf_records_the_mix_mode_keeps_from_drawing_do_not_set_its_scale() {
    // A rectangle of 1440 units in MM_TEXT; then SAVEDC, MM_TWIPS, a mix mode and two records that
    // draw nothing under it, and RESTOREDC.
    let rect = (0x041B, le16(&[1440, 1440, 0, 0]));
    let patblt = |rop: u32| (0x061D, [rop.to_le_bytes().to_vec(), le16(&[100, 100, 0, 0])].concat());
    let cases = [
        // R2_NOP and R2_XORPEN: rectangles draw nothing.
        (11i16, [rect.clone(), rect.clone()]),
        (7, [rect.clone(), rect.clone()]),
        // R2_NOP: PATBLT with PATCOPY fills nothing.
        (11, [patblt(0x00F0_0021), patblt(0x00F0_0021)]),
        // R2_COPYPEN: PATBLT with DSTINVERT is not drawn.
        (13, [patblt(0x0055_0009), patblt(0x0055_0009)]),
    ];
    for (mix, hidden) in cases {
        let mut records = vec![(0x020B, le16(&[0, 0])), (0x020C, le16(&[1440, 1440])), rect.clone()];
        records.extend([(0x001E, Vec::new()), (0x0103, le16(&[6])), (0x0104, le16(&[mix]))]);
        records.extend(hidden);
        records.push((0x0127, le16(&[-1])));
        let plain = hand_wmf(1440, 1440, &records)[22..].to_vec();
        let back = import(&plain).unwrap();
        let p = paths(&back.document);
        assert_eq!(p.len(), 1, "mix {mix}: {p:?}");
        let b = p[0].geometric_bounds().unwrap();
        assert!((b.width() - 72.0).abs() < 1e-4 && (b.height() - 72.0).abs() < 1e-4, "mix {mix}: {b:?}");
    }
    // R2_NOP, then SAVEDC, R2_COPYPEN and RESTOREDC bring R2_NOP back: the MM_TWIPS rectangle after
    // them draws nothing.
    let mut records = vec![(0x020B, le16(&[0, 0])), (0x020C, le16(&[1440, 1440])), rect.clone(), (0x0104, le16(&[11]))];
    records.extend([(0x001E, Vec::new()), (0x0104, le16(&[13])), (0x0127, le16(&[-1])), (0x0103, le16(&[6])), rect]);
    let back = import(&hand_wmf(1440, 1440, &records)[22..]).unwrap();
    let p = paths(&back.document);
    assert_eq!(p.len(), 1, "{p:?}");
    let b = p[0].geometric_bounds().unwrap();
    assert!((b.width() - 72.0).abs() < 1e-4 && (b.height() - 72.0).abs() < 1e-4, "restored: {b:?}");
}

#[test]
fn a_stroke_that_reaches_too_far_leaves_the_strokes_before_it() {
    // The black pen draws from (0, 500) to (100, 500), then on to a point far outside the picture.
    let records = vec![(37, le(&[0x8000_0007u32 as i32])), (27, le(&[0, 500])), (54, le(&[100, 500])), (54, le(&[100_000_000, 500]))];
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    let p = paths(&back.document);
    assert_eq!(p.len(), 1, "{p:?}");
    let mm = 72.0 / 25.4;
    let b = p[0].geometric_bounds().unwrap();
    assert!((b.x0).abs() < 0.01 && (b.x1 - mm).abs() < 0.01 && (b.y0 - 5.0 * mm).abs() < 0.01, "{b:?}");
    assert!(back.warnings.iter().any(|w| w.contains("far outside")), "{:?}", back.warnings);
}

#[test]
fn damaged_files_fail_or_import_partly() {
    let good = emf(&sample()).bytes;
    for cut in [0, 8, 50, 107, 108, 120, good.len() / 2, good.len() - 4] {
        let _ = import(&good[..cut]);
    }
    let cut = import(&good[..good.len() / 2]).unwrap();
    assert!(cut.warnings.iter().any(|w| w.contains("ends early")));
    let good = wmf(&sample()).bytes;
    for cut in [0, 4, 22, 30, 40, good.len() / 2] {
        let _ = import(&good[..cut]);
    }
    assert!(import(b"not a metafile").is_err());
    // Huge counts, sizes and offsets are refused, not allocated.
    let huge = hand_emf(100, 100, &[(3, le(&[0, 0, 0, 0, i32::MAX])), (81, le(&[0; 18])), (8, le(&[0, 0, 0, 0, i32::MAX, i32::MAX]))]);
    let back = import(&huge).unwrap();
    assert!(paths(&back.document).is_empty());
    let huge = hand_wmf(100, 100, &[(0x0324, le16(&[i16::MAX])), (0x0538, le16(&[-1]))]);
    assert!(import(&huge).is_ok());
}

#[test]
fn a_file_with_more_objects_than_are_read_says_so() {
    // RECTANGLE records: 10 more than the reader keeps, then a few under that.
    let rect = (43, le(&[0, 0, 100, 100]));
    let many = hand_emf(5000, 3000, &vec![rect.clone(); 200_010]);
    let back = import(&many).unwrap();
    assert_eq!(paths(&back.document).len(), 200_000);
    assert!(back.warnings.iter().any(|w| w.contains("reached a limit")), "{:?}", back.warnings);
    // Exactly as many as the reader keeps: nothing was left unread.
    let full = import(&hand_emf(5000, 3000, &vec![rect.clone(); 200_000])).unwrap();
    assert_eq!(paths(&full.document).len(), 200_000);
    assert!(full.warnings.is_empty(), "{:?}", full.warnings);
    let few = import(&hand_emf(5000, 3000, &vec![rect; 3])).unwrap();
    assert!(few.warnings.is_empty(), "{:?}", few.warnings);
}

#[test]
fn a_file_cut_short_by_another_limit_says_so() {
    // SAVEDC past the 1,024 levels the reader keeps stops the parse: the rectangle after them is
    // not read.
    let mut records = vec![(43, le(&[0, 0, 100, 100]))];
    records.extend(vec![(33, vec![]); 1100]);
    records.push((43, le(&[200, 200, 300, 300])));
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    assert_eq!(paths(&back.document).len(), 1);
    assert_eq!(
        back.warnings,
        vec!["the file or a picture inside it reached a limit on what one file may hold: what came after the limit was not read".to_string()]
    );
}

#[test]
fn images_the_reader_does_not_read_are_counted() {
    let records = vec![(81, png_stretch_dibits(&png_claiming(1 << 13, 1 << 13))); 2];
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    assert_eq!(back.warnings, vec!["2 images or pattern brushes that couldn't be read were left out".to_string()]);
    let back = import(&hand_emf(5000, 3000, &records[..1])).unwrap();
    assert_eq!(back.warnings, vec!["1 image or pattern brush that couldn't be read was left out".to_string()]);
}

#[test]
fn a_wmf_reports_unread_pattern_brushes_and_raster_operations() {
    let records = vec![
        // DIBCREATEPATTERNBRUSH (style 5, usage 0) whose bitmap header can't be read.
        (0x0142, [le16(&[5, 0]), vec![0xFF; 8]].concat()),
        // A bitmap-less DIBBITBLT with DSTINVERT (0x00550009): rop, ySrc, xSrc, reserved, height,
        // width, yDest, xDest.
        (0x0940, le16(&[0x0009, 0x0055, 0, 0, 0, 100, 100, 0, 0])),
    ];
    let back = import(&hand_wmf(1440, 1440, &records)).unwrap();
    assert_eq!(
        back.warnings,
        vec![
            "1 image or pattern brush that couldn't be read was left out".to_string(),
            "1 raster operation other than a plain copy (masks, inversions) was drawn as a plain copy or left out".to_string(),
        ]
    );
}

#[test]
fn clips_that_are_not_applied_are_counted() {
    // BEGINPATH, a rectangle, ENDPATH, SELECTCLIPPATH with RGN_XOR (3), then a rectangle drawn after it.
    let records = vec![(59, vec![]), (43, le(&[100, 100, 2000, 2000])), (60, vec![]), (67, le(&[3])), (43, le(&[500, 500, 600, 600]))];
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    assert_eq!(paths(&back.document).len(), 1);
    assert_eq!(
        back.warnings,
        vec!["1 clip couldn't be applied as the file sets it: what is drawn after it may show more or less than the file does".to_string()]
    );
    // RGN_AND (1) is applied.
    let mut records = records;
    records[3] = (67, le(&[1]));
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
}

/// An EMR_BITBLT of the 100 × 100 box at the origin with `rop` and no source bitmap.
fn bitblt(rop: u32) -> Vec<u8> {
    // Bounds; destination; the operation; the source origin, transform (identity) and background
    // color; usage, then no BITMAPINFO and no bits.
    let mut body = vec![0; 16];
    body.extend(le(&[0, 0, 100, 100, rop as i32, 0, 0]));
    body.extend(le_f32(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]));
    body.extend(le(&[0, 0, 0, 0, 0, 0]));
    body
}

#[test]
fn raster_operations_that_are_not_played_are_counted() {
    // DSTINVERT and PATINVERT take no source bitmap and are left out.
    let records = vec![(76, bitblt(0x0055_0009)), (76, bitblt(0x005A_0049))];
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    assert_eq!(
        back.warnings,
        vec!["2 raster operations other than a plain copy (masks, inversions) were drawn as a plain copy or left out".to_string()]
    );
    // WHITENESS is played.
    let back = import(&hand_emf(5000, 3000, &[(76, bitblt(0x00FF_0062))])).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
}

/// An EMR_STRETCHDIBITS copying `png` (a BI_PNG bitmap) to a 10 × 10 box.
fn png_stretch_dibits(png: &[u8]) -> Vec<u8> {
    // Bounds; destination, source and its size; the BITMAPINFO and the bits (from the record start);
    // usage, SRCCOPY and the destination size; then the BITMAPINFOHEADER and the PNG.
    let mut body = vec![0; 16];
    body.extend(le(&[0, 0, 0, 0, 1, 1]));
    body.extend(le(&[80, 40, 120, png.len() as i32]));
    body.extend(le(&[0, 0x00CC_0020, 10, 10]));
    body.extend(le(&[40, 1, 1, 1, 5, 0, 0, 0, 0, 0]));
    body.extend(png);
    body
}

#[test]
fn oversized_embedded_bitmaps_are_left_out_before_decoding() {
    // #3600: many records of the largest bitmap a record may hold cost minutes. Each of these claims
    // more pixels than one image may have, so the reader leaves it out from its header. (The reader's
    // budget for all the images of one file is tested where the reader is.)
    let records = vec![(81, png_stretch_dibits(&png_claiming(1 << 13, 1 << 13))); 5];
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    assert!(!all(&back.document).iter().any(|n| matches!(n.kind, NodeKind::Image(_))));
}

#[test]
fn clips_nest_only_so_deep() {
    // Each clip inside the last one nests a clipping group one level deeper; past the stack they
    // are left out with a warning.
    let mut records = vec![];
    // As many as the reader keeps: one more than the stack.
    for i in 0..32 {
        // BEGINPATH, a rectangle a little smaller each time, ENDPATH, SELECTCLIPPATH (RGN_AND).
        records.push((59, vec![]));
        records.push((43, le(&[100 + i, 100 + i, 2000 - i, 2000 - i])));
        records.push((60, vec![]));
        records.push((67, le(&[1])));
    }
    records.push((43, le(&[500, 500, 600, 600])));
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    let mut depth = 0;
    let mut n = back.document.layers[0].children().unwrap()[0].clone();
    while let NodeKind::Group { children, clip: true } = &n.kind {
        depth += 1;
        n = children.last().unwrap().clone();
    }
    assert_eq!(depth, vectorcraft_doc::clipnest::MAX_NEST);
    assert!(back.warnings.iter().any(|w| w.contains("clips nested more than")), "{:?}", back.warnings);
    // A hostile count of clip records: each one used to nest a clipping group one level deeper,
    // past the stack. They make one clip.
    let mut records = vec![(30, le(&[0, 0, 1500, 1500])); 200_000];
    records.push((43, le(&[100, 100, 200, 200])));
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    let layer = back.document.layers[0].children().unwrap();
    assert_eq!(layer.len(), 1);
    let NodeKind::Group { children, clip: true } = &layer[0].kind else { panic!("a clipping group: {:?}", layer[0].kind) };
    assert!(children.iter().all(|c| !matches!(c.kind, NodeKind::Group { .. })), "one level");
    assert_eq!(paths(&back.document).len(), 1);
}

/// An EMR_COMMENT holding EMF+ `records` (type, flags, data).
fn emf_plus(records: &[(u16, u16, Vec<u8>)]) -> Vec<u8> {
    let mut plus = Out::default();
    for (kind, flags, data) in records {
        plus.u16(*kind);
        plus.u16(*flags);
        plus.u32(12 + data.len() as u32);
        plus.u32(data.len() as u32);
        plus.bytes(data);
    }
    let mut body = Out::default();
    body.u32(4 + plus.len() as u32);
    // "EMF+"
    body.u32(0x2B46_4D45);
    body.bytes(&plus.0);
    body.0
}

fn le_f32(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

#[test]
fn emf_plus_only_pictures_import_their_drawing() {
    // An EMF+ only file: its EMF records draw nothing, so it was imported empty with a warning.
    // EmfPlusHeader (version, flags, 96 dpi), FillRects with an inline green color (flag S) and one
    // rectangle of 10 × 5 mm at (10, 10) mm in device pixels, EmfPlusEndOfFile.
    let header = le(&[0xDBC0_1002u32 as i32, 0, 96, 96]);
    let rects = [le(&[0xFF00_FF00u32 as i32, 1]), le_f32(&[1000.0, 1000.0, 1000.0, 500.0])].concat();
    let comment = emf_plus(&[(0x4001, 0, header), (0x400A, 0x8000, rects), (0x4002, 0, vec![])]);
    let back = import(&hand_emf(5000, 3000, &[(70, comment)])).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    let p = paths(&back.document);
    assert_eq!(p.len(), 1);
    assert_eq!(fill_rgb(&p[0]), Some([0, 255, 0]));
    let mm = 72.0 / 25.4;
    let b = p[0].geometric_bounds().unwrap();
    assert!(near(b, Rect::new(10.0 * mm, 10.0 * mm, 20.0 * mm, 15.0 * mm), 0.01), "{b:?}");
}

#[test]
fn hatched_brushes_fill_with_their_average_color() {
    // CREATEBRUSHINDIRECT: a hatched (style 2) blue brush, selected; a null pen; a rectangle. The
    // hatch is drawn over the white background, so the average is a light blue.
    let records = vec![(39, le(&[1, 2, 0x00ff_0000, 0])), (37, le(&[1])), (37, le(&[0x8000_0008u32 as i32])), (43, le(&[1000, 1000, 2000, 1500]))];
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    assert_eq!(back.warnings, vec!["hatched and pattern brushes are filled with their average color".to_string()]);
    let p = paths(&back.document);
    assert_eq!(p.len(), 1);
    let [r, g, b] = fill_rgb(&p[0]).unwrap();
    assert!(b == 255 && r == g && r > 0 && r < 255, "{:?}", [r, g, b]);
    // With a transparent background (SETBKMODE 1) the hatch is blue with gaps: a translucent fill.
    let mut records = records;
    records.insert(0, (18, le(&[1])));
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    let p = paths(&back.document);
    assert_eq!(fill_rgb(&p[0]), Some([0, 0, 255]));
    let AppearanceItem::Fill(f) = &p[0].appearance.items[0] else { panic!("a fill") };
    assert!(f.opacity > 0.0 && f.opacity < 1.0, "{}", f.opacity);
}

#[test]
fn text_keeps_a_stretch_across() {
    // A world transform twice as wide as it is high, a 10 mm font and "Hi".
    let mut text = vec![0; 16];
    text.extend(le(&[1, 0, 0]));
    text.extend(le(&[1000, 1500, 2, 76, 0, 0, 0, 0, 0, 0]));
    text.extend("Hi".encode_utf16().flat_map(u16::to_le_bytes));
    let mut font = le(&[2, -1000, 0, 0, 0, 400]);
    font.extend([0u8; 8]);
    font.extend("Arial".encode_utf16().chain(std::iter::repeat_n(0, 27)).flat_map(u16::to_le_bytes));
    let world = [2.0f32, 0.0, 0.0, 1.0, 0.0, 0.0].iter().flat_map(|v| v.to_le_bytes()).collect();
    let records = vec![(35, world), (82, font), (37, le(&[2])), (84, text)];
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    let t = all(&back.document)
        .into_iter()
        .find_map(|n| match n.kind {
            NodeKind::Text(t) => Some(t),
            _ => None,
        })
        .unwrap();
    let mm = 72.0 / 25.4;
    assert!((t.runs[0].style.size - 10.0 * mm).abs() < 0.01, "{}", t.runs[0].style.size);
    let [a, b, c, d, ..] = t.xf.as_coeffs();
    assert!((a - 2.0).abs() < 1e-6 && b.abs() < 1e-6 && c.abs() < 1e-6 && (d - 1.0).abs() < 1e-6, "{:?}", t.xf);
}

/// Source Sans 3 Regular's own values, read from its table directory: unitsPerEm (`head`),
/// xAvgCharWidth and usWinAscent + usWinDescent (`OS/2`), in font units.
fn source_sans_metrics() -> (f64, f64, f64) {
    let font: &[u8] = include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf");
    let u16_at = |at: usize| u16::from_be_bytes([font[at], font[at + 1]]);
    let table = |tag: &[u8; 4]| {
        let at = (0..usize::from(u16_at(4))).map(|i| 12 + 16 * i).find(|at| &font[*at..*at + 4] == tag).unwrap();
        u32::from_be_bytes([font[at + 8], font[at + 9], font[at + 10], font[at + 11]]) as usize
    };
    let (head, os2) = (table(b"head"), table(b"OS/2"));
    let avg = f64::from(i16::from_be_bytes([font[os2 + 2], font[os2 + 3]]));
    (f64::from(u16_at(head + 18)), avg, f64::from(u16_at(os2 + 74)) + f64::from(u16_at(os2 + 76)))
}

/// The style of "Hi" drawn in an EMF in Source Sans 3 with a LOGFONT of `height` and `width`
/// (hundredths of a millimeter).
fn emf_font_style(height: i32, width: i32) -> vectorcraft_doc::CharStyle {
    emf_font_style_in("Source Sans 3", height, width)
}

/// [`emf_font_style`] with the LOGFONT naming `family`.
fn emf_font_style_in(family: &str, height: i32, width: i32) -> vectorcraft_doc::CharStyle {
    let mut text = vec![0; 16];
    text.extend(le(&[1, 0, 0]));
    text.extend(le(&[1000, 1500, 2, 76, 0, 0, 0, 0, 0, 0]));
    text.extend("Hi".encode_utf16().flat_map(u16::to_le_bytes));
    let mut font = le(&[2, height, width, 0, 0, 400]);
    font.extend([0u8; 8]);
    font.extend(family.encode_utf16().chain(std::iter::repeat_n(0, 32 - family.encode_utf16().count())).flat_map(u16::to_le_bytes));
    let records = vec![(82, font), (37, le(&[2])), (84, text)];
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    let t = all(&back.document)
        .into_iter()
        .find_map(|n| match n.kind {
            NodeKind::Text(t) => Some(t),
            _ => None,
        })
        .unwrap();
    t.runs[0].style.clone()
}

#[test]
fn a_font_width_becomes_horizontal_scale() {
    let (upem, avg, _) = source_sans_metrics();
    let mm = 72.0 / 25.4;
    // A 10 mm em and an average character width of 6 mm: the face's average width, avg/upem of
    // the em, is scaled to 6 mm.
    let want = 6.0 / (avg / upem * 10.0) * 100.0;
    for width in [600, -600] {
        let st = emf_font_style(-1000, width);
        assert_eq!(st.font_family, "Source Sans 3");
        assert!((st.size - 10.0 * mm).abs() < 1e-9, "{}", st.size);
        assert!((st.h_scale - want).abs() < 1e-9, "width {width}: {} for {want}", st.h_scale);
    }
    assert_eq!(emf_font_style(-1000, 0).h_scale, 100.0);
    // A 4 m em is set at the largest type size, 5000 pt, and a 2.4 m average width keeps its
    // width at that size.
    let st = emf_font_style(-400_000, 240_000);
    assert_eq!(st.size, 5000.0);
    let want = 2400.0 * mm / (avg / upem * 5000.0) * 100.0;
    assert!((st.h_scale - want).abs() < 1e-9, "{} for {want}", st.h_scale);
    // With a height of 0 the type is set at the default size and the width is ignored.
    assert_eq!(emf_font_style(0, 600).h_scale, 100.0);
}

#[test]
fn a_positive_font_height_is_the_character_cell() {
    let (upem, avg, cell) = source_sans_metrics();
    let mm = 72.0 / 25.4;
    // A 10 mm cell: the em is the cell times unitsPerEm divided by the face's usWinAscent plus
    // usWinDescent, and a 6 mm average width is measured against that em.
    let em = 10.0 * upem / cell;
    let st = emf_font_style(1000, 600);
    assert!((st.size - em * mm).abs() < 1e-9, "{} for {}", st.size, em * mm);
    let want = 6.0 / (avg / upem * em) * 100.0;
    assert!((st.h_scale - want).abs() < 1e-9, "{} for {want}", st.h_scale);
}

#[test]
fn a_font_the_system_lacks_is_measured_in_the_face_that_draws_it() {
    // A family no system has keeps its name, and layout draws it in Source Sans 3: the cell and
    // the average width are measured in that face.
    let (upem, avg, cell) = source_sans_metrics();
    let mm = 72.0 / 25.4;
    let st = emf_font_style_in("Missing Family 7183", 1000, 600);
    assert_eq!(st.font_family, "Missing Family 7183");
    let em = 10.0 * upem / cell;
    assert!((st.size - em * mm).abs() < 1e-9, "{} for {}", st.size, em * mm);
    let want = 6.0 / (avg / upem * em) * 100.0;
    assert!((st.h_scale - want).abs() < 1e-9, "{} for {want}", st.h_scale);
}

#[test]
fn wmf_font_width_becomes_horizontal_scale() {
    let (upem, avg, _) = source_sans_metrics();
    // CREATEFONTINDIRECT: a 0.1 inch em (144 units) with an average width of 72 units, then
    // SELECTOBJECT and TEXTOUT "Hi" at (720, 360).
    let mut font = le16(&[-144, 72, 0, 0, 400]);
    font.extend([0u8; 8]);
    font.extend(b"Source Sans 3".iter().copied().chain(std::iter::repeat_n(0, 19)));
    let mut text = le16(&[2]);
    text.extend(b"Hi");
    text.extend(le16(&[360, 720]));
    let records = vec![(0x02FB, font), (0x012D, le16(&[0])), (0x0521, text)];
    let back = import(&hand_wmf(1440, 1440, &records)).unwrap();
    let t = all(&back.document)
        .into_iter()
        .find_map(|n| match n.kind {
            NodeKind::Text(t) => Some(t),
            _ => None,
        })
        .unwrap();
    let st = &t.runs[0].style;
    assert!((st.size - 7.2).abs() < 1e-9, "{}", st.size);
    let want = 72.0 / (avg / upem * 144.0) * 100.0;
    assert!((st.h_scale - want).abs() < 1e-9, "{} for {want}", st.h_scale);
}

#[test]
fn a_clip_whose_box_holds_the_picture_still_clips() {
    // A diamond around the middle of the picture, whose box holds the whole frame but which cuts
    // off its corners: BEGINPATH, POLYGON, ENDPATH, SELECTCLIPPATH (RGN_COPY), then a rectangle.
    let mut diamond = le(&[0, 0, 0, 0, 4]);
    diamond.extend(le(&[2500, -500, 5500, 1500, 2500, 3500, -500, 1500]));
    // A bow tie through the four corners of a box around the picture.
    let mut bow_tie = le(&[0, 0, 0, 0, 4]);
    bow_tie.extend(le(&[-500, -500, 5500, 3500, 5500, -500, -500, 3500]));
    // Two open figures through the four corners, the top edge and the bottom edge, which enclose
    // nothing: MOVETOEX, LINETO, MOVETOEX, LINETO.
    let edges = vec![(27, le(&[-500, -500])), (54, le(&[5500, -500])), (27, le(&[5500, 3500])), (54, le(&[-500, 3500]))];
    // One figure through the four corners that runs back over its edges and encloses nothing:
    // POLYLINE top-left, top-right, top-left, bottom-left, bottom-right, bottom-left.
    let mut retraced = le(&[0, 0, 0, 0, 6]);
    retraced.extend(le(&[-500, -500, 5500, -500, -500, -500, -500, 3500, 5500, 3500, -500, 3500]));
    for clip in [vec![(3, diamond)], vec![(3, bow_tie)], edges, vec![(4, retraced)]] {
        let records = [vec![(59, vec![])], clip, vec![(60, vec![]), (67, le(&[5])), (43, le(&[0, 0, 5000, 3000]))]].concat();
        let back = import(&hand_emf(5000, 3000, &records)).unwrap();
        let layer = back.document.layers[0].children().unwrap();
        assert_eq!(layer.len(), 1);
        assert!(matches!(layer[0].kind, NodeKind::Group { clip: true, .. }), "{:?}", layer[0].kind);
    }
}

#[test]
fn dual_emf_plus_files_draw_their_emf_plus_records() {
    // A dual file (header flag 1): a translucent red EMF+ fill, and a plain rectangle that is only
    // the fallback for readers of plain records.
    let header = le(&[0xDBC0_1002u32 as i32, 0, 96, 96]);
    let rects = [le(&[0x80FF_0000u32 as i32, 1]), le_f32(&[1000.0, 1000.0, 1000.0, 500.0])].concat();
    let comment = emf_plus(&[(0x4001, 1, header), (0x400A, 0x8000, rects), (0x4002, 0, vec![])]);
    let back = import(&hand_emf(5000, 3000, &[(70, comment), (43, le(&[3000, 1000, 4000, 2000]))])).unwrap();
    let p = paths(&back.document);
    assert_eq!(p.len(), 1);
    assert_eq!(fill_rgb(&p[0]), Some([255, 0, 0]));
    let AppearanceItem::Fill(f) = &p[0].appearance.items[0] else { panic!("a fill") };
    assert!((f.opacity - 128.0 / 255.0).abs() < 1e-6, "{}", f.opacity);
}

#[test]
fn translucent_strokes_drawn_one_after_another_stay_apart() {
    // EmfPlusObject: pen 0, 1 pixel wide, with a solid brush of `argb`; then two EmfPlusDrawLines
    // with it, from A to B and from B back to A.
    let header = le(&[0xDBC0_1002u32 as i32, 0, 96, 96]);
    let lines = |argb: u32| {
        let pen = [le(&[0xDBC0_1002u32 as i32, 0, 0, 2]), le_f32(&[1.0]), le(&[0xDBC0_1002u32 as i32, 0, argb as i32])].concat();
        let a_to_b = [le(&[2]), le_f32(&[1000.0, 1000.0, 3000.0, 1000.0])].concat();
        let b_to_a = [le(&[2]), le_f32(&[3000.0, 1000.0, 1000.0, 1000.0])].concat();
        let comment = emf_plus(&[(0x4001, 0, header.clone()), (0x4008, 0x0200, pen), (0x400D, 0, a_to_b), (0x400D, 0, b_to_a), (0x4002, 0, vec![])]);
        paths(&import(&hand_emf(5000, 3000, &[(70, comment)])).unwrap().document)
    };
    // Each translucent stroke keeps its own opacity, so where they overlap both show.
    let p = lines(0x80FF_0000);
    assert_eq!(p.len(), 2, "{p:?}");
    for n in &p {
        let AppearanceItem::Stroke(st) = &n.appearance.items[0] else { panic!("a stroke") };
        assert!((st.opacity - 128.0 / 255.0).abs() < 1e-6, "{}", st.opacity);
    }
    // Opaque strokes still join into one path.
    assert_eq!(lines(0xFFFF_0000).len(), 1);
}

#[test]
fn strokes_join_only_with_round_caps_round_joins_and_no_dashes() {
    // CREATEPENINDIRECT: a 20-unit black pen of `style`, selected; then two lines one after the
    // other that meet at a right angle.
    let lines = |style: i16| {
        let records = vec![
            (0x02FA, [le16(&[style, 20, 0]), 0u32.to_le_bytes().to_vec()].concat()),
            (0x012D, le16(&[0])),
            (0x0214, le16(&[100, 100])),
            (0x0213, le16(&[100, 700])),
            (0x0213, le16(&[700, 700])),
        ];
        paths(&import(&hand_wmf(1440, 1440, &records)).unwrap().document)
    };
    // Round caps and round joins, a solid line: one path, as a round join is the two round caps.
    assert_eq!(lines(0).len(), 1);
    // Flat caps and miter joins, flat caps alone and miter joins alone: each line keeps its caps.
    for style in [0x2200, 0x0200, 0x2000] {
        let p = lines(style);
        assert_eq!(p.len(), 2, "style {style:#x}: {p:?}");
    }
    // PS_DASH: each line's dashes start at its own start.
    let p = lines(1);
    assert_eq!(p.len(), 2, "{p:?}");
    assert!(p.iter().all(|n| n.appearance.stroke().is_some_and(|st| st.dash.is_some())), "{p:?}");
}

#[test]
fn pattern_brushes_fill_with_their_average_color() {
    // CREATEDIBPATTERNBRUSHPT: a 2 × 2 tile of 24-bit pixels, two red and two blue (bottom-up rows
    // padded to 4 bytes); selected, a null pen, a rectangle.
    let mut brush = le(&[1, 0, 32, 40, 72, 16]);
    brush.extend(le(&[40, 2, 2, 0x0018_0001, 0, 0, 0, 0, 0, 0]));
    brush.extend([0, 0, 255, 255, 0, 0, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0]);
    let records = vec![(94, brush), (37, le(&[1])), (37, le(&[0x8000_0008u32 as i32])), (43, le(&[1000, 1000, 2000, 1500]))];
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    assert_eq!(back.warnings, vec!["hatched and pattern brushes are filled with their average color".to_string()]);
    let p = paths(&back.document);
    assert_eq!(p.len(), 1);
    let [r, g, b] = fill_rgb(&p[0]).unwrap();
    assert!((126..=128).contains(&r) && g == 0 && (126..=128).contains(&b), "{:?}", [r, g, b]);
}

#[test]
fn clipped_text_is_clipped_to_its_rectangle() {
    // EXTTEXTOUTW with ETO_CLIPPED and a rectangle of 9–12 × 10–16 mm, in an underlined and struck
    // through font.
    let mut text = vec![0; 16];
    text.extend(le(&[1, 0, 0]));
    text.extend(le(&[1000, 1500, 2, 76, 4, 900, 1000, 1200, 1600, 0]));
    text.extend("Hi".encode_utf16().flat_map(u16::to_le_bytes));
    let mut font = le(&[2, -1000, 0, 0, 0, 400]);
    font.extend([0, 1, 1, 0, 0, 0, 0, 0]);
    font.extend("Arial".encode_utf16().chain(std::iter::repeat_n(0, 27)).flat_map(u16::to_le_bytes));
    let mut records = vec![(82, font), (37, le(&[2])), (84, text)];
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    let layer = back.document.layers[0].children().unwrap();
    assert_eq!(layer.len(), 1);
    let NodeKind::Group { children, clip: true } = &layer[0].kind else { panic!("a clipping group: {:?}", layer[0].kind) };
    let mm = 72.0 / 25.4;
    let own = Rect::new(9.0 * mm, 10.0 * mm, 12.0 * mm, 16.0 * mm);
    assert!(near(children[0].geometric_bounds().unwrap(), own, 0.01));
    let NodeKind::Text(t) = &children[1].kind else { panic!("text") };
    assert!(t.runs[0].style.underline && t.runs[0].style.strikethrough);
    // Under clip paths nested as deep as the stack, the text's own clip is still the innermost.
    for i in 0..32 {
        records.insert(0, (67, le(&[1])));
        records.insert(0, (60, vec![]));
        records.insert(0, (43, le(&[100 + i, 100 + i, 4000 - i, 2500 - i])));
        records.insert(0, (59, vec![]));
    }
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    let mut n = back.document.layers[0].children().unwrap()[0].clone();
    while let NodeKind::Group { children, clip: true } = &n.kind
        && matches!(children.last().map(|c| &c.kind), Some(NodeKind::Group { .. }))
    {
        n = children.last().unwrap().clone();
    }
    let NodeKind::Group { children, clip: true } = &n.kind else { panic!("a clipping group: {:?}", n.kind) };
    assert!(near(children[0].geometric_bounds().unwrap(), own, 0.01), "{:?}", children[0].geometric_bounds());
    assert!(matches!(children[1].kind, NodeKind::Text(_)));
}

/// The first text object of `d`.
fn first_text(d: &Document) -> TextObject {
    all(d)
        .into_iter()
        .find_map(|n| match n.kind {
            NodeKind::Text(t) => Some(*t),
            _ => None,
        })
        .unwrap()
}

/// Where each glyph of `t` starts across the document (points), and where the last one ends.
fn glyph_xs(t: &TextObject) -> (Vec<f64>, f64) {
    let laid = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    let xs = laid.glyphs.iter().map(|g| (t.xf * g.origin).x).collect();
    let end = laid.glyphs.last().map_or(0.0, |g| (t.xf * (g.origin + vectorcraft_geom::Vec2::new(g.advance, 0.0))).x);
    (xs, end)
}

/// EXTTEXTOUTW of `s` at (`x`, 1500), with the advances `dx` when there are any.
fn text_out(x: i32, s: &str, dx: &[i32]) -> Vec<u8> {
    let units: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let n = units.len() as i32 / 2;
    // The string 76 bytes from the record start, the advances after it on a 4-byte boundary.
    let dx_at = if dx.is_empty() { 0 } else { 76 + units.len().next_multiple_of(4) as i32 };
    let mut text = vec![0; 16];
    text.extend(le(&[1, 0, 0]));
    text.extend(le(&[x, 1500, n, 76, 0, 0, 0, 0, 0, dx_at]));
    text.extend(&units);
    text.resize(68 + units.len().next_multiple_of(4), 0);
    text.extend(le(dx));
    text
}

/// EXTCREATEFONTINDIRECTW: a 10 mm Arial in slot 2.
fn arial() -> Vec<u8> {
    font_of("Arial", false)
}

/// EXTCREATEFONTINDIRECTW: a 10 mm `family` in slot 2, underlined when `underline` is set.
fn font_of(family: &str, underline: bool) -> Vec<u8> {
    let mut font = le(&[2, -1000, 0, 0, 0, 400]);
    font.extend([0, u8::from(underline), 0, 0, 0, 0, 0, 0]);
    let name: Vec<u16> = family.encode_utf16().collect();
    font.extend(name.iter().copied().chain(std::iter::repeat_n(0, 32 - name.len())).flat_map(u16::to_le_bytes));
    font
}

/// The text and glyph starts (millimeters, rounded to hundredths) of every text object in `d`.
fn texts_at(d: &Document) -> Vec<(String, Vec<f64>)> {
    let mm = 72.0 / 25.4;
    all(d)
        .into_iter()
        .filter_map(|n| match n.kind {
            NodeKind::Text(t) => Some((t.plain_text(), glyph_xs(&t).0.iter().map(|x| (x / mm * 100.0).round() / 100.0).collect())),
            _ => None,
        })
        .collect()
}

#[test]
fn text_is_spaced_by_its_advances() {
    let mm = 72.0 / 25.4;
    // "Hi" with advances of 20 and 10 mm, left, centered and right aligned (SETTEXTALIGN), its
    // line from 10 to 40 mm each time.
    for (align, x) in [(0, 1000), (6, 2500), (2, 4000)] {
        let records = vec![(22, le(&[align])), (82, arial()), (37, le(&[2])), (84, text_out(x, "Hi", &[2000, 1000]))];
        let back = import(&hand_emf(5000, 3000, &records)).unwrap();
        let t = first_text(&back.document);
        assert_eq!(t.plain_text(), "Hi");
        assert!(t.runs.iter().all(|r| r.style.kerning == Some(0.0)), "{:?}", t.runs);
        let (xs, end) = glyph_xs(&t);
        assert_eq!(xs.len(), 2, "{align}");
        assert!((xs[0] - 10.0 * mm).abs() < 0.01 && (xs[1] - 30.0 * mm).abs() < 0.01, "{align}: {xs:?}");
        assert!((end - 40.0 * mm).abs() < 0.01, "{align}: {end}");
    }
    // Characters 5 mm apart and a last "x" 10 mm wide, whatever the font joins or reorders before
    // it: ligatures that the tracking drops, a letter with a combining mark and a conjunct. The
    // "x" starts 5 mm after each character before it, and ends 10 mm later.
    for s in ["ffix", "e\u{301}x", "\u{928}\u{94d}\u{924}x"] {
        let n = s.chars().count();
        let dx: Vec<i32> = (0..n).map(|i| if i + 1 == n { 1000 } else { 500 }).collect();
        let records = vec![(82, arial()), (37, le(&[2])), (84, text_out(1000, s, &dx))];
        let t = first_text(&import(&hand_emf(5000, 3000, &records)).unwrap().document);
        let laid = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
        let x = laid.glyphs.iter().find(|g| g.byte == s.len() - 1).map(|g| (t.xf * g.origin).x).unwrap();
        let want = 10.0 + 5.0 * (n - 1) as f64;
        assert!((x - want * mm).abs() < 0.01, "{s:?}: {x} {:?}", t.runs);
        let (_, end) = glyph_xs(&t);
        assert!((end - (want + 10.0) * mm).abs() < 0.01, "{s:?}: {end}");
    }
    // An advance further than tracking reaches (120 mm on a 10 mm em): the characters after it are
    // type of their own, where the advances put them, left or right aligned.
    for (align, x) in [(0, 1000), (2, 14500)] {
        let records = vec![(22, le(&[align])), (82, arial()), (37, le(&[2])), (84, text_out(x, "Hiy", &[12000, 500, 1000]))];
        let back = import(&hand_emf(20000, 3000, &records)).unwrap();
        let texts: Vec<TextObject> = all(&back.document)
            .into_iter()
            .filter_map(|n| match n.kind {
                NodeKind::Text(t) => Some(*t),
                _ => None,
            })
            .collect();
        let found: Vec<(String, Vec<f64>)> =
            texts.iter().map(|t| (t.plain_text(), glyph_xs(t).0.iter().map(|x| (x / mm * 100.0).round() / 100.0).collect())).collect();
        assert_eq!(found, vec![("H".to_string(), vec![10.0]), ("iy".to_string(), vec![130.0, 135.0])], "{align}");
    }
    // The same advance inside a ligature ("fi" in a font that joins it): the type splits between
    // the "f" and the "i".
    let records = vec![(82, font_of("Source Serif 4", false)), (37, le(&[2])), (84, text_out(1000, "fix", &[12000, 500, 1000]))];
    let found = texts_at(&import(&hand_emf(20000, 3000, &records)).unwrap().document);
    assert_eq!(found, vec![("f".to_string(), vec![10.0]), ("ix".to_string(), vec![130.0, 135.0])]);
    // Advances between the "fi" ligature's and the separate letters' (3.64, 3.08 and 5.26 mm), where
    // the ligature forms at one tracking and breaks at the next: the "x" starts at 16.72 mm.
    let records = vec![(82, font_of("Source Serif 4", false)), (37, le(&[2])), (84, text_out(1000, "fix", &[364, 308, 526]))];
    let t = first_text(&import(&hand_emf(5000, 3000, &records)).unwrap().document);
    let laid = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let x = laid.glyphs.iter().find(|g| g.byte == 2).map(|g| (t.xf * g.origin).x).unwrap();
    let (_, end) = glyph_xs(&t);
    assert!((x - 16.72 * mm).abs() < 0.01 && (end - 21.98 * mm).abs() < 0.01, "{x} {end} {:?}", t.runs);
    // A tab, which layout advances to its tab stop, ends a piece: the "B" after a 15 mm tab starts
    // at 35 mm.
    let records = vec![(82, arial()), (37, le(&[2])), (84, text_out(1000, "A\tB", &[1000, 1500, 1000]))];
    let found = texts_at(&import(&hand_emf(10000, 3000, &records)).unwrap().document);
    let b = found.iter().find(|(s, _)| s.ends_with('B')).and_then(|(_, xs)| xs.last().copied());
    assert_eq!(b, Some(35.0), "{found:?}");
    // A leading NUL's advance counts in the line's width: right aligned at 50 mm, the 10 mm NUL
    // starts at 10 mm and "Hi" at 20 mm.
    let records = vec![(22, le(&[2])), (82, arial()), (37, le(&[2])), (84, text_out(5000, "\0Hi", &[1000, 2000, 1000]))];
    let t = first_text(&import(&hand_emf(5000, 3000, &records)).unwrap().document);
    assert_eq!(t.plain_text(), "Hi");
    let (xs, end) = glyph_xs(&t);
    assert!((xs[0] - 20.0 * mm).abs() < 0.01 && (end - 50.0 * mm).abs() < 0.01, "{xs:?} {end}");
}

#[test]
fn multiline_text_with_advances_aligns_each_line() {
    let mm = 72.0 / 25.4;
    // "Hi\nHi" with advances of 20 and 10 mm on each line, left, centered and right aligned: each
    // line runs from 10 to 40 mm, as "Hi" alone does, the second on the baseline below the first.
    // A second line of 15 and 5 mm, centered or right aligned, leaves the first where it is.
    let at = |records: &[(u32, Vec<u8>)]| -> Vec<(f64, f64)> {
        let back = import(&hand_emf(5000, 3000, records)).unwrap();
        let mut at = vec![];
        for n in all(&back.document) {
            let NodeKind::Text(t) = n.kind else { continue };
            let laid = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
            for g in laid.glyphs.iter().filter(|g| g.advance > 0.0) {
                let p = t.xf * g.origin;
                at.push(((p.x / mm * 100.0).round() / 100.0, (p.y / mm * 100.0).round() / 100.0));
            }
        }
        at
    };
    let mut drops = vec![];
    for (align, x) in [(0, 1000), (6, 2500), (2, 4000)] {
        let records = vec![(22, le(&[align])), (82, arial()), (37, le(&[2])), (84, text_out(x, "Hi\nHi", &[2000, 1000, 0, 2000, 1000]))];
        let found = at(&records);
        let xs: Vec<f64> = found.iter().map(|p| p.0).collect();
        assert_eq!(xs, vec![10.0, 30.0, 10.0, 30.0], "{align}: {found:?}");
        assert!(found[0].1 == found[1].1 && found[2].1 == found[3].1 && found[2].1 > found[0].1, "{align}: {found:?}");
        drops.push(found[2].1 - found[0].1);
    }
    assert!(drops.iter().all(|d| (d - drops[0]).abs() < 0.01), "{drops:?}");
    // Underlined and left aligned, each line's underline ends with its own advances: no bar runs on
    // past either line.
    let records = vec![(82, font_of("Arial", true)), (37, le(&[2])), (84, text_out(1000, "Hi\nHi", &[2000, 1000, 0, 2000, 1000]))];
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    assert!(all(&back.document).iter().all(|n| !matches!(n.kind, NodeKind::Path { .. })));
    for (align, x, second) in [(6, 2500, [15.0, 30.0]), (2, 4000, [20.0, 35.0])] {
        let records = vec![(22, le(&[align])), (82, arial()), (37, le(&[2])), (84, text_out(x, "Hi\nHi", &[2000, 1000, 0, 1500, 500]))];
        let xs: Vec<f64> = at(&records).iter().map(|p| p.0).collect();
        assert_eq!(xs, vec![10.0, 30.0, second[0], second[1]], "{align}");
    }
    // An advance further than tracking reaches on the second line of centered type (at 100 mm):
    // that line splits where the advance puts its characters, and the first line stays in place.
    let records = vec![(22, le(&[6])), (82, arial()), (37, le(&[2])), (84, text_out(10000, "Hi\nHiy", &[2000, 1000, 0, 12000, 500, 1000]))];
    let found = texts_at(&import(&hand_emf(20000, 3000, &records)).unwrap().document);
    let want = vec![("Hi".to_string(), vec![85.0, 105.0]), ("H".to_string(), vec![32.5]), ("iy".to_string(), vec![152.5, 157.5])];
    assert_eq!(found, want);
}

#[test]
fn emf_plus_driver_strings_keep_their_positions() {
    // An EMF+ only file: a 1000 pixel Arial (font object 0) and DrawDriverString of "Hi" in black,
    // as characters (CmapLookup), its glyphs at x = 1000 and 3000 pixels (10 and 30 mm).
    let header = le(&[0xDBC0_1002u32 as i32, 0, 96, 96]);
    let mut font = [le(&[0xDBC0_1002u32 as i32]), le_f32(&[1000.0]), le(&[2, 0, 0, 5])].concat();
    font.extend("Arial".encode_utf16().chain([0]).flat_map(u16::to_le_bytes));
    let mut draw = le(&[0xFF00_0000u32 as i32, 1, 0, 2]);
    draw.extend("Hi".encode_utf16().flat_map(u16::to_le_bytes));
    draw.extend(le_f32(&[1000.0, 1500.0, 3000.0, 1500.0]));
    let comment = emf_plus(&[(0x4001, 0, header), (0x4008, 6 << 8, font), (0x4036, 0x8000, draw), (0x4002, 0, vec![])]);
    let back = import(&hand_emf(5000, 3000, &[(70, comment)])).unwrap();
    let t = first_text(&back.document);
    assert_eq!(t.plain_text(), "Hi");
    let (xs, _) = glyph_xs(&t);
    let mm = 72.0 / 25.4;
    assert_eq!(xs.len(), 2);
    assert!((xs[0] - 10.0 * mm).abs() < 0.01 && (xs[1] - 30.0 * mm).abs() < 0.01, "{xs:?}");
}

#[test]
fn wmf_text_is_spaced_by_its_advances() {
    // CREATEFONTINDIRECT of a 10 pt Arial (slot 0), selected, and EXTTEXTOUT of "Hi" at x = 10 pt
    // with advances of 20 and 10 pt, at 20 units a point.
    let mut font = le16(&[-200, 0, 0, 0, 400]);
    font.extend([0u8; 8]);
    font.extend(b"Arial".iter().copied().chain(std::iter::repeat_n(0, 27)));
    let mut text = le16(&[500, 200, 2, 0]);
    text.extend(b"Hi");
    text.extend(le16(&[400, 200]));
    let records = vec![(0x02FB, font), (0x012D, le16(&[0])), (0x0A32, text)];
    let t = first_text(&import(&hand_wmf(4000, 1000, &records)).unwrap().document);
    assert_eq!(t.plain_text(), "Hi");
    let (xs, end) = glyph_xs(&t);
    assert_eq!(xs.len(), 2);
    assert!((xs[0] - 10.0).abs() < 0.01 && (xs[1] - 30.0).abs() < 0.01 && (end - 40.0).abs() < 0.01, "{xs:?} {end}");
}

#[test]
fn split_type_keeps_its_underline_and_right_to_left_order() {
    let mm = 72.0 / 25.4;
    // Underlined "Hiy" with an advance further than tracking reaches after the "H": the bar runs on
    // from the end of the "H"'s underline to where "iy" starts, at 130 mm.
    let records = vec![(82, font_of("Arial", true)), (37, le(&[2])), (84, text_out(1000, "Hiy", &[12000, 500, 1000]))];
    let back = import(&hand_emf(20000, 3000, &records)).unwrap();
    assert_eq!(texts_at(&back.document), vec![("H".to_string(), vec![10.0]), ("iy".to_string(), vec![130.0, 135.0])]);
    let bars: Vec<Rect> =
        all(&back.document).iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).filter_map(|n| n.geometric_bounds()).collect();
    assert_eq!(bars.len(), 1, "{bars:?}");
    let b = bars[0];
    assert!(b.x0 > 10.0 * mm && b.x0 < 130.0 * mm && (b.x1 - 130.0 * mm).abs() < 0.01 && b.height() < 2.0 * mm, "{b:?}");
    // The "H" and the "i" before the split are tracked apart and draw a bar each: only the bar that
    // ends the piece runs on, from the end of the "i" to 130 mm.
    let records = vec![(82, font_of("Arial", true)), (37, le(&[2])), (84, text_out(1000, "Hiy", &[1000, 12000, 1000]))];
    let back = import(&hand_emf(20000, 3000, &records)).unwrap();
    assert_eq!(texts_at(&back.document), vec![("Hi".to_string(), vec![10.0, 20.0]), ("y".to_string(), vec![140.0])]);
    let gaps: Vec<Node> = all(&back.document).into_iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).collect();
    assert_eq!(gaps.len(), 1);
    let NodeKind::Path { path, .. } = &gaps[0].kind else { panic!("a path") };
    let rects = path.to_bezpath().elements().iter().filter(|e| matches!(e, vectorcraft_geom::PathEl::MoveTo(_))).count();
    assert_eq!(rects, 1);
    let b = gaps[0].geometric_bounds().unwrap();
    assert!(b.x0 > 20.0 * mm && (b.x1 - 140.0 * mm).abs() < 0.01, "{b:?}");
    // Without an underline, nothing but the type.
    let records = vec![(82, arial()), (37, le(&[2])), (84, text_out(1000, "Hiy", &[12000, 500, 1000]))];
    let back = import(&hand_emf(20000, 3000, &records)).unwrap();
    assert!(all(&back.document).iter().all(|n| !matches!(n.kind, NodeKind::Path { .. })));
    // A last advance further than tracking reaches, right aligned at 140 mm: the "H" and the "i"
    // start at 10 and 20 mm, and the underline runs on from the "i" to 140 mm.
    let records = vec![(22, le(&[2])), (82, font_of("Arial", true)), (37, le(&[2])), (84, text_out(14000, "Hi", &[1000, 12000]))];
    let back = import(&hand_emf(20000, 3000, &records)).unwrap();
    assert_eq!(texts_at(&back.document), vec![("Hi".to_string(), vec![10.0, 20.0])]);
    let bars: Vec<Rect> =
        all(&back.document).iter().filter(|n| matches!(n.kind, NodeKind::Path { .. })).filter_map(|n| n.geometric_bounds()).collect();
    assert_eq!(bars.len(), 1, "{bars:?}");
    assert!(bars[0].x0 > 20.0 * mm && (bars[0].x1 - 140.0 * mm).abs() < 0.01, "{bars:?}");
    // Right to left type with the same advance stays one text object, its characters in order.
    let s = "\u{5e9}\u{5dc}\u{5d5}\u{5dd}";
    let records = vec![(82, arial()), (37, le(&[2])), (84, text_out(1000, s, &[12000, 500, 500, 1000]))];
    let found = texts_at(&import(&hand_emf(20000, 3000, &records)).unwrap().document);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].0, s);
}

#[test]
fn excluded_clip_rectangles_clip() {
    // EXCLUDECLIPRECT cuts the top-left of the picture out of the clip; the rectangle crosses it.
    // The clip it cuts from is the picture's bounds (5000 × 3000 device pixels).
    let records = vec![(29, le(&[0, 0, 1500, 1500])), (43, le(&[1000, 1000, 2000, 1500]))];
    let mut bytes = hand_emf(5000, 3000, &records);
    for (i, v) in [0i32, 0, 5000, 3000].iter().enumerate() {
        bytes[8 + i * 4..12 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    let back = import(&bytes).unwrap();
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    let layer = back.document.layers[0].children().unwrap();
    assert_eq!(layer.len(), 1);
    assert!(matches!(layer[0].kind, NodeKind::Group { clip: true, .. }), "{:?}", layer[0].kind);
}

#[test]
fn shapes_under_the_same_clip_share_one_clipping_group() {
    let records = vec![(30, le(&[0, 0, 1500, 1500])), (43, le(&[100, 100, 200, 200])), (43, le(&[300, 300, 400, 400]))];
    let back = import(&hand_emf(5000, 3000, &records)).unwrap();
    let layer = back.document.layers[0].children().unwrap();
    assert_eq!(layer.len(), 1);
    let NodeKind::Group { children, clip: true } = &layer[0].kind else { panic!("a clipping group: {:?}", layer[0].kind) };
    assert_eq!(children.len(), 3, "the clip and both rectangles");
}

#[test]
fn a_record_whose_size_is_not_a_multiple_of_4_ends_the_file() {
    let records = vec![(43, le(&[100, 100, 200, 200])), (43, le(&[300, 300, 400, 400])), (43, le(&[500, 500, 600, 600]))];
    let mut bytes = hand_emf(5000, 3000, &records);
    // The second rectangle's size, after the header and the first rectangle.
    let header = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    let first = u32::from_le_bytes(bytes[header + 4..header + 8].try_into().unwrap()) as usize;
    let at = header + first + 4;
    let size = u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    bytes[at..at + 4].copy_from_slice(&(size + 2).to_le_bytes());
    let back = import(&bytes).unwrap();
    assert_eq!(paths(&back.document).len(), 1);
    assert_eq!(back.warnings, vec!["the file ends early: what it held was imported".to_string()]);
}

#[test]
fn wmf_with_an_inverted_placeable_box_is_read_at_its_units_an_inch() {
    // The box runs from right to left, so it gives no size: the window (1440 units square, one inch
    // at the header's 1440 units an inch) does. The rectangle is its top-left quarter.
    let records = vec![(0x020B, le16(&[0, 0])), (0x020C, le16(&[1440, 1440])), (0x041B, le16(&[720, 720, 0, 0]))];
    let mut bytes = hand_wmf(1440, 1440, &records);
    bytes[6..8].copy_from_slice(&1440i16.to_le_bytes());
    bytes[10..12].copy_from_slice(&0i16.to_le_bytes());
    let back = import(&bytes).unwrap();
    assert!(near(back.document.artboards[0].rect, Rect::new(0.0, 0.0, 72.0, 72.0), 1e-6), "{:?}", back.document.artboards[0].rect);
    let p = paths(&back.document);
    assert_eq!(p.len(), 1);
    assert!(near(p[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 36.0, 36.0), 1e-6), "{:?}", p[0].geometric_bounds());
}

#[test]
fn emf_without_a_frame_is_read_at_its_device_resolution() {
    // An empty frame, and bounds of 5000 × 3000 device pixels at 100 pixels a millimetre.
    let mut bytes = hand_emf(0, 0, &[(43, le(&[1000, 1000, 2000, 1500]))]);
    for (i, v) in [0i32, 0, 5000, 3000].iter().enumerate() {
        bytes[8 + i * 4..12 + i * 4].copy_from_slice(&v.to_le_bytes());
    }
    let back = import(&bytes).unwrap();
    let mm = 72.0 / 25.4;
    assert!(near(back.document.artboards[0].rect, Rect::new(0.0, 0.0, 50.0 * mm, 30.0 * mm), 0.01), "{:?}", back.document.artboards[0].rect);
    let p = paths(&back.document);
    assert_eq!(p.len(), 1);
    assert!(near(p[0].geometric_bounds().unwrap(), Rect::new(10.0 * mm, 10.0 * mm, 20.0 * mm, 15.0 * mm), 0.01), "{:?}", p[0].geometric_bounds());
    // A device 50 mm high: 200 pixels a millimetre down, so everything is half as tall.
    bytes[84..88].copy_from_slice(&50i32.to_le_bytes());
    let back = import(&bytes).unwrap();
    assert!(near(back.document.artboards[0].rect, Rect::new(0.0, 0.0, 50.0 * mm, 15.0 * mm), 0.01), "{:?}", back.document.artboards[0].rect);
    let p = paths(&back.document);
    assert!(near(p[0].geometric_bounds().unwrap(), Rect::new(10.0 * mm, 5.0 * mm, 20.0 * mm, 7.5 * mm), 0.01), "{:?}", p[0].geometric_bounds());
}
