//! Native kind-20 polygon/star seeds, expanded into editable paths in local inches.
//!
//! Layout reference: the MIT Kaitai CDR grammar's `polygon_transform`,
//! `polygon_coords` and `points_list` descriptions:
//! <https://github.com/kaitai-io/coreldraw_cdr.ksy/blob/9d8e2704b946961f5114115f1a488750a3c86ed0/coreldraw_cdr.ksy>.
//! Original local `polygons-v16-records.json` / `polygons-v27-records.json`
//! and `polygons-page1.svg` establish clockwise repetition of the stored side,
//! followed by axis scaling. All four probes agree within 0.000115 mm after
//! placement, including the concave, seven-point and nonuniformly scaled shapes.
//! The v15 file's argument-30/11000 pairs are byte-identical to the v16 probes.
//! The seed's intermediate anchors and handles are geometry, not disposable
//! construction hints. Cubic repetition and unequal argument-11000 scales have
//! synthetic coverage; the local probes have straight sides and equal seed
//! scales (the nonuniform probe is stretched by its separate object transform).

use crate::reader::Reader;
use std::f64::consts::TAU;
use vectorcraft_geom::{BezPath, PathEl, Point};

// Resource bounds, not claims about the authoring application's UI limits.
const MAX_SIDES: usize = 4096;
const MAX_POINTS: usize = 1_000_000;
const UNITS_PER_INCH: f64 = 254000.0;

/// Decode argument 30 and argument 11000 of a native kind-20 object.
///
/// The version uses the importer's convention (1600 = X6). Layouts before 1300,
/// alternate seed modes, displaced centres and compound stars with a non-coprime
/// step are unverified and return an error. A seed must describe one open side
/// whose endpoint agrees with the rotated start within integer quantization.
/// Only the expanded outline is closed, never the individual repeated sides.
///
/// `remaining_points` is the caller's unspent cumulative geometry quota. The
/// returned count is the number of stored point coordinates, including cubic
/// handles and the explicit final endpoint. Expansion is checked against that
/// quota before allocating either path; the caller must charge the returned
/// count with `Drawing::add_points`. Object/page transforms and fill rules remain
/// the caller's responsibility.
pub(super) fn polygon(coords: &[u8], parameters: &[u8], version: u32, remaining_points: usize) -> Result<(BezPath, usize), String> {
    let params = Parameters::read(parameters, version)?;
    let mut r = Reader::new(coords);
    let count = r.u32()? as usize;
    let generated = count
        .checked_sub(1)
        .filter(|n| *n != 0)
        .and_then(|n| n.checked_mul(params.sides))
        .and_then(|n| n.checked_add(1))
        .ok_or("invalid CDR polygon point count")?;
    if generated > remaining_points.min(MAX_POINTS) {
        return Err("CDR polygon geometry point limit exceeded".into());
    }
    let seed = Seed::read(r, count)?;
    let expected_end = rotate(seed.start, params.step, params.sides);
    // Both endpoint coordinates are quantized i32s. Snap only this quantization
    // discrepancy; a genuinely disconnected side must not acquire an invented
    // connector or be silently replaced with a regular polygon.
    if seed.end.distance(expected_end) > 2.0 / UNITS_PER_INCH {
        return Err("CDR polygon seed endpoints do not join under repetition".into());
    }

    let start = scaled(seed.start, params.scales)?;
    let mut path = BezPath::new();
    path.move_to(start);
    for side in 0..params.sides {
        // Both factors are below MAX_SIDES, including on 32-bit WASM. Reducing
        // the turn avoids cumulative angle drift for a complex star.
        let turn = (side * params.step) % params.sides;
        let next_turn = (turn + params.step) % params.sides;
        let map = |p| scaled(rotate(p, turn, params.sides), params.scales);
        let next_start = if next_turn == 0 { start } else { scaled(rotate(seed.start, next_turn, params.sides), params.scales)? };
        for (index, element) in seed.path.elements().iter().enumerate().skip(1) {
            let last = index + 1 == seed.path.elements().len();
            match *element {
                PathEl::LineTo(end) => path.line_to(if last { next_start } else { map(end)? }),
                PathEl::CurveTo(a, b, end) => path.curve_to(map(a)?, map(b)?, if last { next_start } else { map(end)? }),
                _ => return Err("unsupported CDR polygon seed segment".into()),
            }
        }
    }
    path.close_path();
    Ok((path, generated))
}

