use serde_json::json;
use vectorcraft_engine::Session;

use super::{CHECK, ID, ROLES, SET_FOLD_ANGLE, SET_ROLE, SET_ROLE_SWATCH, show};
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
fn each_document_keeps_its_own_check() {
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let (mut app, _) = blank();
    let ctx = egui::Context::default();
    let text = frames(&mut app, &ctx, Some("Check"));
    assert!(text.contains("A cut line ends without meeting another cut line."), "{text}");
    // A second document, checked too: it has no cut lines.
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let text = frames(&mut app, &ctx, Some("Check"));
    assert!(text.contains("No cut lines are in the document.") && !text.contains("A cut line ends"), "{text}");
    // Back on the first document, unchanged since its check: its issues are still listed.
    assert!(app.session.set_active(0));
    let text = frames(&mut app, &ctx, None);
    assert!(text.contains("A cut line ends without meeting another cut line."), "{text}");
    assert!(text.contains("Panels: 2, hinges: 1, errors: 1, warnings: 0"), "{text}");
    // And the second document's check stays its own.
    assert!(app.session.set_active(1));
    let text = frames(&mut app, &ctx, None);
    assert!(text.contains("No cut lines are in the document.") && !text.contains("A cut line ends"), "{text}");
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
        let no_fold_line = "select a crease, partial cut or perforation line that is neither locked nor hidden";
        for m in ISSUE_MESSAGES.into_iter().chain(["Panels: 3, hinges: 2, errors: 1, warnings: 0", no_fold_line]) {
            assert_ne!(crate::i18n::message(lang, m), m, "{code}: {m}");
        }
    }
}

