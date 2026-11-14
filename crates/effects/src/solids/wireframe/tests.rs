use serde_json::{Value, json};
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Node, NodeId};
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

/// The geometry effects after a 3D effect reshape each face on its own, as they reshape each face
/// of the drawn surface: two faces that touch stay two outlines.
#[test]
fn later_geometry_effects_reshape_each_face_on_its_own() {
    let n = solid_square(super::super::EXTRUDE, json!({}));
    let fx = [crate::new_effect("path.offsetPath", &json!({"offset": 1.0})).unwrap()];
    let faces = [square(0.0, 0.0, 10.0), square(10.0, 0.0, 10.0)].map(|path| WireFace { path, stroke: 0.75 });
    let out = reshaped(faces.into(), &fx, &GeomContext::of(&n));
    assert_eq!(out.len(), 2);
    let (a, b) = (out[0].path.bounds().unwrap(), out[1].path.bounds().unwrap());
    assert!((a.x0 + 1.0).abs() < 1e-3 && (a.x1 - 11.0).abs() < 1e-3, "{a:?}");
    assert!((b.x0 - 9.0).abs() < 1e-3 && (b.x1 - 21.0).abs() < 1e-3, "{b:?}");
}

/// An Outline Stroke after a 3D effect outlines the stroke each face is drawn with (the seam between
/// faces, or a Classic wire edge), not a plain 1 pt stroke: the wireframe covers the outlined faces
/// the canvas draws, in the same place.
#[test]
fn a_later_outline_stroke_outlines_the_stroke_each_face_is_drawn_with() {
    let outline = || crate::new_effect("path.outlineStroke", &json!({})).unwrap();
    let cases = [
        (super::super::EXTRUDE, json!({})),
        (super::super::CLASSIC_EXTRUDE, json!({})),
        (super::super::CLASSIC_EXTRUDE, json!({"surface": "wireframe"})),
        (super::super::INFLATE, json!({})),
        (ROTATE, json!({})),
        (crate::REVOLVE, json!({})),
        (crate::CLASSIC_REVOLVE, json!({})),
        (crate::CLASSIC_REVOLVE, json!({"surface": "wireframe"})),
    ];
    for (id, params) in cases {
        let mut n = solid_square(id, params.clone());
        n.appearance.effects.push(outline());
        let wire = three_d_wireframe_in(None, &n).unwrap_or_else(|e| panic!("{id} {params}: {e}"));
        let art = if crate::revolve::is_revolve(id) { crate::revolve_art(&n) } else { super::super::solid_art(&n) }.unwrap();
        let drawn = crate::group::shape_art(None, &art, None).unwrap();
        let (w, a) = (bounds(&wire), bounds(&face_paths(&drawn)));
        for (x, y) in [(w.x0, a.x0), (w.y0, a.y0), (w.x1, a.x1), (w.y1, a.y1)] {
            assert!((x - y).abs() < 1e-6, "{id} {params}: {w:?} {a:?}");
        }
    }
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

/// With two 3D effects, a geometry effect between them reshapes the faces of the one drawn only
/// once: Revolve is drawn, so a Transform before it moves its profile and not its faces again.
#[test]
fn a_geometry_effect_before_the_drawn_3d_effect_is_not_applied_again() {
    let mut n = solid_square(crate::REVOLVE, json!({}));
    n.appearance.effects.insert(0, crate::new_effect("distort.transform", &json!({"moveH": 50.0})).unwrap());
    let alone = bounds(&three_d_wireframe_in(None, &n).unwrap());
    n.appearance.effects.insert(0, crate::new_effect(super::super::EXTRUDE, &json!({})).unwrap());
    let both = bounds(&three_d_wireframe_in(None, &n).unwrap());
    assert!((both.x0 - alone.x0).abs() < 1e-6 && (both.x1 - alone.x1).abs() < 1e-6, "{alone:?} {both:?}");
}

/// A clipping group with Revolve and Rotate draws Revolve, which cuts the art's fills as one piece
/// in the top fill's place. Its wireframe is cut the same way, so art with more fills than a 3D
/// effect takes members still expands, to the edges of Revolve alone.
#[test]
fn a_clipping_group_with_revolve_and_rotate_expands_the_art_revolve_cuts() {
    let mut art = Node::path(NodeId(3), square(100.0, 100.0, 100.0), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    let fill = art.appearance.items.iter().find(|i| matches!(i, AppearanceItem::Fill(_))).cloned().unwrap();
    art.appearance.items = vec![fill; super::super::MAX_MEMBERS + 4];
    let mut clip = Node::path(NodeId(2), square(100.0, 100.0, 50.0), Appearance::default());
    if let vectorcraft_doc::NodeKind::Path { clipping, .. } = &mut clip.kind {
        *clipping = true;
    }
    let mut n = Node::new(NodeId(1), vectorcraft_doc::NodeKind::Group { children: vec![Arc::new(clip), Arc::new(art)], clip: true });
    n.appearance.effects.push(crate::new_effect(crate::REVOLVE, &json!({})).unwrap());
    let alone = segments(&three_d_wireframe_in(None, &n).unwrap());
    n.appearance.effects.push(crate::new_effect(ROTATE, &json!({})).unwrap());
    assert!(crate::revolve_art_in(None, &n).is_some(), "the canvas draws Revolve");
    let both = segments(&three_d_wireframe_in(None, &n).unwrap());
    assert_eq!(both, alone);
}

#[test]
fn an_object_without_a_visible_3d_effect_is_an_error() {
    let mut n = solid_square(super::super::EXTRUDE, json!({}));
    n.appearance.effects[0].visible = false;
    assert!(three_d_wireframe_in(None, &n).unwrap_err().contains("no 3D effect"));
    n.appearance.effects.clear();
    assert!(three_d_wireframe_in(None, &n).is_err());
}
