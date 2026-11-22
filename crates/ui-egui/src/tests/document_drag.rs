//! Real tab-strip and canvas gestures for dragging editable artwork between documents.

use egui::{Event, PointerButton, Pos2};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;
use vectorcraft_tools::{PointerEvent, PointerKind};

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
            egui::CentralPanel::default().show(ui, |ui| {
                chrome::doc_tabs(app, ui);
                canvas::show(app, ui);
            });
        },
    );
    out.textures_delta.clear();
    // A tab's title starts with its document's name (an inactive tab shows the name alone).
    let names: Vec<String> = app.session.documents().iter().map(|d| d.title()).collect();
    let mut titles: Vec<_> = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) if names.iter().any(|n| t.galley.text().starts_with(n.as_str())) => {
                Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
            }
            _ => None,
        })
        .collect();
    titles.sort_by(|a, b| a.left().total_cmp(&b.left()));
    titles
}

/// The screen point of document point `x`, `y` in the active view.
fn at(app: &VectorcraftApp, x: f64, y: f64) -> Pos2 {
    // A document not shown yet takes the canvas area the shown ones were laid out in.
    let view = app.canvas_view().or_else(|| {
        let rect = app.views.iter().map(|v| v.rect).find(|r| r.width() > 0.0 && r.height() > 0.0)?;
        Some(app.view()?.canvas(app.ui_scaling(), rect))
    });
    view.unwrap().to_screen(Point::new(x, y))
}

/// Build a primary-pointer event without keyboard modifiers.
fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// Hold the pointer at `pos` from `time` past the tab rest delay. The move reaches `pos` a frame earlier: art carried
/// off the canvas starts its drag after the tab strip drew, so the strip's wait begins the frame after.
fn rest(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64, pos: Pos2) {
    frame(app, ctx, time - 0.04, vec![Event::PointerMoved(pos)]);
    frame(app, ctx, time, vec![]);
    frame(app, ctx, time + 0.6, vec![]);
}

/// Verify the rest over a tab, editable placement, source and clipboard preservation, and target Undo/Redo.
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
    let ctx = crate::testing::ui_context();
    let tabs = frame(&mut app, &ctx, 0.0, vec![]);
    assert_eq!(tabs.len(), 2);
    let grab = at(&app, 100.0, 100.0);
    frame(&mut app, &ctx, 0.05, vec![Event::PointerMoved(grab), button(grab, true)]);
    frame(&mut app, &ctx, 0.1, vec![Event::PointerMoved(grab + egui::vec2(20.0, 10.0))]);
    assert!(app.session.in_interaction());
    assert!(app.session.tool_busy(), "the Selection tool owns the source drag");
    rest(&mut app, &ctx, 0.2, tabs[1].center());
    assert_eq!(app.session.active_index(), Some(1), "resting the artwork over the target tab activates it");
    assert!(!app.session.tool_busy(), "document activation resets the source Selection tool before drop");
    assert_eq!(app.session.documents()[0].doc, source, "the source preview must be rolled back");
    let drop = at(&app, 170.0, 120.0);
    frame(&mut app, &ctx, 1.0, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 1.1, vec![button(drop, false)]);
    frame(&mut app, &ctx, 1.2, vec![]);
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
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, 0.0, vec![]);
    // Fit changes the zoom in the title, and therefore the tab widths.
    let tabs = frame(&mut app, &ctx, 0.01, vec![]);
    let grab = at(&app, 75.0, 80.0);
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

/// A Cmd-press held while a Shape Builder merge is computed reaches the lent Selection tool once
/// the merge is applied, and its drag carries the artwork over another tab.
#[test]
fn a_cmd_drag_held_during_a_merge_activates_another_tab() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":800,"height":200,"name":"Source"})).unwrap();
    for k in 0..40 {
        app.run("shape.rectangle", json!({"x": k * 10, "y": 0, "width": 20, "height": 20})).unwrap();
    }
    app.run("select.all", json!({})).unwrap();
    app.run("file.new", json!({"width":800,"height":200,"name":"Target"})).unwrap();
    app.session.set_active(0);
    app.select_tool("shapeBuilder");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = crate::testing::ui_context();
    app.ui_ctx = Some(ctx.clone());
    frame(&mut app, &ctx, 0.0, vec![]);
    let tabs = frame(&mut app, &ctx, 0.01, vec![]);
    let view = app.view_info();
    app.session.pointer(&PointerEvent::new(PointerKind::Move, 15.0, 10.0), view).unwrap();
    app.session.tool_poll(view, true).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Down, 15.0, 10.0), view).unwrap();
    app.session.pointer(&PointerEvent::new(PointerKind::Up, 15.0, 10.0), view).unwrap();
    assert!(app.session.tool_editing(), "the click starts a merge on another thread");
    let grab = at(&app, 355.0, 10.0);
    let modifiers = egui::Modifiers::COMMAND;
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
    assert_eq!(app.session.tool_id(), "shapeBuilder", "the press waits for the merge");
    let r = app.session.tool_poll(view, true);
    canvas::apply_requests(&mut app, r);
    frame(&mut app, &ctx, 0.08, vec![]);
    assert_eq!(app.session.tool_id(), "selection", "the held press borrowed the Selection tool");
    frame(&mut app, &ctx, 0.1, vec![Event::PointerMoved(grab + egui::vec2(20.0, 10.0))]);
    rest(&mut app, &ctx, 0.2, tabs[1].center());
    assert_eq!(app.session.active_index(), Some(1), "resting the artwork over the tab activates it");
}

