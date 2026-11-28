use super::*;
use crate::stroke::outline as raster_stroke_outline;
use vectorcraft_doc::{LineCap, LineJoin, StrokeLayer};

/// How many lines a fill of `path` takes once flattened at `tol`.
fn lines_at(path: &BezPath, tol: f64) -> usize {
    let mut n = 0;
    vectorcraft_geom::flatten(path.iter(), tol, |el| n += usize::from(matches!(el, kurbo::PathEl::LineTo(_) | kurbo::PathEl::ClosePath)));
    n
}

/// Whether `path` holds lines only.
fn lines_only(path: &BezPath) -> bool {
    path.elements().iter().all(|el| !matches!(el, kurbo::PathEl::QuadTo(..) | kurbo::PathEl::CurveTo(..)))
}

/// A thin curved stroke's outline takes at most twice the lines the stroke's plain outline takes
/// at the raster tolerance (#7257). 200 cubic curves, each about 100 device pixels across.
#[test]
fn thin_curved_strokes_take_at_most_twice_the_lines_of_their_plain_outline() {
    let mut seed: u64 = 7;
    let mut r = |lo: f64, hi: f64| {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        lo + (hi - lo) * ((seed >> 11) as f64 / (1u64 << 53) as f64)
    };
    let mut paths = Vec::new();
    for _ in 0..200 {
        let (x, y) = (r(20.0, 980.0), r(20.0, 980.0));
        let mut q = [kurbo::Point::ZERO; 3];
        for p in &mut q {
            *p = kurbo::Point::new((x + r(-30.0, 30.0)) * 1.71, (y + r(-30.0, 30.0)) * 1.71);
        }
        let mut path = BezPath::new();
        path.move_to((x * 1.71, y * 1.71));
        path.curve_to(q[0], q[1], q[2]);
        paths.push(path);
    }
    let tol = 0.25;
    for width in [0.5, 0.75, 0.99] {
        let st = StrokeLayer::new(Paint::solid(Color::BLACK), width);
        let (mut plain, mut thin) = (0, 0);
        for path in &paths {
            plain += lines_at(&crate::effects::stroke::line_outline(path, &st, width, tol), tol);
            let outline = raster_stroke_outline(path, &st, width, tol);
            assert!(lines_only(&outline), "width {width}: lines only");
            thin += lines_at(&outline, tol);
        }
        assert!(thin <= plain * 2, "width {width}: {thin} lines, plain outline {plain}");
    }
}

