//! A plug-in's parameters in its dialog as its manifest presents them (docs/plugins.md ›
//! Parameters): labels, units after the values, help on the labels and a button to its page,
//! sections by group, text and lists of numbers, parameters shown only while their
//! `visibleWhen` holds, and sliders. The plug-in is removed after each test: plug-ins are process-wide.

use std::sync::{Arc, Mutex};

use egui::vec2;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_plugins::wat as tpl;

use super::*;

/// The install approval has no OK in the shared footer, so Enter presses OK only when OK has the
/// keyboard focus.
#[test]
fn enter_does_not_answer_the_install_approval() {
    assert!(plugin::INSTALL_SPEC.ok.is_none() && plugin::INSTALL_SPEC.body_with_actions.is_some());
}

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

/// A plug-in's dialog draws a slider for a number that asks for one, named by its row's label,
/// and none for one that doesn't. Screen readers' steps move it by the parameter's `step`, counted
/// from `min`; a value they set lands on the step's grid. Drawing a slider leaves a value off its
/// grid as it is, a fractional step gives the plug-in the step's own decimals, a range under 1
/// without a step still moves, and a drag reports itself to the dialog's preview while it lasts.
#[test]
fn a_plug_in_number_that_asks_for_a_slider_gets_one_that_steps_by_its_step() {
    use egui::accesskit::{Action, ActionData, ActionRequest, Role};
    use egui_kittest::kittest::NodeT;
    type Rows = egui_kittest::Harness<'static, (Dialog, vectorcraft_plugins::Manifest)>;
    let m = vectorcraft_plugins::Manifest::parse(
        br#"{"id":"test.area","name":"Area","kind":"filter","params":{
            "area":{"type":"number","min":0,"max":100,"default":10,"label":"Area","widget":"slider","step":5},
            "plain":{"type":"number","min":0,"max":100,"default":10,"label":"Plain"},
            "third":{"type":"int","min":1,"max":20,"default":1,"label":"Third","widget":"slider","step":3},
            "odd":{"type":"number","min":0,"max":100,"default":12,"label":"Odd","widget":"slider","step":5},
            "tenth":{"type":"number","min":0,"max":1,"default":0,"label":"Tenth","widget":"slider","step":0.1},
            "tiny":{"type":"number","min":0,"max":0.004,"default":0.001,"label":"Tiny","widget":"slider"},
            "milli":{"type":"number","min":0,"max":1,"default":0.004,"label":"Milli","widget":"slider","step":0.001}}}"#,
    )
    .unwrap();
    let fields = json!({ "area": 10, "plain": 10, "third": 1, "odd": 12, "tenth": 0, "tiny": 0.001, "milli": 0.004 });
    let dialog = Dialog::new(plugin::KIND, fields);
    let mut h: Rows = egui_kittest::Harness::builder().with_size(vec2(520.0, 480.0)).build_ui_state(
        |ui, (d, m): &mut (Dialog, vectorcraft_plugins::Manifest)| {
            form::schema_rows(ui, d, m);
        },
        (dialog, m),
    );
    h.run();
    let value = |h: &Rows, key: &str| h.state().0.fields[key].as_f64().unwrap();
    let act = |h: &mut Rows, label: &str, action: Action, data: Option<ActionData>| {
        let (target_node, target_tree) = h.get_by_role_and_label(Role::Slider, label).accesskit_node().locate();
        h.event(egui::Event::AccessKitActionRequest(ActionRequest { target_node, target_tree, action, data }));
        h.run();
    };
    assert_eq!(h.query_all_by_role(Role::Slider).count(), 6, "only the parameters that ask for one have a slider");
    // A number field shows the places its slider moves to.
    assert!(h.query_all_by_value("0.004").next().is_some(), "Milli's field shows 0.004");
    assert_eq!((value(&h, "odd"), value(&h, "tiny")), (12.0, 0.001), "values off the sliders' grids are kept");
    h.state_mut().0.fields.extend([("odd".into(), json!(13)), ("tenth".into(), json!(0.15))]);
    h.run();
    assert_eq!((value(&h, "odd"), value(&h, "tenth")), (13.0, 0.15), "typed values are kept");
    h.state_mut().0.fields.insert("tenth".into(), json!(0));
    let node = h.get_by_role_and_label(Role::Slider, "Area:").accesskit_node();
    assert_eq!((node.numeric_value(), node.min_numeric_value(), node.max_numeric_value()), (Some(10.0), Some(0.0), Some(100.0)));
    act(&mut h, "Area:", Action::Increment, None);
    assert_eq!(value(&h, "area"), 15.0, "one step up");
    act(&mut h, "Area:", Action::SetValue, Some(ActionData::NumericValue(47.0)));
    let set = value(&h, "area");
    assert!((set / 5.0).fract() == 0.0 && (set - 47.0).abs() <= 2.5, "47 lands on the nearest step: {set}");
    // The grid is counted from `min`: 1, 4, 7.
    act(&mut h, "Third:", Action::Increment, None);
    act(&mut h, "Third:", Action::Increment, None);
    assert_eq!(h.state().0.fields["third"], json!(7));
    for _ in 0..3 {
        act(&mut h, "Tenth:", Action::Increment, None);
    }
    assert_eq!(h.state().0.fields["tenth"], json!(0.3));
    act(&mut h, "Tiny:", Action::SetValue, Some(ActionData::NumericValue(0.0025)));
    let tiny = value(&h, "tiny");
    assert!((tiny - 0.0025).abs() < 0.0001, "a small range still moves: {tiny}");
    // Dragging along the rail moves the value and marks the drag for the preview until it ends.
    let rail = h.get_by_role_and_label(Role::Slider, "Area:").rect();
    let at = |x: f32| egui::pos2(rail.left() + x * rail.width(), rail.center().y);
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    h.event(egui::Event::PointerMoved(at(0.2)));
    h.event(button(at(0.2), true));
    h.run();
    for x in [0.4, 0.6, 0.8] {
        h.event(egui::Event::PointerMoved(at(x)));
        h.run();
    }
    assert!(h.state().0.bool("__sliding"), "the drag is in progress");
    let dragged = value(&h, "area");
    assert!(dragged > set && (dragged / 5.0).fract() == 0.0 && dragged <= 100.0, "moved right on the grid: {dragged}");
    h.event(button(at(0.8), false));
    h.run();
    assert!(!h.state().0.bool("__sliding"), "the drag ended");
}

