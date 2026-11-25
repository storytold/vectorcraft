//! `opacity` on `paint.setFill` and `paint.setStroke`: the opacity of the fill or stroke item the
//! paint goes to, and `fillOpacity` and `strokeOpacity` on `paint.proxies`.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::tooling::ViewInfo;
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 100, "height": 50})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn text(s: &mut Session) -> NodeId {
    let r = s.execute("text.create", &json!({"x": 10, "y": 150, "text": "Hi"})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn fill_opacity(s: &Session, id: NodeId) -> f32 {
    node(s, id).appearance.fill().unwrap().opacity
}

fn undo_labels(s: &Session) -> Vec<String> {
    s.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect()
}

fn proxies(s: &mut Session) -> Value {
    s.execute("paint.proxies", &json!({})).unwrap()
}

/// `paint.setFill {opacity}` alone sets the opacity of the selected object's top fill item, as one
/// undo step named Opacity, and leaves its paint, its stroke and the object's own opacity as they
/// were; `paint.setStroke {opacity}` does the same for the top stroke, and `paint.proxies` answers
/// both.
#[test]
fn opacity_alone_sets_the_fill_or_stroke_item() {
    let mut s = session();
    let id = rect(&mut s, 10.0);
    select(&mut s, &[id]);
    let before = node(&s, id);
    let undo = undo_labels(&s).len();
    s.execute("paint.setFill", &json!({"opacity": 40})).unwrap();
    let n = node(&s, id);
    assert!((fill_opacity(&s, id) - 0.4).abs() < 1e-6);
    assert_eq!(n.appearance.fill_paint(), before.appearance.fill_paint());
    assert_eq!(n.appearance.stroke().unwrap().opacity, 1.0);
    assert_eq!(n.opacity, 1.0);
    let labels = undo_labels(&s);
    assert_eq!((labels.len(), labels.last().map(String::as_str)), (undo + 1, Some("Opacity")));
    assert_eq!(proxies(&mut s)["fillOpacity"], 40.0);
    assert_eq!(proxies(&mut s)["strokeOpacity"], 100.0);
    // The defaults for new art keep their paint.
    assert_eq!(s.paint.fill, Paint::solid(Color::WHITE));

    s.execute("paint.setStroke", &json!({"opacity": 25})).unwrap();
    assert!((node(&s, id).appearance.stroke().unwrap().opacity - 0.25).abs() < 1e-6);
    assert!((fill_opacity(&s, id) - 0.4).abs() < 1e-6);
    assert_eq!(proxies(&mut s)["strokeOpacity"], 25.0);
}

/// A selected group stands for its contents: every member's fill takes the opacity, as one undo step.
#[test]
fn a_group_sets_every_member_in_one_step() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 200.0));
    select(&mut s, &[a, b]);
    s.execute("object.group", &json!({})).unwrap();
    let undo = undo_labels(&s).len();
    s.execute("paint.setFill", &json!({"opacity": 60})).unwrap();
    assert!((fill_opacity(&s, a) - 0.6).abs() < 1e-6 && (fill_opacity(&s, b) - 0.6).abs() < 1e-6);
    assert_eq!(undo_labels(&s).len(), undo + 1);
}

/// A group with a fill of its own: without an active Appearance row, `paint.proxies` answers the
/// opacity of the contents' fills, which `paint.setFill {opacity}` sets, not the group's own fill.
#[test]
fn a_group_with_its_own_fill_answers_for_its_contents() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 200.0));
    select(&mut s, &[a, b]);
    s.execute("object.group", &json!({})).unwrap();
    let g = s.doc().unwrap().selection.subjects()[0];
    select(&mut s, &[g]);
    s.execute("appearance.addFill", &json!({})).unwrap();
    let own = node(&s, g).appearance.items.iter().position(|it| it.is_fill()).unwrap();
    s.execute("paint.setFill", &json!({"item": own, "opacity": 30})).unwrap();
    select(&mut s, &[g]);
    assert_eq!(proxies(&mut s)["fillOpacity"], 100.0);
    s.execute("paint.setFill", &json!({"opacity": 60})).unwrap();
    assert_eq!(proxies(&mut s)["fillOpacity"], 60.0);
    assert!((node(&s, g).appearance.items[own].opacity() - 0.3).abs() < 1e-6, "the group's own fill keeps its opacity");
}

