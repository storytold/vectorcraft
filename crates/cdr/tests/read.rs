#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
//! Reading synthetic drawings of every version the reader takes, plain and compressed, and
//! hostile files.

use proptest::prelude::*;
use vectorcraft_cdr::model::{Cap, Color, Geometry, LayerKind, Object, Segment};
use vectorcraft_cdr::synth::{self, V, chunk, list};
use vectorcraft_cdr::{Error, Limits, Warning, read};

fn shapes(objects: &[Object]) -> Vec<&vectorcraft_cdr::model::Shape> {
    let mut out = Vec::new();
    let mut stack: Vec<&Object> = objects.iter().rev().collect();
    while let Some(o) = stack.pop() {
        match o {
            Object::Shape(s) => out.push(&**s),
            Object::Group(kids) => stack.extend(kids.iter().rev()),
        }
    }
    out
}

/// The page and layers of the sample of `version`.
fn check_pages(d: &vectorcraft_cdr::Drawing, version: u16) {
    assert_eq!(d.version, version);
    let (w, h) = d.page_size.unwrap();
    assert!((w - 612.0).abs() < 0.1 && (h - 792.0).abs() < 0.1, "{version}: {w} {h}");
    assert_eq!(d.pages.len(), 1, "{version}");
    let art = &d.pages[0].layers[0];
    assert_eq!((art.name.as_str(), art.kind), ("Art", LayerKind::Drawing));
    assert!(matches!(art.objects[3], Object::Group(ref g) if g.len() == 2));
    assert!(d.warnings.contains(&Warning::UnsupportedObjects));
    // The master page: its guideline every page shows, and its desktop object.
    if version >= 600 {
        assert_eq!(d.guides.len(), 1);
        assert!((d.guides[0].angle - 90.0).abs() < 1e-9 && (d.guides[0].at.x - 36.0).abs() < 0.01);
    }
    assert_eq!(d.master.len(), 1);
    assert_eq!(d.master[0].kind, LayerKind::Desktop);
}

/// The shapes of the sample of `version`: rectangle, ellipse, curve, and the group's polygon and
/// arc; the text is left out.
fn check_shapes(d: &vectorcraft_cdr::Drawing, version: u16) {
    let s = shapes(&d.pages[0].layers[0].objects);
    assert_eq!(s.len(), 5, "{version}");
    assert!(matches!(s[0].geometry, Geometry::Rect { radii, .. } if radii.iter().all(|r| (r - 5.0).abs() < 0.1)));
    assert_eq!(s[0].fill, Some(Color::Rgb([1.0, 0.0, 0.0])));
    let o = s[0].outline.as_ref().unwrap();
    assert!((o.width - 2.0).abs() < 0.05 && o.cap == Cap::Round, "{version}: {o:?}");
    assert!((s[0].matrix.0[4] + 100.0).abs() < 0.1 && (s[0].matrix.0[5] - 100.0).abs() < 0.1);
    assert!(matches!(s[1].geometry, Geometry::Ellipse { .. }) && s[1].outline.is_none());
    assert!(matches!(&s[2].geometry, Geometry::Path(p) if p.subpaths[0].closed && matches!(p.subpaths[0].segments[1], Segment::Cubic(..))));
    assert!(s[2].fill.is_none());
    if version >= 600 {
        let Geometry::Path(p) = &s[3].geometry else { panic!("polygon") };
        assert_eq!(p.subpaths[0].segments.len(), 10, "five sides of two segments");
        // 5 % of 1000 mm.
        let r = p.subpaths[0].start.y;
        assert!((r - 0.05 * synth::POLYGON_SIDE).abs() < 0.5, "{r}");
    }
    assert!(matches!(&s[4].geometry, Geometry::Path(p) if p.subpaths[0].closed));
}

