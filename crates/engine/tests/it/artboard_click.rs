//! Artboard clicks fit the object without changing its art or selection, and are one undo step.
//! The second click of a double-click on an object does not fit another artboard, and that
//! double-click does not open Artboard Options. Empty artboard space still does.

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::{Session, UiRequest, ViewInfo};
use vectorcraft_geom::Rect;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

fn fixture(x: f64, y: f64) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 500, "height": 500})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 100, "height": 60})).unwrap()["id"].as_u64().unwrap();
    s.execute("stroke.set", &json!({"weight": 10})).unwrap();
    s.select_tool("artboard", ViewInfo::default()).unwrap();
    (s, NodeId(id))
}

fn pointer(s: &mut Session, kind: PointerKind, x: f64, y: f64) {
    s.pointer(&PointerEvent::new(kind, x, y), ViewInfo::default()).unwrap();
}

fn pointer_reqs(s: &mut Session, kind: PointerKind, x: f64, y: f64) -> Vec<UiRequest> {
    s.pointer(&PointerEvent::new(kind, x, y), ViewInfo::default()).unwrap()
}

fn is_dialog(r: &UiRequest) -> bool {
    matches!(r, UiRequest::Dialog(_, _))
}

/// The two clicks of a double-click, then the double-click. `suppress` is what the canvas does on
/// the second release, so that click does not fit a second artboard.
fn double_click(s: &mut Session, x: f64, y: f64, suppress: bool) -> Vec<UiRequest> {
    pointer(s, PointerKind::Down, x, y);
    pointer(s, PointerKind::Up, x, y);
    pointer(s, PointerKind::Down, x, y);
    if suppress {
        s.set_tool_option("suppressFitClick", &json!(true));
    }
    pointer(s, PointerKind::Up, x, y);
    pointer_reqs(s, PointerKind::DoubleClick, x, y)
}

#[test]
fn clicks_inside_and_outside_an_artboard_fit_the_visible_object_in_one_undo_step() {
    for (x, y) in [(100.0, 100.0), (650.0, 700.0)] {
        let (mut s, id) = fixture(x, y);
        let before = s.doc().unwrap().doc.clone();
        let sel = s.doc().unwrap().selection.clone();
        let history = s.doc().unwrap().history.undo.len();
        let at = (x + 40.0, y + 25.0);
        pointer(&mut s, PointerKind::Down, at.0, at.1);
        assert_eq!(s.doc().unwrap().doc.artboards.len(), 1, "a press alone does not edit the document");
        pointer(&mut s, PointerKind::Drag, at.0 + 1.0, at.1);
        pointer(&mut s, PointerKind::Up, at.0 + 1.0, at.1);
        let st = s.doc().unwrap();
        assert_eq!(st.doc.artboards.len(), 2);
        let rect = Rect::new(x - 5.0, y - 5.0, x + 105.0, y + 65.0);
        assert_eq!(st.doc.artboards[1].rect, rect, "includes the visible stroke, like Fit to Selected Art");
        assert_eq!(st.doc.bounds_of(&[id], true), Some(rect));
        assert_eq!(json!(st.doc.layers), json!(before.layers));
        assert_eq!(json!(st.selection), json!(sel));
        assert_eq!(st.history.undo.len(), history + 1);
        assert_eq!(s.tool_options()["active"], 1);
        pointer(&mut s, PointerKind::Up, at.0, at.1);
        assert_eq!(s.doc().unwrap().doc.artboards.len(), 2, "one release creates once");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(json!(s.doc().unwrap().doc), json!(before));
        assert_eq!(json!(s.doc().unwrap().selection), json!(sel));
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc.artboards[1].rect, rect);
    }
}

