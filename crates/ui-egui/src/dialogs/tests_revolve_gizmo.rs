//! Pointer gestures exercise the real artboard overlay and command preview, including rollback.
use super::*;
use crate::{dialogs, theme};
use egui::{Event, Modifiers, PointerButton};
use vectorcraft_engine::Session;

struct Harness {
    app: VectorcraftApp,
    ctx: egui::Context,
}

impl Harness {
    fn new() -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width":300,"height":300})).unwrap();
        app.run("path.create", json!({"d":"M150 40 L180 60 L180 140 L150 160"})).unwrap();
        app.run("paint.setFill", json!({"color":"#338ad6"})).unwrap();
        app.run("effect.dialog", json!({"effect":REVOLVE,"item":null})).unwrap();
        app.canvas_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0)));
        let v = app.view_mut().unwrap();
        v.center = Point::new(270.0, 150.0);
        v.zoom = 2.0;
        v.fitted = true;
        let ctx = egui::Context::default();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, Default::default());
        let mut h = Self { app, ctx };
        h.settle();
        h
    }

    fn frame(&mut self, events: Vec<Event>, modifiers: Modifiers) {
        let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 900.0))), events, ..Default::default() };
        let mut out = self.ctx.run_ui(input, |ui| {
            crate::shortcuts::handle(&mut self.app, ui.ctx());
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::show(&mut self.app, ui));
            dialogs::show(&mut self.app, ui.ctx());
        });
        out.textures_delta.clear();
    }

    fn settle(&mut self) {
        for _ in 0..5 {
            self.frame(vec![], Modifiers::NONE);
        }
    }

    fn geometry(&self) -> Geometry {
        Geometry::new(&self.app, self.app.ui.dialog.as_ref().unwrap()).unwrap()
    }

    fn value(&self, key: &str) -> f64 {
        self.app.ui.dialog.as_ref().unwrap().f64(key, 0.0)
    }

    fn start(&mut self, p: Pos2, modifiers: Modifiers) {
        self.frame(vec![Event::PointerMoved(p)], modifiers);
        self.frame(vec![button(p, true, modifiers)], modifiers);
    }

    fn release(&mut self, p: Pos2, modifiers: Modifiers) {
        self.frame(vec![button(p, false, modifiers)], modifiers);
        self.settle();
    }

    fn ring_drag(&mut self, axis: usize, modifiers: Modifiers) {
        let g = self.geometry();
        let ring = g.rings[axis];
        // Choose a visible point belonging only to this ring, away from crossings.
        let phase = (0..SAMPLES)
            .map(|i| i as f32 * std::f32::consts::TAU / SAMPLES as f32)
            .find(|a| g.hit(g.center + ring.point(*a)).is_some_and(|h| h.0 == axis) && ring.tangent(*a).length() > 40.0)
            .unwrap();
        let from = g.center + ring.point(phase);
        self.start(from, modifiers);
        let mut p = from;
        for i in 1..=8 {
            p = g.center + ring.point(phase + i as f32 * 0.045);
            self.frame(vec![Event::PointerMoved(p)], modifiers);
        }
        self.release(p, modifiers);
    }
}

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, pressed, button: PointerButton::Primary, modifiers }
}
fn escape() -> Event {
    Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

#[test]
fn each_ring_changes_only_its_rotation_field_and_cancel_restores_the_source() {
    for (axis, key) in KEYS.iter().enumerate() {
        let mut h = Harness::new();
        let before = h.app.session.active().unwrap().interaction.as_ref().unwrap().doc.clone();
        let original = KEYS.map(|key| h.value(key));
        h.ring_drag(axis, Modifiers::NONE);
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
        dialogs::cancel(&mut h.app);
        assert_eq!(h.app.session.doc().unwrap().doc, before);
    }
}

#[test]
fn ring_drag_snaps_and_confirm_is_one_undo_step() {
    let mut h = Harness::new();
    let before = h.app.session.active().unwrap().interaction.as_ref().unwrap().doc.clone();
    h.ring_drag(2, Modifiers::SHIFT);
    assert!(h.value("rotationZ") > 0.0);
    assert_eq!(h.value("rotationZ") % 15.0, 0.0);
    dialogs::confirm(&mut h.app).unwrap();
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc().unwrap().doc, before);
}

