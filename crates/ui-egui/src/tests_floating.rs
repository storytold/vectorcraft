use egui::{Event, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use crate::VectorcraftApp;
use crate::floating::*;
use crate::state::{DockTab, FloatingPanel, PanelPlace};

const SCREEN: egui::Vec2 = vec2(1440.0, 900.0);

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
    app
}

/// One headless frame of the whole window.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>) {
    let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)), events, ..Default::default() };
    let mut out = ctx.run_ui(raw, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
}

/// Frames until the layout settles (`read_response` reports the frame before last).
fn settle(app: &mut VectorcraftApp, ctx: &egui::Context) {
    for _ in 0..4 {
        frame(app, ctx, vec![]);
    }
}

/// Where the widget `id` was drawn last frame.
fn rect_of(ctx: &egui::Context, id: egui::Id) -> Rect {
    ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} not drawn")).rect
}

/// Press at `from`, move to `to` in steps and release there.
fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, from: Pos2, to: Pos2) {
    let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, vec![Event::PointerMoved(from)]);
    frame(app, ctx, vec![button(from, true)]);
    for i in 1..=10 {
        frame(app, ctx, vec![Event::PointerMoved(from + (to - from) * (i as f32 / 10.0))]);
    }
    frame(app, ctx, vec![button(to, false)]);
    settle(app, ctx);
}

#[test]
fn a_dock_tab_dragged_out_floats_and_goes_back_when_dropped_on_the_dock() {
    let (mut app, ctx) = (app(), egui::Context::default());
    settle(&mut app, &ctx);
    let tab = rect_of(&ctx, handle_id("layers"));
    let to = egui::pos2(600.0, 300.0);
    drag(&mut app, &ctx, tab.center(), to);
    assert_eq!(place(&app, "layers"), Some(PanelPlace::Free));
    assert_eq!(docked_tabs(&app).iter().map(|t| t.0).collect::<Vec<_>>(), ["properties", "libraries"]);
    // It sits where it was dropped: grabbed by its heading, the heading is under the pointer.
    let area = ctx.memory(|m| m.area_rect(area_id("layers"))).unwrap();
    let heading = rect_of(&ctx, handle_id("layers"));
    assert!(heading.contains(to), "heading {heading:?} not under the drop point");
    assert!((area.width() - 300.0).abs() < 4.0, "{area:?}");
    // Dropped on the dock: back as the active tab.
    drag(&mut app, &ctx, heading.center(), egui::pos2(SCREEN.x - 120.0, 300.0));
    assert_eq!(place(&app, "layers"), None);
    assert_eq!(app.ui.dock_tab, DockTab::Layers);
    assert_eq!(docked_tabs(&app).len(), 3);
}

#[test]
fn a_panel_dropped_beside_the_toolbar_locks_there_and_the_canvas_makes_room() {
    let (mut app, ctx) = (app(), egui::Context::default());
    app.ui.open_panel = Some("color".into());
    settle(&mut app, &ctx);
    let canvas = app.canvas_rect.unwrap();
    let toolbar = toolbar_zone(&ctx).unwrap();
    // Tear the Color panel off the pop-out by its heading, then drop it next to the toolbar.
    let heading = rect_of(&ctx, handle_id("color"));
    drag(&mut app, &ctx, heading.left_center() + vec2(20.0, 0.0), egui::pos2(toolbar.right() + 30.0, 200.0));
    assert_eq!(place(&app, "color"), Some(PanelPlace::Toolbar));
    assert_eq!(app.ui.open_panel, None, "no second copy in the pop-out");
    let column = rect_of(&ctx, handle_id("color"));
    assert!((column.left() - toolbar.right()).abs() < 4.0, "locked against the toolbar: {column:?} vs {toolbar:?}");
    let moved = app.canvas_rect.unwrap();
    assert!((moved.left() - canvas.left() - 256.0).abs() < 3.0, "the canvas moves over: {canvas:?} → {moved:?}");
    // A second panel locks below the first.
    app.run("window.panelPlace", json!({"panel": "stroke", "place": "toolbar"})).unwrap();
    settle(&mut app, &ctx);
    assert!(rect_of(&ctx, handle_id("stroke")).top() > column.bottom());
    // Dragged away from the toolbar it floats again.
    drag(&mut app, &ctx, column.center(), egui::pos2(800.0, 400.0));
    assert_eq!(place(&app, "color"), Some(PanelPlace::Free));
    assert!(rect_of(&ctx, handle_id("color")).contains(egui::pos2(800.0, 400.0)));
}

