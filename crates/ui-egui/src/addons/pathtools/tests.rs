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
        ("Tangent Circle Tool", "tangentCircle"),
        ("Trim at Crossings Tool", "trimAtCrossings"),
    ] {
        let e = under(label).expect(label);
        assert_eq!(vectorcraft_tools::tool_info(tool).map(|t| t.label), Some(label), "the toolbar's label");
        assert_eq!((e.command.as_deref(), &e.params), (Some("tool.select"), &json!({ "tool": tool })));
        menus::invoke(&mut app, "tool.select", e.params.clone());
        assert_eq!(app.session.tool_id(), tool);
    }
    for (label, cut) in [("Add Points at Crossings", false), ("Split at Crossings", true)] {
        let e = under(label).expect(label);
        assert_eq!(e.command.as_deref(), Some("addon.pathtools.crossings"));
        assert_eq!(e.params.get("cut").and_then(|v| v.as_bool()).unwrap_or(false), cut);
    }
    // Expand Dashes into Paths runs at once: a dashed line becomes its dashes.
    let line = app.run("path.create", json!({"anchors": [{"x": 0, "y": 0}, {"x": 100, "y": 0}]})).unwrap()["id"].clone();
    app.run("select.set", json!({"ids": [line]})).unwrap();
    app.run("stroke.set", json!({"dash": [30, 10]})).unwrap();
    let expand = under("Expand Dashes into Paths").expect("listed");
    let (cmd, p) = menus::click_target(&expand.label, expand.command.as_deref().unwrap(), &expand.params);
    menus::invoke(&mut app, &cmd, p);
    let selected = app.session.doc().unwrap().selection.objects.len();
    assert!(selected >= 3, "three dashes selected: {selected}");
    app.run("edit.undo", json!({})).unwrap();
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
    // The Tangent Circle tool's radius for Shift.
    app.run("tool.options", json!({"tool": "tangentCircle"})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().f64("radius", 0.0), 36.0);
    let text = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert!(text.contains("Tangent Circle Tool Options") && text.contains("Radius:"), "{text}");
    app.ui.dialog.as_mut().unwrap().fields.insert("radius".into(), json!(12.5));
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!(app.session.tool_options_of("tangentCircle")["radius"], json!(12.5));
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