#[test]
fn every_version_from_5_to_13_reads_its_sample() {
    for version in [500, 600, 700, 800, 900, 1000, 1100, 1200, 1300] {
        for compress in [false, version >= 700] {
            let d = read(&synth::sample(version, compress), Limits::default()).unwrap_or_else(|e| panic!("{version} {compress}: {e}"));
            check_pages(&d, version);
            check_shapes(&d, version);
        }
    }
}

#[test]
fn versions_outside_5_to_13_and_other_files_are_refused() {
    assert_eq!(read(&synth::riff(400, &[]), Limits::default()), Err(Error::Version(400)));
    assert_eq!(read(&synth::riff(1400, &[]), Limits::default()), Err(Error::Version(1400)));
    assert_eq!(read(b"RIFF\x04\0\0\0WAVE", Limits::default()), Err(Error::NotCdr));
    assert_eq!(read(b"RIFF\x04\0\0\0CDRX", Limits::default()), Err(Error::NotCdr));
    assert!(!vectorcraft_cdr::is_cdr(b"PK\x03\x04"));
    assert!(vectorcraft_cdr::is_cdr(&synth::sample(1100, true)));
}

#[test]
fn a_lower_case_form_type_names_the_same_versions() {
    let mut file = synth::sample(600, false);
    file[8..12].copy_from_slice(b"cdr6");
    // The `vrsn` chunk agrees; without it the form type alone decides.
    assert_eq!(read(&file, Limits::default()).unwrap().version, 600);
    assert!(vectorcraft_cdr::is_cdr(&file));
    file[8..12].copy_from_slice(b"cdrX");
    assert_eq!(read(&file, Limits::default()), Err(Error::NotCdr));
}

#[test]
fn rectangles_keep_four_corner_radii_from_version_9() {
    let v = V(900);
    let c = [v.coord(100.0), v.coord(50.0), v.coord(1.0), v.coord(2.0), v.coord(3.0), v.coord(4.0)].concat();
    let file = synth::riff(900, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &[(30, c)], None)])])]);
    let d = read(&file, Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    let Geometry::Rect { radii, .. } = s[0].geometry else { panic!("{:?}", s[0].geometry) };
    for (got, want) in radii.iter().zip([1.0, 2.0, 3.0, 4.0]) {
        assert!((got - want).abs() < 0.01, "{radii:?}");
    }
    // Before version 9 one radius rounds every corner.
    let v = V(800);
    let c = [v.coord(100.0), v.coord(50.0), v.coord(6.0)].concat();
    let file = synth::riff(800, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &[(30, c)], None)])])]);
    let s = read(&file, Limits::default()).unwrap();
    let s = shapes(&s.pages[0].layers[0].objects);
    assert!(matches!(s[0].geometry, Geometry::Rect { radii, .. } if radii.iter().all(|r| (r - 6.0).abs() < 0.01)));
}

#[test]
fn version_7_guidelines_count_toward_the_object_limit() {
    let v = V(700);
    let mut guid = 1000u32.to_le_bytes().to_vec();
    guid.extend(1000u32.to_le_bytes());
    for i in 0..1000 {
        guid.extend([0; 12]);
        guid.extend(90_000_000i32.to_le_bytes());
        guid.extend([v.coord(f64::from(i)), v.coord(0.0), v.coord(f64::from(i)), v.coord(1.0), v.coord(f64::from(i)), v.coord(0.0)].concat());
    }
    let file = synth::riff(700, &[chunk(b"vrsn", 700u16.to_le_bytes().to_vec()), list(b"doc ", vec![chunk(b"guid", guid)])]);
    let d = read(&file, Limits::default()).unwrap();
    assert_eq!(d.guides.len(), 1000);
    let d = read(&file, Limits { objects: 10, ..Limits::default() }).unwrap();
    assert_eq!(d.guides.len(), 10);
    assert!(d.warnings.contains(&Warning::TooDeep));
}

