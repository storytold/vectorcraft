//! Split comparison commands keep per-view settings separate from art and ordinary display modes.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::{Mutex, MutexGuard};

use serde_json::{Value, json};
use vectorcraft_color::cms::{self, ColorSettings};
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;
use vectorcraft_render::proof::{self, ProofView};
use vectorcraft_ui_egui::VectorcraftApp;
use vectorcraft_ui_egui::menus::{self, Item};
use vectorcraft_ui_egui::state::{SplitMode, SplitOrientation, SplitView, View};

static GLOBALS: Mutex<()> = Mutex::new(());

struct GlobalState {
    cms: ColorSettings,
    proof: ProofView,
    _lock: MutexGuard<'static, ()>,
}

impl GlobalState {
    fn new() -> Self {
        let lock = GLOBALS.lock().unwrap_or_else(|e| e.into_inner());
        Self { cms: cms::active_settings(), proof: proof::view(), _lock: lock }
    }
}

impl Drop for GlobalState {
    fn drop(&mut self) {
        cms::set_active(&self.cms).unwrap();
        proof::set_view(self.proof.clone());
    }
}

const COMMANDS: [&str; 9] = [
    "view.splitView",
    "view.splitVertical",
    "view.splitHorizontal",
    "view.splitPreview",
    "view.splitOutline",
    "view.splitPixelPreview",
    "view.splitOverprintPreview",
    "view.splitTrimView",
    "view.splitReset",
];

fn run(app: &mut VectorcraftApp, id: &str) -> Value {
    app.run(id, json!({})).unwrap()
}

fn document_state(app: &VectorcraftApp) -> Value {
    let state = app.session.active().unwrap();
    json!({"doc": state.doc, "selection": state.selection,
        "undo": state.history.undo.len(), "redo": state.history.redo.len()})
}

#[test]
fn split_view_defaults_legacy_state_and_saved_camera_remain_off() {
    let _globals = GlobalState::new();
    let view = View::default();
    assert!(view.split.is_default());
    let old_wire = serde_json::to_value(view).unwrap();
    assert!(old_wire.get("split").is_none(), "ordinary views keep their pre-feature wire shape");
    let old: View = serde_json::from_value(old_wire).unwrap();
    assert_eq!(old.split, SplitView::default());
    assert_eq!(serde_json::from_value::<SplitView>(json!({})).unwrap(), SplitView::default());
    let configured = View {
        split: SplitView { enabled: true, orientation: SplitOrientation::Horizontal, fraction: 0.25, comparison: SplitMode::PixelPreview },
        ..view
    };
    let wire = serde_json::to_value(configured).unwrap();
    assert_eq!(wire["split"]["orientation"], "horizontal");
    assert_eq!(wire["split"]["comparison"], "pixelPreview");
    let restored: View = serde_json::from_value(wire).unwrap();
    assert_eq!(restored.split, configured.split);
    for (fraction, expected) in [(f32::NAN, 0.5), (f32::INFINITY, 0.5), (f32::NEG_INFINITY, 0.5), (-0.25, 0.0), (1.25, 1.0), (0.3, 0.3)] {
        assert_eq!(SplitView { fraction, ..Default::default() }.ratio(), expected);
    }
    let saved = vectorcraft_engine::doc::SavedView { name: "camera".into(), center: Point::new(50.0, 70.0), zoom: 2.0, rotation: 30.0 };
    let camera = View::saved(&saved).unwrap();
    assert_eq!(camera.center, saved.center);
    assert_eq!(camera.rotation, 30.0);
    assert!(camera.split.is_default(), "document SavedView does not acquire a UI split format");
}

