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
    let ctx = egui::Context::default();
    let history = undo_len(&app);
    super::set_wireframe(&mut app, &ctx, true);
    assert_eq!(undo_len(&app), history + 1);
    assert_eq!((mode(&app, a).as_deref(), mode(&app, b).as_deref()), (Some("wireframe"), Some("wireframe")));
    assert_eq!(mode(&app, c), None, "the classic effect takes no render settings");
    assert!(super::wireframe_on(&target(&app).unwrap()));
    // The object type changes in place and keeps the mode.
    app.run("select.set", json!({"ids": [a]})).unwrap();
    set_type(&mut app, "inflate").unwrap();
    assert_eq!(mode(&app, a).as_deref(), Some("wireframe"));
    let history = undo_len(&app);
    super::set_wireframe(&mut app, &ctx, false);
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

/// The panel's controls, drawn once with the accessibility tree on.
fn controls(app: &mut VectorcraftApp, ctx: &egui::Context) -> Vec<egui::accesskit::Node> {
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui));
    out.textures_delta.clear();
    out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default()
}

fn named<'a>(nodes: &'a [egui::accesskit::Node], label: &str) -> &'a egui::accesskit::Node {
    nodes.iter().find(|n| n.label() == Some(label)).unwrap_or_else(|| panic!("no {label:?}"))
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
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
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

/// Revolve's Cap checkbox shows a dash when the selected effects disagree on it.
#[test]
fn a_mixed_revolve_cap_shows_a_dash() {
    let mut app = app();
    let a = app.session.active().unwrap().selection.objects[0].0;
    set_type(&mut app, "revolve").unwrap();
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.revolve", "item": null, "params": {"cap": false}})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.enable_accesskit();
    let history = undo_len(&app);
    assert_eq!(named(&controls(&mut app, &ctx), "Cap").toggled(), Some(egui::accesskit::Toggled::Mixed));
    assert_eq!(undo_len(&app), history);
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
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::panels::set_pstate(&ctx, super::UI_STATE, Map::from_iter([("__tab".to_string(), json!("materials"))]));
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
    super::run_action(&mut app, &ctx, crate::dialogs::three_d_materials::Action::Map("Graphic".into()));
    assert_eq!(graphics(&app).as_array().map(Vec::len), Some(1));
    assert_eq!(graphics(&app)[0]["symbol"], "Graphic");
    assert_eq!(undo_len(&app), history + 1);
    // The mapped graphic is listed with its settings, and Remove Graphic takes it off.
    let nodes = controls(&mut app, &ctx);
    named(&nodes, "Remove Graphic");
    named(&nodes, "Scale");
    // Inflate takes no graphics.
    set_type(&mut app, "inflate").unwrap();
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| show(&mut app, ui));
    out.textures_delta.clear();
    let text = crate::tests_labels::shapes_text(&out);
    assert!(text.contains("Graphics map onto the Plane, Extrude and Revolve object types"), "{text}");
}

#[test]
fn on_revolve_the_materials_tab_maps_onto_the_side_without_a_face_choice() {
    let (mut app, ctx) = materials_app();
    ctx.enable_accesskit();
    super::run_action(&mut app, &ctx, crate::dialogs::three_d_materials::Action::Map("Graphic".into()));
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
    super::run_action(&mut app, &ctx, crate::dialogs::three_d_materials::Action::Map("Graphic".into()));
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
    let ctx = egui::Context::default();
    super::run_action(&mut app, &ctx, crate::dialogs::three_d_materials::Action::Add(None));
    assert_eq!(undo_len(&app), history + 1);
    assert_eq!(app.session.active().unwrap().doc.symbols.len(), 1);
    // The selection is still the square, not an instance.
    let id = app.session.active().unwrap().selection.objects[0];
    assert!(matches!(app.session.active().unwrap().doc.node(id).unwrap().kind, vectorcraft_doc::NodeKind::Path { .. }));
    assert!(painted_text(&mut app, show).contains("Add as Single Graphic"));
}

