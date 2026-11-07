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
    /// The texts its last frame drew, with their rects in the window.
    texts: Vec<(String, Rect)>,
}

struct Host {
    windows: HashMap<ViewportId, Window>,
    child_overlay: Option<(ViewportId, Rect)>,
    child_transparent_overlay: Option<(ViewportId, Rect)>,
    child_transition_overlay: Option<(ViewportId, Rect, bool)>,
    child_empty_overlay: Option<(ViewportId, Rect)>,
    child_pass_through_overlay: Option<(ViewportId, Rect)>,
    child_raise_overlay: Option<(ViewportId, Rect, bool)>,
    child_raise_layer: Option<(ViewportId, egui::LayerId)>,
    child_layer_order: HashMap<ViewportId, Vec<egui::LayerId>>,
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
            host.windows.entry(id).or_insert_with(|| Window { rect, events: Vec::new(), dropped: Vec::new(), frames: 0, texts: Vec::new() });
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
        let child_overlay = host.borrow().child_overlay.filter(|(viewport, _)| *viewport == id).map(|(_, rect)| rect);
        let child_transparent_overlay = host.borrow().child_transparent_overlay.filter(|(viewport, _)| *viewport == id).map(|(_, rect)| rect);
        let child_transition_overlay =
            host.borrow().child_transition_overlay.filter(|(viewport, _, _)| *viewport == id).map(|(_, rect, painted)| (rect, painted));
        let child_empty_overlay = host.borrow().child_empty_overlay.filter(|(viewport, _)| *viewport == id).map(|(_, rect)| rect);
        let child_pass_through_overlay = host.borrow().child_pass_through_overlay.filter(|(viewport, _)| *viewport == id).map(|(_, rect)| rect);
        let child_raise_overlay = host.borrow().child_raise_overlay.filter(|(viewport, _, _)| *viewport == id).map(|(_, rect, raise)| (rect, raise));
        let child_raise_layer = host.borrow().child_raise_layer.filter(|(viewport, _)| *viewport == id).map(|(_, layer)| layer);
        let mut layer_order = Vec::new();
        let mut full = ctx.run_ui(input, |ui| {
            layer_order = ui.ctx().memory(|memory| memory.layer_ids().collect());
            if let Some((rect, _)) = child_raise_overlay {
                egui::Area::new(egui::Id::unique("Child raised overlay")).order(egui::Order::Background).fixed_pos(rect.min).show(ui.ctx(), |ui| {
                    ui.allocate_space(rect.size());
                });
            }
            (viewport.viewport_ui_cb)(ui);
            if let Some(layer) = child_raise_layer {
                ui.ctx().move_to_top(layer);
            }
            if child_raise_overlay.is_some_and(|(_, raise)| raise) {
                ui.ctx().move_to_top(egui::LayerId::new(egui::Order::Background, egui::Id::unique("Child raised overlay")));
            }
            if let Some(rect) = child_overlay {
                egui::Window::new("Child overlay").fixed_pos(rect.min).fixed_size(rect.size()).show(ui.ctx(), |ui| {
                    ui.label("Child overlay");
                });
            }
            if let Some(rect) = child_transparent_overlay {
                egui::Area::new(egui::Id::unique("Child transparent overlay")).order(egui::Order::Foreground).fixed_pos(rect.min).show(
                    ui.ctx(),
                    |ui| {
                        ui.allocate_space(rect.size());
                    },
                );
            }
            if let Some((rect, painted)) = child_transition_overlay {
                egui::Area::new(egui::Id::unique("Child transition overlay")).order(egui::Order::Foreground).fixed_pos(rect.min).show(
                    ui.ctx(),
                    |ui| {
                        ui.allocate_space(rect.size());
                        if painted {
                            ui.label("Child transition overlay");
                        }
                    },
                );
            }
            if let Some(rect) = child_empty_overlay {
                egui::Area::new(egui::Id::unique("Child empty overlay")).order(egui::Order::Foreground).fixed_pos(rect.min).show(ui.ctx(), |ui| {
                    ui.set_min_size(rect.size());
                });
            }
            if let Some(rect) = child_pass_through_overlay {
                egui::Area::new(egui::Id::unique("Child pass-through overlay"))
                    .order(egui::Order::Foreground)
                    .fixed_pos(rect.min)
                    .interactable(false)
                    .show(ui.ctx(), |ui| {
                        let (paint, _) = ui.allocate_exact_size(rect.size(), egui::Sense::hover());
                        ui.painter().rect_filled(paint, 0.0, egui::Color32::TRANSPARENT);
                    });
            }
        });
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
        let mut host = host.borrow_mut();
        host.child_layer_order.insert(id, layer_order);
        if let Some(window) = host.windows.get_mut(&id) {
            window.texts = shown;
        }
        host.apply(&full);
    }
}

