//! Reading PCX files: every depth and plane layout decodes to the pixels it stores, the files open,
//! place and paste at the resolution their header gives, and damaged files are refused with a
//! message.

use serde_json::json;

use super::pcxread::{decode, is_pcx};
use super::*;

/// A PCX file of `version`, `bits` per pixel in `planes` planes, `w` × `h` pixels from (`x0`, 0),
/// `line` bytes per plane line, `dpi` both ways, the header palette `colors` (up to 16 entries),
/// then the encoded image data `data`.
#[allow(clippy::too_many_arguments)]
fn pcx(version: u8, bits: u8, planes: u8, x0: u16, (w, h): (u16, u16), line: u16, dpi: u16, colors: &[[u8; 3]], data: &[u8]) -> Vec<u8> {
    let mut f = vec![0x0a, version, 1, bits];
    for v in [x0, 0, x0.wrapping_add(w).wrapping_sub(1), h.wrapping_sub(1), dpi, dpi] {
        f.extend(v.to_le_bytes());
    }
    let mut palette = [0; 48];
    for (i, c) in colors.iter().take(16).enumerate() {
        palette[i * 3..i * 3 + 3].copy_from_slice(c);
    }
    f.extend(palette);
    f.extend([0, planes]);
    f.extend(line.to_le_bytes());
    f.extend(1u16.to_le_bytes());
    f.resize(128, 0);
    f.extend(data);
    f
}

/// `raw` run-length encoded as the manual describes: runs of up to 63 bytes, and single bytes with
/// their top two bits set written as runs of one.
fn rle(raw: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    let mut i = 0;
    while i < raw.len() {
        let n = raw[i..].iter().take(63).take_while(|b| **b == raw[i]).count();
        if n > 1 || raw[i] >= 0xc0 {
            out.extend([0xc0 | n as u8, raw[i]]);
        } else {
            out.push(raw[i]);
        }
        i += n;
    }
    out
}

/// The pixels `raster_image` stores for `f`, row by row.
fn pixels(f: &[u8]) -> Vec<[u8; 4]> {
    let img = raster_image(f).unwrap();
    image::load_from_memory(&img.blob.bytes).unwrap().to_rgba8().pixels().map(|p| p.0).collect()
}

/// The 16 colors of a header palette: entry `i` is (16 i, 255 − 16 i, i).
fn header_colors() -> Vec<[u8; 3]> {
    (0..16u8).map(|i| [i * 16, 255 - i * 16, i]).collect()
}

fn opaque([r, g, b]: [u8; 3]) -> [u8; 4] {
    [r, g, b, 255]
}

