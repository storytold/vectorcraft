//! A plug-in's parameters in its dialog as its manifest presents them (docs/plugins.md ›
//! Parameters): labels, units after the values, help on the labels and a button to its page,
//! sections by group, text and lists of numbers, and parameters shown only while their
//! `visibleWhen` holds. The plug-in is removed after each test: plug-ins are process-wide.

use std::sync::{Arc, Mutex};

use egui::vec2;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat as tpl;

use super::*;

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

/// The dialog of plug-in `id` in a kittest harness; the web pages it opens go to `opened`.
fn harness(id: &str, opened: Arc<Mutex<Vec<String>>>) -> Harness {
    let services = crate::Services { open_url: Some(Box::new(move |u: &str| opened.lock().unwrap().push(u.to_string()))), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    let wasm = wat::parse_str(tpl::module(&manifest(id), tpl::ECHO, "", 1)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    let mut fonts = false;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 800.0)).build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut fonts, true) {
                crate::theme::install_fonts(ui.ctx());
                return;
            }
            show(app, ui.ctx());
        },
        app,
    );
    h.run_steps(4);
    h
}

type Harness = egui_kittest::Harness<'static, VectorcraftApp>;

/// The text label `text` (a checkbox is called by its row's label too).
fn label<'h>(h: &'h Harness, text: &'h str) -> Option<egui_kittest::Node<'h>> {
    h.query_by_role_and_label(egui::accesskit::Role::Label, text)
}

fn set(h: &mut Harness, key: &str, value: Value) {
    h.state_mut().ui.dialog.as_mut().expect("the dialog is open").fields.insert(key.into(), value);
    h.run_steps(2);
}

fn remove(h: &mut Harness, id: &str) {
    h.state_mut().run("plugin.remove", json!({"id": id})).unwrap();
}

#[test]
fn labels_units_sections_text_and_lists_come_from_the_manifest() {
    let id = "test.ui.fields.shown";
    let mut h = harness(id, Arc::default());
    for text in ["Outline:", "Title:", "Pattern:", "Spacing:", "Angles:", "Width:", "°"] {
        assert!(label(&h, text).is_some(), "{text} isn't shown");
    }
    for section in ["Fill", "Outline"] {
        assert!(h.query_by_label(section).is_some(), "the {section} section isn't shown");
    }
    for value in ["Poster", "0.25 mm", "45, 135", "0.5 pt"] {
        // A text field and the text inside it both carry the value.
        assert!(h.query_all_by_value(value).next().is_some(), "the value {value} isn't shown");
    }
    // The parameters outside groups come first, then each group's section.
    let top = |text: &str| label(&h, text).map(|n| n.rect().top()).unwrap_or_default();
    let (fill, outline) = (h.get_by_label("Fill").rect().top(), h.get_by_label("Outline").rect().top());
    assert!(top("Title:") < fill && fill < top("Spacing:") && top("Angles:") < outline && outline < top("Width:"));
    // The fields line up down the dialog, in sections and out of them.
    let left = |value: &str| h.query_all_by_value(value).next().map(|n| n.rect().left()).unwrap_or_default();
    let (title, spacing, width) = (left("Poster"), left("0.25 mm"), left("0.5 pt"));
    assert!((title - spacing).abs() < 0.5 && (spacing - width).abs() < 0.5, "fields at {title}, {spacing} and {width}");
    remove(&mut h, id);
}

#[test]
fn visible_when_shows_a_parameter_only_while_its_condition_holds() {
    let id = "test.ui.fields.visible";
    let mut h = harness(id, Arc::default());
    set(&mut h, "pattern", json!("dots"));
    assert!(label(&h, "Spacing:").is_none() && label(&h, "Angles:").is_none(), "dots have no line spacing");
    set(&mut h, "outline", json!(false));
    assert!(label(&h, "Width:").is_none() && h.query_by_label("Outline").is_none(), "without an outline, nor its section");
    set(&mut h, "pattern", json!("crosshatch"));
    assert!(label(&h, "Spacing:").is_some());
    // A hidden parameter still goes to the plug-in: the preview runs with every value.
    let fields = &h.state().ui.dialog.as_ref().unwrap().fields;
    assert_eq!(fields.get("width"), Some(&json!(0.5)));
    remove(&mut h, id);
}

