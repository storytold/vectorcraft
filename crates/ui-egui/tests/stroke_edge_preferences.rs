//! Stroke-edge snapping's two UI controls share one persisted, off-by-default preference.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard};

use egui::{Event, Modifiers, PointerButton, Pos2, RawInput, Rect, Shape, vec2};
use serde_json::json;
use vectorcraft_engine::{Prefs, Session};
use vectorcraft_ui_egui::{UiState, VectorcraftApp, i18n, prefs_dialog, theme};

const KEY: &str = "snapToStrokeEdges";
const LABEL: &str = "Snap to Stroke Edges";
static UI_STATE: Mutex<()> = Mutex::new(());

struct Language {
    previous: i18n::Lang,
    _lock: MutexGuard<'static, ()>,
}

impl Language {
    fn english() -> Self {
        let lock = UI_STATE.lock().unwrap_or_else(|e| e.into_inner());
        let previous = i18n::current();
        i18n::set_current(i18n::Lang::EN);
        Self { previous, _lock: lock }
    }
}

impl Drop for Language {
    fn drop(&mut self) {
        i18n::set_current(self.previous);
    }
}

fn text_points(shape: &Shape, clip: Rect, out: &mut Vec<(String, Pos2)>) {
    match shape {
        Shape::Text(text) => {
            let point = text.pos + vec2(4.0, 6.0);
            if clip.contains(point) {
                out.push((text.galley.text().to_owned(), point));
            }
        }
        Shape::Vec(children) => children.iter().for_each(|shape| text_points(shape, clip, out)),
        _ => {}
    }
}

struct Scene {
    app: VectorcraftApp,
    ctx: egui::Context,
    time: f64,
    shown: Vec<(String, Pos2)>,
    nodes: Vec<egui::accesskit::Node>,
}

impl Scene {
    fn new() -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("app.language", json!({"lang": "en"})).unwrap();
        app.run("file.new", json!({"width": 500, "height": 350})).unwrap();
        let id = app.run("shape.rectangle", json!({"x": 70, "y": 60, "width": 120, "height": 80})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [id]})).unwrap();
        app.select_tool("selection");
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        theme::install_fonts(&ctx);
        theme::apply(&ctx, Default::default());
        let mut s = Self { app, ctx, time: 0.0, shown: vec![], nodes: vec![] };
        s.settle();
        s.app.take_prefs_changed();
        s
    }

    fn frame(&mut self, events: Vec<Event>) {
        self.time += 1.0 / 30.0;
        let mut raw = RawInput {
            events,
            time: Some(self.time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 1000.0))),
            focused: true,
            ..Default::default()
        };
        self.app.raw_input_hook(&self.ctx, &mut raw);
        let mut output = self.ctx.run_ui(raw, |ui| self.app.ui(ui));
        self.shown.clear();
        for clipped in &output.shapes {
            text_points(&clipped.shape, clipped.clip_rect, &mut self.shown);
        }
        self.nodes =
            output.platform_output.accesskit_update.take().map(|update| update.nodes.into_iter().map(|(_, node)| node).collect()).unwrap_or_default();
        output.textures_delta.clear();
    }

    fn settle(&mut self) {
        for _ in 0..4 {
            self.frame(vec![]);
        }
    }

    fn points(&self, label: &str, foreground: bool) -> Vec<Pos2> {
        self.shown
            .iter()
            .filter(|(text, point)| {
                text == label && (!foreground || self.ctx.layer_id_at(*point).is_some_and(|layer| layer.order == egui::Order::Foreground))
            })
            .map(|(_, point)| *point)
            .collect()
    }

    fn point(&self, label: &str, foreground: bool) -> Pos2 {
        let points = self.points(label, foreground);
        assert_eq!(points.len(), 1, "one painted {label:?} in its expected UI context; found {points:?}");
        points[0]
    }

    fn click(&mut self, pos: Pos2) {
        self.frame(vec![Event::ModifiersChanged(Modifiers::NONE), Event::PointerMoved(pos)]);
        self.frame(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE }]);
        self.frame(vec![Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE }]);
        self.settle();
    }

    fn value(&mut self) -> bool {
        self.app.run("prefs.get", json!({"key": KEY})).unwrap().as_bool().unwrap()
    }

    fn open_preferences(&mut self) {
        self.app.run("edit.preferences", json!({"category": "Smart Guides"})).unwrap();
        self.settle();
        let dialog = self.app.ui.dialog.as_ref().unwrap();
        assert_eq!(dialog.kind, "preferences");
        assert_eq!(dialog.str("__category"), "Smart Guides");
        assert_eq!(self.points("Preferences", true).len(), 1);
        // Scroll the real settings column if the new Geometric Guides row is below its viewport.
        if self.points(LABEL, true).is_empty() {
            let in_fields = self.point("Alignment Guides", true);
            for _ in 0..8 {
                self.frame(vec![
                    Event::PointerMoved(in_fields),
                    Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: vec2(0.0, -120.0),
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Unknown,
                        modifiers: Modifiers::NONE,
                    },
                ]);
                for _ in 0..10 {
                    self.frame(vec![]);
                }
                if !self.points(LABEL, true).is_empty() {
                    break;
                }
            }
        }
        self.checkbox_point();
    }

    fn checkbox_point(&self) -> Pos2 {
        let point = self.point(LABEL, true);
        let nodes: Vec<_> = self.nodes.iter().filter(|node| node.role() == egui::accesskit::Role::CheckBox && node.label() == Some(LABEL)).collect();
        assert_eq!(nodes.len(), 1, "the painted label must belong to one actual checkbox response");
        let bounds = nodes[0].bounds().expect("actual checkbox has accessible response bounds");
        let rect = Rect::from_min_max(egui::pos2(bounds.x0 as f32, bounds.y0 as f32), egui::pos2(bounds.x1 as f32, bounds.y1 as f32));
        assert!(rect.contains(point), "painted label click {point:?} must hit actual checkbox response {rect:?}");
        point
    }

    fn assert_checkbox_value(&self, expected: bool) {
        self.checkbox_point();
        let node = self.nodes.iter().find(|node| node.role() == egui::accesskit::Role::CheckBox && node.label() == Some(LABEL)).unwrap();
        let expected = if expected { egui::accesskit::Toggled::True } else { egui::accesskit::Toggled::False };
        assert_eq!(node.toggled(), Some(expected), "the actual checkbox response exposes the displayed value");
    }
}

