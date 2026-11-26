//! The Color Wheel panel's Show Contrast row drawn headless through the `Fixture`: the ratio and
//! the AA and AAA results, the color behind the selection, a pinned background, a drag on the
//! wheel, the panel menu and `ui.colorWheel`.

use craft::ui::runtime::access::Role;
use serde_json::{Value, json};
use vectorcraft_color::wcag;
use vectorcraft_engine::Session;

use super::color_wheel::{BehindKey, RatioKey};
use super::tests_color_wheel::Panel;
use crate::VectorcraftApp;
use crate::i18n::LangExt as _;

/// An app with a selected rectangle filled with `fill` and the contrast row shown.
fn app(fill: &str) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": fill})).unwrap();
    app.run("ui.colorWheel", json!({"contrast": true})).unwrap();
    app
}

fn shows(p: &Panel, text: &str) -> bool {
    p.fx.by_label(text).is_some()
}

#[test]
fn the_row_cuts_the_ratio_off_and_fails_aa_just_under_it() {
    let mut app = app("#fdc96e");
    app.run("ui.colorWheel", json!({"contrastBackground": "#355f78"})).unwrap();
    let p = Panel::new(app);
    assert!(shows(&p, "4.49:1"), "4.4963:1 is cut off, not rounded up to 4.50:1");
    assert!(shows(&p, "AA Fail") && shows(&p, "AAA Fail"));
}

#[test]
fn the_contrast_for_menu_shows_the_categories_in_the_interface_language() {
    use vectorcraft_color::wcag::Category;
    crate::i18n::set_current(crate::i18n::Lang::from_code("de").expect("de registered"));
    let names: Vec<&str> = Category::ALL.iter().map(|c| super::color_wheel::category_name(*c)).collect();
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(names, ["Normaler Text", "Großer Text", crate::i18n::tr(crate::i18n::Lang::from_code("de").unwrap(), "Graphics")]);
    assert_ne!(names[0], Category::NormalText.label());
}

/// The panel menu's Show Contrast, Contrast For and Contrast Background rows set the settings, and
/// Current Color pins the color the panel edits.
#[test]
fn the_panel_menu_sets_the_contrast_settings() {
    let mut app = app("#336699");
    app.run("ui.colorWheel", json!({"contrast": false})).unwrap();
    let run = |app: &mut VectorcraftApp, row: &str| super::color_wheel::run(app, &format!("ui.colorWheel.{row}"), &json!({})).unwrap().unwrap();
    assert_eq!(run(&mut app, "contrast")["contrast"], true);
    assert_eq!(run(&mut app, "contrastFor.graphics")["contrastFor"], "graphics");
    run(&mut app, "contrastBackground.current");
    assert_eq!(app.ui.color_wheel.contrast_background.map(|c| c.to_hex()).as_deref(), Some("#336699"));
    assert_eq!(run(&mut app, "contrastBackground.below")["contrastBackground"], Value::Null);
    let mut states = craft::ui::runtime::commands::CommandStates::new();
    app.run("paint.setFill", json!({"none": true})).unwrap();
    super::color_wheel::menu_states(&app, &mut states);
    let current = states.get(&craft::keymap::CommandRef::new("ui.colorWheel.contrastBackground.current")).expect("a state");
    assert!(!current.is_enabled(), "Current Color needs a color to pin");
    let e = super::color_wheel::run(&mut app, "ui.colorWheel.contrastBackground.current", &json!({})).unwrap().unwrap_err();
    assert_eq!(e, "Current Color needs a solid color in the active proxy");
    assert_eq!(app.ui.color_wheel.contrast_background, None, "the error pins nothing");
}