/// Verify destination view transforms preserve the original artwork grab point during placement.
#[test]
fn tab_drop_preserves_hotspot_across_zoom_and_rotation() {
    let (mut app, ctx, _, tabs) = drag_fixture();
    app.views[1].fitted = true;
    app.views[1].zoom = 0.9;
    app.views[1].rotation = 25.0;
    app.views[1].center = Point::new(200.0, 150.0);
    rest(&mut app, &ctx, 0.2, tabs[1].center());
    let drop = at(&app, 175.0, 180.0);
    frame(&mut app, &ctx, 0.9, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 1.0, vec![button(drop, false)]);
    frame(&mut app, &ctx, 1.1, vec![]);
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
    rest(&mut app, &ctx, 0.2, tabs[1].center());
    assert_eq!(app.session.active_index(), Some(1));
    // The current document's tab shows its whole title, so the tabs move as Target becomes current.
    let tabs = frame(&mut app, &ctx, 0.85, vec![]);
    rest(&mut app, &ctx, 0.9, tabs[0].center());
    assert_eq!(app.session.active_index(), Some(0));
    let drop = at(&app, 175.0, 180.0);
    frame(&mut app, &ctx, 1.9, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 2.0, vec![button(drop, false)]);
    frame(&mut app, &ctx, 2.1, vec![]);
    let st = app.session.active().unwrap();
    assert_eq!(st.selection.objects, vec![id]);
    assert_eq!(st.history.undo.len(), undo + 1);
    let bounds = st.doc.node(id).unwrap().geometric_bounds().unwrap();
    assert!((bounds.x0 - 150.0).abs() < 0.001 && (bounds.y0 - 150.0).abs() < 0.001, "{bounds:?}");
}

/// Verify that a drop back in the source tab lands where the button went up and copies when Alt was
/// held then, though the pointer moves and Alt is let go later in the frame of the release.
#[test]
fn a_drop_back_in_the_source_uses_the_release_position_and_alt() {
    let (mut app, ctx, _, tabs) = drag_fixture();
    rest(&mut app, &ctx, 0.2, tabs[1].center());
    let tabs = frame(&mut app, &ctx, 0.85, vec![]);
    rest(&mut app, &ctx, 0.9, tabs[0].center());
    assert_eq!(app.session.active_index(), Some(0));
    let drop = at(&app, 175.0, 180.0);
    let later = at(&app, 300.0, 250.0);
    let alt = egui::Modifiers::ALT;
    frame(&mut app, &ctx, 1.5, vec![Event::ModifiersChanged(alt), Event::PointerMoved(drop)]);
    let up = Event::PointerButton { pos: drop, button: PointerButton::Primary, pressed: false, modifiers: alt };
    frame(&mut app, &ctx, 1.6, vec![up, Event::PointerMoved(later), Event::ModifiersChanged(egui::Modifiers::NONE)]);
    frame(&mut app, &ctx, 1.7, vec![]);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers[0].children().map_or(0, Vec::len), 2, "Alt at the release copies the original");
    assert_eq!(st.selection.objects.len(), 1);
    let bounds = st.doc.node(st.selection.objects[0]).unwrap().geometric_bounds().unwrap();
    assert!((bounds.x0 - 150.0).abs() < 0.001 && (bounds.y0 - 150.0).abs() < 0.001, "{bounds:?}");
}

/// Verify that a release inside the other document's canvas drops the artwork there, though the
/// pointer moves onto the tab strip later in the frame of the release.
#[test]
fn a_release_in_the_canvas_drops_though_the_pointer_then_leaves_it() {
    let (mut app, ctx, _, tabs) = drag_fixture();
    rest(&mut app, &ctx, 0.2, tabs[1].center());
    assert_eq!(app.session.active_index(), Some(1));
    let before = app.session.active().unwrap().doc.layers[0].children().map_or(0, Vec::len);
    let drop = at(&app, 175.0, 180.0);
    assert!(tabs[0].center().y < drop.y, "the tab strip is above the canvas");
    frame(&mut app, &ctx, 0.9, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 1.0, vec![button(drop, false), Event::PointerMoved(tabs[0].center())]);
    frame(&mut app, &ctx, 1.1, vec![]);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers[0].children().map_or(0, Vec::len), before + 1);
    let bounds = st.doc.node(st.selection.objects[0]).unwrap().geometric_bounds().unwrap();
    assert!((bounds.x0 - 150.0).abs() < 0.001 && (bounds.y0 - 150.0).abs() < 0.001, "{bounds:?}");
}