/// A targeted layer without an active Appearance row: `paint.proxies` answers the opacity of the
/// layer's editable art, which `paint.setFill {opacity}` sets, not that of a locked object before it.
#[test]
fn a_targeted_layer_answers_for_its_editable_art() {
    let mut s = session();
    let (locked, open) = (rect(&mut s, 10.0), rect(&mut s, 200.0));
    s.execute("paint.setFill", &json!({"ids": [locked.0], "opacity": 20})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [open.0], "opacity": 80})).unwrap();
    s.execute("object.lock", &json!({"ids": [locked.0]})).unwrap();
    let layer = s.doc().unwrap().doc.layers[0].id;
    s.execute("layer.target", &json!({"id": layer.0})).unwrap();
    assert_eq!(proxies(&mut s)["fillOpacity"], 80.0);
    s.execute("paint.setFill", &json!({"opacity": 40})).unwrap();
    assert!((fill_opacity(&s, open) - 0.4).abs() < 1e-6);
    assert!((fill_opacity(&s, locked) - 0.2).abs() < 1e-6, "the locked object keeps its opacity");
    assert_eq!(proxies(&mut s)["fillOpacity"], 40.0);
}

/// With the Appearance panel's lower fill row active, that row takes the opacity and the top fill
/// keeps its own.
#[test]
fn the_active_appearance_row_takes_the_opacity() {
    let mut s = session();
    let id = rect(&mut s, 10.0);
    select(&mut s, &[id]);
    s.execute("appearance.addFill", &json!({})).unwrap(); // [Fill, Stroke, Fill]
    s.execute("appearance.setActiveItem", &json!({"index": 0})).unwrap();
    s.execute("paint.setFill", &json!({"opacity": 30})).unwrap();
    let ap = node(&s, id).appearance;
    assert!((ap.items[0].opacity() - 0.3).abs() < 1e-6);
    assert_eq!(ap.items[2].opacity(), 1.0);
    assert_eq!(proxies(&mut s)["fillOpacity"], 30.0);
}

/// With a path of a compound path selected and its fill row active, the command edits the compound
/// path's fill, and `paint.proxies` answers that fill's opacity, not the member's own.
#[test]
fn an_active_row_of_a_compound_member_answers_for_its_compound() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 200.0));
    select(&mut s, &[a, b]);
    let compound = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"ids": [compound.0], "opacity": 40})).unwrap();
    // A fill row of the member's own, at 100%.
    s.execute("appearance.addFill", &json!({"ids": [a.0]})).unwrap();
    assert_eq!(fill_opacity(&s, a), 1.0);
    select(&mut s, &[a]);
    let row = node(&s, a).appearance.items.iter().position(|it| it.is_fill()).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": row})).unwrap();
    assert_eq!(proxies(&mut s)["fillOpacity"], 40.0);
    s.execute("paint.setFill", &json!({"opacity": 70})).unwrap();
    assert!((fill_opacity(&s, compound) - 0.7).abs() < 1e-6);
    assert_eq!(proxies(&mut s)["fillOpacity"], 70.0);

    // A fill row of the member's at an index that is not a fill in the compound path: the command
    // edits the compound path's top fill, which the proxy answers.
    s.execute("appearance.addFill", &json!({"ids": [a.0]})).unwrap();
    let (member, outer) = (node(&s, a).appearance, node(&s, compound).appearance);
    let row = (0..member.items.len()).find(|i| member.items[*i].is_fill() && !outer.items.get(*i).is_some_and(|it| it.is_fill())).unwrap();
    select(&mut s, &[a]);
    s.execute("appearance.setActiveItem", &json!({"index": row})).unwrap();
    assert_eq!(proxies(&mut s)["fillOpacity"], 70.0);
    s.execute("paint.setFill", &json!({"opacity": 20})).unwrap();
    assert!((fill_opacity(&s, compound) - 0.2).abs() < 1e-6);
    assert_eq!(proxies(&mut s)["fillOpacity"], 20.0);
}

