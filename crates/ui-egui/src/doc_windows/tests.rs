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
/// 1200 × 800 window, then the floating document windows, which egui draws inside the window, run
/// as the hosts run a frame → the shapes it drew, `Shape::Vec`s flattened.
fn frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> Vec<Shape> {
    fn flat(s: Shape, out: &mut Vec<Shape>) {
        match s {
            Shape::Vec(v) => v.into_iter().for_each(|s| flat(s, out)),
            s => out.push(s),
        }
    }
    let out = run_frame(app, ctx, events);
    let mut shapes = vec![];
    out.shapes.into_iter().for_each(|c| flat(c.shape, &mut shapes));
    shapes
}

/// One headless frame of the canvas panel's body and the floating document windows on a 1200 ×
/// 800 window. The windows' access nodes, by name, are kept for [`window_of`].
fn run_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::FullOutput {
    let raw = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0))),
        max_texture_side: Some(8192),
        events,
        ..Default::default()
    };
    ctx.enable_accesskit();
    let out = crate::testing::part_frame(app, ctx, raw, |app, ui| {
        app.canvas_area(ui);
        show(app, ui);
    });
    let nodes = out.platform_output.accesskit_update.as_ref().map_or(&[][..], |u| &u.nodes[..]);
    let named: Vec<Named> = nodes
        .iter()
        .filter(|(_, n)| matches!(n.role(), egui::accesskit::Role::Group | egui::accesskit::Role::TabList))
        .filter_map(|(_, n)| {
            let b = n.bounds()?;
            Some((n.role(), n.label()?.to_owned(), egui::Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32))))
        })
        .collect();
    ctx.data_mut(|d| d.insert_temp(egui::Id::new(NAMED), std::sync::Arc::new(named)));
    out
}

/// A group or tab list of the last frame's accessibility tree: its role, name and bounds.
type Named = (egui::accesskit::Role, String, egui::Rect);

/// Where [`run_frame`] keeps the last frame's [`Named`] nodes.
const NAMED: &str = "doc-window-tests-named";

/// The last frame's [`Named`] nodes of `role` whose name `matches`.
fn named(ctx: &egui::Context, role: egui::accesskit::Role, matches: impl Fn(&str) -> bool) -> Option<egui::Rect> {
    let nodes: std::sync::Arc<Vec<Named>> = ctx.data(|d| d.get_temp(egui::Id::new(NAMED))).unwrap_or_default();
    nodes.iter().find(|(r, name, _)| *r == role && matches(name)).map(|(_, _, rect)| *rect)
}

