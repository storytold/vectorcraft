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

/// A 60×60 document holding the red 40×40 image of [`doc_with_red_image`] placed by `xf`, linked
/// to a file when `linked`, and with its corners moved to `corners` when given.
fn doc_with_image(linked: bool, xf: Affine, corners: Option<[(f64, f64); 4]>) -> Document {
    let red = doc_with_red_image().images["red"].clone();
    let mut d = Document::new(60.0, 60.0);
    d.images.insert("red".into(), red);
    let link = linked.then(|| vectorcraft_doc::LinkInfo::new("/art/red.png"));
    let im = ImageObject { key: "red".into(), width: 40, height: 40, xf, link, placement: Default::default(), pixels: None };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    if let Some(q) = corners {
        n.set_image_corners(q.map(vectorcraft_geom::Point::from)).unwrap();
    }
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    d
}

/// The alpha of each of `pixels` of `d` drawn 60×60 in outline mode.
fn outline_alphas(d: &Document, pixels: &[(u32, u32)]) -> Vec<u8> {
    let r = Renderer::new().render(d, 60, 60, Affine::IDENTITY, &RenderOptions { outline: true, ..Default::default() });
    pixels.iter().map(|&(x, y)| r.pixel(x, y)[3]).collect()
}

#[test]
fn outline_mode_crosses_a_linked_images_frame_unless_the_setup_shows_images() {
    // (20, 20) is where the diagonals cross, (25, 10) is on neither of them.
    let at = [(20, 20), (25, 10)];
    let mut linked = doc_with_image(true, Affine::IDENTITY, None);
    let [cross, off] = outline_alphas(&linked, &at)[..] else { panic!("two pixels") };
    assert!(cross > 0, "a linked image is crossed from corner to corner");
    assert_eq!(off, 0, "and filled with nothing else");
    let embedded = doc_with_image(false, Affine::IDENTITY, None);
    assert_eq!(outline_alphas(&embedded, &at), vec![0, 0], "an embedded image shows its frame alone");
    linked.setup.outline_images = true;
    let r = Renderer::new().render(&linked, 60, 60, Affine::IDENTITY, &RenderOptions { outline: true, ..Default::default() });
    let [red, green, blue, alpha] = r.pixel(20, 20);
    assert!(alpha == 255 && red == green && green == blue && (50..=58).contains(&red), "the image's gray pixels, not crossed: {:?}", r.pixel(20, 20));
    let preview = Renderer::new().render(&linked, 60, 60, Affine::IDENTITY, &RenderOptions::default());
    assert_eq!(preview.pixel(20, 20), [255, 0, 0, 255], "preview and output draw the pixels uncrossed");
}

#[test]
fn outline_mode_crosses_a_linked_image_with_a_color_mask() {
    use vectorcraft_doc::color_mask::{ColorMask, MaskBackground, MaskMatch};
    let mut d = doc_with_image(true, Affine::IDENTITY, None);
    let red = d.images["red"].clone();
    d.images.insert("red-masked".into(), red);
    let id = d.layers[0].children().unwrap()[0].id;
    let mask = ColorMask {
        key: "red-masked".into(),
        from: "red".into(),
        background: MaskBackground::Colors,
        colors: vec![[255, 255, 255]],
        contiguous: false,
        measure: MaskMatch::Distance,
        tolerance: 10.0,
        softness: 0.0,
    };
    d.node_mut(id).unwrap().extras_mut().color_mask = Some(Box::new(mask));
    let NodeKind::Image(im) = &d.layers[0].children().unwrap()[0].kind else { panic!("an image") };
    assert!(d.shown_image(&d.layers[0].children().unwrap()[0], im).link.is_none(), "the masked copy drawn has no link");
    assert!(outline_alphas(&d, &[(20, 20)])[0] > 0, "the image is crossed as linked");
}

