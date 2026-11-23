#![allow(clippy::unwrap_used)]

use craft::behavior::{Axis, GesturePhase};
use craft::ui::canvas::GuideDrag;
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use super::{guide, scales, zero_begin, zero_cancel, zero_commit, zero_update};
use crate::VectorcraftApp;

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.ui.view.rulers = true;
    app.ui.view.smart_guides = false;
    app.ui.view.snap_to_point = false;
    app
}

fn drag(along: Axis, phase: GesturePhase, at: f64, cmd: bool) -> GuideDrag {
    GuideDrag::new_guide(along, phase, Some(at), craft::keymap::Mods { cmd, ..Default::default() })
}

/// One pass of the strip's guide.
fn send(app: &mut VectorcraftApp, ctx: &egui::Context, drag: GuideDrag) {
    ctx.run_ui(Default::default(), |ui| guide(app, ui, &drag)).textures_delta.clear();
}

/// One pass of the zero-point gesture.
fn zero(app: &mut VectorcraftApp, ctx: &egui::Context, step: impl Fn(&mut VectorcraftApp, &egui::Ui)) {
    ctx.run_ui(Default::default(), |ui| step(app, ui)).textures_delta.clear();
}

fn guides(app: &VectorcraftApp) -> Vec<(bool, f64)> {
    app.session.active().unwrap().doc.ruler_guides().iter().map(|g| (g.vertical, g.pos)).collect()
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// Video Rulers at aspect 0.9 count a 720-wide artboard 800 pixels across and the height a pixel a point; off, the
/// General unit from the ruler origin.
#[test]
fn video_rulers_count_the_artboard_in_pixels_of_its_aspect_ratio() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 720, "height": 480})).unwrap();
    app.session.execute("artboard.setProps", &json!({"index": 0, "videoRulerPixelAspectRatio": 0.9})).unwrap();
    app.session.execute("document.setRulerOrigin", &json!({"x": 10, "y": 20, "artboard": 0})).unwrap();
    app.ui.view.video_rulers = true;
    let r = app.session.active().unwrap().doc.artboards[0].rect;
    let (x, y, zero) = scales(&app);
    assert_eq!(zero, Point::new(r.x0, r.y0));
    assert!((x.units.distance(r.x1, zero.x) - 800.0).abs() < 1e-9);
    assert!((y.units.distance(r.y1, zero.y) - 480.0).abs() < 1e-9);
    assert!((x.units.distance(r.x0 + 9.0, zero.x) - 10.0).abs() < 1e-9, "a pixel is 0.9 pt wide");
    app.ui.view.video_rulers = false;
    let (x, _, zero) = scales(&app);
    assert_eq!(zero, Point::new(10.0, 20.0));
    assert!((x.units.distance(110.0, zero.x) - 100.0).abs() < 1e-9, "points from the ruler origin");
}

#[test]
fn new_ruler_guides_follow_alt_through_preview_release_and_undo() {
    for along in [Axis::X, Axis::Y] {
        let (mut app, ctx) = (app(), crate::testing::ui_context());
        let (before, undo) = (app.session.active().unwrap().doc.clone(), undo_len(&app));
        let vertical = matches!(along, Axis::Y);
        send(&mut app, &ctx, drag(along, GesturePhase::Begin, 80.0, false));
        send(&mut app, &ctx, drag(along, GesturePhase::Update, 120.0, false));
        assert_eq!(guides(&app), [(vertical, 120.0)], "the guide previews");
        assert_eq!(undo_len(&app), undo, "a preview is no undo step");
        send(&mut app, &ctx, drag(along, GesturePhase::Commit, 147.0, false));
        assert_eq!(guides(&app), [(vertical, 147.0)]);
        let st = app.session.active().unwrap();
        assert_eq!((st.history.undo.len(), st.history.undo.last().unwrap().label.as_str()), (undo + 1, "New Guide"));
        app.session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(&*app.session.active().unwrap().doc, &*before);
    }
}