#[test]
fn set_fold_angle_opens_its_dialog_on_the_selected_lines_angle() {
    let (mut app, a, b) = app();
    app.run(SET_ROLE, json!({"role": "crease", "ids": [a]})).unwrap();
    app.run(SET_ROLE, json!({"role": "cut", "ids": [b]})).unwrap();
    let entries = menus::menu_entries(&app);
    let item = entries.iter().find(|e| e.path == ["Object", "Plug-ins", "Packaging Tools"] && e.label == "Set Fold Angle…").expect("listed");
    assert_eq!(item.command.as_deref(), Some(SET_FOLD_ANGLE));
    let opened = |app: &mut VectorcraftApp| {
        let (target, p) = menus::click_target(&item.label, SET_FOLD_ANGLE, &item.params);
        menus::invoke(app, &target, p);
        let d = app.ui.dialog.take().expect("a dialog");
        assert_eq!((d.kind.as_str(), d.str("__command")), ("command", SET_FOLD_ANGLE.to_string()));
        d.fields["angle"].as_f64()
    };
    app.run("select.set", json!({"ids": [a]})).unwrap();
    assert_eq!(opened(&mut app), Some(90.0), "a crease folds at 90° until it has an angle of its own");
    app.run(SET_FOLD_ANGLE, json!({"angle": 45})).unwrap();
    assert_eq!(opened(&mut app), Some(45.0));
    // The cut line does not fold, so the crease's angle is still the one shown.
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    assert_eq!(opened(&mut app), Some(45.0));
    // Lines that differ open on 90°.
    let c = app.run("shape.rectangle", json!({"x": 180, "y": 20, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    app.run(SET_ROLE, json!({"role": "partialCut", "ids": [c]})).unwrap();
    app.run("select.set", json!({"ids": [a, c]})).unwrap();
    assert_eq!(opened(&mut app), Some(90.0));
    // With only a cut line selected there is nothing to set: no dialog opens.
    app.run("select.set", json!({"ids": [b]})).unwrap();
    let (target, p) = menus::click_target(&item.label, SET_FOLD_ANGLE, &item.params);
    menus::invoke(&mut app, &target, p);
    assert!(app.ui.dialog.is_none());
    // ui.menuDialog gives the command's reason.
    let e = app.run("ui.menuDialog", json!({"command": SET_FOLD_ANGLE})).unwrap_err();
    assert!(e.contains("select a crease, partial cut or perforation line"), "{e}");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    app.run("ui.menuDialog", json!({"command": SET_FOLD_ANGLE})).unwrap();
    assert_eq!(app.ui.dialog.take().expect("a dialog").fields["angle"].as_f64(), Some(45.0));
    // The options dialog opened with an angle keeps it.
    menus::invoke(&mut app, "ui.paramDialog", json!({"command": SET_FOLD_ANGLE, "label": "Set Fold Angle", "params": {"angle": 30}}));
    assert_eq!(app.ui.dialog.take().expect("a dialog").fields["angle"].as_f64(), Some(30.0));
}

/// Set Fold Angle…'s OK changes the lines the dialog opened on, even when the selection changes
/// while it is open (a remote command can change it): the other crease keeps its angle.
#[test]
fn set_fold_angle_s_ok_changes_the_lines_the_dialog_opened_on() {
    let (mut app, a, b) = app();
    app.run(SET_ROLE, json!({"role": "crease", "ids": [a, b]})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    menus::invoke(&mut app, SET_FOLD_ANGLE, json!({}));
    assert!(app.ui.dialog.is_some(), "the dialog opens");
    app.run("select.set", json!({"ids": [b]})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("angle".into(), json!(30.0));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "OK closes the dialog");
    let angle = |app: &mut VectorcraftApp, id: u64| {
        app.run("select.set", json!({"ids": [id]})).unwrap();
        vectorcraft_engine::cmd::selection_fold_angles(&app.session).angle
    };
    assert_eq!(angle(&mut app, a), Some(30.0), "the line the dialog opened on");
    assert_eq!(angle(&mut app, b), Some(90.0), "the line selected later keeps its role's angle");
}

/// Set Fold Angle…'s OK changes nothing once another document is the active one, even a line there
/// with the same id as the line the dialog opened on.
#[test]
fn set_fold_angle_s_ok_changes_nothing_in_another_document() {
    let (mut app, a, _) = app();
    app.run(SET_ROLE, json!({"role": "crease", "ids": [a]})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    menus::invoke(&mut app, SET_FOLD_ANGLE, json!({}));
    assert!(app.ui.dialog.is_some(), "the dialog opens");
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let other = app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(other, a, "the same id in the other document");
    app.run(SET_ROLE, json!({"role": "crease", "ids": [other]})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("angle".into(), json!(30.0));
    crate::dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "OK closes the dialog");
    let angle = |app: &mut VectorcraftApp| {
        app.run("select.set", json!({"ids": [a]})).unwrap();
        vectorcraft_engine::cmd::selection_fold_angles(&app.session).angle
    };
    assert_eq!(angle(&mut app), Some(90.0), "the other document's line keeps its role's angle");
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!(angle(&mut app), Some(90.0), "the first document's line is left as it was");
}

/// Set Fold Angle… keeps the angle typed into its dialog within -180° to 180°, so OK applies the
/// nearest end instead of closing on an angle the command refuses.
#[test]
fn set_fold_angle_s_dialog_keeps_the_angle_within_its_range() {
    use egui::accesskit::Role;
    use egui_kittest::kittest::Queryable;
    let (mut app, a, _) = app();
    app.run(SET_ROLE, json!({"role": "crease", "ids": [a]})).unwrap();
    app.run("select.set", json!({"ids": [a]})).unwrap();
    menus::invoke(&mut app, SET_FOLD_ANGLE, json!({}));
    assert!(app.ui.dialog.is_some(), "the dialog opens");
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1100.0, 900.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            crate::dialogs::show(app, ui.ctx());
        },
        app,
    );
    h.run_steps(5);
    let all = egui::Modifiers { ctrl: true, command: true, ..Default::default() };
    h.get_by_role_and_label(Role::TextInput, "Angle:").focus();
    h.run_steps(2);
    h.key_press_modifiers(all, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, "Angle:").type_text("181");
    h.run_steps(2);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_ref().expect("still open").fields["angle"].as_f64(), Some(180.0));
    crate::dialogs::confirm(h.state_mut()).unwrap();
    let r = h.state_mut().session.execute("addon.packaging.roles", &json!({"ids": [a]})).unwrap();
    assert_eq!(r["foldAngle"].as_f64(), Some(180.0), "{r}");
    // An angle typed with more places than the field shows is kept as typed, as the panel keeps it.
    menus::invoke(h.state_mut(), SET_FOLD_ANGLE, json!({}));
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, "Angle:").focus();
    h.run_steps(2);
    h.key_press_modifiers(all, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, "Angle:").type_text("179.9999");
    h.run_steps(2);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_ref().expect("still open").fields["angle"].as_f64(), Some(179.9999));
    crate::dialogs::confirm(h.state_mut()).unwrap();
    let r = h.state_mut().session.execute("addon.packaging.roles", &json!({"ids": [a]})).unwrap();
    assert_eq!(r["foldAngle"].as_f64(), Some(179.9999), "{r}");
    // Opened on an angle with more places than that, the dialog shows them all, so typing 180 sets 180.
    h.state_mut().run(SET_FOLD_ANGLE, json!({"angle": 179.999999, "ids": [a]})).unwrap();
    menus::invoke(h.state_mut(), SET_FOLD_ANGLE, json!({}));
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, "Angle:").focus();
    h.run_steps(2);
    h.key_press_modifiers(all, egui::Key::A);
    h.get_by_role_and_label(Role::TextInput, "Angle:").type_text("180");
    h.run_steps(2);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
    crate::dialogs::confirm(h.state_mut()).unwrap();
    let r = h.state_mut().session.execute("addon.packaging.roles", &json!({"ids": [a]})).unwrap();
    assert_eq!(r["foldAngle"].as_f64(), Some(180.0), "{r}");
    // A fine step from a whole angle steps a tenth of a degree.
    h.state_mut().run(SET_FOLD_ANGLE, json!({"angle": 90, "ids": [a]})).unwrap();
    menus::invoke(h.state_mut(), SET_FOLD_ANGLE, json!({}));
    h.run_steps(3);
    h.get_by_role_and_label(Role::TextInput, "Angle:").focus();
    h.run_steps(2);
    h.key_press_modifiers(all, egui::Key::ArrowUp);
    h.run_steps(3);
    assert_eq!(h.state().ui.dialog.as_ref().expect("still open").fields["angle"].as_f64(), Some(90.1));
}