/// An answer kept under other color settings (here a profiles generation not yet reached) is asked
/// again: Edit › Color Settings changes how a color displays without a document revision.
#[test]
fn an_answer_kept_under_other_color_settings_is_asked_again() {
    let mut app = app("#000000");
    app.run("ui.colorWheel", json!({"contrastBackground": "#ffffff"})).unwrap();
    let mut p = Panel::new(app);
    assert!(shows(&p, "21.00:1"));
    let (mut key, answer) = p.app().ui.color_wheel_panel.ratio.clone().expect("the row keeps its answer");
    let mut stale = answer.expect("an answer");
    stale["ratio"] = json!(1.0);
    key.under.1 = u64::MAX;
    p.app_mut().ui.color_wheel_panel.ratio = Some((key, Some(stale)));
    p.fx.idle(2);
    assert!(shows(&p, "21.00:1"), "the answer is asked again under the current settings");
}

#[test]
fn the_row_follows_the_appearance_of_black() {
    let mut app = app("#000000");
    app.run("paint.setFill", json!({"color": {"c": 0, "m": 0, "y": 0, "k": 100}})).unwrap();
    app.run("ui.colorWheel", json!({"contrastBackground": "#ffffff"})).unwrap();
    app.run("prefs.set", json!({"key": "blackOnScreen", "value": "accurate"})).unwrap();
    let mut p = Panel::new(app);
    let text = |p: &Panel| {
        let r = p.app_mut().session.execute("color.contrast", &json!({"against": "#ffffff"})).unwrap();
        wcag::ratio_text(r["ratio"].as_f64().unwrap())
    };
    let accurate = text(&p);
    assert!(shows(&p, &accurate), "{accurate}");
    p.run("prefs.set", json!({"key": "blackOnScreen", "value": "rich"}));
    let rich = text(&p);
    assert_ne!(accurate, rich, "rich black draws the black darker");
    assert!(shows(&p, &rich), "the row shows {rich}, not {accurate}");
}

#[test]
fn the_category_changes_the_results_and_graphics_have_no_aaa() {
    let mut app = app("#fdc96e");
    app.run("ui.colorWheel", json!({"contrastBackground": "#355f78", "contrastFor": "largeText"})).unwrap();
    let mut p = Panel::new(app);
    assert!(shows(&p, "AA Pass") && shows(&p, "AAA Fail"));
    p.run("ui.colorWheel", json!({"contrastFor": "graphics"}));
    assert!(shows(&p, "AA Pass") && !shows(&p, "AAA Fail") && !shows(&p, "AAA Pass"));
}

#[test]
fn the_background_is_the_art_behind_the_selection() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 180, "height": 180})).unwrap();
    app.run("paint.setFill", json!({"color": "#ffff00"})).unwrap();
    app.run("object.lock", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": "#000000"})).unwrap();
    app.run("ui.colorWheel", json!({"contrast": true})).unwrap();
    let p = Panel::new(app);
    let want = wcag::ratio_text(wcag::contrast_ratio([0.0; 3], [1.0, 1.0, 0.0]));
    assert!(shows(&p, &want), "black on the locked yellow: {want}");
    // Screen readers find the chip by its name, with the background's hex value as its value.
    let chip = p.node("Background");
    assert_eq!(p.fx.tree().role_of(chip.node), Some(Role::COLOR_WELL));
    assert_eq!(p.value("Background").as_deref(), Some("#ffff00"));
}

#[test]
fn nothing_selected_and_nothing_pinned_shows_no_ratio() {
    let mut app = app("#000000");
    app.run("select.none", json!({})).unwrap();
    let mut p = Panel::new(app);
    assert!(shows(&p, "—"));
    // A pinned background needs no selection: the proxy shows the defaults for new art.
    p.run("ui.colorWheel", json!({"contrastBackground": "#ffffff"}));
    assert!(!shows(&p, "—"));
}

/// With nothing selected the row measures the default fill for new art, and follows a change of
/// it, which leaves the document's revision as it is.
#[test]
fn the_row_follows_the_default_fill_with_nothing_selected() {
    let mut app = app("#000000");
    app.run("select.none", json!({})).unwrap();
    app.run("paint.setFill", json!({"color": "#000000"})).unwrap();
    app.run("ui.colorWheel", json!({"contrastBackground": "#ffffff"})).unwrap();
    let mut p = Panel::new(app);
    assert!(shows(&p, "21.00:1"));
    p.run("paint.setFill", json!({"color": "#ffffff"}));
    assert!(shows(&p, "1.00:1"), "white on the pinned white");
}