/// A color and an opacity change together in one undo step, and the same call again records no
/// step.
#[test]
fn color_and_opacity_are_one_step_and_a_repeat_records_none() {
    let mut s = session();
    let id = rect(&mut s, 10.0);
    select(&mut s, &[id]);
    let undo = undo_labels(&s).len();
    let p = json!({"color": "#ff0000", "opacity": 50});
    s.execute("paint.setFill", &p).unwrap();
    assert_eq!(node(&s, id).appearance.fill_paint(), Paint::solid(Color::rgb8(255, 0, 0)));
    assert!((fill_opacity(&s, id) - 0.5).abs() < 1e-6);
    assert_eq!(undo_labels(&s).len(), undo + 1);
    assert_eq!(undo_labels(&s).last().map(String::as_str), Some("Fill Color"));
    s.execute("paint.setFill", &p).unwrap();
    s.execute("paint.setFill", &json!({"opacity": 50})).unwrap();
    assert_eq!(undo_labels(&s).len(), undo + 1);
}

/// Bad values, nothing selected, and characters selected with the Type tool answer errors and leave
/// the document and the defaults for new art as they were.
#[test]
fn errors_leave_the_document_and_defaults_alone() {
    let mut s = session();
    let id = rect(&mut s, 10.0);
    select(&mut s, &[id]);
    for bad in [json!(101), json!(-1), json!("50"), json!(true)] {
        let e = s.execute("paint.setFill", &json!({"opacity": bad})).unwrap_err();
        assert!(e.to_string().contains("`opacity` must be a number from 0 to 100"), "{bad}: {e}");
    }
    let e = s.execute("paint.setFill", &json!({})).unwrap_err();
    assert!(e.to_string().contains("give color, none, swatch, gradient or opacity"), "{e}");
    // An opacity alone places no gradient across the selection.
    let doc = s.doc().unwrap().doc.clone();
    for (span, says) in [(json!("selection"), "give `gradient`"), (json!("all"), "span must be \"object\" or \"selection\"")] {
        let e = s.execute("paint.setFill", &json!({"opacity": 40, "span": span})).unwrap_err();
        assert!(e.to_string().contains(says), "{span}: {e}");
    }
    assert_eq!(*s.doc().unwrap().doc, *doc);
    s.execute("paint.setFill", &json!({"opacity": 40, "span": "object"})).unwrap();
    assert!((fill_opacity(&s, id) - 0.4).abs() < 1e-6);
    s.execute("select.none", &json!({})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    for p in [json!({"opacity": 40}), json!({"color": "#ff0000", "opacity": 40})] {
        let e = s.execute("paint.setFill", &p).unwrap_err();
        assert!(e.to_string().contains("select objects or give ids"), "{p}: {e}");
    }
    assert_eq!(s.paint.fill, Paint::solid(Color::WHITE));
    assert_eq!(*s.doc().unwrap().doc, *doc);

    // Characters selected with the Type tool.
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, 50.0, 250.0), v).unwrap();
    }
    s.tool_text("Sample", v).unwrap();
    s.set_tool_option("select", &json!({"start": 0, "end": 3}));
    let doc = s.doc().unwrap().doc.clone();
    for p in [json!({"opacity": 40}), json!({"color": "#ff0000", "opacity": 40})] {
        let e = s.execute("paint.setFill", &p).unwrap_err();
        assert!(e.to_string().contains("type characters have no opacity of their own"), "{p}: {e}");
    }
    assert_eq!(*s.doc().unwrap().doc, *doc);
    assert_eq!(proxies(&mut s)["fillOpacity"], Value::Null);
}

/// Select characters of `t` with the Type tool.
fn select_characters(s: &mut Session, t: NodeId) {
    s.select_tool("type", ViewInfo::default()).unwrap();
    s.set_tool_option("textSelection", &json!({"editing": t.0, "anchor": 0, "caret": 2}));
}

