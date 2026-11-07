//! Width points, Liquify, Puppet Warp and Perspective Grid commands and tools.

use serde_json::json;
use vectorcraft_geom::{PathData, Point, Rect, Shape};
use vectorcraft_tools::distort::perspective::{PerspectiveGrid, Plane};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn line(s: &mut Session) -> NodeId {
    let r = s.execute("shape.line", &json!({"x1": 100, "y1": 300, "x2": 500, "y2": 300})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("stroke.set", &json!({"ids": [id.0], "weight": 10})).unwrap();
    id
}

fn path(s: &Session, id: NodeId) -> PathData {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
}

fn profile(s: &Session, id: NodeId) -> Option<Vec<(f64, f64, f64)>> {
    s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().profile.as_ref().map(|p| p.points.clone())
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// How many steps there are to undo and to redo, and whether the document has unsaved changes.
fn history(s: &Session) -> (usize, usize, bool) {
    let st = s.doc().unwrap();
    (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
}

/// Leave a step to redo (a rectangle drawn and undone, which keeps the selection) and mark the
/// document saved.
fn with_a_step_to_redo(s: &mut Session) {
    rect(s, 700.0, 500.0, 10.0, 10.0);
    s.execute("edit.undo", &json!({})).unwrap();
    s.doc_mut().unwrap().mark_saved();
}

fn gesture(s: &mut Session, tool: &str, pts: &[(f64, f64)], mods: Mods) {
    let v = ViewInfo::default();
    s.select_tool(tool, v).unwrap();
    for (i, (x, y)) in pts.iter().enumerate() {
        let kind = if i == 0 {
            PointerKind::Down
        } else if i == pts.len() - 1 {
            PointerKind::Up
        } else {
            PointerKind::Drag
        };
        s.pointer(&PointerEvent::new(kind, *x, *y).with_mods(mods), v).unwrap();
    }
}

fn close(a: Point, b: Point, tol: f64) -> bool {
    a.distance(b) <= tol
}

/// `cmd` with `p` fails with the error for `key`, which `p` gives as something other than true or false.
fn refuses_non_boolean(s: &mut Session, cmd: &str, p: &serde_json::Value, key: &str) {
    let r = s.execute(cmd, p);
    let want = format!("`{key}` must be true or false");
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == want), "{cmd} {p}: {r:?}");
}

/// Values for an on/off param that are not true, false or null.
fn not_booleans() -> [serde_json::Value; 4] {
    [json!("true"), json!(1), json!("false"), json!([true])]
}

// ---------- width ----------

#[test]
fn width_point_set_creates_profile_and_undoes() {
    let mut s = session();
    let id = line(&mut s);
    let n = undo_len(&s);
    let r = s.execute("stroke.widthPoint.set", &json!({"id": id.0, "t": 0.5, "left": 10, "right": 5})).unwrap();
    assert_eq!(r["index"], json!(1));
    assert_eq!(profile(&s, id).unwrap(), vec![(0.0, 1.0, 1.0), (0.5, 2.0, 1.0), (1.0, 1.0, 1.0)]);
    assert_eq!(undo_len(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(profile(&s, id), None);
}

#[test]
fn width_point_move_and_remove() {
    let mut s = session();
    let id = line(&mut s);
    s.execute("stroke.widthPoint.set", &json!({"id": id.0, "t": 0.5, "left": 10, "right": 10})).unwrap();
    let r = s.execute("stroke.widthPoint.set", &json!({"id": id.0, "t": 0.25, "left": 10, "right": 10, "index": 1})).unwrap();
    assert_eq!(r["index"], json!(1));
    assert_eq!(profile(&s, id).unwrap()[1], (0.25, 2.0, 2.0));
    assert_eq!(profile(&s, id).unwrap().len(), 3);
    s.execute("stroke.widthPoint.remove", &json!({"id": id.0, "index": 1})).unwrap();
    assert_eq!(profile(&s, id).unwrap().len(), 2);
    assert!(s.execute("stroke.widthPoint.remove", &json!({"id": id.0, "index": 7})).is_err());
    assert!(s.execute("stroke.widthPoint.set", &json!({"id": id.0, "t": 0.5, "left": -1, "right": 1})).is_err());
}

#[test]
fn width_profile_set_and_clear() {
    let mut s = session();
    let id = line(&mut s);
    s.execute("stroke.widthProfile.set", &json!({"ids": [id.0], "points": [[1, 0, 0], [0, 0.5, 0.5]]})).unwrap();
    assert_eq!(profile(&s, id).unwrap(), vec![(0.0, 0.5, 0.5), (1.0, 0.0, 0.0)]);
    s.execute("stroke.widthProfile.set", &json!({"ids": [id.0], "points": null})).unwrap();
    assert_eq!(profile(&s, id), None);
}

/// Width Profile refuses a point with a place or width given as text, null or another value that is
/// not a number, such as `[0.5, "1", 2, 2]`, and leaves the profile and the undo history as they were.
#[test]
fn width_profile_set_refuses_a_width_given_as_text() {
    let mut s = session();
    let id = line(&mut s);
    s.execute("stroke.widthProfile.set", &json!({"ids": [id.0], "points": [[0, 1, 1], [1, 1, 1]]})).unwrap();
    let before = (profile(&s, id), undo_len(&s));
    for points in [json!([[0.5, "1", 2, 2]]), json!([[0.5, 1, 2, "3"]]), json!([[0.5, null, 2, 2]]), json!([[true, 0.5, 2, 2]])] {
        let r = s.execute("stroke.widthProfile.set", &json!({"ids": [id.0], "points": points}));
        let want = "points must be [t, left, right] with non-negative widths";
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg == want), "{points}: {r:?}");
        assert_eq!((profile(&s, id), undo_len(&s)), before, "{points}");
    }
}

#[test]
fn width_tool_gesture_is_one_undo_step() {
    let mut s = session();
    let id = line(&mut s);
    let n = undo_len(&s);
    gesture(&mut s, "width", &[(300.0, 301.0), (300.0, 310.0), (300.0, 320.0)], Mods::default());
    let p = profile(&s, id).unwrap();
    assert_eq!(p.len(), 3);
    // The last drag sample (10 pt from the path) sets both sides: factor 10 / (10 / 2) = 2.
    assert!((p[1].0 - 0.5).abs() < 1e-3 && (p[1].1 - 2.0).abs() < 1e-6 && (p[1].2 - 2.0).abs() < 1e-6, "{p:?}");
    assert_eq!(undo_len(&s), n + 1);
    // Delete removes the selected width point.
    s.tool_key(ToolKey::Delete, Mods::default(), ViewInfo::default()).unwrap();
    assert_eq!(profile(&s, id).unwrap().len(), 2);
}

/// Width Point with the place and widths a point has already (given its index, or at its place, with
/// Adjust Adjoining Width Points, or as Width Point Edit gives back the widths it shows), Copy Width
/// Point onto the side of a discontinuous point that has its widths already, and Width Profile with
/// the profile the strokes have already (or null for a uniform stroke and for art with no stroke)
/// record no undo step: the step to redo stays and the document stays unmodified.
#[test]
fn width_commands_that_change_nothing_record_no_undo_step() {
    let mut s = session();
    let id = line(&mut s);
    let uniform = line(&mut s);
    let r = rect(&mut s, 100.0, 400.0, 20.0, 20.0);
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    // Points 1 and 2 make a discontinuous point at 0.5 whose sides are both 2 pt and 3 pt wide
    // (point 1 copied onto its own place), and point 3 is at 0.75.
    s.execute("stroke.widthPoint.set", &json!({"id": id.0, "t": 0.5, "left": 2, "right": 3})).unwrap();
    s.execute("stroke.widthPoint.copy", &json!({"id": id.0, "index": 1, "t": 0.5})).unwrap();
    s.execute("stroke.widthPoint.set", &json!({"id": id.0, "t": 0.75, "left": 4, "right": 4})).unwrap();
    let points = profile(&s, id).unwrap();
    assert_eq!(points, vec![(0.0, 1.0, 1.0), (0.5, 0.4, 0.6), (0.5, 0.4, 0.6), (0.75, 0.8, 0.8), (1.0, 1.0, 1.0)]);
    // Width Point Edit shows a width as its factor times half the weight (5 pt) and gives that back.
    // For this factor the round trip changes the last bit.
    let dragged = line(&mut s);
    let f = 1.8822996672167678;
    assert_ne!(f * 5.0 / 5.0, f);
    s.execute("stroke.widthProfile.set", &json!({"ids": [dragged.0], "points": [[0, 1, 1], [0.5, f, f], [1, 1, 1]]})).unwrap();
    with_a_step_to_redo(&mut s);
    let before = history(&s);
    assert_eq!((before.1, before.2), (1, false), "a step to redo, and nothing unsaved");
    let calls = [
        ("stroke.widthPoint.set", json!({"id": id.0, "index": 1, "t": 0.5, "left": 2, "right": 3}), json!({"index": 1})),
        ("stroke.widthPoint.set", json!({"id": id.0, "t": 0.75, "left": 4, "right": 4}), json!({"index": 3})),
        // Adjust Adjoining multiplies point 2's 0.4 by 0.8 and divides the product by 0.8, which
        // gives 0.4000000000000001.
        ("stroke.widthPoint.set", json!({"id": id.0, "index": 3, "t": 0.75, "left": 4, "right": 4, "adjustAdjoining": true}), json!({"index": 3})),
        ("stroke.widthPoint.set", json!({"id": dragged.0, "index": 1, "t": 0.5, "left": f * 5.0, "right": f * 5.0}), json!({"index": 1})),
        ("stroke.widthPoint.copy", json!({"id": id.0, "index": 2, "t": 0.5}), json!({"index": 2})),
        ("stroke.widthProfile.set", json!({"ids": [id.0], "points": points}), json!(null)),
        ("stroke.widthProfile.set", json!({"ids": [uniform.0, group.0], "points": null}), json!(null)),
    ];
    for (c, p, reply) in calls {
        assert_eq!(s.execute(c, &p).unwrap(), reply, "{c} {p}");
        assert_eq!(history(&s), before, "{c} {p}");
        assert_eq!(profile(&s, id).unwrap(), points, "{c} {p}");
        assert_eq!(s.journal.last().unwrap().0, c);
    }
    assert_eq!(profile(&s, dragged).unwrap()[1], (0.5, f, f));
    // Other widths are one undo step, and they clear Redo.
    s.execute("stroke.widthPoint.set", &json!({"id": id.0, "index": 1, "t": 0.5, "left": 3, "right": 3})).unwrap();
    assert_eq!(history(&s), (before.0 + 1, 0, true));
}

/// Width Point refuses an `adjustAdjoining` given as text or as another value that is not true or
/// false, for a new point and for a point given by its index: the profile and the undo history stay
/// as they were. Null is read as the key left out, and the points either side keep their widths.
#[test]
fn width_point_set_refuses_adjust_adjoining_given_as_text() {
    let mut s = session();
    let id = line(&mut s);
    s.execute("stroke.widthPoint.set", &json!({"id": id.0, "t": 0.5, "left": 10, "right": 10})).unwrap();
    let before = (profile(&s, id), undo_len(&s));
    for v in not_booleans() {
        let added = json!({"id": id.0, "t": 0.25, "left": 4, "right": 4, "adjustAdjoining": v});
        refuses_non_boolean(&mut s, "stroke.widthPoint.set", &added, "adjustAdjoining");
        let moved = json!({"id": id.0, "index": 1, "t": 0.5, "left": 4, "right": 4, "adjustAdjoining": v});
        refuses_non_boolean(&mut s, "stroke.widthPoint.set", &moved, "adjustAdjoining");
        assert_eq!((profile(&s, id), undo_len(&s)), before, "{v}");
    }
    s.execute("stroke.widthPoint.set", &json!({"id": id.0, "t": 0.25, "left": 4, "right": 4, "adjustAdjoining": null})).unwrap();
    assert_eq!(profile(&s, id).unwrap(), vec![(0.0, 1.0, 1.0), (0.25, 0.8, 0.8), (0.5, 2.0, 2.0), (1.0, 1.0, 1.0)]);
}

// ---------- liquify ----------

fn warp_params() -> serde_json::Value {
    json!({"tool": "warp", "points": [[300, 150], [340, 150], [380, 150]], "diameter": 100, "intensity": 100})
}

#[test]
fn liquify_is_deterministic_and_localized() {
    let run = || {
        let mut s = session();
        let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
        let b = rect(&mut s, 500.0, 400.0, 100.0, 100.0);
        s.execute("select.all", &json!({})).unwrap();
        s.execute("object.liquify", &warp_params()).unwrap();
        (path(&s, a), path(&s, b))
    };
    let (a1, b1) = run();
    let (a2, b2) = run();
    assert_eq!(a1, a2);
    assert_eq!(b1, b2);
    // The far rectangle is untouched; the right edge of the first bulged right near y = 150.
    assert_eq!(b1.bounds().unwrap(), Rect::new(500.0, 400.0, 600.0, 500.0));
    let bb = a1.bounds().unwrap();
    assert!(bb.x1 > 320.0 && bb.x0 == 100.0 && bb.y1 == 300.0, "{bb:?}");
    assert!(a1.anchor_count() > 4);
}

#[test]
fn liquify_without_selection_hits_paths_under_the_brush() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    let b = rect(&mut s, 500.0, 400.0, 100.0, 100.0);
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let r = s.execute("object.liquify", &json!({"tool": "pucker", "points": [[550, 450]], "diameter": 150, "intensity": 1})).unwrap();
    assert_eq!(r["ids"], json!([b.0]));
    assert_eq!(path(&s, a).bounds().unwrap(), Rect::new(100.0, 100.0, 300.0, 300.0));
    assert!(path(&s, b).bounds().unwrap().width() < 100.0);
}

