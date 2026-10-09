//! Metafiles read back: VectorCraft's own EMF and WMF round-trip, hand-built records of other
//! writers play as they draw, and damaged files fail or import partly without a panic.

use vectorcraft_color::{Gradient, GradientPaint, Paint};
use vectorcraft_doc::{AppearanceItem, Dash, Document, Node, NodeKind};
use vectorcraft_geom::{Affine, Rect, shapes};

use crate::bytes::Out;
use crate::tests::*;
use crate::*;

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
    let px = image::load_from_memory(&back.document.images[&im.key].bytes).unwrap().to_rgba8();
    assert_eq!(px.get_pixel(0, 0).0, [0, 200, 0, 255]);
}

// ---------- hand-built records ----------

/// An EMF of `records` (type, body) with a frame of `w` × `h` hundredths of a millimetre on a
/// device of 100 pixels a millimetre.
fn hand_emf(w: i32, h: i32, records: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut o = Out::default();
    let rec = |o: &mut Out, kind: u32, body: &[u8]| {
        o.u32(kind);
        o.u32(8 + body.len() as u32);
        o.bytes(body);
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
    assert_eq!(back.warnings, vec!["2 records of kinds Vector W3K2 doesn't read were skipped".to_string()]);
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
    assert_eq!(back.warnings, vec!["1 record of kinds Vector W3K2 doesn't read was skipped".to_string()]);
    let p = paths(&back.document);
    assert_eq!(p.len(), 2);
    assert_eq!(fill_rgb(&p[0]), Some([0, 0, 255]));
    assert!(near(p[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 72.0, 36.0), 1e-9));
    let st = p[0].appearance.stroke().unwrap();
    assert!((st.width - 1.0).abs() < 1e-9 && st.cap == vectorcraft_doc::LineCap::Square);
    assert_eq!(fill_rgb(&p[1]), Some([0, 255, 0]), "the new brush took the freed slot");
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
