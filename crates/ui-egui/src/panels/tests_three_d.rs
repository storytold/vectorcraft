//! The 3D and Materials panel drawn headless: its Window menu entry, the object types applying or
//! changing a 3D effect, the dialogs' controls for the applied effect, and one undo step for a
//! drag or a typed value.
#![allow(clippy::unwrap_used, clippy::panic)]

use serde_json::{Map, Value, json};
use vectorcraft_engine::Session;

use super::{ID, Target, changed_settings, send, set_type, show, target};
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

#[test]
fn the_window_menu_opens_the_panel() {
    let mut app = app();
    assert!(crate::state::ICON_PANELS.iter().any(|(id, label, _)| *id == ID && *label == "3D and Materials"));
    app.run("window.panel", json!({"panel": ID})).unwrap();
    assert!(crate::panel_docking::showing(&app.ui, ID));
    app.run("window.panel", json!({"panel": "3D and Materials"})).unwrap();
}

#[test]
fn without_a_3d_effect_the_panel_offers_the_object_types() {
    let mut app = app();
    let text = painted_text(&mut app, show);
    for label in ["Plane", "Extrude", "Revolve", "Inflate", "Select objects, then pick an object type"] {
        assert!(text.contains(label), "{label} in {text}");
    }
    app.run("select.none", json!({})).unwrap();
    let text = painted_text(&mut app, show);
    assert!(text.contains("Select objects, then pick an object type"), "{text}");
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
    let ctx = egui::Context::default();
    let history = undo_len(&app);
    send(&mut app, &ctx, Map::from_iter([("depth".to_string(), json!(90.0))]), true);
    assert!(app.session.in_interaction());
    app.session.active_mut().unwrap().selection.objects.clear();
    ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui)).textures_delta.clear();
    assert!(!app.session.in_interaction());
    assert_eq!(undo_len(&app), history + 1);
}

#[test]
fn with_nothing_selected_an_object_type_changes_nothing() {
    let mut app = app();
    app.run("select.none", json!({})).unwrap();
    let history = undo_len(&app);
    assert!(set_type(&mut app, "plane").is_err());
    assert_eq!(undo_len(&app), history);
}

#[test]
fn the_panel_shows_the_dialog_controls_of_each_type() {
    let mut app = app();
    for (kind, label) in [("extrude", "Depth"), ("inflate", "Volume"), ("revolve", "Revolve angle"), ("plane", "Rotation X")] {
        set_type(&mut app, kind).unwrap();
        let text = painted_text(&mut app, show);
        assert!(text.contains(label), "{kind}: {label} in {text}");
        assert!(text.contains("Lighting"), "{kind}: {text}");
        // Drawing the panel is no edit.
        let history = undo_len(&app);
        painted_text(&mut app, show);
        assert_eq!(undo_len(&app), history, "{kind}");
    }
    // The dialog's artboard-axis toggle has no artboard gizmo in the panel.
    set_type(&mut app, "revolve").unwrap();
    assert!(!painted_text(&mut app, show).contains("Show axis"));
}

