//! Floating panels in OS windows of their own: the whole app's frames, with a stand-in for the
//! window system that runs each native window's frame with its own input.
// Clippy's test exemption doesn't cover a `cfg(all(test, …))` module's helpers.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use egui::{Event, Key, Pos2, Rect, ViewportCommand, ViewportId, pos2, vec2};
use vectorcraft_engine::Session;

use super::*;

const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, pos2(1600.0, 1000.0));
/// Where the main window's content sits on the desktop.
const ORIGIN: Pos2 = pos2(100.0, 50.0);

struct Window {
    /// Desktop rect of its content.
    rect: Rect,
    events: Vec<Event>,
    /// The OS files held over it, as the OS sends them before a drop.
    hovered: Vec<egui::HoveredFile>,
    dropped: Vec<egui::DroppedFileHandle>,
    frames: usize,
    /// The texts its last frame drew, with their rects in the window.
    texts: Vec<(String, Rect)>,
    /// The named nodes of its last frame's access tree, with their rects in the window.
    nodes: Vec<(String, Rect)>,
}

struct Host {
    windows: HashMap<ViewportId, Window>,
    /// The window with the keyboard.
    focus: ViewportId,
    time: f64,
}

impl Host {
    fn infos(&self) -> egui::ViewportIdMap<egui::ViewportInfo> {
        let info = |id: ViewportId, rect: Rect| egui::ViewportInfo {
            inner_rect: Some(rect),
            outer_rect: Some(rect),
            focused: Some(self.focus == id),
            native_pixels_per_point: Some(1.0),
            ..Default::default()
        };
        std::iter::once((ViewportId::ROOT, info(ViewportId::ROOT, SCREEN.translate(ORIGIN.to_vec2()))))
            .chain(self.windows.iter().map(|(id, w)| (*id, info(*id, w.rect))))
            .collect()
    }

    fn apply(&mut self, full: &egui::FullOutput) {
        for (id, output) in &full.viewport_output {
            for command in &output.commands {
                let Some(window) = self.windows.get_mut(id) else { continue };
                match command {
                    ViewportCommand::OuterPosition(pos) => window.rect = Rect::from_min_size(*pos, window.rect.size()),
                    ViewportCommand::InnerSize(size) => window.rect = Rect::from_min_size(window.rect.min, *size),
                    _ => {}
                }
            }
        }
    }

    /// One native window's frame.
    fn render(host: &RefCell<Self>, ctx: &egui::Context, mut viewport: egui::ImmediateViewport<'_>) {
        let id = viewport.ids.this;
        let input = {
            let mut host = host.borrow_mut();
            let builder = &viewport.builder;
            let rect = Rect::from_min_size(builder.position.unwrap_or_default(), builder.inner_size.unwrap_or(vec2(300.0, 200.0)));
            host.windows.entry(id).or_insert_with(|| Window {
                rect,
                events: Vec::new(),
                hovered: Vec::new(),
                dropped: Vec::new(),
                frames: 0,
                texts: Vec::new(),
                nodes: Vec::new(),
            });
            let viewports = host.infos();
            let (time, focused) = (host.time, host.focus == id);
            let Some(window) = host.windows.get_mut(&id) else { return };
            window.frames += 1;
            egui::RawInput {
                viewport_id: id,
                viewports,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, window.rect.size())),
                events: std::mem::take(&mut window.events),
                hovered_files: std::mem::take(&mut window.hovered),
                dropped_files: std::mem::take(&mut window.dropped),
                focused,
                time: Some(time),
                ..Default::default()
            }
        };
        let mut full = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
        full.textures_delta.clear();
        fn texts(s: &egui::Shape, out: &mut Vec<(String, Rect)>) {
            match s {
                egui::Shape::Text(t) => out.push((t.galley.text().to_string(), t.visual_bounding_rect())),
                egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
                _ => {}
            }
        }
        let mut shown = Vec::new();
        full.shapes.iter().for_each(|c| texts(&c.shape, &mut shown));
        let nodes = full.platform_output.accesskit_update.as_ref().map_or_else(Vec::new, |update| {
            let rect = |b: egui::accesskit::Rect| Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32));
            update.nodes.iter().filter_map(|(_, node)| Some((node.label()?.to_string(), rect(node.bounds()?)))).collect()
        });
        let mut host = host.borrow_mut();
        if let Some(window) = host.windows.get_mut(&id) {
            window.texts = shown;
            if !nodes.is_empty() {
                window.nodes = nodes;
            }
        }
        host.apply(&full);
    }
}

