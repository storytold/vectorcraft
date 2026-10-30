use serde_json::json;
use vectorcraft_engine::Session;

use super::{ID, KIND, show};
use crate::tests_labels::painted_text;
use crate::{VectorcraftApp, menus};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app
}

#[test]
fn the_plugins_submenu_selects_the_tools_and_opens_the_anchor_inspector() {
    let mut app = app();
    let entries = menus::menu_entries(&app);
    let under = |label: &str| entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Path Precision Tools"] && e.label == label);
    for (label, tool) in
        [("Anchor Eraser Brush", "anchorEraserBrush"), ("Extend Path Tool", "extendPath"), ("Measure & Inspect Tool", "measureInspect")]
    {
        let e = under(label).expect(label);
        assert_eq!((e.command.as_deref(), &e.params), (Some("tool.select"), &json!({ "tool": tool })));
        menus::invoke(&mut app, "tool.select", e.params.clone());
        assert_eq!(app.session.tool_id(), tool);
    }
    for (label, cut) in [("Add Points at Crossings", false), ("Split at Crossings", true)] {
        let e = under(label).expect(label);
        assert_eq!(e.command.as_deref(), Some("addon.pathtools.crossings"));
        assert_eq!(e.params.get("cut").and_then(|v| v.as_bool()).unwrap_or(false), cut);
    }
    // Artwork Cleanup's submenu and Install Plug-in… are still there.
    assert!(entries.iter().any(|e| e.path == ["Object", "Plug-ins", "Artwork Cleanup"]));
    assert!(entries.iter().any(|e| e.path == ["Object", "Plug-ins"] && e.label == "Install Plug-in…"));
    let inspector = under("Anchor Inspector").expect("listed");
    assert!(!crate::panel_docking::showing(&app.ui, ID));
    menus::invoke(&mut app, "window.panel", inspector.params.clone());
    assert!(crate::panel_docking::showing(&app.ui, ID), "the panel opens");
}

#[test]
fn the_inspector_shows_the_selected_anchor_and_its_buttons_edit() {
    let mut app = app();
    let r = app.run("path.create", json!({"anchors": [{"x": 10, "y": 10}, {"x": 110, "y": 10}, {"x": 110, "y": 110}], "closed": false})).unwrap();
    let id = vectorcraft_doc::NodeId(r["id"].as_u64().unwrap());
    let text = painted_text(&mut app, show);
    assert!(text.contains("Direct-select anchor points to edit them.") && text.contains("Make Smooth") && text.contains("Close Path:"), "{text}");
    app.session
        .select(|_, sel| {
            sel.set([id]);
            sel.anchors.insert(id, [(0, 1)].into_iter().collect());
        })
        .unwrap();
    let text = painted_text(&mut app, show);
    assert!(text.contains("Anchors selected: 1"), "{text}");
    // The panel's buttons run the add-on's commands, each one undo step.
    let steps = app.session.doc().unwrap().history.undo.len();
    app.run("addon.pathtools.smooth", json!({})).unwrap();
    let a = app.session.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].anchors[1];
    assert!(a.has_in() && a.has_out());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1);
}

#[test]
fn double_clicking_the_brush_opens_its_options_and_ok_sets_them() {
    let mut app = app();
    app.run("tool.options", json!({"tool": "anchorEraserBrush"})).unwrap();
    let d = app.ui.dialog.as_mut().expect("the dialog");
    assert_eq!(d.kind, KIND);
    assert_eq!((d.f64("size", 0.0), d.f64("tolerance", 0.0), d.bool("pressure")), (20.0, 10.0, false));
    let text = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert!(text.contains("Anchor Eraser Brush Options") && text.contains("Tolerance:") && text.contains("Pressure"), "{text}");
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("size".into(), json!(64));
    d.fields.insert("pressure".into(), json!(true));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    let o = app.session.tool_options_of("anchorEraserBrush");
    assert_eq!((o["size"].as_f64(), o["pressure"].as_bool()), (Some(64.0), Some(true)));
    // The Extend Path tool's mode; the Measure & Inspect tool has none.
    app.run("tool.options", json!({"tool": "extendPath"})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("mode".into(), json!("arc"));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(app.session.tool_options_of("extendPath")["mode"], json!("arc"));
    assert!(app.run("tool.options", json!({"tool": "measureInspect"})).is_err());
}
