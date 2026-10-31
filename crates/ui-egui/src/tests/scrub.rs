//! Scrubbing numeric fields by dragging their label (#400): steps, modifiers, clamps, Escape, the
//! preference, and one undo step in the app.

use std::cell::Cell;

use egui::{CursorIcon, Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use serde_json::json;
use vectorcraft_doc::Unit;
use vectorcraft_engine::{DocState, Session};

use crate::{VectorcraftApp, scrub, widgets};

/// Headless frames of one labelled field: where its label and its box were last drawn.
struct Row {
    ctx: egui::Context,
    time: f64,
    label: Cell<Rect>,
    field: Cell<Rect>,
}

impl Row {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        Self { ctx, time: 0.0, label: Cell::new(Rect::NOTHING), field: Cell::new(Rect::NOTHING) }
    }

    /// One frame with `events`, the keyboard holding `modifiers`; `draw` draws the field after
    /// the label "W:". → What the field returned and the pointer's cursor.
    fn frame(&mut self, events: Vec<Event>, modifiers: Modifiers, draw: &dyn Fn(&mut egui::Ui) -> Option<f64>) -> (Option<f64>, CursorIcon) {
        self.time += 0.05;
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0)));
        let events = std::iter::once(Event::ModifiersChanged(modifiers)).chain(events).collect();
        let input = egui::RawInput { events, time: Some(self.time), screen_rect, focused: true, ..Default::default() };
        let mut got = None;
        let mut out = self.ctx.run_ui(input, |ui| {
            ui.horizontal(|ui| {
                self.label.set(widgets::dim_label(ui, "W:").rect);
                let before = ui.cursor().min;
                got = draw(ui);
                self.field.set(Rect::from_min_max(before, ui.min_rect().max));
            });
        });
        out.textures_delta.clear();
        (got, out.platform_output.cursor_icon)
    }

    /// Drag from `from` by `dx` points in `steps` moves (a frame each) with `modifiers` held, then
    /// release (`escape`: press Escape before releasing). → The values the field returned.
    fn drag(
        &mut self,
        from: Pos2,
        dx: f32,
        steps: usize,
        modifiers: Modifiers,
        escape: bool,
        draw: &dyn Fn(&mut egui::Ui) -> Option<f64>,
    ) -> Vec<f64> {
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers };
        let mut got = vec![];
        let mut push = |r: (Option<f64>, CursorIcon)| got.extend(r.0);
        push(self.frame(vec![Event::PointerMoved(from)], modifiers, draw));
        push(self.frame(vec![button(from, true)], modifiers, draw));
        let mut at = from;
        for _ in 0..steps {
            at.x += dx / steps as f32;
            push(self.frame(vec![Event::PointerMoved(at)], modifiers, draw));
        }
        if escape {
            let esc = Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers };
            push(self.frame(vec![esc], modifiers, draw));
        }
        push(self.frame(vec![button(at, false)], modifiers, draw));
        push(self.frame(vec![], Modifiers::NONE, draw));
        got
    }
}

/// A W field in points over `width`, applying what it returns (as the panels do).
fn w_field(width: &Cell<f64>) -> impl Fn(&mut egui::Ui) -> Option<f64> + '_ {
    |ui| widgets::num_field(ui, "w", Some(width.get()), Unit::Points, 80.0).inspect(|&v| width.set(v))
}

#[test]
fn dragging_the_label_steps_the_field_with_shift_and_ctrl() {
    for (mods, dx, want) in [
        // One unit per PX_PER_STEP points, ten with Shift, a tenth with Ctrl/Cmd; left goes down.
        (Modifiers::NONE, 40.0, 100.0 + 40.0 / scrub::PX_PER_STEP as f64),
        (Modifiers::NONE, -40.0, 100.0 - 40.0 / scrub::PX_PER_STEP as f64),
        (Modifiers::SHIFT, 40.0, 100.0 + 400.0 / scrub::PX_PER_STEP as f64),
        (Modifiers::COMMAND, 40.0, 100.0 + 4.0 / scrub::PX_PER_STEP as f64),
    ] {
        let mut row = Row::new();
        let width = Cell::new(100.0);
        let draw = w_field(&width);
        row.frame(vec![], Modifiers::NONE, &draw);
        let got = row.drag(row.label.get().center(), dx, 4, mods, false, &draw);
        assert!((width.get() - want).abs() < 1e-9, "{mods:?} {dx}: {} (values {got:?})", width.get());
        assert!(got.len() > 1, "the values come live while dragging: {got:?}");
    }
}

