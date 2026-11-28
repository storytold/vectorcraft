//! The 3D and Materials panel drawn headless: its Window menu entry, the object types applying or
//! changing a 3D effect, the dialogs' controls for the applied effect, and one undo step for a
//! drag or a typed value.
#![allow(clippy::unwrap_used, clippy::panic)]

use std::cell::RefCell;
use std::rc::Rc;

use craft::behavior::{CancelReason, EditEvent, GestureId, Mixed};
use craft::ui::canvas::CanvasSpace;
use craft::ui::contract::ShowContainer as _;
use craft::ui::patterns::form_dialogs::DialogFields;
use craft::ui::testing::Fixture;
use serde_json::{Map, Value, json};
use vectorcraft_engine::Session;

use super::{Target, apply, changed_settings, handles, send, set_type, show, target};
use crate::dialogs::form;
use crate::panels::Live;
use crate::tests_labels::painted_text;
use crate::{Services, VectorcraftApp};

/// An app with a selected blue square.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app.run("shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    app.run("paint.setFill", json!({"color": "#338ad6"})).unwrap();
    app
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn effect(app: &VectorcraftApp) -> (String, Value) {
    let t = target(app).unwrap();
    (t.effect, t.params)
}

/// The gesture the tests' edit streams run in.
const GESTURE: GestureId = GestureId::new(1);

/// The panel's values as its rows hold them: the first selected effect's settings with `changes` applied.
fn whole(app: &VectorcraftApp, changes: &[(&str, Value)]) -> DialogFields {
    let t = target(app).unwrap();
    let mut settings = vectorcraft_effects::merged_params(&t.effect, &t.params).as_object().cloned().unwrap();
    settings.extend(changes.iter().map(|(key, value)| (key.to_string(), value.clone())));
    form::fields_of(&super::rows(app, &t).0, &settings)
}

/// The first selected effect's settings with the keys the selected effects disagree on marked mixed, and the same
/// settings as JSON, where a mixed key is absent.
fn mixed_settings(app: &VectorcraftApp) -> (DialogFields, Map<String, Value>) {
    let mut fields = whole(app, &[]);
    super::mark_mixed(app, &mut fields);
    let settings = form::json_of(&super::rows(app, &target(app).unwrap()).0, &fields);
    (fields, settings)
}

/// One pass of the panel's rows that reports `event`, over the values `input` they were shown with.
fn pass(app: &mut VectorcraftApp, input: &DialogFields, event: EditEvent<DialogFields>) {
    let params = super::rows(app, &target(app).unwrap()).0;
    apply(app, &params, input, &event.into());
}

/// A gesture `id` begins on the panel's current values; returns them.
fn begin(app: &mut VectorcraftApp, id: GestureId) -> DialogFields {
    let before = whole(app, &[]);
    pass(app, &before, EditEvent::begin(id, Mixed::Same(before.clone()), super::LABEL.into(), None));
    before
}

/// Gesture `id` proposes the panel's values with `changes`.
fn update(app: &mut VectorcraftApp, id: GestureId, changes: &[(&str, Value)]) {
    let (input, value) = (whole(app, &[]), whole(app, changes));
    pass(app, &input, EditEvent::update(id, value));
}

/// Gesture `id` commits the panel's values as they stand.
fn commit(app: &mut VectorcraftApp, id: GestureId, before: DialogFields) {
    let after = whole(app, &[]);
    pass(app, &after.clone(), EditEvent::commit(id, Mixed::Same(before), after));
}

/// `app` drawn by the panel in a trial that runs frame by frame, and the handle the trial reads the app through.
fn shown(app: VectorcraftApp) -> (Rc<RefCell<VectorcraftApp>>, Fixture) {
    let app = Rc::new(RefCell::new(app));
    let drawn = Rc::clone(&app);
    let mut fixture = Fixture::new(move |ui| show(&mut drawn.borrow_mut(), ui));
    fixture.settle();
    (app, fixture)
}

#[test]
fn an_object_type_applies_its_effect_then_changes_it_in_place() {
    let mut app = app();
    let history = undo_len(&app);
    set_type(&mut app, "extrude").unwrap();
    assert_eq!(effect(&app).0, "threeD.extrude");
    assert_eq!(undo_len(&app), history + 1);
    app.run("effect.setParams", json!({"index": 0, "item": null, "params": {"rotationX": 15.0, "depth": 70.0}})).unwrap();
    for (kind, id) in [("inflate", "threeD.inflate"), ("revolve", "threeD.revolve"), ("plane", "threeD.rotate"), ("extrude", "threeD.extrude")] {
        let history = undo_len(&app);
        set_type(&mut app, kind).unwrap();
        let (now, params) = effect(&app);
        assert_eq!(now, id);
        assert_eq!(params["rotationX"], 15.0, "{kind}");
        assert_eq!(undo_len(&app), history + 1, "{kind}");
        // The object keeps one effect: the type changed in place.
        assert_eq!(app.session.active().unwrap().doc.node(app.session.active().unwrap().selection.objects[0]).unwrap().appearance.effects.len(), 1);
    }
    // The type it already has runs nothing.
    let history = undo_len(&app);
    set_type(&mut app, "extrude").unwrap();
    assert_eq!(undo_len(&app), history);
    assert!(set_type(&mut app, "cube").unwrap_err().contains("must be one of plane, extrude, revolve, inflate"));
    assert_eq!(undo_len(&app), history);
}

/// The ids of the effects of objects `ids`, in order.
fn effect_ids(app: &VectorcraftApp, ids: &[u64]) -> Vec<Vec<String>> {
    let st = app.session.active().unwrap();
    ids.iter().map(|id| st.doc.node(vectorcraft_doc::NodeId(*id)).unwrap().appearance.effects.iter().map(|e| e.id.clone()).collect()).collect()
}

