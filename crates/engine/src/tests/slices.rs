//! Object → Slice, View → Hide/Lock Slices, Select → Object → Slices and `slice.list`.

use serde_json::{Value, json};
use vectorcraft_doc::SliceKind;

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    // Unstroked art: visual bounds are the geometry.
    s.paint.stroke = vectorcraft_color::Paint::None;
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> u64 {
    run(s, "shape.rectangle", json!({"x": x, "y": y, "width": w, "height": h}))["id"].as_u64().unwrap()
}

/// `slice.list`'s slices.
fn slices(s: &mut Session) -> Vec<Value> {
    run(s, "slice.list", json!({}))["slices"].as_array().unwrap().clone()
}

/// The laid-out rectangle of slice `id` as [x, y, width, height].
fn rect_of(s: &mut Session, id: u64) -> [f64; 4] {
    let v = slices(s).into_iter().find(|v| v["id"] == json!(id)).unwrap();
    ["x", "y", "width", "height"].map(|k| v[k].as_f64().unwrap())
}

fn user_count(s: &Session) -> usize {
    s.doc().unwrap().doc.slices.len()
}

/// A user slice over `[x, y, w, h]`, made from a temporary rectangle.
fn user_slice(s: &mut Session, r: [f64; 4]) -> u64 {
    let tmp = rect(s, r[0], r[1], r[2], r[3]);
    let id = run(s, "object.slice.fromSelection", json!({"ids": [tmp]}))["id"].as_u64().unwrap();
    run(s, "select.set", json!({"ids": [tmp]}));
    run(s, "edit.clear", json!({}));
    id
}

#[test]
fn an_object_slice_follows_its_object_and_releases_back_to_it() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 50.0, 40.0);
    let made = run(&mut s, "object.slice.make", json!({}));
    assert_eq!(made["ids"], json!([a]));
    assert_eq!(rect_of(&mut s, a), [100.0, 100.0, 50.0, 40.0]);
    let list = slices(&mut s);
    assert!(list.iter().any(|v| v["source"] == "auto"), "auto slices fill the artboard");
    let area: f64 = list.iter().map(|v| v["width"].as_f64().unwrap() * v["height"].as_f64().unwrap()).sum();
    assert_eq!(area, 300.0 * 300.0);
    // Moving the object moves its slice; one undo step brings it back.
    run(&mut s, "object.move", json!({"dx": 20, "dy": 10}));
    assert_eq!(rect_of(&mut s, a), [120.0, 110.0, 50.0, 40.0]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(rect_of(&mut s, a), [100.0, 100.0, 50.0, 40.0]);
    // Make again changes nothing; Release leaves the object.
    assert_eq!(run(&mut s, "object.slice.make", json!({}))["ids"], json!([]));
    assert_eq!(run(&mut s, "object.slice.release", json!({}))["ids"], json!([a]));
    assert!(s.doc().unwrap().doc.node(NodeId(a)).unwrap().extras().slice.is_none());
    assert!(slices(&mut s).is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().doc.node(NodeId(a)).unwrap().extras().slice.is_some());
}

#[test]
fn releasing_a_user_slice_leaves_an_unpainted_rectangle() {
    let mut s = session();
    let id = user_slice(&mut s, [10.0, 20.0, 30.0, 40.0]);
    run(&mut s, "select.object.slices", json!({}));
    let r = run(&mut s, "object.slice.release", json!({}));
    assert_eq!(s.doc().unwrap().doc.slice(NodeId(id)), None);
    let rid = r["ids"][0].as_u64().unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.slices.is_empty());
    let n = d.node(NodeId(rid)).unwrap();
    assert_eq!(n.geometric_bounds().unwrap(), vectorcraft_geom::Rect::new(10.0, 20.0, 40.0, 60.0));
    assert!(matches!(n.appearance.fill_paint(), vectorcraft_color::Paint::None));
    assert_eq!(s.doc().unwrap().selection.objects, vec![NodeId(rid)]);
}

