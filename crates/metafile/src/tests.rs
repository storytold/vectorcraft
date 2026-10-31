//! EMF and WMF output read back record by record: headers, record sizes, paths, pens, brushes,
//! clips, images, and what each format leaves out.

use std::sync::Arc;

use vectorcraft_color::{Color, Gradient, GradientPaint, Paint};
use vectorcraft_doc::{
    Appearance, CharStyle, Dash, Document, ImageBlob, ImageObject, LineCap, LineJoin, Node, NodeId, NodeKind, PatternDef, TextObject,
};
use vectorcraft_geom::{Affine, PathData, Point, Rect, shapes};

use crate::*;

mod import;

/// Add `n` (with a fresh id) on top of `d`'s first layer.
pub(crate) fn add(d: &mut Document, mut n: Node) {
    n.id = d.alloc_id();
    let layer = d.layers[0].id;
    let at = d.layers[0].children().map_or(0, Vec::len);
    d.insert(Some(layer), at, n).unwrap();
}

/// A document of one `w` × `h` artboard holding `nodes` on its layer.
pub(crate) fn doc_with(w: f64, h: f64, nodes: Vec<Node>) -> Document {
    let mut d = Document::new(w, h);
    nodes.into_iter().for_each(|n| add(&mut d, n));
    d
}

pub(crate) fn path(p: PathData, fill: Paint, stroke: Paint, width: f64) -> Node {
    Node::path(NodeId(0), p, Appearance::basic(fill, stroke, width))
}

pub(crate) fn red() -> Paint {
    Paint::solid(Color::rgb8(255, 0, 0))
}

pub(crate) fn blue() -> Paint {
    Paint::solid(Color::rgb8(0, 0, 255))
}

/// A `w` × `h` PNG of `rgba`.
pub(crate) fn png(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(w, h, image::Rgba(rgba));
    let mut out = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    out
}

/// CRC-32 of a PNG chunk.
pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// A 1 × 1 PNG whose IHDR claims `w` × `h` (and whose CRC matches): its size reads from the header,
/// but its pixel data no longer matches, so decoding it fails.
pub(crate) fn png_claiming(w: u32, h: u32) -> Vec<u8> {
    let mut png = vec![];
    image::GrayImage::new(1, 1).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png[16..20].copy_from_slice(&w.to_be_bytes());
    png[20..24].copy_from_slice(&h.to_be_bytes());
    let crc = crc32(&png[12..29]);
    png[29..33].copy_from_slice(&crc.to_be_bytes());
    png
}

/// An image node of `bytes` placed by `xf`, its blob added to `d`.
pub(crate) fn image_node(d: &mut Document, bytes: Vec<u8>, w: u32, h: u32, xf: Affine) -> Node {
    let blob = ImageBlob::new("image/png", bytes);
    let key = blob.content_key();
    d.images.insert(key.clone(), blob);
    Node::new(
        NodeId(0),
        NodeKind::Image(Box::new(ImageObject { key, width: w, height: h, xf, link: None, placement: Default::default(), pixels: None })),
    )
}

pub(crate) fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

pub(crate) fn i32_at(b: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

pub(crate) fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(b[at..at + 2].try_into().unwrap())
}

/// The EMF's records: (type, the whole record).
pub(crate) fn emf_records(b: &[u8]) -> Vec<(u32, &[u8])> {
    let mut out = vec![];
    let mut at = 0;
    while at < b.len() {
        let size = u32_at(b, at + 4) as usize;
        assert!(size >= 8 && size.is_multiple_of(4), "record size {size} at {at}");
        out.push((u32_at(b, at), &b[at..at + size]));
        at += size;
    }
    assert_eq!(at, b.len(), "records end at the end of the file");
    out
}

/// The WMF's records after both headers: (function, the whole record).
pub(crate) fn wmf_records(b: &[u8]) -> Vec<(u16, &[u8])> {
    let mut out = vec![];
    let mut at = 22 + 18;
    while at < b.len() {
        let words = u32_at(b, at) as usize;
        assert!(words >= 3, "record of {words} words at {at}");
        out.push((u16_at(b, at + 4), &b[at..at + words * 2]));
        at += words * 2;
    }
    assert_eq!(at, b.len());
    out
}

