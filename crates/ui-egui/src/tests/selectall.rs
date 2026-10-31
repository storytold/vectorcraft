//! Select All (Cmd+A) follows the keyboard, from the window and from the system menu bar (macOS):
//! a focused text field selects its own text, the Type tool editing selects the text being edited,
//! the Artboards panel selects every artboard after a click on one of its rows with the Artboard
//! tool, and only otherwise are all the objects selected.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::{VectorcraftApp, menus};

struct App {
    app: VectorcraftApp,
    ctx: egui::Context,
    time: f64,
}

impl App {
    /// A document with a 37 pt wide rectangle (selected) and a second, unselected one.
    fn new() -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("shape.rectangle", json!({"x": 200, "y": 150, "width": 20, "height": 20})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 37, "height": 40})).unwrap();
        let mut a = Self { app, ctx: egui::Context::default(), time: 0.0 };
        for _ in 0..3 {
            a.frame(vec![]);
        }
        a
    }

    /// One whole app frame with `events`; `menu`: a system menu item chosen first, as the macOS
    /// host dispatches it before the app's logic.
    fn frame_with(&mut self, events: Vec<Event>, menu: Option<&str>) -> egui::FullOutput {
        self.time += 0.1;
        let raw = egui::RawInput {
            events,
            time: Some(self.time),
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 850.0))),
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(raw, |ui| {
            if let Some(id) = menu {
                menus::invoke_from_system_menu(app, ui.ctx(), id, json!({}));
            }
            app.logic(ui.ctx());
            app.ui(ui);
        });
        out.textures_delta.clear();
        out
    }

    fn frame(&mut self, events: Vec<Event>) -> egui::FullOutput {
        self.frame_with(events, None)
    }

    fn menu(&mut self, id: &str) -> egui::FullOutput {
        self.frame_with(vec![], Some(id))
    }

    fn selected(&self) -> usize {
        self.app.session.active().unwrap().selection.objects.len()
    }

    /// The panel number field showing `text` (the field keeps its text under its id): id, rect.
    fn field(&self, text: &str) -> (egui::Id, Rect) {
        let widgets: Vec<(egui::Id, Rect)> =
            self.ctx.viewport(|vp| vp.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).map(|w| (w.id, w.rect)).collect());
        widgets
            .into_iter()
            .find(|&(id, _)| {
                self.ctx.data(|d| d.get_temp::<String>(id)).is_some_and(|t| t == text) && egui::TextEdit::load_state(&self.ctx, id).is_some()
            })
            .unwrap_or_else(|| panic!("no field shows {text:?}"))
    }

    /// The selected character range of text field `id`.
    fn field_selection(&self, id: egui::Id) -> Option<(usize, usize)> {
        let r = egui::TextEdit::load_state(&self.ctx, id)?.cursor.char_range()?;
        let (a, b) = (r.primary.index, r.secondary.index);
        Some((a.min(b).into(), a.max(b).into()))
    }
}

