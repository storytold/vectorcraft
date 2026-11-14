//! The Color Wheel panel's box view drawn headless: `ui.colorWheel`, the field and the strip of
//! each channel, the panel menu, the saved settings, the arrow keys and what screen readers read,
//! and the Color Picker's channel slider, which is the same widget as the strip.

use egui::accesskit::{Action, ActionData, ActionRequest};
use egui::{Event, Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use vectorcraft_color::Color;
use vectorcraft_color::cms;
use vectorcraft_engine::Session;

use super::color_wheel::{Settings, View, box_pick, box_pos, display_hsb, show};
use crate::VectorcraftApp;
use crate::dialogs::color_picker::Channel;

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

fn rect_of(h: &Harness<'static, VectorcraftApp>, label: &str) -> Rect {
    h.get_by_label(label).rect()
}

/// The point (x across, y up; 0..1 each) of `r`.
fn at(r: Rect, x: f32, y: f32) -> Pos2 {
    Pos2::new(r.left() + x * r.width(), r.bottom() - y * r.height())
}

fn press(h: &mut Harness<'static, VectorcraftApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

fn click(h: &mut Harness<'static, VectorcraftApp>, pos: Pos2) {
    press(h, pos, true);
    press(h, pos, false);
    h.run_steps(1);
}

/// A drag through `points`, pressed at the first and released at the last.
fn drag(h: &mut Harness<'static, VectorcraftApp>, points: &[Pos2]) {
    let (Some(first), Some(last)) = (points.first(), points.last()) else { return };
    press(h, *first, true);
    for p in points {
        for _ in 0..2 {
            h.event(Event::PointerMoved(*p));
            h.run_steps(1);
        }
    }
    press(h, *last, false);
    h.run_steps(1);
}

fn key(h: &mut Harness<'static, VectorcraftApp>, key: egui::Key) {
    h.key_press_modifiers(Modifiers::NONE, key);
    h.run_steps(1);
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

fn near(a: [f32; 3], b: [f32; 3], tol: f32) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= tol)
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// With Hue as the channel and a red fill, the field's top-left corner is white, its top-right
/// corner red and its bottom edge black.
#[test]
fn the_hue_box_picks_saturation_across_and_brightness_up() {
    let mut h = harness(app(json!("#ff0000"), "hue"));
    let field = rect_of(&h, "Color box");
    click(&mut h, field.left_top() + vec2(0.2, 0.2));
    assert!(near(rgb(&fill(h.state())), [1.0, 1.0, 1.0], 0.01), "{:?}", rgb(&fill(h.state())));
    click(&mut h, field.right_top() + vec2(-0.2, 0.2));
    assert!(near(rgb(&fill(h.state())), [1.0, 0.0, 0.0], 0.01), "the hue is kept through white: {:?}", rgb(&fill(h.state())));
    click(&mut h, at(field, 0.5, 0.0) + vec2(0.0, -0.2));
    assert!(near(rgb(&fill(h.state())), [0.0, 0.0, 0.0], 0.01), "{:?}", rgb(&fill(h.state())));
}

/// Red puts blue across and green up, as the Color Picker's field does, and red stays as it was.
#[test]
fn the_red_box_puts_blue_across_and_green_up() {
    let mut h = harness(app(json!([0.2, 0.4, 0.6]), "red"));
    let field = rect_of(&h, "Color box");
    let red = rgb(&fill(h.state()))[0];
    click(&mut h, at(field, 0.75, 0.25));
    assert!(near(rgb(&fill(h.state())), [red, 0.25, 0.75], 0.01), "{:?}", rgb(&fill(h.state())));
    let read = h.get_by_label("Color box").value().unwrap_or_default();
    assert!(read.starts_with("Blue ") && read.contains(", Green "), "{read}");
}

/// A drag in the field is one undo step, and a drag on the strip sets the third component.
#[test]
fn a_drag_in_the_field_is_one_undo_step_and_the_strip_sets_the_channel() {
    let mut h = harness(app(json!([0.6, 0.3, 0.3]), "hue"));
    let field = rect_of(&h, "Color box");
    let undo = undo_len(h.state());
    drag(&mut h, &[at(field, 0.2, 0.2), at(field, 0.5, 0.5), at(field, 0.8, 0.9)]);
    assert_eq!(undo_len(h.state()), undo + 1, "one undo step");
    assert!(near(hsb(&fill(h.state())), [0.0, 0.8, 0.9], 0.01), "{:?}", hsb(&fill(h.state())));
    let strip = rect_of(&h, "Hue");
    drag(&mut h, &[strip.center_bottom() + vec2(0.0, -1.0), strip.center()]);
    let after = hsb(&fill(h.state()));
    assert!((after[0] - 180.0).abs() < 2.0 && (after[1] - 0.8).abs() < 0.01 && (after[2] - 0.9).abs() < 0.01, "{after:?}");
    assert_eq!(undo_len(h.state()), undo + 2);
}

/// After a click on the strip, ↑ raises the channel by one unit, and the strip declares its range
/// and its adjust actions to screen readers.
#[test]
fn the_strip_takes_the_arrow_keys_and_screen_readers_actions() {
    let mut h = harness(app(json!([0.6, 0.3, 0.3]), "brightness"));
    let strip = rect_of(&h, "Brightness");
    click(&mut h, at(strip, 0.5, 0.5));
    let b = (hsb(&fill(h.state()))[2] * 100.0).round();
    key(&mut h, egui::Key::ArrowUp);
    assert!((hsb(&fill(h.state()))[2] * 100.0 - (b + 1.0)).abs() < 0.01, "{:?}", hsb(&fill(h.state())));
    let node = h.get_by_label("Brightness").accesskit_node();
    assert!([Action::Increment, Action::Decrement, Action::SetValue].iter().all(|a| node.data().supports_action(*a)));
    assert_eq!((node.data().min_numeric_value(), node.data().max_numeric_value()), (Some(0.0), Some(100.0)));
    let (target_node, target_tree) = node.locate();
    let data = Some(ActionData::NumericValue(25.0));
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::SetValue, data }));
    h.run_steps(1);
    assert!((hsb(&fill(h.state()))[2] - 0.25).abs() < 0.01, "{:?}", hsb(&fill(h.state())));
}

