//! Transform & utility tools driven through `Session::pointer`, and their commands.

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_geom::{Point, Rect};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

fn gesture(s: &mut Session, tool: &str, pts: &[(f64, f64)], mods: Mods) -> Vec<UiRequest> {
    let v = ViewInfo::default();
    s.select_tool(tool, v).unwrap();
    let mut out = vec![];
    for (i, (x, y)) in pts.iter().enumerate() {
        let kind = if i == 0 {
            PointerKind::Down
        } else if i == pts.len() - 1 {
            PointerKind::Up
        } else {
            PointerKind::Drag
        };
        out.extend(s.pointer(&PointerEvent::new(kind, *x, *y).with_mods(mods), v).unwrap());
    }
    out
}

fn close(a: Rect, b: Rect) -> bool {
    (a.x0 - b.x0).abs() < 1e-6 && (a.y0 - b.y0).abs() < 1e-6 && (a.x1 - b.x1).abs() < 1e-6 && (a.y1 - b.y1).abs() < 1e-6
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn rotate_tool_drag_rotates_about_center_one_undo() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let n = undo_len(&s);
    gesture(&mut s, "rotate", &[(250.0, 125.0), (200.0, 175.0), (150.0, 225.0), (150.0, 225.0)], Mods::default());
    // 90° about (150,125): 100×50 becomes 50×100.
    assert!(close(bounds(&s, a), Rect::new(125.0, 75.0, 175.0, 175.0)), "{:?}", bounds(&s, a));
    assert_eq!(undo_len(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(close(bounds(&s, a), Rect::new(100.0, 100.0, 200.0, 150.0)));
}

#[test]
fn rotate_tool_alt_release_copies() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let alt = Mods { alt: true, ..Default::default() };
    let v = ViewInfo::default();
    s.select_tool("rotate", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 250.0, 125.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 150.0, 225.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 150.0, 225.0).with_mods(alt), v).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 2);
}

#[test]
fn rotate_tool_alt_click_requests_dialog() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    let alt = Mods { alt: true, ..Default::default() };
    let ui = gesture(&mut s, "rotate", &[(10.0, 20.0), (10.0, 20.0)], alt);
    assert_eq!(ui, vec![UiRequest::Dialog("rotate".into(), json!({"angle": 0, "origin": [10.0, 20.0]}))]);
}

#[test]
fn scale_tool_click_origin_then_drag() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    gesture(&mut s, "scale", &[(100.0, 100.0), (100.0, 100.0)], Mods::default());
    gesture(&mut s, "scale", &[(200.0, 200.0), (250.0, 250.0), (300.0, 300.0), (300.0, 300.0)], Mods::default());
    assert!(close(bounds(&s, a), Rect::new(100.0, 100.0, 300.0, 300.0)), "{:?}", bounds(&s, a));
}

#[test]
fn reflect_and_shear_tools() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 50.0);
    // Reflect across a vertical axis through the centre (150,125): bounds unchanged, geometry mirrored.
    gesture(&mut s, "reflect", &[(150.0, 50.0), (150.0, 40.0), (150.0, 20.0), (150.0, 20.0)], Mods::default());
    assert!(close(bounds(&s, a), Rect::new(100.0, 100.0, 200.0, 150.0)));
    // Shear horizontally: top edge shifts right by 25 relative to the centre row.
    gesture(&mut s, "shear", &[(150.0, 100.0), (160.0, 100.0), (175.0, 100.0), (175.0, 100.0)], Mods::default());
    let b = bounds(&s, a);
    assert!((b.width() - 150.0).abs() < 1e-6, "{b:?}");
}

