//! The Color Wheel panel's Show Hex Field drawn headless through the `Fixture`: the setting and
//! `ui.colorWheel`, hex with an alpha and CSS colors typed into the field, the opacity they set,
//! nothing selected, a selection mixing type and other art, gradient stops and freeform points,
//! and refused text.

use craft::ui::runtime::access::Role;
use craft::ui::testing::Target;
use egui::{Key, Modifiers, Pos2, vec2};
use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_engine::Session;

use super::color_wheel::hex_text;
use super::tests_color_wheel::Panel;
use crate::VectorcraftApp;

const FIELD: &str = "Hex or CSS color";

/// An app with a selected red rectangle and Show Hex Field checked.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    app.run("ui.colorWheel", json!({"hexField": true})).unwrap();
    app
}

/// The panel of `app`, with room for every row.
fn panel(app: VectorcraftApp) -> Panel {
    let mut p = Panel::new(app);
    p.fx.room(vec2(260.0, 480.0)).settle();
    p
}

fn field(p: &Panel) -> Option<Target> {
    p.fx.by_label(FIELD)
}

/// Selects the field's text and types `text` over it, without committing it.
fn type_pending(p: &mut Panel, text: &str) {
    let t = field(p).expect("the hex field");
    p.fx.click(&t).key(Key::A, Modifiers::COMMAND).type_text(text);
}

/// Types `text` over the field's text and presses Enter.
fn type_in(p: &mut Panel, text: &str) {
    type_pending(p, text);
    p.fx.key(Key::Enter, Modifiers::NONE).settle();
}

fn shown(p: &Panel) -> String {
    p.value(FIELD).unwrap_or_default()
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

/// The hex value and opacity of the fill's first gradient stop.
fn first_stop(p: &Panel) -> (String, f32) {
    let Paint::Gradient(g) = crate::panels::current_paints(&p.app()).0 else { panic!("still a gradient") };
    let s = &g.gradient.stops[0];
    (s.color.to_hex(), s.opacity)
}

/// The Opacity row's slider.
fn opacity_slider(p: &Panel) -> Target {
    p.fx.by_role(Role::SLIDER).into_iter().find(|t| p.fx.tree().name_of(t.node) == Some("Opacity")).expect("the Opacity slider")
}

/// A click at `pos` whose press and release come in one frame.
fn quick_click(p: &mut Panel, pos: Pos2) {
    p.fx.frame(vec![egui::Event::PointerMoved(pos)]);
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    p.fx.frame(vec![button(true), button(false)]).idle(2);
}

/// The point near the top right corner of the saturation and brightness square.
fn square_top_right(p: &Panel) -> Pos2 {
    p.node("Saturation and brightness").rect.right_top() + vec2(-1.0, 1.0)
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
    assert_eq!(app.run("ui.colorWheel", json!({"hexField": null})).unwrap()["hexField"], json!(true), "null keeps it");
    assert_eq!(crate::control::inspect(&app, &egui::Context::default())["ui"]["color_wheel"]["hexField"], json!(true));
    assert_eq!(super::color_wheel::run(&mut app, "ui.colorWheel.hexField", &json!({})).unwrap().unwrap()["hexField"], json!(false));
    let saved: super::color_wheel::Settings = serde_json::from_value(json!({"hexField": true, "view": "box"})).unwrap();
    assert!(saved.hex_field);
    let odd: super::color_wheel::Settings = serde_json::from_value(json!({"hexField": "on"})).unwrap();
    assert!(!odd.hex_field, "a saved value this build does not know reads as off");
    let p = panel({
        let mut app = self::app();
        app.run("ui.colorWheel", json!({"hexField": false})).unwrap();
        app
    });
    assert!(field(&p).is_none(), "no field while unchecked");
}

/// An explicit null for any `ui.colorWheel` key keeps that key's value, as leaving the key out does,
/// and the other keys of the same call still apply.
#[test]
fn ui_color_wheel_keeps_a_key_given_as_null() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let set = json!({
        "view": "box", "channel": "red", "opacity": true, "hexField": true, "sliders": "oklch",
        "gamut": true, "harmony": true, "harmonyRule": "triad", "vision": true, "documentColors": true
    });
    let all = app.run("ui.colorWheel", set.clone()).unwrap();
    assert_eq!(all, set);
    for key in ["view", "channel", "opacity", "hexField", "sliders", "gamut", "harmony", "harmonyRule", "vision", "documentColors"] {
        assert_eq!(app.run("ui.colorWheel", json!({ key: null })).unwrap(), all, "{key}: null");
    }
    let got = app.run("ui.colorWheel", json!({"view": null, "channel": "blue", "opacity": null, "hexField": false})).unwrap();
    assert_eq!(
        got,
        json!({
            "view": "box", "channel": "blue", "opacity": true, "hexField": false, "sliders": "oklch",
            "gamut": true, "harmony": true, "harmonyRule": "triad", "vision": true, "documentColors": true
        })
    );
}