pub(crate) fn emf(d: &Document) -> Output {
    export(d, d.artboards[0].rect, Kind::Emf).unwrap()
}

pub(crate) fn wmf(d: &Document) -> Output {
    export(d, d.artboards[0].rect, Kind::Wmf).unwrap()
}

fn kinds(recs: &[(u32, &[u8])]) -> Vec<u32> {
    recs.iter().map(|r| r.0).collect()
}

fn count(k: &[u32], kind: u32) -> usize {
    k.iter().filter(|x| **x == kind).count()
}

#[test]
fn emf_header_frame_is_the_region_in_hundredths_of_a_millimetre() {
    // 72 × 144 pt is 25.4 × 50.8 mm; a rectangle from (36, 36) to (54, 72) pt.
    let d = doc_with(72.0, 144.0, vec![path(shapes::rectangle(Rect::new(36.0, 36.0, 54.0, 72.0)), red(), Paint::None, 0.0)]);
    let b = &emf(&d).bytes;
    assert_eq!(sniff(b), Some(Kind::Emf));
    assert_eq!(u32_at(b, 0), 1);
    assert_eq!(u32_at(b, 40), 0x464D_4520, "the \" EMF\" signature");
    // rclFrame.
    assert_eq!([i32_at(b, 24), i32_at(b, 28), i32_at(b, 32), i32_at(b, 36)], [0, 0, 2540, 5080]);
    // rclBounds, in device units of 0.01 mm: the rectangle.
    assert_eq!([i32_at(b, 8), i32_at(b, 12), i32_at(b, 16), i32_at(b, 20)], [1270, 1270, 1905, 2540]);
    // The reference device has 100 pixels a millimetre.
    let (dev, mm) = ((i32_at(b, 72), i32_at(b, 76)), (i32_at(b, 80), i32_at(b, 84)));
    assert_eq!(dev, (mm.0 * 100, mm.1 * 100));
    assert_eq!((u32_at(b, 100), u32_at(b, 104)), (mm.0 as u32 * 1000, mm.1 as u32 * 1000), "micrometres");
    // The description names the app.
    let n = u32_at(b, 60) as usize;
    let off = u32_at(b, 64) as usize;
    assert_eq!(off, 108);
    let desc: Vec<u16> = (0..n).map(|i| u16_at(b, off + 2 * i)).collect();
    assert!(String::from_utf16_lossy(&desc).starts_with("VectorCraft\0"));
    // An empty picture has empty bounds.
    let empty = emf(&doc_with(72.0, 72.0, vec![]));
    assert_eq!([i32_at(&empty.bytes, 8), i32_at(&empty.bytes, 16)], [0, -1]);
}

#[test]
fn emf_record_sizes_add_up_to_the_header_totals() {
    let mut d = doc_with(200.0, 100.0, vec![]);
    let img = image_node(&mut d, png(3, 2, [0, 128, 0, 255]), 3, 2, Affine::translate((10.0, 10.0)) * Affine::scale(10.0));
    add(&mut d, img);
    add(&mut d, path(shapes::ellipse(Rect::new(50.0, 10.0, 150.0, 90.0)), red(), blue(), 2.0));
    let b = emf(&d).bytes;
    let recs = emf_records(&b);
    assert_eq!(u32_at(&b, 48) as usize, b.len(), "nBytes");
    assert_eq!(u32_at(&b, 52) as usize, recs.len(), "nRecords");
    assert_eq!(u16_at(&b, 56), 3, "nHandles: the reserved slot, a brush and a pen");
    let (last, eof) = recs.last().copied().unwrap();
    assert_eq!((last, eof.len()), (14, 20), "EMR_EOF ends the file");
    assert_eq!(u32_at(eof, 16), 20, "nSizeLast");
    // Fixed-size records have their fixed sizes.
    for (k, r) in &recs {
        let fixed = match k {
            18 | 19 | 34 | 37 | 40 | 58 | 67 => Some(12),
            27 | 54 => Some(16),
            33 | 59 | 60 | 61 => Some(8),
            39 | 62 | 64 => Some(24),
            35 => Some(32),
            36 => Some(36),
            _ => None,
        };
        if let Some(f) = fixed {
            assert_eq!(r.len(), f, "record {k}");
        }
    }
    // StretchDIBits: its bitmap where its offsets say, 3 × 2 pixels of 24 bits (rows of 12 bytes).
    let sdib = recs.iter().find(|r| r.0 == 81).unwrap().1;
    assert_eq!([u32_at(sdib, 48), u32_at(sdib, 52), u32_at(sdib, 56), u32_at(sdib, 60)], [80, 40, 120, 24]);
    assert_eq!(sdib.len(), 144);
}