/// `ui.colorWheel` sets the keys it is given and keeps the others, `{}` reads them, a value off the
/// list answers an error that names the accepted values and changes nothing, and `ui.inspect`
/// reports the settings.
#[test]
fn ui_color_wheel_sets_and_reads_the_view_and_the_channel() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    assert_eq!(app.run("ui.colorWheel", json!({})).unwrap(), json!({"view": "wheel", "channel": "hue"}));
    assert_eq!(app.run("ui.colorWheel", json!({"view": "box"})).unwrap(), json!({"view": "box", "channel": "hue"}));
    assert_eq!(app.run("ui.colorWheel", json!({"channel": "green"})).unwrap(), json!({"view": "box", "channel": "green"}));
    let e = app.run("ui.colorWheel", json!({"view": "triangle"})).unwrap_err();
    assert_eq!(e, "`view` must be wheel or box");
    let e = app.run("ui.colorWheel", json!({"view": "wheel", "channel": 7})).unwrap_err();
    assert_eq!(e, "`channel` must be hue, saturation, brightness, red, green or blue");
    assert_eq!(app.ui.color_wheel, Settings { view: View::Box, channel: Channel::Green }, "an error changes nothing");
    let ui = crate::control::inspect(&app, &egui::Context::default());
    assert_eq!(ui["ui"]["color_wheel"], json!({"view": "box", "channel": "green"}));
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
    assert_eq!(e, "unknown parameter `veiw`; the command takes: view, channel");
    let e = app.run("ui.colorWheel", json!({"view": "wheel", "chanel": "blue"})).unwrap_err();
    assert_eq!(e, "unknown parameter `chanel`; the command takes: view, channel");
    assert_eq!(app.ui.color_wheel, Settings { view: View::Box, channel: Channel::Red }, "an error changes nothing");
}

/// With None there is no color to edit: the strip reads 0 to screen readers, as the hue ring does,
/// and its actions still set the fill from the last solid color the box shows.
#[test]
fn with_a_none_fill_the_strip_reads_zero_and_its_actions_set_the_fill() {
    let mut app = app(json!("#ff0000"), "brightness");
    app.run("paint.setFill", json!({"none": true})).unwrap();
    let mut h = harness(app);
    assert_eq!(h.get_by_label("Color box").value(), None, "no marker in the field");
    let node = h.get_by_label("Brightness").accesskit_node();
    assert_eq!(node.data().numeric_value(), Some(0.0), "no value on the strip");
    let last = hsb(&h.state().session.last_solid);
    let (target_node, target_tree) = node.locate();
    let data = Some(ActionData::NumericValue(25.0));
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::SetValue, data }));
    h.run_steps(1);
    assert!(near(hsb(&fill(h.state())), [last[0], last[1], 0.25], 0.01), "{last:?} → {:?}", hsb(&fill(h.state())));
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
    let ui = crate::UiState { color_wheel: Settings { view: View::Box, channel: Channel::Blue }, color_rgb_percent: true, ..Default::default() };
    let saved = serde_json::to_value(&ui).unwrap();
    let back: crate::UiState = serde_json::from_value(saved.clone()).unwrap();
    assert_eq!(back.color_wheel, ui.color_wheel);
    let mut odd = saved.clone();
    odd["color_wheel"] = json!({"view": "triangle", "channel": 7, "later": true});
    let back: crate::UiState = serde_json::from_value(odd).unwrap();
    assert_eq!(back.color_wheel, Settings::default());
    assert!(back.color_rgb_percent, "the other preferences are kept");
    // Each key reads on its own: an unknown channel does not reset a known view, or the reverse.
    let mut odd = saved.clone();
    odd["color_wheel"] = json!({"view": "box", "channel": 7});
    let back: crate::UiState = serde_json::from_value(odd).unwrap();
    assert_eq!(back.color_wheel, Settings { view: View::Box, channel: Channel::default() });
    let mut odd = saved.clone();
    odd["color_wheel"] = json!({"view": "triangle", "channel": "blue"});
    let back: crate::UiState = serde_json::from_value(odd).unwrap();
    assert_eq!(back.color_wheel, Settings { view: View::Wheel, channel: Channel::Blue });
    let mut odd = saved;
    odd["color_wheel"] = json!("box");
    let back: crate::UiState = serde_json::from_value(odd).unwrap();
    assert_eq!(back.color_wheel, Settings::default());
    assert!(back.color_rgb_percent);
}

