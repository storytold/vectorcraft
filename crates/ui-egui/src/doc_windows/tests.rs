use egui::{Pos2, Shape, pos2, vec2};
use vectorcraft_engine::Session;

use super::*;
use crate::menus::enabled;

/// An app with `n` new 400 × 300 pt documents, the last one current.
fn app_with(n: usize) -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    for _ in 0..n {
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    }
    app
}

/// One headless frame of the canvas panel's body (the tabs and the canvas or panes) on a
/// 1200 × 800 window, then the floating document windows, which egui draws inside the window →
/// the shapes it drew, `Shape::Vec`s flattened.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<Shape> {
    fn flat(s: Shape, out: &mut Vec<Shape>) {
        match s {
            Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
            s => out.push(s),
        }
    }
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
        max_texture_side: Some(8192),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(raw, |ui| {
        app.canvas_area(ui);
        show(app, ui.ctx());
    });
    out.textures_delta.clear();
    let mut shapes = vec![];
    out.shapes.into_iter().for_each(|c| flat(c.shape, &mut shapes));
    shapes
}

fn button(pos: Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// A press and release at `pos`, then a frame to settle.
fn click(app: &mut VectorcraftApp, ctx: &egui::Context, pos: Pos2) {
    frame(app, ctx, vec![egui::Event::PointerMoved(pos), button(pos, true)]);
    frame(app, ctx, vec![button(pos, false)]);
    frame(app, ctx, vec![]);
}

/// Where the last frame drew document `i`'s window body.
fn body_of(app: &VectorcraftApp, i: usize) -> egui::Rect {
    let uid = app.session.documents()[i].uid;
    app.ui.floating.iter().find(|w| w.uid == uid).and_then(|w| w.body).unwrap()
}

/// The whole of document `i`'s window inside the main window, title bar included.
fn window_of(app: &VectorcraftApp, ctx: &egui::Context, i: usize) -> egui::Rect {
    let uid = app.session.documents()[i].uid;
    let w = app.ui.floating.iter().find(|w| w.uid == uid).unwrap();
    ctx.memory(|m| m.area_rect(window_id(w))).unwrap()
}

/// A drag with the primary button from `from` to `to`, then a frame to settle.
fn drag(app: &mut VectorcraftApp, ctx: &egui::Context, from: Pos2, to: Pos2) {
    frame(app, ctx, vec![egui::Event::PointerMoved(from)]);
    frame(app, ctx, vec![button(from, true)]);
    for k in 1..=4 {
        frame(app, ctx, vec![egui::Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))]);
    }
    frame(app, ctx, vec![button(to, false)]);
    frame(app, ctx, vec![]);
}

/// Whether the last frame drew text holding `title` in the tab strip (the top 35 points).
fn tab_drawn(shapes: &[Shape], title: &str) -> bool {
    shapes.iter().any(|s| matches!(s, Shape::Text(t) if t.pos.y < 35.0 && t.galley.text().contains(title)))
}

#[test]
fn cascade_places_step_down_and_right_inside_the_area() {
    let area = egui::Rect::from_min_size(pos2(10.0, 35.0), vec2(1190.0, 765.0));
    let places = cascade_places(area, 5);
    assert_eq!(places.len(), 5);
    for (k, p) in places.iter().enumerate() {
        assert!(area.contains_rect(*p), "place {k} is inside the area: {p:?}");
        assert_eq!(p.size(), places[0].size(), "every window is the same size");
        let offset = CASCADE_MARGIN + k as f32 * CASCADE_STEP;
        assert_eq!(p.min, area.min + vec2(offset, offset), "place {k} is {offset} pt down and right");
    }
    // A small area holds fewer steps: the stack starts again at the corner and stays inside.
    let small = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(500.0, 400.0));
    let places = cascade_places(small, 12);
    assert!(places.iter().all(|p| small.contains_rect(*p)), "{places:?}");
    assert!(places.iter().skip(1).any(|p| p.min == places[0].min), "the stack starts again: {places:?}");
    // An area smaller than a window: the window fills it.
    let tiny = egui::Rect::from_min_size(pos2(5.0, 5.0), vec2(100.0, 80.0));
    assert_eq!(cascade_places(tiny, 2), [tiny, tiny]);
}

#[test]
fn a_control_channel_keeps_the_windows_inside_the_main_window() {
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let plain = app_with(0);
    let wayland = ["WAYLAND_DISPLAY", "WAYLAND_SOCKET"].iter().any(|n| std::env::var_os(n).is_some_and(|v| !v.is_empty()));
    if !wayland {
        assert!(native(&plain, &ctx), "without a control channel the windows are OS windows");
    }
    let (_tx, rx) = std::sync::mpsc::channel();
    let controlled = VectorcraftApp::new(Session::new(), Default::default()).with_control(rx);
    assert!(!native(&controlled, &ctx), "with a control channel the windows stay inside the main window");
    ctx.set_embed_viewports(true);
    assert!(!native(&plain, &ctx), "where egui draws every viewport inside the main window");
}

