//! The Color Wheel panel's Show Opacity row drawn headless: the setting and `ui.colorWheel`, the
//! slider on a fill, a gradient stop and a freeform point, the keys, screen readers, the percent
//! field and the gray row.

use egui::accesskit::{Action, ActionData, ActionRequest, Role};
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;
use vectorcraft_color::Paint;
use vectorcraft_engine::Session;

use super::color_wheel::{Settings, show};
use crate::VectorcraftApp;

/// An app with a selected red rectangle and Show Opacity checked.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    app.run("ui.colorWheel", json!({"opacity": true})).unwrap();
    app
}

fn harness(app: VectorcraftApp) -> Harness<'static, VectorcraftApp> {
    let mut h = Harness::builder().with_size(vec2(260.0, 360.0)).build_ui_state(
        |ui, app: &mut VectorcraftApp| {
            show(app, ui);
            crate::shortcuts::handle(app, ui.ctx());
        },
        app,
    );
    h.run_steps(2);
    h
}

fn click(h: &mut Harness<'static, VectorcraftApp>, pos: Pos2) {
    for pressed in [true, false] {
        h.event(Event::PointerMoved(pos));
        h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(1);
}

fn slider_rect(h: &Harness<'static, VectorcraftApp>) -> egui::Rect {
    h.get_by_role_and_label(Role::Slider, "Opacity").rect()
}

/// The middle of the slider's bar (the rail draws its bar 4 points in from each side).
fn middle(h: &Harness<'static, VectorcraftApp>) -> Pos2 {
    let r = slider_rect(h);
    Pos2::new(r.center().x, r.top() + 7.0)
}

fn fill_opacity(app: &VectorcraftApp) -> f32 {
    app.session.proxy_opacity(false).expect("a fill item")
}

fn undo_labels(app: &VectorcraftApp) -> Vec<String> {
    app.session.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect()
}

fn action(h: &mut Harness<'static, VectorcraftApp>, action: Action, data: Option<ActionData>) {
    let (target_node, target_tree) = h.get_by_role_and_label(Role::Slider, "Opacity").accesskit_node().locate();
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action, data }));
    h.run_steps(2);
}

/// The panel menu's Show Opacity and `ui.colorWheel {opacity}` check and uncheck the row; a bad
/// value is an error and changes nothing.
#[test]
fn show_opacity_is_a_setting_of_the_panel() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    assert_eq!(app.run("ui.colorWheel", json!({})).unwrap()["opacity"], json!(false));
    assert_eq!(app.run("ui.colorWheel", json!({"opacity": true})).unwrap()["opacity"], json!(true));
    let e = app.run("ui.colorWheel", json!({"opacity": "no"})).unwrap_err();
    assert_eq!(e, "`opacity` must be true or false");
    assert!(app.ui.color_wheel.opacity);
    assert_eq!(crate::control::inspect(&app, &egui::Context::default())["ui"]["color_wheel"]["opacity"], json!(true));
    let set = app.run("ui.colorWheel", json!({"view": "box"})).unwrap();
    assert_eq!(set["opacity"], json!(true), "another key keeps Show Opacity");
    app.run("ui.colorWheel", json!({"opacity": false})).unwrap();
    let h = harness({
        let mut app = self::app();
        app.run("ui.colorWheel", json!({"opacity": false})).unwrap();
        app
    });
    assert!(h.query_by_role_and_label(Role::Slider, "Opacity").is_none(), "no row while unchecked");
}

/// The row also shows under the box, and a click at its middle sets the fill item's opacity there.
#[test]
fn the_row_shows_under_the_box_view() {
    let mut app = app();
    app.run("ui.colorWheel", json!({"view": "box"})).unwrap();
    let mut h = harness(app);
    assert!(h.query_by_label("Color box").is_some(), "the box shows");
    let at = middle(&h);
    click(&mut h, at);
    assert!((fill_opacity(h.state()) - 0.5).abs() < 0.03, "{}", fill_opacity(h.state()));
}