/// Guidelines on any layer of the master page show on every page, not only those of its guides
/// layer: one on a drawing layer that also holds art, and one alone on the desktop layer.
#[test]
fn master_guidelines_on_any_layer_show_on_every_page() {
    let v = V(1100);
    let guide = |x: f64| {
        let mut g = vec![0; 12];
        g.extend(90_000_000i32.to_le_bytes());
        g.extend([v.coord(x), v.coord(0.0)].concat());
        synth::object(v, 0x16, &[(16000, g)], None)
    };
    let rect = synth::object(v, 1, &[(30, [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())], None);
    let master = synth::page(
        true,
        vec![synth::layer(v, [0, 0, 1, 0x98], "Art", vec![guide(10.0), rect]), synth::layer(v, [0x08, 0, 1, 0x98], "Desktop", vec![guide(20.0)])],
    );
    let file = synth::riff(1100, &[master, synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "Art", vec![])])]);
    let d = read(&file, Limits::default()).unwrap();
    let mut xs: Vec<f64> = d.guides.iter().map(|g| g.at.x).collect();
    xs.sort_by(f64::total_cmp);
    assert_eq!(xs.len(), 2, "{:?}", d.guides);
    assert!((xs[0] - 10.0).abs() < 0.01 && (xs[1] - 20.0).abs() < 0.01, "{xs:?}");
    assert_eq!(d.master.len(), 1);
    assert_eq!(d.master[0].objects.len(), 1);
}

/// An object with more arguments than the limit keeps the ones read paired with their own types:
/// the types table runs from the stored count, not from the limit.
#[test]
fn arguments_past_the_limit_leave_the_first_ones_paired_with_their_types() {
    let v = V(1100);
    let mut args = vec![(30, [v.coord(40.0), v.coord(20.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())];
    args.extend((0..69).map(|i| (500 + i, vec![0; 4])));
    let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &args, None)])])]);
    let d = read(&file, Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    assert!(
        matches!(s.first().map(|s| &s.geometry), Some(Geometry::Rect { w, h, .. }) if (w - 40.0).abs() < 0.01 && (h - 20.0).abs() < 0.01),
        "{s:?}"
    );
    assert!(d.warnings.contains(&Warning::Damaged));
}

/// Groups count toward the object limit; a group past it leaves its objects ungrouped.
#[test]
fn groups_count_toward_the_object_limit() {
    fn count(objects: &[Object]) -> usize {
        objects.iter().map(|o| 1 + if let Object::Group(kids) = o { count(kids) } else { 0 }).sum()
    }
    let v = V(1100);
    let rect = synth::object(v, 1, &[(30, [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())], None);
    let nested = list(b"grp ", vec![list(b"grp ", vec![rect])]);
    let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![nested])])]);
    let d = read(&file, Limits { objects: 2, ..Limits::default() }).unwrap();
    let objects = &d.pages[0].layers[0].objects;
    assert_eq!(count(objects), 2, "{objects:?}");
    assert!(matches!(objects.as_slice(), [Object::Group(kids)] if matches!(kids.as_slice(), [Object::Shape(_)])), "{objects:?}");
    assert!(d.warnings.contains(&Warning::TooDeep));
}

/// A rectangle, ellipse, curve or polygon stored without its coordinates is left out as damage.
#[test]
fn shapes_without_their_coordinates_are_damage() {
    let v = V(1100);
    for kind in [1, 2, 3, 0x14] {
        let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, kind, &[], None)])])]);
        let d = read(&file, Limits::default()).unwrap();
        assert!(d.pages[0].layers[0].objects.is_empty(), "{kind}");
        assert!(d.warnings.contains(&Warning::Damaged), "{kind}: {:?}", d.warnings);
    }
}

