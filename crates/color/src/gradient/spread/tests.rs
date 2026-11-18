use kurbo::Vec2;

use super::*;
use crate::gradient::{GradientPaint, MAX_REPEATS};
use crate::{Color, GradientMethod};

fn spread(kind: GradientKind, spread: GradientSpread, repeats: u32) -> Gradient {
    Gradient { spread, repeats, ..Gradient::new(kind, Gradient::default().stops) }
}

fn black_white() -> Vec<GradientStop> {
    let mut a = GradientStop::new(0.0, Color::BLACK);
    a.midpoint = 0.3;
    vec![a, GradientStop::new(1.0, Color::WHITE)]
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// Repeat wraps the parameter, `repeats` times along the vector and on past both ends; Reflect
/// runs every other run backwards; Pad leaves it to the stops to clamp; a freeform gradient has
/// no ends to spread past.
#[test]
fn spread_maps_the_parameter_to_a_place_along_the_stops() {
    let repeat = spread(GradientKind::Linear, GradientSpread::Repeat, 3);
    for (t, want) in [(0.0, 0.0), (0.1, 0.3), (0.5, 0.5), (1.0 / 3.0 + 0.1, 0.3), (1.1, 0.3), (-0.1, 0.7)] {
        assert!(close(repeat.spread_param(t), want), "repeat {t}: {}", repeat.spread_param(t));
    }
    let reflect = spread(GradientKind::Radial, GradientSpread::Reflect, 3);
    for (t, want) in [(0.0, 0.0), (0.1, 0.3), (1.0 / 3.0 + 0.1, 0.7), (2.0 / 3.0 + 0.1, 0.3), (1.0, 1.0), (-0.1, 0.3)] {
        assert!(close(reflect.spread_param(t), want), "reflect {t}: {}", reflect.spread_param(t));
    }
    let pad = spread(GradientKind::Linear, GradientSpread::Pad, 3);
    assert_eq!((pad.spread_param(1.7), pad.spread_param(-0.2)), (1.7, -0.2));
    let free = spread(GradientKind::Freeform, GradientSpread::Repeat, 3);
    assert_eq!(free.spread_param(0.7), 0.7);
    assert!(!free.is_spread() && !pad.is_spread() && repeat.is_spread());
    assert!(repeat.spread_param(f64::NAN).is_nan());
    // A count out of range (set in code) counts as the nearest one in range.
    assert_eq!(spread(GradientKind::Linear, GradientSpread::Repeat, 0).repeat_count(), 1);
    assert_eq!(spread(GradientKind::Linear, GradientSpread::Repeat, 5000).repeat_count(), MAX_REPEATS);
}

/// The native format writes the spread and count only when they are not Pad and 1, reads a file
/// without them as Pad and 1, and reads an unknown spread or a count out of range as the nearest
/// one it can hold.
#[test]
fn the_file_keeps_the_spread_and_reads_bad_values_leniently() {
    let plain = serde_json::to_value(Gradient::default()).unwrap();
    assert!(plain.get("spread").is_none() && plain.get("repeats").is_none(), "{plain}");
    let g = spread(GradientKind::Conic, GradientSpread::Reflect, 4);
    let v = serde_json::to_value(&g).unwrap();
    assert_eq!((v["spread"].as_str(), v["repeats"].as_u64()), (Some("Reflect"), Some(4)));
    assert_eq!(serde_json::from_value::<Gradient>(v).unwrap(), g);
    let read = |spread: serde_json::Value, repeats: serde_json::Value| {
        let mut v = serde_json::to_value(Gradient::default()).unwrap();
        v["spread"] = spread;
        v["repeats"] = repeats;
        let g: Gradient = serde_json::from_value(v).unwrap();
        (g.spread, g.repeats)
    };
    assert_eq!(read("Repeat".into(), 3.4.into()), (GradientSpread::Repeat, 3));
    assert_eq!(read("Spiral".into(), (-5).into()), (GradientSpread::Pad, 1));
    assert_eq!(read(7.into(), 1e12.into()), (GradientSpread::Pad, MAX_REPEATS));
    assert_eq!(read(serde_json::Value::Null, "many".into()), (GradientSpread::Pad, 1));
    let missing: Gradient = serde_json::from_value(serde_json::json!({"kind": "Linear", "stops": []})).unwrap();
    assert_eq!((missing.spread, missing.repeats), (GradientSpread::Pad, 1));
}

/// One run's placement: a level of the period is the gradient's level times the count, for
/// linear, diamond and off-center radial gradients; a conic keeps its placement.
#[test]
fn a_period_holds_one_run_of_the_stops() {
    let p = Point::new(72.0, 61.0);
    let linear = GradientGeom { start: Point::new(10.0, 20.0), end: Point::new(60.0, 40.0), aspect: 1.0, focal: None };
    let radial = GradientGeom { start: Point::new(50.0, 50.0), end: Point::new(60.0, 50.0), aspect: 0.6, focal: Some(Point::new(55.0, 52.0)) };
    let diamond = GradientGeom { end: Point::new(58.0, 56.0), focal: None, ..radial };
    for (kind, geom) in [(GradientKind::Linear, linear), (GradientKind::Radial, radial), (GradientKind::Diamond, diamond)] {
        let period = geom.period(kind, 4);
        assert!(close(period.param_at(kind, p), 4.0 * geom.param_at(kind, p)), "{kind:?}");
        assert_eq!(period.focal, geom.focal, "{kind:?}");
    }
    assert_eq!(radial.period(GradientKind::Conic, 4), radial);
    assert_eq!(linear.period(GradientKind::Linear, 0), linear, "a count of 0 counts as 1");
}

/// The stops repeated over the area: as many runs as its corners reach, each in place.
#[test]
fn a_linear_gradient_pads_with_its_stops_repeated_over_the_area() {
    let geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(20.0, 0.0), aspect: 1.0, focal: None };
    let g = Gradient { spread: GradientSpread::Repeat, repeats: 2, ..Gradient::new(GradientKind::Linear, black_white()) };
    let p = g.padded_over(&geom, Rect::new(-5.0, 0.0, 25.0, 10.0)).unwrap();
    assert!(!p.capped);
    // Runs of 10 from -10 to 30: four of them.
    assert_eq!((p.geom.start, p.geom.end), (Point::new(-10.0, 0.0), Point::new(30.0, 0.0)));
    let offsets: Vec<f32> = p.gradient.stops.iter().map(|s| s.offset).collect();
    assert_eq!(offsets, [0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0]);
    assert!(p.gradient.stops.iter().step_by(2).all(|s| s.color == Color::BLACK && (s.midpoint - 0.3).abs() < 1e-6));
    assert_eq!((p.gradient.spread, p.gradient.repeats), (GradientSpread::Pad, 1));
    // Every point gets the color the gradient gives it.
    for x in [-4.0, 1.0, 7.5, 13.0, 24.0] {
        let q = Point::new(x, 5.0);
        let want = g.sample(g.spread_param(geom.param_at(GradientKind::Linear, q)) as f32).0;
        let got = p.gradient.sample(p.geom.param_at(GradientKind::Linear, q) as f32).0;
        assert!(want.to_rgb().iter().zip(got.to_rgb()).all(|(a, b)| (a - b).abs() < 1e-4), "{x}: {want:?} vs {got:?}");
    }
}