#[test]
fn a_graphics_handle_drags_it_over_its_face_in_one_undo_step() {
    let (mut app, ctx) = materials_app();
    app.run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
    let history = undo_len(&app);
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
    let xf = crate::canvas::Xf::new(rect, &app.view().cloned().unwrap_or_default());
    // Straight on, the graphic's handle is at the center of the square.
    let handle = xf.to_screen(vectorcraft_geom::Point::new(150.0, 150.0));
    let target = xf.to_screen(vectorcraft_geom::Point::new(170.0, 140.0));
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut taken = false;
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            crate::panels::set_pstate(ui.ctx(), super::DRAWN, Some(super::root_pass(ui.ctx())));
            let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
            taken = super::graphic_handles_input(app, ui, &resp, rect);
        })
        .textures_delta
        .clear();
        taken
    };
    let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    // A press away from the handle is left to the tool.
    let away = xf.to_screen(vectorcraft_geom::Point::new(110.0, 110.0));
    frame(&mut app, vec![egui::Event::PointerMoved(away)]);
    assert!(!frame(&mut app, vec![button(away, true)]));
    frame(&mut app, vec![button(away, false)]);
    frame(&mut app, vec![egui::Event::PointerMoved(handle)]);
    assert!(frame(&mut app, vec![button(handle, true)]));
    assert!(frame(&mut app, vec![egui::Event::PointerMoved(target)]));
    assert_eq!(undo_len(&app), history, "previewed while the pointer is down");
    assert!(frame(&mut app, vec![button(target, false)]));
    assert_eq!(undo_len(&app), history + 1);
    let g = &graphics(&app)[0];
    assert!((g["x"].as_f64().unwrap() - 20.0).abs() < 0.11 && (g["y"].as_f64().unwrap() + 10.0).abs() < 0.11, "{g}");
    // With another selected object whose graphics differ, the artboard shows no handles.
    let square = app.session.active().unwrap().selection.objects[0].0;
    let other = app.run("shape.rectangle", json!({"x": 400, "y": 400, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {}})).unwrap();
    app.run("select.set", json!({"ids": [square, other]})).unwrap();
    frame(&mut app, vec![egui::Event::PointerMoved(target)]);
    assert!(!frame(&mut app, vec![button(target, true)]));
    frame(&mut app, vec![button(target, false)]);
    app.run("select.set", json!({"ids": [square]})).unwrap();
    // With its 3D effect hidden and a visible one after it, the hidden effect's graphics show no
    // handles.
    {
        let st = app.session.active_mut().unwrap();
        let id = st.selection.objects[0];
        let n = std::sync::Arc::make_mut(&mut st.doc).node_mut(id).unwrap();
        n.appearance.effects[0].visible = false;
        n.appearance.effects.push(vectorcraft_effects::new_effect("threeD.extrude", &json!({})).unwrap());
    }
    let mut shown = true;
    ctx.run_ui(egui::RawInput::default(), |ui| {
        crate::panels::set_pstate(ui.ctx(), super::DRAWN, Some(super::root_pass(ui.ctx())));
        shown = super::graphic_handles(&app, ui.ctx()).is_some();
    })
    .textures_delta
    .clear();
    assert!(!shown);
    // With the tab closed, the artboard shows no handles.
    crate::panels::set_pstate(&ctx, super::UI_STATE, Map::from_iter([("__tab".to_string(), json!("rotation"))]));
    frame(&mut app, vec![egui::Event::PointerMoved(target)]);
    assert!(!frame(&mut app, vec![button(target, true)]));
}

/// One frame of the artboard's graphic handles over an 800 × 600 canvas with `events`: whether the
/// handles took the pointer.
fn handles_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> bool {
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
    let mut taken = false;
    ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
        crate::panels::set_pstate(ui.ctx(), super::DRAWN, Some(super::root_pass(ui.ctx())));
        let resp = ui.allocate_rect(rect, egui::Sense::click_and_drag());
        taken = super::graphic_handles_input(app, ui, &resp, rect);
    })
    .textures_delta
    .clear();
    taken
}

/// Where document point (`x`, `y`) is on the 800 × 600 canvas of [`handles_frame`].
fn handles_screen(app: &VectorcraftApp, x: f64, y: f64) -> egui::Pos2 {
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
    crate::canvas::Xf::new(rect, &app.view().cloned().unwrap_or_default()).to_screen(vectorcraft_geom::Point::new(x, y))
}

fn primary(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

#[test]
fn a_view_gesture_on_a_graphics_handle_is_left_to_the_canvas() {
    let (mut app, ctx) = materials_app();
    app.run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
    let handle = handles_screen(&app, 150.0, 150.0);
    let space = |pressed| egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed, repeat: false, modifiers: Default::default() };
    handles_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(handle)]);
    // With Space held, the press pans the view.
    handles_frame(&mut app, &ctx, vec![space(true)]);
    assert!(!handles_frame(&mut app, &ctx, vec![primary(handle, true)]));
    handles_frame(&mut app, &ctx, vec![primary(handle, false), space(false)]);
    // With the Hand, Zoom or Rotate View tool, the press is the tool's.
    for tool in ["hand", "zoom", "rotateView"] {
        app.select_tool(tool);
        assert!(!handles_frame(&mut app, &ctx, vec![primary(handle, true)]), "{tool}");
        handles_frame(&mut app, &ctx, vec![primary(handle, false)]);
    }
    // With another tool and Space up, the press grabs the handle.
    app.select_tool("selection");
    assert!(handles_frame(&mut app, &ctx, vec![primary(handle, true)]));
    handles_frame(&mut app, &ctx, vec![primary(handle, false)]);
}