#[test]
fn an_object_type_reaches_every_selected_object_in_one_undo_step() {
    let mut app = app();
    let a = app.session.active().unwrap().selection.objects[0].0;
    set_type(&mut app, "extrude").unwrap();
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.inflate", "item": null})).unwrap();
    let c = app.run("shape.rectangle", json!({"x": 100, "y": 250, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    // The first object already has the type; the second has another; the third has none.
    for order in [[a, b, c], [c, b, a]] {
        app.run("select.set", json!({"ids": order})).unwrap();
        let before = app.session.doc().unwrap().doc.clone();
        let history = undo_len(&app);
        set_type(&mut app, "extrude").unwrap();
        assert_eq!(effect_ids(&app, &[a, b, c]), vec![vec!["threeD.extrude".to_string()]; 3], "{order:?}");
        assert_eq!(undo_len(&app), history + 1, "{order:?}");
        app.run("edit.undo", json!({})).unwrap();
        assert_eq!(app.session.doc().unwrap().doc, before, "{order:?}");
    }
    // Every selected object already has the type: nothing is recorded.
    app.run("select.set", json!({"ids": [a, b, c]})).unwrap();
    set_type(&mut app, "extrude").unwrap();
    let history = undo_len(&app);
    set_type(&mut app, "extrude").unwrap();
    assert_eq!(undo_len(&app), history);
}

#[test]
fn a_drag_ends_when_the_selection_goes_away() {
    let mut app = app();
    set_type(&mut app, "extrude").unwrap();
    let ctx = crate::testing::ui_context();
    let history = undo_len(&app);
    begin(&mut app, GESTURE);
    update(&mut app, GESTURE, &[("depth", json!(90.0))]);
    assert!(app.session.in_interaction());
    app.session.active_mut().unwrap().selection.objects.clear();
    ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui)).textures_delta.clear();
    assert!(!app.session.in_interaction());
    assert_eq!(undo_len(&app), history + 1);
}

/// Render Settings › Wireframe: shown for the panel's four effects, it sets `render.mode` on every
/// selected 3D effect that takes it in one undo step, the type switch keeps it, and choosing it
/// again draws the shaded surface. A 3D (Classic) effect beside them is left as it is, and the
/// menu is not shown for it alone.
#[test]
fn render_settings_wireframe_turns_the_mode_on_and_off_for_the_selection() {
    let mut app = app();
    let a = app.session.active().unwrap().selection.objects[0].0;
    set_type(&mut app, "extrude").unwrap();
    assert!(painted_text(&mut app, show).contains("Render Settings"));
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.revolve", "item": null})).unwrap();
    let c = app.run("shape.rectangle", json!({"x": 100, "y": 250, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.classicExtrude", "item": null})).unwrap();
    let mode = |app: &VectorcraftApp, id: u64| {
        let n = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().clone();
        n.appearance.effects[0].params.get("render").and_then(|r| r.get("mode")).and_then(Value::as_str).map(str::to_string)
    };
    app.run("select.set", json!({"ids": [a, b, c]})).unwrap();
    let history = undo_len(&app);
    super::set_wireframe(&mut app, true);
    assert_eq!(undo_len(&app), history + 1);
    assert_eq!((mode(&app, a).as_deref(), mode(&app, b).as_deref()), (Some("wireframe"), Some("wireframe")));
    assert_eq!(mode(&app, c), None, "the classic effect takes no render settings");
    assert!(super::wireframe_on(&target(&app).unwrap()));
    // The object type changes in place and keeps the mode.
    app.run("select.set", json!({"ids": [a]})).unwrap();
    set_type(&mut app, "inflate").unwrap();
    assert_eq!(mode(&app, a).as_deref(), Some("wireframe"));
    let history = undo_len(&app);
    super::set_wireframe(&mut app, false);
    assert_eq!(undo_len(&app), history + 1);
    assert_eq!(mode(&app, a).as_deref(), Some("shaded"));
    assert!(!super::wireframe_on(&target(&app).unwrap()));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(mode(&app, a).as_deref(), Some("wireframe"));
    // The 3D (Classic) effect alone has no Render Settings.
    app.run("select.set", json!({"ids": [c]})).unwrap();
    assert!(!painted_text(&mut app, show).contains("Render Settings"));
}

#[test]
fn a_drag_previews_and_records_one_undo_step_when_released() {
    let mut app = app();
    set_type(&mut app, "extrude").unwrap();
    let history = undo_len(&app);
    let before = begin(&mut app, GESTURE);
    for depth in [60.0, 80.0, 120.0] {
        update(&mut app, GESTURE, &[("depth", json!(depth))]);
        assert_eq!(effect(&app).1["depth"], depth);
    }
    // Another setting in the same drag keeps the earlier one.
    update(&mut app, GESTURE, &[("twist", json!(45.0))]);
    assert_eq!(effect(&app).1["depth"], 120.0);
    assert_eq!(undo_len(&app), history);
    // The gesture commits what it previewed.
    commit(&mut app, GESTURE, before);
    assert_eq!(undo_len(&app), history + 1);
    let (_, params) = effect(&app);
    assert_eq!((params["depth"].as_f64(), params["twist"].as_f64()), (Some(120.0), Some(45.0)));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(effect(&app).1["depth"], 50.0);
    // A typed value is one undo step at once.
    send(&mut app, Map::from_iter([("taper".to_string(), json!(60.0))]), Live::Released);
    assert_eq!(effect(&app).1["taper"], 60.0);
    assert_eq!(undo_len(&app), history + 1);
    // Nothing changed and no drag: nothing runs.
    send(&mut app, Map::new(), Live::Released);
    assert_eq!(undo_len(&app), history + 1);
}

#[test]
fn a_change_reaches_each_selected_3d_effect_and_no_other_effect() {
    let mut app = app();
    let a = app.session.active().unwrap().selection.objects[0].0;
    set_type(&mut app, "extrude").unwrap();
    // A second object with Inflate after a drop shadow, a third with only a drop shadow.
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "stylize.dropShadow", "item": null})).unwrap();
    app.run("effect.apply", json!({"effect": "threeD.inflate", "item": null})).unwrap();
    let c = app.run("shape.rectangle", json!({"x": 100, "y": 250, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "stylize.dropShadow", "item": null})).unwrap();
    app.run("select.set", json!({"ids": [a, b, c]})).unwrap();
    let shadow = |app: &VectorcraftApp, id: u64| {
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params.clone()
    };
    let (b_shadow, c_shadow) = (shadow(&app, b), shadow(&app, c));
    let history = undo_len(&app);
    // `bevel` is an Extrude & Bevel setting only; `depth` both effects take.
    send(&mut app, Map::from_iter([("depth".to_string(), json!(75.0)), ("bevel".to_string(), json!("classic"))]), Live::Dragging);
    send(&mut app, Map::new(), Live::Released);
    assert_eq!(undo_len(&app), history + 1);
    let st = app.session.active().unwrap();
    let fx = |id: u64| st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects.clone();
    assert_eq!(fx(a)[0].params["depth"], 75.0);
    assert_eq!(fx(a)[0].params["bevel"], "classic");
    assert_eq!(fx(b)[1].params["depth"], 75.0);
    assert!(fx(b)[1].params.get("bevel").is_none());
    assert_eq!(fx(b)[0].params, b_shadow);
    assert_eq!(fx(c)[0].params, c_shadow);
}

/// A custom bevel picked in the panel sends its name and points with `bevel`, to the effects that
/// take a bevel only, and another custom bevel replaces them.
#[test]
fn a_custom_bevel_sends_its_name_and_points() {
    let mut app = app();
    let a = app.session.active().unwrap().selection.objects[0].0;
    set_type(&mut app, "extrude").unwrap();
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.inflate", "item": null})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let pick = |name: &str, points: Value| {
        Map::from_iter([
            ("bevel".to_string(), json!("custom")),
            (vectorcraft_effects::BEVEL_NAME.to_string(), json!(name)),
            (vectorcraft_effects::BEVEL_PROFILE.to_string(), points),
        ])
    };
    let notch = json!([[0.0, 0.0], [0.2, 0.6], [0.7, 0.4], [1.0, 1.0]]);
    send(&mut app, pick("Notch", notch.clone()), Live::Released);
    let fx = |app: &VectorcraftApp, id: u64| {
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params.clone()
    };
    let p = fx(&app, a);
    assert_eq!(
        (&p["bevel"], &p[vectorcraft_effects::BEVEL_NAME], &p[vectorcraft_effects::BEVEL_PROFILE]),
        (&json!("custom"), &json!("Notch"), &notch)
    );
    assert!(fx(&app, b).get(vectorcraft_effects::BEVEL_PROFILE).is_none());
    let steps = json!([[0.0, 0.0], [0.0, 0.5], [0.5, 0.5], [0.5, 1.0], [1.0, 1.0]]);
    send(&mut app, pick("Steps", steps.clone()), Live::Released);
    let p = fx(&app, a);
    assert_eq!((&p[vectorcraft_effects::BEVEL_NAME], &p[vectorcraft_effects::BEVEL_PROFILE]), (&json!("Steps"), &steps));
}

/// With one Extrude & Bevel on a custom bevel and one without, picking that custom bevel sends its
/// name and points too, though the form already held them, so both effects take it.
#[test]
fn a_custom_bevel_picked_for_a_mixed_selection_reaches_every_effect() {
    let mut app = app();
    let notch = json!([[0.0, 0.0], [0.2, 0.6], [0.7, 0.4], [1.0, 1.0]]);
    let custom = json!({"bevel": "custom", "bevelName": "Notch", "bevelProfile": notch});
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": custom})).unwrap();
    let a = app.session.active().unwrap().selection.objects[0].0;
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let (marked, mut fields) = mixed_settings(&app);
    assert!(marked.is_mixed("bevel"));
    assert_eq!(fields[vectorcraft_effects::BEVEL_NAME], json!("Notch"));
    let before = Value::Object(fields.clone());
    fields.insert("bevel".into(), json!("custom"));
    let changed = changed_settings(&before, &Value::Object(fields));
    assert_eq!(Value::Object(changed.clone()), json!({"bevel": "custom", "bevelName": "Notch", "bevelProfile": notch}));
    send(&mut app, changed, Live::Released);
    let st = app.session.active().unwrap();
    for id in [a, b] {
        let p = &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params;
        assert_eq!((&p["bevel"], &p[vectorcraft_effects::BEVEL_PROFILE]), (&json!("custom"), &notch), "{id}");
    }
}

/// Two custom bevels the bevels file does not list: the mixed selection unsets the shown points,
/// and Bevel Shape still lists the first effect's own bevel, which then reaches both effects.
#[test]
fn a_mixed_selection_still_lists_the_first_effects_own_custom_bevel() {
    use crate::dialogs::revolve::bevel_options;
    let mut app = app();
    let notch = json!([[0.0, 0.0], [0.2, 0.6], [0.7, 0.4], [1.0, 1.0]]);
    let steps = json!([[0.0, 0.0], [0.0, 0.5], [0.5, 0.5], [0.5, 1.0], [1.0, 1.0]]);
    let custom = |name: &str, points: &Value| json!({"bevel": "custom", "bevelName": name, "bevelProfile": points});
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": custom("Notch", &notch)})).unwrap();
    let a = app.session.active().unwrap().selection.objects[0].0;
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": custom("Steps", &steps)})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let first = target(&app).map(|t| vectorcraft_effects::merged_params(&t.effect, &t.params)).unwrap().as_object().cloned().unwrap();
    let (marked, mut fields) = mixed_settings(&app);
    assert!(marked.is_mixed(vectorcraft_effects::BEVEL_PROFILE));
    // The panel lists the bevels from the first effect's own settings ([`super::rows`]).
    let (options, builtin) = bevel_options(&app, &first, "threeD.extrude");
    assert_eq!(options.len(), builtin + 1);
    let (label, value, picked) = options[builtin].clone();
    assert_eq!((label.as_str(), value), ("Notch", json!("custom")));
    let (name, points) = picked.unwrap();
    assert_eq!(points, notch);
    // Picking it sends the whole bevel to both effects.
    let before = Value::Object(fields.clone());
    fields.insert(vectorcraft_effects::BEVEL_NAME.into(), json!(name));
    fields.insert(vectorcraft_effects::BEVEL_PROFILE.into(), points);
    send(&mut app, changed_settings(&before, &Value::Object(fields)), Live::Released);
    let st = app.session.active().unwrap();
    for id in [a, b] {
        let p = &st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params;
        assert_eq!((&p[vectorcraft_effects::BEVEL_NAME], &p[vectorcraft_effects::BEVEL_PROFILE]), (&json!("Notch"), &notch), "{id}");
    }
}