/// The panel menu lists Wheel View and Box View with the shown one checked, and Box Channel; its
/// items set the view and the channel.
#[test]
fn the_panel_menu_switches_the_view_and_the_channel() {
    let mut app = app(json!("#ff0000"), "hue");
    app.run("ui.colorWheel", json!({"view": "wheel"})).unwrap();
    let mut fonts = false;
    let mut h = Harness::builder().with_size(vec2(260.0, 320.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            // The fonts, which draw the check mark, take effect from the next frame.
            if !fonts {
                crate::theme::install_fonts(ui.ctx());
                fonts = true;
                return;
            }
            assert!(super::panel_menu_items(app, ui, super::color_wheel::ID));
        },
        app,
    );
    h.run_steps(3);
    assert!(h.query_by_label("✓ Wheel View").is_some());
    h.get_by_label("   Box View").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.color_wheel.view, View::Box);
    assert!(h.query_by_label("✓ Box View").is_some());
}

/// Every channel's box puts each color back where it reads it, and the strip's value is the
/// channel's own component.
#[test]
fn box_pick_inverts_box_pos_for_every_channel() {
    for ch in Channel::ALL {
        for hsb in [[30.0, 0.6, 0.8], [200.0, 0.25, 0.5], [300.0, 1.0, 1.0]] {
            let (z, x, y) = box_pos(ch, hsb);
            let back = box_pick(ch, z, x, y, [0.0, 0.0, 0.0]);
            assert!(near(back, hsb, 1e-3), "{ch:?}: {hsb:?} → {back:?}");
        }
    }
    // A gray picked in an RGB channel keeps the hue it had.
    assert_eq!(box_pick(Channel::Red, 0.5, 0.5, 0.5, [120.0, 0.3, 0.3])[0], 120.0);
}

/// The Color Picker's channel slider is the same widget as the strip: after a click on it, ↑ raises
/// its channel by one unit.
#[test]
fn the_color_pickers_channel_slider_takes_the_arrow_keys() {
    let mut app = app(json!([0.6, 0.3, 0.3]), "hue");
    app.run("ui.colorPicker", json!({})).unwrap();
    let mut fonts = false;
    let mut h = Harness::builder().with_size(vec2(1000.0, 800.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            // The fonts take effect from the next frame.
            if !fonts {
                crate::theme::install_fonts(ui.ctx());
                fonts = true;
                return;
            }
            crate::dialogs::show(app, ui.ctx());
        },
        app,
    );
    h.run_steps(3);
    // The H field beside the values has the same name; the slider is the tall one.
    let rail = h.get_all_by_label("H").map(|n| n.rect()).find(|r| r.height() > 100.0).expect("the channel slider");
    click(&mut h, rail.center());
    let picked = |h: &Harness<'static, VectorcraftApp>| -> [f32; 3] {
        serde_json::from_value(h.state().ui.dialog.as_ref().unwrap().fields["__hsb"].clone()).unwrap()
    };
    let before = picked(&h);
    key(&mut h, egui::Key::ArrowUp);
    let after = picked(&h);
    assert!((after[0] - (before[0].round() + 1.0)).abs() < 1e-3, "{before:?} → {after:?}");
    assert!(near([0.0, after[1], after[2]], [0.0, before[1], before[2]], 1e-4), "{before:?} → {after:?}");
    let slider = h.get_all_by_label("H").find(|n| n.rect().height() > 100.0).expect("the channel slider");
    let node = slider.accesskit_node();
    assert!([Action::Increment, Action::Decrement, Action::SetValue].iter().all(|a| node.data().supports_action(*a)));
}
