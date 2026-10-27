//! Delete and Backspace through shortcut routing: Artboard editing protects selected artwork,
//! including when no artboard can be deleted. Width points and Puppet Warp pins take the keys
//! while selected, then leave them to Clear.

use egui::{Key, Modifiers};
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::VectorcraftApp;

/// A 300×300 document with a selected 10 pt line from (100, 200) to (300, 200) → its id.
fn app_with_line() -> (VectorcraftApp, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let id = app.run("shape.line", json!({"x1": 100, "y1": 200, "x2": 300, "y2": 200})).unwrap()["id"].as_u64().unwrap();
    app.run("stroke.set", json!({"weight": 10})).unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    (app, id)
}

/// One frame pressing `key` through the shortcut handler.
fn press(app: &mut VectorcraftApp, key: Key) {
    let ctx = egui::Context::default();
    let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
    out.textures_delta.clear();
}

#[test]
fn escape_leaves_artboard_editing_after_cancelling_an_active_drag() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app.select_tool("artboard");
    press(&mut app, Key::Escape);
    assert_eq!(app.session.tool_id(), "selection", "idle Escape leaves artboard editing");

    app.select_tool("artboard");
    let before = app.session.doc().unwrap().doc.clone();
    let view = app.view_info();
    for (kind, x, y) in [(PointerKind::Down, 200.0, 200.0), (PointerKind::Drag, 250.0, 240.0)] {
        app.session.pointer(&PointerEvent::new(kind, x, y), view).unwrap();
    }
    assert_ne!(*app.session.doc().unwrap().doc, *before, "drag changes the preview");
    press(&mut app, Key::Escape);
    assert_eq!(*app.session.doc().unwrap().doc, *before, "Escape cancels the drag");
    assert_eq!(app.session.tool_id(), "artboard", "first Escape only cancels the gesture");
    press(&mut app, Key::Escape);
    assert_eq!(app.session.tool_id(), "selection", "next Escape leaves artboard editing");
}

fn click(app: &mut VectorcraftApp, x: f64, y: f64) {
    let v = app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up] {
        app.session.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
    }
}

fn exists(app: &VectorcraftApp, id: u64) -> bool {
    app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).is_some()
}

fn width_points(app: &VectorcraftApp, id: u64) -> Value {
    let n = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).cloned().unwrap();
    json!(n.appearance.stroke().unwrap().profile.as_ref().map(|p| p.points.len()))
}

#[test]
fn delete_and_backspace_remove_the_selected_width_point_then_clear() {
    for key in [Key::Delete, Key::Backspace] {
        let (mut app, id) = app_with_line();
        app.run("stroke.widthPoint.set", json!({"id": id, "t": 0.5, "left": 12, "right": 12})).unwrap();
        assert_eq!(width_points(&app, id), json!(3));
        app.select_tool("width");
        // A click on the width point's centre selects it; the key removes it, not the line.
        click(&mut app, 200.0, 200.0);
        press(&mut app, key);
        assert!(exists(&app, id), "{key:?} kept the line");
        assert_eq!(width_points(&app, id), json!(2), "{key:?} removed the width point");
        // Nothing of the tool's is selected now: the key clears the selected line.
        press(&mut app, key);
        assert!(!exists(&app, id), "{key:?} with no width point selected clears the selection");
    }
}

#[test]
fn delete_and_backspace_remove_the_selected_pin_then_clear() {
    for key in [Key::Delete, Key::Backspace] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.select_tool("puppetWarp");
        let pins = |app: &VectorcraftApp| app.session.tool_options()["pins"].as_array().map_or(0, Vec::len);
        // A click on the art away from the automatic pins adds a pin and selects it.
        click(&mut app, 112.0, 188.0);
        let n = pins(&app);
        assert!(n > 1, "{n} pins");
        press(&mut app, key);
        assert!(exists(&app, id), "{key:?} kept the art");
        assert_eq!(pins(&app), n - 1, "{key:?} removed the pin");
        press(&mut app, key);
        assert!(!exists(&app, id), "{key:?} with no pin selected clears the selection");
    }
}

fn artboard_keys_app(boards: usize) -> (VectorcraftApp, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 400, "artboards": boards})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 30})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [id]})).unwrap();
    app.select_tool("artboard");
    (app, id)
}

