//! TIFF export: byte order, LZW, colour models and samples, resolution, profile, strips.

use std::io::Cursor;

use tiff::decoder::{Decoder, DecodingResult};
use tiff::tags::Tag;

use super::jpeg::ColorModel;
use super::tiff::{self as t, ByteOrder, Compression, Image, Layout, Photometric, TiffOptions};

fn decoder(file: &[u8]) -> Decoder<Cursor<&[u8]>> {
    Decoder::new(Cursor::new(file)).expect("a TIFF")
}

fn pixels(file: &[u8]) -> Vec<u8> {
    match decoder(file).read_image().expect("decodes") {
        DecodingResult::U8(v) => v,
        _ => panic!("8-bit samples"),
    }
}

/// Straight RGBA: flat areas (as art is) with a transparent corner when `clear`.
fn art(w: u32, h: u32, clear: bool) -> Vec<u8> {
    (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            match (x * 3 / w, y * 2 / h) {
                (0, 0) if clear => [0, 0, 0, 0],
                (0, _) => [200, 30, 40, 255],
                (1, _) => [20, 120, 220, 255],
                _ => [255, 255, 255, 255],
            }
        })
        .collect()
}

fn opts(model: ColorModel) -> TiffOptions {
    TiffOptions { color_model: model, embed_icc: false, ..TiffOptions::default() }
}

#[test]
fn byte_order_writes_an_ii_or_mm_header() {
    let px = art(20, 10, false);
    for (order, magic) in [(ByteOrder::Little, b"II*\0"), (ByteOrder::Big, b"MM\0*")] {
        let file = t::encode_samples(&px, 20, 10, 72.0, &TiffOptions { byte_order: order, ..opts(ColorModel::Rgb) }).unwrap();
        assert_eq!(&file[..4], magic);
        let rgb: Vec<u8> = px.chunks(4).flat_map(|p| &p[..3]).copied().collect();
        assert_eq!(pixels(&file), rgb, "{order:?}");
        assert_eq!(decoder(&file).get_tag_ascii_string(Tag::Software).unwrap(), "VectorCraft");
    }
    for (s, o) in [("little", ByteOrder::Little), ("MM", ByteOrder::Big), ("mac", ByteOrder::Big), ("ii", ByteOrder::Little), ("big", ByteOrder::Big)]
    {
        assert_eq!(ByteOrder::from_id(s), Some(o), "{s}");
    }
    assert_eq!(ByteOrder::from_id("middle"), None);
}

#[test]
fn lzw_is_smaller_and_decodes_the_same() {
    let px = art(120, 90, true);
    let plain = t::encode_samples(&px, 120, 90, 72.0, &TiffOptions { lzw: false, ..opts(ColorModel::Rgb) }).unwrap();
    let lzw = t::encode_samples(&px, 120, 90, 72.0, &opts(ColorModel::Rgb)).unwrap();
    assert!(lzw.len() * 4 < plain.len(), "{} vs {}", lzw.len(), plain.len());
    assert_eq!(decoder(&lzw).get_tag_unsigned::<u16>(Tag::Compression).unwrap(), 5);
    assert_eq!(decoder(&plain).get_tag_unsigned::<u16>(Tag::Compression).unwrap(), 1);
    assert_eq!(pixels(&lzw), px, "transparency kept as an alpha channel");
    assert_eq!(pixels(&plain), px);
    // The image crate reads it too.
    let img = image::load_from_memory_with_format(&lzw, image::ImageFormat::Tiff).unwrap().to_rgba8();
    assert_eq!(img.into_raw(), px);
}

#[test]
fn colour_models_write_their_samples() {
    let px = art(9, 6, true);
    let rgba = t::encode_samples(&px, 9, 6, 72.0, &opts(ColorModel::Rgb)).unwrap();
    assert_eq!(decoder(&rgba).colortype().unwrap(), tiff::ColorType::RGBA(8));
    assert_eq!(decoder(&rgba).get_tag_u16_vec(Tag::ExtraSamples).unwrap(), [2], "unassociated alpha");
    let opaque = art(9, 6, false);
    let rgb = t::encode_samples(&opaque, 9, 6, 72.0, &opts(ColorModel::Rgb)).unwrap();
    assert_eq!(decoder(&rgb).colortype().unwrap(), tiff::ColorType::RGB(8));
    assert!(decoder(&rgb).find_tag(Tag::ExtraSamples).unwrap().is_none());

    let gray = t::encode_samples(&opaque, 9, 6, 72.0, &opts(ColorModel::Gray)).unwrap();
    assert_eq!(decoder(&gray).colortype().unwrap(), tiff::ColorType::Gray(8));
    assert_eq!(decoder(&gray).get_tag_unsigned::<u16>(Tag::PhotometricInterpretation).unwrap(), 1, "black is zero");
    let g = pixels(&gray);
    assert_eq!((g[0], g[8]), (super::jpeg::to_gray()([200, 30, 40]), 255), "lightness; white stays white");
    let on_white = t::encode_samples(&px, 9, 6, 72.0, &opts(ColorModel::Gray)).unwrap();
    assert_eq!(decoder(&on_white).colortype().unwrap(), tiff::ColorType::Gray(8), "grey has no alpha: few readers take it");
    assert_eq!(pixels(&on_white)[0], 255, "the transparent corner is white");

    // CMYK: four samples a pixel, ink amounts as given.
    let inks: Vec<u8> = (0..9 * 6 * 4).map(|i| (i * 7 % 256) as u8).collect();
    let cmyk = t::encode_samples(&inks, 9, 6, 72.0, &opts(ColorModel::Cmyk)).unwrap();
    let mut d = decoder(&cmyk);
    assert_eq!(d.colortype().unwrap(), tiff::ColorType::CMYK(8));
    assert_eq!(d.get_tag_unsigned::<u16>(Tag::SamplesPerPixel).unwrap(), 4);
    assert_eq!(d.get_tag_unsigned::<u16>(Tag::PhotometricInterpretation).unwrap(), 5, "separated");
    assert_eq!(d.get_tag_unsigned::<u16>(Tag::Unknown(332)).unwrap(), 1, "CMYK inks");
    assert_eq!(pixels(&cmyk), inks);
}