/// The Corner Tool and Shared Edge Merge Tool items leave a floating Anchor Inspector showing
/// instead of closing it, and in Presentation Mode they choose the tool without failing.
#[test]
fn the_tool_items_keep_a_floating_anchor_inspector_showing() {
    let mut app = app();
    app.run("window.panel.float", json!({ "panel": ID })).unwrap();
    assert!(crate::panel_docking::visible(&app.ui, ID));
    for command in [super::corner::COMMAND, super::shared_edge::COMMAND] {
        app.run(command, json!({})).unwrap();
        assert!(crate::panel_docking::visible(&app.ui, ID), "{command} leaves the panel showing");
    }
    app.ui.screen_mode = 3;
    app.run(super::corner::COMMAND, json!({})).unwrap();
    assert_eq!(app.session.tool_id(), "cornerTool");
    app.ui.screen_mode = 0;
    assert!(crate::panel_docking::visible(&app.ui, ID), "the panel shows when Presentation Mode ends");
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

/// Object › Plug-ins › Path Precision Tools › Common Tangents… opens the command dialog on all
/// tangents without the centerline, Preview off. With Preview on the lines show on the canvas;
/// Cancel takes them away with no step of history, and OK keeps them as one step. Nested circles
/// change nothing and say why.
#[test]
fn common_tangents_previews_cancels_and_runs_on_ok() {
    let mut app = app();
    let circle = |app: &mut VectorcraftApp, x: f64, r: f64| {
        app.run("shape.ellipse", json!({"x": x - r, "y": 150.0 - r, "width": 2.0 * r, "height": 2.0 * r})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (circle(&mut app, 60.0, 20.0), circle(&mut app, 220.0, 40.0));
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node_count();
    let (start, n, nodes) = (app.session.doc().unwrap().doc.clone(), steps(&app), count(&app));
    let open = |app: &mut VectorcraftApp| {
        let entries = menus::menu_entries(app);
        let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Path Precision Tools"] && e.label == "Common Tangents…").unwrap();
        assert!(item.enabled);
        let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
        menus::invoke(app, &cmd, p);
    };

    open(&mut app);
    let d = app.ui.dialog.as_ref().expect("a dialog opens");
    assert_eq!((d.str("__command"), d.str("__label")), (super::COMMON_TANGENTS.to_string(), "Common Tangents".to_string()));
    assert_eq!((d.str("tangents"), d.bool("centerline"), d.bool("preview")), ("all".to_string(), false, false));
    let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for s in ["Common Tangents", "Tangents", "All", "Centerline", "Preview"] {
        assert!(shown.contains(s), "{s} in {shown}");
    }
    // Each kind the list offers is one the command takes.
    for (_, kind) in super::TANGENT_KINDS {
        assert!(vectorcraft_engine::check_params(super::COMMON_TANGENTS, &json!({"tangents": kind})).is_ok(), "{kind}");
    }
    assert_eq!(count(&app), nodes, "no preview without the checkbox");

    // Preview on: the four lines are on the canvas.
    crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "preview".into(), json!(true));
    painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert_eq!(count(&app), nodes + 4, "previewed");
    // External only, with the centerline: two lines and the centerline.
    let d = app.ui.dialog.as_mut().unwrap();
    crate::dialogs::set_field(d, "tangents".into(), json!("external"));
    crate::dialogs::set_field(d, "centerline".into(), json!(true));
    painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    assert_eq!(count(&app), nodes + 3, "previewed again");

    // Cancel: as it was, with no step of history.
    crate::dialogs::cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, start, "Cancel takes the preview away");
    assert_eq!(steps(&app), n, "and adds no undo step");

    // OK: one step.
    open(&mut app);
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(steps(&app), n + 1, "one undo step");
    assert_eq!(count(&app), nodes + 4);

    // A circle inside another: nothing changes, and a message says why.
    let inner = circle(&mut app, 220.0, 10.0);
    app.run("select.set", json!({"ids": [b, inner]})).unwrap();
    let (n, nodes) = (steps(&app), count(&app));
    open(&mut app);
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!((steps(&app), count(&app)), (n, nodes));
    assert!(app.ui.status.contains("inside the other"), "{}", app.ui.status);
}

/// Common Tangents… on a point and an ellipse: every Tangents choice previews the same two lines
/// from the point, OK keeps them as one step, and a point inside the ellipse changes nothing and
/// says why.
#[test]
fn common_tangents_on_a_point_gives_the_same_lines_for_every_choice() {
    let mut app = app();
    let oval = app.run("shape.ellipse", json!({"x": 180.0, "y": 120.0, "width": 80.0, "height": 60.0})).unwrap()["id"].as_u64().unwrap();
    let point =
        |app: &mut VectorcraftApp, x: f64, y: f64| app.run("path.create", json!({"anchors": [{"x": x, "y": y}]})).unwrap()["id"].as_u64().unwrap();
    let dot = point(&mut app, 40.0, 150.0);
    app.run("select.set", json!({"ids": [oval, dot]})).unwrap();
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node_count();
    // The end points of every two-anchor path in the document, rounded and sorted.
    let lines = |app: &VectorcraftApp| {
        let mut ends = Vec::new();
        app.session.doc().unwrap().doc.walk(|node| {
            if let Some(path) = node.path_data() {
                for sub in &path.subpaths {
                    if sub.anchors.len() == 2 {
                        let mut pair: Vec<(i64, i64)> =
                            sub.anchors.iter().map(|a| ((a.p.x * 1e6).round() as i64, (a.p.y * 1e6).round() as i64)).collect();
                        pair.sort();
                        ends.push(pair);
                    }
                }
            }
        });
        ends.sort();
        ends
    };
    let (start, n, nodes) = (app.session.doc().unwrap().doc.clone(), steps(&app), count(&app));
    let open = |app: &mut VectorcraftApp| {
        let entries = menus::menu_entries(app);
        let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Path Precision Tools"] && e.label == "Common Tangents…").unwrap();
        assert!(item.enabled);
        let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
        menus::invoke(app, &cmd, p);
    };

    // Each choice previews the same two lines, and OK with that choice keeps them as one step;
    // undo puts the document back as it was.
    let mut first: Option<Vec<Vec<(i64, i64)>>> = None;
    for (_, kind) in super::TANGENT_KINDS {
        open(&mut app);
        let d = app.ui.dialog.as_mut().unwrap();
        crate::dialogs::set_field(d, "preview".into(), json!(true));
        crate::dialogs::set_field(d, "tangents".into(), json!(kind));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert_eq!(count(&app), nodes + 2, "{kind}: the two lines from the point");
        let previewed = lines(&app);
        assert_eq!(previewed.len(), 2, "{kind}");
        let first = first.get_or_insert_with(|| previewed.clone());
        assert_eq!(&previewed, first, "{kind}: the same lines as the first choice");
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!((steps(&app), count(&app)), (n + 1, nodes + 2), "{kind}: one undo step");
        assert_eq!(&lines(&app), first, "{kind}: OK keeps the previewed lines");
        app.run("edit.undo", json!({})).unwrap();
        app.run("select.set", json!({"ids": [oval, dot]})).unwrap();
        assert_eq!((steps(&app), count(&app)), (n, nodes), "{kind}: undone");
        assert_eq!(app.session.doc().unwrap().doc, start, "{kind}: undo restores the document");
    }
    let first = first.unwrap();

    // Switching the choice while the preview is on keeps the same two lines on the canvas.
    open(&mut app);
    crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "preview".into(), json!(true));
    painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
    for (_, kind) in super::TANGENT_KINDS.iter().rev() {
        crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "tangents".into(), json!(kind));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert_eq!(count(&app), nodes + 2, "{kind}: switched in place");
        assert_eq!(lines(&app), first, "{kind}: the same lines after the switch");
    }
    crate::dialogs::cancel(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, start, "Cancel takes the preview away");
    assert_eq!(steps(&app), n, "and adds no undo step");

    // A point inside the ellipse: nothing changes, and a message says why.
    let inside = point(&mut app, 220.0, 150.0);
    app.run("select.set", json!({"ids": [oval, inside]})).unwrap();
    let (n, nodes) = (steps(&app), count(&app));
    open(&mut app);
    crate::dialogs::confirm(&mut app).unwrap();
    assert_eq!((steps(&app), count(&app)), (n, nodes));
    assert!(app.ui.status.contains("The point lies inside the path"), "{}", app.ui.status);
}