#[test]
fn guides_give_nine_slices() {
    let mut s = session();
    for (vertical, pos) in [(true, 100.0), (true, 200.0), (false, 50.0), (false, 250.0), (true, 400.0)] {
        run(&mut s, "guide.add", json!({"vertical": vertical, "pos": pos}));
    }
    user_slice(&mut s, [0.0, 0.0, 10.0, 10.0]);
    let ids = run(&mut s, "object.slice.fromGuides", json!({}))["ids"].as_array().unwrap().len();
    assert_eq!(ids, 9, "the guide off the artboard cuts nothing");
    assert_eq!(user_count(&s), 9, "the earlier slice is replaced");
    let list = slices(&mut s);
    assert_eq!(list.len(), 9, "the grid covers the artboard: no auto slices");
    assert_eq!(list[4]["x"], 100.0);
    assert_eq!(list[4]["width"], 100.0);
    assert_eq!(list[4]["height"], 200.0);
    assert_eq!(list.iter().map(|v| v["number"].as_u64().unwrap()).collect::<Vec<_>>(), (1..=9).collect::<Vec<_>>());
    // One undo step takes the grid back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(user_count(&s), 1);
}

#[test]
fn divide_two_by_three_and_by_size() {
    let mut s = session();
    user_slice(&mut s, [0.0, 0.0, 90.0, 60.0]);
    run(&mut s, "select.object.slices", json!({}));
    run(&mut s, "object.slice.options", json!({"name": "hero", "url": "https://example.com"}));
    let pieces = run(&mut s, "object.slice.divide", json!({"rows": 2, "columns": 3}))["ids"].as_array().unwrap().clone();
    assert_eq!(pieces.len(), 6);
    let d = &s.doc().unwrap().doc;
    let rects: Vec<_> = d.slices.iter().map(|sl| (sl.rect.x0, sl.rect.y0, sl.rect.width(), sl.rect.height())).collect();
    assert_eq!(
        rects,
        [
            (0.0, 0.0, 30.0, 30.0),
            (30.0, 0.0, 30.0, 30.0),
            (60.0, 0.0, 30.0, 30.0),
            (0.0, 30.0, 30.0, 30.0),
            (30.0, 30.0, 30.0, 30.0),
            (60.0, 30.0, 30.0, 30.0)
        ]
    );
    assert!(d.slices.iter().all(|sl| sl.options.url == "https://example.com"));
    assert_eq!(d.slices.iter().filter(|sl| sl.options.name == "hero").count(), 1, "only the first piece keeps the name");
    assert_eq!(s.doc().unwrap().selection.slices.len(), 6, "the pieces are selected");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(user_count(&s), 1);
    // By size: 25 pt rows of a 60 pt slice: 25, 25 and 10.
    run(&mut s, "select.object.slices", json!({}));
    run(&mut s, "object.slice.divide", json!({"rowHeight": 25}));
    let hs: Vec<f64> = s.doc().unwrap().doc.slices.iter().map(|sl| sl.rect.height()).collect();
    assert_eq!(hs, [25.0, 25.0, 10.0]);
    assert!(s.execute("object.slice.divide", &json!({"rows": 0})).is_err());
    assert!(s.execute("object.slice.divide", &json!({"columns": 5000})).is_err());
    // A 32-bit build read 2^32 + 2 as 2, passed the check and went on to list 2^32 + 3 cuts.
    let e = s.execute("object.slice.divide", &json!({"rows": (1u64 << 32) + 2})).unwrap_err().to_string();
    assert!(e.contains("divide into 1 to"), "{e}");
    assert!(s.execute("object.slice.divide", &json!({})).is_err(), "nothing to divide by");
}

#[test]
fn combine_and_duplicate() {
    let mut s = session();
    user_slice(&mut s, [0.0, 0.0, 20.0, 20.0]);
    run(&mut s, "select.object.slices", json!({}));
    assert!(s.execute("object.slice.combine", &json!({})).is_err(), "two or more");
    let b = rect(&mut s, 50.0, 40.0, 10.0, 10.0);
    run(&mut s, "object.slice.make", json!({}));
    run(&mut s, "select.object.slices", json!({}));
    let c = run(&mut s, "object.slice.combine", json!({}))["id"].as_u64().unwrap();
    assert_eq!(rect_of(&mut s, c), [0.0, 0.0, 60.0, 50.0]);
    assert_eq!(user_count(&s), 1);
    assert!(s.doc().unwrap().doc.node(NodeId(b)).unwrap().extras().slice.is_none(), "the object slice was released");
    assert_eq!(s.doc().unwrap().selection.slices, vec![NodeId(c)]);
    let dup = run(&mut s, "object.slice.duplicate", json!({}))["ids"][0].as_u64().unwrap();
    assert_eq!(rect_of(&mut s, dup), [10.0, 10.0, 60.0, 50.0]);
    assert_eq!(s.doc().unwrap().selection.slices, vec![NodeId(dup)]);
}

