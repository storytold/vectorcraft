//! Numeric color controls edit the working RGB channels, including colors outside display sRGB.
// Integration tests may unwrap and panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard};

use serde_json::json;
use vectorcraft_color::Color;
use vectorcraft_color::cms::{self, ColorSettings, DISPLAY_P3, PROPHOTO_RGB, SRGB, WIDE_GAMUT_RGB};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::VectorcraftApp;
use vectorcraft_ui_egui::panels::color::{color_of_values, model_values, show};

// This integration binary contains only these tests; its app-wide CMS is independent of other
// test binaries. Serialize its cases and restore settings even when an assertion fails.
static CMS: Mutex<()> = Mutex::new(());

struct WorkingSpace {
    previous: ColorSettings,
    _lock: MutexGuard<'static, ()>,
}

impl WorkingSpace {
    /// Select a working profile for one case without retaining it after a panic.
    fn new(rgb: &str) -> Self {
        let lock = CMS.lock().unwrap_or_else(|e| e.into_inner());
        let previous = cms::active_settings();
        cms::set_active(&ColorSettings { rgb: rgb.into(), ..ColorSettings::default() }).unwrap();
        Self { previous, _lock: lock }
    }
}

impl Drop for WorkingSpace {
    fn drop(&mut self) {
        cms::set_active(&self.previous).unwrap();
    }
}

/// Compare native channel values without converting them through the display profile.
fn native(color: Color, expected: [f32; 3]) {
    let Color::Rgb { r, g, b } = color else { panic!("expected RGB, got {color:?}") };
    for (actual, expected) in [r, g, b].into_iter().zip(expected) {
        assert!((actual - expected).abs() < 1e-6, "{color:?}: expected {expected}, got {actual}");
    }
}

/// The values the panel shows are the stored working components, and an edit keeps the untouched
/// ones, in every working profile, in RGB and HSB and for colors outside the display gamut.
#[test]
fn rgb_edits_preserve_untouched_working_channels() {
    for profile in [SRGB, DISPLAY_P3, WIDE_GAMUT_RGB, PROPHOTO_RGB] {
        let _space = WorkingSpace::new(profile);
        let mut values = model_values(&Color::rgb(0.5, 0.3, 0.2));
        assert_eq!(values.model, "rgb");
        for (actual, expected) in values.values.iter().zip([127.5, 76.5, 51.0]) {
            assert!((actual - expected).abs() < 1e-4, "{profile}: {:?}", values.values);
        }
        // Editing R keeps G and B.
        let rgb = values.values;
        values.values[0] = 153.0;
        native(color_of_values(&values), [0.6, 0.3, 0.2]);
        // HSB describes the native RGB triplet; changing hue keeps its saturation and brightness.
        values.model = "hsb";
        values.values[..3].copy_from_slice(&[140.0, 60.0, 50.0]);
        native(color_of_values(&values), [0.2, 0.5, 0.3]);
        let _ = rgb;
        // Native primaries survive the round trip even when display conversion clips their gamut.
        for native_rgb in [[1.0, 0.2, 0.0], [0.0, 1.0, 0.2], [0.2, 0.0, 1.0], [1.0, 0.05, 0.0]] {
            let color = Color::rgb(native_rgb[0], native_rgb[1], native_rgb[2]);
            native(color_of_values(&model_values(&color)), native_rgb);
        }
    }
}

/// The picker's R, G and B fields show the stored RGB components in every working profile.
#[test]
fn picker_rgb_fields_show_working_values() {
    use egui_kittest::kittest::Queryable;

    for profile in [SRGB, DISPLAY_P3, WIDE_GAMUT_RGB, PROPHOTO_RGB] {
        let _space = WorkingSpace::new(profile);
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("ui.colorPicker", json!({"color": [0.6, 0.4, 0.2]})).unwrap();
        let mut fonts = false;
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 900.0)).with_accessibility_check(false).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    vectorcraft_ui_egui::theme::install_fonts(ui.ctx());
                    return;
                }
                vectorcraft_ui_egui::dialogs::show(app, ui.ctx());
            },
            app,
        );
        harness.run_steps(3);
        // HSB's Brightness field is also labelled B, so the fields labelled B are read together.
        for (label, expected) in [("R", "153"), ("G", "102"), ("B", "51")] {
            let values: Vec<String> = harness
                .query_all_by_role_and_label(egui::accesskit::Role::TextInput, label)
                .filter_map(|field| field.value().map(|v| v.to_string()))
                .collect();
            assert!(values.iter().any(|v| v == expected), "{profile}: {label} shows {values:?}");
        }
    }
}

/// A panel slider click changes one native component and makes one undo step.
#[test]
fn color_panel_slider_preserves_native_channels_and_undo() {
    use egui_kittest::kittest::Queryable;

    let _space = WorkingSpace::new(DISPLAY_P3);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": [0.5, 0.3, 0.2]})).unwrap();
    let mut fonts = false;
    let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(320.0, 400.0)).with_accessibility_check(false).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                vectorcraft_ui_egui::theme::install_fonts(ui.ctx());
                return;
            }
            show(app, ui);
        },
        app,
    );
    harness.run_steps(3);
    harness.get_by_role_and_label(egui::accesskit::Role::Slider, "R").click();
    harness.run_steps(3);
    let color = harness.state().session.proxy_paints().0.color().unwrap();
    let Color::Rgb { r, .. } = color else { panic!("expected RGB") };
    assert_ne!(r, 0.5, "the slider was edited");
    native(color, [r, 0.3, 0.2]);
    harness.state_mut().run("edit.undo", json!({})).unwrap();
    native(harness.state().session.proxy_paints().0.color().unwrap(), [0.5, 0.3, 0.2]);
}