/// The pieces an envelope cuts a linked image into are each a copy of the whole image; Outline mode
/// does not cross them, so the envelope draws as it does for an embedded image.
#[test]
fn outline_mode_does_not_cross_the_pieces_of_an_enveloped_linked_image() {
    let drawn = |linked: bool| {
        let mut d = doc_with_image(linked, Affine::translate((10.0, 10.0)), None);
        let id = d.layers[0].children().unwrap()[0].id;
        let image = d.remove(id).unwrap();
        let kind = vectorcraft_doc::EnvelopeKind::Warp { style: "arc".into(), bend: 30.0, h: 0.0, v: 0.0, horizontal: true, version: 0 };
        let envelope = NodeKind::Envelope {
            content: vec![image],
            kind: Box::new(kind),
            fidelity: 50.0,
            editing: false,
            options: Default::default(),
            frame: Box::new(Affine::IDENTITY),
        };
        let envelope = Node::new(d.alloc_id(), envelope);
        d.insert(Some(d.layers[0].id), 0, envelope).unwrap();
        Renderer::new().render(&d, 60, 60, Affine::IDENTITY, &RenderOptions { outline: true, ..Default::default() }).pixels
    };
    let embedded = drawn(false);
    assert!(embedded.iter().any(|v| *v > 0), "the envelope draws in outline mode");
    assert!(drawn(true) == embedded, "a linked image's pieces are not crossed");
}

/// A linked image moved into a piece of an expanded envelope is not the piece's copy of its image,
/// and is crossed.
#[test]
fn outline_mode_crosses_a_linked_image_moved_into_a_warp_piece() {
    let mut d = doc_with_image(true, Affine::IDENTITY, None);
    let id = d.layers[0].children().unwrap()[0].id;
    let mine = d.remove(id).unwrap();
    let copy = Arc::new(Node::new(d.alloc_id(), mine.kind.clone()));
    let mut clip = Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 60.0, 60.0)), Default::default());
    if let NodeKind::Path { clipping, .. } = &mut clip.kind {
        *clipping = true;
    }
    let NodeKind::Image(im) = &mine.kind else { panic!("an image") };
    let moved = ImageObject { xf: Affine::translate((10.0, 0.0)), ..(**im).clone() };
    let moved = Node::new(mine.id, NodeKind::Image(Box::new(moved)));
    let mut piece = Node::new(d.alloc_id(), NodeKind::Group { children: vec![Arc::new(clip), copy, Arc::new(moved)], clip: true });
    piece.extras_mut().warp_piece = true;
    d.insert(Some(d.layers[0].id), 0, piece).unwrap();
    // The copy's diagonals cross at (20, 20), the moved image's at (30, 20).
    let [copy_cross, moved_cross] = outline_alphas(&d, &[(20, 20), (30, 20)])[..] else { panic!("two pixels") };
    assert_eq!(copy_cross, 0, "the piece's own image is not crossed");
    assert!(moved_cross > 0, "the image moved into it is");
}

#[test]
fn outline_mode_crosses_a_turned_or_distorted_linked_image_through_its_corners() {
    // Turned 0.5 radians about (30, 30): the points three quarters of the way along each diagonal.
    let xf = Affine::translate((30.0, 30.0)) * Affine::rotate(0.5) * Affine::translate((-20.0, -20.0));
    let on = [xf * vectorcraft_geom::Point::new(30.0, 30.0), xf * vectorcraft_geom::Point::new(10.0, 30.0)];
    let at: Vec<(u32, u32)> = on.iter().map(|p| (p.x.floor() as u32, p.y.floor() as u32)).collect();
    assert!(outline_alphas(&doc_with_image(true, xf, None), &at).iter().all(|a| *a > 0), "{at:?} on the turned diagonals");
    assert_eq!(outline_alphas(&doc_with_image(false, xf, None), &at), vec![0, 0], "an embedded image is not crossed");
    // Distorted to (10, 5), (50, 5), (55, 55), (5, 50): (43.75, 42.5) is on one diagonal, (38.75, 16.25)
    // on the other.
    let q = Some([(10.0, 5.0), (50.0, 5.0), (55.0, 55.0), (5.0, 50.0)]);
    let at = [(43, 42), (38, 16)];
    assert!(outline_alphas(&doc_with_image(true, Affine::IDENTITY, q), &at).iter().all(|a| *a > 0), "on the distorted diagonals");
    assert_eq!(outline_alphas(&doc_with_image(false, Affine::IDENTITY, q), &at), vec![0, 0], "a distorted embedded image is not crossed");
}
