//! Grayscale and 1-bit images colorized by a fill color draw in that color, white pixels
//! transparent.

use crate::*;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::shapes;

/// An uncompressed 1-bit TIFF, BlackIsZero, of 2 × 1 pixels: black, white.
fn bilevel_tiff() -> Vec<u8> {
    let entries: [(u16, u16, u32); 9] =
        [(256, 3, 2), (257, 3, 1), (258, 3, 1), (259, 3, 1), (262, 3, 1), (273, 4, 122), (277, 3, 1), (278, 3, 1), (279, 4, 1)];
    let mut out = b"II*\0".to_vec();
    out.extend(8u32.to_le_bytes());
    out.extend((entries.len() as u16).to_le_bytes());
    for (tag, kind, value) in entries {
        out.extend(tag.to_le_bytes());
        out.extend(kind.to_le_bytes());
        out.extend(1u32.to_le_bytes());
        out.extend(value.to_le_bytes());
    }
    out.extend(0u32.to_le_bytes());
    out.push(0b0100_0000);
    out
}

/// A 2 × 1 gray PNG: black, white.
fn gray_png() -> Vec<u8> {
    let img = image::GrayImage::from_raw(2, 1, vec![0, 255]).unwrap();
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    png
}

/// A 40 × 20 pt page: a green band behind the image of `blob` (black, white), 20 pt a pixel,
/// colorized with `colorize`.
fn doc(blob: ImageBlob, colorize: Option<Paint>) -> Document {
    doc_of(blob, colorize, (2, 1), 20.0)
}

/// [`doc`] for an image of `size` pixels, `scale` pt a pixel.
fn doc_of(blob: ImageBlob, colorize: Option<Paint>, (width, height): (u32, u32), scale: f64) -> Document {
    let mut d = Document::new(40.0, 20.0);
    let layer = Some(d.layers[0].id);
    let band = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 20.0)),
        Appearance::basic(Paint::solid(Color::rgb(0.0, 1.0, 0.0)), Paint::None, 0.0),
    );
    d.insert(layer, 0, band).unwrap();
    d.images.insert("pic".into(), blob);
    let im = ImageObject { key: "pic".into(), width, height, xf: Affine::scale(scale), link: None, placement: Default::default(), pixels: None };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    n.extras_mut().colorize = colorize.map(Box::new);
    d.insert(layer, 1, n).unwrap();
    d
}

#[test]
fn gray_and_1_bit_images_draw_in_their_spot_color_with_white_transparent() {
    let ink = Color::Cmyk { c: 0.0, m: 1.0, y: 1.0, k: 0.0 };
    let spot = Paint::Solid { color: ink, swatch: Some("Ink".into()), tint: 1.0 };
    let sharp = RenderOptions { smooth_images: false, ..Default::default() };
    let shown = Renderer::new().render(&doc(ImageBlob::new("image/png", gray_png()), None), 40, 20, Affine::IDENTITY, &sharp);
    assert_eq!(shown.pixel(10, 10), [0, 0, 0, 255], "uncolorized: black");
    let [r, g, b] = ink.to_rgb().map(|v| (v * 255.0).round() as u8);
    for (what, blob) in [("1-bit", ImageBlob::new("image/tiff", bilevel_tiff())), ("gray", ImageBlob::new("image/png", gray_png()))] {
        for image_mips in [false, true] {
            let d = doc(blob.clone(), Some(spot.clone()));
            let out = Renderer::new().render(&d, 40, 20, Affine::IDENTITY, &RenderOptions { image_mips, smooth_images: false, ..Default::default() });
            let px = out.pixel(10, 10);
            assert!(px[0].abs_diff(r) <= 2 && px[1].abs_diff(g) <= 2 && px[2].abs_diff(b) <= 2, "{what}: the black pixel takes the ink, got {px:?}");
            assert_eq!(out.pixel(30, 10), [0, 255, 0, 255], "{what}: the white pixel shows the band below");
        }
    }
}

/// A shrunk copy kept from one frame draws colorized in the next, and the uncolorized image keeps
/// its own copy.
#[test]
fn shrunk_copies_are_kept_per_color() {
    let black = image::GrayImage::from_raw(64, 64, vec![0; 64 * 64]).unwrap();
    let mut png = vec![];
    black.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let blob = ImageBlob::new("image/png", png);
    let d = doc_of(blob.clone(), Some(Paint::solid(Color::rgb(0.0, 0.0, 1.0))), (64, 64), 1.0);
    let plain = doc_of(blob, None, (64, 64), 1.0);
    let opts = RenderOptions { image_mips: true, ..Default::default() };
    let view = Affine::scale(0.25);
    let mut r = Renderer::new();
    for _ in 0..2 {
        let px = r.render(&d, 16, 16, view, &opts).pixel(8, 8);
        assert!(px[2] >= 250 && px[0] <= 5 && px[1] <= 5, "blue, got {px:?}");
        let px = r.render(&plain, 16, 16, view, &opts).pixel(8, 8);
        assert!(px[..3].iter().all(|v| *v <= 5), "black, got {px:?}");
    }
}