#[test]
fn hovering_the_label_shows_the_resize_cursor_and_clicking_it_focuses_the_field() {
    let mut row = Row::new();
    let width = Cell::new(100.0);
    let draw = w_field(&width);
    row.frame(vec![], Modifiers::NONE, &draw);
    let (_, cursor) = row.frame(vec![Event::PointerMoved(row.label.get().center())], Modifiers::NONE, &draw);
    assert_eq!(cursor, CursorIcon::ResizeHorizontal);
    // The text box keeps the text cursor.
    let (_, cursor) = row.frame(vec![Event::PointerMoved(row.field.get().center())], Modifiers::NONE, &draw);
    assert_eq!(cursor, CursorIcon::Text);
    // A click on the label (no drag) focuses the field, its text all selected.
    let at = row.label.get().center();
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    row.frame(vec![Event::PointerMoved(at), button(true), button(false)], Modifiers::NONE, &draw);
    row.frame(vec![], Modifiers::NONE, &draw);
    let id = row.ctx.memory(|m| m.focused());
    assert!(id.is_some(), "the field has the focus");
    let st = egui::TextEdit::load_state(&row.ctx, id.unwrap()).unwrap();
    let r = st.cursor.char_range().unwrap().as_sorted_char_range();
    assert_eq!((r.start.0, r.end.0), (0, "100 pt".len()));
    assert_eq!(width.get(), 100.0);
}

#[test]
fn dragging_the_text_box_does_not_scrub() {
    let mut row = Row::new();
    let width = Cell::new(100.0);
    let draw = w_field(&width);
    row.frame(vec![], Modifiers::NONE, &draw);
    let got = row.drag(row.field.get().center(), 40.0, 4, Modifiers::NONE, false, &draw);
    assert_eq!(width.get(), 100.0, "{got:?}");
}

#[test]
fn a_field_without_a_label_does_not_scrub() {
    let mut row = Row::new();
    let width = Cell::new(100.0);
    // The label is there, but the field is far from it: not its label.
    let draw = |ui: &mut egui::Ui| {
        ui.add_space(120.0);
        widgets::num_field(ui, "w", Some(width.get()), Unit::Points, 80.0).inspect(|&v| width.set(v))
    };
    row.frame(vec![], Modifiers::NONE, &draw);
    let (_, cursor) = row.frame(vec![Event::PointerMoved(row.label.get().center())], Modifiers::NONE, &draw);
    assert_ne!(cursor, CursorIcon::ResizeHorizontal);
    row.drag(row.label.get().center(), 40.0, 4, Modifiers::NONE, false, &draw);
    assert_eq!(width.get(), 100.0);
}

/// The caller's limits hold, and dragging back changes the value at once (no dead zone past the
/// limit); counts step by whole numbers, Ctrl/Cmd only slows them.
#[test]
fn scrubbing_respects_limits_and_precision() {
    let mut row = Row::new();
    let opacity = Cell::new(95.0);
    let draw = |ui: &mut egui::Ui| widgets::plain_field(ui, "o", opacity.get(), "%", 0, 60.0).inspect(|&v| opacity.set(v.clamp(0.0, 100.0)));
    row.frame(vec![], Modifiers::NONE, &draw);
    let at = row.label.get().center();
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    row.frame(vec![Event::PointerMoved(at), button(at, true)], Modifiers::NONE, &draw);
    for x in [10.0, 20.0, 30.0, 40.0] {
        row.frame(vec![Event::PointerMoved(at + vec2(x, 0.0))], Modifiers::NONE, &draw);
    }
    assert_eq!(opacity.get(), 100.0, "clamped");
    row.frame(vec![Event::PointerMoved(at + vec2(36.0, 0.0))], Modifiers::NONE, &draw);
    assert_eq!(opacity.get(), 98.0, "back down at once");
    row.frame(vec![button(at + vec2(36.0, 0.0), false)], Modifiers::NONE, &draw);

    let mut row = Row::new();
    let count = Cell::new(3.0);
    let draw = |ui: &mut egui::Ui| widgets::plain_field(ui, "n", count.get(), "", 0, 60.0).inspect(|&v| count.set(v));
    row.frame(vec![], Modifiers::NONE, &draw);
    let got = row.drag(row.label.get().center(), 40.0, 4, Modifiers::COMMAND, false, &draw);
    assert_eq!(count.get(), 5.0, "{got:?}");
    assert!(got.iter().all(|v| v.fract() == 0.0), "{got:?}");

    // Units: a millimetre field steps whole millimetres.
    let mut row = Row::new();
    let pt = Cell::new(Unit::Millimeters.to_pt(10.0));
    let draw = |ui: &mut egui::Ui| widgets::num_field(ui, "mm", Some(pt.get()), Unit::Millimeters, 80.0).inspect(|&v| pt.set(v));
    row.frame(vec![], Modifiers::NONE, &draw);
    row.drag(row.label.get().center(), 20.0, 2, Modifiers::NONE, false, &draw);
    assert!((Unit::Millimeters.from_pt(pt.get()) - 20.0).abs() < 1e-9, "{}", Unit::Millimeters.from_pt(pt.get()));
}