/// A value this build does not read keeps the rest of the preferences and reads as the default.
#[test]
fn saved_settings_read_leniently() {
    let mut saved = serde_json::to_value(crate::UiState::default()).unwrap();
    saved["color_rgb_percent"] = json!(true);
    for bad in [json!("box"), json!({"opacity": "yes"}), json!(7)] {
        saved["color_wheel"] = bad.clone();
        let ui: crate::UiState = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(ui.color_wheel, Settings::default(), "{bad}");
        assert!(ui.color_rgb_percent, "{bad}");
    }
    saved["color_wheel"] = json!({"opacity": true, "later": 1});
    let ui: crate::UiState = serde_json::from_value(saved).unwrap();
    assert!(ui.color_wheel.opacity);
}

/// A click at the middle of the track sets the fill item's opacity to about 50 % as one undo step
/// named Opacity, the fill keeps its color, and → then adds 1 %.
#[test]
fn the_slider_sets_the_fill_opacity_and_takes_the_arrow_keys() {
    let mut h = harness(app());
    let undo = undo_labels(h.state()).len();
    let at = middle(&h);
    click(&mut h, at);
    let o = fill_opacity(h.state());
    assert!((o - 0.5).abs() < 0.02, "{o}");
    let labels = undo_labels(h.state());
    assert_eq!((labels.len(), labels.last().map(String::as_str)), (undo + 1, Some("Opacity")));
    assert_eq!(crate::panels::current_paints(h.state()).0.color().unwrap().to_hex(), "#ff0000");
    let before = (o * 100.0).round();
    h.key_press_modifiers(Modifiers::NONE, egui::Key::ArrowRight);
    h.run_steps(2);
    assert_eq!((fill_opacity(h.state()) * 100.0).round(), before + 1.0);
}

/// Screen readers get a slider named Opacity with its percentage and its range, and can step it and
/// set it.
#[test]
fn screen_readers_read_and_set_the_opacity() {
    let mut h = harness(app());
    let node = h.get_by_role_and_label(Role::Slider, "Opacity");
    let data = node.accesskit_node();
    assert!([Action::Increment, Action::Decrement, Action::SetValue].iter().all(|a| data.data().supports_action(*a)));
    assert_eq!(data.numeric_value(), Some(100.0));
    assert_eq!((data.min_numeric_value(), data.max_numeric_value()), (Some(0.0), Some(100.0)));
    action(&mut h, Action::SetValue, Some(ActionData::NumericValue(25.0)));
    assert!((fill_opacity(h.state()) - 0.25).abs() < 1e-6);
    action(&mut h, Action::Decrement, None);
    assert!((fill_opacity(h.state()) - 0.24).abs() < 1e-6);
}

/// The stroke proxy in front: the row sets the stroke item's opacity and leaves the fill's.
#[test]
fn with_the_stroke_in_front_the_row_sets_the_stroke() {
    let mut app = app();
    app.run("paint.toggleActive", json!({})).unwrap();
    let mut h = harness(app);
    action(&mut h, Action::SetValue, Some(ActionData::NumericValue(40.0)));
    assert!((h.state().session.proxy_opacity(true).unwrap() - 0.4).abs() < 1e-6);
    assert_eq!(fill_opacity(h.state()), 1.0);
}

/// A selected gradient stop takes the opacity, and the other stop keeps its own.
#[test]
fn a_selected_gradient_stop_takes_the_opacity() {
    let mut app = app();
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [10, 35], "end": [60, 35]}})).unwrap();
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    let mut h = harness(app);
    action(&mut h, Action::SetValue, Some(ActionData::NumericValue(30.0)));
    let Paint::Gradient(g) = crate::panels::current_paints(h.state()).0 else { panic!("still a gradient") };
    let o: Vec<f32> = g.gradient.stops.iter().map(|s| s.opacity).collect();
    assert!((o[0] - 1.0).abs() < 1e-6 && (o[1] - 0.3).abs() < 1e-6, "{o:?}");
    assert_eq!(fill_opacity(h.state()), 1.0, "the fill item keeps its opacity");
}