/// With nothing selected, a stroke leaves the art on a locked artboard as it is.
#[test]
fn liquify_without_selection_leaves_the_art_on_a_locked_artboard() {
    let mut s = session();
    let b = rect(&mut s, 500.0, 400.0, 100.0, 100.0);
    s.execute("artboard.new", &json!({"x": 450, "y": 350, "width": 300, "height": 300})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let pucker = json!({"tool": "pucker", "points": [[550, 450]], "diameter": 150, "intensity": 1});
    let r = s.execute("object.liquify", &pucker);
    assert_eq!(path(&s, b).bounds().unwrap(), Rect::new(500.0, 400.0, 600.0, 500.0), "{r:?}");
    s.execute("artboard.setProps", &json!({"index": 1, "locked": false})).unwrap();
    s.execute("object.liquify", &pucker).unwrap();
    assert!(path(&s, b).bounds().unwrap().width() < 100.0, "unlocked");
}

#[test]
fn liquify_tool_replays_from_the_journal() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let n = undo_len(&s);
    s.select_tool("twirl", ViewInfo::default()).unwrap();
    s.set_tool_option("diameter", &json!(120));
    gesture(&mut s, "twirl", &[(300.0, 200.0), (302.0, 210.0), (305.0, 220.0), (305.0, 230.0)], Mods::default());
    assert_eq!(undo_len(&s), n + 1);
    let after = path(&s, a);
    assert_ne!(after, path(&session_with_rect(), a));
    let (cmd, params) = s.journal.last().unwrap().clone();
    assert_eq!(cmd, "object.liquify");
    let mut t = session_with_rect();
    t.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    t.execute(&cmd, &params).unwrap();
    assert_eq!(path(&t, a), after);
}

