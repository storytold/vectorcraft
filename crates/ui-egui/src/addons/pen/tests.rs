use serde_json::json;
use vectorcraft_engine::Session;

use super::{ID, show};
use crate::tests_labels::painted_text;
use crate::{VectorcraftApp, menus};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app
}

#[test]
fn the_plugins_submenu_selects_the_tool_and_opens_the_panel() {
    let mut app = app();
    let entries = menus::menu_entries(&app);
    let under = |label: &str| entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Smart Pen"] && e.label == label);
    let tool = under("Smart Pen Tool").expect("listed");
    assert_eq!((tool.command.as_deref(), &tool.params), (Some("tool.select"), &json!({ "tool": "smartPen" })));
    menus::invoke(&mut app, "tool.select", tool.params.clone());
    assert_eq!(app.session.tool_id(), "smartPen");
    let panel = under("Smart Pen Panel").expect("listed");
    assert!(!crate::panel_docking::showing(&app.ui, ID));
    menus::invoke(&mut app, "window.panel", panel.params.clone());
    assert!(crate::panel_docking::showing(&app.ui, ID), "the panel opens");
    // The other add-ons' submenus are still there, and the tool has no shortcut.
    assert!(entries.iter().any(|e| e.path == ["Object", "Plug-ins", "Path Precision Tools"]));
    assert!(vectorcraft_tools::tool_info("smartPen").is_some_and(|t| t.shortcut.is_none()));
}

#[test]
fn the_panel_sets_the_next_segment_and_its_buttons_edit() {
    let mut app = app();
    let text = painted_text(&mut app, show);
    for label in [
        "Next Segment",
        "Direct-select anchor points to edit them.",
        "Remove Points, Keep Shape",
        "Make Smooth",
        "Make Corner",
        "Retract Handles",
        "Close or Join",
    ] {
        assert!(text.contains(label), "{label}: {text}");
    }
    assert_eq!(app.session.tool_options_of("smartPen")["next"], json!("straight"));
    app.run("addon.pen.nextSegment", json!({"curved": true})).unwrap();
    assert_eq!(app.session.tool_options_of("smartPen")["next"], json!("curved"));
    // Close or Join closes an open path selected whole, as one undo step.
    let r = app.run("path.create", json!({"anchors": [{"x": 10, "y": 10}, {"x": 110, "y": 10}, {"x": 110, "y": 110}], "closed": false})).unwrap();
    let id = vectorcraft_doc::NodeId(r["id"].as_u64().unwrap());
    let steps = app.session.doc().unwrap().history.undo.len();
    app.run("addon.pen.closeJoin", json!({})).unwrap();
    assert!(app.session.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].closed);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1);
}
