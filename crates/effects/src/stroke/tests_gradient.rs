use kurbo::{BezPath, Point, Shape, Vec2};
use vectorcraft_color::{Color, Gradient, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::{ArrowAlign, Arrowhead, Dash, LineCap, LineJoin, StrokeAlign, StrokeGradientMode, StrokeLayer, WidthProfile};
use vectorcraft_geom::FillRule;

use super::*;

fn stroke(width: f64, mode: StrokeGradientMode) -> StrokeLayer {
    let mut st = StrokeLayer::new(Paint::Gradient(Box::new(GradientPaint::new(Gradient::default()))), width);
    st.gradient_mode = mode;
    st
}

fn poly(pts: &[(f64, f64)], closed: bool) -> BezPath {
    let mut b = BezPath::new();
    b.move_to(pts[0]);
    for p in &pts[1..] {
        b.line_to(*p);
    }
    if closed {
        b.close_path();
    }
    b
}

/// The gradient position the slices give `p` (the last slice holding it wins, as when filled).
fn param(slices: &[Slice], p: Point) -> Option<f64> {
    slices.iter().rev().find(|s| s.shape.contains(p)).map(|s| {
        let d = s.to - s.from;
        let u = if d.hypot2() > 1e-18 { ((p - s.from).dot(d) / d.hypot2()).clamp(0.0, 1.0) } else { 0.0 };
        s.span.0 + (s.span.1 - s.span.0) * u
    })
}

fn slices(bp: &BezPath, st: &StrokeLayer) -> Vec<Slice> {
    gradient_slices(bp, FillRule::NonZero, st, 0.01, 0.5, 0.0)
}

/// Every point the stroke paints (on a grid over each of its outlines) lies in a slice.
fn assert_covered(bp: &BezPath, st: &StrokeLayer) {
    let sl = slices(bp, st);
    let pieces = stroke_pieces(bp, st);
    let mut parts = vec![line_outline(&pieces.line, st, aligned_width(st, is_closed(bp)), 0.01)];
    parts.extend(pieces.heads.into_iter().map(|h| h.outline));
    for part in &parts {
        let b = part.bounding_box();
        let n = 60;
        for i in 0..=n {
            for j in 0..=n {
                let p = Point::new(b.x0 + b.width() * i as f64 / n as f64, b.y0 + b.height() * j as f64 / n as f64);
                if part.contains(p) {
                    assert!(param(&sl, p).is_some(), "{p:?} of the stroke is in no slice ({:?})", st.join);
                }
            }
        }
    }
}

#[test]
fn span_is_the_part_of_the_gradient_between_two_positions() {
    let g = Gradient::default();
    let grey = |g: &Gradient, t: f32| g.sample(t).0.to_rgb()[0];
    let s = g.span(0.25, 0.75);
    assert_eq!(s.stops.len(), 2);
    assert!((grey(&s, 0.0) - 0.75).abs() < 1e-5 && (grey(&s, 1.0) - 0.25).abs() < 1e-5);
    assert!((grey(&s, 0.5) - 0.5).abs() < 1e-5);
    // Backwards, and one colour when empty.
    let r = g.span(0.75, 0.25);
    assert!((grey(&r, 0.0) - 0.25).abs() < 1e-5 && (grey(&r, 1.0) - 0.75).abs() < 1e-5);
    let c = g.span(0.4, 0.4);
    assert!((grey(&c, 0.0) - grey(&c, 1.0)).abs() < 1e-6);
    // Stops and off-centre midpoints inside the span keep their colours.
    let mut g3 = Gradient::default();
    g3.stops.insert(1, GradientStop::new(0.5, Color::rgb(1.0, 0.0, 0.0)));
    g3.stops[0].midpoint = 0.25;
    let s3 = g3.span(0.0, 1.0);
    for t in [0.05, 0.125, 0.3, 0.5, 0.7, 0.95] {
        let (a, b) = (g3.sample(t).0.to_rgb(), s3.sample(t).0.to_rgb());
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4), "{t}: {a:?} vs {b:?}");
    }
}

