//! HEIC and HEIF Open, Place, pixel, color and resolution regressions using original synthetic
//! fixtures (quadrants of four colors, lossy HEVC, so colors are compared within a tolerance).

#[cfg(feature = "heif")]
use serde_json::json;

use super::heifread::is_heif;
use super::*;

const QUADRANTS: &[u8] = include_bytes!("../../../tests/fixtures/heif/quadrants.heic");
const ALPHA: &[u8] = include_bytes!("../../../tests/fixtures/heif/quadrants-alpha.heic");
const TEN_BIT: &[u8] = include_bytes!("../../../tests/fixtures/heif/quadrants-10bit.heic");
const GRID: &[u8] = include_bytes!("../../../tests/fixtures/heif/quadrants-grid.heic");
const GRID16: &[u8] = include_bytes!("../../../tests/fixtures/heif/quadrants-grid16.heic");
const PPI300: &[u8] = include_bytes!("../../../tests/fixtures/heif/quadrants-300ppi.heic");
const ROTATED: &[u8] = include_bytes!("../../../tests/fixtures/heif/wide-rotated.heic");
const ROTATED_300X150: &[u8] = include_bytes!("../../../tests/fixtures/heif/wide-rotated-300x150.heic");
const TONES: &[u8] = include_bytes!("../../../tests/fixtures/heif/tones.heic");
const TONES_P3_NCLX: &[u8] = include_bytes!("../../../tests/fixtures/heif/tones-p3-nclx.heic");
const TONES_P3_ICC: &[u8] = include_bytes!("../../../tests/fixtures/heif/tones-p3-icc.heic");
const TONES_P3_ICC_ONLY: &[u8] = include_bytes!("../../../tests/fixtures/heif/tones-p3-icc-only.heic");

const EVERY_FIXTURE: [&[u8]; 12] =
    [QUADRANTS, ALPHA, TEN_BIT, GRID, GRID16, PPI300, ROTATED, ROTATED_300X150, TONES, TONES_P3_NCLX, TONES_P3_ICC, TONES_P3_ICC_ONLY];

/// Red, green, blue and white: the top-left, top-right, bottom-left and bottom-right quadrants.
#[cfg(feature = "heif")]
const COLORS: [[u8; 4]; 4] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255; 4]];

/// The centers of the four quadrants of a square image `side` pixels wide.
#[cfg(feature = "heif")]
fn centers(side: u32) -> [(u32, u32); 4] {
    let (a, b) = (side / 4, side * 3 / 4);
    [(a, a), (b, a), (a, b), (b, b)]
}

#[cfg(feature = "heif")]
fn decoded(bytes: &[u8]) -> image::RgbaImage {
    let raster = load::raster_image(bytes).unwrap();
    image::load_from_memory(&raster.blob.bytes).unwrap().to_rgba8()
}

#[cfg(feature = "heif")]
fn close(got: [u8; 4], want: [u8; 4], tolerance: u8) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= tolerance)
}

#[cfg(feature = "heif")]
fn assert_quadrants(img: &image::RgbaImage, side: u32, what: &str) {
    assert_eq!(img.dimensions(), (side, side), "{what}");
    for ((x, y), want) in centers(side).into_iter().zip(COLORS) {
        let got = img.get_pixel(x, y).0;
        assert!(close(got, want, 8), "{what} at ({x}, {y}): {got:?} != {want:?}");
    }
}

#[cfg(feature = "heif")]
#[test]
fn quadrants_open_and_place_as_their_pixels() {
    assert_quadrants(&decoded(QUADRANTS), 32, "quadrants.heic");
    for name in ["photo.heic", "photo.heif", "unknown.bin"] {
        let mut s = Session::new();
        let encoded = vectorcraft_format::base64_encode(QUADRANTS);
        let opened = s.execute("document.open", &json!({"name": name, "dataBase64": encoded})).unwrap();
        assert_eq!(opened["format"], "heif", "{name}");
        assert_eq!(s.doc().unwrap().doc.artboards[0].rect.size(), vectorcraft_geom::Size::new(32.0, 32.0));
        let blob = s.doc().unwrap().doc.images.values().next().unwrap();
        assert_quadrants(&image::load_from_memory(&blob.bytes).unwrap().to_rgba8(), 32, name);
        s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let placed = s.execute("file.place", &json!({"name": name, "dataBase64": encoded})).unwrap();
        assert_eq!(placed["format"], "heif", "{name}");
    }
    assert!(load::texture_image(QUADRANTS).is_ok());
    // The general brand alone, without `heic`.
    let mut general = QUADRANTS.to_vec();
    assert_eq!(&QUADRANTS[8..28], b"heic\0\0\0\0mif1heicmiaf");
    general[8..12].copy_from_slice(b"mif1");
    general[20..24].copy_from_slice(b"miaf");
    assert_quadrants(&decoded(&general), 32, "mif1 only");
}