/// An effect's own custom bevel named like a bevel of the bevels file with other points is listed
/// under its own label, so Bevel Shape does not show two entries of one name.
#[test]
fn an_own_custom_bevel_named_like_a_file_bevel_gets_its_own_label() {
    use crate::dialogs::revolve::bevel_options;
    let dir = std::env::temp_dir().join(format!("vectorcraft-ui-bevels-own-label-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut app = app();
    app.session.attach_bevels_file(dir.join(vectorcraft_engine::cmd::bevels::FILE_NAME).to_string_lossy().to_string());
    app.run("threeD.bevels.edit", json!({})).unwrap();
    let file = app.session.bevels.get("Example Bevel").unwrap().profile_json();
    let other = json!([[0.0, 0.0], [0.5, 0.5], [1.0, 1.0]]);
    assert_ne!(file, other);
    let fields = json!({"bevel": "custom", "bevelName": "Example Bevel", "bevelProfile": other}).as_object().cloned().unwrap();
    let (options, builtin) = bevel_options(&app, &fields, "threeD.extrude");
    let _ = std::fs::remove_dir_all(&dir);
    let labels: Vec<&str> = options[builtin..].iter().map(|(l, _, _)| l.as_str()).collect();
    assert_eq!(labels, ["Example Bevel", "Example Bevel (document)"]);
    assert_eq!(options[builtin + 1].2, Some(("Example Bevel".to_string(), other)));
}

#[test]
fn the_panel_finds_a_3d_effect_on_a_fill() {
    let mut app = app();
    app.run("effect.apply", json!({"effect": "threeD.inflate", "item": 0})).unwrap();
    let t = target(&app).unwrap();
    assert_eq!((t.item, t.index, t.effect.as_str()), (Some(0), 0, "threeD.inflate"));
    set_type(&mut app, "extrude").unwrap();
    assert_eq!(target(&app).map(|t: Target| (t.item, t.effect)), Some((Some(0), "threeD.extrude".to_string())));
}

#[test]
fn settings_the_selected_effects_disagree_on_are_unset_so_a_reset_reaches_them_all() {
    let mut app = app();
    let a = app.session.active().unwrap().selection.objects[0].0;
    set_type(&mut app, "extrude").unwrap();
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"depth": 80.0}})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let (marked, mut fields) = mixed_settings(&app);
    assert!(marked.is_mixed("depth"));
    assert!(!marked.is_mixed("twist"));
    assert_eq!(fields["twist"], 0.0);
    // Resetting Depth to its default (the first object's value) is a change for both objects.
    let before = Value::Object(fields.clone());
    fields.insert("depth".into(), json!(50.0));
    let changed = changed_settings(&before, &Value::Object(fields));
    assert_eq!(Value::Object(changed.clone()), json!({"depth": 50.0}));
    send(&mut app, changed, Live::Released);
    let st = app.session.active().unwrap();
    for id in [a, b] {
        assert_eq!(st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params["depth"], 50.0);
    }
}

#[test]
fn a_light_list_edit_reaches_only_the_objects_with_the_same_lights() {
    let mut app = app();
    let two = json!([{"color": "#ff8040"}, {"color": "#4080ff", "rotation": -35.0}]);
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"lights": two}})).unwrap();
    let a = app.session.active().unwrap().selection.objects[0].0;
    let add = |app: &mut VectorcraftApp, x: f64, lights: Value| {
        let id = app.run("shape.rectangle", json!({"x": x, "y": 250, "width": 80, "height": 80})).unwrap()["id"].as_u64().unwrap();
        app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"lights": lights}})).unwrap();
        id
    };
    let b = add(&mut app, 50.0, json!([{"color": "#00ff00"}]));
    let c = add(&mut app, 200.0, two.clone());
    app.run("select.set", json!({"ids": [a, b, c]})).unwrap();
    let lights = |app: &VectorcraftApp, id: u64| {
        app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params["lights"].clone()
    };
    let b_before = lights(&app, b);
    let three = json!([{"color": "#ff8040"}, {"color": "#4080ff", "rotation": -35.0}, {"color": "#ffffff", "rotation": 0.0, "height": 90.0}]);
    let history = undo_len(&app);
    send(&mut app, Map::from_iter([("lights".to_string(), three.clone())]), Live::Released);
    assert_eq!(undo_len(&app), history + 1);
    assert_eq!(lights(&app, a).as_array().map(Vec::len), Some(3));
    assert_eq!(lights(&app, c), lights(&app, a));
    assert_eq!(lights(&app, b), b_before);
}

#[test]
fn a_drag_keeps_its_objects_and_a_new_selection_starts_a_new_step() {
    let mut app = app();
    let a = app.session.active().unwrap().selection.objects[0];
    set_type(&mut app, "extrude").unwrap();
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let b = vectorcraft_doc::NodeId(b);
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null})).unwrap();
    app.run("select.set", json!({"ids": [a.0]})).unwrap();
    let history = undo_len(&app);
    begin(&mut app, GESTURE);
    update(&mut app, GESTURE, &[("depth", json!(90.0))]);
    // The selection changes during the drag: the drag on `a` is committed, and `b` starts its own.
    app.session.active_mut().unwrap().selection.objects = vec![b];
    let second = GESTURE.next();
    let before = begin(&mut app, second);
    update(&mut app, second, &[("depth", json!(70.0))]);
    commit(&mut app, second, before);
    let depth = |app: &VectorcraftApp, id| app.session.active().unwrap().doc.node(id).unwrap().appearance.effects[0].params["depth"].clone();
    assert_eq!(depth(&app, a), 90.0);
    assert_eq!(depth(&app, b), 70.0);
    assert_eq!(undo_len(&app), history + 2);
}

#[test]
fn a_light_drag_keeps_the_objects_whose_lights_matched_when_it_started() {
    let mut app = app();
    let one = |color: &str| json!([{"color": color}]);
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"lights": one("#ff0000")}})).unwrap();
    let a = app.session.active().unwrap().selection.objects[0].0;
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"lights": one("#00ff00")}})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let before = begin(&mut app, GESTURE);
    // The first object's list passes through the second's value during the drag.
    update(&mut app, GESTURE, &[("lights", one("#00ff00"))]);
    update(&mut app, GESTURE, &[("lights", one("#0000ff"))]);
    commit(&mut app, GESTURE, before);
    let lights =
        |id: u64| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params["lights"].clone();
    assert_eq!(lights(a), one("#0000ff"));
    assert_eq!(lights(b), one("#00ff00"));
}

#[test]
fn a_drag_ends_when_the_panel_is_no_longer_drawn() {
    let mut app = app();
    set_type(&mut app, "extrude").unwrap();
    let ctx = crate::testing::ui_context();
    let history = undo_len(&app);
    begin(&mut app, GESTURE);
    update(&mut app, GESTURE, &[("depth", json!(90.0))]);
    // A pass that draws the panel keeps the gesture.
    ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui)).textures_delta.clear();
    assert!(app.session.in_interaction());
    // The first pass that does not draw it ends the gesture with `Cancel { Removed }`, which commits what was previewed.
    let input = whole(&app, &[]);
    pass(&mut app, &input, EditEvent::cancel(GESTURE, CancelReason::Removed, None));
    assert!(!app.session.in_interaction());
    assert_eq!(undo_len(&app), history + 1);
    assert_eq!(effect(&app).1["depth"], 90.0);
}

/// The panel's controls, drawn once with the accessibility tree on.
fn controls(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<egui::accesskit::Node> {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
    out.textures_delta.clear();
    out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default()
}

/// The control named `label`: a row's side label carries the same name, in no set order among the nodes.
fn named<'a>(nodes: &'a [egui::accesskit::Node], label: &str) -> &'a egui::accesskit::Node {
    nodes.iter().find(|n| n.role() != egui::accesskit::Role::Label && n.label() == Some(label)).unwrap_or_else(|| panic!("no {label:?}"))
}