/// An object list without its `loda`, a fill or outline record that can't be read, and a `guid`
/// chunk that counts more guidelines than it holds are each damage.
#[test]
fn unreadable_records_and_lists_are_damage() {
    let v = V(1300);
    let rect = |args: Vec<(u32, Vec<u8>)>| {
        let mut args = args;
        args.push((30, [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat()));
        synth::object(v, 1, &args, None)
    };
    let on_page = |objects: Vec<synth::Node>, styles: Vec<synth::Node>| {
        let mut kids = vec![chunk(b"vrsn", 1300u16.to_le_bytes().to_vec())];
        kids.extend(styles);
        kids.push(synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", objects)]));
        read(&synth::riff(1300, &kids), Limits::default()).unwrap()
    };
    let d = on_page(vec![list(b"obj ", vec![chunk(b"flgs", vec![0, 0, 0, 0x08])])], vec![]);
    assert!(d.warnings.contains(&Warning::Damaged), "no loda: {:?}", d.warnings);
    // A fill record of only its id.
    let d = on_page(vec![rect(vec![(20, 4u32.to_le_bytes().to_vec())])], vec![list(b"filt", vec![chunk(b"fild", 4u32.to_le_bytes().to_vec())])]);
    assert_eq!(shapes(&d.pages[0].layers[0].objects)[0].fill, None);
    assert!(d.warnings.contains(&Warning::Damaged), "fill: {:?}", d.warnings);
    // A tagged outline whose first records are none of record 1.
    let mut outl = 9u32.to_le_bytes().to_vec();
    for _ in 0..16 {
        outl.extend(5u32.to_le_bytes());
        outl.extend(0u32.to_le_bytes());
    }
    let d = on_page(vec![rect(vec![(10, 9u32.to_le_bytes().to_vec())])], vec![list(b"otlt", vec![chunk(b"outl", outl)])]);
    assert!(shapes(&d.pages[0].layers[0].objects)[0].outline.is_none());
    assert!(d.warnings.contains(&Warning::Damaged), "outline: {:?}", d.warnings);
    // Version 7: a `guid` chunk counting three guidelines and holding one.
    let mut guid = 3u32.to_le_bytes().to_vec();
    guid.extend(3u32.to_le_bytes());
    guid.extend([0; 40]);
    let d = read(&synth::riff(700, &[chunk(b"vrsn", 700u16.to_le_bytes().to_vec()), list(b"doc ", vec![chunk(b"guid", guid)])]), Limits::default())
        .unwrap();
    assert_eq!(d.guides.len(), 1);
    assert!(d.warnings.contains(&Warning::Damaged), "guid: {:?}", d.warnings);
}

/// Pages keep their layers in a `gobj` list, and layers and objects their `loda` (and objects
/// their transform) in a `lgob` list. The layout with those chunks directly in the page, layer and
/// object lists reads the same.
#[test]
fn layers_and_objects_read_from_their_gobj_and_lgob_lists() {
    let v = V(1000);
    let c = [v.coord(10.0), v.coord(20.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat();
    let m = [1.0, 0.0, 0.0, 1.0, 5.0, 6.0];
    let nested = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &[(30, c.clone())], Some(m))])]);
    let plain_object = list(b"obj ", vec![chunk(b"loda", v.loda(1, &[(30, c)])), list(b"trfl", vec![chunk(b"trfd", v.trfd(&[m]))])]);
    let plain_layer = list(b"layr", vec![chunk(b"flgs", vec![0, 0, 0, 0x98]), chunk(b"loda", v.loda(0, &[(1000, b"L\0".to_vec())])), plain_object]);
    let plain = list(b"page", vec![chunk(b"flgs", vec![0, 0, 0, 0x90]), plain_layer]);
    for page in [nested, plain] {
        let d = read(&synth::riff(1000, &[page]), Limits::default()).unwrap();
        let layer = &d.pages[0].layers[0];
        assert_eq!(layer.name, "L");
        let s = shapes(&layer.objects);
        assert_eq!(s.len(), 1, "{:?}", layer.objects);
        assert!(matches!(s[0].geometry, Geometry::Rect { w, .. } if (w - 10.0).abs() < 0.01));
        assert!((s[0].matrix.0[4] - 5.0).abs() < 0.01 && (s[0].matrix.0[5] - 6.0).abs() < 0.01, "{:?}", s[0].matrix);
        assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    }
}