/// The field shows six digits, and two more when the opacity rounds to less than `ff`.
#[test]
fn the_field_shows_the_alpha_only_under_full_opacity() {
    let p = panel(app());
    assert_eq!(shown(&p), "FF0000");
    let mut app = app();
    app.run("paint.setFill", json!({"opacity": 50})).unwrap();
    let p = panel(app);
    assert_eq!(shown(&p), "FF000080");
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
    let mut p = panel(app);
    type_in(&mut p, "#33669980");
    assert_eq!(rgb8(&fill_color(&p.app(), id)), [0x33, 0x66, 0x99]);
    let o = p.app().session.proxy_opacity(false).unwrap();
    assert!((o - 0.502).abs() < 1e-4, "{o}");
    assert_eq!(undo_count(&p.app()), before + 1, "one undo step");
    p.app_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(rgb8(&fill_color(&p.app(), id)), [255, 0, 0]);
    assert_eq!(p.app().session.proxy_opacity(false), Some(1.0));
}

/// A typed color keeps the model of the color it replaces, as a pick on the wheel does: a CMYK fill
/// stays CMYK, converted through the color settings.
#[test]
fn a_typed_color_keeps_a_cmyk_fill_cmyk() {
    let mut app = app();
    app.run("paint.setFill", json!({"color": {"c": 0, "m": 100, "y": 100, "k": 0}})).unwrap();
    let id = selected(&app)[0];
    let mut p = panel(app);
    type_in(&mut p, "#336699");
    let cms = vectorcraft_color::cms::active();
    let want = cms.to_cmyk(&Color::rgb8(0x33, 0x66, 0x99), cms.settings().intent);
    let got = fill_color(&p.app(), id);
    let Color::Cmyk { c, m, y, k } = got else { panic!("a CMYK color: {got:?}") };
    assert_eq!([c, m, y, k], want);
}

/// The alpha sets the opacity while the opacity row is hidden too, and text without an alpha keeps
/// the opacity.
#[test]
fn the_alpha_applies_without_the_opacity_row_and_no_alpha_keeps_the_opacity() {
    let app = app();
    let id = selected(&app)[0];
    let mut p = panel(app);
    type_in(&mut p, "rgb(51 102 153 / 25%)");
    assert_eq!(rgb8(&fill_color(&p.app(), id)), [51, 102, 153]);
    assert_eq!(p.app().session.proxy_opacity(false), Some(0.25));
    type_in(&mut p, "hsl(0 100% 50%)");
    assert_eq!(rgb8(&fill_color(&p.app(), id)), [255, 0, 0]);
    assert_eq!(p.app().session.proxy_opacity(false), Some(0.25), "no alpha in the text: the opacity stays");
    type_in(&mut p, "oklch(62.8% 0.2577 29.23 / 1)");
    assert_eq!(p.app().session.proxy_opacity(false), Some(1.0));
}

/// With nothing selected, `#33669980` sets the default fill to #336699 and changes no opacity.
#[test]
fn with_nothing_selected_the_color_goes_to_the_defaults_without_the_alpha() {
    let mut app = app();
    app.run("select.none", json!({})).unwrap();
    let doc = app.session.doc().unwrap().doc.clone();
    let mut p = panel(app);
    type_in(&mut p, "#33669980");
    assert_eq!(p.app().session.paint.fill.color().map(|c| rgb8(&c)), Some([0x33, 0x66, 0x99]));
    assert_eq!(*p.app().session.doc().unwrap().doc, *doc, "no object changed");
}