/// Reflect and Shear return an error for an `axis` they don't take (Reflect's names it and also
/// takes an angle): nothing moves and no undo step is recorded. A null `axis` counts as not given: Reflect
/// then reflects across the vertical axis and Shear shears along the horizontal one.
#[test]
fn reflect_and_shear_reject_unknown_axes_without_changing_state() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 30.0, 10.0);
    let n = undo_len(&s);
    let reflect = "axis must be \"vertical\", \"horizontal\" or an angle in degrees";
    let shear = "`axis` must be one of horizontal, vertical";
    for (cmd, p, msg) in [
        ("object.reflect", json!({"axis": "diagonal", "origin": [0, 0]}), format!("{reflect}, not `diagonal`")),
        ("object.reflect", json!({"axis": "45", "origin": [0, 0]}), format!("{reflect}, not `45`")),
        ("object.reflect", json!({"axis": true, "origin": [0, 0]}), format!("{reflect}, not `true`")),
        ("object.shear", json!({"angle": 30, "axis": "diagonal"}), shear.to_string()),
        ("object.shear", json!({"angle": 30, "axis": 90}), shear.to_string()),
    ] {
        let e = s.execute(cmd, &p).expect_err(&format!("{cmd} {p}"));
        assert_eq!(e.to_string(), format!("invalid parameters for `{cmd}`: {msg}"));
    }
    assert!(close(bounds(&s, a), Rect::new(10.0, 20.0, 40.0, 30.0)), "nothing moved: {:?}", bounds(&s, a));
    assert_eq!(undo_len(&s), n, "no undo step");
    s.execute("object.reflect", &json!({"axis": null, "origin": [0, 0]})).unwrap();
    assert!(close(bounds(&s, a), Rect::new(-40.0, 20.0, -10.0, 30.0)), "across the vertical axis: {:?}", bounds(&s, a));
    s.execute("object.reflect", &json!({"axis": 0, "origin": [0, 0]})).unwrap();
    assert!(close(bounds(&s, a), Rect::new(-40.0, -30.0, -10.0, -20.0)), "an angle still reflects: {:?}", bounds(&s, a));
    s.execute("object.shear", &json!({"angle": 45, "axis": null, "origin": [0, 0]})).unwrap();
    let b = bounds(&s, a);
    assert!((b.y0 + 30.0).abs() < 1e-6 && (b.y1 + 20.0).abs() < 1e-6 && b.width() > 30.0, "along the horizontal axis: {b:?}");
}

#[test]
fn distort_command_maps_corners() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("object.distort", &json!({"corners": [[10, 0], [90, 0], [100, 100], [0, 100]]})).unwrap();
    let d = &s.doc().unwrap().doc;
    let pts: Vec<Point> = d.node(a).unwrap().path_data().unwrap().anchors().map(|(_, _, an)| an.p).collect();
    for want in [Point::new(10.0, 0.0), Point::new(90.0, 0.0), Point::new(100.0, 100.0), Point::new(0.0, 100.0)] {
        assert!(pts.iter().any(|p| p.distance(want) < 1e-6), "{pts:?} missing {want:?}");
    }
    assert!(s.execute("object.distort", &json!({"corners": [[0, 0]]})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(close(bounds(&s, a), Rect::new(0.0, 0.0, 100.0, 100.0)));
}

#[test]
fn distort_perspective_midpoint_is_projective() {
    // Distort a tiny probe at the centre of a 100×100 source box into a trapezoid: with a real
    // perspective warp the centre lands where the quad's diagonals cross (y = 100/6), not at y = 50.
    let mut s = session();
    let probe = rect(&mut s, 49.999, 49.999, 0.002, 0.002);
    s.execute("object.distort", &json!({"corners": [[40, 0], [60, 0], [100, 100], [0, 100]], "from": [0, 0, 100, 100], "ids": [probe.0]})).unwrap();
    let c = bounds(&s, probe).center();
    assert!((c.x - 50.0).abs() < 1e-6);
    assert!((c.y - 100.0 / 6.0).abs() < 1e-3, "{c:?}");
}

#[test]
fn free_transform_distort_mode_via_pointer() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let v = ViewInfo::default();
    s.select_tool("freeTransform", v).unwrap();
    s.set_tool_option("mode", &json!("distort"));
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 200.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 250.0, 260.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 250.0, 260.0), v).unwrap();
    let d = &s.doc().unwrap().doc;
    let pts: Vec<Point> = d.node(a).unwrap().path_data().unwrap().anchors().map(|(_, _, an)| an.p).collect();
    assert!(pts.iter().any(|p| p.distance(Point::new(250.0, 260.0)) < 1e-6), "{pts:?}");
    assert!(pts.iter().any(|p| p.distance(Point::new(100.0, 100.0)) < 1e-6));
    assert_eq!(s.journal.last().map(|j| j.0.as_str()), Some("object.distort"));
}

