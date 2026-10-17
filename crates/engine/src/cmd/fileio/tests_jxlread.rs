//! Reading JPEG XL images: the bare codestream and the container, alpha, 16-bit samples, the size
//! check before decoding, and damaged files refused without a panic.

use serde_json::json;
use zune_core::bit_depth::BitDepth;
use zune_core::colorspace::ColorSpace;
use zune_core::options::EncoderOptions;

use super::jxlread::{ToSrgb, decode, is_jxl, to_rgba};
use super::*;

/// A lossless JPEG XL codestream of `w` × `h` pixels: `samples` in `space`, 8-bit, or 16-bit in
/// native byte order.
fn jxl(w: usize, h: usize, space: ColorSpace, depth: BitDepth, samples: &[u8]) -> Vec<u8> {
    let mut out = vec![];
    zune_jpegxl::JxlSimpleEncoder::new(samples, EncoderOptions::new(w, h, space, depth)).encode(&mut out).unwrap();
    out
}

/// `codestream` in the ISOBMFF container: the signature box, `ftyp` and a `jxlc` box holding it.
fn container(codestream: &[u8]) -> Vec<u8> {
    let mut f = vec![0, 0, 0, 0x0c, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a];
    f.extend(20u32.to_be_bytes());
    f.extend(b"ftypjxl \0\0\0\0jxl ");
    f.extend((8 + codestream.len() as u32).to_be_bytes());
    f.extend(b"jxlc");
    f.extend(codestream);
    f
}

/// A 3 × 2 RGBA image: opaque red, green and blue, then half-transparent, clear and opaque gray.
const RGBA: [[u8; 4]; 6] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [200, 100, 50, 128], [9, 9, 9, 0], [77, 77, 77, 255]];

fn rgba8() -> Vec<u8> {
    jxl(3, 2, ColorSpace::RGBA, BitDepth::Eight, RGBA.as_flattened())
}

fn b64(bytes: &[u8]) -> String {
    vectorcraft_format::base64_encode(bytes)
}

/// The pixels of the one image the active document holds.
fn stored_pixels(s: &Session) -> Vec<[u8; 4]> {
    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.images.len(), 1);
    let blob = doc.images.values().next().unwrap();
    assert_eq!(blob.mime, "image/png", "stored as PNG");
    image::load_from_memory(&blob.bytes).unwrap().to_rgba8().pixels().map(|p| p.0).collect()
}

#[test]
fn jxl_images_open_and_place_with_their_pixels_and_alpha() {
    let bare = rgba8();
    let boxed = container(&bare);
    assert!(is_jxl(&bare) && is_jxl(&boxed) && !is_jxl(b"\xff\xd8\xff"));
    for (name, bytes) in [("bare.jxl", &bare), ("boxed.jxl", &boxed), ("named-wrong.png", &boxed)] {
        let mut s = Session::new();
        let r = s.execute("document.open", &json!({"name": name, "dataBase64": b64(bytes)})).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(r["format"], "jxl", "{name}: told by its content");
        let ab = s.doc().unwrap().doc.artboards[0].rect;
        assert_eq!((ab.width(), ab.height()), (3.0, 2.0), "{name}: 72 ppi, one point a pixel");
        assert_eq!(stored_pixels(&s), RGBA, "{name}");
        s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        let r = s.execute("file.place", &json!({"name": name, "dataBase64": b64(bytes)})).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(3.0), Some(2.0)), "{name}: {r}");
        assert_eq!(stored_pixels(&s), RGBA, "{name}: placed");
    }
    assert!(OPEN_EXTS.contains(&"jxl") && PLACE_EXTS.contains(&"jxl"));
    assert!(open_filters().any(|f| f == ("JPEG XL", &["jxl"][..])) && place_filters().any(|f| f == ("JPEG XL", &["jxl"][..])));
}

