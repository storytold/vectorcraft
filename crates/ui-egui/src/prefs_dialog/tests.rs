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

    // A document in the default unit, with another unit saved: the row opens showing the default, and the reset
    // still resets the saved unit.
    let mut c = app();
    c.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    c.run("prefs.set", json!({"values": {"unitsGeneral": "inches"}, "documentUnits": false})).unwrap();
    assert_eq!(c.session.active().unwrap().doc.units.key(), Prefs::default().units_general);
    open(&mut c, Some("General"));
    reset_preferences(&mut c, PromptChoice::Proceed);
    confirm(&mut c).unwrap();
    assert_eq!(c.session.prefs.units_general, Prefs::default().units_general, "the saved unit is reset");
    assert_eq!(c.session.active().unwrap().doc.units.key(), Prefs::default().units_general, "the document's units stay");
}

/// Smart Guides, Snap to Grid and Enable Contextual Task Bar, the UI-only rows.
fn ui_rows(a: &VectorcraftApp) -> (bool, bool, bool) {
    (a.ui.view.smart_guides, a.ui.view.snap_to_grid, a.ui.task_bar)
}

fn set_ui_rows(a: &mut VectorcraftApp, rows: (bool, bool, bool)) {
    (a.ui.view.smart_guides, a.ui.view.snap_to_grid, a.ui.task_bar) = rows;
}

/// The Contextual Task Bar workspace `name` was left with.
fn left_task_bar(a: &VectorcraftApp, name: &str) -> bool {
    let id = &crate::workspaces::find(&a.ui, name).expect("a workspace").id;
    a.ui.workspaces.edited.get(id).expect("left").task_bar
}

/// Reset Preferences returns every UI-only row (Smart Guides, Snap to Grid, Enable Contextual Task Bar) to its
/// default in the draft, the value a new app shows. OK applies them; Cancel keeps the values in use.
#[test]
fn reset_preferences_resets_the_ui_only_rows() {
    let default = ui_rows(&app());
    let changed = (!default.0, !default.1, !default.2);
    let reset_changed = |a: &mut VectorcraftApp| {
        set_ui_rows(a, changed);
        open(a, Some("General"));
        reset_preferences(a, PromptChoice::Proceed);
    };
    let mut a = app();
    reset_changed(&mut a);
    assert_eq!(
        (field(&a, "__smartGuides"), field(&a, "__snapToGrid"), field(&a, "__taskBar")),
        (json!(default.0), json!(default.1), json!(default.2))
    );
    assert_eq!(ui_rows(&a), changed, "nothing applied before OK");
    confirm(&mut a).unwrap();
    assert_eq!(ui_rows(&a), default, "OK applies the defaults");

    let mut b = app();
    reset_changed(&mut b);
    cancel(&mut b);
    assert!(!is_open(&b));
    assert_eq!(ui_rows(&b), changed, "Cancel keeps the values in use");
}

/// A page's own reset also resets the UI-only row of that page, and only that one.
#[test]
fn a_page_reset_resets_the_ui_only_row_of_the_page() {
    let mut a = app();
    let default = ui_rows(&a);
    set_ui_rows(&mut a, (!default.0, !default.1, !default.2));
    // The saved Edit Original choice has no setter outside its module; the UI state's saved form sets it.
    let chosen = json!({"path": "editor"});
    let editor = |a: &VectorcraftApp| serde_json::to_value(&a.ui.original_editor).unwrap_or_default();
    a.ui.original_editor = serde_json::from_value(chosen.clone()).unwrap();
    let reset_page = |a: &mut VectorcraftApp, page: &str| {
        open(a, Some(page));
        let mut w = a.prefs.take().expect("Preferences is open");
        model::Live { app: &mut *a, w: &mut w }.reset_page(page);
        a.prefs = Some(w);
    };
    reset_page(&mut a, "General");
    assert_eq!(field(&a, "__taskBar"), json!(default.2), "General: Enable Contextual Task Bar");
    assert_eq!((field(&a, "__smartGuides"), field(&a, "__snapToGrid")), (json!(!default.0), json!(!default.1)), "the other pages' rows");
    confirm(&mut a).unwrap();
    assert_eq!(editor(&a), chosen, "General keeps the Edit Original choice");
    reset_page(&mut a, "File Handling");
    confirm(&mut a).unwrap();
    assert_eq!(editor(&a)["path"], Value::Null, "File Handling resets the Edit Original choice");
}