/// Back in the source tab, the move resumes even when coming to the front measured the source's
/// type again in its own composite fonts, which replaces the document without editing it.
#[test]
fn hovering_back_to_a_source_whose_type_is_measured_again_resumes_move() {
    let font = |roman: u32| {
        json!({
            "name": "CF Tab Drag",
            "parts": {"kanji": {"font": "Source Sans 3"}, "roman": {"font": "JetBrains Mono", "size": roman}},
        })
    };
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":400,"height":300,"name":"Source"})).unwrap();
    app.run("type.compositeFonts.set", json!({"fonts": [font(100)]})).unwrap();
    app.run("text.create", json!({"x": 200, "y": 250, "text": "Ab12", "size": 20, "font": "CF Tab Drag"})).unwrap();
    let id = vectorcraft_doc::NodeId(app.run("shape.rectangle", json!({"x":50,"y":50,"width":100,"height":100})).unwrap()["id"].as_u64().unwrap());
    app.run("file.new", json!({"width":400,"height":300,"name":"Target"})).unwrap();
    app.run("type.compositeFonts.set", json!({"fonts": [font(50)]})).unwrap();
    app.session.set_active(0);
    let undo = app.session.active().unwrap().history.undo.len();
    app.select_tool("selection");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, 0.0, vec![]);
    let tabs = frame(&mut app, &ctx, 0.01, vec![]);
    let grab = at(&app, 75.0, 80.0);
    frame(&mut app, &ctx, 0.05, vec![Event::PointerMoved(grab), button(grab, true)]);
    frame(&mut app, &ctx, 0.1, vec![Event::PointerMoved(grab + egui::vec2(20.0, 10.0))]);
    rest(&mut app, &ctx, 0.2, tabs[1].center());
    assert_eq!(app.session.active_index(), Some(1));
    let tabs = frame(&mut app, &ctx, 0.85, vec![]);
    let left = app.session.documents()[0].doc.clone();
    rest(&mut app, &ctx, 0.9, tabs[0].center());
    assert_eq!(app.session.active_index(), Some(0));
    assert!(!std::sync::Arc::ptr_eq(&app.session.documents()[0].doc, &left), "the source's type is measured again");
    let drop = at(&app, 175.0, 180.0);
    frame(&mut app, &ctx, 1.5, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 1.6, vec![button(drop, false)]);
    frame(&mut app, &ctx, 1.7, vec![]);
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
    rest(&mut app, &ctx, 0.2, tabs[1].center());
    assert_eq!(app.session.active_index(), Some(1));
    rest(&mut app, &ctx, 0.9, tabs[2].center());
    assert_eq!(app.session.active_index(), Some(2));
    let drop = at(&app, 175.0, 180.0);
    frame(&mut app, &ctx, 1.5, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 1.6, vec![button(drop, false)]);
    frame(&mut app, &ctx, 1.7, vec![]);
    assert_eq!(app.session.active().unwrap().selection.objects.len(), 1);
    assert_eq!(app.session.documents()[0].doc, source);
    assert!(app.session.documents()[1].history.undo.is_empty());
}

/// Verify closing the originating document cancels a transfer before its eventual mouse release.
#[test]
fn closing_the_source_during_a_transfer_cancels_it() {
    let (mut app, ctx, _, tabs) = drag_fixture();
    rest(&mut app, &ctx, 0.2, tabs[1].center());
    let target = app.session.active().unwrap().doc.clone();
    app.session.execute("file.close", &json!({"index":0})).unwrap();
    let drop = at(&app, 175.0, 180.0);
    frame(&mut app, &ctx, 0.9, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 1.0, vec![button(drop, false)]);
    frame(&mut app, &ctx, 1.1, vec![]);
    assert_eq!(app.session.active().unwrap().doc, target);
    assert!(app.session.active().unwrap().history.undo.is_empty());
}