/// While the Type tool has characters selected, `paint.proxies` answers no opacity even when the
/// type object has a fill row of its own that the Appearance panel has active, since
/// `paint.setFill {opacity}` would give the opacity to the characters.
#[test]
fn with_characters_selected_the_proxy_answers_no_opacity() {
    let mut s = session();
    let t = text(&mut s);
    select(&mut s, &[t]);
    s.execute("appearance.addFill", &json!({})).unwrap();
    let row = node(&s, t).appearance.items.iter().position(|it| it.is_fill()).unwrap();
    s.execute("appearance.setActiveItem", &json!({"index": row})).unwrap();
    assert_eq!(proxies(&mut s)["fillOpacity"], 100.0, "the type object's own fill row");
    select_characters(&mut s, t);
    let e = s.execute("paint.setFill", &json!({"opacity": 40})).unwrap_err();
    assert!(e.to_string().contains("type characters have no opacity of their own"), "{e}");
    assert_eq!(proxies(&mut s)["fillOpacity"], Value::Null);
}

/// While the Type tool has characters selected, an object named by `id` takes the opacity and the
/// paint, and the characters keep theirs.
#[test]
fn with_characters_selected_an_id_takes_the_opacity() {
    let mut s = session();
    let r = rect(&mut s, 10.0);
    let t = text(&mut s);
    select(&mut s, &[t]);
    select_characters(&mut s, t);
    let before = node(&s, t);
    s.execute("paint.setFill", &json!({"id": r.0, "opacity": 40})).unwrap();
    assert!((fill_opacity(&s, r) - 0.4).abs() < 1e-6);
    s.execute("paint.setStroke", &json!({"id": r.0, "color": "#ff0000", "opacity": 30})).unwrap();
    let stroke = node(&s, r).appearance.stroke().unwrap().clone();
    assert!((stroke.opacity - 0.3).abs() < 1e-6);
    assert_eq!(stroke.paint, Paint::solid(Color::rgb8(255, 0, 0)));
    assert_eq!(node(&s, t), before, "the characters keep their paint");
}

/// While the Type tool has characters selected, a null `id` or `ids` names no object: the paint goes
/// to the selected characters only, as without the key.
#[test]
fn with_characters_selected_a_null_id_paints_the_characters() {
    let runs = |key: Option<&str>| {
        let mut s = session();
        let r = s.execute("text.create", &json!({"x": 10, "y": 150, "text": "Hello"})).unwrap();
        let t = NodeId(r["id"].as_u64().unwrap());
        select(&mut s, &[t]);
        select_characters(&mut s, t);
        let mut p = json!({"color": "#ff0000"});
        if let Some(key) = key {
            p[key] = Value::Null;
        }
        s.execute("paint.setFill", &p).unwrap();
        match &node(&s, t).kind {
            NodeKind::Text(t) => t.runs.clone(),
            _ => panic!("not type"),
        }
    };
    let characters = runs(None);
    assert!(characters.len() > 1, "only the selected characters take the paint");
    for key in ["id", "ids"] {
        assert_eq!(runs(Some(key)), characters, "{key}: null");
    }
}

/// An opacity for characters selected while the Type tool is typing is refused before the typing
/// session ends: the session goes on, and no undo step is recorded.
#[test]
fn an_opacity_for_characters_leaves_the_typing_session_running() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, 50.0, 250.0), v).unwrap();
    }
    s.tool_text("Hello", v).unwrap();
    s.set_tool_option("select", &json!({"start": 0, "end": 2}));
    assert!(crate::cmd::typing_in_progress(&s));
    let undo = undo_labels(&s).len();
    let e = s.execute("paint.setFill", &json!({"opacity": 40})).unwrap_err();
    assert!(e.to_string().contains("type characters have no opacity of their own"), "{e}");
    assert!(crate::cmd::typing_in_progress(&s), "the typing session goes on");
    assert_eq!(undo_labels(&s).len(), undo, "no undo step");
}