#[test]
fn free_transform_cmd_held_once_a_corner_drag_started_distorts_in_one_step() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let v = ViewInfo::default();
    s.select_tool("freeTransform", v).unwrap();
    let before = undo_len(&s);
    let cmd = Mods { cmd: true, ..Default::default() };
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 200.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 210.0, 210.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 250.0, 260.0).with_mods(cmd), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 250.0, 260.0).with_mods(cmd), v).unwrap();
    let d = &s.doc().unwrap().doc;
    let pts: Vec<Point> = d.node(a).unwrap().path_data().unwrap().anchors().map(|(_, _, an)| an.p).collect();
    for p in [Point::new(250.0, 260.0), Point::new(100.0, 100.0), Point::new(200.0, 100.0), Point::new(100.0, 200.0)] {
        assert!(pts.iter().any(|q| q.distance(p) < 1e-6), "{p:?} not in {pts:?}");
    }
    assert_eq!(undo_len(&s), before + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Distort"));
}

#[test]
fn free_transform_side_handle_scales() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    gesture(&mut s, "freeTransform", &[(200.0, 150.0), (250.0, 150.0), (300.0, 150.0), (300.0, 150.0)], Mods::default());
    assert!(close(bounds(&s, a), Rect::new(100.0, 100.0, 300.0, 200.0)));
}

#[test]
fn eyedropper_copies_appearance_to_selection_and_defaults() {
    let mut s = session();
    let src = rect(&mut s, 300.0, 300.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 4})).unwrap();
    s.execute("object.setProps", &json!({"opacity": 50})).unwrap();
    let dst = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.default", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [dst.0]})).unwrap();
    gesture(&mut s, "eyedropper", &[(325.0, 325.0), (325.0, 325.0)], Mods::default());
    let d = &s.doc().unwrap().doc;
    let n = d.node(dst).unwrap();
    assert_eq!(n.appearance.fill_paint(), Paint::solid(Color::from_hex("#ff0000").unwrap()));
    assert_eq!(n.appearance.stroke_width(), 4.0);
    assert!((n.opacity - 0.5).abs() < 1e-6);
    assert_eq!(s.paint.fill, Paint::solid(Color::from_hex("#ff0000").unwrap()));
    assert_eq!(s.paint.stroke_width, 4.0);
    let _ = src;
}

#[test]
fn eyedropper_shift_samples_color_into_active_proxy() {
    let mut s = session();
    rect(&mut s, 300.0, 300.0, 50.0, 50.0);
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    let dst = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("paint.default", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [dst.0]})).unwrap();
    s.execute("paint.toggleActive", &json!({})).unwrap(); // stroke active
    gesture(&mut s, "eyedropper", &[(325.0, 325.0), (325.0, 325.0)], Mods { shift: true, ..Default::default() });
    let n = s.doc().unwrap().doc.node(dst).unwrap().clone();
    assert_eq!(n.appearance.stroke_paint(), Paint::solid(Color::from_hex("#00ff00").unwrap()));
    assert_eq!(n.appearance.fill_paint(), Paint::solid(Color::WHITE));
}

#[test]
fn gradient_tool_sets_vector_on_solid_fill() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    gesture(&mut s, "gradient", &[(100.0, 150.0), (150.0, 150.0), (200.0, 100.0), (200.0, 100.0)], Mods::default());
    let n = s.doc().unwrap().doc.node(a).unwrap().clone();
    let Paint::Gradient(g) = n.appearance.fill_paint() else { panic!("expected gradient") };
    let geom = g.geom.unwrap();
    assert_eq!(geom.start, Point::new(100.0, 150.0));
    assert_eq!(geom.end, Point::new(200.0, 100.0));
    assert_eq!(undo_len(&s), 2);
    // Changing an existing gradient keeps its stops.
    s.execute("paint.setGradientGeom", &json!({"start": [0, 0], "end": [10, 0]})).unwrap();
    let Paint::Gradient(g2) = s.doc().unwrap().doc.node(a).unwrap().appearance.fill_paint() else { panic!() };
    assert_eq!(g2.gradient, g.gradient);
    assert!(s.execute("paint.setGradientGeom", &json!({"start": [0, 0]})).is_err());
}