#[test]
fn escape_puts_the_value_back_and_the_preference_turns_scrubbing_off() {
    let mut row = Row::new();
    let width = Cell::new(100.0);
    let draw = w_field(&width);
    row.frame(vec![], Modifiers::NONE, &draw);
    let got = row.drag(row.label.get().center(), 40.0, 4, Modifiers::NONE, true, &draw);
    assert_eq!(width.get(), 100.0, "{got:?}");
    assert!(got.len() > 1 && got.last() == Some(&100.0), "{got:?}");
    assert_eq!(scrub::phase(&row.ctx), scrub::Phase::Idle);

    scrub::set_enabled(&row.ctx, false);
    let (_, cursor) = row.frame(vec![Event::PointerMoved(row.label.get().center())], Modifiers::NONE, &draw);
    assert_ne!(cursor, CursorIcon::ResizeHorizontal);
    row.drag(row.label.get().center(), 40.0, 4, Modifiers::NONE, false, &draw);
    assert_eq!(width.get(), 100.0);
}

// ---------- in the app ----------

/// One frame of the whole window with `events`. → The rect of the first text drawn as `text`.
fn app_frame(app: &mut VectorcraftApp, ctx: &egui::Context, events: Vec<Event>, text: &str) -> Option<Rect> {
    let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(1600.0, 900.0)));
    let mut out = ctx.run_ui(egui::RawInput { events, screen_rect, ..Default::default() }, |ui| {
        app.logic(ui.ctx());
        app.ui(ui);
    });
    out.textures_delta.clear();
    fn find(shape: &egui::Shape, text: &str) -> Option<Rect> {
        match shape {
            egui::Shape::Text(t) if t.galley.text() == text => Some(t.galley.rect.translate(t.pos.to_vec2())),
            egui::Shape::Vec(v) => v.iter().find_map(|s| find(s, text)),
            _ => None,
        }
    }
    out.shapes.iter().find_map(|s| find(&s.shape, text))
}

fn app_with_rect() -> (VectorcraftApp, egui::Context) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    (app, egui::Context::default())
}

fn width(app: &VectorcraftApp) -> f64 {
    app.session.transform_box(&app.session.doc().unwrap().selection.objects).unwrap().rect.width()
}

fn undo_len(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

fn objects(doc: &DocState) -> usize {
    doc.doc.layers[0].children().map_or(0, |c| c.len())
}

/// Drag the W label by `dx` in four moves (`escape`: Escape before the release).
fn drag_w(app: &mut VectorcraftApp, ctx: &egui::Context, dx: f32, escape: bool) {
    let mut w = None;
    for _ in 0..4 {
        w = app_frame(app, ctx, vec![], "W:");
    }
    let at = w.expect("the W label is shown").center();
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    app_frame(app, ctx, vec![Event::PointerMoved(at)], "");
    app_frame(app, ctx, vec![button(at, true)], "");
    for i in 1..=4 {
        app_frame(app, ctx, vec![Event::PointerMoved(at + vec2(dx * i as f32 / 4.0, 0.0))], "");
    }
    if escape {
        app_frame(app, ctx, vec![Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }], "");
    }
    app_frame(app, ctx, vec![button(at + vec2(dx, 0.0), false)], "");
    app_frame(app, ctx, vec![], "");
}

/// A scrub of the W label resizes the selection live and is one undo step; Escape takes it back
/// without a step (and without deselecting); with the preference off the label doesn't scrub.
#[test]
fn scrubbing_w_in_the_app_is_one_undo_step() {
    let (mut app, ctx) = app_with_rect();
    let undo = undo_len(&app);
    drag_w(&mut app, &ctx, 40.0, false);
    assert!((width(&app) - (100.0 + 40.0 / scrub::PX_PER_STEP as f64)).abs() < 1e-6, "{}", width(&app));
    assert_eq!(undo_len(&app), undo + 1, "one undo step for the drag");
    app.run("edit.undo", json!({})).unwrap();
    assert!((width(&app) - 100.0).abs() < 1e-6);

    drag_w(&mut app, &ctx, 40.0, true);
    assert!((width(&app) - 100.0).abs() < 1e-6, "Escape: {}", width(&app));
    assert_eq!(undo_len(&app), undo, "no step");
    assert!(!app.session.doc().unwrap().selection.objects.is_empty(), "Escape didn't deselect");

    app.run("prefs.set", json!({"key": "scrubNumericFields", "value": false})).unwrap();
    drag_w(&mut app, &ctx, 40.0, false);
    assert!((width(&app) - 100.0).abs() < 1e-6, "off: {}", width(&app));
}

