use super::*;
use crate::Services;
use craft::ui::dialog::PromptChoice;
use craft::ui::patterns::preferences::{PageSpec, PreferencesModel};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::prefscmds::PREF_SPECS;

use crate::theme::Brightness;

fn app() -> VectorcraftApp {
    crate::testing::windowed(VectorcraftApp::new(Session::new(), Services::default()))
}

/// One headless pass of the open window.
fn frame(a: &mut VectorcraftApp, ctx: &egui::Context) {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(a, ui));
    out.textures_delta.clear();
}

/// Reset Preferences asked and answered with `choice`, with the passes the question's host needs.
fn reset_preferences(a: &mut VectorcraftApp, choice: PromptChoice) {
    // The question's host answers on the window's context.
    let ctx = a.ui_ctx.clone().expect("a window context");
    frame(a, &ctx);
    ask_reset(a);
    frame(a, &ctx);
    answer_reset(a, choice);
    frame(a, &ctx);
}

/// Runs `f` over the pages the open window builds from its draft.
fn with_pages<R>(a: &mut VectorcraftApp, f: impl FnOnce(&[PageSpec<'_>]) -> R) -> R {
    let mut w = a.prefs.take().expect("Preferences is open");
    let r = f(&model::Live { app: &mut *a, w: &mut w }.pages());
    a.prefs = Some(w);
    r
}

/// The visible row of `key`: whether it is enabled and its help.
fn row_state(a: &mut VectorcraftApp, key: &str) -> Option<(bool, Option<String>)> {
    let tail = format!(".{key}");
    with_pages(a, |pages| {
        pages
            .iter()
            .flat_map(|p| &p.groups)
            .flat_map(|g| &g.rows)
            .find(|r| r.visible && (r.key == key || r.key.ends_with(&tail)))
            .map(|r| (r.enabled, r.help.map(str::to_string)))
    })
}

/// Two values equal, numbers by value so `4` and `4.0` agree.
fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

fn field(a: &VectorcraftApp, key: &str) -> Value {
    fields(a).and_then(|f| f.get(key).cloned()).unwrap_or(Value::Null)
}

/// One row per kind is saved by OK: the model's value conversion reaches `prefs.set` for each.
#[test]
fn ok_saves_every_row_kind() {
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            vectorcraft_text::set_hyphenation_exceptions("");
        }
    }
    let _clear = Clear;
    let rows = [
        ("marqueeSelection", json!("directional")),
        ("typedLists", json!(false)),
        ("recentFontsCount", json!(8)),
        ("constrainAngle", json!(30.0)),
        ("keyboardIncrement", json!(4)),
        ("hyphenationExceptions", json!("typography, hap-pen")),
        ("guideColor", json!("#ff0000")),
    ];
    let mut a = app();
    open(&mut a, None);
    for (key, value) in &rows {
        set_field(&mut a, key, value.clone()).unwrap();
    }
    confirm(&mut a).unwrap();
    assert!(!is_open(&a));
    let saved = a.session.prefs.to_json();
    for (key, value) in &rows {
        let kept = &saved[*key];
        let ok = kept.as_str().zip(value.as_str()).map_or_else(|| same(kept, value), |(x, y)| x.eq_ignore_ascii_case(y));
        assert!(ok, "{key}: saved {kept} for {value}");
    }
    assert_eq!(a.ui.engine_prefs, saved, "saved with the UI state");
    assert_eq!(vectorcraft_text::hyphenation_exceptions(), "typography, hap-pen");
    assert!(vectorcraft_text::hyphen::hyphen_points("typography").is_empty());
    assert_eq!(vectorcraft_text::hyphen::hyphen_points("happen"), vec![3]);
}

/// User Interface › Canvas Color (#663): White and the three greys are the pasteboard; Match User
/// Interface Brightness leaves the theme's own.
#[test]
fn canvas_color_choices_paint_the_pasteboard() {
    for (key, want) in [
        ("white", Some(egui::Color32::WHITE)),
        ("lightGray", Some(egui::Color32::from_gray(0xc8))),
        ("mediumGray", Some(egui::Color32::from_gray(0x96))),
        ("darkGray", Some(egui::Color32::from_gray(0x5a))),
        ("matchUi", None),
    ] {
        let mut a = app();
        open(&mut a, None);
        set_field(&mut a, "canvasColor", json!(key)).unwrap();
        confirm(&mut a).unwrap();
        assert_eq!(canvas_color(&a.session.prefs.canvas_color), want, "{key}");
    }
    assert!(app().run("prefs.set", json!({"key": "canvasColor", "value": "pink"})).is_err());
}

/// #4837: the saved Color Scheme, Accent and Canvas Color are the ones the first frame shows, not
/// the defaults the first UI pass installs.
#[test]
fn saved_scheme_accent_and_canvas_color_show_from_the_first_frame() {
    let mut app = app();
    for (key, value) in [("uiColorScheme", "classic"), ("uiAccent", "pink"), ("canvasColor", "white")] {
        app.run("prefs.set", json!({"key": key, "value": value})).unwrap();
    }
    let ctx = crate::testing::ui_context();
    for _ in 0..2 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.tick(ui.ctx()));
        out.textures_delta.clear();
        let shown = crate::theme::root_theme(&app.session.prefs);
        assert_ne!(shown.0, crate::theme::root_theme(&Prefs::default()).0, "the saved scheme");
        // Classic keeps its own accent whatever accent is saved, as production's Classic tokens did.
        let classic = Prefs { ui_color_scheme: "classic".into(), ui_brightness: app.session.prefs.ui_brightness.clone(), ..Prefs::default() };
        let want = crate::theme::root_theme(&classic);
        assert!(shown.0 == want.0 && shown.2 == want.2, "the saved accent");
        assert_eq!(canvas_color(&app.session.prefs.canvas_color), Some(egui::Color32::WHITE));
    }
}

