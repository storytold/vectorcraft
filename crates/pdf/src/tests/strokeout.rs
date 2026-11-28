//! Strokes a PDF stroke can't draw — width profiles, brushes — are written as the canvas's filled
//! outlines; aligned strokes are clipped like on the canvas, open paths stroke centred.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, LineJoin, Node, NodeId, StrokeAlign, StrokeLayer, WidthProfile};
use vectorcraft_geom::{PathData, Point, SubPath};
use vectorcraft_testkit::raster::render_artboard;

use crate::tests_stroke::{line_doc, painted_boxes};
use crate::*;

fn black(width: f64, f: impl FnOnce(&mut StrokeLayer)) -> StrokeLayer {
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), width);
    f(&mut st);
    st
}

#[test]
fn a_lens_profile_is_written_as_a_fill() {
    let (d, _) = line_doc(black(10.0, |s| s.profile = Some(WidthProfile::lens())));
    let (fills, strokes) = painted_boxes(&d);
    assert!(strokes.is_empty(), "{strokes:?}");
    assert_eq!(fills.len(), 1);
    // The lens is full width (10 pt) only in the middle of the 20..80 line.
    assert!((fills[0].height() - 10.0).abs() < 0.05 && (fills[0].width() - 60.0).abs() < 0.05, "{:?}", fills[0]);
}

#[test]
fn an_open_path_with_inside_alignment_strokes_centred() {
    let (d, _) = line_doc(black(4.0, |s| s.align = StrokeAlign::Inside));
    let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
    let mut widths = vec![];
    back.walk(|m| {
        assert!(!matches!(m.kind, vectorcraft_doc::NodeKind::Group { clip: true, .. }), "no clip");
        widths.extend(m.appearance.items.iter().filter_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s.width) } else { None }));
    });
    assert_eq!(widths.len(), 1);
    assert!((widths[0] - 4.0).abs() < 1e-3, "{widths:?}");
}

#[test]
fn brushed_strokes_are_written_as_their_art() {
    let (d, _) = line_doc(black(2.0, |s| s.brush = Some("Tapered Stroke".into())));
    let (fills, strokes) = painted_boxes(&d);
    assert!(strokes.is_empty() && !fills.is_empty(), "{fills:?} {strokes:?}");
}

/// The clip frame of an outside stroke covers its miter spikes: a thin spike far above a sharp
/// apex is as dark in the exported PDF as on the canvas.
#[test]
fn outside_strokes_keep_their_miter_spikes() {
    let mut d = Document::new(300.0, 300.0);
    let tri = SubPath::polyline(&[Point::new(130.0, 250.0), Point::new(170.0, 250.0), Point::new(150.0, 100.0)], true);
    let st = black(10.0, |s| {
        s.align = StrokeAlign::Outside;
        s.join = LineJoin::Miter;
        s.miter_limit = 10.0;
    });
    let mut n = Node::path(NodeId(0), PathData::single(tri), Appearance { items: vec![AppearanceItem::Stroke(st)], ..Default::default() });
    n.id = d.alloc_id();
    let layer = d.default_layer().unwrap();
    d.insert(Some(layer), 0, n).unwrap();
    // The spike's tip is ~76 pt above the apex (10 pt / sin 7.6°); 60 pt up it is still 4 pt wide.
    let (x, y) = (150, 40);
    let canvas = render_artboard(&d);
    let back = render_artboard(&import(&export(&d, &PdfOptions::default()).unwrap()).unwrap());
    let dark = |p: [u8; 3]| p.iter().all(|c| *c < 64);
    assert!(dark(canvas.over_white(x, y)), "the canvas draws the spike: {:?}", canvas.over_white(x, y));
    assert!(dark(back.over_white(x, y)), "the PDF keeps the spike: {:?}", back.over_white(x, y));
    assert!(!dark(back.over_white(x, 20)), "past its tip");
}
