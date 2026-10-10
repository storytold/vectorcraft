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
    /// Whether the window reports keyboard focus (the live app's doesn't always).
    focused: bool,
}

impl Field {
    fn new() -> Self {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        Self { ctx, time: 0.0, rect: Cell::new(Rect::NOTHING), id: Cell::new(egui::Id::NULL), focused: true }
    }

    fn frame(&mut self, events: Vec<Event>, draw: &dyn Fn(&mut egui::Ui) -> Option<f64>) -> Option<f64> {
        self.time += 0.05;
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0)));
        let input = egui::RawInput { events, time: Some(self.time), screen_rect, focused: self.focused, ..Default::default() };
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
    key(Key::Enter, Modifiers::NONE)
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

/// Preferences › Units › Numbers Without Units Are Points (#394): on (the default), a number typed
/// with no unit into a field in picas is read in points; a typed unit still wins (`2p6`, `1in`);
/// off, it is in picas. A field in any other unit reads it in its unit either way.
#[test]
fn bare_numbers_in_picas_fields_are_points_when_the_preference_says_so() {
    let mut f = Field::new();
    let typed = |f: &mut Field, u: Unit, text: &str| {
        let pt = Cell::new(u.to_pt(4.0));
        let draw = |ui: &mut egui::Ui| widgets::num_field(ui, "f", Some(pt.get()), u, 80.0).inspect(|&v| pt.set(v));
        f.frame(vec![], &draw);
        f.frame(f.click(1), &draw);
        f.frame(vec![Event::Text(text.into())], &draw);
        f.frame(vec![enter()], &draw)
    };
    let (pc, mm) = (Unit::Picas, Unit::Millimeters);
    assert_eq!(typed(&mut f, pc, "12"), Some(12.0), "on by default: points in a picas field");
    assert_eq!(typed(&mut f, pc, "2p6"), Some(30.0), "picas and points as typed");
    assert_eq!(typed(&mut f, pc, "1in"), Some(72.0), "a typed unit wins");
    assert_eq!(typed(&mut f, pc, "10+5"), Some(15.0), "arithmetic in points too");
    assert_eq!(typed(&mut f, mm, "12"), Some(mm.to_pt(12.0)), "a millimetre field keeps millimetres");
    widgets::set_bare_numbers_are_points(&f.ctx, false);
    assert_eq!(typed(&mut f, pc, "12"), Some(pc.to_pt(12.0)), "off: picas");
    assert_eq!(typed(&mut f, mm, "12"), Some(mm.to_pt(12.0)));
}

/// ↑/↓ step a focused numeric field by one of its unit (Shift: ten, Ctrl/Cmd: a tenth) and apply it
/// at once, the new value selected so typing replaces it; an unfocused field leaves the arrows to
/// the canvas (nudge).
#[test]
fn arrow_keys_step_a_focused_field() {
    let mut f = Field::new();
    let u = Unit::Millimeters;
    let mm = Cell::new(4.0);
    let draw = |ui: &mut egui::Ui| widgets::num_field(ui, "f", Some(u.to_pt(mm.get())), u, 80.0).inspect(|&v| mm.set(u.from_pt(v)));
    f.frame(vec![], &draw);
    assert_eq!(f.frame(vec![key(Key::ArrowUp, Modifiers::NONE)], &draw), None, "unfocused");
    f.frame(f.click(1), &draw);
    f.frame(vec![], &draw);
    // The window needn't report keyboard focus: the field has egui's.
    f.focused = false;
    for (k, mods, want) in [
        (Key::ArrowUp, Modifiers::NONE, 5.0),
        (Key::ArrowUp, Modifiers::SHIFT, 15.0),
        (Key::ArrowDown, Modifiers::COMMAND, 14.9),
        (Key::ArrowDown, Modifiers::NONE, 13.9),
    ] {
        assert!(f.frame(vec![key(k, mods)], &draw).is_some(), "{k:?} {mods:?}");
        assert!((mm.get() - want).abs() < 1e-6, "{k:?} {mods:?}: {}", mm.get());
    }
    assert_eq!(f.selection(), Some((0, "13.9 mm".len())), "the new value is selected");
    // Enter after the steps commits nothing more.
    assert_eq!(f.frame(vec![enter()], &draw), None);
}