#[test]
fn artboard_tool_moves_with_art_creates_and_deletes() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    let outside = rect(&mut s, 900.0, 100.0, 50.0, 50.0);
    s.execute("select.none", &json!({})).unwrap();
    gesture(&mut s, "artboard", &[(400.0, 300.0), (420.0, 310.0), (450.0, 330.0), (450.0, 330.0)], Mods::default());
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.artboards[0].rect, Rect::new(50.0, 30.0, 850.0, 630.0));
    assert!(close(bounds(&s, a), Rect::new(150.0, 130.0, 200.0, 180.0)));
    assert!(close(bounds(&s, outside), Rect::new(900.0, 100.0, 950.0, 150.0)));
    // Draw a new artboard on the pasteboard.
    gesture(&mut s, "artboard", &[(1000.0, 0.0), (1100.0, 50.0), (1200.0, 100.0), (1200.0, 100.0)], Mods::default());
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 2);
    assert_eq!(s.tool_options()["active"], 1);
    assert_eq!(s.doc().unwrap().doc.artboards[1].rect, Rect::new(1000.0, 0.0, 1200.0, 100.0));
    // Delete it: the tool takes the key ahead of the Clear shortcut (the UI only hands Delete to a
    // tool that claims it).
    assert!(s.tool_claims_key(ToolKey::Delete, ViewInfo::default()));
    s.tool_key(ToolKey::Delete, Mods::default(), ViewInfo::default()).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 1);
    // The last artboard can't be deleted, so Delete stays the shortcut's.
    assert!(!s.tool_claims_key(ToolKey::Delete, ViewInfo::default()));
    s.tool_key(ToolKey::Delete, Mods::default(), ViewInfo::default()).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 1);
}

#[test]
fn artboard_resize_via_handle_and_move_command() {
    let mut s = session();
    gesture(&mut s, "artboard", &[(800.0, 300.0), (850.0, 300.0), (900.0, 300.0), (900.0, 300.0)], Mods::default());
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, Rect::new(0.0, 0.0, 900.0, 600.0));
    s.execute("artboard.move", &json!({"index": 0, "dx": -10, "dy": 5})).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, Rect::new(-10.0, 5.0, 890.0, 605.0));
    assert!(s.execute("artboard.move", &json!({"index": 7, "dx": 1})).is_err());
}

/// Alt-dragging an artboard with the Artboard tool leaves it and its art in place and moves
/// copies of both.
#[test]
fn artboard_alt_drag_duplicates_it_with_its_art() {
    let mut s = session();
    let r = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    s.execute("select.none", &json!({})).unwrap();
    let alt = Mods { alt: true, ..Mods::default() };
    gesture(&mut s, "artboard", &[(400.0, 300.0), (600.0, 300.0), (1300.0, 300.0), (1300.0, 300.0)], alt);
    let st = s.doc().unwrap();
    let d = &st.doc;
    assert_eq!(d.artboards.len(), 2);
    assert_eq!(d.artboards[0].rect, Rect::new(0.0, 0.0, 800.0, 600.0), "the original stays");
    assert_eq!((d.artboards[1].rect, d.artboards[1].name.as_str()), (Rect::new(900.0, 0.0, 1700.0, 600.0), "Artboard 1 copy"));
    assert_ne!(d.artboards[1].id, d.artboards[0].id);
    let kids = d.layers[0].children().unwrap();
    assert_eq!(kids.len(), 2, "the rectangle and its copy");
    assert_eq!(d.node(r).unwrap().geometric_bounds().unwrap().x0, 100.0);
    let copy = kids.iter().find(|n| n.id != r).unwrap();
    assert_eq!(copy.geometric_bounds().unwrap().x0, 1000.0);
    assert_eq!(st.history.undo.last().unwrap().label, "Duplicate Artboard");
    // One undo takes the copies away.
    s.execute("edit.undo", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (1, 1));
}