/// A Render Threads change made after the preferences were applied restarts the canvas renderer at
/// the next push.
#[test]
fn a_render_threads_change_restarts_the_renderer_at_the_next_push() {
    let mut app = app();
    let ctx = crate::testing::ui_context();
    apply_runtime(&mut app, &ctx);
    app.canvas.worker_started = true;
    // The automatic count as an explicit value: the process-wide count other tests read stays the same.
    // `VECTORCRAFT_RENDER_THREADS` can give more than the 64 the preference accepts.
    let threads = vectorcraft_render::default_threads().min(64);
    app.run("prefs.set", json!({"key": "renderThreads", "value": threads})).unwrap();
    apply_runtime(&mut app, &ctx);
    let restarted = !app.canvas.worker_started;
    app.run("prefs.set", json!({"key": "renderThreads", "value": -1})).unwrap();
    assert!(restarted, "the canvas renderer kept its old thread count");
}

#[test]
fn dialog_ok_applies_prefs_and_ui_fields() {
    let mut a = app();
    open(&mut a, Some("general"));
    assert_eq!(page(&a), Some("General"));
    set_field(&mut a, "keyboardIncrement", json!(4)).unwrap();
    set_field(&mut a, "uiBrightness", json!("light")).unwrap();
    set_field(&mut a, "__smartGuides", json!(false)).unwrap();
    confirm(&mut a).unwrap();
    assert!(!is_open(&a));
    assert_eq!(a.session.prefs.keyboard_increment, 4.0);
    assert_eq!(a.session.prefs.ui_brightness, "light");
    assert!(!a.ui.view.smart_guides);
    assert_eq!(a.ui.engine_prefs["keyboardIncrement"], json!(4.0));
}

#[test]
fn dialog_ok_with_invalid_value_keeps_dialog_and_prefs() {
    let mut a = app();
    open(&mut a, None);
    set_field(&mut a, "anchorSize", json!(42)).unwrap();
    assert!(confirm(&mut a).is_err());
    assert!(is_open(&a));
    assert_eq!(a.session.prefs, Prefs::default());
}

#[test]
fn prefs_persist_through_ui_state() {
    // PLAN: the `AppSettings` round trip (shell-w8 `settings::from_app`) has no signature in the bundle; the UI state is the saved form here.
    let mut a = app();
    a.run("prefs.set", json!({"key": "gridlineEvery", "value": 50})).unwrap();
    snapshot(&mut a);
    let saved = serde_json::to_vec(&a.ui).unwrap();
    let mut b = app();
    b.ui = serde_json::from_slice::<crate::UiState>(&saved).unwrap().sanitized();
    restore(&mut b);
    assert_eq!(b.session.prefs.gridline_every, 50.0);
}

/// Guides & Grid › Lock Guides by Default: applied by OK, saved with the UI state and read back as
/// locked guides. A file without the key leaves guides unlocked.
#[test]
fn lock_guides_by_default_shows_applies_and_restores() {
    let mut a = app();
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    open(&mut a, Some("Guides & Grid"));
    assert_eq!(field(&a, "lockGuidesByDefault"), json!(false));
    set_field(&mut a, "lockGuidesByDefault", json!(true)).unwrap();
    confirm(&mut a).unwrap();
    assert!(a.session.guides_locked());
    assert_eq!(crate::menus::dynamic_label(&a, "view.guides.lock", ""), "Unlock Guides");
    assert_eq!(a.ui.engine_prefs["lockGuidesByDefault"], json!(true));

    let saved = serde_json::to_vec(&a.ui).unwrap();
    let mut b = app();
    assert!(!b.session.guides_locked());
    b.ui = serde_json::from_slice::<crate::UiState>(&saved).unwrap().sanitized();
    restore(&mut b);
    assert!(b.session.guides_locked());

    let mut older = app();
    older.ui.engine_prefs = json!({"gridlineEvery": 50});
    restore(&mut older);
    assert!(!older.session.guides_locked());
}

/// OK with the option unchanged keeps the lock from the View menu. OK after turning the option
/// off unlocks guides.
#[test]
fn preferences_ok_changes_the_guide_lock_only_with_the_option() {
    let mut a = app();
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    a.run("view.guides.lock", json!({"locked": true})).unwrap();
    open(&mut a, Some("Guides & Grid"));
    confirm(&mut a).unwrap();
    assert!(a.session.guides_locked(), "the option did not change");
    a.run("view.guides.lock", json!({"locked": false})).unwrap();
    a.run("prefs.set", json!({"key": "lockGuidesByDefault", "value": true})).unwrap();
    assert!(a.session.guides_locked());
    open(&mut a, Some("Guides & Grid"));
    set_field(&mut a, "lockGuidesByDefault", Prefs::default().to_json()["lockGuidesByDefault"].clone()).unwrap();
    confirm(&mut a).unwrap();
    assert!(!a.session.guides_locked(), "the option went off");
}

/// Reset Preferences then OK resets the preferences, General included, and leaves the open
/// document's units, with no undo step in the document.
#[test]
fn reset_preferences_then_ok_leaves_the_documents_units() {
    let mut a = app();
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    a.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 5})).unwrap();
    let undo = a.session.active().unwrap().history.undo.len();
    open(&mut a, Some("General"));
    reset_preferences(&mut a, PromptChoice::Proceed);
    assert!(is_open(&a), "Reset leaves Preferences open");
    assert!(same(&field(&a, "keyboardIncrement"), &Prefs::default().to_json()["keyboardIncrement"]), "the draft holds the defaults");
    assert_eq!(field(&a, "unitsGeneral"), json!("points"));
    confirm(&mut a).unwrap();
    assert_eq!(a.session.prefs.keyboard_increment, Prefs::default().keyboard_increment);
    assert_eq!(a.session.prefs.units_general, Prefs::default().units_general);
    let st = a.session.active().unwrap();
    assert_eq!(st.doc.units, vectorcraft_doc::Unit::Millimeters);
    assert_eq!(st.history.undo.len(), undo, "no undo step");
}

