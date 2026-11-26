use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::shapes;

use super::png::{PngOptions, encode_indexed};
use super::quantize::{Dither, Indexed, PaletteOptions, Reduction, clear_mask, nearest_entries, quantize, quantize_to};
use super::*;

/// A `w`×`h` opaque image of `f(x, y)` colours.
fn image(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    (0..h)
        .flat_map(|y| (0..w).map(move |x| (x, y)))
        .flat_map(|(x, y)| {
            let [r, g, b] = f(x, y);
            [r, g, b, 255]
        })
        .collect()
}

/// Smooth colour ramps: thousands of colours.
fn ramps(w: u32, h: u32) -> Vec<u8> {
    image(w, h, |x, y| [(x * 255 / w) as u8, (y * 255 / h) as u8, ((x + y) * 127 / (w + h)) as u8])
}

fn rgb(px: &[u8]) -> Vec<[u8; 3]> {
    px.chunks(4).map(|p| [p[0], p[1], p[2]]).collect()
}

/// The colour each pixel gets back.
fn colours(ix: &Indexed) -> Vec<[u8; 3]> {
    ix.indices.iter().map(|&i| ix.palette[i as usize]).collect()
}

/// The PNG chunk of type `ty` (first match).
fn chunk<'a>(file: &'a [u8], ty: &[u8; 4]) -> Option<&'a [u8]> {
    let mut i = 8;
    while i + 8 <= file.len() {
        let len = u32::from_be_bytes(file[i..i + 4].try_into().unwrap()) as usize;
        if &file[i + 4..i + 8] == ty {
            return Some(&file[i + 8..i + 8 + len]);
        }
        i += 12 + len;
    }
    None
}

