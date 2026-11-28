//! Puppet Warp: turning the art around a pin (`object.puppetWarp {angles}`) and the tool's pins.

use serde_json::{Value, json};
use vectorcraft_geom::{Point, Rect};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

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

fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

#[test]
fn a_pin_with_an_angle_turns_the_art_around_it() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    // One pin at the centre, held at a quarter turn: the 200 × 100 rectangle stands up.
    s.execute("object.puppetWarp", &json!({"id": a.0, "pins": [[200, 150]], "moved": [[200, 150]], "angles": [90]})).unwrap();
    let b = bounds(&s, a);
    assert!((b.width() - 100.0).abs() < 4.0 && (b.height() - 200.0).abs() < 4.0, "{b:?}");
    assert!(b.center().distance(Point::new(200.0, 150.0)) < 2.0, "{b:?}");
    // null leaves a pin free; the list must be as long as the pins.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("object.puppetWarp", &json!({"id": a.0, "pins": [[200, 150]], "moved": [[210, 150]], "angles": [null]})).unwrap();
    assert!((bounds(&s, a).x0 - 110.0).abs() < 0.5, "a free single pin only moves the art");
    for bad in [json!([90, 0]), json!("90"), json!({})] {
        assert!(s.execute("object.puppetWarp", &json!({"id": a.0, "pins": [[200, 150]], "moved": [[200, 150]], "angles": bad})).is_err(), "{bad}");
    }
}

#[test]
fn alt_drag_with_the_tool_turns_the_art_in_one_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("puppetWarp", v).unwrap();
    s.set_tool_option("selectAllPins", &json!(false));
    // Select the pin in the middle, then Alt-drag about a quarter turn around it, 20 pt out.
    s.pointer(&PointerEvent::new(PointerKind::Down, 200.0, 150.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 200.0, 150.0), v).unwrap();
    let n = s.doc().unwrap().history.undo.len();
    let alt = Mods { alt: true, ..Default::default() };
    for (k, x, y) in
        [(PointerKind::Down, 220.0, 150.0), (PointerKind::Drag, 214.0, 164.0), (PointerKind::Drag, 200.0, 170.0), (PointerKind::Up, 200.0, 170.0)]
    {
        s.pointer(&PointerEvent::new(k, x, y).with_mods(alt), v).unwrap();
    }
    assert_eq!(s.doc().unwrap().history.undo.len(), n + 1);
    let (cmd, p) = s.journal.last().unwrap();
    assert_eq!(cmd, "object.puppetWarp");
    // The turn is the angle the pointer swept around the pin.
    let i = p["angles"].as_array().unwrap().iter().position(|a| a.is_number()).unwrap();
    let (cx, cy) = (p["moved"][i][0].as_f64().unwrap(), p["moved"][i][1].as_f64().unwrap());
    let swept = ((170.0 - cy).atan2(200.0 - cx) - (150.0 - cy).atan2(220.0 - cx)).to_degrees();
    assert!((p["angles"][i].as_f64().unwrap() - swept).abs() < 1e-6 && swept > 60.0, "{p}");
    let b = bounds(&s, a);
    assert!(b.height() > 110.0, "the art turned: {b:?}");
}

// ---------- the pins' session (M8.6) ----------

fn pins(s: &mut Session) -> Value {
    s.execute("object.puppetWarp.pins", &json!({})).unwrap()
}

fn anchors(s: &Session, id: NodeId) -> Vec<Point> {
    let mut out = vec![];
    s.doc().unwrap().doc.node(id).unwrap().walk(&mut |n| {
        if let Some(p) = n.path_data() {
            out.extend(p.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)));
        }
    });
    out
}

fn worst(a: &[Point], b: &[Point]) -> f64 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| x.distance(*y)).fold(0.0, f64::max)
}

fn click(s: &mut Session, x: f64, y: f64) {
    let v = ViewInfo::default();
    s.pointer(&PointerEvent::new(PointerKind::Down, x, y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, x, y), v).unwrap();
}

fn drag(s: &mut Session, from: (f64, f64), to: (f64, f64)) {
    let v = ViewInfo::default();
    s.pointer(&PointerEvent::new(PointerKind::Down, from.0, from.1), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, to.0, to.1), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, to.0, to.1), v).unwrap();
}

fn pin_overlays(s: &mut Session) -> usize {
    s.overlays(ViewInfo::default()).iter().filter(|o| matches!(o, vectorcraft_tools::Overlay::Anchor { .. })).count()
}