/// `artboard.move` with `copy` reports the copies it moved, and copies only the artboard when Move/Copy
/// Artwork with Artboard is off.
#[test]
fn artboard_move_copy_reports_the_copies_and_follows_move_art() {
    let mut s = session();
    let r = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    let out = s.execute("artboard.move", &json!({"index": 0, "dx": 900, "dy": 0, "copy": true, "moveArt": true})).unwrap();
    assert_eq!(out["index"], 1);
    let copies: Vec<NodeId> = out["moved"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    assert_eq!(copies.len(), 1);
    assert_ne!(copies[0], r);
    assert_eq!(s.doc().unwrap().doc.node(copies[0]).unwrap().geometric_bounds().unwrap().x0, 1000.0);
    let out = s.execute("artboard.move", &json!({"index": 0, "dx": 0, "dy": 900, "copy": true})).unwrap();
    assert_eq!(out["moved"], json!([]));
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (3, 2), "an artboard alone");
    assert_eq!(d.artboards[2].name, "Artboard 1 copy 2", "a name of its own");
    let ids: std::collections::HashSet<u32> = d.artboards.iter().map(|a| a.id).collect();
    assert_eq!(ids.len(), 3, "every artboard has an id of its own");
}

fn stroke_width(s: &Session, id: NodeId) -> f64 {
    s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().width
}

fn guide_positions(s: &Session) -> Vec<f64> {
    s.doc().unwrap().doc.guides.iter().map(|g| g.pos).collect()
}

/// Scale Artwork with Artboard (#602): `artboard.setProps {scaleArt}` takes the art fully inside the
/// artboard and its guides from the old rectangle onto the new one, each side by its own ratio, in
/// one undo step; art partly outside and canvas guides stay.
#[test]
fn artboard_set_props_scales_the_art_and_guides_with_the_artboard() {
    let mut s = session();
    let inside = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("stroke.set", &json!({"weight": 4})).unwrap();
    let partly = rect(&mut s, 700.0, 500.0, 200.0, 200.0);
    for (vertical, pos, artboard) in [(true, 400.0, json!(0)), (false, 300.0, json!(0)), (true, 400.0, json!(null))] {
        s.execute("guide.add", &json!({"vertical": vertical, "pos": pos, "artboard": artboard})).unwrap();
    }
    let before = s.doc().unwrap().doc.clone();
    // Half the size: half the art, half the stroke (Scale Strokes & Effects).
    let out = s.execute("artboard.setProps", &json!({"index": 0, "width": 400, "height": 300, "scaleArt": true, "strokes": true})).unwrap();
    assert_eq!(out["scaled"], json!([inside.0]));
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, Rect::new(0.0, 0.0, 400.0, 300.0));
    assert_eq!(bounds(&s, inside), Rect::new(50.0, 50.0, 150.0, 100.0));
    assert_eq!(stroke_width(&s, inside), 2.0);
    assert_eq!(bounds(&s, partly), Rect::new(700.0, 500.0, 900.0, 700.0), "art partly outside stays");
    assert_eq!(guide_positions(&s), vec![200.0, 150.0, 400.0], "the artboard's guides scale, the canvas guide stays");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before, "one undo step");
    // Moved and stretched (left edge to -100, twice as wide), strokes kept: the art follows.
    s.execute("artboard.setProps", &json!({"index": 0, "x": -100, "width": 1600, "scaleArt": true, "strokes": false})).unwrap();
    assert_eq!(bounds(&s, inside), Rect::new(100.0, 100.0, 500.0, 200.0));
    assert_eq!(stroke_width(&s, inside), 4.0);
    assert_eq!(guide_positions(&s), vec![700.0, 300.0, 400.0]);
}

