//! Perspective Grid dialogs: Define Grid, the presets manager and the tool options.

use craft::ui::dialog::{DialogHost, HeldOk, dialog_id};
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_tools::distort::perspective::PerspectiveGrid;

use super::perspective_grid::GridDialog;
use crate::VectorcraftApp;

/// One whole app frame, then the pump of the dialogs, which takes the answers the dialogs filed.
fn frame(app: &mut VectorcraftApp) {
    let ctx = window(app);
    crate::tests::synthetic::key_frame(app, &ctx, vec![]);
    super::host::pump(app, &ctx);
}

fn window(app: &VectorcraftApp) -> egui::Context {
    app.ui_ctx.clone().expect("the test app has a window context")
}

fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    app
}

/// Runs `f` on the window's dialog host.
fn host<R>(app: &VectorcraftApp, f: impl FnOnce(&mut DialogHost) -> R) -> R {
    app.dialogs().expect("the test app has a window").with(f)
}

/// The kind of the dialog on top.
fn kind(app: &VectorcraftApp) -> Option<String> {
    host(app, |h| h.open_dialogs().pop().and_then(|d| d.key))
}

/// A field of the dialog on top.
fn field(app: &VectorcraftApp, k: &str) -> Value {
    host(app, |h| h.open_dialogs().pop().and_then(|d| d.fields).and_then(|f| f.get(k).cloned())).unwrap_or_default()
}

/// Sets a field of the dialog on top, as a script does.
fn set(app: &mut VectorcraftApp, k: &str, v: Value) -> Result<(), String> {
    let ctx = window(app);
    host(app, |h| {
        let id = h.open_dialogs().pop().ok_or("no dialog is open")?.id;
        h.set_json_field(&ctx, id, k, v)
    })
}

/// OK on the dialog on top: files its answer, which the pump runs and a later pass closes the dialog with.
fn ok(app: &mut VectorcraftApp) {
    let ctx = window(app);
    host(app, |h| {
        let id = h.open_dialogs().pop().expect("a dialog is open").id;
        h.confirm_by_id(&ctx, id)
    });
    frame(app);
    frame(app);
}

fn grid(app: &VectorcraftApp) -> PerspectiveGrid {
    PerspectiveGrid::effective(&app.session.active().unwrap().doc)
}

#[test]
fn define_grid_opens_on_the_grid_and_applies_its_fields() {
    let mut app = app();
    app.run("perspective.grid.preset", json!({"kind": 3})).unwrap();
    let g = grid(&app);
    app.run("ui.perspectiveGridDialog", json!({})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some(super::perspective_grid::KIND));
    assert_eq!(field(&app, "kind"), json!(3), "prefilled from the grid, not a fixed type");
    assert!((field(&app, "gridline").as_f64().unwrap() - g.cell).abs() < 1e-9);
    frame(&mut app);
    // A new unit converts the lengths shown.
    set(&mut app, "units", json!("inches")).unwrap();
    frame(&mut app);
    assert!((field(&app, "gridline").as_f64().unwrap() - g.cell / 72.0).abs() < 1e-9);
    // OK with nothing changed leaves the grid as it was.
    ok(&mut app);
    assert_eq!(kind(&app), None);
    assert_eq!((grid(&app).cell, grid(&app).vp_left, grid(&app).kind), (g.cell, g.vp_left, 3));
    // A new viewing angle and colour apply in one undo step.
    let undo = app.session.active().unwrap().history.undo.len();
    app.run("ui.perspectiveGridDialog", json!({})).unwrap();
    set(&mut app, "angle", json!(30)).unwrap();
    set(&mut app, "rightColor", json!("#112233")).unwrap();
    frame(&mut app);
    ok(&mut app);
    let n = grid(&app);
    assert!((n.viewing_angle() - 30.0).abs() < 1e-9);
    assert_eq!(n.right_color.hex(), "#112233");
    assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
    // A refused value leaves the dialog open and the grid as it was.
    app.run("ui.perspectiveGridDialog", json!({})).unwrap();
    let distance = field(&app, "distance");
    assert!(set(&mut app, "distance", json!(-5)).is_err());
    assert_eq!(field(&app, "distance"), distance);
    assert_eq!(kind(&app).as_deref(), Some(super::perspective_grid::KIND));
    // OK on what is left defines the grid as it was.
    ok(&mut app);
    assert_eq!((grid(&app).cell, grid(&app).viewing_angle()), (n.cell, n.viewing_angle()));
}