/// Plain fields (degrees, percent, counts) step too, at their precision.
#[test]
fn arrow_keys_step_plain_fields() {
    let mut f = Field::new();
    let angle = |ui: &mut egui::Ui| widgets::plain_field(ui, "f", 10.0, "°", 2, 80.0);
    f.frame(vec![], &angle);
    f.frame(f.click(1), &angle);
    f.frame(vec![], &angle);
    assert_eq!(f.frame(vec![key(Key::ArrowUp, Modifiers::NONE)], &angle), Some(11.0));
    assert_eq!(f.frame(vec![key(Key::ArrowDown, Modifiers::SHIFT)], &angle), Some(1.0));
    // A count ignores the tenth.
    let mut f = Field::new();
    let count = |ui: &mut egui::Ui| widgets::plain_field(ui, "f", 3.0, "", 0, 80.0);
    f.frame(vec![], &count);
    f.frame(f.click(1), &count);
    f.frame(vec![], &count);
    assert_eq!(f.frame(vec![key(Key::ArrowUp, Modifiers::COMMAND)], &count), None);
    assert_eq!(f.frame(vec![key(Key::ArrowUp, Modifiers::NONE)], &count), Some(4.0));
}

/// #991: the Stroke weight spinner keeps the field's keyboard modifiers.
#[test]
fn stroke_spinner_arrow_keys_honor_shift() {
    let mut f = Field::new();
    let weight = Cell::new(14.0);
    let draw = |ui: &mut egui::Ui| {
        ui.horizontal(|ui| {
            widgets::spin_field(ui, "f", Some(weight.get()), Unit::Points, 120.0, 1.0, 0.0, &crate::panels::stroke::weight_presets(Unit::Points))
                .inspect(|&v| weight.set(v))
        })
        .inner
    };
    f.frame(vec![], &draw);
    f.frame(f.click(1), &draw);
    f.frame(vec![], &draw);
    for (k, mods, want) in [
        (Key::ArrowUp, Modifiers::SHIFT, 24.0),
        (Key::ArrowDown, Modifiers::SHIFT, 14.0),
        (Key::ArrowUp, Modifiers::NONE, 15.0),
        (Key::ArrowDown, Modifiers::NONE, 14.0),
        (Key::ArrowDown, Modifiers::SHIFT, 4.0),
        (Key::ArrowDown, Modifiers::SHIFT, 0.0),
    ] {
        assert_eq!(f.frame(vec![Event::ModifiersChanged(mods), key(k, mods)], &draw), Some(want), "{k:?} {mods:?}");
        assert_eq!(weight.get(), want);
    }
}

/// #991: both stepper arrows use ten steps with Shift and one after releasing it.
#[test]
fn stroke_spinner_buttons_honor_shift_and_clamp_at_zero() {
    let mut f = Field::new();
    let weight = Cell::new(14.0);
    let draw = |ui: &mut egui::Ui| {
        ui.horizontal(|ui| {
            widgets::spin_field(ui, "f", Some(weight.get()), Unit::Points, 120.0, 1.0, 0.0, &crate::panels::stroke::weight_presets(Unit::Points))
                .inspect(|&v| weight.set(v))
        })
        .inner
    };
    f.frame(vec![], &draw);
    // A modifier can change after the mouse release but before the next UI frame.
    for (up, mods, after, want) in [
        (true, Modifiers::SHIFT, Modifiers::NONE, 24.0),
        (false, Modifiers::SHIFT, Modifiers::SHIFT, 14.0),
        (true, Modifiers::NONE, Modifiers::SHIFT, 15.0),
        (false, Modifiers::NONE, Modifiers::NONE, 14.0),
        (false, Modifiers::SHIFT, Modifiers::SHIFT, 4.0),
        (false, Modifiers::SHIFT, Modifiers::NONE, 0.0),
    ] {
        let r = f.rect.get();
        let at = r.min + vec2(8.0, if up { 6.5 } else { 19.5 });
        let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: mods };
        f.frame(vec![Event::ModifiersChanged(mods), Event::PointerMoved(at)], &draw);
        f.frame(vec![button(true)], &draw);
        assert_eq!(f.frame(vec![button(false), Event::ModifiersChanged(after)], &draw), Some(want), "up={up} {mods:?} → {after:?}");
        assert_eq!(weight.get(), want);
    }
}

/// A wheel turn of `dy` (`unit`s, up positive) with `modifiers`, the pointer at `at`.
fn wheel(at: Pos2, unit: egui::MouseWheelUnit, dy: f32, modifiers: Modifiers) -> Vec<Event> {
    vec![Event::PointerMoved(at), Event::MouseWheel { unit, delta: vec2(0.0, dy), phase: egui::TouchPhase::Move, modifiers }]
}

