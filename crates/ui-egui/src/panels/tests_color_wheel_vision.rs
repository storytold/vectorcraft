//! The Color Wheel panel's Show Color Vision Preview row drawn headless through the `Fixture`: the
//! simulated colors and their names, a gray fill, no solid color, and `ui.colorWheel`.

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::cms::{self, ProofSetup, ProofTarget};
use vectorcraft_engine::Session;

use super::color_wheel::vision_colors;
use super::tests_color_wheel::Panel;
use crate::VectorcraftApp;

/// `#rrggbb` of display sRGB `rgb`, as the chips report it.
fn hex([r, g, b]: [f32; 3]) -> String {
    craft::theme::Srgba::new(r, g, b, 1.0).to_hex()
}

const NAMES: [&str; 4] = ["Protanopia", "Deuteranopia", "Tritanopia", "Achromatopsia"];

/// A document with a selected rectangle filled with `fill`, with Show Color Vision Preview on.
fn app(fill: Value) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": fill})).unwrap();
    app.run("ui.colorWheel", json!({"vision": true})).unwrap();
    app
}

fn fill(app: &VectorcraftApp) -> Color {
    super::current_paints(app).0.color().expect("a solid fill")
}

/// The protanopia, deuteranopia and tritanopia cells show the fill as View › Proof Setup's
/// simulations show it: each reports the simulation's hex value.
#[test]
fn the_chips_show_the_proof_setup_simulations_of_the_fill() {
    let p = Panel::new(app(json!("#3366cc")));
    let cms = cms::active();
    let srgb = cms.display_rgb(&fill(&p.app()));
    for (name, target) in
        [("Protanopia", ProofTarget::Protanopia), ("Deuteranopia", ProofTarget::Deuteranopia), ("Tritanopia", ProofTarget::Tritanopia)]
    {
        let want = hex(cms.proof_srgb(srgb, &ProofSetup { target, ..Default::default() }));
        assert_eq!(p.value(name).as_deref(), Some(want.as_str()), "{name}");
    }
}

/// The achromatopsia cell of a red fill is the gray with red's CIE L* (D50, about 54.29: 828282),
/// and the simulations of a pure red differ from it and from each other.
#[test]
fn the_achromatopsia_chip_is_the_gray_of_the_colors_lightness() {
    let cms = cms::active();
    let [p, d, t, a] = vision_colors(&cms, &Color::rgb(1.0, 0.0, 0.0));
    assert_eq!(hex(a), "#828282");
    assert_ne!(hex(p), hex(d));
    assert_ne!(hex(t), hex(a));
    let panel = Panel::new(app(json!("#ff0000")));
    assert_eq!(panel.value("Achromatopsia").as_deref(), Some("#828282"));
}

/// A gray fill is the same gray in all four simulations.
#[test]
fn a_gray_fill_is_the_same_gray_in_every_chip() {
    let cms = cms::active();
    let gray = Color::from_hex("#808080").unwrap();
    let want = hex(cms.display_rgb(&gray));
    for (name, sim) in NAMES.iter().zip(vision_colors(&cms, &gray)) {
        assert_eq!(hex(sim), want, "{name} shows {want}");
    }
}

/// With a None fill the cells carry only their names, with no color: a swatch reports a clear one.
#[test]
fn with_a_none_fill_the_chips_have_no_color() {
    let mut a = app(json!("#ff0000"));
    a.run("paint.setFill", json!({"none": true})).unwrap();
    let p = Panel::new(a);
    for name in NAMES {
        assert_eq!(p.value(name).as_deref(), Some("#00000000"), "{name} has no color");
    }
}

/// `ui.colorWheel` sets the keys given and reads them with `{}`, and `ui.inspect` reports them as
/// `ui.color_wheel`. A `vision` value other than true, false or null, params that are not an object
/// and another key are errors that change nothing, the view and the channel given with them
/// included.
#[test]
fn ui_color_wheel_sets_and_reads_the_settings() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let settings = |view: &str, vision: bool| json!({"view": view, "channel": "hue", "gamut": false, "harmony": false, "harmonyRule": "complementary", "vision": vision, "sliders": "none", "documentColors": false});
    assert_eq!(app.run("ui.colorWheel", json!({})).unwrap(), settings("wheel", false));
    assert_eq!(app.run("ui.colorWheel", json!({"vision": true})).unwrap(), settings("wheel", true));
    assert_eq!(crate::control::inspect(&app, &crate::testing::ui_context())["ui"]["color_wheel"]["vision"], true);
    assert_eq!(app.run("ui.colorWheel", json!({"vision": "no"})).unwrap_err(), "`vision` must be true or false");
    assert_eq!(app.run("ui.colorWheel", json!({"view": "box", "channel": "red", "vision": 1})).unwrap_err(), "`vision` must be true or false");
    assert_eq!(app.run("ui.colorWheel", json!({})).unwrap(), settings("wheel", true));
    assert_eq!(app.run("ui.colorWheel", json!("junk")).unwrap_err(), "the params must be an object, not \"junk\"");
    assert_eq!(app.run("ui.colorWheel", json!(5)).unwrap_err(), "the params must be an object, not 5");
    assert_eq!(
        app.run("ui.colorWheel", json!({"visoin": false})).unwrap_err(),
        "unknown parameter `visoin`; the command takes: view, channel, sliders, gamut, harmony, harmonyRule, vision, documentColors"
    );
    assert!(app.ui.color_wheel.vision);
    assert_eq!(app.run("ui.colorWheel", json!({"vision": null})).unwrap()["vision"], true);
    assert_eq!(app.run("ui.colorWheel", Value::Null).unwrap()["vision"], true);
    // Setting the view or the channel keeps the row, and setting the row keeps them.
    assert_eq!(app.run("ui.colorWheel", json!({"view": "box"})).unwrap()["vision"], true);
    assert_eq!(app.run("ui.colorWheel", json!({"vision": false})).unwrap(), settings("box", false));
}

/// `ui.colorWheel` answers every junk parameter, with and without a document, without a panic.
#[test]
fn ui_color_wheel_answers_junk_params_without_a_panic() {
    let doc = crate::menus::UI_COMMANDS.iter().find(|c| c.0 == "ui.colorWheel").expect("registered").3;
    let mut app = app(json!("#3366cc"));
    let mut empty = VectorcraftApp::new(Session::new(), Default::default());
    for p in vectorcraft_testkit::strategies::junk_params(doc, "/nonexistent").into_iter().chain(vectorcraft_testkit::strategies::junk_values()) {
        let _ = app.run("ui.colorWheel", p.clone());
        let _ = empty.run("ui.colorWheel", p);
    }
}

/// The saved settings read leniently: a value this build does not know reads as the default and
/// the other preferences are kept.
#[test]
fn saved_settings_read_leniently() {
    let read = |v: Value| serde_json::from_value::<crate::UiState>(v).expect("the preferences read");
    let s = read(json!({"color_wheel": {"vision": true}, "color_panel": {"rgbPercent": true}}));
    assert!(s.color_wheel.vision && s.color_panel.rgb_percent);
    let s = read(json!({"color_wheel": {"vision": "yes", "later": 3}, "color_panel": {"rgbPercent": true}}));
    assert!(!s.color_wheel.vision && s.color_panel.rgb_percent);
    let s = read(json!({"color_wheel": "junk", "color_panel": {"rgbPercent": true}}));
    assert!(!s.color_wheel.vision && s.color_panel.rgb_percent);
}
