//! The Color Wheel panel's Show Hex Field drawn headless: the setting and `ui.colorWheel`, hex
//! with an alpha and CSS colors typed into the field, the opacity they set, nothing selected, a
//! selection mixing type and other art, gradient stops and freeform points, and refused text.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_engine::Session;

use super::color_wheel::{hex_text, show};
use crate::VectorcraftApp;

/// An app with a selected red rectangle and Show Hex Field checked.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    app.run("ui.colorWheel", json!({"hexField": true})).unwrap();
    app
}

fn harness(app: VectorcraftApp) -> Harness<'static, VectorcraftApp> {
    let mut h = Harness::builder().with_size(vec2(260.0, 420.0)).build_ui_state(
        |ui, app: &mut VectorcraftApp| {
            show(app, ui);
            crate::shortcuts::handle(app, ui.ctx());
        },
        app,
    );
    h.run_steps(2);
    h
}

/// Types `text` over the field's text and presses Enter.
fn type_in(h: &mut Harness<'static, VectorcraftApp>, text: &str) {
    h.get_by_role_and_label(Role::TextInput, "Hex or CSS color").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_role_and_label(Role::TextInput, "Hex or CSS color").type_text(text);
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
}

fn shown(h: &Harness<'static, VectorcraftApp>) -> String {
    h.get_by_role_and_label(Role::TextInput, "Hex or CSS color").value().unwrap_or_default()
}

fn rgb8(c: &Color) -> [u8; 3] {
    let Color::Rgb { r, g, b } = *c else { panic!("an RGB color: {c:?}") };
    [r, g, b].map(|v| (v * 255.0).round() as u8)
}

fn fill_color(app: &VectorcraftApp, id: vectorcraft_doc::NodeId) -> Color {
    app.session.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint().color().expect("a solid fill")
}

fn selected(app: &VectorcraftApp) -> Vec<vectorcraft_doc::NodeId> {
    app.session.doc().unwrap().selection.objects.clone()
}

fn undo_count(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// The panel menu's Show Hex Field and `ui.colorWheel {hexField}` check and uncheck the field; a bad
/// value is an error and changes nothing.
#[test]
fn show_hex_field_is_a_setting_of_the_panel() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    assert_eq!(app.run("ui.colorWheel", json!({})).unwrap()["hexField"], json!(false));
    assert_eq!(app.run("ui.colorWheel", json!({"hexField": true})).unwrap()["hexField"], json!(true));
    let e = app.run("ui.colorWheel", json!({"hexField": "yes"})).unwrap_err();
    assert_eq!(e, "`hexField` must be true or false");
    assert!(app.ui.color_wheel.hex_field, "an error changes nothing");
    assert_eq!(crate::control::inspect(&app, &egui::Context::default())["ui"]["color_wheel"]["hexField"], json!(true));
    let saved: super::color_wheel::Settings = serde_json::from_value(json!({"hexField": true, "view": "box"})).unwrap();
    assert!(saved.hex_field);
    let odd: super::color_wheel::Settings = serde_json::from_value(json!({"hexField": "on"})).unwrap();
    assert!(!odd.hex_field, "a saved value this build does not know reads as off");
    let h = harness({
        let mut app = self::app();
        app.run("ui.colorWheel", json!({"hexField": false})).unwrap();
        app
    });
    assert!(h.query_by_role_and_label(Role::TextInput, "Hex or CSS color").is_none(), "no field while unchecked");
}

/// The field shows six digits, and two more when the opacity rounds to less than `ff`.
#[test]
fn the_field_shows_the_alpha_only_under_full_opacity() {
    let h = harness(app());
    assert_eq!(shown(&h), "FF0000");
    let mut app = app();
    app.run("paint.setFill", json!({"opacity": 50})).unwrap();
    let h = harness(app);
    assert_eq!(shown(&h), "FF000080");
    assert_eq!(hex_text(None, Some(0.5)), "");
    assert_eq!(hex_text(Some(&Color::rgb8(0x33, 0x66, 0x99)), Some(0.999)), "336699", "an opacity that rounds to ff shows no alpha");
    assert_eq!(hex_text(Some(&Color::rgb8(0x33, 0x66, 0x99)), None), "336699");
}

