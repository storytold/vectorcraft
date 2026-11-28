//! The Color Wheel panel's box view drawn headless through the `Fixture`: the field's axes, the
//! strip, `ui.colorWheel` and the saved settings.

use egui::{Key, Modifiers};
use serde_json::json;
use vectorcraft_color::Color;
use vectorcraft_color::cms;
use vectorcraft_engine::Session;

use super::color_wheel::{Channel, Settings, View, display_hsb};
use super::tests_color_wheel::Panel;
use crate::VectorcraftApp;
use crate::i18n::LangExt as _;

/// An app with a selected rectangle filled with `fill`, its Color Wheel panel on the box of
/// `channel`.
fn app(fill: serde_json::Value, channel: &str) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": fill})).unwrap();
    app.run("ui.colorWheel", json!({"view": "box", "channel": channel})).unwrap();
    app
}

fn fill(app: &VectorcraftApp) -> Color {
    crate::panels::current_paints(app).0.color().expect("a solid fill")
}

fn hsb(c: &Color) -> [f32; 3] {
    display_hsb(&cms::active(), c)
}

fn rgb(c: &Color) -> [f32; 3] {
    cms::active().display_rgb(c)
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// The field raises saturation to the right and brightness upward for Hue, and blue to the right
/// and green upward for Red, leaving the other components as they were.
#[test]
fn the_hue_box_picks_saturation_across_and_brightness_up() {
    let mut p = Panel::new(app(json!("#8c4d33"), "hue"));
    let before = hsb(&fill(&p.app()));
    p.key("Color box", Key::ArrowRight, Modifiers::NONE);
    let after = hsb(&fill(&p.app()));
    assert!(after[1] > before[1] && (after[2] - before[2]).abs() < 1e-3 && (after[0] - before[0]).abs() < 1e-2, "{before:?} → {after:?}");
    p.key("Color box", Key::ArrowUp, Modifiers::NONE);
    let up = hsb(&fill(&p.app()));
    assert!(up[2] > after[2] && (up[1] - after[1]).abs() < 1e-3, "{after:?} → {up:?}");

    let mut p = Panel::new(app(json!([0.2, 0.4, 0.6]), "red"));
    let before = rgb(&fill(&p.app()));
    p.key("Color box", Key::ArrowRight, Modifiers::NONE);
    let across = rgb(&fill(&p.app()));
    assert!(across[2] > before[2] && (across[1] - before[1]).abs() < 1e-3 && (across[0] - before[0]).abs() < 1e-3, "{before:?} → {across:?}");
    p.key("Color box", Key::ArrowUp, Modifiers::NONE);
    let up = rgb(&fill(&p.app()));
    assert!(up[1] > across[1] && (up[2] - across[2]).abs() < 1e-3, "{across:?} → {up:?}");
}

/// A strip edit sets only its channel, as one undo step.
#[test]
fn a_drag_in_the_field_is_one_undo_step_and_the_strip_sets_the_channel() {
    let mut p = Panel::new(app(json!([0.6, 0.3, 0.3]), "hue"));
    let (undo, before) = {
        let app = p.app();
        (undo_len(&app), hsb(&fill(&app)))
    };
    p.set("Hue", 180.0);
    let after = hsb(&fill(&p.app()));
    assert!((after[0] - 180.0).abs() < 2.0 && (after[1] - before[1]).abs() < 0.01 && (after[2] - before[2]).abs() < 0.01, "{after:?}");
    assert_eq!(undo_len(&p.app()), undo + 1, "one undo step");
}

/// `ui.colorWheel` sets the keys it is given and keeps the others, `{}` reads them, a value off the
/// list answers an error that names the accepted values and changes nothing, and `ui.inspect`
/// reports the settings.
#[test]
fn ui_color_wheel_sets_and_reads_the_view_and_the_channel() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let others = json!({
        "sliders": "none", "gamut": false, "harmony": false, "harmonyRule": "complementary", "opacity": false, "hexField": false, "contrast": false,
        "contrastFor": "normalText", "contrastBackground": null, "vision": false, "documentColors": false
    });
    let with = |v: serde_json::Value| {
        let mut v = v;
        v.as_object_mut().unwrap().extend(others.as_object().unwrap().clone());
        v
    };
    assert_eq!(app.run("ui.colorWheel", json!({})).unwrap(), with(json!({"view": "wheel", "channel": "hue"})));
    assert_eq!(app.run("ui.colorWheel", json!({"view": "box"})).unwrap(), with(json!({"view": "box", "channel": "hue"})));
    assert_eq!(app.run("ui.colorWheel", json!({"channel": "green"})).unwrap(), with(json!({"view": "box", "channel": "green"})));
    let e = app.run("ui.colorWheel", json!({"view": "triangle"})).unwrap_err();
    assert_eq!(e, "`view` must be wheel or box");
    let e = app.run("ui.colorWheel", json!({"view": "wheel", "channel": 7})).unwrap_err();
    assert_eq!(e, "`channel` must be hue, saturation, brightness, red, green or blue");
    assert_eq!(app.ui.color_wheel, Settings { view: View::Box, channel: Channel::Green, ..Default::default() }, "an error changes nothing");
    let ui = crate::control::inspect(&app, &crate::testing::ui_context());
    assert_eq!(ui["ui"]["color_wheel"], with(json!({"view": "box", "channel": "green"})));
}