#[test]
fn define_grid_applies_only_to_the_document_where_it_opened() {
    let mut app = app();
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    let destination = app.session.active().unwrap();
    let destination_uid = destination.uid;
    let destination_undo = destination.history.undo.len();
    let destination_grid = grid(&app);
    app.run("document.activate", json!({"index": 0})).unwrap();
    let source_uid = app.session.active().unwrap().uid;
    app.run("ui.perspectiveGridDialog", json!({})).unwrap();
    set(&mut app, "rightColor", json!("#112233")).unwrap();
    let ctx = window(&app);
    let id = dialog_id(super::perspective_grid::KIND);
    host(&app, |h| {
        assert!(!h.confirm_by_id(&ctx, id));
        assert_eq!(h.held_state(id), Some(HeldOk::Waiting));
    });

    app.run("document.activate", json!({"index": 1})).unwrap();
    super::perspective_grid::pump(&mut app, &ctx);
    assert_eq!(app.session.active().unwrap().uid, destination_uid);
    assert_eq!(grid(&app), destination_grid, "the first document's draft leaves the active grid alone");
    assert_eq!(app.session.active().unwrap().history.undo.len(), destination_undo);
    assert!(matches!(host(&app, |h| h.held_state(id)), Some(HeldOk::Refused(_))));

    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!(app.session.active().unwrap().uid, source_uid);
    ok(&mut app);
    assert_eq!(kind(&app), None);
    assert_eq!(grid(&app).right_color.hex(), "#112233");
}

#[test]
fn define_grid_loads_and_saves_presets() {
    let mut app = app();
    app.run("ui.perspectiveGridDialog", json!({})).unwrap();
    frame(&mut app);
    assert_eq!(field(&app, "name"), json!("[2P-Normal View]"));
    // Choosing a preset keeps its name while the fields are its; a changed field makes the fields [Custom].
    set(&mut app, "angle", json!(20)).unwrap();
    frame(&mut app);
    assert_eq!(field(&app, "name"), json!(""));
    // Save Preset… asks for a name; OK saves the fields and comes back to Define Grid on it.
    let id = dialog_id(super::perspective_grid::KIND);
    host(&app, |h| h.get_mut_by_id::<GridDialog>(id).expect("Define Grid is open").start_save());
    set(&mut app, "name", json!("Shallow")).unwrap();
    frame(&mut app);
    ok(&mut app);
    assert_eq!(kind(&app).as_deref(), Some(super::perspective_grid::KIND));
    assert_eq!(field(&app, "name"), json!("Shallow"));
    assert_eq!(app.session.prefs.perspective_presets[0].angle, 20.0);
    assert_eq!(host(&app, |h| h.held_state(id)), None, "back in Define Grid, nothing is held");
    // The fields are the saved preset's, and OK now names the grid.
    frame(&mut app);
    assert_eq!(field(&app, "name"), json!("Shallow"), "the fields are the saved preset's");
    ok(&mut app);
    assert_eq!(grid(&app).name, "Shallow");
    assert_eq!(app.session.prefs.perspective_presets.len(), 1, "Define Grid saves no preset");
    // View › Perspective Grid › Save Grid as Preset…: the grid under a new name.
    app.run("ui.savePerspectivePreset", json!({})).unwrap();
    assert_eq!(field(&app, "name"), json!("Perspective Preset 1"));
    frame(&mut app);
    ok(&mut app);
    assert_eq!(kind(&app), None);
    assert_eq!(app.session.prefs.perspective_presets.len(), 2);
    // A name the presets cannot take is refused: the dialog stays, and says why.
    app.run("ui.savePerspectivePreset", json!({})).unwrap();
    set(&mut app, "name", json!("[2P-Normal View]")).unwrap();
    let id = dialog_id(super::perspective_grid::KIND);
    ok(&mut app);
    assert_eq!(kind(&app).as_deref(), Some(super::perspective_grid::KIND));
    assert!(matches!(host(&app, |h| h.held_state(id)), Some(HeldOk::Refused(_))));
    assert_eq!(app.session.prefs.perspective_presets.len(), 2);
}