#[test]
fn pins_appear_when_the_tool_is_chosen_or_art_selected() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 300.0, 60.0);
    // Art selected, then the tool: the automatic pins show at once (the centre and both ends).
    s.select_tool("puppetWarp", ViewInfo::default()).unwrap();
    assert_eq!(pin_overlays(&mut s), 3);
    let p = pins(&mut s);
    assert_eq!((p["auto"].clone(), p["ids"].clone(), p["rest"].clone()), (json!(true), json!([a.0]), json!(true)));
    assert_eq!(p["pins"], p["moved"], "automatic pins sit where they started");
    // The tool first, then art selected.
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(pin_overlays(&mut s), 0);
    assert!(s.execute("object.puppetWarp.pins", &json!({})).is_err(), "nothing selected");
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    assert_eq!(pin_overlays(&mut s), 3);
}

#[test]
fn warps_start_from_the_rest_shape_and_dont_stack() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 300.0, 60.0);
    let rest = anchors(&s, a);
    let p = pins(&mut s);
    // Move the right-end pin up, then back: the original comes back.
    let right = p["pins"].as_array().unwrap().iter().position(|q| q[0].as_f64().unwrap() > 300.0).unwrap();
    let mut up = p.clone();
    up["moved"][right][1] = json!(up["moved"][right][1].as_f64().unwrap() - 60.0);
    s.execute("object.puppetWarp", &up).unwrap();
    let once = anchors(&s, a);
    assert!(worst(&rest, &once) > 30.0);
    // The same pins again: the same warp (it doesn't stack on the last one).
    s.execute("object.puppetWarp", &up).unwrap();
    assert!(worst(&once, &anchors(&s, a)) < 1e-9);
    s.execute("object.puppetWarp", &p).unwrap();
    assert!(worst(&rest, &anchors(&s, a)) < 0.01, "back to the original");
    // The query reads the pins back: where they started, where they are.
    let now = pins(&mut s);
    assert_eq!((now["pins"].clone(), now["auto"].clone()), (p["pins"].clone(), json!(false)));
    // A pin off the mesh is refused.
    let mut off = p.clone();
    off["pins"][0] = json!([900, 900]);
    let e = s.execute("object.puppetWarp", &off).unwrap_err().to_string();
    assert!(e.contains("off the artwork"), "{e}");
}

#[test]
fn pins_follow_undo_and_redo() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 300.0, 60.0);
    let rest = anchors(&s, a);
    s.select_tool("puppetWarp", ViewInfo::default()).unwrap();
    // Adding a pin is one undo step; dragging it is another.
    click(&mut s, 250.0, 150.0);
    let added = pins(&mut s);
    assert_eq!(added["pins"].as_array().unwrap().len(), 4);
    drag(&mut s, (250.0, 150.0), (250.0, 110.0));
    let warped = anchors(&s, a);
    let moved = pins(&mut s);
    assert_eq!(moved["moved"][3], json!([250.0, 110.0]));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(pins(&mut s), added, "the pin went back with the art");
    assert!(worst(&rest, &anchors(&s, a)) < 1e-9);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(pins(&mut s)["auto"], json!(true), "no pin placed");
    assert_eq!(pin_overlays(&mut s), 3);
    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(pins(&mut s), moved);
    assert!(worst(&warped, &anchors(&s, a)) < 1e-9);
    // Pins persist across tool switches while the selection stays.
    s.select_tool("selection", ViewInfo::default()).unwrap();
    s.select_tool("puppetWarp", ViewInfo::default()).unwrap();
    assert_eq!(pins(&mut s), moved);
    assert_eq!(pin_overlays(&mut s), 4);
}

#[test]
fn a_new_selection_clears_the_pins() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 300.0, 60.0);
    let b = rect(&mut s, 100.0, 300.0, 50.0, 50.0);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.select_tool("puppetWarp", ViewInfo::default()).unwrap();
    click(&mut s, 250.0, 150.0);
    drag(&mut s, (250.0, 150.0), (250.0, 120.0));
    let warped = anchors(&s, a);
    assert_eq!(pins(&mut s)["auto"], json!(false));
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    let dirty = s.doc().unwrap().is_dirty();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    // Fresh automatic pins on the art as it is now (the warp stays).
    let p = pins(&mut s);
    assert_eq!(p["auto"], json!(true));
    assert!(worst(&warped, &anchors(&s, a)) < 1e-9);
    assert_eq!((s.doc().unwrap().history.undo.len(), s.doc().unwrap().is_dirty()), (undo, dirty), "not an edit");
    // Something else editing the art keeps the pins, starting again from the art as it is.
    click(&mut s, 200.0, 140.0);
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    let p2 = pins(&mut s);
    assert_eq!(p2["auto"], json!(false));
    assert_eq!(p2["pins"], p2["moved"]);
}