/// From version 7 a `LIST stlt` holds the style tables, which don't parse as chunks: it is not
/// damage.
#[test]
fn a_style_table_list_is_not_read_as_chunks() {
    let v = V(1000);
    let stlt = chunk(b"LIST", [b"stlt".to_vec(), 3u32.to_le_bytes().to_vec(), vec![0xff; 30]].concat());
    let d = read(&synth::riff(1000, &[list(b"doc ", vec![stlt, synth::mcfg(v, 100.0, 50.0)])]), Limits::default()).unwrap();
    assert!(d.warnings.is_empty(), "{:?}", d.warnings);
    assert!(d.page_size.is_some_and(|(w, h)| (w - 100.0).abs() < 0.01 && (h - 50.0).abs() < 0.01));
}

#[test]
fn linked_groups_are_left_out_with_a_warning() {
    let v = V(1100);
    let rect = synth::object(v, 1, &[(30, [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())], None);
    let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![list(b"lnkg", vec![rect])])])]);
    let d = read(&file, Limits::default()).unwrap();
    assert!(d.pages[0].layers[0].objects.is_empty());
    assert!(d.warnings.contains(&Warning::UnsupportedObjects));
}

#[test]
fn a_file_cut_short_keeps_what_it_holds() {
    let full = synth::sample(900, false);
    let cut = &full[..full.len() - 40];
    let d = read(cut, Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged));
}

#[test]
fn a_riff_length_shorter_than_the_form_type_is_damage() {
    for declared in 0u32..4 {
        let mut file = synth::sample(900, false);
        file[4..8].copy_from_slice(&declared.to_le_bytes());
        let d = read(&file, Limits::default()).unwrap();
        assert!(d.warnings.contains(&Warning::Damaged), "{declared}");
        assert!(!d.pages.is_empty(), "{declared}");
    }
}

#[test]
fn a_zip_bomb_stops_at_the_inflate_limit() {
    let v = V(1100);
    let big = chunk(b"junk", vec![0; 4 << 20]);
    let file = synth::riff(1100, &[chunk(b"vrsn", 1100u16.to_le_bytes().to_vec()), synth::cmpr(&[big, synth::mcfg(v, 10.0, 10.0)])]);
    assert!(file.len() < 64 << 10);
    let limits = Limits { inflated: 1 << 20, ..Limits::default() };
    assert_eq!(read(&file, limits), Err(Error::Limit("inflated size")));
}

#[test]
fn a_million_empty_chunks_stop_at_the_chunk_limit() {
    let kids: Vec<_> = (0..5000).map(|_| chunk(b"junk", vec![])).collect();
    let file = synth::riff(1100, &[list(b"doc ", kids)]);
    let limits = Limits { chunks: 1000, ..Limits::default() };
    assert_eq!(read(&file, limits), Err(Error::Limit("chunk count")));
    assert!(read(&file, Limits::default()).is_ok());
}

#[test]
fn deep_nesting_is_left_out_with_a_warning() {
    let v = V(1100);
    let mut inner = synth::object(v, 1, &[(30, [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())], None);
    for _ in 0..300 {
        inner = list(b"grp ", vec![inner]);
    }
    let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![inner])])]);
    let d = read(&file, Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::TooDeep));
}

#[test]
fn a_size_pool_index_out_of_range_is_damage() {
    // A compressed stream whose one chunk names pool entry 7 of a pool of one.
    let mut cmpr = synth::cmpr(&[chunk(b"mcfg", vec![0; 8])]);
    if let synth::Node::Chunk(_, body) = &mut cmpr {
        // Rebuild the stream with a bad index: the pool keeps its one size.
        let mut stream = b"mcfg".to_vec();
        stream.extend(7u32.to_le_bytes());
        stream.extend([0; 8]);
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut e, &stream).unwrap();
        let z = e.finish().unwrap();
        let pool_block = body[4 + 16 + u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize..].to_vec();
        let mut b = b"cmpr".to_vec();
        b.extend(((z.len() + 8) as u32).to_le_bytes());
        b.extend((stream.len() as u32).to_le_bytes());
        b.extend(body[12..20].to_vec());
        b.extend(b"CPng\x01\x00\x04\x00");
        b.extend(z);
        b.extend(pool_block);
        *body = b;
    }
    let d = read(&synth::riff(1100, &[cmpr]), Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged), "{:?}", d.warnings);
}

