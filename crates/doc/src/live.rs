//! Live blends, envelope distortions and gradient meshes: their data and pure evaluation.
//!
//! These are *live* objects. The document stores the source objects plus parameters; the
//! renderer and the exporters evaluate them on demand ([`expand_live`], [`expand_deep`]).
//! Everything here is pure geometry so the SVG/PDF exporters (which only depend on this crate)
//! can export the expanded form.
//!
//! - **Blend**: key objects are matched anchor-by-anchor (subpaths resampled to equal anchor
//!   counts), paints/strokes/opacity interpolated; optional spine (keys spread along it by arc
//!   length, optionally rotated to its tangent).
//! - **Envelope**: all descendant geometry of the content is pushed through a point map from the
//!   content's bounding box to a warp style, a Catmull-Rom point mesh or a top object (Coons
//!   patch built from the top path's outline split at its corners).
//! - **Gradient mesh**: a grid of points with colours and tangent handles; each patch is a Coons
//!   patch coloured by bilinear interpolation of its four corners.

use std::f64::consts::PI;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_color::{Color, Paint};
use vectorcraft_geom::{Affine, Anchor, BezPath, CubicBez, ParamCurve, PathData, Point, Rect, Shape, SubPath, Vec2};

use crate::appearance::{Appearance, AppearanceItem, FillLayer};
use crate::node::{ImageObject, Node, NodeId, NodeKind};

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}
fn fifty() -> f64 {
    50.0
}

// =====================================================================================
// Blends (evaluation in `crate::blend`)
// =====================================================================================

pub use crate::blend::{
    Spine, blend_expand, blend_spine, blend_step_count, lerp_appearance, lerp_color, lerp_node, lerp_paint, lerp_path, pin_spine,
    steps_knockout_shows,
};

mod blend_spec;
mod envelope;
mod expand;
mod mesh;
mod warp;

pub use blend_spec::{BlendDefaults, BlendEase, BlendEaseType, BlendOrientation, BlendSpacing, BlendSpec, BlendSpineMode};
#[cfg(test)]
use blend_spec::{blend_color_shift, blend_ramp};
pub use envelope::{
    EnvelopeKind, EnvelopeMap, EnvelopeOptions, H_DOWN, H_LEFT, H_RIGHT, H_UP, PreserveShape, TypeInk, envelope_bounds, envelope_content_bounds,
    envelope_content_boxes, envelope_expand, envelope_frame, envelope_grid, envelope_mapper, envelope_of, envelope_overlay, envelope_stroke_margin,
    set_type_ink, smooth_handles, warp_grid,
};
pub(crate) use expand::max_outset;
pub use expand::{
    Hooks, MAX_GENERATED, Outliner, PatternTiles, affine_near, expand_deep, expand_deep_hooks, expand_live, expand_live_hooks, expand_live_with,
    expanded_group, expanded_group_hooks, is_live, mesh_quad_nodes, mesh_warp_cost, mesh_warp_image, subtree_has_live,
};
#[cfg(test)]
use mesh::grid_eval;
pub use mesh::{Coons, GradientMesh, MeshAppearance, MeshPiece, MeshPoint, MeshQuad, default_fidelity, grid_points, nodes_bounds};
pub use warp::{WARP_NET, WarpNet, WarpStyle, fit_piece, map_nonlinear, map_nonlinear_within, pieces, uses_net, warp_point, warp_version};

#[cfg(test)]
mod tests {
    use super::*;

