//! Original ordinary Selection body drags against a live, centered stroke (#6595).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::{Prefs, Session, ViewInfo};
use vectorcraft_geom::{Point, Rect, Vec2};
use vectorcraft_tools::{PointerEvent, PointerKind};

struct Fixture {
    session: Session,
    view: ViewInfo,
    target: NodeId,
    moving: NodeId,
}

fn rectangle(session: &mut Session, x: f64, y: f64, width: f64, height: f64) -> NodeId {
    NodeId(session.execute("shape.rectangle", &json!({"x": x, "y": y, "width": width, "height": height})).unwrap()["id"].as_u64().unwrap())
}

fn fixture(stroke_edges: Option<bool>) -> Fixture {
    let mut session = Session::new();
    let mut saved = serde_json::to_value(&session.prefs).unwrap();
    saved["snappingTolerance"] = json!(4.0);
    saved["alignmentGuides"] = json!(true);
    if let Some(on) = stroke_edges {
        // This is deliberately an old-source-compatible saved preference, not an unknown
        // prefs.set command or a reference to a future typed field. Old serde ignores the key.
        saved["snapToStrokeEdges"] = json!(on);
    }
    let prefs: Prefs = serde_json::from_value(saved).unwrap();
    session.apply_prefs(prefs);
    session.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();

    let target = rectangle(&mut session, 100.0, 100.0, 100.0, 100.0);
    session.execute("select.set", &json!({"ids": [target.0]})).unwrap();
    session.execute("paint.setFill", &json!({"none": true})).unwrap();
    session.execute("paint.setStroke", &json!({"color": "#101010"})).unwrap();
    session.execute("stroke.set", &json!({"weight": 10.0, "align": "center"})).unwrap();

    let moving = rectangle(&mut session, 300.0, 300.0, 60.0, 40.0);
    session.execute("select.set", &json!({"ids": [moving.0]})).unwrap();
    session.execute("paint.setFill", &json!({"color": "#202020"})).unwrap();
    session.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let view = ViewInfo {
        smart_guides: true,
        guides: false,
        snap_to_grid: false,
        snap_to_pixel: false,
        snap_to_point: false,
        show_bbox: false,
        corner_widgets: false,
        ..ViewInfo::default()
    };
    session.select_tool("selection", view).unwrap();
    Fixture { session, view, target, moving }
}

fn pointer(f: &mut Fixture, kind: PointerKind, point: Point) {
    f.session.pointer(&PointerEvent::new(kind, point.x, point.y), f.view).unwrap();
    // Normal host completion of any edit made by this event; no direct document edit.
    for _ in 0..4 {
        if !f.session.tool_editing() {
            return;
        }
        f.session.tool_poll(f.view, true).unwrap();
    }
    assert!(!f.session.tool_editing(), "the ordinary pointer event must settle");
}