/// With a rectangle and a type object with no fill item selected, `#33669980` sets both fills to
/// #336699 and the rectangle's fill opacity to 50.2 %.
#[test]
fn a_selection_mixing_type_and_art_takes_the_color_and_the_alpha_where_it_can() {
    let mut app = app();
    let rect = selected(&app)[0];
    let text = app.run("text.create", json!({"x": 10, "y": 150, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [rect.0, text]})).unwrap();
    let mut p = panel(app);
    type_in(&mut p, "#33669980");
    {
        let app = p.app();
        assert_eq!(rgb8(&fill_color(&app, rect)), [0x33, 0x66, 0x99]);
        let fill = app.session.doc().unwrap().doc.node(rect).unwrap().appearance.fill_at(None).map(|f| f.opacity);
        assert!(fill.is_some_and(|o| (o - 0.502).abs() < 1e-4), "{fill:?}");
        let text_node = app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(text)).unwrap().appearance.clone();
        assert!(text_node.fill_at(None).is_none(), "the type object takes no fill item, so no alpha: {text_node:?}");
    }
    p.app_mut().run("select.set", json!({"ids": [text]})).unwrap();
    let (fill, _) = p.app().session.proxy_paints();
    assert_eq!(fill.color().map(|c| c.to_hex()), Some("#336699".into()), "the type's characters take the color: {fill:?}");
}

/// With only a type object selected, where the opacity row is gray, `#33669980` sets the
/// characters' color and adds no fill item for the alpha.
#[test]
fn with_only_type_selected_the_color_applies_without_the_alpha() {
    let mut app = app();
    let text = app.run("text.create", json!({"x": 10, "y": 150, "text": "Hi"})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [text]})).unwrap();
    let mut p = panel(app);
    type_in(&mut p, "#33669980");
    let (fill, _) = p.app().session.proxy_paints();
    assert_eq!(fill.color().map(|c| c.to_hex()), Some("#336699".into()), "the characters take the color: {fill:?}");
    let node = p.app().session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(text)).unwrap().appearance.clone();
    assert!(node.fill_at(None).is_none(), "no fill item for the alpha: {node:?}");
}

/// Retyping the color and opacity the selection has, in another form, records no undo step; an
/// alpha that differs from the opacity by less than the field shows is still applied.
#[test]
fn retyping_the_shown_color_in_another_form_changes_nothing() {
    let mut app = app();
    app.run("paint.setFill", json!({"opacity": 50})).unwrap();
    let before = undo_count(&app);
    let mut p = panel(app);
    for same in ["rgb(255 0 0 / 50%)", "rgb(100% 0% 0% / 0.5)", "#f00", "hsl(0 100% 50%)"] {
        type_in(&mut p, same);
        assert_eq!(undo_count(&p.app()), before, "{same}");
        assert!(p.app().ui.status.is_empty(), "{same}: {}", p.app().ui.status);
    }
    let text = shown(&p);
    assert_eq!(text, "FF000080");
    type_in(&mut p, &text);
    assert_eq!(undo_count(&p.app()), before, "the shown text");
    assert_eq!(p.app().session.proxy_opacity(false), Some(0.5), "the shown alpha keeps 50 %");
    type_in(&mut p, "rgb(255 0 0 / 50.4%)");
    assert_eq!(undo_count(&p.app()), before + 1, "an alpha with the same two digits is applied");
    assert!(p.app().session.proxy_opacity(false).is_some_and(|o| (o - 0.504).abs() < 1e-4));
    let before = undo_count(&p.app());
    type_in(&mut p, "#00ff00");
    assert_eq!(undo_count(&p.app()), before + 1, "another color is applied");
    p.run("paint.setFill", json!({"opacity": 99.9}));
    assert_eq!(shown(&p), "00FF00", "99.9 % shows no alpha");
    let before = undo_count(&p.app());
    type_in(&mut p, "#00ff00ff");
    assert_eq!(undo_count(&p.app()), before + 1, "the alpha ff sets 100 %");
    assert_eq!(p.app().session.proxy_opacity(false), Some(1.0));
}

/// Retyping the shown text with two objects selected gives the second the first one's opacity, the
/// one the field shows, and leaves the first's as it was.
#[test]
fn retyping_the_shown_alpha_gives_the_selection_the_shown_opacity() {
    let mut app = app();
    let first = selected(&app)[0];
    app.run("paint.setFill", json!({"opacity": 50})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 50, "height": 50})).unwrap();
    let second = selected(&app)[0];
    app.run("paint.setFill", json!({"color": "#ff0000", "opacity": 80})).unwrap();
    app.run("select.set", json!({"ids": [first, second]})).unwrap();
    let mut p = panel(app);
    let text = shown(&p);
    assert_eq!(text, "FF000080");
    type_in(&mut p, &text);
    let opacity = |id| p.app().session.doc().unwrap().doc.node(id).unwrap().appearance.fill_at(None).map(|f| f.opacity);
    assert_eq!(opacity(first), Some(0.5), "the first keeps 50 %");
    assert_eq!(opacity(second), Some(0.5), "the second takes 50 %");
}