/// `cmd` with `p` fails with the error for `key`, which `p` gives as something other than a number.
fn refuses_non_number(s: &mut Session, cmd: &str, p: &serde_json::Value, key: &str) {
    let r = s.execute(cmd, p);
    let want = format!("`{key}` must be a number, not {}", p[key]);
    assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == want), "{cmd} {p}: {r:?}");
}

/// Liquify refuses a brush size, angle, intensity, detail, simplify, rate, complexity or wrinkle
/// amount given as text or as another value that is not a number, with the tool that uses the key and
/// with one that does not: the path and the undo history stay as they were. Null is read as the key
/// left out.
#[test]
fn liquify_refuses_a_number_given_as_text() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let before = (path(&s, a), undo_len(&s));
    let with = |tool: &str, key: &str, v: serde_json::Value| {
        let mut p = warp_params();
        p["tool"] = json!(tool);
        p[key] = v;
        (key.to_string(), p)
    };
    let calls = [
        with("warp", "diameter", json!("40")),
        with("warp", "width", json!("80")),
        with("pucker", "height", json!([80])),
        with("twirl", "angle", json!("45")),
        with("bloat", "intensity", json!("80%")),
        with("warp", "detail", json!(true)),
        with("warp", "simplify", json!("50")),
        with("twirl", "rate", json!("90")),
        with("crystallize", "complexity", json!("3")),
        with("wrinkle", "horizontal", json!("50")),
        with("wrinkle", "vertical", json!({"value": 50})),
        with("warp", "complexity", json!("3")),
    ];
    for (key, p) in calls {
        refuses_non_number(&mut s, "object.liquify", &p, &key);
        assert_eq!((path(&s, a), undo_len(&s)), before, "{p}");
    }
    let mut p = warp_params();
    p["diameter"] = json!(null);
    p["intensity"] = json!(null);
    assert_eq!(s.execute("object.liquify", &p).unwrap()["ids"], json!([a.0]));
    assert_eq!(undo_len(&s), before.1 + 1);
}

/// A Liquify stroke that reshapes no path (the brush reaches none) records no undo step, run as a
/// command or dragged with the tool, and the command replies with no ids.
#[test]
fn liquify_that_reshapes_no_path_records_no_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    with_a_step_to_redo(&mut s);
    let before = history(&s);
    let r = s.execute("object.liquify", &json!({"tool": "warp", "points": [[600, 450], [640, 450]], "diameter": 50})).unwrap();
    assert_eq!(r["ids"], json!([]));
    assert_eq!(history(&s), before, "object.liquify");
    gesture(&mut s, "warp", &[(600.0, 450.0), (620.0, 450.0), (640.0, 450.0)], Mods::default());
    assert_eq!(s.journal.last().unwrap().0, "object.liquify");
    assert_eq!(history(&s), before, "a Warp tool drag");
    assert_eq!(path(&s, a).bounds().unwrap(), Rect::new(100.0, 100.0, 300.0, 300.0));
}

