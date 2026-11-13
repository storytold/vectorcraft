//! Blend modes and opacity masks in SVG export look as on the canvas: every mode follows the
//! reference formulas when resvg draws the file, and masks take luminance the canvas's way.

use usvg::roxmltree;
use vectorcraft_color::blend::blend_rgb;
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Document, Node, NodeKind, OpacityMask, TextObject};
use vectorcraft_geom::{Point, Rect, shapes};
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
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).expect("parse");
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
    red.extras_mut().mask = Some(Box::new(m));
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

/// A knockout group of opaque art draws its elements plainly: each covers what it knocks out, and
/// masks of their shapes would leave seams along the antialiased edges. Translucent art still knocks
/// out through masks.
#[test]
fn a_knockout_group_of_opaque_art_needs_no_masks() {
    let knockout = |opacity: f32| {
        let mut d = Document::new(20.0, 20.0);
        let below = rect(&mut d, Rect::new(0.0, 0.0, 20.0, 20.0), [200, 0, 0]);
        let mut top = rect(&mut d, Rect::new(5.0, 5.0, 15.0, 15.0), [0, 0, 200]);
        top.opacity = opacity;
        let mut group = Node::group(d.alloc_id(), vec![below.into(), top.into()]);
        group.knockout = vectorcraft_doc::Knockout::On;
        add(&mut d, group);
        export(&d, &ExportOptions::default())
    };
    let opaque = knockout(1.0);
    assert!(!opaque.contains("<mask"), "{opaque}");
    assert!(within(resvg_render(&opaque, 20, 20).over_white(10, 10), [0, 0, 200], 2));
    let translucent = knockout(0.5);
    assert!(translucent.contains("<mask"), "{translucent}");
    // The top half-shows white, not the red it knocks out.
    assert!(
        within(resvg_render(&translucent, 20, 20).over_white(10, 10), [128, 128, 228], 3),
        "{:?}",
        resvg_render(&translucent, 20, 20).over_white(10, 10)
    );
}

/// A brushed stroke is no opaque art: its pieces carry their own opacity (a bristle brush's vary),
/// so the knockout group keeps its masks.
#[test]
fn a_knockout_group_with_a_brushed_stroke_keeps_its_masks() {
    let mut d = Document::new(40.0, 40.0);
    let below = rect(&mut d, Rect::new(0.0, 0.0, 40.0, 40.0), [200, 0, 0]);
    let mut stroke = vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::rgb8(0, 0, 200)), 8.0);
    stroke.brush = Some("Bristle Round".into());
    let line = shapes::line(vectorcraft_geom::Point::new(5.0, 20.0), vectorcraft_geom::Point::new(35.0, 20.0));
    let top = Node::path(d.alloc_id(), line, Appearance { items: vec![vectorcraft_doc::AppearanceItem::Stroke(stroke)], ..Default::default() });
    let mut group = Node::group(d.alloc_id(), vec![below.into(), top.into()]);
    group.knockout = vectorcraft_doc::Knockout::On;
    add(&mut d, group);
    let svg = export(&d, &ExportOptions::default());
    assert!(svg.contains("<mask"), "{svg}");
}

/// The `x`, `y`, `width` and `height` of each `<mask>` and `<filter>` element in `svg`.
fn regions(svg: &str) -> Vec<[f64; 4]> {
    let xml = roxmltree::Document::parse_with_options(svg, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).unwrap();
    xml.descendants()
        .filter(|n| matches!(n.tag_name().name(), "mask" | "filter"))
        .map(|n| ["x", "y", "width", "height"].map(|a| n.attribute(a).and_then(|v| v.parse().ok()).unwrap_or(f64::NAN)))
        .collect()
}

/// Mask and filter regions cover the art they apply to and no more: readers draw a mask into an
/// image of its region's size, and one drawn at 200,000 units square runs out of memory.
#[test]
fn mask_and_filter_regions_cover_their_art() {
    let knockout = {
        let mut d = Document::new(40.0, 20.0);
        let below = rect(&mut d, Rect::new(0.0, 0.0, 40.0, 20.0), [200, 0, 0]);
        let mut top = rect(&mut d, Rect::new(5.0, 5.0, 15.0, 15.0), [0, 0, 200]);
        top.opacity = 0.5;
        let mut group = Node::group(d.alloc_id(), vec![below.into(), top.into()]);
        group.knockout = vectorcraft_doc::Knockout::On;
        add(&mut d, group);
        d
    };
    for (what, d) in [("clip", masked(true, false)), ("no clip, inverted", masked(false, true)), ("knockout", knockout)] {
        let svg = export(&d, &ExportOptions::default());
        let found = regions(&svg);
        assert!(!found.is_empty(), "{what}: {svg}");
        for [x, y, w, h] in found {
            // The art is 40 × 20, with 2 to spare on each side.
            assert!(x >= -2.5 && y >= -2.5 && w <= 45.0 && h <= 25.0 && w >= 40.0 && h >= 20.0, "{what}: {x} {y} {w} {h} in {svg}");
        }
    }
}