#[test]
fn slice_options_set_read_and_undo_in_one_step() {
    let mut s = session();
    user_slice(&mut s, [0.0, 0.0, 50.0, 50.0]);
    run(&mut s, "select.object.slices", json!({}));
    let o = run(&mut s, "object.slice.options", json!({}));
    assert_eq!((o["kind"].clone(), o["htmlText"].clone()), (json!("image"), json!(false)));
    let undo = s.doc().unwrap().history.undo.len();
    let o = run(
        &mut s,
        "object.slice.options",
        json!({"kind": "No Image", "name": " cell ", "text": " <b>Hi</b>", "background": "#FF0000", "hAlign": "center", "vAlign": "bottom"}),
    );
    assert_eq!(
        o,
        json!({"kind": "noImage", "name": "cell", "url": "", "target": "", "message": "", "alt": "", "text": " <b>Hi</b>", "background": "#ff0000", "hAlign": "center", "vAlign": "bottom", "htmlText": false})
    );
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(slices(&mut s)[0]["name"], "cell");
    assert!(s.execute("object.slice.options", &json!({"kind": "htmlText"})).is_err(), "HTML Text is for type");
    assert!(s.execute("object.slice.options", &json!({"background": "pink-ish"})).is_err());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().doc.slices[0].options, Default::default());
    // An object slice of type can be HTML Text.
    let t = run(&mut s, "text.create", json!({"x": 10, "y": 100, "text": "Hello"}))["id"].as_u64().unwrap();
    run(&mut s, "object.slice.make", json!({"ids": [t]}));
    run(&mut s, "select.set", json!({"ids": [t]}));
    let o = run(&mut s, "object.slice.options", json!({"kind": "htmlText"}));
    assert_eq!((o["kind"].clone(), o["htmlText"].clone()), (json!("htmlText"), json!(true)));
    assert_eq!(s.doc().unwrap().doc.node(NodeId(t)).unwrap().extras().slice.as_ref().unwrap().kind, SliceKind::HtmlText);
}

#[test]
fn clip_to_artboard_hide_lock_select_and_delete_all() {
    let mut s = session();
    let id = user_slice(&mut s, [250.0, 250.0, 100.0, 100.0]);
    assert_eq!(rect_of(&mut s, id), [250.0, 250.0, 50.0, 50.0], "clipped to the artboard");
    assert_eq!(run(&mut s, "object.slice.clipToArtboard", json!({}))["on"], false);
    assert_eq!(rect_of(&mut s, id), [250.0, 250.0, 100.0, 100.0]);
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().doc.slices_clip_to_artboard);
    assert_eq!(run(&mut s, "object.slice.clipToArtboard", json!({"on": true}))["on"], true);

    assert_eq!(run(&mut s, "view.slices.hide", json!({}))["hidden"], true);
    assert!(s.slices_hidden());
    assert_eq!(run(&mut s, "view.slices.lock", json!({"locked": true}))["locked"], true);
    let l = run(&mut s, "slice.list", json!({}));
    assert_eq!((l["hidden"].clone(), l["locked"].clone(), l["clipToArtboard"].clone()), (json!(true), json!(true), json!(true)));
    run(&mut s, "view.slices.hide", json!({"hidden": false}));
    assert!(!s.slices_hidden());
    run(&mut s, "view.slices.lock", json!({"locked": false}));

    let a = rect(&mut s, 10.0, 10.0, 10.0, 10.0);
    run(&mut s, "object.slice.make", json!({}));
    assert_eq!(run(&mut s, "select.object.slices", json!({}))["count"], 2);
    let sel = &s.doc().unwrap().selection;
    assert_eq!((sel.slices.clone(), sel.objects.is_empty()), (vec![NodeId(id), NodeId(a)], true));
    assert!(slices(&mut s).iter().filter(|v| v["selected"] == true).count() == 2);
    // Selecting objects drops the slice selection.
    run(&mut s, "select.all", json!({}));
    assert!(s.doc().unwrap().selection.slices.is_empty());

    assert_eq!(run(&mut s, "object.slice.deleteAll", json!({}))["count"], 2);
    assert_eq!(user_count(&s), 0);
    assert!(s.doc().unwrap().doc.node(NodeId(a)).unwrap().extras().slice.is_none());
    assert!(s.execute("object.slice.deleteAll", &json!({})).is_err(), "nothing to delete");
}

