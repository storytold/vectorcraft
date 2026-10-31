//! Plug-in panels in headless frames: Window › Plug-ins lists panel plug-ins and opens their
//! panels, which draw the plug-in's answer, send their buttons' clicks and show why an event
//! failed; a panel whose plug-in is removed closes.

use serde_json::{Value, json};
use vectorcraft_plugins::wat as tpl;

use super::tests_appearance::{app_with_rect, click, frame_events, text_rect};
use super::*;

const ID: &str = "test.ui.panel";

/// Installs panel `id` answering every event with `output`.
fn install(app: &mut VectorcraftApp, id: &str, output: &Value) {
    let src = tpl::returning(&tpl::manifest(id, "panel", "{}"), &output.to_string());
    let wasm = wat::parse_str(src).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
}

fn paths(app: &mut VectorcraftApp) -> u64 {
    app.session.execute("document.info", &json!({})).unwrap()["objects"]["paths"].as_u64().unwrap_or(0)
}

fn draw(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    plugin_panel::show(app, ui, &plugin_panel::panel_id(ID));
}

#[test]
fn window_plug_ins_opens_a_panel_that_draws_its_answer_and_sends_clicks() {
    let mut app = app_with_rect();
    let square = json!({"command": "shape.rectangle", "params": {"x": 120, "y": 120, "width": 20, "height": 20}});
    let items = json!([
        {"type": "heading", "text": "Quick Shapes"},
        {"type": "text", "text": "Adds a square."},
        {"type": "button", "id": "add", "label": "Add Square"},
        {"type": "mystery", "text": "Not drawn"},
    ]);
    install(&mut app, ID, &json!({"panel": {"items": items}, "commands": [square]}));
    let entries = crate::menus::menu_entries(&app);
    let entry = entries.iter().find(|e| e.path == ["Window", "Plug-ins"] && e.label == "Test test.ui.panel").expect("listed");
    assert_eq!((entry.command.as_deref(), &entry.params), (Some("window.pluginPanel"), &json!({"id": ID})));
    // Its name is the plug-in's own, not interface text to translate.
    assert!(!crate::menus::menu_strings().contains("Test test.ui.panel"));

    let r = app.run("window.pluginPanel", json!({"id": ID})).unwrap();
    assert_eq!(r["panel"], json!("plugin:test.ui.panel"));
    assert!(crate::panel_docking::layout_of(&app.ui).contains(&"plugin:test.ui.panel".to_string()));
    assert_eq!(crate::panels::plugin_panel::tab_label("plugin:test.ui.panel").as_deref(), Some("Test test.ui.panel"));

    // Drawn for the first time, it asks the plug-in what it shows (whose answer here also adds a
    // square), and draws the items it knows.
    let before = paths(&mut app);
    let ctx = egui::Context::default();
    let texts = frame_events(&ctx, &mut app, vec![], draw);
    for shown in ["Quick Shapes", "Adds a square.", "Add Square"] {
        assert!(texts.iter().any(|(t, _)| t == shown), "{shown} not drawn: {texts:?}");
    }
    assert!(!texts.iter().any(|(t, _)| t == "Not drawn"));
    assert_eq!(paths(&mut app), before + 1);
    // Drawn again, it shows what it was told: no other event.
    let texts = frame_events(&ctx, &mut app, vec![], draw);
    assert_eq!(paths(&mut app), before + 1);

    // A click on its button is an event, whose commands run as one undo step.
    click(&ctx, &mut app, text_rect(&texts, "Add Square").center(), draw);
    assert_eq!(paths(&mut app), before + 2);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(paths(&mut app), before + 1);

    // Opened again it is brought forward, not asked again.
    app.run("window.pluginPanel", json!({"id": ID})).unwrap();
    frame_events(&ctx, &mut app, vec![], draw);
    assert_eq!(paths(&mut app), before + 1);

    // Removed, its panel closes.
    app.run("plugin.remove", json!({"id": ID})).unwrap();
    plugin_panel::tidy(&mut app);
    assert!(!crate::panel_docking::layout_of(&app.ui).contains(&"plugin:test.ui.panel".to_string()));
}