#[test]
fn along_maps_the_length_of_the_path_in_any_direction() {
    let st = stroke(10.0, StrokeGradientMode::Along);
    for angle in [0.0f64, 37.0, 90.0, 200.0, 315.0] {
        let d = Vec2::new(angle.to_radians().cos(), angle.to_radians().sin());
        let start = Point::new(50.0, 50.0);
        let sl = slices(&poly(&[(start.x, start.y), (start.x + 100.0 * d.x, start.y + 100.0 * d.y)], false), &st);
        let n = Vec2::new(-d.y, d.x);
        for (at, across) in [(25.0, 0.0), (25.0, 4.0), (25.0, -4.0), (60.0, 2.0)] {
            let t = param(&sl, start + d * at + n * across).unwrap();
            assert!((t - at / 100.0).abs() < 1e-6, "{angle}°: {t} at {at}");
        }
        // Past the ends (under caps and arrowheads) the end colours carry on.
        assert_eq!(param(&sl, start - d * 0.3), Some(0.0));
        // The slices hug the stroke.
        assert_eq!(param(&sl, start + d * 50.0 + n * 7.0), None);
    }
    // Round a closed square: positions are fractions of its perimeter.
    let sl = slices(&poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true), &st);
    assert!((param(&sl, Point::new(50.0, 2.0)).unwrap() - 0.125).abs() < 1e-6);
    assert!((param(&sl, Point::new(102.0, 50.0)).unwrap() - 0.375).abs() < 1e-6);
    assert!((param(&sl, Point::new(101.0, -1.0)).unwrap() - 0.25).abs() < 1e-6);
}

#[test]
fn along_keeps_a_corners_colour_round_its_outside() {
    let st = stroke(20.0, StrokeGradientMode::Along);
    // Right then down: the outside of the corner is up and to the right.
    let sl = slices(&poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], false), &st);
    for p in [(105.0, -5.0), (109.0, -9.0), (101.0, -8.0), (108.0, -1.0)] {
        assert!((param(&sl, p.into()).unwrap() - 0.5).abs() < 1e-6, "{p:?}");
    }
    // Inside the corner each side takes the nearest point of its own segment.
    assert!((param(&sl, Point::new(92.0, 5.0)).unwrap() - 0.46).abs() < 1e-6);
    assert!((param(&sl, Point::new(95.0, 8.0)).unwrap() - 0.54).abs() < 1e-6);
}

/// An Across stroke gradient runs from the stroke's left edge to its right edge, also where a width profile moves them.
#[test]
fn across_runs_from_the_left_edge_to_the_right() {
    let st = stroke(10.0, StrokeGradientMode::Across);
    // Travelling +x (y down), the left edge is up.
    let line = poly(&[(0.0, 0.0), (100.0, 0.0)], false);
    let sl = slices(&line, &st);
    for (y, want) in [(-5.0, 0.0), (-2.5, 0.25), (0.0, 0.5), (4.0, 0.9), (5.0, 1.0)] {
        assert!((param(&sl, Point::new(37.0, y)).unwrap() - want).abs() < 1e-6, "{y}");
    }
    // The same all round a curve: half way across is half way, wherever.
    let sl = slices(&kurbo::Circle::new((0.0, 0.0), 60.0).to_path(1e-3), &st);
    for k in 0..12 {
        let dir = Vec2::from_angle(k as f64 * 0.5);
        let (inner, mid) = (param(&sl, (dir * 56.0).to_point()).unwrap(), param(&sl, (dir * 60.0).to_point()).unwrap());
        assert!((mid - 0.5).abs() < 0.02 && (inner - 0.5).abs() > 0.3, "{mid} {inner}");
    }
    // With a width profile, across runs between the stroke's edges where they are.
    let mut lens = st.clone();
    lens.profile = Some(WidthProfile::lens());
    let sl = slices(&line, &lens);
    // A quarter of the way along, the lens is three quarters as wide: its top edge is 3.75 up.
    assert!(param(&sl, Point::new(25.0, -3.6)).unwrap() < 0.05);
    assert!((param(&sl, Point::new(50.0, -2.5)).unwrap() - 0.25).abs() < 0.02);
}