/// The main window's document tab strip, as the last frame drew it.
fn strip(ctx: &egui::Context) -> egui::Rect {
    named(ctx, egui::accesskit::Role::TabList, |name| name == "Documents").expect("the tab strip was drawn")
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

/// Where the last frame drew document `i`'s window body: the area of its view.
fn body_of(app: &VectorcraftApp, i: usize) -> egui::Rect {
    let uid = app.session.documents()[i].uid;
    let w = app.ui.floating.iter().find(|w| w.uid == uid).unwrap();
    view(app, find(app, (w.uid, w.view)).unwrap()).unwrap().rect
}

/// Where the current window's view drew its art in the last frame.
fn canvas_rect(app: &VectorcraftApp) -> egui::Rect {
    app.view().unwrap().rect
}

/// The whole of document `i`'s window, title bar included, as the last frame drew it: the bounds
/// of its access node, named by its title.
fn window_of(app: &VectorcraftApp, ctx: &egui::Context, i: usize) -> egui::Rect {
    let title = app.session.documents()[i].title();
    // The name is the title, a `*` while it has unsaved changes, then the zoom and color mode.
    let window = |name: &str| {
        name.strip_prefix(title.as_str()).map(|rest| rest.trim_start_matches('*')).is_some_and(|rest| rest.is_empty() || rest.starts_with(' '))
    };
    named(ctx, egui::accesskit::Role::Group, window).unwrap_or_else(|| panic!("document {i}'s window, {title}, was drawn"))
}

/// Where the last frame drew the tab of the window `window` at the top of its body (the title text
/// drawn last inside it, below the main strip).
fn window_tab_at(shapes: &[Shape], title: &str, window: egui::Rect) -> Pos2 {
    shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Text(t) if t.pos.y >= 35.0 && window.contains(t.pos) && t.galley.text().contains(title) => Some(t.pos + vec2(4.0, 4.0)),
            _ => None,
        })
        .last()
        .unwrap()
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
fn float_in_window_moves_the_current_document_out_of_the_tabs() {
    let mut app = app_with(3);
    let ctx = crate::testing::ui_context();
    let shapes = frame(&mut app, &ctx, vec![]);
    let title = app.session.documents()[2].title();
    assert!(tab_drawn(&shapes, &title), "the current document has a tab");
    let tabbed = canvas_rect(&app);
    assert_eq!(app.run("window.arrange.floatInWindow", json!({})).unwrap(), json!({"floating": [2]}));
    frame(&mut app, &ctx, vec![]);
    let shapes = frame(&mut app, &ctx, vec![]);
    assert!(!tab_drawn(&shapes, &title), "a floating document has no tab");
    assert!(tab_drawn(&shapes, &app.session.documents()[0].title()));
    assert_eq!(docked(&app), [0, 1]);
    assert_eq!(app.session.active_index(), Some(2), "the floating document stays the current one");
    // Its window holds the canvas, fitted to the window; the main window shows a tabbed document.
    let body = body_of(&app, 2);
    let canvas = canvas_rect(&app);
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
    let ctx = crate::testing::ui_context();
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
    assert!(body_of(&app, 0).contains_rect(canvas_rect(&app)), "its window holds the canvas now");
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
    let ctx = crate::testing::ui_context();
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
    let ctx = crate::testing::ui_context();
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
    assert!(places.iter().all(|p| area.contains_rect(*p)), "every window is inside the document area: {places:?}");
    for pair in places.windows(2) {
        assert!(pair[1].min.x > pair[0].min.x && pair[1].min.y > pair[0].min.y, "each window is below and right of the one before: {places:?}");
    }
    let order: Vec<usize> = app.ui.floating.iter().map(|w| app.session.documents().iter().position(|d| d.uid == w.uid).unwrap()).collect();
    assert_eq!(order, [0, 1, 2], "the windows stack in tab order, document 1's moved from where it floated");
    for (i, place) in places.iter().copied().enumerate() {
        let body = body_of(&app, i);
        assert!((body.left() - place.left()).abs() < 4.0 && body.top() > place.top(), "document {i}'s window is at its place: {body:?} {place:?}");
        assert!(app.views[i].fitted, "document {i} fits its window");
    }
    assert!(body_of(&app, 1).contains_rect(canvas_rect(&app)), "the current document's window holds the canvas");
    // The current window is in front: a press where it overlaps document 0's window reaches it.
    let overlap = places[1].min + vec2(60.0, 10.0);
    app.run("document.activate", json!({"index": 0})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.run("document.activate", json!({"index": 1})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, overlap);
    assert_eq!(app.session.active_index(), Some(1), "the current window stays in front of the one it overlaps");
}

#[test]
fn float_all_in_windows_floats_the_tabbed_documents_and_consolidate_brings_them_back() {
    let mut app = app_with(3);
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let tabbed = canvas_rect(&app);
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
    assert_eq!(canvas_rect(&app), tabbed, "the canvas is under the tabs again");
    assert!(app.views[2].fitted);
}

#[test]
fn tile_brings_floating_documents_back_into_the_grid() {
    let mut app = app_with(3);
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.tile", json!({})).unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let docs: Vec<usize> = app.canvas.panes.last.iter().map(|(w, _)| w.0).collect();
    assert_eq!(docs, [0, 1], "a floating document leaves the grid; the tabbed ones share it");
    assert_eq!(app.run("window.arrange.tile", json!({})).unwrap(), json!({"tiled": true, "panes": 3, "layout": "grid"}));
    frame(&mut app, &ctx, vec![]);
    let docs: Vec<usize> = app.canvas.panes.last.iter().map(|(w, _)| w.0).collect();
    assert_eq!(docs, [0, 1, 2]);
    assert!(app.ui.floating.is_empty());
}

#[test]
fn closing_a_floating_window_closes_its_document_and_asks_to_save_an_unsaved_one() {
    let mut app = app_with(2);
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    // Document 0's window is behind document 1's: its close button is at the top right of its
    // title bar, clear of document 1's window to the right and below.
    let close_button = |app: &VectorcraftApp, i: usize| (window_of(app, &ctx, i), body_of(app, i));
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
            if app.any_dialog_open() || app.session.documents().len() < n {
                return true;
            }
        }
        false
    };
    assert!(press_close(&mut app, 1), "the close button of document 1's window acts");
    assert!(app.any_dialog_open(), "an unsaved document asks to be saved");
    assert_eq!(app.session.documents().len(), 2);
    // While the prompt is open the windows take no press.
    let body = body_of(&app, 0);
    click(&mut app, &ctx, pos2(body.left() + 12.0, body.bottom() - 12.0));
    assert_eq!(app.session.active_index(), Some(1), "a press in another window waits for the prompt");
    // The prompt goes unanswered: document 1 stays open.
    app.dialogs().unwrap().with(|h| {
        for d in h.open_dialogs() {
            h.close_by_id(d.id);
        }
    });
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
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let body = body_of(&app, 1);
    assert!(body.contains_rect(canvas_rect(&app)), "the current document's window holds the canvas");
    let windowed = (app.views[0].zoom, app.views[1].zoom);
    app.run("view.presentation", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let shown = canvas_rect(&app);
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
    assert!(body_of(&app, 1).contains_rect(canvas_rect(&app)), "the window holds the canvas again");
    assert_eq!(app.ui.floating.len(), 2, "the windows come back");
    for (i, before) in [windowed.0, windowed.1].into_iter().enumerate() {
        assert!((app.views[i].zoom - before).abs() < 1e-6, "document {i} fits its window again: {before} → {}", app.views[i].zoom);
    }
}

