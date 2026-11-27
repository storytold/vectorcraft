use super::quantize::{Dither, Indexed, PaletteOptions, Reduction, quantize, quantize_locked};
use super::web::{ColorSort, gif_comment, jpeg_comment, lossy, snap, sort, to_transparent};
use super::*;

/// A `w`×`h` opaque image of `f(x, y)` colours.
fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).flat_map(|(x, y)| [f(x, y)[0], f(x, y)[1], f(x, y)[2], 255]).collect()
}

fn colours(ix: &Indexed) -> Vec<[u8; 3]> {
    ix.indices.iter().map(|&i| ix.palette[i as usize]).collect()
}

#[test]
fn locked_colours_survive_a_reduction() {
    let px = image(64, 16, |x, y| [(x * 4) as u8, (y * 16) as u8, 90]);
    let rare = [7, 200, 13];
    let mut px = px;
    px[..4].copy_from_slice(&[rare[0], rare[1], rare[2], 255]);
    let o = PaletteOptions { colors: 4, dither: Dither::None, reduction: Reduction::Adaptive, ..Default::default() };
    assert!(!quantize(&px, 64, 16, &o).palette.contains(&rare), "a lone pixel loses its colour");
    let ix = quantize_locked(&px, 64, 16, &o, &[rare]);
    assert!(ix.palette.contains(&rare) && ix.palette.len() <= 4, "{:?}", ix.palette);
    assert_eq!(colours(&ix)[0], rare);
    // Black & White with black locked and one entry left: white takes it, though most pixels are black.
    let px = image(8, 8, |x, _| if x == 7 { [255; 3] } else { [0; 3] });
    let o = PaletteOptions { colors: 2, dither: Dither::None, reduction: Reduction::BlackWhite, ..Default::default() };
    let ix = quantize_locked(&px, 8, 8, &o, &[[0; 3]]);
    assert_eq!(ix.palette, [[0; 3], [255; 3]]);
    // With a light gray locked, black takes the entry left, though most pixels are white.
    let px = image(10, 1, |x, _| if x < 4 { [0; 3] } else { [255; 3] });
    let ix = quantize_locked(&px, 10, 1, &o, &[[200; 3]]);
    assert_eq!(ix.palette, [[200; 3], [0; 3]]);
}

#[test]
fn snapping_to_web_colours() {
    assert_eq!(snap([50, 103, 0], 0), None);
    assert_eq!(snap([50, 103, 0], 10), Some([51, 102, 0]));
    assert_eq!(snap([60, 103, 0], 10), None, "9 levels off is beyond 10%");
    assert_eq!(snap([60, 103, 0], 100), Some([51, 102, 0]));
    assert_eq!(snap([25, 25, 25], 100), Some([0, 0, 0]));
}

#[test]
fn mapped_colours_become_the_transparent_entry() {
    let px = image(4, 1, |x, _| [[255, 0, 0], [0, 0, 255], [0, 255, 0], [255, 0, 0]][x as usize]);
    let mut ix = quantize(&px, 4, 1, &PaletteOptions::default());
    assert_eq!(ix.transparent, None);
    let red = ix.palette.iter().position(|c| *c == [255, 0, 0]).unwrap();
    let map = to_transparent(&mut ix, |i| i == red);
    assert_eq!(map[red], None);
    assert_eq!(ix.transparent, Some(0));
    assert_eq!(ix.palette.len(), 3, "the transparent entry and blue and green");
    assert_eq!((ix.indices[0], ix.indices[3]), (0, 0));
    assert_eq!(ix.palette[ix.indices[1] as usize], [0, 0, 255]);
    // Mapping nothing changes nothing.
    let before = ix.clone();
    to_transparent(&mut ix, |_| false);
    assert_eq!(ix, before);
}

