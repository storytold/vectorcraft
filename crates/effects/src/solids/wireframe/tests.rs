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

/// A Transform effect after the 3D effect scales the faces about the center of the drawn art with
/// its shadow, as it scales the surfaces on the canvas.
#[test]
fn a_later_transform_measures_the_faces_with_their_shadow() {
    for id in [super::super::EXTRUDE, crate::REVOLVE] {
        let scale = crate::new_effect("distort.transform", &json!({"scaleH": 50.0, "scaleV": 50.0})).unwrap();
        let mut n = solid_square(id, json!({"shadow": true, "shadowDistance": 100}));
        n.appearance.effects.push(scale.clone());
        let wire = bounds(&three_d_wireframe_in(None, &n).unwrap_or_else(|e| panic!("{id}: {e}")));
        let art = if id == crate::REVOLVE { crate::revolve_art(&n) } else { super::super::solid_art(&n) }.unwrap();
        let mut drawn = art.clone();
        let fx: Vec<&Effect> = art.appearance.effects.iter().collect();
        crate::reshape::reshape_leaves(&mut drawn, &fx, &art.appearance.effects, None);
        // The shadow is the art's first member; the surfaces follow it.
        let surfaces: Vec<PathData> = drawn.children().unwrap().iter().skip(1).flat_map(|c| face_paths(c)).collect();
        let a = bounds(&surfaces);
        for (x, y) in [(wire.x0, a.x0), (wire.y0, a.y0), (wire.x1, a.x1), (wire.y1, a.y1)] {
            assert!((x - y).abs() < 1e-6, "{id}: {wire:?} {a:?}");
        }
        // Without the shadow, the faces are scaled about their own center.
        let mut alone = solid_square(id, json!({}));
        alone.appearance.effects.push(scale);
        assert!((bounds(&three_d_wireframe_in(None, &alone).unwrap()).x0 - wire.x0).abs() > 1.0, "{id}");
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

/// The leaves `art` draws: each path with its fill and stroke.
fn leaves(art: &Node) -> Vec<(PathData, bool, Option<StrokeLayer>)> {
    let mut out = vec![];
    art.walk(&mut |c| {
        if let Some(p) = c.path_data() {
            out.push((p.clone(), c.appearance.fill().is_some(), c.appearance.stroke().cloned()));
        }
    });
    out
}

/// The art the canvas draws for `n`: Revolve's, else the other 3D effects'.
fn drawn(n: &Node) -> Node {
    crate::revolve_art(n).or_else(|| super::super::solid_art(n)).unwrap_or_else(|| panic!("no art for {n:?}"))
}

const WIRE: [&str; 4] = [super::super::EXTRUDE, crate::REVOLVE, super::super::INFLATE, ROTATE];

/// Render Settings › Wireframe: each of the panel's four effects draws the edges Expand as
/// Wireframes gives, each a 0.5 pt black stroke with round joins and no fill, in place of its
/// shaded faces; shaded again, it draws filled faces.
#[test]
fn wireframe_mode_draws_the_expanded_edges_as_strokes() {
    for id in WIRE {
        let n = solid_square(id, json!({"render": {"mode": "wireframe"}}));
        let art = leaves(&drawn(&n));
        let wire = three_d_wireframe_in(None, &n).unwrap_or_else(|e| panic!("{id}: {e}"));
        assert_eq!(art.len(), wire.len(), "{id}");
        for ((path, filled, stroke), edge) in art.iter().zip(&wire) {
            assert!(!filled, "{id}: an edge has no fill");
            let stroke = stroke.as_ref().unwrap_or_else(|| panic!("{id}: an edge is stroked"));
            assert_eq!((stroke.width, stroke.join, &stroke.paint), (WIRE_WEIGHT, LineJoin::Round, &Paint::solid(Color::BLACK)), "{id}");
            assert_eq!(segments(std::slice::from_ref(path)), segments(std::slice::from_ref(edge)), "{id}");
        }
        let shaded = leaves(&drawn(&solid_square(id, json!({"render": {"mode": "shaded"}}))));
        assert!(shaded.iter().any(|(_, filled, _)| *filled), "{id}: shaded faces are filled");
    }
    // The box turned off axis shows the nine edges of its three faces.
    let n = solid_square(super::super::EXTRUDE, json!({"render": {"mode": "wireframe"}}));
    let paths: Vec<PathData> = leaves(&drawn(&n)).into_iter().map(|(p, ..)| p).collect();
    assert_eq!(segments(&paths).len(), 9);
}

/// The wireframe stays live: a deeper extrusion and a turned view redraw the edges.
#[test]
fn wireframe_mode_follows_the_effects_settings() {
    let paths = |p: Value| -> Vec<PathData> { leaves(&drawn(&solid_square(super::super::EXTRUDE, p))).into_iter().map(|(p, ..)| p).collect() };
    let flat = bounds(&paths(json!({"render": {"mode": "wireframe"}, "depth": 10.0})));
    let deep = bounds(&paths(json!({"render": {"mode": "wireframe"}, "depth": 200.0})));
    assert!(deep.width() > flat.width() + 1.0 || deep.height() > flat.height() + 1.0, "{flat:?} {deep:?}");
    let turned = paths(json!({"render": {"mode": "wireframe"}, "rotationY": 60.0}));
    assert_ne!(segments(&turned), segments(&paths(json!({"render": {"mode": "wireframe"}}))));
}

/// In wireframe mode no shadow and no graphic is drawn: only the edges.
#[test]
fn wireframe_mode_draws_no_shadow() {
    for id in WIRE {
        let n = solid_square(id, json!({"render": {"mode": "wireframe"}, "shadow": true}));
        let art = leaves(&drawn(&n));
        assert!(!art.is_empty() && art.iter().all(|(_, filled, stroke)| !filled && stroke.is_some()), "{id}");
    }
}

/// The mode works where the effect is on a fill and on a group: the fill's solid and every member's
/// solid draw as edges, and the geometry effects after a fill's effect still apply to them.
#[test]
fn wireframe_mode_on_a_fill_and_on_a_group() {
    for id in WIRE {
        let mut n = solid_square(id, json!({}));
        let mut fx = n.appearance.effects.remove(0);
        fx.params["render"] = json!({"mode": "wireframe"});
        let fill = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Fill(_))).unwrap();
        fill.effects_mut().push(fx.clone());
        fill.effects_mut().push(crate::new_effect("distort.transform", &json!({"moveH": 500.0})).unwrap());
        let art = leaves(&drawn(&n));
        assert!(!art.is_empty() && art.iter().all(|(_, filled, stroke)| !filled && stroke.is_some()), "{id} on a fill");

        let member = |x: f64, id: u64| {
            Arc::new(Node::path(NodeId(id), square(x, 100.0, 50.0), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0)))
        };
        let mut g = Node::group(NodeId(1), vec![member(100.0, 2), member(200.0, 3)]);
        g.appearance.effects.push(fx);
        let art = leaves(&drawn(&g));
        assert!(!art.is_empty() && art.iter().all(|(_, filled, stroke)| !filled && stroke.is_some()), "{id} on a group");
        // Both members' edges are drawn.
        let paths: Vec<PathData> = art.into_iter().map(|(p, ..)| p).collect();
        assert!(bounds(&paths).width() > 100.0, "{id}");
    }
}