/// With no stop selected, the first stop takes the opacity, as it takes a color pick.
#[test]
fn without_a_selected_stop_the_first_takes_the_opacity() {
    let mut app = app();
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [10, 35], "end": [60, 35]}})).unwrap();
    assert_eq!(app.session.selected_stop(), None);
    let mut h = harness(app);
    action(&mut h, Action::SetValue, Some(ActionData::NumericValue(30.0)));
    let Paint::Gradient(g) = crate::panels::current_paints(h.state()).0 else { panic!("still a gradient") };
    let o: Vec<f32> = g.gradient.stops.iter().map(|s| s.opacity).collect();
    assert!((o[0] - 0.3).abs() < 1e-6 && (o[1] - 1.0).abs() < 1e-6, "{o:?}");
}

/// The hue picked last for a white gradient stop stays on the ring after the row sets the stop's
/// opacity.
#[test]
fn an_opacity_edit_keeps_the_hue_picked_for_a_white_stop() {
    let mut app = app();
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [10, 35], "end": [60, 35]}})).unwrap();
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 0})).unwrap();
    let mut h = harness(app);
    let ring = |h: &Harness<'static, VectorcraftApp>| h.get_by_label("Hue ring").accesskit_node().numeric_value();
    let (target_node, target_tree) = h.get_by_label("Hue ring").accesskit_node().locate();
    let data = Some(ActionData::NumericValue(120.0));
    h.event(Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action: Action::SetValue, data }));
    h.run_steps(2);
    assert_eq!(ring(&h), Some(120.0), "the ring shows the hue picked for white");
    action(&mut h, Action::SetValue, Some(ActionData::NumericValue(30.0)));
    let Paint::Gradient(g) = crate::panels::current_paints(h.state()).0 else { panic!("still a gradient") };
    assert!((g.gradient.stops[0].opacity - 0.3).abs() < 1e-6, "{:?}", g.gradient.stops);
    assert_eq!(ring(&h), Some(120.0), "the ring keeps the hue after the opacity edit");
}

/// A drag across the slider previews and ends as one undo step named Opacity.
#[test]
fn a_drag_is_one_undo_step() {
    let mut h = harness(app());
    let undo = undo_labels(h.state()).len();
    let r = slider_rect(&h);
    let y = r.top() + 7.0;
    let (from, to) = (Pos2::new(r.left() + 10.0, y), Pos2::new(r.right() - 20.0, y));
    h.event(Event::PointerMoved(from));
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for k in 1..=6 {
        h.event(Event::PointerMoved(from + (to - from) * (k as f32 / 6.0)));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    let o = fill_opacity(h.state());
    assert!(o > 0.7 && o < 1.0, "{o}");
    let labels = undo_labels(h.state());
    assert_eq!((labels.len(), labels.last().map(String::as_str)), (undo + 1, Some("Opacity")));
}

/// A selected object without a fill item beside one with a fill item: the row shows and sets the
/// second one's opacity.
#[test]
fn an_object_without_a_fill_item_leaves_the_row_to_the_next() {
    let mut app = app();
    let first = app.session.doc().unwrap().selection.objects[0];
    app.run("appearance.removeItem", json!({"index": 0})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 50, "height": 50})).unwrap();
    let second = app.session.doc().unwrap().selection.objects[0];
    app.run("select.set", json!({"ids": [first.0, second.0]})).unwrap();
    let mut h = harness(app);
    assert!(!h.get_by_role_and_label(Role::Slider, "Opacity").accesskit_node().is_disabled());
    action(&mut h, Action::SetValue, Some(ActionData::NumericValue(45.0)));
    let n = h.state().session.doc().unwrap().doc.node(second).unwrap().clone();
    assert!((n.appearance.fill().unwrap().opacity - 0.45).abs() < 1e-6);
}