/// A checkbox setting the selected effects hold with different values shows a dash, leaves the
/// controls it guards available, and another change leaves it as each effect has it.
#[test]
fn a_mixed_checkbox_shows_a_dash_and_another_change_keeps_each_effects_value() {
    use crate::panels::tests_appearance::{click, frame_events, text_rect};
    let mut app = app();
    let a = app.session.active().unwrap().selection.objects[0].0;
    set_type(&mut app, "extrude").unwrap();
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"shade": false}})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let ctx = crate::testing::ui_context();
    ctx.enable_accesskit();
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "Lighting").center(), show);
    let nodes = controls(&mut app, &ctx);
    assert_eq!(named(&nodes, "Shading").toggled(), Some(egui::accesskit::Toggled::Mixed), "a mixed Shading shows a dash");
    let behind = named(&nodes, "Light behind the object");
    assert!(!behind.is_disabled(), "a mixed Shading leaves the light settings available");
    assert!(!named(&nodes, "Add Light").is_disabled());
    let at = behind.bounds().map(|r| egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32)).unwrap();
    let history = undo_len(&app);
    click(&ctx, &mut app, at, show);
    assert_eq!(undo_len(&app), history + 1);
    let params = |id: u64| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params.clone();
    let merged = |id: u64| vectorcraft_effects::merged_params("threeD.extrude", &params(id));
    assert_eq!((merged(a)["shade"].clone(), merged(b)["shade"].clone()), (json!(true), json!(false)));
    assert_eq!((merged(a)["lightBehind"].clone(), merged(b)["lightBehind"].clone()), (json!(true), json!(true)));
    // A single light whose settings differ: Add Light is off.
    app.run("select.set", json!({"ids": [b]})).unwrap();
    app.run("effect.setParams", json!({"index": 0, "item": null, "params": {"lightIntensity": 40.0}})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    assert!(named(&controls(&mut app, &ctx), "Add Light").is_disabled());
}

/// The Bevel heading's Reset sets every bevel setting back to its default on each selected Extrude &
/// Bevel, as one undo step, and leaves Depth and the rotation as each effect has them. With every
/// bevel setting at its default it is off and records nothing.
#[test]
fn the_bevel_reset_restores_only_the_bevel_settings_of_each_selected_effect() {
    use crate::panels::tests_appearance::click;
    let mut app = app();
    let bevelled = json!({"bevel": "rounded", "bevelWidth": 9.0, "bevelHeight": 6.0, "bevelRepeat": 3, "bevelSpace": 20.0, "bevelInside": true, "bevelBothSides": true, "depth": 70.0, "rotationX": 15.0});
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": bevelled})).unwrap();
    let a = app.session.active().unwrap().selection.objects[0].0;
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"bevel": "cove", "bevelWidth": 2.0, "depth": 30.0}}))
        .unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let ctx = crate::testing::ui_context();
    ctx.enable_accesskit();
    let nodes = controls(&mut app, &ctx);
    let reset = named(&nodes, "Reset");
    assert!(!reset.is_disabled(), "changed bevel settings turn Reset on");
    let at = reset.bounds().map(|r| egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32)).unwrap();
    let history = undo_len(&app);
    click(&ctx, &mut app, at, show);
    assert_eq!(undo_len(&app), history + 1, "one undo step for both objects");
    let defaults = vectorcraft_effects::default_params("threeD.extrude").unwrap();
    let merged = |app: &VectorcraftApp, id: u64| {
        let params = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params.clone();
        vectorcraft_effects::merged_params("threeD.extrude", &params)
    };
    for id in [a, b] {
        for key in ["bevel", "bevelWidth", "bevelHeight", "bevelRepeat", "bevelSpace", "bevelInside", "bevelBothSides"] {
            // A whole number the panel wrote is a JSON integer: the same number as the default's.
            let (got, want) = (&merged(&app, id)[key], &defaults[key]);
            assert!(got == want || got.as_f64().is_some_and(|v| Some(v) == want.as_f64()), "{key} of {id}: {got} for {want}");
        }
    }
    assert_eq!((merged(&app, a)["depth"].as_f64(), merged(&app, b)["depth"].as_f64()), (Some(70.0), Some(30.0)));
    assert_eq!(merged(&app, a)["rotationX"], 15.0);
    let nodes = controls(&mut app, &ctx);
    assert!(named(&nodes, "Reset").is_disabled(), "every bevel setting at its default: Reset is off");
    let history = undo_len(&app);
    click(&ctx, &mut app, at, show);
    assert_eq!(undo_len(&app), history, "an unavailable Reset records nothing");
}