#[test]
fn a_geometry_effect_after_the_3d_effect_hides_the_graphics_handles() {
    let (mut app, ctx) = materials_app();
    app.run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
    let handle = handles_screen(&app, 150.0, 150.0);
    let push_effect = |app: &mut VectorcraftApp, object: usize, id: &str, params: Value| {
        let st = app.session.active_mut().unwrap();
        let object = st.selection.objects[object];
        let n = std::sync::Arc::make_mut(&mut st.doc).node_mut(object).unwrap();
        n.appearance.effects.truncate(1);
        n.appearance.effects.push(vectorcraft_effects::new_effect(id, &params).unwrap());
    };
    // A Transform effect after the 3D effect draws the graphic 40 pt to the right of the frame's
    // place for it: the artboard shows no handle there.
    push_effect(&mut app, 0, "distort.transform", json!({"moveH": 40.0}));
    handles_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(handle)]);
    assert!(!handles_frame(&mut app, &ctx, vec![primary(handle, true)]));
    handles_frame(&mut app, &ctx, vec![primary(handle, false)]);
    // A Drop Shadow after it leaves the graphic where the frame places it: the handle shows.
    push_effect(&mut app, 0, "stylize.dropShadow", json!({}));
    assert!(handles_frame(&mut app, &ctx, vec![primary(handle, true)]));
    handles_frame(&mut app, &ctx, vec![primary(handle, false)]);
    // A second selected object with the same graphic and a Transform effect after its 3D effect:
    // a drag would move its graphic too, so the artboard shows no handles.
    let square = app.session.active().unwrap().selection.objects[0].0;
    let other = app.run("shape.rectangle", json!({"x": 400, "y": 400, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"rotationX": 0.0, "rotationY": 0.0, "rotationZ": 0.0}}))
        .unwrap();
    app.run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
    app.run("select.set", json!({"ids": [square, other]})).unwrap();
    assert!(handles_frame(&mut app, &ctx, vec![primary(handle, true)]));
    handles_frame(&mut app, &ctx, vec![primary(handle, false)]);
    push_effect(&mut app, 1, "distort.transform", json!({"moveH": 40.0}));
    assert!(!handles_frame(&mut app, &ctx, vec![primary(handle, true)]));
    handles_frame(&mut app, &ctx, vec![primary(handle, false)]);
}

#[test]
fn a_hidden_fill_shows_no_handles_for_its_graphics() {
    let (mut app, ctx) = materials_app();
    app.run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
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
    let shown = |app: &VectorcraftApp| {
        let mut shown = false;
        ctx.run_ui(egui::RawInput::default(), |ui| {
            crate::panels::set_pstate(ui.ctx(), super::DRAWN, Some(super::root_pass(ui.ctx())));
            shown = super::graphic_handles(app, ui.ctx()).is_some_and(|(_, _, handles)| !handles.is_empty());
        })
        .textures_delta
        .clear();
        shown
    };
    assert!(shown(&app), "the visible fill's graphic has a handle");
    {
        let st = app.session.active_mut().unwrap();
        let id = st.selection.objects[0];
        let n = std::sync::Arc::make_mut(&mut st.doc).node_mut(id).unwrap();
        if let Some(vectorcraft_doc::AppearanceItem::Fill(f)) = n.appearance.items.get_mut(fill) {
            f.visible = false;
        }
    }
    assert!(!shown(&app), "the hidden fill draws no graphic, so it has no handle");
}

