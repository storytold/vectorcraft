//! Pointer gestures drive the real gizmo through the 3D dialog's host: the canvas frame runs `revolve_gizmo::show`,
//! the dialog's pump previews the fields, and Escape is the edit stream's cancel.
// PLAN: assumed effects-w13's `three_d::tests_support::ThreeD`: `open(effect, build: &[(&str, Value)], preview: bool) -> ThreeD`,
// `.app`, `.frame(events, mods)`, `.settle()`, `.view() -> CanvasView`, `.fields() -> DialogFields`, `.value(key) -> f64`,
// `.set(key, Value)` (the host's `set_json_field`), `.is_open()`, `.held() -> Option<HeldOk>`, `.cancel()`.
use super::*;
use crate::dialogs::three_d::tests_support::ThreeD;
use craft_ui::dialog::HeldOk;
use egui::{Event, Modifiers, PointerButton};
use serde_json::Value;
use vectorcraft_effects::EXTRUDE;

/// The stroke the gizmo tests turn, as the old harness built it.
fn stroke() -> Vec<(&'static str, Value)> {
    vec![("path.create", json!({"d":"M150 40 L180 60 L180 140 L150 160"})), ("paint.setFill", json!({"color":"#338ad6"}))]
}

fn revolve() -> ThreeD {
    ThreeD::open(REVOLVE, &stroke(), true)
}

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, pressed, button: PointerButton::Primary, modifiers }
}

fn escape() -> Event {
    Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

fn geometry(h: &ThreeD) -> Geometry {
    Geometry::new(&h.view(), &h.fields(), selected_bounds(&h.app).unwrap()).unwrap()
}

fn start(h: &mut ThreeD, p: Pos2, modifiers: Modifiers) {
    h.frame(vec![Event::PointerMoved(p)], modifiers);
    h.frame(vec![button(p, true, modifiers)], modifiers);
}

fn release(h: &mut ThreeD, p: Pos2, modifiers: Modifiers) {
    h.frame(vec![button(p, false, modifiers)], modifiers);
    h.settle();
}

/// Presses at `from`, drags to `to` in two frames and releases.
fn drag(h: &mut ThreeD, from: Pos2, to: Pos2, modifiers: Modifiers) {
    start(h, from, modifiers);
    h.frame(vec![Event::PointerMoved(from.lerp(to, 0.5))], modifiers);
    h.frame(vec![Event::PointerMoved(to)], modifiers);
    release(h, to, modifiers);
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
        h.frame(vec![Event::PointerMoved(p)], modifiers);
    }
    release(h, p, modifiers);
}

fn an_image() -> Vec<(&'static str, Value)> {
    let data = vectorcraft_format::base64_encode(&crate::tests_sysclip::blue_png());
    vec![("file.place", json!({"name": "blue.png", "dataBase64": data, "link": false, "at": [150, 150]}))]
}

#[test]
fn each_ring_changes_only_its_rotation_field_and_cancel_restores_the_source() {
    for (axis, key) in KEYS.iter().enumerate() {
        let mut h = revolve();
        let before = h.app.session.active().unwrap().interaction.as_ref().unwrap().doc.clone();
        let original = KEYS.map(|key| h.value(key));
        ring_drag(&mut h, axis, Modifiers::NONE);
        for (k, key) in KEYS.iter().enumerate() {
            if k == axis {
                assert!(h.value(key) > original[k] + 10.0, "axis {axis}, {}", h.value(key));
            } else {
                assert_eq!(h.value(key), original[k]);
            }
        }
        let state = h.app.session.active().unwrap();
        let node = state.doc.node(state.selection.objects[0]).unwrap();
        assert_eq!(node.path_data(), before.node(node.id).unwrap().path_data(), "ring drags must not move source anchors");
        assert_eq!(node.appearance.effects[0].params[*key], json!(h.value(key)));
        h.cancel();
        assert_eq!(h.app.session.doc().unwrap().doc, before);
    }
}