struct Harness {
    app: VectorcraftApp,
    ctx: egui::Context,
    host: Rc<RefCell<Host>>,
    root_overlay: Option<Rect>,
}

impl Harness {
    fn new() -> Self {
        let host = Rc::new(RefCell::new(Host {
            windows: HashMap::new(),
            child_overlay: None,
            child_transparent_overlay: None,
            child_transition_overlay: None,
            child_empty_overlay: None,
            child_pass_through_overlay: None,
            child_raise_overlay: None,
            child_raise_layer: None,
            child_layer_order: HashMap::new(),
            focus: ViewportId::ROOT,
            time: 0.0,
            commands: Vec::new(),
        }));
        let shared = host.clone();
        egui::Context::set_immediate_viewport_renderer(move |ctx, viewport| Host::render(&shared, ctx, viewport));
        let ctx = egui::Context::default();
        ctx.set_embed_viewports(false);
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let mut h = Self { app, ctx, host, root_overlay: None };
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
        let root_overlay = self.root_overlay;
        let mut full = self.ctx.run_ui(input, |ui| {
            app.logic(ui.ctx());
            app.ui(ui);
            if let Some(rect) = root_overlay {
                egui::Window::new("Root overlay").fixed_pos(rect.min).fixed_size(rect.size()).show(ui.ctx(), |ui| {
                    ui.label("Root overlay");
                });
            }
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

/// Sets the height of the OS window `viewport`, as a user's resize does.
fn resize_window(h: &mut Harness, viewport: ViewportId, height: f32) {
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().rect.set_height(height);
    h.frames(3);
}

/// A click on the `side` pt square button with the highest `rank` in the OS window `viewport`, and
/// the frames its menu takes to settle. Returns where the button is.
fn open_button_menu(h: &mut Harness, viewport: ViewportId, side: f32, rank: impl Fn(Pos2) -> f32) -> Pos2 {
    let ctx = h.ctx.clone();
    let menu = ctx
        .viewport_for(viewport, |vp| {
            vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).map(|w| (w.rect, w.sense.senses_click())).collect::<Vec<_>>()
        })
        .into_iter()
        .filter(|(rect, clicks)| *clicks && rect.size() == vec2(side, side))
        .map(|(rect, _)| rect.center())
        .max_by(|a, b| rank(*a).total_cmp(&rank(*b)))
        .expect("the menu's button");
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([Event::PointerMoved(menu), button(menu, true), button(menu, false)]);
    h.frames(8);
    menu
}

/// A click on the ≡ button in the OS window `viewport`, the 16 pt square at the right end of the
/// panel's tab strip. Returns where the button is.
fn open_panel_menu(h: &mut Harness, viewport: ViewportId) -> Pos2 {
    open_button_menu(h, viewport, 16.0, |at| at.x)
}

/// The rect of the text `wanted` in the last frame of the OS window `viewport`: the last one drawn,
/// so a menu's item comes before a panel's text of the same name.
fn text_rect(h: &Harness, viewport: ViewportId, wanted: &str) -> Option<Rect> {
    h.host.borrow().windows.get(&viewport).unwrap().texts.iter().rfind(|(text, _)| text.trim() == wanted).map(|(_, rect)| *rect)
}

/// Turns the wheel down by `by` points with the pointer at `over` in the OS window `viewport`, and
/// lets the scrolling finish.
fn wheel_down(h: &mut Harness, viewport: ViewportId, over: Pos2, by: f32) {
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(over),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -by),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frames(40);
}

/// A floating panel's window of each height keeps its ≡ menu inside it: the list scrolls, and the
/// wheel brings the last item into the window. The list opens level with the button where the
/// window has room for it, and does not slide up to fit. 100 pt is the shortest a window can be
/// resized to, and there the list takes the whole window.
#[test]
fn a_long_panel_menu_scrolls_inside_a_short_native_window() {
    for (height, level_with_the_button) in [(240.0, true), (100.0, false)] {
        let mut h = Harness::new();
        let viewport = h.float("layers");
        resize_window(&mut h, viewport, height);
        let menu = open_panel_menu(&mut h, viewport);
        if level_with_the_button {
            let first = text_rect(&h, viewport, "New Layer…").expect("the menu's first item");
            assert!(first.top() >= menu.y - 16.0, "the first item at {first:?} is level with the button at {menu:?}");
        }
        let last_item = |h: &Harness| text_rect(h, viewport, "Dock Panel");
        assert!(last_item(&h).is_none_or(|r| r.bottom() > height), "the {height} pt window starts at the top of the list");
        wheel_down(&mut h, viewport, pos2(menu.x - 60.0, height - 30.0), 5000.0);
        let item = last_item(&h).unwrap_or_else(|| panic!("the wheel brings the menu's last item into the {height} pt window"));
        assert!(item.bottom() <= height, "the last item at {item:?} is inside the {height} pt window");
    }
}

/// The Layers menu's Save Selection submenu lists every saved selection: in a short window the
/// wheel brings the last one into view.
#[test]
fn a_long_panel_submenu_scrolls_inside_a_short_native_window() {
    let mut h = Harness::new();
    h.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    h.app.run("select.all", json!({})).unwrap();
    // A document keeps at most 25 saved selections.
    for n in 0..25 {
        h.app.run("select.save", json!({"name": format!("Saved {n}")})).unwrap();
    }
    assert_submenu_reaches_its_end(&mut h, "layers", 240.0, "Save Selection", "Saved 0", "Saved 24");
}

/// The Swatches menu's Open Swatch Library lists the libraries between its head and tail items.
#[test]
fn the_library_submenu_scrolls_inside_a_short_native_window() {
    let mut h = Harness::new();
    assert_submenu_reaches_its_end(&mut h, "swatches", 240.0, "Open Swatch Library", "Default Swatches", "Other Library…");
}

/// The Layers panel's Filter popup, under the topmost 24 pt button: the wheel brings its last row
/// into a short window.
#[test]
fn the_layers_filter_popup_scrolls_inside_a_short_native_window() {
    let mut h = Harness::new();
    let viewport = h.float("layers");
    resize_window(&mut h, viewport, 160.0);
    open_button_menu(&mut h, viewport, 24.0, |at| -at.y);
    let over = text_rect(&h, viewport, "By Object").expect("the popup's By Object heading");
    assert!(text_rect(&h, viewport, "Hidden").is_none_or(|end| end.bottom() > 160.0), "the popup opens at the top of its list");
    wheel_down(&mut h, viewport, over.center(), 5000.0);
    let end = text_rect(&h, viewport, "Hidden").expect("the wheel brings the popup's last row into the window");
    assert!(end.bottom() <= 160.0, "the last row at {end:?} is inside the window");
}

/// The Color Guide's Limit to Library popup, on the leftmost button of the bottom bar: with the
/// window made short while the popup is open, the wheel moves the list up past its Document
/// Swatches row.
#[test]
fn the_color_guide_library_popup_scrolls_inside_a_short_native_window() {
    let mut h = Harness::new();
    let viewport = h.float("colorGuide");
    resize_window(&mut h, viewport, 320.0);
    open_button_menu(&mut h, viewport, 24.0, |at| at.y * 1000.0 - at.x);
    resize_window(&mut h, viewport, 100.0);
    let first = text_rect(&h, viewport, "Document Swatches").expect("the popup's Document Swatches row");
    wheel_down(&mut h, viewport, first.center(), 5000.0);
    let moved = text_rect(&h, viewport, "Document Swatches").is_none_or(|row| row.top() < first.top());
    assert!(moved, "the wheel scrolls the list that starts at {first:?}");
}

/// In an OS window of `panel`, `height` tall, scrolls the ≡ menu until its row `row` shows, clicks
/// the row to open its submenu, turns the wheel over the submenu's item `first`, and checks that its
/// item `last` comes into the window.
fn assert_submenu_reaches_its_end(h: &mut Harness, panel: &str, height: f32, row: &str, first: &str, last: &str) {
    let viewport = h.float(panel);
    resize_window(h, viewport, height);
    let menu = open_panel_menu(h, viewport);
    let inside = |h: &Harness| text_rect(h, viewport, row).filter(|rect| rect.bottom() < height - 12.0);
    for _ in 0..40 {
        if inside(h).is_some() {
            break;
        }
        wheel_down(h, viewport, pos2(menu.x - 60.0, height - 30.0), 40.0);
    }
    let at = inside(h).unwrap_or_else(|| panic!("the {row} row")).center();
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([Event::PointerMoved(at), button(at, true), button(at, false)]);
    h.frames(8);
    let over = text_rect(h, viewport, first).unwrap_or_else(|| panic!("the {row} submenu's item {first}"));
    assert!(text_rect(h, viewport, last).is_none_or(|end| end.bottom() > height), "the submenu opens at the top of its list");
    for _ in 0..3 {
        if text_rect(h, viewport, last).is_some() {
            break;
        }
        wheel_down(h, viewport, pos2(over.center().x, height - 30.0), 5000.0);
    }
    let end = text_rect(h, viewport, last).unwrap_or_else(|| panic!("the {row} submenu's item {last}"));
    assert!(end.bottom() <= height, "{last} at {end:?} is inside the {height} pt window");
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
        assert!(h.app.ui.dialog.is_none(), "{key:?} in the field opens no dialog");
        assert_eq!(h.app.session.active().unwrap().selection.objects.len(), 1, "{key:?} in the field keeps the selection");
    }
    // Keys typed with no field editing still reach the main window.
    h.key_in(viewport, Key::Enter);
    assert!(h.app.ui.dialog.is_some(), "Enter outside a field opens the Move dialog");
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

/// The level and visibility a palette window takes under a dialog or a menu, and otherwise: a
/// palette stays out of the way of anything drawn in the main window.
#[test]
fn a_palette_window_keeps_out_of_the_way_of_the_main_windows_dialog_and_menus() {
    use egui::WindowLevel;
    assert_eq!(palette_window_state(true, true, true), (WindowLevel::Normal, false), "hidden under a dialog, even in a menu");
    assert_eq!(palette_window_state(true, false, true), (WindowLevel::Normal, false), "hidden under a dialog");
    assert_eq!(palette_window_state(false, true, true), (WindowLevel::Normal, false), "hidden under a menu: it is drawn in the main window");
    assert_eq!(palette_window_state(false, false, true), (WindowLevel::AlwaysOnTop, true), "its palette level while the app is active");
    let inactive = palette_window_state(false, false, false);
    assert!(inactive.1, "shown while the app is inactive");
    assert_ne!(inactive.0, WindowLevel::AlwaysOnTop, "and not above other applications' windows: {:?}", inactive.0);
}

/// With a menu of the in-window menu bar open, the palettes take the normal level (still shown):
/// the menu is drawn in the main window, or it would open under them.
#[test]
fn a_menu_of_the_menu_bar_lowers_the_palette_windows() {
    let mut h = Harness::new();
    let viewport = h.float("navigator");
    h.host.borrow_mut().commands.clear();
    let layout = layout_of(&h.app.ui);
    h.ctx.data_mut(|d| d.insert_temp(egui::Id::unique("menu-bar-open"), true));
    assert!(crate::menus::menu_open(&h.ctx), "the menu bar's state says a menu is open");
    palette_window_levels(&h.ctx, &layout, false, crate::menus::menu_open(&h.ctx));
    h.frames(1);
    let commands: Vec<(egui::ViewportId, ViewportCommand)> = h.host.borrow().commands.clone();
    let levels: Vec<egui::WindowLevel> = commands
        .iter()
        .filter_map(|(id, command)| match command {
            ViewportCommand::WindowLevel(level) if *id == viewport => Some(*level),
            _ => None,
        })
        .collect();
    assert!(levels.contains(&egui::WindowLevel::Normal), "the palette drops its topmost level under the menu: {levels:?}");
    assert!(
        commands.iter().any(|(id, command)| *id == viewport && matches!(command, ViewportCommand::Visible(false))),
        "and is hidden until the menu closes"
    );
}
/// A modal dialog is drawn in the main window, so the palettes' OS windows leave their topmost
/// level while it is open and take it back after, or the dialog would be covered by them.
#[test]
fn a_modal_lowers_the_native_palette_windows() {
    let mut h = Harness::new();
    let viewport = h.float("swatches");
    let levels = |h: &Harness| -> Vec<egui::WindowLevel> {
        h.host
            .borrow()
            .commands
            .iter()
            .filter_map(|(id, command)| match command {
                ViewportCommand::WindowLevel(level) if *id == viewport => Some(*level),
                _ => None,
            })
            .collect()
    };
    let hidden = |h: &Harness| -> Vec<bool> {
        h.host
            .borrow()
            .commands
            .iter()
            .filter_map(|(id, command)| match command {
                ViewportCommand::Visible(visible) if *id == viewport => Some(*visible),
                _ => None,
            })
            .collect()
    };
    h.app.ui.about = true;
    h.frames(4);
    assert!(levels(&h).contains(&egui::WindowLevel::Normal), "the palette leaves AlwaysOnTop under the modal: {:?}", levels(&h));
    assert_eq!(hidden(&h), [false], "and is hidden, so a click cannot bring it up over the dialog: {:?}", hidden(&h));
    h.app.ui.about = false;
    h.frames(4);
    assert!(levels(&h).contains(&egui::WindowLevel::AlwaysOnTop), "and takes its palette level back: {:?}", levels(&h));
    assert_eq!(hidden(&h), [false, true], "shown again with the dialog gone: {:?}", hidden(&h));
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
    for _ in 0..20 {
        h.frame(vec![]);
    }
    let fill =
        h.host.borrow().windows.get(&viewport).and_then(|w| w.texts.iter().find(|(t, _)| t == "Fill:").map(|(_, r)| *r)).expect("the Fill row");
    // The row's empty space right of its swatch (its name is a link that opens the Color panel).
    let at = pos2(fill.left() + 110.0, fill.center().y);
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

#[test]
fn wheel_over_a_floating_character_font_field_changes_the_font() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let pointer = pos2(field.min.x + 4.0, field.center().y);
    h.root_overlay = Some(field.expand(10.0));
    h.frames(2);
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(pointer),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    h.frames(2);
    let fonts = h.app.run("text.fonts", json!({})).unwrap();
    assert_ne!(fonts[0]["family"], "Source Sans 3", "{fonts}");

    h.host.borrow_mut().child_overlay = Some((viewport, field.expand(10.0)));
    h.frames(2);
    let covered = h.app.run("text.fonts", json!({})).unwrap();
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(pointer),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), covered);

