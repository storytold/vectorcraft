//! The Color Wheel panel's Show Opacity row drawn headless through the `Fixture`: the setting and
//! `ui.colorWheel`, the slider on a fill, a gradient stop and a freeform point, the keys, screen
//! readers, the percent field and the gray row.

use craft::ui::runtime::access::Role;
use craft::ui::testing::Target;
use craft::ui::testing::focus::AccessActionKind;
use egui::{Key, Modifiers};
use serde_json::json;
use vectorcraft_color::Paint;
use vectorcraft_engine::Session;

use super::color_wheel::Settings;
use super::tests_color_wheel::Panel;
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

/// The node of `role` named Opacity.
fn named(p: &Panel, role: Role) -> Option<Target> {
    p.fx.by_role(role).into_iter().find(|t| p.fx.tree().name_of(t.node) == Some("Opacity"))
}

/// The row's slider.
fn slider(p: &Panel) -> Target {
    named(p, Role::SLIDER).expect("the Opacity slider")
}

/// The row's percent field.
fn field(p: &Panel) -> Target {
    named(p, Role::SPIN_BUTTON).or_else(|| named(p, Role::TEXT_INPUT)).expect("the Opacity field")
}

fn disabled(p: &Panel) -> bool {
    p.fx.tree().is_disabled_key(slider(p).node)
}

/// An AccessKit action on the slider, then two frames.
fn act(p: &mut Panel, kind: AccessActionKind) {
    let t = slider(p);
    p.fx.access(kind, &t).idle(2);
}

fn fill_opacity(app: &VectorcraftApp) -> f32 {
    app.session.proxy_opacity(false).expect("a fill item")
}

fn undo_labels(app: &VectorcraftApp) -> Vec<String> {
    app.session.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect()
}

/// The point at `f` (0 to 1) across the middle of the slider.
fn along(t: &Target, f: f32) -> egui::Pos2 {
    egui::pos2(t.rect.left() + f * t.rect.width(), t.rect.center().y)
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
    assert_eq!(super::color_wheel::run(&mut app, "ui.colorWheel.opacity", &json!({})).unwrap().unwrap()["opacity"], json!(false));
    let p = Panel::new({
        let mut app = self::app();
        app.run("ui.colorWheel", json!({"opacity": false})).unwrap();
        app
    });
    assert!(named(&p, Role::SLIDER).is_none(), "no row while unchecked");
}

/// The row also shows under the box, and a click at its middle sets the fill item's opacity there.
#[test]
fn the_row_shows_under_the_box_view() {
    let mut app = app();
    app.run("ui.colorWheel", json!({"view": "box"})).unwrap();
    let mut p = Panel::new(app);
    assert!(p.fx.by_label("Color box").is_some(), "the box shows");
    let at = along(&slider(&p), 0.5);
    p.fx.click_at(at).idle(2);
    let o = fill_opacity(&p.app());
    assert!((o - 0.5).abs() < 0.05, "{o}");
}

/// A value this build does not read keeps the rest of the preferences and reads as the default.
#[test]
fn saved_settings_read_leniently() {
    let mut saved = serde_json::to_value(crate::UiState::default()).unwrap();
    saved["color_panel"] = json!({"rgbPercent": true});
    for bad in [json!("box"), json!({"opacity": "yes"}), json!(7)] {
        saved["color_wheel"] = bad.clone();
        let ui: crate::UiState = serde_json::from_value(saved.clone()).unwrap();
        assert_eq!(ui.color_wheel, Settings::default(), "{bad}");
        assert!(ui.color_panel.rgb_percent, "{bad}");
    }
    saved["color_wheel"] = json!({"opacity": true, "later": 1});
    let ui: crate::UiState = serde_json::from_value(saved).unwrap();
    assert!(ui.color_wheel.opacity);
}

/// A click at the middle of the track sets the fill item's opacity to about 50 % as one undo step
/// named Opacity, the fill keeps its color, and → then adds 1 %.
#[test]
fn the_slider_sets_the_fill_opacity_and_takes_the_arrow_keys() {
    let mut p = Panel::new(app());
    let undo = undo_labels(&p.app()).len();
    let at = along(&slider(&p), 0.5);
    p.fx.click_at(at).idle(2);
    let o = fill_opacity(&p.app());
    assert!((o - 0.5).abs() < 0.05, "{o}");
    let labels = undo_labels(&p.app());
    assert_eq!((labels.len(), labels.last().map(String::as_str)), (undo + 1, Some("Opacity")));
    assert_eq!(crate::panels::current_paints(&p.app()).0.color().unwrap().to_hex(), "#ff0000");
    let before = (o * 100.0).round();
    let t = slider(&p);
    p.fx.access(AccessActionKind::Focus, &t).key(Key::ArrowRight, Modifiers::NONE).idle(2);
    assert_eq!((fill_opacity(&p.app()) * 100.0).round(), before + 1.0);
}