/// Selecting other art while a background save writes the file drops the pins and leaves the
/// document saved once the file is written; an edit made meanwhile leaves it modified until it is
/// undone.
#[test]
fn a_new_selection_during_a_save_leaves_the_document_saved() {
    use crate::cmd::fileio::{SaveMode, save_job, save_plan};
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 300.0, 60.0);
    let b = rect(&mut s, 100.0, 300.0, 50.0, 50.0);
    s.select_tool("puppetWarp", ViewInfo::default()).unwrap();
    let uid = s.doc().unwrap().uid;
    for edit in [false, true] {
        s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
        click(&mut s, 250.0, 150.0);
        assert!(s.doc().unwrap().doc.puppet.is_some());
        let plan = save_plan(&s, SaveMode::SaveAs, &json!({"path": "/x/pins.vectorcraft", "modified": 777})).unwrap();
        let job = save_job(&mut s, plan).unwrap();
        s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
        if edit {
            s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
        }
        job.write(|_, _| Ok(())).unwrap();
        job.complete(&mut s, uid);
        let st = s.doc().unwrap();
        assert_eq!(st.doc.metadata.modified, Some(777));
        assert!(st.doc.puppet.is_none(), "the pins stay dropped");
        assert_eq!(st.is_dirty(), edit, "modified only by the edit");
    }
    // Undoing the edit is back at what the file holds.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!s.doc().unwrap().is_dirty());
}

#[test]
fn puppet_warp_works_on_groups_compound_paths_and_open_strokes() {
    for kind in ["group", "compound", "open"] {
        let mut s = session();
        let id = match kind {
            "open" => {
                let r = s.execute("shape.line", &json!({"x1": 100, "y1": 200, "x2": 400, "y2": 200})).unwrap();
                s.execute("stroke.set", &json!({"weight": 8})).unwrap();
                NodeId(r["id"].as_u64().unwrap())
            }
            _ => {
                let a = rect(&mut s, 100.0, 170.0, 140.0, 60.0);
                let b = rect(&mut s, 260.0, 170.0, 140.0, 60.0);
                s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
                let cmd = if kind == "group" { "object.group" } else { "object.compoundPath.make" };
                let r = s.execute(cmd, &json!({})).unwrap();
                r["id"].as_u64().map(NodeId).unwrap_or_else(|| s.doc().unwrap().selection.objects[0])
            }
        };
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        let rest = anchors(&s, id);
        s.select_tool("puppetWarp", ViewInfo::default()).unwrap();
        // Pin both ends, lift the right one with the tool, then put it back.
        click(&mut s, 104.0, 200.0);
        click(&mut s, 396.0, 200.0);
        drag(&mut s, (396.0, 200.0), (396.0, 140.0));
        assert!(worst(&rest, &anchors(&s, id)) > 20.0, "{kind}: warped");
        let n = s.doc().unwrap().doc.node(id).unwrap().children().map(Vec::len);
        drag(&mut s, (396.0, 140.0), (396.0, 200.0));
        assert!(worst(&rest, &anchors(&s, id)) < 0.05, "{kind}: back to the original");
        assert_eq!(s.doc().unwrap().doc.node(id).unwrap().children().map(Vec::len), n, "{kind}: the structure stays");
    }
}

#[test]
fn pins_are_never_saved() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 300.0, 60.0);
    s.select_tool("puppetWarp", ViewInfo::default()).unwrap();
    click(&mut s, 250.0, 150.0);
    assert!(s.doc().unwrap().doc.puppet.is_some());
    let r = s.execute("document.serialize", &json!({"format": "vectorcraft"})).unwrap();
    let data = r["dataBase64"].as_str().unwrap().to_string();
    s.execute("document.open", &json!({"name": "pins.vectorcraft", "dataBase64": data})).unwrap();
    let st = s.doc().unwrap();
    assert!(st.doc.puppet.is_none());
    assert_eq!(st.doc.node_count(), 2, "layer + rectangle");
}
