//! The pieces a raster mesh warp cuts an image into (envelopes, Effect › Warp) overlap their
//! neighbors: a partly transparent image keeps its own alpha where they do.

use crate::*;
use vectorcraft_doc::live::mesh_warp_image;
use vectorcraft_doc::{ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::Point;

/// A 4 × 4 px red PNG, every pixel `alpha` opaque.
fn png(alpha: u8) -> Vec<u8> {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, alpha]));
    let mut out = vec![];
    {
        let __dyn: image::DynamicImage = img.into();
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
            .map(|__b| out.extend_from_slice(&__b))
    }
    .unwrap();
    out
}

/// The image of [`png`] over (10, 10)–(90, 90), pushed through `f` as 4 × 4 cells (with `masked`,
/// as Preserve Shape Using Transparency cuts it), on a page of 100 × 100 pt, after `tweak`
/// adjusts the warped group.
fn warped(f: &dyn Fn(Point) -> Point, masked: bool, pixelated: bool, tweak: impl FnOnce(&mut Node)) -> Document {
    let mut d = Document::new(100.0, 100.0);
    d.images.insert("pic".into(), ImageBlob::png(png(128)));
    let im = ImageObject {
        key: "pic".into(),
        width: 4,
        height: 4,
        xf: Affine::translate((10.0, 10.0)) * Affine::scale(20.0),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im.clone())));
    let mut g = mesh_warp_image(&n, &im, f, 4, pixelated, masked);
    tweak(&mut g);
    d.insert(Some(d.layers[0].id), 0, g).unwrap();
    d
}

/// `d` at 4 px a point over transparency.
fn drawn(d: &Document) -> Rendered {
    Renderer::new().render(d, 400, 400, Affine::scale(4.0), &RenderOptions { background: None, ..Default::default() })
}

/// The pixels of `r` inside (x0, y0)–(x1, y1) (points) whose alpha is more than 4 off `want`, with
/// that alpha: the first few.
fn uneven(r: &Rendered, (x0, y0, x1, y1): (u32, u32, u32, u32), want: u8) -> Vec<(u32, u32, u8)> {
    let mut out = vec![];
    for y in y0 * 4..y1 * 4 {
        for x in x0 * 4..x1 * 4 {
            let a = r.pixel(x, y)[3];
            if (i32::from(a) - i32::from(want)).abs() > 4 && out.len() < 8 {
                out.push((x, y, a));
            }
        }
    }
    out
}

/// Inside the image, away from its outer edges.
const INSIDE: (u32, u32, u32, u32) = (12, 12, 88, 88);

#[test]
fn a_half_transparent_image_is_half_transparent_where_its_warp_pieces_overlap() {
    let same = |p: Point| p;
    for masked in [false, true] {
        for pixelated in [false, true] {
            let r = drawn(&warped(&same, masked, pixelated, |_| {}));
            // The cells meet at x and y = 30, 50 and 70, and each cell's diagonal crosses it.
            assert_eq!(uneven(&r, INSIDE, 128), vec![], "masked {masked}, pixelated {pixelated}");
        }
    }
    // A wave bends every edge between the pieces.
    let wave = |p: Point| Point::new(p.x + 3.0 * (p.y / 7.0).sin(), p.y + 2.0 * (p.x / 9.0).cos());
    for masked in [false, true] {
        let r = drawn(&warped(&wave, masked, false, |_| {}));
        assert_eq!(uneven(&r, (16, 16, 84, 84), 128), vec![], "waved, masked {masked}");
    }
}

#[test]
fn the_images_opacity_applies_once_over_the_warp_pieces() {
    let same = |p: Point| p;
    for masked in [false, true] {
        let r = drawn(&warped(&same, masked, false, |g| g.opacity = 0.5));
        assert_eq!(uneven(&r, INSIDE, 64), vec![], "masked {masked}");
    }
}