/// With an opacity alone, the reply counts only the objects that took it: a selected object without
/// that fill item is left out, and one already at that opacity still counts. With a paint, every
/// target counts.
#[test]
fn an_opacity_alone_counts_the_objects_that_took_it() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 200.0));
    select(&mut s, &[a]);
    s.execute("appearance.removeItem", &json!({"index": 0})).unwrap();
    assert!(node(&s, a).appearance.fill().is_none());
    select(&mut s, &[a, b]);
    assert_eq!(s.execute("paint.setFill", &json!({"opacity": 40})).unwrap()["objects"], 1);
    assert_eq!(s.execute("paint.setFill", &json!({"opacity": 40})).unwrap()["objects"], 1, "already at 40%");
    assert_eq!(s.execute("paint.setFill", &json!({"color": "#ff0000", "opacity": 40})).unwrap()["objects"], 2);
}

/// With a layer targeted, `paint.proxies` reads the art the command edits: a locked object's fill
/// row does not count, so the proxy answers null when the edit finds no fill to take the opacity.
#[test]
fn a_targeted_layers_proxy_leaves_out_locked_art() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 200.0));
    select(&mut s, &[b]);
    s.execute("appearance.removeItem", &json!({"index": 0})).unwrap();
    select(&mut s, &[a]);
    s.execute("object.lock", &json!({})).unwrap();
    let layer = s.doc().unwrap().doc.layers[0].id;
    s.execute("layer.target", &json!({"id": layer.0})).unwrap();
    assert_eq!(proxies(&mut s)["fillOpacity"], Value::Null);
    let e = s.execute("paint.setFill", &json!({"opacity": 40})).unwrap_err();
    assert!(e.to_string().contains("no target has that fill or stroke to take the opacity"), "{e}");
    s.execute("object.unlockAll", &json!({})).unwrap();
    s.execute("layer.target", &json!({"id": layer.0})).unwrap();
    assert_eq!(proxies(&mut s)["fillOpacity"], 100.0, "the unlocked fill counts");
}

/// A type object whose characters take the paint is left alone beside a rectangle that takes the
/// opacity, with no error; alone, it answers the characters error.
#[test]
fn type_beside_other_art_is_left_alone() {
    let mut s = session();
    let r = rect(&mut s, 10.0);
    let t = text(&mut s);
    let runs = |s: &Session| match &node(s, t).kind {
        NodeKind::Text(t) => t.runs.clone(),
        _ => panic!("not type"),
    };
    let before = (node(&s, t), runs(&s));
    select(&mut s, &[r, t]);
    let out = s.execute("paint.setFill", &json!({"opacity": 40})).unwrap();
    assert_eq!(out["objects"], 1, "only the rectangle took the opacity");
    assert!((fill_opacity(&s, r) - 0.4).abs() < 1e-6);
    assert_eq!((node(&s, t), runs(&s)), before);
    select(&mut s, &[t]);
    assert_eq!(proxies(&mut s)["fillOpacity"], Value::Null);
    let e = s.execute("paint.setFill", &json!({"opacity": 40})).unwrap_err();
    assert!(e.to_string().contains("type characters have no opacity of their own"), "{e}");
    assert_eq!((node(&s, t), runs(&s)), before);
}

/// A color with an opacity on type alone answers the characters error and takes the color back too.
#[test]
fn type_alone_takes_back_the_paint_given_with_an_opacity() {
    let mut s = session();
    let t = text(&mut s);
    select(&mut s, &[t]);
    let doc = s.doc().unwrap().doc.clone();
    let undo = undo_labels(&s).len();
    let e = s.execute("paint.setFill", &json!({"color": "#ff0000", "opacity": 40})).unwrap_err();
    assert!(e.to_string().contains("type characters have no opacity of their own"), "{e}");
    assert_eq!(*s.doc().unwrap().doc, *doc);
    assert_eq!(undo_labels(&s).len(), undo);
    assert_eq!(s.paint.fill, Paint::solid(Color::WHITE), "the defaults keep their paint");
}

