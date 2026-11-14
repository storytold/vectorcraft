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
    img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
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