/// `prefs.reset` (the Command Palette's Reset Preferences) also returns the UI-only rows to their defaults, as a
/// batch step too; with a category, only that category's rows; with an unknown or malformed category, none.
#[test]
fn prefs_reset_resets_the_ui_only_rows_of_its_category() {
    let default = ui_rows(&app());
    let changed = (!default.0, !default.1, !default.2);
    let mut a = app();
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    set_ui_rows(&mut a, changed);
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 7})).unwrap();
    for bad in [json!("Nowhere"), json!(["General"]), json!(5)] {
        assert!(a.run("prefs.reset", json!({"category": bad})).is_err(), "{bad}");
        assert_eq!(ui_rows(&a), changed, "{bad}: an unknown or malformed category resets nothing");
        assert_eq!(a.session.prefs.keyboard_increment, 7.0, "{bad}: the preferences are kept");
    }
    a.run("prefs.reset", json!({"category": "general"})).unwrap();
    assert_eq!(ui_rows(&a), (changed.0, changed.1, default.2), "General: Enable Contextual Task Bar only");
    a.run("command.batch", json!({"commands": [{"command": "prefs.reset", "params": {"category": "Guides & Grid"}}]})).unwrap();
    assert_eq!(ui_rows(&a), (changed.0, default.1, default.2), "Guides & Grid, as a batch step: Snap to Grid only");
    a.run("prefs.reset", json!({"category": "Smart Guides"})).unwrap();
    assert_eq!(ui_rows(&a), default, "Smart Guides: Smart Guides");
    set_ui_rows(&mut a, changed);
    a.run("prefs.reset", json!({})).unwrap();
    assert_eq!(ui_rows(&a), default, "every category");
}

/// A `prefs.reset` run while Preferences is open shows the reset values in the draft, for the categories it reset,
/// so OK keeps the reset; rows of other categories keep the draft's values.
#[test]
fn prefs_reset_while_preferences_is_open_shows_the_reset_values() {
    let fresh = app();
    let default = ui_rows(&fresh);
    let mut a = app();
    set_ui_rows(&mut a, (!default.0, !default.1, !default.2));
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 7})).unwrap();
    open(&mut a, Some("General"));
    a.run("prefs.reset", json!({"category": "General"})).unwrap();
    assert_eq!(field(&a, "__taskBar"), json!(default.2), "the window shows the reset row");
    assert_eq!(field(&a, "__smartGuides"), json!(!default.0), "a row of another category keeps its value");
    confirm(&mut a).unwrap();
    assert_eq!(ui_rows(&a), (!default.0, !default.1, default.2), "OK keeps the reset");
    assert_eq!(a.session.prefs.keyboard_increment, fresh.session.prefs.keyboard_increment, "OK keeps the reset preference");

    // Every category, the preference groups included.
    let group = fresh.session.prefs.to_json()["pixelSnap"].clone();
    let mut changed = group.clone();
    let flag = changed.as_object_mut().and_then(|o| o.values_mut().find(|v| v.is_boolean())).expect("a pixelSnap option is a boolean");
    *flag = json!(!flag.as_bool().unwrap_or_default());
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 7})).unwrap();
    a.run("prefs.set", json!({"key": "pixelSnap", "value": changed})).unwrap();
    open(&mut a, Some("General"));
    assert_eq!(field(&a, "pixelSnap"), changed);
    a.run("prefs.reset", json!({})).unwrap();
    confirm(&mut a).unwrap();
    assert_eq!(ui_rows(&a), default, "every category");
    assert_eq!(a.session.prefs.keyboard_increment, fresh.session.prefs.keyboard_increment);
    assert_eq!(a.session.prefs.to_json()["pixelSnap"], group, "OK keeps the reset group");

    // Run straight on the session, with OK before the next frame.
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 7})).unwrap();
    open(&mut a, Some("General"));
    a.session.execute("prefs.reset", &json!({"category": "General"})).unwrap();
    confirm(&mut a).unwrap();
    assert_eq!(a.session.prefs.keyboard_increment, fresh.session.prefs.keyboard_increment, "OK keeps a reset outside run");
}