/// During a drag on a plug-in parameter's slider, the preview runs at most every
/// [`form::SLIDE_PREVIEW_EVERY`] on the latest value, and the value the drag ends on previews even
/// when its turn hadn't come; outside a drag every change previews at once.
#[test]
fn a_slider_drag_previews_a_few_times_a_second_and_its_last_value() {
    let mut slide = form::SlidePreview::default();
    let frame = 1.0 / 60.0;
    // A drag of one second, with a new value on every frame.
    let previews = (0..60).filter(|i| slide.due(*i as f64 * frame, true, true) == Ok(true)).count();
    assert!(previews > 1 && previews <= (1.0 / form::SLIDE_PREVIEW_EVERY) as usize + 1, "{previews} previews in 60 frames");
    // A change just after a preview waits; the frame the drag ends on previews it.
    assert_eq!(slide.due(1.5, true, true), Ok(true));
    assert!(slide.due(1.5 + frame, true, true).is_err() && slide.held, "the last change is still to preview");
    assert_eq!(slide.due(1.5 + 2.0 * frame, false, false), Ok(true));
    assert_eq!(slide.due(1.5 + 3.0 * frame, false, false), Ok(false), "nothing left to preview");
    // A change that waits previews on the first frame after its wait, even without another change.
    assert_eq!(slide.due(2.0, true, true), Ok(true));
    let wait = slide.due(2.0 + frame, true, true).expect_err("too soon after the last preview");
    assert!(wait > 0.0 && wait <= form::SLIDE_PREVIEW_EVERY);
    assert_eq!(slide.due(2.0 + frame + wait, true, false), Ok(true));
    // Outside a drag, a change previews at once however soon it comes.
    assert_eq!((slide.due(3.0, false, true), slide.due(3.0 + frame, false, true)), (Ok(true), Ok(true)));
}

