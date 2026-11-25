//! Backtick arrow nudges use the real App input route and original generated pattern art.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use egui::{Event, Key, Modifiers, Pos2, RawInput};
use serde_json::json;
use std::sync::Mutex;
use vectorcraft_color::Paint;
use vectorcraft_doc::{Node, NodeId};
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::VectorcraftApp;

// Integration tests link the production global shortcut mirror; each App owns it until its case ends.
static INPUT: Mutex<()> = Mutex::new(());

struct Scene {
    app: VectorcraftApp,
    ctx: egui::Context,
    time: f64,
    fill: NodeId,
    stroke: NodeId,
    solid: NodeId,
    other: NodeId,
}

fn key(key: Key, pressed: bool, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }
}

impl Scene {
    fn new() -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let rect = |app: &mut VectorcraftApp, x, y, width, height| {
            NodeId(app.run("shape.rectangle", json!({"x": x, "y": y, "width": width, "height": height})).unwrap()["id"].as_u64().unwrap())
        };
        let seed = rect(&mut app, 0, 0, 10, 10);
        app.run("paint.setFill", json!({"ids": [seed.0], "color": "#ff0000"})).unwrap();
        app.run("paint.setStroke", json!({"ids": [seed.0], "none": true})).unwrap();
        app.run("select.set", json!({"ids": [seed.0]})).unwrap();
        app.run("object.pattern.make", json!({"name": "Dots", "width": 20, "height": 20})).unwrap();
        app.run("object.pattern.done", json!({})).unwrap();
        let fill = rect(&mut app, 80, 60, 80, 60);
        let stroke = rect(&mut app, 200, 60, 80, 60);
        let solid = rect(&mut app, 80, 170, 50, 40);
        let other = rect(&mut app, 200, 170, 50, 40);
        app.run("paint.setFill", json!({"ids": [fill.0, other.0], "swatch": "Dots"})).unwrap();
        app.run("paint.setStroke", json!({"ids": [fill.0, stroke.0], "swatch": "Dots"})).unwrap();
        app.run("paint.setFill", json!({"ids": [stroke.0], "none": true})).unwrap();
        app.run("paint.setFill", json!({"ids": [solid.0], "color": "#0077cc"})).unwrap();
        app.run("paint.setStroke", json!({"ids": [solid.0, other.0], "none": true})).unwrap();
        app.run("prefs.set", json!({"values": {"keyboardIncrement": 2, "transformPatternTiles": false}})).unwrap();
        app.run("select.set", json!({"ids": [fill.0, stroke.0, solid.0]})).unwrap();
        app.select_tool("selection");
        let ctx = egui::Context::default();
        let spec =
            craft::ui::InstallSpec::new(craft::host::target::ShellInfo::default(), std::sync::Arc::new(craft::ui::testing::windows::NoWindows));
        craft::ui::shell::embed::install(&ctx, spec, &vectorcraft_ui_egui::ui_fonts()).unwrap();
        let mut scene = Self { app, ctx, time: 0.0, fill, stroke, solid, other };
        for _ in 0..3 {
            scene.frame(vec![]);
        }
        assert!(!scene.ctx.egui_wants_keyboard_input(), "the actual idle canvas must own arrow input");
        scene
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 30.0;
        let raw = RawInput {
            events,
            time: Some(self.time),
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1200.0, 800.0))),
            ..Default::default()
        };
        // As the hosts run a frame: the app gives its input, and the commands the pass fired run as the next frame begins.
        craft::ui::shell::embed::before_input(&self.ctx, |out| self.app.frame_input(out));
        let mut out = self.ctx.run_ui(raw, |ui| self.app.ui(ui));
        craft::ui::shell::embed::before_input(&self.ctx, |out| self.app.frame_input(out));
        out.textures_delta.clear();
    }

    fn backtick(&mut self, pressed: bool) {
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE), key(Key::Backtick, pressed, Modifiers::NONE)]);
    }

    fn arrow(&mut self, arrow: Key, modifiers: Modifiers) {
        self.frame(vec![
            Event::ModifiersChanged(modifiers),
            key(arrow, true, modifiers),
            key(arrow, false, modifiers),
            Event::ModifiersChanged(Modifiers::NONE),
        ]);
    }

    fn node(&self, id: NodeId) -> Node {
        self.app.session.active().unwrap().doc.node(id).unwrap().clone()
    }

    fn history(&self) -> usize {
        self.app.session.active().unwrap().history.undo.len()
    }
}

