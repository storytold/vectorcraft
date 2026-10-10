//! Reading Photoshop documents (#918): the merged image of every colour mode, depth and
//! compression, PSB files, transparency, the resolution, and damaged files refused without a panic.

use serde_json::{Value, json};

use super::psdread::{decode, is_psd, resolution};
use super::*;

/// A Photoshop document built from its parts: `version` 1 (PSD) or 2 (PSB), the header's fields,
/// then the colour mode data, resources, layer and mask section and image data as given.
#[allow(clippy::too_many_arguments)]
fn psd(
    version: u16,
    channels: u16,
    (w, h): (u32, u32),
    depth: u16,
    mode: u16,
    palette: &[u8],
    resources: &[u8],
    layers: &[u8],
    image: &[u8],
) -> Vec<u8> {
    let mut f = b"8BPS".to_vec();
    f.extend(version.to_be_bytes());
    f.extend([0; 6]);
    f.extend(channels.to_be_bytes());
    f.extend(h.to_be_bytes());
    f.extend(w.to_be_bytes());
    f.extend(depth.to_be_bytes());
    f.extend(mode.to_be_bytes());
    for part in [palette, resources] {
        f.extend((part.len() as u32).to_be_bytes());
        f.extend(part);
    }
    if version == 2 {
        f.extend((layers.len() as u64).to_be_bytes());
    } else {
        f.extend((layers.len() as u32).to_be_bytes());
    }
    f.extend(layers);
    f.extend(image);
    f
}

/// Raw image data: the compression method (0), then the planes.
fn raw(planes: &[&[u8]]) -> Vec<u8> {
    let mut d = vec![0, 0];
    planes.iter().for_each(|p| d.extend(*p));
    d
}

/// A layer and mask section whose layer info holds just the layer `count` (negative: the merged
/// image's first extra channel is its transparency).
fn layer_count(count: i16, psb: bool) -> Vec<u8> {
    let mut s = if psb { 2u64.to_be_bytes().to_vec() } else { 2u32.to_be_bytes().to_vec() };
    s.extend(count.to_be_bytes());
    s
}

/// Straight sRGB through a naive CMYK conversion (tests don't depend on colour settings).
fn naive(c: [f32; 4]) -> [f32; 3] {
    [0, 1, 2].map(|i| (1.0 - c[i]) * (1.0 - c[3]))
}

fn pixels(f: &[u8]) -> Vec<[u8; 4]> {
    decode(f, 1 << 30, naive).unwrap().pixels().map(|p| p.0).collect()
}

#[test]
fn every_colour_mode_and_depth_reads() {
    // Grayscale, 16 bits.
    let g = psd(1, 1, (2, 1), 16, 1, &[], &[], &[], &raw(&[&[0, 0, 0xff, 0xff]]));
    assert!(is_psd(&g));
    assert_eq!(pixels(&g), [[0, 0, 0, 255], [255, 255, 255, 255]]);
    // Bitmap: a set bit is black.
    let b = psd(1, 1, (9, 1), 1, 0, &[], &[], &[], &raw(&[&[0b1000_0000, 0b1000_0000]]));
    let px = pixels(&b);
    assert_eq!((px[0], px[1], px[8]), ([0, 0, 0, 255], [255, 255, 255, 255], [0, 0, 0, 255]));
    // Indexed: 256 reds, greens, blues.
    let mut palette = vec![0; 768];
    (palette[1], palette[257], palette[513]) = (10, 20, 30);
    let i = psd(1, 1, (1, 1), 8, 2, &palette, &[], &[], &raw(&[&[1]]));
    assert_eq!(pixels(&i), [[10, 20, 30, 255]]);
    // RGB, 32 bits: linear values, shown through sRGB's curve.
    let half = 0.5f32.to_be_bytes();
    let r = psd(1, 3, (1, 1), 32, 3, &[], &[], &[], &raw(&[&half, &half, &1.0f32.to_be_bytes()]));
    assert_eq!(pixels(&r), [[188, 188, 255, 255]]);
    // CMYK, inks stored as 255 − amount: 100 % K is black, no ink white.
    let c = psd(1, 4, (2, 1), 8, 4, &[], &[], &[], &raw(&[&[255, 255], &[255, 255], &[255, 255], &[0, 255]]));
    assert_eq!(pixels(&c), [[0, 0, 0, 255], [255, 255, 255, 255]]);
    // Lab: L 100, a 0, b 0 is white.
    let l = psd(1, 3, (1, 1), 8, 9, &[], &[], &[], &raw(&[&[255], &[128], &[128]]));
    assert!(pixels(&l)[0].iter().all(|&v| v >= 253), "{:?}", pixels(&l));
    // Duotone keeps its grayscale data in the merged image.
    assert_eq!(pixels(&psd(1, 1, (1, 1), 8, 8, &[0; 40], &[], &[], &raw(&[&[64]]))), [[64, 64, 64, 255]]);
}