/// The command's errors are translated in the catalogs that translate status and error messages,
/// and each translation keeps the accepted values.
#[test]
fn ui_color_wheel_errors_are_translated() {
    let errors = [
        ("`view` must be wheel or box", &["wheel", "box"][..]),
        ("`channel` must be hue, saturation, brightness, red, green or blue", &["hue", "saturation", "brightness", "red", "green", "blue"][..]),
    ];
    for code in [
        "cs", "de", "el", "es", "fr", "hi", "hu", "it", "ja", "ka", "ko", "pl", "pt-br", "ru", "sv", "uk", "vi", "zh-hans", "zh-hant", "fi", "tr",
        "ca", "nl", "bs", "da", "id",
    ] {
        let lang = crate::i18n::Lang::from_code(code).expect("registered");
        for (error, values) in errors {
            let shown = crate::i18n::message(lang, error);
            assert_ne!(shown, error, "{code}");
            assert!(values.iter().all(|v| shown.contains(v)), "{code}: {shown}");
        }
    }
}

/// Params that are not an object, and a key the command does not take, are errors that change
/// nothing.
#[test]
fn ui_color_wheel_rejects_other_params() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("ui.colorWheel", json!({"view": "box", "channel": "red"})).unwrap();
    let e = app.run("ui.colorWheel", json!("box")).unwrap_err();
    assert_eq!(e, "the params must be an object, not \"box\"");
    let e = app.run("ui.colorWheel", json!([1])).unwrap_err();
    assert_eq!(e, "the params must be an object, not [1]");
    let e = app.run("ui.colorWheel", json!({"veiw": "wheel"})).unwrap_err();
    assert_eq!(
        e,
        "unknown parameter `veiw`; the command takes: view, channel, sliders, gamut, harmony, harmonyRule, opacity, hexField, contrast, contrastFor, contrastBackground, vision, documentColors"
    );
    let e = app.run("ui.colorWheel", json!({"view": "wheel", "chanel": "blue"})).unwrap_err();
    assert_eq!(
        e,
        "unknown parameter `chanel`; the command takes: view, channel, sliders, gamut, harmony, harmonyRule, opacity, hexField, contrast, contrastFor, contrastBackground, vision, documentColors"
    );
    assert_eq!(app.ui.color_wheel, Settings { view: View::Box, channel: Channel::Red, ..Default::default() }, "an error changes nothing");
}

/// `ui.colorWheel` answers every junk parameter without a panic.
#[test]
fn ui_color_wheel_answers_junk_params_without_a_panic() {
    let doc = crate::menus::UI_COMMANDS.iter().find(|c| c.0 == "ui.colorWheel").expect("registered").3;
    let mut app = app(json!("#336699"), "hue");
    for p in vectorcraft_testkit::strategies::junk_params(doc, "/nonexistent").into_iter().chain(vectorcraft_testkit::strategies::junk_values()) {
        let _ = app.run("ui.colorWheel", p);
    }
}

/// The settings survive a JSON round trip of the preferences, and a saved value this build does
/// not know reads as the key's default without losing the other preferences.
#[test]
fn the_settings_are_kept_and_read_leniently() {
    let ui = crate::UiState {
        color_wheel: Settings { view: View::Box, channel: Channel::Blue, gamut: true, vision: true, ..Default::default() },
        ..Default::default()
    };
    let mut saved = serde_json::to_value(&ui).unwrap();
    saved["color_panel"] = json!({"rgbPercent": true});
    let back: crate::UiState = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(back.color_wheel, ui.color_wheel);
    let mut odd = saved.clone();
    odd["color_wheel"] = json!({"view": "triangle", "channel": 7, "later": true});
    let back: crate::UiState = serde_json::from_value(odd).unwrap();
    assert_eq!(back.color_wheel, Settings::default());
    assert!(back.color_panel.rgb_percent, "the other preferences are kept");
    // Each key reads on its own: an unknown channel does not reset a known view, or the reverse.
    let mut odd = saved.clone();
    odd["color_wheel"] = json!({"view": "box", "channel": 7});
    let back: crate::UiState = serde_json::from_value(odd).unwrap();
    assert_eq!(back.color_wheel, Settings { view: View::Box, channel: Channel::default(), ..Default::default() });
    let mut odd = saved.clone();
    odd["color_wheel"] = json!({"view": "triangle", "channel": "blue"});
    let back: crate::UiState = serde_json::from_value(odd).unwrap();
    assert_eq!(back.color_wheel, Settings { view: View::Wheel, channel: Channel::Blue, ..Default::default() });
    let mut odd = saved;
    odd["color_wheel"] = json!("box");
    let back: crate::UiState = serde_json::from_value(odd).unwrap();
    assert_eq!(back.color_wheel, Settings::default());
    assert!(back.color_panel.rgb_percent);
}

/// A strip edit in an RGB channel's box that lands on a gray keeps the hue the panel showed.
#[test]
fn box_pick_inverts_box_pos_for_every_channel() {
    // A cyan: the Red strip set to the green the Green box's strip shows lands on a gray.
    let mut p = Panel::new(app(json!([0.3, 0.6, 0.6]), "green"));
    let green = p.number("Green").expect("the green strip has a value");
    p.run("ui.colorWheel", json!({"channel": "red"}));
    p.set("Red", green);
    assert_eq!(hsb(&fill(&p.app()))[1], 0.0, "a gray");
    p.run("ui.colorWheel", json!({"channel": "hue"}));
    let hue = p.number("Hue").expect("the hue strip has a value");
    assert!((hue - 180.0).abs() < 2.0, "the hue the panel showed: {hue}");
}