/// The app with a graphic "Graphic" made from a red square beside the blue one, and the blue square
/// selected with Extrude & Bevel straight on; a context whose panel shows the Materials tab.
fn materials_app() -> (VectorcraftApp, egui::Context) {
    let mut app = app();
    let square = app.session.active().unwrap().selection.objects[0].0;
    let red = app.run("shape.rectangle", json!({"x": 300, "y": 300, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("paint.setFill", json!({"color": "#ff0000"})).unwrap();
    assert_eq!(app.run("threeD.addGraphic", json!({"ids": [red]})).unwrap()["name"], "Graphic");
    app.run("select.set", json!({"ids": [square]})).unwrap();
    set_type(&mut app, "extrude").unwrap();
    app.run("effect.setParams", json!({"index": 0, "item": null, "params": {"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0}})).unwrap();
    let ctx = crate::testing::ui_context();
    app.three_d.page = "materials".into();
    (app, ctx)
}

fn graphics(app: &VectorcraftApp) -> Value {
    effect(app).1.get("graphics").cloned().unwrap_or(Value::Null)
}

#[test]
fn the_materials_tab_lists_the_graphics_and_maps_one_in_one_undo_step() {
    let (mut app, ctx) = materials_app();
    ctx.enable_accesskit();
    let nodes = controls(&mut app, &ctx);
    named(&nodes, "Add as Single Graphic");
    named(&nodes, "Graphic");
    let history = undo_len(&app);
    super::run_action(&mut app, crate::dialogs::three_d_materials::Action::Map("Graphic".into()));
    assert_eq!(graphics(&app).as_array().map(Vec::len), Some(1));
    assert_eq!(graphics(&app)[0]["symbol"], "Graphic");
    assert_eq!(undo_len(&app), history + 1);
    // The mapped graphic is listed with its settings, and Remove Graphic takes it off.
    let nodes = controls(&mut app, &ctx);
    named(&nodes, "Remove Graphic");
    named(&nodes, "Scale");
    // Inflate keeps the graphic and lists it with its face and settings.
    set_type(&mut app, "inflate").unwrap();
    assert_eq!(graphics(&app).as_array().map(Vec::len), Some(1));
    let nodes = controls(&mut app, &ctx);
    named(&nodes, "Remove Graphic");
    named(&nodes, "Scale");
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui));
    out.textures_delta.clear();
    let text = crate::tests_labels::shapes_text(&out);
    assert!(text.contains("Face") && !text.contains("Graphics map onto"), "{text}");
}

#[test]
fn on_revolve_the_materials_tab_maps_onto_the_side_without_a_face_choice() {
    let (mut app, ctx) = materials_app();
    ctx.enable_accesskit();
    super::run_action(&mut app, crate::dialogs::three_d_materials::Action::Map("Graphic".into()));
    let drawn = |app: &mut VectorcraftApp| {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
        out.textures_delta.clear();
        crate::tests_labels::shapes_text(&out)
    };
    assert!(drawn(&mut app).contains("Face"));
    // Revolve keeps the graphic on its side and lists it without Face; a pick maps another.
    set_type(&mut app, "revolve").unwrap();
    assert_eq!(graphics(&app).as_array().map(Vec::len), Some(1));
    let history = undo_len(&app);
    super::run_action(&mut app, crate::dialogs::three_d_materials::Action::Map("Graphic".into()));
    assert_eq!(graphics(&app).as_array().map(Vec::len), Some(2));
    assert_eq!(undo_len(&app), history + 1);
    named(&controls(&mut app, &ctx), "Scale");
    let text = drawn(&mut app);
    assert!(text.contains("Mapped graphics") && !text.contains("Face"), "{text}");
}

#[test]
fn art_can_be_added_as_a_graphic_before_any_3d_effect() {
    let mut app = app();
    let history = undo_len(&app);
    super::run_action(&mut app, crate::dialogs::three_d_materials::Action::Add(None));
    assert_eq!(undo_len(&app), history + 1);
    assert_eq!(app.session.active().unwrap().doc.symbols.len(), 1);
    // The selection is still the square, not an instance.
    let id = app.session.active().unwrap().selection.objects[0];
    assert!(matches!(app.session.active().unwrap().doc.node(id).unwrap().kind, vectorcraft_doc::NodeKind::Path { .. }));
    assert!(painted_text(&mut app, show).contains("Add as Single Graphic"));
}

#[test]
fn three_d_graphics_accepts_artwork_only_from_its_document() {
    use crate::panel_drag::PanelDrag;
    use crate::tests::panel_harness::Panel;
    use craft::ui::contract::{ListDrag, ShowCollection as _};
    use craft::ui::list::{List, ListRow};
    use craft::ui::runtime::drag;

    fn with_artwork(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
        let st = app.session.active().unwrap();
        let payload = PanelDrag::Art { document: st.uid, ids: st.selection.objects.clone() };
        ui.allocate_ui(egui::vec2(ui.available_width(), 80.0), |ui| {
            let keys = craft::behavior::keyed(&[0_usize], |key: &usize| *key);
            List::<usize>::new("Artwork")
                .drag_payload(move |_: &ListDrag<usize>| Some(payload.clone()))
                .show_items(ui, &keys, &mut |_, _| ListRow::new("Dragged artwork"));
        });
        show(app, ui);
    }

    let mut app = app();
    let source = app.session.active().unwrap().uid;
    let first = app.session.active().unwrap().selection.objects[0];
    let add_existing = |app: &mut VectorcraftApp| app.run("threeD.addGraphic", json!({"name": "Existing Graphic"})).unwrap();
    add_existing(&mut app);
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    let second = app.run("shape.rectangle", json!({"x": 30, "y": 30, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap();
    add_existing(&mut app);
    assert_eq!(first.0, second, "the two documents have different artwork with the same local ID");
    assert!(app.session.set_active(0));

    let mut p = Panel::new(app, with_artwork);
    let from = p.rect("Dragged artwork").center();
    let to = p.rect("Existing Graphic").center();
    p.drag_hold_role("Dragged artwork", craft::ui::contract::Role::LIST_ITEM, to - from);
    assert!(drag::mark(p.ctx()).is_some(), "Graphics accepts the artwork before switching documents");
    assert!(p.app().session.set_active(1));
    p.frame();
    let carried = drag::active::<PanelDrag>(p.ctx());
    assert!(matches!(carried.as_deref(), Some(PanelDrag::Art { document, .. }) if *document == source));
    assert!(drag::mark(p.ctx()).is_none(), "Graphics withdraws the drop indicator when the artwork is foreign");
    p.release();
    assert_eq!(p.app().session.active().unwrap().doc.symbols.len(), 1, "the foreign drop adds no symbol from the active document's same-ID artwork");

    p.drag("Dragged artwork", "Existing Graphic", egui::Modifiers::NONE);
    assert_eq!(p.app().session.active().unwrap().doc.symbols.len(), 2, "a same-document drop still makes a graphic");
}

#[test]
fn a_graphics_handle_drags_it_over_its_face_in_one_undo_step() {
    let (mut app, _) = materials_app();
    app.run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
    let (app, mut fixture) = handles_fixture(app);
    let history = undo_len(&app.borrow());
    // Straight on, the graphic's handle is at the center of the square.
    let handle = screen_at(150.0, 150.0);
    let target = screen_at(170.0, 140.0);
    // A press away from the handle is left to the tool.
    let away = screen_at(110.0, 110.0);
    assert!(!grabs(&app, &mut fixture, away));
    let before = graphics(&app.borrow());
    fixture.drag_hold_at(handle, target - handle);
    assert_eq!(undo_len(&app.borrow()), history, "previewed while the pointer is down");
    assert_ne!(graphics(&app.borrow()), before, "the preview moved the graphic");
    fixture.release_at(target);
    assert_eq!(undo_len(&app.borrow()), history + 1);
    let g = graphics(&app.borrow())[0].clone();
    assert!((g["x"].as_f64().unwrap() - 20.0).abs() < 0.11 && (g["y"].as_f64().unwrap() + 10.0).abs() < 0.11, "{g}");
    // With another selected object whose graphics differ, the artboard shows no handles.
    let square = app.borrow().session.active().unwrap().selection.objects[0].0;
    let other = {
        let mut app = app.borrow_mut();
        let other = app.run("shape.rectangle", json!({"x": 400, "y": 400, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
        app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {}})).unwrap();
        app.run("select.set", json!({"ids": [square, other]})).unwrap();
        other
    };
    fixture.idle(1);
    assert!(!grabs(&app, &mut fixture, target));
    app.borrow_mut().run("select.set", json!({"ids": [square]})).unwrap();
    fixture.idle(1);
    assert!(grabs(&app, &mut fixture, target), "the selected object's graphic has its handle");
    // With the tab closed, the artboard shows no handles.
    app.borrow_mut().three_d.page = "rotation".into();
    fixture.idle(1);
    assert!(!grabs(&app, &mut fixture, target));
    app.borrow_mut().three_d.page = "materials".into();
    fixture.idle(1);
    // With its 3D effect hidden and a visible one after it, the hidden effect's graphics show no handles.
    {
        let mut app = app.borrow_mut();
        let st = app.session.active_mut().unwrap();
        let id = st.selection.objects[0];
        let n = std::sync::Arc::make_mut(&mut st.doc).node_mut(id).unwrap();
        n.appearance.effects[0].visible = false;
        n.appearance.effects.push(vectorcraft_effects::new_effect("threeD.extrude", &json!({})).unwrap());
        // An edit moves the revision, as a command's does.
        st.revision += 1;
    }
    fixture.idle(1);
    assert!(!grabs(&app, &mut fixture, target));
    assert_ne!(other, square);
}

/// The graphics' handles are placed on a frame built once for each revision of the document, and not at all for an
/// object without graphics.
#[test]
fn the_graphics_frame_is_built_once_for_each_revision() {
    let (app, _) = materials_app();
    let (app, mut fixture) = handles_fixture(app);
    let cached = |app: &Rc<RefCell<VectorcraftApp>>| app.borrow().three_d.frames.as_ref().map(|(_, frame)| std::sync::Arc::clone(frame));
    fixture.idle(1);
    assert!(cached(&app).is_none_or(|frame| frame.is_none()), "no graphics, no frame");
    app.borrow_mut().run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
    fixture.idle(1);
    let first = cached(&app).unwrap();
    assert!(first.is_some());
    fixture.idle(1);
    assert!(std::sync::Arc::ptr_eq(&first, &cached(&app).unwrap()), "the same revision reuses the frame");
    app.borrow_mut().run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
    fixture.idle(1);
    assert!(!std::sync::Arc::ptr_eq(&first, &cached(&app).unwrap()), "an edit builds it again");
}

/// `app` with the graphics' handles over an 800 × 600 canvas, and the handle the trial reads the app through.
fn handles_fixture(mut app: VectorcraftApp) -> (Rc<RefCell<VectorcraftApp>>, Fixture) {
    show_panel(&mut app);
    let app = Rc::new(RefCell::new(app));
    let drawn = Rc::clone(&app);
    // The handles are canvas controls: they show in the body of a canvas over the app's view, as the document's canvas
    // shows them.
    let mut fixture = Fixture::new(move |ui| {
        let mut app = drawn.borrow_mut();
        let room = ui.max_rect();
        let view = canvas_space(&app, room).view;
        craft::ui::canvas::Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
            if let Some(space) = craft::ui::canvas::canvas_space(&body.env) {
                SHOWN_SPACE.with(|shown| shown.set(Some(space)));
                handles::show(&mut app, ui, &space);
            }
        });
    });
    fixture.room(egui::vec2(800.0, 600.0)).settle();
    (app, fixture)
}

/// Shows the 3D and Materials panel: the graphics' handles show while it shows its Materials tab.
fn show_panel(app: &mut VectorcraftApp) {
    if !crate::panel_docking::visible(app, super::ID) {
        app.run("window.panel", json!({ "panel": super::ID })).unwrap();
    }
}

/// The canvas's mapping between document points and the screen over `rect`.
fn canvas_space(app: &VectorcraftApp, rect: egui::Rect) -> CanvasSpace {
    let view = app.view().cloned().unwrap_or_default();
    CanvasSpace::new(view.canvas(app.ui_scaling(), rect), 1.0)
}

thread_local! {
    /// The space of the canvas [`handles_fixture`] last showed, as its body reads it.
    static SHOWN_SPACE: std::cell::Cell<Option<CanvasSpace>> = const { std::cell::Cell::new(None) };
}

/// Where document point (`x`, `y`) is on the canvas of [`handles_fixture`], as the canvas shows it.
fn screen_at(x: f64, y: f64) -> egui::Pos2 {
    let space = SHOWN_SPACE.with(std::cell::Cell::get).expect("the canvas has shown");
    space.view.to_screen(vectorcraft_geom::Point::new(x, y))
}

/// Whether a drag from `from` moves a graphic (it records an undo step then); a drag that did is undone.
fn grabs(app: &Rc<RefCell<VectorcraftApp>>, fixture: &mut Fixture, from: egui::Pos2) -> bool {
    let history = undo_len(&app.borrow());
    fixture.drag_at(from, egui::vec2(12.0, 0.0));
    let moved = undo_len(&app.borrow()) > history;
    if moved {
        app.borrow_mut().run("edit.undo", json!({})).unwrap();
        fixture.idle(1);
    }
    moved
}

#[test]
fn a_geometry_effect_after_the_3d_effect_hides_the_graphics_handles() {
    let (mut app, _) = materials_app();
    app.run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
    let (app, mut fixture) = handles_fixture(app);
    let handle = screen_at(150.0, 150.0);
    let push_effect = |object: usize, id: &str, params: Value| {
        let mut guard = app.borrow_mut();
        let st = guard.session.active_mut().unwrap();
        let object = st.selection.objects[object];
        let n = std::sync::Arc::make_mut(&mut st.doc).node_mut(object).unwrap();
        n.appearance.effects.truncate(1);
        n.appearance.effects.push(vectorcraft_effects::new_effect(id, &params).unwrap());
    };
    // A Transform effect after the 3D effect draws the graphic 40 pt to the right of the frame's
    // place for it: the artboard shows no handle there.
    push_effect(0, "distort.transform", json!({"moveH": 40.0}));
    fixture.idle(1);
    assert!(!grabs(&app, &mut fixture, handle));
    // A Drop Shadow after it leaves the graphic where the frame places it: the handle shows.
    push_effect(0, "stylize.dropShadow", json!({}));
    fixture.idle(1);
    assert!(grabs(&app, &mut fixture, handle));
    // A second selected object with the same graphic and a Transform effect after its 3D effect:
    // a drag would move its graphic too, so the artboard shows no handles.
    {
        let mut app = app.borrow_mut();
        let square = app.session.active().unwrap().selection.objects[0].0;
        let other = app.run("shape.rectangle", json!({"x": 400, "y": 400, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
        app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0}}))
            .unwrap();
        app.run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
        app.run("select.set", json!({"ids": [square, other]})).unwrap();
    }
    fixture.idle(1);
    assert!(grabs(&app, &mut fixture, handle));
    push_effect(1, "distort.transform", json!({"moveH": 40.0}));
    fixture.idle(1);
    assert!(!grabs(&app, &mut fixture, handle));
}

#[test]
fn a_hidden_fill_shows_no_handles_for_its_graphics() {
    let (mut app, _) = materials_app();
    app.run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
    show_panel(&mut app);
    // The 3D effect, with its graphic, moves onto the square's fill.
    let fill = {
        let st = app.session.active_mut().unwrap();
        let id = st.selection.objects[0];
        let n = std::sync::Arc::make_mut(&mut st.doc).node_mut(id).unwrap();
        let fx = n.appearance.effects.remove(0);
        let fill = n.appearance.items.iter().position(|i| matches!(i, vectorcraft_doc::AppearanceItem::Fill(_))).unwrap();
        n.appearance.effects_mut(Some(fill)).unwrap().push(fx);
        fill
    };
    let shown = |app: &mut VectorcraftApp| handles::graphic_handles(app).is_some_and(|(_, _, handles)| !handles.is_empty());
    assert!(shown(&mut app), "the visible fill's graphic has a handle");
    {
        let st = app.session.active_mut().unwrap();
        let id = st.selection.objects[0];
        let n = std::sync::Arc::make_mut(&mut st.doc).node_mut(id).unwrap();
        if let Some(vectorcraft_doc::AppearanceItem::Fill(f)) = n.appearance.items.get_mut(fill) {
            f.visible = false;
        }
    }
    assert!(!shown(&mut app), "the hidden fill draws no graphic, so it has no handle");
}

#[test]
fn a_click_on_a_graphics_handle_records_no_undo_step() {
    let (mut app, _) = materials_app();
    app.run("threeD.mapGraphic", json!({"symbol": "Graphic", "x": 5.03})).unwrap();
    let (app, mut fixture) = handles_fixture(app);
    let history = undo_len(&app.borrow());
    let handle = screen_at(155.03, 150.0);
    fixture.click_at(handle);
    assert_eq!(undo_len(&app.borrow()), history);
    assert_eq!(graphics(&app.borrow())[0]["x"], json!(5.03));
}

#[test]
fn a_selection_change_during_a_handle_drag_ends_it() {
    let (mut app, _) = materials_app();
    app.run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
    let square = app.session.active().unwrap().selection.objects[0].0;
    // A second object with the same graphic.
    let other = app.run("shape.rectangle", json!({"x": 400, "y": 400, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0}}))
        .unwrap();
    app.run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
    app.run("select.set", json!({"ids": [square]})).unwrap();
    let graphic_of = |app: &VectorcraftApp, id: u64| {
        let n = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
        n.appearance.effects[0].params["graphics"][0].clone()
    };
    let before = graphic_of(&app, other);
    let (app, mut fixture) = handles_fixture(app);
    let (handle, target, further) = (screen_at(150.0, 150.0), screen_at(170.0, 140.0), screen_at(190.0, 130.0));
    fixture.drag_hold_at(handle, target - handle);
    // The selection changes to the other object while the pointer is down: the drag ends there.
    app.borrow_mut().run("select.set", json!({"ids": [other]})).unwrap();
    fixture.idle(1);
    fixture.release_at(further);
    assert_eq!(graphic_of(&app.borrow(), other), before, "the other object's graphic moved");
    let moved = graphic_of(&app.borrow(), square);
    assert!((moved["x"].as_f64().unwrap() - 20.0).abs() < 0.11, "{moved}");
}

/// Quick Actions › Expand as Wireframes shows for every object type, and turns the selected 3D
/// object into a group of stroked edges in one undo step.
#[test]
fn expand_as_wireframes_replaces_the_object_with_its_edges() {
    for kind in ["extrude", "inflate", "revolve", "plane"] {
        let mut app = app();
        set_type(&mut app, kind).unwrap();
        let text = painted_text(&mut app, show);
        assert!(text.contains("Quick Actions") && text.contains("Expand as Wireframes"), "{kind}: {text}");
        let id = app.session.active().unwrap().selection.objects[0];
        let history = undo_len(&app);
        super::expand_as_wireframes(&mut app).unwrap();
        assert_eq!(undo_len(&app), history + 1, "{kind}");
        let st = app.session.active().unwrap();
        assert!(matches!(st.doc.node(id).unwrap().kind, vectorcraft_doc::NodeKind::Group { .. }), "{kind}");
        assert_eq!(st.selection.objects, vec![id], "{kind}");
        // The group has no 3D effect: the panel offers the object types again.
        assert!(target(&app).is_none(), "{kind}");
    }
    // Without a 3D effect in the selection, nothing changes.
    let mut app = app();
    let history = undo_len(&app);
    assert!(super::expand_as_wireframes(&mut app).is_err());
    assert_eq!(undo_len(&app), history);
}

/// An app with a square given Extrude & Bevel with `params`, and the panel on its Lighting page.
fn lighting(params: Value) -> (Rc<RefCell<VectorcraftApp>>, Fixture) {
    let mut app = app();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": params})).unwrap();
    app.three_d.page = "lighting".into();
    let (app, mut fixture) = shown(app);
    // As tall as a docked panel: the button below the sphere sits past a 300-point room's bottom.
    fixture.room(egui::vec2(400.0, crate::tests::panel_harness::ROOM.y)).settle();
    (app, fixture)
}

/// The lighting widget's part for light `n` (from 1): its screen-reader name and the control to press.
fn light(fixture: &Fixture, n: usize) -> (String, craft::ui::testing::Target) {
    let tree = fixture.tree();
    let prefix = format!("Light {n}:");
    let key = tree
        .keys()
        .into_iter()
        .find(|key| tree.name_of(*key).is_some_and(|name| name.starts_with(&prefix)))
        .unwrap_or_else(|| panic!("no lighting widget part for light {n}"));
    (tree.name_of(key).unwrap_or_default().to_string(), fixture.target(key).unwrap())
}

fn merged_light(app: &VectorcraftApp) -> (f64, f64) {
    let (id, params) = effect(app);
    let merged = vectorcraft_effects::merged_params(&id, &params);
    (merged["lightRotation"].as_f64().unwrap(), merged["lightHeight"].as_f64().unwrap())
}

/// A key press of `key`, for a frame that carries several.
fn key_press(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

/// Dragging a light in the lighting widget previews its new Rotation and Height while the pointer
/// is down and records one undo step when it is released; a press without a drag changes nothing.
#[test]
fn the_lighting_widget_drags_a_light_in_one_undo_step() {
    let (app, mut fixture) = lighting(json!({}));
    let (name, at) = light(&fixture, 1);
    assert_eq!(name, "Light 1: Rotation 125.3°, Height 30.0°");
    let start = merged_light(&app.borrow());
    let history = undo_len(&app.borrow());
    let by = egui::vec2(30.0, 20.0);
    fixture.drag_hold_at(at.center(), by);
    assert_ne!(merged_light(&app.borrow()), start, "the light follows the pointer while it is down");
    assert_eq!(undo_len(&app.borrow()), history, "nothing is recorded while the pointer is down");
    fixture.release_at(at.center() + by);
    assert_eq!(undo_len(&app.borrow()), history + 1);
    assert_ne!(merged_light(&app.borrow()), start);
    app.borrow_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(merged_light(&app.borrow()), start);
    // A press that does not move the light changes nothing.
    fixture.idle(1);
    let (_, at) = light(&fixture, 1);
    fixture.click(&at);
    assert_eq!(merged_light(&app.borrow()), start);
    assert_eq!(undo_len(&app.borrow()), history, "the undone step stays undone");
}

/// With objects whose lights differ in Height, an arrow key that turns the light changes only
/// the Rotation, and each object keeps its own Height.
#[test]
fn a_key_leaves_the_other_setting_as_each_selected_effect_has_it() {
    let mut app = app();
    let a = app.session.active().unwrap().selection.objects[0].0;
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"lightRotation": 0.0, "lightHeight": 20.0}})).unwrap();
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"lightRotation": 0.0, "lightHeight": 40.0}})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.three_d.page = "lighting".into();
    let (app, mut fixture) = shown(app);
    let (_, at) = light(&fixture, 1);
    fixture.click(&at).key(egui::Key::ArrowLeft, egui::Modifiers::NONE);
    let settings = |id: u64| {
        let guard = app.borrow();
        let n = guard.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
        let merged = vectorcraft_effects::merged_params("threeD.extrude", &n.appearance.effects[0].params);
        (merged["lightRotation"].as_f64().unwrap(), merged["lightHeight"].as_f64().unwrap())
    };
    assert_eq!((settings(a), settings(b)), ((1.0, 20.0), (1.0, 40.0)));
}

