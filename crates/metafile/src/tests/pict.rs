//! QuickDraw pictures read from hand-built opcodes: versions 1 and 2, with and without the file's
//! application data; shapes, lines, regions, patterns, text, bitmaps, pixel maps and QuickTime
//! JPEG images; and pictures that are cut short, damaged or never end.

use vectorcraft_doc::{AppearanceItem, Document, Node, NodeKind};
use vectorcraft_geom::{Point, Rect};

use crate::import_pict;

/// Big-endian words.
fn be(v: &[i16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_be_bytes()).collect()
}

/// A QuickDraw rectangle: top, left, bottom, right.
fn rect(top: i16, left: i16, bottom: i16, right: i16) -> Vec<u8> {
    be(&[top, left, bottom, right])
}

/// A version 2 PICT file: 512 bytes of application data, then a picture of `frame` (top, left,
/// bottom, right) holding `ops`, each padded to an even length, and the end opcode.
fn v2(frame: [i16; 4], ops: &[(u16, Vec<u8>)]) -> Vec<u8> {
    let mut b = vec![0xA5; 512];
    b.extend([0, 0]);
    b.extend(be(&frame));
    b.extend([0x00, 0x11, 0x02, 0xFF, 0x0C, 0x00]);
    b.extend(be(&[-1, 0]));
    b.extend([0; 20]);
    for (op, data) in ops {
        b.extend(op.to_be_bytes());
        b.extend(data);
        if b.len() % 2 == 1 {
            b.push(0);
        }
    }
    b.extend([0x00, 0xFF]);
    b
}

/// An RGB foreground color.
fn fg(r: u16, g: u16, b: u16) -> (u16, Vec<u8>) {
    (0x1A, [r, g, b].iter().flat_map(|c| c.to_be_bytes()).collect())
}

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

fn stroke(n: &Node) -> Option<([u8; 3], f64)> {
    n.appearance.items.iter().find_map(|i| match i {
        AppearanceItem::Stroke(s) => s.paint.color().map(|c| {
            let [r, g, b, _] = c.to_rgba8(1.0);
            ([r, g, b], s.width)
        }),
        _ => None,
    })
}