#[test]
fn a_fold_angle_field_shows_every_place_the_angle_has() {
    use super::angle_places;
    assert_eq!([None, Some(90.0), Some(-135.5), Some(179.9999999), Some(1e-12)].map(angle_places), [1, 1, 1, 7, 12]);
}

#[test]
fn the_panel_shows_and_resets_the_selected_lines_fold_angle() {
    let (mut app, a, b) = app();
    app.run(SET_ROLE, json!({"role": "crease", "ids": [a]})).unwrap();
    app.run(SET_ROLE, json!({"role": "cut", "ids": [b]})).unwrap();
    app.run("select.set", json!({"ids": [b]})).unwrap();
    let text = painted_text(&mut app, show);
    assert!(!text.contains("Fold Angle"), "a cut line has no fold angle: {text}");
    app.run("select.set", json!({"ids": [a]})).unwrap();
    app.run(SET_FOLD_ANGLE, json!({"angle": 135})).unwrap();
    let text = painted_text(&mut app, show);
    assert!(text.contains("Fold Angle") && text.contains("135"), "{text}");
    click_row(&mut app, "Reset");
    let r = app.session.execute("addon.packaging.roles", &json!({"ids": [a]})).unwrap();
    assert_eq!((r["foldAngle"].as_f64(), r["ownAngles"].as_u64()), (Some(90.0), Some(0)), "{r}");
    // Stored text that is not an angle: the field shows the role's angle, and Reset removes the text.
    app.session
        .edit("Test", |d, _| {
            if let Some(n) = d.node_mut(vectorcraft_doc::NodeId(a)) {
                n.edit_attrs(|at| at.data.push(("vc-fold-angle".into(), "wide".into())));
            }
            Ok(())
        })
        .unwrap();
    let r = app.session.execute("addon.packaging.roles", &json!({"ids": [a]})).unwrap();
    assert_eq!((r["foldAngle"].as_f64(), r["ownAngles"].as_u64(), r["storedAngles"].as_u64()), (Some(90.0), Some(0), Some(1)), "{r}");
    click_row(&mut app, "Reset");
    let r = app.session.execute("addon.packaging.roles", &json!({"ids": [a]})).unwrap();
    assert_eq!(r["storedAngles"].as_u64(), Some(0), "{r}");
}

#[test]
fn the_panel_without_a_document_asks_for_one() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let text = painted_text(&mut app, show);
    assert!(text.contains("No document"), "{text}");
}