#[test]
fn an_aligned_stroke_spans_the_gradient_across_the_side_it_shows_on() {
    for align in [StrokeAlign::Inside, StrokeAlign::Outside] {
        for reverse in [false, true] {
            let mut st = stroke(10.0, StrokeGradientMode::Across);
            st.align = align;
            let mut pts = vec![(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
            if reverse {
                pts.reverse();
            }
            let sl = slices(&poly(&pts, true), &st);
            // Across the top edge: from the path to 10 pt into the side the stroke shows on.
            let dy = if align == StrokeAlign::Inside { 1.0 } else { -1.0 };
            let (at_path, far) = (param(&sl, Point::new(50.0, 0.0)).unwrap(), param(&sl, Point::new(50.0, 10.0 * dy)).unwrap());
            assert!((at_path - far).abs() > 0.99, "{align:?} {reverse}: {at_path} → {far}");
            assert!((param(&sl, Point::new(50.0, 5.0 * dy)).unwrap() - 0.5).abs() < 1e-6);
        }
    }
}

/// The gradient slices cover every point the stroke paints, with each join, caps, arrowheads, a profile and dashes.
#[test]
fn slices_cover_everything_the_stroke_paints() {
    let zig = poly(&[(0.0, 0.0), (60.0, 40.0), (0.0, 80.0), (70.0, 90.0), (75.0, 20.0)], false);
    let mut curve = BezPath::new();
    curve.move_to((0.0, 0.0));
    curve.curve_to((80.0, -60.0), (120.0, 120.0), (10.0, 60.0));
    let triangle = poly(&[(0.0, 0.0), (100.0, 0.0), (50.0, 30.0)], true);
    for mode in [StrokeGradientMode::Along, StrokeGradientMode::Across] {
        for join in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel] {
            let mut st = stroke(12.0, mode);
            st.join = join;
            st.cap = LineCap::Square;
            assert_covered(&zig, &st);
            assert_covered(&curve, &st);
            assert_covered(&triangle, &st);
            st.start_arrow = Some(Arrowhead::Triangle);
            st.end_arrow = Some(Arrowhead::CircleOpen);
            assert_covered(&zig, &st);
            st.arrow_align = ArrowAlign::Tip;
            assert_covered(&curve, &st);
            // Teardrop, not Wave: with most profiles a dash across the zigzag's sharp corner
            // paints outside every slice (#5964). Back to Wave once that is fixed.
            st.profile = WidthProfile::preset("teardrop");
            st.dash = Some(Dash { pattern: vec![12.0, 6.0], ..Default::default() });
            assert_covered(&zig, &st);
            st.align = StrokeAlign::Outside;
            assert_covered(&triangle, &st);
        }
    }
}

#[test]
fn meshes_colour_the_stroke_as_the_slices_do() {
    let g = Gradient::default();
    let shape = poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 60.0)], false);
    for mode in [StrokeGradientMode::Along, StrokeGradientMode::Across] {
        let st = stroke(10.0, mode);
        let sl = slices(&shape, &st);
        let meshes = gradient_meshes(&shape, FillRule::NonZero, &st, &g);
        assert_eq!(meshes.len(), 1);
        assert!(meshes[0].is_valid());
        let quads = meshes[0].quads(4);
        for p in [(25.0, 0.0), (40.0, -3.0), (70.0, 4.0), (100.0, 30.0), (98.0, 50.0), (103.0, 20.0)] {
            let p = Point::new(p.0, p.1);
            let want = 1.0 - param(&sl, p).unwrap();
            let q = quads.iter().find(|q| poly(&q.pts.map(|v| (v.x, v.y)), true).contains(p)).expect("in the mesh");
            let got = q.color.to_rgb()[0] as f64;
            assert!((got - want).abs() < 0.05, "{mode:?} {p:?}: mesh {got} vs {want}");
        }
    }
}

/// #5964: with ribs only at the path's corners (an Along gradient), an arrowhead whose tip sits on
/// the end of a long straight leg is covered all the way to where it can reach: the ribs that
/// reach across it run to the end of its reach, rather than narrowing from its tip towards the
/// next corner and leaving the corners of its base outside.
#[test]
fn slices_cover_an_arrowhead_on_a_long_straight_leg() {
    let zig = poly(&[(0.0, 0.0), (60.0, 40.0), (0.0, 80.0), (70.0, 90.0), (75.0, 20.0)], false);
    for profile in [None, WidthProfile::preset("lens"), WidthProfile::preset("taperStart")] {
        for align in [ArrowAlign::Tip, ArrowAlign::Extend] {
            for mode in [StrokeGradientMode::Along, StrokeGradientMode::Across] {
                let mut st = stroke(12.0, mode);
                st.start_arrow = Some(Arrowhead::Triangle);
                st.end_arrow = Some(Arrowhead::Triangle);
                st.arrow_align = align;
                st.profile = profile.clone();
                assert_covered(&zig, &st);
            }
        }
    }
}

/// Does the last slice holding `p` (the one filled last there) lay its gradient piece on a part of
/// the path running across the page (left or right), for a gradient in `mode`? Along, the piece
/// runs the way the path does; across, square to it.
fn painted_by_level_part(slices: &[Slice], p: Point, mode: StrokeGradientMode) -> Option<bool> {
    slices.iter().rev().find(|s| s.shape.contains(p)).map(|s| {
        let d = s.to - s.from;
        (d.x.abs() > d.y.abs()) == (mode == StrokeGradientMode::Along)
    })
}