#[test]
fn ring_drag_with_shift_snaps_to_15_degrees() {
    let mut h = revolve();
    ring_drag(&mut h, 2, Modifiers::SHIFT);
    assert!(h.value("rotationZ") > 0.0);
    assert_eq!(h.value("rotationZ") % 15.0, 0.0);
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
    h.frame(vec![Event::PointerMoved(p)], Modifiers::NONE);
    assert_ne!(h.value("rotationX"), before.number("rotationX").unwrap());
    assert_ne!(h.value("rotationY"), before.number("rotationY").unwrap());
    h.frame(vec![escape()], Modifiers::NONE);
    release(&mut h, p, Modifiers::NONE);
    assert!(h.is_open());
    assert_eq!(h.fields(), before);
    h.set("__page", json!("lighting"));
    h.settle();
    let light = light_position(geometry(&h), light_options(&h.fields()));
    drag(&mut h, light, light + vec2(20.0, 25.0), Modifiers::NONE);
    assert_ne!(h.value("lightRotation"), before.number("lightRotation").unwrap());
    assert_ne!(h.value("lightHeight"), 30.0);
    // With no gesture open, Escape is the dialog's: the gizmo does not eat it.
    h.frame(vec![escape()], Modifiers::NONE);
    assert!(!h.is_open() && !h.app.session.in_interaction());
}

#[test]
fn preview_off_has_no_gizmo_and_edge_on_rings_remain_finite() {
    let mut h = revolve();
    for key in KEYS {
        h.set(key, json!(0.0));
    }
    h.settle();
    ring_drag(&mut h, 0, Modifiers::NONE);
    assert!(h.value("rotationX").is_finite() && h.value("rotationX").abs() > 1.0);
    h.set("preview", json!(false));
    h.settle();
    assert!(!h.app.session.in_interaction());
    let before = h.fields();
    // The gizmo has no geometry to press while Preview is off: the press point is the one Preview on had.
    let center = geometry(&h).center;
    drag(&mut h, center, center + vec2(40.0, 40.0), Modifiers::NONE);
    assert_eq!(h.fields(), before);
}

#[test]
fn light_rings_adjust_only_their_angle_and_snap_without_rotating_the_object() {
    for (axis, key) in ["lightRotation", "lightHeight"].iter().enumerate() {
        let mut h = revolve();
        h.set("__page", json!("lighting"));
        h.settle();
        let before = h.fields();
        let from = geometry(&h).light_labels()[axis].center();
        let delta = if axis == 0 { vec2(68.0, 0.0) } else { vec2(0.0, -43.0) };
        drag(&mut h, from, from + delta, Modifiers::SHIFT);
        assert_ne!(h.value(key), before.number(key).unwrap());
        assert_eq!(h.value(key) % 15.0, 0.0);
        for unchanged in ["rotationX", "rotationY", "rotationZ", "lightIntensity", "ambient"] {
            assert_eq!(h.value(unchanged), before.number(unchanged).unwrap());
        }
        let other = if axis == 0 { "lightHeight" } else { "lightRotation" };
        assert_eq!(h.value(other), before.number(other).unwrap());
        let state = h.app.session.active().unwrap();
        let node = state.doc.node(state.selection.objects[0]).unwrap();
        assert_eq!(node.appearance.effects[0].params[*key], json!(h.value(key)));
    }
}

#[test]
fn light_handle_preserves_the_back_side_and_escape_cancel_and_undo_restore_changes() {
    let mut h = revolve();
    let original = h.app.session.active().unwrap().interaction.as_ref().unwrap().doc.clone();
    h.set("__page", json!("lighting"));
    h.set("lightBehind", json!(true));
    h.settle();
    let before = h.fields();
    let light = light_position(geometry(&h), light_options(&before));
    start(&mut h, light, Modifiers::NONE);
    let to = light + vec2(-15.0, 12.0);
    h.frame(vec![Event::PointerMoved(to)], Modifiers::NONE);
    assert!(light_options(&h.fields()).light_direction()[2] < 0.0);
    assert_ne!(h.value("lightHeight"), 30.0);
    h.frame(vec![escape()], Modifiers::NONE);
    release(&mut h, to, Modifiers::NONE);
    assert_eq!(h.fields(), before);
    // A second gesture is kept by OK, and the whole dialog is one undo step.
    let from = geometry(&h).light_labels()[0].center();
    drag(&mut h, from, from + vec2(35.0, 0.0), Modifiers::NONE);
    h.confirm();
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc().unwrap().doc, original);
}