    h.host.borrow_mut().child_overlay = None;
    h.frame(vec![]);
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(pointer),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    h.frame(vec![]);
    assert_ne!(h.app.run("text.fonts", json!({})).unwrap(), covered);

    h.host.borrow_mut().child_transparent_overlay = Some((viewport, field.expand(10.0)));
    h.frames(2);
    let covered = h.app.run("text.fonts", json!({})).unwrap();
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(pointer),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), covered);

    h.host.borrow_mut().child_transparent_overlay = None;
    h.host.borrow_mut().child_pass_through_overlay = Some((viewport, field.expand(10.0)));
    h.frames(2);
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(pointer),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    h.frame(vec![]);
    assert_ne!(h.app.run("text.fonts", json!({})).unwrap(), covered);
}

#[test]
fn a_modal_blocks_font_wheel_input_in_a_floating_panel() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let pointer = pos2(field.min.x + 4.0, field.center().y);
    let before = h.app.run("text.fonts", json!({})).unwrap();
    h.app.ui.about = true;
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(pointer),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    h.frames(2);
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), before);
    h.app.ui.about = false;
    h.frames(2);
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), before);
}

#[test]
fn wheel_stays_under_an_overlay_that_stops_painting() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let pointer = pos2(field.min.x + 4.0, field.center().y);
    h.host.borrow_mut().child_transition_overlay = Some((viewport, field.expand(10.0), true));
    h.frames(2);
    let covered = h.app.run("text.fonts", json!({})).unwrap();
    h.host.borrow_mut().child_transition_overlay = Some((viewport, field.expand(10.0), false));
    h.frame(vec![]);
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(pointer),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), covered);
}