#[test]
fn reflect_mirrors_every_other_run_and_its_midpoints() {
    let geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(10.0, 0.0), aspect: 1.0, focal: None };
    let g = Gradient { spread: GradientSpread::Reflect, ..Gradient::new(GradientKind::Linear, black_white()) };
    let p = g.padded_over(&geom, Rect::new(0.0, 0.0, 20.0, 10.0)).unwrap();
    let colors: Vec<Color> = p.gradient.stops.iter().map(|s| s.color).collect();
    assert_eq!(colors, [Color::BLACK, Color::WHITE, Color::WHITE, Color::BLACK]);
    // Black → white with the midpoint at 0.3 mirrors to white → black with it at 0.7.
    assert!((p.gradient.stops[2].midpoint - 0.7).abs() < 1e-6, "{:?}", p.gradient.stops);
}

/// A gradient mixed around the hue wheel that reflects: a mirrored run turns the wheel back the
/// way it came, so every point keeps its color when the runs are written out as a gradient that
/// pads, unrolled along the vector, or as one cycle, with a repeating one's too.
#[test]
fn reflected_and_repeated_hue_turns_keep_every_color() {
    let geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(10.0, 0.0), aspect: 1.0, focal: None };
    let stops = || {
        let mut red = GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0));
        red.midpoint = 0.3;
        vec![red, GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))]
    };
    let near = |a: Color, b: Color, what: &str| {
        assert!(a.to_rgb().iter().zip(b.to_rgb()).all(|(x, y)| (x - y).abs() < 1.0 / 255.0), "{what}: {a:?} vs {b:?}");
    };
    for method in [GradientMethod::HueClockwise, GradientMethod::HueCounterclockwise] {
        for spread in [GradientSpread::Reflect, GradientSpread::Repeat] {
            let g = Gradient { method, spread, repeats: 2, ..Gradient::new(GradientKind::Linear, stops()) };
            let p = g.padded_over(&geom, Rect::new(-10.0, 0.0, 20.0, 10.0)).unwrap();
            assert_eq!((p.gradient.spread, p.gradient.repeats), (GradientSpread::Pad, 1));
            for k in 0..300 {
                let q = Point::new(-10.0 + f64::from(k) * 0.1 + 0.013, 5.0);
                let want = g.sample(g.spread_param(geom.param_at(GradientKind::Linear, q)) as f32).0;
                let got = p.gradient.sample(p.geom.param_at(GradientKind::Linear, q) as f32).0;
                near(want, got, &format!("{method:?} {spread:?} padded at {}", q.x));
            }
            let unrolled = g.unrolled();
            let (cycle, runs) = g.cycle().unwrap();
            for k in 0..=200 {
                let t = (k as f32 + 0.37) / 201.0;
                let want = g.sample(g.spread_param(f64::from(t)) as f32).0;
                near(want, unrolled.sample(t).0, &format!("{method:?} {spread:?} unrolled at {t}"));
                // One cycle covers `runs` of the two runs between 0 and 1.
                let along = t * runs as f32 / 2.0;
                let want = g.sample(g.spread_param(f64::from(along)) as f32).0;
                near(want, cycle.sample(t).0, &format!("{method:?} {spread:?} cycle at {t}"));
            }
        }
    }
}