#[test]
fn emf_paths_keep_their_curves_brushes_and_pens() {
    let mut e = path(shapes::ellipse(Rect::new(10.0, 10.0, 60.0, 40.0)), red(), blue(), 4.0);
    if let Some(st) = e.appearance.stroke_mut() {
        st.cap = LineCap::Round;
        st.join = LineJoin::Bevel;
    }
    let b = emf(&doc_with(100.0, 100.0, vec![e])).bytes;
    let recs = emf_records(&b);
    let k = kinds(&recs);
    assert!(k.contains(&5), "EMR_POLYBEZIERTO keeps the curves");
    assert!(k.contains(&62) && k.contains(&64), "filled, then stroked");
    let brush = recs.iter().find(|r| r.0 == 39).unwrap().1;
    assert_eq!(u32_at(brush, 16), 0x0000_00ff, "a red COLORREF");
    let pen = recs.iter().find(|r| r.0 == 95).unwrap().1;
    let style = u32_at(pen, 28);
    assert_eq!(style & 0x000f_0000, 0x0001_0000, "geometric");
    assert_eq!(style & 0xf00, 0, "round caps");
    assert_eq!(style & 0xf000, 0x1000, "bevel joins");
    assert_eq!(u32_at(pen, 32), (4.0 * 2540.0 / 72.0f64).round() as u32, "4 pt in 0.01 mm");
    assert_eq!(u32_at(pen, 40), 0x00ff_0000, "a blue COLORREF");
    assert!(recs.iter().any(|r| r.0 == 19 && u32_at(r.1, 8) == 2), "WINDING for non-zero");
    // One brush and one pen for two shapes of the same colours.
    let two = doc_with(
        100.0,
        100.0,
        vec![
            path(shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), red(), blue(), 1.0),
            path(shapes::rectangle(Rect::new(20.0, 0.0, 30.0, 10.0)), red(), blue(), 1.0),
        ],
    );
    let k = kinds(&emf_records(&emf(&two).bytes));
    assert_eq!((count(&k, 39), count(&k, 95)), (1, 1));
}

#[test]
fn emf_dashes_are_pen_styles() {
    let mut l = path(shapes::line(Point::new(10.0, 10.0), Point::new(90.0, 10.0)), Paint::None, blue(), 2.0);
    l.appearance.stroke_mut().unwrap().dash = Some(Dash { pattern: vec![6.0, 3.0], offset: 0.0, align_corners: false });
    let b = emf(&doc_with(100.0, 100.0, vec![l.clone()])).bytes;
    let recs = emf_records(&b);
    let pen = recs.iter().find(|r| r.0 == 95).unwrap().1;
    assert_eq!(u32_at(pen, 28) & 0xf, 7, "PS_USERSTYLE");
    assert_eq!(u32_at(pen, 48), 2);
    assert_eq!([u32_at(pen, 52), u32_at(pen, 56)], [(6.0 * 2540.0 / 72.0f64).round() as u32, (3.0 * 2540.0 / 72.0f64).round() as u32]);
    // With an offset, the dashes are separate lines drawn with a solid pen.
    l.appearance.stroke_mut().unwrap().dash = Some(Dash { pattern: vec![6.0, 3.0], offset: 2.0, align_corners: false });
    let b = emf(&doc_with(100.0, 100.0, vec![l])).bytes;
    let recs = emf_records(&b);
    assert_eq!(u32_at(recs.iter().find(|r| r.0 == 95).unwrap().1, 28) & 0xf, 0);
    assert!(count(&kinds(&recs), 27) > 5, "a move-to for each dash");
}

