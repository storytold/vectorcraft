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
use craft_gpu_device::{AdapterKey, Candidate, Preference};
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
        let mut app = VectorcraftApp::new(Session::new(), Default::default()).with_graphics_adapters(vec![adapter()]);
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

/// An original synthetic adapter; building its row opens no hardware or renderer.
fn adapter() -> Candidate {
    Candidate {
        name: "Synthetic GPU".into(),
        backend: wgpu::Backend::Vulkan,
        device_type: wgpu::DeviceType::DiscreteGpu,
        vendor: 0x1002,
        device: 0x73df,
        pci_bus_id: "0000:03:00.0".into(),
        driver: "synthetic-driver".into(),
        driver_info: String::new(),
        features: 1,
        usable: true,
    }
}

#[test]
fn graphics_backend_shows_its_restart_note_and_uses_preferences_ok_cancel() {
    let _language = LanguageGuard::new();
    let mut scene = Scene::new();
    prefs_dialog::open(&mut scene.app, Some("Performance"));
    scene.request("ui.dialog.set", json!({"dialog": "preferences", "field": "__search", "value": "Graphics Processor"}));
    let shown = scene.settle();
    assert!(shown.iter().any(|t| t.starts_with("Graphics Processor")), "{shown:?}");
    assert!(!shown.iter().any(|t| t.starts_with("Graphics Backend")), "one shared processor choice");
    let processor_note = concat!(
        "Applies the next time VectorCraft starts.\n",
        "From the next start, the desktop window uses this graphics processor. ",
        "Automatic and Low Power use the shared adapter policy; an unavailable saved adapter falls back to Automatic. ",
        "The artwork is drawn on the CPU either way."
    );
    assert!(shown.iter().any(|t| t == processor_note), "{shown:?}");
    let chosen = Preference::Adapter(AdapterKey::of(&adapter()));
    let value = json!(chosen);
    let saved_before = scene.app.ui.engine_prefs.clone();
    let latency = scene.app.session.prefs.low_latency_drawing;
    let _ = scene.app.take_prefs_changed();
    scene.request("ui.dialog.set", json!({"dialog": "preferences", "field": "gpuPreference", "value": value}));
    scene.settle();
    assert_eq!(scene.draft()["gpuPreference"], value, "the actual draft keeps the full adapter object");
    assert_eq!(scene.app.session.prefs.gpu_preference, Preference::Auto, "editing does not apply it");
    assert!(!scene.app.take_prefs_changed());
    scene.request("ui.dialog.cancel", json!({"dialog": "preferences"}));
    assert_eq!(scene.app.session.prefs.gpu_preference, Preference::Auto);
    assert_eq!(scene.app.ui.engine_prefs, saved_before);
    assert!(!scene.app.take_prefs_changed());
    prefs_dialog::open(&mut scene.app, Some("Performance"));
    scene.request("ui.dialog.set", json!({"dialog": "preferences", "field": "gpuPreference", "value": value}));
    scene.request("ui.dialog.confirm", json!({"dialog": "preferences"}));
    assert!(scene.app.take_prefs_changed(), "OK asks the host to save for the next start");
    assert!(!scene.app.take_prefs_changed());
    assert_eq!(scene.app.session.prefs.gpu_preference, chosen);
    assert_eq!(scene.app.session.prefs.low_latency_drawing, latency);
    assert!(scene.app.session.prefs.to_json().get("gpuBackend").is_none());
    let saved: Value = serde_json::from_slice(&serde_json::to_vec(&scene.app.ui).unwrap()).unwrap();
    assert_eq!(saved["engine_prefs"]["gpuPreference"], value);
    prefs_dialog::open(&mut scene.app, Some("Performance"));
    assert_eq!(scene.draft()["gpuPreference"], value);
}

#[test]
fn graphics_backend_restore_keeps_old_preferences_and_supported_choices() {
    let _language = LanguageGuard::new();
    // A valid saved adapter is retained on a different platform or while unplugged. Selection,
    // not settings-file parsing, later supplies the shared AdapterMissing fallback notice.
    let chosen = Preference::Adapter(AdapterKey::of(&adapter()));
    let cases = [
        (None, Preference::Auto),
        (Some(json!("unknown")), Preference::Auto),
        (Some(json!("powerSaving")), Preference::Auto),
        (Some(json!("lowPower")), Preference::LowPower),
        (Some(json!("highPerformance")), Preference::Auto),
        (Some(json!(chosen)), chosen),
        (Some(json!({"kind": "lowPower"})), Preference::LowPower),
        (Some(json!({"kind": "software"})), Preference::Software),
        (Some(Value::Null), Preference::Auto),
        (Some(json!({"future": "backend"})), Preference::Auto),
    ];
    for (value, expected) in cases {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.ui.engine_prefs = json!({"keyboardIncrement":4,"lowLatencyDrawing":true,"gpuBackend":"vulkan"});
        if let Some(value) = value {
            app.ui.engine_prefs["gpuPreference"] = value;
        }
        prefs_dialog::restore(&mut app);
        assert_eq!(app.session.prefs.gpu_preference, expected);
        assert_eq!(app.session.prefs.keyboard_increment, 4.0);
        assert!(app.session.prefs.low_latency_drawing);
        assert!(app.session.prefs.to_json().get("gpuBackend").is_none(), "the obsolete independent API selector is gone");
    }
}