/// After Reset Preferences, choosing a General unit still sets the document's units on OK, Points
/// included once another unit was chosen in between. With no document open, Reset Preferences
/// resets General to its default.
#[test]
fn reset_preferences_keeps_an_edit_of_general_units() {
    let mut a = app();
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    a.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    open(&mut a, Some("General"));
    reset_preferences(&mut a, PromptChoice::Proceed);
    set_field(&mut a, "unitsGeneral", json!("inches")).unwrap();
    confirm(&mut a).unwrap();
    assert_eq!(a.session.active().unwrap().doc.units, vectorcraft_doc::Unit::Inches);
    assert_eq!(a.session.prefs.units_general, "inches");

    a.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    open(&mut a, Some("General"));
    reset_preferences(&mut a, PromptChoice::Proceed);
    for u in ["inches", "points"] {
        set_field(&mut a, "unitsGeneral", json!(u)).unwrap();
    }
    confirm(&mut a).unwrap();
    assert_eq!(a.session.active().unwrap().doc.units, vectorcraft_doc::Unit::Points);

    let mut b = app();
    b.run("prefs.set", json!({"key": "unitsGeneral", "value": "Millimeters"})).unwrap();
    open(&mut b, Some("General"));
    reset_preferences(&mut b, PromptChoice::Proceed);
    confirm(&mut b).unwrap();
    assert_eq!(b.session.prefs.units_general, Prefs::default().units_general);
}

#[test]
fn old_pref_files_keep_their_brightness() {
    let mut a = app();
    a.ui.brightness = Brightness::Light;
    a.ui.engine_prefs = Value::Null;
    restore(&mut a);
    assert_eq!(a.session.prefs.ui_brightness, "light");
}

/// Rows that depend on another setting are dimmed while it does not hold; a row the platform does
/// not show is skipped.
#[test]
fn dimmed_rows_follow_the_settings_they_depend_on() {
    // Title bars: the environment variable chooses the frame; System Title Bar takes precedence over the app's.
    for (from_env, system) in [(false, false), (true, false), (false, true), (true, true)] {
        let mut a = app();
        a.titlebar_from_env = from_env;
        open(&mut a, Some("User Interface"));
        set_field(&mut a, "systemTitleBar", json!(system)).unwrap();
        if let Some((enabled, help)) = row_state(&mut a, "appTitleBar") {
            let label = format!("variable {from_env}, system {system}");
            assert_eq!(enabled, !from_env && !system, "app title bar: {label}");
            // The note, not the setting's sentence, which names the variable too.
            let note = help.is_some_and(|h| h.contains("VECTORCRAFT_TITLEBAR environment variable"));
            assert_eq!(note, from_env, "app title bar note: {label}");
        }
        if let Some((enabled, _)) = row_state(&mut a, "systemTitleBar") {
            assert_eq!(enabled, !from_env, "system title bar: variable {from_env}");
        }
    }
    // A switch row, the setting it depends on, and the values that enable it.
    let switches = [
        ("holdShortcuts", "keyHoldDelay", json!(true), json!(false)),
        ("fontPreview", "fontSamplesOnWhite", json!(true), json!(false)),
        ("showToolTips", "showToolCards", json!(true), json!(false)),
    ];
    for (master, dependent, on, off) in switches {
        for (value, enabled) in [(on, true), (off, false)] {
            let mut a = app();
            open(&mut a, None);
            set_field(&mut a, master, value).unwrap();
            let (shown, _) = row_state(&mut a, dependent).unwrap_or_else(|| panic!("no row {dependent}"));
            assert_eq!(shown, enabled, "{dependent} with {master}");
        }
    }
    // Numbers Without Units Are Points matters only while a unit is Picas.
    for unit in ["unitsGeneral", "unitsStroke", "unitsType"] {
        for (value, enabled) in [("picas", true), ("points", false)] {
            let mut a = app();
            open(&mut a, Some("Units"));
            set_field(&mut a, unit, json!(value)).unwrap();
            let (shown, _) = row_state(&mut a, "numbersWithoutUnitsArePoints").expect("the row is built");
            assert_eq!(shown, enabled, "{unit} {value}");
        }
    }
}

/// 0.5.0 saved `powerSaving` for everyone (its default): it reads as Automatic, while the
/// choices this version offers are kept (#502).
#[test]
fn a_graphics_processor_this_version_does_not_offer_reads_as_automatic() {
    for (saved, read) in [("powerSaving", "automatic"), ("turbo", "automatic"), ("lowPower", "lowPower"), ("highPerformance", "highPerformance")] {
        let mut a = app();
        a.ui.engine_prefs = json!({"gpuPreference": saved});
        restore(&mut a);
        assert_eq!(a.session.prefs.gpu_preference, read, "{saved}");
    }
}

/// Reset Preferences asks first, over Preferences. Cancel shows Preferences again as it was: its
/// edits and page kept, and nothing applied.
#[test]
fn reset_preferences_asks_first_and_cancel_returns_to_preferences() {
    let mut a = app();
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 5})).unwrap();
    open(&mut a, Some("Units"));
    set_field(&mut a, "keyboardIncrement", json!(7)).unwrap();
    reset_preferences(&mut a, PromptChoice::Cancel);
    assert!(is_open(&a), "Preferences is open again");
    assert!(same(&field(&a, "keyboardIncrement"), &json!(7)), "the edit is kept");
    assert_eq!(page(&a), Some("Units"));
    assert_eq!(a.session.prefs.keyboard_increment, 5.0, "nothing applied");
}

/// Proceeding fills the draft with the defaults; nothing is applied until OK, and Cancel in
/// Preferences keeps the stored preferences.
#[test]
fn confirming_reset_fills_preferences_and_applies_nothing_until_ok() {
    let mut a = app();
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 5})).unwrap();
    open(&mut a, Some("Units"));
    set_field(&mut a, "keyboardIncrement", json!(7)).unwrap();
    reset_preferences(&mut a, PromptChoice::Proceed);
    assert!(is_open(&a));
    assert!(same(&field(&a, "keyboardIncrement"), &Prefs::default().to_json()["keyboardIncrement"]), "the draft holds the defaults");
    assert_eq!(a.session.prefs.keyboard_increment, 5.0, "nothing applied yet");
    cancel(&mut a);
    assert!(!is_open(&a));
    assert_eq!(a.session.prefs.keyboard_increment, 5.0, "Cancel keeps the preferences");

    open(&mut a, Some("Units"));
    reset_preferences(&mut a, PromptChoice::Proceed);
    confirm(&mut a).unwrap();
    assert_eq!(a.session.prefs.keyboard_increment, Prefs::default().keyboard_increment);
}