/// A filter's and a live effect's dialogs, at frames a sixtieth of a second apart: a drag on a
/// slider previews fewer values than it has frames, and the value it ends on previews. In the
/// filter's dialog, turning Preview off during the drag rolls the preview back at once.
#[test]
fn a_slider_drag_in_a_plug_in_dialog_previews_a_few_times_a_second() {
    use egui::accesskit::Role;
    let params = r#"{"area":{"type":"number","min":0,"max":100,"default":10,"label":"Area","widget":"slider","step":1}}"#;
    for kind in ["filter", "effect"] {
        let id = format!("test.ui.fields.slide{kind}");
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let shape = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
        let wasm = wat::parse_str(tpl::module(&tpl::manifest(&id, kind, params), tpl::ECHO, "", 1)).unwrap();
        app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
        match kind {
            "filter" => app.run("plugin.dialog", json!({"id": id})),
            _ => app.run("effect.dialog", json!({"effect": vectorcraft_plugins::effect::effect_id(&id)})),
        }
        .unwrap();
        let mut fonts = false;
        let mut h: Harness = egui_kittest::Harness::builder().with_size(vec2(900.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
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
        set(&mut h, "preview", json!(true));
        // What the canvas shows: the filter's last previewed params, the effect's previewed value.
        let previewed = |h: &Harness| match kind {
            "filter" => h.state().ui.dialog.as_ref().and_then(|d| d.fields.get(form::PREVIEWED)).map(|p| p["params"]["area"].clone()),
            _ => {
                let doc = h.state().session.doc().unwrap();
                doc.doc.node(vectorcraft_doc::NodeId(shape)).and_then(|n| n.appearance.effects.last()).map(|e| e.params["area"].clone())
            }
        };
        let area = |h: &Harness| h.state().ui.dialog.as_ref().expect("the dialog is open").fields["area"].clone();
        let rail = h.get_by_role_and_label(Role::Slider, "Area:").rect();
        let at = |x: f32| egui::pos2(rail.left() + x * rail.width(), rail.center().y);
        let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
        h.event(egui::Event::PointerMoved(at(0.1)));
        h.event(button(at(0.1), true));
        h.step();
        let mut seen = vec![];
        for i in 1..=30 {
            h.event(egui::Event::PointerMoved(at(0.1 + 0.025 * i as f32)));
            h.step();
            let shown = previewed(&h);
            if seen.last() != Some(&shown) {
                seen.push(shown);
            }
        }
        assert!(seen.len() > 2 && seen.len() < 15, "{kind}: {} values previewed in 30 frames: {seen:?}", seen.len());
        h.event(button(at(0.85), false));
        h.step();
        h.step();
        assert_eq!(previewed(&h), Some(area(&h)), "{kind}: the value the drag ended on previews");
        if kind == "filter" {
            // A second drag, until a frame previews; the next frame waits, and Preview off rolls back then.
            h.event(button(at(0.5), true));
            h.step();
            for i in 1..=10 {
                let before = previewed(&h);
                h.event(egui::Event::PointerMoved(at(0.5 - 0.02 * i as f32)));
                h.step();
                if previewed(&h) != before {
                    break;
                }
            }
            h.state_mut().ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(false));
            h.event(egui::Event::PointerMoved(at(0.2)));
            h.step();
            assert_eq!(previewed(&h), None, "Preview off rolled the preview back");
            assert!(!h.state().session.in_interaction(), "and ended its interaction");
            h.event(button(at(0.2), false));
            h.step();
        }
        h.state_mut().ui.dialog = None;
        let _ = h.state_mut().session.cancel_interaction();
        remove(&mut h, &id);
    }
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