#[test]
fn the_topmost_group_is_fitted_instead_of_the_selection_or_its_clicked_child() {
    let (mut s, bottom) = fixture(100.0, 100.0);
    let a = s.execute("shape.rectangle", &json!({"x": 120, "y": 110, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap();
    let b = s.execute("shape.rectangle", &json!({"x": 250, "y": 120, "width": 40, "height": 60})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let group = s.doc().unwrap().selection.objects[0];
    let expected = s.doc().unwrap().doc.bounds_of(&[group], true).unwrap();
    s.execute("select.set", &json!({"ids": [bottom.0]})).unwrap();
    let layers = json!(s.doc().unwrap().doc.layers);
    pointer(&mut s, PointerKind::Down, 135.0, 125.0);
    pointer(&mut s, PointerKind::Up, 135.0, 125.0);
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 2);
    assert_eq!(s.doc().unwrap().doc.artboards[1].rect, expected);
    assert_eq!(s.doc().unwrap().selection.objects, vec![bottom]);
    assert_eq!(json!(s.doc().unwrap().doc.layers), layers);
}

#[test]
fn cancelled_clicks_do_not_create_an_artboard_or_leak_into_the_next_gesture() {
    for switch in [false, true] {
        let (mut s, _) = fixture(100.0, 100.0);
        let before = s.doc().unwrap().doc.clone();
        let history = s.doc().unwrap().history.undo.len();
        pointer(&mut s, PointerKind::Down, 140.0, 125.0);
        if switch {
            s.select_tool("selection", ViewInfo::default()).unwrap();
            s.select_tool("artboard", ViewInfo::default()).unwrap();
        } else {
            s.tool_key(ToolKey::Escape, Mods::default(), ViewInfo::default()).unwrap();
            s.select_tool("artboard", ViewInfo::default()).unwrap();
        }
        pointer(&mut s, PointerKind::Up, 140.0, 125.0);
        assert_eq!(json!(s.doc().unwrap().doc), json!(before));
        assert_eq!(s.doc().unwrap().history.undo.len(), history);
        pointer(&mut s, PointerKind::Down, 300.0, 300.0);
        pointer(&mut s, PointerKind::Up, 300.0, 300.0);
        assert_eq!(s.doc().unwrap().doc.artboards.len(), 1, "empty clicks do not use a stale object");
        pointer(&mut s, PointerKind::Down, 140.0, 125.0);
        pointer(&mut s, PointerKind::Up, 140.0, 125.0);
        assert_eq!(s.doc().unwrap().doc.artboards.len(), 2);
    }
}

#[test]
fn locked_hidden_modifier_handle_and_drag_gestures_keep_their_existing_meaning() {
    for command in ["object.lock", "object.hide"] {
        let (mut s, _) = fixture(100.0, 100.0);
        s.execute(command, &json!({})).unwrap();
        let before = s.doc().unwrap().doc.clone();
        let history = s.doc().unwrap().history.undo.len();
        pointer(&mut s, PointerKind::Down, 140.0, 125.0);
        pointer(&mut s, PointerKind::Up, 140.0, 125.0);
        assert_eq!(json!(s.doc().unwrap().doc), json!(before));
        assert_eq!(s.doc().unwrap().history.undo.len(), history);
    }
    for mods in [Mods { shift: true, ..Default::default() }, Mods { alt: true, ..Default::default() }, Mods { cmd: true, ..Default::default() }] {
        let (mut s, _) = fixture(100.0, 100.0);
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, 140.0, 125.0).with_mods(mods), ViewInfo::default()).unwrap();
        }
        assert_eq!(s.doc().unwrap().doc.artboards.len(), 1);
    }
    let (mut s, _) = fixture(0.0, 0.0);
    pointer(&mut s, PointerKind::Down, 0.0, 0.0);
    pointer(&mut s, PointerKind::Up, 0.0, 0.0);
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 1, "an artboard handle wins over art beneath it");
    for (x, y) in [(100.0, 100.0), (650.0, 700.0)] {
        let (mut s, _) = fixture(x, y);
        pointer(&mut s, PointerKind::Down, x + 40.0, y + 25.0);
        pointer(&mut s, PointerKind::Drag, x + 80.0, y + 50.0);
        pointer(&mut s, PointerKind::Drag, x + 40.0, y + 25.0);
        pointer(&mut s, PointerKind::Up, x + 40.0, y + 25.0);
        assert_eq!(s.doc().unwrap().doc.artboards.len(), 1, "a drag returning to its start does not become an object click");
    }
}

#[test]
fn a_second_single_click_on_the_same_object_fits_another_artboard() {
    let (mut s, _) = fixture(100.0, 100.0);
    pointer(&mut s, PointerKind::Down, 140.0, 125.0);
    pointer(&mut s, PointerKind::Up, 140.0, 125.0);
    pointer(&mut s, PointerKind::Down, 140.0, 125.0);
    pointer(&mut s, PointerKind::Up, 140.0, 125.0);
    assert_eq!(s.doc().unwrap().doc.artboards.len(), 3, "each single click fits one artboard");
}