#[test]
fn new_ruler_guide_alt_cancel_keeps_document_and_redo() {
    let (mut app, ctx) = (app(), crate::testing::ui_context());
    app.session.execute("shape.rectangle", &json!({"x": 40, "y": 50, "width": 20, "height": 30})).unwrap();
    app.session.execute("edit.undo", &json!({})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    let history = |app: &VectorcraftApp| (undo_len(app), app.session.active().unwrap().history.redo.len());
    let before_history = history(&app);
    assert_eq!(before_history.1, 1);
    send(&mut app, &ctx, drag(Axis::X, GesturePhase::Begin, 80.0, false));
    send(&mut app, &ctx, drag(Axis::X, GesturePhase::Update, 120.0, false));
    assert_eq!(guides(&app).len(), 1);
    send(&mut app, &ctx, GuideDrag::new_guide(Axis::X, GesturePhase::Cancel, None, Default::default()));
    assert_eq!(&*app.session.active().unwrap().doc, &*before);
    assert_eq!(history(&app), before_history);
}

/// Cmd on the drag's first event makes an artboard guide with the preference on, a canvas guide with it off; a guide
/// beyond the artboard is not made with it on.
#[test]
fn cmd_ruler_drags_make_artboard_guides_with_the_preference_on() {
    for on in [false, true] {
        let (mut app, ctx) = (app(), crate::testing::ui_context());
        app.session.execute("prefs.set", &json!({"key": "cmdRulerDragArtboardGuides", "value": on})).unwrap();
        let board = app.session.active().unwrap().doc.artboards[0].id;
        let place = |app: &mut VectorcraftApp, at: f64| {
            send(app, &ctx, drag(Axis::Y, GesturePhase::Begin, at, true));
            send(app, &ctx, drag(Axis::Y, GesturePhase::Commit, at, false));
        };
        place(&mut app, 80.0);
        let made = app.session.active().unwrap().doc.ruler_guides();
        assert_eq!((made.len(), made[0].artboard), (1, on.then_some(board)), "on {on}");
        place(&mut app, 500.0);
        assert_eq!(guides(&app).len(), if on { 1 } else { 2 }, "on {on}");
    }
}

#[test]
fn guides_from_the_rulers_snap_and_artboard_guides_span_their_artboard() {
    let (mut app, ctx) = (app(), crate::testing::ui_context());
    app.ui.view.smart_guides = true;
    app.ui.view.snap_to_point = true;
    app.session.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 400, "height": 300})).unwrap();
    app.session.execute("select.none", &json!({})).unwrap();
    for (along, near) in [(Axis::Y, 203.0), (Axis::X, 297.0)] {
        send(&mut app, &ctx, drag(along, GesturePhase::Begin, near, false));
        send(&mut app, &ctx, drag(along, GesturePhase::Commit, near, false));
    }
    assert_eq!(guides(&app), [(true, 200.0), (false, 300.0)], "onto the side's midpoint and the artboard's edge");
    app.select_tool("artboard");
    let board = app.session.active().unwrap().doc.artboards[0].id;
    send(&mut app, &ctx, drag(Axis::X, GesturePhase::Begin, 50.0, false));
    send(&mut app, &ctx, drag(Axis::X, GesturePhase::Commit, 50.0, false));
    assert_eq!(app.session.active().unwrap().doc.ruler_guides().last().unwrap().artboard, Some(board));
}

/// A zero-point drag edits nothing until the release, which is one history step.
#[test]
fn ruler_guide_shift_origin_preview_release_and_history() {
    let (mut app, ctx) = (app(), crate::testing::ui_context());
    let (before, undo) = (app.session.active().unwrap().doc.clone(), undo_len(&app));
    let to = Point::new(125.25, 90.5);
    zero(&mut app, &ctx, zero_begin);
    zero(&mut app, &ctx, |app, ui| {
        zero_update(app, ui, to);
    });
    assert_eq!(&*app.session.active().unwrap().doc, &*before, "the preview does not edit");
    zero(&mut app, &ctx, |app, ui| zero_commit(app, ui, to));
    assert!(app.ruler_origin().distance(to) < 1e-4);
    assert_eq!(undo_len(&app), undo + 1);
}

