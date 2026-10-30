//! Bounded native rectangle geometry in local inches, with y pointing up.
//!
//! Layout reference: the MIT Kaitai CDR grammar's `rectangle` description:
//! <https://github.com/kaitai-io/coreldraw_cdr.ksy/blob/9d8e2704b946961f5114115f1a488750a3c86ed0/coreldraw_cdr.ksy>.
//! Original local `corners-v16-records.json` / `corners-v27-records.json` and
//! `corners-page1.svg` establish the modern corner order and 0.55 cubic handles.
//! The corresponding `corner-types-*` records and SVG establish round, scallop
//! and chamfer treatments, plus absolute and relative radius modes. Absolute
//! radii divide by the smaller absolute scale; relative radii multiply half the
//! shorter local side. The caller applies the object's transform afterward.
//! Other modes/treatments, nonzero reserved fields and overlapping radii remain
//! unsupported or explicitly warned approximations.

use crate::reader::Reader;
use vectorcraft_geom::{BezPath, Point};

const UNITS_PER_INCH: f64 = 254000.0;
const ROUND_HANDLE: f64 = 0.55;

/// Treatments established by the original native records and independent SVG.
#[derive(Clone, Copy, Default)]
enum Treatment {
    #[default]
    Round,
    Scallop,
    Chamfer,
}

/// A corner's radius is converted from its stored mode to inches before tracing.
#[derive(Clone, Copy, Default)]
struct Corner {
    radius: f64,
    treatment: Treatment,
}

/// Decode argument 30 of a rectangle, retaining its signed local dimensions.
///
/// `true` means an unsupported corner layout was imported square, or an
/// oversized radius was clamped as an approximation. Old corner treatments
/// retain the square fallback. Malformed/truncated numeric data is an error.
/// The caller owns the object/page transforms and cumulative geometry budget;
/// this emits four sides and at most four corner segments (17 stored points).
pub(super) fn rectangle(data: &[u8], version: u32) -> Result<(BezPath, bool), String> {
    let mut r = Reader::new(data);
    if version < 1500 {
        let (width, height) = (r.coord()?, r.coord()?);
        let mut unsupported = false;
        for _ in 0..if version >= 900 { 4 } else { 1 } {
            unsupported |= r.coord()? != 0.0;
        }
        return Ok((outline(width, height, [Corner::default(); 4]), unsupported));
    }

    let (width, height) = (r.f64()? / UNITS_PER_INCH, r.f64()? / UNITS_PER_INCH);
    let (scale_x, scale_y) = (r.f64()?, r.f64()?);
    let mut corners = [Corner::default(); 4];
    let mut relative = [false; 4];
    let mut unsupported = false;
    // The verified 128-byte record has four 24-byte slots, ordered lower-right,
    // upper-right, upper-left, lower-left in the positive local rectangle.
    for (corner, relative) in corners.iter_mut().zip(&mut relative) {
        let mode = r.u32()?;
        let reserved = r.u32()?;
        corner.radius = r.f64()?;
        let treatment = r.u32()?;
        let tail = r.u32()?;
        if corner.radius < 0.0 {
            return Err("negative CDR rectangle corner radius".into());
        }
        // Mode 0 stores a fraction of half the shorter side, including ordinary
        // square rectangles whose four slots are entirely zero. Mode 1 stores
        // an absolute distance in native units, normalized by the scale below.
        *relative = mode == 0;
        unsupported |= mode > 1 || reserved != 0 || tail != 0;
        corner.treatment = match treatment {
            0 => Treatment::Round,
            1 => Treatment::Scallop,
            2 => Treatment::Chamfer,
            _ => {
                unsupported = true;
                Treatment::Round
            }
        };
    }
    unsupported |= r.pos != data.len();
    if unsupported {
        return Ok((outline(width, height, [Corner::default(); 4]), true));
    }

    let scale = scale_x.abs().min(scale_y.abs());
    let limit = width.abs().min(height.abs()) / 2.0;
    let mut approximated = false;
    for (corner, relative) in corners.iter_mut().zip(relative) {
        if corner.radius == 0.0 {
            continue;
        }
        if relative {
            // Clamp the fraction before multiplying to keep even enormous finite
            // input bounded. Relative radii do not divide by the stored scales.
            approximated |= corner.radius > 1.0;
            corner.radius = corner.radius.min(1.0) * limit;
        } else {
            if scale == 0.0 {
                return Err("zero CDR rectangle corner scale".into());
            }
            corner.radius = (corner.radius / UNITS_PER_INCH) / scale;
        }
        if !corner.radius.is_finite() {
            return Err("non-finite CDR rectangle corner extent".into());
        }
        // The probes do not establish overlap resolution. Half the shorter side
        // prevents crossings for all four corners; report any such adjustment.
        if corner.radius > limit {
            corner.radius = limit;
            approximated = true;
        }
    }
    Ok((outline(width, height, corners), approximated))
}

