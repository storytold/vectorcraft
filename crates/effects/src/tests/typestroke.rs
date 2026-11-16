//! Character strokes of outlined type under the type's transform ([`paint_type_piece`]): a live
//! stroke with a scaled weight under a similarity, otherwise the area the stroke paints in text
//! space, placed by the transform and filled.

use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Dash, LineJoin, Node, NodeId, StrokeAlign};
use vectorcraft_geom::{Affine, FillRule, Rect, shapes};

use crate::*;

const BOX: Rect = Rect::new(0.0, 0.0, 10.0, 10.0);

fn red() -> Color {
    Color::rgb(1.0, 0.0, 0.0)
}

/// A black run with a red stroke of `width` and round joins.
fn style(width: f64) -> CharStyle {
    CharStyle {
        fill: Paint::solid(Color::BLACK),
        stroke: Paint::solid(red()),
        stroke_width: width,
        stroke_join: LineJoin::Round,
        ..CharStyle::default()
    }
}

/// A 10 x 10 square in text space standing in for a glyph.
fn square() -> Node {
    Node::path(NodeId(7), shapes::rectangle(BOX), Appearance::default())
}

fn area(n: &Node) -> f64 {
    vectorcraft_pathops::area(n.path_data().unwrap(), FillRule::NonZero).abs()
}

fn near(a: Rect, b: Rect) -> bool {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|d| d.abs() < 0.05)
}

#[test]
fn an_uneven_scale_fills_the_stroke_the_canvas_paints() {
    let mut piece = square();
    let stroke = paint_type_piece(&mut piece, &style(4.0), BOX, Affine::scale_non_uniform(3.0, 1.0)).expect("a filled stroke");
    assert!(piece.appearance.stroke().is_none(), "the piece keeps only its fill");
    assert_eq!(piece.appearance.fill_paint(), Paint::solid(Color::BLACK));
    assert_eq!(piece.geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 30.0, 10.0));
    assert!(stroke.appearance.stroke().is_none());
    assert_eq!(stroke.appearance.fill_paint(), Paint::solid(red()));
    assert_eq!(stroke.id, NodeId(7));
    // 4 pt across in text space: 12 pt across the upright sides and 4 pt across the level ones.
    let b = stroke.geometric_bounds().unwrap();
    assert!(near(b, Rect::new(-6.0, -2.0, 36.0, 12.0)), "{b:?}");
    // The ring of a 10 pt square stroked 4 pt with round joins (180 + 4π outside, 36 inside), 3 times as wide.
    let ring = 180.0 + 4.0 * std::f64::consts::PI - 36.0;
    assert!((area(&stroke) - 3.0 * ring).abs() < 0.5, "{}", area(&stroke));
}

#[test]
fn a_similarity_keeps_the_live_stroke_with_a_scaled_weight() {
    for (xf, weight) in [
        (Affine::translate((5.0, 5.0)), 4.0),
        (Affine::scale(2.0), 8.0),
        (Affine::rotate(0.5) * Affine::scale(2.0), 8.0),
        (Affine::scale_non_uniform(-2.0, 2.0), 8.0),
    ] {
        let mut piece = square();
        assert!(paint_type_piece(&mut piece, &style(4.0), BOX, xf).is_none(), "{xf:?}");
        let st = piece.appearance.stroke().expect("a live stroke");
        assert!((st.width - weight).abs() < 1e-9, "{xf:?}: {}", st.width);
        assert_eq!(st.join, LineJoin::Round);
    }
}

#[test]
fn shears_and_flattening_transforms_fill_the_stroke() {
    // A shear keeps the area.
    let mut piece = square();
    let stroke = paint_type_piece(&mut piece, &style(4.0), BOX, Affine::skew(0.5, 0.0)).expect("a filled stroke");
    let ring = 180.0 + 4.0 * std::f64::consts::PI - 36.0;
    assert!((area(&stroke) - ring).abs() < 0.5, "{}", area(&stroke));
    // Flattened to a line or a point: no division by the determinant, nothing non-finite.
    for xf in [Affine::scale_non_uniform(2.0, 0.0), Affine::scale(0.0)] {
        let mut piece = square();
        let stroke = paint_type_piece(&mut piece, &style(4.0), BOX, xf).expect("a filled stroke");
        assert!(piece.appearance.stroke().is_none());
        if let Some(b) = stroke.geometric_bounds() {
            assert!([b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) && b.height().abs() < 1e-9, "{xf:?}: {b:?}");
        }
    }
}

#[test]
fn the_stroke_options_shape_the_filled_stroke_in_text_space() {
    let xf = Affine::scale_non_uniform(3.0, 1.0);
    let filled = |st: CharStyle| paint_type_piece(&mut square(), &st, BOX, xf);
    // No stroke, or no weight: nothing is filled.
    assert!(filled(style(0.0)).is_none());
    assert!(filled(CharStyle { stroke: Paint::None, ..style(4.0) }).is_none());
    // Inside and outside alignment are kept to their side of the square before it is scaled.
    let inside = filled(CharStyle { stroke_align: StrokeAlign::Inside, ..style(4.0) }).unwrap();
    assert!(near(inside.geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 30.0, 10.0)));
    assert!((area(&inside) - 3.0 * (100.0 - 4.0)).abs() < 0.5, "{}", area(&inside));
    let outside = filled(CharStyle { stroke_align: StrokeAlign::Outside, ..style(4.0) }).unwrap();
    assert!(near(outside.geometric_bounds().unwrap(), Rect::new(-12.0, -4.0, 42.0, 14.0)), "{:?}", outside.geometric_bounds());
    // Dashes are laid out along the square in text space: half of each 10 pt is painted.
    let solid = area(&filled(style(1.0)).unwrap());
    let dash = Dash { pattern: vec![5.0, 5.0], ..Default::default() };
    let dashed = area(&filled(CharStyle { stroke_dash: Some(dash), ..style(1.0) }).unwrap());
    assert!(dashed < 0.7 * solid && dashed > 0.3 * solid, "{dashed} of {solid}");
}

#[test]
fn a_gradient_on_the_stroke_stays_where_the_stroke_had_it() {
    let stops = vec![GradientStop::new(0.0, red()), GradientStop::new(1.0, Color::BLACK)];
    let ramp = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    let mut piece = square();
    let stroke = paint_type_piece(&mut piece, &CharStyle { stroke: ramp, ..style(4.0) }, BOX, Affine::scale_non_uniform(3.0, 1.0)).unwrap();
    let Paint::Gradient(g) = stroke.appearance.fill_paint() else { panic!("a gradient fill") };
    // Fitted across the stroke's box in text space (the square grown by half the weight), then scaled.
    let geom = g.geom.expect("placed");
    assert!((geom.start.x + 6.0).abs() < 1e-6 && (geom.end.x - 36.0).abs() < 1e-6, "{geom:?}");
}
