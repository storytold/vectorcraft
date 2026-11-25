//! The Color Wheel panel's Show Out-of-Gamut Colors: the map the panel hands the area,
//! `ui.colorWheel {gamut}` and the saved setting.

use serde_json::json;
use vectorcraft_color::Color;
use vectorcraft_color::cms::{self, Cms, ColorSettings};
use vectorcraft_engine::Session;

use super::color_wheel::{Channel, Settings, View};
use super::tests_color_wheel::Panel;
use crate::VectorcraftApp;
use crate::i18n::LangExt as _;

/// An app with a selected rectangle filled with `fill`, its Color Wheel panel on `view` with Show
/// Out-of-Gamut Colors set to `gamut`.
fn app(fill: &str, view: &str, gamut: bool) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": fill})).unwrap();
    app.run("ui.colorWheel", json!({"view": view, "channel": "hue", "gamut": gamut})).unwrap();
    app
}

/// With uncalibrated CMYK as the CMYK space every color counts as printable, as the out-of-gamut
/// warning has it, so the map marks nothing.
#[test]
fn uncalibrated_cmyk_marks_nothing() {
    let settings = ColorSettings { cmyk: cms::DEVICE_CMYK.into(), ..ColorSettings::default() };
    let device = std::sync::Arc::new(Cms::new(&settings).unwrap());
    let app = app("#0000ff", "box", true);
    let gamut = super::color::gamut(&app);
    // The map reads the active settings as it tests a color: uncalibrated CMYK on this thread.
    cms::with_active(device, || {
        for c in [Color::rgb(0.0, 0.0, 1.0), Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 0.0)] {
            assert!(gamut.nearest(super::srgba(&c)).is_none(), "{c:?}");
        }
    });
}

/// A profile loaded again under the same name can change which colors are printable while the
/// color settings stay the same; the panel keeps drawing through the change.
#[test]
fn the_grid_is_computed_again_after_a_profile_is_loaded() {
    let mut p = Panel::new(app("#3366cc", "box", true));
    /// Unregisters the profile when dropped, also after a failed assertion.
    struct Unregister(&'static str);
    impl Drop for Unregister {
        fn drop(&mut self) {
            cms::unregister_icc(self.0);
        }
    }
    let name = "Color Wheel Gamut Reload CMYK";
    let bytes = cms::icc_bytes(cms::GENERIC_CMYK).unwrap();
    cms::register_icc(&bytes, Some(name.into())).unwrap();
    let _unregister = Unregister(name);
    p.fx.idle(2);
    assert!(p.fx.by_label("Color box").is_some());
}

/// The wheel's square and the Hue box both show with the setting on.
#[test]
fn the_wheel_and_the_hue_box_shade_the_same_cells() {
    let mut p = Panel::new(app("#3366cc", "wheel", true));
    assert!(p.fx.by_label("Saturation and brightness").is_some());
    p.run("ui.colorWheel", json!({"view": "box"}));
    assert!(p.fx.by_label("Color box").is_some());
}

/// With Show Out-of-Gamut Colors off, the panel still draws and the setting reads off.
#[test]
fn with_the_setting_off_no_grid_is_computed() {
    let p = Panel::new(app("#3366cc", "box", false));
    assert!(!p.app().ui.color_wheel.gamut);
    assert!(p.fx.by_label("Color box").is_some());
}

/// `ui.colorWheel {gamut}` sets the setting and keeps the others; a key set to null keeps its value
/// while the other keys apply; a value that is not true or false is an error that changes nothing,
/// translated where messages are.
#[test]
fn ui_color_wheel_sets_gamut() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let with = |v: serde_json::Value| {
        let mut v = v;
        let rest = json!({
            "opacity": false, "hexField": false, "sliders": "none", "harmony": false, "harmonyRule": "complementary", "vision": false,
            "documentColors": false
        });
        v.as_object_mut().unwrap().extend(rest.as_object().unwrap().clone());
        v
    };
    app.run("ui.colorWheel", json!({"view": "box"})).unwrap();
    assert_eq!(app.run("ui.colorWheel", json!({"gamut": true})).unwrap(), with(json!({"view": "box", "channel": "hue", "gamut": true})));
    let e = app.run("ui.colorWheel", json!({"view": "wheel", "gamut": "yes"})).unwrap_err();
    assert_eq!(e, "`gamut` must be true or false");
    assert_eq!(app.ui.color_wheel, Settings { view: View::Box, gamut: true, ..Default::default() }, "an error changes nothing");
    assert_eq!(app.run("ui.colorWheel", json!({"gamut": null})).unwrap()["gamut"], json!(true), "null keeps the value");
    app.run("ui.colorWheel", json!({"channel": "red"})).unwrap();
    let got = app.run("ui.colorWheel", json!({"view": "wheel", "channel": null, "gamut": null})).unwrap();
    assert_eq!(got, with(json!({"view": "wheel", "channel": "red", "gamut": true})), "null keeps a value while the other keys apply");
    let got = app.run("ui.colorWheel", json!({"view": null, "gamut": false})).unwrap();
    assert_eq!(got, with(json!({"view": "wheel", "channel": "red", "gamut": false})), "a null view keeps the view while gamut applies");
    for code in ["de", "fr", "ja", "zh-hans"] {
        let lang = crate::i18n::Lang::from_code(code).expect("registered");
        let shown = crate::i18n::message(lang, &e);
        assert_ne!(shown, e, "{code}");
        assert!(shown.contains("`gamut`"), "{code}: {shown}");
    }
}

/// The setting survives a JSON round trip of the preferences, and a saved value that is not true
/// or false reads as off without losing the other settings.
#[test]
fn the_gamut_setting_is_kept_and_read_leniently() {
    let ui = crate::UiState { color_wheel: Settings { view: View::Box, gamut: true, ..Default::default() }, ..Default::default() };
    let saved = serde_json::to_value(&ui).unwrap();
    assert_eq!(saved["color_wheel"]["gamut"], json!(true));
    let back: crate::UiState = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(back.color_wheel, ui.color_wheel);
    let mut odd = saved;
    odd["color_wheel"] = json!({"view": "box", "channel": "red", "gamut": "yes"});
    let back: crate::UiState = serde_json::from_value(odd).unwrap();
    assert_eq!(back.color_wheel, Settings { view: View::Box, channel: Channel::Red, gamut: false, ..Default::default() });
}
