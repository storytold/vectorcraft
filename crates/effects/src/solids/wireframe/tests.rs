use serde_json::{Value, json};
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, Node, NodeId};
use vectorcraft_geom::{BezPath, Rect};

use super::*;

fn square(x: f64, y: f64, size: f64) -> PathData {
    let mut b = BezPath::new();
    b.move_to((x, y));
    b.line_to((x + size, y));
    b.line_to((x + size, y + size));
    b.line_to((x, y + size));
    b.close_path();
    PathData::from_bezpath(&b)
}

/// A filled 100 pt square without a stroke, with effect `id` and `params`.
fn solid_square(id: &str, params: Value) -> Node {
    let mut n = Node::path(NodeId(1), square(100.0, 100.0, 100.0), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    n.appearance.effects.push(crate::new_effect(id, &params).unwrap());
    n
}

/// Every segment of `wire`, each as its two ends on the grid.
fn segments(wire: &[PathData]) -> Vec<(Snap, Snap)> {
    wire.iter()
        .flat_map(|p| &p.subpaths)
        .flat_map(|sp| (0..sp.segment_count()).map(move |i| sp.segment(i)))
        .map(|c| (snap(c.p0), snap(c.p3)))
        .collect()
}

fn bounds(paths: &[PathData]) -> Rect {
    paths.iter().filter_map(PathData::bounds).reduce(|a, b| a.union(b)).unwrap()
}

/// The paths of the faces `art` draws (every leaf of the art).
fn face_paths(art: &Node) -> Vec<PathData> {
    let mut out = vec![];
    art.walk(&mut |c| out.extend(c.path_data().cloned()));
    out
}

#[test]
fn two_faces_that_share_an_edge_give_it_once() {
    let wire = edges(&[square(0.0, 0.0, 10.0), square(10.0, 0.0, 10.0)]).unwrap();
    let segs = segments(&wire);
    assert_eq!(segs.len(), 7, "{segs:?}");
    for (i, a) in segs.iter().enumerate() {
        for b in segs.iter().skip(i + 1) {
            assert!(a != b && (a.1, a.0) != *b, "{a:?} twice");
        }
    }
}

#[test]
fn a_loop_of_edges_is_one_closed_path_and_a_curve_stays_a_curve() {
    let wire = edges(&[square(0.0, 0.0, 10.0)]).unwrap();
    assert_eq!(wire.len(), 1);
    assert!(wire[0].subpaths[0].closed);
    assert_eq!(wire[0].subpaths[0].anchors.len(), 4);

    let mut b = BezPath::new();
    b.move_to((0.0, 0.0));
    b.curve_to((0.0, 10.0), (10.0, 10.0), (10.0, 0.0));
    b.close_path();
    let wire = edges(&[PathData::from_bezpath(&b)]).unwrap();
    let curved = wire.iter().flat_map(|p| &p.subpaths).any(|sp| (0..sp.segment_count()).any(|i| !sp.segment_is_line(i)));
    assert!(curved, "{wire:?}");
}

/// A single curve that comes back to its start keeps its segment: it stays open, with two anchors on
/// one point, since a path of one anchor has no segment.
#[test]
fn a_single_curve_that_comes_back_to_its_start_keeps_its_segment() {
    let p = Point::new(0.0, 0.0);
    let anchors = vec![Anchor::with_handles(p, p, Point::new(0.0, 10.0)), Anchor::with_handles(p, Point::new(10.0, 10.0), p)];
    let face = PathData::single(SubPath::new(anchors, false));
    assert_eq!(face.subpaths[0].segment_count(), 1);
    let wire = edges(&[face]).unwrap();
    assert_eq!(wire.len(), 1);
    let sp = &wire[0].subpaths[0];
    assert_eq!(sp.segment_count(), 1, "{sp:?}");
    assert!(!sp.segment_is_line(0));
}

#[test]
fn edges_without_length_or_with_non_finite_points_are_left_out() {
    let point = PathData::single(SubPath::polyline(&[Point::new(5.0, 5.0), Point::new(5.0, 5.0)], false));
    let bad = PathData::single(SubPath::polyline(&[Point::new(f64::NAN, 0.0), Point::new(1.0, 1.0)], false));
    assert!(edges(&[point, bad]).unwrap().is_empty());
    assert!(edges(&[]).unwrap().is_empty());
}

#[test]
fn a_box_turned_off_axis_shows_the_nine_edges_of_its_three_faces() {
    let n = solid_square(super::super::EXTRUDE, json!({}));
    let wire = three_d_wireframe_in(None, &n).unwrap();
    assert_eq!(segments(&wire).len(), 9, "{wire:?}");
    // The wireframe covers what the effect draws, in the same place.
    let art = super::super::solid_art(&n).unwrap();
    let (w, a) = (bounds(&wire), bounds(&face_paths(&art)));
    for (x, y) in [(w.x0, a.x0), (w.y0, a.y0), (w.x1, a.x1), (w.y1, a.y1)] {
        assert!((x - y).abs() < 1e-6, "{w:?} {a:?}");
    }
}

#[test]
fn the_classic_wireframe_surface_expands_to_all_twelve_edges() {
    let n = solid_square(super::super::CLASSIC_EXTRUDE, json!({"surface": "wireframe"}));
    assert_eq!(segments(&three_d_wireframe_in(None, &n).unwrap()).len(), 12);
}

#[test]
fn revolve_inflate_and_rotate_expand_too() {
    for id in [crate::REVOLVE, super::super::INFLATE, ROTATE] {
        let wire = three_d_wireframe_in(None, &solid_square(id, json!({}))).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert!(!wire.is_empty(), "{id}");
        assert!(wire.iter().flat_map(|p| &p.subpaths).flat_map(|sp| &sp.anchors).all(|a| a.p.x.is_finite() && a.p.y.is_finite()), "{id}");
    }
    // Rotate is one flat face: its outline.
    let plane = three_d_wireframe_in(None, &solid_square(ROTATE, json!({}))).unwrap();
    assert_eq!(segments(&plane).len(), 4);
}

/// Revolve (Classic) and Rotate (Classic) expand to the edges of the faces they draw, in the same
/// place.
#[test]
fn revolve_classic_and_rotate_classic_expand_too() {
    let n = solid_square(crate::CLASSIC_REVOLVE, json!({}));
    let wire = three_d_wireframe_in(None, &n).unwrap();
    assert!(!wire.is_empty());
    let art = crate::revolve_art(&n).unwrap();
    let (w, a) = (bounds(&wire), bounds(&face_paths(&art)));
    for (x, y) in [(w.x0, a.x0), (w.y0, a.y0), (w.x1, a.x1), (w.y1, a.y1)] {
        assert!((x - y).abs() < 1e-6, "{w:?} {a:?}");
    }
    // On a fill, with a geometry effect after it.
    let mut n = solid_square(crate::CLASSIC_REVOLVE, json!({}));
    let fx = n.appearance.effects.remove(0);
    for item in &mut n.appearance.items {
        if matches!(item, AppearanceItem::Fill(_)) {
            item.effects_mut().push(fx.clone());
        }
    }
    let plain = bounds(&three_d_wireframe_in(None, &n).unwrap());
    for item in &mut n.appearance.items {
        if matches!(item, AppearanceItem::Fill(_)) {
            item.effects_mut().push(crate::new_effect("distort.transform", &json!({"moveH": 50.0})).unwrap());
        }
    }
    let moved = bounds(&three_d_wireframe_in(None, &n).unwrap());
    assert!((moved.x0 - plain.x0 - 50.0).abs() < 1e-6 && (moved.y0 - plain.y0).abs() < 1e-6, "{plain:?} {moved:?}");
    // Rotate (Classic) is one flat face: its outline.
    let plane = three_d_wireframe_in(None, &solid_square(super::super::CLASSIC_ROTATE, json!({}))).unwrap();
    assert_eq!(segments(&plane).len(), 4);
}

/// A geometry effect after a fill's 3D effect moves that fill's edges, as it moves its drawn surface.
#[test]
fn a_geometry_effect_after_a_fill_s_3d_effect_reshapes_its_edges() {
    for id in [super::super::EXTRUDE, crate::REVOLVE] {
        let mut n = solid_square(id, json!({}));
        let fx = n.appearance.effects.remove(0);
        for item in &mut n.appearance.items {
            if matches!(item, AppearanceItem::Fill(_)) {
                item.effects_mut().push(fx.clone());
            }
        }
        let plain = bounds(&three_d_wireframe_in(None, &n).unwrap_or_else(|e| panic!("{id}: {e}")));
        for item in &mut n.appearance.items {
            if matches!(item, AppearanceItem::Fill(_)) {
                item.effects_mut().push(crate::new_effect("distort.transform", &json!({"moveH": 50.0})).unwrap());
            }
        }
        let moved = bounds(&three_d_wireframe_in(None, &n).unwrap());
        assert!((moved.x0 - plain.x0 - 50.0).abs() < 1e-6 && (moved.y0 - plain.y0).abs() < 1e-6, "{id}: {plain:?} {moved:?}");
    }
}

#[test]
fn an_object_without_a_visible_3d_effect_is_an_error() {
    let mut n = solid_square(super::super::EXTRUDE, json!({}));
    n.appearance.effects[0].visible = false;
    assert!(three_d_wireframe_in(None, &n).unwrap_err().contains("no 3D effect"));
    n.appearance.effects.clear();
    assert!(three_d_wireframe_in(None, &n).is_err());
}
