//! The Simplify and Smooth sliders' strip and the Simplify dialog it opens, driven through the window's frame.

// PLAN: `mod tests_scroll;` (dialogs.rs) goes with the deleted `tests_scroll.rs`; the line is hub-w2's.

use egui::accesskit::Role;
use egui::{Event, Pos2, Rect, Vec2, pos2, vec2};
use serde_json::json;
use vectorcraft_engine::Session;

use super::tests::{SCREEN, confirm, frame, set, settle, window};
use super::*;
use crate::VectorcraftApp;

/// The name of the strip's slider in the access tree: the command's, as production named it.
const STRIP_SLIDER: &str = "Simplify";

fn key(code: egui::Key) -> Event {
    Event::Key { key: code, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// Drag with the mouse from `from` by `by`, in steps, then let go.
fn drag(app: &mut VectorcraftApp, from: Pos2, by: Vec2) {
    frame(app, vec![Event::PointerMoved(from), button(from, true)]);
    for i in 1..=6 {
        frame(app, vec![Event::PointerMoved(from + by * (i as f32 / 6.0))]);
    }
    frame(app, vec![button(from + by, false)]);
    frame(app, vec![]);
}

fn click(app: &mut VectorcraftApp, pos: Pos2) {
    frame(app, vec![Event::PointerMoved(pos), button(pos, true)]);
    frame(app, vec![button(pos, false)]);
    frame(app, vec![]);
}

/// The strip's slider, found by its role and name in the access tree.
fn strip_slider(app: &mut VectorcraftApp) -> Rect {
    settle(app);
    let out = frame(app, vec![]);
    let update = out.platform_output.accesskit_update.expect("an accessibility tree");
    let (_, node) = update
        .nodes
        .iter()
        .find(|(_, node)| node.role() == Role::Slider && node.label().is_some_and(|name| name.contains(STRIP_SLIDER)))
        .expect("the strip's slider");
    let b = node.bounds().expect("bounds");
    Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32))
}

/// Drags the strip's slider to its start (level 0) or its end (level 100).
fn drag_strip(app: &mut VectorcraftApp, to_end: bool) {
    let rail = strip_slider(app);
    let by = rail.width() * if to_end { 2.0 } else { -2.0 };
    drag(app, rail.center(), vec2(by, 0.0));
}

fn app_with_dense_path() -> VectorcraftApp {
    let mut app = window(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let anchors: Vec<_> = (0..20).map(|i| json!({"x": 30 + i * 15, "y": 100.0 + (i % 2) as f64 * 0.1})).collect();
    let id = app.run("path.create", json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap();
    app.run("select.anchors", json!({"id": id, "anchors": [[0, 19]]})).unwrap();
    app
}

/// Object › Path › Simplify, then the slider's More Options: the Simplify dialog.
fn open_simplify_dialog(app: &mut VectorcraftApp) {
    crate::menus::invoke(app, "object.path.simplify", json!({}));
    simplify_slider::more_options(app).unwrap();
}

/// The anchors of the paths in `app`'s document.
fn anchor_count(app: &VectorcraftApp) -> usize {
    let mut n = 0;
    app.session.doc().unwrap().doc.walk(|node| {
        if let vectorcraft_doc::NodeKind::Path { path, .. } = &node.kind {
            n += path.anchor_count();
        }
    });
    n
}

/// The parameters of the Simplify preview on the canvas.
fn simplify_preview(app: &VectorcraftApp) -> serde_json::Value {
    app.session.doc().unwrap().interaction.as_ref().and_then(|it| it.preview.clone()).expect("a preview").1
}

/// Object › Path › Simplify opens a slider under the selection that previews the auto-simplified result (the path is
/// 285 pt wide: level 55, 0.7125 pt). Dragging it to the right previews more anchor points and to the left fewer, and
/// Enter keeps the result as one undo step.
#[test]
fn simplify_opens_a_slider_whose_drag_previews_fewer_or_more_points() {
    let mut app = app_with_dense_path();
    let undo_len = app.session.doc().unwrap().history.undo.len();
    crate::menus::invoke(&mut app, "object.path.simplify", json!({}));
    assert!(simplify_slider::showing(&app));
    settle(&mut app);
    assert_eq!(simplify_preview(&app), json!({"level": 55.0}));
    assert_eq!(anchor_count(&app), 2, "the auto-simplified result");
    drag_strip(&mut app, true);
    assert_eq!(simplify_preview(&app), json!({"level": 100.0}));
    let most = anchor_count(&app);
    assert!(most > 2, "{most}");
    drag_strip(&mut app, false);
    assert_eq!(simplify_preview(&app), json!({"level": 0.0}));
    assert_eq!(anchor_count(&app), 2);
    frame(&mut app, vec![key(egui::Key::Enter)]);
    assert!(!simplify_slider::showing(&app) && !app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo_len + 1);
    assert_eq!(anchor_count(&app), 2);
}

/// A wave of 20 anchors `width` pt wide and `width / 5` pt high, selected.
fn app_with_wave(width: f64) -> VectorcraftApp {
    let mut app = window(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 2000, "height": 2000})).unwrap();
    let anchors: Vec<_> = (0..20)
        .map(|i| {
            let t = i as f64 / 19.0;
            json!({"x": 30.0 + width * t, "y": 500.0 + width / 5.0 * (3.0 * std::f64::consts::PI * t).sin()})
        })
        .collect();
    app.run("path.create", json!({"anchors": anchors})).unwrap();
    app
}

