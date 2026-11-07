//! Raster effects in PDF export: not written yet, so a fill's, a stroke's or the object's own
//! raster effect is reported.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Node, NodeId};
use vectorcraft_geom::{Rect, shapes};

use crate::*;

fn warnings(item: Option<usize>) -> Vec<String> {
    let mut d = Document::new(100.0, 100.0);
    let mut n = Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(20.0, 20.0, 80.0, 80.0)),
        Appearance::basic(Paint::solid(Color::WHITE), Paint::solid(Color::BLACK), 2.0),
    );
    n.appearance.effects_mut(item).unwrap().push(Effect { id: "stylize.outerGlow".into(), params: Default::default(), visible: true });
    n.id = d.alloc_id();
    let layer = d.default_layer().unwrap();
    d.insert(Some(layer), 0, n).unwrap();
    export_with_report(&d, &PdfOptions::default()).unwrap().warnings
}

#[test]
fn raster_effects_warn_on_fills_strokes_and_objects() {
    for item in [None, Some(0), Some(1)] {
        assert!(warnings(item).iter().any(|w| w.starts_with("raster effects")), "{item:?}");
    }
}