/// Hiding the row during a drag on its slider keeps the opacity the drag previewed as one undo step
/// and leaves no interaction open.
#[test]
fn hiding_the_row_during_a_drag_keeps_its_opacity_as_one_undo_step() {
    let mut p = Panel::new(app());
    let undo = undo_labels(&p.app()).len();
    let t = slider(&p);
    let (from, to) = (along(&t, 0.5), along(&t, 0.25));
    p.fx.drag_hold_at(from, to - from);
    assert!(p.app().session.in_interaction(), "the drag previews");
    let previewed = fill_opacity(&p.app());
    assert!(previewed < 0.45, "{previewed}");
    p.run("ui.colorWheel", json!({"opacity": false}));
    assert!(!p.app().session.in_interaction(), "the interaction ended");
    assert!((fill_opacity(&p.app()) - previewed).abs() < 1e-6);
    p.fx.release_at(to).idle(2);
    assert_eq!(undo_labels(&p.app()).len(), undo + 1);
}

/// Screen readers get a slider named Opacity with its percentage and its range, and can step it and
/// set it.
#[test]
fn screen_readers_read_and_set_the_opacity() {
    let mut p = Panel::new(app());
    let numeric = p.fx.tree().numeric_of(slider(&p).node).expect("a numeric slider");
    assert_eq!(numeric.value, Some(100.0));
    assert_eq!(numeric.range, 0.0..=100.0);
    act(&mut p, AccessActionKind::SetValue(25.0));
    assert!((fill_opacity(&p.app()) - 0.25).abs() < 1e-6);
    act(&mut p, AccessActionKind::Decrement);
    assert!((fill_opacity(&p.app()) - 0.24).abs() < 1e-6);
}

/// The stroke proxy in front: the row sets the stroke item's opacity and leaves the fill's.
#[test]
fn with_the_stroke_in_front_the_row_sets_the_stroke() {
    let mut app = app();
    app.run("paint.toggleActive", json!({})).unwrap();
    let mut p = Panel::new(app);
    act(&mut p, AccessActionKind::SetValue(40.0));
    assert!((p.app().session.proxy_opacity(true).unwrap() - 0.4).abs() < 1e-6);
    assert_eq!(fill_opacity(&p.app()), 1.0);
}

/// A selected gradient stop takes the opacity, and the other stop keeps its own.
#[test]
fn a_selected_gradient_stop_takes_the_opacity() {
    let mut app = app();
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [10, 35], "end": [60, 35]}})).unwrap();
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 1})).unwrap();
    let mut p = Panel::new(app);
    act(&mut p, AccessActionKind::SetValue(30.0));
    let Paint::Gradient(g) = crate::panels::current_paints(&p.app()).0 else { panic!("still a gradient") };
    let o: Vec<f32> = g.gradient.stops.iter().map(|s| s.opacity).collect();
    assert!((o[0] - 1.0).abs() < 1e-6 && (o[1] - 0.3).abs() < 1e-6, "{o:?}");
    assert_eq!(fill_opacity(&p.app()), 1.0, "the fill item keeps its opacity");
}

/// With no stop selected, the first stop takes the opacity, as it takes a color pick.
#[test]
fn without_a_selected_stop_the_first_takes_the_opacity() {
    let mut app = app();
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [10, 35], "end": [60, 35]}})).unwrap();
    assert_eq!(app.session.selected_stop(), None);
    let mut p = Panel::new(app);
    act(&mut p, AccessActionKind::SetValue(30.0));
    let Paint::Gradient(g) = crate::panels::current_paints(&p.app()).0 else { panic!("still a gradient") };
    let o: Vec<f32> = g.gradient.stops.iter().map(|s| s.opacity).collect();
    assert!((o[0] - 0.3).abs() < 1e-6 && (o[1] - 1.0).abs() < 1e-6, "{o:?}");
}

/// The hue picked last for a white gradient stop stays on the box's strip after the row sets the
/// stop's opacity.
#[test]
fn an_opacity_edit_keeps_the_hue_picked_for_a_white_stop() {
    let mut app = app();
    app.run("ui.colorWheel", json!({"view": "box", "channel": "hue"})).unwrap();
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [10, 35], "end": [60, 35]}})).unwrap();
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 0})).unwrap();
    let mut p = Panel::new(app);
    p.set("Hue", 120.0);
    assert_eq!(p.number("Hue"), Some(120.0), "the strip shows the hue picked for white");
    act(&mut p, AccessActionKind::SetValue(30.0));
    let Paint::Gradient(g) = crate::panels::current_paints(&p.app()).0 else { panic!("still a gradient") };
    assert!((g.gradient.stops[0].opacity - 0.3).abs() < 1e-6, "{:?}", g.gradient.stops);
    assert_eq!(p.number("Hue"), Some(120.0), "the strip keeps the hue after the opacity edit");
}