#[test]
fn two_colour_art_gives_two_entries() {
    let px = image(9, 5, |x, _| if x < 4 { [200, 30, 40] } else { [10, 10, 90] });
    for reduction in [Reduction::Perceptual, Reduction::Selective, Reduction::Adaptive] {
        let ix = quantize(&px, 9, 5, &PaletteOptions { reduction, ..Default::default() });
        assert_eq!(ix.palette.len(), 2, "{reduction:?}");
        assert_eq!(ix.transparent, None);
        assert_eq!(colours(&ix), rgb(&px), "exact, no dithering");
    }
    // A hard-edged circle exported on white: black and white only.
    let mut d = Document::new(40.0, 30.0);
    let id = d.alloc_id();
    let l = d.layers[0].id;
    let n = Node::path(id, shapes::ellipse(Rect::new(5.3, 4.7, 31.1, 26.2)), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    d.insert(Some(l), 0, n).unwrap();
    let o = RasterExportOptions { background: Some([255; 3]), anti_alias: AntiAlias::None, ..Default::default() };
    let file = Renderer::new().export_region(&d, d.artboards[0].rect, RasterFormat::Png8, &o).unwrap();
    assert_eq!(file[25], 3, "colour type 3");
    assert_eq!(file[24], 1, "two entries fit one bit per pixel");
    assert_eq!(chunk(&file, b"PLTE").unwrap().len(), 6);
}

#[test]
fn colours_cap_the_palette() {
    let px = ramps(64, 48);
    for colors in [2, 16, 64, 256] {
        for dither in Dither::ALL {
            for reduction in Reduction::ALL {
                let ix = quantize(&px, 64, 48, &PaletteOptions { colors, dither, reduction, ..Default::default() });
                assert!(ix.palette.len() <= colors as usize, "{colors} {dither:?} {reduction:?}: {}", ix.palette.len());
                assert!(ix.indices.iter().all(|&i| (i as usize) < ix.palette.len()));
            }
        }
    }
    // Many colours use most of the palette.
    assert!(quantize(&px, 64, 48, &PaletteOptions { colors: 16, ..Default::default() }).palette.len() >= 12);
}

#[test]
fn fixed_palettes() {
    let px = ramps(32, 32);
    let web = quantize(&px, 32, 32, &PaletteOptions { reduction: Reduction::Web, ..Default::default() });
    assert!(web.palette.iter().flatten().all(|v| v % 51 == 0), "web-safe");
    let gray = quantize(&px, 32, 32, &PaletteOptions { reduction: Reduction::Gray, colors: 8, ..Default::default() });
    assert!(gray.palette.iter().all(|[r, g, b]| r == g && g == b) && gray.palette.len() <= 8);
    let bw = quantize(&px, 32, 32, &PaletteOptions { reduction: Reduction::BlackWhite, ..Default::default() });
    assert_eq!(bw.palette, [[0, 0, 0], [255, 255, 255]]);
}

#[test]
fn a_given_palette_keeps_the_entries_pixels_use() {
    let px: Vec<u8> = [[250, 10, 10, 255], [10, 10, 240, 255], [0, 0, 0, 40], [240, 20, 30, 255]].concat();
    let given = [[0, 255, 0], [255, 0, 0], [0, 0, 255], [255, 255, 255]];
    let o = PaletteOptions { dither: Dither::None, matte: None, ..Default::default() };
    let ix = quantize_to(&px, 4, 1, &o, &given);
    assert_eq!(ix.transparent, Some(0));
    assert_eq!(ix.palette[1..], [[255, 0, 0], [0, 0, 255]], "unused entries left out, the order kept");
    assert_eq!(ix.indices, [1, 2, 0, 1]);
    // With no transparent pixel all 256 of the given colors can be used; with one, the first 255
    // leave room for the transparent entry.
    let many: Vec<[u8; 3]> = (0..300u32).map(|i| [(i % 256) as u8, (i / 256) as u8, 0]).collect();
    let all = image(256, 2, |x, y| [x as u8, y as u8, 0]);
    let ix = quantize_to(&all, 256, 2, &o, &many);
    assert!(ix.palette.len() == 256 && ix.palette.iter().all(|c| many[..256].contains(c)), "{}", ix.palette.len());
    assert!(ix.palette.contains(&many[255]), "the 256th color is used");
    let mut clear = all.clone();
    clear[3] = 0;
    let ix = quantize_to(&clear, 256, 2, &o, &many);
    assert_eq!(ix.transparent, Some(0));
    assert!(ix.palette.len() <= 256 && ix.palette[1..].iter().all(|c| many[..255].contains(c)), "{}", ix.palette.len());
}

#[test]
fn dithering_keeps_the_average_colour() {
    let px = image(64, 16, |x, _| [(x * 4) as u8; 3]);
    let mean = |c: &[[u8; 3]]| c.iter().map(|p| p[0] as f64).sum::<f64>() / c.len().max(1) as f64;
    // Per 8-pixel column band, how far the result's mean is from the original's.
    let err = |dither| {
        let ix = quantize(&px, 64, 16, &PaletteOptions { reduction: Reduction::BlackWhite, dither, ..Default::default() });
        let (got, want) = (colours(&ix), rgb(&px));
        let band = |c: &[[u8; 3]], b: usize| c.iter().enumerate().filter(|(i, _)| (i % 64) / 8 == b).map(|(_, p)| *p).collect::<Vec<_>>();
        (0..8).map(|b| (mean(&band(&got, b)) - mean(&band(&want, b))).abs()).sum::<f64>()
    };
    let none = err(Dither::None);
    for d in [Dither::Diffusion, Dither::Pattern, Dither::Noise] {
        assert!(err(d) < none / 2.0, "{d:?}: {} vs {none}", err(d));
    }
    let noisy = PaletteOptions { dither: Dither::Noise, colors: 8, ..Default::default() };
    assert_eq!(quantize(&px, 64, 16, &noisy), quantize(&px, 64, 16, &noisy), "noise is reproducible");
}

#[test]
fn transparent_pixels_get_their_own_entry() {
    let px: Vec<u8> = (0..16u8).flat_map(|i| if i % 2 == 0 { [255, 0, 0, 255] } else { [0, 0, 255, if i < 8 { 0 } else { 100 }] }).collect();
    let ix = quantize(&px, 4, 4, &PaletteOptions::default());
    assert_eq!(ix.transparent, Some(0));
    assert_eq!(ix.palette.len(), 2, "red and the transparent entry: alpha 100 is under half");
    assert!(ix.indices.iter().enumerate().all(|(i, &v)| (v == 0) == (i % 2 == 1)));
    let opaque = quantize(&px, 4, 4, &PaletteOptions { transparency: false, matte: Some([0, 0, 0]), ..Default::default() });
    assert_eq!(opaque.transparent, None);
    assert!(opaque.palette.contains(&[0, 0, 0]) && opaque.palette.contains(&[0, 0, 100]), "blended over the matte: {:?}", opaque.palette);

    let file = encode_indexed(&ix, &PngOptions::default()).unwrap();
    assert_eq!(chunk(&file, b"tRNS").unwrap(), [0]);
    let back = craft_image_codecs::decode(&file)
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
        .to_rgba8();
    assert_eq!(back.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(back.get_pixel(1, 0)[3], 0);
}

#[test]
fn png8_decodes_at_every_bit_depth_and_interlaced() {
    for (colors, depth) in [(2, 1), (4, 2), (16, 4), (200, 8)] {
        let px = ramps(13, 11);
        let ix = quantize(&px, 13, 11, &PaletteOptions { colors, ..Default::default() });
        for interlaced in [false, true] {
            let file = encode_indexed(&ix, &PngOptions { ppi: Some(144.0), interlaced }).unwrap();
            assert_eq!((file[24], file[25], file[28]), (depth, 3, u8::from(interlaced)));
            let back = craft_image_codecs::decode(&file)
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
                .to_rgb8();
            let want: Vec<u8> = colours(&ix).into_iter().flatten().collect();
            assert_eq!(back.into_raw(), want, "{colors} colours, interlaced {interlaced}");
            assert!(chunk(&file, b"pHYs").is_some());
        }
    }
}

#[test]
fn gif_decodes() {
    let px: Vec<u8> = ramps(21, 17).chunks(4).enumerate().flat_map(|(i, p)| [p[0], p[1], p[2], if i % 5 == 0 { 0 } else { 255 }]).collect();
    let ix = quantize(&px, 21, 17, &PaletteOptions { colors: 32, ..Default::default() });
    for interlaced in [false, true] {
        let file = gif::encode(&ix, interlaced).unwrap();
        assert_eq!(&file[..6], b"GIF89a");
        let back = craft_image_codecs::decode_as(craft_image_codecs::Format::Gif, &file)
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
            .to_rgba8();
        assert_eq!(back.dimensions(), (21, 17));
        for (i, p) in back.pixels().enumerate() {
            match ix.indices[i] {
                0 => assert_eq!(p[3], 0, "transparent at {i}"),
                v => {
                    let [r, g, b] = ix.palette[v as usize];
                    assert_eq!(p.0, [r, g, b, 255], "{i} interlaced {interlaced}");
                }
            }
        }
    }
    let wide = Indexed { width: 70_000, height: 1, palette: vec![[0; 3]], transparent: None, indices: vec![0; 70_000] };
    assert!(gif::encode(&wide, false).is_err());
}

#[test]
fn ids_round_trip() {
    for r in Reduction::ALL {
        assert_eq!(Reduction::from_id(r.id()), Some(r));
    }
    for d in Dither::ALL {
        assert_eq!(Dither::from_id(d.id()), Some(d));
    }
}

/// A `w`×`h` red image whose pixels have the opacity `alpha(x, y)`.
fn see_through(w: u32, h: u32, alpha: impl Fn(u32, u32) -> u8) -> Vec<u8> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).flat_map(|(x, y)| [255, 0, 0, alpha(x, y)]).collect()
}