/// The mouse wheel over a focused numeric field steps it as ↑/↓ do (#485): a notch a step, Shift
/// ten, Ctrl/Cmd a tenth; a trackpad's points add up to notches. Unfocused, or with the pointer
/// elsewhere, the wheel isn't the field's.
#[test]
fn the_wheel_steps_a_focused_field_under_the_pointer() {
    use egui::MouseWheelUnit::{Line, Point};
    let mut f = Field::new();
    let u = Unit::Millimeters;
    let mm = Cell::new(4.0);
    let draw = |ui: &mut egui::Ui| widgets::num_field(ui, "f", Some(u.to_pt(mm.get())), u, 80.0).inspect(|&v| mm.set(u.from_pt(v)));
    f.frame(vec![], &draw);
    let at = f.rect.get().center();
    assert_eq!(f.frame(wheel(at, Line, 1.0, Modifiers::NONE), &draw), None, "unfocused");
    f.frame(f.click(1), &draw);
    f.frame(vec![], &draw);
    for (dy, mods, want) in
        [(1.0, Modifiers::NONE, 5.0), (2.0, Modifiers::SHIFT, 25.0), (-1.0, Modifiers::COMMAND, 24.9), (-1.0, Modifiers::NONE, 23.9)]
    {
        assert!(f.frame(wheel(at, Line, dy, mods), &draw).is_some(), "{dy} {mods:?}");
        assert!((mm.get() - want).abs() < 1e-6, "{dy} {mods:?}: {}", mm.get());
    }
    assert_eq!(f.selection(), Some((0, "23.9 mm".len())), "the new value is selected");
    // Points: a step for each line's worth, what is left carried to the next turn.
    let line = f.ctx.options(|o| o.input_options.line_scroll_speed);
    assert_eq!(f.frame(wheel(at, Point, line * 0.6, Modifiers::NONE), &draw), None);
    assert!(f.frame(wheel(at, Point, line * 0.6, Modifiers::NONE), &draw).is_some());
    assert!((mm.get() - 24.9).abs() < 1e-6, "{}", mm.get());
    assert_eq!(f.frame(wheel(Pos2::new(390.0, 190.0), Line, 1.0, Modifiers::NONE), &draw), None, "the pointer elsewhere");
    // Plain fields (degrees, percent, counts) too.
    let mut f = Field::new();
    let count = |ui: &mut egui::Ui| widgets::plain_field(ui, "f", 3.0, "", 0, 80.0);
    f.frame(vec![], &count);
    f.frame(f.click(1), &count);
    f.frame(vec![], &count);
    assert_eq!(f.frame(wheel(f.rect.get().center(), Line, -1.0, Modifiers::NONE), &count), Some(2.0));
}

/// The wheel over a focused field steps it and leaves the panel where it is; over an unfocused one
/// it scrolls the panel as before.
#[test]
fn the_wheel_scrolls_the_panel_unless_over_the_focused_field() {
    for focus in [false, true] {
        let mut f = Field::new();
        let offset = Cell::new(0.0);
        let draw = |ui: &mut egui::Ui| {
            let out = egui::ScrollArea::vertical().max_height(60.0).show(ui, |ui| {
                let v = widgets::plain_field(ui, "f", 10.0, "", 0, 80.0);
                ui.add_space(600.0);
                v
            });
            offset.set(out.state.offset.y);
            out.inner
        };
        f.frame(vec![], &draw);
        let at = Pos2::new(20.0, 12.0);
        if focus {
            f.frame(
                f.click(1)
                    .into_iter()
                    .map(|e| {
                        if let Event::PointerButton { button, pressed, modifiers, .. } = e {
                            Event::PointerButton { pos: at, button, pressed, modifiers }
                        } else {
                            Event::PointerMoved(at)
                        }
                    })
                    .collect(),
                &draw,
            );
        }
        let stepped = f.frame(wheel(at, egui::MouseWheelUnit::Line, -2.0, Modifiers::NONE), &draw);
        for _ in 0..20 {
            f.frame(vec![], &draw);
        }
        if focus {
            assert_eq!((stepped, offset.get()), (Some(8.0), 0.0));
        } else {
            assert!(stepped.is_none() && offset.get() > 0.0, "{stepped:?} {}", offset.get());
        }
    }
}