#[test]
fn stops_inside_a_run_hold_their_colors_to_its_ends() {
    let geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(10.0, 0.0), aspect: 1.0, focal: None };
    let stops = vec![GradientStop::new(0.25, Color::BLACK), GradientStop::new(0.75, Color::WHITE)];
    let g = Gradient { spread: GradientSpread::Repeat, ..Gradient::new(GradientKind::Linear, stops) };
    let p = g.padded_over(&geom, Rect::new(0.0, 0.0, 20.0, 10.0)).unwrap();
    let offsets: Vec<f32> = p.gradient.stops.iter().map(|s| s.offset).collect();
    assert_eq!(offsets, [0.0, 0.125, 0.375, 0.5, 0.5, 0.625, 0.875, 1.0]);
}

/// An off-center radial grows about its focal point, a diamond about its center; every point
/// keeps its color.
#[test]
fn radial_and_diamond_gradients_pad_in_rings_from_their_first_level() {
    let c = Point::new(50.0, 50.0);
    let radial = GradientGeom { start: c, end: Point::new(60.0, 50.0), aspect: 1.0, focal: Some(Point::new(55.0, 50.0)) };
    let diamond = GradientGeom { focal: None, ..radial };
    for (kind, geom) in [(GradientKind::Radial, radial), (GradientKind::Diamond, diamond)] {
        let g = Gradient { spread: GradientSpread::Reflect, repeats: 2, ..Gradient::new(kind, black_white()) };
        let p = g.padded_over(&geom, Rect::new(0.0, 0.0, 100.0, 100.0)).unwrap();
        assert!(p.gradient.stops.len() / 2 >= 10, "{kind:?}: {}", p.gradient.stops.len());
        assert_eq!(p.geom.focal, geom.focal);
        for q in [Point::new(72.0, 61.0), Point::new(10.0, 95.0), Point::new(52.0, 49.0)] {
            let want = g.sample(g.spread_param(geom.param_at(kind, q)) as f32).0.to_rgb();
            let got = p.gradient.sample(p.geom.param_at(kind, q) as f32).0.to_rgb();
            assert!(want.iter().zip(got).all(|(a, b)| (a - b).abs() < 1e-4), "{kind:?} {q:?}: {want:?} vs {got:?}");
        }
    }
}

/// A run of the stops far too short for the area is written with at most so many runs, and pads
/// past them; a gradient of many stops is written with every run the area needs, each run
/// without the stops that change its colors least, within so many stops; a conic, a padding
/// gradient and one without stops have nothing to pad.
#[test]
fn far_repeats_are_capped_and_many_stops_are_sampled() {
    let geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(0.01, 0.0), aspect: 1.0, focal: None };
    let g = Gradient { spread: GradientSpread::Repeat, repeats: MAX_REPEATS, ..Gradient::new(GradientKind::Linear, black_white()) };
    let p = g.padded_over(&geom, Rect::new(0.0, 0.0, 100.0, 1.0)).unwrap();
    assert!(p.capped);
    assert_eq!(p.gradient.stops.len(), 2 * MAX_SPREAD_RUNS);
    let many: Vec<GradientStop> = (0..=4000).map(|i| GradientStop::new(i as f32 / 4000.0, Color::gray(i as f32 / 4000.0))).collect();
    let g = Gradient { spread: GradientSpread::Reflect, repeats: 50, ..Gradient::new(GradientKind::Linear, many) };
    let wide = GradientGeom { end: Point::new(100.0, 0.0), ..geom };
    let p = g.padded_over(&wide, Rect::new(0.0, 0.0, 100.0, 1.0)).unwrap();
    assert!(!p.capped && p.runs == 50 && p.gradient.stops.len() <= MAX_SPREAD_STOPS, "{} {}", p.runs, p.gradient.stops.len());
    for x in [0.7, 33.3, 66.1, 99.5] {
        let q = Point::new(x, 0.5);
        let want = g.sample(g.spread_param(wide.param_at(GradientKind::Linear, q)) as f32).0.to_rgb();
        let got = p.gradient.sample(p.geom.param_at(GradientKind::Linear, q) as f32).0.to_rgb();
        assert!(want.iter().zip(got).all(|(a, b)| (a - b).abs() < 2e-3), "{x}: {want:?} vs {got:?}");
    }
    assert!(g.unrolled().stops.len() <= MAX_SPREAD_STOPS);
    assert!(p.gradient.padded_over(&p.geom, Rect::new(0.0, 0.0, 1.0, 1.0)).is_none(), "it pads");
    assert!(spread(GradientKind::Conic, GradientSpread::Repeat, 3).padded_over(&wide, Rect::new(0.0, 0.0, 1.0, 1.0)).is_none());
    let empty = Gradient { spread: GradientSpread::Repeat, ..Gradient::new(GradientKind::Linear, vec![]) };
    assert!(empty.padded_over(&wide, Rect::new(0.0, 0.0, 1.0, 1.0)).is_none());
}

