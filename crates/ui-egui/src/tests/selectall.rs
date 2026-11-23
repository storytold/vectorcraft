//! Select All (Cmd+A) follows the keyboard, from the window and from the system menu bar (macOS):
//! a focused text field selects its own text, the Type tool editing selects the text being edited,
//! the Artboards panel selects every artboard after a click on one of its rows with the Artboard
//! tool, and only otherwise are all the objects selected. The clipboard keys follow it too: Copy
//! and Cut copy the text selected in a field or in a panel's label before the art.

use egui::accesskit::{Node, NodeId};
use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, pos2, vec2};
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::VectorcraftApp;

struct App {
    app: VectorcraftApp,
    ctx: egui::Context,
    time: f64,
    /// The window has the keyboard.
    focused: bool,
    /// The accessibility nodes of the latest frame, and the one with the keyboard.
    nodes: Vec<(NodeId, Node)>,
    focus: NodeId,
}

impl App {
    /// A document with a 37 pt wide rectangle (selected) and a second, unselected one.
    fn new() -> Self {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        Self::over(app)
    }

    /// The same document over `app`, which already has a new document.
    fn over(mut app: VectorcraftApp) -> Self {
        app.run("shape.rectangle", json!({"x": 200, "y": 150, "width": 20, "height": 20})).unwrap();
        app.run("shape.rectangle", json!({"x": 20, "y": 20, "width": 37, "height": 40})).unwrap();
        let mut a = Self::bare(app);
        for _ in 0..3 {
            a.frame(vec![]);
        }
        a
    }

    /// `app` in a window of its own, with the access tree on; no frame has run.
    fn bare(app: VectorcraftApp) -> Self {
        let ctx = crate::testing::ui_context();
        ctx.enable_accesskit();
        Self { app, ctx, time: 0.0, focused: true, nodes: Vec::new(), focus: NodeId(0) }
    }

    /// One whole app frame with `events`, as the hosts run it; `menu`: a system menu item chosen first, as the macOS
    /// host dispatches it before the app's logic.
    fn frame_with(&mut self, events: Vec<Event>, menu: Option<&str>) -> egui::FullOutput {
        self.time += 0.1;
        let raw = egui::RawInput {
            events,
            time: Some(self.time),
            focused: self.focused,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 850.0))),
            ..Default::default()
        };
        let mut out = crate::testing::part_frame(&mut self.app, &self.ctx, raw, |app, ui| {
            if let Some(id) = menu {
                craft::ui::runtime::commands::fire(ui.ctx(), craft::keymap::CommandRef::new(id), craft::ui::runtime::commands::FireSource::Menu);
            }
            app.tick(ui.ctx());
            app.ui(ui);
        });
        if let Some(update) = out.platform_output.accesskit_update.take() {
            self.focus = update.focus;
            self.nodes = update.nodes;
        }
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

    /// The node of `id` in the latest frame.
    fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.iter().find(|(n, _)| *n == id).map(|(_, node)| node)
    }

    /// The text field showing `text`: its node and rect. A number field's value is its text and the unit painted after it.
    fn field(&self, text: &str) -> (NodeId, Rect) {
        let shows = |value: &str| value.strip_prefix(text).is_some_and(|unit| unit.is_empty() || unit.starts_with(' '));
        self.nodes
            .iter()
            .find(|(_, n)| !n.is_hidden() && n.supports_action(egui::accesskit::Action::SetValue) && n.value().is_some_and(shows))
            .and_then(|(id, n)| n.bounds().map(|b| (*id, Rect::from_min_max(pos2(b.x0 as f32, b.y0 as f32), pos2(b.x1 as f32, b.y1 as f32)))))
            .unwrap_or_else(|| {
                let shown: Vec<_> =
                    self.nodes.iter().filter(|&(_, n)| n.supports_action(egui::accesskit::Action::SetValue)).map(|(_, n)| n.value()).collect();
                panic!("no field shows {text:?} in {shown:?}")
            })
    }

    /// The text field `id` shows.
    fn field_text(&self, id: NodeId) -> Option<&str> {
        self.node(id)?.value()
    }

    /// Whether field `id` has the keyboard.
    fn has_keyboard(&self, id: NodeId) -> bool {
        self.focus == id
    }

    /// The selected character range of text field `id` (its text is one run).
    fn field_selection(&self, id: NodeId) -> Option<(usize, usize)> {
        let r = self.node(id)?.text_selection()?;
        let (a, b) = (r.anchor.character_index, r.focus.character_index);
        Some((a.min(b), a.max(b)))
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
    // The W field shows its number, with its unit painted after it.
    let (id, rect) = a.field("37");
    a.frame(vec![Event::PointerMoved(rect.center()), click(rect.center(), true)]);
    a.frame(vec![click(rect.center(), false)]);
    a.frame(vec![]);
    assert!(a.has_keyboard(id), "the W field has the keyboard");
    let all = Some((0, "37".chars().count()));
    // The window's Cmd+A (Windows, Linux, the web, macOS without the system menu).
    a.frame(vec![press(Key::End, Modifiers::NONE)]);
    assert_eq!(a.field_selection(id), Some((2, 2)), "End collapses the selection");
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
    // A fired command reaches the field in the next pass.
    let after = a.frame(vec![]);
    assert_eq!([copied(&out), copied(&after)].concat(), ["37"]);
    assert!(a.app.session.clipboard.is_empty(), "no art copied");
    assert!(a.has_keyboard(id), "the field keeps the keyboard");
}

