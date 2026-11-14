//! TIFF export: byte order, LZW, colour models and samples, resolution, profile, strips.

use craft_tiff::Tiff;
use craft_tiff::tags;

use super::jpeg::ColorModel;
use super::tiff::{self as t, ByteOrder, Compression, Image, Layout, Photometric, TiffOptions};

fn ifd0(file: &[u8]) -> craft_tiff::reader::Ifd {
    Tiff::parse(file).expect("a TIFF").ifds.into_iter().next().expect("IFD0")
}

fn tag_u16_vec(ifd: &craft_tiff::reader::Ifd, tag: u16) -> Option<Vec<u16>> {
    ifd.value(tag).map(|v| v.to_u64_vec().into_iter().map(|x| x as u16).collect())
}

fn tag_u32_vec(ifd: &craft_tiff::reader::Ifd, tag: u16) -> Option<Vec<u32>> {
    ifd.value(tag).map(|v| v.to_u64_vec().into_iter().map(|x| x as u32).collect())
}

fn pixels(file: &[u8]) -> Vec<u8> {
    craft_image_codecs::decode_as(craft_image_codecs::Format::Tiff, file).expect("decodes").data
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
        assert_eq!(ifd0(&file).string(tags::SOFTWARE).as_deref(), Some("VectorCraft"));
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
    assert_eq!(ifd0(&lzw).u16(tags::COMPRESSION), Some(5));
    assert_eq!(ifd0(&plain).u16(tags::COMPRESSION), Some(1));
    assert_eq!(pixels(&lzw), px, "transparency kept as an alpha channel");
    assert_eq!(pixels(&plain), px);
    let img = craft_image_codecs::decode_as(craft_image_codecs::Format::Tiff, &lzw).unwrap();
    assert_eq!(img.data, px);
}

#[test]
fn colour_models_write_their_samples() {
    let px = art(9, 6, true);
    let rgba = t::encode_samples(&px, 9, 6, 72.0, &opts(ColorModel::Rgb)).unwrap();
    let rgba_ifd = ifd0(&rgba);
    assert_eq!(rgba_ifd.u16(tags::PHOTOMETRIC_INTERPRETATION), Some(2));
    assert_eq!(rgba_ifd.u16(tags::SAMPLES_PER_PIXEL), Some(4));
    assert_eq!(tag_u16_vec(&rgba_ifd, tags::BITS_PER_SAMPLE).as_deref(), Some([8, 8, 8, 8].as_slice()));
    assert_eq!(tag_u16_vec(&rgba_ifd, tags::EXTRA_SAMPLES).as_deref(), Some([2].as_slice()), "unassociated alpha");
    let opaque = art(9, 6, false);
    let rgb = t::encode_samples(&opaque, 9, 6, 72.0, &opts(ColorModel::Rgb)).unwrap();
    let rgb_ifd = ifd0(&rgb);
    assert_eq!(rgb_ifd.u16(tags::PHOTOMETRIC_INTERPRETATION), Some(2));
    assert_eq!(rgb_ifd.u16(tags::SAMPLES_PER_PIXEL), Some(3));
    assert!(rgb_ifd.value(tags::EXTRA_SAMPLES).is_none());

    let gray = t::encode_samples(&opaque, 9, 6, 72.0, &opts(ColorModel::Gray)).unwrap();
    let gray_ifd = ifd0(&gray);
    assert_eq!(gray_ifd.u16(tags::SAMPLES_PER_PIXEL), Some(1));
    assert_eq!(tag_u16_vec(&gray_ifd, tags::BITS_PER_SAMPLE).as_deref(), Some([8].as_slice()));
    assert_eq!(gray_ifd.u16(tags::PHOTOMETRIC_INTERPRETATION), Some(1), "black is zero");
    let g = pixels(&gray);
    assert_eq!((g[0], g[8]), (super::jpeg::to_gray()([200, 30, 40]), 255), "lightness; white stays white");
    let on_white = t::encode_samples(&px, 9, 6, 72.0, &opts(ColorModel::Gray)).unwrap();
    let on_white_ifd = ifd0(&on_white);
    assert_eq!(on_white_ifd.u16(tags::SAMPLES_PER_PIXEL), Some(1), "grey has no alpha: few readers take it");
    assert_eq!(pixels(&on_white)[0], 255, "the transparent corner is white");

    // CMYK: four samples a pixel, ink amounts as given.
    let inks: Vec<u8> = (0..9 * 6 * 4).map(|i| (i * 7 % 256) as u8).collect();
    let cmyk = t::encode_samples(&inks, 9, 6, 72.0, &opts(ColorModel::Cmyk)).unwrap();
    let d = ifd0(&cmyk);
    assert_eq!(d.u16(tags::SAMPLES_PER_PIXEL), Some(4));
    assert_eq!(d.u16(tags::PHOTOMETRIC_INTERPRETATION), Some(5), "separated");
    assert_eq!(d.u16(332), Some(1), "CMYK inks");
    let cmyk_img = craft_image_codecs::decode_as(craft_image_codecs::Format::Tiff, &cmyk).unwrap();
    assert_eq!(cmyk_img.data, inks);
}

