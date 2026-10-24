//! Artboard background colors export with a single artboard's SVG.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use vectorcraft_color::Color;
use vectorcraft_doc::{Document, Node};
use vectorcraft_geom::{Rect, shapes};
use vectorcraft_svg::{ExportOptions, export};

#[test]
fn single_artboard_svg_includes_the_artboard_background_rectangle() {
    let mut d = Document::new(200.0, 150.0);
    d.artboards[0].color = Some(Color::from_hex("#ffeedd").unwrap());
    let path = shapes::rectangle(Rect::new(10.0, 10.0, 50.0, 40.0));
    let n = Node::path(d.alloc_id(), path, Default::default());
    d.insert(Some(d.layers[0].id), usize::MAX, n).unwrap();
    let svg = export(&d, &ExportOptions::default());
    assert!(svg.contains("width=\"200\" height=\"150\" fill=\"#ffeedd\""), "{svg}");
}

#[test]
fn artboard_background_is_omitted_when_exporting_all_art_bounds() {
    let mut d = Document::new(200.0, 150.0);
    d.artboards[0].color = Some(Color::from_hex("#ffeedd").unwrap());
    let svg = export(&d, &ExportOptions { artboard: None, ..ExportOptions::default() });
    assert!(!svg.contains("fill=\"#ffeedd\""), "{svg}");
}
