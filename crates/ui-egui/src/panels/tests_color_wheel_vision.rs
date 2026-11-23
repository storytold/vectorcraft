//! The Color Wheel panel's Show Color Vision Preview row drawn headless: the simulated colors, their
//! names and values, the painted chips, a gray fill, no solid color, clicks and keys on the chips,
//! the keyboard focus, the panel menu and `ui.colorWheel`.

use egui::{Color32, Key, Modifiers, Rect, Shape, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::cms::{self, ProofSetup, ProofTarget};
use vectorcraft_engine::Session;

use super::color_wheel::{bytes, show, vision_colors};
use crate::VectorcraftApp;

/// `#rrggbb` of display sRGB `rgb`, as the chips show it.
fn hex(rgb: [f32; 3]) -> String {
    super::color_wheel::hex(bytes(rgb))
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

fn harness(app: VectorcraftApp) -> Harness<'static, VectorcraftApp> {
    let mut h = Harness::builder().with_size(vec2(260.0, 360.0)).build_ui_state(|ui, app: &mut VectorcraftApp| show(app, ui), app);
    h.run_steps(2);
    h
}

/// The value a chip reports to screen readers.
fn value(h: &Harness<'static, VectorcraftApp>, name: &str) -> Option<String> {
    h.get_by_label(name).value()
}

fn fill(app: &VectorcraftApp) -> Color {
    super::current_paints(app).0.color().expect("a solid fill")
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// The fill painted inside `cell`, if a filled rectangle covers it.
fn painted(h: &Harness<'static, VectorcraftApp>, cell: Rect) -> Option<Color32> {
    h.output().shapes.iter().find_map(|c| match &c.shape {
        Shape::Rect(r)
            if r.fill != Color32::TRANSPARENT && r.rect.expand(0.5).contains_rect(cell.shrink(0.5)) && cell.expand(0.5).contains_rect(r.rect) =>
        {
            Some(r.fill)
        }
        _ => None,
    })
}

fn color32(hex: &str) -> Color32 {
    Color32::from_hex(&format!("#{hex}")).expect("an RRGGBB value")
}

/// The protanopia, deuteranopia and tritanopia chips show the fill as View › Proof Setup's
/// simulations show it: each reports the simulation's hex value and is painted in it.
#[test]
fn the_chips_show_the_proof_setup_simulations_of_the_fill() {
    let h = harness(app(json!("#3366cc")));
    let cms = cms::active();
    let srgb = cms.display_rgb(&fill(h.state()));
    for (name, target) in
        [("Protanopia", ProofTarget::Protanopia), ("Deuteranopia", ProofTarget::Deuteranopia), ("Tritanopia", ProofTarget::Tritanopia)]
    {
        let want = hex(cms.proof_srgb(srgb, &ProofSetup { target, ..Default::default() }));
        assert_eq!(value(&h, name).as_deref(), Some(want.as_str()), "{name}");
        assert_eq!(painted(&h, h.get_by_label(name).rect()), Some(color32(&want)), "{name} is painted {want}");
    }
}

/// The achromatopsia chip of a red fill is the gray with red's CIE L* (D50, about 54.29: 828282),
/// and the simulations of a pure red differ from it and from each other.
#[test]
fn the_achromatopsia_chip_is_the_gray_of_the_colors_lightness() {
    let cms = cms::active();
    let [p, d, t, a] = vision_colors(&cms, &Color::rgb(1.0, 0.0, 0.0));
    assert_eq!(hex(a), "828282");
    assert_ne!(hex(p), hex(d));
    assert_ne!(hex(t), hex(a));
    let h = harness(app(json!("#ff0000")));
    assert_eq!(value(&h, "Achromatopsia").as_deref(), Some("828282"));
    assert_eq!(painted(&h, h.get_by_label("Achromatopsia").rect()), Some(color32("828282")));
}

/// A gray fill shows as the same gray in every chip.
#[test]
fn a_gray_fill_is_the_same_gray_in_every_chip() {
    let h = harness(app(json!("#808080")));
    let gray = hex(cms::active().display_rgb(&fill(h.state())));
    for name in NAMES {
        assert_eq!(value(&h, name).as_deref(), Some(gray.as_str()), "{name} shows {gray}");
    }
}

/// With the box in the wheel's place the row shows under the box, with the same values.
#[test]
fn the_row_shows_under_the_box_too() {
    let mut a = app(json!("#3366cc"));
    a.run("ui.colorWheel", json!({"view": "box", "channel": "red"})).unwrap();
    let h = harness(a);
    let [p, ..] = vision_colors(&cms::active(), &fill(h.state()));
    assert_eq!(value(&h, "Protanopia").as_deref(), Some(hex(p).as_str()));
    let field = h.get_by_label("Color box").rect();
    for name in NAMES {
        assert!(h.get_by_label(name).rect().top() >= field.bottom(), "{name} is under the box");
    }
}

/// With a None fill the chips carry only their names, with no value and no color.
#[test]
fn with_a_none_fill_the_chips_have_no_color() {
    let mut a = app(json!("#ff0000"));
    a.run("paint.setFill", json!({"none": true})).unwrap();
    let h = harness(a);
    for name in NAMES {
        assert_eq!(value(&h, name), None, "{name} has no value");
        assert_eq!(painted(&h, h.get_by_label(name).rect()), None, "{name} is not painted");
    }
}

/// Screen readers find each chip by its deficiency name, with its hex value as its value, and Tab
/// steps through the four chips in order.
#[test]
fn the_chips_take_the_focus_and_report_their_names_and_values() {
    let mut h = harness(app(json!("#3366cc")));
    for name in NAMES {
        let node = h.get_by_label(name);
        assert_eq!(node.accesskit_node().role(), egui::accesskit::Role::Image, "{name}");
        assert!(
            node.value().is_some_and(|v| v.len() == 6 && v.bytes().all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())),
            "{name}: {:?}",
            node.value()
        );
        assert!(!node.accesskit_node().is_disabled(), "{name}");
    }
    tab_to(&mut h, NAMES[0]);
    for name in &NAMES[1..] {
        h.key_press(Key::Tab);
        h.run_steps(1);
        assert!(h.get_by_label(name).accesskit_node().is_focused(), "the next Tab reaches {name}");
    }
}

/// Press Tab until the chip `name` has the keyboard focus. Tab stops on the panel's other controls
/// first.
fn tab_to(h: &mut Harness<'static, VectorcraftApp>, name: &str) {
    for _ in 0..40 {
        if h.get_by_label(name).accesskit_node().is_focused() {
            break;
        }
        h.key_press(Key::Tab);
        h.run_steps(1);
    }
    assert!(h.get_by_label(name).accesskit_node().is_focused(), "Tab reaches {name}");
}

/// A click on a chip, and Enter or Space on a focused chip, leave the fill and the undo history as
/// they were.
#[test]
fn a_click_or_a_key_on_a_chip_applies_nothing() {
    let mut h = harness(app(json!("#3366cc")));
    let (before, undo) = (fill(h.state()), undo_len(h.state()));
    for name in NAMES {
        h.get_by_label(name).click();
        h.run_steps(2);
        tab_to(&mut h, name);
        for key in [Key::Enter, Key::Space] {
            h.key_press_modifiers(Modifiers::NONE, key);
            h.run_steps(1);
        }
        assert_eq!(fill(h.state()), before, "{name}");
        assert_eq!(undo_len(h.state()), undo, "{name}: no undo step");
    }
}

/// Without the setting the row is not drawn, and the panel menu's Show Color Vision Preview turns
/// it on and off.
#[test]
fn the_panel_menu_turns_the_row_on_and_off() {
    let mut a = app(json!("#3366cc"));
    a.run("ui.colorWheel", json!({"vision": false})).unwrap();
    let h = harness(a);
    assert!(h.query_by_label_contains("Protanopia").is_none());
    let mut a = app(json!("#3366cc"));
    a.run("ui.colorWheel", json!({"vision": false})).unwrap();
    let mut fonts = false;
    let mut menu = Harness::builder().build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            // The fonts, which draw the check mark, take effect from the next frame.
            if !fonts {
                crate::theme::install_fonts(ui.ctx());
                fonts = true;
                return;
            }
            super::color_wheel::menu(app, ui);
        },
        a,
    );
    menu.run_steps(3);
    menu.get_by_label_contains("Show Color Vision Preview").click();
    menu.run_steps(2);
    assert!(menu.state().ui.color_wheel.vision);
    menu.get_by_label_contains("Show Color Vision Preview").click();
    menu.run_steps(2);
    assert!(!menu.state().ui.color_wheel.vision);
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
    assert_eq!(crate::control::inspect(&app, &egui::Context::default())["ui"]["color_wheel"]["vision"], true);
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
    let s = read(json!({"color_wheel": {"vision": true}, "color_rgb_percent": true}));
    assert!(s.color_wheel.vision && s.color_rgb_percent);
    let s = read(json!({"color_wheel": {"vision": "yes", "later": 3}, "color_rgb_percent": true}));
    assert!(!s.color_wheel.vision && s.color_rgb_percent);
    let s = read(json!({"color_wheel": "junk", "color_rgb_percent": true}));
    assert!(!s.color_wheel.vision && s.color_rgb_percent);
}
