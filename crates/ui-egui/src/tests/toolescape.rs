//! Escape through shortcut routing during a drag with the Anchor Point, Mesh, Direct Selection,
//! Shape Builder and Measure tools.

use egui::{Key, Modifiers};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::VectorcraftApp;

/// One frame pressing `key` through the shortcut handler.
fn press(app: &mut VectorcraftApp, key: Key) {
    let ctx = egui::Context::default();
    let ev = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let mut out = ctx.run_ui(egui::RawInput { events: vec![ev], ..Default::default() }, |ui| crate::shortcuts::handle(app, ui.ctx()));
    out.textures_delta.clear();
}

#[test]
fn escape_cancels_an_anchor_point_mesh_shape_builder_or_measure_drag() {
    // (the tool, a gradient mesh first, where the drag starts)
    for (tool, mesh, from) in [
        ("anchorPoint", false, (100.0, 100.0)),
        ("mesh", true, (100.0, 100.0)),
        ("directSelection", true, (100.0, 100.0)),
        ("shapeBuilder", false, (150.0, 150.0)),
        ("measure", false, (150.0, 150.0)),
    ] {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        let b = app.run("shape.rectangle", json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        if mesh {
            app.run("object.mesh.create", json!({"ids": [a], "at": [150, 150]})).unwrap();
        }
        app.run("select.set", json!({"ids": if mesh { json!([a]) } else { json!([a, b]) }})).unwrap();
        app.select_tool(tool);
        let st = app.session.doc().unwrap();
        let (before, undo) = (st.doc.clone(), st.history.undo.len());
        let options = app.session.tool_options();
        let view = app.view_info();
        let (mid, to) = ((from.0 + 30.0, from.1 + 10.0), (from.0 + 60.0, from.1 + 20.0));
        for (kind, (x, y)) in [(PointerKind::Down, from), (PointerKind::Drag, mid), (PointerKind::Drag, to)] {
            app.session.pointer(&PointerEvent::new(kind, x, y), view).unwrap();
        }
        assert!(app.session.tool_busy(), "{tool}");
        match tool {
            "shapeBuilder" => {}
            "measure" => assert_ne!(app.session.tool_options(), options, "{tool}: the drag measures"),
            _ => assert_ne!(*app.session.doc().unwrap().doc, *before, "{tool}: the drag changes the preview"),
        }
        press(&mut app, Key::Escape);
        assert_eq!(*app.session.doc().unwrap().doc, *before, "{tool}: Escape cancels the drag");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "{tool}: and adds no undo step");
        assert!(!app.session.tool_busy(), "{tool}");
        assert_eq!(app.session.tool_id(), tool, "{tool}: Escape keeps the tool");
        app.session.pointer(&PointerEvent::new(PointerKind::Up, to.0, to.1), view).unwrap();
        assert_eq!(*app.session.doc().unwrap().doc, *before, "{tool}: the release changes nothing");
        assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "{tool}");
        assert_eq!(app.session.tool_options()["distance"], options["distance"], "{tool}: no measurement stays");
    }
}