struct Parameters {
    sides: usize,
    step: usize,
    scales: [f64; 2],
}

impl Parameters {
    /// Accept the modern, single-side seed layout; reject other layouts explicitly.
    fn read(bytes: &[u8], version: u32) -> Result<Self, String> {
        if !(1300..2800).contains(&version) {
            return Err("unsupported CDR polygon version".into());
        }
        if bytes.len() != 48 {
            return Err("unsupported or truncated CDR polygon parameters".into());
        }
        let mut r = Reader::new(bytes);
        let sides = r.u32()? as usize;
        let mode = r.u32()?;
        let step = r.u32()? as usize;
        let reserved = r.u32()?;
        if !(3..=MAX_SIDES).contains(&sides) || step == 0 || step >= sides {
            return Err("invalid CDR polygon side count or step".into());
        }
        if mode != 1 || reserved != 0 {
            return Err("unsupported CDR polygon seed mode".into());
        }
        if gcd(sides, step) != 1 {
            // Repeating sides times would retrace one component while omitting
            // the others. Component offsets/order need an independent oracle.
            return Err("unsupported CDR compound polygon/star step".into());
        }
        let scales = [r.f64()?, r.f64()?];
        let centre_x = r.u32()?;
        let centre_y = r.u32()?;
        // The nonzero editing hint in the concave probe is already reflected in
        // its stored midpoint. Do not apply it again or regenerate that midpoint.
        let _editing_hint = r.u32()?;
        let tail = r.u32()?;
        if centre_x != 0 || centre_y != 0 || tail != 0 {
            return Err("unsupported CDR polygon centre or metadata".into());
        }
        Ok(Self { sides, step, scales })
    }
}

struct Seed {
    path: BezPath,
    start: Point,
    end: Point,
}

impl Seed {
    /// Parse the ordinary curve node opcodes, requiring exactly one open side.
    fn read(mut r: Reader<'_>, count: usize) -> Result<Self, String> {
        let coordinate_bytes = count.checked_mul(8).ok_or("CDR polygon seed size overflow")?;
        let mut points = Reader::new(r.take(coordinate_bytes)?);
        let flags = r.take(count)?;
        // X6 carries eight trailing zero bytes; the v27 records end at the flags.
        // Other trailers are not silently accepted as geometry-free metadata.
        let trailing = r.take(r.bytes.len().saturating_sub(r.pos))?;
        if !matches!(trailing.len(), 0 | 8) || trailing.iter().any(|b| *b != 0) {
            return Err("unsupported CDR polygon seed trailer".into());
        }
        let first = flags.first().copied().ok_or("empty CDR polygon seed")?;
        validate_flag(first)?;
        if first >> 6 != 0 {
            return Err("CDR polygon seed must begin with a move point".into());
        }
        let start = Point::new(points.coord()?, points.coord()?);
        let mut end = start;
        let mut path = BezPath::new();
        path.move_to(start);
        let mut control_a = None;
        let mut control_b = None;
        for flag in flags.iter().copied().skip(1) {
            validate_flag(flag)?;
            let point = Point::new(points.coord()?, points.coord()?);
            match flag >> 6 {
                1 => {
                    if control_a.is_some() {
                        return Err("CDR polygon line discards cubic control points".into());
                    }
                    path.line_to(point);
                    end = point;
                }
                2 => {
                    let (Some(a), Some(b)) = (control_a.take(), control_b.take()) else {
                        return Err("CDR polygon cubic is missing its control points".into());
                    };
                    path.curve_to(a, b, point);
                    end = point;
                }
                3 => {
                    if control_a.is_none() {
                        control_a = Some(point);
                    } else if control_b.is_none() {
                        control_b = Some(point);
                    } else {
                        return Err("too many CDR polygon cubic control points".into());
                    }
                }
                _ => return Err("unsupported multi-subpath CDR polygon seed".into()),
            }
        }
        if control_a.is_some() {
            return Err("CDR polygon seed ends with unused control points".into());
        }
        Ok(Self { path, start, end })
    }
}