#[test]
fn wheel_stays_under_an_empty_interactive_overlay() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let pointer = pos2(field.min.x + 4.0, field.center().y);
    h.host.borrow_mut().child_empty_overlay = Some((viewport, field.expand(10.0)));
    h.frames(2);
    let covered = h.app.run("text.fonts", json!({})).unwrap();
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(pointer),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), covered);
}

#[test]
fn a_raised_overlay_blocks_the_next_font_wheel_event() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let pointer = pos2(field.min.x + 4.0, field.center().y);
    h.host.borrow_mut().child_raise_overlay = Some((viewport, field.expand(10.0), false));
    h.frames(2);
    let field_layer = crate::font_menu::wheel_field_layer_for_test(&h.ctx, viewport, pointer).expect("the font field layer");
    h.host.borrow_mut().child_raise_layer = Some((viewport, field_layer));
    h.frame(vec![]);
    h.host.borrow_mut().child_raise_layer = None;
    let overlay_layer = egui::LayerId::new(egui::Order::Background, egui::Id::unique("Child raised overlay"));
    let before = h.app.run("text.fonts", json!({})).unwrap();
    h.host.borrow_mut().child_raise_overlay = Some((viewport, field.expand(10.0), true));
    h.frame(vec![]);
    let order = h.host.borrow().child_layer_order.get(&viewport).cloned().expect("child layer order");
    let field_index = order.iter().position(|layer| *layer == field_layer).expect("font field in layer order");
    let overlay_index = order.iter().position(|layer| *layer == overlay_layer).unwrap_or_else(|| panic!("overlay absent from {order:?}"));
    assert!(field_index > overlay_index, "the font field must start above the overlay: {order:?}");
    h.host.borrow_mut().child_raise_overlay = Some((viewport, field.expand(10.0), false));
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(pointer),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
        Event::PointerGone,
    ]);
    h.frame(vec![]);
    let order = h.host.borrow().child_layer_order.get(&viewport).cloned().expect("child layer order");
    let field_index = order.iter().position(|layer| *layer == field_layer).expect("font field in layer order");
    let overlay_index = order.iter().position(|layer| *layer == overlay_layer).unwrap_or_else(|| panic!("overlay absent from {order:?}"));
    assert!(overlay_index > field_index, "the overlay must finish above the font field: {order:?}");
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), before);
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(
        h.ctx.input_for(viewport, |input| input.raw.events.iter().any(|event| matches!(event, Event::MouseWheel { .. }))),
        "the raised overlay must receive the wheel event"
    );
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), before);
    assert_eq!(h.ctx.input_for(viewport, |input| input.pointer.latest_pos()), None);
}

