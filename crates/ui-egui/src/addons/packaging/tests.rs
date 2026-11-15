use serde_json::json;
use vectorcraft_engine::Session;

use super::{ID, ROLES, SET_ROLE, SET_ROLE_SWATCH, show};
use crate::tests_labels::painted_text;
use crate::{VectorcraftApp, menus};

/// An app with two rectangles, nothing selected → (app, their ids).
fn app() -> (VectorcraftApp, u64, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let rect = |app: &mut VectorcraftApp, x: f64| {
        app.run("shape.rectangle", json!({"x": x, "y": 20, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap()
    };
    let (a, b) = (rect(&mut app, 20.0), rect(&mut app, 100.0));
    app.run("select.none", json!({})).unwrap();
    (app, a, b)
}

#[test]
fn the_plugins_submenu_opens_the_panel_and_makes_each_kind_of_line() {
    let (mut app, a, _) = app();
    let entries = menus::menu_entries(&app);
    let under = |label: &str| entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Packaging Tools"] && e.label == label);
    let item = under("Fold Lines").expect("listed");
    assert_eq!((item.command.as_deref(), &item.params), (Some("window.panel"), &json!({ "panel": ID })));
    for (label, role) in
        [("Make Cut Line", "cut"), ("Make Crease Line", "crease"), ("Make Partial Cut Line", "partialCut"), ("Make Perforation Line", "perforation")]
    {
        let e = under(label).unwrap_or_else(|| panic!("{label} listed"));
        assert_eq!((e.command.as_deref(), &e.params), (Some(SET_ROLE), &json!({ "role": role })), "{label}");
    }
    assert!(!crate::panel_docking::showing(&app.ui, ID));
    menus::invoke(&mut app, "window.panel", item.params.clone());
    assert!(crate::panel_docking::showing(&app.ui, ID), "the panel opens");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let make = under("Make Partial Cut Line").unwrap();
    let (target, p) = menus::click_target(&make.label, SET_ROLE, &make.params);
    menus::invoke(&mut app, &target, p);
    let r = app.session.execute("addon.packaging.roles", &json!({"ids": [a]})).unwrap();
    assert_eq!(r["roles"][2]["count"], json!(1), "{r}");
}

#[test]
fn the_panel_counts_the_roles_and_says_when_nothing_is_selected() {
    let (mut app, a, b) = app();
    app.run(SET_ROLE, json!({"role": "cut", "ids": [a, b]})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let text = painted_text(&mut app, show);
    assert!(text.contains("No objects are selected."), "{text}");
    for (label, _) in ROLES {
        assert!(text.contains(label), "{label}: {text}");
    }
    assert!(text.contains('2'), "the document's two cut lines: {text}");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let text = painted_text(&mut app, show);
    assert!(!text.contains("No objects are selected."), "{text}");
    assert!(text.contains('1') && !text.contains('2'), "the selection's one cut line: {text}");
}

/// A click on the panel row that screen readers name `label`. egui finds what the pointer is over
/// from the frame before, so the move gets a frame of its own.
fn click_row(app: &mut VectorcraftApp, label: &str) {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    crate::theme::install_fonts(&ctx);
    let mut time = 0.0;
    let mut frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        time += 0.1;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0));
        let input = egui::RawInput { events, time: Some(time), screen_rect: Some(screen), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| show(app, ui));
        out.textures_delta.clear();
        out
    };
    frame(app, vec![]);
    let out = frame(app, vec![]);
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
    let node = nodes.iter().find(|(_, n)| n.label() == Some(label));
    let b = node.unwrap_or_else(|| panic!("no row named {label:?}")).1.bounds().expect("bounds");
    let at = egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(app, vec![egui::Event::PointerMoved(at)]);
    frame(app, vec![button(true)]);
    frame(app, vec![button(false)]);
}

#[test]
fn clicking_a_row_gives_the_selection_that_role_and_does_nothing_without_one() {
    let (mut app, a, b) = app();
    click_row(&mut app, "Crease Line");
    let none = app.session.execute("addon.packaging.roles", &json!({"ids": [a, b]})).unwrap();
    assert_eq!(none["roles"][1]["count"], json!(0), "nothing selected: {none}");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    click_row(&mut app, "Crease Line");
    let r = app.session.execute("addon.packaging.roles", &json!({"ids": [a, b]})).unwrap();
    assert_eq!(r["roles"][1]["count"], json!(1), "{r}");
}

#[test]
fn role_swatches_opens_the_options_of_its_command() {
    let (target, p) = menus::click_target("Role Swatches…", SET_ROLE_SWATCH, &json!({"role": "cut", "swatch": "Cut"}));
    assert_eq!((target.as_str(), &p["command"]), ("ui.paramDialog", &json!(SET_ROLE_SWATCH)));
    let doc = vectorcraft_engine::find_command(SET_ROLE_SWATCH).unwrap().params;
    let ids: Vec<&str> = ROLES.iter().map(|(_, id)| *id).collect();
    assert!(doc.contains(&format!("role: {}", ids.join("|"))), "{doc}");
}

#[test]
fn role_swatches_shows_the_swatch_of_the_role_picked() {
    let (mut app, a, _) = app();
    app.run(SET_ROLE, json!({"role": "crease", "ids": [a]})).unwrap();
    app.run("swatch.edit", json!({"name": "Crease", "newName": "Score"})).unwrap();
    let mut d = crate::state::Dialog::new("command", json!({"role": "cut", "swatch": "Cut"}));
    super::follow_role(&mut app, &mut d);
    assert_eq!(d.str("swatch"), "Cut");
    d.fields.insert("role".into(), json!("crease"));
    super::follow_role(&mut app, &mut d);
    assert_eq!(d.str("swatch"), "Score", "the Crease role's swatch");
    // A swatch typed for the same role stays.
    d.fields.insert("swatch".into(), json!("Ink"));
    super::follow_role(&mut app, &mut d);
    assert_eq!(d.str("swatch"), "Ink");
    assert_eq!(crate::dialogs::form::params(&d), json!({"role": "crease", "swatch": "Ink"}), "the dialog sends no __role");
}

#[test]
fn the_panel_without_a_document_asks_for_one() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let text = painted_text(&mut app, show);
    assert!(text.contains("No document"), "{text}");
}