#[test]
fn signatures_and_every_extension() {
    for bytes in EVERY_FIXTURE {
        assert!(is_heif(bytes));
        assert_eq!(load::detect("unknown.bin", bytes).map(|f| f.id), Some("heif"));
    }
    for ext in ["heic", "heif"] {
        assert_eq!(load::detect(&format!("a.{ext}"), b"invalid").map(|f| f.id), Some("heif"));
        assert!(OPEN_EXTS.contains(&ext) && PLACE_EXTS.contains(&ext) && UNASSOCIATED_EXTS.contains(&ext), "{ext}");
    }
    // A brand only among the compatible ones counts, and so do the general brands alone.
    assert!(is_heif(b"\0\0\0\x14ftypabcd\0\0\0\0heic"));
    assert!(is_heif(b"\0\0\0\x18ftypmif1\0\0\0\0mif1heix"));
    assert!(is_heif(b"\0\0\0\x18ftypmsf1\0\0\0\0msf1hevc"));
    assert!(is_heif(b"\0\0\0\x18ftypmif1\0\0\0\0mif1miaf"));
    // A 64-bit box size (ISO/IEC 14496-12 4.2).
    assert!(is_heif(b"\0\0\0\x01ftyp\0\0\0\0\0\0\0\x18heic\0\0\0\0"));
    let mut extended = b"\0\0\0\x01ftyp\0\0\0\0\0\0\0\x18heic\0\0\0\0".to_vec();
    extended.extend_from_slice(&QUADRANTS[QUADRANTS.windows(4).position(|w| w == b"meta").unwrap() - 4..]);
    assert_eq!(load::detect("unknown.bin", &extended).map(|f| f.id), Some("heif"));
    // AVIF, with its brand major or compatible, and brands heic-rs doesn't read.
    assert!(!is_heif(b"\0\0\0\x18ftypavif\0\0\0\0mif1miaf"));
    assert!(!is_heif(b"\0\0\0\x1cftypmif1\0\0\0\0mif1avifmiaf"));
    assert!(!is_heif(b"\0\0\0\x1cftypheic\0\0\0\0mif1heicavis"));
    assert!(!is_heif(b"\0\0\0\x18ftyphevx\0\0\0\0hevxiso8"));
    assert!(!is_heif(b"\0\0\0\x18ftypisom\0\0\0\0isomavc1"));
    // A box shorter than its brands, longer than the file, or of another type.
    assert!(!is_heif(b"\0\0\0\x0cftypheic"));
    assert!(!is_heif(b"\0\0\0\x40ftypheic\0\0\0\0mif1"));
    assert!(!is_heif(b"\0\0\0\x14styphei\0\0\0\0\0heic"));
    assert!(!is_heif(b""));
}

#[cfg(feature = "heif")]
#[test]
fn alpha_10_bit_and_grid_images_keep_their_pixels() {
    let alpha = decoded(ALPHA);
    assert_eq!(alpha.dimensions(), (32, 32));
    for ((x, y), want) in centers(32).into_iter().zip(COLORS) {
        let got = alpha.get_pixel(x, y).0;
        // The left half is opaque; the right half is transparent.
        if x < 16 {
            assert!(close(got, want, 8), "({x}, {y}): {got:?} != {want:?}");
        } else {
            assert!(got[3] <= 8, "({x}, {y}): {got:?}");
        }
    }
    assert_quadrants(&decoded(TEN_BIT), 32, "10-bit");
    // Four 64 × 64 tiles, one for each quadrant.
    assert_quadrants(&decoded(GRID), 128, "2 × 2 grid");
}

#[cfg(feature = "heif")]
#[test]
fn a_grid_of_cropped_tiles_is_refused() {
    // 32 × 32 quadrants as a 2 × 2 grid of 16 × 16 tiles, each coded 64 × 64 and cropped back
    // with its own clap, which heic-rs 0.1.4 leaves out when it composes the grid.
    let error = load::raster_image(GRID16).err().unwrap().to_string();
    assert!(error.contains("kind of image that can't be read"), "{error}");
}