fn press(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

fn click(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

#[test]
fn cmd_a_in_a_panel_field_selects_its_text_and_leaves_the_art() {
    let mut a = App::new();
    assert_eq!(a.selected(), 1);
    let (id, rect) = a.field("37 pt");
    a.frame(vec![Event::PointerMoved(rect.center()), click(rect.center(), true)]);
    a.frame(vec![click(rect.center(), false)]);
    a.frame(vec![]);
    assert!(a.ctx.text_edit_focused(), "the W field has the keyboard");
    let all = Some((0, "37 pt".chars().count()));
    // The window's Cmd+A (Windows, Linux, the web, macOS without the system menu).
    a.frame(vec![press(Key::End, Modifiers::NONE)]);
    assert_eq!(a.field_selection(id), Some((5, 5)), "End collapses the selection");
    a.frame(vec![press(Key::A, Modifiers::COMMAND)]);
    assert_eq!(a.field_selection(id), all, "Cmd+A selects the field's text");
    assert_eq!(a.selected(), 1, "and not the art");
    // Select All from the macOS menu bar (its key equivalent or a click): the same.
    a.frame(vec![press(Key::End, Modifiers::NONE)]);
    a.menu("select.all");
    a.frame(vec![]);
    assert_eq!(a.field_selection(id), all, "the menu's Select All selects the field's text");
    assert_eq!(a.selected(), 1, "and not the art");
    // The menu's Copy copies the field's text, not the art.
    let out = a.menu("edit.copy");
    let copied: Vec<_> = out
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(copied, ["37 pt"]);
    assert!(a.app.session.clipboard.is_empty(), "no art copied");
    assert!(a.ctx.text_edit_focused(), "the field keeps the keyboard");
}

#[test]
fn cmd_a_while_editing_type_selects_all_of_its_text() {
    let mut a = App::new();
    a.app.select_tool("type");
    let view = a.app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up] {
        a.app.session.pointer(&PointerEvent::new(kind, 100.0, 200.0), view).unwrap();
    }
    a.frame(vec![Event::Text("abc".into())]);
    let text = a.app.session.active().unwrap().selection.objects.clone();
    let range = |a: &App| {
        let o = a.app.session.tool_options();
        (o["start"].as_u64().unwrap(), o["end"].as_u64().unwrap().min(3))
    };
    assert_eq!(range(&a), (3, 3), "the caret after the text");
    // The window's Cmd+A.
    a.frame(vec![press(Key::A, Modifiers::COMMAND)]);
    assert_eq!(range(&a), (0, 3), "Cmd+A selects the text being edited");
    assert_eq!(a.app.session.active().unwrap().selection.objects, text, "and not the art");
    a.frame(vec![press(Key::End, Modifiers::NONE)]);
    assert_eq!(range(&a), (3, 3));
    // The macOS menu bar's Select All.
    a.menu("select.all");
    assert_eq!(range(&a), (0, 3), "the menu's Select All selects the text being edited");
    assert_eq!(a.app.session.active().unwrap().selection.objects, text);
    // Typing replaces it.
    a.frame(vec![Event::Text("Z".into())]);
    let doc = a.app.session.execute("document.inspect", &json!({})).unwrap();
    let names: Vec<_> = doc["layers"][0]["children"].as_array().unwrap().iter().map(|c| c["name"].clone()).collect();
    assert!(names.contains(&json!("Z")), "{names:?}");
}

#[test]
fn cmd_a_on_the_canvas_selects_all_objects() {
    let mut a = App::new();
    assert!(!a.ctx.egui_wants_keyboard_input());
    a.frame(vec![press(Key::A, Modifiers::COMMAND)]);
    assert_eq!(a.selected(), 2, "Cmd+A selects every object");
    a.app.run("select.none", json!({})).unwrap();
    a.menu("select.all");
    assert_eq!(a.selected(), 2, "so does the menu's Select All");
}

/// With the Artboard tool, Cmd+A after a click on an Artboards panel row selects every artboard
/// and leaves the active artboard and the art as they are; so does the macOS menu bar's Select All.
/// After a press on the canvas, with the panel closed, or with the Selection tool, Cmd+A selects
/// every object.
#[test]
fn cmd_a_after_a_click_in_the_artboards_panel_selects_every_artboard() {
    let mut a = App::new();
    for x in [500, 1000] {
        a.app.run("artboard.new", json!({"x": x, "y": 0, "width": 400, "height": 300})).unwrap();
    }
    a.app.run("window.panel", json!({"panel": "artboards"})).unwrap();
    a.app.select_tool("artboard");
    let picked = |a: &App| (crate::panels::artboards::picked(&a.app, 3), crate::panels::artboards::selected(&a.app, 3));
    // Artboard 2's row, while Artboard 1 is active (the Control bar and the navigator show its name).
    let row = |a: &mut App| {
        crate::panels::artboards::select(&mut a.app, 0);
        a.frame(vec![]);
        text_at(&a.frame(vec![]), "Artboard 2", false)
    };
    let at = row(&mut a);
    click_at(&mut a, at);
    assert_eq!(picked(&a), (vec![1], 1));
    let art = a.selected();
    a.frame(vec![press(Key::A, Modifiers::COMMAND)]);
    assert_eq!(picked(&a), (vec![0, 1, 2], 1), "Cmd+A selects every artboard, and Artboard 2 stays active");
    assert_eq!(a.selected(), art, "and not the art");
    // So does Select All in the macOS menu bar.
    click_at(&mut a, at);
    assert_eq!(picked(&a), (vec![1], 1));
    a.menu("select.all");
    assert_eq!(picked(&a), (vec![0, 1, 2], 1), "the menu's Select All selects every artboard");
    assert_eq!(a.selected(), art, "and not the art");
    // A press on the canvas gives the keys back to the art.
    let canvas = a.app.canvas_rect.unwrap();
    click_at(&mut a, canvas.left_top() + vec2(20.0, 20.0));
    a.frame(vec![press(Key::A, Modifiers::COMMAND)]);
    assert_eq!(a.selected(), 2, "after a press on the canvas, Cmd+A selects every object");
    // So does closing the panel after the click on a row.
    a.app.run("select.none", json!({})).unwrap();
    let at = row(&mut a);
    click_at(&mut a, at);
    a.app.run("window.panel", json!({"panel": "artboards", "toggle": true})).unwrap();
    a.frame(vec![]);
    a.frame(vec![press(Key::A, Modifiers::COMMAND)]);
    assert_eq!(a.selected(), 2, "with the panel closed, Cmd+A selects every object");
    // With the Selection tool, Cmd+A after a click on a row selects every object.
    a.app.run("select.none", json!({})).unwrap();
    a.app.run("window.panel", json!({"panel": "artboards"})).unwrap();
    a.app.select_tool("selection");
    let at = row(&mut a);
    click_at(&mut a, at);
    a.frame(vec![press(Key::A, Modifiers::COMMAND)]);
    assert_eq!(a.selected(), 2, "with the Selection tool, Cmd+A selects every object");
}

/// The copied texts of a frame's output.
fn copied(out: &egui::FullOutput) -> Vec<String> {
    out.platform_output.commands.iter().filter_map(|c| if let egui::OutputCommand::CopyText(t) = c { Some(t.clone()) } else { None }).collect()
}

/// #516: Ctrl/Cmd+C, X and V in a focused field (the window's own clipboard keys, which egui sends
/// as Copy, Cut and Paste events) edit the field's text, never the art.
#[test]
fn the_clipboard_keys_in_a_focused_field_edit_its_text() {
    let mut a = App::new();
    a.app.run("window.panel", json!({"panel": "color"})).unwrap();
    for _ in 0..3 {
        a.frame(vec![]);
    }
    for text in ["37 pt", "FFFFFF"] {
        let (id, rect) = a.field(text);
        a.frame(vec![Event::PointerMoved(rect.center()), click(rect.center(), true)]);
        a.frame(vec![click(rect.center(), false)]);
        a.frame(vec![press(Key::A, Modifiers::COMMAND)]);
        assert!(a.ctx.memory(|m| m.has_focus(id)), "{text}: the field has the keyboard");
        let out = a.frame(vec![Event::Copy]);
        assert_eq!(copied(&out), [text], "Copy copies the field's text");
        assert!(a.app.session.clipboard.is_empty(), "{text}: no art copied");
        let out = a.frame(vec![Event::Cut]);
        assert_eq!(copied(&out), [text], "Cut cuts the field's text");
        a.frame(vec![Event::Paste("12".into())]);
        assert_eq!(a.ctx.data(|d| d.get_temp::<String>(id)).as_deref(), Some("12"), "Paste pastes into the field");
        assert_eq!(a.selected(), 1, "{text}: the art stays");
        a.frame(vec![press(Key::Escape, Modifiers::NONE)]);
    }
}

/// The centre of the text `label` painted last (in the topmost layer) in a frame's output; with
/// `title`, the first one painted (the menu bar's).
fn text_at(out: &egui::FullOutput, label: &str, title: bool) -> Pos2 {
    let mut shapes: Box<dyn Iterator<Item = _>> = if title { Box::new(out.shapes.iter()) } else { Box::new(out.shapes.iter().rev()) };
    shapes
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text().trim() == label => Some(Rect::from_min_size(t.pos, t.galley.size()).center()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {label:?} painted"))
}

/// A click on `at` → the frame after it.
fn click_at(a: &mut App, at: Pos2) -> egui::FullOutput {
    a.frame(vec![Event::PointerMoved(at), click(at, true)]);
    a.frame(vec![click(at, false)]);
    a.frame(vec![])
}

/// The in-window menus' Select › All, Edit › Copy and Edit › Paste act on the text field that had the keyboard
/// when the menu was opened (the click on the menu took it), and give it the keyboard back.
#[test]
fn the_in_window_edit_menu_acts_on_the_field_it_was_opened_from() {
    let mut a = App::new();
    // Text on the system clipboard.
    a.app.services.clipboard_read = Some(Box::new(|| Some("42 pt".into())));
    let (id, rect) = a.field("37 pt");
    a.frame(vec![Event::PointerMoved(rect.center()), click(rect.center(), true)]);
    a.frame(vec![click(rect.center(), false)]);
    let out = a.frame(vec![press(Key::End, Modifiers::NONE)]);
    let (edit, select) = (text_at(&out, "Edit", true), text_at(&out, "Select", true));
    let menu = |a: &mut App, title: Pos2, item: &str| {
        let out = click_at(a, title);
        assert!(!a.ctx.text_edit_focused(), "the menu took the keyboard");
        click_at(a, text_at(&out, item, false))
    };
    menu(&mut a, select, "All");
    assert!(a.ctx.memory(|m| m.has_focus(id)), "the field has the keyboard back");
    assert_eq!(a.field_selection(id), Some((0, 5)), "Select All selected the field's text");
    assert_eq!(a.selected(), 1, "not the art");
    a.frame(vec![press(Key::End, Modifiers::NONE)]);
    a.frame(vec![press(Key::A, Modifiers::COMMAND)]);
    a.frame(vec![]);
    let out = menu(&mut a, edit, "Copy");
    assert_eq!(copied(&out), ["37 pt"], "Copy copied the field's text");
    assert!(a.app.session.clipboard.is_empty(), "no art copied");
    // Art on the app's clipboard (Paste is enabled): the field still takes the text.
    a.app.run("edit.copy", json!({})).unwrap();
    a.frame(vec![press(Key::A, Modifiers::COMMAND)]);
    menu(&mut a, edit, "Paste");
    assert_eq!(a.ctx.data(|d| d.get_temp::<String>(id)).as_deref(), Some("42 pt"), "Paste pasted into the field");
    assert_eq!(a.selected(), 1, "nothing pasted on the canvas");
    // With no field focused, the menu's Select All selects the art.
    a.frame(vec![press(Key::Escape, Modifiers::NONE)]);
    a.frame(vec![]);
    menu(&mut a, select, "All");
    assert_eq!(a.selected(), 2);
}