#[test]
fn a_document_made_current_in_presentation_mode_comes_to_the_front_after_it() {
    let mut app = app_with(2);
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    // Where the two windows overlap: document 1's title bar, over document 0's body.
    let overlap = app.ui.floating[1].place.min + vec2(60.0, 10.0);
    app.run("view.presentation", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.run("document.activate", json!({"index": 0})).unwrap();
    frame(&mut app, &ctx, vec![]);
    app.run("view.presentation", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    click(&mut app, &ctx, overlap);
    assert_eq!(app.session.active_index(), Some(0), "document 0's window is in front where they overlap");
}

/// A stand-in for the window system of the documents' OS windows: it runs each one's frame and asks
/// each to close while `close` is set. Each
/// window is 500 × 400 points with its top-left corner at `origin` on the screen, and its next
/// frame takes `events`.
#[derive(Default)]
struct Host {
    close: bool,
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
    });
    host
}

#[test]
fn closing_an_os_window_keeps_it_while_its_unsaved_document_stays_open() {
    let host = native_host();
    let ctx = crate::testing::ui_context();
    ctx.set_embed_viewports(false);
    let mut app = app_with(2);
    if !WindowHost::of(&ctx).viewports_available() {
        // Wayland: the windows are inside the main window.
        return;
    }
    frame(&mut app, &ctx, vec![]);
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    host.borrow_mut().close = true;
    frame(&mut app, &ctx, vec![]);
    host.borrow_mut().close = false;
    assert!(app.any_dialog_open(), "the unsaved document asks to be saved");
    assert_eq!(app.session.documents().len(), 2);
    assert!(is_floating(&app, 1), "the document still floats");
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.ui.floating.len(), 1, "its window is still shown");
}

