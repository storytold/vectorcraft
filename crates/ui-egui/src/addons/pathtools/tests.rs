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
    for (label, tool) in [
        ("Anchor Eraser Brush", "anchorEraserBrush"),
        ("Extend Path Tool", "extendPath"),
        ("Measure & Inspect Tool", "measureInspect"),
        ("Parametric Shape Tool", "parametricShape"),
        ("Slide Anchor Along Path", "slideAnchor"),
    ] {
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

#[test]
fn the_parametric_shape_panel_opens_from_the_submenu_and_shows_the_shape() {
    let mut app = app();
    let entries = menus::menu_entries(&app);
    let under = |label: &str| {
        let e = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Path Precision Tools"] && e.label == label).expect(label);
        (e.command.clone(), e.params.clone())
    };
    assert_eq!(under("Convert to Parametric Shape").0.as_deref(), Some("addon.pathtools.shape.convert"));
    let (command, params) = under("Parametric Shape");
    assert_eq!(command.as_deref(), Some("window.panel"));
    menus::invoke(&mut app, "window.panel", params);
    assert!(crate::panel_docking::showing(&app.ui, super::shape::ID), "the panel opens");
    // Nothing selected: the fields set the tool's next shape.
    let text = painted_text(&mut app, super::shape::show);
    assert!(text.contains("With none selected") && text.contains("Polygon") && text.contains("Count:") && text.contains("Corner Radius:"), "{text}");
    app.run("tool.setOption", json!({"tool": "parametricShape", "values": {"kind": "gear"}})).unwrap();
    let text = painted_text(&mut app, super::shape::show);
    assert!(text.contains("Gear") && text.contains("Tooth Height:"), "{text}");
    // A shape selected: its own numbers.
    app.run("addon.pathtools.shape.create", json!({"kind": "bubble", "center": [150, 150]})).unwrap();
    let text = painted_text(&mut app, super::shape::show);
    assert!(text.contains("Speech Bubble") && text.contains("Tail:") && text.contains("Tip:") && !text.contains("With none selected"), "{text}");
    assert!(!text.contains("Count:"), "a speech bubble has no count: {text}");
}

#[test]
fn the_corner_tool_item_selects_the_tool_and_opens_its_panel_section() {
    let mut app = app();
    let entries = menus::menu_entries(&app);
    let e = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Path Precision Tools"] && e.label == "Corner Tool").expect("listed");
    assert!(!crate::panel_docking::showing(&app.ui, ID));
    menus::invoke(&mut app, e.command.as_deref().unwrap(), e.params.clone());
    assert_eq!(app.session.tool_id(), "cornerTool");
    assert!(crate::panel_docking::showing(&app.ui, ID), "the panel opens");
    // Neither toolbar shows it, the All Tools list does.
    assert!(
        vectorcraft_tools::catalog::all_tools().any(|t| t.id == "cornerTool")
            && vectorcraft_tools::catalog::all_tools().any(|t| t.id == "slideAnchor")
    );
    assert!(!vectorcraft_tools::TOOL_GROUPS.iter().flat_map(|g| g.iter()).any(|t| t.id == "cornerTool" || t.id == "slideAnchor"));
}

#[test]
fn the_shared_edge_item_selects_the_tool_and_opens_its_panel_section() {
    let mut app = app();
    let entries = menus::menu_entries(&app);
    let e = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Path Precision Tools"] && e.label == "Shared Edge Merge Tool").expect("listed");
    assert!(!crate::panel_docking::showing(&app.ui, ID));
    menus::invoke(&mut app, e.command.as_deref().unwrap(), e.params.clone());
    assert_eq!(app.session.tool_id(), "sharedEdge");
    assert!(crate::panel_docking::showing(&app.ui, ID), "the panel opens");
    // Neither toolbar shows it and it has no shortcut; the All Tools list does.
    let tool = vectorcraft_tools::catalog::all_tools().find(|t| t.id == "sharedEdge").expect("in All Tools");
    assert!(tool.shortcut.is_none());
    assert!(!vectorcraft_tools::TOOL_GROUPS.iter().flat_map(|g| g.iter()).any(|t| t.id == "sharedEdge"));
    // The panel's section shows the tool's values and its option acts.
    let text = painted_text(&mut app, show);
    assert!(text.contains("Shared Edges") && text.contains("Coincidence:") && text.contains("Keep Shared Points Together"), "{text}");
    app.run("addon.pathtools.movePointsTogether", json!({ "enabled": true })).unwrap();
    assert_eq!(app.session.tool_options_of("sharedEdge")["together"], json!(true));
}

#[test]
fn the_corner_section_shows_the_tools_values_and_applies_them_to_the_selection() {
    let mut app = app();
    let r = app
        .run("path.create", json!({"anchors": [{"x": 10, "y": 10}, {"x": 110, "y": 10}, {"x": 110, "y": 110}, {"x": 10, "y": 110}], "closed": true}))
        .unwrap();
    let id = vectorcraft_doc::NodeId(r["id"].as_u64().unwrap());
    let text = painted_text(&mut app, show);
    assert!(
        text.contains("Corners selected: 4") && text.contains("Radius:") && text.contains("Round") && text.contains("Apply to Selected Anchors"),
        "{text}"
    );
    app.run("tool.setOption", json!({"tool": "cornerTool", "values": {"radius": 12, "kind": "chamfer"}})).unwrap();
    assert!(painted_text(&mut app, show).contains("Chamfer"));
    app.session
        .select(|_, sel| {
            sel.set([id]);
            sel.anchors.insert(id, [(0, 1), (0, 2)].into_iter().collect());
        })
        .unwrap();
    assert!(painted_text(&mut app, show).contains("Corners selected: 2"));
    // What Apply runs: the tool's values on the selected corners, one undo step.
    let steps = app.session.doc().unwrap().history.undo.len();
    let options = app.session.tool_options_of("cornerTool");
    app.run("addon.pathtools.cornerSet", json!({"radius": options["radius"], "kind": options["kind"]})).unwrap();
    assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1);
    assert_eq!(app.session.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().anchor_count(), 6);
    assert!(painted_text(&mut app, show).contains("Corners selected: 2"), "still selected");
}