/// The runs written are the ones the area shows: a radial or diamond centered a thousand runs
/// from the area writes the hundred it crosses, not the first 256 from the center; a linear
/// gradient over an area of 1,100 runs far before its start, capped, keeps the runs nearest it.
/// Every point checked gets the color the gradient gives it.
#[test]
fn the_runs_written_cover_a_distant_area() {
    let centered = GradientGeom { start: Point::new(0.0, 50.0), end: Point::new(1.0, 50.0), aspect: 1.0, focal: None };
    let before = GradientGeom { start: Point::new(5000.0, 0.0), end: Point::new(5001.0, 0.0), aspect: 1.0, focal: None };
    for (kind, geom, area) in [
        (GradientKind::Radial, centered, Rect::new(1000.0, 0.0, 1100.0, 100.0)),
        (GradientKind::Diamond, centered, Rect::new(1000.0, 0.0, 1100.0, 100.0)),
        (GradientKind::Linear, before, Rect::new(0.0, 0.0, 1100.0, 100.0)),
    ] {
        let g = Gradient { spread: GradientSpread::Reflect, repeats: 1, ..Gradient::new(kind, black_white()) };
        let p = g.padded_over(&geom, area).unwrap();
        assert_eq!(p.capped, kind == GradientKind::Linear, "{kind:?}");
        assert!(p.gradient.stops.len() <= MAX_SPREAD_STOPS);
        for q in [Point::new(1000.5, 0.5), Point::new(1050.25, 50.0), Point::new(1099.5, 99.5), Point::new(1010.75, 90.0)] {
            let want = g.sample(g.spread_param(geom.param_at(kind, q)) as f32).0.to_rgb();
            let got = p.gradient.sample(p.geom.param_at(kind, q) as f32).0.to_rgb();
            assert!(want.iter().zip(got).all(|(a, b)| (a - b).abs() < 2e-3), "{kind:?} {q:?}: {want:?} vs {got:?}");
        }
    }
}

/// A tiny radial centered just outside the area, beside a side between any evenly spaced
/// samples of it: the runs written start from the area's true innermost level.
#[test]
fn the_innermost_level_is_found_between_samples() {
    let c = Point::new(-0.0005, 7.8125);
    let geom = GradientGeom { start: c, end: c + Vec2::new(0.001, 0.0), aspect: 1.0, focal: None };
    let g = Gradient { spread: GradientSpread::Reflect, repeats: 1, ..Gradient::new(GradientKind::Radial, black_white()) };
    let p = g.padded_over(&geom, Rect::new(0.0, 0.0, 1000.0, 1000.0)).unwrap();
    for q in [Point::new(0.05, 7.8125), Point::new(0.1, 7.9)] {
        let want = g.sample(g.spread_param(geom.param_at(GradientKind::Radial, q)) as f32).0.to_rgb();
        let got = p.gradient.sample(p.geom.param_at(GradientKind::Radial, q) as f32).0.to_rgb();
        assert!(want.iter().zip(got).all(|(a, b)| (a - b).abs() < 2e-3), "{q:?}: {want:?} vs {got:?}");
    }
}