/// Retyping a gradient stop's or a freeform point's color, with or without its alpha, records no
/// step.
#[test]
fn retyping_a_stop_color_changes_nothing() {
    for gradient in [json!({}), json!({"kind": "freeform"})] {
        let mut app = app();
        app.run("paint.setFill", json!({ "gradient": gradient })).unwrap();
        let mut p = panel(app);
        type_in(&mut p, "#33669980");
        let before = undo_count(&p.app());
        type_in(&mut p, "#33669980");
        type_in(&mut p, "#369");
        assert_eq!(undo_count(&p.app()), before, "{gradient}");
    }
}

/// Retyping the color of a global swatch's tint unlinks the fill from the swatch in one step.
#[test]
fn retyping_a_tint_color_unlinks_the_swatch() {
    let mut app = app();
    app.run("swatch.new", json!({"name": "Ink", "color": "#ff0000", "spot": true})).unwrap();
    app.run("paint.setFill", json!({"swatch": "Ink", "tint": 100})).unwrap();
    let before = undo_count(&app);
    let mut p = panel(app);
    assert_eq!(shown(&p), "FF0000");
    type_in(&mut p, "#ff0000");
    assert_eq!(undo_count(&p.app()), before + 1);
    let Paint::Solid { swatch, .. } = crate::panels::current_paints(&p.app()).0 else { panic!("a solid fill") };
    assert_eq!(swatch, None);
}

/// A click on the square that takes the focus from an edited field applies the text, then the
/// click's pick to the target the text left: a gradient stop keeps the typed alpha.
#[test]
fn a_wheel_click_after_typing_applies_both() {
    let mut app = app();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    let before = undo_count(&app);
    let mut p = panel(app);
    type_pending(&mut p, "#33669980");
    let at = square_top_right(&p);
    p.fx.click_at(at).idle(2);
    let (hex, opacity) = first_stop(&p);
    assert!((opacity - 0.502).abs() < 1e-4, "the typed alpha stays: {opacity}");
    assert_ne!(hex, "#336699", "the click picked a color");
    let ch = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
    assert!(ch(5) >= ch(3) && ch(3) >= ch(1), "the pick keeps the typed hue: {hex}");
    assert_eq!(undo_count(&p.app()), before + 2);
}

/// A click on the square in one frame that takes the focus from an edited field applies the text,
/// then the click's saturation and brightness at the hue the text gave: a gradient stop keeps the
/// typed alpha.
#[test]
fn a_wheel_click_in_one_frame_after_typing_applies_both() {
    let mut app = app();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    let before = undo_count(&app);
    let mut p = panel(app);
    type_pending(&mut p, "#33669980");
    let at = square_top_right(&p);
    quick_click(&mut p, at);
    let (hex, opacity) = first_stop(&p);
    assert!((opacity - 0.502).abs() < 1e-4, "the typed alpha stays: {opacity}");
    assert_ne!(hex, "#336699", "the click picked a color");
    let ch = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).unwrap();
    assert!(ch(5) > 0xf0 && ch(1) < 0x10 && ch(3) > 0x60 && ch(3) < 0xa0, "the pick keeps the typed hue: {hex}");
    assert_eq!(undo_count(&p.app()), before + 2);
}

/// A lightness typed in the OKLCH L field and committed by a click on the C row in one frame keeps
/// both edits: the color takes the typed L and the clicked C, and the hue stays.
#[test]
fn a_typed_lightness_committed_by_a_click_on_chroma_keeps_both() {
    let mut app = app();
    app.run("ui.colorWheel", json!({"hexField": false, "sliders": "oklch"})).unwrap();
    let mut p = panel(app);
    let named = |p: &Panel, role: Role, name: &str| p.fx.by_role(role).into_iter().find(|t| p.fx.tree().name_of(t.node) == Some(name));
    let oklch = |p: &Panel| {
        let id = selected(&p.app())[0];
        let Color::Rgb { r, g, b } = fill_color(&p.app(), id) else { panic!("an RGB fill") };
        vectorcraft_color::oklab::to_oklch([r, g, b])
    };
    let h0 = oklch(&p)[2];
    let field = named(&p, Role::SPIN_BUTTON, "Lightness").or_else(|| named(&p, Role::TEXT_INPUT, "Lightness")).expect("the L field");
    p.fx.click(&field).key(Key::A, Modifiers::COMMAND).type_text("50");
    let rail = named(&p, Role::SLIDER, "Chroma").expect("the C slider").rect;
    quick_click(&mut p, egui::pos2(rail.left() + 0.2 * rail.width(), rail.center().y));
    let [l, c, h] = oklch(&p);
    assert!((l - 0.5).abs() < 0.01, "the typed lightness stays: {l}");
    assert!((c - 0.08).abs() < 0.015, "the click sets the chroma: {c}");
    assert!((h - h0).abs() < 1.0, "the hue stays: {h0} → {h}");
}

