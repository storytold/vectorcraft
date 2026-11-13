use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::shapes;

use super::png::{PngOptions, encode_indexed};
use super::quantize::{Dither, Indexed, PaletteOptions, Reduction, quantize, quantize_to};
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
            for reduction in [Reduction::Perceptual, Reduction::Selective, Reduction::Adaptive, Reduction::Web, Reduction::Gray] {
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
    // At most 255 of the given colors are used, leaving room for the transparent entry.
    let many: Vec<[u8; 3]> = (0..300u32).map(|i| [(i % 256) as u8, (i / 256) as u8, 0]).collect();
    let all = image(256, 2, |x, y| [x as u8, y as u8, 0]);
    let ix = quantize_to(&all, 256, 2, &o, &many);
    assert!(ix.palette.len() <= 255 && ix.palette.iter().all(|c| many[..255].contains(c)), "{}", ix.palette.len());
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
    let back = image::load_from_memory(&file).unwrap().to_rgba8();
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
            let back = image::load_from_memory(&file).unwrap().to_rgb8();
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
        let back = image::load_from_memory_with_format(&file, image::ImageFormat::Gif).unwrap().to_rgba8();
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