/// Reset Preferences and `prefs.reset` give Enable Contextual Task Bar the value the current workspace was saved
/// with, which for a workspace saved with the bar off differs from the default.
#[test]
fn reset_preferences_gives_the_task_bar_the_workspaces_value() {
    let off = crate::workspaces::builtins().into_iter().find(|w| !w.record.task_bar).expect("a workspace saved with the bar off");
    let in_workspace = |a: &mut VectorcraftApp| {
        crate::workspaces::switch(a, off.name).unwrap();
        assert!(!a.ui.task_bar, "{} starts with the bar off", off.name);
        a.ui.task_bar = true;
    };
    let mut a = app();
    in_workspace(&mut a);
    open(&mut a, Some("General"));
    reset_preferences(&mut a, PromptChoice::Proceed);
    assert_eq!(field(&a, "__taskBar"), json!(false), "the draft shows the workspace's value");
    confirm(&mut a).unwrap();
    assert!(!a.ui.task_bar, "OK applies the workspace's value");

    let mut b = app();
    in_workspace(&mut b);
    b.run("prefs.reset", json!({"category": "General"})).unwrap();
    assert!(!b.ui.task_bar, "prefs.reset gives the workspace's value");
    let mut c = app();
    assert!(c.ui.task_bar, "the default workspace starts with the bar on");
    c.ui.task_bar = false;
    c.run("prefs.reset", json!({})).unwrap();
    assert!(c.ui.task_bar, "the default workspace's value");
}

/// On a touch screen, OK after Reset Preferences turns Enable Touch Workspace back on: the reset task bar goes to
/// the workspace left, and Touch keeps its own. With Touch entered automatically while Preferences was open,
/// clearing Enable Touch Workspace gives the row to the workspace returned to.
#[test]
fn ok_sets_the_task_bar_in_the_workspace_outside_touch() {
    use crate::workspaces::{TOUCH, is_current, switch, sync_touch_workspace};
    let mut a = app();
    a.touch_screen = true;
    a.run("prefs.set", json!({"key": "touchWorkspace", "value": false})).unwrap();
    switch(&mut a, "Essentials Classic").unwrap();
    a.ui.task_bar = true;
    open(&mut a, Some("General"));
    reset_preferences(&mut a, PromptChoice::Proceed);
    assert_eq!(field(&a, "touchWorkspace"), json!(true), "the reset turns it on");
    confirm(&mut a).unwrap();
    assert!(is_current(&a.ui, TOUCH));
    assert!(a.ui.task_bar, "Touch keeps its own task bar");
    assert!(!left_task_bar(&a, "Essentials Classic"), "the workspace left takes the reset value");

    let mut b = app();
    b.run("prefs.set", json!({"key": "touchWorkspace", "value": true})).unwrap();
    switch(&mut b, "Essentials Classic").unwrap();
    open(&mut b, Some("General"));
    b.touch_screen = true;
    sync_touch_workspace(&mut b, true);
    assert!(is_current(&b.ui, TOUCH));
    set_field(&mut b, "touchWorkspace", json!(false)).unwrap();
    confirm(&mut b).unwrap();
    assert!(is_current(&b.ui, "Essentials Classic"));
    assert!(!b.ui.task_bar, "the workspace returned to takes the row");
    assert!(left_task_bar(&b, TOUCH), "Touch keeps its own task bar");
}

/// After an Apply that enters Touch, the task bar row shows Touch's task bar: a later Apply that leaves the row
/// unedited keeps it, and an edit or a reset of the row reaches it. Leaving Touch with the row unedited gives the
/// workspace returned to the value it left with.
#[test]
fn apply_after_entering_touch_keeps_touchs_task_bar() {
    use crate::workspaces::{TOUCH, is_current, switch};
    use craft::ui::patterns::preferences::PreferencesDraft as _;
    let apply = |a: &mut VectorcraftApp| {
        let mut w = a.prefs.take().expect("Preferences is open");
        let r = model::Live { app: &mut *a, w: &mut w }.apply();
        a.prefs = Some(w);
        r.unwrap();
    };
    let mut a = app();
    a.touch_screen = true;
    a.run("prefs.set", json!({"key": "touchWorkspace", "value": false})).unwrap();
    switch(&mut a, "Essentials Classic").unwrap();
    a.ui.task_bar = true;
    open(&mut a, Some("General"));
    reset_preferences(&mut a, PromptChoice::Proceed);
    assert_eq!(field(&a, "__taskBar"), json!(false), "the reset row differs from Touch's task bar");
    apply(&mut a);
    assert!(is_current(&a.ui, TOUCH));
    assert!(a.ui.task_bar, "Touch keeps its own task bar");
    set_field(&mut a, "keyboardIncrement", json!(9)).unwrap();
    apply(&mut a);
    assert!(is_current(&a.ui, TOUCH));
    assert!(a.ui.task_bar, "an Apply without a task bar edit keeps Touch's task bar");
    assert_eq!(field(&a, "__taskBar"), json!(true), "the row shows Touch's task bar");
    set_field(&mut a, "__taskBar", json!(false)).unwrap();
    apply(&mut a);
    assert!(!a.ui.task_bar, "an edit to the row reaches Touch");

    // Reset Preferences in Touch gives Touch its saved value, the value the row showed before the edit.
    reset_preferences(&mut a, PromptChoice::Proceed);
    apply(&mut a);
    assert!(is_current(&a.ui, TOUCH));
    assert!(a.ui.task_bar, "the reset reaches Touch");

    // Leaving Touch with the row unedited: the workspace returned to keeps the value it left with.
    set_field(&mut a, "touchWorkspace", json!(false)).unwrap();
    apply(&mut a);
    assert!(is_current(&a.ui, "Essentials Classic"));
    assert!(!a.ui.task_bar, "the workspace returned to keeps its reset value");
}

