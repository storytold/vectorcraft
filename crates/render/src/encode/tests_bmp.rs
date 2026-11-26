//! BMP export: depths, palettes and RLE, row order, layouts, alpha.

use super::bmp::{self, BmpOptions, DEPTHS};
use super::on_white;
use super::png::pixels_per_metre;
use super::quantize::PaletteOptions;

pub(super) fn u16_at(f: &[u8], at: usize) -> u16 {
    u16::from_le_bytes(f[at..at + 2].try_into().unwrap())
}

fn u32_at(f: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(f[at..at + 4].try_into().unwrap())
}

/// Straight RGBA: bands of a few colours with runs (as art is), a transparent and a half
/// transparent pixel in the first row.
pub(super) fn art(w: u32, h: u32) -> Vec<u8> {
    let colors = [[230, 40, 30, 255], [30, 90, 200, 255], [250, 250, 250, 255], [20, 160, 60, 255]];
    let mut px: Vec<u8> = (0..w * h).flat_map(|i| colors[((i % w) * 4 / w + (i / w) % 2) as usize % 4]).collect();
    px[..4].copy_from_slice(&[0, 0, 0, 0]);
    px[4..8].copy_from_slice(&[100, 0, 0, 128]);
    px
}

pub(super) fn decode(file: &[u8], format: craft_image_codecs::Format) -> image::RgbaImage {
    craft_image_codecs::decode_as(format, file)
        .map(|__cimg| {
            let (__w, __h) = __cimg.dimensions();
            match (__cimg.layout(), __cimg.sample_type()) {
                (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageLumaA8(image::GrayAlphaImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8) => {
                    image::DynamicImage::ImageRgb8(image::RgbImage::from_raw(__w, __h, __cimg.into_data()).unwrap())
                }
                _ => image::DynamicImage::ImageRgba8(image::RgbaImage::from_raw(__w, __h, __cimg.to_rgba8()).unwrap()),
            }
        })
        .expect("decodes")
        .to_rgba8()
}

fn bmp(px: &[u8], w: u32, h: u32, o: &BmpOptions) -> Vec<u8> {
    bmp::encode(px, w, h, 150.0, o, &PaletteOptions::default()).unwrap()
}

#[test]
fn every_depth_writes_its_bit_count_and_decodes() {
    let (w, h) = (13, 7);
    let px = art(w, h);
    for depth in DEPTHS {
        let file = bmp(&px, w, h, &BmpOptions { depth, ..Default::default() });
        assert_eq!(&file[..2], b"BM");
        assert_eq!(u32_at(&file, 2) as usize, file.len(), "file size");
        assert_eq!(u16_at(&file, 28), u16::from(depth), "bit count");
        assert_eq!(u32_at(&file, 38), pixels_per_metre(150.0), "resolution");
        let img = decode(&file, craft_image_codecs::Format::Bmp);
        assert_eq!(img.dimensions(), (w, h));
        let got = img.get_pixel(5, 3).0;
        let want = on_white(px[(3 * w as usize + 5) * 4..][..4].try_into().unwrap());
        match depth {
            1 => assert!(got[..3].iter().all(|c| *c == 0 || *c == 255), "black and white: {got:?}"),
            16 => assert!(got[..3].iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 8), "5 bits a channel: {got:?} vs {want:?}"),
            24 | 4 | 8 => assert_eq!(got[..3], want, "depth {depth}: few colours keep theirs exactly"),
            _ => {}
        }
        if depth == 32 {
            assert_eq!(u32_at(&file, 14), 108, "a version 4 header with an alpha mask");
            assert_eq!(img.get_pixel(0, 0).0[3], 0, "transparency kept");
            assert_eq!(img.get_pixel(1, 0).0[3], 128);
        } else {
            assert_eq!(img.get_pixel(0, 0).0, [255; 4], "flattened on white");
        }
    }
}