    /// A bad Ramp or Shift in a file clamps (non-finite values take the default) instead of
    /// failing the whole document.
    #[test]
    fn blend_easing_numbers_clamp_when_read() {
        use serde::de::value::{Error, F64Deserializer};
        let spec: BlendSpec = serde_json::from_str(r#"{"ease":{"kind":"out","ramp":7.5},"colorShift":-900}"#).unwrap();
        assert_eq!((spec.ease.ramp, spec.color_shift), (1.0, -100.0));
        let spec: BlendSpec = serde_json::from_str(r#"{"ease":{"ramp":-3},"colorShift":250}"#).unwrap();
        assert_eq!((spec.ease.ramp, spec.color_shift), (0.0, 100.0));
        let spec: BlendSpec = serde_json::from_str(r#"{"ease":{"ramp":null},"colorShift":"NaN"}"#).unwrap();
        assert_eq!((spec.ease.ramp, spec.color_shift), (0.0, 0.0));
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(blend_ramp(F64Deserializer::<Error>::new(bad)).unwrap(), 0.0);
            assert_eq!(blend_color_shift(F64Deserializer::<Error>::new(bad)).unwrap(), 0.0);
        }
    }

    /// #3578: a non-linear map within an anchor budget splits its segments into longer pieces,
    /// down to one a segment, and matches the plain map where the budget leaves room.
    #[test]
    fn map_nonlinear_within_keeps_to_its_anchor_budget() {
        let square = vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
        let f = |p: Point| Point::new(p.x + (p.y / 10.0).sin(), p.y);
        let full = map_nonlinear(&square, 1.0, f);
        assert_eq!(full.anchor_count(), 4 * 64);
        assert_eq!(map_nonlinear_within(&square, 1.0, 4 * 64, f), full);
        // Every side takes the same pieces: of 400 / (max - 4) pt, so 12 a side for 50 anchors.
        for (max, want) in [(0, 4), (4, 4), (5, 4), (50, 4 * 12), (255, 4 * 63)] {
            assert_eq!(map_nonlinear_within(&square, 1.0, max, f).anchor_count(), want, "{max}");
        }
        // Lengths that are not numbers stay one piece; with no room every segment is one piece.
        assert_eq!(fit_piece(&[f64::NAN, 10.0], 1.0, 64, 5), 2.0);
        assert_eq!(fit_piece(&[f64::INFINITY, 10.0], 1.0, 64, 0), f64::INFINITY);
        assert_eq!(pieces(f64::INFINITY, f64::INFINITY, 64), 1);
        assert_eq!(pieces(10.0, 1.0, 0), 1);
    }

    /// #1578: Arc bends in the box's real proportions. In half-width units (y scaled by the
    /// aspect), the top and bottom edges are concentric circles about the bend's centre, at their
    /// real distance from the centre line; an inner edge that would pass the centre stops there.
    #[test]
    fn arc_warp_is_round_in_real_proportions() {
        for (aspect, b) in [(0.25, 0.1), (0.25, 0.5), (0.25, 1.0), (1.0, 0.5), (2.0, 0.3)] {
            let r0 = 2.0 / (b * PI);
            for (y, r) in [(-1.0, r0 + aspect), (1.0, (r0 - aspect).max(0.0))] {
                for i in 0..=10 {
                    let x = -1.0 + 0.2 * f64::from(i);
                    let (nx, ny) = warp_point(WarpStyle::Arc, b, 0.0, 0.0, aspect, x, y);
                    let d = nx.hypot(ny * aspect - r0);
                    assert!((d - r).abs() < 1e-9, "aspect {aspect}, bend {b}, y {y}, x {x}: {d} vs {r}");
                }
            }
        }
        // At 100 % on a square the inner edge meets at the centre instead of folding over.
        let (nx, _) = warp_point(WarpStyle::Arc, 1.0, 0.0, 0.0, 1.0, 0.8, 1.0);
        assert!(nx.abs() < 1e-12);
        // A square bent less than that is what it always was.
        let (nx, ny) = warp_point(WarpStyle::Arc, 0.5, 0.0, 0.0, 1.0, 0.5, 0.25);
        let (r0, a) = (4.0 / PI, 0.5 / (4.0 / PI));
        assert!((nx - (r0 - 0.25) * a.sin()).abs() < 1e-12 && (ny - (r0 - (r0 - 0.25) * a.cos())).abs() < 1e-12);
    }

    /// Each warp style's Bézier net, at points worked out by a separate implementation of the same
    /// nets: (style, bend %, horizontal and vertical distortion %, vertical, box w × h, (u, v)) →
    /// the point in the box (origin at its top left, y down).
    #[test]
    fn warp_nets_bend_each_style_as_its_own_patch() {
        #[rustfmt::skip]
        let samples = [
            ("arc", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, -31.1605665, 30.1243045),
            ("arc", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 170.241998, 58.8743443),
            ("arc", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 50.6087118, 47.6275948),
            ("arcLower", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, 18.8784581, 35.5028749),
            ("arcLower", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 164.486168, 119.348666),
            ("arcLower", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 38.1073767, 36.0911629),
            ("arcUpper", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, 17.3830689, 17.1599587),
            ("arcUpper", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 160.498463, 86.7390371),
            ("arcUpper", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 46.1081993, 36.0252855),
            ("arch", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, 16.261527, 11.6570838),
            ("arch", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 164.984631, 57.3903712),
            ("arch", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 49.2508226, 36.2064484),
            ("bulge", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, 16.261527, 22.6628335),
            ("bulge", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 164.984631, 116.087703),
            ("bulge", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 42.965576, 36.2064484),
            ("shellLower", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, 10.6489723, 25.9914004),
            ("shellLower", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 208.565464, 99.4817585),
            ("shellLower", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 41.043987, 36.1302767),
            ("shellUpper", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, -21.8346746, 41.6042708),
            ("shellUpper", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 160.791499, 90.3680631),
            ("shellUpper", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 42.9282896, 36.1475509),
            ("flag", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, 20.0, 55.92),
            ("flag", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 160.0, 55.44),
            ("flag", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 58.098, 35.91),
            ("wave", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, 20.0, 40.8864),
            ("wave", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 160.0, 83.7792),
            ("wave", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 42.33, 35.91),
            ("fish", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, 20.0, 19.632),
            ("fish", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 160.0, 62.352),
            ("fish", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 44.994, 35.91),
            ("rise", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, 20.0, 146.64),
            ("rise", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 160.0, 102.48),
            ("rise", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 118.686, 35.91),
            ("fisheye", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, 9.1136, 27.2784),
            ("fisheye", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 166.2208, 94.1472),
            ("fisheye", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 47.514, 35.7048),
            ("inflate", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, -0.16, 27.84),
            ("inflate", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 166.48, 97.68),
            ("inflate", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 45.75, 36.54),
            ("squeeze", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, 40.16, 27.84),
            ("squeeze", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 153.52, 97.68),
            ("squeeze", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 45.75, 36.0),
            ("twist", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.1, 0.3, 32.2472, 27.9588),
            ("twist", 60.0, 0.0, 0.0, false, 200.0, 100.0, 0.8, 0.9, 149.1136, 89.2224),
            ("twist", -40.0, 30.0, -50.0, true, 150.0, 60.0, 0.25, 0.6, 45.56325, 33.8769),
        ];
        for (style, bend, dh, dv, vertical, w, h, u, v, x, y) in samples {
            let Some(st) = WarpStyle::from_id(style) else { panic!("{style}") };
            let p = WarpNet::new(st, bend / 100.0, dh / 100.0, dv / 100.0, w, h, vertical).at(u, v);
            assert!(p.distance(Point::new(x, y)) < 1e-6, "{style} {bend} {dh} {dv} {vertical} ({u}, {v}): {p:?} vs ({x}, {y})");
        }
    }

    /// Arc's edges are concentric circles: at 100 % on a 200 × 100 box the bottom edge is the half
    /// circle through the bottom corners about the bottom's middle, and the top edge the one 100
    /// further out. One cubic for a half circle strays up to 1.84 % of the radius from the circle.
    #[test]
    fn warp_net_arc_edges_are_concentric_circles() {
        let net = WarpNet::new(WarpStyle::Arc, 1.0, 0.0, 0.0, 200.0, 100.0, false);
        let center = Point::new(100.0, 100.0);
        for i in 0..=20 {
            let u = f64::from(i) / 20.0;
            for (v, r) in [(0.0, 200.0), (1.0, 100.0)] {
                let d = net.at(u, v).distance(center);
                assert!((d - r).abs() < 0.0184 * r, "u {u}, v {v}: {d} from the center, not {r}");
                // Its ends and middle lie on the circle.
                if i % 10 == 0 {
                    assert!((d - r).abs() < 1e-9, "u {u}, v {v}: {d} from the center, not {r}");
                }
            }
        }
        // Bend 0 leaves the box as it is.
        for st in [WarpStyle::Arc, WarpStyle::ShellLower, WarpStyle::Bulge, WarpStyle::Twist] {
            let flat = WarpNet::new(st, 0.0, 0.0, 0.0, 200.0, 100.0, false);
            assert!(flat.at(0.3, 0.7).distance(Point::new(60.0, 70.0)) < 1e-9, "{st:?}");
        }
    }

    /// The horizontal distortion scales each column of the net about its middle (narrowing the left
    /// side for a positive value), then the vertical one each row (narrowing the top).
    #[test]
    fn warp_net_distortion_scales_the_columns_then_the_rows() {
        let corners = |dh: f64, dv: f64| {
            let net = WarpNet::new(WarpStyle::Arc, 0.0, dh, dv, 200.0, 100.0, false);
            [net.at(0.0, 0.0), net.at(1.0, 0.0), net.at(0.0, 1.0), net.at(1.0, 1.0)]
        };
        let near = |got: [Point; 4], want: [(f64, f64); 4]| got.iter().zip(want).all(|(p, (x, y))| p.distance(Point::new(x, y)) < 1e-9);
        assert!(near(corners(0.5, 0.0), [(0.0, 25.0), (200.0, -25.0), (0.0, 75.0), (200.0, 125.0)]), "{:?}", corners(0.5, 0.0));
        assert!(near(corners(0.0, 0.5), [(50.0, 0.0), (150.0, 0.0), (-50.0, 100.0), (250.0, 100.0)]), "{:?}", corners(0.0, 0.5));
        // Both coordinates scale: after the columns the top row runs from (0, 15) to (200, -15), and
        // it then scales by 1.4 about its middle, (100, 0).
        assert!(near(corners(0.3, -0.4), [(-40.0, 21.0), (240.0, -21.0), (40.0, 91.0), (160.0, 109.0)]), "{:?}", corners(0.3, -0.4));
    }

    /// Numbers no warp can take still give a finite net: a bend or distortion that isn't a number
    /// counts as 0, and out of range as its limit; a size that isn't a number as 0.
    #[test]
    fn warp_net_takes_any_numbers() {
        for st in [WarpStyle::Arc, WarpStyle::ShellUpper, WarpStyle::Wave, WarpStyle::Inflate, WarpStyle::Twist] {
            for (b, dh, w, h) in
                [(f64::NAN, 0.2, 10.0, 10.0), (5.0, f64::INFINITY, 10.0, 0.0), (-1e300, -3.0, f64::NAN, 1e-300), (1e-300, 0.0, 0.0, 0.0)]
            {
                for vertical in [false, true] {
                    let p = WarpNet::new(st, b, dh, dh, w, h, vertical).at(0.3, 0.6);
                    assert!(p.x.is_finite() && p.y.is_finite(), "{st:?} {b} {dh} {w} {h}: {p:?}");
                }
            }
        }
        let clamped = WarpNet::new(WarpStyle::Flag, 7.0, 0.0, 0.0, 200.0, 100.0, false);
        assert_eq!(clamped, WarpNet::new(WarpStyle::Flag, 1.0, 0.0, 0.0, 200.0, 100.0, false));
    }

    /// A box near the largest finite size still warps: Flag on a 1e308 × 100 box moves points only
    /// up and down, as on a 1 × 1e-306 one scaled up, and on a 1e308 × 1e-300 box keeps the box's
    /// height. Fisheye at 100 % on a 1.7e308 square, whose net goes past the largest finite number,
    /// gives finite points inside the box and past it, and the unit square's points scaled up where
    /// those are finite. Fisheye moves y by the height alone, so on boxes of the largest finite
    /// width and a height of 1e-300 or 100 every y is the unit square's times the height, also
    /// where x goes past the largest finite number.
    #[test]
    fn warp_net_bends_a_huge_box_without_overflowing() {
        let w = 1e308;
        let flag = WarpNet::new(WarpStyle::Flag, 0.5, 0.0, 0.0, w, 100.0, false);
        let small = WarpNet::new(WarpStyle::Flag, 0.5, 0.0, 0.0, 1.0, 100.0 / w, false);
        for (u, v) in [(0.0, 0.0), (0.3, 0.6), (0.9, 1.0), (1.0, 0.5)] {
            let (p, q) = (flag.at(u, v), small.at(u, v));
            assert!(p.is_finite(), "({u}, {v}): {p:?}");
            assert!((p.x - u * w).abs() <= 1e-12 * w, "({u}, {v}): {p:?}");
            assert!((p.y - q.y * w).abs() <= 1e-9 * p.y.abs().max(1.0), "({u}, {v}): {p:?} against {q:?}");
        }
        let thin = WarpNet::new(WarpStyle::Flag, 0.5, 0.0, 0.0, w, 1e-300, false);
        assert_eq!(thin.at(0.0, 1.0), Point::new(0.0, 1e-300));
        let fisheye = WarpNet::new(WarpStyle::Fisheye, 1.0, 0.0, 0.0, 1.7e308, 1.7e308, false);
        for (u, v) in [(0.1, 0.1), (0.8, 0.3), (1.5, 0.5), (3.0, 3.0)] {
            let p = fisheye.at(u, v);
            assert!(p.is_finite(), "({u}, {v}): {p:?}");
        }
        let unit = WarpNet::new(WarpStyle::Fisheye, 1.0, 0.0, 0.0, 1.0, 1.0, false);
        let near = |a: f64, b: f64| (a - b).abs() <= 1e-12 * b.abs().max(f64::MIN_POSITIVE);
        let (p, q) = (fisheye.at(0.1, 0.1), unit.at(0.1, 0.1));
        assert!(near(p.x, q.x * 1.7e308) && near(p.y, q.y * 1.7e308), "{p:?} against {q:?}");
        // The bottom of a box with a subnormal height stays exactly at that height: the net is
        // built on the box halved, and the height's lowest bit but one is set.
        let low = f64::MIN_POSITIVE / 4.0 + f64::from_bits(2);
        let sub = WarpNet::new(WarpStyle::Fisheye, 1.0, 0.0, 0.0, f64::MAX, low, false).at(0.0, 1.0);
        assert_eq!(sub.y, low, "{sub:?}");
        for h in [1e-300, 100.0] {
            let tall = WarpNet::new(WarpStyle::Fisheye, 1.0, 0.0, 0.0, f64::MAX, h, false);
            for (u, v) in [(0.1, 0.1), (0.9, 0.4), (0.5, 0.8)] {
                let (p, q) = (tall.at(u, v), unit.at(u, v));
                assert!(p.is_finite() && near(p.y, q.y * h), "h {h}, ({u}, {v}): {p:?} against {q:?}");
            }
        }
    }

    /// Content with no width or no height takes the middle of a warp envelope's patch along that
    /// side: under Arc a vertical line stays vertical, under Bulge a horizontal line stays straight.
    #[test]
    fn warp_envelopes_bend_flat_content_as_the_middle_of_the_patch() {
        let warp = |style: &str| EnvelopeKind::Warp { style: style.into(), bend: 50.0, h: 0.0, v: 0.0, horizontal: true, version: WARP_NET };
        let arc = warp("arc");
        let map = envelope_mapper(&arc, Rect::new(100.0, 0.0, 100.0, 100.0));
        for y in [0.0, 30.0, 100.0] {
            let p = map(Point::new(100.0, y));
            assert!((p.x - 100.0).abs() <= 1e-9, "{y}: {p:?}");
        }
        let bulge = warp("bulge");
        let map = envelope_mapper(&bulge, Rect::new(0.0, 50.0, 200.0, 50.0));
        for x in [0.0, 70.0, 200.0] {
            let p = map(Point::new(x, 50.0));
            assert!((p.y - 50.0).abs() <= 1e-9, "{x}: {p:?}");
        }
    }

    /// A warp envelope's map version: none in a file reads as 0 (the earlier map), 0 is written
    /// as none, so files keep their bytes, and the nets' version is written.
    #[test]
    fn warp_envelopes_write_their_map_version_only_when_it_has_one() {
        let old: EnvelopeKind = serde_json::from_str(r#"{"type":"warp","style":"arc","bend":20}"#).unwrap();
        assert!(matches!(old, EnvelopeKind::Warp { version: 0, .. }), "{old:?}");
        assert!(!serde_json::to_string(&old).unwrap().contains("version"));
        let new = EnvelopeKind::Warp { style: "arc".into(), bend: 20.0, h: 0.0, v: 0.0, horizontal: true, version: WARP_NET };
        let text = serde_json::to_string(&new).unwrap();
        assert!(text.contains(r#""version":1"#), "{text}");
        assert_eq!(serde_json::from_str::<EnvelopeKind>(&text).unwrap(), new);
    }
    use vectorcraft_geom::shapes;

    fn rect_node(x: f64, y: f64, w: f64, h: f64, c: Color) -> Arc<Node> {
        Arc::new(Node::path(NodeId(1), shapes::rectangle(Rect::new(x, y, x + w, y + h)), Appearance::basic(Paint::solid(c), Paint::None, 0.0)))
    }

    #[test]
    fn lerp_path_equalises_anchor_counts() {
        let a = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = shapes::ellipse(Rect::new(0.0, 0.0, 10.0, 10.0));
        let t = lerp_path(&a, &shapes::polygon(Point::new(5.0, 5.0), 5.0, 7, 0.0), 0.5);
        assert_eq!(t.subpaths[0].anchors.len(), 7);
        let m = lerp_path(&a, &b, 0.0);
        assert_eq!(m.subpaths[0].anchors.len(), 4);
    }

    #[test]
    fn blend_steps_and_colours() {
        let keys = vec![rect_node(0.0, 0.0, 10.0, 10.0, Color::BLACK), rect_node(100.0, 0.0, 10.0, 10.0, Color::WHITE)];
        let out = blend_expand(&keys, &BlendSpec { spacing: BlendSpacing::Steps(3), ..Default::default() });
        assert_eq!(out.len(), 5);
        let mid = out[2].appearance.fill_paint().color().unwrap().to_rgb()[0];
        assert!((mid - 0.5).abs() < 1e-3);
    }

    #[test]
    fn coons_rect_is_identity() {
        let c = Coons::from_path(&shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 50.0))).unwrap();
        let p = c.eval(0.25, 0.5);
        assert!((p.x - 25.0).abs() < 1e-6 && (p.y - 25.0).abs() < 1e-6, "{p:?}");
    }

    #[test]
    fn maps_go_on_past_the_content_box() {
        // A stroke outline reaches past the box: a flat mesh and a rectangle stay the identity there.
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let p = grid_eval(2, 2, &grid_points(r, 2, 2), -0.05, 1.1);
        assert!((p.x + 5.0).abs() < 1e-6 && (p.y - 110.0).abs() < 1e-6, "{p:?}");
        let c = Coons::from_path(&shapes::rectangle(r)).unwrap().eval(1.05, -0.1);
        assert!((c.x - 105.0).abs() < 1e-6 && (c.y + 10.0).abs() < 1e-6, "{c:?}");
    }

    /// A 100 × 50 outline whose top side runs (0,0) → (30,0) → (100,0) as two straight segments.
    fn top_split_at_30() -> PathData {
        let a = |x: f64, y: f64| Anchor::corner(Point::new(x, y));
        PathData::single(SubPath::new(vec![a(0.0, 0.0), a(30.0, 0.0), a(100.0, 0.0), a(100.0, 50.0), a(0.0, 50.0)], true))
    }

    /// The length of `c`'s control polygon.
    fn polygon(c: CubicBez) -> f64 {
        c.p0.distance(c.p1) + c.p1.distance(c.p2) + c.p2.distance(c.p3)
    }

    /// An anchor and its incoming and outgoing handles.
    type Knot = ((f64, f64), (f64, f64), (f64, f64));

    /// A closed path through `pts`.
    fn outline(pts: &[Knot]) -> PathData {
        let a = |&(p, i, o): &Knot| Anchor { p: p.into(), h_in: i.into(), h_out: o.into(), ..Anchor::corner(p.into()) };
        PathData::single(SubPath::new(pts.iter().map(a).collect(), true))
    }

    #[test]
    fn a_top_object_side_of_straight_segments_spreads_by_length() {
        // The top side runs (0,0) → (30,0) → (100,0): each straight piece takes its length's share.
        let kind = EnvelopeKind::TopObject { path: top_split_at_30(), corners: None };
        let map = envelope_mapper(&kind, Rect::new(0.0, 0.0, 100.0, 50.0));
        for (q, want) in [((50.0, 0.0), (50.0, 0.0)), ((15.0, 0.0), (15.0, 0.0)), ((80.0, 0.0), (80.0, 0.0)), ((50.0, 25.0), (50.0, 25.0))] {
            let p = map(Point::new(q.0, q.1));
            assert!(p.distance(Point::new(want.0, want.1)) < 1e-6, "{q:?} → {p:?}, want {want:?}");
        }
    }

    #[test]
    fn a_top_object_shares_a_side_by_control_polygon_length() {
        // Two curves of unequal polygons make the top side; the anchor between them lands at the
        // first one's share, and inside each curve the content follows its parameter.
        let first = CubicBez::new((0.0, 0.0), (10.0, -20.0), (30.0, -20.0), (40.0, 0.0));
        let second = CubicBez::new((40.0, 0.0), (60.0, 10.0), (90.0, 10.0), (100.0, 0.0));
        let path = outline(&[
            ((0.0, 0.0), (0.0, 0.0), (10.0, -20.0)),
            ((40.0, 0.0), (30.0, -20.0), (60.0, 10.0)),
            ((100.0, 0.0), (90.0, 10.0), (100.0, 0.0)),
            ((100.0, 80.0), (100.0, 80.0), (100.0, 80.0)),
            ((0.0, 80.0), (0.0, 80.0), (0.0, 80.0)),
        ]);
        let c = Coons::from_top_object(&path, None).unwrap();
        let share = polygon(first) / (polygon(first) + polygon(second));
        assert!(c.eval(share, 0.0).distance(Point::new(40.0, 0.0)) < 1e-9, "{:?}", c.eval(share, 0.0));
        for f in [0.25, 0.5, 0.75] {
            let p = c.eval(share * f, 0.0);
            assert!(p.distance(first.eval(f)) < 1e-9, "first curve at {f}: {p:?}");
            let p = c.eval(share + (1.0 - share) * f, 0.0);
            assert!(p.distance(second.eval(f)) < 1e-9, "second curve at {f}: {p:?}");
        }
    }

    #[test]
    fn a_rounded_rectangle_top_object_stays_close_to_the_identity() {
        // The corners fall at the middle of each corner arc. Each half arc takes the share of its
        // control polygon, so the straight edges stay close to where the content was.
        let r = Rect::new(0.0, 0.0, 200.0, 100.0);
        let path = shapes::rounded_rectangle(r, 10.0);
        let c = Coons::from_top_object(&path, None).unwrap();
        let [tl, tr, br, bl] = c.corners();
        let k = 10.0 * (1.0 - std::f64::consts::FRAC_1_SQRT_2);
        for (p, want) in [(tl, (k, k)), (tr, (200.0 - k, k)), (br, (200.0 - k, 100.0 - k)), (bl, (k, 100.0 - k))] {
            assert!(p.distance(Point::from(want)) < 0.01, "corner {p:?}, want {want:?}");
        }
        for (u, v) in [(0.25, 0.0), (0.5, 0.0), (0.8, 1.0), (0.0, 0.5), (0.5, 0.5), (1.0, 0.3)] {
            let p = c.eval(u, v);
            assert!(p.distance(Point::new(200.0 * u, 100.0 * v)) < 3.0, "({u}, {v}) → {p:?}");
        }
        // The top edge starts after the first half arc's share.
        let sp = &path.subpaths[0];
        let edge = Point::new(10.0, 0.0);
        let arc = (0..sp.segment_count())
            .filter(|i| !sp.segment_is_line(*i))
            .map(|i| sp.segment(i))
            .find(|s| s.p0.distance(edge) < 1e-9 || s.p3.distance(edge) < 1e-9)
            .unwrap();
        let half = polygon(if arc.p3.distance(edge) < 1e-9 { arc.subsegment(0.5..1.0) } else { arc.subsegment(0.0..0.5) });
        let p = c.eval(half / (2.0 * half + 180.0), 0.0);
        assert!(p.distance(Point::new(10.0, 0.0)) < 1e-6, "{p:?}");
    }

    #[test]
    fn a_top_object_corner_inside_a_bulging_curve_is_the_nearest_outline_point() {
        // The top left anchor's handle bulges out past the corner, so the anchor turns inward
        // there and the corner falls inside the curve.
        // The outline's bounding box starts at (-7.957, -22.222); the top curve's point nearest
        // that corner, found by sampling the curve 2,000,000 times, is (-5.072093, -18.127254).
        let path = outline(&[
            ((0.0, 0.0), (0.0, 0.0), (-50.0, -50.0)),
            ((200.0, 0.0), (150.0, 0.0), (200.0, 0.0)),
            ((200.0, 100.0), (200.0, 100.0), (200.0, 100.0)),
            ((0.0, 100.0), (0.0, 100.0), (0.0, 100.0)),
        ]);
        let want = Point::new(-5.072093, -18.127254);
        let tl = Coons::from_top_object(&path, None).unwrap().corners()[0];
        assert!(tl.distance(want) < 1e-5, "{tl:?}, want {want:?}");
        assert!(tl.distance(Point::ZERO) > 1.0, "{tl:?}");
    }

    #[test]
    fn a_top_object_corner_takes_an_anchor_only_near_its_box_corner() {
        // A chamfered top left corner of a 200 × 100 outline. With both chamfer anchors within
        // 47.5% of the box from its corner, the nearer anchor is the corner.
        let chamfer = |a: (f64, f64), b: (f64, f64)| {
            let c = |p: (f64, f64)| (p, p, p);
            outline(&[c(a), c(b), c((200.0, 0.0)), c((200.0, 100.0)), c((0.0, 100.0))])
        };
        let tl = Coons::from_top_object(&chamfer((0.0, 46.0), (94.0, 0.0)), None).unwrap().corners()[0];
        assert!(tl.distance(Point::new(0.0, 46.0)) < 1e-9, "{tl:?}");
        // Farther out, neither anchor counts: the corner is the chamfer's point nearest the box corner.
        let tl = Coons::from_top_object(&chamfer((0.0, 50.0), (100.0, 0.0)), None).unwrap().corners()[0];
        assert!(tl.distance(Point::new(20.0, 40.0)) < 1e-6, "{tl:?}");
    }

    #[test]
    fn a_top_object_corner_skips_smooth_and_inward_anchors() {
        // A top left anchor whose outgoing handle turns from its incoming one by `deg` degrees
        // (positive turns out of the outline). The top side bulges up past it.
        let shape = |deg: f64| {
            let a = deg.to_radians();
            let (hx, hy) = (50.0 * a.cos() + 40.0 * a.sin(), 40.0 * a.cos() - 50.0 * a.sin());
            outline(&[
                ((50.0, -300.0), (25.0, -280.0), (50.0 + hx, -300.0 - hy)),
                ((250.0, -300.0), (200.0, -340.0), (250.0, -300.0)),
                ((250.0, -200.0), (250.0, -200.0), (250.0, -200.0)),
                ((50.0, -200.0), (50.0, -200.0), (50.0, -230.0)),
            ])
        };
        let corner = |deg: f64| Coons::from_top_object(&shape(deg), None).unwrap().corners()[0];
        let anchor = Point::new(50.0, -300.0);
        for deg in [0.0, 0.4, 0.6, -1.5] {
            assert!(corner(deg).distance(anchor) > 1.0, "{deg}°: {:?}", corner(deg));
        }
        for deg in [0.9, 1.5, 10.0] {
            assert!(corner(deg).distance(anchor) < 1e-9, "{deg}°: {:?}", corner(deg));
        }
    }

    #[test]
    fn a_top_object_corner_skips_an_inward_anchor_between_straight_segments() {
        // A 200 × 100 outline of straight segments with a notch at its top left: (40,40) is the
        // anchor nearest the box corner, but it turns into the outline, so (60,0) is the corner.
        let c = |p: (f64, f64)| (p, p, p);
        let path = outline(&[c((60.0, 0.0)), c((200.0, 0.0)), c((200.0, 100.0)), c((0.0, 100.0)), c((0.0, 60.0)), c((40.0, 40.0))]);
        let tl = Coons::from_top_object(&path, None).unwrap().corners()[0];
        assert!(tl.distance(Point::new(60.0, 0.0)) < 1e-9, "{tl:?}");
    }

    #[test]
    fn a_top_object_corner_tie_goes_to_the_first_anchor() {
        // Both chamfer anchors lie 30 from the box corner: the first one along the outline, in
        // the order it was drawn, wins.
        let c = |p: (f64, f64)| (p, p, p);
        let tl = |pts: &[Knot]| Coons::from_top_object(&outline(pts), None).unwrap().corners()[0];
        let from_left = tl(&[c((0.0, 30.0)), c((30.0, 0.0)), c((100.0, 0.0)), c((100.0, 100.0)), c((0.0, 100.0))]);
        assert!(from_left.distance(Point::new(0.0, 30.0)) < 1e-9, "{from_left:?}");
        let from_top = tl(&[c((30.0, 0.0)), c((100.0, 0.0)), c((100.0, 100.0)), c((0.0, 100.0)), c((0.0, 30.0))]);
        assert!(from_top.distance(Point::new(30.0, 0.0)) < 1e-9, "{from_top:?}");
        // Drawn counter-clockwise, the anchor met first in that order wins.
        let back = tl(&[c((0.0, 100.0)), c((100.0, 100.0)), c((100.0, 0.0)), c((30.0, 0.0)), c((0.0, 30.0))]);
        assert!(back.distance(Point::new(30.0, 0.0)) < 1e-9, "{back:?}");
        let back = tl(&[c((0.0, 30.0)), c((0.0, 100.0)), c((100.0, 100.0)), c((100.0, 0.0)), c((30.0, 0.0))]);
        assert!(back.distance(Point::new(0.0, 30.0)) < 1e-9, "{back:?}");
    }

    #[test]
    fn a_reflected_top_object_keeps_the_corners_a_file_gives() {
        // Reflecting the outline across x = 0 keeps its anchors in the same order, now running
        // counter-clockwise: the corners stay on the same anchors, so the content is reflected too.
        let mut path = top_split_at_30();
        path.transform(Affine::FLIP_X);
        let c = Coons::from_top_object(&path, Some([1, 2, 3, 4])).unwrap();
        let want = [(-30.0, 0.0), (-100.0, 0.0), (-100.0, 50.0), (0.0, 50.0)].map(Point::from);
        assert!(c.corners().iter().zip(want).all(|(p, q)| p.distance(q) < 1e-9), "{:?}", c.corners());
        let p = c.eval(0.5, 0.5);
        assert!(p.distance(Point::new(-50.0, 17.5)) < 1e-9, "{p:?}");
    }

    #[test]
    fn a_top_object_keeps_the_corners_a_file_gives() {
        let same = |a: [Point; 4], b: [Point; 4]| a.iter().zip(b).all(|(p, q)| p.distance(q) < 1e-9);
        let want = [(30.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)].map(Point::from);
        let c = Coons::from_top_object(&top_split_at_30(), Some([1, 2, 3, 4])).unwrap();
        assert!(same(c.corners(), want), "{:?}", c.corners());
        // Drawn the other way round, with the corners counted along that order.
        let mut back = top_split_at_30();
        back.subpaths[0].reverse();
        let c = Coons::from_top_object(&back, Some([3, 2, 1, 0])).unwrap();
        assert!(same(c.corners(), want), "{:?}", c.corners());
        // Corners that repeat, run out of order or go past the outline are found from it instead.
        let found = [(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)].map(Point::from);
        for bad in [[0, 0, 3, 4], [2, 1, 3, 4], [1, 2, 3, 9]] {
            let c = Coons::from_top_object(&top_split_at_30(), Some(bad)).unwrap();
            assert!(same(c.corners(), found), "{bad:?}: {:?}", c.corners());
        }
    }

    /// The outline of a 200 × 100 box whose top side is `pieces`, end to end from (0, 0) to
    /// (200, 0), and whose other sides are straight.
    fn top_of_pieces(pieces: &[CubicBez]) -> PathData {
        let pt = |p: Point| (p.x, p.y);
        let mut knots: Vec<Knot> = vec![];
        for (k, c) in pieces.iter().enumerate() {
            let h_in = if k == 0 { pt(c.p0) } else { pt(pieces[k - 1].p2) };
            knots.push((pt(c.p0), h_in, pt(c.p1)));
        }
        let last = pieces.last().unwrap();
        knots.push(((200.0, 0.0), pt(last.p2), (200.0, 0.0)));
        knots.extend([((200.0, 100.0), (200.0, 100.0), (200.0, 100.0)), ((0.0, 100.0), (0.0, 100.0), (0.0, 100.0))]);
        outline(&knots)
    }

    /// A top curve whose parameter runs fast at its start and slow at its end.
    const UNEVEN_TOP: CubicBez =
        CubicBez { p0: Point::new(0.0, 0.0), p1: Point::new(120.0, -60.0), p2: Point::new(190.0, -30.0), p3: Point::new(200.0, 0.0) };

    #[test]
    fn a_split_curve_keeps_the_top_object_patch() {
        // Splitting the top curve at t = 0.3, or into thirds, keeps its shape, and the patch stays
        // the unsplit curve's: the new anchors land at u = 0.3, 1/3 and 2/3 (to within how finely
        // the side is sampled), where the pieces' control polygons would have put them several
        // points away.
        let whole = Coons::from_top_object(&top_of_pieces(&[UNEVEN_TOP]), None).unwrap();
        for cuts in [vec![0.3], vec![1.0 / 3.0, 2.0 / 3.0]] {
            let ends: Vec<f64> = std::iter::once(0.0).chain(cuts.iter().copied()).chain([1.0]).collect();
            let pieces: Vec<CubicBez> = ends.windows(2).map(|w| UNEVEN_TOP.subsegment(w[0]..w[1])).collect();
            let c = Coons::from_top_object(&top_of_pieces(&pieces), None).unwrap();
            let total: f64 = pieces.iter().map(|p| polygon(*p)).sum();
            for (k, t) in cuts.iter().enumerate() {
                let anchor = pieces[k].p3;
                assert!(c.eval(*t, 0.0).distance(anchor) < 0.02, "{cuts:?}: u {t} → {:?}, want {anchor:?}", c.eval(*t, 0.0));
                let share: f64 = pieces[..=k].iter().map(|p| polygon(*p)).sum::<f64>() / total;
                assert!(c.eval(share, 0.0).distance(anchor) > 2.0, "{cuts:?}: the polygon share {share} lands on the anchor");
            }
            for (u, v) in [(0.1, 0.0), (0.3, 0.2), (0.5, 0.5), (0.62, 0.0), (0.9, 0.7), (0.97, 1.0)] {
                let (p, q) = (c.eval(u, v), whole.eval(u, v));
                assert!(p.distance(q) < 1e-9, "{cuts:?}: ({u}, {v}) → {p:?}, unsplit {q:?}");
            }
        }
    }

    #[test]
    fn a_split_curve_keeps_the_top_object_patch_when_split_at_a_cusp() {
        // The top curve stops at t = 0.25 (a cusp), and an anchor splits it there: both handles at
        // the split lie on the anchor, and the patch with the corners given stays the unsplit
        // curve's.
        let cusp = CubicBez::new((0.0, 0.0), (30.0, -40.0), (-58.0, 24.0), (200.0, 0.0));
        let pieces = [cusp.subsegment(0.0..0.25), cusp.subsegment(0.25..1.0)];
        assert!(pieces[0].p2.distance(pieces[0].p3) < 1e-9 && pieces[1].p1.distance(pieces[1].p0) < 1e-9);
        let whole = Coons::from_top_object(&top_of_pieces(&[cusp]), Some([0, 1, 2, 3])).unwrap();
        let c = Coons::from_top_object(&top_of_pieces(&pieces), Some([0, 2, 3, 4])).unwrap();
        for (u, v) in [(0.1, 0.0), (0.25, 0.0), (0.3, 0.2), (0.5, 0.5), (0.62, 0.0), (0.9, 0.7), (0.97, 1.0)] {
            let (p, q) = (c.eval(u, v), whole.eval(u, v));
            assert!(p.distance(q) < 1e-6, "({u}, {v}) → {p:?}, unsplit {q:?}");
        }
    }

    #[test]
    fn a_top_object_joins_a_split_curve_saved_with_rounded_coordinates() {
        // A file saves the pieces of a curve split into thirds, or into tenths, with its
        // coordinates rounded to 0.001 pt, on an outline 5, 200 or 5,000 pt wide: the side is
        // still that curve, to within 0.02 pt.
        let round = |p: Point| Point::new((p.x * 1000.0).round() / 1000.0, (p.y * 1000.0).round() / 1000.0);
        for cuts in [3u32, 10] {
            let pieces: Vec<CubicBez> =
                (0..cuts).map(|k| UNEVEN_TOP.subsegment(f64::from(k) / f64::from(cuts)..f64::from(k + 1) / f64::from(cuts))).collect();
            for scale in [0.025, 1.0, 25.0] {
                let mut split = top_of_pieces(&pieces);
                split.transform(Affine::scale(scale));
                for a in split.subpaths.iter_mut().flat_map(|sp| sp.anchors.iter_mut()) {
                    (a.p, a.h_in, a.h_out) = (round(a.p), round(a.h_in), round(a.h_out));
                }
                let mut whole = top_of_pieces(&[UNEVEN_TOP]);
                whole.transform(Affine::scale(scale));
                let (c, whole) = (Coons::from_top_object(&split, None).unwrap(), Coons::from_top_object(&whole, None).unwrap());
                for (u, v) in [(1.0 / 6.0, 0.0), (1.0 / 3.0, 0.0), (0.45, 0.0), (0.5, 0.5), (2.0 / 3.0, 0.0), (0.9, 0.3)] {
                    let (p, q) = (c.eval(u, v), whole.eval(u, v));
                    assert!(p.distance(q) < 0.02, "{cuts} pieces at {scale}x: ({u}, {v}) → {p:?}, unsplit {q:?}");
                }
            }
        }
    }

    #[test]
    fn a_top_object_keeps_smoothly_joined_curves_apart() {
        // The top side's second piece leaves the split anchor in line with the first piece's
        // handle, but its far handle is 1 pt off the split curve's: the pieces are two curves, and
        // each takes the share of its control polygon.
        let (a, b) = (UNEVEN_TOP.subsegment(0.0..0.3), UNEVEN_TOP.subsegment(0.3..1.0));
        let b = CubicBez::new(b.p0, b.p1, b.p2 + Vec2::new(0.0, 1.0), b.p3);
        let c = Coons::from_top_object(&top_of_pieces(&[a, b]), None).unwrap();
        let share = polygon(a) / (polygon(a) + polygon(b));
        assert!(c.eval(share, 0.0).distance(a.p3) < 1e-9, "{:?}", c.eval(share, 0.0));
        assert!((share - 0.3).abs() > 0.02, "{share}");
    }

    #[test]
    fn a_top_object_side_partly_of_one_curve_keeps_the_polygon_shares() {
        // The top side is a curve split at t = 0.3, then a straight segment: not all of it is one
        // curve, so each piece keeps the share of its control polygon.
        let curve = CubicBez::new((0.0, 0.0), (90.0, -45.0), (142.5, -22.5), (150.0, 0.0));
        let (a, b) = (curve.subsegment(0.0..0.3), curve.subsegment(0.3..1.0));
        let line = CubicBez::new((150.0, 0.0), (150.0 + 50.0 / 3.0, 0.0), (200.0 - 50.0 / 3.0, 0.0), (200.0, 0.0));
        let c = Coons::from_top_object(&top_of_pieces(&[a, b, line]), None).unwrap();
        let share = polygon(a) / (polygon(a) + polygon(b) + polygon(line));
        assert!(c.eval(share, 0.0).distance(a.p3) < 1e-9, "{:?}", c.eval(share, 0.0));
        // Joined into their curve, the two pieces would have put the anchor 0.3 of the way along
        // their part of the side.
        let joined = 0.3 * (polygon(a) + polygon(b)) / (polygon(a) + polygon(b) + polygon(line));
        assert!(c.eval(joined, 0.0).distance(a.p3) > 1.0, "{share} vs {joined}");
    }

    #[test]
    fn a_top_object_side_of_many_arcs_stays_on_them() {
        // A 120° arc of radius 500 drawn as `n` circular arcs, each close to a piece of one cubic
        // while the whole arc is not one: the side stays on the circle, and the content's middle
        // stays on the middle anchor, as the arcs' polygon shares put it.
        let centre = Point::new(300.0, 600.0);
        for n in [64u32, 128, 256] {
            let step = 120f64.to_radians() / f64::from(n);
            let reach = 500.0 * 4.0 / 3.0 * (step / 4.0).tan();
            let on = |a: f64| centre + Vec2::new(a.cos(), a.sin()) * 500.0;
            let tangent = |a: f64| Vec2::new(-a.sin(), a.cos()) * reach;
            let mut anchors: Vec<Anchor> = (0..=n)
                .map(|i| {
                    let a = (-150f64).to_radians() + step * f64::from(i);
                    let p = on(a);
                    let h_in = if i == 0 { p } else { p - tangent(a) };
                    let h_out = if i == n { p } else { p + tangent(a) };
                    Anchor { p, h_in, h_out, ..Anchor::corner(p) }
                })
                .collect();
            let (start, end) = (on((-150f64).to_radians()), on((-30f64).to_radians()));
            anchors.extend([Anchor::corner(Point::new(end.x, 800.0)), Anchor::corner(Point::new(start.x, 800.0))]);
            let path = PathData::single(SubPath::new(anchors, true));
            let c = Coons::from_top_object(&path, Some([0, n, n + 1, n + 2])).unwrap();
            let off = (0..=4000).map(|k| (c.eval(f64::from(k) / 4000.0, 0.0).distance(centre) - 500.0).abs()).fold(0.0, f64::max);
            assert!(off < 0.001, "{n} arcs: the side lies {off} pt off the circle");
            let mid = c.eval(0.5, 0.0);
            assert!(mid.distance(Point::new(300.0, 100.0)) < 0.001, "{n} arcs: the middle lands at {mid:?}");
        }
    }

    #[test]
    fn a_rectangle_top_object_is_the_identity() {
        let r = Rect::new(10.0, 20.0, 110.0, 70.0);
        let c = Coons::from_top_object(&shapes::rectangle(r), None).unwrap();
        for (u, v) in [(0.125, 0.3), (0.9, 0.05), (0.37, 0.81)] {
            let p = c.eval(u, v);
            assert!(p.distance(Point::new(10.0 + 100.0 * u, 20.0 + 50.0 * v)) < 1e-9, "({u}, {v}) → {p:?}");
        }
    }

    #[test]
    fn a_straight_line_top_object_maps_onto_its_bounding_box() {
        let line = PathData::single(SubPath::new(vec![Anchor::corner(Point::new(0.0, 0.0)), Anchor::corner(Point::new(100.0, 40.0))], false));
        let p = Coons::from_top_object(&line, None).unwrap().eval(0.25, 0.75);
        assert!(p.distance(Point::new(25.0, 30.0)) < 1e-9, "{p:?}");
    }

    #[test]
    fn a_top_object_spreads_a_segment_by_its_parameter() {
        // The top side is one straight segment whose handles sit at 60 and 80: its parameter
        // reaches the middle at x = 63.75, not at 45.
        let mut top = vec![Anchor::corner(Point::new(0.0, 0.0)), Anchor::corner(Point::new(90.0, 0.0))];
        top[0].h_out = Point::new(60.0, 0.0);
        top[1].h_in = Point::new(80.0, 0.0);
        top.extend([Anchor::corner(Point::new(90.0, 40.0)), Anchor::corner(Point::new(0.0, 40.0))]);
        let kind = EnvelopeKind::TopObject { path: PathData::single(SubPath::new(top, true)), corners: None };
        let map = envelope_mapper(&kind, Rect::new(0.0, 0.0, 10.0, 10.0));
        let p = map(Point::new(5.0, 0.0));
        assert!(p.distance(Point::new(63.75, 0.0)) < 1e-6, "{p:?}");
        let p = map(Point::new(5.0, 10.0));
        assert!(p.distance(Point::new(45.0, 40.0)) < 1e-6, "{p:?}");
    }

    #[test]
    fn a_top_object_follows_a_curved_side_by_its_parameter() {
        // The top side bends down from (0,0) to (100,0) through handles at (20,30) and (90,30):
        // halfway along its parameter it passes (53.75, 22.5).
        let mut top = vec![Anchor::corner(Point::new(0.0, 0.0)), Anchor::corner(Point::new(100.0, 0.0))];
        top[0].h_out = Point::new(20.0, 30.0);
        top[1].h_in = Point::new(90.0, 30.0);
        top.extend([Anchor::corner(Point::new(100.0, 80.0)), Anchor::corner(Point::new(0.0, 80.0))]);
        let path = PathData::single(SubPath::new(top, true));
        let c = Coons::from_top_object(&path, None).unwrap();
        for (u, t) in [(0.5, 0.5), (0.25, 0.25), (0.8125, 0.8125)] {
            let want = CubicBez::new((0.0, 0.0), (20.0, 30.0), (90.0, 30.0), (100.0, 0.0)).eval(t);
            let p = c.eval(u, 0.0);
            assert!(p.distance(want) < 1e-6, "u {u}: {p:?} vs {want:?}");
        }
        // Drawn the other way round, the outline gives the same patch.
        let mut back = path.clone();
        back.subpaths[0].reverse();
        let p = Coons::from_top_object(&back, None).unwrap().eval(0.5, 0.0);
        assert!(p.distance(Point::new(53.75, 22.5)) < 1e-6, "{p:?}");
    }

    #[test]
    fn a_top_object_side_of_many_segments_keeps_each_corner() {
        // The top side zigzags through `n` straight segments, between y = 0 and y = 2.
        for n in [3, 100] {
            let w = f64::from(n);
            let zig = |i: i32| Point::new(f64::from(i) * 100.0 / w, if i % 2 == 1 { 2.0 } else { 0.0 });
            let mut pts: Vec<Anchor> = (0..=n).map(|i| Anchor::corner(zig(i))).collect();
            pts.extend([Anchor::corner(Point::new(100.0, 50.0)), Anchor::corner(Point::new(0.0, 50.0))]);
            let c = Coons::from_top_object(&PathData::single(SubPath::new(pts, true)), None).unwrap();
            for i in (1..n).filter(|i| n < 10 || [1, 37, 38, 99].contains(i)) {
                let p = c.eval(f64::from(i) / w, 0.0);
                assert!(p.distance(zig(i)) < 1e-9, "{n} segments, anchor {i}: {p:?}");
            }
        }
    }

    #[test]
    fn a_gradient_mesh_outline_keeps_its_sides_measured_by_length() {
        let c = Coons::from_path(&top_split_at_30()).unwrap();
        let p = c.eval(0.5, 0.0);
        assert!(p.distance(Point::new(50.0, 0.0)) < 1e-6, "{p:?}");
    }

    #[test]
    fn grid_identity() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let pts = grid_points(r, 3, 4);
        let p = grid_eval(3, 4, &pts, 0.37, 0.81);
        assert!((p.x - 37.0).abs() < 1e-6 && (p.y - 81.0).abs() < 1e-6, "{p:?}");
    }

    /// #3592: a mesh read from a file can have any number of patches, and the view any zoom, so the
    /// quads one tessellation makes are budgeted: the subdivision per patch gives way before memory
    /// does, and each patch stays at least one quad.
    #[test]
    fn a_large_mesh_tessellates_within_the_quad_budget() {
        let mesh = |n: u32| GradientMesh::from_surface(n, n, &|u, v| Point::new(u * 300.0, v * 300.0), &|u, v| Color::rgb(u as f32, v as f32, 0.5));
        // 3,600 patches at 32 x 32 would be 3.7 million quads.
        let m = mesh(60);
        let quads = m.quads(32);
        assert!(quads.len() <= GradientMesh::MAX_QUADS, "{} quads", quads.len());
        assert!(quads.len() >= 3600 * 4, "still finer than a quad per patch: {}", quads.len());
        // A small mesh keeps the subdivision asked for.
        assert_eq!(mesh(3).quads(8).len(), 9 * 64);
        // The finest the view asks for, on a mesh of 22,500 patches, is a few quads per patch.
        let big = mesh(150).quads(64);
        assert!(big.len() <= GradientMesh::MAX_QUADS && big.len() >= 22_500, "{} quads", big.len());
        // More patches than the budget has quads is malformed, as a point count that doesn't fit is.
        let point = MeshPoint { p: Point::ZERO, color: Color::rgb(0.0, 0.0, 0.0), opacity: 1.0, handles: [Vec2::ZERO; 4] };
        let huge = GradientMesh { rows: 2000, cols: 2000, points: vec![point; 2001 * 2001] };
        assert!(!huge.is_valid());
        assert!(huge.quads(1).is_empty());
    }

    /// Flat pieces of a mesh over the quad budget follow the coarser subdivision [`GradientMesh::quads`]
    /// gives way to: each patch's pieces are its own quads', so together they cover the mesh once.
    #[test]
    fn a_large_meshs_pieces_follow_the_quad_budget() {
        let m = GradientMesh::from_surface(60, 60, &|u, v| Point::new(u * 300.0, v * 300.0), &|u, v| Color::rgb(u as f32, v as f32, 0.5));
        let area = |pts: &[Point]| {
            let n = pts.len();
            (0..n).map(|i| pts[i].x * pts[(i + 1) % n].y - pts[(i + 1) % n].x * pts[i].y).sum::<f64>().abs() / 2.0
        };
        let pieces = m.pieces(32);
        assert!(pieces.len() >= 3600, "{} pieces", pieces.len());
        let total: f64 = pieces.iter().map(|p| area(&p.pts)).sum();
        assert!((total - 90_000.0).abs() < 90.0, "covered {total}");
    }

    /// A file's rows and columns can be anything: a point count too large to compute is no match
    /// for the points (u32::MAX × u32::MAX wrapped to 0 and passed for a grid without points).
    #[test]
    fn a_grid_too_large_to_count_is_malformed() {
        let m = GradientMesh { rows: u32::MAX, cols: u32::MAX, points: vec![] };
        assert!(!m.is_valid());
        assert_eq!((m.lines(), m.outline(), m.quads(8).len()), (PathData::default(), PathData::default(), 0));
        assert!(envelope_grid(u32::MAX, u32::MAX, &[], &[]).is_none());
        // A mesh envelope without handles maps through its points: malformed, it leaves art as it is.
        let kind = EnvelopeKind::Mesh { rows: u32::MAX, cols: u32::MAX, points: vec![], handles: vec![] };
        let p = Point::new(25.0, 50.0);
        assert_eq!(envelope_mapper(&kind, Rect::new(0.0, 0.0, 100.0, 100.0))(p), p);
    }

    /// `bounds_margin` grows the box an envelope maps from, only while Distort Appearance is on. It
    /// holds shares of the box, and an envelope with an uneven frame maps from the same box in that
    /// frame. A native file writes it only when it isn't zero.
    #[test]
    fn an_envelope_maps_from_its_box_with_its_margins() {
        let rect = Node::path(NodeId(2), vectorcraft_geom::shapes::rectangle(Rect::new(10.0, 20.0, 110.0, 70.0)), crate::Appearance::default_art());
        let kind = EnvelopeKind::Warp { style: "arc".into(), bend: 0.0, h: 0.0, v: 0.0, horizontal: true, version: 0 };
        let content = [Arc::new(rect)];
        let src = |content: &[Arc<Node>], frame: Affine, options: EnvelopeOptions| EnvelopeMap::new(content, &kind, frame, options).unwrap().src();
        // Top, bottom, left and right.
        let on = EnvelopeOptions { bounds_margin: [0.125, 0.25, 0.0625, 0.5], ..EnvelopeOptions::NEW };
        let geometric = Rect::new(10.0, 20.0, 110.0, 70.0);
        let grown = Rect::new(3.75, 13.75, 160.0, 82.5);
        assert_eq!(src(&content, Affine::IDENTITY, EnvelopeOptions::default()), geometric);
        assert_eq!(src(&content, Affine::IDENTITY, on), grown);
        assert_eq!(src(&content, Affine::IDENTITY, EnvelopeOptions { distort_appearance: false, ..on }), geometric);
        for bad in [f32::NAN, f32::INFINITY, -0.05] {
            assert_eq!(src(&content, Affine::IDENTITY, EnvelopeOptions { bounds_margin: [bad, 0.25, 0.0625, 0.5], ..on }), geometric);
        }
        // A finite share of a box too wide to grow leaves the box as it is.
        let wide = Node::path(NodeId(3), vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 1e308, 10.0)), crate::Appearance::default_art());
        let wide = [Arc::new(wide)];
        let huge = EnvelopeOptions { bounds_margin: [0.0, 0.0, f32::MAX, 0.0], ..on };
        assert_eq!(src(&wide, Affine::IDENTITY, huge), src(&wide, Affine::IDENTITY, EnvelopeOptions::NEW));
        // Bounds follow the box: a warp that doesn't bend covers it.
        let b = envelope_bounds(&content, &kind, Affine::IDENTITY, on).unwrap();
        assert!((b.x0 - 3.75).abs() < 1e-6 && (b.y1 - 82.5).abs() < 1e-6, "{b:?}");
        // Art taken through an uneven frame maps from the same box in that frame.
        let frame = Affine::rotate(std::f64::consts::FRAC_PI_2) * Affine::scale_non_uniform(2.0, 0.5);
        let mut placed = (*content[0]).clone();
        placed.transform(frame, true);
        let framed = src(&[Arc::new(placed)], frame, on);
        assert!((framed.x0 - 3.75).abs() + (framed.y0 - 13.75).abs() + (framed.x1 - 160.0).abs() + (framed.y1 - 82.5).abs() < 1e-6, "{framed:?}");

        // A frame that turns and scales gives the same margins as the art had before it took them.
        let mut stroked = (*content[0]).clone();
        if let Some(s) = stroked.appearance.stroke_mut() {
            s.width = 10.0;
        }
        let margin = |n: &Node, frame: Affine| {
            let (with, plain) = envelope_content_boxes(&[Arc::new(n.clone())], frame).unwrap();
            envelope_stroke_margin(with, plain).unwrap()
        };
        let before = margin(&stroked, Affine::IDENTITY);
        assert_eq!(before.map(|m| (m * 1000.0).round()), [100.0, 100.0, 50.0, 50.0]);
        let frame = Affine::rotate(std::f64::consts::FRAC_PI_2) * Affine::scale(2.0);
        let mut turned = stroked.clone();
        turned.transform(frame, true);
        assert_eq!(margin(&turned, frame).map(|m| (m * 1000.0).round()), before.map(|m| (m * 1000.0).round()));
        // Margins an `f32` can't hold are none.
        assert_eq!(envelope_stroke_margin(Rect::new(-5e41, 0.0, 100.0, 100.0), Rect::new(0.0, 0.0, 100.0, 100.0)), None);

        assert!(serde_json::to_value(EnvelopeOptions::default()).unwrap().get("boundsMargin").is_none());
        let saved = serde_json::to_value(on).unwrap();
        assert_eq!(saved["boundsMargin"], serde_json::json!([0.125, 0.25, 0.0625, 0.5]));
        assert_eq!(serde_json::from_value::<EnvelopeOptions>(saved).unwrap(), on);
    }