/// A thin stroke drawn as lines keeps its caps, its joins and the hole of a closed path.
#[test]
fn thin_strokes_drawn_as_lines_keep_caps_joins_and_holes() {
    let (w, tol) = (0.5, 0.25);
    // A quarter circle from (100, 100) heading right, then a right-angle corner at (200, 200).
    let mut open = BezPath::new();
    open.move_to((100.0, 100.0));
    open.curve_to((155.23, 100.0), (200.0, 144.77), (200.0, 200.0));
    open.line_to((100.0, 200.0));
    for cap in [LineCap::Butt, LineCap::Round, LineCap::Square] {
        for join in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel] {
            let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), w);
            st.cap = cap;
            st.join = join;
            let o = raster_stroke_outline(&open, &st, w, tol);
            assert!(lines_only(&o), "{cap:?} {join:?}: lines only");
            let inside = |x: f64, y: f64| o.winding(kurbo::Point::new(x, y)) != 0;
            assert_eq!(inside(100.0 - 0.4 * w, 100.0), cap != LineCap::Butt, "{cap:?} {join:?}: start cap");
            assert!(!inside(100.0 - 0.6 * w, 100.0), "{cap:?} {join:?}: past the start cap");
            assert_eq!(inside(100.0 - 0.45 * w, 100.0 + 0.45 * w), cap == LineCap::Square, "{cap:?} {join:?}: cap corner");
            assert!(
                inside(200.0 + 0.4 * w, 199.0) && inside(200.0 - 0.4 * w, 199.0) && !inside(200.0 + 0.6 * w, 199.0),
                "{cap:?} {join:?}: curve end"
            );
            assert_eq!(inside(200.0 + 0.3 * w, 200.0 + 0.3 * w), join != LineJoin::Bevel, "{cap:?} {join:?}: corner");
            assert_eq!(inside(200.0 + 0.45 * w, 200.0 + 0.45 * w), join == LineJoin::Miter, "{cap:?} {join:?}: miter tip");
            assert!(inside(150.0, 200.0 + 0.4 * w) && !inside(150.0, 200.0 + 0.6 * w), "{cap:?} {join:?}: line");
        }
    }
    let ring = kurbo::Circle::new((300.0, 300.0), 50.0).to_path(0.01);
    let st = StrokeLayer::new(Paint::solid(Color::BLACK), w);
    let o = raster_stroke_outline(&ring, &st, w, tol);
    assert!(lines_only(&o), "closed: lines only");
    for k in 0..36 {
        let a = f64::from(k) * 10f64.to_radians();
        let at = |r: f64| o.winding(kurbo::Point::new(300.0 + r * a.cos(), 300.0 + r * a.sin()));
        assert!(at(50.0 - 0.4 * w) != 0 && at(50.0 + 0.4 * w) != 0, "ring band at {k}");
        assert!(at(50.0 - 0.6 * w) == 0 && at(50.0 + 0.6 * w) == 0, "ring edges at {k}");
    }
    assert_eq!(o.winding(kurbo::Point::new(300.0, 300.0)), 0, "the hole");
}