#[test]
fn effect_path_effects_lists_the_live_effects_and_their_dialogs_pick_from_their_values() {
    let mut app = app();
    let entries = menus::menu_entries(&app);
    let at = |label: &str| entries.iter().find(|e| e.path == ["Effect", "Path Effects"] && e.label == label);
    for label in [
        "Dashes to Geometry…",
        "Add Anchors…",
        "Extend Path…",
        "Close Path…",
        "Smooth Anchors…",
        "Retract Handles…",
        "Simplify Anchors…",
        "Split at Anchors…",
    ] {
        assert_eq!(at(label).expect(label).command.as_deref(), Some("effect.dialog"), "{label}");
    }
    // No options: applied at once.
    assert_eq!(at("Reverse Direction").expect("listed").command.as_deref(), Some("effect.apply"));
    assert_eq!(entries.iter().filter(|e| e.path == ["Effect", "Path Effects"]).count(), 9);
    // The dialog opens with the effect's parameters, each choice on a list.
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    menus::invoke(&mut app, "effect.dialog", json!({"effect": "addon.path.addAnchors"}));
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("effect"));
    let text = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert!(text.contains("Add Anchors") && text.contains("Max Count:") && text.contains("All"), "{text}");
    app.ui.dialog = None;
    let _ = app.session.cancel_interaction();
    menus::invoke(&mut app, "effect.dialog", json!({"effect": "addon.path.dashes"}));
    let text = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert!(text.contains("Dash 1:") && text.contains("Gap 5:") && text.contains("Align to Corners and Ends:"), "{text}");
    for (effect, key) in [
        ("addon.path.addAnchors", "target"),
        ("addon.path.addAnchors", "spacing"),
        ("addon.path.extend", "startShape"),
        ("addon.path.extend", "endShape"),
        ("addon.path.close", "mode"),
        ("addon.path.retract", "handles"),
        ("addon.path.split", "anchors"),
    ] {
        let doc = vectorcraft_effects::effect_info(effect).unwrap();
        let options = super::effect_choices(effect, key).expect(key);
        let default = doc.defaults[key].as_str().unwrap();
        assert!(options.iter().any(|(_, value)| *value == default), "{effect} {key}");
        assert!(options.iter().all(|(_, value)| doc.params.contains(&format!("\"{value}\""))), "{effect} {key}");
    }
}