#[test]
fn resolution_tags_and_the_profile() {
    let px = art(4, 4, false);
    for (ppi, rational) in [(300.0, (300, 1)), (72.5, (7250, 100))] {
        let file = t::encode_samples(&px, 4, 4, ppi, &opts(ColorModel::Rgb)).unwrap();
        let mut d = decoder(&file);
        for tag in [Tag::XResolution, Tag::YResolution] {
            assert_eq!(d.get_tag(tag).unwrap(), tiff::decoder::ifd::Value::Rational(rational.0, rational.1));
        }
        assert_eq!(d.get_tag_unsigned::<u16>(Tag::ResolutionUnit).unwrap(), 2, "inches");
    }
    for model in ColorModel::ALL {
        let src = if model == ColorModel::Cmyk { vec![0; 64] } else { px.clone() };
        let file = t::encode_samples(&src, 4, 4, 72.0, &TiffOptions { embed_icc: true, ..opts(model) }).unwrap();
        let icc = decoder(&file).get_tag_u8_vec(Tag::IccProfile).unwrap();
        assert_eq!(&icc[36..40], b"acsp", "{model:?}: an ICC profile");
        let space: &[u8; 4] = match model {
            ColorModel::Rgb => b"RGB ",
            ColorModel::Cmyk => b"CMYK",
            ColorModel::Gray => b"GRAY",
        };
        assert_eq!(&icc[16..20], space, "{model:?}");
    }
    let file = t::encode_samples(&px, 4, 4, 72.0, &opts(ColorModel::Rgb)).unwrap();
    assert!(decoder(&file).find_tag(Tag::IccProfile).unwrap().is_none());
}

#[test]
fn big_images_take_several_strips_in_either_order() {
    let (w, h) = (300, 100);
    let px = art(w, h, false);
    for order in ByteOrder::ALL {
        for lzw in [false, true] {
            let file = t::encode_samples(&px, w, h, 72.0, &TiffOptions { lzw, byte_order: order, ..opts(ColorModel::Gray) }).unwrap();
            let mut d = decoder(&file);
            let offsets = d.get_tag_u32_vec(Tag::StripOffsets).unwrap();
            // 300 bytes a row: 27 rows a strip.
            assert_eq!(d.get_tag_u32(Tag::RowsPerStrip).unwrap(), 27);
            assert_eq!(offsets.len(), 4);
            assert!(offsets.iter().all(|o| o % 2 == 0), "strips start on a word");
            assert_eq!(pixels(&file).len(), (w * h) as usize, "{order:?} lzw {lzw}");
        }
    }
}

#[test]
fn packbits_and_bilevel_images_decode() {
    // 10 × 3 at one bit a pixel: two bytes a row, WhiteIsZero.
    let rows = [0b1010_1010u8, 0b1100_0000, 0xFF, 0xC0, 0, 0];
    let image = Image { width: 10, height: 3, data: &rows, photometric: Photometric::WhiteIsZero, samples: 1, bits: 1, alpha: false };
    let file = t::write(&image, &Layout { byte_order: ByteOrder::Big, compression: Compression::PackBits, ppi: 72.0, icc: None }).unwrap();
    let mut d = decoder(&file);
    assert_eq!(d.colortype().unwrap(), tiff::ColorType::Gray(1));
    assert_eq!(d.get_tag_unsigned::<u16>(Tag::Compression).unwrap(), 32773);
    // The decoder turns WhiteIsZero into black-is-zero.
    assert_eq!(pixels(&file), rows.map(|b| !b));
    // A buffer that doesn't match the size is refused, not written.
    let short = Image { data: &rows[..5], ..image };
    assert!(t::write(&short, &Layout { byte_order: ByteOrder::Little, compression: Compression::None, ppi: 72.0, icc: None }).is_err());
}

#[test]
fn packbits_round_trips() {
    for data in [vec![], vec![7], vec![1, 1, 1, 1, 2, 3, 3, 4, 4, 4], (0..400).map(|i| (i / 3 % 7) as u8).collect(), vec![9; 300]] {
        let mut packed = vec![];
        t::packbits(&data, &mut packed);
        let mut out = vec![];
        let mut i = 0;
        while i < packed.len() {
            let n = packed[i] as i8;
            if n >= 0 {
                out.extend(&packed[i + 1..i + 2 + n as usize]);
                i += 2 + n as usize;
            } else {
                out.extend(std::iter::repeat_n(packed[i + 1], (1 - n as i32) as usize));
                i += 2;
            }
        }
        assert_eq!(out, data);
    }
}