#[test]
fn cmd_a_while_editing_type_selects_all_of_its_text() {
    let mut a = App::new();
    a.app.select_tool("type");
    let view = a.app.view_info();
    for kind in [PointerKind::Down, PointerKind::Up] {
        a.app.session.pointer(&PointerEvent::new(kind, 100.0, 200.0), view).unwrap();
    }
    // The frame of the click: the edit's text part appears and takes the keys.
    a.frame(vec![]);
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
    let canvas = a.app.canvas_view().unwrap().rect;
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
    for text in ["37", "FFFFFF"] {
        let (id, rect) = a.field(text);
        a.frame(vec![Event::PointerMoved(rect.center()), click(rect.center(), true)]);
        a.frame(vec![click(rect.center(), false)]);
        a.frame(vec![press(Key::A, Modifiers::COMMAND)]);
        assert!(a.has_keyboard(id), "{text}: the field has the keyboard");
        let out = a.frame(vec![Event::Copy]);
        assert_eq!(copied(&out), [text], "Copy copies the field's text");
        assert!(a.app.session.clipboard.is_empty(), "{text}: no art copied");
        let out = a.frame(vec![Event::Cut]);
        assert_eq!(copied(&out), [text], "Cut cuts the field's text");
        a.frame(vec![Event::Paste("12".into())]);
        // A number field's value is its text and the unit painted after it.
        let text = a.field_text(id).unwrap_or_default();
        assert!(text.strip_prefix("12").is_some_and(|unit| unit.is_empty() || unit.starts_with(' ')), "Paste pastes into the field: {text:?}");
        assert_eq!(a.selected(), 1, "{text}: the art stays");
        a.frame(vec![press(Key::Escape, Modifiers::NONE)]);
    }
}

/// The rectangle of the text `text` painted last in a frame's output.
fn text_rect(out: &egui::FullOutput, text: &str) -> Rect {
    out.shapes
        .iter()
        .rev()
        .find_map(|s| match &s.shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.galley.rect.translate(t.pos.to_vec2())),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no {text:?} painted"))
}

/// Document Info open on its Objects category, after a few frames to lay it out: the last frame.
fn show_document_info(a: &mut App) -> egui::FullOutput {
    a.app.ui.doc_info.category = "objects".into();
    a.app.run("window.panel", json!({"panel": "docInfo"})).unwrap();
    let mut out = a.frame(vec![]);
    for _ in 0..3 {
        out = a.frame(vec![]);
    }
    out
}

/// Drag across the label text `text` painted in `out`, selecting it: the frame after the drag.
fn select_label_text(a: &mut App, out: &egui::FullOutput, text: &str) -> egui::FullOutput {
    let r = text_rect(out, text);
    let (from, to) = (r.left_center() + vec2(1.0, 0.0), r.right_center() - vec2(1.0, 0.0));
    a.frame(vec![Event::PointerMoved(from)]);
    a.frame(vec![click(from, true)]);
    for k in 1..=4 {
        a.frame(vec![Event::PointerMoved(from + (to - from) * k as f32 / 4.0)]);
    }
    a.frame(vec![click(to, false)]);
    a.frame(vec![])
}

/// Cmd+C or Cmd+X (`e`): the chord's frame, then the frame after it, when the keys are let go.
fn clipboard_chord(a: &mut App, e: Event) -> (egui::FullOutput, egui::FullOutput) {
    let out = a.frame(vec![Event::ModifiersChanged(Modifiers::COMMAND), e]);
    (out, a.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]))
}