#[cfg(feature = "heif")]
#[test]
fn a_grid_without_its_own_color_box_uses_its_tiles() {
    // The grid item's associations (ipma) list ispe (1), pixi (5) and colr (3); the tiles carry
    // the same colr. Pointing the grid's third association at its ispe leaves the grid without one.
    let mut tiles_only = GRID.to_vec();
    assert_eq!(&GRID[565..571], &[0, 1, 3, 1, 0x85, 3], "item 1 with three associations, the last colr");
    tiles_only[570] = 1;
    assert_quadrants(&decoded(&tiles_only), 128, "colr on the tiles only");
}

#[cfg(feature = "heif")]
#[test]
fn the_containers_rotation_is_applied() {
    // A 32 × 16 image, red then blue, stored with a quarter turn clockwise.
    let img = decoded(ROTATED);
    assert_eq!(img.dimensions(), (16, 32));
    assert!(close(img.get_pixel(8, 4).0, [255, 0, 0, 255], 8), "{:?}", img.get_pixel(8, 4));
    assert!(close(img.get_pixel(8, 27).0, [0, 0, 255, 255], 8), "{:?}", img.get_pixel(8, 27));
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "a.heic", "dataBase64": vectorcraft_format::base64_encode(ROTATED)})).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect.size(), vectorcraft_geom::Size::new(16.0, 32.0));
}

#[cfg(feature = "heif")]
#[test]
fn the_containers_mirror_and_crop_are_applied() {
    // The rotated image's irot (property 6) turned to no rotation, and to an imir of the same
    // size: mirrored left to right (ISO/IEC 23008-12 6.5.12, axis 0).
    assert_eq!(&ROTATED[384..393], b"\0\0\0\x09irot\x03");
    let mut stored = ROTATED.to_vec();
    stored[392] = 0;
    let mut mirrored = ROTATED.to_vec();
    mirrored[388..393].copy_from_slice(b"imir\0");
    let (stored, mirrored) = (decoded(&stored), decoded(&mirrored));
    assert_eq!((stored.dimensions(), mirrored.dimensions()), ((32, 16), (32, 16)));
    let red = [255, 0, 0, 255];
    assert!(close(stored.get_pixel(8, 8).0, red, 8) && !close(stored.get_pixel(24, 8).0, red, 8), "{:?}", stored.get_pixel(8, 8));
    for (x, y) in [(4, 4), (8, 8), (20, 12), (27, 3)] {
        assert!(close(mirrored.get_pixel(x, y).0, stored.get_pixel(31 - x, y).0, 8), "({x}, {y})");
    }
    // The quadrants' clap (property 4) takes the top-left 32 × 32 of the 64 × 64 coded picture.
    // Rewritten to its top-left 16 × 16: a center offset of -24 (ISO/IEC 14496-12 12.1.4) leaves
    // the red quadrant alone. Its edge next to green is left out, as lossy chroma blends there.
    assert_eq!(&QUADRANTS[329..337], b"\0\0\0\x28clap");
    let mut cropped = QUADRANTS.to_vec();
    let clap: Vec<u8> = [16u32, 1, 16, 1, (-48i32).cast_unsigned(), 2, (-48i32).cast_unsigned(), 2].iter().flat_map(|v| v.to_be_bytes()).collect();
    cropped[337..369].copy_from_slice(&clap);
    let cropped = decoded(&cropped);
    assert_eq!(cropped.dimensions(), (16, 16));
    for (x, y) in [(0, 0), (8, 8), (12, 0), (0, 12), (12, 12)] {
        assert!(close(cropped.get_pixel(x, y).0, red, 8), "({x}, {y}): {:?}", cropped.get_pixel(x, y));
    }
}

#[cfg(feature = "heif")]
#[test]
fn exif_resolution_sizes_the_image_and_72_ppi_without_it() {
    let got = ppi::resolution(PPI300).unwrap();
    assert!((got.0 - 300.0).abs() < 0.01 && (got.1 - 300.0).abs() < 0.01, "{got:?}");
    assert_eq!(ppi::resolution(QUADRANTS), None);
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "a.heic", "dataBase64": vectorcraft_format::base64_encode(PPI300)})).unwrap();
    let size = s.doc().unwrap().doc.artboards[0].rect.size();
    assert!((size.width - 7.68).abs() < 0.001 && (size.height - 7.68).abs() < 0.001, "{size:?}");
    let blob = s.doc().unwrap().doc.images.values().next().unwrap();
    let stored = ppi::resolution(&blob.bytes).unwrap();
    assert!((stored.0 - 300.0).abs() < 0.01 && (stored.1 - 300.0).abs() < 0.01);
    // 32 × 16 pixels at 300 ppi across and 150 down, turned a quarter: 16 × 32 pixels at 150 ppi
    // across and 300 down, the same 7.68 × 7.68 pt either way.
    assert_eq!(ppi::resolution(ROTATED_300X150), Some((300.0, 150.0)), "as stored");
    let img = load::raster_image(ROTATED_300X150).unwrap();
    assert_eq!((img.width, img.height, img.ppi), (16, 32, Some((150.0, 300.0))));
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "a.heic", "dataBase64": vectorcraft_format::base64_encode(ROTATED_300X150)})).unwrap();
    let size = s.doc().unwrap().doc.artboards[0].rect.size();
    assert!((size.width - 7.68).abs() < 0.001 && (size.height - 7.68).abs() < 0.001, "{size:?}");
}

