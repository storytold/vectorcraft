//! Real tab-strip and canvas gestures for dragging editable artwork between documents.

use egui::{Event, PointerButton, Pos2};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

use crate::{VectorcraftApp, canvas, chrome};

/// Run a real tab-strip/canvas frame and return title bounds for pointer-driven gestures.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, events: Vec<Event>) -> Vec<egui::Rect> {
    app.sync_views();
    let focused = events
        .iter()
        .rev()
        .find_map(|e| match e {
            Event::WindowFocused(focused) => Some(*focused),
            _ => None,
        })
        .unwrap_or(true);
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 700.0))),
            time: Some(time),
            focused,
            events,
            ..Default::default()
        },
        |ui| {
            canvas::release_over_panel(app, ui.ctx());
            egui::CentralPanel::default().show(ui, |ui| {
                chrome::doc_tabs(app, ui);
                canvas::show(app, ui);
            });
        },
    );
    out.textures_delta.clear();
    let mut titles: Vec<_> = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text().contains(" @ ") => Some(egui::Rect::from_min_size(t.pos, t.galley.size())),
            _ => None,
        })
        .collect();
    titles.sort_by(|a, b| a.left().total_cmp(&b.left()));
    titles
}

/// Build a primary-pointer event without keyboard modifiers.
fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// Verify dwell activation, editable placement, source and clipboard preservation, and target Undo/Redo.
#[test]
fn dragging_artwork_over_a_tab_switches_then_copies_on_drop() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "name": "Logo"})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    let source = app.session.active().unwrap().doc.clone();
    let source_undo = app.session.active().unwrap().history.undo.len();
    app.run("file.new", json!({"width": 400, "height": 300, "name": "Flyer"})).unwrap();
    app.run("shape.rectangle", json!({"x": 300, "y": 230, "width": 50, "height": 50})).unwrap();
    app.session.execute("edit.copy", &json!({})).unwrap();
    let clipboard = app.session.clipboard.clone();
    let destination = app.session.active().unwrap().doc.clone();
    let dest_undo = app.session.active().unwrap().history.undo.len();
    app.session.set_active(0);
    app.select_tool("selection");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let tabs = frame(&mut app, &ctx, 0.0, vec![]);
    assert_eq!(tabs.len(), 2);
    let grab = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(100.0, 100.0));
    frame(&mut app, &ctx, 0.05, vec![Event::PointerMoved(grab), button(grab, true)]);
    frame(&mut app, &ctx, 0.1, vec![Event::PointerMoved(grab + egui::vec2(20.0, 10.0))]);
    assert!(app.session.in_interaction());
    assert!(app.session.tool_busy(), "the Selection tool owns the source drag");
    let tab = tabs[1].center();
    frame(&mut app, &ctx, 0.2, vec![Event::PointerMoved(tab)]);
    frame(&mut app, &ctx, 0.25, vec![]);
    assert_eq!(app.session.active_index(), Some(0), "crossing a tab briefly must not switch");
    frame(&mut app, &ctx, 0.8, vec![]);
    assert_eq!(app.session.active_index(), Some(1), "holding the artwork over the target tab activates it");
    assert!(!app.session.tool_busy(), "document activation resets the source Selection tool before drop");
    assert_eq!(app.session.documents()[0].doc, source, "the source preview must be rolled back");
    let drop = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(170.0, 120.0));
    frame(&mut app, &ctx, 0.9, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 1.0, vec![button(drop, false)]);
    let st = app.session.active().unwrap();
    assert_eq!(st.selection.objects.len(), 1);
    let placed = st.doc.node(st.selection.objects[0]).unwrap().geometric_bounds().unwrap();
    assert!((placed.center().x - 170.0).abs() < 0.01 && (placed.center().y - 120.0).abs() < 0.01, "{placed:?}");
    assert_eq!(st.history.undo.len(), dest_undo + 1);
    assert_eq!(app.session.clipboard, clipboard, "a drag must preserve the user's clipboard");
    assert!(!app.session.tool_busy(), "a completed tab transfer leaves the Selection tool idle");
    assert_eq!(app.session.documents()[0].doc, source);
    assert_eq!(app.session.documents()[0].history.undo.len(), source_undo);
    assert!(app.session.documents()[0].doc.node(vectorcraft_doc::NodeId(id)).is_some());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc, destination);
    app.run("edit.redo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().history.undo.len(), dest_undo + 1);
}