#[test]
fn the_lock_button_and_the_command_lock_unlock_and_dock_panels() {
    let mut app = app();
    assert_eq!(app.run("window.panelPlace", json!({"panel": "swatches", "place": "free", "x": 500, "y": 220})).unwrap()["place"], "free");
    assert_eq!(app.ui.floating_panels[0].pos, [500.0, 220.0]);
    assert_eq!(app.run("window.panelPlace", json!({"panel": "swatches", "place": "toolbar"})).unwrap()["place"], "toolbar");
    // Window → Swatches shows it checked; choosing it again puts it back in the dock.
    assert_eq!(crate::menus::checked(&app, "window.panel", &json!({"panel": "swatches"})), Some(true));
    app.run("window.panel", json!({"panel": "swatches"})).unwrap();
    assert_eq!(place(&app, "swatches"), None);
    assert!(app.run("window.panelPlace", json!({"panel": "nope"})).is_err());
    assert!(app.run("window.panelPlace", json!({"panel": "layers", "place": "sideways"})).is_err());
    assert!(app.run("window.panelPlace", json!({"panel": "layers", "x": f64::MAX})).is_ok());
}

#[test]
fn panel_places_are_kept_between_sessions_and_reset_by_a_workspace() {
    let mut app = app();
    app.run("window.panelPlace", json!({"panel": "layers", "place": "free", "x": 400, "y": 300})).unwrap();
    app.run("window.panelPlace", json!({"panel": "align", "place": "toolbar"})).unwrap();
    let saved = serde_json::to_value(&app.ui).unwrap();
    let mut back: crate::state::UiState = serde_json::from_value(saved).unwrap();
    back.floating_panels.push(FloatingPanel { id: "gone".into(), ..Default::default() });
    back.floating_panels.push(FloatingPanel { id: "layers".into(), ..Default::default() });
    let back = back.sanitized();
    assert_eq!(back.floating_panels, app.ui.floating_panels, "unknown and repeated panels dropped");
    // A custom workspace keeps the layout; a built-in one puts every panel back.
    app.run("window.workspace.new", json!({"name": "Mine"})).unwrap();
    app.run("window.workspace", json!({"name": "Essentials"})).unwrap();
    assert!(app.ui.floating_panels.is_empty());
    app.run("window.workspace", json!({"name": "Mine"})).unwrap();
    assert_eq!(app.ui.floating_panels.len(), 2);
}

#[test]
fn a_floating_panel_stays_reachable_when_the_window_shrinks() {
    let (mut app, ctx) = (app(), egui::Context::default());
    app.run("window.panelPlace", json!({"panel": "info", "place": "free", "x": 5000, "y": -300})).unwrap();
    settle(&mut app, &ctx);
    let heading = rect_of(&ctx, handle_id("info"));
    assert!(heading.right() <= SCREEN.x + 256.0 && heading.left() < SCREEN.x - 40.0 && heading.top() >= 0.0, "{heading:?}");
}

/// The window-style dialogs (Preferences, Find Font, Keyboard Shortcuts, New Workspace) move when
/// dragged by their top margin and stay where they are dropped.
#[test]
fn window_dialogs_can_be_dragged_and_stay_where_dropped() {
    type Open = fn(&mut VectorcraftApp);
    let cases: [(&str, Open, &str); 4] = [
        ("dialog-preferences", |app| crate::prefs_dialog::open(app, None), "preferences"),
        ("dialog-find-font", |app| crate::find_font::open(app), "findFont"),
        ("dialog-shortcuts", |app| app.ui.dialog = Some(crate::state::Dialog::new("shortcuts", json!({}))), "shortcuts"),
        ("dialog-workspace", |app| app.ui.dialog = Some(crate::state::Dialog::new("newWorkspace", json!({"name": "W"}))), "newWorkspace"),
    ];
    for (window, open, kind) in cases {
        let (mut app, ctx) = (app(), egui::Context::default());
        open(&mut app);
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(kind));
        settle(&mut app, &ctx);
        let id = egui::Id::new(window);
        let before = ctx.memory(|m| m.area_rect(id)).unwrap_or_else(|| panic!("{window} not shown"));
        let grab = before.min + vec2(60.0, 6.0);
        // Towards the middle of the window (a dialog is kept on screen).
        let by = vec2(if before.center().x > SCREEN.x / 2.0 { -150.0 } else { 150.0 }, 0.0);
        // (Only across: some dialogs are nearly as tall as the window.)
        drag(&mut app, &ctx, grab, grab + by);
        let after = ctx.memory(|m| m.area_rect(id)).unwrap();
        let moved = after.min - before.min;
        assert!((moved.x - by.x).abs() < 2.0, "{window} moved by {moved:?}, not {by:?}");
        settle(&mut app, &ctx);
        assert_eq!(ctx.memory(|m| m.area_rect(id)).unwrap().min, after.min, "{window} stays where it was dropped");
    }
}
