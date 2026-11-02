//! Editing a repeat's source art (#5054): a double-click on a repeat with the Selection tool
//! enters isolation mode on it; there a double-click on its type edits the text, and the repeat
//! keeps its settings and draws the new text in every copy.

use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::*;

fn click(s: &mut Session, kind: PointerKind, x: f64, y: f64) -> Vec<UiRequest> {
    s.pointer(&PointerEvent::new(kind, x, y), ViewInfo { smart_guides: false, ..ViewInfo::default() }).unwrap()
}

#[test]
fn a_double_click_on_a_repeat_isolates_it_to_edit_its_text() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let text = NodeId(s.execute("text.create", &json!({"x": 50, "y": 60, "text": "Alice", "size": 24})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [text.0]})).unwrap();
    let repeat = NodeId(s.execute("object.repeat.grid", &json!({})).unwrap()["id"].as_u64().unwrap());
    let spec = |s: &Session| match &s.doc().unwrap().doc.node(repeat).unwrap().kind {
        NodeKind::Repeat(r) => serde_json::to_value(r).unwrap(),
        k => panic!("not a repeat: {k:?}"),
    };
    let before = spec(&s);
    s.select_tool("selection", ViewInfo::default()).unwrap();
    let b = s.doc().unwrap().doc.node(text).unwrap().geometric_bounds().unwrap();
    let (x, y) = (b.center().x, b.center().y);
    click(&mut s, PointerKind::DoubleClick, x, y);
    assert_eq!(s.doc().unwrap().isolation, Some(repeat), "the double-click isolates the repeat");
    // Inside it, a double-click on the type asks for the Type tool, which edits that text.
    let asked = click(&mut s, PointerKind::DoubleClick, x, y);
    assert!(asked.iter().any(|r| matches!(r, UiRequest::SwitchTool(t) if t == "type")), "{asked:?}");
    s.execute("text.setText", &json!({"id": text.0, "text": "Bob"})).unwrap();
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(text).unwrap().kind else { panic!("not type") };
    assert_eq!(t.plain_text(), "Bob");
    assert_eq!(s.doc().unwrap().doc.position(text).and_then(|(p, ..)| p), Some(repeat), "still the repeat's source");
    assert_eq!(spec(&s)["kind"], before["kind"], "the repeat keeps its settings (rows, columns, spacing)");
    assert!(before["kind"].is_object(), "the settings are read: {before}");
}

/// A text repeat isolated by a double-click on it: returns (session, text, repeat, point on the text).
fn isolated_text_repeat() -> (Session, NodeId, NodeId, (f64, f64)) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let text = NodeId(s.execute("text.create", &json!({"x": 50, "y": 60, "text": "Alice", "size": 24})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [text.0]})).unwrap();
    let repeat = NodeId(s.execute("object.repeat.grid", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.select_tool("selection", ViewInfo::default()).unwrap();
    let b = s.doc().unwrap().doc.node(text).unwrap().geometric_bounds().unwrap();
    let at = (b.center().x, b.center().y);
    click(&mut s, PointerKind::DoubleClick, at.0, at.1);
    assert_eq!(s.doc().unwrap().isolation, Some(repeat));
    (s, text, repeat, at)
}

fn plain_text(s: &Session, id: NodeId) -> String {
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("not type") };
    t.plain_text()
}

/// #5054 review: a double-click outside the repeat leaves isolation mode, and so does
/// `object.exitIsolation` (what Escape and Exit Isolation Mode run); the repeat stays a repeat.
#[test]
fn isolation_on_a_repeat_ends_by_double_click_outside_or_exit() {
    let (mut s, _, repeat, _) = isolated_text_repeat();
    click(&mut s, PointerKind::DoubleClick, 580.0, 380.0);
    assert_eq!(s.doc().unwrap().isolation, None, "a double-click on empty canvas leaves isolation mode");
    let (mut s, _, repeat2, at) = isolated_text_repeat();
    assert_eq!(repeat, repeat2);
    s.execute("object.exitIsolation", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().isolation, None, "Escape's command leaves isolation mode");
    assert!(matches!(s.doc().unwrap().doc.node(repeat).unwrap().kind, NodeKind::Repeat(_)), "still a repeat");
    // And a double-click on it isolates it again.
    click(&mut s, PointerKind::DoubleClick, at.0, at.1);
    assert_eq!(s.doc().unwrap().isolation, Some(repeat));
}

/// #5054 review: an edit made in a repeat's isolation mode undoes in one step and the repeat
/// keeps drawing its source; isolating it adds no undo step of its own.
#[test]
fn an_edit_inside_an_isolated_repeat_undoes() {
    let (mut s, text, repeat, _) = isolated_text_repeat();
    let steps = s.doc().unwrap().history.undo.len();
    s.execute("text.setText", &json!({"id": text.0, "text": "Bob"})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(plain_text(&s, text), "Alice");
    assert_eq!(s.doc().unwrap().doc.position(text).and_then(|(p, ..)| p), Some(repeat), "still the repeat's source");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(plain_text(&s, text), "Bob");
}

/// #5054 review: a repeat of a group isolates on a double-click, and a double-click on the group
/// inside it goes one level further, into the group, where its art can be selected alone.
#[test]
fn a_repeat_of_a_group_isolates_then_enters_the_group() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let a = s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap();
    let b = s.execute("shape.rectangle", &json!({"x": 80, "y": 40, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [group.0]})).unwrap();
    let repeat = NodeId(s.execute("object.repeat.grid", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.select_tool("selection", ViewInfo::default()).unwrap();
    click(&mut s, PointerKind::DoubleClick, 55.0, 55.0);
    assert_eq!(s.doc().unwrap().isolation, Some(repeat), "the first double-click isolates the repeat");
    click(&mut s, PointerKind::DoubleClick, 55.0, 55.0);
    assert_eq!(s.doc().unwrap().isolation, Some(group), "the second enters the group inside it");
    click(&mut s, PointerKind::Down, 55.0, 55.0);
    click(&mut s, PointerKind::Up, 55.0, 55.0);
    assert_eq!(s.doc().unwrap().selection.objects, [NodeId(a)], "inside the group a click selects one rectangle");
}

/// #5054: in a repeat's isolation mode the repeat itself draws as usual (its source and every
/// copy); only the art around it is dimmed, as for an isolated group.
#[test]
fn an_isolated_repeat_is_not_dimmed() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"ids": [r], "color": "#000000"})).unwrap();
    s.execute("select.set", &json!({"ids": [r]})).unwrap();
    let repeat = s.execute("object.repeat.grid", &json!({})).unwrap()["id"].as_u64().unwrap();
    let other = s.execute("shape.rectangle", &json!({"x": 150, "y": 150, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"ids": [other], "color": "#000000"})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let render = |iso: Option<u64>| {
        let opts = vectorcraft_render::RenderOptions { isolated: iso.map(NodeId), ..Default::default() };
        vectorcraft_render::Renderer::new().render(&doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts)
    };
    let alpha = |img: &vectorcraft_render::Rendered, x: usize, y: usize| img.pixels[(y * 200 + x) * 4 + 3];
    let plain = render(None);
    let iso = render(Some(repeat));
    assert_eq!(alpha(&plain, 20, 20), 255, "the source");
    assert_eq!(alpha(&iso, 20, 20), 255, "the isolated repeat's source is not dimmed");
    assert!(alpha(&iso, 165, 165) < 200, "the art around it is: {}", alpha(&iso, 165, 165));
}