/// An area four billion units wide whose long side passes 50 runs from a tiny radial or diamond's
/// center: the runs written start from that side's true innermost level, so the runs beside it
/// keep their colors.
#[test]
fn the_innermost_level_is_found_along_a_very_long_side() {
    let area = Rect::new(-1.3e9, 0.0, 2.9e9, 1.0);
    for x in [0.3, 12345.678, -7777.25, 98765.4321] {
        let c = Point::new(x, -0.00005);
        let run = GradientGeom { start: c, end: c + Vec2::new(1e-6, 0.0), aspect: 1.0, focal: None };
        let off_center = GradientGeom { focal: Some(c + Vec2::new(0.5e-6, 0.0)), ..run };
        for (kind, geom) in [(GradientKind::Diamond, run), (GradientKind::Radial, run), (GradientKind::Radial, off_center)] {
            let g = Gradient { spread: GradientSpread::Reflect, repeats: 1, ..Gradient::new(kind, black_white()) };
            let p = g.padded_over(&geom, area).unwrap();
            assert!(p.capped, "{kind:?}");
            for q in [Point::new(x, 1e-5), Point::new(x + 2e-5, 2e-5), Point::new(x - 1e-5, 0.0001)] {
                let level = geom.param_at(kind, q);
                assert!((40.0..200.0).contains(&level), "{kind:?} {q:?}: level {level}");
                let want = g.sample(g.spread_param(level) as f32).0.to_rgb();
                let got = p.gradient.sample(p.geom.param_at(kind, q) as f32).0.to_rgb();
                assert!(want.iter().zip(got).all(|(a, b)| (a - b).abs() < 2e-3), "{kind:?} {q:?}: {want:?} vs {got:?}");
            }
        }
    }
}

/// Runs a billion levels from a radial's center are past what the stops' offsets can tell apart:
/// the gradient is written capped, its stops still apart and in order.
#[test]
fn runs_too_far_from_the_center_are_capped() {
    let c = Point::new(0.0, 0.0);
    let geom = GradientGeom { start: c, end: c + Vec2::new(1e-6, 0.0), aspect: 1.0, focal: None };
    let g = Gradient { spread: GradientSpread::Reflect, repeats: 1, ..Gradient::new(GradientKind::Radial, black_white()) };
    let p = g.padded_over(&geom, Rect::new(1000.0, 0.0, 1000.0001, 0.0001)).unwrap();
    assert!(p.capped);
    assert!(p.gradient.stops.windows(2).all(|w| w[0].offset <= w[1].offset), "in order");
    let distinct = p.gradient.stops.windows(2).filter(|w| w[1].offset > w[0].offset).count();
    assert!(distinct * 2 + 2 >= p.gradient.stops.len(), "the runs stay apart: {distinct} of {}", p.gradient.stops.len());
}

/// A run of 1,000 close stops is written only as far out as their offsets stay apart: around
/// level 50,000 it is capped, every stop written apart from the next.
#[test]
fn close_stops_are_written_only_where_they_stay_apart() {
    let many: Vec<GradientStop> = (0..=1000).map(|i| GradientStop::new(i as f32 / 1000.0, Color::gray(i as f32 / 1000.0))).collect();
    let geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(0.001, 0.0), aspect: 1.0, focal: None };
    let g = Gradient { spread: GradientSpread::Repeat, repeats: 1, ..Gradient::new(GradientKind::Diamond, many) };
    let p = g.padded_over(&geom, Rect::new(50.0, 0.0, 50.002, 0.002)).unwrap();
    assert!(p.capped);
    // Only where one run meets the next do two stops share an offset.
    let runs = p.gradient.stops.len() / 1001;
    let shared = p.gradient.stops.windows(2).filter(|w| w[1].offset <= w[0].offset).count();
    assert!(shared < runs, "{shared} stops share an offset in {runs} runs");
    // Stops 0.0001234 of a run apart are written out to level 123, on whole runs: an area within
    // it gets the gradient's colors, Reflect's mirrored runs included.
    let stops = vec![GradientStop::new(0.0, Color::BLACK), GradientStop::new(0.0001234, Color::gray(0.5)), GradientStop::new(1.0, Color::WHITE)];
    let g = Gradient { spread: GradientSpread::Reflect, repeats: 1, ..Gradient::new(GradientKind::Diamond, stops) };
    let geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(1.0, 0.0), aspect: 1.0, focal: None };
    let p = g.padded_over(&geom, Rect::new(121.0, 0.0, 123.9, 0.2)).unwrap();
    assert!(p.capped, "past level 123 it pads");
    for x in [121.3, 121.9, 122.5] {
        let q = Point::new(x, 0.1);
        let want = g.sample(g.spread_param(geom.param_at(GradientKind::Diamond, q)) as f32).0.to_rgb();
        let got = p.gradient.sample(p.geom.param_at(GradientKind::Diamond, q) as f32).0.to_rgb();
        assert!(want.iter().zip(got).all(|(a, b)| (a - b).abs() < 2e-3), "{x}: {want:?} vs {got:?}");
    }
}

