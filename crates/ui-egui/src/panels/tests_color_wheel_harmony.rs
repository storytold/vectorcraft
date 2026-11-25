//! The Color Wheel panel's Show Harmony drawn headless through the `Fixture`: the markers of a
//! rule's other colors, `ui.colorWheel`'s `harmony` and `harmonyRule`, and the saved settings.

use serde_json::json;
use vectorcraft_color::Color;
use vectorcraft_color::cms;
use vectorcraft_color::harmony::{self, Harmony};
use vectorcraft_engine::Session;

use super::color_wheel::{Settings, View, display_hsb, harmony_rules};
use super::tests_color_wheel::Panel;
use crate::VectorcraftApp;
use crate::i18n::LangExt as _;

/// An app with a selected rectangle filled with `fill`, its Color Wheel panel showing `rule`.
fn app(fill: serde_json::Value, rule: &str) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": fill})).unwrap();
    app.run("ui.colorWheel", json!({"harmony": true, "harmonyRule": rule})).unwrap();
    app
}

fn fill(app: &VectorcraftApp) -> Color {
    crate::panels::current_paints(app).0.color().expect("a solid fill")
}

fn hsb(c: &Color) -> [f32; 3] {
    display_hsb(&cms::active(), c)
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// With the complementary rule and a red fill, the marker is the complement the rule gives, and
/// picking it sets the fill to that color, as one undo step.
#[test]
fn a_complementary_marker_sits_opposite_and_a_click_applies_it() {
    let mut p = Panel::new(app(json!("#ff0000"), "complementary"));
    assert_eq!(p.value("Harmony color").as_deref(), Some("#00ffff"));
    let undo = undo_len(&p.app());
    p.click("Harmony color");
    let want = harmony::apply(Harmony::Complementary, Color::rgb(1.0, 0.0, 0.0))[1];
    assert_eq!(fill(&p.app()).to_hex(), want.to_hex());
    assert_eq!(undo_len(&p.app()), undo + 1);
}

/// Picking a High Contrast marker applies the rule's color, whose brightness differs from the
/// current one, and not the hue under the marker at the current brightness.
#[test]
fn a_press_held_on_a_marker_applies_the_rules_color() {
    let mut p = Panel::new(app(json!([0.9, 0.8, 0.2]), "highContrast"));
    let base = hsb(&fill(&p.app()));
    let undo = undo_len(&p.app());
    p.click("Harmony color");
    let want = Harmony::HighContrast.apply_hsb(base)[1];
    let got = hsb(&fill(&p.app()));
    assert!((got[2] - want[2]).abs() < 0.01 && got[2] < 0.5 && (got[0] - want[0]).abs() < 1.0, "{got:?} {want:?}");
    assert_eq!(undo_len(&p.app()), undo + 1);
}

/// No markers show with Show Harmony off, in the box view, or with no color to edit.
#[test]
fn markers_show_only_on_the_wheel_with_a_color() {
    let shown = |p: &Panel| p.fx.by_label("Harmony color").is_some();
    assert!(shown(&Panel::new(app(json!("#ff0000"), "triad"))));
    let mut a = app(json!("#ff0000"), "triad");
    a.run("ui.colorWheel", json!({"harmony": false})).unwrap();
    assert!(!shown(&Panel::new(a)));
    let mut a = app(json!("#ff0000"), "triad");
    a.run("ui.colorWheel", json!({"view": "box"})).unwrap();
    assert!(!shown(&Panel::new(a)));
    let mut a = app(json!("#ff0000"), "triad");
    a.run("paint.setFill", json!({"none": true})).unwrap();
    assert!(!shown(&Panel::new(a)));
}

/// A gray still shows its rule's markers, as the hue it was picked at turns them around the ring.
#[test]
fn a_gray_markers_turn_from_its_hue() {
    let mut p = Panel::new(app(json!([0.3, 0.6, 0.9]), "tetrad2"));
    for _ in 0..200 {
        if hsb(&fill(&p.app()))[1] == 0.0 {
            break;
        }
        p.key("Saturation and brightness", egui::Key::ArrowLeft, egui::Modifiers::NONE);
    }
    assert_eq!(hsb(&fill(&p.app()))[1], 0.0, "a gray");
    assert!(p.fx.by_label("Harmony color").is_some(), "the markers stay");
}

/// `ui.colorWheel` sets and answers `harmony` and `harmonyRule`, takes a rule's label as well as its
/// id, and answers an error that changes nothing for a value that is not a boolean or a rule
/// outside the wheel's list.
#[test]
fn ui_color_wheel_sets_the_harmony_and_its_rule() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let got = app.run("ui.colorWheel", json!({"harmony": true, "harmonyRule": "Split Complementary"})).unwrap();
    assert_eq!((got["harmony"].clone(), got["harmonyRule"].clone()), (json!(true), json!("splitComplementary")));
    let e = app.run("ui.colorWheel", json!({"harmony": "yes"})).unwrap_err();
    assert_eq!(e, "`harmony` must be true or false");
    let e = app.run("ui.colorWheel", json!({"view": "box", "harmonyRule": "monochromatic"})).unwrap_err();
    assert_eq!(
        e,
        "`harmonyRule` must be one of complementary, splitComplementary, leftComplement, rightComplement, analogous, analogous2, triad, \
         tetrad, tetrad2, tetrad3, compound, compound2, highContrast, highContrast3, pentagram"
    );
    assert_eq!(app.ui.color_wheel, Settings { harmony: true, harmony_rule: Harmony::SplitComplementary, ..Default::default() }, "nothing changed");
    // Turning Show Harmony off and on keeps the rule.
    app.run("ui.colorWheel", json!({"harmony": false})).unwrap();
    let got = app.run("ui.colorWheel", json!({"harmony": true})).unwrap();
    assert_eq!(got["harmonyRule"], json!("splitComplementary"));
}