#[test]
fn a_press_that_closes_the_context_menu_does_not_grab_a_graphics_handle() {
    let (mut app, ctx) = materials_app();
    app.run("threeD.mapGraphic", json!({"symbol": "Graphic"})).unwrap();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(800.0, 600.0));
    // One frame of the canvas's input with a context menu: whether the menu is open after it.
    let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
        let mut open = false;
        ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
            crate::panels::set_pstate(ui.ctx(), super::DRAWN, Some(super::root_pass(ui.ctx())));
            let resp = ui.interact(rect, egui::Id::unique("three-d-test-canvas"), egui::Sense::click_and_drag());
            crate::canvas::canvas_input(app, ui, &resp, rect);
            resp.context_menu(|ui| {
                ui.label("Menu");
            });
            open = resp.context_menu_opened();
        })
        .textures_delta
        .clear();
        open
    };
    // A right-click below and to the right of the handle opens the menu there, clear of the handle.
    let (handle, menu) = (handles_screen(&app, 150.0, 150.0), handles_screen(&app, 195.0, 195.0));
    let secondary =
        |pressed| egui::Event::PointerButton { pos: menu, button: egui::PointerButton::Secondary, pressed, modifiers: Default::default() };
    frame(&mut app, vec![egui::Event::PointerMoved(menu)]);
    frame(&mut app, vec![secondary(true)]);
    frame(&mut app, vec![secondary(false)]);
    assert!(frame(&mut app, vec![egui::Event::PointerMoved(handle)]));
    // The press on the handle only closes the menu: no drag of the graphic starts.
    let (history, before) = (undo_len(&app), graphics(&app));
    assert!(!frame(&mut app, vec![primary(handle, true)]));
    assert!(ctx.data(|m| m.get_temp::<super::HandleDrag>(super::handle_drag_id())).is_none());
    frame(&mut app, vec![egui::Event::PointerMoved(handle + egui::vec2(20.0, 0.0))]);
    frame(&mut app, vec![primary(handle + egui::vec2(20.0, 0.0), false)]);
    assert_eq!(undo_len(&app), history);
    assert_eq!(graphics(&app), before);
}

#[test]
fn a_click_on_a_graphics_handle_records_no_undo_step() {
    let (mut app, ctx) = materials_app();
    app.run("threeD.mapGraphic", json!({"symbol": "Graphic", "x": 5.03})).unwrap();
    let history = undo_len(&app);
    let handle = handles_screen(&app, 155.03, 150.0);
    handles_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(handle)]);
    assert!(handles_frame(&mut app, &ctx, vec![primary(handle, true)]));
    assert!(handles_frame(&mut app, &ctx, vec![primary(handle, false)]));
    assert_eq!(undo_len(&app), history);
    assert_eq!(graphics(&app)[0]["x"], json!(5.03));
}

#[test]
fn a_selection_change_during_a_handle_drag_ends_it() {
    let (mut app, ctx) = materials_app();
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
    let (handle, target, further) = (handles_screen(&app, 150.0, 150.0), handles_screen(&app, 170.0, 140.0), handles_screen(&app, 190.0, 130.0));
    handles_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(handle)]);
    assert!(handles_frame(&mut app, &ctx, vec![primary(handle, true)]));
    handles_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(target)]);
    // The selection changes to the other object while the pointer is down: the drag ends there.
    app.run("select.set", json!({"ids": [other]})).unwrap();
    handles_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(further)]);
    handles_frame(&mut app, &ctx, vec![primary(target, false)]);
    assert_eq!(graphic_of(&app, other), before, "the other object's graphic moved");
    let moved = graphic_of(&app, square);
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

/// The Quick Actions show on the Object tab of the effects that have one (Revolve (Classic)
/// included) and not on their Rotation tab; Revolve and Rotate show them on their first tab.
#[test]
fn quick_actions_show_on_the_object_tab_only() {
    use crate::state::Dialog;
    use vectorcraft_effects::{CLASSIC_EXTRUDE, CLASSIC_REVOLVE, EXTRUDE, INFLATE, REVOLVE};
    for effect in [EXTRUDE, INFLATE, CLASSIC_EXTRUDE, CLASSIC_REVOLVE] {
        for (tab, shown) in [("object", true), ("rotation", false)] {
            let d = Dialog::new("x", json!({"__effect": effect, "__tab": tab, "__gizmo": "rotation"}));
            assert_eq!(super::quick_actions_shown(&d), shown, "{effect} {tab}");
        }
    }
    let d = Dialog::new("x", json!({"__effect": REVOLVE, "__tab": "rotation", "__gizmo": "rotation"}));
    assert!(super::quick_actions_shown(&d));
}

/// An app with an Extrude & Bevel square given `params`, and the panel on its Lighting tab with the
/// accessibility tree on.
fn lighting_app(params: Value) -> (VectorcraftApp, egui::Context) {
    lighting_app_of("threeD.extrude", params)
}