/// Type painted with a gradient: the row sets the gradient's stop, which belongs to the gradient.
#[test]
fn a_gradient_stop_on_type_takes_the_opacity() {
    let mut app = app();
    app.run("text.create", json!({"x": 10, "y": 150, "text": "Hi"})).unwrap();
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops}})).unwrap();
    let mut h = harness(app);
    assert!(!h.get_by_role_and_label(Role::Slider, "Opacity").accesskit_node().is_disabled());
    action(&mut h, Action::SetValue, Some(ActionData::NumericValue(30.0)));
    let Paint::Gradient(g) = crate::panels::current_paints(h.state()).0 else { panic!("still a gradient") };
    assert!((g.gradient.stops[0].opacity - 0.3).abs() < 1e-6, "{:?}", g.gradient.stops);
}

/// The selected freeform point takes the opacity.
#[test]
fn the_selected_freeform_point_takes_the_opacity() {
    let mut app = app();
    app.run("paint.setFill", json!({"gradient": {"kind": "freeform"}})).unwrap();
    let mut h = harness(app);
    action(&mut h, Action::SetValue, Some(ActionData::NumericValue(60.0)));
    let Paint::Gradient(g) = crate::panels::current_paints(h.state()).0 else { panic!("still a gradient") };
    let f = super::gradient::shown_points(&g);
    assert!((f.points[0].opacity - 0.6).abs() < 1e-6, "{:?}", f.points.iter().map(|p| p.opacity).collect::<Vec<_>>());
}

/// The percent field takes a typed value.
#[test]
fn the_percent_field_takes_a_typed_value() {
    let mut h = harness(app());
    h.get_by_role_and_label(Role::TextInput, "Opacity").click();
    h.run_steps(1);
    h.get_by_role_and_label(Role::TextInput, "Opacity").type_text("35");
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!((fill_opacity(h.state()) - 0.35).abs() < 1e-6, "{}", fill_opacity(h.state()));
}

/// With nothing selected, and with only type selected (its characters take the paint), the row is
/// gray and screen readers cannot set it.
#[test]
fn the_row_is_gray_with_nothing_selected_and_on_type() {
    let mut app = app();
    app.run("select.none", json!({})).unwrap();
    let mut h = harness(app);
    assert!(h.get_by_role_and_label(Role::Slider, "Opacity").accesskit_node().is_disabled());
    let doc = h.state().session.doc().unwrap().doc.clone();
    action(&mut h, Action::SetValue, Some(ActionData::NumericValue(25.0)));
    assert_eq!(*h.state().session.doc().unwrap().doc, *doc);

    let mut app = self::app();
    app.run("text.create", json!({"x": 10, "y": 150, "text": "Hi"})).unwrap();
    let h = harness(app);
    assert!(h.get_by_role_and_label(Role::Slider, "Opacity").accesskit_node().is_disabled());

    // A gradient left as the default for new art: its stop is not the row's to change.
    let mut app = self::app();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let mut h = harness(app);
    assert!(h.get_by_role_and_label(Role::Slider, "Opacity").accesskit_node().is_disabled());
    let defaults = h.state().session.paint.fill.clone();
    action(&mut h, Action::SetValue, Some(ActionData::NumericValue(25.0)));
    assert_eq!(h.state().session.paint.fill, defaults);

    // An object with a gradient fill selected in a dynamic symbol instance.
    let mut app = self::app();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    let id = app.session.doc().unwrap().selection.objects[0].0;
    let inst = app.run("symbol.new", json!({"name": "One", "ids": [id], "type": "dynamic"})).unwrap()["id"].clone();
    app.run("symbol.instance.selectPart", json!({"id": inst, "path": []})).unwrap();
    let h = harness(app);
    assert!(h.get_by_role_and_label(Role::Slider, "Opacity").accesskit_node().is_disabled());
}

/// `ui.colorWheel` answers every junk parameter, with and without a document, without a panic.
#[test]
fn ui_color_wheel_answers_junk_params_without_a_panic() {
    let doc = crate::menus::UI_COMMANDS.iter().find(|c| c.0 == "ui.colorWheel").expect("registered").3;
    for mut app in [app(), VectorcraftApp::new(Session::new(), Default::default())] {
        for p in vectorcraft_testkit::strategies::junk_params(doc, "/nonexistent").into_iter().chain(vectorcraft_testkit::strategies::junk_values()) {
            let _ = app.run("ui.colorWheel", p);
        }
    }
}