/// A restored Hold Shortcut Delay outside its range opens as the delay in use, and OK keeps it.
#[test]
fn a_restored_hold_shortcut_delay_outside_its_range_opens_as_the_delay_in_use() {
    for (saved, used) in [(5.0, 1.0), (0.05, 0.2)] {
        let mut a = app();
        a.session.prefs.key_hold_delay = saved;
        open(&mut a, Some("General"));
        assert!(same(&field(&a, "keyHoldDelay"), &json!(used)), "{saved}");
        confirm(&mut a).unwrap();
        assert_eq!(a.session.prefs.key_hold_delay, used, "{saved}");
    }
}

/// Every page the model builds has a visible row on this platform.
#[test]
fn every_category_renders_fields() {
    let mut a = app();
    open(&mut a, None);
    let empty = with_pages(&mut a, |pages| pages.iter().filter(|p| !p.has_content()).map(|p| p.id.to_string()).collect::<Vec<_>>());
    assert!(empty.is_empty(), "pages without a visible row: {empty:?}");
    for c in PREF_CATEGORIES {
        assert!(PREF_SPECS.iter().any(|s| s.category == *c));
    }
}

/// A new, empty folder of its own for test `name`.
fn temp_folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-ui-prefs-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// An app that writes and reads real files.
fn app_with_files() -> VectorcraftApp {
    let mut a = app();
    a.services.write = Some(Box::new(|p: &str, b: &[u8]| std::fs::write(p, b).map_err(|e| e.to_string())));
    a.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    a
}

