//! Reading JPEG XL images: the bare codestream and the container, alpha, 16-bit samples, the size
//! check before decoding, and damaged files refused without a panic.

use serde_json::json;
use zune_core::bit_depth::BitDepth;
use zune_core::colorspace::ColorSpace;
use zune_core::options::EncoderOptions;

use super::jxlread::{ToSrgb, decode, is_jxl, oriented_resolution, output_profile, pixel_boxes, to_rgba};
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

#[test]
fn container_copy_stays_within_the_input_allowance() {
    let f = container(&[0; 128]);
    assert!(pixel_boxes(&f, 64).unwrap_err().contains("too large"));
    assert_eq!(pixel_boxes(&f, f.len() as u64).unwrap().unwrap(), f);
}

#[test]
fn container_exif_resolution_sizes_open_and_place() {
    let bare = rgba8();
    let mut f = container(&[]);
    f.truncate(32);
    let mut exif = Vec::new();
    exif.extend(4u32.to_be_bytes());
    exif.extend(b"Exif");
    exif.extend(b"II*\0");
    exif.extend(8u32.to_le_bytes());
    exif.extend(3u16.to_le_bytes());
    for (tag, value) in [(282u16, 50u32), (283, 58), (296, 2)] {
        exif.extend(tag.to_le_bytes());
        exif.extend((if tag == 296 { 3u16 } else { 5 }).to_le_bytes());
        exif.extend(1u32.to_le_bytes());
        exif.extend(value.to_le_bytes());
    }
    exif.extend(0u32.to_le_bytes());
    for value in [144u32, 1, 72, 1] {
        exif.extend(value.to_le_bytes());
    }
    f.extend((8 + exif.len() as u32).to_be_bytes());
    f.extend(b"Exif");
    f.extend(&exif);
    f.extend((8 + bare.len() as u32).to_be_bytes());
    f.extend(b"jxlc");
    f.extend(&bare);
    assert_eq!(super::ppi::resolution(&f), Some((144.0, 72.0)));
    assert_eq!(super::ppi::resolution(&bare), None);
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "resolved.jxl", "dataBase64": b64(&f)})).unwrap();
    let rect = s.doc().unwrap().doc.artboards[0].rect;
    assert_eq!((rect.width(), rect.height()), (1.5, 2.0));
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let r = s.execute("file.place", &json!({"name": "resolved.jxl", "dataBase64": b64(&f)})).unwrap();
    assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(1.5), Some(2.0)));
    assert_eq!(super::ppi::resolution(&s.doc().unwrap().doc.images.values().next().unwrap().bytes), Some((144.0, 72.0)));
}

#[test]
fn jxl_axis_exchanging_orientations_swap_resolution() {
    let ppi = Some((144.0, 72.0));
    for orientation in 1..=8 {
        let expected = if orientation >= 5 { Some((72.0, 144.0)) } else { ppi };
        assert_eq!(oriented_resolution(ppi, orientation), expected, "orientation {orientation}");
    }
    assert_eq!(oriented_resolution(None, 6), None);
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
    assert_eq!(decode(&f, 1 << 30).unwrap().0.pixels().map(|p| p.0).collect::<Vec<_>>(), want);
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
    // The allowance counts RGBA bytes: 3 × 2 pixels take 24.
    assert!(decode(&rgba8(), 23).unwrap_err().contains("too large"));
    assert!(decode(&rgba8(), 1 << 20).is_ok());
}

