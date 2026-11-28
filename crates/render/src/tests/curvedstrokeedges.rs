//! Wide strokes along curves: both rims cover their edge pixels close to the exact coverage of
//! the curves they follow, and narrow strokes, short curves and straight outlines are drawn as
//! they were.

use super::*;
use crate::stroke::{fine, wide};
use vectorcraft_doc::{AppearanceItem, StrokeGradientMode, StrokeLayer};

/// The share of pixel (`px`, `py`) inside the circle at `c` with radius `r`, integrated over 512
/// columns of the pixel.
fn disc_coverage(c: kurbo::Point, r: f64, px: u32, py: u32) -> f64 {
    const COLUMNS: u32 = 512;
    let (top, bottom) = (f64::from(py), f64::from(py) + 1.0);
    (0..COLUMNS)
        .map(|k| {
            let x = f64::from(px) + (f64::from(k) + 0.5) / f64::from(COLUMNS);
            let u = (x - c.x) / r;
            if u.abs() >= 1.0 {
                return 0.0;
            }
            let h = r * (1.0 - u * u).sqrt();
            ((c.y + h).min(bottom) - (c.y - h).max(top)).max(0.0)
        })
        .sum::<f64>()
        / f64::from(COLUMNS)
}

/// A circle of radius `r` at `c`, as eight cubics, each an eighth of it, within five millionths
/// of `r` of it.
fn circle(c: (f64, f64), r: f64) -> BezPath {
    let c = kurbo::Point::new(c.0, c.1);
    let at = |a: f64| c + kurbo::Vec2::from_angle(a) * r;
    let step = std::f64::consts::FRAC_PI_4;
    let handle = r * 4.0 / 3.0 * (step / 4.0).tan();
    let mut p = BezPath::new();
    p.move_to(at(0.0));
    for k in 0..8 {
        let (a, b) = (f64::from(k) * step, f64::from(k + 1) * step);
        let tangent = |a: f64| kurbo::Vec2::from_angle(a + std::f64::consts::FRAC_PI_2) * handle;
        p.curve_to(at(a) + tangent(a), at(b) - tangent(b), at(b));
    }
    p.close_path();
    p
}

/// A document of 1200 points holding a circle of radius 400 at its middle, stroked 80 pt with
/// `paint` at `opacity`. A gradient is laid along the stroke.
fn ring(paint: Paint, opacity: f32) -> Document {
    let mut d = Document::new(1200.0, 1200.0);
    let path = vectorcraft_geom::PathData::from_bezpath(&circle((600.0, 600.0), 400.0));
    let mut node = Node::path(d.alloc_id(), path, Appearance::basic(Paint::None, paint, 80.0));
    for item in &mut node.appearance.items {
        if let AppearanceItem::Stroke(st) = item {
            st.opacity = opacity;
            st.gradient_mode = StrokeGradientMode::Along;
        }
    }
    let l = d.layers[0].id;
    d.insert(Some(l), 0, node).unwrap();
    d
}

/// The worst edge pixel of one rim: how many alpha levels it is off, where, and how many edge
/// pixels the rim has.
#[derive(Debug, Default)]
struct Rim {
    worst: u8,
    pixels: usize,
    at: String,
}

/// The rims of the ring of [`ring`] (outer radius 440, inner 360) drawn through `view`, which
/// scales by `scale`, at `side`×`side`, with `paint` and `opacity`. An edge pixel is one whose
/// exact coverage by the circle of its rim is between 2% and 98%; its alpha is compared with that
/// coverage of the alpha the ring paints.
fn ring_rims(view: Affine, side: u32, scale: f64, paint: Paint, opacity: f32) -> (Rim, Rim) {
    let opts = RenderOptions { background: None, artboards: false, ..Default::default() };
    let got = Renderer::new().render(&ring(paint, opacity), side, side, view, &opts);
    let c = view * kurbo::Point::new(600.0, 600.0);
    let full = 255.0 * f64::from(opacity);
    let (mut outer, mut inner) = (Rim::default(), Rim::default());
    for y in 0..side {
        for x in 0..side {
            let from = (kurbo::Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5) - c).hypot();
            for (rim, r, outside) in [(&mut outer, 440.0 * scale, true), (&mut inner, 360.0 * scale, false)] {
                // Only pixels within two pixels of a rim can be crossed by it.
                if (from - r).abs() >= 2.0 {
                    continue;
                }
                let e = disc_coverage(c, r, x, y);
                if !(0.02..=0.98).contains(&e) {
                    continue;
                }
                // Inside the outer rim the ring covers the pixel; inside the inner rim it doesn't.
                let want = if outside { e } else { 1.0 - e } * full;
                let alpha = f64::from(got.pixel(x, y)[3]);
                let diff = (alpha - want).abs().round().min(255.0) as u8;
                rim.pixels += 1;
                if diff > rim.worst {
                    rim.worst = diff;
                    rim.at = format!("({x},{y}) {alpha} vs {want:.1}");
                }
            }
        }
    }
    (outer, inner)
}