#[test]
fn rle8_and_rle4_decode_like_uncompressed() {
    let (w, h) = (61, 9);
    let px = art(w, h);
    for depth in [4, 8] {
        let plain = bmp(&px, w, h, &BmpOptions { depth, ..Default::default() });
        let rle = bmp(&px, w, h, &BmpOptions { depth, rle: true, ..Default::default() });
        assert_eq!(u32_at(&rle, 30), if depth == 8 { 1 } else { 2 }, "BI_RLE8 / BI_RLE4");
        assert_eq!(u32_at(&rle, 34) as usize, rle.len() - u32_at(&rle, 10) as usize, "the compressed size");
        assert!(rle.len() < plain.len(), "{} vs {}", rle.len(), plain.len());
        assert_eq!(decode(&rle, craft_image_codecs::Format::Bmp), decode(&plain, craft_image_codecs::Format::Bmp), "depth {depth}");
    }
    // Rows of many different pixels go in absolute mode.
    let noisy: Vec<u8> = (0..40u32 * 4).flat_map(|i| [(i * 37 % 256) as u8, (i * 91 % 256) as u8, (i * 13 % 256) as u8, 255]).collect();
    let plain = bmp(&noisy, 40, 4, &BmpOptions { depth: 8, ..Default::default() });
    let rle = bmp(&noisy, 40, 4, &BmpOptions { depth: 8, rle: true, ..Default::default() });
    assert_eq!(decode(&rle, craft_image_codecs::Format::Bmp), decode(&plain, craft_image_codecs::Format::Bmp));
}

#[test]
fn flipped_rows_have_a_negative_height() {
    let (w, h) = (6, 5);
    let px = art(w, h);
    let up = bmp(&px, w, h, &BmpOptions::default());
    let down = bmp(&px, w, h, &BmpOptions { top_down: true, ..Default::default() });
    assert_eq!(u32_at(&up, 22) as i32, 5);
    assert_eq!(u32_at(&down, 22) as i32, -5);
    let offset = u32_at(&down, 10) as usize;
    // Top-down: the first row stored is the image's first (its transparent pixel, on white).
    assert_eq!(&down[offset..offset + 3], &[255, 255, 255]);
    assert_eq!(decode(&up, craft_image_codecs::Format::Bmp), decode(&down, craft_image_codecs::Format::Bmp));
}

#[test]
fn os2_bitmaps_have_a_core_header() {
    let (w, h) = (9, 4);
    let px = art(w, h);
    for depth in bmp::OS2_DEPTHS {
        let file = bmp(&px, w, h, &BmpOptions { os2: true, depth, ..Default::default() });
        assert_eq!(u32_at(&file, 14), 12, "core header");
        assert_eq!((u16_at(&file, 18), u16_at(&file, 20), u16_at(&file, 24)), (9, 4, u16::from(depth)));
        // A three-byte entry for every index.
        let entries = if depth <= 8 { 1 << depth } else { 0 };
        assert_eq!(u32_at(&file, 10), 14 + 12 + entries * 3);
        let windows = bmp(&px, w, h, &BmpOptions { depth, ..Default::default() });
        assert_eq!(decode(&file, craft_image_codecs::Format::Bmp), decode(&windows, craft_image_codecs::Format::Bmp), "depth {depth}");
    }
}

#[test]
fn greys_and_impossible_combinations() {
    let (w, h) = (8, 2);
    let px = art(w, h);
    for depth in [8, 24] {
        let img = decode(&bmp(&px, w, h, &BmpOptions { depth, gray: true, ..Default::default() }), craft_image_codecs::Format::Bmp);
        assert!(img.pixels().all(|p| p[0] == p[1] && p[1] == p[2]), "depth {depth}: greys");
    }
    let bad = [
        BmpOptions { depth: 3, ..Default::default() },
        BmpOptions { os2: true, depth: 32, ..Default::default() },
        BmpOptions { rle: true, ..Default::default() },
        BmpOptions { rle: true, depth: 8, os2: true, ..Default::default() },
        BmpOptions { rle: true, depth: 8, top_down: true, ..Default::default() },
        BmpOptions { os2: true, top_down: true, ..Default::default() },
    ];
    for o in bad {
        assert!(o.check().is_err() && bmp::encode(&px, w, h, 72.0, &o, &PaletteOptions::default()).is_err(), "{o:?}");
    }
    assert!(bmp::encode(&px[..8], w, h, 72.0, &BmpOptions::default(), &PaletteOptions::default()).is_err(), "short buffer");
}