#[test]
fn ruler_guide_shift_origin_cancel_preserves_redo() {
    let (mut app, ctx) = (app(), crate::testing::ui_context());
    app.session.execute("shape.rectangle", &json!({"x": 40, "y": 50, "width": 20, "height": 30})).unwrap();
    app.session.execute("edit.undo", &json!({})).unwrap();
    let before = app.session.active().unwrap().doc.clone();
    zero(&mut app, &ctx, zero_begin);
    zero(&mut app, &ctx, |app, ui| {
        zero_update(app, ui, Point::new(50.0, 60.0));
    });
    zero(&mut app, &ctx, zero_cancel);
    assert_eq!(&*app.session.active().unwrap().doc, &*before);
    assert_eq!(app.session.active().unwrap().history.redo.len(), 1);
}

/// A guide being dragged shows its position readout.
#[test]
fn ruler_guide_origin_tooltip_matches_selected_guide_fields() {
    let (mut app, ctx) = (app(), crate::testing::ui_context());
    send(&mut app, &ctx, drag(Axis::Y, GesturePhase::Begin, 80.0, false));
    send(&mut app, &ctx, drag(Axis::Y, GesturePhase::Update, 120.0, false));
    let overlays = app.session.overlays(app.view_info());
    assert!(overlays.iter().any(|o| format!("{o:?}").starts_with("Measure")), "{overlays:?}");
}

#[test]
fn trim_view_makes_no_guide_from_the_rulers() {
    let (mut app, ctx) = (app(), crate::testing::ui_context());
    app.ui.view.trim_view = true;
    let undo = undo_len(&app);
    send(&mut app, &ctx, drag(Axis::Y, GesturePhase::Begin, 80.0, false));
    send(&mut app, &ctx, drag(Axis::Y, GesturePhase::Update, 100.0, false));
    send(&mut app, &ctx, drag(Axis::Y, GesturePhase::Commit, 120.0, false));
    // A strip's double press.
    send(&mut app, &ctx, drag(Axis::Y, GesturePhase::Commit, 150.0, false));
    assert!(guides(&app).is_empty() && !app.session.ruler_guide_dragging());
    assert_eq!(undo_len(&app), undo);
}

#[test]
fn a_ruler_drag_trim_view_cancelled_stays_cancelled_until_released() {
    let (mut app, ctx) = (app(), crate::testing::ui_context());
    app.ui.view.trim_view = true;
    send(&mut app, &ctx, drag(Axis::Y, GesturePhase::Begin, 80.0, false));
    app.ui.view.trim_view = false;
    send(&mut app, &ctx, drag(Axis::Y, GesturePhase::Update, 100.0, false));
    send(&mut app, &ctx, drag(Axis::Y, GesturePhase::Commit, 120.0, false));
    assert!(guides(&app).is_empty());
    // The next drag is free again.
    send(&mut app, &ctx, drag(Axis::Y, GesturePhase::Begin, 80.0, false));
    send(&mut app, &ctx, drag(Axis::Y, GesturePhase::Commit, 120.0, false));
    assert_eq!(guides(&app), [(true, 120.0)]);
}

#[test]
fn trim_view_toggled_during_a_ruler_origin_drag_cancels_it() {
    let (mut app, ctx) = (app(), crate::testing::ui_context());
    let (before, undo) = (app.session.active().unwrap().doc.clone(), undo_len(&app));
    zero(&mut app, &ctx, zero_begin);
    app.ui.view.trim_view = true;
    zero(&mut app, &ctx, |app, ui| {
        zero_update(app, ui, Point::new(50.0, 60.0));
    });
    zero(&mut app, &ctx, |app, ui| zero_commit(app, ui, Point::new(50.0, 60.0)));
    assert_eq!(&*app.session.active().unwrap().doc, &*before);
    assert_eq!(undo_len(&app), undo);
}

/// A strip's double press reports one `Commit` with no `Begin`: one guide at the pointer, even with guides hidden.
#[test]
fn double_clicking_a_ruler_adds_a_guide_at_that_position() {
    let (mut app, ctx) = (app(), crate::testing::ui_context());
    app.ui.view.guides = false;
    let undo = undo_len(&app);
    send(&mut app, &ctx, drag(Axis::Y, GesturePhase::Commit, 150.0, false));
    assert_eq!(guides(&app), [(true, 150.0)]);
    assert!(app.ui.view.guides, "the new guide shows");
    assert_eq!(undo_len(&app), undo + 1);
}