#[test]
fn stroke_edge_preferences_generic_commands_and_saved_ui_restore_one_false_default() {
    let _language = Language::english();
    let mut s = Scene::new();
    assert!(!s.value());
    let mut legacy = Prefs::default().to_json();
    legacy.as_object_mut().unwrap().remove(KEY);
    let loaded: Prefs = serde_json::from_value(legacy).unwrap();
    assert_eq!(loaded.to_json()[KEY], json!(false), "old preference JSON keeps the opt-in off");
    let state = s.app.session.active().unwrap();
    let (document, selection, history) = (state.doc.clone(), state.selection.clone(), (state.history.undo.len(), state.history.redo.len()));
    let listed = s.app.run("prefs.list", json!({})).unwrap();
    let entries: Vec<_> = listed.as_array().unwrap().iter().filter(|entry| entry["key"] == KEY).collect();
    assert_eq!(entries.len(), 1);
    let entry = entries[0];
    assert_eq!(entry["category"], "Smart Guides");
    assert_eq!(entry["section"], "Geometric Guides");
    assert_eq!(entry["label"], LABEL);
    assert_eq!(entry["kind"], "bool");
    assert_eq!(entry["value"], false);
    s.app.run("prefs.set", json!({"key": "keyboardIncrement", "value": 3})).unwrap();
    assert_eq!(s.app.run("prefs.set", json!({"key": KEY, "value": true})).unwrap()[KEY], true);
    assert!(s.value());
    assert!(s.app.run("prefs.set", json!({"key": KEY, "value": "not a bool"})).is_err());
    assert!(s.value());
    // Use the same snapshot/UiState/restore path as the host; never touch user preference files.
    prefs_dialog::snapshot(&mut s.app);
    let bytes = serde_json::to_vec(&s.app.ui).unwrap();
    let mut reopened = VectorcraftApp::new(Session::new(), Default::default());
    reopened.ui = serde_json::from_slice::<UiState>(&bytes).unwrap().sanitized();
    prefs_dialog::restore(&mut reopened);
    assert_eq!(reopened.run("prefs.get", json!({"key": KEY})).unwrap(), true);
    reopened.run("prefs.reset", json!({"category": "Smart Guides"})).unwrap();
    assert_eq!(reopened.run("prefs.get", json!({"key": KEY})).unwrap(), false);
    assert_eq!(reopened.run("prefs.get", json!({"key": "keyboardIncrement"})).unwrap(), 3.0);
    prefs_dialog::snapshot(&mut reopened);
    assert_eq!(reopened.ui.engine_prefs[KEY], false);
    let mut legacy_ui = reopened.ui.clone();
    legacy_ui.engine_prefs.as_object_mut().unwrap().remove(KEY);
    let bytes = serde_json::to_vec(&legacy_ui).unwrap();
    let mut reset_reopened = VectorcraftApp::new(Session::new(), Default::default());
    reset_reopened.ui = serde_json::from_slice::<UiState>(&bytes).unwrap().sanitized();
    prefs_dialog::restore(&mut reset_reopened);
    assert_eq!(
        reset_reopened.run("prefs.get", json!({"key": KEY})).unwrap(),
        false,
        "legacy saved UiState without this key restores the false default"
    );
    let state = s.app.session.active().unwrap();
    assert_eq!(state.doc, document);
    assert_eq!(state.selection, selection);
    assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
}