#[test]
fn backward_and_huge_loda_offsets_do_not_panic() {
    let v = V(1100);
    let mut loda = v.loda(3, &[(30, v.points(&[(0.0, 0.0, 0), (1.0, 1.0, 0x40)]))]);
    // The offsets table now points before the header and far past the end.
    let at = u32::from_le_bytes(loda[8..12].try_into().unwrap()) as usize;
    loda[at..at + 4].copy_from_slice(&40u32.to_le_bytes());
    loda[at + 4..at + 8].copy_from_slice(&2u32.to_le_bytes());
    let obj = list(b"obj ", vec![chunk(b"loda", loda)]);
    let file = synth::riff(1100, &[synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![obj])])]);
    let d = read(&file, Limits::default()).unwrap();
    assert!(d.warnings.contains(&Warning::Damaged));
}

#[test]
fn outlines_flagged_none_and_dashes_read() {
    let v = V(1300);
    let black = synth::color(2, [0, 0, 0, 100]);
    let dashed = v.outline(9, synth::Pen { line_type: 4, width: 1.0, cap: 2, join: 2 }, &black, &[3, 1]);
    let style = vec![(10, 9u32.to_le_bytes().to_vec()), (30, v.points(&[(0.0, 0.0, 0), (10.0, 0.0, 0x40)]))];
    let file = synth::riff(
        1300,
        &[
            chunk(b"vrsn", 1300u16.to_le_bytes().to_vec()),
            list(b"otlt", vec![chunk(b"outl", dashed)]),
            synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 3, &style, None)])]),
        ],
    );
    let d = read(&file, Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    let o = s[0].outline.as_ref().unwrap();
    assert_eq!(o.dashes, vec![3.0, 1.0]);
    assert_eq!((o.cap, o.join), (Cap::Square, vectorcraft_cdr::model::Join::Bevel));
}