/// A Touch return marker left over from an earlier automatic switch (Window › Workspace does not clear it) does not
/// change where OK puts the task bar: OK enters Touch from the workspace it leaves, so that workspace takes the row
/// and Touch keeps its own.
#[test]
fn ok_sets_the_task_bar_in_the_workspace_left_with_a_stale_touch_marker() {
    use crate::workspaces::{TOUCH, is_current, switch};
    let mut a = app();
    a.touch_screen = true;
    a.run("prefs.set", json!({"key": "touchWorkspace", "value": false})).unwrap();
    a.ui.touch_workspace_restore = Some("Typography".into());
    switch(&mut a, "Essentials Classic").unwrap();
    a.ui.task_bar = true;
    open(&mut a, Some("General"));
    reset_preferences(&mut a, PromptChoice::Proceed);
    confirm(&mut a).unwrap();
    assert!(is_current(&a.ui, TOUCH));
    assert!(a.ui.task_bar, "Touch keeps its own task bar");
    assert!(!left_task_bar(&a, "Essentials Classic"), "the workspace left takes the reset value");
}

/// A `prefs.reset` run straight on the session, outside `VectorcraftApp::run`, resets the UI-only rows in the next
/// frame, and before the next command switches workspaces.
#[test]
fn prefs_reset_outside_run_resets_the_ui_only_rows_in_the_next_frame() {
    let mut a = app();
    let default = ui_rows(&a);
    set_ui_rows(&mut a, (!default.0, !default.1, !default.2));
    a.session.execute("prefs.reset", &json!({})).unwrap();
    let ctx = a.ui_ctx.clone().expect("a window context");
    crate::testing::app_frame(&mut a, &ctx, egui::RawInput::default());
    assert_eq!(ui_rows(&a), default);

    // Before the next command switches workspaces: the workspace left keeps the reset value.
    let mut b = app();
    crate::workspaces::switch(&mut b, "Essentials Classic").unwrap();
    b.ui.task_bar = true;
    b.session.execute("prefs.reset", &json!({"category": "General"})).unwrap();
    b.run("window.workspace", json!({"name": crate::workspaces::ESSENTIALS})).unwrap();
    assert!(b.ui.task_bar, "Essentials keeps its own task bar");
    assert!(!left_task_bar(&b, "Essentials Classic"), "the workspace left keeps the reset value");
}

/// OK writes only the rows changed in Preferences: a preference, a UI-only row or the document's units changed by a
/// command while the window is open keeps its new value, and a row changed in the window is still written.
#[test]
fn ok_keeps_settings_changed_by_commands_while_preferences_is_open() {
    let mut a = app();
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let before = ui_rows(&a);
    open(&mut a, Some("General"));
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 9})).unwrap();
    a.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    for id in ["view.smartGuides", "view.snapToGrid", "window.taskBar"] {
        a.run(id, json!({})).unwrap();
    }
    set_field(&mut a, "constrainAngle", json!(30.0)).unwrap();
    confirm(&mut a).unwrap();
    assert_eq!(a.session.prefs.keyboard_increment, 9.0, "the command's preference");
    assert_eq!(a.session.active().unwrap().doc.units, vectorcraft_doc::Unit::Millimeters, "the command's units");
    assert_eq!(ui_rows(&a), (!before.0, !before.1, !before.2), "the commands' UI-only rows");
    assert_eq!(a.session.prefs.constrain_angle, 30.0, "the row changed in the window");
    assert_eq!(a.ui.engine_prefs["keyboardIncrement"], json!(9.0), "saved with the UI state");

    // A row changed both ways takes the window's value; Cancel writes nothing.
    open(&mut a, Some("General"));
    set_field(&mut a, "keyboardIncrement", json!(3)).unwrap();
    set_field(&mut a, "constrainAngle", json!(15.0)).unwrap();
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 5})).unwrap();
    confirm(&mut a).unwrap();
    assert_eq!(a.session.prefs.keyboard_increment, 3.0, "the window's value");
    open(&mut a, Some("General"));
    set_field(&mut a, "constrainAngle", json!(45.0)).unwrap();
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 6})).unwrap();
    cancel(&mut a);
    assert_eq!((a.session.prefs.keyboard_increment, a.session.prefs.constrain_angle), (6.0, 15.0), "Cancel writes nothing");

    // A reset in the window is written even where the row already showed the default: a command changed the
    // preference after the window opened.
    let default = Prefs::default().keyboard_increment;
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": default})).unwrap();
    open(&mut a, Some("General"));
    a.run("prefs.set", json!({"key": "keyboardIncrement", "value": 8})).unwrap();
    reset_preferences(&mut a, PromptChoice::Proceed);
    confirm(&mut a).unwrap();
    assert_eq!(a.session.prefs.keyboard_increment, default, "the reset is written");
}