/// Object › Path › Simplify opens the slider at Auto-Simplify's level for the selection's size, not at a fixed level, and
/// the canvas previews the paths as Auto-Simplify leaves them.
#[test]
fn simplify_slider_opens_at_the_auto_simplify_level_of_the_selection() {
    // 1/400 of the longer side: 2.85 pt (level 35), 0.7125 pt (55), 0.07125 pt (89).
    for (width, level) in [(1140.0, 35.0), (285.0, 55.0), (28.5, 89.0)] {
        let mut app = app_with_wave(width);
        crate::menus::invoke(&mut app, "object.path.simplify", json!({}));
        assert_eq!(simplify_slider::opening_level(&app, simplify_slider::StripKind::Simplify), level, "{width} pt");
        settle(&mut app);
        assert_eq!(simplify_preview(&app), json!({ "level": level }), "{width} pt");
        let previewed = app.session.doc().unwrap().doc.clone();
        frame(&mut app, vec![key(egui::Key::Escape)]);
        assert!(!simplify_slider::showing(&app), "{width} pt");
        let auto = app.run("object.path.simplify", json!({"auto": true})).unwrap();
        assert_eq!(auto["level"], json!(level), "{width} pt");
        assert_eq!(app.session.doc().unwrap().doc, previewed, "{width} pt: the preview is Auto-Simplify's result");
    }
}

/// Escape closes the Simplify slider and puts the paths back as they were, with no undo step.
#[test]
fn simplify_slider_escape_puts_the_paths_back() {
    let mut app = app_with_dense_path();
    let before = app.session.doc().unwrap().doc.clone();
    let undo_len = app.session.doc().unwrap().history.undo.len();
    crate::menus::invoke(&mut app, "object.path.simplify", json!({}));
    // Level 0's tolerance of 32 pt takes out the path's 0.1 pt zigzag; level 100's 1/32 pt keeps it.
    drag_strip(&mut app, false);
    assert_eq!(simplify_preview(&app), json!({"level": 0.0}));
    assert_ne!(app.session.doc().unwrap().doc, before, "the preview changes the path");
    frame(&mut app, vec![key(egui::Key::Escape)]);
    assert!(!simplify_slider::showing(&app) && !app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo_len);
    assert_eq!(anchor_count(&app), 20);
}