/// The decoder's buffers take more than the image as RGBA. When they need more than the allowance,
/// the image is too large to draw, as when its size alone is.
#[test]
fn an_image_whose_decoder_buffers_pass_the_allowance_is_too_large() {
    assert_eq!(decode(&rgba8(), 24).unwrap_err(), "an image of 3 × 2 pixels is too large to draw");
    let gray: Vec<u8> = (0..64 * 64).map(|i| (i % 251) as u8).collect();
    let f = container(&jxl(64, 64, ColorSpace::Luma, BitDepth::Eight, &gray));
    assert_eq!(decode(&f, 64 * 64 * 4).unwrap_err(), "an image of 64 × 64 pixels is too large to draw");
    assert!(decode(&f, 1 << 20).is_ok());
    // A file cut short is still reported as cut short or unreadable.
    let e = decode(&f[..f.len() - 3], 1 << 20).unwrap_err();
    assert!(!e.contains("too large"), "{e}");
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
    assert_eq!(decode(&f, 1 << 30).unwrap().0.pixels().map(|p| p.0).collect::<Vec<_>>(), RGBA);
}

/// Colors left in their embedded RGB profile keep it: the PNG they are stored in carries the
/// profile. Colors converted to sRGB, and a gray image's colors, which come back as RGB, are stored
/// without one.
#[test]
fn colors_left_in_their_icc_profile_keep_it() {
    use image::ImageDecoder as _;
    use jxl_oxide::{EnumColourEncoding, RenderingIntent};
    // A profile jxl-color can't turn into a JPEG XL color space, so the decoder leaves the colors
    // in it: the sRGB profile with the tag of its red curve renamed.
    let unnamed = |encoding: &EnumColourEncoding, curve: &[u8; 4]| {
        let mut icc = jxl_color::icc::colour_encoding_to_icc(encoding);
        let count = u32::from_be_bytes(icc[128..132].try_into().unwrap()) as usize;
        for at in (0..count).map(|i| 132 + 12 * i) {
            if icc[at..at + 4] == curve[..] {
                icc[at..at + 4].copy_from_slice(b"xTRC");
            }
        }
        icc
    };
    let srgb = EnumColourEncoding::srgb(RenderingIntent::Relative);
    let rgb = unnamed(&srgb, b"rTRC");
    assert_ne!(rgb, jxl_color::icc::colour_encoding_to_icc(&srgb), "a tag renamed");
    assert_eq!(output_profile(false, false, Some(&rgb)).unwrap(), Some(rgb.clone()));
    assert_eq!(output_profile(true, false, Some(&rgb)).unwrap(), None, "XYB-encoded colors become sRGB");
    assert_eq!(output_profile(false, false, Some(&jxl_color::icc::colour_encoding_to_icc(&srgb))).unwrap(), None, "converted to sRGB");
    let gray = unnamed(&EnumColourEncoding::gray_srgb(RenderingIntent::Relative), b"kTRC");
    assert!(!jxl_oxide::ColorEncodingWithProfile::with_icc(&gray).unwrap().icc_profile().is_empty());
    assert_eq!(output_profile(false, true, Some(&gray)).unwrap_err(), "the JPEG XL file can't be read: unsupported grayscale ICC profile");
    assert_eq!(
        output_profile(false, true, Some(&jxl_color::icc::colour_encoding_to_icc(&EnumColourEncoding::gray_srgb(RenderingIntent::Relative))))
            .unwrap(),
        None
    );
    assert_eq!(output_profile(false, false, None).unwrap(), None);
    // The PNG carries the profile, the pixels and the resolution.
    let img = image::RgbaImage::from_raw(2, 1, vec![200, 100, 50, 255, 9, 9, 9, 0]).unwrap();
    for icc in [Some(rgb), None] {
        let RasterImage { blob, width, height, .. } = super::load::as_png(img.clone(), icc.clone(), Some((144.0, 144.0))).unwrap();
        assert_eq!((width, height), (2, 1));
        let mut png = image::codecs::png::PngDecoder::new(std::io::Cursor::new(&blob.bytes[..])).unwrap();
        assert_eq!(png.icc_profile().unwrap(), icc);
        assert_eq!(image::DynamicImage::from_decoder(png).unwrap().to_rgba8(), img);
        assert_eq!(super::ppi::resolution(&blob.bytes).map(|(x, y)| (x.round(), y.round())), Some((144.0, 144.0)));
    }
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
