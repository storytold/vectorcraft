//! The Color Wheel panel's Show Harmony drawn headless: the markers of a rule's other colors on the
//! ring, what a click, Alt, Enter and a screen reader do with them, `ui.colorWheel`'s `harmony` and
//! `harmonyRule`, the panel menu and the saved settings.

use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use vectorcraft_color::Color;
use vectorcraft_color::cms;
use vectorcraft_color::harmony::Harmony;
use vectorcraft_engine::Session;

use super::color_wheel::{Settings, View, display_hsb, harmony_rules, show};
use crate::VectorcraftApp;

/// An app with a selected rectangle filled with `fill`, its Color Wheel panel showing `rule`.
fn app(fill: serde_json::Value, rule: &str) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": fill})).unwrap();
    app.run("ui.colorWheel", json!({"harmony": true, "harmonyRule": rule})).unwrap();
    app
}

/// The panel in a 260-point-wide flyout, with the app's shortcuts after it as in a frame of the app.
fn harness(app: VectorcraftApp) -> Harness<'static, VectorcraftApp> {
    let mut h = Harness::builder().with_size(vec2(260.0, 320.0)).build_ui_state(
        |ui, app: &mut VectorcraftApp| {
            show(app, ui);
            crate::shortcuts::handle(app, ui.ctx());
        },
        app,
    );
    h.run_steps(2);
    h
}

/// The hue (degrees counterclockwise from 3 o'clock) of point `p` on the ring drawn in `ring`.
fn hue_at(ring: Rect, p: Pos2) -> f32 {
    let v = p - ring.center();
    (-v.y).atan2(v.x).to_degrees().rem_euclid(360.0)
}

/// The point at hue `deg` in the middle of the band of the ring drawn in `ring`.
fn on_ring(ring: Rect, deg: f32) -> Pos2 {
    let r = ring.width() / 2.0 - 4.0 - 7.0;
    let a = deg.to_radians();
    ring.center() + vec2(a.cos(), -a.sin()) * r
}

/// The centers of the harmony markers, in the rule's order.
fn markers(h: &Harness<'static, VectorcraftApp>) -> Vec<Pos2> {
    h.query_all_by_label("Harmony color").map(|n| n.rect().center()).collect()
}

fn ring(h: &Harness<'static, VectorcraftApp>) -> Rect {
    h.get_by_label("Hue ring").rect()
}

fn press(h: &mut Harness<'static, VectorcraftApp>, pos: Pos2, pressed: bool, modifiers: Modifiers) {
    h.event(Event::ModifiersChanged(modifiers));
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers });
    h.run_steps(1);
}

fn click(h: &mut Harness<'static, VectorcraftApp>, pos: Pos2, modifiers: Modifiers) {
    press(h, pos, true, modifiers);
    press(h, pos, false, modifiers);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(1);
}

