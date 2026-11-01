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
    EnvelopeKind, EnvelopeMap, EnvelopeOptions, H_DOWN, H_LEFT, H_RIGHT, H_UP, PreserveShape, envelope_bounds, envelope_expand, envelope_frame,
    envelope_grid, envelope_mapper, envelope_of, envelope_overlay, smooth_handles,
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
