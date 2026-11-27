//! Desktop backend preferences use the native window's draft and public automation route.
//! These headless frames and restore cases do not create a graphics device or verify physical blur.
#![cfg(any(
    windows,
    target_os = "macos",
    target_os = "linux",
    target_os = "freebsd",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "dragonfly"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard};

use craft::automation::{ControlRequest, Reply};
use craft::ui::runtime::frame::take_output;
use craft::ui::shell::embed;
use egui::{Pos2, RawInput, Rect, Shape, vec2};
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{VectorcraftApp, control, i18n, prefs_dialog};

static UI_STATE: Mutex<()> = Mutex::new(());

struct LanguageGuard {
    language: i18n::Lang,
    _lock: MutexGuard<'static, ()>,
}

impl LanguageGuard {
    fn new() -> Self {
        let lock = UI_STATE.lock().unwrap_or_else(|e| e.into_inner());
        let language = i18n::current();
        i18n::set_current(i18n::Lang::EN);
        Self { language, _lock: lock }
    }
}

impl Drop for LanguageGuard {
    fn drop(&mut self) {
        i18n::set_current(self.language);
    }
}

fn text(shape: &Shape, out: &mut Vec<String>) {
    match shape {
        Shape::Text(t) => out.push(t.galley.text().to_string()),
        Shape::Vec(shapes) => shapes.iter().for_each(|s| text(s, out)),
        _ => {}
    }
}

struct Scene {
    app: VectorcraftApp,
    ctx: egui::Context,
    time: f64,
}

impl Scene {
    fn new() -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("app.language", json!({"lang": "en"})).unwrap();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        embed::install(&ctx, craft::ui::testing::install_spec(), &vectorcraft_ui_egui::ui_fonts()).unwrap();
        let mut scene = Self { app, ctx, time: 0.0 };
        scene.settle();
        scene
    }

    fn frame(&mut self) -> Vec<String> {
        self.time += 0.02;
        let mut raw = RawInput {
            time: Some(self.time),
            focused: true,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 900.0))),
            ..Default::default()
        };
        self.app.adopt_context(&self.ctx);
        i18n::set_current(self.app.ui_language());
        self.app.raw_input(&self.ctx, &mut raw);
        embed::before_input(&self.ctx, |out| self.app.frame_input(out));
        let mut out = self.ctx.run_ui(raw, |ui| {
            self.app.tick(ui.ctx());
            self.app.ui(ui);
        });
        let _ = self.app.frame_input(take_output(&self.ctx));
        let mut shown = Vec::new();
        out.shapes.iter().for_each(|s| text(&s.shape, &mut shown));
        out.textures_delta.clear();
        shown
    }

    fn settle(&mut self) -> Vec<String> {
        let mut shown = Vec::new();
        for _ in 0..6 {
            shown = self.frame();
        }
        shown
    }

    fn request(&mut self, method: &str, params: Value) {
        let (req, _) = ControlRequest::new(method, params);
        let Reply::Done(reply) = control::handle(&mut self.app, &self.ctx, &req) else { panic!("expected immediate Preferences reply") };
        assert_eq!(reply["ok"], true, "{method}: {reply}");
    }

    fn draft(&self) -> Value {
        control::inspect(&self.app, &self.ctx)["ui"]["dialog"]["fields"].clone()
    }
}

#[test]
fn graphics_backend_shows_its_restart_note_and_uses_preferences_ok_cancel() {
    let _language = LanguageGuard::new();
    let mut scene = Scene::new();
    prefs_dialog::open(&mut scene.app, Some("Performance"));
    // Match this row's complete note: another startup-only setting cannot satisfy the oracle.
    scene.request("ui.dialog.set", json!({"dialog": "preferences", "field": "__search", "value": "Graphics Backend"}));
    let shown = scene.settle();
    assert!(shown.iter().any(|t| t.starts_with("Graphics Backend")), "{shown:?}");
    let backend_note = concat!(
        "Applies the next time VectorCraft starts.\n",
        "From the next start, the desktop window uses this graphics API. ",
        "Automatic keeps the platform default; WGPU_BACKEND overrides this choice. ",
        "The artwork is drawn on the CPU either way."
    );
    assert!(shown.iter().any(|t| t == backend_note), "{shown:?}");
    let chosen = if cfg!(target_os = "macos") { "metal" } else { "vulkan" };
    let saved_before = scene.app.ui.engine_prefs.clone();
    let processor = scene.app.session.prefs.gpu_preference.clone();
    let latency = scene.app.session.prefs.low_latency_drawing;
    let _ = scene.app.take_prefs_changed();
    scene.request("ui.dialog.set", json!({"dialog": "preferences", "field": "gpuBackend", "value": chosen}));
    scene.settle();
    assert_eq!(scene.draft()["gpuBackend"], chosen, "the actual Preferences model holds the edited choice");
    assert_eq!(scene.app.session.prefs.to_json()["gpuBackend"], "automatic", "editing the working copy does not apply it");
    assert!(!scene.app.take_prefs_changed());
    scene.request("ui.dialog.cancel", json!({"dialog": "preferences"}));
    assert_eq!(scene.app.session.prefs.to_json()["gpuBackend"], "automatic");
    assert_eq!(scene.app.ui.engine_prefs, saved_before);
    assert!(!scene.app.take_prefs_changed());
    prefs_dialog::open(&mut scene.app, Some("Performance"));
    scene.request("ui.dialog.set", json!({"dialog": "preferences", "field": "gpuBackend", "value": chosen}));
    scene.request("ui.dialog.confirm", json!({"dialog": "preferences"}));
    assert!(scene.app.take_prefs_changed(), "OK asks the host to save for the next start");
    assert!(!scene.app.take_prefs_changed());
    assert_eq!(scene.app.session.prefs.to_json()["gpuBackend"], chosen);
    assert_eq!(scene.app.session.prefs.gpu_preference, processor);
    assert_eq!(scene.app.session.prefs.low_latency_drawing, latency);
    let saved: Value = serde_json::from_slice(&serde_json::to_vec(&scene.app.ui).unwrap()).unwrap();
    assert_eq!(saved["engine_prefs"]["gpuBackend"], chosen);
    prefs_dialog::open(&mut scene.app, Some("Performance"));
    assert_eq!(scene.draft()["gpuBackend"], chosen);
}

#[test]
fn graphics_backend_restore_keeps_old_preferences_and_supported_choices() {
    let _language = LanguageGuard::new();
    let chosen = if cfg!(target_os = "macos") { "metal" } else { "vulkan" };
    let unsupported = if cfg!(target_os = "macos") { "vulkan" } else { "metal" };
    let cases = [
        (None, "automatic"),
        (Some(json!("unknown")), "automatic"),
        (Some(json!(unsupported)), "automatic"),
        (Some(json!(chosen)), chosen),
        (Some(Value::Null), "automatic"),
        (Some(json!({"future": "backend"})), "automatic"),
    ];
    for (value, expected) in cases {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui.engine_prefs = json!({"keyboardIncrement":4,"gpuPreference":"highPerformance","lowLatencyDrawing":true});
        if let Some(value) = value {
            app.ui.engine_prefs["gpuBackend"] = value;
        }
        prefs_dialog::restore(&mut app);
        assert_eq!(app.session.prefs.to_json()["gpuBackend"], expected);
        assert_eq!(app.session.prefs.keyboard_increment, 4.0);
        assert_eq!(app.session.prefs.gpu_preference, "highPerformance");
        assert!(app.session.prefs.low_latency_drawing);
    }
}