/// With Show Opacity on, typing `#33669980` sets the fill to #336699 at 50.2 % (128 of 255) in one
/// undo step.
#[test]
fn typing_hex_with_an_alpha_sets_the_color_and_the_opacity() {
    let mut app = app();
    app.run("ui.colorWheel", json!({"opacity": true})).unwrap();
    let id = selected(&app)[0];
    let before = undo_count(&app);
    let mut h = harness(app);
    type_in(&mut h, "#33669980");
    assert_eq!(rgb8(&fill_color(h.state(), id)), [0x33, 0x66, 0x99]);
    let o = h.state().session.proxy_opacity(false).unwrap();
    assert!((o - 0.502).abs() < 1e-4, "{o}");
    assert_eq!(undo_count(h.state()), before + 1, "one undo step");
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(rgb8(&fill_color(h.state(), id)), [255, 0, 0]);
    assert_eq!(h.state().session.proxy_opacity(false), Some(1.0));
}

/// A typed color keeps the model of the color it replaces, as a pick on the wheel does: a CMYK fill
/// stays CMYK, converted through the color settings.
#[test]
fn a_typed_color_keeps_a_cmyk_fill_cmyk() {
    let mut app = app();
    app.run("paint.setFill", json!({"color": {"c": 0, "m": 100, "y": 100, "k": 0}})).unwrap();
    let id = selected(&app)[0];
    let mut h = harness(app);
    type_in(&mut h, "#336699");
    let cms = vectorcraft_color::cms::active();
    let want = cms.to_cmyk(&Color::rgb8(0x33, 0x66, 0x99), cms.settings().intent);
    let Color::Cmyk { c, m, y, k } = fill_color(h.state(), id) else { panic!("a CMYK color: {:?}", fill_color(h.state(), id)) };
    assert_eq!([c, m, y, k], want);
}

/// The alpha sets the opacity while the opacity row is hidden too, and text without an alpha keeps
/// the opacity.
#[test]
fn the_alpha_applies_without_the_opacity_row_and_no_alpha_keeps_the_opacity() {
    let app = app();
    let id = selected(&app)[0];
    let mut h = harness(app);
    type_in(&mut h, "rgb(51 102 153 / 25%)");
    assert_eq!(rgb8(&fill_color(h.state(), id)), [51, 102, 153]);
    assert_eq!(h.state().session.proxy_opacity(false), Some(0.25));
    type_in(&mut h, "hsl(0 100% 50%)");
    assert_eq!(rgb8(&fill_color(h.state(), id)), [255, 0, 0]);
    assert_eq!(h.state().session.proxy_opacity(false), Some(0.25), "no alpha in the text: the opacity stays");
    type_in(&mut h, "oklch(62.8% 0.2577 29.23 / 1)");
    assert_eq!(h.state().session.proxy_opacity(false), Some(1.0));
}

/// With nothing selected, `#33669980` sets the default fill to #336699 and changes no opacity.
#[test]
fn with_nothing_selected_the_color_goes_to_the_defaults_without_the_alpha() {
    let mut app = app();
    app.run("select.none", json!({})).unwrap();
    let doc = app.session.doc().unwrap().doc.clone();
    let mut h = harness(app);
    type_in(&mut h, "#33669980");
    assert_eq!(h.state().session.paint.fill.color().map(|c| rgb8(&c)), Some([0x33, 0x66, 0x99]));
    assert_eq!(*h.state().session.doc().unwrap().doc, *doc, "no object changed");
}

/// With a rectangle and a type object with no fill item selected, `#33669980` sets both fills to
/// #336699 and the rectangle's fill opacity to 50.2 %.
#[test]
fn a_selection_mixing_type_and_art_takes_the_color_and_the_alpha_where_it_can() {
    let mut app = app();
    let rect = selected(&app)[0];
    let text = app.run("text.create", json!({"x": 10, "y": 150, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [rect.0, text]})).unwrap();
    let mut h = harness(app);
    type_in(&mut h, "#33669980");
    let app = h.state();
    assert_eq!(rgb8(&fill_color(app, rect)), [0x33, 0x66, 0x99]);
    let fill = app.session.doc().unwrap().doc.node(rect).unwrap().appearance.fill_at(None).map(|f| f.opacity);
    assert!(fill.is_some_and(|o| (o - 0.502).abs() < 1e-4), "{fill:?}");
    let text_node = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(text)).unwrap().appearance.clone();
    assert!(text_node.fill_at(None).is_none(), "the type object takes no fill item, so no alpha: {text_node:?}");
    h.state_mut().run("select.set", json!({"ids": [text]})).unwrap();
    let (fill, _) = h.state().session.proxy_paints();
    assert_eq!(fill.color().map(|c| c.to_hex()), Some("#336699".into()), "the type's characters take the color: {fill:?}");
}