/// A text that leaves a white as it was, committed by a click on the square in one frame, keeps the
/// hue the white was picked from: the click's saturation brings that hue back.
#[test]
fn a_click_that_commits_the_same_white_keeps_the_remembered_hue() {
    let mut app = app();
    let rect = selected(&app)[0];
    app.run("paint.setFill", json!({"color": "#00ff00"})).unwrap();
    let mut p = panel(app);
    let corner = p.node("Saturation and brightness").rect.left_top();
    quick_click(&mut p, corner);
    assert_eq!(rgb8(&fill_color(&p.app(), rect)), [255, 255, 255], "the square picks white");
    type_pending(&mut p, "#ffffff");
    let at = square_top_right(&p);
    quick_click(&mut p, at);
    let [r, g, b] = rgb8(&fill_color(&p.app(), rect));
    assert!(g > 0xf0 && r < 0x10 && b < 0x10, "the green comes back: {r:02x}{g:02x}{b:02x}");
}

/// A press on the opacity slider in one frame that takes the focus from a field holding hex with an
/// alpha applies the typed color and keeps the opacity the press set, not the typed alpha.
#[test]
fn an_opacity_click_in_one_frame_keeps_its_opacity_over_a_typed_alpha() {
    let mut app = app();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    app.run("ui.colorWheel", json!({"opacity": true})).unwrap();
    let mut p = panel(app);
    type_pending(&mut p, "#33669900");
    let rail = opacity_slider(&p);
    quick_click(&mut p, rail.rect.center());
    let (hex, opacity) = first_stop(&p);
    assert_eq!(hex, "#336699", "the text applies");
    assert!(opacity > 0.2 && opacity < 0.8, "the press sets the opacity: {opacity}");
}

/// A press on the opacity slider that takes the focus from an edited field applies the text and
/// keeps the opacity the press set: a gradient stop takes both.
#[test]
fn an_opacity_press_after_typing_keeps_both() {
    let mut app = app();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    app.run("ui.colorWheel", json!({"opacity": true})).unwrap();
    let mut p = panel(app);
    type_pending(&mut p, "#336699");
    let rail = opacity_slider(&p);
    p.fx.click_at(rail.rect.left_center() + vec2(rail.rect.width() * 0.25, 0.0)).idle(2);
    let (hex, opacity) = first_stop(&p);
    assert_eq!(hex, "#336699", "the text applies");
    assert!(opacity < 0.9, "the press sets the opacity: {opacity}");
}

/// The same for a solid fill: the rectangle takes the typed color and the opacity the press set.
#[test]
fn an_opacity_press_after_typing_keeps_both_on_a_solid_fill() {
    let mut app = app();
    let rect = selected(&app)[0];
    app.run("ui.colorWheel", json!({"opacity": true})).unwrap();
    let mut p = panel(app);
    type_pending(&mut p, "#336699");
    let rail = opacity_slider(&p);
    p.fx.click_at(rail.rect.left_center() + vec2(rail.rect.width() * 0.25, 0.0)).idle(2);
    assert_eq!(rgb8(&fill_color(&p.app(), rect)), [0x33, 0x66, 0x99], "the text applies");
    let fill = p.app().session.proxy_opacity(false);
    assert!(fill.is_some_and(|o| o < 0.9), "the press sets the opacity: {fill:?}");
}

/// A text typed for one rectangle is dropped when the selection changes to another showing the same
/// hex: Enter then changes neither of them.
#[test]
fn a_text_typed_for_one_object_does_not_go_to_another_with_the_same_hex() {
    let mut app = app();
    let a = selected(&app)[0];
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let b = selected(&app)[0];
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let before = undo_count(&app);
    let mut p = panel(app);
    type_pending(&mut p, "#33669980");
    p.run("select.set", json!({"ids": [b]}));
    p.fx.key(Key::Enter, Modifiers::NONE).settle();
    for id in [a, b] {
        assert_eq!(rgb8(&fill_color(&p.app(), id)), [255, 0, 0]);
    }
    assert_eq!(p.app().session.proxy_opacity(false), Some(1.0));
    assert_eq!(undo_count(&p.app()), before, "nothing was applied");
}