/// A piece of a gradient that starts where two runs meet (a gradient along a stroke, sliced at a
/// corner there) starts with the new run's first color, not the old run's last.
#[test]
fn a_piece_starting_at_a_sharp_step_takes_the_color_after_it() {
    let g = Gradient { spread: GradientSpread::Repeat, repeats: 2, ..Gradient::new(GradientKind::Linear, Gradient::default().stops) };
    let piece = g.unrolled().span(0.5, 1.0);
    assert_eq!(piece.stops.first().map(|s| s.color), Some(Color::WHITE), "{:?}", piece.stops);
    assert_eq!(piece.stops.last().map(|s| s.color), Some(Color::BLACK));
    let before = g.unrolled().span(0.0, 0.5);
    assert_eq!((before.stops[0].color, before.stops.last().unwrap().color), (Color::WHITE, Color::BLACK));
}

/// An area very far past a very short reflecting vector (a file's numbers) is padded without the
/// run count overflowing.
#[test]
fn a_far_area_pads_without_overflowing() {
    let geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(1e-300, 0.0), aspect: 1.0, focal: None };
    let g = Gradient { spread: GradientSpread::Reflect, repeats: MAX_REPEATS, ..Gradient::new(GradientKind::Linear, black_white()) };
    let p = g.padded_over(&geom, Rect::new(1e3, 0.0, 2e3, 1.0));
    assert!(p.is_none_or(|p| p.gradient.stops.len() <= MAX_SPREAD_STOPS));
    // Runs of 1e-8: the area's levels lie past 1e19, more than an integer holds.
    let geom = GradientGeom { end: Point::new(1e-6, 0.0), ..geom };
    let p = g.padded_over(&geom, Rect::new(1e11, 0.0, 1e11 + 1.0, 1.0)).unwrap();
    assert!(p.capped && p.gradient.stops.len() <= MAX_SPREAD_STOPS, "{} {} {:?}", p.capped, p.gradient.stops.len(), p.geom);
}

/// Unrolled, the stops run `repeats` times between 0 and 1 (every other run mirrored with
/// Reflect); one cycle is a run, or a run and its mirror.
#[test]
fn unrolled_and_cycle_stops() {
    let g = Gradient { spread: GradientSpread::Reflect, repeats: 3, ..Gradient::new(GradientKind::Linear, black_white()) };
    let u = g.unrolled();
    let offsets: Vec<f32> = u.stops.iter().map(|s| (s.offset * 3.0).round()).collect();
    assert_eq!(offsets, [0.0, 1.0, 1.0, 2.0, 2.0, 3.0]);
    let colors: Vec<Color> = u.stops.iter().map(|s| s.color).collect();
    assert_eq!(colors, [Color::BLACK, Color::WHITE, Color::WHITE, Color::BLACK, Color::BLACK, Color::WHITE]);
    for t in [0.05, 0.4, 0.71, 0.99] {
        let (a, b) = (g.sample(g.spread_param(t) as f32).0.to_rgb(), u.sample(t as f32).0.to_rgb());
        assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4), "{t}");
    }
    let (cycle, runs) = g.cycle().unwrap();
    assert_eq!((runs, cycle.stops.len()), (2, 4));
    let repeat = Gradient { spread: GradientSpread::Repeat, ..g.clone() };
    assert_eq!(repeat.cycle().unwrap().1, 1);
    assert!(Gradient::default().cycle().is_none());
    assert!(matches!(Gradient::default().unrolled(), std::borrow::Cow::Borrowed(_)));
}

/// A gradient of more stops than [`MAX_SPREAD_STOPS`] allows for all its runs keeps every run
/// when unrolled, each run without the stops that change its colors least: its last run ends on
/// its last color instead of the rest padding with it.
#[test]
fn unrolled_many_stops_keep_every_run() {
    let stops: Vec<GradientStop> = (0..200).map(|i| GradientStop::new(i as f32 / 199.0, Color::gray(i as f32 / 199.0))).collect();
    let g = Gradient { spread: GradientSpread::Repeat, repeats: MAX_REPEATS, ..Gradient::new(GradientKind::Linear, stops) };
    let u = g.unrolled();
    assert!(u.stops.len() <= MAX_SPREAD_STOPS, "{}", u.stops.len());
    for t in [0.005, 0.5051, 0.9149, 0.9951] {
        let want = g.sample(g.spread_param(t) as f32).0.to_rgb();
        let got = u.sample(t as f32).0.to_rgb();
        assert!(want.iter().zip(got).all(|(a, b)| (a - b).abs() < 2e-3), "{t}: {want:?} vs {got:?}");
    }
    // A padding gradient fitted to fewer stops keeps its ends.
    let pad = Gradient::new(GradientKind::Linear, g.stops.clone());
    let fitted = pad.fitted(16);
    assert_eq!(fitted.stops.len(), 16);
    for (stop, want) in [(&fitted.stops[0], Color::gray(0.0)), (&fitted.stops[15], Color::gray(1.0))] {
        assert!(stop.color.to_rgb().iter().zip(want.to_rgb()).all(|(a, b)| (a - b).abs() < 1e-5), "{:?}", stop.color);
    }
    assert!(matches!(pad.fitted(500), std::borrow::Cow::Borrowed(_)));
}