#[test]
fn resolution_tags_and_the_profile() {
    let px = art(4, 4, false);
    for (ppi, rational) in [(300.0, (300, 1)), (72.5, (7250, 100))] {
        let file = t::encode_samples(&px, 4, 4, ppi, &opts(ColorModel::Rgb)).unwrap();
        let d = ifd0(&file);
        for tag in [tags::X_RESOLUTION, tags::Y_RESOLUTION] {
            let Some(craft_tiff::reader::IfdValue::Rationals(v)) = d.value(tag) else { panic!("rational") };
            assert_eq!(v.as_slice(), &[rational]);
        }
        assert_eq!(d.u16(tags::RESOLUTION_UNIT), Some(2), "inches");
    }
    for model in ColorModel::ALL {
        let src = if model == ColorModel::Cmyk { vec![0; 64] } else { px.clone() };
        let file = t::encode_samples(&src, 4, 4, 72.0, &TiffOptions { embed_icc: true, ..opts(model) }).unwrap();
        let icc = ifd0(&file).bytes(tags::ICC_PROFILE).unwrap();
        assert_eq!(&icc[36..40], b"acsp", "{model:?}: an ICC profile");
        let space: &[u8; 4] = match model {
            ColorModel::Rgb => b"RGB ",
            ColorModel::Cmyk => b"CMYK",
            ColorModel::Gray => b"GRAY",
        };
        assert_eq!(&icc[16..20], space, "{model:?}");
    }
    let file = t::encode_samples(&px, 4, 4, 72.0, &opts(ColorModel::Rgb)).unwrap();
    assert!(ifd0(&file).value(tags::ICC_PROFILE).is_none());
}

#[test]
fn big_images_take_several_strips_in_either_order() {
    let (w, h) = (300, 100);
    let px = art(w, h, false);
    for order in ByteOrder::ALL {
        for lzw in [false, true] {
            let file = t::encode_samples(&px, w, h, 72.0, &TiffOptions { lzw, byte_order: order, ..opts(ColorModel::Gray) }).unwrap();
            let d = ifd0(&file);
            let offsets = tag_u32_vec(&d, tags::STRIP_OFFSETS).unwrap();
            // 300 bytes a row: 27 rows a strip.
            assert_eq!(d.u32(tags::ROWS_PER_STRIP), Some(27));
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
    let d = ifd0(&file);
    assert_eq!(d.u16(tags::SAMPLES_PER_PIXEL), Some(1));
    assert_eq!(tag_u16_vec(&d, tags::BITS_PER_SAMPLE).as_deref(), Some([1].as_slice()));
    assert_eq!(d.u16(tags::COMPRESSION), Some(32773));
    // The decoder turns WhiteIsZero into black-is-zero.
    let dec = craft_image_codecs::decode_as(craft_image_codecs::Format::Tiff, &file).unwrap();
    assert_eq!(dec.width, 10);
    assert_eq!(dec.height, 3);
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