#[test]
fn float_in_window_moves_the_current_document_out_of_the_tabs() {
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    let shapes = frame(&mut app, &ctx, vec![]);
    let title = app.session.documents()[2].title();
    assert!(tab_drawn(&shapes, &title), "the current document has a tab");
    let tabbed = app.canvas_rect.unwrap();
    assert_eq!(app.run("window.arrange.floatInWindow", json!({})).unwrap(), json!({"floating": [2]}));
    frame(&mut app, &ctx, vec![]);
    let shapes = frame(&mut app, &ctx, vec![]);
    assert!(!tab_drawn(&shapes, &title), "a floating document has no tab");
    assert!(tab_drawn(&shapes, &app.session.documents()[0].title()));
    assert_eq!(docked(&app), [0, 1]);
    assert_eq!(app.session.active_index(), Some(2), "the floating document stays the current one");
    // Its window holds the canvas, fitted to the window; the main window shows a tabbed document.
    let body = body_of(&app, 2);
    let canvas = app.canvas_rect.unwrap();
    assert!(body.contains_rect(canvas) && canvas.width() < tabbed.width(), "the canvas is in the window: {canvas:?} in {body:?}");
    assert!(app.views[2].fitted);
    assert_eq!(tab_document(&app), Some(0));
    assert_eq!(crate::control::inspect(&app, &ctx)["floating"], json!([2]));
    // Floating it again leaves it where it is.
    assert_eq!(app.run("window.arrange.floatInWindow", json!({})).unwrap(), json!({"floating": [2]}));
    assert_eq!(app.ui.floating.len(), 1);
}

#[test]
fn a_press_in_another_documents_window_or_the_main_window_makes_its_document_current() {
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.active_index(), Some(1));
    // The lower left of document 0's window, clear of document 1's window, which cascades below
    // and to the right of it, and of the window's resize edges.
    let body = body_of(&app, 0);
    click(&mut app, &ctx, pos2(body.left() + 12.0, body.bottom() - 12.0));
    assert_eq!(app.session.active_index(), Some(0), "a press in its window makes document 0 current");
    assert!(body_of(&app, 0).contains_rect(app.canvas_rect.unwrap()), "its window holds the canvas now");
    // The main window shows the only tabbed document as a pane: a press there makes it current.
    assert_eq!(tab_document(&app), Some(2));
    click(&mut app, &ctx, pos2(1100.0, 700.0));
    assert_eq!(app.session.active_index(), Some(2));
    // A press on the title bar of document 0's window, in front, makes document 0 current again.
    let (window, body) = (window_of(&app, &ctx, 0), body_of(&app, 0));
    click(&mut app, &ctx, pos2(window.left() + 60.0, (window.top() + body.top()) / 2.0));
    assert_eq!(app.session.active_index(), Some(0), "a press on its title bar makes document 0 current");
}

#[test]
fn a_resized_window_fits_its_document_again_unless_it_was_zoomed() {
    let mut app = app_with(2);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let (before, other) = (app.views[1].zoom, app.views[0].zoom);
    // Document 1's window, in front and holding the canvas, grows from its lower-right corner.
    let window = window_of(&app, &ctx, 1);
    drag(&mut app, &ctx, window.right_bottom() - vec2(2.0, 2.0), window.right_bottom() + vec2(150.0, 100.0));
    assert!(window_of(&app, &ctx, 1).width() > window.width() + 100.0, "the window grew");
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.active_index(), Some(1));
    assert!(app.views[1].zoom > before, "the document fits its larger window: {before} → {}", app.views[1].zoom);
    assert_eq!(app.views[0].zoom, other, "the other window keeps its size and its document's zoom");
    // A zoomed view keeps its zoom when its window changes size again.
    app.run("view.zoomIn", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let zoomed = app.views[1].zoom;
    let window = window_of(&app, &ctx, 1);
    drag(&mut app, &ctx, window.right_bottom() - vec2(2.0, 2.0), window.right_bottom() - vec2(100.0, 60.0));
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.views[1].zoom, zoomed, "a zoomed document keeps its zoom");
}

#[test]
fn cascade_floats_every_document_and_stacks_the_windows() {
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.tile", json!({})).unwrap();
    app.run("document.activate", json!({"index": 1})).unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    assert_eq!(app.run("window.arrange.cascade", json!({})).unwrap(), json!({"floating": [0, 1, 2]}));
    assert!(!app.ui.tiled, "Cascade ends Tile");
    assert!(docked(&app).is_empty());
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let area = app.canvas.windows.area.unwrap();
    let places: Vec<egui::Rect> = app.ui.floating.iter().map(|w| w.place).collect();
    assert_eq!(places, cascade_places(area, 3), "the windows stack in tab order, document 1's moved from where it floated");
    for (i, place) in places.iter().copied().enumerate() {
        let body = body_of(&app, i);
        assert!((body.left() - place.left()).abs() < 4.0 && body.top() > place.top(), "document {i}'s window is at its place: {body:?} {place:?}");
        assert!(app.views[i].fitted, "document {i} fits its window");
    }
    assert!(body_of(&app, 1).contains_rect(app.canvas_rect.unwrap()), "the current document's window holds the canvas");
}

