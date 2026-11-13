//! The pieces a raster mesh warp cuts an image into overlap their neighbors: in SVG a partly
//! transparent image keeps its own alpha where they do, and the pieces share one copy of the image.

use vectorcraft_doc::live::mesh_warp_image;
use vectorcraft_doc::{Document, ImageBlob, ImageObject, Node, NodeKind};
use vectorcraft_geom::{Affine, Point};
use vectorcraft_svg::{ExportOptions, export_full};

/// A 100 × 100 pt page holding a 4 × 4 px red image at half alpha over (10, 10)–(90, 90), pushed
/// through `f` as 4 × 4 cells (with `masked`, as Preserve Shape Using Transparency cuts it).
fn warped(f: &dyn Fn(Point) -> Point, masked: bool) -> Document {
    warped_pixels(f, masked, [255, 0, 0, 128], |_| {})
}

/// [`warped`] with every pixel `rgba`, after `tweak` adjusts the image object.
fn warped_pixels(f: &dyn Fn(Point) -> Point, masked: bool, rgba: [u8; 4], tweak: impl FnOnce(&mut Node)) -> Document {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba(rgba));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut d = Document::new(100.0, 100.0);
    d.images.insert("pic".into(), ImageBlob::png(png));
    let im = ImageObject {
        key: "pic".into(),
        width: 4,
        height: 4,
        xf: Affine::translate((10.0, 10.0)) * Affine::scale(20.0),
        link: None,
        placement: Default::default(),
        pixels: None,
    };
    let mut n = Node::new(d.alloc_id(), NodeKind::Image(Box::new(im.clone())));
    tweak(&mut n);
    let g = mesh_warp_image(&n, &im, f, 4, false, masked);
    d.insert(d.default_layer(), 0, g).unwrap();
    d
}

/// `svg` drawn at 4 px a point over transparency: of the pixels inside (12, 12)–(88, 88), the most
/// and the least opaque, each with its position.
fn alpha_range(svg: &str) -> ((u32, u32, u8), (u32, u32, u8)) {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).unwrap();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(400, 400).unwrap();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(4.0, 4.0), &mut pixmap.as_mut());
    let (mut most, mut least) = ((0, 0, 0u8), (0, 0, 255u8));
    for y in 48..352 {
        for x in 48..352 {
            let a = pixmap.pixel(x, y).unwrap().alpha();
            if a > most.2 {
                most = (x, y, a);
            }
            if a < least.2 {
                least = (x, y, a);
            }
        }
    }
    (most, least)
}

#[test]
fn a_half_transparent_image_is_half_transparent_where_its_warp_pieces_overlap() {
    let same = |p: Point| p;
    for masked in [false, true] {
        let svg = export_full(&warped(&same, masked), &ExportOptions::default(), None).svg;
        // Where pieces overlap, the image's alpha does not add up (it reached 75 % there). The
        // viewer rasterizes each piece's triangle and the masks over it one by one, so pixels on a
        // triangle's antialiased edge come out a little lighter.
        let (most, least) = alpha_range(&svg);
        assert!(most.2 <= 134, "masked {masked}: {most:?}");
        assert!(least.2 >= 104, "masked {masked}: {least:?}");
        // The 32 pieces draw one copy of the image.
        assert_eq!(svg.matches("data:image/png;base64,").count(), 1, "masked {masked}");
    }
}

/// Pieces of an opaque image are drawn one over another without knockout masks; a colorize paint
/// draws a gray image's light pixels partly transparent, so its pieces take them.
#[test]
fn only_pieces_that_show_through_take_knockout_masks() {
    let same = |p: Point| p;
    let knockouts = |d: &Document| export_full(d, &ExportOptions::default(), None).svg.matches("id=\"knockout").count();
    assert_eq!(knockouts(&warped_pixels(&same, false, [128, 128, 128, 255], |_| {})), 0);
    let colorized = warped_pixels(&same, false, [128, 128, 128, 255], |n| {
        n.extras_mut().colorize = Some(Box::new(vectorcraft_doc::color::Paint::solid(vectorcraft_doc::color::Color::rgb(1.0, 0.0, 0.0))));
    });
    assert!(knockouts(&colorized) > 0);
}