/// A `.jxl` file copied in a file manager pastes as its image.
#[test]
fn a_copied_jxl_file_pastes_as_its_image() {
    use crate::cmd::clipboard::{FILE_HEAD, PASTE_ORDER, file_flavour};
    let f = container(&rgba8());
    let mime = file_flavour("copied.jxl", &f[..f.len().min(FILE_HEAD)], &PASTE_ORDER);
    assert_eq!(mime, Some("image/jxl"));
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let r = s.execute("clipboard.importImage", &json!({"dataBase64": b64(&f), "mime": mime})).unwrap();
    assert_eq!((r["pixelWidth"].as_u64(), r["pixelHeight"].as_u64()), (Some(3), Some(2)), "{r}");
    s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(stored_pixels(&s), RGBA);
}

#[test]
fn sixteen_bit_samples_become_eight_bits() {
    let rgb: [[u16; 3]; 4] = [[0x0000, 0x8080, 0xffff], [0x1234, 0x80ff, 0x7f00], [0xff00, 0x00ff, 0x4000], [0xc000, 0x0101, 0xfefe]];
    let samples: Vec<u8> = rgb.as_flattened().iter().flat_map(|v| v.to_ne_bytes()).collect();
    let f = jxl(2, 2, ColorSpace::RGB, BitDepth::Sixteen, &samples);
    let to8 = |v: u16| (f64::from(v) / 65535.0 * 255.0).round() as u8;
    let want: Vec<[u8; 4]> = rgb.iter().map(|p| [to8(p[0]), to8(p[1]), to8(p[2]), 255]).collect();
    assert_eq!(decode(&f, 1 << 30).unwrap().pixels().map(|p| p.0).collect::<Vec<_>>(), want);
    // Gray, 16 bits, in the container, opened as a document.
    let gray: Vec<u8> = [0xffffu16, 0x8080, 0x0000, 0x4040].iter().flat_map(|v| v.to_ne_bytes()).collect();
    let f = container(&jxl(2, 2, ColorSpace::Luma, BitDepth::Sixteen, &gray));
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "gray.jxl", "dataBase64": b64(&f)})).unwrap();
    assert_eq!(stored_pixels(&s), [[255, 255, 255, 255], [128, 128, 128, 255], [0, 0, 0, 255], [64, 64, 64, 255]]);
}

/// A bare codestream that is only a header: `w` × `h` pixels and default metadata.
fn header_only(w: u32, h: u32) -> Vec<u8> {
    // Each field's bits, read from the lowest bit of each byte up: `div8`, the height as a U32
    // (selector 3: 1 + 30 bits), `ratio` 0, the width likewise, then `all_default` for the
    // metadata and `default_m` for the transform data.
    let fields = [(0, 1), (3, 2), (h - 1, 30), (0, 3), (3, 2), (w - 1, 30), (1, 1), (1, 1)];
    let (mut out, mut acc, mut n) = (vec![0xff, 0x0a], 0u64, 0);
    for (v, bits) in fields {
        acc |= u64::from(v) << n;
        n += bits;
        while n >= 8 {
            out.push(acc as u8);
            (acc, n) = (acc >> 8, n - 8);
        }
    }
    out.push(acc as u8);
    out
}

#[test]
fn an_image_too_large_is_refused_from_its_header() {
    // The header of a 100000 × 100000 image (40 GB as RGBA), with no pixel data after it.
    let huge = header_only(100_000, 100_000);
    assert_eq!(decode(&huge, MAX_RASTER_ALLOC).unwrap_err(), "an image of 100000 × 100000 pixels is too large to draw");
    let mut s = Session::new();
    let e = s.execute("document.open", &json!({"name": "huge.jxl", "dataBase64": b64(&huge)})).unwrap_err().to_string();
    assert!(e.contains("too large to draw"), "{e}");
    // The allowance counts RGBA bytes: 3 × 2 pixels take 24 (the decoder's own buffers then need more).
    assert!(decode(&rgba8(), 23).unwrap_err().contains("too large"));
    assert!(!decode(&rgba8(), 24).unwrap_err().contains("too large"));
    assert!(decode(&rgba8(), 1 << 20).is_ok());
}