struct Harness {
    app: VectorcraftApp,
    ctx: egui::Context,
    host: Rc<RefCell<Host>>,
}

impl Harness {
    fn new() -> Self {
        let host = Rc::new(RefCell::new(Host { windows: HashMap::new(), focus: ViewportId::ROOT, time: 0.0 }));
        let shared = host.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| Host::render(&shared, ctx, viewport));
        let ctx = crate::testing::ui_context();
        ctx.set_embed_viewports(false);
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let mut h = Self { app, ctx, host };
        h.frames(3);
        h
    }

    fn frame(&mut self, events: Vec<Event>) {
        let (viewports, focused, time) = {
            let mut host = self.host.borrow_mut();
            host.time += 0.05;
            (host.infos(), host.focus == ViewportId::ROOT, host.time)
        };
        let input = egui::RawInput { time: Some(time), events, screen_rect: Some(SCREEN), viewports, focused, ..Default::default() };
        let full = crate::testing::app_frame(&mut self.app, &self.ctx, input);
        self.host.borrow_mut().apply(&full);
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.frame(vec![]);
        }
    }

    /// The viewport of the floating window holding `panel`.
    fn viewport(&self, panel: &str) -> ViewportId {
        window_viewport(&self.app, panel).expect("the panel floats in a window of its own")
    }

    /// Float `panel` into a native window and give that window the keyboard.
    fn float(&mut self, panel: &str) -> ViewportId {
        self.app.run("window.panel.float", json!({"panel": panel, "x": 200, "y": 200})).unwrap();
        self.frames(3);
        let viewport = self.viewport(panel);
        assert!(self.host.borrow().windows.get(&viewport).is_some_and(|w| w.frames > 0), "{panel} opened an OS window");
        self.host.borrow_mut().focus = viewport;
        viewport
    }

    /// Press and release `key` in the window `viewport`, with the text a letter types, then let
    /// the main window run.
    fn key_in(&mut self, viewport: ViewportId, key: Key) {
        let letter = key.name().len() == 1 && key.name().chars().all(|c| c.is_ascii_alphabetic());
        for pressed in [true, false] {
            let mut events = vec![Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Default::default() }];
            if pressed && letter {
                events.push(Event::Text(key.name().to_lowercase()));
            }
            if let Some(window) = self.host.borrow_mut().windows.get_mut(&viewport) {
                window.events.extend(events);
            }
            self.frame(vec![]);
        }
        self.frames(2);
    }

    fn objects(&self) -> usize {
        self.app.session.active().map_or(0, |d| d.doc.selectable_art().len())
    }
}

#[derive(Debug)]
struct MockDrop(Vec<u8>);

impl egui::DroppedFile for MockDrop {
    fn path(&self) -> &std::path::Path {
        std::path::Path::new("")
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        Ok(self.0.clone())
    }
}

/// `window.panel.float` with a rect moves an OS window the panel already has, in each of the modes
/// (a panel, its group, its column); a missing `x`/`y` or size keeps the window's own.
#[test]
fn an_explicit_float_command_updates_an_existing_native_windows_geometry() {
    for mode in [json!({}), json!({"group": true}), json!({"column": true})] {
        let mut h = Harness::new();
        h.app.run("window.panel.float", json!({"panel": "swatches", "x": 240, "y": 160, "width": 320, "height": 360})).unwrap();
        h.frames(3);
        let viewport = h.viewport("swatches");
        assert_eq!(h.host.borrow().windows[&viewport].rect, Rect::from_min_size(ORIGIN + vec2(240.0, 160.0), vec2(320.0, 360.0)));
        let mut params = json!({"panel": "swatches", "x": 280, "y": 190, "width": 350, "height": 390});
        params.as_object_mut().unwrap().extend(mode.as_object().unwrap().clone());
        h.app.run("window.panel.float", params).unwrap();
        h.frames(3);
        assert_eq!(h.viewport("swatches"), viewport);
        assert_eq!(h.host.borrow().windows[&viewport].rect, Rect::from_min_size(ORIGIN + vec2(280.0, 190.0), vec2(350.0, 390.0)));
        let moved = Rect::from_min_size(pos2(-500.0, 120.0), vec2(380.0, 410.0));
        h.host.borrow_mut().windows.get_mut(&viewport).unwrap().rect = moved;
        h.frames(3);
        let mut params = json!({"panel": "swatches", "width": 400});
        params.as_object_mut().unwrap().extend(mode.as_object().unwrap().clone());
        h.app.run("window.panel.float", params).unwrap();
        h.frames(3);
        let resized = Rect::from_min_size(moved.min, vec2(400.0, moved.height()));
        assert_eq!(h.host.borrow().windows[&viewport].rect, resized);
        let mut params = json!({"panel": "swatches"});
        params.as_object_mut().unwrap().extend(mode.as_object().unwrap().clone());
        h.app.run("window.panel.float", params).unwrap();
        h.frames(3);
        assert_eq!(h.host.borrow().windows[&viewport].rect, resized);
    }
}

