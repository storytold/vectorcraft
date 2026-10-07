//! Numeric fields: math typed before the unit suffix, and the whole text (unit included) selected
//! on focus or double-click so typing replaces it.

use std::cell::Cell;

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use vectorcraft_doc::Unit;

use crate::widgets;

/// Headless frames of one field: its rect and id as last drawn.
struct Field {
    ctx: egui::Context,
    time: f64,
    rect: Cell<Rect>,
    id: Cell<egui::Id>,
}

impl Field {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        Self { ctx, time: 0.0, rect: Cell::new(Rect::NOTHING), id: Cell::new(egui::Id::NULL) }
    }

    fn frame(&mut self, mut events: Vec<Event>, draw: &dyn Fn(&mut egui::Ui) -> Option<f64>) -> Option<f64> {
        self.time += 0.05;
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0)));
        if let Some(modifiers) = events.iter().find_map(|event| match event {
            Event::Key { modifiers, .. } => Some(*modifiers),
            _ => None,
        }) {
            events.insert(0, Event::ModifiersChanged(modifiers));
        }
        let input = egui::RawInput { events, time: Some(self.time), screen_rect, ..Default::default() };
        let mut got = None;
        let mut out = self.ctx.run_ui(input, |ui| {
            self.id.set(ui.id().with("f"));
            got = draw(ui);
            self.rect.set(ui.min_rect());
        });
        out.textures_delta.clear();
        got
    }

    fn num(&mut self, events: Vec<Event>) -> Option<f64> {
        self.frame(events, &|ui| widgets::num_field(ui, "f", Some(100.0), Unit::Pixels, 80.0))
    }

    fn click(&self, count: usize) -> Vec<Event> {
        let at = self.rect.get().center();
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
        std::iter::once(Event::PointerMoved(at)).chain((0..count).flat_map(|_| [button(true), button(false)])).collect()
    }

    /// The selected character range of the field's text.
    fn selection(&self) -> Option<(usize, usize)> {
        let st = egui::TextEdit::load_state(&self.ctx, self.id.get())?;
        let r = st.cursor.char_range()?.as_sorted_char_range();
        Some((r.start.0, r.end.0))
    }
}

fn enter() -> Event {
    Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
}

