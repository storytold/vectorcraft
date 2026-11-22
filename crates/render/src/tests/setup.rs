//! Document Setup options the renderer honours.

use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};

use crate::*;

fn doc_with_red_image() -> Document {
    let mut d = Document::new(40.0, 40.0);
    let mut png = Vec::new();
    {
        let __dyn: image::DynamicImage = (image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]))).into();
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| png.extend_from_slice(&__b))
    }
    .unwrap();
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