#[test]
fn an_overlay_raised_with_the_wheel_event_does_not_change_the_font() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let pointer = pos2(field.min.x + 4.0, field.center().y);
    h.host.borrow_mut().child_raise_overlay = Some((viewport, field.expand(10.0), false));
    h.frames(2);
    let field_layer = crate::font_menu::wheel_field_layer_for_test(&h.ctx, viewport, pointer).expect("the font field layer");
    h.host.borrow_mut().child_raise_layer = Some((viewport, field_layer));
    h.frame(vec![]);
    h.host.borrow_mut().child_raise_layer = None;
    h.frame(vec![]);
    let overlay_layer = egui::LayerId::new(egui::Order::Background, egui::Id::unique("Child raised overlay"));
    let order = h.host.borrow().child_layer_order.get(&viewport).cloned().expect("child layer order");
    let field_index = order.iter().position(|layer| *layer == field_layer).expect("font field in layer order");
    let overlay_index = order.iter().position(|layer| *layer == overlay_layer).expect("overlay in layer order");
    assert!(field_index > overlay_index, "the font field must be above the overlay before the wheel event: {order:?}");
    let before = h.app.run("text.fonts", json!({})).unwrap();
    h.host.borrow_mut().child_raise_overlay = Some((viewport, field.expand(10.0), true));
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(pointer),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), before);
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(
        h.ctx.input_for(viewport, |input| input.raw.events.iter().any(|event| matches!(event, Event::MouseWheel { .. }))),
        "the raised overlay must receive the wheel event"
    );
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), before);
}

