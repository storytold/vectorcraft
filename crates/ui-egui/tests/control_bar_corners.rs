//! Numeric selected-corner radii in the actual Control bar.
// Tests may unwrap/panic (AGENTS.md, Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard};

use craft::ui::shell::embed;
use egui::{Event, Rect, vec2};
use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{VectorcraftApp, chrome, i18n, ui_fonts};

static CASE: Mutex<()> = Mutex::new(());

struct Language {
    _lock: MutexGuard<'static, ()>,
    previous: i18n::Lang,
}
impl Language {
    fn english() -> Self {
        let lock = CASE.lock().unwrap_or_else(|e| e.into_inner());
        let previous = i18n::current();
        i18n::set_current(i18n::Lang::EN);
        Self { _lock: lock, previous }
    }
}
impl Drop for Language {
    fn drop(&mut self) {
        i18n::set_current(self.previous);
    }
}

fn ordinary_corner() -> (VectorcraftApp, NodeId) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 500, "height": 400, "profile": null})).unwrap();
    // A plain, original Pen-like path, not a live rectangle.
    let result = app
        .run(
            "path.create",
            json!({"anchors": [
        {"x": 100, "y": 100}, {"x": 220, "y": 100},
        {"x": 220, "y": 180}, {"x": 100, "y": 180}
    ], "closed": true}),
        )
        .unwrap();
    let id = NodeId(result["id"].as_u64().unwrap());
    app.select_tool("directSelection");
    app.run("select.anchors", json!({"id": id.0, "anchors": [[0, 1]], "mode": "set"})).unwrap();
    (app, id)
}

/// The roles a number field's accessibility node can have.
fn is_input(role: egui::accesskit::Role) -> bool {
    matches!(role, egui::accesskit::Role::TextInput | egui::accesskit::Role::NumberInput | egui::accesskit::Role::SpinButton)
}

/// What a frame of the bar exposed to assistive technology: the field and the texts are read from here.
struct Painted {
    nodes: Vec<egui::accesskit::Node>,
}
impl Painted {
    fn radius_field(&self) -> Option<&egui::accesskit::Node> {
        self.nodes.iter().find(|node| is_input(node.role()) && node.label().is_some_and(|name| name.trim_end_matches(':') == "Corner Radius"))
    }

    /// Whether a text of the bar (not an input) reads `name`.
    fn has_text(&self, name: &str) -> bool {
        self.nodes.iter().any(|node| !is_input(node.role()) && node.label() == Some(name))
    }
}

struct Frames {
    ctx: egui::Context,
    time: f64,
    width: f32,
}
impl Frames {
    fn new(width: f32) -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        embed::install(&ctx, craft::ui::testing::install_spec(), &ui_fonts()).unwrap();
        Self { ctx, time: 0.0, width }
    }

    /// One frame of the bar as the hosts run the window's: the app gives the frame's input, and the commands the
    /// pass fired run as the next frame begins.
    fn frame(&mut self, app: &mut VectorcraftApp, events: Vec<Event>) -> Painted {
        self.time += 0.05;
        app.adopt_context(&self.ctx);
        embed::before_input(&self.ctx, |out| app.frame_input(out));
        let mut output = self.ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(self.width, 1100.0))),
                time: Some(self.time),
                focused: true,
                events,
                ..Default::default()
            },
            |ui| chrome::control_bar(app, ui),
        );
        embed::before_input(&self.ctx, |out| app.frame_input(out));
        let nodes =
            output.platform_output.accesskit_update.take().map(|update| update.nodes.into_iter().map(|(_, node)| node).collect()).unwrap_or_default();
        output.textures_delta.clear();
        Painted { nodes }
    }

    fn settled(&mut self, app: &mut VectorcraftApp) -> Painted {
        self.frame(app, vec![]);
        self.frame(app, vec![]);
        self.frame(app, vec![])
    }
}

fn node_path(app: &VectorcraftApp, id: NodeId) -> vectorcraft_geom::PathData {
    app.session.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().clone()
}

fn corner_state(app: &VectorcraftApp, id: NodeId) -> (vectorcraft_geom::PathData, serde_json::Value, vectorcraft_doc::Selection, usize) {
    let state = app.session.doc().unwrap();
    let node = state.doc.node(id).unwrap();
    (node.path_data().unwrap().clone(), serde_json::to_value(node.live_shape()).unwrap(), state.selection.clone(), state.history.undo.len())
}