#[test]
fn every_depth_and_plane_layout_reads() {
    let colors = header_colors();
    let (white, black) = ([255; 4], [0, 0, 0, 255]);

    // 1 bit, three pixels in a two-byte line: 101 and 010, in the header palette's first two colors
    // (here inverted: white, then black).
    let bits = rle(&[0b1010_0000, 0, 0b0100_0000, 0]);
    let f = pcx(5, 1, 1, 0, (3, 2), 2, 0, &[[255; 3], [0; 3]], &bits);
    assert_eq!(pixels(&f), [black, white, black, white, black, white]);
    let f = pcx(5, 1, 1, 0, (3, 2), 2, 0, &colors, &bits);
    let (zero, one) = (opaque(colors[0]), opaque(colors[1]));
    assert_eq!(pixels(&f), [one, zero, one, zero, one, zero]);
    // Black and white when the palette is left unset or the version has none.
    for (version, palette) in [(5, &[][..]), (3, &colors[..])] {
        let f = pcx(version, 1, 1, 0, (3, 2), 2, 0, palette, &bits);
        assert_eq!(pixels(&f), [white, black, white, black, white, black], "version {version}");
    }

    // 1 bit in four planes: plane p gives bit p of the index; indices 5 and 10.
    let planes = rle(&[0x80, 0, 0x40, 0, 0x80, 0, 0x40, 0]);
    let f = pcx(5, 1, 4, 0, (2, 1), 2, 0, &colors, &planes);
    assert_eq!(pixels(&f), [opaque(colors[5]), opaque(colors[10])], "the header palette");
    // Versions 0 and 3 have no palette in their header: the EGA colors.
    for version in [0, 3] {
        let f = pcx(version, 1, 4, 0, (2, 1), 2, 0, &colors, &planes);
        assert_eq!(pixels(&f), [[0xaa, 0, 0xaa, 255], [0x55, 0xff, 0x55, 255]], "version {version}");
    }
    // Two planes give four colors, three give eight: index 3 and 6 (planes 1 and 2).
    let f = pcx(5, 1, 3, 0, (2, 1), 2, 0, &colors, &rle(&[0x80, 0, 0xc0, 0, 0x40, 0]));
    assert_eq!(pixels(&f), [opaque(colors[3]), opaque(colors[6])]);

    // 2 bits: indices 0 to 3 in one byte, the first pixel in the top bits, in the CGA colors that
    // header bytes 16 (the background, in its top four bits) and 19 (C, P and I) pick.
    let cga = |background: u8, settings: u8| {
        let f = pcx(5, 2, 1, 0, (4, 1), 2, 0, &[[background, 0, 0], [settings, 0, 0]], &rle(&[0b0001_1011, 0]));
        pixels(&f)
    };
    let (blue, cyan, magenta, light_gray) = ([0, 0, 0xaa, 255], [0, 0xaa, 0xaa, 255], [0xaa, 0, 0xaa, 255], [0xaa; 3]);
    assert_eq!(cga(0x10, 0b0100_0000), [blue, cyan, magenta, opaque(light_gray)], "P = 1, dim");
    let (light_green, light_red, yellow) = ([0x55, 0xff, 0x55, 255], [0xff, 0x55, 0x55, 255], [0xff, 0xff, 0x55, 255]);
    assert_eq!(cga(0x0f, 0b0011_1111), [black, light_green, light_red, yellow], "P = 0, bright; low bits ignored");
    assert_eq!(cga(0xf0, 0b1010_0000), [white, [0x55, 0xff, 0xff, 255], light_red, white], "C = 1, bright");
    assert_eq!(cga(0, 0)[1..], [[0, 0xaa, 0, 255], [0xaa, 0, 0, 255], [0xaa, 0x55, 0, 255]], "P = 0, dim");

    // 4 bits: indices 15, 1 and 7.
    let f = pcx(5, 4, 1, 0, (3, 1), 2, 0, &colors, &rle(&[0xf1, 0x70]));
    assert_eq!(pixels(&f), [opaque(colors[15]), opaque(colors[1]), opaque(colors[7])]);

    // 8 bits with the 256-color palette after the image data, from a window that starts at x 10:
    // entry i is (i, 255 − i, 7).
    let mut f = pcx(5, 8, 1, 10, (2, 2), 2, 0, &[], &rle(&[0, 1, 254, 255]));
    f.push(0x0c);
    f.extend((0..=255u8).flat_map(|i| [i, 255 - i, 7]));
    assert_eq!(pixels(&f), [[0, 255, 7, 255], [1, 254, 7, 255], [254, 1, 7, 255], [255, 0, 7, 255]]);
    // Without it, the index is a gray level.
    let f = pcx(5, 8, 1, 0, (2, 2), 2, 0, &[], &rle(&[0, 1, 254, 255]));
    assert_eq!(pixels(&f), [[0, 0, 0, 255], [1, 1, 1, 255], [254, 254, 254, 255], [255; 4]]);
    // A gray level 12 769 bytes before the end of image data that runs to the end of the file.
    let mut levels = vec![1; 1000];
    levels[231] = 0x0c;
    let f = pcx(5, 8, 1, 0, (1000, 1), 1000, 0, &[], &levels);
    assert_eq!(f.len() - 769, 128 + 231);
    let read = pixels(&f);
    assert_eq!((read.len(), read[231], read[232]), (1000, [12, 12, 12, 255], [1, 1, 1, 255]));
    // A run may cross the end of a scan line.
    let f = pcx(5, 8, 1, 0, (2, 2), 2, 0, &[], &[0xc4, 9]);
    assert_eq!(pixels(&f), [[9, 9, 9, 255]; 4]);

    // 24 bits: each line holds its red, green and blue planes in turn; the padding byte is skipped.
    let f = pcx(5, 8, 3, 0, (1, 2), 2, 0, &[], &rle(&[10, 0, 30, 0, 50, 0, 20, 0, 40, 0, 60, 0]));
    assert_eq!(pixels(&f), [[10, 30, 50, 255], [20, 40, 60, 255]]);
}

#[test]
fn pcx_files_open_place_and_paste_at_their_resolution() {
    use crate::cmd::clipboard::{PASTE_ORDER, file_flavour};
    let b64 = vectorcraft_format::base64_encode;
    let f = pcx(5, 8, 3, 0, (2, 1), 2, 144, &[], &rle(&[10, 20, 30, 40, 50, 60]));
    assert_eq!(ppi::resolution(&f), Some((144.0, 144.0)));
    assert_eq!(ppi::resolution(&pcx(5, 8, 3, 0, (2, 1), 2, 0, &[], &[])), None, "no resolution");

    let mut s = Session::new();
    let r = s.execute("document.open", &json!({"name": "art.pcx", "dataBase64": b64(&f)})).unwrap();
    assert_eq!((&r["format"], &r["warnings"]), (&json!("pcx"), &json!([])), "{r}");
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect.size(), vectorcraft_geom::Size::new(1.0, 0.5), "2 × 1 px at 144 ppi");
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let r = s.execute("file.place", &json!({"name": "ART.PCX", "dataBase64": b64(&f)})).unwrap();
    assert_eq!((&r["format"], r["ids"].as_array().map(Vec::len)), (&json!("pcx"), Some(1)), "{r}");

    // Copied in a file manager, a PCX file pastes as a bitmap.
    assert_eq!(file_flavour("art.pcx", &f, &PASTE_ORDER), Some("image/x-pcx"));
    let r = s.execute("clipboard.importImage", &json!({"mime": "image/x-pcx", "dataBase64": b64(&f)})).unwrap();
    assert_eq!((&r["count"], &r["pixelWidth"], &r["pixelHeight"]), (&json!(1), &json!(2), &json!(1)), "{r}");
    // Glass's Load Texture… reads it too.
    assert!(texture_image(&f).is_ok());

    // The dialogs list PCX; the desktop packaging doesn't associate it; nothing writes it.
    assert!(OPEN_EXTS.contains(&"pcx") && PLACE_EXTS.contains(&"pcx") && UNASSOCIATED_EXTS.contains(&"pcx"));
    let filter = ("PCX", &["pcx"][..]);
    assert!(open_filters().any(|f| f == filter) && place_filters().any(|f| f == filter));
    let r = s.execute("document.formats", &json!({})).unwrap();
    let pcx_format = r["formats"].as_array().unwrap().iter().find(|f| f["id"] == "pcx").unwrap().clone();
    assert_eq!((pcx_format["read"].as_bool(), pcx_format["write"].as_bool()), (Some(true), Some(false)));
    assert_eq!(format_for_name("a.pcx").map(|f| f.id), Some("pcx"));
}