fn pattern_shift(before: &Paint, after: &Paint, dx: f64, dy: f64) {
    let (Paint::Pattern { xf: a, pattern: name_a }, Paint::Pattern { xf: b, pattern: name_b }) = (before, after) else {
        panic!("both paints must remain editable patterns");
    };
    assert_eq!(name_a, name_b);
    let (a, b) = (a.as_coeffs(), b.as_coeffs());
    assert_eq!(&a[..4], &b[..4], "translation must preserve the pattern basis");
    assert!((b[4] - a[4] - dx).abs() < 1e-9 && (b[5] - a[5] - dy).abs() < 1e-9, "pattern placement {a:?} -> {b:?}, expected ({dx}, {dy})");
}

fn geometry_unchanged(before: &Node, after: &Node) {
    let mut normalized = after.clone();
    normalized.appearance = before.appearance.clone();
    assert_eq!(&normalized, before, "pattern nudging must preserve every non-appearance object field");
}

#[test]
fn pattern_nudge_baseline_backtick_arrows_move_paints_not_mixed_art() {
    let _input = INPUT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = Scene::new();
    let (fill, stroke, solid, other) = (s.node(s.fill), s.node(s.stroke), s.node(s.solid), s.node(s.other));
    let selection = s.app.session.active().unwrap().selection.clone();
    let definitions = s.app.session.active().unwrap().doc.patterns.clone();
    let history = s.history();
    s.backtick(true);
    s.arrow(Key::ArrowRight, Modifiers::NONE);
    s.backtick(false);
    let (a, b) = (s.node(s.fill), s.node(s.stroke));
    // Assert paint movement first: unchanged main follows ordinary geometry movement instead.
    pattern_shift(&fill.appearance.fill_paint(), &a.appearance.fill_paint(), 2.0, 0.0);
    pattern_shift(&fill.appearance.stroke_paint(), &a.appearance.stroke_paint(), 2.0, 0.0);
    pattern_shift(&stroke.appearance.stroke_paint(), &b.appearance.stroke_paint(), 2.0, 0.0);
    geometry_unchanged(&fill, &a);
    geometry_unchanged(&stroke, &b);
    assert_eq!(s.node(s.solid), solid);
    assert_eq!(s.node(s.other), other, "unselected user of the same definition stays unchanged");
    assert_eq!(s.app.session.active().unwrap().doc.patterns, definitions);
    assert_eq!(s.app.session.active().unwrap().selection, selection);
    assert_eq!(s.history(), history + 1);
    s.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(s.node(s.fill), fill);
    assert_eq!(s.node(s.stroke), stroke);
    assert_eq!(s.node(s.solid), solid);
}

#[test]
fn pattern_nudge_baseline_ordinary_arrow_keeps_existing_geometry_behavior() {
    let _input = INPUT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = Scene::new();
    let before = s.node(s.fill);
    let pattern = before.appearance.fill_paint();
    let other = s.node(s.other);
    let history = s.history();
    s.arrow(Key::ArrowRight, Modifiers::NONE);
    let after = s.node(s.fill);
    let (a, b) = (before.geometric_bounds().unwrap(), after.geometric_bounds().unwrap());
    assert!((b.x0 - a.x0 - 2.0).abs() < 1e-9 && b.y0 == a.y0);
    assert_eq!(after.appearance.fill_paint(), pattern, "ordinary nudge respects Transform Pattern Tiles off");
    assert_eq!(s.node(s.other), other);
    assert_eq!(s.history(), history + 1);
    s.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(s.node(s.fill), before);
}

