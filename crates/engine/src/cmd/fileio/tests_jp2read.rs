//! JPEG 2000 Open, Place, pixel and resolution regressions using original synthetic fixtures.

use serde_json::json;

use super::jp2read::is_jpeg2000;
use super::*;

const JP2: &[u8] = include_bytes!("../../../tests/fixtures/jpeg2000/checker.jp2");
const RAW: &[u8] = include_bytes!("../../../tests/fixtures/jpeg2000/checker-raw.j2k");
const PPI300: &[u8] = include_bytes!("../../../tests/fixtures/jpeg2000/checker-300ppi.jp2");
const COLORS: [[u8; 4]; 4] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255; 4]];

fn pixels(bytes: &[u8]) -> Vec<[u8; 4]> {
    let raster = load::raster_image(bytes).unwrap();
    image::load_from_memory(&raster.blob.bytes).unwrap().to_rgba8().pixels().map(|p| p.0).collect()
}

fn boxed(ty: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = ((data.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend(ty);
    out.extend(data);
    out
}

fn with_resolution(children: &[Vec<u8>]) -> Vec<u8> {
    let mut file = JP2.to_vec();
    let at = file.windows(4).position(|w| w == b"jp2h").unwrap() - 4;
    let old_len = u32::from_be_bytes(file[at..at + 4].try_into().unwrap()) as usize;
    let mut header = file[at + 8..at + old_len].to_vec();
    if !children.is_empty() {
        header.extend(boxed(b"res ", &children.concat()));
    }
    file.splice(at..at + old_len, boxed(b"jp2h", &header));
    file
}

fn res(ty: &[u8; 4], vr: u16, hr: u16) -> Vec<u8> {
    let mut data = Vec::new();
    for n in [vr, hr] {
        data.extend(n.to_be_bytes());
        data.extend(254u16.to_be_bytes());
    }
    data.extend([2, 2]);
    boxed(ty, &data)
}

#[test]
fn checker_opens_and_places_with_exact_pixels() {
    for (name, bytes) in [("checker.jp2", JP2), ("checker-raw.j2k", RAW)] {
        assert_eq!(pixels(bytes), COLORS, "{name}");
        let mut s = Session::new();
        let encoded = vectorcraft_format::base64_encode(bytes);
        let opened = s.execute("document.open", &json!({"name": name, "dataBase64": encoded})).unwrap();
        assert_eq!(opened["format"], "jpeg2000");
        assert_eq!(s.doc().unwrap().doc.artboards[0].rect.size(), vectorcraft_geom::Size::new(2.0, 2.0));
        let blob = s.doc().unwrap().doc.images.values().next().unwrap();
        assert_eq!(image::load_from_memory(&blob.bytes).unwrap().to_rgba8().pixels().map(|p| p.0).collect::<Vec<_>>(), COLORS);
        s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let placed = s.execute("file.place", &json!({"name": name, "dataBase64": encoded})).unwrap();
        assert_eq!(placed["format"], "jpeg2000");
        let blob = s.doc().unwrap().doc.images.values().next().unwrap();
        assert_eq!(image::load_from_memory(&blob.bytes).unwrap().to_rgba8().pixels().map(|p| p.0).collect::<Vec<_>>(), COLORS);
        assert!(load::texture_image(bytes).is_ok());
    }
}

#[test]
fn signatures_and_every_extension() {
    assert!(is_jpeg2000(JP2) && is_jpeg2000(RAW));
    for ext in ["jpf", "jpx", "jp2", "j2k", "j2c", "jpc"] {
        assert_eq!(load::detect(&format!("a.{ext}"), b"invalid").map(|f| f.id), Some("jpeg2000"));
        assert!(OPEN_EXTS.contains(&ext) && PLACE_EXTS.contains(&ext) && UNASSOCIATED_EXTS.contains(&ext));
    }
    assert_eq!(load::detect("unknown.bin", JP2).map(|f| f.id), Some("jpeg2000"));
    assert_eq!(load::detect("unknown.bin", RAW).map(|f| f.id), Some("jpeg2000"));
}

#[test]
fn resolution_boxes_use_last_valid_axis_and_72_fallback() {
    let check = |children: &[Vec<u8>], want: (f64, f64)| {
        let file = with_resolution(children);
        let got = ppi::resolution(&file).unwrap();
        assert!((got.0 - want.0).abs() < 0.01 && (got.1 - want.1).abs() < 0.01, "{got:?} != {want:?}");
        let mut s = Session::new();
        s.execute("document.open", &json!({"name": "a.jp2", "dataBase64": vectorcraft_format::base64_encode(&file)})).unwrap();
        let size = s.doc().unwrap().doc.artboards[0].rect.size();
        assert!((size.width - 144.0 / want.0).abs() < 0.01 && (size.height - 144.0 / want.1).abs() < 0.01);
    };
    check(&[], (72.0, 72.0));
    check(&[res(b"resc", 30000, 30000)], (300.0, 300.0));
    check(&[res(b"resd", 15000, 15000)], (150.0, 150.0));
    check(&[res(b"resc", 30000, 30000), res(b"resd", 15000, 15000)], (150.0, 150.0));
    check(&[res(b"resd", 15000, 15000), res(b"resc", 30000, 30000)], (300.0, 300.0));
    check(&[res(b"resc", 30000, 15000)], (150.0, 300.0));
    assert_eq!(ppi::resolution(RAW), Some((72.0, 72.0)));
    assert!((ppi::resolution(PPI300).unwrap().0 - 300.0).abs() < 0.01);
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "a.jp2", "dataBase64": vectorcraft_format::base64_encode(PPI300)})).unwrap();
    let size = s.doc().unwrap().doc.artboards[0].rect.size();
    assert!((size.width - 0.48).abs() < 0.001 && (size.height - 0.48).abs() < 0.001);
    let blob = s.doc().unwrap().doc.images.values().next().unwrap();
    let stored = ppi::resolution(&blob.bytes).unwrap();
    assert!((stored.0 - 300.0).abs() < 0.01 && (stored.1 - 300.0).abs() < 0.01);
}