/// An opacity that no target takes (the only selected object has no fill item) is an error, with or
/// without a paint, and leaves the document as it was; `paint.proxies` answers null for it, and
/// beside an object with a fill item it answers that object's opacity.
#[test]
fn an_opacity_no_target_takes_is_an_error() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 200.0));
    select(&mut s, &[a]);
    s.execute("appearance.removeItem", &json!({"index": 0})).unwrap();
    assert!(node(&s, a).appearance.fill().is_none());
    assert_eq!(proxies(&mut s)["fillOpacity"], Value::Null);
    let doc = s.doc().unwrap().doc.clone();
    let e = s.execute("paint.setFill", &json!({"opacity": 40})).unwrap_err();
    assert!(e.to_string().contains("no target has that fill or stroke to take the opacity"), "{e}");
    assert_eq!(*s.doc().unwrap().doc, *doc);
    // A paint given with the opacity creates the fill, which takes the opacity.
    s.execute("paint.setFill", &json!({"color": "#ff0000", "opacity": 40})).unwrap();
    assert!((fill_opacity(&s, a) - 0.4).abs() < 1e-6);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(*s.doc().unwrap().doc, *doc);

    select(&mut s, &[b]);
    s.execute("paint.setFill", &json!({"opacity": 70})).unwrap();
    select(&mut s, &[a, b]);
    assert_eq!(proxies(&mut s)["fillOpacity"], 70.0, "the first selected object with a fill item answers");
    let out = s.execute("paint.setFill", &json!({"opacity": 20})).unwrap();
    assert_eq!(out["objects"], 1, "the object without a fill item is not counted");
    assert!((fill_opacity(&s, b) - 0.2).abs() < 1e-6);
    assert!(node(&s, a).appearance.fill().is_none());
}

