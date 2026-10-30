//! Native kind-38 B-splines, expanded into cubic paths in local inches, y-up.
//!
//! Original local `bspline-v16-records.json` / `bspline-v27-records.json` and
//! `bspline-page1.svg` establish the layout and geometry. Independent Python
//! comparison checked all 11 SVG cubics (every anchor, handle, power coefficient
//! and 101 positions per span); position/control discrepancies are at most
//! 0.000067 mm, within SVG export rounding. Both records are byte-identical in
//! v16/v27 and in the v15 file's bounded CPng/length-pool expansion.
//!
//! Geometry is the `ppdt` of an immediate `LIST npps` with `ppid` bytes
//! `ef e9 0e 66 4c a5 bc 40 91 0b 8a 16 16 c1 5e 4a`, not argument 30.
//! This evidence does not establish other property identifiers or future layouts.

use crate::reader::Reader;
use vectorcraft_geom::{BezPath, Point};

/// Decode the verified little-endian `ppdt`: u16 count, u16 closed, u16 zero,
/// `count` signed i32 coordinate pairs, then `count` u32 clamped flags (0 or 1).
/// The length must be exactly `6 + 12 * count`. Require at least two controls
/// for an open path and three for a closed path.
///
/// The caller must select kind 38, the exact property identifier and a supported
/// file version. It also owns object/page transforms and their coordinate bounds.
/// `remaining_points` is its unspent cumulative geometry quota; the returned
/// count charges every emitted coordinate, including handles and the explicit
/// closing endpoint (one move plus three points per cubic). Check this quota
/// before allocating controls or path storage. The u16 source count also bounds
/// an individual expansion to at most 196,606 output points.
pub(super) fn bspline(bytes: &[u8], remaining_points: usize) -> Result<(BezPath, usize), String> {
    let mut r = Reader::new(bytes);
    let count = usize::from(r.u16()?);
    let closed = match r.u16()? {
        0 => false,
        1 => true,
        _ => return Err("unsupported CDR B-spline closure flag".into()),
    };
    if r.u16()? != 0 {
        return Err("unsupported CDR B-spline reserved field".into());
    }
    if count < if closed { 3 } else { 2 } {
        return Err("invalid CDR B-spline control count".into());
    }
    let spans = if closed { count } else { count - 1 };
    let generated = spans.checked_mul(3).and_then(|n| n.checked_add(1)).ok_or("CDR B-spline point count overflow")?;
    if generated > remaining_points {
        return Err("CDR B-spline geometry point limit exceeded".into());
    }
    let expected = count.checked_mul(12).and_then(|n| n.checked_add(6)).ok_or("CDR B-spline payload size overflow")?;
    if bytes.len() != expected {
        return Err("unsupported or truncated CDR B-spline payload length".into());
    }

    // All products below fit even on 32-bit WASM: count originated as a u16.
    let mut coordinates = Reader::new(r.take(count * 8)?);
    let mut flags = Reader::new(r.take(count * 4)?);
    let mut controls = Vec::with_capacity(count);
    for _ in 0..count {
        let point = Point::new(coordinates.coord()?, coordinates.coord()?);
        let clamped = match flags.u32()? {
            0 => false,
            1 => true,
            _ => return Err("unsupported CDR B-spline clamped flag".into()),
        };
        controls.push(Control { point, clamped });
    }

    let first = controls.first().ok_or("missing CDR B-spline first control")?;
    let last = controls.last().ok_or("missing CDR B-spline last control")?;
    let start = anchor(closed.then_some(last.point), first, controls.get(1).map(|c| c.point));
    let mut path = BezPath::new();
    path.move_to(start);
    for (index, (a, b)) in controls.iter().zip(controls.iter().skip(1)).enumerate() {
        let next = controls.get(index + 2).or(if closed { Some(first) } else { None });
        let end = anchor(Some(a.point), b, next.map(|c| c.point));
        append_span(&mut path, a.point, b.point, end);
    }
    if closed {
        // Include the closing cubic, not a straight connector. Reuse the exact
        // initial anchor so rounding cannot create a separate closing segment.
        append_span(&mut path, last.point, first.point, start);
        path.close_path();
    }
    Ok((path, generated))
}

struct Control {
    point: Point,
    clamped: bool,
}

/// Open endpoints and clamped controls are interpolated. Other anchors use the
/// neighbouring controls' uniform cubic B-spline average, cyclic when closed.
/// Signed i32 coordinates are converted to f64 inches before any arithmetic;
/// these convex combinations remain finite and within the input control hull.
fn anchor(previous: Option<Point>, current: &Control, next: Option<Point>) -> Point {
    match (previous, next) {
        (Some(a), Some(b)) if !current.clamped => Point::new((a.x + 4.0 * current.point.x + b.x) / 6.0, (a.y + 4.0 * current.point.y + b.y) / 6.0),
        _ => current.point,
    }
}