/// The OKLCH hue picked last for a white gradient stop stays on the OKLCH Hue row after the row
/// sets the stop's opacity.
#[test]
fn an_opacity_edit_keeps_the_oklch_hue_picked_for_a_white_stop() {
    let mut app = app();
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [10, 35], "end": [60, 35]}})).unwrap();
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 0})).unwrap();
    app.run("ui.colorWheel", json!({"sliders": "oklch"})).unwrap();
    let mut p = Panel::new(app);
    p.fx.room(egui::vec2(260.0, 480.0)).settle();
    p.set("Hue", 120.0);
    assert_eq!(p.number("Hue"), Some(120.0), "the Hue row shows the hue picked for white");
    act(&mut p, AccessActionKind::SetValue(30.0));
    let Paint::Gradient(g) = crate::panels::current_paints(&p.app()).0 else { panic!("still a gradient") };
    assert!((g.gradient.stops[0].opacity - 0.3).abs() < 1e-6, "{:?}", g.gradient.stops);
    assert_eq!(p.number("Hue"), Some(120.0), "the Hue row keeps the hue after the opacity edit");
}

/// A screen reader's SetValue for the Opacity slider and one for the node named `label`, in one
/// frame.
fn set_both(p: &mut Panel, opacity: f64, (label, value): (&str, f64)) {
    use egui::accesskit::{Action, ActionData, ActionRequest, NodeId, TreeId};
    let set = |t: &Target, v: f64| {
        let target_node = NodeId(t.node.raw());
        egui::Event::AccessKitActionRequest(ActionRequest {
            action: Action::SetValue,
            target_tree: TreeId::ROOT,
            target_node,
            data: Some(ActionData::NumericValue(v)),
        })
    };
    let events = vec![set(&slider(p), opacity), set(&p.node(label), value)];
    p.fx.frame(events).idle(2);
}

/// A gradient stop with stops red and blue, the first selected.
fn stop_app() -> VectorcraftApp {
    let mut app = app();
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops, "start": [10, 35], "end": [60, 35]}})).unwrap();
    app.select_tool("gradient");
    app.run("gradient.selectStop", json!({"index": 0})).unwrap();
    app
}

/// The first stop of the fill's gradient.
fn first_stop(p: &Panel) -> vectorcraft_color::GradientStop {
    let Paint::Gradient(g) = crate::panels::current_paints(&p.app()).0 else { panic!("still a gradient") };
    g.gradient.stops[0].clone()
}

/// An opacity edit and a pick on the box's hue strip in one frame: the stop keeps both.
#[test]
fn an_opacity_edit_and_a_hue_pick_in_one_frame_keep_both() {
    let mut app = stop_app();
    app.run("ui.colorWheel", json!({"view": "box", "channel": "hue"})).unwrap();
    let mut p = Panel::new(app);
    set_both(&mut p, 30.0, ("Hue", 120.0));
    let stop = first_stop(&p);
    assert!((stop.opacity - 0.3).abs() < 1e-6, "the stop keeps the opacity: {stop:?}");
    assert_ne!(stop.color.to_hex(), "#ff0000", "the stop takes the pick: {stop:?}");
}

/// An opacity edit and an OKLCH Hue edit in one frame: the stop keeps both.
#[test]
fn an_opacity_edit_and_an_oklch_edit_in_one_frame_keep_both() {
    let mut app = stop_app();
    app.run("ui.colorWheel", json!({"sliders": "oklch"})).unwrap();
    let mut p = Panel::new(app);
    p.fx.room(egui::vec2(260.0, 480.0)).settle();
    set_both(&mut p, 30.0, ("Hue", 140.0));
    let stop = first_stop(&p);
    assert!((stop.opacity - 0.3).abs() < 1e-6, "the stop keeps the opacity: {stop:?}");
    assert_ne!(stop.color.to_hex(), "#ff0000", "the stop takes the edit: {stop:?}");
}