/// Trace the rectangle counter-clockwise before reflecting signed dimensions.
/// Radii are finite, nonnegative and no larger than half either absolute side.
fn outline(width: f64, height: f64, [lr, ur, ul, ll]: [Corner; 4]) -> BezPath {
    let (w, h) = (width.abs(), height.abs());
    let point = |x: f64, y: f64| Point::new(x * width.signum(), y * height.signum());
    let mut path = BezPath::new();
    path.move_to(point(ll.radius, 0.0));
    path.line_to(point(w - lr.radius, 0.0));
    if lr.radius > 0.0 {
        append_corner(&mut path, lr.treatment, point(w - lr.radius, 0.0), point(w, 0.0), point(w, lr.radius));
    }
    path.line_to(point(w, h - ur.radius));
    if ur.radius > 0.0 {
        append_corner(&mut path, ur.treatment, point(w, h - ur.radius), point(w, h), point(w - ur.radius, h));
    }
    path.line_to(point(ul.radius, h));
    if ul.radius > 0.0 {
        append_corner(&mut path, ul.treatment, point(ul.radius, h), point(0.0, h), point(0.0, h - ul.radius));
    }
    if ll.radius > 0.0 {
        path.line_to(point(0.0, ll.radius));
        append_corner(&mut path, ll.treatment, point(0.0, ll.radius), point(0.0, 0.0), point(ll.radius, 0.0));
    }
    path.close_path();
    path
}