#[test]
fn free_orbit_and_light_drag_update_the_canvas_and_escape_restores_only_the_gesture() {
    let mut h = Harness::new();
    let before = super::super::form::params(h.app.ui.dialog.as_ref().unwrap());
    let center = h.geometry().center;
    h.start(center, Modifiers::NONE);
    let p = center + vec2(50.0, 30.0);
    h.frame(vec![Event::PointerMoved(p)], Modifiers::NONE);
    assert_ne!(h.value("rotationX"), before["rotationX"].as_f64().unwrap());
    assert_ne!(h.value("rotationY"), before["rotationY"].as_f64().unwrap());
    h.frame(vec![escape()], Modifiers::NONE);
    h.release(p, Modifiers::NONE);
    assert!(h.app.ui.dialog.is_some());
    assert_eq!(super::super::form::params(h.app.ui.dialog.as_ref().unwrap()), before);
    let state = h.app.session.active().unwrap();
    let node = state.doc.node(state.selection.objects[0]).unwrap();
    assert_eq!(node.appearance.effects[0].params["rotationX"], before["rotationX"]);
    assert_eq!(node.appearance.effects[0].params["rotationY"], before["rotationY"]);
    h.app.ui.dialog.as_mut().unwrap().fields.insert("__gizmo".into(), json!("light"));
    h.settle();
    let light = light_position(h.geometry(), revolve_options(&before));
    h.start(light, Modifiers::NONE);
    let p = light + vec2(20.0, 25.0);
    h.frame(vec![Event::PointerMoved(p)], Modifiers::NONE);
    h.release(p, Modifiers::NONE);
    assert_ne!(h.value("lightAzimuth"), -45.0);
    assert_ne!(h.value("lightElevation"), 45.0);
    h.frame(vec![escape()], Modifiers::NONE);
    assert!(h.app.ui.dialog.is_none() && !h.app.session.in_interaction());
}

#[test]
fn preview_off_has_no_gizmo_and_edge_on_rings_remain_finite() {
    let mut h = Harness::new();
    let d = h.app.ui.dialog.as_mut().unwrap();
    d.fields.insert("rotationX".into(), json!(0.0));
    d.fields.insert("rotationY".into(), json!(0.0));
    d.fields.insert("rotationZ".into(), json!(0.0));
    h.settle();
    h.ring_drag(0, Modifiers::NONE);
    assert!(h.value("rotationX").is_finite() && h.value("rotationX").abs() > 1.0);
    h.app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(false));
    h.settle();
    assert!(!h.app.session.in_interaction());
    let before = super::super::form::params(h.app.ui.dialog.as_ref().unwrap());
    h.ring_drag(2, Modifiers::NONE);
    assert_eq!(super::super::form::params(h.app.ui.dialog.as_ref().unwrap()), before);
}

