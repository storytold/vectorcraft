//! Numeric color controls edit the working RGB channels, including colors outside display sRGB.
// Integration tests may unwrap and panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard};

use egui::{Rect, vec2};
use serde_json::json;
use vectorcraft_color::Color;
use vectorcraft_color::cms::{self, ColorSettings, DISPLAY_P3, PROPHOTO_RGB, SRGB, WIDE_GAMUT_RGB};
use vectorcraft_engine::Session;
use vectorcraft_tools::params::color_json;
use vectorcraft_ui_egui::VectorcraftApp;
use vectorcraft_ui_egui::panels::color::{Mode, components, convert_to, from_components, hex_digits, parse_hex, show, web_safe};

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

/// An ordinary selected rectangle for Color panel hover and Shift-click checks.
fn spectrum_app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app
}

/// Every string actually painted by a headless frame, including a settled hover tooltip.
fn spectrum_painted_texts(output: &egui::FullOutput) -> Vec<String> {
    fn collect(shape: &egui::Shape, texts: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => texts.push(t.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|shape| collect(shape, texts)),
            _ => {}
        }
    }
    let mut texts = vec![];
    output.shapes.iter().for_each(|shape| collect(&shape.shape, &mut texts));
    texts
}

/// Draw the real Color panel and return a point over its spectrum and the frame's text.
fn spectrum_frame(
    app: &mut VectorcraftApp,
    ctx: &egui::Context,
    time: f64,
    events: Vec<egui::Event>,
    modifiers: egui::Modifiers,
) -> (egui::Pos2, Vec<String>) {
    let screen = Rect::from_min_size(egui::Pos2::ZERO, vec2(260.0, 700.0));
    let events = std::iter::once(egui::Event::ModifiersChanged(modifiers)).chain(events).collect();
    let mut at = None;
    let input = egui::RawInput { screen_rect: Some(screen), time: Some(time), events, ..Default::default() };
    let mut output = ctx.run_ui(input, |ui| {
        show(app, ui);
        at = if app.ui.color_hide_options {
            // The existing hidden-options spectrum test uses this point right of the proxy.
            let row = ui.min_rect();
            Some(egui::pos2(row.right() - 4.0, row.center().y))
        } else {
            ctx.read_response(ui.scope_id().with("color-spectrum")).map(|r| r.rect.center())
        };
    });
    output.textures_delta.clear();
    (at.expect("the spectrum is drawn"), spectrum_painted_texts(&output))
}

/// The panel's drawn text after the pointer has rested on or away from the spectrum.
fn spectrum_hover_texts(app: &mut VectorcraftApp, ctx: &egui::Context, over: bool) -> (egui::Pos2, Vec<String>) {
    let away = egui::pos2(255.0, 695.0);
    let time = ctx.input(|i| i.time) + 1.0;
    let none = egui::Modifiers::NONE;
    let (at, _) = spectrum_frame(app, ctx, time, vec![egui::Event::PointerMoved(away)], none);
    let pos = if over { at } else { away };
    spectrum_frame(app, ctx, time + 0.1, vec![egui::Event::PointerMoved(pos)], none);
    // Allow the tooltip delay and its following layout pass, as the Appearance panel tests do.
    spectrum_frame(app, ctx, time + 2.0, vec![], none);
    (at, spectrum_frame(app, ctx, time + 2.1, vec![], none).1)
}

#[test]
fn spectrum_tooltip_names_its_displayed_mode() {
    let _space = WorkingSpace::new(SRGB);
    for hidden in [false, true] {
        for mode in [Mode::Grayscale, Mode::Rgb, Mode::Hsb, Mode::Cmyk, Mode::WebSafe, Mode::Lab] {
            let mut app = spectrum_app();
            let color = convert_to(mode, &Color::rgb8(51, 102, 153));
            app.run("paint.setFill", json!({"color": color_json(&color), "keepModel": true})).unwrap();
            // Lab follows the selected paint even when the stored mode is RGB.
            app.ui.color_mode = Some(if mode == Mode::Lab { Mode::Rgb } else { mode });
            app.ui.color_hide_options = hidden;
            let ctx = egui::Context::default();
            ctx.global_style_mut(|style| style.interaction.tooltip_delay = 0.1);
            let paints = app.session.proxy_paints();
            let undo = app.session.active().unwrap().history.undo.len();
            let (_, away) = spectrum_hover_texts(&mut app, &ctx, false);
            assert!(!away.iter().any(|text| text == mode.label()), "{mode:?}, hidden {hidden}: no tooltip away from the spectrum: {away:?}");
            let (_, hovered) = spectrum_hover_texts(&mut app, &ctx, true);
            assert_eq!(
                hovered.iter().filter(|text| text.as_str() == mode.label()).count(),
                1,
                "{mode:?}, hidden {hidden}: the tooltip names the displayed mode: {hovered:?}"
            );
            let (_, away) = spectrum_hover_texts(&mut app, &ctx, false);
            assert!(!away.iter().any(|text| text == mode.label()), "{mode:?}, hidden {hidden}: the tooltip hides again: {away:?}");
            assert_eq!(app.session.proxy_paints(), paints, "hovering changes no paint");
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "hovering adds no undo step");
        }
    }
}

