//! Pointer gestures drive the real gizmo through the 3D dialog's host: the canvas frame runs `revolve_gizmo::show`,
//! the dialog's pump previews the fields, and Escape is the edit stream's cancel.
use super::*;
use crate::dialogs::three_d::tests_support::ThreeD;
use craft::ui::dialog::{HeldOk, StackDialog};
use egui::{Event, Key, Modifiers, PointerButton};
use serde_json::Value;
use vectorcraft_effects::EXTRUDE;
use vectorcraft_engine::Session;

/// The dialog of `effect` over what `build` makes in a new document, its Preview `on`.
fn dialog(effect: &str, build: &[(&str, Value)], on: bool) -> ThreeD {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":300,"height":300})).unwrap();
    for (id, p) in build {
        app.run(id, p.clone()).unwrap();
    }
    let mut h = ThreeD::new(app);
    // The first frames give the app its window and lay the canvas out.
    h.settle();
    h.open(json!({"effect": effect, "item": null}));
    if !on {
        h.set("preview", json!(false));
    }
    h
}

/// The stroke the gizmo tests turn.
fn revolve() -> ThreeD {
    let stroke = [("path.create", json!({"d":"M150 40 L180 60 L180 140 L150 160"})), ("paint.setFill", json!({"color":"#338ad6"}))];
    dialog(REVOLVE, &stroke, true)
}

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, pressed, button: PointerButton::Primary, modifiers }
}

fn value(h: &ThreeD, key: &str) -> f64 {
    h.fields().number(key).unwrap_or(0.0)
}

fn is_open(h: &ThreeD) -> bool {
    h.app().three_d_dialog.is_some()
}

fn refused(h: &ThreeD) -> Option<String> {
    match three_d::with_fields(&mut h.app(), |d| d.held_state()).flatten() {
        Some(HeldOk::Refused(reason)) => Some(reason.as_str().to_owned()),
        _ => None,
    }
}

fn geometry(h: &ThreeD) -> Geometry {
    let fields = h.fields();
    let app = h.app();
    Geometry::new(&app.canvas_view().unwrap(), &fields, selected_bounds(&app).unwrap()).unwrap()
}

/// Hovers `p`, then presses there with `modifiers` held for the gesture.
fn start(h: &mut ThreeD, p: Pos2, modifiers: Modifiers) {
    h.fixture.hold(modifiers);
    h.fixture.frame(vec![Event::PointerMoved(p)]);
    h.fixture.frame(vec![button(p, true, modifiers)]);
}

fn to(h: &mut ThreeD, p: Pos2) {
    h.fixture.frame(vec![Event::PointerMoved(p)]);
}

fn escape(h: &mut ThreeD) {
    h.fixture.key(Key::Escape, Modifiers::NONE);
}

fn release(h: &mut ThreeD, p: Pos2, modifiers: Modifiers) {
    h.fixture.frame(vec![button(p, false, modifiers)]);
    h.fixture.hold(Modifiers::NONE);
    h.settle();
}

/// Presses at `from`, drags to `to` in two frames and releases.
fn drag(h: &mut ThreeD, from: Pos2, end: Pos2, modifiers: Modifiers) {
    start(h, from, modifiers);
    to(h, from.lerp(end, 0.5));
    to(h, end);
    release(h, end, modifiers);
}

/// Drags the ring `axis` along its curve from a point that belongs to it alone, away from crossings.
fn ring_drag(h: &mut ThreeD, axis: usize, modifiers: Modifiers) {
    let g = geometry(h);
    let ring = g.rings[axis];
    let phase = (0..SAMPLES)
        .map(|i| i as f32 * std::f32::consts::TAU / SAMPLES as f32)
        .find(|a| g.hit(g.center + ring.point(*a)).is_some_and(|hit| hit.0 == axis) && ring.tangent(*a).length() > 40.0)
        .unwrap();
    let from = g.center + ring.point(phase);
    start(h, from, modifiers);
    let mut p = from;
    for i in 1..=8 {
        p = g.center + ring.point(phase + i as f32 * 0.045);
        to(h, p);
    }
    release(h, p, modifiers);
}

/// The effect's stored value of `key` on the selected object.
fn stored(h: &ThreeD, key: &str) -> Value {
    let app = h.app();
    let state = app.session.active().unwrap();
    state.doc.node(state.selection.objects[0]).unwrap().appearance.effects[0].params[key].clone()
}

fn an_image() -> Vec<(&'static str, Value)> {
    let data = vectorcraft_format::base64_encode(&crate::tests_sysclip::blue_png());
    vec![("file.place", json!({"name": "blue.png", "dataBase64": data, "link": false, "at": [150, 150]}))]
}