#[cfg(feature = "heif")]
#[test]
fn display_p3_images_are_converted_to_srgb() {
    use vectorcraft_color::cms::{DISPLAY_P3, Intent, icc::builtin_rgb};
    let p3 = builtin_rgb(DISPLAY_P3).unwrap();
    let plain = decoded(TONES);
    // The ICC-only file's samples are full-range BT.601, which only its HEVC stream says.
    for (what, bytes) in [("nclx", TONES_P3_NCLX), ("ICC", TONES_P3_ICC), ("ICC only", TONES_P3_ICC_ONLY)] {
        let converted = decoded(bytes);
        for (x, y) in centers(32) {
            let [r, g, b, a] = plain.get_pixel(x, y).0;
            let rgb = p3.to_srgb_into(&[r, g, b].map(|v| f32::from(v) / 255.0), Intent::RelativeColorimetric).unwrap();
            let [r, g, b] = rgb.map(|v| (v * 255.0).round() as u8);
            let got = converted.get_pixel(x, y).0;
            assert!(close(got, [r, g, b, a], 2), "{what} at ({x}, {y}): {got:?} != {:?}", [r, g, b, a]);
        }
    }
    // The orange quadrant changes, so the conversion ran.
    assert!(!close(decoded(TONES_P3_NCLX).get_pixel(8, 8).0, plain.get_pixel(8, 8).0, 4));
}

#[cfg(feature = "heif")]
#[test]
fn display_p3_named_by_the_stream_alone_is_converted_to_srgb() {
    // The nclx file's item associates hvcC, colr (property 2), ispe, pixi and clap. Pointing the
    // colr association at the ispe leaves the item without a color box, and its HEVC stream's
    // video signal type, which names Display P3 with the sRGB curve, describes the samples.
    let ipma = TONES_P3_NCLX.windows(4).position(|w| w == b"ipma").unwrap();
    let at = ipma + 4 + 4 + 4 + 2 + 1 + 1;
    assert_eq!(&TONES_P3_NCLX[at - 4..at + 4], &[0, 1, 5, 0x81, 2, 3, 5, 0x84], "item 1 with five associations, the second colr");
    let mut stream_only = TONES_P3_NCLX.to_vec();
    stream_only[at] = 3;
    let want = decoded(TONES_P3_NCLX);
    let got = decoded(&stream_only);
    for (x, y) in centers(32) {
        assert!(close(got.get_pixel(x, y).0, want.get_pixel(x, y).0, 2), "at ({x}, {y}): {:?} != {:?}", got.get_pixel(x, y), want.get_pixel(x, y));
    }
}