/// Without `scaleArt` a resize leaves the art and guides alone (and returns nothing, as before);
/// with it, a pure move scales nothing (guides move along, as they always have).
#[test]
fn artboard_set_props_scales_art_only_when_asked_and_resized() {
    let mut s = session();
    let r = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 400, "artboard": 0})).unwrap();
    assert_eq!(s.execute("artboard.setProps", &json!({"index": 0, "width": 400})).unwrap(), Value::Null);
    assert_eq!(bounds(&s, r), Rect::new(100.0, 100.0, 300.0, 200.0));
    assert_eq!(guide_positions(&s), vec![400.0]);
    let out = s.execute("artboard.setProps", &json!({"index": 0, "x": 50, "scaleArt": true})).unwrap();
    assert_eq!(out["scaled"], json!([]));
    assert_eq!(bounds(&s, r), Rect::new(100.0, 100.0, 300.0, 200.0));
    assert_eq!(guide_positions(&s), vec![450.0]);
    assert!(s.execute("artboard.setProps", &json!({"index": 9, "width": 10, "scaleArt": true})).is_err());
}

/// Move Artwork is independent of scaling and takes only fully contained art and board guides.
#[test]
fn coordinate_moves_honor_move_art_and_keep_resize_behavior() {
    let mut s = session();
    let inside = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("stroke.set", &json!({"weight": 4})).unwrap();
    let partly = rect(&mut s, 700.0, 500.0, 200.0, 200.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 400, "artboard": 0})).unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 400})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    for scale in [false, true] {
        let out = s.execute("artboard.setProps", &json!({"index": 0, "x": 30, "y": -10, "moveArt": true, "scaleArt": scale})).unwrap();
        assert_eq!(bounds(&s, inside), Rect::new(130.0, 90.0, 330.0, 190.0));
        assert_eq!(bounds(&s, partly), Rect::new(700.0, 500.0, 900.0, 700.0));
        assert_eq!(stroke_width(&s, inside), 4.0);
        assert_eq!(guide_positions(&s), vec![430.0, 400.0]);
        assert_eq!(out, if scale { json!({"scaled": []}) } else { Value::Null });
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(*s.doc().unwrap().doc, *before);
    }
    for p in [json!({"index": 0, "x": 30, "moveArt": false}), json!({"index": 0, "x": 30, "width": 400, "moveArt": true})] {
        s.execute("artboard.setProps", &p).unwrap();
        assert_eq!(bounds(&s, inside), Rect::new(100.0, 100.0, 300.0, 200.0));
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

#[test]
fn coordinate_moves_journal_the_locked_art_preference() {
    let mut s = session();
    let locked = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("object.lock", &json!({})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 0, "x": 30, "moveArt": true})).unwrap();
    assert_eq!(bounds(&s, locked), Rect::new(100.0, 100.0, 300.0, 200.0));
    s.execute("edit.undo", &json!({})).unwrap();
    s.prefs.move_locked_with_artboard = true;
    s.execute("artboard.setProps", &json!({"index": 0, "x": 30, "moveArt": true})).unwrap();
    assert_eq!(bounds(&s, locked), Rect::new(130.0, 100.0, 330.0, 200.0));
    let (id, p) = s.journal.last().unwrap().clone();
    assert_eq!(p["lockedAndHidden"], true);
    s.execute("edit.undo", &json!({})).unwrap();
    s.prefs.move_locked_with_artboard = false;
    s.execute(&id, &p).unwrap();
    assert_eq!(bounds(&s, locked), Rect::new(130.0, 100.0, 330.0, 200.0));
}