#[test]
fn emf_clips_gradients_and_images() {
    let g = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
    let mut d = doc_with(100.0, 100.0, vec![path(shapes::rectangle(Rect::new(10.0, 10.0, 60.0, 30.0)), g, Paint::None, 0.0)]);
    let opaque = image_node(&mut d, png(4, 4, [10, 20, 30, 255]), 4, 4, Affine::translate((0.0, 50.0)));
    let clear = image_node(&mut d, png(4, 4, [10, 20, 30, 100]), 4, 4, Affine::translate((50.0, 50.0)) * Affine::rotate(0.3));
    add(&mut d, opaque);
    add(&mut d, clear);
    let out = emf(&d);
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    let recs = emf_records(&out.bytes);
    let k = kinds(&recs);
    // The region's clip, then the gradient's clip around its image.
    assert_eq!(count(&k, 67), 2);
    assert_eq!(count(&k, 33), count(&k, 34), "every save is restored");
    // The (opaque) gradient's image and the opaque image are StretchDIBits; the translucent one
    // AlphaBlend with per-pixel alpha.
    assert_eq!(count(&k, 81), 2);
    let ab = recs.iter().find(|r| r.0 == 114).unwrap().1;
    assert_eq!(&ab[40..44], &[0, 0, 255, 1]);
    // Images are placed by a world transform, reset after each.
    assert_eq!((count(&k, 35), count(&k, 36)), (3, 3));
}

#[test]
fn type_is_written_as_outlines() {
    let t = TextObject::point(Point::new(10.0, 50.0), "Hi", CharStyle { fill: red(), ..CharStyle::default() });
    let d = doc_with(100.0, 100.0, vec![Node::new(NodeId(0), NodeKind::Text(Box::new(t)))]);
    let out = emf(&d);
    let k = kinds(&emf_records(&out.bytes));
    assert!(k.contains(&62), "glyphs are filled paths");
    assert!(!k.contains(&84) && !k.contains(&83), "no text records");
    // Said so (#3850).
    assert!(out.warnings.iter().any(|w| w.contains("type is written as outlines")), "{:?}", out.warnings);
}

/// Ruby (furigana) is written with its base text, as the canvas draws it: one more `MoveToEx`
/// per contour of the reading's glyphs in EMF, more polygons in WMF.
#[test]
fn ruby_is_written_with_its_type() {
    let typed = |ruby: bool| {
        let mut t = TextObject::point(Point::new(10.0, 60.0), "AB", CharStyle { size: 30.0, fill: red(), ..CharStyle::default() });
        if ruby {
            t.ruby = vec![vectorcraft_doc::RubySpan::new(0, 2, "xyz")];
        }
        t
    };
    let contours = |gs: &[vectorcraft_text::PositionedGlyph]| {
        gs.iter().flat_map(|g| g.outline.elements()).filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count()
    };
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &typed(true));
    let reading = contours(&lay.ruby);
    assert!(reading >= 3, "the reading has ink: {reading}");
    let doc = |ruby: bool| doc_with(100.0, 100.0, vec![Node::new(NodeId(0), NodeKind::Text(Box::new(typed(ruby))))]);
    let moves = |d: &Document| count(&kinds(&emf_records(&emf(d).bytes)), crate::emf::MOVETOEX);
    assert_eq!(moves(&doc(true)), moves(&doc(false)) + reading, "the reading's contours are written");
    assert!(wmf(&doc(true)).bytes.len() > wmf(&doc(false)).bytes.len(), "WMF writes the reading too");
}

