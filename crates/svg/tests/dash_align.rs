//! Dashes fitted to corners export as the canvas's filled outlines (a dash array can't fit them);
//! exact dashes stay a `stroke-dasharray`. Checked by rendering the exported SVG with resvg.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Dash, Document, Node, StrokeLayer};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_svg::{ExportOptions, export};
use vectorcraft_testkit::raster::{Image, assert_similar, render_artboard};

fn rect_doc(align_corners: bool) -> Document {
    let mut d = Document::new(140.0, 90.0);
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), 4.0);
    st.dash = Some(Dash { pattern: vec![12.0, 6.0], offset: 0.0, align_corners });
    let n = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(20.0, 20.0, 120.0, 70.0)),
        Appearance { items: vec![AppearanceItem::Stroke(st)], ..Default::default() },
    );
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

/// The SVG rendered by resvg on white at 1 px/pt. usvg defaults to 96 dpi (1 pt = 4/3 px), which
/// would scale a pt-typed SVG to 75% of its declared size; pin dpi to 72 so pt and px match.
fn resvg_render(svg: &str, w: u32, h: u32) -> Image {
    let tree = resvg::usvg::Tree::from_str(svg, &resvg::usvg::Options { dpi: 72.0, ..Default::default() }).expect("parse");
    let mut pm = resvg::tiny_skia::Pixmap::new(w, h).unwrap();
    pm.fill(resvg::tiny_skia::Color::WHITE);
    resvg::render(&tree, resvg::tiny_skia::Transform::identity(), &mut pm.as_mut());
    Image { width: w, height: h, rgba: pm.data().to_vec() }
}

#[test]
fn fitted_dashes_export_as_outlines_that_render_like_the_canvas() {
    let d = rect_doc(true);
    let svg = export(&d, &ExportOptions::default());
    assert!(!svg.contains("stroke-dasharray") && !svg.contains("stroke="), "{svg}");
    assert_similar(&render_artboard(&d), &resvg_render(&svg, 140, 90), 24.0, 0.002);
    // Exact dashes are still a dash array.
    assert!(export(&rect_doc(false), &ExportOptions::default()).contains("stroke-dasharray=\"12 6\""));
}