/// The share of the pixels of `ix` that are not transparent.
fn opaque_share(ix: &Indexed) -> f64 {
    ix.indices.iter().filter(|&&i| ix.transparent != Some(i)).count() as f64 / ix.indices.len() as f64
}

fn alpha_dither(d: Dither, amount: u8) -> PaletteOptions {
    PaletteOptions { transparency_dither: d, transparency_dither_amount: amount, ..Default::default() }
}

#[test]
fn transparency_dither_keeps_about_as_many_pixels_as_their_opacity() {
    let (w, h) = (32, 32);
    for alpha in [64u8, 128, 191] {
        let px = see_through(w, h, |_, _| alpha);
        // No transparency dither cuts at half opacity, and so does Diffusion at 0.
        let none = quantize(&px, w, h, &alpha_dither(Dither::None, 100));
        assert_eq!(opaque_share(&none), if alpha < 128 { 0.0 } else { 1.0 }, "none at {alpha}");
        assert_eq!(quantize(&px, w, h, &alpha_dither(Dither::Diffusion, 0)), none, "diffusion at 0, {alpha}");
        for d in [Dither::Diffusion, Dither::Pattern, Dither::Noise] {
            let ix = quantize(&px, w, h, &alpha_dither(d, 100));
            let share = opaque_share(&ix);
            assert!((share - f64::from(alpha) / 255.0).abs() < 0.08, "{d:?} at {alpha}: {share}");
            assert_eq!(ix.transparent, Some(0), "{d:?} at {alpha}");
            // The same pixels and options give the same image.
            assert_eq!(quantize(&px, w, h, &alpha_dither(d, 100)), ix, "{d:?} at {alpha}");
            // A fixed palette and the nearest entries make the same pixels transparent.
            let fixed = quantize_to(&px, w, h, &alpha_dither(d, 100), &[[255, 0, 0], [0, 0, 255]]);
            assert_eq!(opaque_share(&fixed), share, "{d:?} at {alpha}, fixed palette");
            let mut near = Vec::new();
            nearest_entries(&px, w, &alpha_dither(d, 100), &[[255, 0, 0]], |n| near.push(n));
            let mask = clear_mask(&px, w, &alpha_dither(d, 100));
            assert_eq!(near.iter().map(Option::is_none).collect::<Vec<_>>(), mask, "{d:?} at {alpha}");
        }
        // Diffusion at half strength keeps fewer pixels of a light area opaque than at full.
        if alpha < 128 {
            let half = opaque_share(&quantize(&px, w, h, &alpha_dither(Dither::Diffusion, 50)));
            let full = opaque_share(&quantize(&px, w, h, &alpha_dither(Dither::Diffusion, 100)));
            assert!(0.0 < half && half < full, "{alpha}: {half} {full}");
        }
    }
    // The methods arrange the pixels differently.
    let px = see_through(w, h, |_, _| 96);
    let masks: Vec<Vec<bool>> = [Dither::Diffusion, Dither::Pattern, Dither::Noise].map(|d| clear_mask(&px, w, &alpha_dither(d, 100))).into();
    assert!(masks[0] != masks[1] && masks[1] != masks[2] && masks[0] != masks[2]);
}