/// One run of more stops than the limit (255 inside it, and a copy of each end color at 0 and 1)
/// is written within the limit, every run the area needs included.
#[test]
fn a_run_over_the_stop_limit_is_written_within_it() {
    let stops: Vec<GradientStop> = (1..=255).map(|i| GradientStop::new(i as f32 / 256.0, Color::gray(i as f32 / 256.0))).collect();
    let g = Gradient { spread: GradientSpread::Repeat, repeats: 2, ..Gradient::new(GradientKind::Linear, stops) };
    let geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(20.0, 0.0), aspect: 1.0, focal: None };
    let p = g.padded_within(&geom, Rect::new(0.0, 0.0, 20.0, 1.0), 256).unwrap();
    assert!(!p.capped && p.runs == 2 && p.gradient.stops.len() <= 256, "{} {} {}", p.capped, p.runs, p.gradient.stops.len());
    assert!(g.unrolled_within(256).stops.len() <= 256);
}

/// Fitting a run to fewer stops keeps a narrow band between two sharp steps, a stop linked to a
/// swatch, and each kept stop's link and tint, where sampling at evenly spaced places would lose
/// them.
#[test]
fn fitting_keeps_sharp_steps_and_swatch_links() {
    let mut stops: Vec<GradientStop> = (0..300).map(|i| GradientStop::new(i as f32 / 299.0, Color::gray(i as f32 / 299.0))).collect();
    let red = Color::rgb(1.0, 0.0, 0.0);
    // A band of red from 0.5003 to 0.5013, with a sharp step at each side.
    for (offset, color) in [(0.5003, None), (0.5003, Some(red)), (0.5013, Some(red)), (0.5013, None)] {
        let gray = Color::gray(offset);
        stops.push(GradientStop::new(offset, color.unwrap_or(gray)));
    }
    let spot = GradientStop { swatch: Some("Ink".into()), tint: 0.4, ..GradientStop::new(0.25, Color::gray(0.25)) };
    stops.push(spot.clone());
    // Three stops of one ink that look alike but print at different tints.
    for (offset, tint) in [(0.8, 0.2), (0.81, 0.9), (0.82, 0.2)] {
        stops.push(GradientStop { swatch: Some("Ink2".into()), tint, ..GradientStop::new(offset, Color::gray(offset)) });
    }
    // A span shaped by an off-center midpoint.
    if let Some(s) = stops.iter_mut().find(|s| (s.offset - 0.6).abs() < 2e-3) {
        s.midpoint = 0.13;
    }
    stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let g = Gradient::new(GradientKind::Linear, stops);
    let fitted = g.fitted(32);
    assert!(fitted.stops.len() <= 32, "{}", fitted.stops.len());
    let red_at = |t: f32| fitted.sample(t).0.to_rgb();
    assert_eq!(red_at(0.5008), [1.0, 0.0, 0.0], "the band stays");
    assert!(red_at(0.499)[1] > 0.4 && red_at(0.503)[1] > 0.4, "the band stays narrow");
    assert!(fitted.stops.iter().any(|s| s.swatch.as_deref() == Some("Ink") && s.tint == 0.4 && s.offset == 0.25), "{:?}", fitted.stops);
    assert!(fitted.stops.iter().any(|s| s.swatch.as_deref() == Some("Ink2") && s.tint == 0.9), "the tint change stays");
    assert!(fitted.stops.iter().any(|s| s.midpoint == 0.13), "the shaped span stays");
    // Of a stop that changes only its ink's tint and one that the colors' line nearly gives, the
    // first stays. (The stop before an ink starts stays as well: leaving it out would blend the
    // ink in sooner.)
    let ink = |offset: f32, tint: f32| GradientStop { swatch: Some("Ink2".into()), tint, ..GradientStop::new(offset, Color::gray(0.5)) };
    let few = vec![
        GradientStop::new(0.0, Color::gray(0.0)),
        GradientStop::new(0.2, Color::gray(0.17)),
        GradientStop::new(0.3, Color::gray(0.3)),
        ink(0.4, 0.2),
        ink(0.5, 0.9),
        ink(0.6, 0.2),
        GradientStop::new(1.0, Color::gray(1.0)),
    ];
    let fitted = Gradient::new(GradientKind::Linear, few).fitted(6).into_owned();
    let offsets: Vec<f32> = fitted.stops.iter().map(|s| s.offset).collect();
    assert_eq!(offsets, [0.0, 0.3, 0.4, 0.5, 0.6, 1.0]);
    // A Perceptual gradient leaves out the stop its own mixing gives, not the one the RGB line
    // gives.
    let (blue, yellow, gray) = (Color::rgb(0.0, 0.0, 1.0), Color::rgb(1.0, 1.0, 0.0), Color::rgb(0.5, 0.5, 0.5));
    let on_curve = GradientMethod::Perceptual.mix(&gray, &yellow, 0.5);
    let stops = vec![GradientStop::new(0.0, blue), GradientStop::new(0.5, gray), GradientStop::new(0.75, on_curve), GradientStop::new(1.0, yellow)];
    let perceptual = Gradient { method: GradientMethod::Perceptual, ..Gradient::new(GradientKind::Linear, stops) };
    let fitted = perceptual.fitted(3).into_owned();
    let offsets: Vec<f32> = fitted.stops.iter().map(|s| s.offset).collect();
    assert_eq!(offsets, [0.0, 0.5, 1.0]);
    // A black of other inks between blacks that look the same prints differently: it stays.
    let k = |offset: f32| GradientStop::new(offset, Color::Cmyk { c: 1.0, m: 1.0, y: 1.0, k: 1.0 });
    let rich = GradientStop::new(0.5, Color::Cmyk { c: 0.6, m: 0.4, y: 0.4, k: 1.0 });
    let blacks = vec![k(0.0), k(0.4), rich, k(0.6), k(1.0)];
    let fitted = Gradient::new(GradientKind::Linear, blacks).fitted(3).into_owned();
    let offsets: Vec<f32> = fitted.stops.iter().map(|s| s.offset).collect();
    assert_eq!(offsets, [0.0, 0.5, 1.0]);
    let spread = Gradient { spread: GradientSpread::Repeat, repeats: 4, ..g.clone() };
    let u = spread.unrolled_within(128);
    assert_eq!(u.stops.iter().filter(|s| s.swatch.as_deref() == Some("Ink")).count(), 4, "one linked stop a run");
}