/// Liquify refuses a `usePressure`, `simplifyOn`, `affectAnchors`, `affectIn` or `affectOut` given as
/// text or as another value that is not true or false, whichever tool it is given with: the path and
/// the undo history stay as they were. Null is read as the key left out.
#[test]
fn liquify_refuses_a_boolean_given_as_text() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let before = (path(&s, a), undo_len(&s));
    let crystallize = json!({"tool": "crystallize", "points": [[300, 150]], "diameter": 100, "intensity": 100});
    let keys = [("usePressure", warp_params()), ("simplifyOn", warp_params()), ("affectAnchors", crystallize.clone())];
    let keys = keys.into_iter().chain([("affectIn", crystallize.clone()), ("affectOut", crystallize), ("affectOut", warp_params())]);
    for (key, mut p) in keys {
        for v in not_booleans() {
            p[key] = v;
            refuses_non_boolean(&mut s, "object.liquify", &p, key);
        }
    }
    assert_eq!((path(&s, a), undo_len(&s)), before, "nothing changed");
    let mut p = warp_params();
    p["usePressure"] = json!(null);
    p["simplifyOn"] = json!(null);
    assert_eq!(s.execute("object.liquify", &p).unwrap()["ids"], json!([a.0]));
    assert_eq!(undo_len(&s), before.1 + 1);
}

fn session_with_rect() -> Session {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 200.0, 200.0);
    s
}

// ---------- puppet warp ----------

#[test]
fn puppet_warp_satisfies_pins_and_keeps_area() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 300.0, 100.0);
    let n = undo_len(&s);
    s.execute("object.puppetWarp", &json!({"id": a.0, "pins": [[120, 150], [250, 150], [380, 150]], "moved": [[120, 150], [250, 150], [370, 90]]}))
        .unwrap();
    let p = path(&s, a);
    let area0 = 300.0 * 100.0;
    let area1 = p.to_bezpath().area().abs();
    assert!((area1 / area0 - 1.0).abs() < 0.08, "area {area1}");
    // The pinned left end stays; the right end rose.
    let bb = p.bounds().unwrap();
    assert!((bb.x0 - 100.0).abs() < 6.0, "{bb:?}");
    assert!(bb.y0 < 80.0, "{bb:?}");
    assert_eq!(undo_len(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&s, a).bounds().unwrap(), Rect::new(100.0, 100.0, 400.0, 200.0));
    assert!(s.execute("object.puppetWarp", &json!({"id": a.0, "pins": [[1, 1]], "moved": []})).is_err());
}

/// Puppet Warp and Puppet Warp Pins refuse an `expand` given as text or as another value that is not
/// a number, and Puppet Warp refuses an angle that is not a number or null, such as "90": the art and
/// the undo history stay as they were. A null `expand` is read as the key left out.
#[test]
fn puppet_warp_refuses_a_number_given_as_text() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 300.0, 100.0);
    let before = (path(&s, a), undo_len(&s));
    let with = |key: &str, v: serde_json::Value| {
        let mut p = json!({"id": a.0, "pins": [[120, 150], [380, 150]], "moved": [[120, 150], [370, 90]]});
        p[key] = v;
        p
    };
    refuses_non_number(&mut s, "object.puppetWarp", &with("expand", json!("5")), "expand");
    refuses_non_number(&mut s, "object.puppetWarp", &with("expand", json!([5])), "expand");
    refuses_non_number(&mut s, "object.puppetWarp.pins", &json!({"id": a.0, "expand": "5"}), "expand");
    for angles in [json!(["90", null]), json!([null, true])] {
        let r = s.execute("object.puppetWarp", &with("angles", angles.clone()));
        let want = "angles must be a list as long as pins (degrees or null)";
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg == want), "{angles}: {r:?}");
    }
    assert_eq!((path(&s, a), undo_len(&s)), before);
    s.execute("object.puppetWarp", &with("expand", json!(null))).unwrap();
    assert_eq!(undo_len(&s), before.1 + 1);
}

#[test]
fn puppet_tool_drag_warps_selection() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 300.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("puppetWarp", v).unwrap();
    // The automatic pins show at once, one in the middle of each end: drag the right end's up.
    let pins = s.execute("object.puppetWarp.pins", &json!({})).unwrap()["moved"].clone();
    let right = pins
        .as_array()
        .unwrap()
        .iter()
        .map(|q| (q[0].as_f64().unwrap(), q[1].as_f64().unwrap()))
        .fold((0.0, 0.0), |a, q| if q.0 > a.0 { q } else { a });
    assert!(right.0 > 350.0, "{pins}");
    let n = undo_len(&s);
    s.pointer(&PointerEvent::new(PointerKind::Down, right.0, right.1), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, right.0, right.1 - 30.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, right.0, right.1 - 30.0), v).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(s.journal.last().unwrap().0, "object.puppetWarp");
    assert!(path(&s, a).bounds().unwrap().y0 < 95.0);
}