#[test]
fn transparency_dither_leaves_clear_and_opaque_pixels_and_needs_transparency() {
    let (w, h) = (16, 8);
    // Fully transparent on the left, opaque on the right: every method keeps them as they are.
    let hard = see_through(w, h, |x, _| if x < 8 { 0 } else { 255 });
    let none = clear_mask(&hard, w, &alpha_dither(Dither::None, 100));
    assert_eq!(none.iter().filter(|c| **c).count(), 64);
    for d in Dither::ALL {
        assert_eq!(clear_mask(&hard, w, &alpha_dither(d, 100)), none, "{d:?}");
    }
    // Without transparency nothing is transparent, whatever the transparency dither.
    let soft = see_through(w, h, |x, _| (x * 16) as u8);
    for d in Dither::ALL {
        let o = PaletteOptions { transparency: false, ..alpha_dither(d, 100) };
        assert!(!clear_mask(&soft, w, &o).contains(&true), "{d:?}");
        assert_eq!(quantize(&soft, w, h, &o).transparent, None, "{d:?}");
    }
    // A palette of two entries keeps one for the transparent pixels and one for the others.
    let o = PaletteOptions { colors: 2, ..alpha_dither(Dither::Pattern, 100) };
    let ix = quantize(&see_through(w, h, |_, _| 128), w, h, &o);
    assert_eq!((ix.palette.len(), ix.transparent), (2, Some(0)));
    // So does Black & White, with black and white pixels both left opaque.
    let bw = PaletteOptions { reduction: Reduction::BlackWhite, matte: None, ..o };
    let px: Vec<u8> = (0..64u32).flat_map(|i| if i % 8 < 4 { [0, 0, 0, 128] } else { [255, 255, 255, 128] }).collect();
    let ix = quantize(&px, 8, 8, &bw);
    assert_eq!(ix.transparent, Some(0));
    assert!(ix.palette.len() <= 2, "{:?}", ix.palette);
    // No pixels, and a width past the pixels there are.
    assert!(clear_mask(&[], 0, &alpha_dither(Dither::Diffusion, 100)).is_empty());
    assert_eq!(clear_mask(&see_through(3, 1, |_, _| 200), 1000, &alpha_dither(Dither::Diffusion, 100)).len(), 3);
}

#[test]
fn transparency_diffusion_passes_error_through_clear_and_opaque_pixels() {
    // Half-opaque pixels between fully transparent or fully opaque ones, in one row and in a
    // checkerboard: about half of them stay opaque, and the others keep their state.
    for other in [0u8, 255] {
        for (w, h, half) in [(64, 1, (|x, _| x % 2 == 0) as fn(u32, u32) -> bool), (32, 32, |x, y| (x + y) % 2 == 0)] {
            let px = see_through(w, h, |x, y| if half(x, y) { 128 } else { other });
            let mask = clear_mask(&px, w, &alpha_dither(Dither::Diffusion, 100));
            let at = |x: u32, y: u32| mask[(y * w + x) as usize];
            let cells: Vec<(u32, u32)> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).collect();
            assert!(cells.iter().filter(|&&(x, y)| !half(x, y)).all(|&(x, y)| at(x, y) == (other == 0)), "{other} {w}x{h}");
            let halves: Vec<_> = cells.iter().filter(|&&(x, y)| half(x, y)).collect();
            let share = halves.iter().filter(|&&&(x, y)| !at(x, y)).count() as f64 / halves.len() as f64;
            assert!((share - 128.0 / 255.0).abs() < 0.1, "{other} {w}x{h}: {share}");
        }
    }
}
