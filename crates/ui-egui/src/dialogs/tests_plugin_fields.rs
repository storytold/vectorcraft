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
use crate::i18n::LangExt as _;

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

/// A row of a plug-in's dialog as shown: its key, its label and its choices' (value, label) pairs.
type ShownRow = (String, String, Vec<(String, String)>);

/// A plug-in's dialog shows the text its manifest's `translations` give in the UI language (the
/// heading, a label, a group heading and a choice's label) and its own text in any other language;
/// the choice still passes its value. Plug-in text with a right-to-left override is shown escaped.
#[test]
fn plugin_dialogs_show_the_plugins_text_in_the_ui_language() {
    let id = "test.dialogs.translated";
    // The override as a JSON escape in the manifest.
    let rlo = format!("\\u{:04X}", 0x202E);
    let manifest = format!(
        r#"{{"id":"{id}","name":"Tidy","kind":"filter","params":{{
            "mode":{{"type":"choice","options":["soft","hard"],"label":"Mode","group":"Shape","help":"How hard."}},
            "flip":{{"type":"bool","label":"Flip{rlo}txt"}}}},
            "commands":[{{"id":"grow","label":"Grow","params":{{"by":{{"type":"number","min":0,"max":10,"default":2,"label":"By"}}}}}}],
            "translations":{{"de":{{"name":"Aufräumen","params":{{"mode":{{"label":"Modus","options":{{"soft":"Weich"}}}}}},
                "groups":{{"Shape":"Form"}},"commands":{{"grow":{{"label":"Wachsen","params":{{"by":{{"label":"Um"}}}}}}}}}}}}}}"#
    );
    let module = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    let installed = vectorcraft_plugins::registry::install_bytes(&module).unwrap();
    // The rows' labels, section headings and choice labels in `lang`, as (key, label, choices).
    let shown = |lang: &str, m: &vectorcraft_plugins::Manifest| {
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).expect("language registered"));
        let rows = super::form::plugin_params(m, &Map::new());
        crate::i18n::set_current(crate::i18n::Lang::EN);
        rows.iter()
            .map(|p| {
                let choices: Vec<(String, String)> = match &p.kind {
                    craft::ui::patterns::form_dialogs::ParamKind::Choice { options, .. } => {
                        options.iter().map(|(v, l)| (v.clone(), l.as_str().to_string())).collect()
                    }
                    _ => Vec::new(),
                };
                (p.key.clone(), p.label.as_str().to_string(), choices)
            })
            .collect::<Vec<_>>()
    };
    let label = |rows: &[ShownRow], key: &str| rows.iter().find(|r| r.0 == key).map(|r| r.1.clone()).unwrap_or_else(|| panic!("no row for {key}"));
    let sections = |rows: &[ShownRow]| rows.iter().filter(|r| r.0.starts_with("section:")).map(|r| r.1.clone()).collect::<Vec<_>>();
    let choices = |rows: &[ShownRow]| rows.iter().find(|r| r.0 == "mode").map(|r| r.2.clone()).unwrap_or_default();
    let m = installed.manifest();
    let (de, en) = (shown("de", m), shown("en", m));
    assert_eq!((label(&de, "mode"), sections(&de)), ("Modus".to_string(), vec!["Form".to_string()]));
    assert_eq!(choices(&de), [("soft".to_string(), "Weich".to_string()), ("hard".to_string(), "hard".to_string())], "values stay");
    assert_eq!((label(&en, "mode"), sections(&en)), ("Mode".to_string(), vec!["Shape".to_string()]));
    assert_eq!(choices(&en), [("soft".to_string(), "soft".to_string()), ("hard".to_string(), "hard".to_string())]);
    assert_eq!(label(&de, "flip"), "Flip\\u{202E}txt");
    assert!(!de.iter().chain(&en).any(|r| r.1.contains('\u{202E}')), "the override is never shown as one");
    // The heading follows the language it is drawn in, for the filter and for its command.
    let command = format!("plugin.{id}.grow");
    let heading = |lang: &str, command: Option<&str>, gone: &str| {
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).unwrap());
        let shown = super::plugin::heading(id, command, gone);
        crate::i18n::set_current(crate::i18n::Lang::EN);
        shown
    };
    assert_eq!((heading("de", None, "Tidy"), heading("en", None, "Tidy")), ("Aufräumen".to_string(), "Tidy".to_string()));
    assert_eq!((heading("de", Some(&command), "Grow"), heading("fr", Some(&command), "Grow")), ("Wachsen".to_string(), "Grow".to_string()));
    let grow = m.for_command(m.command("grow").unwrap());
    assert_eq!(label(&shown("de", &grow), "by"), "Um");
    vectorcraft_plugins::registry::remove(id);
    assert_eq!(heading("de", None, "Tidy"), "Tidy", "the manifest's own name once the plug-in is gone");
}
