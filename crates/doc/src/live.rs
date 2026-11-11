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

pub use blend_spec::{BlendDefaults, BlendEase, BlendEaseType, BlendOrientation, BlendSpacing, BlendSpec};
#[cfg(test)]
use blend_spec::{blend_color_shift, blend_ramp};
pub use envelope::{
    EnvelopeKind, EnvelopeMap, EnvelopeOptions, H_DOWN, H_LEFT, H_RIGHT, H_UP, PreserveShape, TypeInk, envelope_bounds, envelope_content_bounds,
    envelope_content_boxes, envelope_expand, envelope_frame, envelope_grid, envelope_mapper, envelope_of, envelope_overlay, envelope_stroke_margin,
    set_type_ink, smooth_handles,
};
pub(crate) use expand::max_outset;
pub use expand::{
    Hooks, MAX_GENERATED, Outliner, PatternTiles, affine_near, expand_deep, expand_deep_hooks, expand_live, expand_live_hooks, expand_live_with,
    expanded_group, expanded_group_hooks, is_live, mesh_quad_nodes, subtree_has_live,
};
#[cfg(test)]
use mesh::grid_eval;
pub use mesh::{Coons, GradientMesh, MeshAppearance, MeshPiece, MeshPoint, MeshQuad, default_fidelity, grid_points, nodes_bounds};
pub use warp::{WarpStyle, fit_piece, map_nonlinear, map_nonlinear_within, pieces, warp_point};

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

    #[test]
    fn a_split_curve_moves_the_top_object_share_to_its_new_anchor() {
        // Splitting the top curve at t = 0.3 keeps its shape but changes the polygon shares: the
        // new anchor lands at the first piece's share, not at u = 0.3.
        let top = CubicBez::new((0.0, 0.0), (40.0, -40.0), (160.0, -40.0), (200.0, 0.0));
        let (a, b) = (top.subsegment(0.0..0.3), top.subsegment(0.3..1.0));
        let path = outline(&[
            ((0.0, 0.0), (0.0, 0.0), a.p1.into()),
            (a.p3.into(), a.p2.into(), b.p1.into()),
            ((200.0, 0.0), b.p2.into(), (200.0, 0.0)),
            ((200.0, 100.0), (200.0, 100.0), (200.0, 100.0)),
            ((0.0, 100.0), (0.0, 100.0), (0.0, 100.0)),
        ]);
        let c = Coons::from_top_object(&path, None).unwrap();
        let share = polygon(a) / (polygon(a) + polygon(b));
        assert!(c.eval(share, 0.0).distance(a.p3) < 1e-9, "{:?}", c.eval(share, 0.0));
        assert!((share - 0.3).abs() > 0.01, "{share}");
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
        let kind = EnvelopeKind::Warp { style: "arc".into(), bend: 0.0, h: 0.0, v: 0.0, horizontal: true };
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