#[test]
fn light_rings_adjust_only_their_angle_and_snap_without_rotating_the_object() {
    for (axis, key) in ["lightAzimuth", "lightElevation"].iter().enumerate() {
        let mut h = Harness::new();
        h.app.ui.dialog.as_mut().unwrap().fields.insert("__gizmo".into(), json!("light"));
        h.settle();
        let before = super::super::form::params(h.app.ui.dialog.as_ref().unwrap());
        let from = h.geometry().light_labels()[axis].center();
        h.start(from, Modifiers::SHIFT);
        let delta = if axis == 0 { vec2(68.0, 0.0) } else { vec2(0.0, -43.0) };
        h.frame(vec![Event::PointerMoved(from + delta)], Modifiers::SHIFT);
        h.release(from + delta, Modifiers::SHIFT);
        assert_ne!(h.value(key), before[*key].as_f64().unwrap());
        assert_eq!(h.value(key) % 15.0, 0.0);
        for unchanged in ["rotationX", "rotationY", "rotationZ", "lightIntensity", "ambient"] {
            assert_eq!(h.value(unchanged), before[unchanged].as_f64().unwrap());
        }
        let other = if axis == 0 { "lightElevation" } else { "lightAzimuth" };
        assert_eq!(h.value(other), before[other].as_f64().unwrap());
        let state = h.app.session.active().unwrap();
        let node = state.doc.node(state.selection.objects[0]).unwrap();
        assert_eq!(node.appearance.effects[0].params[*key], json!(h.value(key)));
    }
}

#[test]
fn light_handle_preserves_the_back_side_and_escape_cancel_and_undo_restore_changes() {
    let mut h = Harness::new();
    let original = h.app.session.active().unwrap().interaction.as_ref().unwrap().doc.clone();
    let d = h.app.ui.dialog.as_mut().unwrap();
    d.fields.insert("__gizmo".into(), json!("light"));
    d.fields.insert("lightAzimuth".into(), json!(135.0));
    h.settle();
    let before = super::super::form::params(h.app.ui.dialog.as_ref().unwrap());
    let light = light_position(h.geometry(), revolve_options(&before));
    h.start(light, Modifiers::NONE);
    let to = light + vec2(-15.0, 12.0);
    h.frame(vec![Event::PointerMoved(to)], Modifiers::NONE);
    assert!(revolve_options(&super::super::form::params(h.app.ui.dialog.as_ref().unwrap())).light_direction()[2] < 0.0);
    assert_ne!(h.value("lightElevation"), 45.0);
    h.frame(vec![escape()], Modifiers::NONE);
    h.release(to, Modifiers::NONE);
    assert_eq!(super::super::form::params(h.app.ui.dialog.as_ref().unwrap()), before);
    let state = h.app.session.active().unwrap();
    assert_eq!(state.doc.node(state.selection.objects[0]).unwrap().appearance.effects[0].params["lightAzimuth"], before["lightAzimuth"]);
    // A second gesture is kept by OK, and the whole dialog is one undo step.
    let from = h.geometry().light_labels()[0].center();
    h.start(from, Modifiers::NONE);
    h.frame(vec![Event::PointerMoved(from + vec2(35.0, 0.0))], Modifiers::NONE);
    h.release(from + vec2(35.0, 0.0), Modifiers::NONE);
    dialogs::confirm(&mut h.app).unwrap();
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(h.app.session.doc().unwrap().doc, original);
}

#[test]
fn light_elevation_is_bounded_and_shading_off_disables_the_light_gizmo() {
    let mut h = Harness::new();
    h.app.ui.dialog.as_mut().unwrap().fields.insert("__gizmo".into(), json!("light"));
    h.settle();
    let from = h.geometry().light_labels()[1].center();
    h.start(from, Modifiers::NONE);
    h.frame(vec![Event::PointerMoved(from - vec2(0.0, 400.0))], Modifiers::NONE);
    h.release(from - vec2(0.0, 400.0), Modifiers::NONE);
    assert_eq!(h.value("lightElevation"), 90.0);
    h.app.ui.dialog.as_mut().unwrap().fields.insert("shade".into(), json!(false));
    h.settle();
    let before = super::super::form::params(h.app.ui.dialog.as_ref().unwrap());
    let from = h.geometry().light_labels()[0].center();
    h.start(from, Modifiers::NONE);
    h.frame(vec![Event::PointerMoved(from + vec2(40.0, 0.0))], Modifiers::NONE);
    h.release(from + vec2(40.0, 0.0), Modifiers::NONE);
    assert_eq!(super::super::form::params(h.app.ui.dialog.as_ref().unwrap()), before);
}