#[test]
fn spectrum_tooltip_follows_shift_cycle_without_painting() {
    let _space = WorkingSpace::new(SRGB);
    for hidden in [false, true] {
        let mut app = spectrum_app();
        app.run("paint.setFill", json!({"color": "#336699"})).unwrap();
        app.ui.color_mode = Some(Mode::Rgb);
        app.ui.color_hide_options = hidden;
        let ctx = egui::Context::default();
        ctx.global_style_mut(|style| style.interaction.tooltip_delay = 0.1);
        let paints = app.session.proxy_paints();
        let undo = app.session.active().unwrap().history.undo.len();
        let (pos, hovered) = spectrum_hover_texts(&mut app, &ctx, true);
        assert!(hovered.iter().any(|text| text == "RGB"));
        let modifiers = egui::Modifiers::SHIFT;
        let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
        let time = ctx.input(|i| i.time) + 1.0;
        spectrum_frame(&mut app, &ctx, time, vec![egui::Event::PointerMoved(pos), button(true)], modifiers);
        spectrum_frame(&mut app, &ctx, time + 0.1, vec![button(false)], modifiers);
        spectrum_frame(&mut app, &ctx, time + 0.2, vec![], egui::Modifiers::NONE);
        assert_eq!(app.ui.color_mode, Some(Mode::Hsb));
        let (_, hovered) = spectrum_hover_texts(&mut app, &ctx, true);
        assert!(hovered.iter().any(|text| text == "HSB") && !hovered.iter().any(|text| text == "RGB"), "hidden {hidden}: {hovered:?}");
        assert_eq!(app.session.proxy_paints(), paints, "RGB to HSB changes no paint");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "cycling to HSB adds no undo step");
    }
}

/// Spectrum hover text uses the selected UI language, in both panel layouts.
#[test]
fn spectrum_tooltip_uses_the_interface_language() {
    use vectorcraft_ui_egui::i18n::{self, Lang};

    // i18n is process-global in this integration binary. Every case holds the CMS lock.
    // Restore the language before WorkingSpace releases that lock, including after a panic.
    let _space = WorkingSpace::new(SRGB);
    struct Language(Lang);
    impl Drop for Language {
        fn drop(&mut self) {
            i18n::set_current(self.0);
        }
    }
    let _language = Language(i18n::current());
    let german = Lang::from_code("de").unwrap();
    let mode = Mode::Grayscale;
    let expected = "Graustufen";
    assert_eq!(i18n::tr(german, mode.label()), expected, "use an existing, distinct catalog translation");

    for hidden in [false, true] {
        let mut app = spectrum_app();
        app.session.prefs.interface_language = german.code().into();
        // The host normally does this in tick_frame; this harness draws the panel directly.
        assert_eq!(app.ui_language(), german);
        i18n::set_current(app.ui_language());
        let color = convert_to(mode, &Color::rgb8(51, 102, 153));
        app.run("paint.setFill", json!({"color": color_json(&color), "keepModel": true})).unwrap();
        app.ui.color_mode = Some(mode);
        app.ui.color_hide_options = hidden;
        let ctx = egui::Context::default();
        ctx.global_style_mut(|style| style.interaction.tooltip_delay = 0.1);
        let paints = app.session.proxy_paints();
        let undo = app.session.active().unwrap().history.undo.len();
        let (_, away) = spectrum_hover_texts(&mut app, &ctx, false);
        assert!(!away.iter().any(|text| text == expected), "hidden {hidden}: no translated tooltip away: {away:?}");
        let (_, hovered) = spectrum_hover_texts(&mut app, &ctx, true);
        assert_eq!(hovered.iter().filter(|text| text.as_str() == expected).count(), 1, "hidden {hidden}: translated tooltip: {hovered:?}");
        assert!(!hovered.iter().any(|text| text == mode.label()), "hidden {hidden}: no untranslated tooltip: {hovered:?}");
        let (_, away) = spectrum_hover_texts(&mut app, &ctx, false);
        assert!(!away.iter().any(|text| text == expected), "hidden {hidden}: translated tooltip hides again: {away:?}");
        assert_eq!(app.session.proxy_paints(), paints, "translated hover changes no paint");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "translated hover adds no undo step");
    }
}