/// Begin a Selection move in the first of three documents with no keyboard modifiers.
fn drag_fixture() -> (VectorcraftApp, egui::Context, Pos2, Vec<egui::Rect>) {
    drag_fixture_modifiers(Default::default())
}

/// Create three document tabs and start dragging source artwork with the supplied modifiers.
fn drag_fixture_modifiers(modifiers: egui::Modifiers) -> (VectorcraftApp, egui::Context, Pos2, Vec<egui::Rect>) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":400,"height":300,"name":"Source"})).unwrap();
    app.run("shape.rectangle", json!({"x":50,"y":50,"width":100,"height":100})).unwrap();
    app.run("file.new", json!({"width":400,"height":300,"name":"Target"})).unwrap();
    app.run("file.new", json!({"width":400,"height":300,"name":"Third"})).unwrap();
    app.session.set_active(0);
    app.select_tool("selection");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, 0.0, vec![]);
    // Fit changes the zoom in the title, and therefore the tab widths.
    let tabs = frame(&mut app, &ctx, 0.01, vec![]);
    let grab = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(75.0, 80.0));
    frame(
        &mut app,
        &ctx,
        0.05,
        vec![
            Event::ModifiersChanged(modifiers),
            Event::PointerMoved(grab),
            Event::PointerButton { pos: grab, button: PointerButton::Primary, pressed: true, modifiers },
        ],
    );
    frame(&mut app, &ctx, 0.1, vec![Event::PointerMoved(grab + egui::vec2(20.0, 10.0))]);
    (app, ctx, grab, tabs)
}

/// Verify that leaving a tab restarts its dwell timer and hovering its close button never activates it.
#[test]
fn tab_hover_delay_restarts_after_leaving_and_close_button_does_not_activate() {
    let (mut app, ctx, grab, tabs) = drag_fixture();
    let tab = tabs[1].center();
    frame(&mut app, &ctx, 0.2, vec![Event::PointerMoved(tab)]);
    frame(&mut app, &ctx, 0.25, vec![]);
    frame(&mut app, &ctx, 0.5, vec![Event::PointerMoved(grab)]);
    frame(&mut app, &ctx, 0.6, vec![Event::PointerMoved(tab)]);
    frame(&mut app, &ctx, 0.7, vec![]);
    frame(&mut app, &ctx, 0.95, vec![]);
    assert_eq!(app.session.active_index(), Some(0));
    // On Linux the close icon is 16 pt from the right edge, after the title.
    let close = egui::pos2(if cfg!(target_os = "macos") { tabs[1].left() - 16.0 } else { tabs[1].right() + 20.0 }, tab.y);
    frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(close)]);
    frame(&mut app, &ctx, 2.0, vec![]);
    assert_eq!(app.session.active_index(), Some(0));
    frame(&mut app, &ctx, 2.1, vec![Event::PointerMoved(tab)]);
    frame(&mut app, &ctx, 2.6, vec![]);
    assert_eq!(app.session.active_index(), Some(1));
}

/// Verify Escape and release outside the canvas preserve both documents and their undo histories.
#[test]
fn escape_or_release_over_tab_cancels_the_transfer_without_edits() {
    for escape in [false, true] {
        let (mut app, ctx, _, tabs) = drag_fixture();
        let original = app.session.documents()[0].interaction.as_ref().unwrap().doc.clone();
        let target = app.session.documents()[1].doc.clone();
        let undo = app.session.documents()[1].history.undo.len();
        let tab = tabs[1].center();
        frame(&mut app, &ctx, 0.2, vec![Event::PointerMoved(tab)]);
        frame(&mut app, &ctx, 0.25, vec![]);
        frame(&mut app, &ctx, 0.8, vec![]);
        assert_eq!(app.session.active_index(), Some(1));
        if escape {
            frame(
                &mut app,
                &ctx,
                0.9,
                vec![Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
            );
            let drop = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(170.0, 120.0));
            frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(drop), button(drop, false)]);
        } else {
            frame(&mut app, &ctx, 1.0, vec![button(tab, false)]);
        }
        assert_eq!(app.session.documents()[0].doc, original);
        assert_eq!(app.session.active().unwrap().doc, target);
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
        assert!(!app.session.in_interaction());
        assert!(!app.session.tool_busy(), "Escape and outside release leave the Selection tool idle");
    }
}