#[test]
fn a_closed_overlay_does_not_replay_its_wheel_into_the_panel() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let pointer = pos2(field.min.x + 4.0, field.center().y);
    h.host.borrow_mut().child_raise_overlay = Some((viewport, field.expand(10.0), false));
    h.frames(2);
    let field_layer = crate::font_menu::wheel_field_layer_for_test(&h.ctx, viewport, pointer).expect("the font field layer");
    h.host.borrow_mut().child_raise_layer = Some((viewport, field_layer));
    h.frame(vec![]);
    h.host.borrow_mut().child_raise_layer = None;
    h.frame(vec![]);
    let before = h.app.run("text.fonts", json!({})).unwrap();
    h.host.borrow_mut().child_raise_overlay = Some((viewport, field.expand(10.0), true));
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(pointer),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    h.host.borrow_mut().child_raise_overlay = None;
    h.frame(vec![]);
    h.frame(vec![]);
    assert!(
        !h.ctx.input_for(viewport, |input| input.raw.events.iter().any(|event| matches!(event, Event::MouseWheel { .. }))),
        "a closed overlay cannot receive a later wheel event"
    );
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), before);
}

#[test]
fn a_wheel_before_opening_a_floating_font_menu_changes_the_font() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let pointer = pos2(field.min.x + 4.0, field.center().y);
    let before = h.app.run("text.fonts", json!({})).unwrap();
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(pointer),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
        Event::PointerButton { pos: pointer, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() },
        Event::PointerButton { pos: pointer, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() },
    ]);
    h.frames(3);
    assert_ne!(h.app.run("text.fonts", json!({})).unwrap(), before);
}