/// Most alpha levels an edge pixel of a wide curved stroke may differ from the exact coverage of
/// its rim.
const EDGE_LEVELS: u8 = 36;

/// A wide stroke along a curve paints both of its rims within [`EDGE_LEVELS`] of their exact
/// coverage: zoomed out and at 1:1, through a turned view, with a gradient laid along it, and
/// translucent (within half as many levels).
#[test]
fn wide_curved_strokes_cover_their_rims_as_the_curves_do() {
    let turned = Affine::translate((150.0, 150.0)) * Affine::rotate(0.5) * Affine::scale(0.25) * Affine::translate((-600.0, -600.0));
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
    let black = || Paint::solid(Color::BLACK);
    let mut off = vec![];
    for (name, view, side, scale, paint, opacity) in [
        ("zoomed out", Affine::scale(0.25), 300, 0.25, black(), 1.0),
        ("1:1", Affine::translate((-100.0, -100.0)), 1000, 1.0, black(), 1.0),
        ("turned", turned, 300, 0.25, black(), 1.0),
        ("gradient", Affine::scale(0.25), 300, 0.25, gradient, 1.0),
        ("translucent", Affine::scale(0.25), 300, 0.25, black(), 0.5),
    ] {
        let (outer, inner) = ring_rims(view, side, scale, paint, opacity);
        let bound = if opacity < 1.0 { EDGE_LEVELS / 2 + 1 } else { EDGE_LEVELS };
        for (rim, r) in [("outer", outer), ("inner", inner)] {
            assert!(r.pixels > 200, "{name}, {rim} rim: {} edge pixels", r.pixels);
            if r.worst > bound {
                off.push(format!("{name}, {rim} rim: {} levels off at {} ({} edge pixels)", r.worst, r.at, r.pixels));
            }
        }
    }
    assert!(off.is_empty(), "{off:#?}");
}

/// The outline of the 80 pt stroke along a circle of radius 400 about (600, 600), at `tol`.
fn ring_outline(tol: f64) -> BezPath {
    let st = StrokeLayer::new(Paint::solid(Color::BLACK), 80.0);
    crate::stroke::outline(&circle((600.0, 600.0), 400.0), &st, 80.0, tol)
}

/// How many moves and closes `p` has.
fn moves_and_closes(p: &BezPath) -> (usize, usize) {
    let els = p.elements();
    (els.iter().filter(|e| matches!(e, kurbo::PathEl::MoveTo(_))).count(), els.iter().filter(|e| matches!(e, kurbo::PathEl::ClosePath)).count())
}

/// A wide stroke's outline has its long curves drawn as lines that cross them: the line ends lie
/// outside the circles, the lines' middles inside, all within half of the tolerance, with its
/// subpaths kept.
#[test]
fn wide_outlines_draw_their_curves_as_lines_across_them() {
    for tol in [1.0, 0.25] {
        let raw = ring_outline(tol);
        let lines = wide(raw.clone(), 80.0, tol);
        assert_eq!(moves_and_closes(&lines), moves_and_closes(&raw), "tolerance {tol}");
        assert!(lines.elements().iter().all(|e| matches!(e, kurbo::PathEl::MoveTo(_) | kurbo::PathEl::LineTo(_) | kurbo::PathEl::ClosePath)));
        let c = kurbo::Point::new(600.0, 600.0);
        // How far a point is outside the rim it is nearest to.
        let off = |p: kurbo::Point| {
            let r = (p - c).hypot();
            if r > 400.0 { r - 440.0 } else { r - 360.0 }
        };
        let (mut ends, mut middles) = (vec![], vec![]);
        for seg in lines.segments() {
            if let kurbo::PathSeg::Line(l) = seg
                && l.p1 != l.p0
            {
                ends.push(off(l.p1));
                middles.push(off(l.p0.midpoint(l.p1)));
            }
        }
        assert!(ends.len() > 100, "tolerance {tol}: {} lines", ends.len());
        let most = ends.iter().chain(&middles).fold(0.0_f64, |m, d| m.max(d.abs()));
        assert!(most <= tol / 2.0, "tolerance {tol}: a line {most} from its curve");
        assert!(ends.iter().filter(|d| **d > tol * 0.1).count() > ends.len() / 2, "tolerance {tol}: line ends outside the curves");
        assert!(middles.iter().filter(|d| **d < -tol * 0.1).count() > middles.len() / 2, "tolerance {tol}: line middles inside the curves");
    }
}

