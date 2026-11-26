use super::*;
use crate::stroke::outline as raster_stroke_outline;
use vectorcraft_doc::StrokeLayer;

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