#[test]
fn a_double_click_on_an_object_fits_once_and_skips_options() {
    let (mut s, id) = fixture(100.0, 100.0);
    s.execute("artboard.new", &json!({"x": 2000.0, "y": 0.0, "width": 80.0, "height": 40.0, "name": "Far"})).unwrap();
    s.execute("artboard.new", &json!({"x": 205.0, "y": 95.0, "width": 40.0, "height": 40.0, "name": "Touch"})).unwrap();
    s.execute("artboard.new", &json!({"x": 180.0, "y": 150.0, "width": 40.0, "height": 40.0, "name": "Overlap"})).unwrap();
    let before: Vec<_> = s.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    let layers = json!(s.doc().unwrap().doc.layers);
    let sel = s.doc().unwrap().selection.clone();
    let history = s.doc().unwrap().history.undo.len();
    let reqs = double_click(&mut s, 140.0, 125.0, true);
    assert!(reqs.iter().all(|r| !is_dialog(r)), "no Artboard Options on the object, {reqs:?}");
    let st = s.doc().unwrap();
    let fitted = Rect::new(95.0, 95.0, 205.0, 165.0);
    let rects: Vec<_> = st.doc.artboards.iter().map(|a| a.rect).collect();
    assert_eq!(rects.iter().filter(|r| **r == fitted).count(), 1, "the second click does not fit another: {rects:?}");
    for kept in &before {
        assert!(rects.contains(kept), "existing artboards stay: missing {kept:?} in {rects:?}");
    }
    assert_eq!(rects.len(), before.len() + 1);
    assert_eq!(st.doc.bounds_of(&[id], true), Some(fitted));
    assert_eq!(json!(st.doc.layers), layers);
    assert_eq!(json!(st.selection), json!(sel));
    assert_eq!(st.history.undo.len(), history + 1, "only the first click's new artboard");
    s.execute("edit.undo", &json!({})).unwrap();
    let after: Vec<_> = s.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect();
    assert_eq!(after, before);
    assert_eq!(json!(s.doc().unwrap().selection), json!(sel));
}

#[test]
fn without_the_canvas_flag_the_second_release_still_fits_another_artboard() {
    let (mut s, _) = fixture(100.0, 100.0);
    let reqs = double_click(&mut s, 140.0, 125.0, false);
    assert!(reqs.iter().all(|r| !is_dialog(r)), "{reqs:?}");
    let fitted = Rect::new(95.0, 95.0, 205.0, 165.0);
    let n = s.doc().unwrap().doc.artboards.iter().filter(|a| a.rect == fitted).count();
    assert_eq!(n, 2, "each release fits until the canvas suppresses the second one");
    assert!(s.doc().unwrap().doc.artboards.iter().any(|a| a.rect == Rect::new(0.0, 0.0, 500.0, 500.0)));
}

#[test]
fn a_double_click_on_an_object_does_not_open_options_or_change_the_document() {
    let (mut s, _) = fixture(100.0, 100.0);
    let before = s.doc().unwrap().doc.clone();
    let history = s.doc().unwrap().history.undo.len();
    let reqs = pointer_reqs(&mut s, PointerKind::DoubleClick, 140.0, 125.0);
    assert!(reqs.iter().all(|r| !is_dialog(r)), "{reqs:?}");
    assert_eq!(json!(s.doc().unwrap().doc), json!(before));
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
}

#[test]
fn double_click_on_an_artboard_handle_opens_options() {
    let (mut s, _) = fixture(0.0, 0.0);
    let before = s.doc().unwrap().doc.clone();
    let reqs = pointer_reqs(&mut s, PointerKind::DoubleClick, 0.0, 0.0);
    assert!(reqs.iter().any(is_dialog), "a handle keeps Artboard Options, {reqs:?}");
    assert_eq!(json!(s.doc().unwrap().doc), json!(before));
}

#[test]
fn double_click_on_empty_artboard_space_opens_options_and_changes_nothing() {
    let (mut s, _) = fixture(100.0, 100.0);
    let before = s.doc().unwrap().doc.clone();
    let history = s.doc().unwrap().history.undo.len();
    let reqs = pointer_reqs(&mut s, PointerKind::DoubleClick, 300.0, 300.0);
    assert!(reqs.iter().any(is_dialog), "{reqs:?}");
    assert_eq!(json!(s.doc().unwrap().doc), json!(before));
    assert_eq!(s.doc().unwrap().history.undo.len(), history);
}