/// The Simplify slider stays where it opened, under the paths as they were, while the preview changes their bounds, so a
/// drag doesn't move the rail under the pointer.
#[test]
fn simplify_slider_stays_where_it_opened_while_the_preview_changes_the_bounds() {
    let mut app = window(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.view_mut().unwrap().rect = Rect::from_min_size(Pos2::ZERO, SCREEN);
    // A hump 20 pt high, which level 0 (32 pt) fits with a lower curve, seen at 400%.
    let anchors: Vec<_> = (0..20).map(|i| json!({"x": 30 + i * 15, "y": 100.0 + 20.0 * (std::f64::consts::PI * i as f64 / 19.0).sin()})).collect();
    app.run("path.create", json!({"anchors": anchors})).unwrap();
    app.run("view.setZoom", json!({"zoom": 400})).unwrap();
    app.view_mut().unwrap().center = vectorcraft_geom::Point::new(172.5, 60.0);
    let before = app.selection_bounds().unwrap();
    crate::menus::invoke(&mut app, "object.path.simplify", json!({}));
    let opened = strip_slider(&mut app);
    drag_strip(&mut app, false);
    assert_eq!(simplify_preview(&app), json!({"level": 0.0}));
    let after = app.selection_bounds().unwrap();
    assert!(after.height() < before.height() - 0.5, "the preview changes the bounds: {before:?} → {after:?}");
    assert_eq!(strip_slider(&mut app), opened);
}

/// Auto-Simplify moves the slider back to the level that keeps the path within 1/400 of its size, and a click outside the
/// strip keeps the result as one undo step.
#[test]
fn simplify_slider_auto_simplify_and_a_click_outside_keep_the_result() {
    let mut app = app_with_dense_path();
    let before = app.session.doc().unwrap().doc.clone();
    let undo_len = app.session.doc().unwrap().history.undo.len();
    crate::menus::invoke(&mut app, "object.path.simplify", json!({}));
    drag_strip(&mut app, true);
    assert_eq!(simplify_preview(&app), json!({"level": 100.0}));
    simplify_slider::auto(&mut app).unwrap();
    frame(&mut app, vec![]);
    // The path is 285 pt wide: 0.7125 pt at most, level 55.
    assert_eq!(app.session.doc().unwrap().interaction.as_ref().and_then(|it| it.preview.clone()).unwrap().1, json!({"auto": true}));
    assert_eq!(anchor_count(&app), 2);
    assert!(app.session.in_interaction(), "the result waits for a click outside");
    click(&mut app, Pos2::new(20.0, 20.0));
    assert!(!simplify_slider::showing(&app) && !app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo_len + 1);
    assert_ne!(app.session.doc().unwrap().doc, before);
}

/// With anchors 10 to 30 of a 41-anchor wave direct-selected, the Simplify slider previews and Enter keeps a refit of only
/// the segments between them, as one undo step: the anchors before and after the run stay exactly as they were. The
/// Simplify dialog (More Options) does the same.
#[test]
fn simplify_slider_and_dialog_simplify_only_the_direct_selected_part_of_a_path() {
    let mut app = window(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let anchors: Vec<_> = (0..=40).map(|i| json!({"x": 20 + i * 8, "y": 150.0 + 60.0 * (f64::from(i) / 6.0).sin()})).collect();
    let id = app.run("path.create", json!({"anchors": anchors})).unwrap()["id"].as_u64().unwrap();
    let path = |app: &VectorcraftApp| {
        app.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).and_then(|n| n.path_data()).expect("a path").subpaths[0].anchors.to_vec()
    };
    let original = path(&app);
    let run: Vec<_> = (10..=30).map(|i| json!([0, i])).collect();
    app.run("select.anchors", json!({"id": id, "anchors": run})).unwrap();
    let undo_len = app.session.doc().unwrap().history.undo.len();
    let kept = |app: &VectorcraftApp, how: &str| {
        let now = path(app);
        assert!(now.len() < 41, "{how}: the run loses anchors");
        assert_eq!(now[..10], original[..10], "{how}: the anchors before the run");
        assert_eq!(now[10].p, original[10].p, "{how}: the run's start anchor");
        assert_eq!(now[now.len() - 10..], original[31..], "{how}: the anchors after the run");
        assert_eq!(now[now.len() - 11].p, original[30].p, "{how}: the run's end anchor");
    };
    crate::menus::invoke(&mut app, "object.path.simplify", json!({}));
    app.object_dialogs.strip.as_mut().expect("the slider is open").level = 20.0;
    settle(&mut app);
    assert_eq!(simplify_preview(&app), json!({"level": 20.0}));
    kept(&app, "the slider's preview");
    frame(&mut app, vec![key(egui::Key::Enter)]);
    assert!(!simplify_slider::showing(&app) && !app.session.in_interaction());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo_len + 1);
    kept(&app, "the slider");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(path(&app), original);
    open_simplify_dialog(&mut app);
    set(&mut app, "tolerance", json!(4));
    settle(&mut app);
    assert!(confirm(&mut app));
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo_len + 1);
    kept(&app, "the dialog");
}
