//! SVG export/import properties: never panics, always re-imports, and plain art survives the round
//! trip visually (render comparison with a perceptual tolerance).
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use proptest::prelude::*;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::Appearance;
use vectorcraft_geom::{PathData, Rect, shapes};

fn close_rect(a: Rect, b: Rect, tol: f64) -> bool {
    (a.x0 - b.x0).abs() < tol && (a.y0 - b.y0).abs() < tol && (a.x1 - b.x1).abs() < tol && (a.y1 - b.y1).abs() < tol
}
use vectorcraft_svg::{ExportOptions, ObjectIds, Styling, export, import};
use vectorcraft_testkit::fixtures::{self, DocBuilder, art_nodes};
use vectorcraft_testkit::invariants::check_document;
use vectorcraft_testkit::raster::{assert_similar, render_artboard};
use vectorcraft_testkit::strategies::{arb_closed_shape, arb_ops, arb_path_data};

fn all_options() -> Vec<ExportOptions> {
    let mut v = vec![];
    for styling in [Styling::PresentationAttributes, Styling::InlineStyle, Styling::StyleEntities, Styling::InternalCss] {
        for (minify, decimals, object_ids, responsive) in
            [(false, 3, ObjectIds::LayerNames, false), (true, 1, ObjectIds::Minimal, true), (false, 7, ObjectIds::Unique, true)]
        {
            v.push(ExportOptions { artboard: Some(0), styling, decimals, object_ids, minify, responsive, ..Default::default() });
        }
    }
    v.push(ExportOptions { artboard: None, ..Default::default() });
    v
}

fn arb_color() -> impl Strategy<Value = Color> {
    (0u8..=255, 0u8..=255, 0u8..=255).prop_map(|(r, g, b)| Color::rgb8(r, g, b))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 48, failure_persistence: None, ..ProptestConfig::default() })]

    /// Documents from random editing export (all option combos) and re-import to valid documents.
    #[test]
    fn random_documents_export_and_reimport(ops in arb_ops(5..40)) {
        let mut s = fixtures::session();
        for op in &ops {
            let _ = op.apply(&mut s);
        }
        let d = s.doc().unwrap().doc.clone();
        for o in all_options() {
            let svg = export(&d, &o);
            let back = import(&svg).map_err(|e| TestCaseError::fail(format!("{o:?}: {e}\n{svg}")))?;
            check_document(&back).map_err(TestCaseError::fail)?;
        }
    }

    /// Solid-filled shapes look the same after export → import.
    #[test]
    fn plain_art_survives_visually(shapes_ in prop::collection::vec((arb_closed_shape(), arb_color(), 0.2f32..1.0), 1..6)) {
        let mut b = DocBuilder::new(200.0, 200.0);
        for (p, c, o) in &shapes_ {
            b.path(p.clone(), Appearance::basic(Paint::solid(*c), Paint::None, 0.0), |n| n.opacity = *o);
        }
        let d = b.build();
        let back = import(&export(&d, &ExportOptions::default())).unwrap();
        assert_similar(&render_artboard(&d), &render_artboard(&back), 6.0, 0.002);
    }

    /// Path geometry survives within the export precision.
    #[test]
    fn path_bounds_survive(p in arb_path_data(), decimals in 2u8..6) {
        let mut b = DocBuilder::new(200.0, 200.0);
        b.path(p.clone(), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0), |_| {});
        let d = b.build();
        let back = import(&export(&d, &ExportOptions { decimals, ..Default::default() })).unwrap();
        let nodes = art_nodes(&back);
        let paths: Vec<&PathData> = nodes.iter().filter_map(|n| n.path_data()).collect();
        prop_assert!(!paths.is_empty(), "no path back");
        // Multi-subpath paths may come back split into several objects; compare the union.
        let bb = paths.iter().filter_map(|q| q.bounds()).reduce(|x, y| x.union(y)).unwrap();
        let a = p.bounds().unwrap();
        let eps = 4.0 * 10f64.powi(-(decimals as i32));
        prop_assert!(close_rect(a, bb, eps), "{a:?} vs {bb:?}");
    }

    /// Importing arbitrary text never panics.
    #[test]
    fn import_garbage_never_panics(s in ".{0,200}") {
        let _ = import(&s);
        let _ = import(&format!("<svg xmlns=\"http://www.w3.org/2000/svg\">{s}</svg>"));
    }

    /// Importing random path data never panics and gives a valid document.
    #[test]
    fn import_random_path_data(d in "[MLCQZHVAmlcqzhva0-9 .,eE+-]{0,80}") {
        let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\"><path d=\"{d}\"/></svg>");
        if let Ok(doc) = import(&svg) {
            check_document(&doc).map_err(TestCaseError::fail)?;
        }
    }
}

#[test]
fn rich_fixture_exports_with_all_options() {
    let s = fixtures::rich_session();
    let d = s.doc().unwrap().doc.clone();
    for o in all_options() {
        let svg = export(&d, &o);
        let back = import(&svg).unwrap_or_else(|e| panic!("{o:?}: {e}"));
        check_document(&back).unwrap();
        if o.minify {
            assert!(!svg.contains("\n  "), "minified output is indented");
        }
    }
}

#[test]
fn rect_survives_visually_with_stroke() {
    let mut b = DocBuilder::new(100.0, 100.0);
    b.path(
        shapes::rectangle(Rect::new(20.0, 20.0, 80.0, 70.0)),
        Appearance::basic(Paint::solid(Color::rgb8(30, 120, 200)), Paint::solid(Color::BLACK), 3.0),
        |_| {},
    );
    let d = b.build();
    let back = import(&export(&d, &ExportOptions::default())).unwrap();
    assert_similar(&render_artboard(&d), &render_artboard(&back), 4.0, 0.001);
}