/// Locked and hidden art scales only with Move Locked and Hidden Artwork with Artboard, as it
/// moves only with it.
#[test]
fn scale_artwork_with_artboard_leaves_locked_art_unless_the_preference_says() {
    let mut s = session();
    let locked = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("object.lock", &json!({})).unwrap();
    let scaled = |s: &mut Session| {
        let out = s.execute("artboard.setProps", &json!({"index": 0, "width": 400, "height": 300, "scaleArt": true})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        out["scaled"].clone()
    };
    assert_eq!(scaled(&mut s), json!([]));
    s.execute("prefs.set", &json!({"key": "moveLockedWithArtboard", "value": true})).unwrap();
    assert_eq!(scaled(&mut s), json!([locked.0]));
    // The choice is journaled: replayed with the preference off, the same art scales.
    s.execute("artboard.setProps", &json!({"index": 0, "width": 400, "height": 300, "scaleArt": true})).unwrap();
    assert_eq!(s.journal.last().unwrap().1["lockedAndHidden"], true);
    let (id, p) = s.journal.last().unwrap().clone();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("prefs.set", &json!({"key": "moveLockedWithArtboard", "value": false})).unwrap();
    assert_eq!(s.execute(&id, &p).unwrap()["scaled"], json!([locked.0]));
}

/// The Artboard tool with its `scaleArt` option on resizes proportionally and the art scales with
/// the artboard, in one undo step.
#[test]
fn artboard_tool_scales_art_with_the_artboard() {
    let mut s = session();
    let r = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    let before = s.doc().unwrap().doc.clone();
    s.set_tool_options(Some("artboard"), json!({"scaleArt": true}).as_object().unwrap());
    // The right handle 100 pt out: 9/8 as wide, and as tall about the middle of the left side.
    gesture(&mut s, "artboard", &[(800.0, 300.0), (850.0, 300.0), (900.0, 300.0), (900.0, 300.0)], Mods::default());
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, Rect::new(0.0, -37.5, 900.0, 637.5));
    assert_eq!(bounds(&s, r), Rect::new(112.5, 75.0, 337.5, 187.5));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *before, "one undo step");
    // Off again (the option is remembered like Move Artwork with Artboard): the art stays.
    s.set_tool_options(Some("artboard"), json!({"scaleArt": false}).as_object().unwrap());
    gesture(&mut s, "artboard", &[(800.0, 300.0), (850.0, 300.0), (900.0, 300.0), (900.0, 300.0)], Mods::default());
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, Rect::new(0.0, 0.0, 900.0, 600.0));
    assert_eq!(bounds(&s, r), Rect::new(100.0, 100.0, 300.0, 200.0));
}

#[test]
fn magic_wand_selects_same_fill() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    let b = rect(&mut s, 50.0, 10.0, 20.0, 20.0);
    let c = rect(&mut s, 90.0, 10.0, 20.0, 20.0);
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": [c.0]})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    gesture(&mut s, "magicWand", &[(20.0, 20.0), (20.0, 20.0)], Mods::default());
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, b]);
    gesture(&mut s, "magicWand", &[(100.0, 20.0), (100.0, 20.0)], Mods { shift: true, ..Default::default() });
    assert_eq!(s.doc().unwrap().selection.len(), 3);
}

#[test]
fn lasso_selects_anchor_subset() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("select.none", &json!({})).unwrap();
    gesture(&mut s, "lasso", &[(90.0, 90.0), (210.0, 90.0), (210.0, 110.0), (90.0, 110.0), (90.0, 110.0)], Mods::default());
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, vec![a]);
    assert_eq!(st.selection.partial(a).map(|p| p.len()), Some(2));
}

/// Shift-drag a marquee with the Selection tool (#483): the selected objects it reaches are
/// deselected and the others selected, the rest of the selection staying as it was.
#[test]
fn selection_shift_marquee_toggles_objects() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    let b = rect(&mut s, 200.0, 100.0, 50.0, 50.0);
    let c = rect(&mut s, 300.0, 100.0, 50.0, 50.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let shift = Mods { shift: true, ..Default::default() };
    gesture(&mut s, "selection", &[(180.0, 80.0), (300.0, 200.0), (380.0, 200.0)], shift);
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, c]);
    // Again over all three: a leaves, b joins, c leaves.
    gesture(&mut s, "selection", &[(80.0, 80.0), (300.0, 200.0), (380.0, 200.0)], shift);
    assert_eq!(s.doc().unwrap().selection.objects, vec![b]);
    // Without Shift the marquee replaces the selection.
    gesture(&mut s, "selection", &[(80.0, 80.0), (300.0, 200.0), (260.0, 200.0)], Mods::default());
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, b]);
}