/// Puppet Warp with the pins the art holds already (as object.puppetWarp.pins lists them), and
/// Puppet Warp with every pin left where it is and turning the art by 0° or free, record no undo
/// step and leave the art as it is: a live shape stays live.
#[test]
fn puppet_warp_that_moves_nothing_records_no_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 300.0, 100.0);
    let b = rect(&mut s, 100.0, 300.0, 300.0, 100.0);
    // The automatic pins, the right end's moved up 30 pt, warp the art in one undo step.
    let mut p = s.execute("object.puppetWarp.pins", &json!({"id": a.0})).unwrap();
    let moved = p["moved"].as_array().unwrap();
    let right = (0..moved.len()).max_by(|&i, &j| moved[i][0].as_f64().unwrap().total_cmp(&moved[j][0].as_f64().unwrap())).unwrap();
    let y = p["moved"][right][1].as_f64().unwrap();
    p["moved"][right][1] = json!(y - 30.0);
    let undo = undo_len(&s);
    s.execute("object.puppetWarp", &p).unwrap();
    assert_eq!(undo_len(&s), undo + 1);
    with_a_step_to_redo(&mut s);
    let before = history(&s);
    let pins = json!([[120, 350], [250, 350]]);
    assert_eq!(s.execute("object.puppetWarp", &json!({"id": b.0, "pins": pins, "moved": pins})).unwrap(), json!({"ids": [b.0]}));
    let live = matches!(s.doc().unwrap().doc.node(b).unwrap().kind, NodeKind::Path { live: Some(_), .. });
    assert_eq!((history(&s), live), (before, true), "a live shape with its pins left where they are");
    let held = s.execute("object.puppetWarp.pins", &json!({"id": a.0})).unwrap();
    assert_eq!(held["auto"], json!(false));
    let warped = path(&s, a);
    let still = json!({"id": a.0, "pins": [[120, 150], [250, 150]], "moved": [[120, 150], [250, 150]], "angles": [null, 0]});
    for p in [held, still] {
        assert_eq!(s.execute("object.puppetWarp", &p).unwrap(), json!({"ids": [a.0]}), "{p}");
        assert_eq!(history(&s), before, "{p}");
        assert_eq!(path(&s, a), warped, "{p}");
    }
}

/// Puppet Warp refuses a `rest` given as text or as another value that is not true or false: the art
/// and the undo history stay as they were. Null is read as the key left out.
#[test]
fn puppet_warp_refuses_rest_given_as_text() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 300.0, 100.0);
    let before = (path(&s, a), undo_len(&s));
    let warp = |rest| json!({"id": a.0, "pins": [[120, 150], [250, 150], [380, 150]], "moved": [[120, 150], [250, 150], [370, 90]], "rest": rest});
    for v in not_booleans() {
        refuses_non_boolean(&mut s, "object.puppetWarp", &warp(v), "rest");
    }
    assert_eq!((path(&s, a), undo_len(&s)), before, "nothing changed");
    s.execute("object.puppetWarp", &warp(json!(null))).unwrap();
    assert_eq!(undo_len(&s), before.1 + 1);
    assert!(path(&s, a).bounds().unwrap().y0 < 80.0);
}

// ---------- perspective ----------

fn grid(s: &Session) -> PerspectiveGrid {
    PerspectiveGrid::from_doc(&s.doc().unwrap().doc).unwrap()
}

#[test]
fn grid_show_and_plane_are_view_state() {
    let mut s = session();
    let n = undo_len(&s);
    assert_eq!(s.execute("perspective.grid.show", &json!({})).unwrap(), json!({"visible": true}));
    s.execute("perspective.plane.set", &json!({"plane": "ground"})).unwrap();
    assert_eq!(grid(&s).plane, Plane::Ground);
    assert_eq!(undo_len(&s), n);
    assert_eq!(s.execute("perspective.grid.show", &json!({})).unwrap(), json!({"visible": false}));
    assert!(s.execute("perspective.plane.set", &json!({"plane": "up"})).is_err());
    let ov = s.overlays(ViewInfo::default());
    assert!(ov.is_empty(), "hidden grid draws nothing with the selection tool");
    s.select_tool("perspectiveGrid", ViewInfo::default()).unwrap();
    assert!(s.overlays(ViewInfo::default()).len() > 20);
}

#[test]
fn the_grid_hides_while_a_perspective_tool_is_chosen() {
    // #321: choosing the Perspective Grid tool showed the grid for good.
    let mut s = session();
    let v = ViewInfo::default();
    for tool in ["perspectiveGrid", "perspectiveSelection"] {
        s.select_tool(tool, v).unwrap();
        assert!(s.overlays(v).len() > 20, "{tool}");
        let n = undo_len(&s);
        assert_eq!(s.execute("perspective.grid.show", &json!({})).unwrap(), json!({"visible": false}));
        assert!(s.overlays(v).is_empty(), "{tool}: the hidden grid still draws");
        assert_eq!(undo_len(&s), n);
        // Choosing the tool again shows it again.
        s.select_tool(tool, v).unwrap();
        assert!(s.overlays(v).len() > 20, "{tool}");
        s.execute("perspective.grid.show", &json!({"visible": false})).unwrap();
        s.select_tool("selection", v).unwrap();
    }
}

#[test]
fn a_hidden_grid_stays_hidden_back_from_a_temporary_tool() {
    // Cmd-dragging with a perspective tool borrows the Selection tool; going back to the
    // perspective tool is not choosing it, so the grid hidden meanwhile stays hidden.
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("perspectiveGrid", v).unwrap();
    s.execute("perspective.grid.show", &json!({"visible": false})).unwrap();
    s.switch_tool("selection", v).unwrap();
    s.switch_tool("perspectiveGrid", v).unwrap();
    assert!(s.overlays(v).is_empty());
    s.select_tool("selection", v).unwrap();
    s.select_tool("perspectiveGrid", v).unwrap();
    assert!(s.overlays(v).len() > 20, "choosing it shows the grid");
}

