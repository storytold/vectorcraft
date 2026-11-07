//! Stroke geometry in PDF export: arrowheads and the trimmed line are written as the shared
//! `vectorcraft_effects::stroke` outlines, and they take the stroke opacity once.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, ArrowAlign, Arrowhead, Document, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_effects::stroke::{OUTLINE_TOL, line_outline, stroke_pieces};
use vectorcraft_geom::{Point, Rect, Shape, shapes};

use crate::*;

pub(crate) fn line_doc(st: StrokeLayer) -> (Document, Node) {
    let mut d = Document::new(100.0, 100.0);
    let mut n = Node::path(
        NodeId(0),
        shapes::line(Point::new(20.0, 50.0), Point::new(80.0, 50.0)),
        Appearance { items: vec![AppearanceItem::Stroke(st)], ..Default::default() },
    );
    n.id = d.alloc_id();
    let layer = d.default_layer().unwrap();
    d.insert(Some(layer), 0, n.clone()).unwrap();
    (d, n)
}

fn arrow_stroke(kind: Arrowhead, align: ArrowAlign, opacity: f32) -> StrokeLayer {
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), 4.0);
    st.end_arrow = Some(kind);
    st.arrow_align = align;
    st.opacity = opacity;
    st
}

fn close(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(p, q)| (p - q).abs() < 0.01)
}

/// Bounding boxes of the filled and the stroked paths of a PDF read back.
pub(crate) fn painted_boxes(d: &Document) -> (Vec<Rect>, Vec<Rect>) {
    let back = import(&export(d, &PdfOptions::default()).unwrap()).unwrap();
    let (mut fills, mut strokes) = (vec![], vec![]);
    back.walk(|m| {
        let Some(bb) = m.geometric_bounds().filter(|_| matches!(m.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })) else { return };
        match m.appearance.items.first() {
            Some(AppearanceItem::Fill(_)) => fills.push(bb),
            Some(AppearanceItem::Stroke(_)) => strokes.push(bb),
            None => {}
        }
    });
    (fills, strokes)
}

#[test]
fn arrowheads_and_the_trimmed_line_match_the_shared_geometry() {
    for align in [ArrowAlign::Extend, ArrowAlign::Tip] {
        for kind in Arrowhead::ALL {
            let (d, n) = line_doc(arrow_stroke(kind, align, 1.0));
            let bp = n.path_data().unwrap().to_bezpath();
            let st = n.appearance.stroke().unwrap();
            let pieces = stroke_pieces(&bp, st);
            let (fills, strokes) = painted_boxes(&d);
            let head = pieces.heads[0].outline.bounding_box();
            let line = line_outline(&pieces.line, st, st.width, OUTLINE_TOL).bounding_box();
            assert!(strokes.is_empty(), "{kind:?} {align:?}: the line is written as its outline");
            assert_eq!(fills.len(), 2, "{kind:?} {align:?}: the line, then the head");
            assert!(close(fills[0], line), "{kind:?} {align:?}: {:?} vs {line:?}", fills[0]);
            assert!(close(fills[1], head), "{kind:?} {align:?}: {:?} vs {head:?}", fills[1]);
            if align == ArrowAlign::Tip {
                assert!((fills[0].x1 - (80.0 - pieces.heads[0].inset)).abs() < 0.01, "{kind:?}: the stroke stops under the head");
            }
        }
    }
}

#[test]
fn line_and_head_share_one_opacity_group() {
    let (d, _) = line_doc(arrow_stroke(Arrowhead::Triangle, ArrowAlign::Tip, 0.5));
    let back = import(&export(&d, &PdfOptions::default()).unwrap()).unwrap();
    let mut group_opacity = None;
    let mut leaf_opacities = vec![];
    back.walk(|m| match &m.kind {
        NodeKind::Group { .. } if m.opacity < 1.0 => group_opacity = Some(m.opacity),
        NodeKind::Path { .. } => {
            leaf_opacities.push(m.opacity);
            for i in &m.appearance.items {
                leaf_opacities.push(match i {
                    AppearanceItem::Fill(f) => f.opacity,
                    AppearanceItem::Stroke(s) => s.opacity,
                });
            }
        }
        _ => {}
    });
    assert!(group_opacity.is_some_and(|o| (o - 0.5).abs() < 0.01), "{group_opacity:?}");
    assert!(leaf_opacities.iter().all(|o| (o - 1.0).abs() < 1e-3), "{leaf_opacities:?}");
}