/// The update prompt, opening on its own, takes the keyboard once from a document's OS window that
/// has it, and leaves the keyboard alone when none of those windows has it.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn the_update_prompt_takes_the_keyboard_from_a_document_window() {
    let _host = native_host();
    let ctx = crate::testing::ui_context();
    ctx.set_embed_viewports(false);
    let mut app = app_with(2);
    if !WindowHost::of(&ctx).viewports_available() {
        // Wayland: the windows are inside the main window.
        return;
    }
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    let focus_commands = |app: &mut VectorcraftApp| {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::updates_ui::pump(app, ui.ctx()));
        out.textures_delta.clear();
        out.viewport_output.get(&egui::ViewportId::ROOT).map_or(0, |v| v.commands.iter().filter(|c| **c == egui::ViewportCommand::Focus).count())
    };

    app.ui.floating[0].focused = true;
    assert_eq!(focus_commands(&mut app), 0, "no prompt is open");
    crate::updates_ui::open_prompt_for_test(&mut app);
    assert_eq!(focus_commands(&mut app), 1, "the main window gets the keyboard");
    assert_eq!(focus_commands(&mut app), 0, "once");

    crate::updates_ui::close_dialogs_for_test(&mut app);
    assert_eq!(focus_commands(&mut app), 0, "the prompt closed");
    app.ui.floating[0].focused = false;
    crate::updates_ui::open_prompt_for_test(&mut app);
    assert_eq!(focus_commands(&mut app), 0, "no document window has the keyboard");
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
fn dragging_a_tab_off_the_strip_floats_its_document_at_the_pointer() {
    let mut app = app_with(3);
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    let shapes = frame(&mut app, &ctx, vec![]);
    let title = app.session.documents()[0].title();
    let tab = tab_at(&shapes, &title);
    // A short drag, and a drag along the strip short of the next tab's middle, leave the tabs as
    // they are.
    // A drag that stays within the strip's height is no tear-off.
    drag(&mut app, &ctx, tab, pos2(tab.x, strip(&ctx).bottom()));
    drag(&mut app, &ctx, tab, tab + vec2(40.0, 0.0));
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
    assert!(body_of(&app, 0).contains_rect(canvas_rect(&app)), "its window holds the canvas");
}

#[test]
fn a_window_dragged_by_its_tab_onto_the_strip_goes_back_under_the_tabs() {
    let mut app = app_with(3);
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    frame(&mut app, &ctx, vec![]);
    let tab_of = |app: &mut VectorcraftApp, i: usize| {
        let shapes = frame(app, &ctx, vec![]);
        window_tab_at(&shapes, &app.session.documents()[i].title(), window_of(app, &ctx, i))
    };
    // Dropped on the strip, document 2's tab goes back under its tab, current; the other windows stay.
    let from = tab_of(&mut app, 2);
    drag(&mut app, &ctx, from, pos2(600.0, 15.0));
    assert_eq!(docked(&app), [2]);
    assert_eq!(app.session.active_index(), Some(2));
    assert_eq!(app.ui.floating.len(), 2);
    frame(&mut app, &ctx, vec![]);
    let shapes = frame(&mut app, &ctx, vec![]);
    assert!(tab_drawn(&shapes, &app.session.documents()[2].title()), "it has its tab again");
    assert!(app.views[2].fitted, "it fits the canvas under the tabs");
    let canvas = canvas_rect(&app);
    assert!(canvas.top() >= strip(&ctx).bottom() && canvas.width() > 1000.0, "the canvas under the tabs shows it: {canvas:?}");
    // Torn off again, it opens where its tab is dropped, not where its last window was.
    let to = pos2(200.0, 300.0);
    let tab = tab_at(&shapes, &app.session.documents()[2].title());
    drag(&mut app, &ctx, tab, to);
    frame(&mut app, &ctx, vec![]);
    let window = window_of(&app, &ctx, 2);
    assert!((window.min - (to - GRAB)).length() < 1.0, "its new window is under the pointer: {window:?}");
}

#[test]
fn after_touch_input_a_window_tab_dragged_onto_the_strip_still_docks() {
    let mut app = app_with(2);
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    app.run("window.arrange.floatAllInWindows", json!({})).unwrap();
    frame(&mut app, &ctx, vec![]);
    // A touch first: the tab drop docks all the same.
    let touch = egui::Event::Touch {
        device_id: egui::TouchDeviceId(1),
        id: egui::TouchId(1),
        phase: egui::TouchPhase::Start,
        pos: pos2(1100.0, 700.0),
        force: None,
    };
    frame(&mut app, &ctx, vec![touch]);
    frame(&mut app, &ctx, vec![]);
    let shapes = frame(&mut app, &ctx, vec![]);
    let from = window_tab_at(&shapes, &app.session.documents()[1].title(), window_of(&app, &ctx, 1));
    drag(&mut app, &ctx, from, pos2(600.0, 15.0));
    assert_eq!(docked(&app), [1], "document 1 is back under the tabs");
}