/// Swatches and Color as tabs of one native window, with its OS window drawn.
fn native_tab_group(h: &mut Harness) -> ViewportId {
    let viewport = h.float("swatches");
    h.app.run("window.panel.float", json!({"panel": "color", "onto": "swatches"})).unwrap();
    h.frames(3);
    assert_eq!(h.viewport("color"), viewport);
    viewport
}

#[test]
fn a_group_float_of_a_tab_group_resizes_its_native_window() {
    let mut h = Harness::new();
    let viewport = native_tab_group(&mut h);
    let id = floating_window(&layout_of(&h.app.ui), "swatches").unwrap().id;
    let before = h.host.borrow().windows[&viewport].rect;
    h.app.run("window.panel.float", json!({"panel": "swatches", "group": true, "width": 400})).unwrap();
    h.frames(3);
    assert_eq!(floating_window(&layout_of(&h.app.ui), "swatches").unwrap().id, id);
    assert_eq!(h.viewport("color"), viewport);
    assert_eq!(h.host.borrow().windows[&viewport].rect, Rect::from_min_size(before.min, vec2(400.0, before.height())));
}

#[test]
fn keys_typed_in_a_native_window_run_the_main_windows_shortcuts() {
    let mut h = Harness::new();
    let viewport = h.float("swatches");
    h.key_in(viewport, Key::P);
    assert_eq!(h.app.session.tool_id(), "pen", "a tool key picks the tool");
    h.key_in(viewport, Key::V);
    assert_eq!(h.app.session.tool_id(), "selection");

    h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    h.app.run("shape.rectangle", json!({"x": 80, "y": 10, "width": 50, "height": 40})).unwrap();
    h.app.run("select.all", json!({})).unwrap();
    let group = h.app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
    h.app.run("object.isolate", json!({"id": group})).unwrap();
    h.frames(2);
    h.key_in(viewport, Key::Escape);
    assert!(h.app.session.active().unwrap().isolation.is_none(), "Escape leaves isolation mode");

    h.app.run("select.all", json!({})).unwrap();
    let before = h.objects();
    h.key_in(viewport, Key::Delete);
    assert!(h.objects() < before, "Delete deletes the selection");
    assert!(h.app.session.active().unwrap().selection.objects.is_empty());
}

/// Enter that commits a field of a floating panel's window, and Escape that cancels one, are the
/// field's: with an object selected and the Selection tool, Enter didn't also open the Move dialog
/// (#4803).
#[test]
fn enter_and_escape_in_a_native_windows_field_stay_in_the_field() {
    let mut h = Harness::new();
    h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    h.app.run("select.all", json!({})).unwrap();
    let viewport = h.float("transparency");
    // The window's first text field (Opacity), by the state its text box keeps.
    let ctx = h.ctx.clone();
    let field = ctx
        .viewport_for(viewport, |vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).map(|w| (w.id, w.rect)).collect::<Vec<_>>())
        .into_iter()
        .find(|(id, _)| egui::TextEdit::load_state(&ctx, *id).is_some())
        .map(|(_, rect)| rect.center())
        .expect("the panel has a text field");
    for key in [Key::Enter, Key::Escape] {
        if let Some(window) = h.host.borrow_mut().windows.get_mut(&viewport) {
            window.events.extend([button(field, true), button(field, false)]);
        }
        h.frames(2);
        h.key_in(viewport, key);
        assert!(!h.app.any_dialog_open(), "{key:?} in the field opens no dialog");
        assert_eq!(h.app.session.active().unwrap().selection.objects.len(), 1, "{key:?} in the field keeps the selection");
    }
    // Keys typed with no field editing still reach the main window.
    h.key_in(viewport, Key::Enter);
    assert!(h.app.any_dialog_open(), "Enter outside a field opens the Move dialog");
}