/// `prefs.export` saves the applied preferences to a file, and `prefs.import` applies them again:
/// the open window shows them, OK keeps them, and the open document's units stay as they are.
#[test]
fn export_and_import_preferences_buttons_round_trip() {
    let dir = temp_folder("prefs-file");
    let path = dir.join("VectorCraft Preferences.json").to_string_lossy().into_owned();
    let mut a = app_with_files();
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    a.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    let values = json!({"keyboardIncrement": 5, "uiAccent": "green", "unitsGeneral": "inches"});
    a.run("prefs.set", json!({"values": values, "documentUnits": false})).unwrap();
    let saved = a.session.prefs.to_json();
    open(&mut a, Some("General"));
    a.run("prefs.export", json!({"path": path})).unwrap();
    let file: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(file["format"], "vectorcraft-preferences");
    assert_eq!(file["prefs"]["uiAccent"], "green");

    cancel(&mut a);
    a.run("prefs.reset", json!({})).unwrap();
    let undo = a.session.active().unwrap().history.undo.len();
    open(&mut a, Some("General"));
    set_field(&mut a, "keyboardIncrement", json!(9)).unwrap();
    a.run("prefs.import", json!({"path": path})).unwrap();
    assert_eq!(a.session.prefs.to_json(), saved, "the exported preferences are applied");
    assert_eq!(a.ui.engine_prefs, saved, "and saved with the UI state");
    assert!(is_open(&a), "the window stays open");
    assert!(same(&field(&a, "keyboardIncrement"), &json!(5.0)) && field(&a, "unitsGeneral") == json!("inches"), "the window shows them");
    confirm(&mut a).unwrap();
    assert_eq!(a.session.prefs.to_json(), saved);
    let st = a.session.active().unwrap();
    assert_eq!(st.doc.units, vectorcraft_doc::Unit::Millimeters, "the document's units stay");
    assert_eq!(st.history.undo.len(), undo, "no undo step");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file `prefs.import` can't use changes nothing, the draft's edits included, and the refusal
/// says why.
#[test]
fn import_preferences_of_a_bad_file_changes_nothing() {
    let dir = temp_folder("prefs-bad-file");
    let path = dir.join("newer.json");
    std::fs::write(&path, r#"{"format": "vectorcraft-preferences", "version": 2, "prefs": {"uiAccent": "pink"}}"#).unwrap();
    let path = path.to_string_lossy().into_owned();
    let mut a = app_with_files();
    let before = a.session.prefs.to_json();
    open(&mut a, Some("General"));
    set_field(&mut a, "keyboardIncrement", json!(9)).unwrap();
    let err = a.run("prefs.import", json!({"path": path})).unwrap_err();
    assert!(err.contains("newer version of VectorCraft"), "{err}");
    assert_eq!(a.session.prefs.to_json(), before);
    assert!(same(&field(&a, "keyboardIncrement"), &json!(9)), "the window keeps its edits");
    let _ = std::fs::remove_dir_all(&dir);
}

/// `prefs.import` from a script or MCP while Preferences is open: the window shows the imported
/// values.
#[test]
fn prefs_import_refreshes_an_open_preferences_dialog() {
    let mut a = app();
    open(&mut a, Some("General"));
    let data = json!({"format": "vectorcraft-preferences", "version": 1, "prefs": {"keyboardIncrement": 7}}).to_string();
    a.run("prefs.import", json!({ "data": data })).unwrap();
    assert!(same(&field(&a, "keyboardIncrement"), &json!(7.0)));
    assert_eq!(a.ui.engine_prefs["keyboardIncrement"], json!(7.0));
}

/// An import that turns Enable Touch Workspace on or off switches the workspace on a touch screen,
/// as OK in Preferences does.
#[test]
fn prefs_import_of_touch_workspace_switches_the_workspace() {
    let mut a = app();
    a.run("prefs.set", json!({"key": "touchWorkspace", "value": false})).unwrap();
    a.touch_screen = true;
    crate::workspaces::switch(&mut a, "Typography").unwrap();
    let file = |on: bool| json!({"format": "vectorcraft-preferences", "version": 1, "prefs": {"touchWorkspace": on}}).to_string();
    a.run("prefs.import", json!({ "data": file(true) })).unwrap();
    assert!(crate::workspaces::is_current(&a.ui, crate::workspaces::TOUCH));
    a.run("prefs.import", json!({ "data": file(false) })).unwrap();
    assert!(crate::workspaces::is_current(&a.ui, "Typography"));
}

/// A folder that no folder can be created in, or that is not there, is refused on OK with the
/// engine's message, and the draft keeps its value.
#[test]
fn a_recovery_folder_that_cannot_be_written_is_refused() {
    let dir = temp_folder("recovery-refused");
    let file = dir.join("not-a-folder.txt");
    std::fs::write(&file, b"x").unwrap();
    let mut a = app();
    open(&mut a, Some("File Handling"));
    for path in [file.clone(), dir.join("missing")] {
        let path = path.to_string_lossy().into_owned();
        set_field(&mut a, "recoveryFolder", json!(path)).unwrap();
        assert_eq!(confirm(&mut a), Err(format!("{path} is not a folder")));
        assert!(is_open(&a));
        assert_eq!(field(&a, "recoveryFolder"), json!(path), "the draft keeps what was typed");
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = dir.join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o555)).unwrap();
        // An account that may write anyway (root) has nothing to refuse.
        if std::fs::create_dir(locked.join("probe")).is_err() {
            set_field(&mut a, "recoveryFolder", json!(locked.to_string_lossy())).unwrap();
            let err = confirm(&mut a).unwrap_err();
            assert!(err.starts_with("Data Recovery can't keep copies in "), "{err}");
        }
        let _ = std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755));
        // A folder that can be written but not listed: the recovery store lists its folders.
        let unlisted = dir.join("unlisted");
        std::fs::create_dir(&unlisted).unwrap();
        std::fs::set_permissions(&unlisted, std::fs::Permissions::from_mode(0o333)).unwrap();
        // An account that may list anyway (root) has nothing to refuse.
        if std::fs::read_dir(&unlisted).is_err() {
            set_field(&mut a, "recoveryFolder", json!(unlisted.to_string_lossy())).unwrap();
            let err = confirm(&mut a).unwrap_err();
            assert!(err.starts_with("Data Recovery can't keep copies in "), "{err}");
        }
        let _ = std::fs::set_permissions(&unlisted, std::fs::Permissions::from_mode(0o755));
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A typed folder is refused the way a picked one is; a saved folder is not checked again, and an
/// empty field and the field's spaces are saved as typed.
#[test]
fn ok_refuses_a_typed_folder_that_choose_would_refuse() {
    let dir = temp_folder("typed-folders");
    let file = dir.join("not-a-folder.txt");
    std::fs::write(&file, b"x").unwrap();
    let missing = dir.join("missing").to_string_lossy().into_owned();
    for key in ["recoveryFolder", "pluginsFolder"] {
        let mut a = app();
        open(&mut a, Some("File Handling"));
        let before = a.session.prefs.to_json();
        for path in [file.to_string_lossy().into_owned(), missing.clone()] {
            set_field(&mut a, key, json!(path)).unwrap();
            let tips = !a.session.prefs.show_tool_tips;
            set_field(&mut a, "showToolTips", json!(tips)).unwrap();
            assert_eq!(confirm(&mut a), Err(format!("{path} is not a folder")), "{key}");
            assert!(is_open(&a), "{key}: the window stays open");
            assert_eq!(a.session.prefs.to_json(), before, "{key}: no preference changes");
            assert!(!a.prefs_changed, "{key}: nothing to save");
        }
        // A typed folder that passes is saved, with the spaces around it as typed.
        let typed = format!(" {} ", dir.to_string_lossy());
        set_field(&mut a, key, json!(typed)).unwrap();
        confirm(&mut a).unwrap();
        assert_eq!(a.session.prefs.to_json()[key], json!(typed), "{key}");
        // An empty field keeps the default folder.
        open(&mut a, Some("File Handling"));
        set_field(&mut a, key, json!("")).unwrap();
        confirm(&mut a).unwrap();
        assert_eq!(a.session.prefs.to_json()[key], json!(""), "{key}");
        // A folder already saved is not checked again, so OK works while it is unavailable.
        std::fs::create_dir(&missing).unwrap();
        a.run("prefs.set", json!({"key": key, "value": missing})).unwrap();
        std::fs::remove_dir(&missing).unwrap();
        open(&mut a, Some("File Handling"));
        let tips = !a.session.prefs.show_tool_tips;
        set_field(&mut a, "showToolTips", json!(tips)).unwrap();
        confirm(&mut a).unwrap();
        assert_eq!(a.session.prefs.to_json()[key], json!(missing), "{key}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Under automation roots, OK refuses a typed folder outside them before it looks at the folder.
#[test]
fn ok_refuses_a_typed_folder_outside_the_automation_roots_before_looking_at_it() {
    use vectorcraft_engine::file_access::{self, Access};
    let dir = temp_folder("typed-roots");
    let inside = dir.join("inside");
    std::fs::create_dir_all(&inside).unwrap();
    let outside = dir.join("outside-missing").to_string_lossy().into_owned();
    let roots = file_access::AutomationRoots::new(&[inside.as_path()], &[inside.as_path()]).unwrap().unwrap();
    for (key, access) in [("recoveryFolder", Access::Write), ("pluginsFolder", Access::Read)] {
        let mut a = app();
        open(&mut a, Some("File Handling"));
        set_field(&mut a, key, json!(outside)).unwrap();
        let r = file_access::confine(Some(&roots), || confirm(&mut a));
        assert_eq!(r, Err(roots.check(&outside, access).unwrap_err()), "{key}");
        assert!(is_open(&a), "{key}: the window stays open");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Over the categories from General to Devices plus Type, every setting has exactly one non-empty
/// sentence, and no key is listed twice.
#[test]
fn every_setting_from_general_to_devices_has_one_description() {
    let mut bad = vec![];
    let categories = [
        "General",
        "Type",
        "Selection & Anchor Display",
        "Guides & Grid",
        "Smart Guides",
        "Slices",
        "Hyphenation",
        "Plug-ins & Scratch Disks",
        "User Interface",
        "Performance",
        "File Handling",
        "Clipboard Handling",
        "Appearance of Black",
        "Devices",
    ];
    for s in PREF_SPECS.iter().filter(|s| categories.contains(&s.category)) {
        match descriptions::DESCRIPTIONS.iter().filter(|(k, _)| *k == s.key).count() {
            1 => {}
            n => bad.push(format!("`{}` ({}) has {n} sentences", s.key, s.label)),
        }
        if descriptions::description(s.key).is_some_and(|d| d.trim().is_empty()) {
            bad.push(format!("`{}` has an empty sentence", s.key));
        }
    }
    for (i, (key, _)) in descriptions::DESCRIPTIONS.iter().enumerate() {
        if descriptions::DESCRIPTIONS[..i].iter().any(|(k, _)| k == key) {
            bad.push(format!("`{key}` is listed twice"));
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Every sentence names a setting the window builds or the engine lists, and shows in its
/// catalog's translation where there is one, in English where there is none.
#[test]
fn every_description_names_a_setting_and_fits_in_every_language() {
    let mut a = app();
    open(&mut a, None);
    let row_keys =
        with_pages(&mut a, |pages| pages.iter().flat_map(|p| &p.groups).flat_map(|g| &g.rows).map(|r| r.key.to_string()).collect::<Vec<_>>());
    let mut bad = vec![];
    for (key, sentence) in descriptions::DESCRIPTIONS {
        let tail = format!(".{key}");
        if !PREF_SPECS.iter().any(|s| s.key == *key) && !row_keys.iter().any(|k| k == key || k.ends_with(&tail)) {
            bad.push(format!("`{key}` is not a setting the window shows"));
        }
        for lang in crate::i18n::Lang::all() {
            let text = crate::i18n::tr(lang, sentence);
            let translated = crate::i18n::has(lang, sentence);
            if lang != crate::i18n::Lang::EN && translated && text == *sentence {
                bad.push(format!("{}: the available translation for {sentence:?} was not used", lang.code()));
            }
            if !translated && text != *sentence {
                bad.push(format!("{}: missing {sentence:?} did not fall back to English", lang.code()));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// A kept script file grant, as the grants file holds it.
fn grant(script: &std::path::Path, path: &std::path::Path, sha256: &str, write: bool, folder: bool) -> Value {
    json!({"script": script.to_string_lossy(), "sha256": sha256, "code": [], "path": path.to_string_lossy(), "write": write, "folder": folder})
}

/// An app whose session keeps its script data in a new folder for test `name`, with a grants file
/// beside it that holds `grants`. → The app and the grants file.
fn app_with_grants(name: &str, grants: Value) -> (VectorcraftApp, std::path::PathBuf) {
    let base = temp_folder(name);
    let store = base.join(vectorcraft_engine::script::files::GRANTS_FILE);
    std::fs::write(&store, serde_json::to_vec_pretty(&json!({"version": 1, "grants": grants})).unwrap()).unwrap();
    let mut a = app();
    a.session.attach_script_data(base.join("Script Data"));
    (a, store)
}

/// The rows General › Script File Access shows, as (script, path, access, versions).
fn access_rows(a: &VectorcraftApp) -> Vec<(String, String, String, usize)> {
    script_access::shown_rows(a.prefs.as_ref().and_then(|w| w.script_access.as_ref()))
}

/// One frame of General › Script File Access in the window's context with `events` → the texts it drew, each with
/// its place, and the frame's output.
fn access_frame(a: &mut VectorcraftApp, events: Vec<egui::Event>) -> (Vec<(String, egui::Rect)>, egui::FullOutput) {
    let ctx = a.ui_ctx.clone().expect("a window context");
    let raw =
        egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0))), events, ..Default::default() };
    let out = crate::testing::part_frame(a, &ctx, raw, |a, ui| {
        let Some(mut w) = a.prefs.take() else { return };
        script_access::show(a, &mut w.script_access, ui);
        a.prefs = Some(w);
    });
    (crate::tests_removeanchors::shapes_text(&out.shapes.iter().map(|c| c.shape.clone()).collect::<Vec<_>>()), out)
}

/// The texts of General › Script File Access after the frames it lays itself out in.
fn access_texts(a: &mut VectorcraftApp) -> Vec<(String, egui::Rect)> {
    access_frame(a, vec![]);
    access_frame(a, vec![]).0
}

/// Click the first text `label` the section draws, with the pointer.
fn click_access(a: &mut VectorcraftApp, label: &str) {
    let texts = access_texts(a);
    let at = texts.iter().find(|(t, _)| t == label).map(|(_, r)| r.center()).unwrap_or_else(|| panic!("no `{label}` in {texts:?}"));
    let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    access_frame(a, vec![egui::Event::PointerMoved(at)]);
    access_frame(a, vec![press(true)]);
    access_frame(a, vec![press(false)]);
    access_frame(a, vec![]);
}

/// Revoke on the section's row `i`, as its button does.
fn revoke_row(a: &mut VectorcraftApp, i: usize) {
    let mut w = a.prefs.take().expect("Preferences is open");
    script_access::revoke(a, &mut w.script_access, i);
    a.prefs = Some(w);
}

/// General › Script File Access lists each script and path once, with the widest access any kept
/// version of the pair has, and Revoke removes every kept version of that pair through
/// `script.grants.revoke`, so the next run asks again; the other pairs stay.
#[test]
fn script_file_access_lists_each_script_and_path_once_and_revoke_removes_them() {
    let dir = std::env::temp_dir().join("vc-ui-prefs-grants-nowhere");
    let (a_js, b_js) = (dir.join("a.jsx"), dir.join("b.jsx"));
    let (out, folder) = (dir.join("out.txt"), dir.join("work"));
    let grants = json!([
        grant(&a_js, &out, "1", false, false),
        grant(&a_js, &out, "2", true, false),
        grant(&a_js, &folder, "1", true, true),
        grant(&b_js, &out, "3", false, false),
    ]);
    let (mut a, store) = app_with_grants("grants-list", grants);
    open(&mut a, Some("General"));
    let (a_s, b_s, out_s, folder_s) = (a_js.to_string_lossy(), b_js.to_string_lossy(), out.to_string_lossy(), folder.to_string_lossy());
    let row = |s: &str, p: &str, access: &str, n: usize| (s.to_string(), p.to_string(), access.to_string(), n);
    assert_eq!(
        access_rows(&a),
        [row(&a_s, &out_s, "write", 2), row(&a_s, &folder_s, "folder", 1), row(&b_s, &out_s, "read", 1)],
        "one row for each script and path, sorted by script and then path"
    );
    assert_eq!(row_state(&mut a, script_access::KEY).map(|r| r.0), Some(true), "General shows the section");
    let texts = access_texts(&mut a);
    let painted = |label: &str| texts.iter().filter(|(t, _)| t == label).count();
    let note = "Revoke removes the access kept for every version of the script. A folder's access covers everything in it.";
    for label in ["Script File Access", "Read and write file", "Read and write folder", "Read file", note] {
        assert_eq!(painted(label), 1, "{label:?} in {texts:?}");
    }
    assert_eq!(painted("Revoke"), 3, "a Revoke for each row: {texts:?}");
    assert_eq!(painted(&a_s), 1, "the script shows once over its rows: {texts:?}");
    assert_eq!(painted(&out_s), 2, "{texts:?}");
    let rect = |label: &str| texts.iter().find(|(t, _)| t == label).map(|(_, r)| *r).unwrap_or_else(|| panic!("no `{label}`"));
    let (path, access, revoke) = (rect(&out_s), rect("Read and write file"), rect("Revoke"));
    assert!(path.max.x < access.min.x && access.max.x < revoke.min.x, "a row's path, access and Revoke do not overlap: {texts:?}");
    // The first Revoke is the first row's: a.jsx and out.txt, kept for two versions.
    click_access(&mut a, "Revoke");
    assert_eq!(a.ui.status, "Script file access revoked");
    assert_eq!(access_rows(&a), [row(&a_s, &folder_s, "folder", 1), row(&b_s, &out_s, "read", 1)]);
    let kept = a.session.execute("script.grants", &json!({})).unwrap();
    assert_eq!(kept["grants"].as_array().unwrap().len(), 2, "{kept}");
    let stored: Value = serde_json::from_slice(&std::fs::read(&store).unwrap()).unwrap();
    assert!(stored["grants"].as_array().unwrap().iter().all(|g| !(g["script"] == *a_s && g["path"] == *out_s)), "{stored}");
    let _ = std::fs::remove_dir_all(store.parent().unwrap());
}

/// Revoke on a row whose grants were removed elsewhere removes nothing and says so; with no kept
/// grants the section says there are none, and a session without a settings folder (the web among
/// them) shows no section.
#[test]
fn script_file_access_shows_none_and_a_revoke_of_a_removed_row() {
    let dir = std::env::temp_dir().join("vc-ui-prefs-grants-nowhere");
    let (mut a, store) = app_with_grants("grants-gone", json!([grant(&dir.join("a.jsx"), &dir.join("out.txt"), "1", false, false)]));
    open(&mut a, Some("General"));
    assert_eq!(access_rows(&a).len(), 1);
    a.session.execute("script.grants.revoke", &json!({"all": true})).unwrap();
    revoke_row(&mut a, 0);
    assert_eq!(a.ui.status, "The script file access was already revoked");
    assert!(access_rows(&a).is_empty());
    let texts = access_texts(&mut a);
    for label in ["Script File Access", "No script has kept file access."] {
        assert!(texts.iter().any(|(t, _)| t == label), "{label:?} in {texts:?}");
    }
    let _ = std::fs::remove_dir_all(store.parent().unwrap());
    let mut web = app();
    open(&mut web, Some("General"));
    assert!(web.prefs.as_ref().unwrap().script_access.is_none());
    assert_eq!(row_state(&mut web, script_access::KEY), None, "no section without a settings folder");
    assert!(!fields(&web).unwrap().contains_key(script_access::ROWS));
    assert!(access_texts(&mut web).is_empty());
}

/// A kept grant with an empty script or path, which `script.grants` lists and `script.grants.revoke` does not take,
/// has no row.
#[test]
fn script_file_access_leaves_out_a_grant_with_an_empty_script_or_path() {
    let dir = std::env::temp_dir().join("vc-ui-prefs-grants-nowhere");
    let (script, path) = (dir.join("a.jsx"), dir.join("out.txt"));
    let grants = json!([
        grant(&script, &path, "1", false, false),
        grant(&script, "".as_ref(), "1", false, false),
        grant("".as_ref(), &path, "1", false, false)
    ]);
    let (mut a, store) = app_with_grants("grants-empty", grants);
    assert_eq!(a.session.execute("script.grants", &json!({})).unwrap()["grants"].as_array().map(Vec::len), Some(3));
    open(&mut a, Some("General"));
    let row = (script.to_string_lossy().into_owned(), path.to_string_lossy().into_owned(), "read".to_string(), 1);
    assert_eq!(access_rows(&a), [row]);
    let _ = std::fs::remove_dir_all(store.parent().unwrap());
}

/// Preferences opened with a read root in force shows a script and a path outside it by their file
/// names alone, as `script.grants` lists them, and `ui.inspect` names them the same way; Revoke
/// still removes that pair's kept grants, both with the root in force and without it.
#[test]
fn script_file_access_revokes_a_pair_outside_the_read_roots() {
    let dir = std::env::temp_dir().join("vc-ui-prefs-grants-nowhere");
    let (script, path) = (dir.join("a.jsx"), dir.join("out.txt"));
    let root = temp_folder("grants-root-in");
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[root.as_path()], &[]).unwrap();
    for confined in [true, false] {
        let (mut a, store) = app_with_grants("grants-outside-roots", json!([grant(&script, &path, "1", false, false)]));
        let ctx = a.ui_ctx.clone().expect("a window context");
        let (shown, inspected) = vectorcraft_engine::file_access::confine(roots.as_ref(), || {
            open(&mut a, Some("General"));
            (access_rows(&a), crate::control::inspect(&a, &ctx))
        });
        assert_eq!(shown, [("a.jsx".to_string(), "out.txt".to_string(), "read".to_string(), 1)]);
        // The person's frame draws the rows as the request that opened the window was shown them.
        let texts = access_texts(&mut a);
        let whole = dir.to_string_lossy();
        assert!(texts.iter().any(|(t, _)| t == "a.jsx") && !texts.iter().any(|(t, _)| t.contains(&*whole)), "{texts:?}");
        let row = &inspected["ui"]["dialog"]["fields"][script_access::ROWS][0];
        assert_eq!(json!({"script": row["script"], "path": row["path"]}), json!({"script": "a.jsx", "path": "out.txt"}), "{inspected}");
        let roots_now = if confined { roots.as_ref() } else { None };
        vectorcraft_engine::file_access::confine(roots_now, || revoke_row(&mut a, 0));
        assert_eq!(a.ui.status, "Script file access revoked", "confined: {confined}");
        assert!(access_rows(&a).is_empty(), "confined: {confined}");
        let stored: Value = serde_json::from_slice(&std::fs::read(&store).unwrap()).unwrap();
        assert!(stored["grants"].as_array().is_some_and(Vec::is_empty), "confined: {confined}: {stored}");
        let _ = std::fs::remove_dir_all(store.parent().unwrap());
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// A grants file that is not well formed or is larger than may be read is refused: Revoke shows
/// the error, grants nothing and leaves the file and the rows as they were.
#[test]
fn script_file_access_revoke_refuses_a_malformed_or_oversized_grants_file() {
    let dir = std::env::temp_dir().join("vc-ui-prefs-grants-nowhere");
    let mut oversized = br#"{"version": 1, "grants": []}"#.to_vec();
    oversized.resize((4 << 20) + 1, b' ');
    for (name, held) in
        [("malformed", br#"{"version": 1, "grants": [{"script": "x"}]}"#.to_vec()), ("broken", b"{ not json".to_vec()), ("oversized", oversized)]
    {
        let (mut a, store) = app_with_grants(&format!("grants-{name}"), json!([grant(&dir.join("a.jsx"), &dir.join("out.txt"), "1", false, false)]));
        open(&mut a, Some("General"));
        let before = access_rows(&a);
        assert_eq!(before.len(), 1, "{name}");
        std::fs::write(&store, &held).unwrap();
        revoke_row(&mut a, 0);
        assert!(a.ui.status.contains("could not be saved"), "{name}: {}", a.ui.status);
        assert_eq!(std::fs::read(&store).unwrap(), held, "{name}: the grants file is left as it was");
        assert_eq!(access_rows(&a), before, "{name}: the rows stay");
        let listed = a.session.execute("script.grants", &json!({}));
        assert!(
            listed.as_ref().is_ok_and(|v| v["grants"].as_array().is_some_and(Vec::is_empty)) || listed.is_err(),
            "{name}: nothing is granted: {listed:?}"
        );
        let _ = std::fs::remove_dir_all(store.parent().unwrap());
    }
}

/// Preferences opened over a grants file `script.grants` cannot read (not JSON, or over 4 MiB) shows
/// why under Script File Access instead of saying that no script has kept file access, and leaves the
/// file as it was; a grant in the file that is not well formed is left out of the rows.
#[test]
fn script_file_access_shows_why_the_grants_file_cannot_be_read() {
    let dir = std::env::temp_dir().join("vc-ui-prefs-grants-nowhere");
    let kept = grant(&dir.join("a.jsx"), &dir.join("out.txt"), "1", false, false);
    let malformed = serde_json::to_vec(&json!({"version": 1, "grants": [kept, {"script": "x"}]})).unwrap();
    let mut oversized = br#"{"version": 1, "grants": []}"#.to_vec();
    oversized.resize((4 << 20) + 1, b' ');
    for (name, held, rows, unread) in
        [("malformed", malformed, 1, false), ("broken", b"{ not json".to_vec(), 0, true), ("oversized", oversized, 0, true)]
    {
        let (mut a, store) = app_with_grants(&format!("grants-unread-{name}"), json!([]));
        std::fs::write(&store, &held).unwrap();
        open(&mut a, Some("General"));
        assert_eq!(access_rows(&a).len(), rows, "{name}");
        let error = script_access::error(a.prefs.as_ref().and_then(|w| w.script_access.as_ref())).map(str::to_string);
        assert_eq!(error.is_some(), unread, "{name}: {error:?}");
        let texts = access_texts(&mut a);
        if let Some(error) = &error {
            assert!(error.starts_with("the script file permissions could not be read: "), "{name}: {error:?}");
            assert!(texts.iter().any(|(t, _)| t == error), "{name}: {texts:?}");
            assert_eq!(fields(&a).unwrap()[script_access::ERROR], json!(error), "{name}");
        }
        assert!(!texts.iter().any(|(t, _)| t == "No script has kept file access."), "{name}: {texts:?}");
        assert_eq!(std::fs::read(&store).unwrap(), held, "{name}: the grants file is left as it was");
        let _ = std::fs::remove_dir_all(store.parent().unwrap());
    }
}

/// A script or path with control or bidirectional formatting characters shows them as escapes, so
/// it cannot break its row or draw as another name.
#[test]
fn script_file_access_shows_paths_as_plain_text() {
    let dir = std::env::temp_dir().join("vc-ui-prefs-grants-nowhere");
    let (script, path) = (dir.join("a\u{202E}txt.jsx"), dir.join("two\nlines.txt"));
    let (mut a, store) = app_with_grants("grants-plain", json!([grant(&script, &path, "1", false, false)]));
    open(&mut a, Some("General"));
    let (script_s, path_s) = (script_access::shown(&script.to_string_lossy()), script_access::shown(&path.to_string_lossy()));
    assert!(script_s.ends_with("a\\u{202E}txt.jsx") && path_s.ends_with("two\\u{000A}lines.txt"), "{script_s} {path_s}");
    let texts = access_texts(&mut a);
    assert!(texts.iter().any(|(t, _)| *t == script_s) && texts.iter().any(|(t, _)| *t == path_s), "{texts:?}");
    assert!(!texts.iter().any(|(t, _)| t.contains('\u{202E}') || t.contains("two\nlines")), "{texts:?}");
    let _ = std::fs::remove_dir_all(store.parent().unwrap());
}

/// Two paths that differ only after the characters a row shows keep their whole paths in the Revoke
/// buttons' screen-reader descriptions, so each button names the grant it removes.
#[test]
fn script_file_access_revoke_names_the_whole_path() {
    let mut long = std::env::temp_dir().join("vc-ui-prefs-grants-nowhere");
    for _ in 0..11 {
        long.push("d".repeat(100));
    }
    let (script, one, two) = (long.with_file_name("a.jsx"), long.join("1.txt"), long.join("2.txt"));
    let (mut a, store) = app_with_grants("grants-long", json!([grant(&script, &one, "1", false, false), grant(&script, &two, "1", false, false)]));
    open(&mut a, Some("General"));
    access_frame(&mut a, vec![]);
    let (_, out) = access_frame(&mut a, vec![]);
    let nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    let descriptions: Vec<String> =
        nodes.iter().filter(|(_, n)| n.label() == Some("Revoke")).filter_map(|(_, n)| n.description().map(str::to_string)).collect();
    for path in [&one, &two] {
        let whole = path.to_string_lossy();
        assert!(descriptions.iter().any(|d| d.ends_with(&*whole)), "{whole} in {descriptions:?}");
    }
    let _ = std::fs::remove_dir_all(store.parent().unwrap());
}