#[test]
fn malformed_resolution_and_image_data_are_bounded() {
    let good = res(b"resc", 30000, 15000);
    let mut zero = res(b"resd", 15000, 15000);
    zero[10..12].copy_from_slice(&0u16.to_be_bytes());
    let got = ppi::resolution(&with_resolution(&[good.clone(), zero])).unwrap();
    assert!((got.0 - 150.0).abs() < 0.01 && (got.1 - 300.0).abs() < 0.01);
    let short = boxed(b"resd", &[0; 9]);
    assert!(ppi::resolution(&with_resolution(&[good.clone(), short])).unwrap().0 > 149.0);
    assert_eq!(ppi::resolution(&with_resolution(&[boxed(b"resc", &[0; 9])])), Some((72.0, 72.0)));
    let mut over = good.clone();
    over[..4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(ppi::resolution(&with_resolution(&[over])), Some((72.0, 72.0)));
    let mut outer_over = JP2.to_vec();
    let jp2h = outer_over.windows(4).position(|w| w == b"jp2h").unwrap() - 4;
    outer_over[jp2h..jp2h + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(ppi::resolution(&outer_over), Some((72.0, 72.0)));
    let mut extended = Vec::from(1u32.to_be_bytes());
    extended.extend(b"resd");
    extended.extend(26u64.to_be_bytes());
    extended.extend(&res(b"resd", 15000, 30000)[8..]);
    assert!(ppi::resolution(&with_resolution(&[extended])).unwrap().0 > 299.0);
    let mut to_end = res(b"resd", 15000, 30000);
    to_end[..4].copy_from_slice(&0u32.to_be_bytes());
    assert!(ppi::resolution(&with_resolution(&[to_end])).unwrap().0 > 299.0);
    for bytes in [&JP2[..12], &RAW[..4], &JP2[..JP2.len() - 5]] {
        assert!(load::raster_image(bytes).is_err());
    }
    assert!(super::jp2read::decode(JP2, 8).is_err());
    // RGBA (16 bytes) and the float component planes (48 bytes) each fit;
    // their concurrent allocation with samples and scratch does not.
    assert!(super::jp2read::decode(JP2, 80).unwrap_err().contains("too large to draw"));
    let mut huge = RAW.to_vec();
    huge[8..12].copy_from_slice(&100_000u32.to_be_bytes());
    huge[12..16].copy_from_slice(&100_000u32.to_be_bytes());
    assert!(load::raster_image(&huge).is_err());
    let mut jpx = JP2.to_vec();
    jpx.extend(boxed(b"jp2c", RAW));
    assert!(load::raster_image(&jpx).err().unwrap().to_string().contains("unsupported color space or JPX content"));
    let mut layered = JP2.to_vec();
    layered.extend(boxed(b"jpch", &[]));
    assert!(load::raster_image(&layered).err().unwrap().to_string().contains("unsupported color space or JPX content"));
    let mut wrapped = JP2.to_vec();
    wrapped.extend(boxed(b"asoc", &boxed(b"jp2c", RAW)));
    assert!(load::raster_image(&wrapped).err().unwrap().to_string().contains("unsupported color space or JPX content"));
}

#[test]
fn compatible_jpx_brand_alone_keeps_checker_pixels() {
    let mut branded = JP2.to_vec();
    branded[12..16].copy_from_slice(&24u32.to_be_bytes());
    branded.splice(32..32, b"jpx ".iter().copied());
    assert_eq!(pixels(&branded), COLORS);
}

#[test]
fn codestream_component_count_bounds_decode_planes() {
    assert_eq!(super::jp2read::decode(RAW, 1000).unwrap().pixels().map(|p| p.0).collect::<Vec<_>>(), COLORS);
    let mut many = RAW.to_vec();
    let component = many[42..45].to_vec();
    many.splice(42..42, component.repeat(17));
    many[4..6].copy_from_slice(&98u16.to_be_bytes()); // 38 + 3 * 20
    many[40..42].copy_from_slice(&20u16.to_be_bytes());
    assert!(super::jp2read::decode(&many, 1000).unwrap_err().contains("too large to draw"));
    many[0..4].copy_from_slice(&[0, 0, 0, 0]);
    assert!(super::jp2read::decode(&many, 1000).unwrap_err().contains("can't be read"));
}

fn with_header_boxes(extra: &[Vec<u8>]) -> Vec<u8> {
    let header = super::jp2read::boxes(JP2).find_map(|(ty, data)| (ty == b"jp2h").then_some(data)).unwrap();
    let mut children = header.to_vec();
    children.extend(extra.concat());
    super::jp2read::boxes(JP2)
        .map(|(ty, data)| if ty == b"jp2h" { boxed(b"jp2h", &children) } else { boxed(ty.try_into().unwrap(), data) })
        .collect::<Vec<_>>()
        .concat()
}

#[test]
fn channel_definitions_must_cover_every_channel() {
    // (channel, type, association): color channels 1 to 3, an opacity channel (type 1).
    let cdef = |defs: &[[u16; 3]]| {
        let mut data = (defs.len() as u16).to_be_bytes().to_vec();
        data.extend(defs.iter().flatten().flat_map(|v| v.to_be_bytes()));
        boxed(b"cdef", &data)
    };
    assert_eq!(pixels(&with_header_boxes(&[cdef(&[[0, 0, 1], [1, 0, 2], [2, 0, 3]])])), COLORS);
    // Channel 0 as green and channel 1 as red swap the two.
    let swapped = [[0, 255, 0, 255], [255, 0, 0, 255], [0, 0, 255, 255], [255; 4]];
    assert_eq!(pixels(&with_header_boxes(&[cdef(&[[0, 0, 2], [1, 0, 1], [2, 0, 3]])])), swapped);
    for defs in [
        vec![[0, 0, 1]],
        vec![[0, 0, 1], [1, 0, 2], [2, 1, 0]],
        vec![[0, 0, 1], [1, 0, 2], [2, 0, 3], [3, 1, 0]],
        // Opacity first, a premultiplied opacity, a repeated color, an unassociated color.
        vec![[0, 1, 0], [1, 0, 1], [2, 0, 2]],
        vec![[0, 0, 1], [1, 0, 2], [2, 2, 0]],
        vec![[0, 0, 1], [1, 0, 1], [2, 0, 3]],
        vec![[0, 0, 1], [1, 0, 2], [2, 0, 0]],
    ] {
        assert!(load::raster_image(&with_header_boxes(&[cdef(&defs)])).err().unwrap().to_string().contains("can't be read"), "{defs:?}");
    }
    // A second definition box, and a second header, are refused.
    let full = cdef(&[[0, 0, 1], [1, 0, 2], [2, 0, 3]]);
    assert!(load::raster_image(&with_header_boxes(&[full.clone(), cdef(&[[0, 0, 1]])])).err().unwrap().to_string().contains("can't be read"));
    let mut two_headers = JP2.to_vec();
    let header = super::jp2read::boxes(JP2).find_map(|(ty, data)| (ty == b"jp2h").then_some(data)).unwrap();
    two_headers.extend(boxed(b"jp2h", header));
    assert!(load::raster_image(&two_headers).err().unwrap().to_string().contains("can't be read"));
}

#[test]
fn palette_needs_one_mapping_per_column() {
    // 256 entries of three 8-bit columns, entry i being (i, i, i).
    let mut palette = vec![1, 0, 3, 7, 7, 7];
    palette.extend((0..=255u8).flat_map(|i| [i; 3]));
    let pclr = boxed(b"pclr", &palette);
    let mut deep = vec![1, 0, 3, 7, 7, 31];
    deep.extend((0..=255u8).flat_map(|i| [i, i, 0, 0, 0, i]));
    let deep = boxed(b"pclr", &deep);
    let cmap = |entries: &[[u8; 4]]| boxed(b"cmap", &entries.concat());
    // Each component through its own column gives the same pixels.
    let mapped = with_header_boxes(&[pclr.clone(), cmap(&[[0, 0, 1, 0], [0, 1, 1, 1], [0, 2, 0, 0]])]);
    assert_eq!(pixels(&mapped), COLORS);
    // 256 rows of three 8-byte values and a 24-byte list each (12,288 bytes) pass a 12,000-byte allowance.
    assert!(super::jp2read::decode(&mapped, 12_000).unwrap_err().contains("too large to draw"));
    for extra in [
        vec![pclr.clone(), cmap(&[[0, 0, 1, 0], [0, 1, 1, 1], [0, 2, 1, 2], [0, 0, 1, 0]])],
        vec![pclr.clone(), cmap(&[[0, 0, 1, 0], [0, 1, 1, 1], [0, 9, 1, 2]])],
        vec![pclr.clone(), cmap(&[[0, 0, 1, 0], [0, 1, 1, 1], [0, 2, 1, 3]])],
        vec![pclr.clone(), cmap(&[[0, 0, 1, 0], [0, 1, 1, 1]])],
        // A 32-bit column.
        vec![deep.clone(), cmap(&[[0, 0, 1, 0], [0, 1, 1, 1], [0, 2, 1, 2]])],
    ] {
        assert!(load::raster_image(&with_header_boxes(&extra)).err().unwrap().to_string().contains("can't be read"));
    }
}

#[test]
fn tile_grid_is_bounded_before_decoding() {
    let tiled = |size: u32, tile: u32| {
        let mut file = RAW.to_vec();
        file[8..12].copy_from_slice(&size.to_be_bytes());
        file[12..16].copy_from_slice(&size.to_be_bytes());
        file[24..28].copy_from_slice(&tile.to_be_bytes());
        file[28..32].copy_from_slice(&tile.to_be_bytes());
        file
    };
    // 1,000 × 1,000 one-pixel tiles are more than a codestream can number.
    assert!(super::jp2read::decode(&tiled(1000, 1), MAX_RASTER_ALLOC).unwrap_err().contains("can't be read"));
    assert!(super::jp2read::decode(&tiled(2, 0), MAX_RASTER_ALLOC).unwrap_err().contains("can't be read"));
    // 255 × 255 one-pixel tiles of three components need more than this allowance before decoding.
    assert!(super::jp2read::decode(&tiled(255, 1), 1 << 20).unwrap_err().contains("too large to draw"));
}

#[test]
fn cielab_images_are_refused() {
    let header = super::jp2read::boxes(JP2).find_map(|(ty, data)| (ty == b"jp2h").then_some(data)).unwrap();
    // Enumerated color space 14, CIELab, with its default range and offset.
    let children = super::jp2read::boxes(header)
        .map(|(ty, data)| if ty == b"colr" { boxed(b"colr", &[1, 0, 0, 0, 0, 0, 14]) } else { boxed(ty.try_into().unwrap(), data) })
        .collect::<Vec<_>>();
    let file = super::jp2read::boxes(JP2)
        .map(|(ty, data)| if ty == b"jp2h" { boxed(b"jp2h", &children.concat()) } else { boxed(ty.try_into().unwrap(), data) })
        .collect::<Vec<_>>()
        .concat();
    assert!(load::raster_image(&file).err().unwrap().to_string().contains("unsupported color space"));
}

#[test]
fn embedded_srgb_profile_keeps_checker_pixels() {
    let icc = vectorcraft_color::cms::icc_bytes(vectorcraft_color::cms::SRGB).unwrap();
    let mut colr_data = vec![2, 0, 0];
    colr_data.extend_from_slice(&icc);
    let header = super::jp2read::boxes(JP2).find_map(|(ty, data)| (ty == b"jp2h").then_some(data)).unwrap();
    let children = super::jp2read::boxes(header)
        .map(|(ty, data)| if ty == b"colr" { boxed(b"colr", &colr_data) } else { boxed(ty.try_into().unwrap(), data) })
        .collect::<Vec<_>>();
    let file = super::jp2read::boxes(JP2)
        .map(|(ty, data)| if ty == b"jp2h" { boxed(b"jp2h", &children.concat()) } else { boxed(ty.try_into().unwrap(), data) })
        .collect::<Vec<_>>()
        .concat();
    assert_eq!(pixels(&file), COLORS);
    // A profile past 4 MiB is refused before the decoder reads the file.
    let mut big = vec![2, 0, 0];
    big.resize(3 + 4 * 1024 * 1024 + 1, 0);
    assert!(super::jp2read::decode(&with_header_boxes(&[boxed(b"colr", &big)]), MAX_RASTER_ALLOC).unwrap_err().contains("unsupported color space"));
}