#[cfg(feature = "heif")]
#[test]
fn two_quarter_turns_keep_the_resolution_axes() {
    // The rotated image's ipma lists its irot once; a second association with the same irot turns
    // it a half turn, so the EXIF resolution keeps its axes.
    let ipma = ROTATED_300X150.windows(4).position(|w| w == b"ipma").unwrap();
    let count = ipma + 4 + 4 + 4 + 2;
    let irot = ROTATED_300X150.windows(4).position(|w| w == b"irot").unwrap();
    let ipco = ROTATED_300X150.windows(4).position(|w| w == b"ipco").unwrap();
    // The irot's property index: its place among the boxes of ipco, counted from 1.
    let mut index = 1u8;
    let mut at = ipco + 4;
    while at < irot - 4 {
        index += 1;
        at += u32::from_be_bytes(ROTATED_300X150[at..at + 4].try_into().unwrap()) as usize;
    }
    assert_eq!(at, irot - 4, "irot is a box of ipco");
    let n = usize::from(ROTATED_300X150[count]);
    assert!(ROTATED_300X150[count + 1..count + 1 + n].contains(&(0x80 | index)), "the item's irot association, essential");
    let mut twice = ROTATED_300X150.to_vec();
    twice[count] += 1;
    twice.insert(count + 1 + n, 0x80 | index);
    // The ipma, iprp and meta boxes each grow by one byte, and the iloc offsets move with them.
    let grow = |bytes: &mut Vec<u8>, name: &[u8; 4]| {
        let at = bytes.windows(4).position(|w| w == name).unwrap() - 4;
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) + 1;
        bytes[at..at + 4].copy_from_slice(&size.to_be_bytes());
    };
    for name in [b"ipma", b"iprp", b"meta"] {
        grow(&mut twice, name);
    }
    let iloc = twice.windows(4).position(|w| w == b"iloc").unwrap();
    assert_eq!(&twice[iloc + 4..iloc + 8], &[0, 0, 0, 0], "iloc version 0");
    let sizes = twice[iloc + 8];
    assert_eq!((sizes >> 4, sizes & 0xf), (4, 4), "4-byte offsets and lengths");
    assert_eq!(twice[iloc + 9] >> 4, 4, "4-byte base offsets");
    let items = u16::from_be_bytes([twice[iloc + 10], twice[iloc + 11]]);
    // Each item: its id, data reference index and base offset, which moves, then its extents.
    let mut at = iloc + 12;
    for _ in 0..items {
        at += 2 + 2;
        let offset = u32::from_be_bytes(twice[at..at + 4].try_into().unwrap()) + 1;
        twice[at..at + 4].copy_from_slice(&offset.to_be_bytes());
        at += 4;
        let extents = u16::from_be_bytes([twice[at], twice[at + 1]]);
        at += 2 + 8 * usize::from(extents);
    }
    let img = load::raster_image(&twice).unwrap();
    assert_eq!((img.width, img.height, img.ppi), (32, 16, Some((300.0, 150.0))), "a half turn keeps the stored axes");
}

#[cfg(feature = "heif")]
#[test]
fn damaged_unsupported_and_oversized_files_are_refused() {
    let error = |bytes: &[u8]| load::raster_image(bytes).err().unwrap().to_string();
    for cut in [16, 40, QUADRANTS.len() / 2, QUADRANTS.len() - 5] {
        assert!(load::raster_image(&QUADRANTS[..cut]).is_err(), "{cut}");
    }
    // An ftyp and nothing else, as an image sequence starts (its `moov` would follow).
    assert!(error(b"\0\0\0\x18ftypmsf1\0\0\0\0msf1hevc").contains("kind of image that can't be read"));
    assert!(error(b"\0\0\0\x18ftypheic\0\0\0\0mif1heic\0\0\0\x08free").contains("HEIC or HEIF file"));
    // The primary item's type (infe) turned from HEVC to AV1, with its hvcC kept, and with its
    // hvcC turned into an av1C.
    let mut av1 = QUADRANTS.to_vec();
    assert_eq!((&QUADRANTS[137..141], &QUADRANTS[176..180]), (&b"hvc1"[..], &b"hvcC"[..]));
    av1[137..141].copy_from_slice(b"av01");
    assert!(error(&av1).contains("kind of image that can't be read"), "{}", error(&av1));
    av1[176..180].copy_from_slice(b"av1C");
    assert!(error(&av1).contains("kind of image that can't be read"), "{}", error(&av1));
    // The image spatial extents (`ispe`) claim 60,000 × 60,000 pixels.
    let mut huge = QUADRANTS.to_vec();
    let at = huge.windows(4).position(|w| w == b"ispe").unwrap() + 8;
    huge[at..at + 4].copy_from_slice(&60_000u32.to_be_bytes());
    huge[at + 4..at + 8].copy_from_slice(&60_000u32.to_be_bytes());
    assert!(error(&huge).contains("60000 × 60000 pixels is too large to draw"), "{}", error(&huge));
    // A single 64 × 64 coded picture (cropped to 32 × 32) takes its RGBA output and copy,
    // 64 × 64 × 8, and its planes, 64 × 64 × 9: no composed frame is counted.
    assert!(super::heifread::decode(QUADRANTS, 64 * 64 * 17).is_ok());
    assert!(super::heifread::decode(QUADRANTS, 64 * 64 * 17 - 1).unwrap_err().contains("too large to draw"));
    // The same picture with an ispe of 32 × 32 (its crop's size): its stream still decodes
    // 64 × 64, and that size is counted.
    let mut small = QUADRANTS.to_vec();
    let at = small.windows(4).position(|w| w == b"ispe").unwrap() + 8;
    assert_eq!(&small[at..at + 8], b"\0\0\0\x40\0\0\0\x40");
    small[at..at + 4].copy_from_slice(&32u32.to_be_bytes());
    small[at + 4..at + 8].copy_from_slice(&32u32.to_be_bytes());
    let refused = super::heifread::decode(&small, 64 * 64 * 17 - 1).map(|img| img.dimensions());
    assert!(refused.as_ref().is_err_and(|e| e.contains("too large to draw")), "{refused:?}");
    // A 128 × 128 grid of four 64 × 64 tiles adds its composed frame, 128 × 128 × 6.
    let grid = 128 * 128 * (8 + 6) + 4 * 64 * 64 * 9;
    assert!(super::heifread::decode(GRID, grid).is_ok());
    assert!(super::heifread::decode(GRID, grid - 1).unwrap_err().contains("too large to draw"));
    // The grid's ispe (property 1) claims 16 × 16 pixels; its grid data still composes 128 × 128,
    // and that size is counted.
    let mut understated = GRID.to_vec();
    assert_eq!(&GRID[358..370], b"ispe\0\0\0\0\0\0\0\x80", "property 1 is the grid's 128 × 128 ispe");
    understated[366..370].copy_from_slice(&16u32.to_be_bytes());
    understated[370..374].copy_from_slice(&16u32.to_be_bytes());
    assert!(super::heifread::decode(&understated, grid - 1).unwrap_err().contains("too large to draw"));
}