fn picked_corner(app: &mut VectorcraftApp, id: NodeId, corner: usize, mode: &str) {
    let anchors = {
        let node = app.session.doc().unwrap().doc.node(id).unwrap();
        vectorcraft_doc::LiveCorners::of(node).unwrap().anchors_of(&std::collections::BTreeSet::from([corner]))
    };
    app.run("select.anchors", json!({"id": id.0, "anchors": anchors, "mode": mode})).unwrap();
}

fn radius(app: &VectorcraftApp, id: NodeId, corner: usize) -> f64 {
    let node = app.session.doc().unwrap().doc.node(id).unwrap();
    vectorcraft_doc::LiveCorners::of(node).unwrap().radius(corner)
}

fn field_text(painted: &Painted) -> &str {
    painted.radius_field().unwrap().value().unwrap_or_default()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-8
}

impl Frames {
    fn edit_radius(&mut self, app: &mut VectorcraftApp, text: &str) -> Painted {
        let painted = self.settled(app);
        let bounds = painted.radius_field().unwrap().bounds().unwrap();
        let at = egui::pos2(((bounds.x0 + bounds.x1) / 2.0) as f32, ((bounds.y0 + bounds.y1) / 2.0) as f32);
        let button = |pressed| Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        // The framework hits the control the pointer rested on in the frame before the press.
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.frame(app, vec![button(true)]);
        self.frame(app, vec![button(false)]);
        let command =
            egui::Modifiers { command: true, ctrl: !cfg!(target_os = "macos"), mac_cmd: cfg!(target_os = "macos"), ..egui::Modifiers::NONE };
        let key = |key, pressed, modifiers| Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers };
        self.frame(app, vec![Event::ModifiersChanged(command), key(egui::Key::A, true, command)]);
        self.frame(app, vec![key(egui::Key::A, false, command), Event::ModifiersChanged(egui::Modifiers::NONE), Event::Text(text.into())]);
        self.frame(app, vec![key(egui::Key::Enter, true, egui::Modifiers::NONE)]);
        self.frame(app, vec![key(egui::Key::Enter, false, egui::Modifiers::NONE)]);
        self.settled(app)
    }
}

#[test]
fn corner_radius_typing_edits_only_the_selected_corner_and_undo_restores_it() {
    let _language = Language::english();
    let (mut app, id) = ordinary_corner();
    app.run("object.setLiveShape", json!({"id": id.0, "corners": [1], "radius": 160, "kind": "chamfer"})).unwrap();
    picked_corner(&mut app, id, 1, "set");
    let before = corner_state(&app, id);
    let base = {
        let node = app.session.doc().unwrap().doc.node(id).unwrap();
        let corners = vectorcraft_doc::LiveCorners::of(node).unwrap();
        assert!(corners.drawn_radius(1) < corners.radius(1), "the stored request exceeds the fitted radius");
        corners.base.clone().into_owned()
    };
    let mut frames = Frames::new(1800.0);
    assert_eq!(field_text(&frames.settled(&mut app)), "160 pt", "show requested absolute radius, not fitted size");
    frames.edit_radius(&mut app, "6 pt");
    let state = app.session.doc().unwrap();
    let node = state.doc.node(id).unwrap();
    let corners = vectorcraft_doc::LiveCorners::of(node).unwrap();
    assert_eq!(&*corners.base, &base, "the uncut outline does not move");
    assert!(close(corners.radius(1), 6.0));
    assert_eq!(corners.kind(1), vectorcraft_geom::shapes::CornerKind::Chamfer);
    for corner in [0, 2, 3] {
        assert_eq!(corners.radius(corner), 0.0);
        assert_eq!(corners.kind(corner), vectorcraft_geom::shapes::CornerKind::Round);
    }
    let logical = std::collections::BTreeSet::from([1]);
    assert_eq!(state.selection.objects, vec![id]);
    assert_eq!(corners.corners_of(state.selection.partial(id).unwrap()), logical);
    assert_eq!(state.selection.partial(id).unwrap(), &corners.anchors_of(&logical));
    assert_eq!(state.history.undo.len(), before.3 + 1);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(corner_state(&app, id), before);
}