    /// How far the border of `m` strays from the ellipse in `r`, in points (measured across the
    /// ellipse's short radius), checked at 20 places along every border edge.
    fn mesh_border_off_ellipse(m: &GradientMesh, r: Rect) -> f64 {
        let (rows, cols) = (m.rows as usize, m.cols as usize);
        let edges = (0..cols).flat_map(|c| [m.edge_u(0, c), m.edge_u(rows, c)]).chain((0..rows).flat_map(|k| [m.edge_v(k, 0), m.edge_v(k, cols)]));
        let (c, a, b) = (r.center(), r.width() / 2.0, r.height() / 2.0);
        edges
            .flat_map(|e| (0..=20).map(move |i| e.eval(f64::from(i) / 20.0)))
            .map(|p| ((p.x - c.x) / a).hypot((p.y - c.y) / b) - 1.0)
            .fold(0.0f64, |w, d| w.max(d.abs() * a.min(b)))
    }

    /// #3187: a gradient mesh made from an ellipse keeps the ellipse's shape: a Mesh tool click
    /// (a 1×1 mesh with lines through the point) and Create Gradient Mesh's 4×4 grid both lay
    /// their border on the outline, within about twice the 0.35 pt the outline is flattened to.
    /// The 1×1 mesh was up to 7 pt off this 400 × 300 pt ellipse.
    #[test]
    fn a_mesh_from_an_ellipse_keeps_its_outline() {
        const OFF: f64 = 0.75;
        let r = Rect::new(100.0, 100.0, 500.0, 400.0);
        let e = shapes::ellipse(r);
        for (rows, cols) in [(1, 1), (4, 4), (2, 5)] {
            let m = GradientMesh::for_path(&e, rows, cols, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
            let off = mesh_border_off_ellipse(&m, r);
            assert!(off < OFF, "{rows}×{cols} mesh: border {off} pt off the ellipse");
        }
        let made = GradientMesh::for_path(&e, 1, 1, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
        let mut m = made.clone();
        // Off centre, so each line splits its edges unevenly.
        let i = m.add_lines_at(Point::new(200.0, 330.0), 0.0).unwrap();
        let off = mesh_border_off_ellipse(&m, r);
        assert!(off < OFF, "after a Mesh tool click: {off} pt");
        // Deleting the lines again (Alt-click) gives the border back as it was.
        assert!(m.remove_point_lines(i));
        assert_eq!((m.rows, m.cols), (1, 1));
        for (k, (now, was)) in m.points.iter().zip(&made.points).enumerate() {
            let moved = now.handles.iter().zip(&was.handles).map(|(a, b)| (*a - *b).hypot()).fold(0.0, f64::max);
            assert!(now.p.distance(was.p) < 1e-6 && moved < 1e-3, "point {k}: {now:?}, made as {was:?}");
        }
        // With a handle at the new border point shortened since, the two edges no longer make one
        // curve: the merged edge is straight rather than grown past its ends.
        let mut m = made.clone();
        let i = m.add_lines_at(Point::new(200.0, 330.0), 0.0).unwrap();
        let top = m.idx(0, 1);
        m.points[top].handles[H_LEFT] *= 1e-4;
        assert!(m.remove_point_lines(i));
        let (a, b) = (m.points[0].p, m.points[1].p);
        assert!((m.points[0].handles[H_RIGHT] - (b - a) / 3.0).hypot() < 1e-9, "{:?}", m.points[0].handles);
        // A rectangle's mesh keeps straight edges with handles a third of the way along.
        let m = GradientMesh::for_path(&shapes::rectangle(Rect::new(0.0, 0.0, 90.0, 60.0)), 1, 1, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
        let e = m.edge_u(0, 0);
        assert!(e.p1.distance(Point::new(30.0, 0.0)) < 1e-6 && e.p2.distance(Point::new(60.0, 0.0)) < 1e-6, "{e:?}");
    }

    /// The polylines of `bp` flattened to 0.01 pt.
    fn flattened(bp: &BezPath) -> Vec<Vec<Point>> {
        let mut out: Vec<Vec<Point>> = vec![];
        vectorcraft_geom::kurbo::flatten(bp.iter(), 0.01, |el| match el {
            vectorcraft_geom::PathEl::MoveTo(p) => out.push(vec![p]),
            vectorcraft_geom::PathEl::LineTo(p) => out.last_mut().into_iter().for_each(|v| v.push(p)),
            vectorcraft_geom::PathEl::ClosePath => out.last_mut().into_iter().for_each(|v| v.push(v[0])),
            _ => {}
        });
        out
    }

    /// The distance from `p` to the nearest of `lines`.
    fn off_lines(lines: &[Vec<Point>], p: Point) -> f64 {
        lines
            .iter()
            .flat_map(|v| v.windows(2))
            .map(|w| {
                let d = w[1] - w[0];
                let t = if d.hypot2() > 0.0 { ((p - w[0]).dot(d) / d.hypot2()).clamp(0.0, 1.0) } else { 0.0 };
                p.distance(w[0] + d * t)
            })
            .fold(f64::INFINITY, f64::min)
    }

    /// How far apart the border of `m` and the outline of `path` are, in points: the larger of the
    /// farthest a point of either lies from the other.
    fn mesh_border_off_path(m: &GradientMesh, path: &PathData) -> f64 {
        let (border, outline) = (flattened(&m.outline().to_bezpath()), flattened(&path.to_bezpath()));
        let one = border.iter().flatten().map(|p| off_lines(&outline, *p)).fold(0.0, f64::max);
        let other = outline.iter().flatten().map(|p| off_lines(&border, *p)).fold(0.0, f64::max);
        one.max(other)
    }

    /// #2752: a gradient mesh keeps the exact outline of the path it is made from, sharp corners
    /// included, whether made by Create Gradient Mesh (1×1 and 4×4) or by a Mesh tool click at a
    /// point inside. Before, the border of each mesh was one cubic per edge between four corners
    /// near the bounding box's: up to 49 pt off a five-point star, 54 pt off a notched rectangle.
    #[test]
    fn a_mesh_keeps_the_outline_of_every_shape() {
        const OFF: f64 = 0.1;
        let poly = |pts: &[(f64, f64)]| PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| Point::new(x, y)).collect::<Vec<_>>(), true));
        let curves = |start: (f64, f64), cubics: &[[(f64, f64); 3]]| {
            let mut bp = BezPath::new();
            bp.move_to(start);
            for [a, b, c] in cubics {
                bp.curve_to(*a, *b, *c);
            }
            bp.close_path();
            PathData::from_bezpath(&bp)
        };
        let box_ = Rect::new(100.0, 100.0, 500.0, 400.0);
        let heart = curves(
            (300.0, 400.0),
            &[
                [(200.0, 320.0), (100.0, 250.0), (100.0, 170.0)],
                [(100.0, 100.0), (180.0, 70.0), (230.0, 90.0)],
                [(270.0, 105.0), (290.0, 130.0), (300.0, 150.0)],
                [(310.0, 130.0), (330.0, 105.0), (370.0, 90.0)],
                [(420.0, 70.0), (500.0, 100.0), (500.0, 170.0)],
                [(500.0, 250.0), (400.0, 320.0), (300.0, 400.0)],
            ],
        );
        let teardrop = curves(
            (300.0, 100.0),
            &[
                [(350.0, 200.0), (420.0, 250.0), (420.0, 320.0)],
                [(420.0, 386.0), (366.0, 440.0), (300.0, 440.0)],
                [(234.0, 440.0), (180.0, 386.0), (180.0, 320.0)],
                [(180.0, 250.0), (250.0, 200.0), (300.0, 100.0)],
            ],
        );
        let shapes: [(&str, PathData, Point); 14] = [
            ("rectangle", shapes::rectangle(box_), Point::new(260.0, 230.0)),
            ("rotated rectangle", shapes::rectangle(box_).transformed(Affine::rotate_about(0.5, box_.center())), Point::new(280.0, 240.0)),
            ("triangle", poly(&[(300.0, 100.0), (500.0, 400.0), (100.0, 400.0)]), Point::new(300.0, 320.0)),
            ("five-point star", shapes::star(Point::new(300.0, 300.0), 200.0, 80.0, 5, 0.0), Point::new(300.0, 300.0)),
            ("twelve-point star", shapes::star(Point::new(300.0, 300.0), 200.0, 140.0, 12, 0.0), Point::new(300.0, 300.0)),
            (
                "L-shape",
                poly(&[(100.0, 100.0), (200.0, 100.0), (200.0, 300.0), (400.0, 300.0), (400.0, 400.0), (100.0, 400.0)]),
                Point::new(150.0, 350.0),
            ),
            ("rounded rectangle", shapes::rounded_rectangle(box_, 40.0), Point::new(260.0, 230.0)),
            ("heart", heart, Point::new(300.0, 250.0)),
            ("teardrop", teardrop, Point::new(300.0, 330.0)),
            ("ellipse", shapes::ellipse(box_), Point::new(200.0, 330.0)),
            ("thin rectangle", shapes::rectangle(Rect::new(100.0, 100.0, 500.0, 110.0)), Point::new(260.0, 104.0)),
            (
                "notched rectangle",
                poly(&[(100.0, 100.0), (250.0, 100.0), (300.0, 200.0), (350.0, 100.0), (500.0, 100.0), (500.0, 400.0), (100.0, 400.0)]),
                Point::new(260.0, 300.0),
            ),
            // A notch shorter than the steps along the outline an edge is checked at, and deeper
            // than the tolerance.
            (
                "rectangle with a tiny notch",
                poly(&[(100.0, 100.0), (301.1, 100.0), (301.2, 100.3), (301.3, 100.0), (500.0, 100.0), (500.0, 400.0), (100.0, 400.0)]),
                Point::new(260.0, 300.0),
            ),
            // An outline that crosses itself, whose two lobes' areas cancel.
            ("bow tie", poly(&[(100.0, 100.0), (500.0, 400.0), (500.0, 100.0), (100.0, 400.0)]), Point::new(150.0, 250.0)),
        ];
        // Every shape and way of making the mesh is checked, and all that stray are named.
        let mut strays = vec![];
        for (name, path, click) in shapes {
            for (rows, cols) in [(1, 1), (4, 4)] {
                let m = GradientMesh::for_path(&path, rows, cols, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
                let off = mesh_border_off_path(&m, &path);
                if off >= OFF {
                    strays.push(format!("{name}, {rows}×{cols}: border {off:.2} pt off the outline"));
                }
            }
            let mut m = GradientMesh::for_path(&path, 1, 1, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
            assert!(m.add_lines_at(click, 0.0).is_some(), "{name}: the click at {click:?} is outside the mesh");
            let off = mesh_border_off_path(&m, &path);
            if off >= OFF {
                strays.push(format!("{name}, Mesh tool click: border {off:.2} pt off the outline"));
            }
        }
        assert!(strays.is_empty(), "{strays:#?}");
    }

    /// The lines a mesh takes to keep a path's outline are as few as do it: a rectangle's and an
    /// ellipse's meshes keep the rows and columns asked for, and a star's 1×1 mesh takes lines
    /// through its points, at most one each way per anchor. Paths without an outline to follow
    /// still give a mesh (an open path closes straight; a line there and back maps onto its box),
    /// and a point that isn't finite gives none.
    #[test]
    fn a_mesh_takes_lines_only_where_the_outline_needs_them() {
        let box_ = Rect::new(100.0, 100.0, 500.0, 400.0);
        for path in [shapes::rectangle(box_), shapes::ellipse(box_)] {
            for (rows, cols) in [(1, 1), (4, 4), (2, 5)] {
                let m = GradientMesh::for_path(&path, rows, cols, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
                assert_eq!((m.rows, m.cols), (rows, cols));
            }
        }
        let star = shapes::star(Point::new(300.0, 300.0), 200.0, 80.0, 5, 0.0);
        let m = GradientMesh::for_path(&star, 1, 1, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
        assert!(m.rows > 1 && m.cols > 1 && m.rows <= 10 && m.cols <= 10 && m.is_valid(), "{}×{}", m.rows, m.cols);
        // Every anchor of the star is a point of the mesh.
        for a in &star.subpaths[0].anchors {
            assert!(m.points.iter().any(|q| q.p.distance(a.p) < 1e-6), "{:?} is not a mesh point", a.p);
        }
        // An open path closes with a straight segment.
        let open = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 80.0)], false));
        let m = GradientMesh::for_path(&open, 2, 2, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
        let closed = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0), Point::new(100.0, 80.0)], true));
        assert!(mesh_border_off_path(&m, &closed) < 0.1);
        // A line there and back.
        let line = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 50.0)], true));
        let m = GradientMesh::for_path(&line, 1, 1, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
        assert!(m.is_valid() && m.points.iter().all(|q| q.p.is_finite()));
        let mut nan = shapes::rectangle(box_);
        nan.subpaths[0].anchors[1].p.x = f64::NAN;
        assert!(GradientMesh::for_path(&nan, 1, 1, Color::BLACK, MeshAppearance::Flat, 0.0).is_none());
        // A path of many anchors stays within the line limit and keeps its outline.
        let many = shapes::star(Point::new(300.0, 300.0), 200.0, 190.0, 400, 0.0);
        let m = GradientMesh::for_path(&many, 1, 1, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
        assert!(m.is_valid() && m.rows as usize <= mesh::MAX_BORDER_LINES && m.cols as usize <= mesh::MAX_BORDER_LINES, "{}×{}", m.rows, m.cols);
    }

    #[test]
    fn mesh_insert_and_remove_lines() {
        let mut m =
            GradientMesh::for_path(&shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), 1, 1, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
        let i = m.add_lines_at(Point::new(30.0, 60.0), 0.0).unwrap();
        assert_eq!((m.rows, m.cols), (2, 2));
        assert!(m.points[i].p.distance(Point::new(30.0, 60.0)) < 0.5, "{:?}", m.points[i].p);
        assert!(m.remove_point_lines(i));
        assert_eq!((m.rows, m.cols), (1, 1));
    }

    /// A point outside the mesh within the tolerance locates the nearest point of its edge.
    #[test]
    fn mesh_locate_within_reaches_the_edge() {
        let m = GradientMesh::for_path(&shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), 2, 2, Color::BLACK, MeshAppearance::Flat, 0.0).unwrap();
        let p = Point::new(102.0, 30.0);
        assert_eq!(m.locate(p), None);
        assert_eq!(m.locate_within(p, 1.0), None);
        let (r, c, u, v) = m.locate_within(p, 3.0).unwrap();
        assert_eq!((r, c), (0, 1));
        assert_eq!(u, 1.0, "on the right edge");
        let q = m.eval(r, c, u, v);
        assert!(q.distance(Point::new(100.0, 30.0)) < 1e-6, "{q:?}");
        assert_eq!(m.locate_within(Point::new(30.0, 60.0), 3.0), m.locate(Point::new(30.0, 60.0)));
    }

    /// A point on a mesh's outline locates on the edge it lies on, on all four sides alike, with no
    /// tolerance; a point just outside does not, and a tolerance reaches only as far as it says.
    /// A point that isn't finite locates nowhere.
    #[test]
    fn mesh_locate_takes_a_point_on_any_side_of_the_outline() {
        for (rows, cols) in [(1, 1), (2, 3)] {
            let m =
                GradientMesh::for_path(&shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), rows, cols, Color::BLACK, MeshAppearance::Flat, 0.0)
                    .unwrap();
            let sides = [
                ("top", Point::new(30.0, 0.0), Vec2::new(0.0, -0.01)),
                ("bottom", Point::new(30.0, 100.0), Vec2::new(0.0, 0.01)),
                ("left", Point::new(0.0, 70.0), Vec2::new(-0.01, 0.0)),
                ("right", Point::new(100.0, 70.0), Vec2::new(0.01, 0.0)),
            ];
            for (side, p, out) in sides {
                let (r, c, u, v) = m.locate(p).unwrap_or_else(|| panic!("{rows}×{cols} {side}: {p:?} is not located"));
                assert!(r < m.rows as usize && c < m.cols as usize && (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v), "{side}");
                assert!(m.eval(r, c, u, v).distance(p) < 1e-3, "{rows}×{cols} {side}: {:?}", m.eval(r, c, u, v));
                let mut e = m.clone();
                assert!(e.add_lines_at(p, 0.0).is_some(), "{rows}×{cols} {side}: no line at {p:?}");
                assert_eq!(m.locate(p + out), None, "{rows}×{cols} {side}: just outside");
                assert_eq!(m.locate_within(p + out * 100.0, 0.5), None, "{rows}×{cols} {side}: past the tolerance");
                assert!(m.locate_within(p + out * 100.0, 1.5).is_some(), "{rows}×{cols} {side}: within the tolerance");
            }
            for p in [Point::new(f64::NAN, 0.0), Point::new(30.0, f64::INFINITY)] {
                assert_eq!(m.locate_within(p, f64::INFINITY), None, "{p:?}");
            }
        }
    }

    /// Corners all in CMYK mix in CMYK (the inks halfway), not in RGB.
    #[test]
    fn a_cmyk_mesh_mixes_its_inks() {
        let cyan = Color::Cmyk { c: 1.0, m: 0.0, y: 0.0, k: 0.0 };
        let mut m = GradientMesh::for_path(&shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), 1, 1, cyan, MeshAppearance::Flat, 0.0).unwrap();
        m.points[0].color = Color::Cmyk { c: 0.0, m: 0.0, y: 0.0, k: 1.0 };
        let (c, op) = m.color_at(0, 0, 0.0, 0.0);
        assert_eq!((c, op), (Color::Cmyk { c: 0.0, m: 0.0, y: 0.0, k: 1.0 }, 1.0));
        let Color::Cmyk { c, k, .. } = m.color_at(0, 0, 0.5, 0.5).0 else { panic!("mixed out of CMYK") };
        assert!((c - 0.75).abs() < 1e-6 && (k - 0.25).abs() < 1e-6, "{c} {k}");
        // A corner in another space mixes them all in RGB.
        m.points[1].color = Color::rgb(1.0, 0.0, 0.0);
        assert!(matches!(m.color_at(0, 0, 0.5, 0.5).0, Color::Rgb { .. }));
    }

    /// #3232: the Appearance highlight lightens each point towards white in its own model, so a
    /// CMYK mesh stays CMYK (K shrinks with the other inks) and a grey one stays grey.
    #[test]
    fn mesh_highlight_keeps_each_points_model() {
        let square = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
        let ink = Color::Cmyk { c: 0.0, m: 0.4, y: 0.6, k: 0.1 };
        let m = GradientMesh::for_path(&square, 2, 2, ink, MeshAppearance::ToCenter, 50.0).unwrap();
        assert_eq!(m.points[0].color, ink, "corners stay");
        let centre = m.points[m.idx(1, 1)].color;
        let Color::Cmyk { c, m: mg, y, k } = centre else { panic!("{centre:?}") };
        assert!(c.abs() < 1e-6 && (mg - 0.2).abs() < 1e-6 && (y - 0.3).abs() < 1e-6 && (k - 0.05).abs() < 1e-6, "{c} {mg} {y} {k}");
        assert!(matches!(m.color_at(0, 0, 0.5, 0.5).0, Color::Cmyk { .. }), "patches mix in CMYK");
        let m = GradientMesh::for_path(&square, 2, 2, Color::gray(0.8), MeshAppearance::ToEdge, 100.0).unwrap();
        assert_eq!((m.points[0].color, m.points[m.idx(1, 1)].color), (Color::gray(0.0), Color::gray(0.8)));
        // RGB lightens in RGB as before.
        let m = GradientMesh::for_path(&square, 2, 2, Color::rgb(1.0, 0.0, 0.0), MeshAppearance::ToCenter, 100.0).unwrap();
        assert_eq!(m.points[m.idx(1, 1)].color.to_hex(), "#ffffff");
    }

    /// Alt-clicking a point of a mesh from a file with fewer points than its rows and columns need
    /// deletes nothing (it indexed past the points, a panic).
    #[test]
    fn mesh_lines_of_a_malformed_mesh_stay() {
        let point = |i: u32| MeshPoint { p: Point::new(f64::from(i), 0.0), color: Color::BLACK, opacity: 1.0, handles: [Vec2::ZERO; 4] };
        // An interior row's point and an interior column's point, each with 3 of the 6 points.
        for (rows, cols, index) in [(2, 1, 2), (1, 2, 1)] {
            let mut m = GradientMesh { rows, cols, points: (0..3).map(point).collect() };
            let before = m.clone();
            assert!(!m.remove_point_lines(index), "{rows}×{cols}, point {index}");
            assert_eq!(m, before);
        }
    }
}