#[test]
fn wmf_placeable_header_checksum_and_record_sizes() {
    let d = doc_with(72.0, 144.0, vec![path(shapes::ellipse(Rect::new(10.0, 10.0, 60.0, 40.0)), red(), blue(), 1.0)]);
    let out = wmf(&d);
    let b = &out.bytes;
    assert_eq!(sniff(b), Some(Kind::Wmf));
    assert_eq!(u32_at(b, 0), 0x9AC6_CDD7);
    // The box in logical units at 1440 an inch: 1 × 2 inches.
    assert_eq!([u16_at(b, 6), u16_at(b, 8), u16_at(b, 10), u16_at(b, 12)], [0, 0, 1440, 2880]);
    assert_eq!(u16_at(b, 14), 1440);
    let xor = (0..10).fold(0u16, |a, i| a ^ u16_at(b, 2 * i));
    assert_eq!(u16_at(b, 20), xor, "the checksum");
    assert_eq!(crate::wmf::checksum(&b[..20]), xor);
    // The WMF header: a memory metafile of 9 words, its size in words, the largest record.
    assert_eq!([u16_at(b, 22), u16_at(b, 24), u16_at(b, 26)], [1, 9, 0x0300]);
    assert_eq!(u32_at(b, 28) as usize * 2, b.len() - 22);
    let recs = wmf_records(b);
    let largest = recs.iter().map(|r| r.1.len() / 2).max().unwrap();
    assert_eq!(u32_at(b, 34) as usize, largest, "MaxRecord");
    assert_eq!(recs.last().map(|r| (r.0, r.1.len())), Some((0, 6)), "META_EOF ends the file");
    // Objects: the null pen and brush, the brush, the pen.
    assert_eq!(u16_at(b, 32), 4);
    // The curve is flattened into a polygon, its outline a polygon drawn with the null brush.
    let poly = recs.iter().find(|r| r.0 == 0x0538).unwrap().1;
    assert_eq!(u16_at(poly, 6), 1);
    assert!(u16_at(poly, 8) > 16, "many points");
    assert!(recs.iter().any(|r| r.0 == 0x0324));
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
}

#[test]
fn wmf_sizes_fit_sixteen_bits() {
    assert_eq!(crate::wmf::units_per_inch(612.0, 792.0), 1440);
    let inch = crate::wmf::units_per_inch(3000.0, 1000.0);
    assert!(inch < 1440 && f64::from(inch) * 3000.0 / 72.0 <= 32767.0);
    let d = doc_with(3000.0, 1000.0, vec![path(shapes::rectangle(Rect::new(0.0, 0.0, 3000.0, 1000.0)), red(), Paint::None, 0.0)]);
    let b = wmf(&d).bytes;
    assert_eq!(u16_at(&b, 14), inch);
    assert!(u16_at(&b, 10) <= 32767);
}

#[test]
fn wmf_reports_what_it_leaves_out() {
    let g = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
    let mut clip = path(shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Paint::None, Paint::None, 0.0);
    if let NodeKind::Path { clipping, .. } = &mut clip.kind {
        *clipping = true;
    }
    let inside = path(shapes::rectangle(Rect::new(5.0, 5.0, 40.0, 40.0)), g, Paint::None, 0.0);
    let group = Node::new(NodeId(0), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(inside)], clip: true });
    let mut d = doc_with(100.0, 100.0, vec![group]);
    let img = image_node(&mut d, png(2, 2, [0, 0, 0, 10]), 2, 2, Affine::translate((50.0, 50.0)) * Affine::rotate(0.5));
    add(&mut d, img);
    let w = wmf(&d).warnings.join("\n");
    for what in ["clipping masks", "gradients", "transparency", "rotate"] {
        assert!(w.contains(what), "{what}: {w}");
    }
    // EMF keeps all of it.
    let e = emf(&d).warnings;
    assert!(e.is_empty(), "{e:?}");
}

#[test]
fn hidden_and_offboard_art_is_left_out() {
    let mut hidden = path(shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), red(), Paint::None, 0.0);
    hidden.visible = false;
    let away = path(shapes::rectangle(Rect::new(500.0, 500.0, 510.0, 510.0)), red(), Paint::None, 0.0);
    let d = doc_with(100.0, 100.0, vec![hidden, away]);
    let k = kinds(&emf_records(&emf(&d).bytes));
    assert!(!k.contains(&62) && !k.contains(&39));
    assert!(export(&d, Rect::new(0.0, 0.0, 0.0, 10.0), Kind::Emf).is_err(), "an empty region");
}