/// The mesh shows every edge, as Expand as Wireframes gives them, whatever the opacity of the fill
/// and the opacity masks of the group's members; the object's own opacity stays on the art. Two
/// members that touch, one at 50% and one masked, draw their shared edge once: seven edges.
#[test]
fn the_mesh_shows_every_edge_whatever_the_members_transparency() {
    let member = |x: f64, id: u64, opacity: f32| {
        let mut n = Node::path(NodeId(id), square(x, 100.0, 50.0), Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
        if let Some(AppearanceItem::Fill(f)) = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Fill(_))) {
            f.opacity = opacity;
        }
        n
    };
    let mut masked = member(150.0, 3, 1.0);
    masked.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(member(0.0, 900, 1.0), true)));
    let mut g = Node::group(NodeId(1), vec![Arc::new(member(100.0, 2, 0.5)), Arc::new(masked)]);
    g.opacity = 0.75;
    g.appearance.effects.push(crate::new_effect(ROTATE, &json!({"render": {"mode": "wireframe"}})).unwrap());
    let art = drawn(&g);
    assert_eq!(art.opacity, 0.75);
    let paths: Vec<PathData> = leaves(&art).into_iter().map(|(p, ..)| p).collect();
    assert_eq!(segments(&paths).len(), 7, "{paths:?}");
    assert_eq!(segments(&paths).len(), segments(&three_d_wireframe_in(None, &g).unwrap()).len());
    let mut scoped = 0;
    for c in art.children().into_iter().flatten() {
        c.walk(&mut |c| scoped += usize::from(c.extras().mask.is_some() || c.opacity < 1.0));
    }
    assert_eq!(scoped, 0, "no part of the mesh is masked or faded");
    // A fill at 0% still shows its mesh.
    for id in WIRE {
        let mut n = solid_square(id, json!({"render": {"mode": "wireframe"}}));
        if let Some(AppearanceItem::Fill(f)) = n.appearance.items.iter_mut().find(|i| matches!(i, AppearanceItem::Fill(_))) {
            f.opacity = 0.0;
        }
        assert!(!leaves(&drawn(&n)).is_empty(), "{id}");
    }
}

/// Expand Appearance of a Revolve in wireframe mode gives the stroked edges, not the shaded faces.
#[test]
fn expanding_a_revolve_in_wireframe_mode_gives_the_edges() {
    let n = solid_square(crate::REVOLVE, json!({"render": {"mode": "wireframe"}}));
    let art = leaves(&crate::revolve::expand_revolve_art(&n).unwrap());
    assert_eq!(art.len(), three_d_wireframe_in(None, &n).unwrap().len());
    assert!(art.iter().all(|(_, filled, stroke)| !filled && stroke.as_ref().is_some_and(|s| s.width == WIRE_WEIGHT)));
}