#[test]
fn split_view_commands_are_discoverable_checked_and_leave_art_and_main_modes_unchanged() {
    let _globals = GlobalState::new();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let tree = menus::menu_tree();
    let (_, view) = tree.iter().find(|(name, _)| *name == "View").unwrap();
    let split = view
        .iter()
        .find_map(|item| match item {
            Item::Sub("Split View", children) => Some(children),
            _ => None,
        })
        .unwrap();
    let ids: Vec<_> = split
        .iter()
        .filter_map(|item| match item {
            Item::Cmd(_, id, _) => Some(*id),
            _ => None,
        })
        .collect();
    assert_eq!(ids, COMMANDS, "one discoverable submenu contains the complete command surface");
    for id in COMMANDS {
        let rows: Vec<_> = menus::UI_COMMANDS.iter().filter(|row| row.0 == id).collect();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2, "", "no default shortcut");
        assert!(rows[0].3.starts_with("{}"));
        assert!(!menus::enabled(&app, id), "no document: {id}");
        assert!(app.run(id, json!({})).is_err(), "no view to mutate: {id}");
    }
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 30, "height": 40})).unwrap();
    run(&mut app, "view.outline");
    run(&mut app, "view.pixelPreview");
    run(&mut app, "view.trimView");
    run(&mut app, "view.overprintPreview");
    let main = (app.outline_view(), app.ui.view.pixel_preview, app.ui.view.trim_view, proof::view());
    let art = document_state(&app);
    assert_eq!(menus::checked(&app, "view.splitView", &Value::Null), Some(false));
    run(&mut app, "view.splitHorizontal");
    assert!(!app.view().unwrap().split.enabled, "choosing an option does not enable the comparison");
    assert_eq!(menus::checked(&app, "view.splitHorizontal", &Value::Null), Some(true));
    assert_eq!(menus::checked(&app, "view.splitVertical", &Value::Null), Some(false));
    run(&mut app, "view.splitView");
    assert_eq!(menus::checked(&app, "view.splitView", &Value::Null), Some(true));
    for (id, mode) in [
        ("view.splitPreview", SplitMode::Preview),
        ("view.splitOutline", SplitMode::Outline),
        ("view.splitPixelPreview", SplitMode::PixelPreview),
        ("view.splitOverprintPreview", SplitMode::OverprintPreview),
        ("view.splitTrimView", SplitMode::TrimView),
    ] {
        run(&mut app, id);
        assert_eq!(app.view().unwrap().split.comparison, mode);
        assert_eq!(menus::checked(&app, id, &Value::Null), Some(true));
        assert_eq!((app.outline_view(), app.ui.view.pixel_preview, app.ui.view.trim_view, proof::view()), main);
        assert_eq!(document_state(&app), art);
    }
    app.view_mut().unwrap().split.fraction = 0.8;
    let before = app.view().unwrap().split;
    run(&mut app, "view.splitReset");
    assert_eq!(app.view().unwrap().split, SplitView { fraction: 0.5, ..before });
    run(&mut app, "view.splitVertical");
    run(&mut app, "view.splitView");
    let disabled = app.view().unwrap().split;
    assert!(!disabled.enabled);
    assert_eq!(disabled.comparison, SplitMode::TrimView);
    assert_eq!(disabled.orientation, SplitOrientation::Vertical);
    assert_eq!(document_state(&app), art);
    assert_eq!((app.outline_view(), app.ui.view.pixel_preview, app.ui.view.trim_view, proof::view()), main);
}

#[test]
fn split_view_settings_follow_their_document_across_tabs_and_view_roundtrip() {
    let _globals = GlobalState::new();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    let first_uid = app.session.active().unwrap().uid;
    run(&mut app, "view.splitView");
    run(&mut app, "view.splitHorizontal");
    run(&mut app, "view.splitOverprintPreview");
    app.view_mut().unwrap().split.fraction = 0.3;
    let first = app.view().unwrap().split;
    let wire = serde_json::to_value(app.view().unwrap()).unwrap();
    assert_eq!(serde_json::from_value::<View>(wire).unwrap().split, first);
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    let second_uid = app.session.active().unwrap().uid;
    assert_ne!(first_uid, second_uid);
    assert!(app.view().unwrap().split.is_default());
    run(&mut app, "view.splitView");
    run(&mut app, "view.splitPixelPreview");
    let second = app.view().unwrap().split;
    app.run("document.activate", json!({"index": 0})).unwrap();
    assert_eq!(app.session.active().unwrap().uid, first_uid);
    assert_eq!(app.view().unwrap().split, first);
    app.run("document.activate", json!({"index": 1})).unwrap();
    assert_eq!(app.session.active().unwrap().uid, second_uid);
    assert_eq!(app.view().unwrap().split, second);
    app.run("document.move", json!({"from": 1, "to": 0})).unwrap();
    assert_eq!(app.session.active().unwrap().uid, second_uid);
    assert_eq!(app.view().unwrap().split, second);
    app.run("document.activate", json!({"index": 1})).unwrap();
    assert_eq!(app.session.active().unwrap().uid, first_uid);
    assert_eq!(app.view().unwrap().split, first);
}
