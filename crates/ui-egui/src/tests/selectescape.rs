//! Escape through shortcut routing during a Selection tool move, Alt-drag copy, scale or rotation.

use egui::{Key, Modifiers};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::VectorcraftApp;

/// One frame pressing `key` through the shortcut handler.
fn press(app: &mut VectorcraftApp, key: Key) {
    let ctx = egui::Context::default();
    let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
    out.textures_delta.clear();
}

#[test]
fn escape_cancels_a_selection_tool_move_copy_scale_or_rotation() {
    // (what, Alt held, where the drag starts, where it goes)
    for (what, alt, from, to) in [
        ("move", false, (150.0, 150.0), (190.0, 170.0)),
        ("copy", true, (150.0, 150.0), (190.0, 170.0)),
        ("scale", false, (200.0, 200.0), (260.0, 240.0)),
        ("rotate", false, (210.0, 210.0), (150.0, 240.0)),
    ] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.run("select.set", json!({"ids": [a]})).unwrap();
        app.select_tool("selection");
        let st = app.session.doc().unwrap();
        let (before, undo) = (st.doc.clone(), st.history.undo.len());
        let view = app.view_info();
        let mods = Mods { alt, ..Default::default() };
        let mid = ((from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
        for (kind, (x, y)) in [(PointerKind::Down, from), (PointerKind::Drag, mid), (PointerKind::Drag, to)] {
            app.session.pointer(&PointerEvent::new(kind, x, y).with_mods(mods), view).unwrap();
        }
        assert!(app.session.tool_busy(), "{what}");
        assert_ne!(*app.session.doc().unwrap().doc, *before, "{what}: the drag changes the preview");
        press(&mut app, Key::Escape);
        assert_eq!(*app.session.doc().unwrap().doc, *before, "{what}: Escape cancels the drag, and no copy is left");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "{what}: and adds no undo step");
        assert!(!app.session.tool_busy(), "{what}");
        assert_eq!(app.session.tool_id(), "selection", "{what}: Escape keeps the tool");
        app.session.pointer(&PointerEvent::new(PointerKind::Up, to.0, to.1).with_mods(mods), view).unwrap();
        assert_eq!(*app.session.doc().unwrap().doc, *before, "{what}: the release changes nothing");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "{what}");
    }
}