/// The errors are translated in the catalogs that translate status and error messages, and each
/// translation keeps the key and the accepted values.
#[test]
fn ui_color_wheel_harmony_errors_are_translated() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let flag = app.run("ui.colorWheel", json!({"harmony": 1})).unwrap_err();
    let rule = app.run("ui.colorWheel", json!({"harmonyRule": "shades"})).unwrap_err();
    for code in [
        "cs", "de", "el", "es", "fr", "hi", "hu", "it", "ja", "ka", "ko", "pl", "pt-br", "ru", "sv", "uk", "vi", "zh-hans", "zh-hant", "fi", "tr",
        "ca", "nl", "bs", "da", "id",
    ] {
        let lang = crate::i18n::Lang::from_code(code).expect("registered");
        let shown = crate::i18n::message(lang, &flag);
        assert!(shown != flag && shown.contains("`harmony`"), "{code}: {shown}");
        let shown = crate::i18n::message(lang, &rule);
        assert!(shown != rule && shown.contains("`harmonyRule`") && shown.contains("highContrast3"), "{code}: {shown}");
    }
}

/// The wheel offers exactly the rules whose colors each have a hue of their own, so no two markers
/// share a place on the ring.
#[test]
fn the_wheel_offers_fifteen_rules_without_monochromatic() {
    let rules: Vec<Harmony> = harmony_rules().collect();
    assert!(!rules.is_empty());
    assert!(Harmony::ALL.into_iter().all(|h| rules.contains(&h) == h.hues_differ()));
}

/// The settings survive a JSON round trip of the preferences, and a saved value this build does
/// not know, or a rule the wheel leaves out, reads as the key's default.
#[test]
fn the_harmony_settings_are_kept_and_read_leniently() {
    let set = Settings { view: View::Wheel, harmony: true, harmony_rule: Harmony::Pentagram, ..Default::default() };
    let ui = crate::UiState { color_wheel: set.clone(), ..Default::default() };
    let mut saved = serde_json::to_value(&ui).unwrap();
    saved["color_panel"] = json!({"rgbPercent": true});
    assert_eq!(saved["color_wheel"]["harmonyRule"], json!("pentagram"));
    let back: crate::UiState = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(back.color_wheel, set);
    let mut odd = saved;
    odd["color_wheel"]["harmony"] = json!("yes");
    odd["color_wheel"]["harmonyRule"] = json!("monochromatic");
    let back: crate::UiState = serde_json::from_value(odd).unwrap();
    assert_eq!((back.color_wheel.harmony, back.color_wheel.harmony_rule), (false, Harmony::Complementary));
    assert!(back.color_panel.rgb_percent, "the other preferences are kept");
}
