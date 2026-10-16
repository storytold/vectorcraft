//! Opacity-mask options survive SVG (M3.87): exported masks list the options that differ from the
//! defaults in `data-vectorcraft-mask`, and import restores them and the original mask art (without
//! the backdrop rectangle and inverting filter that draw the options in SVG).
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, OpacityMask};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_svg::{ExportOptions, export, import_with_report};
use vectorcraft_testkit::raster::{Image, render_artboard};

/// A red square masked by white art over its left half.
fn masked(clip: bool, invert: bool) -> Document {
    let mut d = Document::new(200.0, 200.0);
    let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let white = Appearance::basic(Paint::solid(Color::WHITE), Paint::None, 0.0);
    let mut n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 90.0)), red);
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 90.0)), white);
    let mut m = OpacityMask::new(art, clip);
    m.invert = invert;
    n.extras_mut().mask = Some(Box::new(m));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn masks(d: &Document) -> Vec<OpacityMask> {
    let mut v = vec![];
    d.walk(|n| v.extend(n.extras().mask.as_deref().cloned()));
    v
}

#[test]
fn an_inverted_unclipped_mask_round_trips() {
    for (clip, invert, attr) in
        [(false, true, Some("noclip invert")), (true, true, Some("invert")), (false, false, Some("noclip")), (true, false, None)]
    {
        let s = export(&masked(clip, invert), &ExportOptions::default());
        match attr {
            Some(a) => assert!(s.contains(&format!("data-vectorcraft-mask=\"{a}\"")), "{s}"),
            None => assert!(!s.contains("data-vectorcraft-mask"), "{s}"),
        }
        let (back, warnings) = import_with_report(&s).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let [m] = &masks(&back)[..] else { panic!("one masked object: {s}") };
        assert_eq!((m.clip, m.invert), (clip, invert), "{s}");
        // The original art: one white rectangle, no backdrop and no filter group around it.
        assert!(m.art.path_data().is_some(), "{:?}", m.art.kind);
        let b = m.art.geometric_bounds().unwrap();
        assert!((b.x0 - 10.0).abs() < 1e-6 && (b.x1 - 50.0).abs() < 1e-6 && (b.y0 - 10.0).abs() < 1e-6 && (b.y1 - 90.0).abs() < 1e-6, "{b:?}");
        assert_eq!(m.art.appearance.fill_paint(), Paint::solid(Color::WHITE));
        // Exporting the import again writes the same mask.
        let again = export(&back, &ExportOptions::default());
        let mask_def = |s: &str| s.lines().skip_while(|l| !l.contains("<mask id=")).take_while(|l| !l.contains("</mask>")).count();
        assert_eq!(mask_def(&again), mask_def(&s), "{again}\n{s}");
    }
}

#[test]
fn foreign_masks_keep_their_art() {
    // Without the attribute a white backdrop is part of the mask art (another app's file).
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
      <mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="100" height="100"><rect width="100" height="100" fill="white"/><rect width="50" height="100" fill="black"/></mask>
      <g mask="url(#m)"><rect width="100" height="100" fill="red"/></g></svg>"#;
    let (d, _) = import_with_report(svg).unwrap();
    let [m] = &masks(&d)[..] else { panic!("one mask") };
    assert!(m.clip && !m.invert);
    assert_eq!(m.art.children().map(|c| c.len()), Some(2));
}

/// `svg` drawn by resvg, `w`×`h` on white.
fn resvg_render(svg: &str, w: u32, h: u32) -> Image {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options::default()).expect("parse");
    let mut pm = resvg::tiny_skia::Pixmap::new(w, h).unwrap();
    pm.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pm.as_mut());
    Image { width: w, height: h, rgba: pm.data().to_vec() }
}

#[test]
fn a_mask_whose_art_is_all_hidden_hides_its_object() {
    // Hidden mask art draws nothing: the mask is black all over and the red square doesn't show,
    // as resvg draws the file. The square keeps its (empty) mask, so it can still be edited.
    for art in [
        r#"<rect width="100" height="100" fill="white" visibility="hidden"/>"#,
        r#"<g visibility="hidden"><rect width="100" height="100" fill="white"/></g>"#,
    ] {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
              <mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="100" height="100">{art}</mask>
              <g mask="url(#m)"><rect width="100" height="100" fill="red"/></g></svg>"#
        );
        let (d, _) = import_with_report(&svg).unwrap();
        let [m] = &masks(&d)[..] else { panic!("the square keeps its mask: {svg}") };
        assert!(m.clip && !m.invert);
        assert!(m.art.geometric_bounds().is_none(), "no visible art: {:?}", m.art.kind);
        let (canvas, reference) = (render_artboard(&d), resvg_render(&svg, 100, 100));
        for (x, y) in [(10, 10), (50, 50), (90, 90)] {
            assert_eq!(reference.over_white(x, y), [255, 255, 255], "resvg ({x},{y}): {svg}");
            assert_eq!(canvas.over_white(x, y), [255, 255, 255], "canvas ({x},{y}): {svg}");
        }
    }
}

#[test]
fn an_alpha_mask_uses_its_art_alpha_not_its_luminance() {
    // `mask-type: alpha` (CSS Masking §7.2): an opaque dark red shows the square fully, black at
    // half opacity shows it half, whatever their luminance. As a property and as an attribute.
    for mask_type in [r#"style="mask-type:alpha""#, r#"mask-type="alpha""#] {
        let svg = format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
              <mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="100" height="100" {mask_type}>
                <rect width="50" height="100" fill="#b43333"/><rect x="50" width="50" height="100" fill="black" fill-opacity="0.5"/></mask>
              <rect width="100" height="100" fill="#1a73e8" mask="url(#m)"/></svg>"##
        );
        let (d, warnings) = import_with_report(&svg).unwrap();
        let [m] = &masks(&d)[..] else { panic!("one mask: {svg}") };
        assert!(m.clip && !m.invert);
        let (canvas, reference) = (render_artboard(&d), resvg_render(&svg, 100, 100));
        for (x, y, want) in [(25, 50, [26, 115, 232]), (75, 50, [141, 185, 244])] {
            let (got, r) = (canvas.over_white(x, y), reference.over_white(x, y));
            let near = |a: [u8; 3]| a.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 3);
            assert!(near(r), "resvg ({x},{y}) {r:?}: {svg}");
            assert!(near(got), "canvas ({x},{y}) {got:?}, want {want:?}: {svg}");
        }
        assert!(warnings.is_empty(), "{warnings:?}");
    }
}