#[test]
fn damaged_and_unsupported_files_are_refused_with_a_message() {
    let message = |f: &[u8]| raster_image(f).err().expect("refused").to_string();
    let gray = pcx(5, 8, 1, 0, (2, 2), 2, 0, &[], &[1, 2, 3, 4]);

    // A header or image data cut short.
    assert_eq!(message(&gray[..60]), "the PCX file is cut short");
    assert_eq!(message(&gray[..131]), "the PCX file is cut short");
    assert_eq!(message(&pcx(5, 8, 1, 0, (2, 2), 2, 0, &[], &[1, 0xc2])), "the PCX file is cut short", "a run without its byte");
    // The palette after the image data isn't read as pixels the image data lacks.
    let mut short = pcx(5, 8, 1, 0, (2, 2), 2, 0, &[], &[1, 2, 3]);
    short.push(0x0c);
    short.extend([7; 768]);
    assert_eq!(message(&short), "the PCX file is cut short", "three pixels and a palette");
    // A palette marker after the image data with fewer than 768 colors after it.
    let mut cut_palette = gray.clone();
    cut_palette.push(0x0c);
    cut_palette.extend([7; 767]);
    assert_eq!(message(&cut_palette), "the PCX file is cut short", "a palette cut short");
    assert_eq!(message(&[&gray[..], &[0x0c]].concat()), "the PCX file is cut short", "a marker alone");
    let mut s = Session::new();
    let e = s.execute("document.open", &json!({"name": "a.pcx", "dataBase64": vectorcraft_format::base64_encode(&gray[..131])}));
    assert!(e.expect_err("refused").to_string().contains("the PCX file is cut short"));

    // Planes and depths the manual doesn't define.
    assert_eq!(message(&pcx(5, 8, 2, 0, (2, 2), 2, 0, &[], &[])), "PCX files with 8-bit pixels in 2 planes can't be read");
    assert_eq!(message(&pcx(5, 4, 3, 0, (2, 2), 2, 0, &[], &[])), "PCX files with 4-bit pixels in 3 planes can't be read");
    assert_eq!(message(&pcx(5, 1, 5, 0, (2, 2), 2, 0, &[], &[])), "PCX files with 1-bit pixels in 5 planes can't be read");

    // A window that ends before it starts, and lines too short for the width.
    let mut backwards = gray.clone();
    backwards[8..10].copy_from_slice(&0u16.to_le_bytes());
    backwards[4..6].copy_from_slice(&5u16.to_le_bytes());
    assert_eq!(message(&backwards), "the PCX file's header is damaged");
    assert_eq!(message(&pcx(5, 8, 1, 0, (3, 1), 2, 0, &[], &[1, 2, 3, 4])), "the PCX file's header is damaged");
    assert_eq!(message(&pcx(5, 1, 1, 0, (17, 1), 2, 0, &[], &[0; 4])), "the PCX file's header is damaged");

    // Past the allowance, from the header: the pixels as RGBA, and the scan lines of a narrow image
    // with long lines.
    let huge = pcx(5, 1, 1, 0, (0, 0), 8192, 0, &[], &[]);
    assert_eq!(message(&huge), "an image of 65536 × 65536 pixels is too large to draw");
    let long = pcx(5, 8, 3, 0, (1, 0), 65535, 0, &[], &[]);
    assert_eq!(message(&long), "an image of 1 × 65536 pixels is too large to draw");
    assert!(decode(&long, 1 << 20).is_err());

    // Headers with values the manual doesn't define aren't PCX files.
    assert!(is_pcx(&gray));
    for bad in [[0x0a, 1, 1, 8], [0x0a, 6, 1, 8], [0x0a, 5, 0, 8], [0x0a, 5, 1, 3], [0x0b, 5, 1, 8]] {
        assert!(!is_pcx(&bad), "{bad:?}");
    }
}