/// Outlines of strokes narrower than 32 tolerances, outlines without a curve spanning 128
/// tolerances, and outlines that would take more than a million lines are kept as they are.
#[test]
fn narrow_short_straight_or_huge_outlines_are_kept() {
    let tol = 0.25;
    let st = |w: f64| StrokeLayer::new(Paint::solid(Color::BLACK), w);
    let narrow = crate::stroke::outline(&circle((600.0, 600.0), 400.0), &st(7.0), 7.0, tol);
    let small = crate::stroke::outline(&circle((50.0, 50.0), 10.0), &st(9.0), 9.0, tol);
    let straight = crate::stroke::outline(&Rect::new(0.0, 0.0, 400.0, 300.0).to_path(0.1), &st(20.0), 20.0, tol);
    // Eight curves of about 200,000 lines each.
    let huge = circle((0.0, 0.0), 1e11);
    for (name, p, width) in [("narrow", narrow, 7.0), ("small curves", small, 9.0), ("straight", straight, 20.0), ("huge", huge, 20.0)] {
        assert!(p.elements().len() > 1, "{name}");
        assert_eq!(wide(p.clone(), width, tol).elements(), p.elements(), "{name}: kept");
    }
    for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let raw = ring_outline(1.0);
        assert_eq!(wide(raw.clone(), 80.0, bad).elements(), raw.elements(), "tolerance {bad}");
    }
}

/// Where a long curve meets a corner, a line or a curve that isn't finite, its end stays where it
/// was, and the segments that aren't long curves are kept as they are.
#[test]
fn curve_ends_at_corners_lines_and_bad_curves_stay_put() {
    let mut p = BezPath::new();
    p.move_to((0.0, 0.0));
    p.curve_to((0.0, 100.0), (100.0, 200.0), (200.0, 200.0));
    p.line_to((300.0, 200.0));
    p.curve_to((300.0, 100.0), (400.0, 0.0), (500.0, 0.0));
    p.curve_to((f64::NAN, 0.0), (600.0, 0.0), (700.0, 0.0));
    p.curve_to((700.0, 300.0), (900.0, 300.0), (900.0, 0.0));
    p.quad_to((905.0, 5.0), (910.0, 0.0));
    let lines = wide(p.clone(), 40.0, 0.25);
    let points: Vec<_> = lines.elements().iter().filter_map(|e| e.end_point()).collect();
    for corner in [(0.0, 0.0), (200.0, 200.0), (300.0, 200.0), (500.0, 0.0), (700.0, 0.0), (900.0, 0.0), (910.0, 0.0)] {
        assert!(points.contains(&kurbo::Point::from(corner)), "{corner:?} kept: {lines:?}");
    }
    let curves: Vec<_> = lines.elements().iter().filter(|e| matches!(e, kurbo::PathEl::CurveTo(..) | kurbo::PathEl::QuadTo(..))).collect();
    assert_eq!(curves.len(), 2, "the curve that isn't finite and the short curve are kept: {curves:?}");
    assert!(lines.elements().len() > 40, "{} elements", lines.elements().len());
}

/// An outline is drawn with lines through a view that stretches no length by more than the frame's
/// pixel size allows: one that zooms, turns or shrinks the stroke's space, and not one that
/// stretches it unevenly, enlarges it beyond the frame's pixels, collapses it or isn't finite.
#[test]
fn outlines_are_drawn_as_lines_only_through_even_views() {
    let px = |v: Affine| 1.0 / v.determinant().abs().sqrt();
    for (name, m, px, want) in [
        ("zoomed out", Affine::scale(0.25), px(Affine::scale(0.25)), true),
        ("turned", Affine::rotate(0.5) * Affine::scale(3.0), px(Affine::scale(3.0)), true),
        ("flipped", Affine::FLIP_X, 1.0, true),
        ("text made smaller", Affine::scale(0.5), 1.0, true),
        ("stretched", Affine::scale_non_uniform(0.25, 0.2), px(Affine::scale_non_uniform(0.25, 0.2)), false),
        ("text made larger", Affine::scale(2.0), 1.0, false),
        ("collapsed", Affine::scale(0.0), 1.0, false),
        ("not a number", Affine::scale(f64::NAN), 1.0, false),
        ("no pixel size", Affine::IDENTITY, f64::INFINITY, false),
    ] {
        assert_eq!(fine(m, px), want, "{name}");
    }
}