/// A clip group of a half transparent image made by hand, in a knockout group, knocks out by
/// where its image is opaque: only the pieces of a warp knock out by their whole triangle.
#[test]
fn other_clip_groups_in_a_knockout_group_knock_out_by_their_alpha() {
    let same = |p: Point| p;
    let d = warped(&same, false, false, |g| {
        let NodeKind::Group { children, .. } = &mut g.kind else { panic!("a group") };
        for c in children {
            Arc::make_mut(c).extras_mut().warp_piece = false;
        }
    });
    let r = drawn(&d);
    // On the shared edge at x = 30, y = 20: the neighbors' alphas add up there.
    let a = r.pixel(120, 80)[3];
    assert!(a > 160, "{a}");
}

/// Show Images in Outline Mode paints a half transparent distorted image's pixels once, where the
/// pieces of its mesh overlap too: inside the image, its alpha is the preview's.
#[test]
fn outline_mode_paints_a_distorted_images_pixels_once_where_its_pieces_overlap() {
    let mut d = Document::new(100.0, 100.0);
    d.images.insert("pic".into(), ImageBlob::png(png(128)));
    d.setup.outline_images = true;
    let im = ImageObject {
        key: "pic".into(),
        width: 4,
        height: 4,
        xf: Affine::translate((10.0, 10.0)) * Affine::scale(20.0),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im)));
    n.set_image_corners([(20.0, 10.0), (80.0, 10.0), (90.0, 90.0), (10.0, 90.0)].map(|(x, y)| Point::new(x, y))).unwrap();
    let pieces = vectorcraft_doc::image_quad::distorted_pieces(&vectorcraft_doc::image_quad::distorted_art(&n).unwrap()).len();
    assert!(pieces > 2, "{pieces}");
    d.insert(Some(d.layers[0].id), 0, n).unwrap();
    let r = Renderer::new().render(&d, 400, 400, Affine::scale(4.0), &RenderOptions { outline: true, background: None, ..Default::default() });
    let preview = drawn(&d);
    let a = preview.pixel(200, 200)[3];
    assert!(a.abs_diff(128) <= 4, "the image's own alpha inside it: {a}");
    let mut off = vec![];
    for y in 80..320 {
        for x in 100..300 {
            let (a, b) = (r.pixel(x, y)[3], preview.pixel(x, y)[3]);
            if a.abs_diff(b) > 2 && off.len() < 8 {
                off.push((x, y, a, b));
            }
        }
    }
    assert_eq!(off, vec![], "outline alpha against preview alpha");
}

/// The canvas cuts a distorted image's mesh for the view's scale: more pieces zoomed in, fewer
/// zoomed out, than the mesh files and print take.
#[test]
fn the_canvas_cuts_a_distorted_images_mesh_for_the_zoom() {
    let im =
        ImageObject { key: "pic".into(), width: 400, height: 400, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
    let mut n = Node::new(vectorcraft_doc::NodeId(1), NodeKind::Image(Box::new(im)));
    n.set_image_corners([(40.0, 0.0), (360.0, 0.0), (400.0, 400.0), (0.0, 400.0)].map(|(x, y)| Point::new(x, y))).unwrap();
    let page = vectorcraft_doc::image_quad::distorted_pieces(&vectorcraft_doc::image_quad::distorted_art(&n).unwrap()).len();
    let n = std::sync::Arc::new(n);
    let mut r = Renderer::new();
    let mut pieces = |scale: f64| vectorcraft_doc::image_quad::distorted_pieces(&r.distorted_art(&n, Affine::scale(scale), false).unwrap()).len();
    let (small, large) = (pieces(1.0 / 25.0), pieces(8.0));
    assert!(small < page && page < large, "{small} {page} {large}");
    // A view that stretches one way and squeezes the other as much keeps its area, and still
    // takes a finer mesh than an unscaled one.
    let (plain, stretched) = (pieces(1.0), r_pieces(&n, Affine::scale_non_uniform(16.0, 1.0 / 16.0)));
    assert!(stretched > plain, "{stretched} {plain}");
}

fn r_pieces(n: &std::sync::Arc<Node>, view: Affine) -> usize {
    vectorcraft_doc::image_quad::distorted_pieces(&Renderer::new().distorted_art(n, view, false).unwrap()).len()
}