/// An app with a square given 3D effect `effect` with `params`, and the panel on its Lighting tab
/// with the accessibility tree on.
fn lighting_app_of(effect: &str, params: Value) -> (VectorcraftApp, egui::Context) {
    use crate::panels::tests_appearance::{click, frame_events, text_rect};
    let mut app = app();
    app.run("effect.apply", json!({"effect": effect, "item": null, "params": params})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.enable_accesskit();
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "Lighting").center(), show);
    (app, ctx)
}

/// The lighting widget's node for light `n` (from 1): its screen-reader name and its center.
fn light_node(app: &mut VectorcraftApp, ctx: &egui::Context, n: usize) -> (String, egui::Pos2) {
    let nodes = controls(app, ctx);
    let prefix = format!("Light {n}:");
    let node = nodes
        .iter()
        .find(|x| x.role() == egui::accesskit::Role::Slider && x.label().is_some_and(|l| l.starts_with(&prefix)))
        .unwrap_or_else(|| panic!("no lighting widget node for light {n}"));
    let r = node.bounds().unwrap();
    (node.label().unwrap_or_default().to_string(), egui::pos2(((r.x0 + r.x1) / 2.0) as f32, ((r.y0 + r.y1) / 2.0) as f32))
}

fn pointer(pos: egui::Pos2, pressed: Option<bool>) -> egui::Event {
    match pressed {
        Some(pressed) => egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() },
        None => egui::Event::PointerMoved(pos),
    }
}

fn merged_light(app: &VectorcraftApp) -> (f64, f64) {
    let (id, params) = effect(app);
    let merged = vectorcraft_effects::merged_params(&id, &params);
    (merged["lightRotation"].as_f64().unwrap(), merged["lightHeight"].as_f64().unwrap())
}

/// Dragging a light in the lighting widget previews its new Rotation and Height while the pointer
/// is down and records one undo step when it is released, at the point of release; the rim of the
/// sphere is Height 0.
#[test]
fn the_lighting_widget_drags_a_light_in_one_undo_step() {
    use crate::dialogs::revolve_lights::RADIUS;
    use crate::panels::tests_appearance::frame_events;
    let (mut app, ctx) = lighting_app(json!({}));
    let (name, from) = light_node(&mut app, &ctx, 1);
    assert_eq!(name, "Light 1: Rotation 125.3°, Height 30.0°");
    let light = crate::dialogs::revolve_lights::lights(&crate::state::Dialog::new(
        "x",
        json!({"__effect": "threeD.extrude", "lightRotation": vectorcraft_three_d::DEFAULT_LIGHT.0, "lightHeight": 30.0}),
    ))[0];
    let center = from - light.offset() * RADIUS;
    let history = undo_len(&app);
    frame_events(&ctx, &mut app, vec![pointer(from, None)], show);
    frame_events(&ctx, &mut app, vec![pointer(from, Some(true))], show);
    // Halfway out along the light's own direction, then onto the rim straight right of the center.
    frame_events(&ctx, &mut app, vec![pointer(center + egui::vec2(0.0, -RADIUS * 0.5), None)], show);
    assert_eq!(merged_light(&app), (90.0, 60.0), "the light follows the pointer while it is down");
    let rim = center + egui::vec2(RADIUS, 0.0);
    frame_events(&ctx, &mut app, vec![pointer(rim, None)], show);
    assert_eq!(merged_light(&app), (0.0, 0.0));
    assert_eq!(undo_len(&app), history, "nothing is recorded while the pointer is down");
    // A frame that moves the pointer below the center and releases it there drops the light there.
    let below = center + egui::vec2(0.0, RADIUS);
    frame_events(&ctx, &mut app, vec![pointer(below, None), pointer(below, Some(false))], show);
    frame_events(&ctx, &mut app, vec![], show);
    assert_eq!(undo_len(&app), history + 1);
    assert_eq!(merged_light(&app), (-90.0, 0.0));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(merged_light(&app), (vectorcraft_three_d::DEFAULT_LIGHT.0, 30.0));
    // A drag that comes back to the press drops the light there, not where it last was.
    for event in [pointer(from, None), pointer(from, Some(true)), pointer(center + egui::vec2(0.0, -RADIUS * 0.5), None), pointer(from, None)] {
        frame_events(&ctx, &mut app, vec![event], show);
    }
    frame_events(&ctx, &mut app, vec![pointer(from, Some(false))], show);
    frame_events(&ctx, &mut app, vec![], show);
    assert_eq!(merged_light(&app), (125.3, 30.0));
}

