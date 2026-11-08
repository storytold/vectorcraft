//! Gradients along and across strokes: positions along the path and across the stroke, no seams
//! between the slices, translucent stops painted once.

use vectorcraft_color::{Gradient, GradientPaint, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Node, StrokeGradientMode, StrokeLayer};
use vectorcraft_geom::PathData;

use crate::*;

const SIZE: u32 = 200;

/// `bp` stroked 20 pt with `g` laid `mode`, on a 200×200 page (white, or transparent).
fn render(bp: &BezPath, g: Gradient, mode: StrokeGradientMode, white: bool) -> Rendered {
    let mut st = StrokeLayer::new(Paint::Gradient(Box::new(GradientPaint::new(g))), 20.0);
    st.gradient_mode = mode;
    let mut d = Document::new(SIZE as f64, SIZE as f64);
    let n = Node::path(d.alloc_id(), PathData::from_bezpath(bp), Appearance { items: vec![AppearanceItem::Stroke(st)], ..Default::default() });
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    let background = white.then_some([255, 255, 255, 255]);
    Renderer::new().render(&d, SIZE, SIZE, Affine::IDENTITY, &RenderOptions { background, ..Default::default() })
}

fn line(a: (f64, f64), b: (f64, f64)) -> BezPath {
    let mut p = BezPath::new();
    p.move_to(a);
    p.line_to(b);
    p
}

/// Grey level (0..1) at the centre of pixel (x, y).
fn grey(img: &Rendered, x: f64, y: f64) -> f64 {
    img.pixel(x as u32, y as u32)[0] as f64 / 255.0
}

#[test]
fn along_a_quarter_of_the_length_is_a_quarter_of_the_gradient_in_any_direction() {
    for (a, b) in [((20.0, 100.0), (180.0, 100.0)), ((100.0, 180.0), (100.0, 20.0)), ((30.0, 30.0), (170.0, 170.0))] {
        let img = render(&line(a, b), Gradient::default(), StrokeGradientMode::Along, true);
        for (f, off) in [(0.25, 0.0), (0.25, 6.0), (0.75, -6.0), (0.5, 0.0)] {
            let (dx, dy) = (b.0 - a.0, b.1 - a.1);
            let len = (dx * dx + dy * dy).sqrt();
            let (x, y) = (a.0 + dx * f - dy / len * off, a.1 + dy * f + dx / len * off);
            let want = 1.0 - f;
            assert!((grey(&img, x, y) - want).abs() < 0.03, "{a:?}→{b:?} at {f}: {} vs {want}", grey(&img, x, y));
        }
    }
}

#[test]
fn across_runs_from_the_left_edge_to_the_right() {
    // Travelling right, the left edge is the top one (y 90), the right edge the bottom (y 110).
    let img = render(&line((20.0, 100.0), (180.0, 100.0)), Gradient::default(), StrokeGradientMode::Across, true);
    for (y, want) in [(91.0, 0.925), (95.0, 0.725), (100.0, 0.475), (108.0, 0.075)] {
        for x in [40.0, 100.0, 160.0] {
            assert!((grey(&img, x, y) - want).abs() < 0.03, "({x}, {y}): {} vs {want}", grey(&img, x, y));
        }
    }
    // Within (the default) the gradient lies on the page: left to right here.
    let within = render(&line((20.0, 100.0), (180.0, 100.0)), Gradient::default(), StrokeGradientMode::Within, true);
    assert!(grey(&within, 40.0, 91.0) > grey(&within, 160.0, 91.0) + 0.5);
}

#[test]
fn slices_meet_without_seams_and_translucent_stops_paint_once() {
    let circle = kurbo::Circle::new((100.0, 100.0), 60.0).to_path(0.01);
    let mut half = Gradient::default();
    for s in &mut half.stops {
        s.opacity = 0.5;
    }
    for mode in [StrokeGradientMode::Along, StrokeGradientMode::Across] {
        let solid = render(&circle, Gradient::default(), mode, false);
        let translucent = render(&circle, half.clone(), mode, false);
        for k in 0..720 {
            let a = k as f64 * std::f64::consts::TAU / 720.0;
            for r in [55.0, 60.0, 65.0] {
                let (x, y) = (100.0 + r * a.cos(), 100.0 + r * a.sin());
                assert_eq!(solid.pixel(x as u32, y as u32)[3], 255, "{mode:?}: a seam at ({x:.1}, {y:.1})");
                let alpha = translucent.pixel(x as u32, y as u32)[3] as i32;
                assert!((alpha - 128).abs() <= 2, "{mode:?}: alpha {alpha} at ({x:.1}, {y:.1})");
            }
        }
    }
}

#[test]
fn corners_and_tight_turns_have_no_holes() {
    let mut corner = line((40.0, 40.0), (160.0, 40.0));
    corner.line_to((160.0, 160.0));
    corner.line_to((60.0, 120.0));
    let tight = kurbo::Circle::new((100.0, 100.0), 6.0).to_path(0.01);
    let mut half = Gradient::default();
    for s in &mut half.stops {
        s.opacity = 0.5;
    }
    for mode in [StrokeGradientMode::Within, StrokeGradientMode::Along, StrokeGradientMode::Across] {
        for (bp, probes) in [
            (
                &corner,
                vec![(160.0, 40.0), (156.0, 36.0), (165.0, 45.0), (163.0, 37.0), (157.0, 43.0), (160.0, 160.0), (166.0, 158.0), (152.0, 157.0)],
            ),
            // The stroke's inside edge folds over the centre: a 4 pt hole stays there.
            (&tight, vec![(105.5, 100.0), (95.0, 103.0), (108.0, 100.0), (100.0, 113.0), (91.0, 92.0), (100.0, 94.0)]),
        ] {
            let solid = render(bp, Gradient::default(), mode, false);
            let translucent = render(bp, half.clone(), mode, false);
            for (x, y) in probes {
                assert_eq!(solid.pixel(x as u32, y as u32)[3], 255, "{mode:?}: a hole at ({x}, {y})");
                let alpha = translucent.pixel(x as u32, y as u32)[3] as i32;
                assert!((alpha - 128).abs() <= 2, "{mode:?}: alpha {alpha} at ({x}, {y})");
            }
        }
    }
}
