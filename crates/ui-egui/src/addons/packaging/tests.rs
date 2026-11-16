use serde_json::json;
use vectorcraft_engine::Session;

use super::{CHECK, ID, ROLES, SET_ROLE, SET_ROLE_SWATCH, show};
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

/// An app with a square blank cut on its outline, a crease across it and a cut line that stops
/// inside it, nothing selected → (app, the cut line that stops).
fn blank() -> (VectorcraftApp, u64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    let path = |app: &mut VectorcraftApp, d: &str| app.run("path.create", json!({ "d": d })).unwrap()["id"].as_u64().unwrap();
    let outline = path(&mut app, "M50 50 L250 50 L250 250 L50 250 Z");
    let crease = path(&mut app, "M50 150 L250 150");
    let slit = path(&mut app, "M100 50 L100 100");
    app.run(SET_ROLE, json!({"role": "cut", "ids": [outline, slit]})).unwrap();
    app.run(SET_ROLE, json!({"role": "crease", "ids": [crease]})).unwrap();
    app.run("select.none", json!({})).unwrap();
    (app, slit)
}

/// Frames of the panel in one context, the pointer clicking the middle of the element that screen
/// readers name `click` (egui finds what the pointer is over from the frame before, so the move
/// gets a frame of its own) → the text the last frame painted.
fn frames(app: &mut VectorcraftApp, ctx: &egui::Context, click: Option<&str>) -> String {
    ctx.enable_accesskit();
    crate::theme::install_fonts(ctx);
    // The context's clock runs on from its last frame: egui requires time to move forward.
    let mut time = ctx.input(|i| i.time);
    let mut frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        time += 0.1;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(320.0, 480.0));
        let input = egui::RawInput { events, time: Some(time), screen_rect: Some(screen), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| show(app, ui));
        out.textures_delta.clear();
        out
    };
    frame(app, vec![]);
    let out = frame(app, vec![]);
    let Some(label) = click else { return crate::tests_labels::shapes_text(&out) };
    let nodes = &out.platform_output.accesskit_update.as_ref().expect("an accessibility tree").nodes;
    // A button carries its text as its name, a label as its value.
    let node = nodes.iter().find(|(_, n)| n.label() == Some(label) || n.value() == Some(label));
    let b = node.unwrap_or_else(|| panic!("nothing named {label:?}")).1.bounds().expect("bounds");
    let at = egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    frame(app, vec![egui::Event::PointerMoved(at)]);
    frame(app, vec![button(true)]);
    let out = frame(app, vec![button(false)]);
    crate::tests_labels::shapes_text(&out)
}

#[test]
fn check_fold_lines_lists_its_issues_in_the_panel_until_the_document_changes() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let (mut app, _) = blank();
    let entries = menus::menu_entries(&app);
    let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Packaging Tools"] && e.label == "Check Fold Lines").expect("listed");
    assert_eq!(item.command.as_deref(), Some(CHECK));
    let ctx = egui::Context::default();
    app.ui_ctx = Some(ctx.clone());
    assert!(!crate::panel_docking::showing(&app.ui, ID));
    menus::invoke(&mut app, CHECK, item.params.clone());
    assert!(crate::panel_docking::showing(&app.ui, ID), "the panel opens");
    assert_eq!(app.ui.status, "Panels: 2, hinges: 1, errors: 1, warnings: 0");
    let text = frames(&mut app, &ctx, None);
    assert!(text.contains("A cut line ends without meeting another cut line."), "{text}");
    assert!(text.contains("Panels: 2, hinges: 1, errors: 1, warnings: 0") && text.contains("Select"), "{text}");
    // Any change to the document takes the list away.
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    let text = frames(&mut app, &ctx, None);
    assert!(!text.contains("A cut line ends"), "{text}");
}

#[test]
fn the_check_button_checks_and_clicking_an_issue_selects_its_lines() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let (mut app, slit) = blank();
    let ctx = egui::Context::default();
    let text = frames(&mut app, &ctx, Some("Check"));
    assert!(text.contains("A cut line ends without meeting another cut line."), "{text}");
    assert!(app.session.active().unwrap().selection.is_empty());
    frames(&mut app, &ctx, Some("A cut line ends without meeting another cut line."));
    let selected: Vec<u64> = app.session.active().unwrap().selection.objects.iter().map(|id| id.0).collect();
    assert_eq!(selected, [slit]);
    let text = frames(&mut app, &ctx, None);
    assert!(text.contains("A cut line ends without meeting another cut line."), "selecting keeps the list: {text}");
    // A clean blank: no issues.
    app.run("edit.clear", json!({})).unwrap();
    let text = frames(&mut app, &ctx, Some("Check"));
    assert!(text.contains("No issues found.") && text.contains("Panels: 2, hinges: 1, errors: 0, warnings: 0"), "{text}");
}

#[test]
fn issues_and_the_summary_show_in_the_interface_language() {
    let (mut app, _) = blank();
    let ctx = egui::Context::default();
    crate::i18n::set_current(crate::i18n::Lang::from_code("de").unwrap());
    let text = frames(&mut app, &ctx, Some(crate::i18n::t("Check")));
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert!(!text.contains("A cut line ends") && !text.contains("Panels:"), "{text}");
    assert!(text.contains("Eine Schnittlinie endet, ohne auf eine andere Schnittlinie zu treffen."), "{text}");
    assert!(text.contains("Felder: 2, Scharniere: 1, Fehler: 1, Warnungen: 0"), "{text}");
}

/// The messages `addon.packaging.check` gives its issues, one for each code (three for `noCut`).
const ISSUE_MESSAGES: [&str; 17] = [
    "No cut lines are in the selection.",
    "No cut lines are in the document.",
    "No cut lines are among the given objects.",
    "The cut lines do not enclose an area.",
    "A cut line ends without meeting another cut line.",
    "A gap between cut lines was closed.",
    "The fold lines are too complex to check.",
    "A cut line and a crease lie on top of each other.",
    "Two fold lines lie on top of each other.",
    "A crease ends inside a panel.",
    "A crease does not divide two panels.",
    "A crease lies outside the cut outline, wholly or in part.",
    "The pieces of a hinge are not on one straight line.",
    "A gap between cut lines is too narrow to tell apart.",
    "A cut-out area holds other fold lines or another blank.",
    "A panel is not joined to the rest by a crease.",
    "A fold line is on a non-printing layer.",
];

#[test]
fn every_issue_message_translates_in_every_complete_message_catalog() {
    for code in [
        "cs", "de", "el", "es", "fr", "hi", "hu", "it", "ja", "ka", "ko", "pl", "pt-br", "ru", "sv", "uk", "vi", "zh-hans", "zh-hant", "fi", "tr",
        "ca", "nl", "da",
    ] {
        let lang = crate::i18n::Lang::from_code(code).unwrap();
        for m in ISSUE_MESSAGES.into_iter().chain(["Panels: 3, hinges: 2, errors: 1, warnings: 0"]) {
            assert_ne!(crate::i18n::message(lang, m), m, "{code}: {m}");
        }
    }
}

#[test]
fn the_panel_without_a_document_asks_for_one() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let text = painted_text(&mut app, show);
    assert!(text.contains("No document"), "{text}");
}