#[test]
fn an_instance_whose_art_reaches_the_artboard_is_written() {
    // The symbol's red rectangle, (100, 100) to (140, 140), drawn from (-50, -50): on the artboard,
    // while the 20 × 20 square around the origin, moved to (-60, -60) to (-40, -40), is off it.
    let inst = Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: "Far".into(), xf: Affine::translate((-50.0, -50.0)) });
    let mut d = doc_with(100.0, 100.0, vec![inst]);
    let art = path(shapes::rectangle(Rect::new(100.0, 100.0, 140.0, 140.0)), red(), Paint::None, 0.0);
    d.symbols.push(vectorcraft_doc::Symbol { name: "Far".into(), art: Arc::new(art) });
    let k = kinds(&emf_records(&emf(&d).bytes));
    assert!(k.contains(&39), "a brush fills the art: {k:?}");
}

/// A pattern `name` repeating `art` every point.
fn point_tiles(name: &str, art: Node) -> PatternDef {
    let mut def = PatternDef::new(name, vec![Arc::new(art)]);
    def.tile = Rect::new(0.0, 0.0, 1.0, 1.0);
    def
}

fn pattern(name: &str) -> Paint {
    Paint::Pattern { pattern: name.into(), xf: Affine::IDENTITY }
}

#[test]
fn patterns_inside_pattern_tiles_share_one_tile_budget() {
    // About 3,700 tiles of a 12 pt square, each filled with about 140 tiles of a dot: over half a
    // million fills when every tile is written.
    let square = path(shapes::rectangle(Rect::new(0.0, 0.0, 12.0, 12.0)), pattern("dots"), Paint::None, 0.0);
    let mut d = doc_with(50.0, 50.0, vec![path(shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), pattern("squares"), Paint::None, 0.0)]);
    d.patterns.push(point_tiles("dots", path(shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), red(), Paint::None, 0.0)));
    d.patterns.push(point_tiles("squares", square));
    let (ops, warnings) = crate::scene::Scene::new(&d, d.artboards[0].rect, true).run();
    assert!(ops.len() < 300_000, "{} operations", ops.len());
    assert!(warnings.iter().any(|w| w.contains("one colour")), "{warnings:?}");
    // The tiles that were written are reported too (#3850).
    assert!(warnings.iter().any(|w| w.contains("written as their tiles")), "{warnings:?}");

    // A tile filled with its own pattern multiplies at every level down to the nesting limit.
    let square = path(shapes::rectangle(Rect::new(0.0, 0.0, 4.0, 4.0)), pattern("self"), Paint::None, 0.0);
    let mut d = doc_with(10.0, 10.0, vec![path(shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), pattern("self"), Paint::None, 0.0)]);
    d.patterns.push(point_tiles("self", square));
    let (ops, _) = crate::scene::Scene::new(&d, d.artboards[0].rect, true).run();
    assert!(ops.len() < 300_000, "{} operations", ops.len());
}

/// A 24 pt tile clipped to itself, red on its left half and white on its right: the striped
/// pattern of #3911.
fn stripes(name: &str, extra: Option<Node>) -> PatternDef {
    let cell = Rect::new(0.0, 0.0, 24.0, 24.0);
    let clip = Node::new(
        NodeId(0),
        NodeKind::Path { path: shapes::rectangle(cell), rule: vectorcraft_geom::FillRule::NonZero, live: None, clipping: true, guide: false },
    );
    let mut children = vec![
        Arc::new(clip),
        Arc::new(path(shapes::rectangle(Rect::new(0.0, 0.0, 12.0, 24.0)), red(), Paint::None, 0.0)),
        Arc::new(path(shapes::rectangle(Rect::new(12.0, 0.0, 24.0, 24.0)), Paint::solid(Color::rgb8(255, 255, 255)), Paint::None, 0.0)),
    ];
    children.extend(extra.map(Arc::new));
    let group = Node::new(NodeId(0), NodeKind::Group { children, clip: true });
    let mut def = PatternDef::new(name, vec![Arc::new(group)]);
    def.tile = cell;
    def
}