/// A closing flag would close each side, not the whole expanded polygon. That
/// alternate seed layout has no oracle. Editable and smooth/symmetric node bits
/// need no geometric conversion because the explicit handles are retained.
fn validate_flag(flag: u8) -> Result<(), String> {
    if flag & 8 != 0 {
        return Err("unsupported closed CDR polygon seed".into());
    }
    if flag & 3 != 0 || flag & 0x30 == 0x30 {
        return Err("unsupported CDR polygon node flags".into());
    }
    Ok(())
}

/// Clockwise rotation about the native seed origin, before axis scaling.
fn rotate(point: Point, turn: usize, sides: usize) -> Point {
    let (sin, cos) = (-TAU * turn as f64 / sides as f64).sin_cos();
    Point::new(point.x * cos - point.y * sin, point.x * sin + point.y * cos)
}

/// Finite input scales can still overflow when applied to a finite seed.
fn scaled(point: Point, [x, y]: [f64; 2]) -> Result<Point, String> {
    let point = Point::new(point.x * x, point.y * y);
    if !point.x.is_finite() || !point.y.is_finite() {
        return Err("non-finite CDR polygon coordinate".into());
    }
    Ok(point)
}

/// Euclid's algorithm, used only after validating the positive side count/step.
fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use vectorcraft_geom::{CubicBez, PathSeg, Shape};

    /// Original records assembled from the documented field types, not fixture bytes.
    fn parameters(sides: u32, step: u32, scales: [f64; 2]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [sides, 1, step, 0] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for value in scales {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.resize(48, 0);
        bytes
    }

    /// Encode original integer points followed by their ordinary curve flags.
    fn seed(points: &[(i32, i32)], flags: &[u8]) -> Vec<u8> {
        assert_eq!(points.len(), flags.len());
        let mut bytes = (points.len() as u32).to_le_bytes().to_vec();
        for (x, y) in points {
            bytes.extend_from_slice(&x.to_le_bytes());
            bytes.extend_from_slice(&y.to_le_bytes());
        }
        bytes.extend_from_slice(flags);
        bytes
    }

    /// Synthesize a straight side with an editable midpoint on a ten-million-unit
    /// circle. The integer midpoint deliberately retains native quantization.
    fn regular_seed(sides: u32, step: u32) -> Vec<u8> {
        let angle = TAU * f64::from(step) / f64::from(sides);
        let end = ((10_000_000.0 * angle.sin()).round() as i32, (10_000_000.0 * angle.cos()).round() as i32);
        seed(&[(0, 10_000_000), (end.0 / 2, (10_000_000 + end.1) / 2), end], &[4, 0x44, 0x44])
    }

    /// Count actual allocated geometry coordinates, not just path elements.
    fn stored_points(path: &BezPath) -> usize {
        path.elements()
            .iter()
            .map(|el| match el {
                PathEl::MoveTo(_) | PathEl::LineTo(_) => 1,
                PathEl::QuadTo(_, _) => 2,
                PathEl::CurveTo(_, _, _) => 3,
                PathEl::ClosePath => 0,
            })
            .sum()
    }

    /// Collect anchors from the all-straight synthetic outlines.
    fn anchors(path: &BezPath) -> Vec<Point> {
        path.elements()
            .iter()
            .filter_map(|el| match el {
                PathEl::MoveTo(p) | PathEl::LineTo(p) => Some(*p),
                _ => None,
            })
            .collect()
    }

    /// Compare independent expectations, allowing only the requested rounding tolerance.
    fn assert_point(actual: Point, expected: (f64, f64), tolerance: f64) {
        assert!((actual.x - expected.0).abs() <= tolerance, "x: {actual:?} != {expected:?}");
        assert!((actual.y - expected.1).abs() <= tolerance, "y: {actual:?} != {expected:?}");
    }

    #[test]
    fn hexagon_retains_midpoints_and_closes_once_in_local_inches() {
        let coords = regular_seed(6, 1);
        let params = parameters(6, 1, [0.022; 2]);
        let radius = 22.0 / 25.4;
        let x = 3.0f64.sqrt() / 2.0;
        let expected = [
            (0.0, 1.0),
            (x / 2.0, 0.75),
            (x, 0.5),
            (x, 0.0),
            (x, -0.5),
            (x / 2.0, -0.75),
            (0.0, -1.0),
            (-x / 2.0, -0.75),
            (-x, -0.5),
            (-x, 0.0),
            (-x, 0.5),
            (-x / 2.0, 0.75),
            (0.0, 1.0),
        ];
        for version in [1500, 1600, 2700] {
            let (path, count) = polygon(&coords, &params, version, 13).unwrap();
            assert_eq!(count, 13);
            assert_eq!(stored_points(&path), count);
            let points = anchors(&path);
            assert_eq!(points.len(), expected.len());
            for (p, (x, y)) in points.iter().zip(expected) {
                assert_point(*p, (radius * x, radius * y), 1e-7);
            }
            assert_eq!(points.first(), points.last());
            assert_eq!(path.elements().iter().filter(|el| matches!(el, PathEl::MoveTo(_))).count(), 1);
            assert_eq!(path.elements().iter().filter(|el| matches!(el, PathEl::ClosePath)).count(), 1);
            assert_eq!(path.elements().last(), Some(&PathEl::ClosePath));
            assert_eq!(path.winding(Point::ZERO), -1);
        }
    }

    #[test]
    fn seven_point_complex_star_visits_every_tip_in_step_three_order() {
        let (path, count) = polygon(&regular_seed(7, 3), &parameters(7, 3, [0.021; 2]), 1600, 15).unwrap();
        let radius = 21.0 / 25.4;
        // Independent unit-circle vertices in the observed 0,3,6,2,5,1,4 order.
        let tips = [
            (0.0, 1.0),
            (0.433883739117558, -0.900968867902419),
            (-0.781831482468030, 0.623489801858734),
            (0.974927912181824, -0.222520933956314),
            (-0.974927912181824, -0.222520933956314),
            (0.781831482468030, 0.623489801858734),
            (-0.433883739117558, -0.900968867902419),
        ];
        let points = anchors(&path);
        assert_eq!(count, 15);
        assert_eq!(stored_points(&path), count);
        assert_eq!(points.len(), 15);
        for (i, (x, y)) in tips.iter().copied().enumerate() {
            let next = tips[(i + 1) % tips.len()];
            assert_point(points[2 * i], (x * radius, y * radius), 1e-7);
            assert_point(points[2 * i + 1], ((x + next.0) * radius / 2.0, (y + next.1) * radius / 2.0), 1e-7);
        }
        assert_eq!(points.first(), points.last());
        // Closing the individual seed sides or substituting a heptagon changes this.
        assert_eq!(path.winding(Point::ZERO), -3);
    }

    #[test]
    fn concave_midpoint_is_not_replaced_or_modified_again_by_editing_hint() {
        let coords = seed(&[(0, 254000), (50800, 50800), (254000, 0)], &[4, 0x44, 0x44]);
        let expected = [(0.0, 1.0), (0.2, 0.2), (1.0, 0.0), (0.2, -0.2), (0.0, -1.0), (-0.2, -0.2), (-1.0, 0.0), (-0.2, 0.2), (0.0, 1.0)];
        for hint in [0u32, 46, 73] {
            let mut params = parameters(4, 1, [1.0; 2]);
            params[40..44].copy_from_slice(&hint.to_le_bytes());
            let (path, count) = polygon(&coords, &params, 2700, 9).unwrap();
            assert_eq!(count, 9);
            let points = anchors(&path);
            assert_eq!(points.len(), expected.len());
            for (p, expected) in points.into_iter().zip(expected) {
                assert_point(p, expected, 1e-12);
            }
        }
    }

    #[test]
    fn cubic_seed_preserves_handles_and_continuity_under_nonuniform_scaling() {
        // A deliberately non-circular cubic side, tangent at each quarter turn.
        let coords = seed(&[(0, 254000), (127000, 254000), (254000, 127000), (254000, 0)], &[0x24, 0xc0, 0xc0, 0xa4]);
        let (path, count) = polygon(&coords, &parameters(4, 1, [2.0, 0.5]), 2700, 13).unwrap();
        let curves: Vec<CubicBez> = path.segments().filter_map(|s| if let PathSeg::Cubic(c) = s { Some(c) } else { None }).collect();
        assert_eq!(curves.len(), 4);
        assert_eq!(count, 13);
        assert_eq!(stored_points(&path), count);
        assert_point(curves[0].p0, (0.0, 0.5), 1e-12);
        assert_point(curves[0].p1, (1.0, 0.5), 1e-12);
        assert_point(curves[0].p2, (2.0, 0.25), 1e-12);
        assert_point(curves[0].p3, (2.0, 0.0), 1e-12);
        assert_point(curves[1].p1, (2.0, -0.25), 1e-12);
        assert_point(curves[1].p2, (1.0, -0.5), 1e-12);
        assert_point(curves[1].p3, (0.0, -0.5), 1e-12);
        for (i, curve) in curves.iter().enumerate() {
            let next = &curves[(i + 1) % curves.len()];
            assert_eq!(curve.p3, next.p0);
            assert!(((curve.p3 - curve.p2) - (next.p1 - next.p0)).hypot() < 1e-12);
        }
        assert_eq!(curves[3].p3, curves[0].p0);
        assert_eq!(path.elements().last(), Some(&PathEl::ClosePath));
        assert!(polygon(&coords, &parameters(4, 1, [2.0, 0.5]), 2700, 12).is_err());
    }

    #[test]
    fn mixed_line_and_cubic_seed_keeps_the_internal_anchor() {
        let coords = seed(&[(0, 254000), (50800, 101600), (101600, 101600), (254000, 50800), (254000, 0)], &[4, 0x44, 0xc0, 0xc0, 0x84]);
        let (path, count) = polygon(&coords, &parameters(4, 1, [1.0; 2]), 1500, 17).unwrap();
        assert_eq!(count, 17);
        assert_eq!(stored_points(&path), count);
        assert_eq!(path.elements().get(1), Some(&PathEl::LineTo(Point::new(0.2, 0.4))));
        assert_eq!(path.elements().iter().filter(|el| matches!(el, PathEl::LineTo(_))).count(), 4);
        assert_eq!(path.elements().iter().filter(|el| matches!(el, PathEl::CurveTo(_, _, _))).count(), 4);
    }

    #[test]
    fn only_integer_quantization_discrepancies_are_joined() {
        let params = parameters(4, 1, [1.0; 2]);
        let rounded = seed(&[(0, 254000), (254001, 1)], &[4, 0x44]);
        let (path, _) = polygon(&rounded, &params, 2700, 5).unwrap();
        assert_point(anchors(&path)[1], (1.0, 0.0), 1e-12);
        let disconnected = seed(&[(0, 254000), (254003, 0)], &[4, 0x44]);
        assert!(polygon(&disconnected, &params, 2700, 5).unwrap_err().contains("do not join"));
    }

    #[test]
    fn expansion_uses_remaining_quota_including_closing_endpoint_and_controls() {
        let coords = regular_seed(6, 1);
        let params = parameters(6, 1, [0.022; 2]);
        for remaining in [0, 3, 6, 12] {
            assert!(polygon(&coords, &params, 1600, remaining).unwrap_err().contains("point limit"));
        }
        assert_eq!(polygon(&coords, &params, 1600, 13).unwrap().1, 13);
        // Rejected before reading or allocating the advertised seed, even if a
        // caller accidentally passes an unlimited remaining budget.
        assert!(polygon(&246u32.to_le_bytes(), &parameters(4096, 1, [1.0; 2]), 2700, usize::MAX).unwrap_err().contains("point limit"));
        assert!(polygon(&u32::MAX.to_le_bytes(), &params, 1600, usize::MAX).is_err());
        for count in [0u32, 1] {
            assert!(polygon(&count.to_le_bytes(), &params, 1600, 100).is_err());
        }
    }

    #[test]
    fn malformed_and_compound_steps_do_not_silently_retrace_one_component() {
        let coords = regular_seed(6, 1);
        for sides in [0, 1, 2, 4097, u32::MAX] {
            assert!(polygon(&coords, &parameters(sides, 1, [1.0; 2]), 1600, 100).is_err());
        }
        for step in [0, 6, 7, u32::MAX] {
            assert!(polygon(&coords, &parameters(6, step, [1.0; 2]), 1600, 100).is_err());
        }
        let compound = regular_seed(6, 2);
        assert!(polygon(&compound, &parameters(6, 2, [1.0; 2]), 1600, 100).unwrap_err().contains("compound"));
    }

    #[test]
    fn parameter_layouts_and_finite_numbers_are_checked() {
        let coords = regular_seed(6, 1);
        let params = parameters(6, 1, [0.022; 2]);
        for version in [0, 1200, 1299, 2800, u32::MAX] {
            assert!(polygon(&coords, &params, version, 100).is_err());
        }
        for offset in [4, 12, 32, 36, 44] {
            let mut altered = params.clone();
            altered[offset..offset + 4].copy_from_slice(&2u32.to_le_bytes());
            assert!(polygon(&coords, &altered, 2700, 100).is_err());
        }
        for mode in [0u32, 3, u32::MAX] {
            let mut altered = params.clone();
            altered[4..8].copy_from_slice(&mode.to_le_bytes());
            assert!(polygon(&coords, &altered, 2700, 100).is_err());
        }
        for offset in [16, 24] {
            for number in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX] {
                let mut altered = params.clone();
                altered[offset..offset + 8].copy_from_slice(&number.to_le_bytes());
                assert!(polygon(&coords, &altered, 2700, 100).is_err());
            }
        }
    }

    #[test]
    fn truncated_records_and_unknown_trailers_are_rejected() {
        let coords = regular_seed(6, 1);
        let params = parameters(6, 1, [0.022; 2]);
        for len in 0..coords.len() {
            assert!(polygon(&coords[..len], &params, 2700, 100).is_err(), "accepted coordinate prefix {len}");
        }
        for len in 0..params.len() {
            assert!(polygon(&coords, &params[..len], 2700, 100).is_err(), "accepted parameter prefix {len}");
        }
        let (original, _) = polygon(&coords, &params, 2700, 100).unwrap();
        for padding in 1..=9 {
            let mut extended = coords.clone();
            extended.resize(extended.len() + padding, 0);
            if padding == 8 {
                let (padded, _) = polygon(&extended, &params, 1600, 100).unwrap();
                assert_eq!(original, padded);
                *extended.last_mut().unwrap() = 1;
            }
            assert!(polygon(&extended, &params, 1600, 100).is_err());
        }
        let mut extended = params;
        extended.push(0);
        assert!(polygon(&coords, &extended, 1600, 100).is_err());
    }

    #[test]
    fn invalid_seed_opcodes_controls_and_per_side_closures_are_rejected() {
        let params = parameters(4, 1, [1.0; 2]);
        let points = [(0, 254000), (127000, 127000), (254000, 0)];
        for flags in [
            [0x44, 0x44, 0x44], // no initial move
            [4, 4, 0x44],       // multiple subpaths
            [4, 0xc0, 0x84],    // missing second handle
            [4, 0xc0, 0x44],    // line discards a handle
            [4, 0x44, 0xc0],    // unused final handle
            [0x0c, 0x44, 0x44], // closed-seed marker on move
            [4, 0x4c, 0x44],    // closing an intermediate side segment
            [4, 0x44, 0x4c],    // closing each seed side
            [4, 0x45, 0x44],    // unknown flag
            [4, 0x46, 0x44],    // character marker
            [4, 0x74, 0x44],    // unknown continuation
        ] {
            assert!(polygon(&seed(&points, &flags), &params, 1600, 100).is_err(), "accepted {flags:?}");
        }
        let controls = seed(&[(0, 254000), (1, 2), (3, 4), (5, 6), (254000, 0)], &[4, 0xc0, 0xc0, 0xc0, 0x84]);
        assert!(polygon(&controls, &params, 1600, 100).unwrap_err().contains("too many"));
    }
}