#[test]
fn a_failed_event_says_why_and_fields_keep_their_values() {
    let mut app = app_with_rect();
    let id = "test.ui.declines";
    install(&mut app, id, &json!({"error": "Select a path first."}));
    app.run("window.pluginPanel", json!({"id": id})).unwrap();
    let ctx = egui::Context::default();
    let texts = frame_events(&ctx, &mut app, vec![], |app, ui| plugin_panel::show(app, ui, &plugin_panel::panel_id(id)));
    assert!(texts.iter().any(|(t, _)| t.contains("Select a path first.")), "{texts:?}");

    // Fields take the values the answer gives them; a checkbox clicked is a change of its own.
    let fields = json!([
        {"type": "string", "id": "name", "label": "Name", "value": "Leaf"},
        {"type": "int", "id": "count", "label": "Count", "value": 3, "min": 1, "max": 9},
        {"type": "choice", "id": "shape", "label": "Shape", "options": ["Square", "Circle"], "value": "Circle"},
        {"type": "row", "items": [{"type": "bool", "id": "outline", "label": "Outline"}]},
        {"type": "group", "label": "More", "items": [{"type": "separator"}]},
    ]);
    install(&mut app, id, &json!({"panel": {"items": fields}}));
    app.ui.plugin_panels.remove(id);
    let texts = frame_events(&ctx, &mut app, vec![], |app, ui| plugin_panel::show(app, ui, &plugin_panel::panel_id(id)));
    for shown in ["Name", "Leaf", "Count", "Shape", "Circle", "Outline", "More"] {
        assert!(texts.iter().any(|(t, _)| t == shown), "{shown} not drawn: {texts:?}");
    }
    let view = &app.ui.plugin_panels[id];
    assert_eq!(view.error, None);
    assert_eq!(Value::Object(view.values.clone()), json!({"name": "Leaf", "count": 3, "shape": "Circle"}));
    click(&ctx, &mut app, text_rect(&texts, "Outline").center(), |app, ui| plugin_panel::show(app, ui, &plugin_panel::panel_id(id)));
    // The answer gives the checkbox no value, so it keeps the one it was set to; the others take
    // the answer's again.
    let values = Value::Object(app.ui.plugin_panels[id].values.clone());
    assert_eq!(values, json!({"name": "Leaf", "count": 3, "shape": "Circle", "outline": true}));
}

#[test]
fn a_file_field_sends_the_file_picked_with_its_name_and_bytes() {
    use std::cell::RefCell;
    use std::rc::Rc;

    let mut app = app_with_rect();
    let id = "test.ui.file";
    let draw = |app: &mut VectorcraftApp, ui: &mut egui::Ui| plugin_panel::show(app, ui, &plugin_panel::panel_id(id));
    let file = json!({"type": "file", "id": "pick", "label": "Choose Data…", "extensions": ["csv", "txt", "No.Dots"]});
    install(&mut app, id, &json!({"panel": {"items": [file]}}));
    app.run("window.pluginPanel", json!({"id": id})).unwrap();
    let ctx = egui::Context::default();
    let texts = frame_events(&ctx, &mut app, vec![], draw);
    // From now on the panel answers only a file event that brings the file's name and bytes; any
    // other event fails and says so under it.
    let csv = b"name,Title\nOne,Alpha\n";
    let needle = format!(r#""dataBase64":"{}","name":"data.csv""#, vectorcraft_format::base64_encode(csv));
    let read = json!({"panel": {"items": [file, {"type": "text", "text": "Read data.csv"}]}});
    let src = tpl::when_given(&tpl::manifest(id, "panel", "{}"), tpl::Given::Input, &needle, &read.to_string());
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap())})).unwrap();

    // Without an open dialog (as on the web), the field is off: a click sends nothing.
    click(&ctx, &mut app, text_rect(&texts, "Choose Data…").center(), draw);
    assert_eq!(app.ui.plugin_panels[id].error, None);

    // With one, it offers the field's file types, and the file picked comes with its name and
    // bytes, not its folder.
    let offered = Rc::new(RefCell::new(vec![]));
    let seen = offered.clone();
    app.services.pick_open = Some(Box::new(move |pick: &crate::FilePick| {
        seen.borrow_mut().push(pick.filters.clone());
        Some("/somewhere/data.csv".into())
    }));
    app.services.read = Some(Box::new(|_: &str| Ok(b"name,Title\nOne,Alpha\n".to_vec())));
    click(&ctx, &mut app, text_rect(&texts, "Choose Data…").center(), draw);
    assert_eq!(app.ui.plugin_panels[id].error, None);
    assert_eq!(*offered.borrow(), vec![vec![crate::FilePick::filter("CSV, TXT", &["csv", "txt"])]]);
    let texts = frame_events(&ctx, &mut app, vec![], draw);
    assert!(texts.iter().any(|(t, _)| t == "Read data.csv"), "{texts:?}");

    // A file larger than a panel takes is not sent: the panel says why.
    app.services.read = Some(Box::new(|_: &str| Ok(vec![0; vectorcraft_plugins::panel::MAX_FILE + 1])));
    click(&ctx, &mut app, text_rect(&texts, "Choose Data…").center(), draw);
    let error = app.ui.plugin_panels[id].error.clone().unwrap_or_default();
    assert!(error.contains("larger than 4 MiB"), "{error}");
}