/// The background read on its own, with what it was read for.
fn behind(p: &Panel) -> Option<(BehindKey, Option<String>)> {
    p.app().ui.color_wheel_panel.behind.clone()
}

/// The background in the row's kept answer.
fn measured(p: &Panel) -> Option<String> {
    let kept: Option<(RatioKey, Option<Value>)> = p.app().ui.color_wheel_panel.ratio.clone();
    kept.and_then(|(_, r)| r).and_then(|r| r["against"].as_str().map(str::to_string))
}

/// During a drag on the wheel every preview raises the document's revision, and the row asks again
/// for the ratio, whose answer gives the chip: the background is not read on its own as well. The
/// drag is an interaction whose previews set the fill, as the Show Document Colors row's drag test
/// runs it.
#[test]
fn a_drag_on_the_wheel_reads_the_background_only_with_the_ratio() {
    let mut p = Panel::new(app("#336699"));
    assert_eq!(measured(&p).as_deref(), Some("#ffffff"));
    assert!(behind(&p).is_none(), "the answer gave the background");
    p.app_mut().session.begin_interaction("test").unwrap();
    for fill in ["#3a6a9a", "#406f9f", "#4675a5"] {
        p.run("paint.setFill", json!({"color": fill}));
    }
    assert!(p.app().session.in_interaction(), "the drag previews on an interaction");
    p.app_mut().session.commit_interaction().unwrap();
    p.fx.idle(2);
    assert!(behind(&p).is_none(), "the background was not read on its own during or after the drag");
    assert_eq!(measured(&p).as_deref(), Some("#ffffff"));
}

/// With no color to measure the row has no ratio, and the chip still shows the background, read on
/// its own, and read again for another selection during an interaction.
#[test]
fn a_paint_with_no_color_still_shows_the_background() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 200})).unwrap();
    app.run("paint.setFill", json!({"color": "#ffff00"})).unwrap();
    app.run("object.lock", json!({})).unwrap();
    let id = |v: Value| v["id"].as_u64().unwrap();
    let b = id(app.run("shape.rectangle", json!({"x": 130, "y": 20, "width": 40, "height": 40})).unwrap());
    app.run("paint.setFill", json!({"none": true})).unwrap();
    app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"none": true})).unwrap();
    app.run("ui.colorWheel", json!({"contrast": true})).unwrap();
    let mut p = Panel::new(app);
    assert!(shows(&p, "—"), "a fill of None has no ratio");
    let hex = |p: &Panel| behind(p).and_then(|(_, hex)| hex);
    assert_eq!(hex(&p).as_deref(), Some("#ffff00"));
    assert_eq!(p.value("Background").as_deref(), Some("#ffff00"), "the chip shows it");
    p.app_mut().run("select.set", json!({"ids": [b]})).unwrap();
    p.app_mut().session.begin_interaction("test").unwrap();
    p.fx.idle(2);
    assert_eq!(hex(&p).as_deref(), Some("#ffffff"), "the other selection's background is read during the interaction");
}

/// With no color to measure, a move during an interaction reads the background again at the
/// selection's new place.
#[test]
fn a_move_during_an_interaction_reads_the_background_again() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 200})).unwrap();
    app.run("paint.setFill", json!({"color": "#ffff00"})).unwrap();
    app.run("object.lock", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"none": true})).unwrap();
    app.run("ui.colorWheel", json!({"contrast": true})).unwrap();
    let mut p = Panel::new(app);
    let hex = |p: &Panel| behind(p).and_then(|(_, hex)| hex);
    p.app_mut().session.begin_interaction("test").unwrap();
    p.fx.idle(2);
    assert_eq!(hex(&p).as_deref(), Some("#ffff00"));
    p.app_mut().run("object.move", json!({"dx": 110, "dy": 0})).unwrap();
    assert!(p.app().session.in_interaction(), "the move is part of the interaction");
    p.fx.idle(2);
    assert_eq!(hex(&p).as_deref(), Some("#ffffff"), "the background at the new place");
}