/// A press on a light selects it in the list without an undo step, and the button below the sphere
/// moves only the selected light to the back of the object, then back to the front.
#[test]
fn the_lighting_widget_selects_a_light_and_moves_it_behind_the_object() {
    let spec = |rotation: f64| json!({"color": "#ffffff", "intensity": 80.0, "rotation": rotation, "height": 30.0, "softness": 0.0, "behind": false});
    let (app, mut fixture) = lighting(json!({"lights": [spec(0.0), spec(180.0)]}));
    let history = undo_len(&app.borrow());
    let (_, second) = light(&fixture, 2);
    fixture.click(&second);
    assert_eq!(undo_len(&app.borrow()), history, "a press on a light changes nothing");
    let to_back = fixture.by_label("Move Light to Back").unwrap();
    fixture.click(&to_back);
    assert_eq!(undo_len(&app.borrow()), history + 1);
    let behind = || effect(&app.borrow()).1["lights"].as_array().unwrap().iter().map(|l| l["behind"].clone()).collect::<Vec<_>>();
    assert_eq!(behind(), [json!(false), json!(true)]);
    let (name, _) = light(&fixture, 2);
    assert_eq!(name, "Light 2: Rotation 180.0°, Height 30.0°, Light behind the object");
    let to_front = fixture.by_label("Move Light to Front").unwrap();
    fixture.click(&to_front);
    assert_eq!(behind(), [json!(false), json!(false)]);
    assert_eq!(undo_len(&app.borrow()), history + 2);
}