/// Independently integrate coverage in radial/angle bins of an original notched ring. Checking
/// only the outline geometry misses raster flattening that cancels its two nearby contours.
#[test]
fn fractional_curved_strokes_retain_coverage_between_notches() {
    for rounded in [false, true] {
        for (weight, rotation, phase, opacity) in
            [(1.0 / 3.0, 0.0, 0.0, 1.0), (0.5, 30.0, 0.25, 1.0), (0.75, 75.0, 0.5, 0.5), (1.0, 135.0, 0.75, 1.0)]
        {
            let mut d = Document::new(1000.0, 1000.0);
            let mut path = BezPath::new();
            let mut radius = 450.0;
            let mut previous = 0.0;
            path.move_to((950.0, 500.0));
            let mut arcs = Vec::new();
            let point = |r: f64, a: f64| vectorcraft_geom::Point::new(500.0 + r * a.cos(), 500.0 + r * a.sin());
            for degrees in [17.0_f64, 64.0, 119.0, 147.0, 201.0, 242.0, 313.0, 360.0] {
                let a = degrees.to_radians();
                let k = 4.0 / 3.0 * ((a - previous) / 4.0).tan();
                let start = point(radius, previous);
                let end = point(radius, a);
                path.curve_to(
                    start + kurbo::Vec2::new(-previous.sin(), previous.cos()) * (k * radius),
                    end + kurbo::Vec2::new(a.sin(), -a.cos()) * (k * radius),
                    end,
                );
                arcs.push((radius, previous + 0.06, a - 0.06));
                radius = if radius == 450.0 { 438.0 } else { 450.0 };
                path.line_to(point(radius, a));
                previous = a;
            }
            path.close_path();
            // Match a parser's f32 coordinate round-trip and exact closed endpoint as well as the
            // original f64 construction. A near-coincident final anchor must not hide this case.
            let q = |p: kurbo::Point| kurbo::Point::new(f64::from(p.x as f32), f64::from(p.y as f32));
            let path = if rounded {
                BezPath::from_vec(
                    path.iter()
                        .map(|el| match el {
                            kurbo::PathEl::MoveTo(p) => kurbo::PathEl::MoveTo(q(p)),
                            kurbo::PathEl::LineTo(p) => kurbo::PathEl::LineTo(q(p)),
                            kurbo::PathEl::QuadTo(a, b) => kurbo::PathEl::QuadTo(q(a), q(b)),
                            kurbo::PathEl::CurveTo(a, b, c) => kurbo::PathEl::CurveTo(q(a), q(b), q(c)),
                            kurbo::PathEl::ClosePath => kurbo::PathEl::ClosePath,
                        })
                        .collect(),
                )
            } else {
                path
            };
            {
                use kurbo::{ParamCurve, ParamCurveDeriv};
                let st = StrokeLayer::new(Paint::solid(Color::BLACK), weight);
                let outline = raster_stroke_outline(&path, &st, weight.max(0.5), 0.25);
                for segment in path.segments() {
                    if let kurbo::PathSeg::Cubic(c) = segment {
                        for step in 1..20 {
                            let t = step as f64 / 20.0;
                            let p = c.eval(t);
                            let v = c.deriv().eval(t).to_vec2().normalize();
                            let n = kurbo::Vec2::new(-v.y, v.x);
                            for side in [-1.0, 1.0] {
                                assert_ne!(outline.winding(p + n * (side * weight.max(0.5) * 0.4)), 0, "missing geometric band, rounded={rounded}");
                                assert_eq!(outline.winding(p + n * (side * weight.max(0.5) * 0.6)), 0, "outside geometric band, rounded={rounded}");
                            }
                        }
                    }
                }
            }
            let mut node = Node::path(
                d.alloc_id(),
                vectorcraft_geom::PathData::from_bezpath(&path),
                Appearance::basic(Paint::None, Paint::solid(Color::BLACK), weight),
            );
            node.opacity = opacity;
            d.insert(Some(d.layers[0].id), 0, node).unwrap();
            let view =
                Affine::translate((500.0 + phase, 500.0 + phase)) * Affine::rotate(f64::to_radians(rotation)) * Affine::translate((-500.0, -500.0));
            let image =
                Renderer::new().render(&d, 1000, 1000, view, &RenderOptions { background: Some([255; 4]), artboards: false, ..Default::default() });
            // Collect only the measured radial bands once, in the original row-major order.
            // Each bin then adds the same pixels in the same order without rescanning the image.
            let mut ring = Vec::new();
            for y in 0..1000 {
                for x in 0..1000 {
                    let dx = x as f64 + 0.5 - 500.0 - phase;
                    let dy = y as f64 + 0.5 - 500.0 - phase;
                    let distance = dx.hypot(dy);
                    if !arcs.iter().any(|(r, _, _)| (distance - r).abs() <= 3.0) {
                        continue;
                    }
                    let angle = (dy.atan2(dx) - f64::to_radians(rotation)).rem_euclid(std::f64::consts::TAU);
                    ring.push((distance, angle, f64::from(255 - image.pixel(x, y)[0]) / 255.0));
                }
            }
            for (r, begin, end) in arcs {
                // 3 bins per arc avoid averaging away a washed-out section. The expected ink area
                // is independently r * angle * displayed weight, with the existing half-pixel floor.
                for bin in 0..3 {
                    let lo = begin + (end - begin) * bin as f64 / 3.0;
                    let hi = begin + (end - begin) * (bin + 1) as f64 / 3.0;
                    let mut ink = 0.0;
                    for &(distance, angle, value) in &ring {
                        if (distance - r).abs() > 3.0 {
                            continue;
                        }
                        if angle >= lo && angle < hi {
                            ink += value;
                        }
                    }
                    let expected = r * (hi - lo) * weight.max(0.5) * f64::from(opacity);
                    assert!(
                        (ink / expected - 1.0).abs() < 0.15,
                        "weight {weight}, rotation {rotation}, phase {phase}, opacity {opacity}, arc {r} {lo}..{hi}: coverage {}",
                        ink / expected
                    );
                }
            }
        }
    }
}