#[test]
fn a_document_floated_again_after_cascade_opens_where_it_is_asked_to() {
    let mut app = app_with(2);
    let ctx = crate::testing::ui_context();
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
    let ctx = crate::testing::ui_context();
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
    assert!(body_of(&app, 2).contains_rect(canvas_rect(&app)), "the new document's window holds the canvas");
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
fn an_os_windows_tab_docks_on_the_strip() {
    let host = native_host();
    let ctx = crate::testing::ui_context();
    ctx.set_embed_viewports(false);
    let mut app = app_with(2);
    if !WindowHost::of(&ctx).viewports_available() {
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
    // The tab at the top of the window.
    let tab = pos2(20.0, 15.0);
    // Released over the main window's tab strip, it docks.
    let over_strip = strip(&ctx).center().y - 300.0;
    os_drag(&mut app, tab, pos2(20.0, over_strip));
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
    let ctx = crate::testing::ui_context();
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
fn closing_a_view_leaves_the_document_open() {
    let mut app = crate::testing::windowed(app_with(1));
    let ctx = app.ui_ctx.clone().expect("a window");
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    app.run("window.newWindow", json!({})).unwrap();
    app.run("window.newWindow", json!({})).unwrap();
    show_window(&mut app, (0, 2));
    assert_eq!(app.run("file.close", json!({})).unwrap(), json!({"index": 0, "views": 2}));
    assert!(!app.any_dialog_open(), "closing a view does not ask to save");
    assert_eq!((app.session.documents().len(), windows(&app)), (1, vec![(0, 1), (0, 3)]));
    assert_eq!(current(&app), Some((0, 3)), "the next view becomes the current window");
    frame(&mut app, &ctx, vec![]);
    assert_eq!(app.run("file.close", json!({})).unwrap(), json!({"index": 0, "views": 1}));
    assert_eq!(windows(&app), [(0, 1)], "the view left is the document's only view again");
    let name = app.session.documents()[0].title();
    assert!(!title_of(&app, (0, 1)).contains(&format!("{name}:")), "{}", title_of(&app, (0, 1)));
    // The last view: the document closes, asking to save it first.
    app.run("file.close", json!({})).unwrap();
    assert!(app.any_dialog_open(), "the changed document asks to be saved");
    assert_eq!(app.session.documents().len(), 1);
}

#[test]
fn arrange_treats_each_view_as_a_window() {
    let mut app = app_with(1);
    let ctx = crate::testing::ui_context();
    frame(&mut app, &ctx, vec![]);
    assert!(!enabled(&app, "window.arrange.tile") && !enabled(&app, "window.arrange.cascade"), "one window");
    app.run("window.newWindow", json!({})).unwrap();
    assert!(enabled(&app, "window.arrange.tile") && enabled(&app, "window.arrange.cascade"), "two views of one document");
    assert_eq!(app.run("window.arrange.tile", json!({})).unwrap(), json!({"tiled": true, "panes": 2, "layout": "grid"}));
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

/// One headless frame as the app runs it: a Selection move released off the canvas ends first
/// ([`crate::canvas::release_over_panel`]), then the tabs and the canvas or panes, then the floating
/// document windows. → the cursor.
fn app_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<egui::Event>) -> egui::CursorIcon {
    run_frame(app, ctx, events).platform_output.cursor_icon
}

/// Two 400 × 300 pt documents on `ctx`: the first, current under its tab, holds a 50 pt square at
/// 300, 230, and the second, in a floating window, a 50 pt square at 50, 50. The Selection tool is
/// chosen.
fn floating_pair(ctx: &egui::Context) -> VectorcraftApp {
    let mut app = app_with(0);
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 300, "y": 230, "width": 50, "height": 50})).unwrap();
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 50, "height": 50})).unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.select_tool("selection");
    app.ui.task_bar = false;
    app_frame(&mut app, ctx, vec![]);
    app_frame(&mut app, ctx, vec![]);
    app
}

/// Press on document point `at` on the canvas, clear of document 1's window when it floats, and
/// move the pointer 10 × 5 points: the Selection tool moves the art under it.
fn grab(app: &mut VectorcraftApp, ctx: &egui::Context, at: vectorcraft_geom::Point) {
    let p = app.canvas_view().unwrap().to_screen(at);
    if app.session.documents().len() > 1 && app.session.active_index() != Some(1) && is_floating(app, 1) {
        assert!(!window_of(app, ctx, 1).contains(p), "the art is clear of the floating window");
    }
    app_frame(app, ctx, vec![egui::Event::PointerMoved(p), button(p, true)]);
    app_frame(app, ctx, vec![egui::Event::PointerMoved(p + vec2(10.0, 5.0))]);
    assert!(app.session.in_interaction(), "the Selection tool moves the art");
}