/// Object › Plug-ins › Path Precision Tools › Add Bounding Rectangle… and Add Bounding Circle… are
/// dimmed with nothing selected and open the command dialog with Offset 0, One per Object off and
/// Preview off. With Preview on the shape shows on the canvas; Cancel takes it away with no step of
/// history. OK with the defaults draws one shape around the whole selection; OK with an Offset and
/// One per Object draws a larger shape around each object, as one undo step. An Offset that takes
/// the shape past the canvas edge leaves the dialog open, the document unchanged and an error.
#[test]
fn add_bounding_rectangle_and_circle_open_a_dialog_with_offset_and_one_per_object() {
    let mut app = app();
    let items = [("Add Bounding Rectangle…", super::BOUNDING_RECTANGLE), ("Add Bounding Circle…", super::BOUNDING_CIRCLE)];
    let find = |app: &VectorcraftApp, label: &str| {
        let entries = menus::menu_entries(app);
        entries.into_iter().find(|e| e.path == ["Object", "Plug-ins", "Path Precision Tools"] && e.label == label).expect(label)
    };
    for (label, command) in items {
        let item = find(&app, label);
        assert_eq!((item.command.as_deref(), &item.params), (Some(command), &json!({"offset": 0, "each": false})));
        assert!(!item.enabled, "{label} with nothing selected");
    }
    let a = app.run("shape.rectangle", json!({"x": 10, "y": 20, "width": 30, "height": 40})).unwrap()["id"].as_u64().unwrap();
    let b = app.run("shape.rectangle", json!({"x": 100, "y": 20, "width": 20, "height": 40})).unwrap()["id"].as_u64().unwrap();
    let steps = |app: &VectorcraftApp| app.session.doc().unwrap().history.undo.len();
    let count = |app: &VectorcraftApp| app.session.doc().unwrap().doc.node_count();
    let made = |app: &VectorcraftApp| {
        let st = app.session.doc().unwrap();
        st.selection.objects.iter().map(|id| st.doc.bounds_of(&[*id], false).unwrap()).collect::<Vec<_>>()
    };
    let open = |app: &mut VectorcraftApp, label: &str| {
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        let item = find(app, label);
        assert!(item.enabled, "{label}");
        let (cmd, p) = menus::click_target(&item.label, item.command.as_deref().unwrap(), &item.params);
        menus::invoke(app, &cmd, p);
    };
    let near = |r: vectorcraft_geom::Rect, want: [f64; 4]| [r.x0, r.y0, r.x1, r.y1].iter().zip(want).all(|(v, w)| (v - w).abs() < 1e-6);
    for (label, command) in items {
        let (start, n, nodes) = (app.session.doc().unwrap().doc.clone(), steps(&app), count(&app));
        open(&mut app, label);
        let d = app.ui.dialog.as_ref().unwrap_or_else(|| panic!("{label}: a dialog opens"));
        assert_eq!((d.str("__command"), d.str("__label")), (command.to_string(), label.trim_end_matches('…').to_string()));
        assert_eq!((d.f64("offset", -1.0), d.bool("each"), d.bool("preview")), (0.0, false, false), "{label}");
        let shown = painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        for s in [label.trim_end_matches('…'), "Offset", "One per Object", "Preview"] {
            assert!(shown.contains(s), "{label}: {s} in {shown}");
        }
        assert_eq!(count(&app), nodes, "{label}: no preview without the checkbox");

        // Preview on: the shape is on the canvas. Cancel: as it was, with no step of history.
        crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "preview".into(), json!(true));
        painted_text(&mut app, |app, ui| crate::dialogs::show(app, ui.ctx()));
        assert_eq!(count(&app), nodes + 1, "{label}: previewed");
        crate::dialogs::cancel(&mut app);
        assert_eq!(app.session.doc().unwrap().doc, start, "{label}: Cancel takes the preview away");
        assert_eq!(steps(&app), n, "{label}: and adds no undo step");

        // OK with the defaults: one shape around the whole selection, from (10, 20) to (120, 60).
        open(&mut app, label);
        crate::dialogs::confirm(&mut app).unwrap();
        assert!(app.ui.dialog.is_none());
        assert_eq!((steps(&app), count(&app)), (n + 1, nodes + 1), "{label}: one shape, one undo step");
        let whole = made(&app);
        if command == super::BOUNDING_RECTANGLE {
            assert!(near(whole[0], [10.0, 20.0, 120.0, 60.0]), "{label}: {:?}", whole[0]);
        } else {
            // The circle on the diagonal, with the 0.01 pt allowance.
            let r = 110.0f64.hypot(40.0) / 2.0 + 0.01;
            assert!(near(whole[0], [65.0 - r, 40.0 - r, 65.0 + r, 40.0 + r]), "{label}: {:?}", whole[0]);
        }

        // OK with an Offset of 5 pt and One per Object: a shape around each object, one undo step.
        open(&mut app, label);
        let d = app.ui.dialog.as_mut().unwrap();
        crate::dialogs::set_field(d, "offset".into(), json!(5));
        crate::dialogs::set_field(d, "each".into(), json!(true));
        crate::dialogs::confirm(&mut app).unwrap();
        assert_eq!((steps(&app), count(&app)), (n + 2, nodes + 3), "{label}: two shapes, one more undo step");
        let each = made(&app);
        if command == super::BOUNDING_RECTANGLE {
            assert!(near(each[0], [5.0, 15.0, 45.0, 65.0]) && near(each[1], [95.0, 15.0, 125.0, 65.0]), "{label}: {each:?}");
        } else {
            let (ra, rb) = (30.0f64.hypot(40.0) / 2.0 + 5.01, 20.0f64.hypot(40.0) / 2.0 + 5.01);
            let want = [[25.0 - ra, 40.0 - ra, 25.0 + ra, 40.0 + ra], [110.0 - rb, 40.0 - rb, 110.0 + rb, 40.0 + rb]];
            assert!(near(each[0], want[0]) && near(each[1], want[1]), "{label}: {each:?}");
        }

        // An Offset past the canvas edge: the dialog stays open, nothing is drawn, and the error
        // says why.
        let (n, nodes) = (steps(&app), count(&app));
        open(&mut app, label);
        crate::dialogs::set_field(app.ui.dialog.as_mut().unwrap(), "offset".into(), json!(5_000_000));
        let e = crate::dialogs::confirm(&mut app).unwrap_err();
        assert!(e.contains("the shape would reach past the edge of the canvas"), "{label}: {e}");
        assert!(app.ui.dialog.is_some(), "{label}: the dialog stays open");
        crate::dialogs::cancel(&mut app);
        assert_eq!((steps(&app), count(&app)), (n, nodes), "{label}: nothing drawn");
    }
    // Every language with complete menus has the menu items, the dialog's headings and its labels.
    for lang in crate::i18n::Lang::all().filter(|l| l.complete_menus()) {
        for s in ["Add Bounding Rectangle…", "Add Bounding Circle…", "Offset", "One per Object"] {
            assert!(crate::i18n::has(lang, s), "{}: {s}", lang.code());
        }
    }
}