/// #8432: with a Document Info value's text selected, Copy and Cut (Cmd+C and Cmd+X, F3 and F2,
/// or the menu bar's items) copy that text and leave the art. When the selected label is not
/// drawn, Copy copies the art.
#[test]
fn copy_and_cut_copy_the_text_selected_in_a_panel_label() {
    let mut a = App::new();
    let out = show_document_info(&mut a);
    // The two rectangles' outlines: 2 × (20 + 20) + 2 × (37 + 40).
    let length = "234 pt";
    let select = |a: &mut App, out: &egui::FullOutput| select_label_text(a, out, length);
    let chord = clipboard_chord;
    let out = select(&mut a, &out);
    let (copy, after) = chord(&mut a, Event::Copy);
    assert_eq!(copied(&copy), [length], "Copy copies the selected value");
    assert!(copied(&after).is_empty(), "and nothing after it");
    assert!(a.app.session.clipboard.is_empty(), "no art copied");
    let (cut, after) = chord(&mut a, Event::Cut);
    assert_eq!(copied(&cut), [length], "Cut copies the selected value");
    assert!(copied(&after).is_empty(), "and nothing after it");
    assert_eq!(a.selected(), 1, "and leaves the art");
    assert!(a.app.session.clipboard.is_empty(), "no art cut");
    // The menu bar's Edit › Copy and Edit › Cut (macOS).
    for (id, what) in [("edit.copy", "Copy"), ("edit.cut", "Cut")] {
        let menu = a.menu(id);
        // A fired command reaches the selected label in the next pass.
        let after = a.frame(vec![]);
        assert_eq!([copied(&menu), copied(&after)].concat(), [length], "the menu's {what} copies the selected value, once");
        assert_eq!(a.selected(), 1, "the menu's {what} leaves the art");
        assert!(a.app.session.clipboard.is_empty(), "no art copied or cut");
    }
    // Tab hides the panels: the selected value is not drawn, so Copy copies the art, a frame later,
    // even when the window has lost the keyboard by then.
    select(&mut a, &out);
    a.frame(vec![press(Key::Tab, Modifiers::NONE)]);
    assert!(!crate::panel_docking::panels_shown(&a.app), "Tab hid the panels");
    let copy = a.frame(vec![Event::ModifiersChanged(Modifiers::COMMAND), Event::Copy]);
    a.focused = false;
    let after = a.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    a.focused = true;
    assert!(copied(&copy).is_empty(), "no label copied {length}");
    assert!(copied(&after).iter().any(|t| t.starts_with("<?xml")), "Copy copied the art");
}

/// Drag across a label in a window of its own (a floating panel's), selecting its text there. The
/// main window's labels then copy nothing on Copy or Cut, which run on the art a pass later.
fn select_label_text_in_another_window(a: &mut App) {
    let window = egui::ViewportId::from_hash_of("floating panel");
    let rect = std::cell::Cell::new(Rect::NOTHING);
    let pass = |a: &mut App, events: Vec<Event>| {
        a.time += 0.1;
        let screen = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0));
        let mut input = egui::RawInput { viewport_id: window, events, time: Some(a.time), screen_rect: Some(screen), ..Default::default() };
        input.viewports.insert(window, egui::ViewportInfo { parent: Some(egui::ViewportId::ROOT), ..Default::default() });
        a.ctx.run_ui(input, |ui| rect.set(ui.label("234 pt").rect)).drop_without_applying_deltas();
    };
    pass(a, vec![]);
    let (from, to) = (rect.get().left_center() + vec2(1.0, 0.0), rect.get().right_center() - vec2(1.0, 0.0));
    pass(a, vec![Event::PointerMoved(from)]);
    pass(a, vec![click(from, true)]);
    for k in 1..=4 {
        pass(a, vec![Event::PointerMoved(from + (to - from) * k as f32 / 4.0)]);
    }
    pass(a, vec![click(to, false)]);
    pass(a, vec![]);
}

/// The objects of the first layer of the active document.
fn objects(a: &App) -> usize {
    a.app.session.active().unwrap().doc.layers[0].children().unwrap().len()
}