#[test]
fn pattern_nudge_partial_anchors_and_idle_shape_tool_preserve_owner_geometry() {
    let _input = INPUT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for tool in ["directSelection", "rectangle"] {
        let mut s = Scene::new();
        s.app.run("select.set", json!({"ids": [s.fill.0]})).unwrap();
        s.app.select_tool(tool);
        if tool == "directSelection" {
            s.app.run("select.anchors", json!({"id": s.fill.0, "anchors": [[0, 0]], "mode": "set"})).unwrap();
        }
        s.frame(vec![]);
        let before = s.node(s.fill);
        let selection = s.app.session.active().unwrap().selection.clone();
        let history = s.history();
        s.backtick(true);
        s.arrow(Key::ArrowDown, Modifiers::NONE);
        s.backtick(false);
        let after = s.node(s.fill);
        pattern_shift(&before.appearance.fill_paint(), &after.appearance.fill_paint(), 0.0, 2.0);
        geometry_unchanged(&before, &after);
        assert_eq!(s.app.session.active().unwrap().selection, selection);
        assert_eq!(s.history(), history + 1);
        s.app.run("edit.undo", json!({})).unwrap();
        assert_eq!(s.node(s.fill), before);
        s.arrow(Key::ArrowDown, Modifiers::NONE);
        let ordinary = s.node(s.fill);
        assert_eq!(ordinary.appearance.fill_paint(), before.appearance.fill_paint());
        if tool == "directSelection" {
            let a: Vec<_> = before.path_data().unwrap().anchors().map(|(_, _, a)| a.p).collect();
            let b: Vec<_> = ordinary.path_data().unwrap().anchors().map(|(_, _, a)| a.p).collect();
            assert_eq!(a.len(), b.len());
            assert_eq!(b[0], a[0] + vectorcraft_geom::Vec2::new(0.0, 2.0));
            assert_eq!(&b[1..], &a[1..], "ordinary Direct Selection nudges only the selected anchor");
        } else {
            let (a, b) = (before.geometric_bounds().unwrap(), ordinary.geometric_bounds().unwrap());
            assert!((b.y0 - a.y0 - 2.0).abs() < 1e-9 && b.x0 == a.x0);
        }
    }
}

#[test]
fn pattern_nudge_shift_constrain_release_and_undo_use_press_time_increment() {
    let _input = INPUT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = Scene::new();
    s.app.run("prefs.set", json!({"values": {"keyboardIncrement": 3, "constrainAngle": 90}})).unwrap();
    let before = s.node(s.fill);
    let solid = s.node(s.solid);
    let history = s.history();
    s.backtick(true);
    s.arrow(Key::ArrowRight, Modifiers::SHIFT);
    let after = s.node(s.fill);
    pattern_shift(&before.appearance.fill_paint(), &after.appearance.fill_paint(), 0.0, -30.0);
    geometry_unchanged(&before, &after);
    assert_eq!(s.node(s.solid), solid);
    assert_eq!(s.history(), history + 1);
    s.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(s.node(s.fill), before);
    s.app.run("edit.redo", json!({})).unwrap();
    assert_eq!(s.node(s.fill), after);
    s.backtick(false);
    s.arrow(Key::ArrowRight, Modifiers::NONE);
    let ordinary = s.node(s.fill);
    let (a, b) = (after.geometric_bounds().unwrap(), ordinary.geometric_bounds().unwrap());
    assert!((b.y0 - a.y0 + 3.0).abs() < 1e-9 && b.x0 == a.x0);
    assert_eq!(ordinary.appearance.fill_paint(), after.appearance.fill_paint());
    s.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(s.node(s.fill), after);
}