#[test]
fn library_colors_of_version_13_keep_their_names_and_tints() {
    let v = V(1300);
    // A model 21 color (blue, green, red, tint) shown at 50 %, and its library name.
    let shown = synth::color(21, [255, 191, 128, 50]);
    let mut props = vec![1];
    props.extend(12u32.to_le_bytes());
    props.extend(shown);
    props.push(8);
    let name: Vec<u8> = "Sky Blue".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    props.extend((16 + 4 + name.len() as u32).to_le_bytes());
    props.extend([0; 16]);
    props.extend(8u32.to_le_bytes());
    props.extend(name);
    props.extend([0, 0, 0, 0, 0]);
    let mut solid = 1300u32.to_le_bytes().to_vec();
    solid.extend((props.len() as u32).to_le_bytes());
    solid.extend(props);
    let mut body = 1u16.to_le_bytes().to_vec();
    body.extend(solid);
    let mut fill = 4u32.to_le_bytes().to_vec();
    // A record of a later layout first, which the reader steps over.
    fill.extend(2400u32.to_le_bytes());
    fill.extend(16u32.to_le_bytes());
    fill.extend([0; 16]);
    fill.extend(1300u32.to_le_bytes());
    fill.extend((body.len() as u32).to_le_bytes());
    fill.extend(body);
    let style =
        vec![(20, 4u32.to_le_bytes().to_vec()), (30, [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())];
    let file = synth::riff(
        1300,
        &[
            chunk(b"vrsn", 1300u16.to_le_bytes().to_vec()),
            list(b"filt", vec![chunk(b"fild", fill)]),
            synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", vec![synth::object(v, 1, &style, None)])]),
        ],
    );
    let d = read(&file, Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    let Some(Color::Spot { name, tint, look: Some(look) }) = &s[0].fill else { panic!("{:?}", s[0].fill) };
    assert_eq!((name.as_str(), *tint), ("Sky Blue", 0.5));
    // 128/255 shown at half strength comes from about 1/255 at full strength.
    assert!(look[0] < 0.01 && (look[2] - 1.0).abs() < 1e-6, "{look:?}");
}

/// One rectangle of version `v` that uses fill `fill` and outline `outline` (0: none).
fn styled_rect(v: V, fill: u32, outline: u32) -> synth::Node {
    let mut args = vec![(30, [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat())];
    if fill != 0 {
        args.push((20, fill.to_le_bytes().to_vec()));
    }
    if outline != 0 {
        args.push((10, outline.to_le_bytes().to_vec()));
    }
    synth::object(v, 1, &args, None)
}

/// A drawing of version `v` with `styles` in its `doc` list and `objects` on one page.
fn styled_drawing(v: V, styles: Vec<synth::Node>, objects: Vec<synth::Node>) -> Vec<u8> {
    let page = synth::page(false, vec![synth::layer(v, [0, 0, 0, 0x98], "L", objects)]);
    synth::riff(v.0, &[chunk(b"vrsn", v.0.to_le_bytes().to_vec()), list(b"doc ", styles), page])
}

/// An outline's dashes count toward the read's dashes for every shape that uses it: one outline
/// of 65,535 dashes on 260 shapes would copy 17 million dashes, past the default limit.
#[test]
fn a_shared_dashed_outline_counts_its_dashes_for_every_shape() {
    let v = V(1300);
    let pen = synth::Pen { line_type: 4, width: 1.0, cap: 0, join: 0 };
    let outl = |dashes: &[u16]| list(b"otlt", vec![chunk(b"outl", v.outline(9, pen, &synth::color(2, [0, 0, 0, 100]), dashes))]);
    let file = styled_drawing(v, vec![outl(&[2, 1].repeat(10))], (0..20).map(|_| styled_rect(v, 0, 9)).collect());
    let d = read(&file, Limits { dashes: 100, ..Limits::default() }).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    assert_eq!(s.len(), 20);
    let dashed = s.iter().filter(|s| s.outline.as_ref().is_some_and(|o| o.dashes.len() == 20)).count();
    assert_eq!(dashed, 5, "100 dashes hold five outlines of 20");
    assert!(s.iter().all(|s| s.outline.as_ref().is_some_and(|o| o.dashes.is_empty() || o.dashes.len() == 20)));
    assert!(d.warnings.contains(&Warning::TooDeep), "{:?}", d.warnings);
    // An outline with more dashes than the limit is solid, not cut short.
    let file = styled_drawing(v, vec![outl(&[2, 1])], vec![styled_rect(v, 0, 9)]);
    let d = read(&file, Limits { dashes: 1, ..Limits::default() }).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    assert!(s[0].outline.as_ref().is_some_and(|o| o.dashes.is_empty()), "{:?}", s[0].outline);
    assert_eq!(d.warnings, vec![Warning::TooDeep]);
    // At the default limit.
    let file = styled_drawing(v, vec![outl(&[1; 65_535])], (0..260).map(|_| styled_rect(v, 0, 9)).collect());
    assert!(file.len() < 256 << 10);
    let d = read(&file, Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    let total: usize = s.iter().filter_map(|s| s.outline.as_ref()).map(|o| o.dashes.len()).sum();
    assert!(total <= Limits::default().dashes, "{total}");
    assert!(d.warnings.contains(&Warning::TooDeep), "{:?}", d.warnings);
}

/// A library color's name is copied for every shape that uses it: a long one keeps its first 256
/// units and a hash of the whole name.
#[test]
fn a_library_color_keeps_the_first_256_units_of_its_name() {
    let v = V(1300);
    let mut props = vec![1];
    props.extend(12u32.to_le_bytes());
    props.extend(synth::color(21, [255, 191, 128, 100]));
    props.push(8);
    let name: Vec<u8> = "N".repeat(10_000).encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    props.extend((16 + 4 + name.len() as u32).to_le_bytes());
    props.extend([0; 16]);
    props.extend(10_000u32.to_le_bytes());
    props.extend(name);
    props.extend([0, 0, 0, 0, 0]);
    let mut body = 1u16.to_le_bytes().to_vec();
    body.extend(1300u32.to_le_bytes());
    body.extend((props.len() as u32).to_le_bytes());
    body.extend(props);
    let mut fill = 4u32.to_le_bytes().to_vec();
    fill.extend(1300u32.to_le_bytes());
    fill.extend((body.len() as u32).to_le_bytes());
    fill.extend(body);
    let d = read(&styled_drawing(v, vec![list(b"filt", vec![chunk(b"fild", fill)])], vec![styled_rect(v, 4, 0)]), Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    let Some(Color::Spot { name, .. }) = &s[0].fill else { panic!("{:?}", s[0].fill) };
    assert_eq!(name.chars().count(), 256 + 17, "{name}");
    assert!(name.starts_with(&"N".repeat(256)), "{name}");
    assert!(d.warnings.contains(&Warning::TooDeep), "{:?}", d.warnings);
}

/// A transform or a uniform fill's color that is there but can't be read is damage: the shape
/// stays, without its transform or its fill, and the warnings say so.
#[test]
fn unreadable_transforms_and_fill_colors_are_damage() {
    let v = V(1100);
    let rect = [v.coord(1.0), v.coord(1.0), v.coord(0.0), v.coord(0.0), v.coord(0.0), v.coord(0.0)].concat();
    let obj = list(b"obj ", vec![chunk(b"loda", v.loda(1, &[(30, rect)])), list(b"trfl", vec![chunk(b"trfd", vec![1, 2])])]);
    let d = read(&styled_drawing(v, vec![], vec![obj]), Limits::default()).unwrap();
    let s = shapes(&d.pages[0].layers[0].objects);
    assert_eq!((s.len(), s[0].matrix), (1, vectorcraft_cdr::model::Matrix::IDENTITY));
    assert!(d.warnings.contains(&Warning::Damaged), "trfd: {:?}", d.warnings);
    // A uniform fill cut off in its color record, before version 13 and from it.
    for v in [V(1100), V(1300)] {
        let mut fill = v.fill(4, &synth::color(2, [0, 100, 0, 0]));
        fill.truncate(fill.len() - 10);
        let d = read(&styled_drawing(v, vec![list(b"filt", vec![chunk(b"fild", fill)])], vec![styled_rect(v, 4, 0)]), Limits::default()).unwrap();
        let s = shapes(&d.pages[0].layers[0].objects);
        assert_eq!(s[0].fill, None, "{}", v.0);
        assert!(d.warnings.contains(&Warning::Damaged), "fill {}: {:?}", v.0, d.warnings);
    }
}

/// The inflate limit may be as large as `usize` holds.
#[test]
fn the_largest_inflate_limit_reads_compressed_lists() {
    let d = read(&synth::sample(1100, true), Limits { inflated: usize::MAX, ..Limits::default() }).unwrap();
    check_shapes(&d, 1100);
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn never_panics_on_garbage(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        let mut b = b"RIFF".to_vec();
        b.extend(bytes);
        let _ = read(&b, Limits::default());
    }

    #[test]
    fn never_panics_on_mutated_samples(version in prop::sample::select(vec![500u16, 700, 900, 1100, 1300]), compress: bool, edits in prop::collection::vec((any::<usize>(), any::<u8>()), 1..24)) {
        let mut b = synth::sample(version, compress && version >= 700);
        let n = b.len();
        for (at, v) in edits {
            b[at % n] = v;
        }
        let _ = read(&b, Limits::default());
    }
}