/// Verify destination view transforms preserve the original artwork grab point during placement.
#[test]
fn tab_drop_preserves_hotspot_across_zoom_and_rotation() {
    let (mut app, ctx, _, tabs) = drag_fixture();
    app.views[1].fitted = true;
    app.views[1].zoom = 0.9;
    app.views[1].rotation = 25.0;
    app.views[1].center = Point::new(200.0, 150.0);
    let tab = tabs[1].center();
    frame(&mut app, &ctx, 0.2, vec![Event::PointerMoved(tab)]);
    frame(&mut app, &ctx, 0.25, vec![]);
    frame(&mut app, &ctx, 0.8, vec![]);
    let drop = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(175.0, 180.0));
    frame(&mut app, &ctx, 0.9, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 1.0, vec![button(drop, false)]);
    let st = app.session.active().unwrap();
    let bounds = st.doc.node(st.selection.objects[0]).unwrap().geometric_bounds().unwrap();
    assert!((bounds.x0 - 150.0).abs() < 0.001 && (bounds.y0 - 150.0).abs() < 0.001, "{bounds:?}");
}

/// Verify returning to the source tab moves its original object instead of inserting a duplicate.
#[test]
fn hovering_back_to_source_resumes_move_instead_of_copying() {
    let (mut app, ctx, _, tabs) = drag_fixture();
    let id = app.session.active().unwrap().selection.objects[0];
    let undo = app.session.active().unwrap().history.undo.len();
    frame(&mut app, &ctx, 0.2, vec![Event::PointerMoved(tabs[1].center())]);
    frame(&mut app, &ctx, 0.25, vec![]);
    frame(&mut app, &ctx, 0.8, vec![]);
    frame(&mut app, &ctx, 0.9, vec![Event::PointerMoved(tabs[0].center())]);
    frame(&mut app, &ctx, 1.4, vec![]);
    assert_eq!(app.session.active_index(), Some(0));
    let drop = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(175.0, 180.0));
    frame(&mut app, &ctx, 1.5, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 1.6, vec![button(drop, false)]);
    let st = app.session.active().unwrap();
    assert_eq!(st.selection.objects, vec![id]);
    assert_eq!(st.history.undo.len(), undo + 1);
    let bounds = st.doc.node(id).unwrap().geometric_bounds().unwrap();
    assert!((bounds.x0 - 150.0).abs() < 0.001 && (bounds.y0 - 150.0).abs() < 0.001, "{bounds:?}");
}

/// Verify Alt-drag transfers restored originals across tabs without retaining preview duplicates.
#[test]
fn alt_drag_can_continue_across_multiple_tabs_without_copying_the_source_preview() {
    let (mut app, ctx, _, tabs) = drag_fixture_modifiers(egui::Modifiers::ALT);
    assert_eq!(app.session.active().unwrap().interaction.as_ref().unwrap().label, "Copy");
    let source = app.session.active().unwrap().interaction.as_ref().unwrap().doc.clone();
    frame(&mut app, &ctx, 0.2, vec![Event::PointerMoved(tabs[1].center())]);
    frame(&mut app, &ctx, 0.25, vec![]);
    frame(&mut app, &ctx, 0.8, vec![]);
    assert_eq!(app.session.active_index(), Some(1));
    frame(&mut app, &ctx, 0.9, vec![Event::PointerMoved(tabs[2].center())]);
    frame(&mut app, &ctx, 1.4, vec![]);
    assert_eq!(app.session.active_index(), Some(2));
    let drop = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(175.0, 180.0));
    frame(&mut app, &ctx, 1.5, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 1.6, vec![button(drop, false)]);
    assert_eq!(app.session.active().unwrap().selection.objects.len(), 1);
    assert_eq!(app.session.documents()[0].doc, source);
    assert!(app.session.documents()[1].history.undo.is_empty());
}

/// Verify closing the originating document cancels a transfer before its eventual mouse release.
#[test]
fn closing_the_source_during_a_transfer_cancels_it() {
    let (mut app, ctx, _, tabs) = drag_fixture();
    frame(&mut app, &ctx, 0.2, vec![Event::PointerMoved(tabs[1].center())]);
    frame(&mut app, &ctx, 0.25, vec![]);
    frame(&mut app, &ctx, 0.8, vec![]);
    let target = app.session.active().unwrap().doc.clone();
    app.session.execute("file.close", &json!({"index":0})).unwrap();
    let drop = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(175.0, 180.0));
    frame(&mut app, &ctx, 0.9, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 1.0, vec![button(drop, false)]);
    assert_eq!(app.session.active().unwrap().doc, target);
    assert!(app.session.active().unwrap().history.undo.is_empty());
}