/// With only a type object selected, where the opacity row is gray, `#33669980` sets the
/// characters' color and adds no fill item for the alpha.
#[test]
fn with_only_type_selected_the_color_applies_without_the_alpha() {
    let mut app = app();
    let text = app.run("text.create", json!({"x": 10, "y": 150, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [text]})).unwrap();
    let mut h = harness(app);
    type_in(&mut h, "#33669980");
    let (fill, _) = h.state().session.proxy_paints();
    assert_eq!(fill.color().map(|c| c.to_hex()), Some("#336699".into()), "the characters take the color: {fill:?}");
    let node = h.state().session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(text)).unwrap().appearance.clone();
    assert!(node.fill_at(None).is_none(), "no fill item for the alpha: {node:?}");
}

/// Retyping the color and opacity the selection has, in another form, records no undo step; an
/// alpha that differs from the opacity by less than the field shows is still applied.
#[test]
fn retyping_the_shown_color_in_another_form_changes_nothing() {
    let mut app = app();
    app.run("paint.setFill", json!({"opacity": 50})).unwrap();
    let before = undo_count(&app);
    let mut h = harness(app);
    for same in ["rgb(255 0 0 / 50%)", "rgb(100% 0% 0% / 0.5)", "#f00", "hsl(0 100% 50%)"] {
        type_in(&mut h, same);
        assert_eq!(undo_count(h.state()), before, "{same}");
        assert!(h.state().ui.status.is_empty(), "{same}: {}", h.state().ui.status);
    }
    type_in(&mut h, "#00ff00");
    assert_eq!(undo_count(h.state()), before + 1, "another color is applied");
    h.state_mut().run("paint.setFill", json!({"opacity": 99.9})).unwrap();
    h.run_steps(2);
    assert_eq!(shown(&h), "00FF00", "99.9 % shows no alpha");
    let before = undo_count(h.state());
    type_in(&mut h, "#00ff00ff");
    assert_eq!(undo_count(h.state()), before + 1, "the alpha ff sets 100 %");
    assert_eq!(h.state().session.proxy_opacity(false), Some(1.0));
}

/// Retyping a gradient stop's or a freeform point's color, with or without its alpha, records no
/// step.
#[test]
fn retyping_a_stop_color_changes_nothing() {
    for gradient in [json!({}), json!({"kind": "freeform"})] {
        let mut app = app();
        app.run("paint.setFill", json!({ "gradient": gradient })).unwrap();
        let mut h = harness(app);
        type_in(&mut h, "#33669980");
        let before = undo_count(h.state());
        type_in(&mut h, "#33669980");
        type_in(&mut h, "#369");
        assert_eq!(undo_count(h.state()), before, "{gradient}");
    }
}

/// Retyping the color of a global swatch's tint unlinks the fill from the swatch in one step.
#[test]
fn retyping_a_tint_color_unlinks_the_swatch() {
    let mut app = app();
    app.run("swatch.new", json!({"name": "Ink", "color": "#ff0000", "spot": true})).unwrap();
    app.run("paint.setFill", json!({"swatch": "Ink", "tint": 100})).unwrap();
    let before = undo_count(&app);
    let mut h = harness(app);
    assert_eq!(shown(&h), "FF0000");
    type_in(&mut h, "#ff0000");
    assert_eq!(undo_count(h.state()), before + 1);
    let Paint::Solid { swatch, .. } = crate::panels::current_paints(h.state()).0 else { panic!("a solid fill") };
    assert_eq!(swatch, None);
}

/// A click on the wheel that takes the focus from an edited field applies the text, then the
/// click's pick to the target the text left: a gradient stop keeps the typed alpha.
#[test]
fn a_wheel_click_after_typing_applies_both() {
    let mut app = app();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    let before = undo_count(&app);
    let mut h = harness(app);
    h.get_by_role_and_label(Role::TextInput, "Hex or CSS color").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_role_and_label(Role::TextInput, "Hex or CSS color").type_text("#33669980");
    h.run_steps(1);
    h.get_by_label("Saturation and brightness").click();
    h.run_steps(2);
    let Paint::Gradient(g) = crate::panels::current_paints(h.state()).0 else { panic!("still a gradient") };
    let stop = &g.gradient.stops[0];
    assert!((stop.opacity - 0.502).abs() < 1e-4, "the typed alpha stays: {}", stop.opacity);
    let hex = stop.color.to_hex();
    assert_ne!(hex, "#336699", "the click picked a color");
    let ch = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
    assert!(ch(5) >= ch(3) && ch(3) >= ch(1), "the pick keeps the typed hue: {hex}");
    assert_eq!(undo_count(h.state()), before + 2);
}