fn key(key: Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

#[test]
fn math_before_the_unit_suffix_evaluates_in_every_unit() {
    assert_eq!(Unit::Pixels.parse("1080/2 px"), Some(540.0));
    assert_eq!(Unit::Points.parse("1080/2 px"), Some(540.0));
    assert_eq!(Unit::Pixels.parse("100*3 px"), Some(300.0));
    for u in Unit::ALL {
        let shown = u.format(u.to_pt(12.0));
        // "12 mm" with "*2" typed after the number, before the suffix.
        let typed = shown.replacen(&u.number(u.to_pt(12.0)), "12*2", 1);
        let v = u.parse(&typed).unwrap_or_else(|| panic!("{u:?}: `{typed}`"));
        assert!((v - u.to_pt(24.0)).abs() < 1e-9, "{u:?}: `{typed}` → {v}");
        let typed = shown.replacen(&u.number(u.to_pt(12.0)), "12/4", 1);
        let v = u.parse(&typed).unwrap_or_else(|| panic!("{u:?}: `{typed}`"));
        assert!((v - u.to_pt(3.0)).abs() < 1e-9, "{u:?}: `{typed}` → {v}");
    }
    // The suffix measures the expression; units given in it still win.
    assert_eq!(Unit::Points.parse("1in/2 mm"), Some(36.0));
    assert_eq!(Unit::Points.parse("10/0 px"), None);
    // Unitless fields.
    assert_eq!(vectorcraft_doc::parse_number("45*2"), Some(90.0));
    assert_eq!(vectorcraft_doc::parse_number(" 100 / 4 "), Some(25.0));
    assert_eq!(vectorcraft_doc::parse_number("+5"), Some(5.0));
    assert_eq!(vectorcraft_doc::parse_number("-5"), Some(-5.0));
    assert_eq!(vectorcraft_doc::parse_number("3 mm"), None);
    assert_eq!(vectorcraft_doc::parse_number("inf"), None);
}

#[test]
fn focus_selects_the_whole_field_so_typing_replaces_it() {
    let mut f = Field::new();
    f.num(vec![]);
    f.num(f.click(1));
    f.num(vec![]);
    assert_eq!(f.selection(), Some((0, "100 px".len())), "the unit is selected too");
    f.num(vec![Event::Text("1080/2".into())]);
    assert_eq!(f.num(vec![enter()]), Some(540.0));
}

#[test]
fn double_click_selects_the_whole_field() {
    let mut f = Field::new();
    f.num(vec![]);
    f.num(f.click(1));
    f.num(vec![]);
    // A click in the focused field (not part of a double-click) places the caret; a double-click
    // takes everything again.
    f.time += 1.0;
    f.num(f.click(1));
    f.num(vec![]);
    assert!(f.selection().is_some_and(|(a, b)| a == b), "{:?}", f.selection());
    f.num(f.click(2));
    f.num(vec![]);
    assert_eq!(f.selection(), Some((0, "100 px".len())));
}

#[test]
fn plain_fields_do_math_with_their_suffix() {
    let mut f = Field::new();
    let draw = |ui: &mut egui::Ui| widgets::plain_field(ui, "f", 10.0, "°", 2, 80.0);
    f.frame(vec![], &draw);
    f.frame(f.click(1), &draw);
    f.frame(vec![], &draw);
    assert_eq!(f.selection(), Some((0, "10°".chars().count())));
    f.frame(vec![Event::Text("45*2°".into())], &draw);
    assert_eq!(f.frame(vec![enter()], &draw), Some(90.0));
}

#[test]
fn arrows_step_length_fields_in_the_displayed_unit() {
    let mut f = Field::new();
    let value = Cell::new(Unit::Millimeters.to_pt(10.0));
    let draw = |ui: &mut egui::Ui| widgets::num_field(ui, "f", Some(value.get()), Unit::Millimeters, 80.0);
    f.frame(vec![], &draw);
    f.frame(f.click(1), &draw);
    f.frame(vec![], &draw);

    let next = f.frame(vec![key(Key::ArrowUp, Modifiers::NONE)], &draw).unwrap();
    assert!((next - Unit::Millimeters.to_pt(11.0)).abs() < 1e-9);
    value.set(next);

    let next = f.frame(vec![key(Key::ArrowDown, Modifiers { shift: true, ..Default::default() })], &draw).unwrap();
    assert!((next - Unit::Millimeters.to_pt(1.0)).abs() < 1e-9);
    value.set(next);

    let next = f.frame(vec![key(Key::ArrowDown, Modifiers { command: true, ..Default::default() })], &draw).unwrap();
    assert!((next - Unit::Millimeters.to_pt(0.9)).abs() < 1e-9);
}

#[test]
fn arrows_step_plain_and_empty_optional_fields_immediately() {
    let mut plain = Field::new();
    let value = Cell::new(20.0);
    let draw = |ui: &mut egui::Ui| widgets::plain_field(ui, "f", value.get(), "°", 2, 80.0);
    plain.frame(vec![], &draw);
    plain.frame(plain.click(1), &draw);
    plain.frame(vec![], &draw);
    assert_eq!(plain.frame(vec![key(Key::ArrowUp, Modifiers { ctrl: true, ..Default::default() })], &draw), Some(20.1));

    let mut optional = Field::new();
    let draw = |ui: &mut egui::Ui| widgets::opt_field(ui, "f", None, Unit::Points, 80.0).flatten();
    optional.frame(vec![], &draw);
    optional.frame(optional.click(1), &draw);
    optional.frame(vec![], &draw);
    assert_eq!(optional.frame(vec![key(Key::ArrowUp, Modifiers::NONE)], &draw), Some(1.0));
}