/// Pixels (what the canvas draws a diamond, dithered or Perceptual gradient as, and what SVG and
/// PDF write as images) repeat with the stops, for each kind.
#[test]
fn pixels_repeat_and_reflect_for_every_kind() {
    let b = Rect::new(0.0, 0.0, 40.0, 40.0);
    let c = Point::new(20.0, 20.0);
    for kind in [GradientKind::Linear, GradientKind::Radial, GradientKind::Conic, GradientKind::Diamond] {
        for sp in [GradientSpread::Repeat, GradientSpread::Reflect] {
            for method in [GradientMethod::Classic, GradientMethod::Perceptual] {
                let mut g = GradientPaint::new(Gradient { spread: sp, repeats: 3, method, ..Gradient::new(kind, Gradient::default().stops) });
                let geom = if kind == GradientKind::Linear {
                    GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(40.0, 10.0), aspect: 1.0, focal: None }
                } else {
                    GradientGeom { start: c, end: c + Vec2::new(18.0, 4.0), aspect: 0.8, focal: None }
                };
                g.geom = Some(geom);
                let px: Vec<[u8; 4]> = g.pixels(b, b, 40, 40, false, &Color::to_rgb).collect();
                for (i, got) in px.iter().enumerate().step_by(37) {
                    let p = Point::new((i % 40) as f64 + 0.5, (i / 40) as f64 + 0.5);
                    let t = g.gradient.spread_param(geom.param_at(kind, p));
                    // Not where Repeat jumps from the last stop back to the first.
                    if sp == GradientSpread::Repeat && !(0.03..0.97).contains(&t) {
                        continue;
                    }
                    let want = g.gradient.sample(t as f32).0.to_rgba8(1.0);
                    assert!(got[0].abs_diff(want[0]) <= 2, "{kind:?} {sp:?} {method:?} {p:?}: {got:?} vs {want:?}");
                }
            }
        }
    }
}

/// A conic gradient's wedges: as many for each time the stops run around as for one turn, at most
/// so many.
#[test]
fn conic_wedges_count_each_run() {
    let g = spread(GradientKind::Conic, GradientSpread::Repeat, 4);
    assert_eq!(g.conic_wedge_count(360), 1440);
    assert_eq!(spread(GradientKind::Conic, GradientSpread::Pad, 4).conic_wedge_count(360), 360);
    assert_eq!(spread(GradientKind::Conic, GradientSpread::Reflect, MAX_REPEATS).conic_wedge_count(360), crate::gradient::MAX_CONIC_WEDGES);
}