/// With objects whose lights differ in Height, an arrow key that turns the light changes only
/// the Rotation, and each object keeps its own Height.
#[test]
fn a_key_leaves_the_other_setting_as_each_selected_effect_has_it() {
    use crate::panels::tests_appearance::{click, frame_events, text_rect};
    let mut app = app();
    let a = app.session.active().unwrap().selection.objects[0].0;
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"lightRotation": 0.0, "lightHeight": 20.0}})).unwrap();
    let b = app.run("shape.rectangle", json!({"x": 250, "y": 100, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("effect.apply", json!({"effect": "threeD.extrude", "item": null, "params": {"lightRotation": 0.0, "lightHeight": 40.0}})).unwrap();
    app.run("select.set", json!({"ids": [a, b]})).unwrap();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.enable_accesskit();
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "Lighting").center(), show);
    let (_, at) = light_node(&mut app, &ctx, 1);
    click(&ctx, &mut app, at, show);
    let left = egui::Event::Key { key: egui::Key::ArrowLeft, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    frame_events(&ctx, &mut app, vec![left], show);
    let light = |id: u64| {
        let n = app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap();
        let merged = vectorcraft_effects::merged_params("threeD.extrude", &n.appearance.effects[0].params);
        (merged["lightRotation"].as_f64().unwrap(), merged["lightHeight"].as_f64().unwrap())
    };
    assert_eq!((light(a), light(b)), ((1.0, 20.0), (1.0, 40.0)));
}

/// A press on a light selects it in the list without an undo step, and the button below the sphere
/// moves only the selected light to the back of the object, then back to the front.
#[test]
fn the_lighting_widget_selects_a_light_and_moves_it_behind_the_object() {
    use crate::panels::tests_appearance::{click, frame_events, text_rect};
    let light =
        |rotation: f64| json!({"color": "#ffffff", "intensity": 80.0, "rotation": rotation, "height": 30.0, "softness": 0.0, "behind": false});
    let (mut app, ctx) = lighting_app(json!({"lights": [light(0.0), light(180.0)]}));
    let history = undo_len(&app);
    let (_, second) = light_node(&mut app, &ctx, 2);
    click(&ctx, &mut app, second, show);
    assert_eq!(undo_len(&app), history, "a press on a light changes nothing");
    let texts = frame_events(&ctx, &mut app, vec![], show);
    assert!(texts.iter().any(|(s, _)| s == "Light 2"), "the list shows the pressed light: {texts:?}");
    click(&ctx, &mut app, text_rect(&texts, "Move Light to Back").center(), show);
    assert_eq!(undo_len(&app), history + 1);
    let behind = |app: &VectorcraftApp| effect(app).1["lights"].as_array().unwrap().iter().map(|l| l["behind"].clone()).collect::<Vec<_>>();
    assert_eq!(behind(&app), [json!(false), json!(true)]);
    let (name, _) = light_node(&mut app, &ctx, 2);
    assert_eq!(name, "Light 2: Rotation 180.0°, Height 30.0°, Light behind the object");
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "Move Light to Front").center(), show);
    assert_eq!(behind(&app), [json!(false), json!(false)]);
    assert_eq!(undo_len(&app), history + 2);
}

/// A light the pointer took has the keyboard focus: the arrow keys raise and lower it and turn it
/// around the object, 1° a press or 15° with Shift, changing only the setting each press moves, one
/// undo step a frame, and a press at a limit records nothing.
#[test]
fn the_arrow_keys_move_the_focused_light() {
    use crate::panels::tests_appearance::{click, frame_events};
    let (mut app, ctx) = lighting_app(json!({"lightRotation": 10.04, "lightHeight": 89.5}));
    let (_, at) = light_node(&mut app, &ctx, 1);
    click(&ctx, &mut app, at, show);
    let history = undo_len(&app);
    let key = |key: egui::Key, modifiers: egui::Modifiers| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
    frame_events(&ctx, &mut app, vec![key(egui::Key::ArrowUp, egui::Modifiers::NONE)], show);
    frame_events(&ctx, &mut app, vec![], show);
    assert_eq!(merged_light(&app), (10.04, 90.0), "Up leaves the Rotation as it is");
    assert_eq!(undo_len(&app), history + 1);
    frame_events(&ctx, &mut app, vec![key(egui::Key::ArrowUp, egui::Modifiers::NONE)], show);
    frame_events(&ctx, &mut app, vec![], show);
    assert_eq!(merged_light(&app), (10.04, 90.0));
    assert_eq!(undo_len(&app), history + 1, "Height stays at 90° and nothing is recorded");
    // Down then Up in one frame, in that order: back at 90°.
    frame_events(&ctx, &mut app, vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE), key(egui::Key::ArrowUp, egui::Modifiers::NONE)], show);
    frame_events(&ctx, &mut app, vec![], show);
    assert_eq!(merged_light(&app), (10.04, 90.0));
    assert_eq!(undo_len(&app), history + 1);
    // Presses that arrive in one frame all move the light, as one undo step.
    let presses = vec![
        key(egui::Key::ArrowLeft, egui::Modifiers::SHIFT),
        key(egui::Key::ArrowDown, egui::Modifiers::NONE),
        key(egui::Key::ArrowDown, egui::Modifiers::NONE),
    ];
    frame_events(&ctx, &mut app, presses, show);
    frame_events(&ctx, &mut app, vec![], show);
    assert_eq!(merged_light(&app), (25.0, 88.0));
    assert_eq!(undo_len(&app), history + 2);
}