#[test]
fn a_delayed_wheel_does_not_change_a_font_field_that_moved() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let pointer = pos2(field.min.x + 4.0, field.center().y);
    let before = h.app.run("text.fonts", json!({})).unwrap();
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(pointer),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    crate::font_menu::offset_wheel_fields_for_test(&h.ctx, viewport, vec2(300.0, 0.0));
    h.frame(vec![]);
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), before);
}

#[test]
fn wheel_events_on_both_sides_of_an_overlay_are_checked_separately() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let covered = pos2(field.min.x + 4.0, field.center().y);
    let clear = pos2(field.max.x - 4.0, field.center().y);
    let overlay = Rect::from_min_max(field.min - vec2(2.0, 2.0), pos2(field.center().x, field.max.y + 2.0));
    assert!(covered.x < overlay.max.x && clear.x > overlay.max.x);
    h.host.borrow_mut().child_raise_overlay = Some((viewport, overlay, false));
    h.frames(2);
    let field_layer = crate::font_menu::wheel_field_layer_for_test(&h.ctx, viewport, clear).expect("the font field layer");
    h.host.borrow_mut().child_raise_layer = Some((viewport, field_layer));
    h.frame(vec![]);
    h.host.borrow_mut().child_raise_layer = None;
    h.frame(vec![]);
    let before = h.app.run("text.fonts", json!({})).unwrap();
    h.host.borrow_mut().child_raise_overlay = Some((viewport, overlay, true));
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(covered),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
        Event::PointerMoved(clear),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    h.frame(vec![]);
    h.frame(vec![]);
    assert_ne!(h.app.run("text.fonts", json!({})).unwrap(), before);
    assert_eq!(h.ctx.input_for(viewport, |input| input.pointer.latest_pos()), Some(clear));
}