/// A scrub does not close an undo group that is open in another document, and that document keeps
/// its edits, when Escape ends the drag and when the drag is released.
#[test]
fn a_scrub_leaves_another_documents_undo_group_open() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    // The second document exists before the group begins, so no group opens in it.
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("document.activate", &json!({"index": 0})).unwrap();
    app.session.execute("edit.undoGroup.begin", &json!({})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 100, "height": 50})).unwrap();
    app.session.execute("document.activate", &json!({"index": 1})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    let ctx = egui::Context::default();
    let other = |app: &VectorcraftApp| (app.session.documents()[0].undo_group.is_some(), objects(&app.session.documents()[0]));
    drag_w(&mut app, &ctx, 40.0, true);
    assert!((width(&app) - 100.0).abs() < 1e-6, "Escape: {}", width(&app));
    assert_eq!(other(&app), (true, 1), "Escape");
    drag_w(&mut app, &ctx, 40.0, false);
    assert!((width(&app) - (100.0 + 40.0 / scrub::PX_PER_STEP as f64)).abs() < 1e-6, "{}", width(&app));
    assert_eq!(other(&app), (true, 1), "released");
}

/// A scrub in a document whose undo group is already open joins that group's undo step and
/// leaves the group open; Escape puts the document back as it was before the drag, with the edits
/// made in the group, and drops the drag's journal entries.
#[test]
fn a_scrub_inside_an_open_undo_group_joins_its_step() {
    let (mut app, ctx) = app_with_rect();
    app.run("edit.undoGroup.begin", json!({})).unwrap();
    app.run("shape.rectangle", json!({"x": 200, "y": 10, "width": 100, "height": 50})).unwrap();
    let undo = undo_len(&app);
    let (before, journal) = (app.session.doc().unwrap().doc.clone(), app.session.journal.len());
    drag_w(&mut app, &ctx, 40.0, true);
    assert!((width(&app) - 100.0).abs() < 1e-6, "Escape: {}", width(&app));
    assert!(std::sync::Arc::ptr_eq(&app.session.doc().unwrap().doc, &before), "Escape puts the document back");
    assert_eq!(app.session.journal.len(), journal, "and drops the drag's journal entries");
    assert_eq!(objects(app.session.doc().unwrap()), 2, "the rectangle drawn in the group stays");
    drag_w(&mut app, &ctx, 40.0, false);
    assert!((width(&app) - (100.0 + 40.0 / scrub::PX_PER_STEP as f64)).abs() < 1e-6, "{}", width(&app));
    assert!(app.session.doc().unwrap().undo_group.is_some(), "the group stays open");
    assert_eq!(undo_len(&app), undo, "the drag joins the group's step");
    app.run("edit.undoGroup.end", json!({})).unwrap();
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(objects(app.session.doc().unwrap()), 1, "one undo takes back the rectangle and the drag");
}

/// A scrub closes the undo group it opened in its own document when another document becomes
/// active during the drag (as `document.activate` does), and leaves that document's group open.
#[test]
fn a_scrub_closes_its_group_in_its_own_document_after_a_switch() {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    // The second document exists before the group begins, so no group opens in it.
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    app.session.execute("document.activate", &json!({"index": 0})).unwrap();
    app.session.execute("edit.undoGroup.begin", &json!({})).unwrap();
    app.session.execute("document.activate", &json!({"index": 1})).unwrap();
    app.session.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    let ctx = egui::Context::default();
    let undo = undo_len(&app);
    let mut w = None;
    for _ in 0..4 {
        w = app_frame(&mut app, &ctx, vec![], "W:");
    }
    let at = w.expect("the W label is shown").center();
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    app_frame(&mut app, &ctx, vec![Event::PointerMoved(at)], "");
    app_frame(&mut app, &ctx, vec![button(at, true)], "");
    for i in 1..=4 {
        app_frame(&mut app, &ctx, vec![Event::PointerMoved(at + vec2(10.0 * i as f32, 0.0))], "");
    }
    app.session.set_active(0);
    app_frame(&mut app, &ctx, vec![button(at + vec2(40.0, 0.0), false)], "");
    app_frame(&mut app, &ctx, vec![], "");
    let second = &app.session.documents()[1];
    assert!(second.undo_group.is_none(), "the scrub's group is closed");
    assert_eq!(second.history.undo.len(), undo + 1, "one undo step for the drag");
    assert!(app.session.documents()[0].undo_group.is_some(), "the active document's group stays open");
}
