//! A plug-in's parameters in its dialog as its manifest presents them (docs/plugins.md ›
//! Parameters): the rows the manifest makes, help on the labels and a button to its page, and
//! parameters shown only while their `visibleWhen` holds. The plug-in is removed after each test:
//! plug-ins are process-wide.

use egui_kittest::kittest::Queryable;
use serde_json::{Map, Value, json};
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat as tpl;

use super::tests_export::fields;
use super::tests_plugin_export::top;
use crate::VectorcraftApp;

/// A filter whose parameters use every presentation key.
fn manifest(id: &str) -> String {
    format!(
        r#"{{"id":"{id}","name":"Hatch Test","version":"1","kind":"filter","params":{{
        "outline":{{"type":"bool","default":true,"label":"Outline"}},
        "title":{{"type":"string","default":"Poster","maxLength":8,"label":"Title"}},
        "pattern":{{"type":"choice","options":["lines","crosshatch","dots"],"default":"lines","label":"Pattern",
                   "group":"Fill","help":"How the shape is filled."}},
        "spacing":{{"type":"number","min":0.1,"max":10,"default":0.25,"label":"Spacing","unit":"mm","group":"Fill",
                   "helpUrl":"https://example.org/params/spacing","visibleWhen":{{"pattern":["lines","crosshatch"]}}}},
        "angles":{{"type":"numberList","min":0,"max":360,"default":[45,135],"minItems":1,"maxItems":4,"label":"Angles",
                  "unit":"°","group":"Fill","visibleWhen":{{"pattern":["lines","crosshatch"]}}}},
        "width":{{"type":"number","min":0,"max":10,"default":0.5,"label":"Width","unit":"pt","group":"Outline",
                 "visibleWhen":{{"outline":[true]}}}}}}}}"#
    )
}

type Harness = egui_kittest::Harness<'static, VectorcraftApp>;

/// The dialog of plug-in `id` in a window running whole app frames.
fn harness(id: &str) -> Harness {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let wasm = wat::parse_str(tpl::module(&manifest(id), tpl::ECHO, "", 1)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 800.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                crate::testing::install(ui.ctx());
                return;
            }
            // As the hosts run a frame: the tick (which opens the links asked for since the last), then the window.
            app.tick(ui.ctx());
            app.ui(ui);
        },
        app,
    );
    h.run_steps(2);
    h.state_mut().run("plugin.dialog", json!({"id": id})).unwrap();
    h.run_steps(4);
    h
}

/// Sets `key` of the open dialog as `ui.dialog.set` does, then draws two frames.
fn set(h: &mut Harness, key: &str, value: Value) -> Result<(), String> {
    let (ctx, id) = (h.ctx.clone(), top(h.state()).ok_or("no dialog is open")?);
    let done = h.state().dialogs().ok_or("no dialog host")?.with(|host| host.set_json_field(&ctx, id, key, value));
    h.run_steps(2);
    done
}

fn remove(h: &mut Harness, id: &str) {
    h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();
}

#[test]
fn labels_units_sections_text_and_lists_come_from_the_manifest() {
    let id = "test.ui.fields.shown";
    let parsed = vectorcraft_plugins::manifest::Manifest::parse(manifest(id).as_bytes()).unwrap();
    let rows = super::form::plugin_params(&parsed, &Map::new());
    let at = |key: &str| rows.iter().position(|p| p.key == key).unwrap_or_else(|| panic!("no row for {key}"));
    let section = |label: &str| {
        rows.iter().position(|p| p.key.starts_with("section:") && p.label.as_str() == label).unwrap_or_else(|| panic!("no {label} section"))
    };
    // The parameters outside groups come first, then each group's section, its rows in manifest order.
    let (fill, outline) = (section("Fill"), section("Outline"));
    assert!(at("outline") < at("title") && at("title") < fill, "ungrouped rows lead");
    assert!(fill < at("pattern") && at("pattern") < at("spacing") && at("spacing") < at("angles") && at("angles") < outline);
    assert!(outline < at("width"));
    assert!(rows[at("spacing")].unit.is_some() && rows[at("width")].unit.is_some(), "a unit stays with its number");
    // A list of numbers is a row of its own, not folded into the one before it.
    assert_eq!(rows.iter().filter(|p| p.key == "angles").count(), 1);
}

#[test]
fn a_slider_widget_makes_a_whole_number_range_row_of_an_int_parameter() {
    use craft::ui::patterns::form_dialogs::ParamKind;
    let manifest = r#"{"id":"test.ui.fields.slider","name":"Slide","version":"1","kind":"filter","params":{
        "count":{"type":"int","min":1,"max":20,"default":4,"widget":"slider"},
        "plain":{"type":"int","min":1,"max":20,"default":4}}}"#;
    let parsed = vectorcraft_plugins::manifest::Manifest::parse(manifest.as_bytes()).unwrap();
    let rows = super::form::plugin_params(&parsed, &Map::new());
    let kind = |key: &str| &rows.iter().find(|p| p.key == key).unwrap_or_else(|| panic!("no row for {key}")).kind;
    assert!(matches!(kind("count"), ParamKind::Range { min, max, .. } if (*min, *max) == (1.0, 20.0)), "an int slider is a range row");
    assert!(matches!(kind("plain"), ParamKind::Int { .. }), "an int without the key stays a number field");
    // The slider's value reaches the plug-in as a whole number.
    assert_eq!(super::form::typed_number(7.0), json!(7));
}

#[test]
fn visible_when_shows_a_parameter_only_while_its_condition_holds() {
    let id = "test.ui.fields.visible";
    let mut h = harness(id);
    set(&mut h, "pattern", json!("dots")).unwrap();
    assert!(set(&mut h, "spacing", json!(1.0)).is_err() && set(&mut h, "angles", json!([10])).is_err(), "dots have no line spacing");
    set(&mut h, "outline", json!(false)).unwrap();
    assert!(set(&mut h, "width", json!(1.0)).is_err(), "without an outline, no width");
    set(&mut h, "pattern", json!("crosshatch")).unwrap();
    assert!(set(&mut h, "spacing", json!(1.0)).is_ok());
    // A hidden parameter still goes to the plug-in: the preview runs with every value.
    assert_eq!(fields(h.state())["width"], json!(0.5));
    remove(&mut h, id);
}

/// The URLs the last frame asked egui to open.
fn opened(h: &Harness) -> Vec<String> {
    // PLAN: assumed `Harness::output()` gives the last frame's `FullOutput`.
    h.output()
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::OpenUrl(open) => Some(open.url.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_help_button_opens_the_parameter_s_page() {
    let id = "test.ui.fields.help";
    let mut h = harness(id);
    // The window's Help menu has the button's name too: look inside the dialog.
    let press = |h: &mut Harness| {
        {
            let dialog = h.get_by_role(egui::accesskit::Role::Dialog);
            assert_eq!(dialog.query_all_by_label("Help").count(), 1, "only Spacing has a page");
            dialog.get_by_label("Help").click();
        }
        // The click asks for the page in one frame, and the next frame's tick opens it.
        h.step();
        h.step();
    };
    press(&mut h);
    assert_eq!(opened(&h), ["https://example.org/params/spacing"]);
    // Automation can't send the person's browser anywhere: the press while a control request is answered.
    h.state_mut().ui.answering_control = true;
    press(&mut h);
    h.state_mut().ui.answering_control = false;
    assert!(opened(&h).is_empty(), "nothing opened for automation");
    remove(&mut h, id);
}