#[test]
fn float_all_in_windows_floats_the_tabbed_documents_and_consolidate_brings_them_back() {
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let tabbed = app.canvas_rect.unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let first = app.ui.floating[0].place;
    assert_eq!(app.run("window.arrange.floatAllInWindows", json!({})).unwrap(), json!({"floating": [2, 0, 1]}));
    assert_eq!(app.ui.floating[0].place, first, "a window already floating stays where it is");
    assert!(docked(&app).is_empty());
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.run("window.arrange.consolidateAllWindows", json!({})).unwrap(), json!({"tiled": false, "floating": []}));
    frame(&mut app, &ctx, vec![]);
    let shapes = frame(&mut app, &ctx, vec![]);
    assert_eq!(docked(&app), [0, 1, 2]);
    for d in app.session.documents() {
        assert!(tab_drawn(&shapes, &d.title()), "{} has its tab again", d.title());
    }
    assert_eq!(app.canvas_rect, Some(tabbed), "the canvas is under the tabs again");
    assert!(app.views[2].fitted);
}

#[test]
fn tile_brings_floating_documents_back_into_the_grid() {
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.tile", json!({})).unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let docs: Vec<usize> = app.canvas.panes.last.iter().map(|(w, _)| w.0).collect();
    assert_eq!(docs, [0, 1], "a floating document leaves the grid; the tabbed ones share it");
    assert_eq!(app.run("window.arrange.tile", json!({})).unwrap(), json!({"tiled": true, "panes": 3}));
    frame(&mut app, &ctx, vec![]);
    let docs: Vec<usize> = app.canvas.panes.last.iter().map(|(w, _)| w.0).collect();
    assert_eq!(docs, [0, 1, 2]);
    assert!(app.ui.floating.is_empty());
}

#[test]
fn closing_a_floating_window_closes_its_document_and_asks_to_save_an_unsaved_one() {
    let mut app = app_with(2);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    // Document 0's window is behind document 1's: its close button is at the top right of its
    // title bar, clear of document 1's window to the right and below.
    let close_button = |app: &VectorcraftApp, i: usize| {
        let uid = app.session.documents()[i].uid;
        let w = app.ui.floating.iter().find(|w| w.uid == uid).unwrap();
        let area = ctx.memory(|m| m.area_rect(window_id(w))).unwrap();
        let body = w.body.unwrap();
        (area, body)
    };
    let (area0, body0) = close_button(&app, 0);
    assert!(body0.top() > area0.top(), "the window has a title bar");
    // Close document 1 (in front): it is unsaved after an edit, so the app asks first.
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    assert!(app.session.documents()[1].is_dirty());
    frame(&mut app, &ctx, vec![]);
    // The close button sits near the right end of the title bar.
    let press_close = |app: &mut VectorcraftApp, i: usize| {
        let (area, body) = close_button(app, i);
        let bar = (area.top() + body.top()) / 2.0;
        let n = app.session.documents().len();
        for dx in (4..40).step_by(3) {
            click(app, &ctx, pos2(area.right() - dx as f32, bar));
            if app.ui.dialog.is_some() || app.session.documents().len() < n {
                return true;
            }
        }
        false
    };
    assert!(press_close(&mut app, 1), "the close button of document 1's window acts");
    assert!(app.ui.dialog.is_some(), "an unsaved document asks to be saved");
    assert_eq!(app.session.documents().len(), 2);
    // While the prompt is open the windows take no press.
    let body = body_of(&app, 0);
    click(&mut app, &ctx, pos2(body.left() + 12.0, body.bottom() - 12.0));
    assert_eq!(app.session.active_index(), Some(1), "a press in another window waits for the prompt");
    app.ui.dialog = None;
    // Document 0 has no changes: its window closes with it.
    app.run("document.activate", json!({"index": 0})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert!(press_close(&mut app, 0), "the close button of document 0's window acts");
    assert_eq!(app.session.documents().len(), 1);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.ui.floating.len(), 1, "its window has gone");
}