/// A 360 × 260 pt rectangle filled with `def`, turned 45°.
fn striped_doc(def: PatternDef) -> Document {
    let paint = Paint::Pattern { pattern: def.name.clone(), xf: Affine::rotate(std::f64::consts::FRAC_PI_4) };
    let mut d = doc_with(400.0, 300.0, vec![path(shapes::rectangle(Rect::new(20.0, 20.0, 380.0, 280.0)), paint, Paint::None, 0.0)]);
    d.patterns.push(def);
    d
}

fn fills(ops: &[crate::scene::Op]) -> Vec<(&vectorcraft_geom::BezPath, [u8; 3])> {
    ops.iter()
        .filter_map(|op| match op {
            crate::scene::Op::Fill { path, rgb, .. } => Some((path, *rgb)),
            _ => None,
        })
        .collect()
}

fn clips(ops: &[crate::scene::Op]) -> usize {
    ops.iter().filter(|op| matches!(op, crate::scene::Op::Clip { .. })).count()
}

/// #3911: a pattern whose tiles stay apart is written as one shape for each part of its tile,
/// clipped once to what it fills, instead of every tile's parts in a clip of their own.
#[test]
fn pattern_tiles_that_stay_apart_are_one_shape_for_each_part() {
    let d = striped_doc(stripes("stripes", None));
    let (ops, warnings) = crate::scene::Scene::new(&d, d.artboards[0].rect, true).run();
    let f = fills(&ops);
    assert_eq!(f.iter().map(|(_, rgb)| *rgb).collect::<Vec<_>>(), [[255, 0, 0], [255, 255, 255]], "red, then white");
    assert_eq!(clips(&ops), 2, "the page's clip and one to the filled rectangle");
    // Every tile's half is in the merged shape: one figure for each tile, red and white alike.
    let figures = |bp: &vectorcraft_geom::BezPath| bp.elements().iter().filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count();
    let tiles = d.pattern("stripes").unwrap().instances_in(Affine::rotate(std::f64::consts::FRAC_PI_4), Rect::new(20.0, 20.0, 380.0, 280.0)).len();
    assert!(tiles > 200, "{tiles} tiles");
    assert_eq!((figures(f[0].0), figures(f[1].0)), (tiles, tiles));
    assert!(warnings.iter().any(|w| w.contains("one shape for each part of the tile")), "{warnings:?}");
    // The EMF: two fills and one saved clip, about half the bytes the tiles took apart.
    let bytes = emf(&d).bytes;
    let k = kinds(&emf_records(&bytes));
    assert_eq!((count(&k, 62), count(&k, 33)), (2, 2), "FILLPATH and SAVEDC records");
}

/// Art reaching past its tile overlaps the next tile's, so the tiles keep their own shapes and
/// clips, in paint order.
#[test]
fn pattern_art_past_its_tile_keeps_each_tile_apart() {
    let mut def = stripes("wide", None);
    // The tile is smaller than the art: neighbours overlap.
    def.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
    let d = striped_doc(def);
    let (ops, warnings) = crate::scene::Scene::new(&d, d.artboards[0].rect, true).run();
    assert!(fills(&ops).len() > 400, "{} fills", fills(&ops).len());
    assert!(clips(&ops) > 200, "each tile's clip stays");
    assert!(warnings.iter().any(|w| w.contains("one shape each")), "{warnings:?}");
}

/// Hex tilings overlap their tiles' boxes, and a stroke may reach past a clip: both keep each
/// tile apart.
#[test]
fn hex_tilings_and_stroked_tiles_keep_each_tile_apart() {
    let mut hex = stripes("hex", None);
    hex.tile_type = vectorcraft_doc::TileType::HexByRow;
    let d = striped_doc(hex);
    let (ops, _) = crate::scene::Scene::new(&d, d.artboards[0].rect, true).run();
    assert!(fills(&ops).len() > 400, "hex: {} fills", fills(&ops).len());

    let line = path(shapes::rectangle(Rect::new(4.0, 4.0, 20.0, 20.0)), Paint::None, blue(), 2.0);
    let d = striped_doc(stripes("stroked", Some(line)));
    let (ops, _) = crate::scene::Scene::new(&d, d.artboards[0].rect, true).run();
    assert!(fills(&ops).len() > 400 && clips(&ops) > 200, "stroked: {} fills, {} clips", fills(&ops).len(), clips(&ops));
}