/// Enter pressed in the field that has the focus, then the passes that run the filed answer and close the dialog.
fn enter(app: &mut VectorcraftApp) {
    let ctx = window(app);
    let key = |pressed| egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    crate::tests::synthetic::key_frame(app, &ctx, vec![key(true)]);
    crate::tests::synthetic::key_frame(app, &ctx, vec![key(false)]);
    frame(app);
    frame(app);
}

#[test]
fn enter_is_ok_in_define_grid_and_save_grid_as_preset() {
    let mut app = app();
    // Define Grid: Enter in the focused field defines the grid and closes the dialog.
    app.run("ui.perspectiveGridDialog", json!({})).unwrap();
    set(&mut app, "angle", json!(30)).unwrap();
    frame(&mut app);
    frame(&mut app);
    assert!(window(&app).memory(|m| m.focused()).is_some(), "a field of the dialog has the focus");
    enter(&mut app);
    assert_eq!(kind(&app), None, "Enter closes Define Grid");
    assert!((grid(&app).viewing_angle() - 30.0).abs() < 1e-9, "Enter defines the grid");
    // Save Grid as Preset: Enter in the name field saves the preset and closes the dialog.
    app.run("ui.savePerspectivePreset", json!({})).unwrap();
    frame(&mut app);
    frame(&mut app);
    assert!(window(&app).memory(|m| m.focused()).is_some(), "a field of the dialog has the focus");
    enter(&mut app);
    assert_eq!(kind(&app), None, "Enter closes Save Grid as Preset");
    assert_eq!(app.session.prefs.perspective_presets.len(), 1, "Enter saves the preset");
    // A name the presets refuse: Enter files it, the refusal shows, and the dialog stays.
    app.run("ui.savePerspectivePreset", json!({})).unwrap();
    set(&mut app, "name", json!("[2P-Normal View]")).unwrap();
    frame(&mut app);
    frame(&mut app);
    enter(&mut app);
    assert_eq!(kind(&app).as_deref(), Some(super::perspective_grid::KIND), "a refused name keeps the dialog open");
    assert_eq!(app.session.prefs.perspective_presets.len(), 1);
}