/// A protected deletion must preserve both history branches and the saved snapshot.
fn protected_artboard_key(key: Key) {
    let (mut app, id) = artboard_keys_app(1);
    app.run("object.move", json!({"dx": 7, "dy": 9})).unwrap();
    let redo_doc = app.session.doc().unwrap().doc.clone();
    app.run("edit.undo", json!({})).unwrap();
    app.session.active_mut().unwrap().mark_saved();
    let st = app.session.doc().unwrap();
    let before = st.doc.clone();
    let selection = st.selection.clone();
    let history = (st.history.undo.len(), st.history.redo.len());
    assert!(history.0 > 0 && history.1 == 1);
    let options = app.session.tool_options();
    let active = app.view().unwrap().artboard;

    press(&mut app, key);

    assert!(exists(&app, id), "{key:?} with the last artboard must keep selected artwork");
    let st = app.session.doc().unwrap();
    assert_eq!(*st.doc, *before, "protected artboards and artwork stay unchanged");
    assert_eq!(st.selection, selection);
    assert_eq!((st.history.undo.len(), st.history.redo.len()), history);
    assert!(!st.is_dirty());
    assert_eq!(app.session.tool_options(), options);
    assert_eq!(app.view().unwrap().artboard, active);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(*app.session.doc().unwrap().doc, *redo_doc, "the original Redo still works");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(*app.session.doc().unwrap().doc, *before);
    assert!(!app.session.doc().unwrap().is_dirty());
}

#[test]
fn artboard_keys_delete_keeps_first_of_all_selected_boards() {
    delete_artboards_with_key(Key::Delete, true);
}

#[test]
fn artboard_keys_backspace_keeps_first_of_all_selected_boards() {
    delete_artboards_with_key(Key::Backspace, true);
}

#[test]
fn artboard_keys_delete_protects_last_board_and_art() {
    protected_artboard_key(Key::Delete);
}

#[test]
fn artboard_keys_backspace_protects_last_board_and_art() {
    protected_artboard_key(Key::Backspace);
}

fn delete_artboards_with_key(key: Key, select_all: bool) {
    let (mut app, id) = artboard_keys_app(3);
    // Reorder so the first board is neither the original lowest-ID board nor the active last one.
    app.run("artboard.reorder", json!({"index": 2, "to": 0})).unwrap();
    let selected = if select_all { json!([2, 1, 0]) } else { json!([2, 0]) };
    crate::panels::artboards::select(&mut app, 2);
    app.session.set_tool_option("selected", &selected);
    app.session.active_mut().unwrap().mark_saved();
    let st = app.session.doc().unwrap();
    let before = st.doc.clone();
    let selection = st.selection.clone();
    let survivor = if select_all { 0 } else { 1 };
    let undo = st.history.undo.len();

    press(&mut app, key);

    let st = app.session.doc().unwrap();
    assert_eq!(st.doc.artboards, vec![before.artboards[survivor].clone()], "the correct original board record survives");
    assert_eq!(st.selection, selection);
    assert_eq!(st.doc.node(vectorcraft_doc::NodeId(id)), before.node(vectorcraft_doc::NodeId(id)), "artwork is unchanged");
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Delete Artboard");
    assert!(st.history.redo.is_empty());
    assert!(st.is_dirty());
    let after = st.doc.clone();
    assert_eq!(app.session.tool_options()["selected"], json!([0]));
    assert_eq!(app.session.tool_options()["active"], json!(0));
    assert_eq!(app.view().unwrap().artboard, 0);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(*app.session.doc().unwrap().doc, *before);
    assert!(!app.session.doc().unwrap().is_dirty());
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(*app.session.doc().unwrap().doc, *after);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
}

#[test]
fn artboard_keys_delete_removes_partial_selection_in_one_step() {
    delete_artboards_with_key(Key::Delete, false);
}

#[test]
fn artboard_keys_backspace_removes_partial_selection_in_one_step() {
    delete_artboards_with_key(Key::Backspace, false);
}