/// `render` is checked before an edit: an object holding only `mode`, `shaded` or `wireframe`, and
/// only on the panel's four effects.
#[test]
fn render_settings_are_validated() {
    for id in WIRE {
        for good in [json!({}), json!({"render": {}}), json!({"render": {"mode": "shaded"}}), json!({"render": {"mode": "wireframe"}})] {
            assert_eq!(crate::validate_three_d_params(id, &good), Ok(()), "{id} {good}");
        }
        for bad in [
            json!({"render": "wireframe"}),
            json!({"render": null}),
            json!({"render": {"mode": "raytrace"}}),
            json!({"render": {"mode": true}}),
            json!({"render": {"mode": "wireframe", "quality": "high"}}),
        ] {
            let e = crate::validate_three_d_params(id, &bad).unwrap_err();
            assert_eq!(e, "`render` must be an object holding only `mode`, set to shaded or wireframe", "{id} {bad}");
        }
        // The command check on the object refuses it too.
        let n = solid_square(id, json!({}));
        assert!(crate::validate_three_d(&n, id, &json!({"render": {"mode": "x"}})).is_err(), "{id}");
    }
    for id in [super::super::CLASSIC_EXTRUDE, crate::CLASSIC_REVOLVE, super::super::CLASSIC_ROTATE] {
        let e = crate::validate_three_d_params(id, &json!({"render": {"mode": "wireframe"}})).unwrap_err();
        assert!(e.contains("`render`"), "{id}: {e}");
    }
}

/// A Revolve whose shaded surface is past the budget of paint copies (a gradient with many stops) is
/// refused shaded, but in wireframe mode, which draws only the black edges, it is accepted and drawn:
/// on the object and on a fill.
#[test]
fn wireframe_mode_is_checked_against_the_edges_it_draws() {
    use vectorcraft_doc::color::{Gradient, GradientKind, GradientPaint, GradientStop};
    let n = solid_square(crate::REVOLVE, json!({}));
    let path = n.path_data().unwrap().clone();
    let faces = vectorcraft_three_d::revolve(&path, path.bounds().unwrap(), crate::revolve::revolve_options(&json!({}))).unwrap().len();
    let stops = crate::revolve::MAX_PAINT_COPIES / (faces * 4) / std::mem::size_of::<GradientStop>() + 1;
    let stops = (0..stops).map(|i| GradientStop::new(i as f32 / stops as f32, Color::BLACK)).collect();
    let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(GradientKind::Linear, stops))));
    let mut n = n;
    n.appearance = Appearance::basic(gradient, Paint::None, 0.0).into();
    let shaded = json!({});
    let wire = json!({"render": {"mode": "wireframe"}});
    let refused = crate::validate_revolve(&n, &shaded).unwrap_err();
    assert!(refused.contains("simplify the gradient"), "{refused}");
    assert_eq!(crate::validate_revolve(&n, &wire), Ok(()));
    n.appearance.effects.push(crate::new_effect(crate::REVOLVE, &wire).unwrap());
    assert!(leaves(&drawn(&n)).iter().all(|(_, filled, stroke)| !filled && stroke.is_some()));
    // On the fill.
    let fx = n.appearance.effects.remove(0);
    let at = n.appearance.items.iter().position(|i| matches!(i, AppearanceItem::Fill(_))).unwrap();
    assert!(crate::revolve::validate_item_revolve(&n, &shaded, at, 0).is_err());
    assert_eq!(crate::revolve::validate_item_revolve(&n, &wire, at, 0), Ok(()));
    n.appearance.items[at].effects_mut().push(fx);
    assert!(leaves(&drawn(&n)).iter().all(|(_, filled, stroke)| !filled && stroke.is_some()));
}

/// A 3D (Classic) effect ignores a `render` it does not take in a loaded file: it draws as before.
#[test]
fn a_classic_effect_is_not_drawn_as_a_render_wireframe() {
    let mut n = solid_square(super::super::CLASSIC_EXTRUDE, json!({}));
    n.appearance.effects[0].params["render"] = json!({"mode": "wireframe"});
    assert!(!renders_wireframe(&n.appearance.effects[0]));
    assert!(leaves(&drawn(&n)).iter().any(|(_, filled, _)| *filled));
}

#[test]
fn an_object_without_a_visible_3d_effect_is_an_error() {
    let mut n = solid_square(super::super::EXTRUDE, json!({}));
    n.appearance.effects[0].visible = false;
    assert!(three_d_wireframe_in(None, &n).unwrap_err().contains("no 3D effect"));
    n.appearance.effects.clear();
    assert!(three_d_wireframe_in(None, &n).is_err());
}