#[test]
fn the_presets_manager_creates_edits_and_deletes_presets() {
    use super::perspective_presets::{self, Action};
    let mut app = app();
    app.run("ui.perspectivePresetsDialog", json!({"selected": "[3P-Normal View]"})).unwrap();
    frame(&mut app);
    let press = |app: &mut VectorcraftApp, act: Action, current: &str| {
        perspective_presets::run(app, act, current).unwrap();
        frame(app);
    };
    // New… opens the editor on a copy of the selected preset; its OK comes back here.
    press(&mut app, Action::New, "[3P-Normal View]");
    assert_eq!((kind(&app).as_deref(), field(&app, "kind")), (Some(super::perspective_grid::KIND), json!(3)));
    set(&mut app, "name", json!("Canyon")).unwrap();
    set(&mut app, "opacity", json!(90)).unwrap();
    ok(&mut app);
    assert_eq!((kind(&app).as_deref(), field(&app, "selected")), (Some(perspective_presets::KIND), json!("Canyon")));
    // Edit… renames it.
    press(&mut app, Action::Edit, "Canyon");
    set(&mut app, "name", json!("Gorge")).unwrap();
    ok(&mut app);
    assert_eq!(field(&app, "selected"), json!("Gorge"));
    assert_eq!(app.session.prefs.perspective_presets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Gorge"]);
    assert_eq!(app.session.prefs.perspective_presets[0].opacity, 90.0);
    // Its type's menu lists it in a slot, which applies it.
    assert_eq!(crate::menus::dynamic_label(&app, "ui.perspectiveUserPreset3.1", ""), "Gorge");
    assert!(
        ["ui.perspectiveUserPreset3.2", "ui.perspectiveUserPreset2.1"]
            .iter()
            .all(|id| crate::menus::shown_state(&app, id, &serde_json::Value::Null).is_none())
    );
    app.run("ui.perspectiveUserPreset3.1", json!({})).unwrap();
    assert_eq!(grid(&app).name, "Gorge");
    // Delete selects the one above it.
    press(&mut app, Action::Delete, "Gorge");
    assert!(app.session.prefs.perspective_presets.is_empty());
    assert_eq!(field(&app, "selected"), json!("[3P-Low View]"));
    assert!(app.run("ui.perspectiveUserPreset3.1", json!({})).is_err());
}

/// Export… suggests the selected preset's name as the file name, a built-in one's without its brackets.
#[test]
fn the_presets_manager_exports_to_a_file_named_after_the_preset() {
    use super::perspective_presets::{self, Action};
    let mut app = app();
    let picker = crate::tests_picks::scripted(&mut app);
    app.run("perspective.presets.save", json!({"name": "Street"})).unwrap();
    app.run("ui.perspectivePresetsDialog", json!({"selected": "Street"})).unwrap();
    frame(&mut app);
    for (preset, file) in [("Street", "Street.vcperspective"), ("[2P-High View]", "2P-High View.vcperspective")] {
        perspective_presets::run(&mut app, Action::Export, preset).unwrap();
        assert_eq!(picker.requests().last().and_then(|r| r.file_name.clone()).as_deref(), Some(file), "{preset}");
        picker.cancel();
        frame(&mut app);
    }
}

#[test]
fn the_presets_manager_imports_browser_contents_and_desktop_paths() {
    use super::perspective_presets::{self, Action};
    use vectorcraft_engine::cmd::perspgrid::PRESET_FORMAT;

    let mut app = app();
    let picker = crate::tests_picks::scripted(&mut app);
    let ctx = window(&app);
    let fixture = |name: &str| json!({"format": PRESET_FORMAT, "presets": [{"name": name}]}).to_string();
    app.run("ui.perspectivePresetsDialog", json!({})).unwrap();
    let default = "[1P-Normal View]";

    perspective_presets::run(&mut app, Action::Import, default).unwrap();
    picker.answer_contents(&[("Browser.vcperspective", fixture("Browser").as_bytes())]);
    crate::tests_picks::poll(&mut app, &ctx);
    assert!(app.session.perspective_preset("Browser").is_some(), "the selected browser file adds its preset");
    assert_eq!(field(&app, "selected"), json!("Browser"));

    let dir = vectorcraft_testkit::TempDir::new("perspective-presets-picker");
    let file = dir.join("Desktop.vcperspective");
    std::fs::write(&file, fixture("Desktop")).unwrap();
    perspective_presets::run(&mut app, Action::Import, "Browser").unwrap();
    picker.answer(&[file.to_str().unwrap()]);
    crate::tests_picks::poll(&mut app, &ctx);
    assert!(app.session.perspective_preset("Desktop").is_some(), "the selected desktop file adds its preset");
    assert_eq!(field(&app, "selected"), json!("Desktop"));

    perspective_presets::run(&mut app, Action::Import, "Desktop").unwrap();
    picker.cancel();
    crate::tests_picks::poll(&mut app, &ctx);
    assert_eq!(app.session.prefs.perspective_presets.len(), 2);
    assert_eq!(field(&app, "selected"), json!("Desktop"));
}

#[test]
fn perspective_preset_menus_list_the_views_of_each_type() {
    let app = app();
    let entries = crate::menus::menu_entries(&app);
    let under = |sub: &str| entries.iter().filter(|e| e.path.iter().any(|p| p == sub)).map(|e| e.label.clone()).collect::<Vec<_>>();
    assert_eq!(under("One Point Perspective"), ["[1P-Normal View]", "[1P-Low View]", "[1P-High View]"]);
    assert_eq!(under("Three Point Perspective"), ["[3P-Normal View]", "[3P-Low View]"]);
    let presets = entries.iter().find(|e| e.command.as_deref() == Some("ui.perspectivePresetsDialog")).unwrap();
    assert_eq!((presets.label.as_str(), presets.path.first().map(String::as_str)), ("Perspective Grid Presets…", Some("Edit")));
}

#[test]
fn perspective_grid_view_toggles_flip_their_labels_and_checks() {
    use crate::menus::{checked, dynamic_label};
    let mut app = app();
    assert_eq!(dynamic_label(&app, "perspective.grid.show", ""), "Show Grid");
    assert_eq!(checked(&app, "perspective.grid.snap", &Value::Null), Some(true), "Snap to Grid is on by default");
    app.run("perspective.grid.show", json!({})).unwrap();
    app.run("perspective.grid.rulers", json!({})).unwrap();
    app.run("perspective.grid.lock", json!({})).unwrap();
    app.run("perspective.grid.lockStation", json!({})).unwrap();
    app.run("perspective.grid.snap", json!({})).unwrap();
    assert_eq!(
        ["perspective.grid.show", "perspective.grid.rulers", "perspective.grid.lock"].map(|id| dynamic_label(&app, id, "")),
        ["Hide Grid", "Hide Rulers", "Unlock Grid"]
    );
    assert_eq!(checked(&app, "perspective.grid.lockStation", &Value::Null), Some(true));
    assert_eq!(checked(&app, "perspective.grid.snap", &Value::Null), Some(false));
    let entries = crate::menus::menu_entries(&app);
    let labels: Vec<&str> = entries.iter().filter(|e| e.path.last().is_some_and(|p| p == "Perspective Grid")).map(|e| e.label.as_str()).collect();
    assert_eq!(&labels[..6], ["Hide Grid", "Hide Rulers", "Snap to Grid", "Unlock Grid", "Lock Station Point", "Define Grid…"]);
}

#[test]
fn perspective_grid_options_hold_automatic_plane_positioning() {
    let mut app = app();
    app.run("tool.options", json!({"tool": "perspectiveGrid"})).unwrap();
    assert_eq!((field(&app, "anchorPoint"), field(&app, "gridlineIntersection")), (json!(true), json!(true)), "both on by default");
    frame(&mut app);
    set(&mut app, "gridlineIntersection", json!(false)).unwrap();
    frame(&mut app);
    ok(&mut app);
    assert_eq!(kind(&app), None);
    let o = app.session.prefs.perspective_widget;
    assert!(o.anchor_point && !o.gridline_intersection && o.show);
    // The dialog opens on what was set, and a value that is not true or false is refused.
    app.run("tool.options", json!({"tool": "perspectiveGrid"})).unwrap();
    assert_eq!(field(&app, "gridlineIntersection"), json!(false));
    assert!(set(&mut app, "anchorPoint", json!("off")).is_err());
    assert!(app.session.prefs.perspective_widget.anchor_point);
}

#[test]
fn digit_keys_switch_planes_and_the_widget_follows_the_window() {
    let mut app = app();
    app.run("perspective.grid.preset", json!({"kind": 2})).unwrap();
    let ctx = window(&app);
    // The keymap builds in the first pass.
    crate::tests::synthetic::key_frame(&mut app, &ctx, vec![]);
    let key = |app: &mut VectorcraftApp, k: egui::Key| {
        let press = |pressed| egui::Event::Key { key: k, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE };
        crate::tests::synthetic::key_frame(app, &ctx, vec![press(true)]);
        crate::tests::synthetic::key_frame(app, &ctx, vec![press(false)]);
    };
    key(&mut app, egui::Key::Num3);
    assert_eq!(grid(&app).plane.id(), "right");
    key(&mut app, egui::Key::Num2);
    assert_eq!(grid(&app).plane.id(), "ground");
    // The window the widget stays in: none before the canvas is laid out.
    app.view_mut().unwrap().rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
    let f = app.screen_frame().unwrap();
    assert_eq!(f.size, (800.0, 600.0));
    let z = app.view().unwrap().zoom;
    assert!((f.right.x - 1.0 / z).abs() < 1e-9 && f.right.y.abs() < 1e-9);
    assert_eq!(app.view_info().screen, Some(f));
}