/// The images of `d` with their pixels.
fn images(d: &Document) -> Vec<(Node, image::RgbaImage)> {
    all(d)
        .into_iter()
        .filter_map(|n| match &n.kind {
            NodeKind::Image(im) => {
                let png = image::load_from_memory(&d.images[&im.key].bytes).unwrap().to_rgba8();
                Some((n.clone(), png))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_version_2_picture_opens_as_shapes_in_its_frame() {
    let bytes = v2(
        [10, 20, 110, 220],
        &[
            fg(0xFFFF, 0, 0),
            // paintRect
            (0x31, rect(20, 30, 60, 80)),
            (0x07, be(&[4, 4])),
            fg(0, 0, 0xFFFF),
            // frameRect: stroked inside the rectangle.
            (0x30, rect(20, 120, 80, 200)),
            // A line, and one from its end: one path.
            (0x20, be(&[90, 40, 90, 140])),
            (0x21, be(&[100, 140])),
        ],
    );
    assert!(crate::is_pict(&bytes));
    let p = import_pict(&bytes).unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let a = p.document.artboards[0].rect;
    assert!(near(a, Rect::new(0.0, 0.0, 200.0, 100.0), 1e-9), "{a:?}");
    let v = paths(&p.document);
    assert_eq!(v.len(), 3, "{v:?}");
    assert_eq!(fill_rgb(&v[0]), Some([255, 0, 0]));
    assert!(near(v[0].geometric_bounds().unwrap(), Rect::new(10.0, 10.0, 60.0, 50.0), 1e-6));
    assert!(v[0].appearance.stroke().is_none());
    // The pen's outer edge is the rectangle.
    assert_eq!(stroke(&v[1]), Some(([0, 0, 255], 4.0)));
    assert!(near(v[1].geometric_bounds().unwrap(), Rect::new(102.0, 12.0, 178.0, 68.0), 1e-6));
    assert!(fill_rgb(&v[1]).is_none());
    // The pen hangs below and to the right of the line.
    assert_eq!(stroke(&v[2]), Some(([0, 0, 255], 4.0)));
    assert!(near(v[2].geometric_bounds().unwrap(), Rect::new(22.0, 82.0, 122.0, 92.0), 1e-6));
}

#[test]
fn a_bare_picture_and_a_version_1_picture_open() {
    // A version 2 picture without the file's application data.
    let file = v2([0, 0, 50, 50], &[(0x31, rect(10, 10, 20, 20))]);
    let bare = &file[512..];
    assert!(crate::is_pict(bare));
    assert_eq!(paths(&import_pict(bare).unwrap().document).len(), 1);
    // Version 1: byte opcodes, known by name only.
    let mut v1 = vec![0, 0];
    v1.extend(rect(0, 0, 40, 60));
    v1.extend([0x11, 0x01, 0x31]);
    v1.extend(rect(5, 5, 15, 25));
    v1.extend([0xA0, 0x00, 0x05, 0x50]);
    v1.extend(rect(20, 20, 30, 30));
    v1.push(0xFF);
    assert!(!crate::is_pict(&v1));
    let p = import_pict(&v1).unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    assert_eq!(p.document.artboards[0].rect.size(), (60.0, 40.0).into());
    let v = paths(&p.document);
    assert_eq!(v.len(), 2);
    assert!(near(v[0].geometric_bounds().unwrap(), Rect::new(5.0, 5.0, 25.0, 15.0), 1e-6));
    // The default pen: one point, black.
    assert_eq!(stroke(&v[1]), Some(([0, 0, 0], 1.0)));
    // Inside a file's application data too.
    let mut file = vec![0; 512];
    file.extend(&v1);
    assert_eq!(paths(&import_pict(&file).unwrap().document).len(), 2);
}

#[test]
fn files_that_are_not_pictures_are_refused() {
    assert_eq!(import_pict(&[0; 600]).unwrap_err(), "not a PICT file");
    assert_eq!(import_pict(b"").unwrap_err(), "not a PICT file");
    // An empty frame.
    let mut b = vec![0, 0];
    b.extend(rect(10, 10, 10, 50));
    b.extend([0x11, 0x01, 0xFF]);
    assert!(import_pict(&b).is_err());
}

#[test]
fn rounded_rectangles_ovals_arcs_and_same_shapes() {
    let bytes = v2(
        [0, 0, 100, 100],
        &[
            (0x0B, be(&[20, 20])),
            (0x41, rect(0, 0, 50, 50)),
            (0x51, rect(50, 0, 100, 50)),
            // A wedge from 12 to 3 o'clock.
            (0x61, [rect(0, 50, 100, 150), be(&[0, 90])].concat()),
            fg(0, 0xFFFF, 0),
            // paintSameRect after paintRect: the same rectangle.
            (0x31, rect(60, 60, 70, 80)),
            (0x39, vec![]),
        ],
    );
    let p = import_pict(&bytes).unwrap();
    let v = paths(&p.document);
    assert_eq!(v.len(), 5, "{v:?}");
    assert!(near(v[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 50.0, 50.0), 1e-6));
    assert!(near(v[1].geometric_bounds().unwrap(), Rect::new(0.0, 50.0, 50.0, 100.0), 1e-3));
    assert!(near(v[2].geometric_bounds().unwrap(), Rect::new(100.0, 0.0, 150.0, 50.0), 1e-3), "{:?}", v[2].geometric_bounds());
    assert_eq!(v[3].geometric_bounds(), v[4].geometric_bounds());
    assert_eq!(fill_rgb(&v[4]), Some([0, 255, 0]));
}

#[test]
fn polygons_and_regions_fill_and_clip() {
    // An L: the top band 20 wide, the bottom band 10.
    let region = [be(&[36]), rect(0, 0, 20, 20), be(&[0, 0, 20, 0x7FFF, 10, 10, 20, 0x7FFF, 20, 0, 10, 0x7FFF, 0x7FFF])].concat();
    let poly = [be(&[22]), rect(0, 0, 40, 40), be(&[0, 0, 40, 40, 0, 40])].concat();
    let bytes = v2([0, 0, 100, 100], &[(0x81, region.clone()), (0x71, poly), (0x01, region), (0x31, rect(0, 0, 100, 100))]);
    let p = import_pict(&bytes).unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let v = paths(&p.document);
    assert_eq!(v.len(), 3);
    let NodeKind::Path { path, .. } = &v[0].kind else { panic!("a path") };
    assert_eq!(path.subpaths.len(), 2, "two bands");
    assert!(near(v[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 20.0, 20.0), 1e-9));
    assert!(near(v[1].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 40.0, 40.0), 1e-9));
    // The rectangle after the clip region is clipped to it.
    let layer = p.document.layers[0].children().unwrap();
    let NodeKind::Group { children, clip: true } = &layer[2].kind else { panic!("a clipping group: {:?}", layer[2].kind) };
    assert!(near(children[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 20.0, 20.0), 1e-9));
}

#[test]
fn patterns_fill_with_their_average_color() {
    let gray = vec![0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55, 0xAA, 0x55];
    let bytes = v2(
        [0, 0, 10, 10],
        &[(0x0A, gray), (0x34, rect(0, 0, 10, 10)), (0x12, [be(&[2]), vec![0; 8], be(&[0, -1, 0])].concat()), (0x32, rect(0, 0, 5, 5))],
    );
    let p = import_pict(&bytes).unwrap();
    let v = paths(&p.document);
    assert_eq!(fill_rgb(&v[0]), Some([128, 128, 128]));
    // eraseRect with an RGB background pattern.
    assert_eq!(fill_rgb(&v[1]), Some([0, 255, 0]));
    assert!(p.warnings.iter().any(|w| w.contains("average of their colors")), "{:?}", p.warnings);
}

#[test]
fn text_comes_in_as_point_type_in_the_named_font() {
    let name = [be(&[9, 300]), vec![6], b"Geneva".to_vec()].concat();
    let bytes = v2(
        [0, 0, 100, 200],
        &[(0x2C, name), (0x03, be(&[300])), (0x0D, be(&[18])), (0x04, vec![1]), (0x28, [be(&[50, 10]), vec![4], b"Caf\x8E".to_vec()].concat())],
    );
    let p = import_pict(&bytes).unwrap();
    let texts: Vec<_> = all(&p.document).into_iter().filter(|n| matches!(n.kind, NodeKind::Text(_))).collect();
    let NodeKind::Text(t) = &texts[0].kind else { panic!("text") };
    assert_eq!(t.plain_text(), "Café");
    let st = &t.runs[0].style;
    assert_eq!((st.font_family.as_str(), st.font_style.as_str(), st.size), ("Geneva", "Bold", 18.0));
    // The text location is the baseline's start.
    assert_eq!(t.xf.translation(), Point::new(10.0, 50.0).to_vec2());
}

/// A pixel map's fields after `rowBytes` and its bounds: version, packing, resolution, depth and
/// components.
fn pixmap_fields(pack: i16, size: i16, cmps: i16) -> Vec<u8> {
    be(&[0, pack, 0, 0, 72, 0, 72, 0, 0, size, cmps, 8, 0, 0, 0, 0, 0, 0])
}

#[test]
fn a_packed_indexed_pixel_map_becomes_an_image() {
    let table = be(&[0, 0, 0, 1, 0, -1, 0, 0, 1, 0, 0, -1]);
    // An 8 × 2 map: a run of red, then blue and red by turns.
    let rows = [vec![2, 0xF9, 0], vec![9, 7, 1, 0, 1, 0, 1, 0, 1, 0]].concat();
    let data =
        [be(&[0x8008u16 as i16]), rect(0, 0, 2, 8), pixmap_fields(0, 8, 1), table, rect(0, 0, 2, 8), rect(10, 20, 14, 36), be(&[0]), rows].concat();
    let p = import_pict(&v2([0, 0, 50, 50], &[(0x98, data)])).unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let imgs = images(&p.document);
    assert_eq!(imgs.len(), 1);
    let (n, px) = &imgs[0];
    assert_eq!(px.dimensions(), (8, 2));
    assert_eq!(px.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(px.get_pixel(0, 1).0, [0, 0, 255, 255]);
    assert_eq!(px.get_pixel(1, 1).0, [255, 0, 0, 255]);
    // Drawn into its destination, twice its size.
    assert!(near(n.geometric_bounds().unwrap(), Rect::new(20.0, 10.0, 36.0, 14.0), 1e-9));
}

#[test]
fn direct_pixel_maps_and_masked_bitmaps() {
    // 32-bit, packed by component planes: red, then green.
    let row = vec![7, 5, 255, 0, 0, 255, 0, 0];
    let direct =
        [be(&[0, 0xFF, 0x8008u16 as i16]), rect(0, 0, 1, 2), pixmap_fields(4, 32, 3), rect(0, 0, 1, 2), rect(0, 0, 1, 2), be(&[0]), row].concat();
    // A one-bit bitmap ORed in (its clear bits left out), masked by a region.
    let mask = [be(&[10]), rect(0, 0, 1, 1)].concat();
    let bitmap = [be(&[2]), rect(0, 0, 1, 2), rect(0, 0, 1, 2), rect(5, 0, 6, 2), be(&[1]), mask, vec![0x80, 0]].concat();
    let p = import_pict(&v2([0, 0, 10, 10], &[(0x9A, direct), (0x91, bitmap)])).unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let imgs = images(&p.document);
    assert_eq!(imgs.len(), 2);
    assert_eq!(imgs[0].1.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(imgs[0].1.get_pixel(1, 0).0, [0, 255, 0, 255]);
    assert_eq!(imgs[1].1.get_pixel(0, 0).0, [0, 0, 0, 255]);
    assert_eq!(imgs[1].1.get_pixel(1, 0).0[3], 0, "the clear bit is left out");
    let layer = p.document.layers[0].children().unwrap();
    assert!(matches!(layer[1].kind, NodeKind::Group { clip: true, .. }), "the mask region clips the bitmap");
}

/// A pixel map's rows may be only a little longer than its pixels need: a map one pixel wide
/// with rows of 16,383 bytes is left out, and a pixel pattern like it fills with its bitmap.
#[test]
fn rows_far_longer_than_their_pixels_are_refused() {
    let table = be(&[0, 0, 0, 1, 0, -1, 0, 0, 1, -1, 0, 0]);
    // A row of 16,383 ones: 127 runs of 128 and one of 127, after its byte count.
    let mut row = be(&[256]);
    (0..127).for_each(|_| row.extend([0x81, 1]));
    row.extend([0x82, 1]);
    let rows = row.repeat(4);
    let map = [be(&[0xBFFFu16 as i16]), rect(0, 0, 4, 1), pixmap_fields(0, 8, 1), table.clone()].concat();
    let data = [map, rect(0, 0, 4, 1), rect(0, 0, 4, 1), be(&[0]), rows].concat();
    let p = import_pict(&v2([0, 0, 10, 10], &[(0x98, data)])).unwrap();
    assert!(images(&p.document).is_empty());
    assert!(p.warnings.iter().any(|w| w.contains("couldn't be read")), "{:?}", p.warnings);

    let pattern = [be(&[1]), vec![0xFF; 8], be(&[0xBFFFu16 as i16]), rect(0, 0, 1, 1), pixmap_fields(0, 8, 1), table, row].concat();
    let p = import_pict(&v2([0, 0, 10, 10], &[(0x12, pattern), (0x32, rect(0, 0, 5, 5))])).unwrap();
    assert_eq!(fill_rgb(&paths(&p.document)[0]), Some([0, 0, 0]), "the pattern's bitmap in the foreground color");
}

/// BitsRect rows are stored unpacked, also when a row is 8 bytes or longer.
#[test]
fn bitmap_rows_of_eight_bytes_or_more_are_read_unpacked() {
    let row = vec![0x80, 0, 0, 0, 0, 0, 0, 0];
    let bitmap = [be(&[8]), rect(0, 0, 1, 16), rect(0, 0, 1, 16), rect(0, 0, 1, 16), be(&[0]), row].concat();
    let p = import_pict(&v2([0, 0, 10, 20], &[(0x90, bitmap), (0x31, rect(2, 2, 4, 4))])).unwrap();
    assert!(p.warnings.is_empty(), "{:?}", p.warnings);
    let imgs = images(&p.document);
    assert_eq!(imgs.len(), 1);
    assert_eq!(imgs[0].1.dimensions(), (16, 1));
    assert_eq!(imgs[0].1.get_pixel(0, 0).0, [0, 0, 0, 255]);
    assert_eq!(imgs[0].1.get_pixel(1, 0).0, [255, 255, 255, 255]);
    assert_eq!(paths(&p.document).len(), 1, "the rectangle after it");
}

/// An empty clip region clips away what follows it.
#[test]
fn an_empty_clip_region_hides_what_follows() {
    let empty = [be(&[10]), rect(0, 0, 0, 0)].concat();
    let p = import_pict(&v2([0, 0, 50, 50], &[(0x01, empty), (0x31, rect(0, 0, 10, 10))])).unwrap();
    let layer = p.document.layers[0].children().unwrap();
    let NodeKind::Group { children, clip: true } = &layer[0].kind else { panic!("a clipping group: {:?}", layer[0].kind) };
    let b = children[0].geometric_bounds().unwrap();
    assert_eq!((b.width(), b.height()), (0.0, 0.0), "{b:?}");
}

/// A QuickTime image of `ctype` with `matrix` (a, b, c, d, tx, ty in points) and source rectangle
/// `w` × `h`, holding `data`.
fn quicktime(ctype: &[u8; 4], matrix: [f64; 6], w: i16, h: i16, data: &[u8]) -> Vec<u8> {
    let fixed = |v: f64| ((v * 65536.0) as i32).to_be_bytes();
    let [a, b, c, d, tx, ty] = matrix;
    let mut q = be(&[0]);
    for v in [a, b, 0.0, c, d, 0.0, tx, ty] {
        q.extend(fixed(v));
    }
    q.extend((1i32 << 30).to_be_bytes());
    q.extend([0; 4]);
    q.extend([0; 8]);
    q.extend(be(&[0]));
    q.extend(rect(0, 0, h, w));
    q.extend([0; 4]);
    q.extend([0; 4]);
    let mut desc = vec![];
    desc.extend(86u32.to_be_bytes());
    desc.extend(ctype);
    desc.extend([0; 24]);
    desc.extend(be(&[w, h]));
    desc.extend([0; 8]);
    desc.extend((data.len() as u32).to_be_bytes());
    desc.resize(86, 0);
    q.extend(desc);
    q.extend(data);
    [(q.len() as u32).to_be_bytes().to_vec(), q].concat()
}

fn red_jpeg() -> Vec<u8> {
    let img = image::RgbImage::from_pixel(4, 2, image::Rgb([255, 0, 0]));
    let mut out = std::io::Cursor::new(vec![]);
    img.write_to(&mut out, image::ImageFormat::Jpeg).unwrap();
    out.into_inner()
}

#[test]
fn quicktime_jpeg_images_are_placed_by_their_matrix() {
    let jpeg = red_jpeg();
    let ops = [
        (0x8200, quicktime(b"jpeg", [2.0, 0.0, 0.0, 2.0, 10.0, 5.0], 4, 2, &jpeg)),
        (0x8200, quicktime(b"rle ", [1.0, 0.0, 0.0, 1.0, 0.0, 0.0], 4, 2, &[1, 2, 3])),
    ];
    let p = import_pict(&v2([0, 0, 50, 50], &ops)).unwrap();
    let imgs = images(&p.document);
    assert_eq!(imgs.len(), 1);
    let (n, px) = &imgs[0];
    assert_eq!(px.dimensions(), (4, 2));
    let [r, g, b, _] = px.get_pixel(1, 1).0;
    assert!(r > 240 && g < 16 && b < 16, "{r} {g} {b}");
    assert!(near(n.geometric_bounds().unwrap(), Rect::new(10.0, 5.0, 18.0, 9.0), 1e-9));
    assert!(p.warnings.iter().any(|w| w.contains("other than JPEG")), "{:?}", p.warnings);
}

/// An image whose matrix flattens it is left out, and adds nothing to the document's images.
#[test]
fn a_flattened_image_adds_no_image_data() {
    let p = import_pict(&v2([0, 0, 50, 50], &[(0x8200, quicktime(b"jpeg", [0.0; 6], 4, 2, &red_jpeg()))])).unwrap();
    assert!(images(&p.document).is_empty());
    assert!(p.document.images.is_empty(), "{:?}", p.document.images.keys().collect::<Vec<_>>());
}

#[test]
fn pictures_cut_short_keep_what_came_before() {
    let full = v2([0, 0, 50, 50], &[(0x31, rect(0, 0, 10, 10)), (0x31, rect(20, 20, 30, 30))]);
    // Cut inside the second rectangle.
    let cut = &full[..full.len() - 6];
    let p = import_pict(cut).unwrap();
    assert_eq!(paths(&p.document).len(), 1);
    assert!(p.warnings.iter().any(|w| w.contains("cut short")), "{:?}", p.warnings);
    // A long comment longer than the file.
    let p = import_pict(&v2([0, 0, 50, 50], &[(0x31, rect(0, 0, 10, 10)), (0xA1, be(&[100, 5000]))])).unwrap();
    assert_eq!(paths(&p.document).len(), 1);
    assert!(p.warnings.iter().any(|w| w.contains("cut short")), "{:?}", p.warnings);
    // Nothing drawn.
    let p = import_pict(&v2([0, 0, 50, 50], &[(0xA0, be(&[100]))])).unwrap();
    assert!(p.warnings.iter().any(|w| w.contains("draws nothing")), "{:?}", p.warnings);
}

#[test]
fn reserved_opcodes_are_skipped_by_their_lengths() {
    let ops = [
        // A 16-bit length, a fixed length for Apple's reserved words, and a 32-bit length.
        (0x0024, be(&[4, 1, 2])),
        (0x0200, be(&[1, 2])),
        (0x00D0, [8u32.to_be_bytes().to_vec(), vec![9; 8]].concat()),
        (0x8201, [2u32.to_be_bytes().to_vec(), vec![9; 2]].concat()),
        (0x31, rect(0, 0, 10, 10)),
    ];
    let p = import_pict(&v2([0, 0, 50, 50], &ops)).unwrap();
    assert_eq!(paths(&p.document).len(), 1, "{:?}", p.warnings);
}

/// A picture of nothing but no-ops stops at the opcode limit.
#[test]
fn a_picture_that_never_ends_stops_at_the_opcode_limit() {
    let mut b = v2([0, 0, 50, 50], &[(0x31, rect(0, 0, 10, 10))]);
    b.truncate(b.len() - 2);
    b.resize(b.len() + 2 * ((1 << 22) + 1), 0);
    let p = import_pict(&b).unwrap();
    assert_eq!(paths(&p.document).len(), 1);
    assert!(p.warnings.iter().any(|w| w.contains("more opcodes")), "{:?}", p.warnings);
}

/// Regions share one budget of inversion points per picture: past it, a region is left out.
#[test]
fn regions_past_the_picture_budget_are_left_out() {
    // One scanline of 16,000 inversion points: 8,000 rectangles.
    let mut words = vec![0];
    words.extend(0..16_000i16);
    words.extend([0x7FFF, 1, 0x7FFF, 0x7FFF]);
    let body = [rect(0, 0, 1, 16_000), be(&words)].concat();
    let region = [be(&[(body.len() + 2) as i16]), body].concat();
    let p = import_pict(&v2([0, 0, 300, 300], &[(0x81, region.clone()), (0x81, region.clone()), (0x81, region)])).unwrap();
    assert!(p.warnings.iter().any(|w| w.contains("regions larger")), "{:?}", p.warnings);
    let v = paths(&p.document);
    assert_eq!(v.len(), 2);
    let NodeKind::Path { path, .. } = &v[0].kind else { panic!("a path") };
    assert_eq!(path.subpaths.len(), 8_000);
}
