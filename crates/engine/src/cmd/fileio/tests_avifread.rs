//! AVIF Open, Place, pixel, color and resolution regressions using original synthetic fixtures
//! (quadrants of four colors and four mid tones, written with libavif's `avifenc`; most are
//! lossless, so colors are compared within a small tolerance for the 10-bit and 4:2:0 ones).

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
use serde_json::json;

use super::avifread::{is_avif, quarter_turned};
use super::heifread::is_heif;
use super::*;

const QUADRANTS: &[u8] = include_bytes!("../../../tests/fixtures/avif/quadrants.avif");
const ALPHA: &[u8] = include_bytes!("../../../tests/fixtures/avif/quadrants-alpha.avif");
const TEN_BIT: &[u8] = include_bytes!("../../../tests/fixtures/avif/quadrants-10bit.avif");
const LOSSY_420: &[u8] = include_bytes!("../../../tests/fixtures/avif/quadrants-420.avif");
const GRID: &[u8] = include_bytes!("../../../tests/fixtures/avif/quadrants-grid.avif");
const PPI300: &[u8] = include_bytes!("../../../tests/fixtures/avif/quadrants-300ppi.avif");
const ROTATED: &[u8] = include_bytes!("../../../tests/fixtures/avif/wide-rotated.avif");
const ROTATED_300X150: &[u8] = include_bytes!("../../../tests/fixtures/avif/wide-rotated-300x150.avif");
const MIRRORED: &[u8] = include_bytes!("../../../tests/fixtures/avif/wide-mirrored.avif");
const TONES: &[u8] = include_bytes!("../../../tests/fixtures/avif/tones.avif");
const TONES_P3_NCLX: &[u8] = include_bytes!("../../../tests/fixtures/avif/tones-p3-nclx.avif");
const TONES_BT2020_NCLX: &[u8] = include_bytes!("../../../tests/fixtures/avif/tones-bt2020-nclx.avif");
const SEQUENCE: &[u8] = include_bytes!("../../../tests/fixtures/avif/sequence.avif");

const EVERY_FIXTURE: [&[u8]; 13] =
    [QUADRANTS, ALPHA, TEN_BIT, LOSSY_420, GRID, PPI300, ROTATED, ROTATED_300X150, MIRRORED, TONES, TONES_P3_NCLX, TONES_BT2020_NCLX, SEQUENCE];

/// The four tones of `tones*.avif`, in the order of [`centers`].
#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
const TONE_VALUES: [[u8; 3]; 4] = [[200, 100, 50], [60, 160, 90], [90, 80, 200], [128, 128, 128]];

/// Red, green, blue and white: the top-left, top-right, bottom-left and bottom-right quadrants.
#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
const COLORS: [[u8; 4]; 4] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255; 4]];