/// A drag across the slider previews and ends as one undo step named Opacity.
#[test]
fn a_drag_is_one_undo_step() {
    let mut p = Panel::new(app());
    let undo = undo_labels(&p.app()).len();
    let t = slider(&p);
    let (from, to) = (along(&t, 0.1), along(&t, 0.9));
    p.fx.drag_at(from, to - from).idle(2);
    let o = fill_opacity(&p.app());
    assert!(o > 0.7 && o < 1.0, "{o}");
    let labels = undo_labels(&p.app());
    assert_eq!((labels.len(), labels.last().map(String::as_str)), (undo + 1, Some("Opacity")));
    assert!(!p.app().session.in_interaction());
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
    let mut p = Panel::new(app);
    assert!(!disabled(&p));
    act(&mut p, AccessActionKind::SetValue(45.0));
    let n = p.app().session.doc().unwrap().doc.node(second).unwrap().clone();
    assert!((n.appearance.fill().unwrap().opacity - 0.45).abs() < 1e-6);
}

/// Type painted with a gradient: the row sets the gradient's stop, which belongs to the gradient.
#[test]
fn a_gradient_stop_on_type_takes_the_opacity() {
    let mut app = app();
    app.run("text.create", json!({"x": 10, "y": 150, "text": "Hi"})).unwrap();
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    app.run("paint.setFill", json!({"gradient": {"stops": stops}})).unwrap();
    let mut p = Panel::new(app);
    assert!(!disabled(&p));
    act(&mut p, AccessActionKind::SetValue(30.0));
    let Paint::Gradient(g) = crate::panels::current_paints(&p.app()).0 else { panic!("still a gradient") };
    assert!((g.gradient.stops[0].opacity - 0.3).abs() < 1e-6, "{:?}", g.gradient.stops);
}

/// The selected freeform point takes the opacity.
#[test]
fn the_selected_freeform_point_takes_the_opacity() {
    let mut app = app();
    app.run("paint.setFill", json!({"gradient": {"kind": "freeform"}})).unwrap();
    let mut p = Panel::new(app);
    act(&mut p, AccessActionKind::SetValue(60.0));
    let Paint::Gradient(g) = crate::panels::current_paints(&p.app()).0 else { panic!("still a gradient") };
    let f = super::gradient::shown_points(&g);
    assert!((f.points[0].opacity - 0.6).abs() < 1e-6, "{:?}", f.points.iter().map(|p| p.opacity).collect::<Vec<_>>());
}

/// The percent field takes a typed value.
#[test]
fn the_percent_field_takes_a_typed_value() {
    let mut p = Panel::new(app());
    let t = field(&p);
    p.fx.click(&t).key(Key::A, Modifiers::COMMAND).type_text("35").key(Key::Enter, Modifiers::NONE).settle();
    assert!((fill_opacity(&p.app()) - 0.35).abs() < 1e-6, "{}", fill_opacity(&p.app()));
}

/// With nothing selected, and with only type selected (its characters take the paint), the row is
/// gray and screen readers cannot set it.
#[test]
fn the_row_is_gray_with_nothing_selected_and_on_type() {
    let mut app = app();
    app.run("select.none", json!({})).unwrap();
    let mut p = Panel::new(app);
    assert!(disabled(&p));
    let doc = p.app().session.doc().unwrap().doc.clone();
    act(&mut p, AccessActionKind::SetValue(25.0));
    assert_eq!(*p.app().session.doc().unwrap().doc, *doc);

    let mut app = self::app();
    app.run("text.create", json!({"x": 10, "y": 150, "text": "Hi"})).unwrap();
    let p = Panel::new(app);
    assert!(disabled(&p));

    // A gradient left as the default for new art: its stop is not the row's to change.
    let mut app = self::app();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    app.run("select.none", json!({})).unwrap();
    let mut p = Panel::new(app);
    assert!(disabled(&p));
    let defaults = p.app().session.paint.fill.clone();
    act(&mut p, AccessActionKind::SetValue(25.0));
    assert_eq!(p.app().session.paint.fill, defaults);

    // The Live Paint Bucket painting a gradient with an object selected: the stop is the bucket's.
    let mut app = self::app();
    app.session.select_tool("livePaintBucket", Default::default()).unwrap();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    let mut p = Panel::new(app);
    assert!(disabled(&p));
    let defaults = p.app().session.paint.fill.clone();
    act(&mut p, AccessActionKind::SetValue(25.0));
    assert_eq!(p.app().session.paint.fill, defaults);

    // An object with a gradient fill selected in a dynamic symbol instance.
    let mut app = self::app();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    let id = app.session.doc().unwrap().selection.objects[0].0;
    let inst = app.run("symbol.new", json!({"name": "One", "ids": [id], "type": "dynamic"})).unwrap()["id"].clone();
    app.run("symbol.instance.selectPart", json!({"id": inst, "path": []})).unwrap();
    let p = Panel::new(app);
    assert!(disabled(&p));
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
