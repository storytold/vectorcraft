//! The Color Wheel panel's Show Contrast row drawn headless: the ratio and the AA and AAA results,
//! the color behind the selection, a pinned background, a drag on the wheel, and `ui.colorWheel`.

use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use vectorcraft_color::wcag;
use vectorcraft_engine::Session;

use super::color_wheel::show;
use super::pstate;
use crate::VectorcraftApp;

/// An app with a selected rectangle filled with `fill` and the contrast row shown.
fn app(fill: &str) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 40, "y": 40, "width": 50, "height": 50})).unwrap();
    app.run("paint.setFill", json!({"color": fill})).unwrap();
    app.run("ui.colorWheel", json!({"contrast": true})).unwrap();
    app
}

fn harness(app: VectorcraftApp) -> Harness<'static, VectorcraftApp> {
    let mut h = Harness::builder().with_size(vec2(260.0, 320.0)).build_ui_state(|ui, app: &mut VectorcraftApp| show(app, ui), app);
    h.run_steps(2);
    h
}

fn shows(h: &Harness<'static, VectorcraftApp>, text: &str) -> bool {
    h.query_by_label(text).is_some()
}

fn press(h: &mut Harness<'static, VectorcraftApp>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerMoved(pos));
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

#[test]
fn the_row_cuts_the_ratio_off_and_fails_aa_just_under_it() {
    let mut app = app("#fdc96e");
    app.run("ui.colorWheel", json!({"contrastBackground": "#355f78"})).unwrap();
    let h = harness(app);
    assert!(shows(&h, "4.49:1"), "4.4963:1 is cut off, not rounded up to 4.50:1");
    assert!(shows(&h, "AA Fail") && shows(&h, "AAA Fail"));
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

/// An answer kept under other color settings (here a profiles generation not yet reached) is asked
/// again: Edit › Color Settings changes how a color displays without a document revision.
#[test]
fn an_answer_kept_under_other_color_settings_is_asked_again() {
    type Kept = Option<(super::color_wheel::RatioKey, Option<Value>)>;
    let mut app = app("#000000");
    app.run("ui.colorWheel", json!({"contrastBackground": "#ffffff"})).unwrap();
    let mut h = harness(app);
    assert!(shows(&h, "21.00:1"));
    let (mut key, answer) = pstate::<Kept>(&h.ctx, "color-wheel-ratio").expect("the row keeps its answer");
    let mut stale = answer.expect("an answer");
    stale["ratio"] = json!(1.0);
    key.10.1 = u64::MAX;
    super::set_pstate(&h.ctx, "color-wheel-ratio", Some((key, Some(stale))));
    h.run_steps(2);
    assert!(shows(&h, "21.00:1"), "the answer is asked again under the current settings");
}

#[test]
fn the_row_follows_the_appearance_of_black() {
    let mut app = app("#000000");
    app.run("paint.setFill", json!({"color": {"c": 0, "m": 0, "y": 0, "k": 100}})).unwrap();
    app.run("ui.colorWheel", json!({"contrastBackground": "#ffffff"})).unwrap();
    app.run("prefs.set", json!({"key": "blackOnScreen", "value": "accurate"})).unwrap();
    let mut h = harness(app);
    let text = |h: &mut Harness<'static, VectorcraftApp>| {
        let r = h.state_mut().session.execute("color.contrast", &json!({"against": "#ffffff"})).unwrap();
        wcag::ratio_text(r["ratio"].as_f64().unwrap())
    };
    let accurate = text(&mut h);
    assert!(shows(&h, &accurate), "{accurate}");
    h.state_mut().run("prefs.set", json!({"key": "blackOnScreen", "value": "rich"})).unwrap();
    h.run_steps(2);
    let rich = text(&mut h);
    assert_ne!(accurate, rich, "rich black draws the black darker");
    assert!(shows(&h, &rich), "the row shows {rich}, not {accurate}");
}

#[test]
fn the_category_changes_the_results_and_graphics_have_no_aaa() {
    let mut app = app("#fdc96e");
    app.run("ui.colorWheel", json!({"contrastBackground": "#355f78", "contrastFor": "largeText"})).unwrap();
    let mut h = harness(app);
    assert!(shows(&h, "AA Pass") && shows(&h, "AAA Fail"));
    h.state_mut().run("ui.colorWheel", json!({"contrastFor": "graphics"})).unwrap();
    h.run_steps(2);
    assert!(shows(&h, "AA Pass") && !shows(&h, "AAA Fail") && !shows(&h, "AAA Pass"));
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
    let h = harness(app);
    let want = wcag::ratio_text(wcag::contrast_ratio([0.0; 3], [1.0, 1.0, 0.0]));
    assert!(shows(&h, &want), "black on the locked yellow: {want}");
}

#[test]
fn nothing_selected_and_nothing_pinned_shows_no_ratio() {
    let mut app = app("#000000");
    app.run("select.none", json!({})).unwrap();
    let mut h = harness(app);
    assert!(shows(&h, "—"));
    // A pinned background needs no selection: the proxy shows the defaults for new art.
    h.state_mut().run("ui.colorWheel", json!({"contrastBackground": "#ffffff"})).unwrap();
    h.run_steps(2);
    assert!(!shows(&h, "—"));
}

/// With nothing selected the row measures the default fill for new art, and follows a change of
/// it, which leaves the document's revision as it is.
#[test]
fn the_row_follows_the_default_fill_with_nothing_selected() {
    let mut app = app("#000000");
    app.run("select.none", json!({})).unwrap();
    app.run("paint.setFill", json!({"color": "#000000"})).unwrap();
    app.run("ui.colorWheel", json!({"contrastBackground": "#ffffff"})).unwrap();
    let mut h = harness(app);
    assert!(shows(&h, "21.00:1"));
    h.state_mut().run("paint.setFill", json!({"color": "#ffffff"})).unwrap();
    h.run_steps(2);
    assert!(shows(&h, "1.00:1"), "white on the pinned white");
}

/// While a drag on the wheel is in progress the color behind the selection is not read again,
/// although every preview raises the document's revision.
#[test]
fn a_drag_on_the_wheel_keeps_the_background() {
    type Under = (vectorcraft_color::cms::ColorSettings, u64);
    type Kept = Option<((u64, u64, Vec<vectorcraft_doc::NodeId>, Option<Vec<u32>>, String, Under), Option<String>)>;
    let mut h = harness(app("#336699"));
    let read = |h: &Harness<'static, VectorcraftApp>| pstate::<Kept>(&h.ctx, "color-wheel-behind").map(|(k, _)| k.1);
    let before = read(&h).expect("the background was read");
    let sq = h.get_by_label("Saturation and brightness").rect();
    press(&mut h, sq.center(), true);
    for i in 1..4 {
        h.event(Event::PointerMoved(sq.center() + vec2(8.0 * i as f32, 0.0)));
        h.run_steps(1);
    }
    assert!(h.state().session.in_interaction(), "the drag previews on an interaction");
    assert!(h.state().session.active().unwrap().revision > before, "the previews raised the revision");
    assert_eq!(read(&h), Some(before), "the background was not read again during the drag");
    press(&mut h, sq.center() + vec2(24.0, 0.0), false);
    h.run_steps(2);
    assert!(read(&h).unwrap() > before, "after the drag it is read for the new revision");
}

/// An interaction keeps the background only for the same document and selection: another
/// selection is read again.
#[test]
fn an_interaction_reads_the_background_again_for_another_selection() {
    type Under = (vectorcraft_color::cms::ColorSettings, u64);
    type Kept = Option<((u64, u64, Vec<vectorcraft_doc::NodeId>, Option<Vec<u32>>, String, Under), Option<String>)>;
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
    let mut h = harness(app);
    let hex = |h: &Harness<'static, VectorcraftApp>| pstate::<Kept>(&h.ctx, "color-wheel-behind").and_then(|(_, hex)| hex);
    assert_eq!(hex(&h).as_deref(), Some("#ffff00"));
    h.state_mut().run("select.set", json!({"ids": [b]})).unwrap();
    h.state_mut().session.begin_interaction("test").unwrap();
    h.run_steps(2);
    assert_eq!(hex(&h).as_deref(), Some("#ffffff"), "the other selection's background is read during the interaction");
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
    let mut h = harness(app);
    let want = h.state_mut().session.execute("color.contrast", &json!({})).unwrap();
    let against = want["against"].clone();
    let given = h.state_mut().session.execute("color.contrast", &json!({"against": against})).unwrap();
    let (want, given) = (wcag::ratio_text(want["ratio"].as_f64().unwrap()), wcag::ratio_text(given["ratio"].as_f64().unwrap()));
    assert_ne!(want, given, "the group's other art makes the two answers differ");
    assert!(shows(&h, &want), "the row shows {want}, not {given}");
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