/// A file dropped on a native panel window opens through the same route as one dropped on the app
/// window: the drop's place is unknown, so it is not placed on the canvas.
#[test]
fn a_file_dropped_on_a_native_window_opens() {
    let mut h = Harness::new();
    let viewport = h.float("layers");
    let documents = h.app.session.documents().len();
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><rect width="10" height="10"/></svg>"#;
    // The OS holds the file over the window first, then drops it.
    if let Some(window) = h.host.borrow_mut().windows.get_mut(&viewport) {
        window.hovered.push(egui::HoveredFile::default());
    }
    h.frame(vec![]);
    if let Some(window) = h.host.borrow_mut().windows.get_mut(&viewport) {
        window.dropped.push(Arc::new(MockDrop(svg.to_vec())));
    }
    h.frames(4);
    assert_eq!(h.app.session.documents().len(), documents + 1, "the SVG opens as a document");
}

fn button(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// Art dragged off the canvas onto a panel in an OS window of its own, which lies over the
/// canvas: that panel takes it, and the art stays where it was.
#[test]
fn art_dragged_from_the_canvas_onto_a_native_panel_window_drops_there() {
    let mut h = Harness::new();
    h.ctx.enable_accesskit();
    let id = h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap();
    let id = vectorcraft_doc::NodeId(id);
    let viewport = h.float("graphicStyles");
    h.host.borrow_mut().focus = ViewportId::ROOT;
    h.frames(2);
    let styles = h.app.session.active().unwrap().doc.graphic_styles.len();
    let art = |h: &Harness| format!("{:?}", h.app.session.active().unwrap().doc.node(id).map(|n| &n.kind));
    let before = art(&h);
    let view = h.app.canvas_view().expect("the canvas was drawn");
    let from = view.to_screen(vectorcraft_geom::Point::new(35.0, 30.0));
    // The first style's tile, found by its name: the panel takes art dropped onto a style or between styles.
    let first = h.app.session.active().unwrap().doc.graphic_styles[0].name.clone();
    let to = {
        let host = h.host.borrow();
        let window = &host.windows[&viewport];
        let tile = window.nodes.iter().find(|(name, _)| *name == first).map(|(_, r)| *r).expect("the first style's tile");
        window.rect.min + tile.center().to_vec2() - ORIGIN.to_vec2()
    };
    assert!(view.rect.contains(to), "the window lies over the canvas");
    h.frame(vec![Event::PointerMoved(from), button(from, true)]);
    for k in 1..=8 {
        h.frame(vec![Event::PointerMoved(from + (to - from) * (k as f32 / 8.0))]);
    }
    h.frame(vec![Event::PointerMoved(to), button(to, false)]);
    h.frames(4);
    let st = h.app.session.active().unwrap();
    assert_eq!(st.doc.graphic_styles.len(), styles + 1, "the art became a graphic style");
    assert!(st.interaction.is_none(), "no move is left open");
    assert_eq!(art(&h), before, "the art stays where it was");
}

/// A click on a row of the Appearance panel floating in an OS window of its own gives the panel
/// Delete, as in the main window: the panel removes the highlighted row in its own window, which
/// forwards only the keys its widgets left over, so Edit › Clear doesn't delete the object.
#[test]
fn delete_after_a_row_click_in_a_native_appearance_window_removes_the_row() {
    let mut h = Harness::new();
    h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let items = |h: &Harness| {
        h.app.session.active().and_then(|d| d.selection.objects.first().and_then(|id| d.doc.node(*id))).map(|n| n.appearance.items.len())
    };
    assert_eq!(items(&h), Some(2), "a fill and a stroke");
    let viewport = h.float("appearance");
    h.frames(20);
    let fill =
        h.host.borrow().windows.get(&viewport).and_then(|w| w.texts.iter().find(|(t, _)| t == "Fill:").map(|(_, r)| *r)).expect("the Fill row");
    // The row's empty space just right of its label, taken from the label so it holds wherever the swatch sits
    // (the label is a link that opens the Color panel).
    let at = fill.right_center() + vec2(fill.height(), 0.0);
    for (events, pressed) in [(vec![Event::PointerMoved(at)], None), (vec![], Some(true)), (vec![], Some(false))] {
        let mut events = events;
        if let Some(pressed) = pressed {
            events.push(button(at, pressed));
        }
        if let Some(window) = h.host.borrow_mut().windows.get_mut(&viewport) {
            window.events.extend(events);
        }
        h.frame(vec![]);
    }
    h.frames(2);
    let before = h.objects();
    h.key_in(viewport, Key::Delete);
    assert_eq!(h.objects(), before, "the object stays");
    assert_eq!(items(&h), Some(1), "the fill row goes");
}