/// A press on another light selects it and gives it the keyboard focus, so the next arrow key
/// moves that light and leaves the one selected before where it is.
#[test]
fn the_arrow_keys_move_the_light_pressed_last() {
    use crate::panels::tests_appearance::{click, frame_events};
    let light =
        |rotation: f64| json!({"color": "#ffffff", "intensity": 80.0, "rotation": rotation, "height": 30.0, "softness": 0.0, "behind": false});
    let (mut app, ctx) = lighting_app(json!({"lights": [light(0.0), light(180.0)]}));
    let (_, first) = light_node(&mut app, &ctx, 1);
    click(&ctx, &mut app, first, show);
    let (_, second) = light_node(&mut app, &ctx, 2);
    click(&ctx, &mut app, second, show);
    let left = egui::Event::Key { key: egui::Key::ArrowLeft, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    frame_events(&ctx, &mut app, vec![left], show);
    frame_events(&ctx, &mut app, vec![], show);
    let rotations = effect(&app).1["lights"].as_array().unwrap().iter().map(|l| l["rotation"].clone()).collect::<Vec<_>>();
    assert_eq!(rotations, [json!(0.0), json!(-179.0)]);
}

/// The Lighting tab of Extrude & Bevel (Classic) has the lighting sphere: dragging its light is one
/// undo step, the button below it moves the light behind the object, Add Light adds a light at the
/// front center of the sphere, and Delete Light removes the selected light but never the last one.
#[test]
fn the_classic_lighting_tab_has_the_lighting_sphere() {
    use crate::dialogs::revolve_lights::RADIUS;
    use crate::panels::tests_appearance::{click, frame_events, text_rect};
    use vectorcraft_effects::CLASSIC_EXTRUDE;
    let (mut app, ctx) = lighting_app_of(CLASSIC_EXTRUDE, json!({}));
    let (name, from) = light_node(&mut app, &ctx, 1);
    assert_eq!(name, "Light 1: Rotation 125.3°, Height 30.0°");
    let light = crate::dialogs::revolve_lights::lights(&crate::state::Dialog::new(
        "x",
        json!({"__effect": CLASSIC_EXTRUDE, "lightRotation": vectorcraft_three_d::DEFAULT_LIGHT.0, "lightHeight": 30.0}),
    ))[0];
    let center = from - light.offset() * RADIUS;
    let history = undo_len(&app);
    let rim = center + egui::vec2(RADIUS, 0.0);
    for event in [pointer(from, None), pointer(from, Some(true)), pointer(rim, None)] {
        frame_events(&ctx, &mut app, vec![event], show);
    }
    assert_eq!(merged_light(&app), (0.0, 0.0), "the light follows the pointer while it is down");
    assert_eq!(undo_len(&app), history);
    frame_events(&ctx, &mut app, vec![pointer(rim, Some(false))], show);
    frame_events(&ctx, &mut app, vec![], show);
    assert_eq!(undo_len(&app), history + 1);
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "Move Light to Back").center(), show);
    assert_eq!(undo_len(&app), history + 2);
    assert_eq!(effect(&app).1["lightBehind"], json!(true));
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "Add Light").center(), show);
    assert_eq!(undo_len(&app), history + 3);
    let (name, _) = light_node(&mut app, &ctx, 2);
    assert_eq!(name, "Light 2: Rotation 0.0°, Height 90.0°", "a new light is at the front center");
    let count = |app: &VectorcraftApp| effect(app).1["lights"].as_array().map_or(1, Vec::len);
    assert_eq!(count(&app), 2);
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "Delete Light").center(), show);
    assert_eq!(count(&app), 1);
    assert_eq!(undo_len(&app), history + 4);
    let (name, _) = light_node(&mut app, &ctx, 1);
    assert_eq!(name, "Light 1: Rotation 0.0°, Height 0.0°, Light behind the object", "the selected light was deleted");
    // One light stays: Delete Light does nothing then.
    let texts = frame_events(&ctx, &mut app, vec![], show);
    click(&ctx, &mut app, text_rect(&texts, "Delete Light").center(), show);
    assert_eq!(count(&app), 1);
    assert_eq!(undo_len(&app), history + 4);
}