fn body_drag_and_undo(f: &mut Fixture, unsnapped_left: f64) -> Rect {
    let state = f.session.doc().unwrap();
    let before = state.doc.clone();
    let selection = state.selection.clone();
    let history = (state.history.undo.len(), state.history.redo.len());
    let target_before = state.doc.node(f.target).unwrap().clone();
    let moving_before = state.doc.node(f.moving).unwrap().clone();
    assert_eq!(target_before.geometric_bounds().unwrap(), Rect::new(100.0, 100.0, 200.0, 200.0));
    assert_eq!(moving_before.geometric_bounds().unwrap(), Rect::new(300.0, 300.0, 360.0, 340.0));
    assert_eq!(selection.objects, vec![f.moving]);
    assert!(selection.anchors.is_empty());

    // More than the existing 6pt hover reach from the center AND every path edge/anchor.
    // With widgets hidden this is an actual painted BODY hit, held 23pt/17pt from top-left.
    let press = Point::new(323.0, 317.0);
    let end = Point::new(unsnapped_left + 23.0, 157.0);
    pointer(f, PointerKind::Down, press);
    pointer(f, PointerKind::Drag, end);
    assert!(f.session.tool_transforming(), "the Selection body drag must move, not scale or marquee");
    pointer(f, PointerKind::Up, end);
    assert!(!f.session.tool_busy());

    let state = f.session.doc().unwrap();
    let moving_after = state.doc.node(f.moving).unwrap();
    let actual = moving_after.geometric_bounds().unwrap();
    let delta = Vec2::new(actual.x0 - 300.0, -160.0);
    assert_eq!((actual.y0, actual.width(), actual.height()), (140.0, 60.0, 40.0));
    assert_eq!(state.doc.node(f.target).unwrap(), &target_before, "the live target's entire geometry/stroke must stay unchanged");
    assert_eq!(state.selection, selection);
    assert_eq!(moving_after.appearance, moving_before.appearance);
    let old_anchors: Vec<_> = moving_before.path_data().unwrap().anchors().collect();
    let new_anchors: Vec<_> = moving_after.path_data().unwrap().anchors().collect();
    assert_eq!(old_anchors.len(), new_anchors.len());
    for ((old_sub, old_index, old), (new_sub, new_index, new)) in old_anchors.into_iter().zip(new_anchors) {
        assert_eq!((new_sub, new_index, new.kind), (old_sub, old_index, old.kind));
        for (a, b) in [(old.p, new.p), (old.h_in, new.h_in), (old.h_out, new.h_out)] {
            assert!(((b - a) - delta).hypot() < 1e-9, "every selected anchor/handle must move by {delta:?}");
        }
    }
    // Normalize ONLY a comparison copy: all document data outside the selected node must match.
    let mut other_data = (*state.doc).clone();
    *other_data.node_mut(f.moving).unwrap() = moving_before;
    assert_eq!(other_data, *before, "only the selected object may change during this drag");
    assert_eq!((state.history.undo.len(), state.history.redo.len()), (history.0 + 1, 0));
    f.session.execute("edit.undo", &json!({})).unwrap();
    let state = f.session.doc().unwrap();
    assert_eq!(state.doc, before, "one ordinary Undo restores the exact original document");
    assert_eq!(state.selection, selection);
    assert_eq!(state.history.undo.len(), history.0);
    actual
}

#[test]
fn stroke_edge_default_off_body_drag_preserves_geometric_alignment() {
    for option in [None, Some(false)] {
        let mut f = fixture(option);
        // Unsnapped moving left202 is2pt from the target's geometric right200, within tolerance4.
        assert_eq!(body_drag_and_undo(&mut f, 202.0).x0, 200.0, "default/OFF must keep geometric alignment");
    }
}

#[test]
fn stroke_edge_opt_in_body_drag_targets_visible_outline() {
    let mut f = fixture(Some(true));
    // Unsnapped moving left207 is7pt from path200 but2pt from the live centered stroke edge205.
    // Old source ignores the future option and leaves207; the requested feature must reach205.
    assert_eq!(body_drag_and_undo(&mut f, 207.0).x0, 205.0, "ON must snap the moving edge to the live stroke outline");
}

// Additional original scenes. Setup may edit appearance data through the public Session API;
// every observed snap is produced by a real drawing, Selection move, or bounding-box gesture.
use vectorcraft_doc::{Effect, WidthProfile};
use vectorcraft_geom::{FillRule, PathData};
use vectorcraft_tools::Mods;