#[test]
fn transparency_comes_back_straight_and_extra_channels_without_it_stay_out() {
    // Half-transparent red, stored on white: (255, 127, 127) with alpha 128.
    let planes: [&[u8]; 4] = [&[255], &[127], &[127], &[128]];
    let with = psd(1, 4, (1, 1), 8, 3, &[], &[], &layer_count(-1, false), &raw(&planes));
    let [r, g, b, a] = pixels(&with)[0];
    assert!(r == 255 && g <= 1 && b <= 1 && a == 128, "{:?}", [r, g, b, a]);
    // A flat document's extra channel is a saved selection, not transparency.
    let flat = psd(1, 4, (1, 1), 8, 3, &[], &[], &[], &raw(&planes));
    assert_eq!(pixels(&flat), [[255, 127, 127, 255]]);
    // A 16-bit document says so in its Lr16 block (its layer info is empty).
    let mut layers = 0u32.to_be_bytes().to_vec();
    layers.extend(0u32.to_be_bytes());
    layers.extend(b"8BIMLr16");
    layers.extend(4u32.to_be_bytes());
    layers.extend((-1i16).to_be_bytes());
    layers.extend([0, 0]);
    let wide: [&[u8]; 4] = [&[0xff, 0xff], &[0x7f, 0x7f], &[0x7f, 0x7f], &[0x80, 0x80]];
    let deep = psd(1, 4, (1, 1), 16, 3, &[], &[], &layers, &raw(&wide));
    let [r, g, _, a] = pixels(&deep)[0];
    assert!(r == 255 && g <= 1 && a == 128, "{:?}", pixels(&deep));
}

#[test]
fn packbits_zip_and_psb_files_read() {
    // PackBits in a PSB: 4-byte row counts, transparency from a negative layer count.
    let mut rle = vec![0, 1];
    for _ in 0..4 {
        rle.extend(2u32.to_be_bytes());
    }
    for v in [200u8, 100, 50, 255] {
        rle.extend([0xfe, v]); // a run of three
    }
    let big = psd(2, 4, (3, 1), 8, 3, &[], &[], &layer_count(-1, true), &rle);
    assert_eq!(pixels(&big), [[200, 100, 50, 255]; 3]);
    let mut s = Session::new();
    let r = s.execute("document.open", &json!({"name": "big.psb", "dataBase64": vectorcraft_format::base64_encode(&big)})).unwrap();
    assert_eq!(r["format"], "psb");
    // Zipped with prediction, 16 bits: each sample is the difference from the one before.
    let deltas: Vec<u8> = [0x1000u16, 0x1000, 0x1000].iter().flat_map(|d| d.to_be_bytes()).collect();
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    std::io::Write::write_all(&mut z, &deltas).unwrap();
    let mut zip = vec![0, 3];
    zip.extend(z.finish().unwrap());
    let px = pixels(&psd(1, 1, (3, 1), 16, 1, &[], &[], &[], &zip));
    assert_eq!(px.iter().map(|p| p[0]).collect::<Vec<_>>(), [16, 32, 48]);
}

#[test]
fn the_resolution_resource_sizes_the_placed_image() {
    // ResolutionInfo (1005): 300 ppi across, 118.11 per cm (300 ppi) down.
    let mut res = b"8BIM".to_vec();
    res.extend(0x03edu16.to_be_bytes());
    res.extend([0, 0]); // an empty name, padded
    res.extend(16u32.to_be_bytes());
    res.extend((300u32 << 16).to_be_bytes());
    res.extend([0, 1, 0, 1]);
    res.extend(((118.11f64 * 65536.0) as u32).to_be_bytes());
    res.extend([0, 2, 0, 1]);
    let f = psd(1, 3, (300, 150), 8, 3, &[], &res, &[], &raw(&[&[9; 45_000], &[9; 45_000], &[9; 45_000]]));
    let (x, y) = resolution(&f).unwrap();
    assert!((x - 300.0).abs() < 1e-9 && (y - 300.0).abs() < 0.01, "{x} {y}");
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let r = s.execute("file.place", &json!({"name": "photo.psd", "dataBase64": vectorcraft_format::base64_encode(&f)})).unwrap();
    assert!(
        (r["width"].as_f64().unwrap() - 72.0).abs() < 0.01 && (r["height"].as_f64().unwrap() - 36.0).abs() < 0.01,
        "an inch by half an inch: {r}"
    );
}