/// The slices selected with the Slice Selection tool or a slice command.
fn picked(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().selection.slices.clone()
}

/// View → Lock Slices deselects the slices in every open document, and while it is on nothing
/// selects one, so the commands that act on the selected slices leave them alone. Turning it off
/// leaves them deselected.
#[test]
fn lock_slices_deselects_the_slices_and_keeps_them_unselected() {
    let mut s = session();
    let id = user_slice(&mut s, [0.0, 0.0, 50.0, 50.0]);
    run(&mut s, "select.object.slices", json!({}));
    run(&mut s, "view.slices.lock", json!({"locked": true}));
    assert!(picked(&s).is_empty(), "after Lock Slices");
    let _ = s.execute("object.slice.move", &json!({"dx": 10, "dy": 0}));
    let _ = s.execute("object.slice.options", &json!({"name": "hero"}));
    assert_eq!(rect_of(&mut s, id), [0.0, 0.0, 50.0, 50.0]);
    assert_eq!(s.doc().unwrap().doc.slices[0].options.name, "");
    assert_eq!(run(&mut s, "select.object.slices", json!({}))["count"], 0);
    assert_eq!(run(&mut s, "object.slice.select", json!({"slices": [id]}))["selected"], json!([]));
    run(&mut s, "object.slice.create", json!({"x": 100, "y": 100, "width": 20, "height": 20}));
    assert!(picked(&s).is_empty(), "after Create Slice");
    // Locking from another document deselects this one's slices too.
    run(&mut s, "view.slices.lock", json!({"locked": false}));
    assert!(picked(&s).is_empty(), "after turning Lock Slices off");
    assert_eq!(run(&mut s, "select.object.slices", json!({}))["count"], 2);
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    run(&mut s, "view.slices.lock", json!({"locked": true}));
    run(&mut s, "document.activate", json!({"index": 0}));
    assert!(picked(&s).is_empty(), "in the other document");
}

/// View → Hide Slices deselects the slices in every open document, and while they are hidden
/// nothing selects one, so the commands that act on the selected slices leave them alone. Turning
/// it off leaves them deselected.
#[test]
fn hide_slices_deselects_the_slices_and_keeps_them_unselected() {
    let mut s = session();
    let id = user_slice(&mut s, [0.0, 0.0, 50.0, 50.0]);
    run(&mut s, "select.object.slices", json!({}));
    run(&mut s, "view.slices.hide", json!({"hidden": true}));
    assert!(picked(&s).is_empty(), "after Hide Slices");
    let _ = s.execute("object.slice.move", &json!({"dx": 10, "dy": 0}));
    let _ = s.execute("object.slice.options", &json!({"name": "hero"}));
    assert_eq!(rect_of(&mut s, id), [0.0, 0.0, 50.0, 50.0]);
    assert_eq!(s.doc().unwrap().doc.slices[0].options.name, "");
    assert_eq!(run(&mut s, "select.object.slices", json!({}))["count"], 0);
    assert_eq!(run(&mut s, "object.slice.select", json!({"slices": [id]}))["selected"], json!([]));
    run(&mut s, "object.slice.create", json!({"x": 100, "y": 100, "width": 20, "height": 20}));
    assert!(picked(&s).is_empty(), "after Create Slice");
    // Hiding from another document deselects this one's slices too.
    run(&mut s, "view.slices.hide", json!({"hidden": false}));
    assert!(picked(&s).is_empty(), "after turning Hide Slices off");
    assert_eq!(run(&mut s, "select.object.slices", json!({}))["count"], 2);
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    run(&mut s, "view.slices.hide", json!({"hidden": true}));
    run(&mut s, "document.activate", json!({"index": 0}));
    assert!(picked(&s).is_empty(), "in the other document");
}