/// The 3D (Classic) options dialog shows the lighting sphere on its Lighting tab too, and the
/// sphere draws a 3D (Classic) effect's lights white, since those effects shade without light color.
#[test]
fn the_classic_options_dialog_has_the_lighting_sphere_with_white_lights() {
    use crate::panels::tests_appearance::frame_events;
    use vectorcraft_effects::{CLASSIC_REVOLVE, EXTRUDE};
    let mut app = app();
    app.run("effect.dialog", json!({"effect": CLASSIC_REVOLVE, "item": null})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("__gizmo".into(), json!("light"));
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    ctx.enable_accesskit();
    let dialog = |app: &mut VectorcraftApp, ui: &mut egui::Ui| crate::dialogs::show(app, ui.ctx());
    frame_events(&ctx, &mut app, vec![], dialog);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| dialog(&mut app, ui));
    out.textures_delta.clear();
    let nodes: Vec<_> = out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default();
    assert!(
        nodes.iter().any(|n| n.role() == egui::accesskit::Role::Slider && n.label() == Some("Light 1: Rotation 125.3°, Height 30.0°")),
        "the dialog shows the sphere"
    );
    let red = json!([{"color": "#ff0000", "rotation": 0.0, "height": 30.0}]);
    let color =
        |effect: &str| crate::dialogs::revolve_lights::lights(&crate::state::Dialog::new("x", json!({"__effect": effect, "lights": red})))[0].color;
    assert_eq!(color(CLASSIC_REVOLVE), egui::Color32::WHITE);
    assert_eq!(color(EXTRUDE), egui::Color32::from_rgb(255, 0, 0));
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
    let ctx = egui::Context::default();
    // Without a 3D object: an error, nothing recorded, no asset.
    let history = undo_len(&app);
    assert!(super::export_3d_object(&mut app, &ctx).is_err());
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
    assert!(super::export_3d_object(&mut app, &ctx).is_err());
    app.run("select.set", json!({"ids": [plain, a, hidden]})).unwrap();
    assert_eq!(super::selected_3d_objects(&app), [a]);
    assert!(!crate::panel_docking::visible(&app.ui, crate::panels::asset_export::ID));
    let history = undo_len(&app);
    super::export_3d_object(&mut app, &ctx).unwrap();
    assert_eq!(undo_len(&app), history + 1);
    let list = assets(&mut app);
    assert_eq!(list.len(), 1, "{list:?}");
    assert_eq!((list[0]["name"].clone(), list[0]["format3D"].clone()), (json!("Box"), json!("usda")));
    let asset = list[0]["id"].as_u64().unwrap();
    assert_eq!(super::pstate::<Vec<u64>>(&ctx, "asset-export-selected"), [asset]);
    assert!(crate::panel_docking::visible(&app.ui, crate::panels::asset_export::ID));
    // Already an asset: no undo step, the same asset selected again, the panel left showing.
    super::set_pstate(&ctx, "asset-export-selected", Vec::<u64>::new());
    super::export_3d_object(&mut app, &ctx).unwrap();
    assert_eq!((undo_len(&app), assets(&mut app).len()), (history + 1, 1));
    assert_eq!(super::pstate::<Vec<u64>>(&ctx, "asset-export-selected"), [asset]);
    assert!(crate::panel_docking::visible(&app.ui, crate::panels::asset_export::ID));
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
    assert_eq!(super::selected_3d_objects(&app), [group]);
}

#[test]
fn changed_settings_lists_only_new_values() {
    let before = json!({"depth": 50.0, "cap": true, "bevel": "none"});
    let after = json!({"depth": 60.0, "cap": true, "bevel": "classic", "twist": 0.0});
    let changed = changed_settings(&before, &after);
    assert_eq!(Value::Object(changed), json!({"depth": 60.0, "bevel": "classic", "twist": 0.0}));
    assert!(changed_settings(&after, &after).is_empty());
}