/// The screen point that shows document point `at` in window `w`'s pane, through the view the pane
/// last drew with.
fn in_noted(app: &VectorcraftApp, w: Win, at: vectorcraft_geom::Point) -> Pos2 {
    let v = view(app, w).unwrap();
    v.canvas(1.0, v.rect).to_screen(at)
}

/// How many objects document `i`'s first layer holds.
fn objects(app: &VectorcraftApp, i: usize) -> usize {
    app.session.documents()[i].doc.layers[0].children().map_or(0, Vec::len)
}

/// The center of the one object selected in the current document.
fn selected_center(app: &VectorcraftApp) -> vectorcraft_geom::Point {
    let st = app.session.active().unwrap();
    assert_eq!(st.selection.objects.len(), 1, "one object is selected");
    st.doc.node(st.selection.objects[0]).unwrap().geometric_bounds().unwrap().center()
}

#[test]
fn a_selection_drag_released_in_another_documents_floating_window_copies_the_art_there() {
    let ctx = crate::testing::ui_context();
    let mut app = floating_pair(&ctx);
    // The target's view differs from the source's: the grab point still lands under the pointer.
    let v = view_mut(&mut app, (1, 1)).unwrap();
    v.zoom *= 0.6;
    v.rotation = 30.0;
    app_frame(&mut app, &ctx, vec![]);
    let source = app.session.documents()[0].doc.clone();
    let source_undo = app.session.documents()[0].history.undo.len();
    let target = app.session.documents()[1].doc.clone();
    let target_undo = app.session.documents()[1].history.undo.len();
    let clipboard = app.session.clipboard.clone();
    grab(&mut app, &ctx, vectorcraft_geom::Point::new(325.0, 255.0));
    let drop = in_noted(&app, (1, 1), vectorcraft_geom::Point::new(170.0, 120.0));
    let cursor = app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(drop)]);
    assert_eq!(cursor, egui::CursorIcon::Copy, "over another document's window the art is copied");
    assert_eq!(app.session.active_index(), Some(0), "holding the art over the window does not switch documents");
    app_frame(&mut app, &ctx, vec![button(drop, false)]);
    app_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.active_index(), Some(1), "the window's document is the current one");
    assert_eq!(objects(&app, 1), 2, "the copy joins the target's square");
    let placed = selected_center(&app);
    assert!((placed.x - 170.0).abs() < 0.01 && (placed.y - 120.0).abs() < 0.01, "{placed:?}");
    assert_eq!(app.session.documents()[1].history.undo.len(), target_undo + 1, "one step to undo");
    assert_eq!(app.session.documents()[0].doc, source, "the source keeps its art where it was");
    assert_eq!(app.session.documents()[0].history.undo.len(), source_undo);
    assert_eq!(app.session.clipboard, clipboard, "the drag leaves the clipboard alone");
    assert!(!app.session.tool_busy());
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.documents()[1].doc, target);
}

#[test]
fn a_release_in_the_first_frame_over_a_floating_window_still_copies_the_art() {
    let ctx = crate::testing::ui_context();
    let mut app = floating_pair(&ctx);
    let source = app.session.documents()[0].doc.clone();
    grab(&mut app, &ctx, vectorcraft_geom::Point::new(325.0, 255.0));
    let drop = in_noted(&app, (1, 1), vectorcraft_geom::Point::new(200.0, 150.0));
    // The canvas has not handed the move over yet: the pane takes it before the canvas sees the
    // release.
    app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(drop), button(drop, false)]);
    app_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.active_index(), Some(1));
    assert_eq!(objects(&app, 1), 2);
    let placed = selected_center(&app);
    assert!((placed.x - 200.0).abs() < 0.01 && (placed.y - 150.0).abs() < 0.01, "{placed:?}");
    assert_eq!(app.session.documents()[0].doc, source);
}