/// Verify a source edit (with a reused node id), a closed target and a switched target cannot release stale artwork.
#[test]
fn source_edits_target_closure_and_switches_cancel_stale_transfers() {
    for change in ["source", "close target", "switch target"] {
        let (mut app, ctx, _, tabs) = drag_fixture();
        let source_id = app.session.active().unwrap().selection.objects[0];
        rest(&mut app, &ctx, 0.2, tabs[1].center());
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
            _ => {
                app.session.set_active(2);
            }
        }
        let before = app.session.active().unwrap().doc.clone();
        let undo = app.session.active().unwrap().history.undo.len();
        let drop = at(&app, 175.0, 180.0);
        frame(&mut app, &ctx, 0.9, vec![Event::PointerMoved(drop)]);
        frame(&mut app, &ctx, 1.0, vec![button(drop, false)]);
        frame(&mut app, &ctx, 1.1, vec![]);
        assert_eq!(app.session.active().unwrap().doc, before, "{change}");
        assert_eq!(app.session.active().unwrap().history.undo.len(), undo, "{change}");
    }
}

/// Artwork held over the tab of a document with other assigned color profiles switches to it and
/// drops there: bringing those profiles to the front redraws the source without editing it.
#[test]
fn a_drop_on_a_document_with_other_assigned_profiles_copies_the_artwork() {
    const CHILD: &str = "VECTORCRAFT_TEST_DRAG_ASSIGNED_PROFILES";
    if std::env::var_os(CHILD).is_none() {
        // Color settings are process-wide: change them in a separate test process.
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "tests::document_drag::a_drop_on_a_document_with_other_assigned_profiles_copies_the_artwork", "--nocapture"])
            .env(CHILD, "1")
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
            "isolated drag test failed:\n{stdout}\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        return;
    }
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width":400,"height":300,"name":"Source"})).unwrap();
    app.run("shape.rectangle", json!({"x":50,"y":50,"width":100,"height":100})).unwrap();
    app.run("file.new", json!({"width":400,"height":300,"name":"Target"})).unwrap();
    app.run("edit.assignProfile", json!({"cmyk": vectorcraft_color::cms::DEVICE_CMYK})).unwrap();
    app.session.set_active(0);
    let source = app.session.active().unwrap().doc.clone();
    app.select_tool("selection");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, 0.0, vec![]);
    let tabs = frame(&mut app, &ctx, 0.01, vec![]);
    let grab = at(&app, 75.0, 80.0);
    frame(&mut app, &ctx, 0.05, vec![Event::PointerMoved(grab), button(grab, true)]);
    frame(&mut app, &ctx, 0.1, vec![Event::PointerMoved(grab + egui::vec2(20.0, 10.0))]);
    assert!(app.session.in_interaction());
    rest(&mut app, &ctx, 0.2, tabs[1].center());
    assert_eq!(app.session.active_index(), Some(1));
    let undo = app.session.active().unwrap().history.undo.len();
    let drop = at(&app, 175.0, 180.0);
    frame(&mut app, &ctx, 0.9, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 1.0, vec![button(drop, false)]);
    frame(&mut app, &ctx, 1.1, vec![]);
    let placed = app.session.active().unwrap().selection.objects.len();
    let steps = app.session.active().unwrap().history.undo.len();
    assert_eq!((placed, steps), (1, undo + 1), "the drop places the artwork");
    assert_eq!(app.session.documents()[0].doc, source);
}

/// Artwork held over the tab of another view of its own document (Window › New Window) switches to
/// that view, and a drop there moves it.
#[test]
fn dragging_artwork_over_another_views_tab_switches_to_that_view() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300, "name": "Logo"})).unwrap();
    let id = app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()["id"].as_u64().unwrap();
    app.run("window.newWindow", json!({})).unwrap();
    crate::doc_windows::show_window(&mut app, (0, 1));
    app.select_tool("selection");
    app.ui.task_bar = false;
    app.ui.view.rulers = false;
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, 0.0, vec![]);
    let tabs = frame(&mut app, &ctx, 0.01, vec![]);
    assert_eq!(tabs.len(), 2);
    let grab = at(&app, 100.0, 100.0);
    frame(&mut app, &ctx, 0.05, vec![Event::PointerMoved(grab), button(grab, true)]);
    frame(&mut app, &ctx, 0.1, vec![Event::PointerMoved(grab + egui::vec2(20.0, 10.0))]);
    assert!(app.session.in_interaction());
    rest(&mut app, &ctx, 0.2, tabs[1].center());
    assert_eq!(crate::doc_windows::current(&app), Some((0, 2)), "resting the artwork over the other view's tab shows that view");
    assert!(!app.session.tool_busy(), "the switch resets the Selection tool before the drop");
    let drop = at(&app, 200.0, 150.0);
    frame(&mut app, &ctx, 0.9, vec![Event::PointerMoved(drop)]);
    frame(&mut app, &ctx, 1.0, vec![button(drop, false)]);
    frame(&mut app, &ctx, 1.1, vec![]);
    let st = app.session.active().unwrap();
    let moved = st.doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap();
    assert!((moved.center().x - 200.0).abs() < 0.01 && (moved.center().y - 150.0).abs() < 0.01, "{moved:?}");
    assert!(!app.session.tool_busy(), "the move leaves the Selection tool idle");
}