/// Appends `value` as `n` bits, most significant first.
#[cfg(feature = "heif")]
fn put(bits: &mut Vec<bool>, value: u64, n: u32) {
    bits.extend((0..n).rev().map(|i| i < 64 && value >> i & 1 == 1));
}

/// Appends `value` as an unsigned Exp-Golomb code.
#[cfg(feature = "heif")]
fn put_ue(bits: &mut Vec<bool>, value: u64) {
    let len = 64 - (value + 1).leading_zeros();
    put(bits, 0, len - 1);
    put(bits, value + 1, len);
}

/// The fields of SPS 0 of `width` × `height` 4:2:0 samples with `sub_layers` sub-layers, each
/// with its profile and level, up to its height.
#[cfg(feature = "heif")]
fn sps_head(width: u64, height: u64, sub_layers: u64) -> Vec<bool> {
    sps_head_with_id(width, height, sub_layers, 0)
}

/// [`sps_head`] for SPS `id`.
#[cfg(feature = "heif")]
fn sps_head_with_id(width: u64, height: u64, sub_layers: u64, id: u64) -> Vec<bool> {
    let mut bits = Vec::new();
    put(&mut bits, 0, 4);
    put(&mut bits, sub_layers, 3);
    put(&mut bits, 1, 1);
    put(&mut bits, 0, 96);
    for _ in 0..sub_layers {
        put(&mut bits, 0b11, 2);
    }
    if sub_layers > 0 {
        put(&mut bits, 0, 2 * (8 - sub_layers as u32));
    }
    for _ in 0..sub_layers {
        put(&mut bits, 0, 88 + 8);
    }
    put_ue(&mut bits, id);
    put_ue(&mut bits, 1);
    put_ue(&mut bits, width);
    put_ue(&mut bits, height);
    bits
}

/// An SPS NAL unit of `bits` (see [`nal`]).
#[cfg(feature = "heif")]
fn sps_nal(bits: Vec<bool>) -> Vec<u8> {
    nal(33, bits)
}

/// A NAL unit of type `kind` holding `bits`, with its stop bit, and emulation prevention bytes
/// inserted as a stream carries them.
#[cfg(feature = "heif")]
fn nal(kind: u8, mut bits: Vec<bool>) -> Vec<u8> {
    put(&mut bits, 1, 1);
    while !bits.len().is_multiple_of(8) {
        bits.push(false);
    }
    let rbsp: Vec<u8> = bits.chunks(8).map(|b| b.iter().fold(0u8, |v, &bit| v << 1 | u8::from(bit))).collect();
    let mut nal = vec![kind << 1, 1];
    let mut zeros = 0;
    for b in rbsp {
        if zeros >= 2 && b <= 3 {
            nal.push(3);
            zeros = 0;
        }
        zeros = if b == 0 { zeros + 1 } else { 0 };
        nal.push(b);
    }
    nal
}

#[cfg(feature = "heif")]
fn sps(width: u64, height: u64, sub_layers: u64) -> Vec<u8> {
    sps_nal(sps_head(width, height, sub_layers))
}