/// Verify document changes, reused IDs, focus loss and touch cancellation cannot release stale artwork.
#[test]
fn source_edits_target_closure_switches_and_focus_loss_cancel_stale_transfers() {
    for change in ["source", "close target", "switch target", "focus", "touch cancel", "two fingers"] {
        let (mut app, ctx, _, tabs) = drag_fixture();
        let source_id = app.session.active().unwrap().selection.objects[0];
        frame(&mut app, &ctx, 0.2, vec![Event::PointerMoved(tabs[1].center())]);
        frame(&mut app, &ctx, 0.25, vec![]);
        frame(&mut app, &ctx, 0.8, vec![]);
        match change {
            "source" => {
                app.session.set_active(0);
                app.session.execute("edit.undo", &json!({})).unwrap();
                let replacement = app.session.execute("shape.ellipse", &json!({"x":50,"y":50,"width":100,"height":100})).unwrap();
                assert_eq!(replacement["id"].as_u64(), Some(source_id.0), "Undo can reuse a node id for unrelated artwork");
                app.session.set_active(1);
            }
            "close target" => {
                app.session.execute("file.close", &json!({"index":1})).unwrap();
            }
            "switch target" => {
                app.session.set_active(2);
            }
            "touch cancel" | "two fingers" => {
                let touch = |id, phase| Event::Touch {
                    device_id: egui::TouchDeviceId(1),
                    id: egui::TouchId(id),
                    phase,
                    pos: Pos2::new(400.0 + id as f32 * 10.0, 350.0),
                    force: None,
                };
                let events = if change == "touch cancel" {
                    vec![touch(1, egui::TouchPhase::Cancel)]
                } else {
                    vec![touch(1, egui::TouchPhase::Start), touch(2, egui::TouchPhase::Start)]
                };
                frame(&mut app, &ctx, 0.85, events);
            }
            _ => {
                frame(&mut app, &ctx, 0.85, vec![Event::WindowFocused(false)]);
            }
        }
        let before = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let drop = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(175.0, 180.0));
        frame(&mut app, &ctx, 0.9, vec![Event::PointerMoved(drop)]);
        frame(&mut app, &ctx, 1.0, vec![button(drop, false)]);
        assert_eq!(app.session.active().unwrap().doc, before, "{change}");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "{change}");
    }
}

/// Verify tab-strip relayout under a stationary pointer cannot trigger another document switch.
#[test]
fn scrolling_tabs_under_a_stationary_drag_does_not_cascade_to_other_documents() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    for i in 0..8 {
        app.run("file.new", json!({"width":400,"height":300,"name":format!("Document {i}")})).unwrap();
    }
    app.run("shape.rectangle", json!({"x":50,"y":50,"width":100,"height":100})).unwrap();
    app.select_tool("selection");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    frame(&mut app, &ctx, 0.0, vec![]);
    let tabs = frame(&mut app, &ctx, 0.01, vec![]);
    assert!(tabs.len() >= 2 && tabs.len() < 8, "must exercise the scrolling tab strip");
    let target = 8 - tabs.len();
    let grab = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(75.0, 80.0));
    frame(&mut app, &ctx, 0.05, vec![Event::PointerMoved(grab), button(grab, true)]);
    frame(&mut app, &ctx, 0.1, vec![Event::PointerMoved(grab + egui::vec2(20.0, 10.0))]);
    frame(&mut app, &ctx, 0.2, vec![Event::PointerMoved(tabs[0].center())]);
    frame(&mut app, &ctx, 0.25, vec![]);
    frame(&mut app, &ctx, 0.8, vec![]);
    assert_eq!(app.session.active_index(), Some(target));
    for time in [0.9, 1.5, 2.1] {
        frame(&mut app, &ctx, time, vec![]);
        assert_eq!(app.session.active_index(), Some(target), "layout movement must not act as pointer movement");
    }
    let drop = canvas::Xf::new(app.canvas_rect.unwrap(), app.view().unwrap()).to_screen(Point::new(175.0, 180.0));
    frame(&mut app, &ctx, 2.2, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 2.3, vec![button(drop, false)]);
    assert_eq!(app.session.active().unwrap().selection.objects.len(), 1);
}