/// The centers of the four quadrants of a square image `side` pixels wide.
#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
fn centers(side: u32) -> [(u32, u32); 4] {
    let (a, b) = (side / 4, side * 3 / 4);
    [(a, a), (b, a), (a, b), (b, b)]
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
fn decoded(bytes: &[u8]) -> image::RgbaImage {
    let raster = load::raster_image(bytes).unwrap();
    image::load_from_memory(&raster.blob.bytes).unwrap().to_rgba8()
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
fn close(got: [u8; 4], want: [u8; 4], tolerance: u8) -> bool {
    got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= tolerance)
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
fn assert_quadrants(img: &image::RgbaImage, side: u32, tolerance: u8, what: &str) {
    assert_eq!(img.dimensions(), (side, side), "{what}");
    for ((x, y), want) in centers(side).into_iter().zip(COLORS) {
        let got = img.get_pixel(x, y).0;
        assert!(close(got, want, tolerance), "{what} at ({x}, {y}): {got:?} != {want:?}");
    }
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
#[test]
fn quadrants_open_and_place_as_their_pixels() {
    assert_quadrants(&decoded(QUADRANTS), 32, 1, "quadrants.avif");
    for name in ["photo.avif", "unknown.bin"] {
        let mut s = Session::new();
        let encoded = vectorcraft_format::base64_encode(QUADRANTS);
        let opened = s.execute("document.open", &json!({"name": name, "dataBase64": encoded})).unwrap();
        assert_eq!(opened["format"], "avif", "{name}");
        assert_eq!(s.doc().unwrap().doc.artboards[0].rect.size(), vectorcraft_geom::Size::new(32.0, 32.0));
        let blob = s.doc().unwrap().doc.images.values().next().unwrap();
        assert_quadrants(&image::load_from_memory(&blob.bytes).unwrap().to_rgba8(), 32, 1, name);
        s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let placed = s.execute("file.place", &json!({"name": name, "dataBase64": encoded})).unwrap();
        assert_eq!(placed["format"], "avif", "{name}");
    }
    assert!(load::texture_image(QUADRANTS).is_ok());
}

#[test]
fn signatures_and_every_extension() {
    for bytes in EVERY_FIXTURE {
        assert!(is_avif(bytes));
        assert!(!is_heif(bytes));
        assert_eq!(load::detect("unknown.bin", bytes).map(|f| f.id), Some("avif"));
    }
    assert_eq!(load::detect("a.avif", b"invalid").map(|f| f.id), Some("avif"));
    assert!(OPEN_EXTS.contains(&"avif") && PLACE_EXTS.contains(&"avif") && UNASSOCIATED_EXTS.contains(&"avif"));
    // A brand only among the compatible ones counts, the sequence brand too.
    assert!(is_avif(b"\0\0\0\x18ftypmif1\0\0\0\0mif1avif"));
    assert!(is_avif(b"\0\0\0\x14ftypavis\0\0\0\0msf1"));
    // A 64-bit box size (ISO/IEC 14496-12 4.2).
    assert!(is_avif(b"\0\0\0\x01ftyp\0\0\0\0\0\0\0\x18avif\0\0\0\0"));
    // HEIC brands, a cut-short `ftyp` and other files aren't AVIF.
    assert!(!is_avif(b"\0\0\0\x18ftypheic\0\0\0\0mif1heic"));
    assert!(!is_avif(b"\0\0\0\x0cftypavif"));
    assert!(!is_avif(b"\x89PNG\r\n\x1a\n"));
    assert!(!is_avif(b""));
}

#[test]
fn the_container_turn_is_read_from_irot() {
    assert!(quarter_turned(ROTATED));
    assert!(quarter_turned(ROTATED_300X150));
    for bytes in [QUADRANTS, MIRRORED, GRID, PPI300] {
        assert!(!quarter_turned(bytes));
    }
    assert!(!quarter_turned(b"\0\0\0\x18ftypavif\0\0\0\0mif1avif"));
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
#[test]
fn alpha_depth_subsampling_and_grids() {
    let alpha = decoded(ALPHA);
    assert_eq!(alpha.get_pixel(8, 8).0, [255, 0, 0, 255]);
    assert_eq!(alpha.get_pixel(24, 8).0[3], 0, "the right half is transparent");
    assert_eq!(alpha.get_pixel(8, 24).0, [0, 0, 255, 255]);
    assert_quadrants(&decoded(TEN_BIT), 32, 2, "10 bits");
    assert_quadrants(&decoded(LOSSY_420), 32, 12, "4:2:0");
    assert_quadrants(&decoded(GRID), 128, 1, "a 2 × 2 grid");
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
#[test]
fn rotation_mirror_and_resolution() {
    // 32 × 16 red and blue halves, turned a quarter anti-clockwise: red at the bottom.
    let turned = decoded(ROTATED);
    assert_eq!(turned.dimensions(), (16, 32));
    assert_eq!(turned.get_pixel(8, 4).0, [0, 0, 255, 255]);
    assert_eq!(turned.get_pixel(8, 28).0, [255, 0, 0, 255]);
    // Mirrored left to right: blue on the left.
    let mirrored = decoded(MIRRORED);
    assert_eq!(mirrored.dimensions(), (32, 16));
    assert_eq!(mirrored.get_pixel(4, 8).0, [0, 0, 255, 255]);
    assert_eq!(mirrored.get_pixel(28, 8).0, [255, 0, 0, 255]);
    // EXIF resolution, 72 ppi without it, and its axes swapped by the quarter turn.
    assert_eq!(load::raster_image(QUADRANTS).unwrap().ppi, None);
    let ppi300 = load::raster_image(PPI300).unwrap();
    assert_eq!(ppi300.ppi, Some((300.0, 300.0)));
    assert_eq!(super::ppi::resolution(&ppi300.blob.bytes), Some((300.0, 300.0)), "the stored PNG keeps it");
    assert_eq!(load::raster_image(ROTATED_300X150).unwrap().ppi, Some((150.0, 300.0)));
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
#[test]
fn nclx_primaries_become_srgb() {
    use vectorcraft_color::cms::{self, Intent};
    let srgb = decoded(TONES);
    for ((x, y), want) in centers(32).into_iter().zip(TONE_VALUES) {
        let got = srgb.get_pixel(x, y).0;
        assert!(close(got, [want[0], want[1], want[2], 255], 1), "sRGB tones at ({x}, {y}): {got:?}");
    }
    // Display P3 converts as the color engine's own Display P3 profile does.
    let p3 = cms::icc::builtin_rgb(cms::DISPLAY_P3).unwrap();
    let converted = decoded(TONES_P3_NCLX);
    for ((x, y), tone) in centers(32).into_iter().zip(TONE_VALUES) {
        let want = p3.to_srgb_into(&tone.map(|v| f32::from(v) / 255.0), Intent::RelativeColorimetric).unwrap();
        let want = want.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
        let got = converted.get_pixel(x, y).0;
        assert!(close(got, [want[0], want[1], want[2], 255], 2), "P3 tones at ({x}, {y}): {got:?} != {want:?}");
    }
    // BT.2020 is wider still: the tones move further than P3's, and gray stays gray.
    let wide = decoded(TONES_BT2020_NCLX);
    let [x, y] = [8, 8];
    let (p3_red, wide_red) = (converted.get_pixel(x, y).0[0], wide.get_pixel(x, y).0[0]);
    assert!(wide_red > p3_red && p3_red > TONE_VALUES[0][0], "{wide_red} > {p3_red} > {}", TONE_VALUES[0][0]);
    assert!(close(wide.get_pixel(24, 24).0, [128, 128, 128, 255], 1));
}

#[cfg(all(feature = "avif", not(target_arch = "wasm32")))]
#[test]
fn damaged_and_oversized_files_are_refused() {
    let message = |bytes: &[u8]| load::raster_image(bytes).err().map(|e| e.to_string()).unwrap_or_default();
    // libavif gives an image sequence a primary still item, its first frame, which is read; a
    // sequence without one is refused by the decoder.
    let first = decoded(SEQUENCE);
    assert_eq!(first.dimensions(), (16, 16));
    assert_eq!(first.get_pixel(8, 8).0, [255, 0, 0, 255]);
    for cut in [40, QUADRANTS.len() / 2, QUADRANTS.len() - 8] {
        assert!(message(&QUADRANTS[..cut]).contains("the AVIF file can't be read"), "cut at {cut}: {}", message(&QUADRANTS[..cut]));
    }
    // The pixel allowance is checked from the declared size before any picture is decoded.
    let too_large = super::avifread::decode(GRID, 128 * 128 * 4 - 1).unwrap_err();
    assert!(too_large.contains("too large to draw"), "{too_large}");
    assert!(super::avifread::decode(GRID, MAX_RASTER_ALLOC).is_ok());
}

#[cfg(not(all(feature = "avif", not(target_arch = "wasm32"))))]
#[test]
fn a_build_without_the_decoder_refuses_avif() {
    let e = load::raster_image(QUADRANTS).err().map(|e| e.to_string()).unwrap_or_default();
    assert!(e.contains("AVIF images can't be opened in this build"), "{e}");
}