/// The lights' list, buttons and sphere are one group and the selected light's settings another, a stack gap apart;
/// the rows inside the first group are a row gap apart.
#[test]
fn the_light_settings_are_a_stack_gap_below_the_sphere_and_its_button() {
    let gaps = Rc::new(std::cell::Cell::new((0.0, 0.0)));
    let seen = Rc::clone(&gaps);
    Fixture::new(move |ui| {
        let gap = craft::ui::runtime::env::Env::of(ui).tokens.space.gap;
        seen.set((gap.row, gap.stack));
    })
    .frame(vec![]);
    let (row, stack) = gaps.get();
    assert!(row > 0.0 && stack > row, "the interface resolved its gaps: row {row}, stack {stack}");
    let (_app, fixture) = lighting(json!({}));
    let tree = fixture.tree();
    let rect = |name: &str, role: craft::ui::contract::Role| {
        tree.keys()
            .into_iter()
            .find(|key| tree.name_of(*key) == Some(name) && tree.role_of(*key) == Some(role))
            .and_then(|key| fixture.target(key))
            .unwrap_or_else(|| panic!("the panel shows {name}"))
            .rect
    };
    let (sphere, button, settings) = (
        rect("Lights", craft::ui::contract::Role::GROUP),
        rect("Move Light to Back", craft::ui::contract::Role::BUTTON),
        rect("Light Settings", craft::ui::contract::Role::GROUP),
    );
    let below_button = settings.min.y - button.max.y;
    assert!((below_button - stack).abs() < 0.01, "the settings are {below_button} below the button, the stack gap is {stack}");
    let below_sphere = button.min.y - sphere.max.y;
    assert!((below_sphere - row).abs() < 0.01, "the button is {below_sphere} below the sphere, the row gap is {row}");
}

/// A light the pointer took has the keyboard focus: the arrow keys raise and lower it and turn it
/// around the object, 1° a press or 15° with Shift, changing only the setting each press moves, one
/// undo step a frame, and a press at a limit records nothing.
#[test]
fn the_arrow_keys_move_the_focused_light() {
    let (app, mut fixture) = lighting(json!({"lightRotation": 10.04, "lightHeight": 89.5}));
    let (_, at) = light(&fixture, 1);
    fixture.click(&at);
    let history = undo_len(&app.borrow());
    fixture.key(egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(merged_light(&app.borrow()), (10.04, 90.0), "Up leaves the Rotation as it is");
    assert_eq!(undo_len(&app.borrow()), history + 1);
    fixture.key(egui::Key::ArrowUp, egui::Modifiers::NONE);
    assert_eq!(merged_light(&app.borrow()), (10.04, 90.0));
    assert_eq!(undo_len(&app.borrow()), history + 1, "Height stays at 90° and nothing is recorded");
    // Down then Up in one frame, in that order: back at 90°.
    fixture.frame(vec![key_press(egui::Key::ArrowDown, egui::Modifiers::NONE), key_press(egui::Key::ArrowUp, egui::Modifiers::NONE)]).idle(1);
    assert_eq!(merged_light(&app.borrow()), (10.04, 90.0));
    assert_eq!(undo_len(&app.borrow()), history + 1);
    // Presses that arrive in one frame all move the light, as one undo step.
    let presses = vec![
        key_press(egui::Key::ArrowLeft, egui::Modifiers::SHIFT),
        key_press(egui::Key::ArrowDown, egui::Modifiers::NONE),
        key_press(egui::Key::ArrowDown, egui::Modifiers::NONE),
    ];
    fixture.frame(presses).idle(1);
    assert_eq!(merged_light(&app.borrow()), (25.0, 88.0));
    assert_eq!(undo_len(&app.borrow()), history + 2);
}

fn assets(app: &mut VectorcraftApp) -> Vec<Value> {
    app.run("assets.list", json!({})).unwrap()["assets"].as_array().unwrap().clone()
}

/// The panel always shows Export 3D Object; it adds only the selected objects that have a 3D
/// effect to Asset Export as 3D assets, in one undo step, selects their assets there and shows the
/// Asset Export panel. Run again, it records nothing and still selects them.
#[test]
fn export_3d_object_adds_the_selected_3d_objects_to_asset_export() {
    let mut app = app();
    assert!(painted_text(&mut app, show).contains("Export 3D Object"));
    // Without a 3D object: the button is dimmed, and an error, nothing recorded, no asset.
    let history = undo_len(&app);
    assert!(!super::can_export_3d(&app));
    assert!(super::export_3d_object(&mut app).is_err());
    assert_eq!((undo_len(&app), assets(&mut app).len()), (history, 0));
    let a = app.session.active().unwrap().selection.objects[0].0;
    set_type(&mut app, "extrude").unwrap();
    app.run("layer.setProps", json!({"id": a, "name": "Box"})).unwrap();
    assert!(painted_text(&mut app, show).contains("Export 3D Object"));
    let plain = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    // A hidden 3D effect draws nothing to export: the object is left out like one without.
    let hidden = app.run("shape.rectangle", json!({"x": 100, "y": 250, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null})).unwrap();
    app.run("effect.setParams", json!({"index": 0, "item": null, "visible": false})).unwrap();
    assert!(!super::can_export_3d(&app));
    assert!(super::export_3d_object(&mut app).is_err());
    app.run("select.set", json!({"ids": [plain, a, hidden]})).unwrap();
    assert!(super::can_export_3d(&app));
    assert!(!crate::panel_docking::visible(&app, crate::panels::asset_export::ID));
    let history = undo_len(&app);
    super::export_3d_object(&mut app).unwrap();
    assert_eq!(undo_len(&app), history + 1);
    let list = assets(&mut app);
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!((list[0]["name"].clone(), list[0]["format3D"].clone()), (json!("Box"), json!("usda")));
    let asset = list[0]["id"].as_u64().unwrap();
    assert_eq!(crate::panels::asset_export::selected(&app), [asset]);
    assert!(crate::panel_docking::visible(&app, crate::panels::asset_export::ID));
    // Already an asset: no undo step, the same asset selected again, the panel left showing.
    crate::panels::asset_export::doc_panel(&mut app).unwrap().selection = Default::default();
    super::export_3d_object(&mut app).unwrap();
    assert_eq!((undo_len(&app), assets(&mut app).len()), (history + 1, 1));
    assert_eq!(crate::panels::asset_export::selected(&app), [asset]);
    assert!(crate::panel_docking::visible(&app, crate::panels::asset_export::ID));
    // Undo takes the asset away.
    app.run("edit.undo", json!({})).unwrap();
    assert!(assets(&mut app).is_empty());
}

/// A selected 3D object inside another selected 3D object is not collected on its own: the outer
/// object's asset holds it.
#[test]
fn export_3d_object_leaves_out_a_3d_object_inside_another() {
    let mut app = app();
    let inner = app.session.active().unwrap().selection.objects[0].0;
    set_type(&mut app, "extrude").unwrap();
    let other = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [inner, other]})).unwrap();
    let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [group]})).unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null})).unwrap();
    app.run("select.set", json!({"ids": [inner, group]})).unwrap();
    let selected: Vec<u64> = app.session.active().unwrap().selection.objects.iter().map(|id| id.0).collect();
    assert!(selected.contains(&inner) && selected.contains(&group), "{selected:?}");
    super::export_3d_object(&mut app).unwrap();
    let list = assets(&mut app);
    assert_eq!(list.iter().map(|a| a["nodes"].clone()).collect::<Vec<_>>(), [json!([group])]);
}