#[test]
fn sorting_keeps_every_pixel_colour() {
    let px: Vec<u8> = image(8, 8, |x, y| [(x * 30) as u8, 255 - (y * 30) as u8, ((x + y) * 15) as u8])
        .chunks(4)
        .enumerate()
        .flat_map(|(i, p)| [p[0], p[1], p[2], if i == 5 { 0 } else { 255 }])
        .collect();
    let ix = quantize(&px, 8, 8, &PaletteOptions { colors: 16, ..Default::default() });
    for order in ColorSort::ALL {
        let mut s = ix.clone();
        let new = sort(&mut s, order);
        assert_eq!(s.transparent, Some(0), "{order:?}: the transparent entry stays first");
        assert_eq!(new[0], 0);
        for (i, (&a, &b)) in ix.indices.iter().zip(&s.indices).enumerate() {
            assert_eq!(ix.palette[a as usize], s.palette[b as usize], "{order:?} pixel {i}");
        }
    }
    let mut s = ix.clone();
    sort(&mut s, ColorSort::Luminance);
    let luma = |c: &[u8; 3]| 299 * c[0] as u32 + 587 * c[1] as u32 + 114 * c[2] as u32;
    assert!(s.palette[1..].windows(2).all(|w| luma(&w[0]) <= luma(&w[1])));
    assert_eq!(ColorSort::from_id("popularity"), Some(ColorSort::Popularity));
}

#[test]
fn lossy_makes_runs_and_smaller_gifs() {
    let px = image(96, 64, |x, y| [(x * 2 + y) as u8, (y * 3) as u8, ((x * y) % 255) as u8]);
    let ix = quantize(&px, 96, 64, &PaletteOptions { colors: 64, dither: Dither::Diffusion, ..Default::default() });
    let mut lossy_ix = ix.clone();
    lossy(&mut lossy_ix, &px, 80, &[]);
    let runs = |ix: &Indexed| ix.indices.windows(2).filter(|w| w[0] != w[1]).count();
    assert!(runs(&lossy_ix) < runs(&ix));
    let size = |ix: &Indexed| gif::encode(ix, false).unwrap().len();
    assert!(size(&lossy_ix) < size(&ix), "{} vs {}", size(&lossy_ix), size(&ix));
    let mut none = ix.clone();
    lossy(&mut none, &px, 0, &[]);
    assert_eq!(none, ix);
    // A fixed entry keeps its pixels and takes no others.
    let fixed: Vec<u8> = (0..ix.palette.len()).filter_map(|i| u8::try_from(i).ok()).collect();
    let mut kept = ix.clone();
    lossy(&mut kept, &px, 100, &fixed);
    assert_eq!(kept, ix);
    // With one fixed entry of three, no pixel moves into or out of it, and the others still merge.
    let palette: Vec<[u8; 3]> = vec![[100, 0, 0], [110, 0, 0], [120, 0, 0]];
    let indices: Vec<u8> = vec![1, 0, 0, 1, 1, 2];
    let px: Vec<u8> = indices.iter().flat_map(|&i| palette[usize::from(i)].into_iter().chain([255])).collect();
    let mut part = Indexed { width: 2, height: 3, palette, transparent: None, indices };
    lossy(&mut part, &px, 100, &[0]);
    assert_eq!(part.indices, vec![1, 0, 0, 1, 1, 1]);
}

#[test]
fn comments_are_valid_files() {
    let ix = quantize(&image(5, 4, |x, _| [x as u8 * 40, 0, 0]), 5, 4, &PaletteOptions::default());
    let file = gif_comment(gif::encode(&ix, false).unwrap(), &"Copyright: Someone ".repeat(20));
    assert!(file.windows(2).any(|w| w == [0x21, 0xFE]));
    assert_eq!(
        craft_image_codecs::decode(&file)
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
            .unwrap()
            .width(),
        5
    );
    let px = vec![200u8; 5 * 4 * 3];
    let jpg = jpeg::encode(&px, 5, 4, 80, Some(72.0), &jpeg::JpegOptions::default()).unwrap();
    let with = jpeg_comment(jpg.clone(), "Copyright: Someone");
    assert_eq!(with.len(), jpg.len() + 4 + 18);
    assert!(with.windows(2).any(|w| w == [0xFF, 0xFE]));
    assert_eq!(
        craft_image_codecs::decode(&with)
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
            .unwrap()
            .height(),
        4
    );
    assert_eq!(jpeg_comment(vec![1, 2, 3], "x"), vec![1, 2, 3], "not a JPEG");
    let png = png::with_srgb(png::encode(&[0, 0, 0, 255], 1, 1, &png::PngOptions::default()).unwrap());
    assert!(png.windows(4).any(|w| w == b"sRGB"));
    assert_eq!(
        craft_image_codecs::decode(&png)
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
            .unwrap()
            .width(),
        1
    );
}
