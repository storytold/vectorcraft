//! Opacity-mask options survive SVG (M3.87): exported masks list the options that differ from the
//! defaults in `data-vectorcraft-mask`, and import restores them and the original mask art (without
//! the backdrop rectangle and inverting filter that draw the options in SVG).
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, OpacityMask};
use vectorcraft_geom::{Rect, shapes};

fn close_rect(a: Rect, b: Rect, tol: f64) -> bool {
    (a.x0 - b.x0).abs() < tol && (a.y0 - b.y0).abs() < tol && (a.x1 - b.x1).abs() < tol && (a.y1 - b.y1).abs() < tol
}
use vectorcraft_svg::{ExportOptions, export, import_with_report};

/// A red square masked by white art over its left half.
fn masked(clip: bool, invert: bool) -> Document {
    let mut d = Document::new(200.0, 200.0);
    let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let white = Appearance::basic(Paint::solid(Color::WHITE), Paint::None, 0.0);
    let mut n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 90.0, 90.0)), red);
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 90.0)), white);
    let mut m = OpacityMask::new(art, clip);
    m.invert = invert;
    n.mask = Some(Box::new(m));
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn masks(d: &Document) -> Vec<OpacityMask> {
    let mut v = vec![];
    d.walk(|n| v.extend(n.mask.as_deref().cloned()));
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
        // #864: appending `pt` to width/height makes usvg do a px→pt round trip on import, costing
        // ~1e-7 of float error; relax to 1e-4.
        assert!(close_rect(b, Rect::new(10.0, 10.0, 50.0, 90.0), 1e-4), "{b:?}");
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