#[test]
fn a_floating_window_drop_lands_where_the_button_went_up() {
    let ctx = crate::testing::ui_context();
    let mut app = floating_pair(&ctx);
    grab(&mut app, &ctx, vectorcraft_geom::Point::new(325.0, 255.0));
    let drop = in_noted(&app, (1, 1), vectorcraft_geom::Point::new(200.0, 150.0));
    let later = in_noted(&app, (1, 1), vectorcraft_geom::Point::new(120.0, 220.0));
    app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(drop)]);
    // A pointer move later in the frame of the release does not move the drop.
    app_frame(&mut app, &ctx, vec![button(drop, false), egui::Event::PointerMoved(later)]);
    app_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.active_index(), Some(1));
    assert_eq!(objects(&app, 1), 2);
    let placed = selected_center(&app);
    assert!((placed.x - 200.0).abs() < 0.01 && (placed.y - 150.0).abs() < 0.01, "{placed:?}");
}

#[test]
fn art_dragged_from_a_floating_window_drops_into_the_main_windows_document() {
    let ctx = crate::testing::ui_context();
    let mut app = floating_pair(&ctx);
    show_window(&mut app, (1, 1));
    app_frame(&mut app, &ctx, vec![]);
    app_frame(&mut app, &ctx, vec![]);
    assert!(current_floats(&app), "the floating window holds the canvas");
    let target = app.session.documents()[1].doc.clone();
    grab(&mut app, &ctx, vectorcraft_geom::Point::new(75.0, 75.0));
    // The main window draws document 0 as a pane; this point is clear of the floating window.
    let at = vectorcraft_geom::Point::new(360.0, 40.0);
    let drop = in_noted(&app, (0, 1), at);
    assert!(!window_of(&app, &ctx, 1).contains(drop), "{drop:?}");
    let cursor = app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(drop)]);
    assert_eq!(cursor, egui::CursorIcon::Copy);
    app_frame(&mut app, &ctx, vec![button(drop, false)]);
    app_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.active_index(), Some(0), "the main window's document is the current one");
    assert_eq!(objects(&app, 0), 2);
    let placed = selected_center(&app);
    assert!((placed.x - at.x).abs() < 0.01 && (placed.y - at.y).abs() < 0.01, "{placed:?}");
    assert_eq!(app.session.documents()[1].doc, target, "the source in the floating window is untouched");
}

#[test]
fn a_selection_drag_into_a_floating_view_of_its_document_moves_the_art() {
    let ctx = crate::testing::ui_context();
    let mut app = app_with(0);
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.run("shape.rectangle", json!({"x": 300, "y": 230, "width": 50, "height": 50})).unwrap();
    app.run("window.newWindow", json!({})).unwrap();
    app.run("window.arrange.floatInWindow", json!({})).unwrap();
    show_window(&mut app, (0, 1));
    app.select_tool("selection");
    app.ui.task_bar = false;
    app_frame(&mut app, &ctx, vec![]);
    app_frame(&mut app, &ctx, vec![]);
    assert!(floats(&app, (0, 2)) && current(&app) == Some((0, 1)));
    let undo = app.session.documents()[0].history.undo.len();
    grab(&mut app, &ctx, vectorcraft_geom::Point::new(325.0, 255.0));
    let drop = in_noted(&app, (0, 2), vectorcraft_geom::Point::new(150.0, 100.0));
    let cursor = app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(drop)]);
    assert_eq!(cursor, egui::CursorIcon::Move, "another view of the same document moves the art");
    app_frame(&mut app, &ctx, vec![button(drop, false)]);
    app_frame(&mut app, &ctx, vec![]);
    assert_eq!(current(&app), Some((0, 2)), "the floating view is the current window");
    assert_eq!(objects(&app, 0), 1, "the square moved, not copied");
    let moved = selected_center(&app);
    assert!((moved.x - 150.0).abs() < 0.01 && (moved.y - 100.0).abs() < 0.01, "{moved:?}");
    assert_eq!(app.session.documents()[0].history.undo.len(), undo + 1);
}