#[test]
fn corner_radius_mixed_targets_resolve_relative_units_and_skip_noncorner_partials() {
    let _language = Language::english();
    let (mut app, ordinary) = ordinary_corner();
    app.run("object.setLiveShape", json!({"id": ordinary.0, "corners": [1], "radius": 6})).unwrap();
    let result = app.run("shape.rectangle", json!({"x": 300, "y": 100, "width": 100, "height": 100})).unwrap();
    let live = NodeId(result["id"].as_u64().unwrap());
    app.run("object.setLiveShape", json!({"id": live.0, "relativeRadii": true, "radius": 10})).unwrap();
    let result = app
        .run(
            "path.create",
            json!({"anchors": [
                {"x": 300, "y": 250}, {"x": 400, "y": 250},
                {"x": 400, "y": 350, "in": [400, 330], "out": [400, 370]},
                {"x": 450, "y": 400}
            ]}),
        )
        .unwrap();
    let excluded = NodeId(result["id"].as_u64().unwrap());
    let excluded_before = node_path(&app, excluded);
    let noncorners = std::collections::BTreeSet::from([(0, 0), (0, 2)]);
    {
        let node = app.session.doc().unwrap().doc.node(excluded).unwrap();
        let corners = vectorcraft_doc::LiveCorners::of(node).unwrap();
        assert!(!corners.all().is_empty(), "this path has another corner that fallback must not edit");
        assert!(corners.corners_of(&noncorners).is_empty(), "an endpoint and a smooth anchor are selected");
    }
    picked_corner(&mut app, ordinary, 1, "set");
    picked_corner(&mut app, live, 1, "add");
    app.run("select.anchors", json!({"id": excluded.0, "anchors": noncorners, "mode": "add"})).unwrap();
    app.run("document.setUnits", json!({"units": "Millimeters"})).unwrap();
    let mut frames = Frames::new(1800.0);
    assert!(field_text(&frames.settled(&mut app)).is_empty(), "6 pt and 10 pt are mixed");
    let depth = app.session.doc().unwrap().history.undo.len();
    let shown = frames.edit_radius(&mut app, "2 mm");
    let expected = 2.0 * 72.0 / 25.4;
    assert!(close(radius(&app, ordinary, 1), expected));
    assert_eq!(field_text(&shown), app.session.general_unit().format(expected));
    let node = app.session.doc().unwrap().doc.node(live).unwrap();
    let corners = vectorcraft_doc::LiveCorners::of(node).unwrap();
    assert!(corners.radii_relative());
    let maximum = corners.corner(1).unwrap().max_radius();
    assert!(close(corners.radius(1), expected / maximum));
    for corner in [0, 2, 3] {
        assert!(close(corners.radius(corner), 0.2));
        assert_eq!(radius(&app, ordinary, corner), 0.0);
    }
    assert_eq!(node_path(&app, excluded), excluded_before);
    assert!(app.session.doc().unwrap().doc.node(excluded).unwrap().live_shape().is_none());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), depth + 1, "all targets in one edit");
    // With only the non-corner partial left, there is no eligible field.
    app.run("select.anchors", json!({"id": excluded.0, "anchors": noncorners, "mode": "set"})).unwrap();
    assert!(frames.settled(&mut app).radius_field().is_none());
}

#[test]
fn corner_radius_respects_anchor_group_and_narrow_control_bar_space() {
    let _language = Language::english();
    let (mut app, id) = ordinary_corner();
    let mut wide = Frames::new(1800.0);
    let shown = wide.settled(&mut app);
    assert!(shown.radius_field().is_some() && shown.has_text("Corners"));
    app.run("window.control.options", json!({"group": "anchors", "visible": false})).unwrap();
    let hidden = wide.settled(&mut app);
    assert!(hidden.radius_field().is_none());
    assert!(!hidden.has_text("Corners"));
    app.run("window.control.options", json!({"group": "anchors", "visible": true})).unwrap();
    // The Corners link and the field are one row, so a narrow bar leaves out both or neither.
    let narrow = Frames::new(400.0).settled(&mut app);
    assert_eq!(narrow.radius_field().is_some(), narrow.has_text("Corners"), "the link and the field leave together");
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    app.select_tool("selection");
    assert!(wide.settled(&mut app).radius_field().is_none(), "ordinary whole-path Selection has no anchor controls");
    app.select_tool("pen");
    assert!(wide.settled(&mut app).radius_field().is_some(), "whole-path anchor-editing tools use all eligible corners");
}
