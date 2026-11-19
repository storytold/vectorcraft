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

#[cfg(not(target_arch = "wasm32"))]
fn review_597971_next_palette(app: &VectorcraftApp) -> Value {
    // Exercise the serialized UiState/startup boundary, not the desktop's private preferences writer.
    let mut fresh = restored(app);
    fresh.run("file.new", json!({"width": 200, "height": 200, "profile": null})).unwrap();
    palette(&fresh)
}

#[cfg(not(target_arch = "wasm32"))]
fn review_597971_native_source(partial: bool) -> (vectorcraft_testkit::TempDir, Vec<u8>) {
    let src = vectorcraft_testkit::TempDir::new("swatch-defaults-action-source");
    let path = src.join("a.vectorcraft");
    let mut session = Session::new();
    session.execute("file.new", &json!({"width": 20, "height": 20, "artboards": 1})).unwrap();
    session.execute("document.save", &json!({"path": path.to_string_lossy()})).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert!(!bytes.is_empty());
    if partial {
        std::fs::write(src.join("b.vectorcraft"), b"not a native document").unwrap();
    }
    (src, bytes)
}

#[cfg(not(target_arch = "wasm32"))]
fn review_597971_action_params(src: &std::path::Path, dst: &std::path::Path, command: &str) -> Value {
    json!({
        "source": "folder", "folder": src.to_string_lossy(),
        "destination": "folder", "destinationFolder": dst.to_string_lossy(), "stopOnError": false,
        "commands": [{"command": command, "params": {}}]
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn review_597971_run_action(app: &mut VectorcraftApp, params: Value, outer: bool) -> Value {
    if outer {
        let result = app.run("command.batch", json!({"commands": [{"command": "action.batch", "params": params}]})).unwrap();
        assert_eq!(result["results"].as_array().unwrap().len(), 1);
        result["results"][0].clone()
    } else {
        app.run("action.batch", params).unwrap()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn review_597971_original_document(app: &VectorcraftApp) -> Value {
    let st = app.session.active().unwrap();
    json!({
        "active": app.session.active_index(), "count": app.session.documents().len(), "uid": st.uid,
        "document": &*st.doc, "selection": format!("{:?}", st.selection),
        "undo": st.history.undo.len(), "redo": st.history.redo.len()
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn review_597971_report(result: &Value, partial: bool) {
    assert_eq!(result["count"], if partial { 2 } else { 1 }, "count includes failed source files: {result}");
    assert_eq!(result["failed"], usize::from(partial), "{result}");
    assert_eq!(result["stopped"], false, "{result}");
    let processed = result["processed"].as_array().unwrap();
    assert_eq!(processed.len(), if partial { 2 } else { 1 });
    assert!(processed[0]["path"].as_str().unwrap().ends_with("a.vectorcraft"), "{result}");
    assert!(processed[0].get("result").is_some(), "the native file's command actually ran: {result}");
    assert!(processed[0].get("error").is_none(), "{result}");
    if partial {
        assert!(processed[1]["path"].as_str().unwrap().ends_with("b.vectorcraft"), "{result}");
        assert!(processed[1]["error"].as_str().is_some_and(|error| !error.is_empty()), "a supported bad file is attempted: {result}");
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn review_597971_action_observation(outer: bool, partial: bool) -> (Value, Value) {
    let mut app = source_app();
    let custom = palette(&app);
    let mut stock = VectorcraftApp::new(Session::new(), Default::default());
    stock.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let factory = palette(&stock);
    assert_ne!(custom, factory);
    app.run("swatch.defaults.save", json!({})).unwrap();
    assert!(app.take_prefs_changed());
    assert!(!app.take_prefs_changed());
    assert_eq!(app.ui.engine_prefs, app.session.prefs.to_json());
    assert_eq!(review_597971_next_palette(&app), custom, "custom defaults persist before action.batch");
    assert_ne!(app.session.prefs.default_swatches, stock.session.prefs.default_swatches);
    let original = review_597971_original_document(&app);
    let (src, source_bytes) = review_597971_native_source(partial);

    // An actual native-file action without defaults commands leaves the preference boundary alone.
    let unrelated_dst = vectorcraft_testkit::TempDir::new("swatch-defaults-action-unrelated");
    let saved = app.ui.engine_prefs.clone();
    let prefs = app.session.prefs.to_json();
    let unrelated = review_597971_run_action(&mut app, review_597971_action_params(&src, &unrelated_dst, "select.none"), outer);
    review_597971_report(&unrelated, partial);
    assert!(unrelated_dst.join("a.vectorcraft").is_file());
    assert_eq!(review_597971_original_document(&app), original);
    assert_eq!(app.session.prefs.to_json(), prefs);
    assert_eq!(app.ui.engine_prefs, saved);
    assert!(!app.take_prefs_changed(), "an unrelated action does not request a preference save");

    let dst = vectorcraft_testkit::TempDir::new("swatch-defaults-action-reset");
    let result = review_597971_run_action(&mut app, review_597971_action_params(&src, &dst, "swatch.defaults.reset"), outer);
    review_597971_report(&result, partial);
    assert!(dst.join("a.vectorcraft").is_file(), "Folder destination writes the successful native result");
    assert!(!dst.join("b.vectorcraft").exists(), "a failed open does not produce an output file");
    assert_eq!(std::fs::read(src.join("a.vectorcraft")).unwrap(), source_bytes, "Folder destination preserves the input");
    assert_eq!(review_597971_original_document(&app), original, "opened source closes back to the unchanged original document");
    assert_eq!(
        app.session.prefs.default_swatches,
        Some(vectorcraft_engine::SavedSwatchDefaults::Factory),
        "the real batch changed the live preference to Factory"
    );
    let first_save = app.take_prefs_changed();
    let second_save = app.take_prefs_changed();
    let next_palette = review_597971_next_palette(&app);
    (
        json!({"outer": outer, "partial": partial, "prompt_save": first_save, "second_save": second_save,
            "ui_engine_prefs": app.ui.engine_prefs, "next_palette": next_palette}),
        json!({"outer": outer, "partial": partial, "prompt_save": true, "second_save": false,
            "ui_engine_prefs": app.session.prefs.to_json(), "next_palette": factory}),
    )
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn review_597971_action_batches_publish_defaults_to_the_ui_preference_boundary() {
    let _case = CASE.lock().unwrap_or_else(|e| e.into_inner());
    let mut observed = Vec::new();
    let mut expected = Vec::new();
    for (outer, partial) in [(false, false), (false, true), (true, false), (true, true)] {
        let (actual, required) = review_597971_action_observation(outer, partial);
        observed.push(actual);
        expected.push(required);
    }
    assert_eq!(observed, expected, "each successful defaults action requests one prompt save and updates the serialized UiState/startup boundary");
}