#[test]
fn the_help_button_opens_the_parameter_s_page() {
    let id = "test.ui.fields.help";
    let opened: Arc<Mutex<Vec<String>>> = Arc::default();
    let mut h = harness(id, opened.clone());
    let buttons = h.query_all_by_label("Open this setting's help page").count();
    assert_eq!(buttons, 1, "only Spacing has a page");
    h.get_by_label("Open this setting's help page").click();
    h.run_steps(2);
    assert_eq!(*opened.lock().unwrap(), ["https://example.org/params/spacing"]);
    remove(&mut h, id);
}

/// `CRAFT_UI_PLUGIN_FIELDS_SHOT=<folder>` saves a picture of a dialog whose parameters use every
/// presentation key there, and the test plug-in beside it (`hatch-test-filter.wasm`) to install in
/// the app. Its parameter names are written as many plug-ins write them, in snake case, and it uses
/// only the types every host reads, so a host from before the keys installs it too and shows it as
/// it did.
#[test]
fn plugin_fields_picture() {
    let Some(dir) = std::env::var_os("CRAFT_UI_PLUGIN_FIELDS_SHOT") else { return };
    let dir = std::path::PathBuf::from(dir);
    let manifest = r#"{"id":"org.example.hatch","name":"Hatch Fill (test plug-in)","version":"1","kind":"filter",
        "description":"Fills the selected shapes with lines.","params":{
        "pattern":{"type":"choice","options":["lines","crosshatch","dots"],"default":"lines",
            "label":"Pattern","help":"How the shapes are filled."},
        "line_angle":{"type":"number","min":0,"max":360,"default":45,"label":"Angle","unit":"°","group":"Lines",
            "help":"The direction of the lines.","visibleWhen":{"pattern":["lines","crosshatch"]}},
        "line_spacing_mm":{"type":"number","min":0.1,"max":10,"default":1.5,"label":"Spacing","unit":"mm","group":"Lines",
            "help":"Distance between the lines. Smaller is denser.","helpUrl":"https://example.org/params/line_spacing_mm",
            "visibleWhen":{"pattern":["lines","crosshatch"]}},
        "pen_width_mm":{"type":"number","min":0.1,"max":5,"default":0.3,"label":"Pen width","unit":"mm","group":"Lines"},
        "outline":{"type":"bool","default":true,"label":"Outline","group":"Outline"},
        "outline_inset_mm":{"type":"number","min":-10,"max":10,"default":0,"label":"Inset","unit":"mm","group":"Outline",
            "visibleWhen":{"outline":[true]}},
        "outline_passes":{"type":"int","min":1,"max":4,"default":1,"label":"Passes","group":"Outline",
            "visibleWhen":{"outline":[true]}}}}"#;
    let src = tpl::module(manifest, tpl::ECHO, "", 1);
    std::fs::write(dir.join("hatch-test-filter.wasm"), wat::parse_str(&src).unwrap()).unwrap();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wat::parse_str(&src).unwrap())})).unwrap();
    app.run("plugin.dialog", json!({"id": "org.example.hatch"})).unwrap();
    let mut ready = false;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(720.0, 620.0)).wgpu().build_ui_state(
        move |ui, app: &mut VectorcraftApp| {
            if !std::mem::replace(&mut ready, true) {
                crate::theme::install_fonts(ui.ctx());
                crate::theme::apply(ui.ctx(), crate::theme::Brightness::MediumDark);
                return;
            }
            show(app, ui.ctx());
        },
        app,
    );
    h.run_steps(6);
    h.render().unwrap().save(dir.join("plugin-fields.png")).unwrap();
    h.state_mut().run("plugin.remove", json!({"id": "org.example.hatch"})).unwrap();
}