/// A 3D effect on a group's fill draws nothing to export: the button is dimmed for that group
/// alone, and beside a 3D object the group is left out instead of refusing them both.
#[test]
fn export_3d_object_leaves_out_a_group_with_a_3d_fill() {
    let mut app = app();
    let a = app.session.active().unwrap().selection.objects[0].0;
    let other = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [other]})).unwrap();
    let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [group]})).unwrap();
    app.run("appearance.addFill", json!({})).unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude"})).unwrap();
    // `effect.apply` puts a group's 3D effect on the group itself; a file can hold one on its fill.
    app.session
        .edit("fill", |d, _| {
            let n = d.node_mut(vectorcraft_doc::NodeId(group)).unwrap();
            let fx = n.appearance.effects.remove(0);
            n.appearance.items[0].effects_mut().push(fx);
            Ok(())
        })
        .unwrap();
    assert!(
        vectorcraft_effects::has_solid(app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(group)).unwrap()),
        "the fill holds the effect"
    );
    assert!(!super::can_export_3d(&app));
    app.run("select.set", json!({"ids": [a]})).unwrap();
    set_type(&mut app, "extrude").unwrap();
    app.run("select.set", json!({"ids": [a, group]})).unwrap();
    assert!(super::can_export_3d(&app));
    super::export_3d_object(&mut app).unwrap();
    let list = assets(&mut app);
    assert_eq!(list.iter().map(|a| a["nodes"].clone()).collect::<Vec<_>>(), [json!([a])]);
}

/// Whether Export 3D Object is available as the panel reads it ([`super::can_export_3d_shown`]),
/// checked against the state worked out from scratch ([`super::can_export_3d`]); with a document
/// open, the panel keeps it for the document's uid and revision.
fn exportable(app: &mut VectorcraftApp, why: &str) -> bool {
    let fresh = super::can_export_3d(app);
    assert_eq!(super::can_export_3d_shown(app), fresh, "{why}");
    if let Some(key) = app.session.active().map(|st| (st.uid, st.revision)) {
        assert_eq!(app.canvas.three_d_exportable, Some((key, fresh)), "{why}");
    }
    fresh
}

/// The Export 3D Object state the panel keeps for a revision equals the state worked out from
/// scratch: with no document and no selection, for plain paths, with a 3D object selected first or
/// last, for a path of a compound path whose compound path has the effect, after the effect is
/// hidden, Undo and Redo, in another document and back, and for a group whose 3D effect moves onto
/// its fill.
#[test]
fn export_3d_object_state_kept_for_a_revision_equals_a_fresh_one() {
    let mut app = VectorcraftApp::new(Session::new(), Services::default());
    assert!(!exportable(&mut app, "no document"));
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    assert!(!exportable(&mut app, "no selection"));
    let rect = |app: &mut VectorcraftApp, x: f64| {
        app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap()
    };
    let (p, q) = (rect(&mut app, 10.0), rect(&mut app, 40.0));
    app.run("select.set", json!({"ids": [p, q]})).unwrap();
    assert!(!exportable(&mut app, "plain paths"));
    let solid = rect(&mut app, 70.0);
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null})).unwrap();
    app.run("select.set", json!({"ids": [solid, p, q]})).unwrap();
    assert!(exportable(&mut app, "a 3D object first"));
    app.run("select.set", json!({"ids": [p, q]})).unwrap();
    assert!(!exportable(&mut app, "plain paths again"));
    app.run("select.set", json!({"ids": [p, q, solid]})).unwrap();
    assert!(exportable(&mut app, "a 3D object last"));
    let (a, b) = (rect(&mut app, 100.0), rect(&mut app, 110.0));
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    app.run("object.compoundPath.make", json!({})).unwrap();
    let compound = app.session.active().unwrap().selection.objects[0].0;
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null})).unwrap();
    let member = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(compound)).unwrap().children().unwrap()[0].id.0;
    app.run("select.set", json!({"ids": [p, q]})).unwrap();
    assert!(!exportable(&mut app, "plain paths before the compound path's member"));
    app.run("select.set", json!({"ids": [p, member]})).unwrap();
    assert!(exportable(&mut app, "a path of a 3D compound path"));
    // An edit that leaves the selection as it is.
    app.run("effect.setParams", json!({"ids": [compound], "index": 0, "item": null, "visible": false})).unwrap();
    assert!(!exportable(&mut app, "the compound path's 3D effect hidden"));
    app.run("edit.undo", json!({})).unwrap();
    assert!(exportable(&mut app, "Undo"));
    app.run("edit.redo", json!({})).unwrap();
    assert!(!exportable(&mut app, "Redo"));
    app.run("edit.undo", json!({})).unwrap();
    assert!(exportable(&mut app, "Undo again"));
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    assert!(!exportable(&mut app, "a new document"));
    app.session.set_active(0);
    assert!(exportable(&mut app, "back to the first document"));
    // A group's own 3D effect exports; the same effect on its fill does not.
    app.run("select.set", json!({"ids": [p, q]})).unwrap();
    let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [group]})).unwrap();
    app.run("appearance.addFill", json!({})).unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude"})).unwrap();
    assert!(exportable(&mut app, "a group with its own 3D effect"));
    app.session
        .edit("fill", |d, _| {
            let n = d.node_mut(vectorcraft_doc::NodeId(group)).unwrap();
            let fx = n.appearance.effects.remove(0);
            n.appearance.items[0].effects_mut().push(fx);
            Ok(())
        })
        .unwrap();
    assert!(!exportable(&mut app, "a group with a 3D effect on its fill"));
}

/// A frame of the panel works out the Export 3D Object state once and keeps it for the document's
/// revision: later frames read it, and a new revision works it out again.
#[test]
fn export_3d_object_state_is_worked_out_once_per_revision() {
    let key = |app: &VectorcraftApp| app.session.active().map(|st| (st.uid, st.revision)).unwrap();
    let mut app = app();
    painted_text(&mut app, show);
    assert_eq!(app.canvas.three_d_exportable, Some((key(&app), false)));
    // A wrong answer planted for this revision is what the panel reads: nothing works it out again.
    app.canvas.three_d_exportable = Some((key(&app), true));
    assert!(super::can_export_3d_shown(&mut app));
    painted_text(&mut app, show);
    assert_eq!(app.canvas.three_d_exportable, Some((key(&app), true)));
    app.run("select.none", json!({})).unwrap();
    painted_text(&mut app, show);
    assert_eq!(app.canvas.three_d_exportable, Some((key(&app), false)));
}