/// Handles depend on raw adjacent controls, including beside clamped anchors.
/// Moving a clamped anchor must not rescale or recompute those handles from it.
fn append_span(path: &mut BezPath, a: Point, b: Point, end: Point) {
    let h1 = Point::new((2.0 * a.x + b.x) / 3.0, (2.0 * a.y + b.y) / 3.0);
    let h2 = Point::new((a.x + 2.0 * b.x) / 3.0, (a.y + 2.0 * b.y) / 3.0);
    path.curve_to(h1, h2, end);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use vectorcraft_geom::{CubicBez, ParamCurve, PathEl, PathSeg, Rect, Shape};

    /// Original synthetic field values, deliberately unrelated to fixture bytes.
    fn record(points: &[(i32, i32)], flags: &[u32], closed: bool) -> Vec<u8> {
        assert_eq!(points.len(), flags.len());
        let mut bytes = u16::try_from(points.len()).unwrap().to_le_bytes().to_vec();
        bytes.extend_from_slice(&u16::from(closed).to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        for (x, y) in points {
            bytes.extend_from_slice(&x.to_le_bytes());
            bytes.extend_from_slice(&y.to_le_bytes());
        }
        for flag in flags {
            bytes.extend_from_slice(&flag.to_le_bytes());
        }
        bytes
    }

    /// Convert small, integral test inches to native integer coordinates.
    fn inches(points: &[(i32, i32)], flags: &[u32], closed: bool) -> Vec<u8> {
        record(&points.iter().map(|(x, y)| (x * 254000, y * 254000)).collect::<Vec<_>>(), flags, closed)
    }

    /// Include the start of every actual cubic, rather than only stored endpoints.
    fn cubics(path: &BezPath) -> Vec<CubicBez> {
        path.segments().filter_map(|s| if let PathSeg::Cubic(c) = s { Some(c) } else { None }).collect()
    }

    /// Count actual output coordinates to verify the caller's quota charge.
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

    /// Compare independently chosen expected positions, allowing f64 rounding.
    fn assert_point(actual: Point, expected: (f64, f64)) {
        assert!((actual.x - expected.0).abs() < 1e-12, "x: {actual:?} != {expected:?}");
        assert!((actual.y - expected.1).abs() < 1e-12, "y: {actual:?} != {expected:?}");
    }

    #[test]
    fn open_endpoints_are_interpolated_even_without_clamped_flags() {
        let bytes = inches(&[(-3, 1), (0, 7), (6, -2), (9, 4)], &[0; 4], false);
        let (path, count) = bspline(&bytes, 10).unwrap();
        assert_eq!(count, 10);
        assert_eq!(stored_points(&path), count);
        assert_eq!(path.elements().len(), 4);
        let curves = cubics(&path);
        assert_eq!(curves.len(), 3);
        let expected = [
            [(-3.0, 1.0), (-2.0, 3.0), (-1.0, 5.0), (0.5, 4.5)],
            [(0.5, 4.5), (2.0, 4.0), (4.0, 1.0), (5.5, 0.5)],
            [(5.5, 0.5), (7.0, 0.0), (8.0, 2.0), (9.0, 4.0)],
        ];
        for (curve, expected) in curves.iter().zip(expected) {
            for (actual, expected) in [curve.p0, curve.p1, curve.p2, curve.p3].into_iter().zip(expected) {
                assert_point(actual, expected);
            }
        }
        assert_point(curves[1].eval(0.5), (3.0, 2.5));
        assert!(!path.elements().iter().any(|el| matches!(el, PathEl::ClosePath)));
        // Endpoint flags are valid metadata but do not change open-end geometry.
        assert_eq!(path, bspline(&inches(&[(-3, 1), (0, 7), (6, -2), (9, 4)], &[1, 0, 0, 1], false), 10).unwrap().0);
    }

    #[test]
    fn interior_clamp_changes_anchor_without_moving_adjacent_handles() {
        let bytes = inches(&[(-3, 1), (0, 7), (6, -2), (9, 4)], &[0, 1, 0, 0], false);
        let (path, count) = bspline(&bytes, 10).unwrap();
        assert_eq!(count, stored_points(&path));
        let curves = cubics(&path);
        assert_eq!(curves.len(), 3);
        assert_point(curves[0].p1, (-2.0, 3.0));
        assert_point(curves[0].p2, (-1.0, 5.0));
        assert_point(curves[0].p3, (0.0, 7.0));
        assert_eq!(curves[1].p0, curves[0].p3);
        assert_point(curves[1].p1, (2.0, 4.0));
        assert_point(curves[1].p2, (4.0, 1.0));
        assert_point(curves[1].p3, (5.5, 0.5));
        // The interpolated point is a genuine cusp, not a smooth fitted anchor.
        assert!((curves[0].p3 - curves[0].p2).cross(curves[1].p1 - curves[1].p0).abs() > 1.0);
    }

    #[test]
    fn closed_spline_averages_cyclic_neighbours_and_emits_the_last_cubic() {
        let bytes = inches(&[(-3, 0), (0, 6), (9, 0), (0, -3)], &[0; 4], true);
        let (path, count) = bspline(&bytes, 13).unwrap();
        assert_eq!(count, 13);
        assert_eq!(stored_points(&path), count);
        assert_eq!(path.elements().len(), 6);
        assert_eq!(path.elements().last(), Some(&PathEl::ClosePath));
        assert_eq!(path.bounding_box(), Rect::new(-2.0, -2.0, 6.0, 4.0));
        assert_eq!(path.winding(Point::ZERO), -1);
        let curves = cubics(&path);
        assert_eq!(curves.len(), 4);
        let expected = [
            [(-2.0, 0.5), (-2.0, 2.0), (-1.0, 4.0), (1.0, 4.0)],
            [(1.0, 4.0), (3.0, 4.0), (6.0, 2.0), (6.0, 0.5)],
            [(6.0, 0.5), (6.0, -1.0), (3.0, -2.0), (1.0, -2.0)],
            [(1.0, -2.0), (-1.0, -2.0), (-2.0, -1.0), (-2.0, 0.5)],
        ];
        for (i, (curve, expected)) in curves.iter().zip(expected).enumerate() {
            for (actual, expected) in [curve.p0, curve.p1, curve.p2, curve.p3].into_iter().zip(expected) {
                assert_point(actual, expected);
            }
            let next = curves[(i + 1) % curves.len()];
            assert_eq!(curve.p3, next.p0);
            assert!(((curve.p3 - curve.p2) - (next.p1 - next.p0)).hypot() < 1e-12);
        }
    }

    #[test]
    fn clamped_controls_at_both_sides_of_the_closed_seam_are_preserved() {
        let bytes = inches(&[(-3, 0), (0, 6), (9, 0), (0, -3)], &[1, 0, 0, 1], true);
        let (path, _) = bspline(&bytes, 13).unwrap();
        let curves = cubics(&path);
        assert_eq!(curves.len(), 4);
        assert_point(curves[0].p0, (-3.0, 0.0));
        assert_point(curves[0].p1, (-2.0, 2.0));
        assert_point(curves[0].p3, (1.0, 4.0));
        assert_point(curves[2].p3, (0.0, -3.0));
        assert_eq!(curves[3].p0, curves[2].p3);
        assert_point(curves[3].p1, (-1.0, -2.0));
        assert_point(curves[3].p2, (-2.0, -1.0));
        assert_eq!(curves[3].p3, curves[0].p0);
    }

    #[test]
    fn two_open_controls_form_one_straight_cubic_in_signed_local_inches() {
        let bytes = inches(&[(-3, -6), (3, 3)], &[0, 0], false);
        let (path, count) = bspline(&bytes, 4).unwrap();
        assert_eq!(count, 4);
        assert_eq!(
            path.elements(),
            &[PathEl::MoveTo(Point::new(-3.0, -6.0)), PathEl::CurveTo(Point::new(-1.0, -3.0), Point::new(1.0, 0.0), Point::new(3.0, 3.0))]
        );
        assert_eq!(path.bounding_box(), Rect::new(-3.0, -6.0, 3.0, 3.0));
    }

    #[test]
    fn fractional_native_units_are_retained_in_anchors_and_handles() {
        let (path, _) = bspline(&record(&[(0, 0), (1, 6), (6, 0)], &[0; 3], false), 7).unwrap();
        let curves = cubics(&path);
        assert_eq!(curves.len(), 2);
        assert_point(curves[0].p1, (1.0 / 762000.0, 2.0 / 254000.0));
        assert_point(curves[0].p2, (2.0 / 762000.0, 4.0 / 254000.0));
        assert_point(curves[0].p3, (5.0 / 762000.0, 4.0 / 254000.0));
        assert_eq!(curves[0].p3, curves[1].p0);
    }

    #[test]
    fn truncation_trailing_data_and_mismatched_counts_are_rejected() {
        let bytes = inches(&[(0, 0), (3, 6), (9, 0)], &[0; 3], false);
        for len in 0..bytes.len() {
            assert!(bspline(&bytes[..len], usize::MAX).is_err(), "accepted prefix {len}");
        }
        for padding in 1..=12 {
            let mut extended = bytes.clone();
            extended.resize(bytes.len() + padding, 0);
            assert!(bspline(&extended, usize::MAX).unwrap_err().contains("payload length"));
        }
        for count in [2u16, 4, u16::MAX] {
            let mut altered = bytes.clone();
            altered[..2].copy_from_slice(&count.to_le_bytes());
            assert!(bspline(&altered, usize::MAX).unwrap_err().contains("payload length"));
        }
    }

    #[test]
    fn unknown_flags_and_reserved_fields_are_not_treated_as_booleans() {
        let points = [(0, 0), (3, 6), (9, 0)];
        let bytes = inches(&points, &[0; 3], false);
        for (offset, values) in [(2, [2u16, 256, u16::MAX]), (4, [1u16, 256, u16::MAX])] {
            for value in values {
                let mut altered = bytes.clone();
                altered[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
                assert!(bspline(&altered, usize::MAX).is_err());
            }
        }
        for i in 0..points.len() {
            for invalid in [2, 0x100, 0x10000, u32::MAX] {
                let mut flags = [0; 3];
                flags[i] = invalid;
                assert!(bspline(&inches(&points, &flags, false), 7).unwrap_err().contains("clamped flag"));
            }
        }
    }

    #[test]
    fn degenerate_control_counts_fail_and_coincident_valid_controls_stay_finite() {
        for closed in [false, true] {
            for count in 0..if closed { 3 } else { 2 } {
                assert!(bspline(&record(&vec![(0, 0); count], &vec![0; count], closed), usize::MAX).unwrap_err().contains("control count"));
            }
            let (path, count) = bspline(&inches(&[(3, -6); 3], &[0, 1, 0], closed), 10).unwrap();
            assert_eq!(stored_points(&path), count);
            assert_eq!(path.bounding_box(), Rect::new(3.0, -6.0, 3.0, -6.0));
            for c in cubics(&path) {
                assert_eq!([c.p0, c.p1, c.p2, c.p3], [Point::new(3.0, -6.0); 4]);
            }
        }
    }

    #[test]
    fn quota_counts_handles_and_closing_endpoint_before_allocating() {
        for (closed, required) in [(false, 10), (true, 13)] {
            let bytes = inches(&[(-3, 0), (0, 6), (9, 0), (0, -3)], &[0; 4], closed);
            for remaining in [0, 4, required - 1] {
                assert!(bspline(&bytes, remaining).unwrap_err().contains("point limit"));
            }
            let (path, count) = bspline(&bytes, required).unwrap();
            assert_eq!(count, required);
            assert_eq!(stored_points(&path), required);
        }
        // No payload is necessary to reject an advertised expansion over quota.
        // The maximum u16 count remains bounded even with an unlimited caller.
        for (closed, required) in [(0u16, 196603), (1u16, 196606)] {
            let mut header = u16::MAX.to_le_bytes().to_vec();
            header.extend_from_slice(&closed.to_le_bytes());
            header.extend_from_slice(&0u16.to_le_bytes());
            assert!(bspline(&header, required - 1).unwrap_err().contains("point limit"));
            assert!(bspline(&header, usize::MAX).unwrap_err().contains("payload length"));
        }
    }

    #[test]
    fn extreme_signed_coordinates_cannot_overflow_weighted_geometry_or_bounds() {
        let points = [(i32::MIN, i32::MIN), (i32::MAX, i32::MAX), (i32::MIN, i32::MAX), (i32::MAX, i32::MIN)];
        let min = f64::from(i32::MIN) / 254000.0;
        let max = f64::from(i32::MAX) / 254000.0;
        for closed in [false, true] {
            for flags in [[0; 4], [1, 0, 1, 1]] {
                let (path, count) = bspline(&record(&points, &flags, closed), 13).unwrap();
                assert_eq!(stored_points(&path), count);
                for c in cubics(&path) {
                    for p in [c.p0, c.p1, c.p2, c.p3] {
                        assert!(p.x.is_finite() && p.y.is_finite());
                        assert!((min - 1e-10..=max + 1e-10).contains(&p.x));
                        assert!((min - 1e-10..=max + 1e-10).contains(&p.y));
                    }
                }
                let b = path.bounding_box();
                assert!([b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite() && (v * 72.0).abs() < 1e6));
            }
        }
    }
}