/// A mask's region reaches as far as the masked art paints: a drop shadow of the object, or of an
/// object in the masked group, shows past the object as on the canvas.
#[test]
fn a_mask_region_keeps_the_shadows_of_the_masked_art() {
    let shadow = vectorcraft_doc::Effect {
        id: "stylize.dropShadow".into(),
        params: serde_json::json!({"x": 15, "y": 15, "blur": 0, "opacity": 100}),
        visible: true,
    };
    for in_group in [false, true] {
        let mut d = Document::new(60.0, 60.0);
        let mut red = rect(&mut d, Rect::new(10.0, 10.0, 30.0, 30.0), [255, 0, 0]);
        red.appearance.effects.push(shadow.clone());
        let mut masked = if in_group { Node::group(d.alloc_id(), vec![red.into()]) } else { red };
        // An unclipped mask lets the art show outside the mask art: all of the shadow shows.
        let art = rect(&mut d, Rect::new(10.0, 10.0, 20.0, 30.0), [255, 255, 255]);
        masked.extras_mut().mask = Some(Box::new(OpacityMask::new(art, false)));
        add(&mut d, masked);
        let svg = export(&d, &ExportOptions::default());
        let (canvas, written) = (render_artboard(&d), resvg_render(&svg, 60, 60));
        for (x, y) in [(15, 15), (40, 40), (35, 28)] {
            let (c, w) = (canvas.over_white(x, y), written.over_white(x, y));
            assert!(within(c, w, 3), "in a group {in_group}, at {x},{y}: canvas {c:?}, svg {w:?}\n{svg}");
        }
        assert!(canvas.over_white(40, 40)[0] < 100, "the shadow shows on the canvas: {:?}", canvas.over_white(40, 40));
    }
}

/// Type is measured by its layout or its path, and a reader sets live type in its own font, so a
/// mask or knockout over type keeps the region that covers any artwork.
#[test]
fn mask_and_filter_regions_over_type_cover_any_artwork() {
    let style = CharStyle { size: 24.0, fill: Paint::solid(Color::BLACK), ..CharStyle::default() };
    let text = |d: &mut Document| Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(5.0, 30.0), "Wide", style.clone()))));
    let masked = {
        let mut d = Document::new(80.0, 40.0);
        let mut t = text(&mut d);
        let art = rect(&mut d, Rect::new(0.0, 0.0, 40.0, 40.0), [255, 255, 255]);
        t.extras_mut().mask = Some(Box::new(OpacityMask::new(art, false)));
        add(&mut d, t);
        d
    };
    let knockout = {
        let mut d = Document::new(80.0, 40.0);
        let below = rect(&mut d, Rect::new(0.0, 0.0, 80.0, 40.0), [200, 0, 0]);
        let mut top = text(&mut d);
        top.opacity = 0.5;
        let mut group = Node::group(d.alloc_id(), vec![below.into(), top.into()]);
        group.knockout = vectorcraft_doc::Knockout::On;
        add(&mut d, group);
        d
    };
    for (what, d) in [("masked type", masked), ("type in a knockout group", knockout)] {
        let svg = export(&d, &ExportOptions::default());
        let found = regions(&svg);
        assert!(!found.is_empty(), "{what}: {svg}");
        for [x, y, w, h] in found {
            assert!(x <= -100000.0 && y <= -100000.0 && w >= 200000.0 && h >= 200000.0, "{what}: {x} {y} {w} {h} in {svg}");
        }
    }
}

/// A live object under a mask is measured as it is written, expanded: the mask's region covers
/// the drop shadows of a blend's steps.
#[test]
fn a_mask_region_covers_a_blend_as_written() {
    let shadow = vectorcraft_doc::Effect {
        id: "stylize.dropShadow".into(),
        params: serde_json::json!({"x": 15, "y": 15, "blur": 0, "opacity": 100}),
        visible: true,
    };
    let mut d = Document::new(100.0, 50.0);
    let mut a = rect(&mut d, Rect::new(10.0, 10.0, 20.0, 20.0), [255, 0, 0]);
    let mut b = rect(&mut d, Rect::new(70.0, 10.0, 80.0, 20.0), [0, 0, 255]);
    a.appearance.effects.push(shadow.clone());
    b.appearance.effects.push(shadow);
    let mut blend = Node::new(d.alloc_id(), NodeKind::Blend { children: vec![a.into(), b.into()], spec: Default::default() });
    let art = rect(&mut d, Rect::new(0.0, 0.0, 5.0, 5.0), [255, 255, 255]);
    blend.extras_mut().mask = Some(Box::new(OpacityMask::new(art, false)));
    add(&mut d, blend);
    let svg = export(&d, &ExportOptions::default());
    // The steps' drop shadows are filters with regions of their own; the mask's covers them all:
    // the steps run from 10 to 80 across, and their shadows 15 further.
    let xml = roxmltree::Document::parse_with_options(&svg, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).unwrap();
    let mask = xml.descendants().find(|n| n.has_tag_name("mask")).unwrap();
    let [x, y, w, h] = ["x", "y", "width", "height"].map(|a| mask.attribute(a).and_then(|v| v.parse::<f64>().ok()).unwrap_or(f64::NAN));
    assert!(x <= 0.0 && y <= 0.0 && x + w >= 95.0 && y + h >= 35.0 && w < 200.0 && h < 200.0, "{x} {y} {w} {h} in {svg}");
}