/// A drag through `points`, pressed at the first and released at the last.
fn drag(h: &mut Harness<'static, VectorcraftApp>, points: &[Pos2]) {
    let (Some(first), Some(last)) = (points.first(), points.last()) else { return };
    press(h, *first, true, Modifiers::NONE);
    for p in points {
        for _ in 0..2 {
            h.event(Event::PointerMoved(*p));
            h.run_steps(1);
        }
    }
    press(h, *last, false, Modifiers::NONE);
    h.run_steps(1);
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

/// With the complementary rule and a red fill, a marker sits at 180° on the ring, and a click on it
/// sets the fill to the complement that the rule gives, as one undo step.
#[test]
fn a_complementary_marker_sits_opposite_and_a_click_applies_it() {
    let mut h = harness(app(json!("#ff0000"), "complementary"));
    let at = markers(&h);
    assert_eq!(at.len(), 1, "one other color");
    assert!((hue_at(ring(&h), at[0]) - 180.0).abs() < 1.0, "{}", hue_at(ring(&h), at[0]));
    assert_eq!(h.get_by_label("Harmony color").value().as_deref(), Some("#00ffff"));
    let undo = undo_len(h.state());
    click(&mut h, at[0], Modifiers::NONE);
    let want = Harmony::Complementary.apply(Color::rgb(1.0, 0.0, 0.0))[1];
    assert_eq!(fill(h.state()).to_hex(), want.to_hex());
    assert_eq!(undo_len(h.state()), undo + 1);
}

/// A click on a High Contrast marker applies the rule's color, whose brightness differs from the
/// current one, and not the hue under the pointer at the current brightness.
#[test]
fn a_click_on_a_marker_applies_the_rules_color_not_the_hue_under_it() {
    let mut h = harness(app(json!([0.9, 0.8, 0.2]), "highContrast"));
    let base = hsb(&fill(h.state()));
    let at = markers(&h);
    assert_eq!(at.len(), 1);
    click(&mut h, at[0], Modifiers::NONE);
    let want = Harmony::HighContrast.apply_hsb(base)[1];
    let got = hsb(&fill(h.state()));
    assert!((got[2] - want[2]).abs() < 0.01 && got[2] < 0.5, "{got:?} {want:?}");
    assert!((got[0] - want[0]).abs() < 1.0, "{got:?} {want:?}");
}

/// Dragging the current color around the ring moves the other markers by the same angle.
#[test]
fn the_markers_follow_the_current_color() {
    let mut h = harness(app(json!("#ff0000"), "triad"));
    let r = ring(&h);
    let before: Vec<f32> = markers(&h).iter().map(|p| hue_at(r, *p)).collect();
    assert_eq!(before.len(), 2);
    assert!((before[0] - 120.0).abs() < 1.0 && (before[1] - 240.0).abs() < 1.0, "{before:?}");
    drag(&mut h, &[on_ring(r, 0.0), on_ring(r, 30.0), on_ring(r, 60.0)]);
    assert!((hsb(&fill(h.state()))[0] - 60.0).abs() < 1.0, "{:?}", hsb(&fill(h.state())));
    let after: Vec<f32> = markers(&h).iter().map(|p| hue_at(r, *p)).collect();
    assert!((after[0] - 180.0).abs() < 1.0 && (after[1] - 300.0).abs() < 1.0, "{after:?}");
}

/// A drag that starts on a harmony marker moves the ring's hue as a drag on the band does, as one
/// undo step.
#[test]
fn a_drag_from_a_marker_moves_the_hue() {
    let mut h = harness(app(json!("#ff0000"), "triad"));
    let r = ring(&h);
    let at = markers(&h);
    assert!((hue_at(r, at[0]) - 120.0).abs() < 1.0);
    let undo = undo_len(h.state());
    drag(&mut h, &[at[0], on_ring(r, 135.0), on_ring(r, 150.0)]);
    assert!((hsb(&fill(h.state()))[0] - 150.0).abs() < 1.0, "{:?}", hsb(&fill(h.state())));
    assert_eq!(undo_len(h.state()), undo + 1);
}

/// A press held on a High Contrast marker past the length of a click, without moving, applies the
/// rule's color on release, and the hue under the pointer is not previewed while it is held.
#[test]
fn a_press_held_on_a_marker_applies_the_rules_color() {
    let mut h = harness(app(json!([0.9, 0.8, 0.2]), "highContrast"));
    let base = hsb(&fill(h.state()));
    let at = markers(&h);
    assert_eq!(at.len(), 1);
    let undo = undo_len(h.state());
    press(&mut h, at[0], true, Modifiers::NONE);
    h.run_steps(6);
    assert_eq!(hsb(&fill(h.state())), base, "nothing changes while the press is held");
    press(&mut h, at[0], false, Modifiers::NONE);
    h.run_steps(1);
    let want = Harmony::HighContrast.apply_hsb(base)[1];
    let got = hsb(&fill(h.state()));
    assert!((got[2] - want[2]).abs() < 0.01 && (got[0] - want[0]).abs() < 1.0, "{got:?} {want:?}");
    assert_eq!(undo_len(h.state()), undo + 1);
}

/// Alt-click on a marker paints the stroke, and the fill stays in front.
#[test]
fn alt_click_on_a_marker_paints_the_stroke() {
    let mut app = app(json!("#ff0000"), "complementary");
    app.run("paint.setStroke", json!({"color": "#808080", "focus": false})).unwrap();
    let mut h = harness(app);
    let at = markers(&h);
    click(&mut h, at[0], Modifiers::ALT);
    let (f, s) = crate::panels::current_paints(h.state());
    assert_eq!((f.color().unwrap().to_hex(), s.color().unwrap().to_hex()), ("#ff0000".into(), "#00ffff".into()));
    assert!(h.state().session.fill_active);
}

/// A click gives a marker the keyboard focus, and Enter or Space then applies its color; a screen
/// reader's click applies it too.
#[test]
fn the_markers_take_enter_space_and_a_screen_readers_click() {
    let mut h = harness(app(json!("#ff0000"), "splitComplementary"));
    let at = markers(&h);
    assert_eq!(at.len(), 2);
    click(&mut h, at[0], Modifiers::NONE);
    assert!((hsb(&fill(h.state()))[0] - 150.0).abs() < 1.0, "{:?}", hsb(&fill(h.state())));
    // The markers moved with the color: from 150°, the focused first one is at 300°.
    let undo = undo_len(h.state());
    h.key_press_modifiers(Modifiers::NONE, egui::Key::Enter);
    h.run_steps(1);
    assert!((hsb(&fill(h.state()))[0] - 300.0).abs() < 1.0, "{:?}", hsb(&fill(h.state())));
    h.key_press_modifiers(Modifiers::NONE, egui::Key::Space);
    h.run_steps(1);
    assert!((hsb(&fill(h.state()))[0] - 90.0).abs() < 1.0, "{:?}", hsb(&fill(h.state())));
    assert_eq!(undo_len(h.state()), undo + 2, "one undo step a key");
    let node = h.get_all_by_label("Harmony color").nth(1).unwrap();
    assert!(node.accesskit_node().data().supports_action(egui::accesskit::Action::Click));
    node.click_accesskit();
    h.run_steps(2);
    assert!((hsb(&fill(h.state()))[0] - 300.0).abs() < 1.0, "{:?}", hsb(&fill(h.state())));
}

/// No markers show with Show Harmony off, in the box view, or with no color to edit.
#[test]
fn markers_show_only_on_the_wheel_with_a_color() {
    let mut a = app(json!("#ff0000"), "triad");
    a.run("ui.colorWheel", json!({"harmony": false})).unwrap();
    assert!(markers(&harness(a)).is_empty());
    let mut a = app(json!("#ff0000"), "triad");
    a.run("ui.colorWheel", json!({"view": "box"})).unwrap();
    assert!(markers(&harness(a)).is_empty());
    let mut a = app(json!("#ff0000"), "triad");
    a.run("paint.setFill", json!({"none": true})).unwrap();
    assert!(markers(&harness(a)).is_empty());
}

/// A gray keeps the hue it was picked at, and its markers stay apart around the ring.
#[test]
fn a_gray_markers_turn_from_its_hue() {
    let mut h = harness(app(json!([0.3, 0.6, 0.9]), "tetrad2"));
    let r = ring(&h);
    let hue = hsb(&fill(h.state()))[0];
    let sq = h.get_by_label("Saturation and brightness").rect();
    // A press between the square and the ring picks the square's left edge: no saturation.
    click(&mut h, Pos2::new(sq.left() - 3.0, sq.center().y), Modifiers::NONE);
    assert_eq!(hsb(&fill(h.state()))[1], 0.0, "a gray");
    let hues: Vec<f32> = markers(&h).iter().map(|p| hue_at(r, *p)).collect();
    assert_eq!(hues.len(), 3);
    for (got, turn) in hues.iter().zip([90.0, 180.0, 270.0]) {
        assert!(((got - hue - turn).rem_euclid(360.0) + 0.5).rem_euclid(360.0) < 2.0, "{hue} {hues:?}");
    }
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

/// The wheel offers the fifteen rules whose colors each have a hue of their own.
#[test]
fn the_wheel_offers_fifteen_rules_without_monochromatic() {
    let rules: Vec<Harmony> = harmony_rules().collect();
    assert_eq!(rules.len(), 15);
    assert!(!rules.contains(&Harmony::Monochromatic) && !rules.contains(&Harmony::Triad2));
    assert!(rules.contains(&Harmony::Pentagram) && rules.contains(&Harmony::HighContrast3));
}

/// The settings survive a JSON round trip of the preferences, and a saved value this build does
/// not know, or a rule the wheel leaves out, reads as the key's default.
#[test]
fn the_harmony_settings_are_kept_and_read_leniently() {
    let set = Settings { view: View::Wheel, harmony: true, harmony_rule: Harmony::Pentagram, ..Default::default() };
    let ui = crate::UiState { color_wheel: set.clone(), color_rgb_percent: true, ..Default::default() };
    let saved = serde_json::to_value(&ui).unwrap();
    assert_eq!(saved["color_wheel"]["harmonyRule"], json!("pentagram"));
    let back: crate::UiState = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(back.color_wheel, set);
    let mut odd = saved;
    odd["color_wheel"]["harmony"] = json!("yes");
    odd["color_wheel"]["harmonyRule"] = json!("monochromatic");
    let back: crate::UiState = serde_json::from_value(odd).unwrap();
    assert_eq!((back.color_wheel.harmony, back.color_wheel.harmony_rule), (false, Harmony::Complementary));
    assert!(back.color_rgb_percent, "the other preferences are kept");
}

/// The panel menu's Show Harmony is checked while on and gray while the box shows, and Harmony Rule
/// lists the wheel's rules with the current one checked; its items set the rule.
#[test]
fn the_panel_menu_shows_the_harmony_and_picks_the_rule() {
    let a = app(json!("#ff0000"), "triad");
    let mut fonts = false;
    let mut h = Harness::builder().with_size(vec2(260.0, 600.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            // The fonts, which draw the check mark, take effect from the next frame.
            if !fonts {
                crate::theme::install_fonts(ui.ctx());
                fonts = true;
                return;
            }
            assert!(super::panel_menu_items(app, ui, super::color_wheel::ID));
        },
        a,
    );
    h.run_steps(3);
    assert!(!h.get_by_label("✓ Show Harmony").accesskit_node().data().is_disabled());
    h.get_by_label("   Harmony Rule").click();
    h.run_steps(2);
    assert!(h.query_by_label("✓ Triad").is_some());
    assert!(h.query_by_label("   Pentagram").is_some());
    assert!(h.query_by_label("   Monochromatic").is_none() && h.query_by_label("   Shades").is_none());
    h.get_by_label("   Pentagram").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.color_wheel.harmony_rule, Harmony::Pentagram);
    h.state_mut().run("ui.colorWheel", json!({"view": "box"})).unwrap();
    h.run_steps(2);
    assert!(h.get_by_label("✓ Show Harmony").accesskit_node().data().is_disabled(), "gray while the box shows");
    assert!(h.get_by_label("   Harmony Rule").accesskit_node().data().is_disabled(), "gray while the box shows");
}