/// A 60 × 40 pt document: a red square on a transparent page, and a half-transparent blue one.
fn art() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 60, "height": 40})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 30, "height": 40})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 40, "y": 0, "width": 20, "height": 40})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    s
}

fn exported(s: &mut Session, p: Value) -> Vec<u8> {
    let r = s.execute("document.export", &p).unwrap();
    vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()
}

#[test]
fn psd_exports_open_and_place_again() {
    let mut s = art();
    let png = image::load_from_memory(&exported(&mut s, json!({"format": "png"}))).unwrap().to_rgba8();
    for (options, what) in [(json!({}), "layered RGB"), (json!({"layers": false}), "flat RGB"), (json!({"colorModel": "gray"}), "grey")] {
        let f = exported(&mut s, merge(json!({"format": "psd"}), options));
        let back = decode(&f, 1 << 30, naive).unwrap();
        assert_eq!(back.dimensions(), png.dimensions(), "{what}");
        let (red, clear, blue) = (back.get_pixel(10, 20).0, back.get_pixel(35, 20).0, back.get_pixel(50, 20).0);
        if what == "layered RGB" {
            assert_eq!((red, clear[3]), ([255, 0, 0, 255], 0), "{what}");
            assert!(blue[2] >= 250 && blue[0] <= 3 && (blue[3] as i32 - png.get_pixel(50, 20).0[3] as i32).abs() <= 1, "{what}: {blue:?}");
        } else if what == "flat RGB" {
            assert_eq!((red, clear), ([255, 0, 0, 255], [255, 255, 255, 255]), "{what}: flat files are on white");
        } else {
            assert!(red[0] == red[1] && red[3] == 255, "{what}: {red:?}");
        }
        // Open and Place take it by its content, whatever its name.
        let r = s.execute("document.open", &json!({"name": "back.bin", "dataBase64": vectorcraft_format::base64_encode(&f)})).unwrap();
        assert_eq!(r["format"], "psd", "{what}");
        let d = &s.doc().unwrap().doc;
        assert!((d.artboards[0].rect.width() - 60.0).abs() < 1e-6, "{what}");
    }
    let r = s.execute("document.formats", &json!({})).unwrap();
    assert!(r["readable"].as_array().unwrap().iter().any(|v| v == "psd"));
    assert!(PLACE_EXTS.contains(&"psd") && PLACE_EXTS.contains(&"psb") && OPEN_EXTS.contains(&"psb"));
}

#[test]
fn damaged_and_unsupported_files_are_refused_without_a_panic() {
    let good = psd(2, 4, (3, 2), 8, 3, &[], &[], &layer_count(-1, true), &raw(&[&[1; 6], &[2; 6], &[3; 6], &[4; 6]]));
    assert!(decode(&good, 1 << 30, naive).is_ok());
    for n in 0..good.len() {
        assert!(decode(&good[..n], 1 << 30, naive).is_err(), "cut at {n}");
        let _ = resolution(&good[..n]);
    }
    // Every byte changed in turn: an error or an image, never a panic.
    for i in 0..good.len() {
        for v in [0, 1, 0x7f, 0x80, 0xff] {
            let mut f = good.clone();
            f[i] = v;
            let _ = decode(&f, 1 << 30, naive);
            let _ = resolution(&f);
        }
    }
    // An image too large for the memory allowed is refused before anything is allocated.
    let huge = psd(2, 3, (300_000, 300_000), 16, 3, &[], &[], &[], &[0, 0]);
    assert!(decode(&huge, 1 << 30, naive).unwrap_err().contains("too large"));
    // Multichannel, and depths a mode doesn't have.
    assert!(decode(&psd(1, 2, (1, 1), 8, 7, &[], &[], &[], &raw(&[&[0], &[0]])), 1 << 30, naive).unwrap_err().contains("Multichannel"));
    assert!(decode(&psd(1, 4, (1, 1), 32, 4, &[], &[], &[], &[0, 0]), 1 << 30, naive).is_err());
    // A placed file with a damaged image reports it.
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let cut = &good[..good.len() - 3];
    assert!(s.execute("file.place", &json!({"name": "cut.psd", "dataBase64": vectorcraft_format::base64_encode(cut)})).is_err());
}