/// #8432: with text selected in another window's label, Copy and then Cut in the main window copy
/// the art and then cut it: that window's text is not the main window's label selection.
#[test]
fn copy_then_cut_with_text_selected_in_another_window_copy_and_cut_the_art() {
    let mut a = App::new();
    select_label_text_in_another_window(&mut a);
    let copy = a.frame(vec![Event::ModifiersChanged(Modifiers::COMMAND), Event::Copy]);
    let cut = a.frame(vec![Event::ModifiersChanged(Modifiers::COMMAND), Event::Cut]);
    a.frame(vec![Event::ModifiersChanged(Modifiers::NONE)]);
    assert!(copied(&copy).iter().chain(&copied(&cut)).all(|t| t != "234 pt"), "the other window's text was not copied");
    assert!(copied(&copy).iter().chain(&copied(&cut)).any(|t| t.starts_with("<?xml")), "Copy copied the art");
    assert_eq!(objects(&a), 1, "Cut cut the selected object");
}

/// #8432: Copy or Cut with a Document Info value's text selected, by its keys or the menu bar's
/// item, drops the formats an earlier Copy of the art has not written to the system clipboard yet,
/// so they do not replace the copied value.
#[test]
fn panel_text_copied_before_the_art_formats_are_written_stays_on_the_clipboard() {
    // `menu`: the menu bar's item (macOS) while the window does not have the keyboard, else the
    // keys (Cmd+C, Cmd+X).
    for (menu, cut) in [(false, false), (false, true), (true, false), (true, true)] {
        let (id, event) = if cut { ("edit.cut", Event::Cut) } else { ("edit.copy", Event::Copy) };
        let (app, board, hold) = super::sysclip::app_with_busy_worker("");
        let mut a = App::bare(app);
        super::sysclip::copy_rect(&mut a.app);
        let out = show_document_info(&mut a);
        // The rectangle's outline: 2 × (40 + 20).
        let length = "120 pt";
        select_label_text(&mut a, &out, length);
        let copy = if menu {
            a.focused = false;
            let copy = a.menu(id);
            a.focused = true;
            copy
        } else {
            clipboard_chord(&mut a, event).0
        };
        assert_eq!(copied(&copy), [length], "{id} copies the selected value (menu: {menu})");
        assert_eq!(a.selected(), 1, "{id} leaves the art (menu: {menu})");
        drop(hold);
        crate::background::wait_all(&mut a.app);
        assert_eq!(super::sysclip::writes(&board), 0, "the art's formats were not written over it ({id}, menu: {menu})");
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
    let (app, board) = super::sysclip::app();
    let mut a = App::over(app);
    // Text on the system clipboard.
    super::sysclip::copy_elsewhere(&board, vec![(vectorcraft_engine::cmd::clipboard::TEXT, b"42 pt".to_vec())]);
    let (id, rect) = a.field("37");
    a.frame(vec![Event::PointerMoved(rect.center()), click(rect.center(), true)]);
    a.frame(vec![click(rect.center(), false)]);
    let out = a.frame(vec![press(Key::End, Modifiers::NONE)]);
    let (edit, select) = (text_at(&out, "Edit", true), text_at(&out, "Select", true));
    let menu = |a: &mut App, title: Pos2, item: &str| {
        let out = click_at(a, title);
        assert!(!a.has_keyboard(id), "the menu took the keyboard");
        click_at(a, text_at(&out, item, false))
    };
    menu(&mut a, select, "All");
    assert!(a.has_keyboard(id), "the field has the keyboard back");
    assert_eq!(a.field_selection(id), Some((0, 2)), "Select All selected the field's text");
    assert_eq!(a.selected(), 1, "not the art");
    a.frame(vec![press(Key::End, Modifiers::NONE)]);
    a.frame(vec![press(Key::A, Modifiers::COMMAND)]);
    a.frame(vec![]);
    let out = menu(&mut a, edit, "Copy");
    assert_eq!(copied(&out), ["37"], "Copy copied the field's text");
    assert!(a.app.session.clipboard.is_empty(), "no art copied");
    // Art on the app's clipboard (Paste is enabled): the field still takes the text.
    a.app.run("edit.copy", json!({})).unwrap();
    a.frame(vec![press(Key::A, Modifiers::COMMAND)]);
    menu(&mut a, edit, "Paste");
    assert_eq!(a.field_text(id), Some("42 pt"), "Paste pasted into the field");
    assert_eq!(a.selected(), 1, "nothing pasted on the canvas");
    // With no field focused, the menu's Select All selects the art.
    a.frame(vec![press(Key::Escape, Modifiers::NONE)]);
    a.frame(vec![]);
    menu(&mut a, select, "All");
    assert_eq!(a.selected(), 2);
}
