//! The preferences' runtime half: carrying the engine preferences in the UI state, restoring them
//! after a load, and pushing the UI-side ones into the renderer and the toolbar each frame.

use vectorcraft_engine::Prefs;
use vectorcraft_engine::cmd::prefscmds::GPU_PREFERENCES;

use crate::VectorcraftApp;
use crate::i18n::LangExt as _;
use crate::theme::Brightness;

/// Copy the engine prefs into `UiState` so the host persists them with the UI state.
pub fn snapshot(app: &mut VectorcraftApp) {
    app.ui.engine_prefs = app.session.prefs.to_json();
}

/// Restore engine prefs from `UiState` after the host loaded saved UI state, and hand its shortcut set to the key
/// stage.
pub fn restore(app: &mut VectorcraftApp) {
    crate::shortcut_editor::use_active_set(app);
    let mut p: Prefs = serde_json::from_value(app.ui.engine_prefs.clone()).unwrap_or_default();
    if app.ui.engine_prefs.is_null() {
        // Older preference files: keep the saved brightness.
        p.ui_brightness = app.ui.brightness.id().into();
    }
    // Older preference files kept the interface language with the UI state; English was the
    // default there, so only another language is carried over (onto an unset preference).
    if let Some(code) = app.ui.legacy_language.take()
        && p.interface_language == "auto"
        && code != "en"
        && crate::i18n::Lang::from_code(&code).is_some()
    {
        p.interface_language = code;
    }
    // 0.5.0 saved its default, `powerSaving`, which Automatic replaced (#502): any value this
    // version doesn't offer reads as the default, as the desktop app reads it at startup.
    if !GPU_PREFERENCES.iter().any(|(v, _)| *v == p.gpu_preference) {
        p.gpu_preference = Prefs::default().gpu_preference;
    }
    app.session.apply_prefs(p);
}

/// What the host last pushed that needs a restart of something when it changes.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct Applied {
    threads: i32,
}

/// User Interface › Canvas Color: the canvas around the artboards, or `None` to match the
/// interface brightness. The greys keep the white page's edge in view (#663).
pub(crate) fn canvas_color(key: &str) -> Option<egui::Color32> {
    match key {
        "white" => Some(egui::Color32::WHITE),
        "lightGray" => Some(egui::Color32::from_gray(0xc8)),
        "mediumGray" => Some(egui::Color32::from_gray(0x96)),
        "darkGray" => Some(egui::Color32::from_gray(0x5a)),
        _ => None,
    }
}

/// Per frame: push UI-side preferences into the renderer and the toolbar when they change. The
/// theme, the UI scale and Show Tool Tips reach the framework through the frame's root values.
pub fn apply_runtime(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let p = &app.session.prefs;
    if let Some(b) = Brightness::parse(&p.ui_brightness)
        && b != app.ui.brightness
    {
        app.ui.brightness = b;
    }
    let want = Applied { threads: p.render_threads };
    // The first push only records: the renderer starts with the saved count.
    if app.applied_prefs.replace(want).is_some_and(|prev| prev != want) {
        vectorcraft_render::set_default_threads(u16::try_from(want.threads).ok());
        app.canvas.renderer.threads = vectorcraft_render::default_threads();
        app.canvas.worker = None;
        app.canvas.worker_started = false;
        app.canvas.key = None;
    }
    let touch = ctx.input(|i| i.has_touch_screen());
    if touch != app.touch_screen {
        app.touch_screen = touch;
        crate::workspaces::sync_touch_workspace(app, touch);
    }
}
