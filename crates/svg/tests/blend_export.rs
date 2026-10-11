//! Blend modes and opacity masks in SVG export look as on the canvas: every mode follows the
//! reference formulas when resvg draws the file, and masks take luminance the canvas's way.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_color::blend::blend_rgb;
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, OpacityMask};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_svg::{ExportOptions, export};
use vectorcraft_testkit::raster::{Image, render_artboard};

fn rect(d: &mut Document, r: Rect, rgb: [u8; 3]) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(Paint::solid(Color::rgb8(rgb[0], rgb[1], rgb[2])), Paint::None, 0.0))
}

fn add(d: &mut Document, n: Node) {
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, n).unwrap();
}

/// The exported SVG rendered by resvg on white at 1 px/pt.
fn resvg_render(svg: &str, w: u32, h: u32) -> Image {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options { dpi: 72.0, ..Default::default() }).expect("parse");
    let mut pm = resvg::tiny_skia::Pixmap::new(w, h).unwrap();
    pm.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pm.as_mut());
    Image { width: w, height: h, rgba: pm.data().to_vec() }
}

fn within(a: [u8; 3], b: [u8; 3], tol: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}

#[test]
fn every_blend_mode_follows_the_reference_formulas() {
    // (backdrop, source, whether a non-separable result leaves the gamut). resvg's rasteriser
    // departs from the reference formulas when Hue, Saturation, Color or Luminosity have to clip
    // their result back into the gamut, so those cases are checked on the canvas only
    // (vectorcraft-render's blend tests).
    let pairs: [([u8; 3], [u8; 3], bool); 4] = [
        ([51, 128, 230], [153, 64, 128], false),
        ([230, 26, 102], [77, 204, 179], true),
        ([200, 180, 40], [20, 60, 250], true),
        ([10, 90, 160], [128, 128, 128], false),
    ];
    for mode in BlendMode::ALL {
        for (b, s, clips) in pairs {
            if clips && !mode.is_separable() {
                continue;
            }
            let mut d = Document::new(20.0, 20.0);
            let below = rect(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0), b);
            let mut top = rect(&mut d, Rect::new(5.0, 5.0, 15.0, 15.0), s);
            top.blend = mode;
            add(&mut d, below);
            add(&mut d, top);
            let svg = export(&d, &ExportOptions::default());
            let px = resvg_render(&svg, 20, 20).over_white(10, 10);
            let f = |c: [u8; 3]| c.map(|v| v as f32 / 255.0);
            let want = blend_rgb(mode, f(b), f(s)).map(|v| (v * 255.0).round() as u8);
            assert!(within(px, want, 1), "{mode:?}, {b:?} under {s:?}: {px:?}, want {want:?}");
        }
    }
}

/// A red square under a mask of green art (luminance 0.7152 with Rec. 709 weights, 0.59 with the
/// blend-mode weights) that covers its left half.
fn masked(clip: bool, invert: bool) -> Document {
    let mut d = Document::new(40.0, 20.0);
    let art = rect(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0), [0, 255, 0]);
    let mut red = rect(&mut d, Rect::new(0.0, 0.0, 40.0, 20.0), [255, 0, 0]);
    let mut m = OpacityMask::new(art, clip);
    m.invert = invert;
    red.mask = Some(Box::new(m));
    add(&mut d, red);
    d
}

#[test]
fn clip_and_invert_masks_match_the_canvas() {
    for (clip, invert) in [(true, false), (false, false), (true, true), (false, true)] {
        let d = masked(clip, invert);
        let svg = export(&d, &ExportOptions::default());
        assert!(svg.contains("color-interpolation=\"sRGB\""), "{svg}");
        let canvas = render_artboard(&d);
        let written = resvg_render(&svg, 40, 20);
        for x in [10, 30] {
            let (c, w) = (canvas.over_white(x, 10), written.over_white(x, 10));
            assert!(within(c, w, 2), "clip {clip}, invert {invert}, x {x}: canvas {c:?}, svg {w:?}");
        }
        // Over the green art the red shows at the art's luminance (inverted: its complement).
        let lum: f32 = if invert { 1.0 - 0.7152 } else { 0.7152 };
        let g = (255.0 * (1.0 - lum)).round() as u8;
        assert!(within(canvas.over_white(10, 10), [255, g, g], 2), "{:?}", canvas.over_white(10, 10));
    }
}