#[test]
fn pattern_nudge_same_frame_event_order_uses_backtick_at_arrow_press() {
    let _input = INPUT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = Scene::new();
    let before = s.node(s.fill);
    s.backtick(true);
    s.frame(vec![
        key(Key::ArrowRight, true, Modifiers::NONE),
        key(Key::Backtick, false, Modifiers::NONE),
        key(Key::ArrowRight, false, Modifiers::NONE),
    ]);
    let after = s.node(s.fill);
    pattern_shift(&before.appearance.fill_paint(), &after.appearance.fill_paint(), 2.0, 0.0);
    geometry_unchanged(&before, &after);
    s.arrow(Key::ArrowRight, Modifiers::NONE);
    let ordinary = s.node(s.fill);
    assert!((ordinary.geometric_bounds().unwrap().x0 - after.geometric_bounds().unwrap().x0 - 2.0).abs() < 1e-9);
    assert_eq!(ordinary.appearance.fill_paint(), after.appearance.fill_paint());

    let mut s = Scene::new();
    let before = s.node(s.fill);
    s.frame(vec![
        key(Key::ArrowRight, true, Modifiers::NONE),
        key(Key::Backtick, true, Modifiers::NONE),
        key(Key::ArrowRight, false, Modifiers::NONE),
    ]);
    s.backtick(false);
    let after = s.node(s.fill);
    assert!((after.geometric_bounds().unwrap().x0 - before.geometric_bounds().unwrap().x0 - 2.0).abs() < 1e-9);
    assert_eq!(after.appearance.fill_paint(), before.appearance.fill_paint(), "a later Backtick press cannot reinterpret an earlier arrow press");
}

#[test]
fn pattern_nudge_solid_and_guide_only_are_noops_but_ordinary_arrows_work() {
    let _input = INPUT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = Scene::new();
    s.app.run("select.set", json!({"ids": [s.solid.0]})).unwrap();
    let before = s.app.session.active().unwrap().doc.clone();
    let selection = s.app.session.active().unwrap().selection.clone();
    let history = s.history();
    s.backtick(true);
    s.arrow(Key::ArrowRight, Modifiers::NONE);
    s.backtick(false);
    assert_eq!(s.app.session.active().unwrap().doc, before);
    assert_eq!(s.app.session.active().unwrap().selection, selection);
    assert_eq!(s.history(), history);
    let solid = s.node(s.solid);
    s.arrow(Key::ArrowRight, Modifiers::NONE);
    assert!((s.node(s.solid).geometric_bounds().unwrap().x0 - solid.geometric_bounds().unwrap().x0 - 2.0).abs() < 1e-9);

    s.app.run("view.guides.lock", json!({"locked": false})).unwrap();
    let index = s.app.run("guide.add", json!({"vertical": true, "pos": 40})).unwrap()["index"].as_u64().unwrap();
    s.app.run("guide.select", json!({"indexes": [index]})).unwrap();
    let before = s.app.session.active().unwrap().doc.clone();
    let selection = s.app.session.active().unwrap().selection.clone();
    let history = s.history();
    let guides = s.app.run("guide.list", json!({})).unwrap();
    s.backtick(true);
    s.arrow(Key::ArrowRight, Modifiers::NONE);
    s.backtick(false);
    assert_eq!(s.app.session.active().unwrap().doc, before);
    assert_eq!(s.app.session.active().unwrap().selection, selection);
    assert_eq!(s.history(), history);
    s.arrow(Key::ArrowRight, Modifiers::NONE);
    let moved = s.app.run("guide.list", json!({})).unwrap();
    assert_eq!(moved[index as usize]["pos"].as_f64().unwrap(), guides[index as usize]["pos"].as_f64().unwrap() + 2.0);
    assert_eq!(s.history(), history + 1);
    s.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(s.app.session.active().unwrap().doc, before);
}

#[test]
fn pattern_nudge_user_shortcut_precedes_global_arrow_dispatch() {
    let _input = INPUT.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut s = Scene::new();
    s.app.run("shortcuts.set", json!({"id": "select.none", "shortcut": "Right", "force": true})).unwrap();
    let before = s.app.session.active().unwrap().doc.clone();
    let history = s.history();
    s.backtick(true);
    s.arrow(Key::ArrowRight, Modifiers::NONE);
    s.backtick(false);
    let after = s.app.session.active().unwrap().doc.clone();
    let empty = s.app.session.active().unwrap().selection.is_empty();
    let after_history = s.history();
    // Reset the global command map before assertions, including a failing ownership assertion.
    s.app.run("shortcuts.set", json!({"id": "select.none", "shortcut": null})).unwrap();
    assert!(empty, "the user's bound Select None command owns this arrow press");
    assert_eq!(after, before);
    assert_eq!(after_history, history);
}