/// Join two side endpoints around the original vertex, using the observed 0.55
/// handle length. Scallops turn inward with handles transverse to the sides.
fn append_corner(path: &mut BezPath, treatment: Treatment, start: Point, vertex: Point, end: Point) {
    match treatment {
        Treatment::Round => path.curve_to(start + (vertex - start) * ROUND_HANDLE, end + (vertex - end) * ROUND_HANDLE, end),
        Treatment::Scallop => path.curve_to(start + (end - vertex) * ROUND_HANDLE, end + (start - vertex) * ROUND_HANDLE, end),
        Treatment::Chamfer => path.line_to(end),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use vectorcraft_geom::{Affine, CubicBez, ParamCurve, PathEl, PathSeg, Rect, Shape};

    /// Generate original records using inches for dimensions and stored radii.
    fn modern(width: f64, height: f64, scales: [f64; 2], radii: [f64; 4]) -> Vec<u8> {
        record(width, height, scales, radii.map(|radius| (1, radius * 254000.0, 0)))
    }

    /// Generate slots with explicit (mode, raw radius, treatment); dimensions are inches.
    fn record(width: f64, height: f64, scales: [f64; 2], corners: [(u32, f64, u32); 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        for value in [width * 254000.0, height * 254000.0, scales[0], scales[1]] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        for (mode, radius, treatment) in corners {
            bytes.extend_from_slice(&mode.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&radius.to_le_bytes());
            bytes.extend_from_slice(&treatment.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
        }
        bytes
    }

    /// Extract actual cubic segments, including their starting points.
    fn cubics(path: &BezPath) -> Vec<CubicBez> {
        path.segments().filter_map(|segment| if let PathSeg::Cubic(curve) = segment { Some(curve) } else { None }).collect()
    }

    /// Compare independent expected local positions without relying on exact rounding.
    fn assert_point(actual: Point, expected: (f64, f64)) {
        assert!((actual.x - expected.0).abs() < 1e-12, "x: {actual:?} != {expected:?}");
        assert!((actual.y - expected.1).abs() < 1e-12, "y: {actual:?} != {expected:?}");
    }

    #[test]
    fn asymmetric_rounds_preserve_corner_identity_and_square_corner() {
        let bytes = modern(6.0, 4.0, [1.0, 1.0], [1.0, 0.75, 0.25, 0.0]);
        for version in [1500, 1600, 2700] {
            let (path, warning) = rectangle(&bytes, version).unwrap();
            assert!(!warning);
            assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 6.0, 4.0));
            assert_eq!(path.elements().last(), Some(&PathEl::ClosePath));
            let curves = cubics(&path);
            assert_eq!(curves.len(), 3);
            assert_point(curves[0].p0, (5.0, 0.0));
            assert_point(curves[0].p3, (6.0, 1.0));
            assert_point(curves[1].p0, (6.0, 3.25));
            assert_point(curves[1].p3, (5.25, 4.0));
            assert_point(curves[2].p0, (0.25, 4.0));
            assert_point(curves[2].p3, (0.0, 3.75));
            // A conventional circular kappa would give a different midpoint.
            assert_point(curves[0].eval(0.5), (5.70625, 0.29375));
            assert_ne!(path.winding(Point::new(0.01, 0.01)), 0);
            assert_eq!(path.winding(Point::new(0.01, 3.99)), 0);
            assert_eq!(path.winding(Point::new(5.75, 0.25)), 0);
            assert_ne!(path.winding(Point::new(5.5, 0.5)), 0);
        }
    }

    #[test]
    fn corner_scale_is_normalized_before_the_object_transform() {
        for scales in [[2.5, 0.5], [-2.5, 0.5], [2.5, -0.5]] {
            let (path, warning) = rectangle(&modern(7.0, 5.0, scales, [0.25; 4]), 1600).unwrap();
            assert!(!warning);
            assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 7.0, 5.0));
            // The magnitudes alone affect local radii; placement/reflection is external.
            let placed = Affine::new([2.5, 0.0, 0.0, 0.5, 0.0, 0.0]) * path;
            assert_eq!(placed.bounding_box(), Rect::new(0.0, 0.0, 17.5, 2.5));
            let curves = cubics(&placed);
            assert_eq!(curves.len(), 4);
            assert_point(curves[0].p0, (16.25, 0.0));
            assert_point(curves[0].p3, (17.5, 0.25));
        }
    }

    #[test]
    fn relative_and_absolute_modes_have_distinct_nonuniform_scaling() {
        let scales = [1.5, 0.5];
        let relative = record(8.0, 5.0, scales, [(0, 0.4, 0); 4]);
        let absolute = modern(8.0, 5.0, scales, [1.0; 4]);
        for version in [1500, 1600, 2700] {
            let (relative_path, relative_warning) = rectangle(&relative, version).unwrap();
            let (absolute_path, absolute_warning) = rectangle(&absolute, version).unwrap();
            assert!(!relative_warning && !absolute_warning);
            assert_eq!(relative_path.bounding_box(), absolute_path.bounding_box());
            let relative_curves = cubics(&relative_path);
            let absolute_curves = cubics(&absolute_path);
            assert_point(relative_curves[0].p0, (7.0, 0.0));
            assert_point(relative_curves[0].p3, (8.0, 1.0));
            assert_point(absolute_curves[0].p0, (6.0, 0.0));
            assert_point(absolute_curves[0].p3, (8.0, 2.0));

            let transform = Affine::new([1.5, 0.0, 0.0, 0.5, 0.0, 0.0]);
            let relative_curves = cubics(&(transform * relative_path));
            let absolute_curves = cubics(&(transform * absolute_path));
            assert_point(relative_curves[0].p0, (10.5, 0.0));
            assert_point(relative_curves[0].p3, (12.0, 0.5));
            assert_point(absolute_curves[0].p0, (9.0, 0.0));
            assert_point(absolute_curves[0].p3, (12.0, 1.0));
        }
    }

    #[test]
    fn scallops_have_inward_transverse_handles_at_all_four_corners() {
        let radii = [1.0, 0.75, 0.25, 0.5];
        let bytes = record(6.0, 4.0, [1.0, 1.0], radii.map(|radius| (1, radius * 254000.0, 1)));
        for version in [1500, 1600, 2700] {
            let (path, warning) = rectangle(&bytes, version).unwrap();
            assert!(!warning);
            assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 6.0, 4.0));
            let curves = cubics(&path);
            assert_eq!(curves.len(), 4);
            assert_point(curves[0].p1, (5.0, 0.55));
            assert_point(curves[0].p2, (5.45, 1.0));
            assert_point(curves[0].eval(0.5), (5.29375, 0.70625));
            assert_point(curves[1].p1, (5.5875, 3.25));
            assert_point(curves[1].p2, (5.25, 3.5875));
            assert_point(curves[2].p1, (0.25, 3.8625));
            assert_point(curves[2].p2, (0.1375, 3.75));
            assert_point(curves[3].p1, (0.275, 0.5));
            assert_point(curves[3].p2, (0.5, 0.275));
            // This point is inside the corresponding round rectangle but lies
            // in the circular cutout of the lower-right scalloped corner.
            assert_eq!(path.winding(Point::new(5.5, 0.5)), 0);
            assert_ne!(path.winding(Point::new(5.1, 0.9)), 0);
        }
    }

    #[test]
    fn chamfers_remove_four_straight_triangular_corners() {
        let radii = [1.0, 0.75, 0.25, 0.5];
        let bytes = record(6.0, 4.0, [1.0, 1.0], radii.map(|radius| (1, radius * 254000.0, 2)));
        let (path, warning) = rectangle(&bytes, 1600).unwrap();
        assert!(!warning);
        assert!(cubics(&path).is_empty());
        assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 6.0, 4.0));
        // 6 * 4 minus four right triangles, with four distinct leg lengths.
        assert!((path.area() - 23.0625).abs() < 1e-12);
        assert_eq!(path.elements().len(), 10);
        assert_eq!(path.winding(Point::new(5.75, 0.5)), 0);
        assert_ne!(path.winding(Point::new(5.25, 0.5)), 0);
        assert_eq!(path.elements().last(), Some(&PathEl::ClosePath));
    }

    #[test]
    fn ordinary_zeroed_corner_slots_are_unwarned_square_rectangles() {
        for scales in [[1.0, 1.0], [1.5, 0.5], [0.0, 0.0]] {
            let bytes = record(6.0, 4.0, scales, [(0, 0.0, 0); 4]);
            assert!(bytes[32..].iter().all(|b| *b == 0));
            let (path, warning) = rectangle(&bytes, 2700).unwrap();
            assert!(!warning);
            assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 6.0, 4.0));
            assert_eq!(path.elements().len(), 5);
            assert!(cubics(&path).is_empty());
            assert_ne!(path.winding(Point::new(5.99, 0.01)), 0);
        }
    }

    #[test]
    fn relative_radii_ignore_unused_scales_and_bound_extreme_fractions() {
        for treatment in [0, 1, 2] {
            let bytes = record(8.0, 5.0, [0.0, 0.0], [(0, 0.4, treatment); 4]);
            let (path, warning) = rectangle(&bytes, 1600).unwrap();
            assert!(!warning);
            assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 8.0, 5.0));

            let huge = record(8.0, 5.0, [1.0, 1.0], [(0, f64::MAX, treatment); 4]);
            let (path, warning) = rectangle(&huge, 1600).unwrap();
            assert!(warning);
            assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 8.0, 5.0));
            assert!(path.elements().len() <= 10);
        }
        for radius in [-0.1, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let bytes = record(8.0, 5.0, [1.0, 1.0], [(0, radius, 0); 4]);
            assert!(rectangle(&bytes, 1600).is_err());
        }
    }

    #[test]
    fn signed_dimensions_reflect_the_same_corner_geometry() {
        let radii = [1.0, 0.75, 0.25, 0.5];
        for treatment in [0, 1, 2] {
            let corners = radii.map(|radius| (1, radius * 254000.0, treatment));
            let (positive, _) = rectangle(&record(6.0, 4.0, [1.0, 1.0], corners), 2700).unwrap();
            for (x, y) in [(-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                let (path, warning) = rectangle(&record(6.0 * x, 4.0 * y, [1.0, 1.0], corners), 2700).unwrap();
                assert!(!warning);
                let reflected = Affine::new([x, 0.0, 0.0, y, 0.0, 0.0]) * positive.clone();
                assert_eq!(path, reflected);
            }
        }
    }

    #[test]
    fn unknown_modes_treatments_and_reserved_fields_keep_warnings() {
        let original = modern(6.0, 4.0, [1.0, 1.0], [0.5; 4]);
        let (square, _) = rectangle(&modern(6.0, 4.0, [1.0, 1.0], [0.0; 4]), 1600).unwrap();
        for offset in [32, 36, 48, 52, 56, 60, 72, 76, 80, 84, 96, 100, 104, 108, 120, 124] {
            let mut bytes = original.clone();
            bytes[offset] ^= 0x80;
            let (path, warning) = rectangle(&bytes, 1600).unwrap();
            assert!(warning, "unknown metadata at {offset}");
            assert_eq!(path, square);
        }
        let mut extended = original;
        extended.push(1);
        let (path, warning) = rectangle(&extended, 1600).unwrap();
        assert!(warning);
        assert_eq!(path, square);
    }

    #[test]
    fn oversized_radii_are_bounded_and_reported_as_approximations() {
        let (path, warning) = rectangle(&modern(8.0, 4.0, [1.0, 1.0], [10.0, 1.0, 0.5, 0.0]), 1500).unwrap();
        assert!(warning);
        assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 8.0, 4.0));
        let curves = cubics(&path);
        assert_point(curves[0].p0, (6.0, 0.0));
        assert_point(curves[0].p3, (8.0, 2.0));
        assert_point(curves[1].p3, (7.0, 4.0));
        assert_ne!(path.winding(Point::new(4.0, 2.0)), 0);

        let mut huge = modern(8.0, 4.0, [1.0, 1.0], [0.0; 4]);
        for offset in [40, 64, 88, 112] {
            huge[offset..offset + 8].copy_from_slice(&f64::MAX.to_le_bytes());
        }
        let (path, warning) = rectangle(&huge, 1500).unwrap();
        assert!(warning);
        assert!(path.elements().len() <= 10);
        assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 8.0, 4.0));
    }

    #[test]
    fn modern_records_require_all_four_complete_corner_slots() {
        let bytes = modern(6.0, 4.0, [1.0, 1.0], [0.5; 4]);
        assert_eq!(bytes.len(), 128);
        for len in 0..bytes.len() {
            assert!(rectangle(&bytes[..len], 1600).is_err(), "accepted prefix length {len}");
        }
    }

    #[test]
    fn nonfinite_dimensions_scales_and_radii_are_errors() {
        let original = modern(6.0, 4.0, [1.0, 1.0], [0.5; 4]);
        for offset in [0, 8, 16, 24, 40, 64, 88, 112] {
            for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
                let mut bytes = original.clone();
                bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
                assert!(rectangle(&bytes, 2700).is_err(), "accepted {value} at {offset}");
            }
        }
        for offset in [40, 64, 88, 112] {
            let mut bytes = original.clone();
            bytes[offset..offset + 8].copy_from_slice(&(-1.0f64).to_le_bytes());
            assert!(rectangle(&bytes, 2700).is_err(), "accepted negative radius at {offset}");
        }
    }

    #[test]
    fn zero_and_overflowing_scales_cannot_create_invalid_corner_points() {
        for scales in [[0.0, 1.0], [1.0, -0.0], [f64::from_bits(1), 1.0]] {
            assert!(rectangle(&modern(6.0, 4.0, scales, [0.5; 4]), 1600).is_err());
        }
        // A square rectangle never divides by its otherwise unused zero scale.
        let (path, warning) = rectangle(&modern(6.0, 4.0, [0.0, 0.0], [0.0; 4]), 1600).unwrap();
        assert!(!warning);
        assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 6.0, 4.0));
        assert!(cubics(&path).is_empty());
    }

    #[test]
    fn collapsed_dimensions_remain_finite_and_warn_when_rounding_is_lost() {
        for (width, height) in [(0.0, 4.0), (6.0, 0.0), (0.0, 0.0)] {
            let (path, warning) = rectangle(&modern(width, height, [1.0, 1.0], [0.5; 4]), 1600).unwrap();
            assert!(warning);
            assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, width, height));
            assert!(cubics(&path).is_empty());
        }
    }

    #[test]
    fn legacy_layouts_keep_signed_rectangles_and_corner_warnings() {
        for version in [600, 899, 900, 1499] {
            let mut bytes = Vec::new();
            bytes.extend_from_slice(&508000i32.to_le_bytes());
            bytes.extend_from_slice(&(-254000i32).to_le_bytes());
            bytes.resize(if version >= 900 { 24 } else { 12 }, 0);
            let (square, warning) = rectangle(&bytes, version).unwrap();
            assert!(!warning);
            assert_eq!(square.bounding_box(), Rect::new(0.0, -1.0, 2.0, 0.0));
            for len in 0..bytes.len() {
                assert!(rectangle(&bytes[..len], version).is_err());
            }
            let last_radius = bytes.len() - 4;
            bytes[last_radius..].copy_from_slice(&10000i32.to_le_bytes());
            let (path, warning) = rectangle(&bytes, version).unwrap();
            assert!(warning);
            assert_eq!(path, square);
            assert!(cubics(&path).is_empty());
        }
    }
}