#[test]
fn grid_preset_and_set_are_undoable() {
    let mut s = session();
    s.execute("perspective.grid.preset", &json!({"kind": 3})).unwrap();
    assert_eq!(grid(&s).kind, 3);
    s.execute("perspective.grid.set", &json!({"cell": 40, "horizon": 200})).unwrap();
    assert_eq!((grid(&s).cell, grid(&s).horizon), (40.0, 200.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(grid(&s).kind, 3);
    assert_ne!(grid(&s).cell, 40.0);
    assert!(s.execute("perspective.grid.set", &json!({"distance": 0})).is_err());
    assert!(s.execute("perspective.grid.preset", &json!({"kind": 4})).is_err());
    // The grid is saved with the document.
    let saved = serde_json::to_value(&*s.doc().unwrap().doc).unwrap();
    assert_eq!(saved["unknown"]["perspectiveGrid"]["kind"], json!(3));
}

#[test]
fn attach_projects_onto_the_plane_and_releases() {
    let mut s = session();
    s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
    let a = rect(&mut s, 450.0, 300.0, 100.0, 100.0);
    s.execute("perspective.attach", &json!({"ids": [a.0], "plane": "right"})).unwrap();
    let g = grid(&s);
    assert_eq!(g.attached_plane(a), Some(Plane::Right));
    // Every anchor lies on the plane: its plane coordinates form an axis-aligned rectangle.
    let p = path(&s, a);
    let q: Vec<Point> = p.anchors().map(|(_, _, an)| g.to_plane(Plane::Right, an.p).unwrap()).collect();
    let xs: Vec<f64> = q.iter().map(|v| v.x).collect();
    let ys: Vec<f64> = q.iter().map(|v| v.y).collect();
    for v in &q {
        assert!(xs.iter().filter(|x| (*x - v.x).abs() < 1e-6).count() == 2 && ys.iter().filter(|y| (*y - v.y).abs() < 1e-6).count() == 2, "{q:?}");
    }
    // The dragged corners are kept.
    assert!(p.anchors().any(|(_, _, an)| close(an.p, Point::new(450.0, 300.0), 1e-6)));
    assert!(p.anchors().any(|(_, _, an)| close(an.p, Point::new(550.0, 400.0), 1e-6)));
    s.execute("perspective.release", &json!({"ids": [a.0]})).unwrap();
    assert_eq!(grid(&s).attached_plane(a), None);
    assert_eq!(path(&s, a), p);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&s, a).bounds().unwrap(), Rect::new(450.0, 300.0, 550.0, 400.0));
}

#[test]
fn rectangle_tool_draws_in_perspective_on_the_active_plane() {
    let mut s = session();
    s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
    s.execute("perspective.plane.set", &json!({"plane": "left"})).unwrap();
    // Snap to Grid (on by default) would move the dragged corners onto gridlines.
    s.execute("perspective.grid.snap", &json!({"on": false})).unwrap();
    let n = undo_len(&s);
    gesture(&mut s, "rectangle", &[(300.0, 320.0), (330.0, 360.0), (350.0, 400.0)], Mods::default());
    assert_eq!(undo_len(&s), n + 1);
    let (cmd, params) = s.journal.last().unwrap().clone();
    assert_eq!(cmd, "perspective.draw");
    assert_eq!(params["command"], json!("shape.rectangle"));
    let id = *s.doc().unwrap().selection.objects.last().unwrap();
    assert_eq!(grid(&s).attached_plane(id), Some(Plane::Left));
    let p = path(&s, id);
    assert!(p.anchors().any(|(_, _, an)| close(an.p, Point::new(300.0, 320.0), 1e-6)));
    // Not a plain rectangle any more: the top edge slopes toward the left vanishing point.
    let ys: Vec<f64> = p.anchors().map(|(_, _, an)| an.p.y).collect();
    assert!(ys.iter().any(|y| (y - 320.0).abs() > 0.5 && (y - 400.0).abs() > 0.5), "{ys:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(id).is_none());
}

/// Draw in Perspective refuses a `snap` given as text or as another value that is not true or false
/// before it draws: no shape is made and the undo history stays as it was. Null is read as the key
/// left out.
#[test]
fn perspective_draw_refuses_snap_given_as_text() {
    let mut s = session();
    s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
    s.execute("perspective.plane.set", &json!({"plane": "left"})).unwrap();
    let before = (history(&s), s.doc().unwrap().selection.objects.clone());
    let draw = |snap| json!({"command": "shape.rectangle", "params": {"x": 300, "y": 320, "width": 50, "height": 80}, "snap": snap});
    for v in not_booleans() {
        refuses_non_boolean(&mut s, "perspective.draw", &draw(v), "snap");
    }
    assert_eq!((history(&s), s.doc().unwrap().selection.objects.clone()), before, "nothing drawn");
    let r = s.execute("perspective.draw", &draw(json!(null))).unwrap();
    assert_eq!((r["plane"].clone(), history(&s).0), (json!("left"), before.0.0 + 1));
}

#[test]
fn perspective_selection_moves_within_the_plane() {
    let mut s = session();
    s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
    let a = rect(&mut s, 450.0, 380.0, 60.0, 60.0);
    s.execute("perspective.attach", &json!({"ids": [a.0], "plane": "right"})).unwrap();
    // Snap to Grid (on by default) would land the moved edges on gridlines.
    s.execute("perspective.grid.snap", &json!({"on": false})).unwrap();
    let g = grid(&s);
    let before: Vec<Point> = path(&s, a).anchors().map(|(_, _, an)| g.to_plane(Plane::Right, an.p).unwrap()).collect();
    let centre = before.iter().fold(vectorcraft_geom::Vec2::ZERO, |a, p| a + p.to_vec2()) / before.len() as f64;
    let from = g.to_page(Plane::Right, centre.to_point()).unwrap();
    let to = g.to_page(Plane::Right, (centre + vectorcraft_geom::Vec2::new(40.0, 20.0)).to_point()).unwrap();
    let n = undo_len(&s);
    gesture(&mut s, "perspectiveSelection", &[(from.x, from.y), (to.x, to.y), (to.x, to.y)], Mods::default());
    assert_eq!(undo_len(&s), n + 1);
    let after: Vec<Point> = path(&s, a).anchors().map(|(_, _, an)| g.to_plane(Plane::Right, an.p).unwrap()).collect();
    for (b, c) in before.iter().zip(&after) {
        assert!((*c - *b - vectorcraft_geom::Vec2::new(40.0, 20.0)).hypot() < 1e-6, "{b:?} → {c:?}");
    }
}

#[test]
fn perspective_move_attaches_loose_objects_to_the_given_plane() {
    let mut s = session();
    s.execute("perspective.grid.preset", &json!({"kind": 1})).unwrap();
    let a = rect(&mut s, 250.0, 380.0, 50.0, 50.0);
    s.execute("perspective.move", &json!({"ids": [a.0], "from": [260, 390], "to": [270, 390], "plane": "ground"})).unwrap();
    assert_eq!(grid(&s).attached_plane(a), Some(Plane::Ground));
    assert!(s.execute("perspective.attach", &json!({"ids": [a.0], "plane": "none"})).is_err());
}

/// Transform in Perspective refuses a `depth`, Nudge in Perspective refuses a `dx` or `dy`, and Move Plane
/// refuses an `offset` or `by` given as text or as another value that is not a number, such as "10". Draw in
/// Perspective refuses an `at` that is not two numbers. The object, the grid and the undo history stay as they
/// were. A null `depth` or `at` is read as the key left out.
#[test]
fn perspective_commands_refuse_a_number_given_as_text() {
    let mut s = session();
    s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
    s.execute("perspective.grid.snap", &json!({"on": false})).unwrap();
    let a = rect(&mut s, 450.0, 380.0, 60.0, 60.0);
    s.execute("perspective.attach", &json!({"ids": [a.0], "plane": "right"})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let before = (path(&s, a), grid(&s).definition_json(), undo_len(&s));
    let calls = [
        ("perspective.transform", json!({"matrix": [1, 0, 0, 1, 0, 0], "depth": "10"}), "depth"),
        ("perspective.nudge", json!({"dx": "1", "dy": 0}), "dx"),
        ("perspective.nudge", json!({"dx": 0, "dy": [1]}), "dy"),
        ("perspective.plane.move", json!({"plane": "right", "offset": "10"}), "offset"),
        ("perspective.plane.move", json!({"plane": "right", "offset": "10", "by": 5}), "offset"),
        ("perspective.plane.move", json!({"plane": "right", "by": true}), "by"),
    ];
    for (c, p, key) in calls {
        refuses_non_number(&mut s, c, &p, key);
        assert_eq!((path(&s, a), grid(&s).definition_json(), undo_len(&s)), before, "{c} {p}");
    }
    let draw = |at: serde_json::Value| json!({"command": "shape.rectangle", "params": {"x": 450, "y": 380, "width": 20, "height": 20}, "plane": "right", "at": at});
    for at in [json!(["450", 380]), json!("450, 380"), json!([450]), json!([450, 380, 0])] {
        let r = s.execute("perspective.draw", &draw(at.clone()));
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg == "`at` must be [x, y]"), "at {at}: {r:?}");
        assert_eq!((path(&s, a), grid(&s).definition_json(), undo_len(&s)), before, "at {at}");
    }
    s.execute("perspective.transform", &json!({"matrix": [1, 0, 0, 1, 10, 0], "depth": null})).unwrap();
    assert_eq!(undo_len(&s), before.2 + 1);
    s.execute("perspective.draw", &draw(json!(null))).unwrap();
    assert_eq!(undo_len(&s), before.2 + 2);
}