#[test]
fn presentation_mode_shows_the_floating_current_document_in_the_main_window() {
    let mut app = app_with(2);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let body = body_of(&app, 1);
    assert!(body.contains_rect(app.canvas_rect.unwrap()), "the current document's window holds the canvas");
    let windowed = (app.views[0].zoom, app.views[1].zoom);
    app.run("view.presentation", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let shown = app.canvas_rect.unwrap();
    assert!(shown.width() > body.width() + 100.0, "the main window shows the current document: {shown:?}, its window was {body:?}");
    // The presentation goes on to the other floating document, fitted to the main window, then
    // ends on the first one.
    app.run("document.activate", json!({"index": 0})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    app.run("document.activate", json!({"index": 1})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert!(app.views[0].zoom > windowed.0, "the presentation fitted document 0 to the main window");
    app.run("view.presentation", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert!(body_of(&app, 1).contains_rect(app.canvas_rect.unwrap()), "the window holds the canvas again");
    assert_eq!(app.ui.floating.len(), 2, "the windows come back");
    for (i, before) in [windowed.0, windowed.1].into_iter().enumerate() {
        assert!((app.views[i].zoom - before).abs() < 1e-6, "document {i} fits its window again: {before} → {}", app.views[i].zoom);
    }
}

#[test]
fn a_document_made_current_in_presentation_mode_comes_to_the_front_after_it() {
    let mut app = app_with(2);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let layer = |app: &VectorcraftApp, i: usize| {
        let uid = app.session.documents()[i].uid;
        let w = app.ui.floating.iter().find(|w| w.uid == uid).unwrap();
        egui::LayerId::new(egui::Order::Middle, window_id(w))
    };
    let top = |ctx: &egui::Context| ctx.memory(|m| m.areas().top_layer_id(egui::Order::Middle));
    assert_eq!(top(&ctx), Some(layer(&app, 1)), "the current document's window is in front");
    app.run("view.presentation", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.run("document.activate", json!({"index": 0})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.run("view.presentation", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    assert_eq!(top(&ctx), Some(layer(&app, 0)), "document 0's window comes to the front");
}

/// A stand-in for the window system of the documents' OS windows: it runs each one's frame, asks
/// each to close while `close` is set, and records the commands the windows were sent. Each
/// window is 500 × 400 points with its top-left corner at `origin` on the screen, and its next
/// frame takes `events`.
#[derive(Default)]
struct Host {
    close: bool,
    commands: Vec<(egui::ViewportId, egui::ViewportCommand)>,
    origin: Pos2,
    events: Vec<egui::Event>,
}

fn native_host() -> std::rc::Rc<std::cell::RefCell<Host>> {
    let host = std::rc::Rc::new(std::cell::RefCell::new(Host::default()));
    let shared = host.clone();
    egui::Context::set_immediate_viewport_renderer(move |ctx, mut viewport| {
        let id = viewport.ids.this;
        let close = if shared.borrow().close { vec![egui::ViewportEvent::Close] } else { vec![] };
        let size = vec2(500.0, 400.0);
        let rect = egui::Rect::from_min_size(Pos2::ZERO, size);
        let on_screen = egui::Rect::from_min_size(shared.borrow().origin, size);
        let info = egui::ViewportInfo {
            events: close,
            inner_rect: Some(on_screen),
            outer_rect: Some(on_screen),
            focused: Some(false),
            ..Default::default()
        };
        let events = std::mem::take(&mut shared.borrow_mut().events);
        let input = egui::RawInput {
            viewport_id: id,
            viewports: std::iter::once((id, info)).collect(),
            screen_rect: Some(rect),
            events,
            ..Default::default()
        };
        let mut full = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
        full.textures_delta.clear();
        let mut host = shared.borrow_mut();
        for (vid, output) in full.viewport_output {
            host.commands.extend(output.commands.into_iter().map(|c| (vid, c)));
        }
    });
    host
}

#[test]
fn closing_an_os_window_keeps_it_while_its_unsaved_document_stays_open() {
    let host = native_host();
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = app_with(2);
    if !native(&app, &ctx) {
        // Wayland: the windows are inside the main window.
        return;
    }
    frame(&mut app, &ctx, vec![]);
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let vid = viewport_id(&app.ui.floating[0]);
    host.borrow_mut().close = true;
    frame(&mut app, &ctx, vec![]);
    host.borrow_mut().close = false;
    assert!(app.ui.dialog.is_some(), "the unsaved document asks to be saved");
    assert_eq!(app.session.documents().len(), 2);
    assert!(is_floating(&app, 1), "the document still floats");
    assert!(
        host.borrow().commands.iter().any(|(id, c)| *id == vid && *c == egui::ViewportCommand::CancelClose),
        "its OS window is kept open: {:?}",
        host.borrow().commands
    );
}

/// Where the last frame drew the title of `title`'s tab in the tab strip.
fn tab_at(shapes: &[Shape], title: &str) -> Pos2 {
    shapes
        .iter()
        .find_map(|s| match s {
            Shape::Text(t) if t.pos.y < 35.0 && t.galley.text().contains(title) => Some(t.pos + vec2(4.0, 4.0)),
            _ => None,
        })
        .unwrap()
}

#[test]
fn a_tab_is_torn_off_only_well_above_or_below_the_strip() {
    let strip = egui::Rect::from_min_max(pos2(0.0, 40.0), pos2(1200.0, 75.0));
    assert!(!torn_off(strip, pos2(300.0, 60.0)), "along the strip");
    assert!(!torn_off(strip, pos2(300.0, 75.0 + TEAR_OFF - 1.0)), "a wobble below the strip");
    assert!(torn_off(strip, pos2(300.0, 75.0 + TEAR_OFF + 1.0)));
    assert!(torn_off(strip, pos2(300.0, 40.0 - TEAR_OFF - 1.0)), "dragged up");
}

#[test]
fn dragging_a_tab_off_the_strip_floats_its_document_at_the_pointer() {
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let shapes = frame(&mut app, &ctx, vec![]);
    let title = app.session.documents()[0].title();
    let tab = tab_at(&shapes, &title);
    // A short drag, and a drag along the strip, leave the tabs as they are.
    drag(&mut app, &ctx, tab, tab + vec2(0.0, 35.0 + TEAR_OFF - 10.0 - tab.y));
    drag(&mut app, &ctx, tab, tab + vec2(300.0, 0.0));
    assert!(app.ui.floating.is_empty(), "the document stays under its tab");
    assert_eq!(app.session.active_index(), Some(2), "a drag is not a click");
    // Dragged well below the strip, it floats in a window under the pointer and becomes current.
    let to = pos2(300.0, 200.0);
    drag(&mut app, &ctx, tab, to);
    assert_eq!(docked(&app), [1, 2]);
    assert_eq!(app.session.active_index(), Some(0), "the torn-off document is the current one");
    assert_eq!(app.ui.floating[0].place.min, to - GRAB, "its window opens under the pointer");
    frame(&mut app, &ctx, vec![]);
    let shapes = frame(&mut app, &ctx, vec![]);
    assert!(!tab_drawn(&shapes, &title), "it has left the strip");
    let window = window_of(&app, &ctx, 0);
    assert!(window.contains(to) && (window.min - (to - GRAB)).length() < 1.0, "the pointer is over its window: {window:?}");
    assert!(body_of(&app, 0).contains_rect(app.canvas_rect.unwrap()), "its window holds the canvas");
}

#[test]
fn a_window_dragged_by_its_title_bar_onto_the_strip_goes_back_under_the_tabs() {
    let mut app = app_with(3);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let title_bar = |app: &VectorcraftApp, i: usize| {
        let (window, body) = (window_of(app, &ctx, i), body_of(app, i));
        pos2(window.left() + 60.0, (window.top() + body.top()) / 2.0)
    };
    // Moved elsewhere, document 2's window (in front) stays a window.
    let from = title_bar(&app, 2);
    drag(&mut app, &ctx, from, from + vec2(40.0, 30.0));
    assert_eq!(app.ui.floating.len(), 3, "a window moved off the strip still floats");
    // Dropped on the strip, it goes back under its tab, current; the other windows stay.
    let from = title_bar(&app, 2);
    drag(&mut app, &ctx, from, pos2(600.0, 15.0));
    assert_eq!(docked(&app), [2]);
    assert_eq!(app.session.active_index(), Some(2));
    assert_eq!(app.ui.floating.len(), 2);
    frame(&mut app, &ctx, vec![]);
    let shapes = frame(&mut app, &ctx, vec![]);
    assert!(tab_drawn(&shapes, &app.session.documents()[2].title()), "it has its tab again");
    assert!(app.views[2].fitted, "it fits the canvas under the tabs");
    let canvas = app.canvas_rect.unwrap();
    assert!(canvas.top() >= 35.0 && canvas.width() > 1000.0, "the canvas under the tabs shows it: {canvas:?}");
    // Torn off again, it opens where its tab is dropped, not where its last window was.
    let to = pos2(200.0, 300.0);
    let tab = tab_at(&shapes, &app.session.documents()[2].title());
    drag(&mut app, &ctx, tab, to);
    frame(&mut app, &ctx, vec![]);
    let window = window_of(&app, &ctx, 2);
    assert!((window.min - (to - GRAB)).length() < 1.0, "its new window is under the pointer: {window:?}");
}

#[test]
fn after_touch_input_a_window_dragged_onto_the_strip_still_docks() {
    let mut app = app_with(2);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    // A touch makes egui move windows by a drag anywhere on them.
    let touch = egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(1),
        phase: egui::TouchPhase::Start,
        pos: pos2(1100.0, 700.0),
        force: None,
    };
    frame(&mut app, &ctx, vec![touch]);
    frame(&mut app, &ctx, vec![]);
    let (window, body) = (window_of(&app, &ctx, 1), body_of(&app, 1));
    let from = pos2(window.left() + 60.0, (window.top() + body.top()) / 2.0);
    drag(&mut app, &ctx, from, pos2(600.0, 15.0));
    assert_eq!(docked(&app), [1], "document 1 is back under the tabs");
}

#[test]
fn a_document_floated_again_after_cascade_opens_where_it_is_asked_to() {
    let mut app = app_with(2);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.cascade", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.consolidateAllWindows", json!({"index": 1})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let at = pos2(200.0, 300.0);
    app.run("window.arrange.floatInWindow", json!({"index": 1, "at": [at.x, at.y]})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let window = window_of(&app, &ctx, 1);
    assert!((window.min - at).length() < 1.0, "its new window opens at {at:?}: {window:?}");
}

#[test]
fn float_in_window_and_consolidate_take_a_document() {
    let mut app = app_with(3);
    assert_eq!(app.run("window.arrange.floatInWindow", json!({"index": 0, "at": [100, 120]})).unwrap(), json!({"floating": [0]}));
    assert_eq!(app.session.active_index(), Some(0), "document 0 becomes the current one");
    assert_eq!(app.ui.floating[0].place.min, pos2(100.0, 120.0));
    app.run("window.arrange.floatInWindow", json!({"index": 1})).unwrap();
    assert_eq!(app.run("window.arrange.floatInWindow", json!({"index": 3})), Err("no such document".to_string()));
    app.run("window.arrange.tile", json!({})).unwrap();
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    assert_eq!(app.run("window.arrange.consolidateAllWindows", json!({"index": 1})).unwrap(), json!({"tiled": true, "floating": [0, 2]}));
    assert_eq!(app.session.active_index(), Some(1));
    assert_eq!(docked(&app), [1], "only document 1 goes back under the tabs");
    assert_eq!(app.run("window.arrange.consolidateAllWindows", json!({"index": 7})), Err("no such document".to_string()));
    assert_eq!(app.run("window.arrange.consolidateAllWindows", json!({})).unwrap(), json!({"tiled": false, "floating": []}));
}

#[test]
fn with_open_documents_as_tabs_cleared_new_documents_open_in_floating_windows() {
    let mut app = app_with(1);
    let ctx = egui::Context::default();
    app.run("prefs.set", json!({"key": "openDocumentsAsTabs", "value": false})).unwrap();
    assert_eq!(docked(&app), [0], "a document already open stays under its tab");
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    assert_eq!(docked(&app), [0]);
    assert_eq!(floating_indices(&app), [1, 2], "each new document opens in a window of its own");
    assert_eq!(app.session.active_index(), Some(2), "the newest document is the current one");
    assert_ne!(app.ui.floating[0].place.min, app.ui.floating[1].place.min, "the windows open at cascade places");
    frame(&mut app, &ctx, vec![]);
    let shapes = frame(&mut app, &ctx, vec![]);
    assert!(!tab_drawn(&shapes, &app.session.documents()[2].title()), "a document in a window has no tab");
    assert!(tab_drawn(&shapes, &app.session.documents()[0].title()));
    assert!(body_of(&app, 2).contains_rect(app.canvas_rect.unwrap()), "the new document's window holds the canvas");
}

#[test]
fn documents_open_when_the_app_starts_keep_their_tabs() {
    let mut session = Session::new();
    session.prefs.open_documents_as_tabs = false;
    session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let mut app = VectorcraftApp::new(session, Default::default());
    app.sync_views();
    assert_eq!(docked(&app), [0]);
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    assert_eq!(floating_indices(&app), [1]);
}

#[test]
fn a_document_created_in_a_batch_that_closes_another_still_floats() {
    let mut app = app_with(2);
    app.run("prefs.set", json!({"key": "openDocumentsAsTabs", "value": false})).unwrap();
    let steps = json!([
        {"command": "file.close", "params": {}},
        {"command": "file.new", "params": {"width": 400, "height": 300}},
    ]);
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!(app.session.documents().len(), 2, "one document closed and one opened");
    assert_eq!(docked(&app), [0]);
    assert_eq!(floating_indices(&app), [1], "the new document opens in a window of its own");
}

#[test]
fn each_document_in_a_batch_follows_the_preference_when_it_opened() {
    let mut app = app_with(1);
    let new = json!({"command": "file.new", "params": {"width": 400, "height": 300}});
    let tabs = |on: bool| json!({"command": "prefs.set", "params": {"key": "openDocumentsAsTabs", "value": on}});
    let steps = json!([tabs(false), new, tabs(true), new, tabs(false)]);
    app.run("command.batch", json!({"commands": steps})).unwrap();
    assert_eq!(floating_indices(&app), [1], "opened with the box cleared");
    assert_eq!(docked(&app), [0, 2], "opened with the box checked");
}

#[test]
fn checking_open_documents_as_tabs_again_leaves_floating_documents_where_they_are() {
    let mut app = app_with(0);
    app.run("prefs.set", json!({"key": "openDocumentsAsTabs", "value": false})).unwrap();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("prefs.set", json!({"key": "openDocumentsAsTabs", "value": true})).unwrap();
    assert_eq!(floating_indices(&app), [0], "the floating document stays in its window");
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    assert_eq!(docked(&app), [1], "a new document opens under a tab");
    assert_eq!(floating_indices(&app), [0]);
}

#[test]
fn an_os_window_moves_by_its_tab_and_docks_on_the_strip() {
    let host = native_host();
    let ctx = egui::Context::default();
    ctx.set_embed_viewports(false);
    let mut app = app_with(2);
    if !native(&app, &ctx) {
        // Wayland: the windows are inside the main window.
        return;
    }
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    // The OS window's top-left corner is at (300, 300) on the screen; the main window's at (0, 0).
    host.borrow_mut().origin = pos2(300.0, 300.0);
    frame(&mut app, &ctx, vec![]);
    let os_drag = |app: &mut VectorcraftApp, from: Pos2, to: Pos2| {
        let steps = [vec![egui::Event::PointerMoved(from)], vec![button(from, true)]]
            .into_iter()
            .chain((1..=4).map(|k| vec![egui::Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))]))
            .chain([vec![button(to, false)], vec![]]);
        for events in steps {
            host.borrow_mut().events = events;
            frame(app, &ctx, vec![]);
        }
    };
    // The tab at the top of the window: dragged down and right, the window follows the pointer.
    let tab = pos2(20.0, 15.0);
    os_drag(&mut app, tab, tab + vec2(100.0, 50.0));
    assert!(is_floating(&app, 1), "a window moved off the strip still floats");
    let w = &app.ui.floating[0];
    assert_eq!(w.screen.map(|s| s.0), Some(pos2(400.0, 350.0)), "the window moved with the pointer");
    assert_eq!(w.place.min, pos2(400.0, 350.0), "in the main window's points");
    // Released over the main window's tab strip (y 0 to 35 on the screen), it docks.
    os_drag(&mut app, tab, pos2(20.0, -280.0));
    assert!(app.ui.floating.is_empty(), "the document is back under the tabs");
    assert_eq!(docked(&app), [0, 1]);
    assert_eq!(app.session.active_index(), Some(1));
}

#[test]
fn the_arrange_items_are_enabled_as_the_documents_allow() {
    let ids = [
        "window.arrange.cascade",
        "window.arrange.tile",
        "window.arrange.floatInWindow",
        "window.arrange.floatAllInWindows",
        "window.arrange.consolidateAllWindows",
    ];
    let states = |app: &VectorcraftApp| ids.map(|id| enabled(app, id));
    let mut app = app_with(0);
    assert_eq!(states(&app), [false; 5], "no document");
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    assert_eq!(states(&app), [false, false, true, true, false], "one tabbed document");
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    assert_eq!(states(&app), [true, true, true, true, false], "two tabbed documents");
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    assert_eq!(states(&app), [true, true, false, true, true], "the current document floats, one is tabbed");
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    assert_eq!(states(&app), [true, true, false, false, true], "every document floats");
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("window.arrange.consolidateAllWindows", json!({})).unwrap();
    assert_eq!(states(&app), [true, true, true, true, false], "consolidated");
}

/// The title of window `w`'s tab.
fn title_of(app: &VectorcraftApp, w: Win) -> String {
    crate::chrome::document_window_title(app, w).unwrap()
}

#[test]
fn new_window_opens_a_numbered_view_with_its_own_zoom() {
    let mut app = app_with(1);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    let name = app.session.documents()[0].title();
    assert!(!title_of(&app, (0, 1)).contains(&format!("{name}:")), "one view: no number");
    let zoom = app.view().unwrap().zoom;
    assert_eq!(app.run("window.newWindow", json!({})).unwrap(), json!({"index": 0, "view": 2}));
    assert_eq!(windows(&app), [(0, 1), (0, 2)]);
    assert_eq!(current(&app), Some((0, 2)), "the new view is the current window");
    assert_eq!(app.view().unwrap().zoom, zoom, "it opens where the first view is");
    assert!(title_of(&app, (0, 1)).starts_with(&format!("{name}:1 @")), "{}", title_of(&app, (0, 1)));
    assert!(title_of(&app, (0, 2)).starts_with(&format!("{name}:2 @")), "{}", title_of(&app, (0, 2)));
    let shapes = frame(&mut app, &ctx, vec![]);
    assert!(tab_drawn(&shapes, &format!("{name}:1")) && tab_drawn(&shapes, &format!("{name}:2")), "each view has a tab");
    // Each view keeps its own zoom.
    app.run("view.zoomIn", json!({})).unwrap();
    let zoomed = app.view().unwrap().zoom;
    assert_ne!(zoomed, zoom);
    show_window(&mut app, (0, 1));
    assert_eq!(app.view().unwrap().zoom, zoom, "the first view did not zoom");
    assert_eq!(view(&app, (0, 2)).unwrap().zoom, zoomed);
    show_window(&mut app, (0, 2));
    assert_eq!(app.view().unwrap().zoom, zoomed);
    assert_eq!(app.run("window.newWindow", json!({})).unwrap(), json!({"index": 0, "view": 3}));
}

#[test]
fn an_edit_in_one_view_shows_in_the_other() {
    let mut app = app_with(1);
    let ctx = egui::Context::default();
    app.run("window.newWindow", json!({})).unwrap();
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.canvas.panes.last.iter().map(|(w, _)| *w).collect::<Vec<_>>(), [(0, 1), (0, 2)], "one pane per view");
    view_mut(&mut app, (0, 1)).unwrap().zoom = 0.5;
    app.view_mut().unwrap().zoom = 2.0;
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let d = &app.session.documents()[0];
    let shown = app.canvas.panes.shown.get(&(d.uid, 1)).map(|(k, _)| (k.revision, k.zoom));
    assert_eq!(shown, Some((d.revision, 0.5)), "the other view's pane shows the edit at its own zoom");
    assert_eq!(app.view().unwrap().zoom, 2.0, "the view edited in keeps its zoom");
}

#[test]
fn closing_a_view_leaves_the_document_open() {
    let mut app = app_with(1);
    let ctx = egui::Context::default();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    app.run("window.newWindow", json!({})).unwrap();
    app.run("window.newWindow", json!({})).unwrap();
    show_window(&mut app, (0, 2));
    assert_eq!(app.run("file.close", json!({})).unwrap(), json!({"index": 0, "views": 2}));
    assert!(app.ui.dialog.is_none(), "closing a view does not ask to save");
    assert_eq!((app.session.documents().len(), windows(&app)), (1, vec![(0, 1), (0, 3)]));
    assert_eq!(current(&app), Some((0, 3)), "the next view becomes the current window");
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.run("file.close", json!({})).unwrap(), json!({"index": 0, "views": 1}));
    assert_eq!(windows(&app), [(0, 1)], "the view left is the document's only view again");
    let name = app.session.documents()[0].title();
    assert!(!title_of(&app, (0, 1)).contains(&format!("{name}:")), "{}", title_of(&app, (0, 1)));
    // The last view: the document closes, asking to save it first.
    app.run("file.close", json!({})).unwrap();
    assert!(app.ui.dialog.is_some(), "the changed document asks to be saved");
    assert_eq!(app.session.documents().len(), 1);
}

#[test]
fn arrange_treats_each_view_as_a_window() {
    let mut app = app_with(1);
    let ctx = egui::Context::default();
    frame(&mut app, &ctx, vec![]);
    assert!(!enabled(&app, "window.arrange.tile") && !enabled(&app, "window.arrange.cascade"), "one window");
    app.run("window.newWindow", json!({})).unwrap();
    assert!(enabled(&app, "window.arrange.tile") && enabled(&app, "window.arrange.cascade"), "two views of one document");
    assert_eq!(app.run("window.arrange.tile", json!({})).unwrap(), json!({"tiled": true, "panes": 2}));
    app.run("window.arrange.cascade", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let uid = app.session.documents()[0].uid;
    let floating: Vec<(u64, u32)> = app.ui.floating.iter().map(|w| (w.uid, w.view)).collect();
    assert_eq!(floating, [(uid, 1), (uid, 2)], "each view in a window of its own, in tab order");
    assert_ne!(app.ui.floating[0].place.min, app.ui.floating[1].place.min, "stacked down and to the right");
    assert!(docked_windows(&app).is_empty());
    app.run("window.arrange.consolidateAllWindows", json!({})).unwrap();
    assert!(app.ui.floating.is_empty());
    assert_eq!(docked_windows(&app), [(0, 1), (0, 2)]);
    // Float in Window floats the current view only.
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    assert_eq!(app.ui.floating.iter().map(|w| (w.uid, w.view)).collect::<Vec<_>>(), [(uid, 2)]);
    assert_eq!(docked_windows(&app), [(0, 1)], "the first view keeps its tab");
    frame(&mut app, &ctx, vec![]);
    let shapes = frame(&mut app, &ctx, vec![]);
    let name = app.session.documents()[0].title();
    assert!(tab_drawn(&shapes, &format!("{name}:1")) && !tab_drawn(&shapes, &format!("{name}:2")));
    // A new view of a floating view floats too.
    app.run("window.newWindow", json!({})).unwrap();
    assert!(floats(&app, (0, 3)), "it opens beside the view it came from");
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    assert!(docked_windows(&app).is_empty());
    assert_eq!(app.run("window.arrange.consolidateAllWindows", json!({"index": 0, "view": 3})).unwrap()["floating"], json!([0, 0]));
    assert_eq!((docked_windows(&app), current(&app)), (vec![(0, 3)], Some((0, 3))));
}

#[test]
fn each_pane_draws_its_own_views_outline_or_preview() {
    let mut app = app_with(1);
    let ctx = egui::Context::default();
    app.run("window.newWindow", json!({})).unwrap();
    show_window(&mut app, (0, 1));
    app.run("view.outline", json!({})).unwrap();
    show_window(&mut app, (0, 2));
    assert!(!app.outline_view(), "the second view is in Preview");
    app.run("window.arrange.tile", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let uid = app.session.documents()[0].uid;
    assert_eq!(app.canvas.panes.shown.get(&(uid, 1)).map(|(k, _)| k.outline), Some(true), "the first view's pane is in Outline");
}

#[test]
fn next_and_previous_document_step_through_the_views() {
    let mut app = app_with(2);
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.run("window.newWindow", json!({})).unwrap();
    assert_eq!(app.run("window.nextDocument", json!({})).unwrap(), json!({"index": 1}));
    assert_eq!(app.run("window.nextDocument", json!({})).unwrap(), json!({"index": 0, "view": 1}));
    assert_eq!(app.run("window.nextDocument", json!({})).unwrap(), json!({"index": 0, "view": 2}));
    assert_eq!(current(&app), Some((0, 2)));
    assert_eq!(app.run("window.previousDocument", json!({})).unwrap(), json!({"index": 0, "view": 1}));
    assert_eq!(app.run("window.previousDocument", json!({})).unwrap(), json!({"index": 1}));
}

#[test]
fn switching_views_resets_the_tool() {
    let mut app = app_with(2);
    app.run("window.newWindow", json!({})).unwrap();
    let before = app.session.tool_resets();
    show_window(&mut app, (1, 1));
    assert_eq!(app.session.tool_resets(), before + 1, "another view of the current document resets the tool");
    show_window(&mut app, (1, 1));
    assert_eq!(app.session.tool_resets(), before + 1, "the current view stays as it is");
}

#[test]
fn switching_views_targets_the_artboard_the_view_shows() {
    let mut app = app_with(1);
    app.run("artboard.new", json!({})).unwrap();
    app.run("tool.select", json!({"tool": "artboard"})).unwrap();
    crate::panels::artboards::select(&mut app, 0);
    app.run("window.newWindow", json!({})).unwrap();
    crate::panels::artboards::select(&mut app, 1);
    assert_eq!(app.session.tool_options()["active"], json!(1));
    show_window(&mut app, (0, 1));
    assert_eq!(app.session.tool_options()["active"], json!(0), "the first view shows the first artboard");
    show_window(&mut app, (0, 2));
    assert_eq!(app.session.tool_options()["active"], json!(1));
}