#[test]
fn each_ring_changes_only_its_rotation_field_and_cancel_restores_the_source() {
    for (axis, key) in KEYS.iter().enumerate() {
        let mut h = revolve();
        let before = h.app().session.active().unwrap().interaction.as_ref().unwrap().doc.clone();
        let original = KEYS.map(|key| value(&h, key));
        ring_drag(&mut h, axis, Modifiers::NONE);
        for (k, key) in KEYS.iter().enumerate() {
            if k == axis {
                assert!(value(&h, key) > original[k] + 10.0, "axis {axis}, {}", value(&h, key));
            } else {
                assert_eq!(value(&h, key), original[k]);
            }
        }
        {
            let app = h.app();
            let state = app.session.active().unwrap();
            let node = state.doc.node(state.selection.objects[0]).unwrap();
            assert_eq!(node.path_data(), before.node(node.id).unwrap().path_data(), "ring drags must not move source anchors");
        }
        assert_eq!(stored(&h, key), json!(value(&h, key)));
        h.cancel();
        assert_eq!(h.app().session.doc().unwrap().doc, before);
    }
}

#[test]
fn ring_drag_with_shift_snaps_to_15_degrees() {
    let mut h = revolve();
    ring_drag(&mut h, 2, Modifiers::SHIFT);
    assert!(value(&h, "rotationZ") > 0.0);
    assert_eq!(value(&h, "rotationZ") % 15.0, 0.0);
}

#[test]
fn a_press_without_a_drag_changes_nothing() {
    let mut h = revolve();
    let before = h.fields();
    let at = geometry(&h).center;
    start(&mut h, at, Modifiers::NONE);
    release(&mut h, at, Modifiers::NONE);
    assert_eq!(h.fields(), before);
}

#[test]
fn free_orbit_and_light_drag_update_the_canvas_and_escape_restores_only_the_gesture() {
    let mut h = revolve();
    let before = h.fields();
    let center = geometry(&h).center;
    start(&mut h, center, Modifiers::NONE);
    let p = center + vec2(50.0, 30.0);
    to(&mut h, p);
    assert_ne!(value(&h, "rotationX"), before.number("rotationX").unwrap());
    assert_ne!(value(&h, "rotationY"), before.number("rotationY").unwrap());
    escape(&mut h);
    release(&mut h, p, Modifiers::NONE);
    assert!(is_open(&h));
    assert_eq!(h.fields(), before);
    h.set("__page", json!("lighting"));
    let light = light_position(geometry(&h), light_options(&h.fields()));
    drag(&mut h, light, light + vec2(20.0, 25.0), Modifiers::NONE);
    assert_ne!(value(&h, "lightRotation"), before.number("lightRotation").unwrap());
    assert_ne!(value(&h, "lightHeight"), 30.0);
    // With no gesture open, Escape is the dialog's: the gizmo does not eat it.
    escape(&mut h);
    h.settle();
    assert!(!is_open(&h) && !h.app().session.in_interaction());
}

#[test]
fn preview_off_has_no_gizmo_and_edge_on_rings_remain_finite() {
    let mut h = revolve();
    for key in KEYS {
        h.set(key, json!(0.0));
    }
    ring_drag(&mut h, 0, Modifiers::NONE);
    assert!(value(&h, "rotationX").is_finite() && value(&h, "rotationX").abs() > 1.0);
    // The gizmo has no geometry to press while Preview is off: the press point is the one Preview on had.
    let center = geometry(&h).center;
    h.set("preview", json!(false));
    assert!(!h.app().session.in_interaction());
    let before = h.fields();
    drag(&mut h, center, center + vec2(40.0, 40.0), Modifiers::NONE);
    assert_eq!(h.fields(), before);
}

#[test]
fn light_rings_adjust_only_their_angle_and_snap_without_rotating_the_object() {
    for (axis, key) in ["lightRotation", "lightHeight"].iter().enumerate() {
        let mut h = revolve();
        h.set("__page", json!("lighting"));
        let before = h.fields();
        let from = geometry(&h).light_labels()[axis].center();
        let delta = if axis == 0 { vec2(68.0, 0.0) } else { vec2(0.0, -43.0) };
        drag(&mut h, from, from + delta, Modifiers::SHIFT);
        assert_ne!(value(&h, key), before.number(key).unwrap());
        assert_eq!(value(&h, key) % 15.0, 0.0);
        for unchanged in ["rotationX", "rotationY", "rotationZ", "lightIntensity", "ambient"] {
            assert_eq!(value(&h, unchanged), before.number(unchanged).unwrap());
        }
        let other = if axis == 0 { "lightHeight" } else { "lightRotation" };
        assert_eq!(value(&h, other), before.number(other).unwrap());
        // The effect may keep a whole number as an integer.
        assert_eq!(stored(&h, key).as_f64(), Some(value(&h, key)));
    }
}

