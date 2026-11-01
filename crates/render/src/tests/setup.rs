//! Document Setup options the renderer honours.

use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};

use crate::*;

fn doc_with_red_image() -> Document {
    let mut d = Document::new(40.0, 40.0);
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    d.images.insert("red".into(), ImageBlob::new("image/png", png));
    let (id, layer) = (d.alloc_id(), d.layers[0].id);
    let im = ImageObject { key: "red".into(), width: 40, height: 40, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
    d.insert(Some(layer), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
    d
}

#[test]
fn outline_mode_draws_images_in_grey_only_when_the_setup_asks() {
    let mut d = doc_with_red_image();
    let outline = RenderOptions { outline: true, ..Default::default() };
    let centre = |d: &Document| Renderer::new().render(d, 40, 40, Affine::IDENTITY, &outline).pixel(20, 20);
    assert_eq!(centre(&d)[3], 0, "outline mode draws only the image frame");
    d.setup.outline_images = true;
    let [r, g, b, a] = centre(&d);
    // Red's luma (0.2126 × 255 ≈ 54) in every channel.
    assert_eq!(a, 255);
    assert!(r == g && g == b && (50..=58).contains(&r), "{:?}", [r, g, b]);
    // Preview mode keeps the colours.
    assert_eq!(Renderer::new().render(&d, 40, 40, Affine::IDENTITY, &RenderOptions::default()).pixel(20, 20), [255, 0, 0, 255]);
}
