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
    dropped: Vec<egui::DroppedFileHandle>,
    frames: usize,
}

struct Host {
    windows: HashMap<ViewportId, Window>,
    /// The window with the keyboard.
    focus: ViewportId,
    time: f64,
    /// Every command sent to a window, in order.
    commands: Vec<(ViewportId, ViewportCommand)>,
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
                self.commands.push((*id, command.clone()));
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
            host.windows.entry(id).or_insert_with(|| Window { rect, events: Vec::new(), dropped: Vec::new(), frames: 0 });
            let viewports = host.infos();
            let (time, focused) = (host.time, host.focus == id);
            let Some(window) = host.windows.get_mut(&id) else { return };
            window.frames += 1;
            egui::RawInput {
                viewport_id: id,
                viewports,
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, window.rect.size())),
                events: std::mem::take(&mut window.events),
                dropped_files: std::mem::take(&mut window.dropped),
                focused,
                time: Some(time),
                ..Default::default()
            }
        };
        let mut full = ctx.run_ui(input, |ui| (viewport.viewport_ui_cb)(ui));
        full.textures_delta.clear();
        host.borrow_mut().apply(&full);
    }
}

struct Harness {
    app: VectorcraftApp,
    ctx: egui::Context,
    host: Rc<RefCell<Host>>,
}

impl Harness {
    fn new() -> Self {
        let host = Rc::new(RefCell::new(Host { windows: HashMap::new(), focus: ViewportId::ROOT, time: 0.0, commands: Vec::new() }));
        let shared = host.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| Host::render(&shared, ctx, viewport));
        let ctx = egui::Context::default();
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
        let mut input = egui::RawInput { time: Some(time), events, screen_rect: Some(SCREEN), viewports, focused, ..Default::default() };
        self.app.raw_input_hook(&self.ctx, &mut input);
        let app = &mut self.app;
        let mut full = self.ctx.run_ui(input, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        full.textures_delta.clear();
        self.host.borrow_mut().apply(&full);
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.frame(vec![]);
        }
    }

    /// The viewport of the floating window holding `panel`.
    fn viewport(&self, panel: &str) -> ViewportId {
        let layout = layout_of(&self.app.ui);
        let window = floating_window(&layout, panel).expect("the panel floats");
        craft_ui::docking::DockArea::<String>::window_viewport(area_id(), window.id)
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

#[test]
fn a_floated_panel_opens_its_own_os_window_where_the_backend_has_viewports() {
    let mut h = Harness::new();
    let viewport = h.float("swatches");
    let rect = h.host.borrow().windows.get(&viewport).map(|w| w.rect).unwrap();
    assert!(rect.min.x >= ORIGIN.x && rect.min.y >= ORIGIN.y, "opens over its place in the app window: {rect:?}");
    // The window keeps its id, and with it its OS window.
    let before = layout_of(&h.app.ui).floating[0].id;
    h.frames(3);
    assert_eq!(layout_of(&h.app.ui).floating[0].id, before);
    assert_eq!(h.host.borrow().windows.len(), 1);

    // Embedded viewports (the web) draw it in the app window.
    let ctx = egui::Context::default();
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("window.panel.float", json!({"panel": "swatches", "x": 200, "y": 200})).unwrap();
    for _ in 0..3 {
        let input = egui::RawInput { screen_rect: Some(SCREEN), ..Default::default() };
        let mut full = ctx.run_ui(input, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
        });
        full.textures_delta.clear();
    }
    let tab = craft_ui::docking::DockArea::<String>::hit_rects(&ctx, area_id())
        .into_iter()
        .any(|(hit, _)| hit == craft_ui::docking::Hit::Tab("swatches".into()));
    assert!(tab, "its tab is drawn in the app window");
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

#[test]
fn a_modal_blocks_native_windows_and_takes_the_keyboard_from_them() {
    let mut h = Harness::new();
    let viewport = h.float("swatches");
    h.app.ui.about = true;
    h.frames(6);
    let focus_commands =
        |h: &Harness| h.host.borrow().commands.iter().filter(|(id, c)| *id == ViewportId::ROOT && *c == ViewportCommand::Focus).count();
    assert_eq!(focus_commands(&h), 1, "the main window gets the keyboard");
    h.key_in(viewport, Key::P);
    assert_eq!(h.app.session.tool_id(), "selection", "a key typed in the blocked window does nothing");
    assert_eq!(focus_commands(&h), 1, "once");

    h.app.ui.about = false;
    h.frames(3);
    h.key_in(viewport, Key::P);
    assert_eq!(h.app.session.tool_id(), "pen", "the window takes keys again once the modal closes");
}

#[test]
fn a_file_dropped_on_a_native_window_opens() {
    let mut h = Harness::new();
    let viewport = h.float("layers");
    let documents = h.app.session.documents().len();
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40"><rect width="10" height="10"/></svg>"#;
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
    let id = h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap();
    let id = vectorcraft_doc::NodeId(id);
    let viewport = h.float("graphicStyles");
    h.host.borrow_mut().focus = ViewportId::ROOT;
    h.frames(2);
    let styles = h.app.session.active().unwrap().doc.graphic_styles.len();
    let art = |h: &Harness| format!("{:?}", h.app.session.active().unwrap().doc.node(id).map(|n| &n.kind));
    let before = art(&h);
    let xf = crate::canvas::Xf::new(h.app.canvas_rect.unwrap(), h.app.view().unwrap());
    let from = xf.to_screen(vectorcraft_geom::Point::new(35.0, 30.0));
    let window = h.host.borrow().windows.get(&viewport).map(|w| w.rect).unwrap();
    let to = window.min + vec2(60.0, 120.0) - ORIGIN.to_vec2();
    assert!(h.app.canvas_rect.unwrap().contains(to), "the window lies over the canvas");
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

#[test]
fn ui_docking_leaves_out_the_controls_of_os_windows() {
    let mut h = Harness::new();
    h.float("swatches");
    let report = inspect(&h.app, &h.ctx);
    let hits = report["hitRects"].as_array().unwrap();
    assert!(hits.iter().any(|hit| hit["hit"] == json!({"Tab": "layers"})), "the main window's tabs are there");
    let swatches = |hit: &Value| hit["hit"].as_object().is_some_and(|o| o.values().any(|v| v == "swatches"));
    assert!(!hits.iter().any(swatches), "none of the OS window's: {hits:?}");
    assert!(report["layout"]["floating"][0]["native"].is_array(), "its place is the window's native rect");
}