fn choose(f: &mut Fixture, ids: &[NodeId]) {
    f.session.execute("select.set", &json!({"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
}

fn choose_target(f: &mut Fixture) {
    let id = f.target;
    choose(f, &[id]);
}

fn choose_moving(f: &mut Fixture) {
    let id = f.moving;
    choose(f, &[id]);
}

fn edges(f: &mut Fixture, on: bool) {
    let mut saved = serde_json::to_value(&f.session.prefs).unwrap();
    saved["snapToStrokeEdges"] = json!(on);
    f.session.apply_prefs(serde_json::from_value(saved).unwrap());
}

fn event(f: &mut Fixture, kind: PointerKind, at: Point, mods: Mods) {
    f.session.pointer(&PointerEvent::new(kind, at.x, at.y).with_mods(mods), f.view).unwrap();
    for _ in 0..4 {
        if !f.session.tool_editing() {
            return;
        }
        f.session.tool_poll(f.view, true).unwrap();
    }
    assert!(!f.session.tool_editing());
}

fn replace_target(f: &mut Fixture, svg: &str, width: f64) {
    choose_target(f);
    f.session.execute("edit.clear", &json!({})).unwrap();
    f.target = NodeId(f.session.execute("path.create", &json!({"d": svg})).unwrap()["id"].as_u64().unwrap());
    choose_target(f);
    f.session.execute("paint.setFill", &json!({"none": true})).unwrap();
    f.session.execute("paint.setStroke", &json!({"color": "#101010"})).unwrap();
    f.session.execute("stroke.set", &json!({"weight": width, "align": "center"})).unwrap();
    choose_moving(f);
}

fn outline(f: &Fixture) -> PathData {
    let n = f.session.doc().unwrap().doc.node(f.target).unwrap();
    vectorcraft_render::effects::stroke::outline_region(n.path_data().unwrap(), FillRule::NonZero, n.appearance.stroke().unwrap())
}

fn near(a: Point, b: Point, why: &str) {
    assert!(a.distance(b) < 0.15, "{why}: actual {a:?}, expected {b:?}");
}

// A true point consumer: the Rectangle tool snaps its press before drawing ordinary new art.
// Existing art, including the live stroke and all its resources, must survive one Undo exactly.
fn draw_from_and_undo(f: &mut Fixture, raw: Point) -> Point {
    choose(f, &[]);
    f.session.select_tool("rectangle", f.view).unwrap();
    let state = f.session.doc().unwrap();
    let before = state.doc.clone();
    let history = state.history.undo.len();
    event(f, PointerKind::Down, raw, Mods::default());
    let end = raw + Vec2::new(37.0, 29.0);
    event(f, PointerKind::Drag, end, Mods::default());
    event(f, PointerKind::Up, end, Mods::default());
    let state = f.session.doc().unwrap();
    assert_eq!(state.selection.objects.len(), 1);
    let added = state.selection.objects[0];
    assert!(before.node(added).is_none(), "the gesture must draw, not transform a pre-existing object");
    let bounds = state.doc.node(added).unwrap().geometric_bounds().unwrap();
    assert_eq!(state.history.undo.len(), history + 1);
    f.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(f.session.doc().unwrap().doc, before, "drawing Undo preserves every original node and resource");
    Point::new(bounds.x0, bounds.y0)
}

fn move_from_and_undo(f: &mut Fixture, start: Point, end: Point, mods: Mods) -> Rect {
    choose_moving(f);
    f.session.select_tool("selection", f.view).unwrap();
    let state = f.session.doc().unwrap();
    let before = state.doc.clone();
    let selected = state.selection.clone();
    let history = state.history.undo.len();
    let old = state.doc.node(f.moving).unwrap().clone();
    event(f, PointerKind::Down, start, mods);
    event(f, PointerKind::Drag, end, mods);
    assert!(f.session.tool_transforming(), "the public Selection gesture must be an art transform");
    event(f, PointerKind::Up, end, mods);
    let state = f.session.doc().unwrap();
    let now = state.doc.node(f.moving).unwrap();
    let bounds = now.geometric_bounds().unwrap();
    assert_eq!(now.appearance, old.appearance);
    assert_eq!(state.selection, selected);
    let mut untouched = (*state.doc).clone();
    *untouched.node_mut(f.moving).unwrap() = old;
    assert_eq!(untouched, *before, "no target, clipping ancestor, or resource may change");
    assert_eq!(state.history.undo.len(), history + 1);
    f.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(f.session.doc().unwrap().doc, before);
    assert_eq!(f.session.doc().unwrap().selection, selected);
    bounds
}

#[test]
fn stroke_edge_alignment_inner_outer_bands_are_real_body_targets() {
    for (align, raw_left, expected_left) in [
        ("center", 133.0, 135.0),  // mover RIGHT193 approaches center-stroke INNER195 from inside
        ("inside", 128.0, 130.0),  // RIGHT188 approaches inner190; no other mover edge crosses the band
        ("inside", 202.0, 200.0),  // outer200 coincides with the legitimate source path
        ("outside", 138.0, 140.0), // RIGHT198 approaches inner200 from inside
        ("outside", 212.0, 210.0), // LEFT212 approaches outside outer210 from outside
    ] {
        let mut f = fixture(Some(true));
        choose_target(&mut f);
        f.session.execute("stroke.set", &json!({"align": align})).unwrap();
        choose_moving(&mut f);
        assert_eq!(body_drag_and_undo(&mut f, raw_left).x0, expected_left, "{align}");
    }
}

fn asymmetric_curve() -> (Fixture, PathData, Point, Point) {
    let mut f = fixture(Some(true));
    replace_target(&mut f, "M100 100 C100 230 250 240 240 100", 24.0);
    let id = f.target;
    f.session
        .edit("Original asymmetric width profile", |d, _| {
            d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().profile =
                Some(WidthProfile { points: vec![(0.0, 0.3, 1.6), (0.43, 1.9, 0.55), (1.0, 0.4, 1.2)] });
            Ok(())
        })
        .unwrap();
    let region = outline(&f);
    // A left-side interior point, away from every outline-box extremum and source anchor.
    let outside = Point::new(100.0, 184.0);
    let q = region.nearest(outside).unwrap().3;
    let b = region.bounds().unwrap();
    assert!(
        q.x > b.x0 + 5.0 && q.x < b.x1 - 5.0 && q.y > b.y0 + 5.0 && q.y < b.y1 - 5.0,
        "original curved probe must be interior to every AABB extremum: q={q:?}, box={b:?}"
    );
    let v = outside - q;
    let raw = q + v * (2.0 / v.hypot());
    assert!(f.session.doc().unwrap().doc.node(f.target).unwrap().path_data().unwrap().nearest(raw).unwrap().4 > 4.0);
    (f, region, q, raw)
}

#[test]
fn stroke_edge_curved_asymmetric_interior_reaches_point_and_body_consumers() {
    let (mut f, region, q, raw) = asymmetric_curve();
    near(draw_from_and_undo(&mut f, raw), q, "point consumer must reach the painted curved contour, not its AABB");
    // Upper-right mover corner approaches the curve from its empty left/below side.
    let destination = Point::new(raw.x - 60.0, raw.y);
    let moved = move_from_and_undo(&mut f, Point::new(323.0, 317.0), destination + Vec2::new(23.0, 17.0), Mods::default());
    let corner = Point::new(moved.x1, moved.y0);
    assert!(region.nearest(corner).unwrap().4 < 0.15, "the BODY edge must contact the independent rendered contour: {corner:?}");
    assert!(corner.distance(raw) > 0.5, "the body witness must actually snap");
}

#[test]
fn stroke_edge_dashes_nonpaint_hidden_and_locked_follow_painted_geometry() {
    let mut f = fixture(Some(true));
    replace_target(&mut f, "M100 100 L300 100", 10.0);
    choose_target(&mut f);
    f.session.execute("stroke.set", &json!({"dash": [20.0, 20.0], "cap": "butt"})).unwrap();
    near(draw_from_and_undo(&mut f, Point::new(110.0, 107.0)), Point::new(110.0, 105.0), "painted dash");
    near(draw_from_and_undo(&mut f, Point::new(130.0, 107.0)), Point::new(130.0, 107.0), "dash gap must not acquire a solid-stroke boundary");
    choose_target(&mut f);
    f.session.execute("stroke.set", &json!({"dash": null, "cap": "round"})).unwrap();
    near(draw_from_and_undo(&mut f, Point::new(93.0, 100.0)), Point::new(95.0, 100.0), "the real round cap extends beyond the source endpoint");
    let id = f.target;
    f.session
        .edit("Hidden original target", |d, _| {
            d.node_mut(id).unwrap().visible = false;
            Ok(())
        })
        .unwrap();
    near(draw_from_and_undo(&mut f, Point::new(137.0, 107.0)), Point::new(137.0, 107.0), "hidden stroke");
    f.session
        .edit("Visible locked target", |d, _| {
            let n = d.node_mut(id).unwrap();
            n.visible = true;
            n.locked = true;
            Ok(())
        })
        .unwrap();
    near(draw_from_and_undo(&mut f, Point::new(137.0, 107.0)), Point::new(137.0, 105.0), "visible locked stroke remains a target");
    f.session
        .edit("Unpainted original target", |d, _| {
            d.node_mut(id).unwrap().appearance.stroke_mut().unwrap().visible = false;
            Ok(())
        })
        .unwrap();
    near(draw_from_and_undo(&mut f, Point::new(137.0, 107.0)), Point::new(137.0, 107.0), "invisible stroke row");
    f.session
        .edit("Visible unlocked row for None-paint control", |d, _| {
            let n = d.node_mut(id).unwrap();
            n.locked = false;
            n.appearance.stroke_mut().unwrap().visible = true;
            Ok(())
        })
        .unwrap();
    f.session.execute("paint.setStroke", &json!({"id": id.0, "none": true})).unwrap();
    near(draw_from_and_undo(&mut f, Point::new(137.0, 107.0)), Point::new(137.0, 107.0), "None paint must not create a contour");
}

#[test]
fn stroke_edge_brush_uses_stroke_pieces_without_original_fill_effect_leak() {
    let mut f = fixture(Some(true));
    let art = rectangle(&mut f.session, 10.0, 10.0, 20.0, 2.0);
    choose(&mut f, &[art]);
    f.session.execute("paint.setFill", &json!({"color": "#203040"})).unwrap();
    f.session.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let brush =
        f.session.execute("brush.new", &json!({"type": "art", "name": "Original two-unit ribbon"})).unwrap()["name"].as_str().unwrap().to_owned();
    f.session.execute("edit.clear", &json!({})).unwrap();
    choose_target(&mut f);
    f.session.execute("paint.setFill", &json!({"color": "#c04040"})).unwrap();
    f.session.execute("brush.apply", &json!({"name": brush})).unwrap();
    let id = f.target;
    f.session
        .edit("Move only the original fill far away", |d, _| {
            d.node_mut(id).unwrap().appearance.fill_mut().unwrap().effects.push(Effect {
                id: "distort.transform".into(),
                params: json!({"moveH": 300.0}),
                visible: true,
            });
            Ok(())
        })
        .unwrap();
    let state = f.session.doc().unwrap();
    let n = state.doc.node(id).unwrap();
    let definition = vectorcraft_brush::library(&state.doc).into_iter().find(|b| b.name == brush).unwrap();
    let pieces = vectorcraft_brush::stroke_pieces(&state.doc, &definition, &n.path_data().unwrap().to_bezpath(), n.appearance.stroke().unwrap());
    let probe = Point::new(207.0, 137.0);
    let mut nearest: Option<(Point, f64)> = None;
    for piece in pieces {
        piece.walk(&mut |p| {
            if let Some(path) = p.path_data()
                && !p.appearance.fill_paint().is_none()
                && let Some((_, _, _, q, d)) = path.nearest(probe)
                && nearest.is_none_or(|(_, old)| d < old)
            {
                nearest = Some((q, d));
            }
        });
    }
    let expected = nearest.expect("the original filled brush art must paint vector pieces").0;
    near(expected, Point::new(210.0, 137.0), "two-unit art at stroke weight10 has a twenty-unit ribbon");
    near(draw_from_and_undo(&mut f, probe), expected, "do not invent a closer plain-stroke205 boundary from the fill's original path");
}

#[test]
fn stroke_edge_compound_own_stroke_obeys_parent_transform_without_scaling_strokes() {
    let mut f = fixture(Some(true));
    let hole = rectangle(&mut f.session, 120.0, 120.0, 40.0, 40.0);
    let target = f.target;
    choose(&mut f, &[target, hole]);
    let compound = NodeId(f.session.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    choose(&mut f, &[compound]);
    let group = NodeId(f.session.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    f.session
        .edit("Original parent Transform effect", |d, _| {
            d.node_mut(group).unwrap().appearance.effects.push(Effect {
                id: "distort.transform".into(),
                params: json!({"scaleH": 200.0, "scaleV": 200.0, "reference": 0, "strokes": false}),
                visible: true,
            });
            Ok(())
        })
        .unwrap();
    assert_eq!(f.session.doc().unwrap().doc.node(compound).unwrap().appearance.stroke().unwrap().width, 10.0);
    near(
        draw_from_and_undo(&mut f, Point::new(307.0, 177.0)),
        Point::new(305.0, 177.0),
        "parent doubles the compound path, not its ten-point stroke",
    );
}

fn clipping_group(f: &mut Fixture, contents: NodeId, rect: Rect) -> (NodeId, NodeId) {
    let mask = rectangle(&mut f.session, rect.x0, rect.y0, rect.width(), rect.height());
    choose(f, &[contents, mask]);
    let group = NodeId(f.session.execute("object.clippingMask.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    (group, mask)
}

#[test]
fn stroke_edge_nested_ancestor_clips_remove_hidden_contours_but_keep_clip_owner_stroke() {
    let mut f = fixture(Some(true));
    let target = f.target;
    let (inner, inner_mask) = clipping_group(&mut f, target, Rect::new(180.0, 110.0, 220.0, 190.0));
    // Keep the outer mask's genuine path beyond4pt reach of the clip-owner stroke probe227.
    let (_outer, _) = clipping_group(&mut f, inner, Rect::new(170.0, 130.0, 260.0, 170.0));
    near(draw_from_and_undo(&mut f, Point::new(207.0, 137.0)), Point::new(205.0, 137.0), "retained portion of nested clipped stroke");
    near(
        draw_from_and_undo(&mut f, Point::new(207.0, 182.0)),
        Point::new(207.0, 182.0),
        "point outside ancestor clip must not see its removed stroke",
    );
    choose(&mut f, &[inner_mask]);
    f.session.execute("paint.setStroke", &json!({"color": "#003070"})).unwrap();
    f.session.execute("stroke.set", &json!({"weight": 10.0})).unwrap();
    near(
        draw_from_and_undo(&mut f, Point::new(227.0, 137.0)),
        Point::new(225.0, 137.0),
        "clipping path's own painted stroke is outside its own clip, inside its ancestor",
    );
}

#[test]
fn stroke_edge_shift_constrained_body_contact_preserves_horizontal_constraint() {
    let (mut f, region, _, raw) = asymmetric_curve();
    choose_moving(&mut f);
    // Start clear of the curve; the raw drag is exactly horizontal. Setup is not the observed move.
    let before = f.session.doc().unwrap().doc.node(f.moving).unwrap().geometric_bounds().unwrap();
    let source = Point::new(raw.x - 60.0 + 300.0, raw.y);
    f.session.execute("object.transform", &json!({"matrix": [1.0, 0.0, 0.0, 1.0, source.x-before.x0, source.y-before.y0]})).unwrap();
    let end = Point::new(raw.x - 60.0 + 23.0, raw.y + 17.0);
    let moved = move_from_and_undo(&mut f, source + Vec2::new(23.0, 17.0), end, Mods { shift: true, ..Mods::default() });
    assert!((moved.y0 - source.y).abs() < 1e-6, "snapping must stay on the selected horizontal Shift constraint");
    assert!(region.nearest(Point::new(moved.x1, moved.y0)).unwrap().4 < 0.15, "constrained BODY contact must reach the curved stroke");
}

#[test]
fn stroke_edge_proportional_bbox_resize_preserves_anchor_and_aspect() {
    let mut f = fixture(Some(true));
    choose_moving(&mut f);
    f.session.execute("object.transform", &json!({"matrix": [1.0, 0.0, 0.0, 1.0, -260.0, -260.0]})).unwrap();
    f.view.show_bbox = true;
    let moved = move_from_and_undo(&mut f, Point::new(100.0, 80.0), Point::new(207.0, 40.0 + 167.0 / 1.5), Mods { shift: true, ..Mods::default() });
    near(Point::new(moved.x0, moved.y0), Point::new(40.0, 40.0), "opposite resize corner remains fixed");
    assert!((moved.width() / moved.height() - 1.5).abs() < 1e-6);
    near(Point::new(moved.x1, moved.y1), Point::new(205.0, 150.0), "proportional corner reaches the painted edge");
}

#[test]
fn stroke_edge_cache_toggle_and_copy_keep_owner_exclusion_and_original_art() {
    let mut f = fixture(Some(true));
    assert_eq!(body_drag_and_undo(&mut f, 207.0).x0, 205.0);
    edges(&mut f, false);
    assert_eq!(body_drag_and_undo(&mut f, 207.0).x0, 207.0, "OFF invalidates contour targets in the next gesture");
    edges(&mut f, true);
    assert_eq!(body_drag_and_undo(&mut f, 207.0).x0, 205.0, "ON restores contours after Undo and a preference change");
    f.view.smart_guides = false;
    f.view.snap_to_point = true;
    near(draw_from_and_undo(&mut f, Point::new(207.0, 137.0)), Point::new(207.0, 137.0), "Snap to Point alone must not acquire stroke contours");
    f.view.smart_guides = true;
    f.view.snap_to_point = false;
    choose_moving(&mut f);
    f.session.select_tool("selection", f.view).unwrap();
    let before = f.session.doc().unwrap().doc.clone();
    let history = f.session.doc().unwrap().history.undo.len();
    let alt = Mods { alt: true, ..Mods::default() };
    event(&mut f, PointerKind::Down, Point::new(323.0, 317.0), alt);
    event(&mut f, PointerKind::Drag, Point::new(230.0, 157.0), alt);
    assert!(f.session.tool_transforming(), "Alt-copy must be a Selection transform, not a new drawing");
    event(&mut f, PointerKind::Up, Point::new(230.0, 157.0), alt);
    let state = f.session.doc().unwrap();
    assert_eq!(state.doc.node(f.moving), before.node(f.moving), "Alt-copy retains the original complete node");
    assert_eq!(state.doc.node(f.target), before.node(f.target));
    assert_eq!(state.selection.objects.len(), 1);
    let copied = state.selection.objects[0];
    assert_ne!(copied, f.moving);
    assert_eq!(state.doc.node(copied).unwrap().geometric_bounds().unwrap().x0, 205.0);
    assert_eq!(state.history.undo.len(), history + 1);
    f.session.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(f.session.doc().unwrap().doc, before);
    choose_moving(&mut f);
    f.session.execute("paint.setStroke", &json!({"color": "#506070"})).unwrap();
    f.session.execute("stroke.set", &json!({"weight": 10.0})).unwrap();
    let self_move = move_from_and_undo(&mut f, Point::new(323.0, 317.0), Point::new(330.0, 317.0), Mods::default());
    assert_eq!(self_move, Rect::new(307.0, 300.0, 367.0, 340.0), "selected art must not snap its7pt move to its own old stroke edges305/365");
}

#[test]
fn stroke_edge_opt_in_keeps_grid_and_pixel_moving_priority() {
    let mut f = fixture(Some(true));
    let grid = &f.session.doc().unwrap().doc.grid;
    assert_eq!((grid.spacing, grid.subdivisions), (72.0, 8));
    // Existing document defaults are9pt minor gridlines; pixel options independently select Moving.
    let mut prefs: Prefs = serde_json::from_value(serde_json::to_value(&f.session.prefs).unwrap()).unwrap();
    prefs.pixel_snap.drawing = false;
    prefs.pixel_snap.moving = true;
    prefs.pixel_snap.scaling = false;
    f.session.apply_prefs(prefs);
    f.view.snap_to_pixel = true;
    let press = Point::new(323.0, 317.0);
    let raw_end = Point::new(207.4 + 23.0, 141.2 + 17.0);
    for (grid_on, expected) in [
        // Grid wins over pixels when both views are on: left207 is a gridline; bottom180 is nearer
        // than top144. This moves BOTH raw axes and must not acquire the nearby stroke edge205.
        (true, Rect::new(207.0, 140.0, 267.0, 180.0)),
        // Without the grid, While Moving rounds the geometric top-left, not the held pointer.
        (false, Rect::new(207.0, 141.0, 267.0, 181.0)),
    ] {
        f.view.snap_to_grid = grid_on;
        let actual = move_from_and_undo(&mut f, press, raw_end, Mods::default());
        for (a, e) in [(actual.x0, expected.x0), (actual.y0, expected.y0), (actual.x1, expected.x1), (actual.y1, expected.y1)] {
            assert!(
                (a - e).abs() < 1e-9,
                "grid_on={grid_on}: actual {actual:?}, expected {expected:?}; opt-in stroke205 must yield to the existing priority"
            );
        }
    }
}