/// Shift-drag a marquee with Direct Selection (#483): the anchors inside toggle; a path left with
/// none of them leaves the selection, one with all of them is selected whole again.
#[test]
fn direct_selection_shift_marquee_toggles_anchors() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let b = rect(&mut s, 300.0, 100.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let shift = Mods { shift: true, ..Default::default() };
    // Around a's top edge (two anchors, selected) and b's top-left anchor (not selected).
    let top = [(90.0, 90.0), (310.0, 110.0), (310.0, 110.0)];
    gesture(&mut s, "directSelection", &top, shift);
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, vec![a, b]);
    assert_eq!(st.selection.partial(a).map(|p| p.len()), Some(2), "a's bottom anchors stay");
    assert_eq!(st.selection.partial(b).map(|p| p.len()), Some(1));
    // The same again: a whole once more, b out.
    gesture(&mut s, "directSelection", &top, shift);
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, vec![a]);
    assert_eq!(st.selection.partial(a), None);
    // Group Selection's marquee toggles the same way.
    gesture(&mut s, "groupSelection", &top, shift);
    assert_eq!(s.doc().unwrap().selection.partial(a).map(|p| p.len()), Some(2));
}

/// The Lasso: Shift adds anchors, Alt takes them away, and neither drops the rest of the
/// selection (a path selected whole stays whole when its anchors are added again).
#[test]
fn lasso_shift_adds_and_alt_subtracts() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let t = s.execute("text.create", &json!({"x": 300, "y": 300, "text": "Hi"})).unwrap();
    let t = NodeId(t["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [a.0, t.0]})).unwrap();
    // A loop round a's top-left anchor.
    let corner = [(90.0, 90.0), (110.0, 90.0), (110.0, 110.0), (90.0, 110.0), (90.0, 110.0)];
    gesture(&mut s, "lasso", &corner, Mods { shift: true, ..Default::default() });
    let st = s.doc().unwrap();
    assert_eq!((st.selection.objects.clone(), st.selection.partial(a)), (vec![a, t], None));
    gesture(&mut s, "lasso", &corner, Mods { alt: true, ..Default::default() });
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, vec![a, t], "the type stays selected");
    assert_eq!(st.selection.partial(a).map(|p| p.len()), Some(3));
}

#[test]
fn measure_tool_leaves_document_untouched() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let rev = s.doc().unwrap().revision;
    let n = undo_len(&s);
    gesture(&mut s, "measure", &[(0.0, 0.0), (30.0, 40.0), (30.0, 40.0)], Mods::default());
    assert_eq!(s.doc().unwrap().revision, rev);
    assert_eq!(undo_len(&s), n);
    assert!((s.tool_options()["distance"].as_f64().unwrap() - 50.0).abs() < 1e-9);
    assert!(!s.overlays(ViewInfo::default()).is_empty());
}

#[test]
fn copy_from_requires_source() {
    let mut s = session();
    assert!(s.execute("appearance.copyFrom", &json!({})).is_err());
    assert!(s.execute("appearance.copyFrom", &json!({"source": 9999})).is_err());
}

#[test]
fn distort_rejects_malformed_coordinates_and_ids_without_editing_art() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let original = bounds(&s, a);
    let history = undo_len(&s);
    let corners = json!([[10, 0], [90, 0], [100, 100], [0, 100]]);
    for p in [
        json!({"corners": [[10, 0, 50], [90, 0], [100, 100], [0, 100]]}),
        json!({"corners": [[10, 0], [90, 0], ["bad", 100], [0, 100]]}),
        json!({"corners": [[10, 0], [90, 0], [100, 100], [0, 100], ["bad"]]}),
        json!({"corners": corners, "from": [0, 0, 100]}),
        json!({"corners": corners, "from": "not bounds"}),
        json!({"corners": corners, "ids": [a.0, "bad"]}),
        json!({"corners": corners, "ids": [a.0, u64::MAX]}),
    ] {
        assert!(s.execute("object.distort", &p).is_err(), "invalid input: {p}");
        assert!(close(bounds(&s, a), original));
        assert_eq!(undo_len(&s), history);
    }
    s.execute("object.distort", &json!({"corners": corners, "from": [0, 0, 100, 100], "ids": [a.0]})).unwrap();
    assert_eq!(undo_len(&s), history + 1);
}