/// With the grid defined, Define Perspective Grid with the values the grid has (Reproject or not),
/// Attach to Active Plane with no object, Release with Perspective on art not in perspective, Move
/// Plane to where the plane is (its objects moved with it or not), and Move Plane to Match Object for
/// an object the active plane is at already record no undo step and reply as before.
#[test]
fn perspective_grid_commands_that_change_nothing_record_no_undo_step() {
    let mut s = session();
    s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
    s.execute("perspective.grid.set", &json!({"cell": 40})).unwrap();
    s.execute("perspective.plane.set", &json!({"plane": "left"})).unwrap();
    let a = rect(&mut s, 300.0, 320.0, 40.0, 40.0);
    s.execute("perspective.attach", &json!({"ids": [a.0], "plane": "left"})).unwrap();
    let loose = rect(&mut s, 600.0, 100.0, 20.0, 20.0);
    with_a_step_to_redo(&mut s);
    let before = history(&s);
    let definition = grid(&s).definition_json();
    let left = |ids: Vec<u64>| json!({"plane": "left", "offset": 0.0, "ids": ids});
    let calls = [
        ("perspective.grid.set", json!({"cell": 40}), definition.clone()),
        ("perspective.grid.set", json!({"cell": 40, "reproject": true}), definition),
        ("perspective.attach", json!({"ids": []}), json!({"ids": [], "plane": "left"})),
        ("perspective.release", json!({"ids": [loose.0]}), json!(null)),
        ("perspective.plane.move", json!({"by": 0}), left(vec![])),
        ("perspective.plane.move", json!({"offset": 0, "objects": "move"}), left(vec![a.0])),
        ("perspective.plane.matchObject", json!({"id": a.0}), json!({"plane": "left", "offset": 0.0})),
    ];
    for (c, p, reply) in calls {
        assert_eq!(s.execute(c, &p).unwrap(), reply, "{c} {p}");
        assert_eq!(history(&s), before, "{c} {p}");
        assert_eq!(s.journal.last().unwrap().0, c);
    }
    // Moving the plane by 0 with copies of its objects is one undo step.
    s.execute("perspective.plane.move", &json!({"by": 0, "objects": "copy"})).unwrap();
    assert_eq!(history(&s), (before.0 + 1, 0, true));
}

/// Define Perspective Grid refuses a `reproject`, and Show Perspective Grid a `visible`, given as text
/// or as another value that is not true or false: the grid, the object on it and the undo history
/// stay as they were. A null `reproject` leaves the object where it is, and a null `visible` toggles
/// the grid, as the keys left out do.
#[test]
fn perspective_grid_commands_refuse_a_boolean_given_as_text() {
    let mut s = session();
    s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
    let a = rect(&mut s, 450.0, 380.0, 60.0, 60.0);
    s.execute("perspective.attach", &json!({"ids": [a.0], "plane": "right"})).unwrap();
    let state = |s: &Session| (grid(s).definition_json(), grid(s).visible, path(s, a), history(s));
    let before = state(&s);
    for v in not_booleans() {
        refuses_non_boolean(&mut s, "perspective.grid.set", &json!({"horizon": 250, "reproject": v}), "reproject");
        refuses_non_boolean(&mut s, "perspective.grid.show", &json!({"visible": v}), "visible");
        assert_eq!(state(&s), before, "{v}");
    }
    s.execute("perspective.grid.set", &json!({"horizon": 250, "reproject": null})).unwrap();
    assert_eq!((grid(&s).horizon, path(&s, a)), (250.0, before.2));
    assert_eq!(s.execute("perspective.grid.show", &json!({"visible": null})).unwrap(), json!({"visible": !before.1}));
}