/// Keyboard-only history sequences must not need a canvas event to align the navigator and tool.
fn artboard_keys_history_sequence(redo_before_delete: bool) {
    for key in [Key::Delete, Key::Backspace] {
        for select_all in [false, true] {
            let (mut app, id) = artboard_keys_app(3);
            app.run("artboard.reorder", json!({"index": 2, "to": 0})).unwrap();
            // Match the panel's selection: both the navigator and tool start on the last board.
            crate::panels::artboards::select(&mut app, 2);
            app.session.set_tool_option("selected", &if select_all { json!([2, 1, 0]) } else { json!([2, 0]) });
            assert_eq!(app.view().unwrap().artboard, 2);
            assert_eq!(app.session.tool_options()["active"], json!(2));
            app.session.active_mut().unwrap().mark_saved();
            let st = app.session.doc().unwrap();
            let before = st.doc.clone();
            let selection = st.selection.clone();
            let undo = st.history.undo.len();
            let survivor = if select_all { 0 } else { 1 };
            let check = |app: &VectorcraftApp, indices: &[usize], undo: usize, redo: usize, dirty: bool| {
                let st = app.session.doc().unwrap();
                let expected: Vec<_> = indices.iter().map(|&i| before.artboards[i].clone()).collect();
                assert_eq!(st.doc.artboards, expected, "{key:?}: survivor IDs, order and full records");
                assert_eq!(st.doc.node(vectorcraft_doc::NodeId(id)), before.node(vectorcraft_doc::NodeId(id)), "selected artwork stays unchanged");
                assert_eq!(st.selection, selection);
                assert_eq!((st.history.undo.len(), st.history.redo.len()), (undo, redo));
                assert_eq!(st.is_dirty(), dirty);
                assert_eq!(app.view().unwrap().artboard, 0, "navigator must follow the surviving active board without a canvas event");
                assert_eq!(app.session.tool_options()["active"], json!(0));
                assert_eq!(app.session.tool_options()["selected"], json!([0]));
            };

            press(&mut app, key);
            check(&app, &[survivor], undo + 1, 0, true);
            assert_eq!(app.session.doc().unwrap().history.undo.last().unwrap().label, "Delete Artboard");
            let deleted = app.session.doc().unwrap().doc.clone();
            app.run("edit.undo", json!({})).unwrap();
            check(&app, &[0, 1, 2], undo, 1, false);
            assert_eq!(*app.session.doc().unwrap().doc, *before);

            if redo_before_delete {
                app.run("edit.redo", json!({})).unwrap();
                check(&app, &[survivor], undo + 1, 0, true);
                assert_eq!(*app.session.doc().unwrap().doc, *deleted);
                press(&mut app, key);
                check(&app, &[survivor], undo + 1, 0, true);
                assert_eq!(*app.session.doc().unwrap().doc, *deleted, "deleting the final board is a protected no-op");
            } else {
                // Undo restores the boards, not the old multi-selection: the next key deletes
                // the board both the navigator and tool now show, replacing the Redo branch.
                press(&mut app, key);
                check(&app, &[1, 2], undo + 1, 0, true);
                assert_eq!(app.session.doc().unwrap().history.undo.last().unwrap().label, "Delete Artboard");
                let repeated = app.session.doc().unwrap().doc.clone();
                app.run("edit.undo", json!({})).unwrap();
                check(&app, &[0, 1, 2], undo, 1, false);
                assert_eq!(*app.session.doc().unwrap().doc, *before);
                app.run("edit.redo", json!({})).unwrap();
                check(&app, &[1, 2], undo + 1, 0, true);
                assert_eq!(*app.session.doc().unwrap().doc, *repeated);
            }
        }
    }
}

#[test]
fn artboard_keys_keep_navigator_in_sync_through_undo_redo_delete() {
    artboard_keys_history_sequence(true);
}

#[test]
fn artboard_keys_keep_navigator_in_sync_when_deleting_after_undo() {
    artboard_keys_history_sequence(false);
}

#[test]
fn artboard_keys_preserve_drag_preview_and_art() {
    for key in [Key::Delete, Key::Backspace] {
        let (mut app, id) = artboard_keys_app(3);
        app.session.active_mut().unwrap().mark_saved();
        let before = app.session.doc().unwrap().doc.clone();
        let undo = app.session.doc().unwrap().history.undo.len();
        let view = app.view_info();
        for (kind, x, y) in [(PointerKind::Down, 200.0, 200.0), (PointerKind::Drag, 230.0, 225.0)] {
            app.session.pointer(&PointerEvent::new(kind, x, y), view).unwrap();
        }
        let preview = app.session.doc().unwrap().doc.clone();
        assert_ne!(*preview, *before);
        assert!(app.session.tool_busy());

        press(&mut app, key);

        assert!(exists(&app, id), "{key:?} during a drag must keep selected artwork");
        assert_eq!(*app.session.doc().unwrap().doc, *preview);
        assert!(app.session.tool_busy());
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo);
        press(&mut app, Key::Escape);
        assert_eq!(*app.session.doc().unwrap().doc, *before);
        assert!(!app.session.tool_busy());
        assert!(!app.session.doc().unwrap().is_dirty());
    }
}