/// With `kept_images_only` (the canvas's low-resolution fill), a colorized image draws from the
/// colorized pixels the store holds, not from the image's own.
#[test]
fn kept_images_only_draw_the_colorized_pixels() {
    let black = image::GrayImage::from_raw(48, 48, vec![0; 48 * 48]).unwrap();
    let mut png = vec![];
    black.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let d = doc_of(ImageBlob::new("image/png", png), Some(Paint::solid(Color::rgb(0.0, 0.0, 1.0))), (48, 48), 1.0);
    let view = Affine::scale(0.25);
    let mut r = Renderer::new();
    let _ = r.render(&d, 12, 12, view, &RenderOptions { image_mips: true, ..Default::default() });
    let px = r.render(&d, 12, 12, view, &RenderOptions { kept_images_only: true, image_mips: true, ..Default::default() }).pixel(6, 6);
    assert!(px[2] >= 250 && px[0] <= 5 && px[1] <= 5, "blue, got {px:?}");
}

/// Drawn as ink amounts (CMYK exports, CMYK documents that show transparency), a colorized image
/// keeps its color's own inks, as a fill of that color does.
#[test]
fn ink_planes_keep_the_colors_own_inks() {
    for (color, want) in [(Color::cmyk(0.0, 0.0, 0.0, 1.0), [0u8, 0, 0, 255]), (Color::cmyk(1.0, 0.0, 0.0, 0.0), [255, 0, 0, 0])] {
        let mut d = Document::new_with_mode(40.0, 20.0, vectorcraft_doc::ColorMode::Cmyk);
        d.images.insert("pic".into(), ImageBlob::new("image/png", gray_png()));
        let im =
            ImageObject { key: "pic".into(), width: 2, height: 1, xf: Affine::scale(20.0), link: None, placement: Default::default(), pixels: None };
        let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
        n.extras_mut().colorize = Some(Box::new(Paint::solid(color)));
        d.insert(Some(d.layers[0].id), 0, n).unwrap();
        let opts = RenderOptions { smooth_images: false, ..Default::default() };
        let inks = Renderer::new().render_region_inks(&d, Rect::new(0.0, 0.0, 40.0, 20.0), 1.0, &opts);
        let at = |x: usize| inks[(10 * 40 + x) * 4..][..4].to_vec();
        assert!(at(10).iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 1), "{color:?}: the black pixel, got {:?}", at(10));
        assert_eq!(at(30), [0, 0, 0, 0], "{color:?}: the white pixel has no ink");
    }
}

#[test]
fn colorize_dims_with_the_grays() {
    let gray = image::GrayImage::from_raw(2, 1, vec![128, 255]).unwrap();
    let mut png = vec![];
    gray.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let d = doc(ImageBlob::new("image/png", png), Some(Paint::solid(Color::rgb(0.0, 0.0, 1.0))));
    let px = Renderer::new().render(&d, 40, 20, Affine::IDENTITY, &RenderOptions { smooth_images: false, ..Default::default() }).pixel(10, 10);
    // Half blue over the green band.
    assert!(px[1].abs_diff(128) <= 3 && px[2].abs_diff(127) <= 3 && px[0] <= 2, "got {px:?}");
}

/// Separations Preview shows a spot-colorized image on its ink's plate only.
#[test]
fn separations_preview_shows_the_image_on_its_spot_plate() {
    let mut d = Document::new_with_mode(40.0, 20.0, vectorcraft_doc::ColorMode::Cmyk);
    let ink = Color::cmyk(0.0, 1.0, 1.0, 0.0);
    d.swatches.push(vectorcraft_color::Swatch { name: "Ink".into(), paint: Paint::solid(ink), global: true, spot: true });
    d.images.insert("pic".into(), ImageBlob::new("image/png", gray_png()));
    let im = ImageObject { key: "pic".into(), width: 2, height: 1, xf: Affine::scale(20.0), link: None, placement: Default::default(), pixels: None };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    n.extras_mut().colorize = Some(Box::new(Paint::Solid { color: ink, swatch: Some("Ink".into()), tint: 1.0 }));
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    let shown = |plates: &[&str]| {
        let proof = proof::ProofSetup { separations: Some(plates.iter().map(|p| p.to_string()).collect()), ..Default::default() };
        let opts = RenderOptions { proof: Some(proof), overprint_preview: true, smooth_images: false, ..Default::default() };
        Renderer::new().render(&d, 40, 20, Affine::IDENTITY, &opts).pixel(10, 10)
    };
    let on = shown(&["Ink"]);
    // One plate shows as black ink.
    assert!(on[..3].iter().all(|v| *v < 128), "the Ink plate shows the image, got {on:?}");
    let off = shown(&["Cyan", "Magenta", "Yellow", "Black"]);
    assert!(off[..3].iter().all(|v| *v >= 250), "the process plates show paper, got {off:?}");
}