#[test]
fn light_height_is_bounded_and_shading_off_disables_the_light_gizmo() {
    let mut h = revolve();
    h.set("__page", json!("lighting"));
    h.settle();
    let from = geometry(&h).light_labels()[1].center();
    drag(&mut h, from, from - vec2(0.0, 400.0), Modifiers::NONE);
    assert_eq!(h.value("lightHeight"), 90.0);
    h.set("shade", json!(false));
    h.settle();
    let before = h.fields();
    let from = geometry(&h).light_labels()[0].center();
    drag(&mut h, from, from + vec2(40.0, 0.0), Modifiers::NONE);
    assert_eq!(h.fields(), before);
}

#[test]
fn selected_light_gizmo_edits_only_that_light_and_escape_restores_the_list() {
    let mut h = revolve();
    h.set("__page", json!("lighting"));
    h.set("__light", json!(1));
    h.set(
        "lights",
        json!([
            {"color":"#ff0000","rotation":30,"height":45,"intensity":60},
            {"color":"#0000ff","rotation":-45,"height":30,"intensity":50}
        ]),
    );
    h.settle();
    let before = h.fields().list("lights").unwrap().into_owned();
    let p = geometry(&h).light_labels()[0].center();
    start(&mut h, p, Modifiers::NONE);
    h.frame(vec![Event::PointerMoved(p + vec2(60.0, 0.0))], Modifiers::NONE);
    h.frame(vec![Event::PointerMoved(p + vec2(80.0, 0.0))], Modifiers::NONE);
    let now = h.fields().list("lights").unwrap().into_owned();
    assert_eq!(now[0], before[0]);
    assert_ne!(now[1], before[1]);
    h.frame(vec![escape()], Modifiers::NONE);
    release(&mut h, p + vec2(80.0, 0.0), Modifiers::NONE);
    assert_eq!(h.fields().list("lights").unwrap().into_owned(), before);
}

/// #6140: on an object a 3D effect can't take, the dialog says why, and OK, which could only fail, is off.
#[test]
fn a_refused_3d_effect_says_why_and_turns_ok_off() {
    let mut h = ThreeD::open(EXTRUDE, &an_image(), true);
    let Some(HeldOk::Refused(reason)) = h.held() else { panic!("the dialog is not refused") };
    assert!(!reason.as_str().is_empty());
    assert!(!reason.as_str().contains("effect.apply"), "the reason is written without the command's id");
    let enter = Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    h.frame(vec![enter], Modifiers::NONE);
    assert!(h.is_open());
}

/// With Preview off from the start, nothing has tried the effect: the pump still finds the refusal, and trying it
/// leaves no preview open.
#[test]
fn a_refused_3d_effect_is_found_with_preview_off() {
    let h = ThreeD::open(EXTRUDE, &an_image(), false);
    assert!(matches!(h.held(), Some(HeldOk::Refused(_))));
    assert!(!h.app.session.in_interaction());
}

/// With Preview off, trying the values waits for another interaction to end: trying them during one would replace
/// its preview and cancel it.
#[test]
fn a_3d_effect_with_preview_off_leaves_another_interaction_alone() {
    let mut h = ThreeD::open(EXTRUDE, &an_image(), false);
    h.app.session.begin_interaction("Drag").unwrap();
    if let Some(session) = h.app.three_d_dialog.as_mut() {
        session.tried = None;
    }
    h.settle();
    assert!(h.app.session.in_interaction(), "the other interaction goes on");
    assert!(!matches!(h.held(), Some(HeldOk::Refused(_))), "nothing has tried the values yet");
    h.app.session.cancel_interaction().unwrap();
    h.settle();
    assert!(matches!(h.held(), Some(HeldOk::Refused(_))));
    assert!(!h.app.session.in_interaction());
}
