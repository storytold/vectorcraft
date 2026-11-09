//! Numeric color controls edit the working RGB channels, including colors outside display sRGB.
// Integration tests may unwrap and panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard};

use serde_json::json;
use vectorcraft_color::Color;
use vectorcraft_color::cms::{self, ColorSettings, DISPLAY_P3, PROPHOTO_RGB, SRGB, WIDE_GAMUT_RGB};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::VectorcraftApp;
use vectorcraft_ui_egui::panels::color::{Mode, components, from_components, hex_digits, parse_hex, web_safe};

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

/// Editing R preserves the stored G and B components.
#[test]
fn rgb_edits_preserve_untouched_working_channels() {
    for profile in [SRGB, DISPLAY_P3, WIDE_GAMUT_RGB, PROPHOTO_RGB] {
        let _space = WorkingSpace::new(profile);
        let color = Color::rgb(0.5, 0.3, 0.2);
        let mut values = components(Mode::Rgb, &color);
        for (actual, expected) in values.iter().zip([127.5, 76.5, 51.0]) {
            assert!((actual - expected).abs() < 1e-4, "{profile}: {values:?}");
        }
        values[0] = 153.0;
        native(from_components(Mode::Rgb, &values), [0.6, 0.3, 0.2]);
    }
}

/// HSB describes the native RGB triplet; changing hue preserves its saturation and brightness.
#[test]
fn hsb_edits_preserve_native_saturation_and_brightness() {
    for profile in [SRGB, DISPLAY_P3, WIDE_GAMUT_RGB, PROPHOTO_RGB] {
        let _space = WorkingSpace::new(profile);
        let mut values = components(Mode::Hsb, &Color::rgb(0.5, 0.3, 0.2));
        for (actual, expected) in values.iter().zip([20.0, 60.0, 50.0]) {
            assert!((actual - expected).abs() < 1e-4, "{profile}: {values:?}");
        }
        values[0] = 140.0;
        native(from_components(Mode::Hsb, &values), [0.2, 0.5, 0.3]);
    }
}

/// Native primaries survive numeric round trips even when display conversion clips their gamut.
#[test]
fn numeric_roundtrips_preserve_out_of_display_gamut_colors() {
    {
        let _space = WorkingSpace::new(DISPLAY_P3);
        let red = Color::rgb(1.0, 0.0, 0.0);
        let red_with_green = Color::rgb(1.0, 0.05, 0.0);
        assert_eq!(red.to_rgb(), red_with_green.to_rgb(), "display clipping loses their native distinction");
        native(from_components(Mode::Rgb, &components(Mode::Rgb, &red_with_green)), [1.0, 0.05, 0.0]);
    }
    for profile in [DISPLAY_P3, WIDE_GAMUT_RGB, PROPHOTO_RGB] {
        let _space = WorkingSpace::new(profile);
        for rgb in [[1.0, 0.2, 0.0], [0.0, 1.0, 0.2], [0.2, 0.0, 1.0]] {
            let color = Color::rgb(rgb[0], rgb[1], rgb[2]);
            for mode in [Mode::Rgb, Mode::Hsb] {
                native(from_components(mode, &components(mode, &color)), rgb);
            }
        }
    }
}

/// Opening the picker reads HSB from the stored RGB components.
#[test]
fn picker_initialization_preserves_working_values() {
    for profile in [SRGB, DISPLAY_P3, WIDE_GAMUT_RGB, PROPHOTO_RGB] {
        let _space = WorkingSpace::new(profile);
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("ui.colorPicker", json!({"color": [0.5, 0.3, 0.2]})).unwrap();
        let dialog = app.ui.dialog.as_ref().unwrap();
        let hsb = dialog.fields.get("__hsb").unwrap().as_array().unwrap();
        for (actual, expected) in hsb.iter().zip([20.0, 0.6, 0.5]) {
            assert!((actual.as_f64().unwrap() - expected).abs() < 1e-4, "{profile}: {hsb:?}");
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

/// Shared hex and web-safe numeric controls round the native working RGB components.
#[test]
fn shared_hex_and_websafe_preserve_working_values() {
    for profile in [SRGB, DISPLAY_P3, WIDE_GAMUT_RGB, PROPHOTO_RGB] {
        let _space = WorkingSpace::new(profile);
        let color = Color::rgb(0.5, 0.3, 0.2);
        assert_eq!(hex_digits(&color), "804D33", "{profile}");
        native(parse_hex("804D33").unwrap(), [128.0 / 255.0, 77.0 / 255.0, 0.2]);
        native(web_safe(&color), [0.6, 0.4, 0.2]);
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
            vectorcraft_ui_egui::panels::color::show(app, ui);
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