#[test]
fn light_handle_preserves_the_back_side_and_escape_cancel_and_undo_restore_changes() {
    let mut h = revolve();
    let original = h.app().session.active().unwrap().interaction.as_ref().unwrap().doc.clone();
    h.set("__page", json!("lighting"));
    h.set("lightBehind", json!(true));
    let before = h.fields();
    let light = light_position(geometry(&h), light_options(&before));
    start(&mut h, light, Modifiers::NONE);
    let end = light + vec2(-15.0, 12.0);
    to(&mut h, end);
    assert!(light_options(&h.fields()).light_direction()[2] < 0.0);
    assert_ne!(value(&h, "lightHeight"), 30.0);
    escape(&mut h);
    release(&mut h, end, Modifiers::NONE);
    assert_eq!(h.fields(), before);
    // A second gesture is kept by OK, and the whole dialog is one undo step.
    let from = geometry(&h).light_labels()[0].center();
    drag(&mut h, from, from + vec2(35.0, 0.0), Modifiers::NONE);
    h.ok();
    h.app().run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app().session.doc().unwrap().doc, original);
}

#[test]
fn light_height_is_bounded_and_shading_off_disables_the_light_gizmo() {
    let mut h = revolve();
    h.set("__page", json!("lighting"));
    let from = geometry(&h).light_labels()[1].center();
    drag(&mut h, from, from - vec2(0.0, 400.0), Modifiers::NONE);
    assert_eq!(value(&h, "lightHeight"), 90.0);
    let from = geometry(&h).light_labels()[0].center();
    h.set("shade", json!(false));
    let before = h.fields();
    drag(&mut h, from, from + vec2(40.0, 0.0), Modifiers::NONE);
    assert_eq!(h.fields(), before);
}

#[test]
fn selected_light_gizmo_edits_only_that_light_and_escape_restores_the_list() {
    let mut h = revolve();
    h.set("__page", json!("lighting"));
    h.set(
        "lights",
        json!([
            {"color":"#ff0000","rotation":30,"height":45,"intensity":60},
            {"color":"#0000ff","rotation":-45,"height":30,"intensity":50}
        ]),
    );
    h.set("__light", json!(1));
    let before = h.fields().list("lights").unwrap().into_owned();
    let p = geometry(&h).light_labels()[0].center();
    start(&mut h, p, Modifiers::NONE);
    to(&mut h, p + vec2(60.0, 0.0));
    to(&mut h, p + vec2(80.0, 0.0));
    let now = h.fields().list("lights").unwrap().into_owned();
    assert_eq!(now[0], before[0]);
    assert_ne!(now[1], before[1]);
    escape(&mut h);
    release(&mut h, p + vec2(80.0, 0.0), Modifiers::NONE);
    assert_eq!(h.fields().list("lights").unwrap().into_owned(), before);
}

/// #6140: on an object a 3D effect can't take, the dialog says why, and OK, which could only fail, is off.
#[test]
fn a_refused_3d_effect_says_why_and_turns_ok_off() {
    let mut h = dialog(EXTRUDE, &an_image(), true);
    let reason = refused(&h).expect("the dialog is refused");
    assert!(!reason.is_empty());
    assert!(!reason.contains("effect.apply"), "the reason is written without the command's id");
    h.fixture.key(Key::Enter, Modifiers::NONE);
    h.settle();
    assert!(is_open(&h));
}

/// With Preview off, nothing has previewed the effect: the pump still finds the refusal, and trying it leaves no
/// preview open.
#[test]
fn a_refused_3d_effect_is_found_with_preview_off() {
    let h = dialog(EXTRUDE, &an_image(), false);
    assert!(refused(&h).is_some());
    assert!(!h.app().session.in_interaction());
}

/// With Preview off, trying changed values waits for another interaction to end: trying them during one would
/// replace its preview and cancel it.
#[test]
fn a_3d_effect_with_preview_off_leaves_another_interaction_alone() {
    let mut h = dialog(EXTRUDE, &an_image(), false);
    assert!(refused(&h).is_some());
    h.app().session.begin_interaction("Drag").unwrap();
    // A changed value ends the refusal of the values before it.
    h.set("depth", json!(80.0));
    assert!(h.app().session.in_interaction(), "the other interaction goes on");
    assert!(refused(&h).is_none(), "nothing has tried the changed values yet");
    h.app().session.cancel_interaction().unwrap();
    h.settle();
    assert!(refused(&h).is_some());
    assert!(!h.app().session.in_interaction());
}