/// An object selected in a dynamic symbol instance has no opacity of its own: `opacity` is an error
/// that leaves the document alone, and `paint.proxies` answers null.
#[test]
fn an_object_in_a_symbol_instance_refuses_an_opacity() {
    let mut s = session();
    let id = rect(&mut s, 10.0);
    select(&mut s, &[id]);
    let inst = s.execute("symbol.new", &json!({"name": "One", "ids": [id.0], "type": "dynamic"})).unwrap()["id"].clone();
    s.execute("symbol.instance.selectPart", &json!({"id": inst, "path": []})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    for p in [json!({"opacity": 40}), json!({"color": "#ff0000", "opacity": 40})] {
        let e = s.execute("paint.setFill", &p).unwrap_err();
        assert!(e.to_string().contains("an object selected in a symbol instance has no fill or stroke opacity of its own"), "{p}: {e}");
    }
    assert_eq!(*s.doc().unwrap().doc, *doc);
    assert_eq!(proxies(&mut s)["fillOpacity"], Value::Null);
}

/// With an object selected in a dynamic symbol instance, a bad `target` is refused whether `id` and
/// `ids` are left out or given as null, and leaves the document alone; a good one with a null `id`
/// still paints the object.
#[test]
fn an_object_in_a_symbol_instance_refuses_a_bad_target() {
    let mut s = session();
    let id = rect(&mut s, 10.0);
    select(&mut s, &[id]);
    let inst = s.execute("symbol.new", &json!({"name": "One", "ids": [id.0], "type": "dynamic"})).unwrap()["id"].clone();
    s.execute("symbol.instance.selectPart", &json!({"id": inst, "path": []})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    for p in [
        json!({"color": "#ff0000", "target": "content"}),
        json!({"color": "#ff0000", "target": "content", "id": null}),
        json!({"color": "#ff0000", "target": "content", "ids": null}),
    ] {
        let e = s.execute("paint.setFill", &p).unwrap_err();
        assert!(e.to_string().contains("`target` must be"), "{p}: {e}");
        assert_eq!(*s.doc().unwrap().doc, *doc, "{p}");
    }
    assert_eq!(s.execute("paint.setFill", &json!({"color": "#ff0000", "target": "object", "id": null})).unwrap()["objects"], 1);
    assert_ne!(*s.doc().unwrap().doc, *doc);
}

/// A bad `target` is refused when `id` or `ids` names an object, and a bad `target` or `aligned`
/// while the Type tool has characters selected; each leaves the document alone. A good `target`
/// with an `id` still paints the object.
#[test]
fn a_bad_target_is_refused_with_an_id_or_selected_characters() {
    let mut s = session();
    let r = rect(&mut s, 10.0);
    select(&mut s, &[r]);
    let doc = s.doc().unwrap().doc.clone();
    for cmd in ["paint.setFill", "paint.setStroke"] {
        for p in [
            json!({"color": "#ff0000", "target": "content", "id": r.0}),
            json!({"color": "#ff0000", "target": "content", "ids": [r.0]}),
            json!({"opacity": 40, "target": "content", "id": r.0}),
            json!({"opacity": 40, "target": "content", "ids": [r.0]}),
        ] {
            let e = s.execute(cmd, &p).unwrap_err();
            assert!(e.to_string().contains("`target` must be"), "{cmd} {p}: {e}");
            assert_eq!(*s.doc().unwrap().doc, *doc, "{cmd} {p}");
        }
    }
    assert_eq!(s.execute("paint.setFill", &json!({"color": "#ff0000", "target": "object", "id": r.0})).unwrap()["objects"], 1);
    assert_ne!(*s.doc().unwrap().doc, *doc);
    let t = text(&mut s);
    select(&mut s, &[t]);
    select_characters(&mut s, t);
    let doc = s.doc().unwrap().doc.clone();
    for p in [json!({"color": "#ff0000", "target": "content"}), json!({"color": "#ff0000", "aligned": "yes"})] {
        assert!(s.execute("paint.setFill", &p).is_err(), "{p}");
        assert_eq!(*s.doc().unwrap().doc, *doc, "{p}");
    }
}

/// A gradient mesh point picked with the Mesh tool takes a fill opacity itself, and the proxy shows
/// it; the object's fill item and the other points stay as they were.
#[test]
fn a_picked_mesh_point_takes_the_fill_opacity() {
    let mut s = session();
    let m = rect(&mut s, 10.0);
    select(&mut s, &[m]);
    s.execute("object.mesh.create", &json!({"rows": 2, "cols": 2})).unwrap();
    select(&mut s, &[m]);
    let v = ViewInfo::default();
    let NodeKind::Mesh(g) = &node(&s, m).kind else { panic!("not a mesh") };
    let q = g.points[4].p;
    s.select_tool("mesh", v).unwrap();
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, q.x, q.y), v).unwrap();
    }
    s.execute("paint.setFill", &json!({"opacity": 30})).unwrap();
    let NodeKind::Mesh(g) = &node(&s, m).kind else { panic!("not a mesh") };
    let o: Vec<f32> = g.points.iter().map(|p| p.opacity).collect();
    assert!((o[4] - 0.3).abs() < 1e-6, "{o:?}");
    assert!(o.iter().enumerate().all(|(i, a)| i == 4 || *a == 1.0), "{o:?}");
    assert_eq!(proxies(&mut s)["fillOpacity"], 30.0);
}

/// A null `id` or `ids` names no object, so while the Type tool has characters selected the paint
/// goes to those characters only, and the rest of the type keeps its paint.
#[test]
fn a_null_id_leaves_the_paint_to_the_selected_characters() {
    for key in ["id", "ids"] {
        let mut s = session();
        let t = text(&mut s);
        select(&mut s, &[t]);
        s.select_tool("type", ViewInfo::default()).unwrap();
        s.set_tool_option("textSelection", &json!({"editing": t.0, "anchor": 0, "caret": 1}));
        s.execute("paint.setFill", &json!({"color": "#ff0000", key: null})).unwrap();
        let NodeKind::Text(tx) = &node(&s, t).kind else { panic!("not type") };
        let fills: Vec<Paint> = tx.runs.iter().map(|r| r.style.fill.clone()).collect();
        assert_eq!(fills.first(), Some(&Paint::solid(Color::rgb8(255, 0, 0))), "{key}: {fills:?}");
        assert!(fills.len() > 1 && fills[1..].iter().all(|f| *f != Paint::solid(Color::rgb8(255, 0, 0))), "{key}: {fills:?}");
        let e = s.execute("paint.setFill", &json!({"opacity": 40, key: null})).unwrap_err();
        assert!(e.to_string().contains("type characters have no opacity of their own"), "{key}: {e}");
    }
}