#[test]
fn a_drag_previews_and_records_one_undo_step_when_released() {
    let mut app = app();
    set_type(&mut app, "extrude").unwrap();
    let ctx = egui::Context::default();
    let history = undo_len(&app);
    for depth in [60.0, 80.0, 120.0] {
        send(&mut app, &ctx, Map::from_iter([("depth".to_string(), json!(depth))]), true);
        assert_eq!(effect(&app).1["depth"], depth);
    }
    // Another setting in the same drag keeps the earlier one.
    send(&mut app, &ctx, Map::from_iter([("twist".to_string(), json!(45.0))]), true);
    assert_eq!(effect(&app).1["depth"], 120.0);
    assert_eq!(undo_len(&app), history);
    // Released without a change in the last frame: the drag commits.
    send(&mut app, &ctx, Map::new(), false);
    assert_eq!(undo_len(&app), history + 1);
    let (_, params) = effect(&app);
    assert_eq!((params["depth"].clone(), params["twist"].clone()), (json!(120.0), json!(45.0)));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(effect(&app).1["depth"], 50.0);
    // A typed value is one undo step at once.
    send(&mut app, &ctx, Map::from_iter([("taper".to_string(), json!(60.0))]), false);
    assert_eq!(effect(&app).1["taper"], 60.0);
    assert_eq!(undo_len(&app), history + 1);
    // Nothing changed and no drag: nothing runs.
    send(&mut app, &ctx, Map::new(), false);
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
    let ctx = egui::Context::default();
    // `bevel` is an Extrude & Bevel setting only; `depth` both effects take.
    send(&mut app, &ctx, Map::from_iter([("depth".to_string(), json!(75.0)), ("bevel".to_string(), json!("classic"))]), true);
    send(&mut app, &ctx, Map::new(), false);
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
    let mut fields = target(&app).map(|t| vectorcraft_effects::merged_params(&t.effect, &t.params)).unwrap().as_object().cloned().unwrap();
    super::mark_mixed(&app, &mut fields);
    assert_eq!(fields["depth"], Value::Null);
    assert_eq!(fields["twist"], 0.0);
    // Resetting Depth to its default (the first object's value) is a change for both objects.
    let before = Value::Object(fields.clone());
    fields.insert("depth".into(), json!(50.0));
    let changed = changed_settings(&before, &Value::Object(fields));
    assert_eq!(Value::Object(changed.clone()), json!({"depth": 50.0}));
    send(&mut app, &egui::Context::default(), changed, false);
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
    send(&mut app, &egui::Context::default(), Map::from_iter([("lights".to_string(), three.clone())]), false);
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
    let ctx = egui::Context::default();
    send(&mut app, &ctx, Map::from_iter([("depth".to_string(), json!(90.0))]), true);
    // The selection changes during the drag: the drag on `a` is committed, and `b` starts its own.
    app.session.active_mut().unwrap().selection.objects = vec![b];
    send(&mut app, &ctx, Map::from_iter([("depth".to_string(), json!(70.0))]), true);
    send(&mut app, &ctx, Map::new(), false);
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
    let ctx = egui::Context::default();
    // The first object's list passes through the second's value during the drag.
    send(&mut app, &ctx, Map::from_iter([("lights".to_string(), one("#00ff00"))]), true);
    send(&mut app, &ctx, Map::from_iter([("lights".to_string(), one("#0000ff"))]), true);
    send(&mut app, &ctx, Map::new(), false);
    let lights =
        |id: u64| app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.effects[0].params["lights"].clone();
    assert_eq!(lights(a), one("#0000ff"));
    assert_eq!(lights(b), one("#00ff00"));
}

#[test]
fn a_drag_ends_when_the_panel_is_no_longer_drawn() {
    let mut app = app();
    set_type(&mut app, "extrude").unwrap();
    let ctx = egui::Context::default();
    let history = undo_len(&app);
    ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui)).textures_delta.clear();
    send(&mut app, &ctx, Map::from_iter([("depth".to_string(), json!(90.0))]), true);
    // A pass that draws the panel keeps the drag; the first pass that does not draw it ends it.
    ctx.run_ui(egui::RawInput::default(), |ui| {
        // Drawn with the pointer still down: only the mark `show` leaves.
        crate::panels::set_pstate(ui.ctx(), super::DRAWN, Some(super::root_pass(ui.ctx())));
        super::end_stale_drag(&mut app, ui.ctx());
    })
    .textures_delta
    .clear();
    assert!(app.session.in_interaction());
    ctx.run_ui(egui::RawInput::default(), |ui| super::end_stale_drag(&mut app, ui.ctx())).textures_delta.clear();
    assert!(!app.session.in_interaction());
    assert_eq!(undo_len(&app), history + 1);
    assert_eq!(effect(&app).1["depth"], 90.0);
}

#[test]
fn changed_settings_lists_only_new_values() {
    let before = json!({"depth": 50.0, "cap": true, "bevel": "none"});
    let after = json!({"depth": 60.0, "cap": true, "bevel": "classic", "twist": 0.0});
    let changed = changed_settings(&before, &after);
    assert_eq!(Value::Object(changed), json!({"depth": 60.0, "bevel": "classic", "twist": 0.0}));
    assert!(changed_settings(&after, &after).is_empty());
}