#[test]
fn a_drag_released_on_a_floating_windows_title_bar_or_after_escape_changes_nothing() {
    for (case, escape) in [("title bar", false), ("art area after Escape", true)] {
        let ctx = crate::testing::ui_context();
        let mut app = floating_pair(&ctx);
        let docs: Vec<_> = app.session.documents().iter().map(|d| (d.doc.clone(), d.history.undo.len())).collect();
        grab(&mut app, &ctx, vectorcraft_geom::Point::new(325.0, 255.0));
        let (window, body) = (window_of(&app, &ctx, 1), body_of(&app, 1));
        let at = if escape {
            in_noted(&app, (1, 1), vectorcraft_geom::Point::new(200.0, 150.0))
        } else {
            pos2(window.left() + 60.0, (window.top() + body.top()) / 2.0)
        };
        app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(at)]);
        if escape {
            let key = egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
            app_frame(&mut app, &ctx, vec![key]);
        }
        app_frame(&mut app, &ctx, vec![button(at, false)]);
        app_frame(&mut app, &ctx, vec![]);
        assert_eq!(app.session.active_index(), Some(0), "{case}: the source stays current");
        let after: Vec<_> = app.session.documents().iter().map(|d| (d.doc.clone(), d.history.undo.len())).collect();
        assert!(after == docs, "{case}: neither document changes");
        assert!(!app.session.in_interaction(), "{case}: the move ended");
    }
}

#[test]
fn a_tab_transfer_whose_source_changed_drops_nothing_in_a_floating_window() {
    let ctx = crate::testing::ui_context();
    let mut app = floating_pair(&ctx);
    app.run("file.new", json!({"width": 400, "height": 300, "name": "Third"})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.select_tool("selection");
    app_frame(&mut app, &ctx, vec![]);
    let tab = tab_at(&frame(&mut app, &ctx, vec![]), "Third");
    grab(&mut app, &ctx, vectorcraft_geom::Point::new(325.0, 255.0));
    // Held over the third document's tab, the drag becomes a transfer to that document.
    app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(tab)]);
    for _ in 0..60 {
        if app.session.active_index() == Some(2) {
            break;
        }
        app_frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(app.session.active_index(), Some(2), "the tab hover switched documents");
    let drop = in_noted(&app, (1, 1), vectorcraft_geom::Point::new(200.0, 150.0));
    app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(drop)]);
    // The source changes between frames, as a command from the control channel would change it.
    app.session.set_active(0);
    app.session.execute("shape.ellipse", &json!({"x": 20, "y": 20, "width": 30, "height": 30})).unwrap();
    app.session.set_active(2);
    let target = app.session.documents()[1].doc.clone();
    let target_undo = app.session.documents()[1].history.undo.len();
    app_frame(&mut app, &ctx, vec![button(drop, false)]);
    app_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.documents()[1].doc, target, "the floating window's document does not take the changed art");
    assert_eq!(app.session.documents()[1].history.undo.len(), target_undo);
    assert_eq!(app.session.active_index(), Some(2), "the window does not become current");
}

#[test]
fn a_tab_transfer_whose_target_is_no_longer_current_drops_nothing_in_a_floating_window() {
    let ctx = crate::testing::ui_context();
    let mut app = floating_pair(&ctx);
    app.run("file.new", json!({"width": 400, "height": 300, "name": "Third"})).unwrap();
    app.run("document.activate", json!({"index": 0})).unwrap();
    app.select_tool("selection");
    app_frame(&mut app, &ctx, vec![]);
    let tab = tab_at(&frame(&mut app, &ctx, vec![]), "Third");
    grab(&mut app, &ctx, vectorcraft_geom::Point::new(325.0, 255.0));
    app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(tab)]);
    for _ in 0..60 {
        if app.session.active_index() == Some(2) {
            break;
        }
        app_frame(&mut app, &ctx, vec![]);
    }
    assert_eq!(app.session.active_index(), Some(2), "the tab hover switched documents");
    let drop = in_noted(&app, (1, 1), vectorcraft_geom::Point::new(200.0, 150.0));
    app_frame(&mut app, &ctx, vec![egui::Event::PointerMoved(drop)]);
    // Another document becomes current between frames, as a command from the control channel
    // would make it.
    app.session.set_active(0);
    let target = app.session.documents()[1].doc.clone();
    let target_undo = app.session.documents()[1].history.undo.len();
    app_frame(&mut app, &ctx, vec![button(drop, false)]);
    app_frame(&mut app, &ctx, vec![]);
    assert_eq!(app.session.documents()[1].doc, target, "the floating window's document does not take the art");
    assert_eq!(app.session.documents()[1].history.undo.len(), target_undo);
    assert_eq!(app.session.active_index(), Some(0), "the window does not become current");
}