/// For an object in perspective, Move in Perspective by 0 (along its plane or its normal),
/// Transform in Perspective by the identity, Transform Again after each, and a Perspective Selection
/// drag that ends where it started record no undo step. So do a Move by 0 and a 1 pt Nudge that
/// Snap to Grid takes back to the gridlines the object's edges are on, and Transform Again after
/// each. With Copy, the identity copies the object in one undo step.
#[test]
fn perspective_moves_that_move_nothing_record_no_undo_step() {
    let mut s = session();
    s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
    s.execute("perspective.grid.snap", &json!({"on": false})).unwrap();
    let a = rect(&mut s, 450.0, 380.0, 60.0, 60.0);
    s.execute("perspective.attach", &json!({"ids": [a.0], "plane": "right"})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    with_a_step_to_redo(&mut s);
    let before = history(&s);
    let shape = path(&s, a);
    let at = json!([480, 410]);
    let calls = [
        ("perspective.move", json!({"from": at, "to": at})),
        ("object.transformAgain", json!({})),
        ("perspective.move", json!({"from": at, "to": at, "perpendicular": true})),
        ("object.transformAgain", json!({})),
        ("perspective.transform", json!({"matrix": [1, 0, 0, 1, 0, 0]})),
        ("object.transformAgain", json!({})),
    ];
    for (c, p) in calls {
        assert_eq!(s.execute(c, &p).unwrap(), json!({"ids": [a.0]}), "{c} {p}");
        assert_eq!((history(&s), path(&s, a)), (before, shape.clone()), "{c} {p}");
        assert_eq!(s.journal.last().unwrap().0, c);
    }
    // Grabbing the object makes its plane the active one (perspective.plane.set), which marks the
    // document modified without an undo step.
    gesture(&mut s, "perspectiveSelection", &[(480.0, 410.0), (500.0, 420.0), (480.0, 410.0), (480.0, 410.0)], Mods::default());
    assert_eq!(s.journal.last().unwrap().0, "perspective.move");
    let (undo, redo, _) = history(&s);
    assert_eq!(((undo, redo), path(&s, a)), ((before.0, before.1), shape.clone()), "a drag back to its start");
    // Snap to Grid takes a move that ends 0.3 pt past a gridline crossing onto the crossing. Then it
    // takes a move by 0 and a 1 pt nudge back there, and Transform Again after each moves nothing.
    s.execute("perspective.grid.snap", &json!({"on": true})).unwrap();
    let g = grid(&s);
    let (_, _, b) = g.plane_bounds(&s.doc().unwrap().doc, &[a]).unwrap();
    let past = |v: f64| (v / g.cell).ceil() * g.cell + 0.3 - v;
    let (c, dv) = (b.center(), vectorcraft_geom::Vec2::new(past(b.x0), past(b.y0)));
    let (from, to) = (g.to_page(Plane::Right, c).unwrap(), g.to_page(Plane::Right, c + dv).unwrap());
    s.execute("perspective.move", &json!({"from": [from.x, from.y], "to": [to.x, to.y]})).unwrap();
    with_a_step_to_redo(&mut s);
    let before = history(&s);
    let on_grid = path(&s, a);
    let calls = [
        ("perspective.move", json!({"from": [to.x, to.y], "to": [to.x, to.y]})),
        ("object.transformAgain", json!({})),
        ("perspective.nudge", json!({"dx": 1, "dy": 0})),
        ("object.transformAgain", json!({})),
    ];
    for (c, p) in calls {
        assert_eq!(s.execute(c, &p).unwrap(), json!({"ids": [a.0]}), "{c} {p}");
        assert_eq!((history(&s), path(&s, a)), (before, on_grid.clone()), "{c} {p}");
    }
    // The identity with Copy copies the object in one undo step.
    let r = s.execute("perspective.transform", &json!({"matrix": [1, 0, 0, 1, 0, 0], "copy": true})).unwrap();
    assert_ne!(r, json!({"ids": [a.0]}));
    assert_eq!(history(&s), (before.0 + 1, 0, true));
}

/// Move in Perspective refuses a `copy`, `perpendicular` or `snap`, Transform in Perspective a `copy`,
/// and Nudge in Perspective a `big` or `copy`, given as text or as another value that is not true or
/// false: the object stays where it is, nothing is copied, and the undo history stays as it was. Null
/// is read as the key left out.
#[test]
fn perspective_moves_refuse_a_boolean_given_as_text() {
    let mut s = session();
    s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
    let a = rect(&mut s, 450.0, 380.0, 60.0, 60.0);
    s.execute("perspective.attach", &json!({"ids": [a.0], "plane": "right"})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let state = |s: &Session| (path(s, a), history(s), s.doc().unwrap().selection.objects.clone());
    let before = state(&s);
    let (from, to) = (json!([480, 410]), json!([500, 420]));
    for v in not_booleans() {
        for key in ["copy", "perpendicular", "snap"] {
            refuses_non_boolean(&mut s, "perspective.move", &json!({"from": from, "to": to, key: v}), key);
        }
        refuses_non_boolean(&mut s, "perspective.transform", &json!({"matrix": [1.5, 0, 0, 1.5, 0, 0], "copy": v}), "copy");
        for key in ["big", "copy"] {
            refuses_non_boolean(&mut s, "perspective.nudge", &json!({"dx": 1, "dy": 0, key: v}), key);
        }
        assert_eq!(state(&s), before, "{v}");
    }
    let calls = [
        ("perspective.move", json!({"from": from, "to": to, "copy": null, "perpendicular": null, "snap": null})),
        ("perspective.transform", json!({"matrix": [1.5, 0, 0, 1.5, 0, 0], "copy": null})),
        ("perspective.nudge", json!({"dx": 1, "dy": 0, "big": null, "copy": null})),
    ];
    for (i, (c, p)) in calls.into_iter().enumerate() {
        assert_eq!(s.execute(c, &p).unwrap(), json!({"ids": [a.0]}), "{c} {p}");
        assert_eq!(history(&s).0, before.1.0 + i + 1, "{c} {p}");
    }
}