#[test]
fn stroke_edge_preferences_painted_smart_guides_checkbox_is_draft_until_ok() {
    let _language = Language::english();
    let mut s = Scene::new();
    let state = s.app.session.active().unwrap();
    let (document, selection, history) = (state.doc.clone(), state.selection.clone(), (state.history.undo.len(), state.history.redo.len()));
    s.open_preferences();
    assert!(!s.app.ui.dialog.as_ref().unwrap().bool(KEY));
    s.assert_checkbox_value(false);
    s.click(s.checkbox_point());
    assert!(s.app.ui.dialog.as_ref().unwrap().bool(KEY), "actual checkbox changes the dialog draft");
    assert!(!s.value(), "draft is not a live tool preference");
    assert!(!s.app.take_prefs_changed());
    s.click(s.point("Cancel", true));
    assert!(s.app.ui.dialog.is_none());
    assert!(!s.value());
    assert!(!s.app.take_prefs_changed());
    s.open_preferences();
    assert!(!s.app.ui.dialog.as_ref().unwrap().bool(KEY), "Cancel discarded the changed draft");
    s.click(s.checkbox_point());
    s.click(s.point("OK", true));
    assert!(s.app.ui.dialog.is_none());
    assert!(s.value(), "OK applies the same engine preference");
    assert!(s.app.take_prefs_changed());
    s.open_preferences();
    assert!(s.app.ui.dialog.as_ref().unwrap().bool(KEY), "reopening reads the applied value");
    s.assert_checkbox_value(true);
    s.click(s.point("Cancel", true));
    let state = s.app.session.active().unwrap();
    assert_eq!(state.doc, document);
    assert_eq!(state.selection, selection);
    assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
}

#[test]
fn stroke_edge_preferences_control_bar_snapping_click_and_preferences_share_the_value() {
    let _language = Language::english();
    let mut s = Scene::new();
    s.app.run("window.control", json!({})).unwrap();
    assert!(s.app.ui.control_bar, "Window > Control shows the normally hidden bar");
    s.settle();
    let state = s.app.session.active().unwrap();
    let (document, selection, history) = (state.doc.clone(), state.selection.clone(), (state.history.undo.len(), state.history.redo.len()));
    assert!(!s.value());
    s.click(s.point("Snapping", false));
    assert!(s.app.ui.dialog.is_none(), "this checkbox is in the real Control bar popup");
    s.assert_checkbox_value(false);
    s.click(s.checkbox_point());
    assert!(s.value(), "actual popup checkbox applies the shared preference immediately");
    // The host's debounced save snapshots Session prefs; this menu route need not set the
    // immediate Preferences-confirmation flag. Exercise the existing public equivalent.
    prefs_dialog::snapshot(&mut s.app);
    let bytes = serde_json::to_vec(&s.app.ui).unwrap();
    let mut reopened = VectorcraftApp::new(Session::new(), Default::default());
    reopened.ui = serde_json::from_slice::<UiState>(&bytes).unwrap().sanitized();
    prefs_dialog::restore(&mut reopened);
    assert_eq!(reopened.run("prefs.get", json!({"key": KEY})).unwrap(), true, "the actual popup change survives the host snapshot/restore path");
    // Dismiss the menu normally before opening the other UI surface.
    if !s.points(LABEL, true).is_empty() {
        s.click(s.point("Snapping", false));
    }
    s.open_preferences();
    assert!(s.app.ui.dialog.as_ref().unwrap().bool(KEY));
    s.assert_checkbox_value(true);
    s.click(s.checkbox_point());
    assert!(s.value(), "the reverse change stays a draft until OK");
    s.click(s.point("OK", true));
    assert!(!s.value());
    assert!(s.app.take_prefs_changed());
    s.click(s.point("Snapping", false));
    s.assert_checkbox_value(false);
    assert!(!s.value(), "reopened Snapping popup reads the value applied by Preferences");
    let state = s.app.session.active().unwrap();
    assert_eq!(state.doc, document);
    assert_eq!(state.selection, selection);
    assert_eq!((state.history.undo.len(), state.history.redo.len()), history);
}