/// A press on the opacity slider that takes the focus from an edited field applies the text and
/// keeps the opacity the press set: a gradient stop takes both.
#[test]
fn an_opacity_press_after_typing_keeps_both() {
    let mut app = app();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    app.run("ui.colorWheel", json!({"opacity": true})).unwrap();
    let mut h = harness(app);
    h.get_by_role_and_label(Role::TextInput, "Hex or CSS color").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_role_and_label(Role::TextInput, "Hex or CSS color").type_text("#336699");
    h.run_steps(1);
    h.get_by_role_and_label(Role::Slider, "Opacity").click();
    h.run_steps(2);
    let Paint::Gradient(g) = crate::panels::current_paints(h.state()).0 else { panic!("still a gradient") };
    let stop = &g.gradient.stops[0];
    assert_eq!(stop.color.to_hex(), "#336699", "the text applies");
    assert!(stop.opacity < 0.9, "the press sets the opacity: {}", stop.opacity);
}

/// The same for a solid fill: the rectangle takes the typed color and the opacity the press set.
#[test]
fn an_opacity_press_after_typing_keeps_both_on_a_solid_fill() {
    let mut app = app();
    let rect = selected(&app)[0];
    app.run("ui.colorWheel", json!({"opacity": true})).unwrap();
    let mut h = harness(app);
    h.get_by_role_and_label(Role::TextInput, "Hex or CSS color").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.run_steps(1);
    h.get_by_role_and_label(Role::TextInput, "Hex or CSS color").type_text("#336699");
    h.run_steps(1);
    h.get_by_role_and_label(Role::Slider, "Opacity").click();
    h.run_steps(2);
    let app = h.state();
    assert_eq!(rgb8(&fill_color(app, rect)), [0x33, 0x66, 0x99], "the text applies");
    let fill = app.session.doc().unwrap().doc.node(rect).unwrap().appearance.fill_at(None).map(|f| f.opacity);
    assert!(fill.is_some_and(|o| o < 0.9), "the press sets the opacity: {fill:?}");
}

/// A selected gradient stop takes the color and the alpha in one step.
#[test]
fn a_gradient_stop_takes_the_color_and_the_alpha() {
    let mut app = app();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    let mut h = harness(app);
    type_in(&mut h, "#33669980");
    let Paint::Gradient(g) = crate::panels::current_paints(h.state()).0 else { panic!("still a gradient") };
    let stop = &g.gradient.stops[0];
    assert_eq!(stop.color.to_hex(), "#336699");
    assert!((stop.opacity - 0.502).abs() < 1e-4, "{}", stop.opacity);
}

/// A selected freeform point takes the color and the alpha.
#[test]
fn a_freeform_point_takes_the_color_and_the_alpha() {
    let mut app = app();
    app.run("paint.setFill", json!({"gradient": {"kind": "freeform"}})).unwrap();
    let mut h = harness(app);
    type_in(&mut h, "#33669980");
    let Paint::Gradient(g) = crate::panels::current_paints(h.state()).0 else { panic!("still a gradient") };
    let f = super::gradient::shown_points(&g);
    assert_eq!(f.points[0].color.to_hex(), "#336699");
    assert!((f.points[0].opacity - 0.502).abs() < 1e-4, "{}", f.points[0].opacity);
}

/// Text `color.parse` refuses changes nothing and shows its error in the status bar.
#[test]
fn refused_text_changes_nothing_and_says_why() {
    let app = app();
    let doc = app.session.doc().unwrap().doc.clone();
    let mut h = harness(app);
    for bad in ["#12345", "rgb(1, 2%, 3)", "not a color", "cmyk(0 0 0 0)"] {
        type_in(&mut h, bad);
        assert_eq!(*h.state().session.doc().unwrap().doc, *doc, "{bad}");
        assert!(h.state().ui.status.contains("must be a hex color"), "{bad}: {}", h.state().ui.status);
    }
    assert_eq!(shown(&h), "FF0000", "the field shows the color again");
    h.state_mut().ui.status.clear();
    h.get_by_role_and_label(Role::TextInput, "Hex or CSS color").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.key_press(egui::Key::Backspace);
    h.run_steps(1);
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(*h.state().session.doc().unwrap().doc, *doc, "an emptied field");
    assert!(h.state().ui.status.contains("must be a hex color"), "an emptied field: {}", h.state().ui.status);
}

/// The Color panel's hex field still takes only 3 or 6 digits.
#[test]
fn the_color_panels_hex_field_still_refuses_an_alpha() {
    assert_eq!(super::color::parse_hex("336699").map(|c| c.to_hex()), Some("#336699".into()));
    assert_eq!(super::color::parse_hex("33669980"), None);
    assert_eq!(super::color::parse_hex("3698"), None);
}