/// A 64 × 64 SPS that uses every optional part before the VUI's video signal type: a conformance
/// window, sub-layer ordering for two sub-layers, explicit scaling lists, PCM, a reference picture
/// set predicted from another and a long-term picture, then an Extended_SAR aspect ratio and
/// overscan information. `signal` is the full-range flag and the color description, if any.
#[cfg(feature = "heif")]
fn sps_with_vui(signal: Option<(u64, Option<[u64; 3]>)>) -> Vec<u8> {
    sps_with_vui_and_id(signal, 0)
}

/// [`sps_with_vui`] for SPS `id`.
#[cfg(feature = "heif")]
fn sps_with_vui_and_id(signal: Option<(u64, Option<[u64; 3]>)>, id: u64) -> Vec<u8> {
    let mut bits = sps_head_with_id(64, 64, 1, id);
    put(&mut bits, 1, 1);
    for offset in [0, 2, 0, 2] {
        put_ue(&mut bits, offset);
    }
    put_ue(&mut bits, 0);
    put_ue(&mut bits, 0);
    // 8 bits of picture order count.
    put_ue(&mut bits, 4);
    put(&mut bits, 1, 1);
    for _ in 0..2 * 3 {
        put_ue(&mut bits, 1);
    }
    for value in [0, 3, 0, 3, 1, 1] {
        put_ue(&mut bits, value);
    }
    // Scaling lists: matrix 1 of each size given explicitly, the others copied.
    put(&mut bits, 0b11, 2);
    for size in 0..4 {
        for matrix in 0..if size == 3 { 2 } else { 6 } {
            if matrix == 1 {
                put(&mut bits, 1, 1);
                if size > 1 {
                    put_ue(&mut bits, 5);
                }
                for _ in 0..if size == 0 { 16 } else { 64 } {
                    put_ue(&mut bits, 3);
                }
            } else {
                put(&mut bits, 0, 1);
                put_ue(&mut bits, 0);
            }
        }
    }
    // AMP off, SAO on, PCM with its depths, block sizes and loop filter flag.
    put(&mut bits, 0b01, 2);
    put(&mut bits, 1, 1);
    put(&mut bits, 0x77, 8);
    put_ue(&mut bits, 0);
    put_ue(&mut bits, 1);
    put(&mut bits, 0, 1);
    // Two short-term sets: one negative and one positive picture, then a set predicted from it
    // with three entries (unused with use_delta_flag, used, unused).
    put_ue(&mut bits, 2);
    put_ue(&mut bits, 1);
    put_ue(&mut bits, 1);
    for _ in 0..2 {
        put_ue(&mut bits, 0);
        put(&mut bits, 1, 1);
    }
    put(&mut bits, 1, 1);
    put(&mut bits, 0, 1);
    put_ue(&mut bits, 0);
    put(&mut bits, 0b01100, 5);
    // One long-term picture: 8 bits of picture order count and its used flag.
    put(&mut bits, 1, 1);
    put_ue(&mut bits, 1);
    put(&mut bits, 0xab, 8);
    put(&mut bits, 1, 1);
    put(&mut bits, 0b11, 2);
    // VUI: Extended_SAR 1:1 and overscan information.
    put(&mut bits, 1, 1);
    put(&mut bits, 1, 1);
    put(&mut bits, 255, 8);
    put(&mut bits, 0x0001_0001, 32);
    put(&mut bits, 1, 1);
    put(&mut bits, 0, 1);
    match signal {
        Some((full, description)) => {
            put(&mut bits, 1, 1);
            put(&mut bits, 5, 3);
            put(&mut bits, full, 1);
            put(&mut bits, u64::from(description.is_some()), 1);
            for code in description.into_iter().flatten() {
                put(&mut bits, code, 8);
            }
        }
        None => put(&mut bits, 0, 1),
    }
    // No chroma location, field or display window information, timing or restrictions.
    put(&mut bits, 0, 8);
    sps_nal(bits)
}