/// Undo and Redo bring back a selection without the slices that Lock Slices has locked since, so
/// a move after them leaves the slice where it is.
#[test]
fn undo_and_redo_leave_locked_slices_deselected() {
    let mut s = session();
    let id = run(&mut s, "object.slice.create", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    run(&mut s, "object.slice.move", json!({"dx": 10, "dy": 0}));
    run(&mut s, "view.slices.lock", json!({"locked": true}));
    run(&mut s, "edit.undo", json!({}));
    assert!(picked(&s).is_empty(), "after Undo");
    let _ = s.execute("object.slice.move", &json!({"dx": 10, "dy": 0}));
    assert_eq!(rect_of(&mut s, id)[0], 0.0);
    // Undone with the slice selected, redone once it is locked.
    run(&mut s, "view.slices.lock", json!({"locked": false}));
    run(&mut s, "object.slice.select", json!({"slices": [id]}));
    run(&mut s, "object.slice.move", json!({"dx": 10, "dy": 0}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "view.slices.lock", json!({"locked": true}));
    run(&mut s, "edit.redo", json!({}));
    assert!(picked(&s).is_empty(), "after Redo");
    let _ = s.execute("object.slice.move", &json!({"dx": 10, "dy": 0}));
    assert_eq!(rect_of(&mut s, id)[0], 10.0);
}

/// Undo and Redo bring back a selection without the slices that Hide Slices has hidden since, so
/// a move after them leaves the slice where it is.
#[test]
fn undo_and_redo_leave_hidden_slices_deselected() {
    let mut s = session();
    let id = run(&mut s, "object.slice.create", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    run(&mut s, "object.slice.move", json!({"dx": 10, "dy": 0}));
    run(&mut s, "view.slices.hide", json!({"hidden": true}));
    run(&mut s, "edit.undo", json!({}));
    assert!(picked(&s).is_empty(), "after Undo");
    let _ = s.execute("object.slice.move", &json!({"dx": 10, "dy": 0}));
    assert_eq!(rect_of(&mut s, id)[0], 0.0);
    // Undone with the slice selected, redone once it is hidden.
    run(&mut s, "view.slices.hide", json!({"hidden": false}));
    run(&mut s, "object.slice.select", json!({"slices": [id]}));
    run(&mut s, "object.slice.move", json!({"dx": 10, "dy": 0}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "view.slices.hide", json!({"hidden": true}));
    run(&mut s, "edit.redo", json!({}));
    assert!(picked(&s).is_empty(), "after Redo");
    let _ = s.execute("object.slice.move", &json!({"dx": 10, "dy": 0}));
    assert_eq!(rect_of(&mut s, id)[0], 10.0);
}

/// A failed batch, a canceled undo group, a failed drag preview and a canceled drag bring back the
/// selection from before them, without the slices that Lock Slices locked inside them.
#[test]
fn a_failed_batch_a_canceled_undo_group_and_a_canceled_drag_leave_locked_slices_deselected() {
    let mut s = session();
    let id = run(&mut s, "object.slice.create", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    let steps = json!([{"command": "view.slices.lock", "params": {"locked": true}}, {"command": "object.isolate", "params": {}}]);
    assert!(s.execute("command.batch", &json!({"commands": steps})).is_err());
    assert!(s.slices_locked());
    assert!(picked(&s).is_empty(), "after the failed batch");
    run(&mut s, "view.slices.lock", json!({"locked": false}));
    run(&mut s, "object.slice.select", json!({"slices": [id]}));
    run(&mut s, "edit.undoGroup.begin", json!({}));
    run(&mut s, "object.slice.move", json!({"dx": 10, "dy": 0}));
    run(&mut s, "view.slices.lock", json!({"locked": true}));
    run(&mut s, "edit.undoGroup.end", json!({"cancel": true}));
    assert_eq!(rect_of(&mut s, id)[0], 0.0);
    assert!(picked(&s).is_empty(), "after the canceled undo group");
    run(&mut s, "view.slices.lock", json!({"locked": false}));
    run(&mut s, "object.slice.select", json!({"slices": [id]}));
    s.begin_interaction("Move Slice").unwrap();
    s.preview("object.slice.move", &json!({"dx": 10, "dy": 0})).unwrap();
    run(&mut s, "view.slices.lock", json!({"locked": true}));
    // A preview that fails puts back the snapshot, without the locked slice.
    assert!(s.preview("object.slice.move", &json!({"dx": "far", "dy": 0})).is_err());
    assert!(picked(&s).is_empty(), "after a failed preview");
    s.cancel_interaction().unwrap();
    assert_eq!(rect_of(&mut s, id)[0], 0.0);
    assert!(picked(&s).is_empty(), "after the canceled drag");
}

/// A failed batch, a canceled undo group, a failed drag preview and a canceled drag bring back the
/// selection from before them, without the slices that Hide Slices hid inside them.
#[test]
fn a_failed_batch_a_canceled_undo_group_and_a_canceled_drag_leave_hidden_slices_deselected() {
    let mut s = session();
    let id = run(&mut s, "object.slice.create", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
    let steps = json!([{"command": "view.slices.hide", "params": {"hidden": true}}, {"command": "object.isolate", "params": {}}]);
    assert!(s.execute("command.batch", &json!({"commands": steps})).is_err());
    assert!(s.slices_hidden());
    assert!(picked(&s).is_empty(), "after the failed batch");
    run(&mut s, "view.slices.hide", json!({"hidden": false}));
    run(&mut s, "object.slice.select", json!({"slices": [id]}));
    run(&mut s, "edit.undoGroup.begin", json!({}));
    run(&mut s, "object.slice.move", json!({"dx": 10, "dy": 0}));
    run(&mut s, "view.slices.hide", json!({"hidden": true}));
    run(&mut s, "edit.undoGroup.end", json!({"cancel": true}));
    assert_eq!(rect_of(&mut s, id)[0], 0.0);
    assert!(picked(&s).is_empty(), "after the canceled undo group");
    run(&mut s, "view.slices.hide", json!({"hidden": false}));
    run(&mut s, "object.slice.select", json!({"slices": [id]}));
    s.begin_interaction("Move Slice").unwrap();
    s.preview("object.slice.move", &json!({"dx": 10, "dy": 0})).unwrap();
    run(&mut s, "view.slices.hide", json!({"hidden": true}));
    // A preview that fails puts back the snapshot, without the hidden slice.
    assert!(s.preview("object.slice.move", &json!({"dx": "far", "dy": 0})).is_err());
    assert!(picked(&s).is_empty(), "after a failed preview");
    s.cancel_interaction().unwrap();
    assert_eq!(rect_of(&mut s, id)[0], 0.0);
    assert!(picked(&s).is_empty(), "after the canceled drag");
}

/// A canceled undo scope that joined an open undo group brings back the selection from when the
/// scope began, without the slices that Lock Slices or Hide Slices took out inside it.
#[test]
fn a_canceled_undo_scope_leaves_locked_or_hidden_slices_deselected() {
    for (command, key) in [("view.slices.lock", "locked"), ("view.slices.hide", "hidden")] {
        let mut s = session();
        let id = run(&mut s, "object.slice.create", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].as_u64().unwrap();
        run(&mut s, "object.slice.select", json!({"slices": [id]}));
        run(&mut s, "edit.undoGroup.begin", json!({}));
        run(&mut s, "object.slice.move", json!({"dx": 10, "dy": 0}));
        let scope = s.begin_undo_scope().unwrap();
        run(&mut s, command, json!({ key: true }));
        s.end_undo_scope(scope, true);
        assert!(picked(&s).is_empty(), "{command}: after the canceled scope");
        let _ = s.execute("object.slice.move", &json!({"dx": 10, "dy": 0}));
        assert_eq!(rect_of(&mut s, id)[0], 10.0, "{command}");
    }
}

/// Select slice `id` and the first ruler guide together, through [`Session::select`].
fn pick_slice_and_guide(s: &mut Session, id: u64) {
    s.select(|_, sel| {
        sel.slices = vec![NodeId(id)];
        sel.guides = vec![0];
    })
    .unwrap();
}

/// With Lock Guides, Lock Slices and Hide Slices all on, Undo and a selection deselect both the
/// guide and the slice, which is locked and hidden at once, and the slices stay deselected until
/// both Lock Slices and Hide Slices are off.
#[test]
fn locked_guides_and_a_locked_hidden_slice_are_deselected_together() {
    let mut s = session();
    let id = user_slice(&mut s, [0.0, 0.0, 50.0, 50.0]);
    run(&mut s, "guide.add", json!({"vertical": true, "pos": 100}));
    pick_slice_and_guide(&mut s, id);
    // A step whose selection before it holds the guide and the slice, for Undo to bring back.
    rect(&mut s, 200.0, 200.0, 10.0, 10.0);
    run(&mut s, "view.guides.lock", json!({"locked": true}));
    run(&mut s, "view.slices.lock", json!({"locked": true}));
    run(&mut s, "view.slices.hide", json!({"hidden": true}));
    let none = |s: &Session| {
        let sel = &s.doc().unwrap().selection;
        sel.slices.is_empty() && sel.guides.is_empty()
    };
    run(&mut s, "edit.undo", json!({}));
    assert!(none(&s), "after Undo");
    pick_slice_and_guide(&mut s, id);
    assert!(none(&s), "after a selection");
    // Shown while still locked, or unlocked while still hidden, the slices stay out.
    run(&mut s, "view.slices.hide", json!({"hidden": false}));
    assert_eq!(run(&mut s, "select.object.slices", json!({}))["count"], 0);
    run(&mut s, "view.slices.hide", json!({"hidden": true}));
    run(&mut s, "view.slices.lock", json!({"locked": false}));
    assert_eq!(run(&mut s, "select.object.slices", json!({}))["count"], 0);
    run(&mut s, "view.slices.hide", json!({"hidden": false}));
    assert_eq!(run(&mut s, "select.object.slices", json!({}))["count"], 1);
}

/// `slice.list` reports an object slice as selected while its object is selected: the Object →
/// Slice commands act on it then, and the canvas draws it as selected.
#[test]
fn slice_list_reports_an_object_slice_selected_with_its_object() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    run(&mut s, "object.slice.make", json!({}));
    rect(&mut s, 100.0, 100.0, 20.0, 20.0);
    run(&mut s, "object.slice.make", json!({}));
    let selected = |s: &mut Session| slices(s).into_iter().filter(|v| v["selected"] == true).map(|v| v["id"].clone()).collect::<Vec<_>>();
    run(&mut s, "select.set", json!({"ids": [a]}));
    assert_eq!(selected(&mut s), [json!(a)]);
    run(&mut s, "select.none", json!({}));
    assert!(selected(&mut s).is_empty());
}

#[test]
fn slices_survive_a_native_round_trip() {
    let mut s = session();
    user_slice(&mut s, [100.0, 100.0, 40.0, 40.0]);
    run(&mut s, "select.object.slices", json!({}));
    run(&mut s, "object.slice.options", json!({"kind": "noImage", "text": "x"}));
    let a = rect(&mut s, 10.0, 10.0, 10.0, 10.0);
    run(&mut s, "object.slice.make", json!({}));
    run(&mut s, "object.slice.options", json!({"alt": "logo"}));
    run(&mut s, "object.slice.clipToArtboard", json!({"on": false}));
    let doc = s.doc().unwrap().doc.clone();
    vectorcraft_testkit::invariants::check_native_roundtrip_exact(&doc).unwrap();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(back.slices, doc.slices);
    assert_eq!(back.node(NodeId(a)).unwrap().extras().slice.as_ref().unwrap().alt, "logo");
    assert!(!back.slices_clip_to_artboard);
    // A copy of an object slice is an object slice too, under its own id.
    run(&mut s, "select.set", json!({"ids": [a]}));
    run(&mut s, "edit.duplicate", json!({}));
    assert_eq!(s.doc().unwrap().doc.object_slices().len(), 2);
}

#[test]
fn slice_tool_gestures_are_one_undo_step_each() {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind as K, ToolKey};
    let mut s = session();
    let view = ViewInfo { smart_guides: false, ..Default::default() };
    let undo = |s: &Session| s.doc().unwrap().history.undo.len();
    let gesture = |s: &mut Session, pts: &[(K, f64, f64)]| {
        for (k, x, y) in pts {
            s.pointer(&PointerEvent::new(*k, *x, *y), view).unwrap();
        }
    };
    s.select_tool("slice", view).unwrap();
    let n = undo(&s);
    gesture(&mut s, &[(K::Down, 10.0, 10.0), (K::Drag, 50.0, 30.0), (K::Drag, 110.0, 60.0), (K::Up, 110.0, 60.0)]);
    assert_eq!(undo(&s), n + 1);
    let id = s.doc().unwrap().doc.slices[0].id;
    assert_eq!(s.doc().unwrap().doc.slices.len(), 1, "the previews left one slice");
    assert_eq!(rect_of(&mut s, id.0), [10.0, 10.0, 100.0, 50.0]);
    assert_eq!(s.doc().unwrap().selection.slices, vec![id]);

    // Slice Selection: drag the slice, then its bottom-right handle, then Delete.
    s.select_tool("sliceSelection", view).unwrap();
    gesture(&mut s, &[(K::Down, 20.0, 20.0), (K::Drag, 30.0, 40.0), (K::Drag, 40.0, 50.0), (K::Up, 40.0, 50.0)]);
    assert_eq!(undo(&s), n + 2);
    assert_eq!(rect_of(&mut s, id.0), [30.0, 40.0, 100.0, 50.0]);
    gesture(&mut s, &[(K::Down, 130.0, 90.0), (K::Drag, 140.0, 95.0), (K::Drag, 150.0, 100.0), (K::Up, 150.0, 100.0)]);
    assert_eq!(undo(&s), n + 3);
    assert_eq!(rect_of(&mut s, id.0), [30.0, 40.0, 120.0, 60.0]);
    assert!(s.tool_claims_key(ToolKey::Delete, view));
    s.tool_key(ToolKey::Delete, Mods::default(), view).unwrap();
    assert!(s.doc().unwrap().doc.slices.is_empty());
    assert_eq!(undo(&s), n + 4);
    // Undo walks the gestures back one at a time.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(rect_of(&mut s, id.0), [30.0, 40.0, 120.0, 60.0]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(rect_of(&mut s, id.0), [30.0, 40.0, 100.0, 50.0]);

    // An object slice moves its object; locked slices stay put.
    let a = rect(&mut s, 200.0, 200.0, 40.0, 40.0);
    run(&mut s, "object.slice.make", json!({}));
    gesture(&mut s, &[(K::Down, 220.0, 220.0), (K::Drag, 230.0, 220.0), (K::Drag, 240.0, 230.0), (K::Up, 240.0, 230.0)]);
    assert_eq!(s.doc().unwrap().doc.node(NodeId(a)).unwrap().geometric_bounds().unwrap().x0, 220.0);
    run(&mut s, "view.slices.lock", json!({"locked": true}));
    let before = undo(&s);
    gesture(&mut s, &[(K::Down, 250.0, 240.0), (K::Drag, 280.0, 260.0), (K::Up, 280.0, 260.0)]);
    assert_eq!(undo(&s), before);
    // The journal replays the gestures as commands.
    let journal: Vec<String> = s.journal.iter().map(|(c, _)| c.clone()).collect();
    for c in ["object.slice.create", "object.slice.move", "object.slice.setRect", "object.slice.delete", "object.slice.select"] {
        assert!(journal.iter().any(|j| j == c), "{c} journaled: {journal:?}");
    }
}

#[test]
fn slice_commands_check_their_params() {
    let mut s = session();
    assert!(s.execute("object.slice.create", &json!({"x": 0, "y": 0, "width": 0, "height": 10})).is_err(), "no area");
    assert!(s.execute("object.slice.create", &json!({"x": 1e9, "y": 0, "width": 10, "height": 10})).is_err(), "off the canvas");
    let id = run(&mut s, "object.slice.create", json!({"x": 50, "y": 60, "width": -20, "height": 10}))["id"].as_u64().unwrap();
    assert_eq!(rect_of(&mut s, id), [30.0, 60.0, 20.0, 10.0], "a negative size draws the other way");
    run(&mut s, "object.slice.setRect", json!({"id": id, "x": 0, "y": 0, "width": 5, "height": 5}));
    assert_eq!(rect_of(&mut s, id), [0.0, 0.0, 5.0, 5.0]);
    let a = rect(&mut s, 100.0, 100.0, 10.0, 10.0);
    run(&mut s, "object.slice.make", json!({}));
    assert!(
        s.execute("object.slice.setRect", &json!({"id": a, "x": 0, "y": 0, "width": 5, "height": 5})).is_err(),
        "object slices follow their object"
    );
    assert!(s.execute("object.slice.move", &json!({"slices": [12345], "dx": 1, "dy": 1})).is_err(), "not a slice");
    assert_eq!(run(&mut s, "object.slice.select", json!({"slices": [id, a]}))["selected"], json!([id, a]));
    assert_eq!(run(&mut s, "object.slice.select", json!({"slices": [a], "toggle": true}))["selected"], json!([id]));
    assert_eq!(run(&mut s, "object.slice.delete", json!({"slices": [a]}))["count"], 1);
    assert!(s.doc().unwrap().doc.node(NodeId(a)).is_some(), "deleting an object slice keeps its object");
    assert_eq!(run(&mut s, "object.slice.select", json!({}))["selected"], json!([]));
}