/// An interaction keeps no answer for another selection: the row asks again, and the chip shows
/// the other selection's background.
#[test]
fn an_interaction_reads_the_background_again_for_another_selection() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 200})).unwrap();
    app.run("paint.setFill", json!({"color": "#ffff00"})).unwrap();
    app.run("object.lock", json!({})).unwrap();
    let id = |v: Value| v["id"].as_u64().unwrap();
    let b = id(app.run("shape.rectangle", json!({"x": 130, "y": 20, "width": 40, "height": 40})).unwrap());
    app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap();
    app.run("paint.setFill", json!({"color": "#000000"})).unwrap();
    app.run("ui.colorWheel", json!({"contrast": true})).unwrap();
    let mut p = Panel::new(app);
    assert_eq!(measured(&p).as_deref(), Some("#ffff00"));
    p.app_mut().run("select.set", json!({"ids": [b]})).unwrap();
    p.app_mut().session.begin_interaction("test").unwrap();
    p.fx.idle(2);
    assert_eq!(measured(&p).as_deref(), Some("#ffffff"), "the other selection's background is read during the interaction");
    assert_eq!(p.value("Background").as_deref(), Some("#ffffff"), "the chip shows it");
}

/// Without a pin the row asks for the color behind the selection, so a translucent group holding
/// other art behind the object counts as `color.contrast {}` counts it.
#[test]
fn the_row_answers_as_color_contrast_does_inside_a_translucent_group() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 200})).unwrap();
    app.run("paint.setFill", json!({"color": "#ffff00"})).unwrap();
    app.run("object.lock", json!({})).unwrap();
    let id = |v: Value| v["id"].as_u64().unwrap();
    let back = id(app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 120, "height": 120})).unwrap());
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    let front = id(app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 50, "height": 50})).unwrap());
    app.run("paint.setFill", json!({"color": "#0000ff"})).unwrap();
    let g = id(app.run("object.group", json!({"ids": [back, front]})).unwrap());
    app.run("select.set", json!({"ids": [g]})).unwrap();
    app.run("transparency.set", json!({"opacity": 50})).unwrap();
    app.run("select.set", json!({"ids": [front]})).unwrap();
    app.run("ui.colorWheel", json!({"contrast": true})).unwrap();
    let p = Panel::new(app);
    let want = p.app_mut().session.execute("color.contrast", &json!({})).unwrap();
    let against = want["against"].clone();
    let given = p.app_mut().session.execute("color.contrast", &json!({"against": against})).unwrap();
    let (want, given) = (wcag::ratio_text(want["ratio"].as_f64().unwrap()), wcag::ratio_text(given["ratio"].as_f64().unwrap()));
    assert_ne!(want, given, "the group's other art makes the two answers differ");
    assert!(shows(&p, &want), "the row shows {want}, not {given}");
}

/// Clicks the point of the selected gradient mesh at (`x`, `y`) with the Mesh tool, which picks it
/// alone.
fn pick_point(s: &mut Session, x: f64, y: f64) {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let v = vectorcraft_engine::ViewInfo::default();
    s.select_tool("mesh", v).unwrap();
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
    }
}

/// The row asks again when the Mesh tool picks another point, which changes no document revision:
/// here a black point at 50 % after an opaque black one.
#[test]
fn the_row_follows_the_picked_mesh_points() {
    let mut app = app("#000000");
    app.run("object.mesh.create", json!({"rows": 1, "cols": 1})).unwrap();
    app.run("ui.colorWheel", json!({"contrastBackground": "#ffffff"})).unwrap();
    pick_point(&mut app.session, 40.0, 40.0);
    app.run("transparency.set", json!({"opacity": 50})).unwrap();
    pick_point(&mut app.session, 90.0, 40.0);
    let mut p = Panel::new(app);
    assert!(shows(&p, "21.00:1"), "the opaque top-right point");
    let revision = |p: &Panel| p.app().session.active().map(|d| d.revision);
    let before = revision(&p);
    pick_point(&mut p.app_mut().session, 40.0, 40.0);
    assert_eq!(revision(&p), before, "a pick is no document change");
    p.fx.idle(2);
    let gray = wcag::ratio_text(wcag::contrast_ratio([0.5; 3], [1.0; 3]));
    assert!(shows(&p, &gray), "the top-left point at 50 % shows {gray}");
}