#[cfg(feature = "heif")]
#[test]
fn picture_sizes_come_from_the_sequence_parameter_sets() {
    use super::heifread::{picture_bytes, sps_size};
    for sub_layers in [0, 1, 6] {
        for (w, h) in [(32, 32), (4032, 3024), (65535, 65535)] {
            let nal = sps(w, h, sub_layers);
            assert!(nal.windows(3).any(|w| w == [0, 0, 3]), "the profile's zeros need emulation prevention");
            assert_eq!(sps_size(&nal), Some((w, h, 1)), "{w} × {h}, {sub_layers} sub-layers");
        }
    }
    let mut vps = sps(32, 32, 0);
    vps[0] = 32 << 1;
    assert_eq!(sps_size(&vps), None);
    assert_eq!(sps_size(&sps(32, 32, 0)[..8]), None);
    // 4:2:0 is 3 bytes a sample position in each of three sets of planes, in 64-sample blocks.
    assert_eq!(picture_bytes(QUADRANTS), Some(64 * 64 * 9));
    assert_eq!(picture_bytes(GRID), Some(4 * 64 * 64 * 9));
    assert!(picture_bytes(ALPHA).unwrap() > 64 * 64 * 9, "the alpha image is counted");
}

#[cfg(feature = "heif")]
#[test]
fn the_color_description_comes_from_the_streams_video_signal_type() {
    use super::heifread::{sps_signal, sps_size};
    use heic_rs::{MatrixCoefficients, Range};
    let signal = |nal: &[u8]| sps_signal(nal).map(|n| (n.primaries, n.transfer, n.matrix, n.matrix_code, n.range));
    let described = sps_with_vui(Some((1, Some([12, 13, 6]))));
    assert_eq!(sps_size(&described), Some((64, 64, 1)));
    assert_eq!(signal(&described), Some((12, 13, MatrixCoefficients::Bt601, 6, Range::Full)));
    assert_eq!(signal(&sps_with_vui(Some((0, None)))), Some((2, 2, MatrixCoefficients::Unspecified, 2, Range::Limited)));
    assert_eq!(signal(&sps_with_vui(None)), None);
    assert_eq!(signal(&sps(64, 64, 0)), None, "cut short before its VUI");
    // The ICC-only fixture's stream: heif-enc writes full-range BT.601.
    let ctx = heic_rs::context::Context::open(TONES_P3_ICC_ONLY).unwrap();
    let props = ctx.props(ctx.meta.primary).unwrap();
    assert!(props.nclx.is_none() && props.icc.is_some());
    let sets = props.hvcc.unwrap().parameter_sets();
    let nal = sets.into_iter().find(|nal| nal[0] >> 1 & 0x3f == 33).unwrap();
    assert_eq!(sps_signal(nal).map(|n| (n.matrix, n.range)), Some((MatrixCoefficients::Bt601, Range::Full)));
}

#[cfg(feature = "heif")]
#[test]
fn the_slices_picture_parameter_set_picks_the_sps() {
    use super::heifread::{active_sps, sps_signal};
    use heic_rs::Range;
    let full = sps_with_vui_and_id(Some((1, Some([1, 13, 6]))), 0);
    let limited = sps_with_vui_and_id(Some((0, Some([1, 13, 1]))), 1);
    let pps = |id: u64, sps: u64| {
        let mut bits = Vec::new();
        put_ue(&mut bits, id);
        put_ue(&mut bits, sps);
        nal(34, bits)
    };
    // first_slice_segment_in_pic_flag, no_output_of_prior_pics_flag for an IDR picture (type 19),
    // slice_pic_parameter_set_id.
    let slice = |kind: u8, pps: u64| {
        let mut bits = vec![true];
        if kind == 19 {
            bits.push(false);
        }
        put_ue(&mut bits, pps);
        nal(kind, bits)
    };
    let (pps0, pps3) = (pps(0, 0), pps(3, 1));
    let sets = [full.as_slice(), limited.as_slice(), pps0.as_slice(), pps3.as_slice()];
    let (idr3, trail0, idr5) = (slice(19, 3), slice(1, 0), slice(19, 5));
    assert_eq!(active_sps(&sets, &[&idr3]), Some(limited.as_slice()));
    assert_eq!(active_sps(&sets, &[&trail0]), Some(full.as_slice()));
    assert_eq!(active_sps(&sets, &[&idr5]), None, "no PPS 5");
    assert_eq!(active_sps(&sets, &[&pps0]), None, "no slice");
    assert_eq!(active_sps(&sets, &[&idr3]).and_then(sps_signal).map(|n| (n.matrix_code, n.range)), Some((1, Range::Limited)));
}

#[cfg(not(feature = "heif"))]
#[test]
fn a_build_without_the_decoder_refuses_heif_files_with_a_message() {
    for bytes in EVERY_FIXTURE {
        assert_eq!(load::detect("unknown.bin", bytes).map(|f| f.id), Some("heif"));
        let error = load::raster_image(bytes).err().unwrap().to_string();
        assert!(error.contains("can't be opened in this build"), "{error}");
    }
    assert_eq!(ppi::resolution(PPI300), None);
}