/// Premultiplied colors are divided by their alpha before they are converted to sRGB: a linear
/// 0.25 at alpha 0.5 is a straight linear 0.5, which sRGB stores as 0.735 (188 of 255).
#[test]
fn associated_alpha_is_divided_out_before_the_color_conversion() {
    use jxl_oxide::{ColorEncodingWithProfile, EnumColourEncoding, NullCms, RenderingIntent};
    let srgb = EnumColourEncoding::srgb(RenderingIntent::Relative);
    let linear = EnumColourEncoding { tf: jxl_oxide::color::TransferFunction::Linear, ..srgb.clone() };
    let header = jxl_oxide::JxlImage::builder().read(&rgba8()[..]).unwrap();
    let m = &header.image_header().metadata;
    let to_srgb = ToSrgb::Transform(
        jxl_color::ColorTransform::new(
            &ColorEncodingWithProfile::new(linear),
            &ColorEncodingWithProfile::new(srgb),
            &m.opsin_inverse_matrix,
            &m.tone_mapping,
            &NullCms,
        )
        .unwrap(),
    );
    let mut colors = [vec![0.0; 2], vec![0.0; 2], vec![0.0; 2]];
    let mut out = [0u8; 8];
    for convert in [&to_srgb, &ToSrgb::Curve] {
        to_rgba(&[0.25, 0.25, 0.25, 0.5, 0.0, 0.0, 0.0, 0.0], 4, true, Some(convert), &mut colors, &mut out).unwrap();
        assert_eq!(out, [188, 188, 188, 128, 0, 0, 0, 0]);
        // Straight alpha is left as it is.
        to_rgba(&[0.25, 0.25, 0.25, 0.5, 1.0, 1.0, 1.0, 1.0], 4, false, Some(convert), &mut colors, &mut out).unwrap();
        assert_eq!(out, [137, 137, 137, 128, 255, 255, 255, 255]);
    }
    // Gray with another white point converts through three channels and comes back as gray.
    let gray_e = EnumColourEncoding {
        white_point: jxl_oxide::color::WhitePoint::E,
        tf: jxl_oxide::color::TransferFunction::Linear,
        ..EnumColourEncoding::gray_srgb(RenderingIntent::Relative)
    };
    let gray_srgb = ColorEncodingWithProfile::new(EnumColourEncoding::gray_srgb(RenderingIntent::Relative));
    let to_gray =
        jxl_color::ColorTransform::new(&ColorEncodingWithProfile::new(gray_e), &gray_srgb, &m.opsin_inverse_matrix, &m.tone_mapping, &NullCms)
            .unwrap();
    to_rgba(&[0.25, 0.5, 1.0, 1.0], 2, true, Some(&ToSrgb::Transform(to_gray)), &mut colors, &mut out).unwrap();
    assert_eq!(out, [188, 188, 188, 128, 255, 255, 255, 255]);
}

/// A container's Exif and Brotli-compressed boxes are left out: an unreadable `brob` box before the
/// codestream doesn't stop the image from opening.
#[test]
fn metadata_boxes_are_not_read() {
    let mut f = container(&[]);
    f.truncate(32); // the signature and `ftyp`
    let mut boxed = |ty: &[u8], body: &[u8]| {
        f.extend((8 + body.len() as u32).to_be_bytes());
        f.extend(ty);
        f.extend(body);
    };
    boxed(b"Exif", &[0, 0, 0, 0, b'M', b'M', 0, 42]);
    boxed(b"brob", &[b'x', b'm', b'l', b' ', 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    boxed(b"jxlc", &rgba8());
    assert_eq!(decode(&f, 1 << 30).unwrap().pixels().map(|p| p.0).collect::<Vec<_>>(), RGBA);
}

#[test]
fn damaged_files_are_refused_without_a_panic() {
    let good = rgba8();
    for n in 0..good.len() {
        assert!(decode(&good[..n], 1 << 30).is_err(), "cut at {n}");
    }
    // Every byte changed in turn: an error or an image, never a panic.
    for i in 0..good.len() {
        for v in [0, 1, 0x7f, 0x80, 0xff] {
            let mut f = good.clone();
            f[i] = v;
            let _ = decode(&f, 1 << 30);
        }
    }
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let cut = &good[..good.len() - 3];
    assert!(s.execute("file.place", &json!({"name": "cut.jxl", "dataBase64": b64(cut)})).is_err());
}