#[test]
fn ui_color_wheel_sets_the_keys_given_and_refuses_bad_values() {
    let mut app = app("#000000");
    let r = app.run("ui.colorWheel", json!({"contrastFor": "graphics", "contrastBackground": "#ff0000"})).unwrap();
    assert_eq!(r["contrast"], true, "keys left out keep their values");
    assert_eq!(r["contrastFor"], "graphics");
    assert_eq!(app.ui.color_wheel.contrast_background.map(|c| c.to_hex()).as_deref(), Some("#ff0000"));
    let before = app.ui.color_wheel.clone();
    for (p, e) in [
        (json!({"contrast": "yes"}), "`contrast` must be true or false"),
        (json!({"contrastFor": "huge"}), "`contrastFor` must be one of normalText, largeText, graphics"),
        (json!({"contrastFor": 3}), "`contrastFor` must be one of normalText, largeText, graphics"),
        (json!({"contrastBackground": "nope"}), "`contrastBackground` must be a color or null"),
        (json!({"contrast": false, "contrastBackground": 5}), "`contrastBackground` must be a color or null"),
    ] {
        assert_eq!(app.run("ui.colorWheel", p.clone()).unwrap_err(), e, "{p}");
    }
    assert_eq!(app.ui.color_wheel, before, "an error changes nothing");
    let r = app.run("ui.colorWheel", json!({"contrastBackground": null})).unwrap();
    assert_eq!(r["contrastBackground"], Value::Null, "null goes back to the color behind the selection");
    assert_eq!(app.run("ui.colorWheel", json!({})).unwrap(), r, "{{}} reads the settings");
}

#[test]
fn the_settings_are_saved_and_each_key_reads_on_its_own() {
    let mut app = app("#000000");
    app.run("ui.colorWheel", json!({"contrastFor": "largeText", "contrastBackground": {"c": 0, "m": 1, "y": 0, "k": 0}})).unwrap();
    let saved: crate::UiState = serde_json::from_value(serde_json::to_value(&app.ui).unwrap()).unwrap();
    assert_eq!(saved.color_wheel, app.ui.color_wheel);
    let ui: crate::UiState =
        serde_json::from_value(json!({"color_wheel": {"contrast": true, "contrastFor": "huge", "contrastBackground": 5}, "place_link": false}))
            .unwrap();
    assert!(ui.color_wheel.contrast, "a key that reads keeps its value");
    assert_eq!((ui.color_wheel.contrast_for, ui.color_wheel.contrast_background), (wcag::Category::NormalText, None));
    assert!(!ui.place_link, "the other preferences still load");
    let ui: crate::UiState = serde_json::from_value(json!({"color_wheel": 3, "place_link": false})).unwrap();
    assert_eq!((ui.color_wheel, ui.place_link), (Default::default(), false));
}

#[test]
fn ui_color_wheel_answers_junk_params_without_a_panic() {
    let doc = crate::menus::UI_COMMANDS.iter().find(|c| c.0 == "ui.colorWheel").expect("registered").3;
    let mut app = app("#336699");
    for p in vectorcraft_testkit::strategies::junk_params(doc, "/nonexistent").into_iter().chain(vectorcraft_testkit::strategies::junk_values()) {
        let _ = app.run("ui.colorWheel", p);
    }
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    assert_eq!(app.run("ui.colorWheel", json!({"contrast": true})).unwrap()["contrast"], true);
}