/// A text typed for one object of a dynamic symbol instance is dropped when another object of the
/// same instance showing the same hex is selected: Enter then changes neither of them.
#[test]
fn a_text_typed_for_one_object_of_a_symbol_instance_does_not_go_to_another() {
    let mut app = app();
    let a = selected(&app)[0];
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let b = selected(&app)[0];
    let inst = app.run("symbol.new", json!({"name": "Pair", "ids": [a.0, b.0], "type": "dynamic"})).unwrap()["id"].as_u64().unwrap();
    app.run("symbol.instance.selectPart", json!({"id": inst, "path": [0]})).unwrap();
    let before = undo_count(&app);
    let mut p = panel(app);
    type_pending(&mut p, "#336699");
    p.run("symbol.instance.selectPart", json!({"id": inst, "path": [1]}));
    p.fx.key(Key::Enter, Modifiers::NONE).settle();
    let Paint::Solid { color, .. } = crate::panels::current_paints(&p.app()).0 else { panic!("a solid fill") };
    assert_eq!(rgb8(&color), [255, 0, 0], "the second object keeps its color");
    assert_eq!(undo_count(&p.app()), before, "nothing was applied");
}

/// A text with a character `color.parse` does not read as whitespace (a no-break space) is refused,
/// as `color.parse` refuses it, and changes nothing.
#[test]
fn a_no_break_space_in_the_text_is_refused() {
    let app = app();
    let doc = app.session.doc().unwrap().doc.clone();
    let mut p = panel(app);
    type_in(&mut p, "\u{a0}#336699");
    assert_eq!(*p.app().session.doc().unwrap().doc, *doc);
    assert!(!p.app().ui.status.is_empty(), "the status bar says why");
    assert!(p.app_mut().run("color.parse", json!({"text": "\u{a0}#336699"})).is_err());
}

/// A selected gradient stop takes the color and the alpha in one step.
#[test]
fn a_gradient_stop_takes_the_color_and_the_alpha() {
    let mut app = app();
    app.run("paint.setFill", json!({"gradient": {}})).unwrap();
    let mut p = panel(app);
    type_in(&mut p, "#33669980");
    let (hex, opacity) = first_stop(&p);
    assert_eq!(hex, "#336699");
    assert!((opacity - 0.502).abs() < 1e-4, "{opacity}");
}

/// A selected freeform point takes the color and the alpha.
#[test]
fn a_freeform_point_takes_the_color_and_the_alpha() {
    let mut app = app();
    app.run("paint.setFill", json!({"gradient": {"kind": "freeform"}})).unwrap();
    let mut p = panel(app);
    type_in(&mut p, "#33669980");
    let Paint::Gradient(g) = crate::panels::current_paints(&p.app()).0 else { panic!("still a gradient") };
    let f = super::gradient::shown_points(&g);
    assert_eq!(f.points[0].color.to_hex(), "#336699");
    assert!((f.points[0].opacity - 0.502).abs() < 1e-4, "{}", f.points[0].opacity);
}

/// Text `color.parse` refuses changes nothing and shows its error in the status bar.
#[test]
fn refused_text_changes_nothing_and_says_why() {
    let app = app();
    let doc = app.session.doc().unwrap().doc.clone();
    let mut p = panel(app);
    for bad in ["#12345", "rgb(1, 2%, 3)", "not a color", "cmyk(0 0 0 0)"] {
        type_in(&mut p, bad);
        assert_eq!(*p.app().session.doc().unwrap().doc, *doc, "{bad}");
        assert!(p.app().ui.status.contains("must be a hex color"), "{bad}: {}", p.app().ui.status);
    }
    assert_eq!(shown(&p), "FF0000", "the field shows the color again");
    p.app_mut().ui.status.clear();
    let t = field(&p).expect("the hex field");
    p.fx.click(&t).key(Key::A, Modifiers::COMMAND).key(Key::Backspace, Modifiers::NONE).key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*p.app().session.doc().unwrap().doc, *doc, "an emptied field");
    assert!(p.app().ui.status.contains("must be a hex color"), "an emptied field: {}", p.app().ui.status);
}