/// #5960: where a variable-width stroke crosses itself, the later pass paints over the earlier one
/// only where the two overlap. Its slices reach as far as the stroke is wide there, not as far as
/// it is wide anywhere, so a narrow later pass leaves the wide earlier one its own colour beside it.
#[test]
fn a_narrow_pass_over_a_wide_one_paints_only_where_they_overlap() {
    // Right along y = 50, round, then down through the first leg at x = 50, near the narrow end.
    let crossing = poly(&[(0.0, 50.0), (100.0, 50.0), (100.0, 100.0), (50.0, 100.0), (50.0, 0.0)], false);
    for mode in [StrokeGradientMode::Along, StrokeGradientMode::Across] {
        let mut st = stroke(10.0, mode);
        st.profile = Some(WidthProfile::taper_end());
        let sl = slices(&crossing, &st);
        // In the first leg (about 4 pt either side of it there) but 4 pt beside the second leg,
        // which is less than 1 pt either side of itself there.
        let level = painted_by_level_part(&sl, Point::new(54.0, 52.0), mode);
        assert_eq!(level, Some(true), "{mode:?}: painted by the second leg");
        // Where the two overlap, the second leg is on top.
        let level = painted_by_level_part(&sl, Point::new(50.5, 52.0), mode);
        assert_eq!(level, Some(false), "{mode:?}: painted by the first leg");
        assert_covered(&crossing, &st);
    }
}

/// #5960: a narrow pass running close beside a wide one doesn't paint the wide one's edge.
#[test]
fn a_narrow_pass_beside_a_wide_one_leaves_its_edge_alone() {
    // Right along y = 0, then back along y = 7: the way back is the narrow end of a taper.
    let hairpin = poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 7.0), (0.0, 7.0)], false);
    let total = 207.0;
    let mut st = stroke(10.0, StrokeGradientMode::Along);
    st.profile = Some(WidthProfile::taper_end());
    let sl = slices(&hairpin, &st);
    // Half way along the first leg, 3 pt off it (it is about 3.8 pt wide there either side); the
    // way back is about 1.2 pt wide either side there, so it reaches no nearer than 5.8.
    let p = Point::new(50.0, 3.0);
    let t = param(&sl, p).expect("in a slice");
    assert!((t - 50.0 / total).abs() < 1e-6, "painted with {t}, not the first leg's {}", 50.0 / total);
    assert_covered(&hairpin, &st);
}

/// #5960: with dashes, the ribs reach as far as the stroke gets wide within a cap's length along
/// the path, however many points lie that far: not as far as it is wide anywhere when more than
/// 256 do.
#[test]
fn a_densely_flattened_dashed_pass_beside_a_wide_one_leaves_its_edge_alone() {
    // The hairpin above, the way back in 0.01 pt steps: 500 points within a round cap's 5 pt.
    let mut pts = vec![(0.0, 0.0), (100.0, 0.0)];
    pts.extend((0..=10_000).map(|i| (100.0 - i as f64 * 0.01, 7.0)));
    let hairpin = poly(&pts, false);
    let total = 207.0;
    let mut st = stroke(10.0, StrokeGradientMode::Along);
    st.profile = Some(WidthProfile::taper_end());
    st.cap = LineCap::Round;
    st.dash = Some(Dash { pattern: vec![12.0, 6.0], ..Default::default() });
    let sl = slices(&hairpin, &st);
    let p = Point::new(50.0, 3.0);
    let t = param(&sl, p).expect("in a slice");
    assert!((t - 50.0 / total).abs() < 1e-6, "painted with {t}, not the first leg's {}", 50.0 / total);
}

/// The widest the ribs ahead reach is the widest over the points out to the first one more than
/// the window along, as a walk along the subpath finds it, open and closed.
#[test]
fn widest_ahead_is_the_widest_within_the_window() {
    let mut seed = 0x9e37_79b9_u64;
    let mut rnd = move || {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    for _ in 0..200 {
        let n = 1 + (rnd() * 40.0) as usize;
        let cover: Vec<f64> = (0..n).map(|_| rnd() * 10.0).collect();
        // Some points on top of each other.
        let seg: Vec<f64> = (0..n).map(|_| if rnd() < 0.2 { 0.0 } else { rnd() * 3.0 }).collect();
        let window = [0.0, rnd() * 5.0, rnd() * 200.0][(rnd() * 3.0) as usize];
        for closed in [false, true] {
            let got = super::gradient::widest_ahead(&cover, &seg, closed, window);
            assert_eq!(got.len(), n);
            for (i, &got) in got.iter().enumerate() {
                let (mut j, mut d, mut want) = (i, 0.0, 0.0_f64);
                for _ in 0..n {
                    let k = if closed {
                        (j + 1) % n
                    } else if j + 1 < n {
                        j + 1
                    } else {
                        break;
                    };
                    want = want.max(cover[k]);
                    d += seg[j];
                    j = k;
                    if d > window {
                        break;
                    }
                }
                assert_eq!(got, want, "n {n}, closed {closed}, window {window}, point {i}");
            }
        }
    }
}
