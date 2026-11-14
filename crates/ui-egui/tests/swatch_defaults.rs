//! The real Swatches menu saves application defaults without narrowing to shown/selected rows.
// Integration tests may unwrap and panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;

use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{UiState, VectorcraftApp, panels::swatches, prefs_dialog};

static CASE: Mutex<()> = Mutex::new(());
const SAVE: &str = "   Use Current Swatches as Default";
const RESET: &str = "   Restore Factory Default Swatches";

fn palette(app: &VectorcraftApp) -> Value {
    let d = &app.session.active().unwrap().doc;
    json!({"swatches": &*d.swatches, "groups": &*d.swatch_groups})
}

fn source_app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let d = &app.session.active().unwrap().doc;
    let names: Vec<String> =
        d.swatches.iter().filter(|s| !s.is_reserved()).map(|s| s.name.clone()).chain(d.swatch_groups.iter().map(|g| g.name.clone())).collect();
    app.run("swatch.delete", json!({"names": names})).unwrap();
    for group in ["Warm", "Cool", "Empty"] {
        app.run("swatch.newGroup", json!({"name": group})).unwrap();
    }
    app.run("swatch.new", json!({"name": "Visible Blue", "group": "Warm", "color": [0.2, 0.4, 0.6], "global": true, "keepModel": true})).unwrap();
    app.run(
        "swatch.new",
        json!({"name": "Hidden Ink", "group": "Cool", "color": {"c": 0.1, "m": 0.2, "y": 0.3, "k": 0.4}, "spot": true, "keepModel": true}),
    )
    .unwrap();
    app
}

fn panel(app: VectorcraftApp) -> (egui_kittest::Harness<'static, VectorcraftApp>, egui::Context) {
    let context = Rc::new(RefCell::new(None));
    let captured = context.clone();
    let h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(800.0, 900.0))
        .with_step_dt(0.1)
        .with_accessibility_check(false)
        .with_context_setup(move |ctx| {
            vectorcraft_ui_egui::theme::install_fonts(ctx);
            // Same actual egui panel-state keys as the existing Swatches tests.
            ctx.data_mut(|d| {
                d.insert_temp(egui::Id::unique(("panel-state", "swatch-show-find")), true);
                d.insert_temp(egui::Id::unique("swatch-find"), "Visible".to_string());
            });
            *captured.borrow_mut() = Some(ctx.clone());
        })
        .build_ui_state(
            |ui, app: &mut VectorcraftApp| {
                ui.columns(2, |columns| {
                    swatches::show(app, &mut columns[0]);
                    swatches::menu(app, &mut columns[1]);
                });
            },
            app,
        );
    let ctx = context.borrow().clone().unwrap();
    (h, ctx)
}

fn restored(app: &VectorcraftApp) -> VectorcraftApp {
    // The real UiState preference payload and public startup restore, not a Session clone.
    let bytes = serde_json::to_vec(&app.ui).unwrap();
    let ui = UiState::read_preferences(&bytes).unwrap().ui.sanitized();
    let mut fresh = VectorcraftApp::new(Session::new(), Default::default());
    fresh.ui = ui;
    prefs_dialog::restore(&mut fresh);
    fresh
}

fn click(h: &mut egui_kittest::Harness<'_, VectorcraftApp>, label: &str) {
    h.get_by_role_and_label(egui::accesskit::Role::Button, label).click();
    h.run_steps(3);
}

#[test]
fn swatch_defaults_menu_saves_all_rows_and_restores_factory_after_preferences_roundtrip() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut h, ctx) = panel(source_app());
    h.run_steps(3);
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("panel-state", "swatch-selected")), vec!["Visible Blue".to_string()]));
    h.run_steps(2);
    assert!(ctx.read_response(egui::Id::unique(("swatch-tile", "Visible Blue"))).is_some());
    assert!(ctx.read_response(egui::Id::unique(("swatch-tile", "Hidden Ink"))).is_none(), "Find hides a swatch that must still be saved");
    let selected: Vec<String> = ctx.data(|d| d.get_temp(egui::Id::unique(("panel-state", "swatch-selected")))).unwrap();
    assert_eq!(selected, ["Visible Blue"]);
    let expected = palette(h.state());
    assert_eq!(expected["groups"].as_array().unwrap().len(), 3);
    assert_eq!(expected["groups"][2]["name"], "Empty");
    assert!(expected["groups"][2]["swatches"].as_array().unwrap().is_empty());
    let original = serde_json::to_value(&*h.state().session.active().unwrap().doc).unwrap();
    let undo = h.state().session.active().unwrap().history.undo.len();
    assert!(!h.state_mut().take_prefs_changed());

    click(&mut h, SAVE);
    assert!(h.state_mut().take_prefs_changed(), "a successful menu action requests a prompt host save");
    assert!(!h.state_mut().take_prefs_changed(), "the host flag is one-shot");
    assert_eq!(h.state().ui.engine_prefs, h.state().session.prefs.to_json(), "the UI payload includes the engine choice");
    assert_eq!(serde_json::to_value(&*h.state().session.active().unwrap().doc).unwrap(), original);
    assert_eq!(h.state().session.active().unwrap().history.undo.len(), undo);
    let mut fresh = restored(h.state());
    fresh.run("file.new", json!({"width": 200, "height": 200, "profile": null})).unwrap();
    assert_eq!(palette(&fresh), expected, "the whole ordered palette, including hidden and empty groups, survives restart");

    click(&mut h, RESET);
    assert!(h.state_mut().take_prefs_changed());
    assert!(!h.state_mut().take_prefs_changed());
    assert_eq!(h.state().ui.engine_prefs, h.state().session.prefs.to_json());
    assert_eq!(serde_json::to_value(&*h.state().session.active().unwrap().doc).unwrap(), original, "Factory affects future documents");
    assert_eq!(h.state().session.active().unwrap().history.undo.len(), undo);
    let mut factory = restored(h.state());
    factory.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let mut stock = VectorcraftApp::new(Session::new(), Default::default());
    stock.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    assert_eq!(palette(&factory), palette(&stock));
    assert_ne!(palette(&factory), expected);
}