#[test]
fn leaving_a_floating_font_field_resets_queued_fractional_wheel_input() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let inside = pos2(field.min.x + 4.0, field.center().y);
    let outside = pos2(field.max.x + 500.0, field.center().y);
    let before = h.app.run("text.fonts", json!({})).unwrap();
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(inside),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -0.6),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
        Event::PointerMoved(outside),
    ]);
    h.frame(vec![]);
    h.frame(vec![]);
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(inside),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -0.6),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), before);
}

#[test]
fn a_floating_wheel_turn_precedes_a_pointer_leave_in_the_menu_order() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    h.app.ui.recent_fonts = ["Source Sans 3", "Source Serif 4", "Inter"].map(str::to_string).into();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let inside = pos2(field.min.x + 4.0, field.center().y);
    let outside = pos2(field.max.x + 500.0, field.center().y);
    let wheel = Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, -1.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    };
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([Event::PointerMoved(inside), wheel.clone()]);
    h.frame(vec![]);
    h.frames(2);
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap()[0]["family"], "Source Serif 4");
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([wheel, Event::PointerMoved(outside)]);
    h.frame(vec![]);
    h.frames(2);
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap()[0]["family"], "Inter");
}

#[test]
fn moving_under_an_overlay_resets_queued_fractional_font_wheel_input() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let covered = pos2(field.min.x + 4.0, field.center().y);
    let clear = pos2(field.max.x - 4.0, field.center().y);
    let overlay = Rect::from_min_max(field.min - vec2(2.0, 2.0), pos2(field.center().x, field.max.y + 2.0));
    h.host.borrow_mut().child_raise_overlay = Some((viewport, overlay, true));
    h.frames(2);
    let before = h.app.run("text.fonts", json!({})).unwrap();
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.extend([
        Event::PointerMoved(clear),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -0.6),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
        Event::PointerMoved(covered),
        Event::PointerMoved(clear),
        Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -0.6),
            phase: egui::TouchPhase::Move,
            modifiers: Default::default(),
            source: egui::MouseWheelSource::Unknown,
        },
    ]);
    h.frame(vec![]);
    h.frame(vec![]);
    h.frame(vec![]);
    assert_eq!(h.app.run("text.fonts", json!({})).unwrap(), before);
}

#[test]
fn wheel_without_a_move_uses_the_floating_viewports_pointer() {
    let mut h = Harness::new();
    h.app.run("text.create", json!({"x": 30, "y": 50, "text": "Hi", "font": "Source Sans 3"})).unwrap();
    let viewport = h.float("character");
    let field = h
        .host
        .borrow()
        .windows
        .get(&viewport)
        .and_then(|w| w.texts.iter().find(|(text, _)| text == "Source Sans 3").map(|(_, rect)| *rect))
        .expect("the Character panel's font family field");
    let pointer = pos2(field.min.x + 4.0, field.center().y);
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.push(Event::PointerMoved(pointer));
    h.frame(vec![Event::PointerMoved(pos2(1200.0, 800.0))]);
    h.app.run("text.create", json!({"x": 30, "y": 90, "text": "New", "font": "Source Sans 3"})).unwrap();
    h.frames(2);
    let before = h.app.run("text.fonts", json!({})).unwrap();
    h.host.borrow_mut().windows.get_mut(&viewport).unwrap().events.push(Event::MouseWheel {
        unit: egui::MouseWheelUnit::Line,
        delta: vec2(0.0, -1.0),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
        source: egui::MouseWheelSource::Unknown,
    });
    h.frame(vec![]);
    h.frame(vec![]);
    assert_ne!(h.app.run("text.fonts", json!({})).unwrap(), before);
}