/// A selected Live Paint group stands for its faces in a fill opacity and for its edges in a stroke
/// opacity, and `paint.proxies` answers from those same parts.
#[test]
fn a_live_paint_group_answers_from_the_parts_it_sets() {
    let mut s = session();
    let id = rect(&mut s, 10.0);
    s.execute("livePaint.make", &json!({"ids": [id.0]})).unwrap();
    let g = s.doc().unwrap().selection.objects[0];
    select(&mut s, &[g]);
    s.execute("paint.setStroke", &json!({"opacity": 30})).unwrap();
    assert_eq!(proxies(&mut s)["strokeOpacity"], 30.0);
    s.execute("paint.setFill", &json!({"opacity": 40})).unwrap();
    assert_eq!(proxies(&mut s)["fillOpacity"], 40.0);
    assert_eq!(proxies(&mut s)["strokeOpacity"], 30.0);
}

/// While the Live Paint Bucket is the active tool, a fill or stroke chosen goes to the bucket, so
/// `paint.proxies` answers no opacity for the selection and an opacity is an error that leaves the
/// document as it was.
#[test]
fn the_live_paint_bucket_answers_no_opacity() {
    let mut s = session();
    let id = rect(&mut s, 10.0);
    select(&mut s, &[id]);
    assert_eq!(proxies(&mut s)["fillOpacity"], 100.0);
    s.select_tool("livePaintBucket", ViewInfo::default()).unwrap();
    assert_eq!(proxies(&mut s)["fillOpacity"], Value::Null);
    assert_eq!(proxies(&mut s)["strokeOpacity"], Value::Null);
    let doc = s.doc().unwrap().doc.clone();
    assert!(s.execute("paint.setFill", &json!({"opacity": 40})).is_err());
    assert_eq!(*s.doc().unwrap().doc, *doc);
}

/// The Eyedropper armed to sample into the Live Paint Bucket's gradient works for the bucket, so
/// the proxies answer no opacity for the selection then too, as they show no paint of its own.
#[test]
fn the_eyedropper_armed_for_the_bucket_answers_no_opacity() {
    let mut s = session();
    let id = rect(&mut s, 10.0);
    select(&mut s, &[id]);
    s.select_tool("eyedropper", ViewInfo::default()).unwrap();
    assert_eq!(proxies(&mut s)["fillOpacity"], 100.0, "the Eyedropper alone shows the selection's");
    s.set_tool_option("stop", &json!("livePaintBucket"));
    assert!(s.bucket_paints());
    assert_eq!(proxies(&mut s)["fillOpacity"], Value::Null);
    assert_eq!(proxies(&mut s)["strokeOpacity"], Value::Null);
}

/// A `swatch` that is no name does not turn a request into an opacity alone: it is an error and
/// leaves the document as it was.
#[test]
fn a_swatch_that_is_no_name_is_refused_with_an_opacity() {
    let mut s = session();
    let id = rect(&mut s, 10.0);
    select(&mut s, &[id]);
    let doc = s.doc().unwrap().doc.clone();
    for cmd in ["paint.setFill", "paint.setStroke"] {
        for swatch in [json!(5), json!(true), json!(["a"]), json!({})] {
            assert!(s.execute(cmd, &json!({"swatch": swatch, "opacity": 40})).is_err(), "{cmd} {swatch}");
        }
        // A null swatch names none, so the opacity goes alone.
        s.execute(cmd, &json!({"swatch": null, "opacity": 40})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert_eq!(*s.doc().unwrap().doc, *doc);
}