#[test]
fn swatch_defaults_failed_command_keeps_the_saved_payload_and_host_flag() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let mut original = source_app();
    original.run("swatch.defaults.save", json!({})).unwrap();
    assert!(original.take_prefs_changed());
    let (mut h, _) = panel(restored(&original));
    h.run_steps(3);
    assert!(h.state().session.active().is_none());
    assert!(!h.state_mut().take_prefs_changed());
    assert!(!vectorcraft_ui_egui::menus::enabled(h.state(), "swatch.defaults.save"));
    let saved = h.state().ui.engine_prefs.clone();
    let prefs = h.state().session.prefs.to_json();
    click(&mut h, SAVE);
    assert!(!h.state_mut().take_prefs_changed(), "a disabled menu row does not write preferences");
    assert!(h.state_mut().run("swatch.defaults.save", json!({})).is_err(), "there is no current palette to capture");
    assert!(!h.state_mut().take_prefs_changed(), "an error must not request a preference write");
    assert_eq!(h.state().ui.engine_prefs, saved);
    assert_eq!(h.state().session.prefs.to_json(), prefs);
}

#[test]
fn swatch_defaults_batches_persist_the_final_choice_and_request_one_host_save() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let mut app = source_app();
    let expected = palette(&app);
    let original = serde_json::to_value(&*app.session.active().unwrap().doc).unwrap();
    let undo = app.session.active().unwrap().history.undo.len();
    let mut stock = VectorcraftApp::new(Session::new(), Default::default());
    stock.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let factory = palette(&stock);
    assert_ne!(expected, factory);
    assert!(!app.take_prefs_changed());

    for (commands, custom) in [
        (json!([{"command": "swatch.defaults.save", "params": {}}]), true),
        (json!([{"command": "swatch.defaults.reset", "params": {}}]), false),
        (json!([{"command": "swatch.defaults.save", "params": {}}, {"command": "swatch.defaults.reset", "params": {}}]), false),
        (json!([{"command": "swatch.defaults.reset", "params": {}}, {"command": "swatch.defaults.save", "params": {}}]), true),
    ] {
        app.run("command.batch", json!({"commands": commands})).unwrap();
        assert_eq!(app.ui.engine_prefs, app.session.prefs.to_json(), "the batch's final preference reaches the host payload");
        assert!(app.take_prefs_changed(), "a successful defaults batch requests a host save");
        assert!(!app.take_prefs_changed(), "the host flag remains one-shot");
        assert_eq!(serde_json::to_value(&*app.session.active().unwrap().doc).unwrap(), original);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        let mut fresh = restored(&app);
        fresh.run("file.new", json!({"width": 200, "height": 200, "profile": null})).unwrap();
        assert_eq!(palette(&fresh), if custom { expected.clone() } else { factory.clone() }, "the final batch step survives restart");
    }

    let saved = app.ui.engine_prefs.clone();
    let prefs = app.session.prefs.to_json();
    app.run("command.batch", json!({"commands": []})).unwrap();
    assert!(!app.take_prefs_changed(), "an unrelated empty batch does not request a preference write");
    assert_eq!(app.ui.engine_prefs, saved);
    assert_eq!(app.session.prefs.to_json(), prefs);
}

#[test]
fn swatch_defaults_failed_batches_restore_the_choice_without_requesting_a_host_save() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    for change in ["swatch.defaults.save", "swatch.defaults.reset"] {
        for failure in
            [json!({"command": "document.activate", "params": {"index": 1}}), json!({"command": "command.batch", "params": {"commands": []}})]
        {
            let mut app = source_app();
            // Start from the other choice so the first step really changes the preference.
            let initial = if change == "swatch.defaults.save" { "swatch.defaults.reset" } else { "swatch.defaults.save" };
            app.run(initial, json!({})).unwrap();
            assert!(app.take_prefs_changed());
            assert!(!app.take_prefs_changed());
            assert_eq!(app.session.documents().len(), 1, "index 1 names a missing second document");
            let prefs = app.session.prefs.to_json();
            let saved = app.ui.engine_prefs.clone();
            let original = serde_json::to_value(&*app.session.active().unwrap().doc).unwrap();
            let undo = app.session.active().unwrap().history.undo.len();
            let mut before = restored(&app);
            before.run("file.new", json!({"width": 200, "height": 200, "profile": null})).unwrap();
            let expected = palette(&before);

            let result = app.run("command.batch", json!({"commands": [{"command": change, "params": {}}, failure]}));
            assert!(result.is_err(), "a missing document or forbidden nested batch fails after the defaults step");
            assert_eq!(app.session.prefs.to_json(), prefs, "a failed batch rolls its defaults step back");
            assert_eq!(app.ui.engine_prefs, saved, "a failed batch keeps the persisted payload");
            assert!(!app.take_prefs_changed(), "an error must not request a preference write");
            assert_eq!(serde_json::to_value(&*app.session.active().unwrap().doc).unwrap(), original);
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
            let mut fresh = restored(&app);
            fresh.run("file.new", json!({"width": 200, "height": 200, "profile": null})).unwrap();
            assert_eq!(palette(&fresh), expected, "restart keeps the pre-batch choice");
        }
    }
}