/// A reset in Preferences that leaves a row showing the value it opened with, after a command changed the setting,
/// still enables Apply, and Apply writes it.
#[test]
fn apply_writes_a_reset_of_a_row_a_command_changed() {
    use craft::ui::patterns::preferences::PreferencesDraft as _;
    let live = |a: &mut VectorcraftApp, apply: bool| {
        let mut w = a.prefs.take().expect("Preferences is open");
        let mut m = model::Live { app: &mut *a, w: &mut w };
        let differs = m.differs();
        if apply {
            m.apply().unwrap();
        }
        a.prefs = Some(w);
        differs
    };
    let mut a = app();
    let on = a.ui.view.smart_guides;
    open(&mut a, Some("General"));
    assert!(!live(&mut a, false), "nothing to apply at open");
    a.run("view.smartGuides", json!({})).unwrap();
    reset_preferences(&mut a, PromptChoice::Proceed);
    assert_eq!(field(&a, "__smartGuides"), json!(on), "the row shows the value it opened with");
    assert!(live(&mut a, true), "Apply is enabled");
    assert_eq!(a.ui.view.smart_guides, on, "Apply writes the reset");
    assert!(!live(&mut a, false), "nothing left to apply");
}

/// OK with nothing changed in Preferences writes nothing: no preference, no undo step in the document and no save
/// of the settings.
#[test]
fn ok_with_nothing_changed_writes_nothing() {
    let mut a = app();
    a.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let undo = a.session.active().unwrap().history.undo.len();
    let prefs = a.session.prefs.clone();
    a.take_prefs_changed();
    open(&mut a, Some("Units"));
    confirm(&mut a).unwrap();
    assert!(!is_open(&a));
    assert_eq!(a.session.prefs, prefs);
    assert_eq!(a.session.active().unwrap().history.undo.len(), undo, "no undo step");
    assert!(!a.take_prefs_changed(), "the settings are not saved again");
}

/// Enable Contextual Task Bar left alone in Preferences stays with each workspace: a workspace switch while the
/// window is open keeps the bar of the workspace switched to, and the workspace left keeps its own.
#[test]
fn ok_after_a_workspace_switch_keeps_each_workspaces_task_bar() {
    let off = crate::workspaces::builtins().into_iter().find(|w| !w.record.task_bar).expect("a workspace saved with the bar off");
    let mut a = app();
    crate::workspaces::switch(&mut a, crate::workspaces::ESSENTIALS).unwrap();
    assert!(a.ui.task_bar, "Essentials starts with the bar on");
    open(&mut a, Some("General"));
    a.run("window.workspace", json!({"name": off.name})).unwrap();
    set_field(&mut a, "keyboardIncrement", json!(3)).unwrap();
    confirm(&mut a).unwrap();
    assert!(!a.ui.task_bar, "{} keeps its bar off", off.name);
    assert!(left_task_bar(&a, crate::workspaces::ESSENTIALS), "Essentials keeps its bar on");
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

/// A `prefs.reset` run straight on the session, then a touch screen that switches to Touch: the workspace left keeps
/// the reset value.
#[test]
fn a_pending_prefs_reset_applies_before_a_touch_switch() {
    use crate::workspaces::{TOUCH, is_current, switch, sync_touch_workspace};
    let mut a = app();
    a.run("prefs.set", json!({"key": "touchWorkspace", "value": false})).unwrap();
    switch(&mut a, "Essentials Classic").unwrap();
    a.ui.task_bar = true;
    a.session.execute("prefs.reset", &json!({})).unwrap();
    assert!(a.session.prefs.touch_workspace, "the reset turns Enable Touch Workspace on");
    sync_touch_workspace(&mut a, true);
    assert!(is_current(&a.ui, TOUCH));
    assert!(a.ui.task_bar, "Touch keeps its own task bar");
    assert!(!left_task_bar(&a, "Essentials Classic"), "the workspace left keeps the reset value");
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
