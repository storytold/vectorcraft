//! Numeric fields: math typed before the unit suffix, and the whole text (unit included) selected
//! on keyboard focus, a released focus click or double-click so typing replaces it; a drag
//! keeps the pressed character as its selection anchor.

use std::cell::{Cell, RefCell};

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Rect, vec2};
use vectorcraft_doc::Unit;

use crate::widgets;

/// #1457: a focused field follows its value when that changes from outside (the slider beside
/// it), so the arrow keys step from the value shown and losing focus doesn't put the old one back.
#[test]
fn a_focused_field_follows_a_value_changed_from_outside() {
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut time = 0.0;
    let mut frame = |value: f64, events: Vec<Event>| {
        time += 0.05;
        let screen_rect = Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0)));
        let mut got = (None, Rect::NOTHING);
        let mut out = ctx.run_ui(egui::RawInput { events, time: Some(time), screen_rect, focused: true, ..Default::default() }, |ui| {
            got.0 = widgets::range_field(ui, "f", value, 0.0..=1000.0, "%", 2, 80.0);
            got.1 = ui.min_rect();
        });
        out.textures_delta.clear();
        got
    };
    let key = |key| Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    // Click into the field showing 100 %.
    let at = frame(100.0, vec![]).1.center();
    frame(100.0, vec![Event::PointerMoved(at), button(at, true)]);
    frame(100.0, vec![button(at, false)]);
    assert_eq!(frame(100.0, vec![]).0, None);
    // The value changes under the focused field (a slider dragged to 142): ↓ steps from 142, not
    // from the 100 the field held when it took the focus.
    frame(142.0, vec![]);
    assert_eq!(frame(142.0, vec![key(Key::ArrowDown)]).0, Some(141.0));
    // And changed again, losing focus leaves the value alone.
    frame(50.0, vec![]);
    let away = at + vec2(0.0, 150.0);
    frame(50.0, vec![Event::PointerMoved(away), button(away, true)]);
    assert_eq!(frame(50.0, vec![button(away, false)]).0, None);
}

/// #1692: a hex field that the focus only passes through shows the color as it is now and commits
/// nothing. Tab out of an edited field gives the hex field after it the focus in the frame the edit
/// is committed, and the color changes after the hex field is drawn, as in the Color Picker.
/// Shift+Tab out of the field after it gives it the focus a frame later, in the frame that field
/// commits.
#[test]
fn a_hex_field_passed_through_with_tab_commits_nothing() {
    let mut f = Field::new();
    // Red and Blue fields on either side of the hex field of the color they make.
    let rgb = Cell::new([255.0, 128.0, 0.0]);
    let (hex, blue, hex_id) = (Cell::new(None), Cell::new(None), Cell::new(egui::Id::NULL));
    let draw = |ui: &mut egui::Ui| {
        let [r, g, b] = rgb.get();
        let red = widgets::plain_field(ui, "red", r, "", 0, 80.0);
        hex_id.set(ui.id().with("hex"));
        hex.set(widgets::hex_field(ui, "hex", &format!("{:02X}{:02X}{:02X}", r as u8, g as u8, b as u8)));
        blue.set(widgets::plain_field(ui, "blue", b, "", 0, 80.0));
        red
    };
    // One frame, its edits applied after it. → What the hex field committed.
    let step = |f: &mut Field, events: Vec<Event>| {
        let red = f.frame(events, &draw);
        let [r, g, b] = rgb.get();
        rgb.set([red.unwrap_or(r), g, blue.take().unwrap_or(b)]);
        hex.take()
    };
    // The text the hex field shows (it keeps it under its id).
    let shows = |f: &Field| f.ctx.data(|d| d.get_temp::<String>(hex_id.get()));
    let (tab, back) = (key(Key::Tab, Modifiers::NONE), key(Key::Tab, Modifiers::SHIFT));
    step(&mut f, vec![]);
    // Tab into Red, type 50 over it, and Tab on to the hex field, then on to Blue.
    for events in [vec![tab.clone()], vec![Event::Text("50".into())], vec![tab.clone()], vec![]] {
        assert_eq!(step(&mut f, events), None);
    }
    assert_eq!(shows(&f).as_deref(), Some("328000"), "the hex field shows Red's edit");
    for events in [vec![tab.clone()], vec![]] {
        assert_eq!(step(&mut f, events), None);
    }
    assert_eq!(rgb.get(), [50.0, 128.0, 0.0], "Red keeps what was typed");
    // Type 200 over Blue, Shift+Tab back into the hex field and Tab out of it again.
    for events in [vec![Event::Text("200".into())], vec![back], vec![], vec![]] {
        assert_eq!(step(&mut f, events), None);
    }
    assert_eq!(shows(&f).as_deref(), Some("3280C8"), "the hex field shows Blue's edit");
    for events in [vec![tab], vec![]] {
        assert_eq!(step(&mut f, events), None);
    }
    assert_eq!(rgb.get(), [50.0, 128.0, 200.0], "Blue keeps what was typed");
}

/// What is typed into a hex field is committed when Tab or Enter ends its focus, and Escape drops
/// it.
#[test]
fn typing_into_a_hex_field_commits_it() {
    let mut f = Field::new();
    let hex = Cell::new(None);
    let draw = |ui: &mut egui::Ui| {
        let red = widgets::plain_field(ui, "red", 255.0, "", 0, 80.0);
        hex.set(widgets::hex_field(ui, "hex", "FF8000"));
        red
    };
    let step = |f: &mut Field, events: Vec<Event>| {
        f.frame(events, &draw);
        hex.take()
    };
    let tab = || key(Key::Tab, Modifiers::NONE);
    let escape = || key(Key::Escape, Modifiers::NONE);
    step(&mut f, vec![]);
    for (end, commits) in [(tab(), true), (enter(), true), (escape(), false)] {
        // Tab into Red and on into the hex field, select its text and type over it.
        let mut got = vec![];
        for events in
            [vec![tab()], vec![tab()], vec![], vec![key(Key::A, Modifiers::COMMAND)], vec![Event::Text("00FF00".into())], vec![end.clone()], vec![]]
        {
            got.extend(step(&mut f, events));
        }
        let want: Vec<String> = if commits { vec!["00FF00".into()] } else { vec![] };
        assert_eq!(got, want, "{end:?}");
        // Neither field has the focus for the next round.
        step(&mut f, vec![escape()]);
    }
}

/// #1692: the Stroke panel's dash and gap fields ([`widgets::opt_field`]) are blank while the
/// selected objects' dashes differ. Tab out of the dash after typing into it gives the gap the
/// focus; the gap then shows the dash's length, which the committed dash repeats as its gap. Tab
/// out of the gap commits nothing, and what is typed into the gap replaces all of that length.
#[test]
fn a_gap_field_passed_through_with_tab_commits_nothing() {
    use crate::panels::stroke::{dash_fields, dash_pattern};
    let shown = Cell::new([None; 6]);
    let (dash, gap) = (Cell::new(None), Cell::new(None));
    let draw = |ui: &mut egui::Ui| -> Option<f64> {
        let [d, g, ..] = shown.get();
        dash.set(widgets::opt_field(ui, "dash", d, Unit::Points, 60.0));
        gap.set(widgets::opt_field(ui, "gap", g, Unit::Points, 60.0));
        None
    };
    // One frame, then its commit applied the way the Stroke panel applies it. → What the gap
    // committed.
    let step = |f: &mut Field, events: Vec<Event>| {
        f.frame(events, &draw);
        let (d, g) = (dash.take(), gap.take());
        if d.is_some() || g.is_some() {
            let mut fields = shown.get();
            fields[0] = d.unwrap_or(fields[0]);
            fields[1] = g.unwrap_or(fields[1]);
            shown.set(dash_fields(&dash_pattern(&fields)));
        }
        g
    };
    let tab = || key(Key::Tab, Modifiers::NONE);
    for (typed, commits, pattern) in [(None, None, [20.0, 20.0]), (Some("5"), Some(Some(5.0)), [20.0, 5.0])] {
        let mut f = Field::new();
        shown.set([None; 6]);
        step(&mut f, vec![]);
        // Tab into the dash, type 20 and Tab on into the gap.
        for events in [vec![tab()], vec![Event::Text("20".into())], vec![tab()], vec![]] {
            assert_eq!(step(&mut f, events), None);
        }
        if let Some(text) = typed {
            assert_eq!(step(&mut f, vec![Event::Text(text.into())]), None);
        }
        assert_eq!(step(&mut f, vec![tab()]), commits, "{typed:?}");
        assert_eq!(step(&mut f, vec![]), None);
        assert_eq!(dash_pattern(&shown.get()), pattern, "{typed:?}");
    }
}

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

    fn text_point(&self, text: &str, index: usize, margin: f32) -> Pos2 {
        let galley = self.ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), egui::FontId::proportional(12.5), egui::Color32::BLACK));
        let x = galley.pos_from_cursor(egui::text::CCursor::new(index)).left();
        Pos2::new(self.rect.get().left() + margin + x + 0.1, self.rect.get().center().y)
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
fn a_percentage_typed_in_a_field_is_a_share_of_the_value_before_it() {
    let mut f = Field::new();
    let cm = |ui: &mut egui::Ui| widgets::num_field(ui, "f", Some(72.0), Unit::Centimeters, 80.0);
    f.frame(vec![], &cm);
    f.frame(f.click(1), &cm);
    f.frame(vec![], &cm);
    f.frame(vec![Event::Text("3cm*50%".into())], &cm);
    let got = f.frame(vec![enter()], &cm).expect("the field reads `3cm*50%`");
    assert!((got - Unit::Centimeters.to_pt(1.5)).abs() < 1e-9, "{got}");
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

/// Shift+Enter applies what is typed into a field and keeps the field active, its new value all
/// selected so typing replaces it; Enter then ends its editing. In a dialog, Shift+Enter ends the
/// editing as Enter does.
#[test]
fn shift_enter_applies_the_value_and_keeps_the_field_active() {
    let shift_enter = || key(Key::Enter, Modifiers::SHIFT);
    let has_focus = |f: &Field| f.ctx.memory(|m| m.has_focus(f.id.get()));
    // A unit field, a plain field with a suffix and a field that may be empty, each applying what
    // it returns. → (text typed, value applied, text shown afterwards).
    let px = Cell::new(100.0);
    let num = |ui: &mut egui::Ui| widgets::num_field(ui, "f", Some(px.get()), Unit::Pixels, 80.0).inspect(|&v| px.set(v));
    let deg = Cell::new(10.0);
    let plain = |ui: &mut egui::Ui| widgets::plain_field(ui, "f", deg.get(), "°", 2, 80.0).inspect(|&v| deg.set(v));
    let dash = Cell::new(Some(20.0));
    let opt = |ui: &mut egui::Ui| widgets::opt_field(ui, "f", dash.get(), Unit::Points, 60.0).map(|v| dash.set(v)).and(dash.get());
    type Draw<'a> = &'a dyn Fn(&mut egui::Ui) -> Option<f64>;
    let fields: [(&str, Draw, &str, f64, &str); 3] =
        [("num_field", &num, "150", 150.0, "150 px"), ("plain_field", &plain, "45", 45.0, "45°"), ("opt_field", &opt, "2+3", 5.0, "5")];
    for (name, draw, typed, want, shows) in fields {
        let mut f = Field::new();
        f.frame(vec![], draw);
        f.frame(f.click(1), draw);
        f.frame(vec![], draw);
        f.frame(vec![Event::Text(typed.into())], draw);
        assert_eq!(f.frame(vec![shift_enter()], draw), Some(want), "{name}: Shift+Enter applies the value");
        assert!(has_focus(&f), "{name}: the field keeps the focus");
        assert_eq!(f.frame(vec![], draw), None, "{name}: applied once");
        assert_eq!(f.selection(), Some((0, shows.chars().count())), "{name}: the value shown is selected");
        assert_eq!(f.ctx.data(|d| d.get_temp::<String>(f.id.get())).as_deref(), Some(shows), "{name}");
        // Typing replaces it, and Enter applies that and ends the editing.
        f.frame(vec![Event::Text("2".into())], draw);
        assert_eq!(f.frame(vec![enter()], draw), Some(2.0), "{name}: Enter after it");
        assert!(!has_focus(&f), "{name}: Enter ends the editing");
    }
    // A text field and a hex field (its color shown in capitals) the same way; a field of several
    // lines takes Shift+Enter as a line break.
    let (text, hex, hex_id) = (Cell::new(None), Cell::new(None), Cell::new(egui::Id::NULL));
    let color = RefCell::new(String::from("FF8000"));
    let draw = |ui: &mut egui::Ui| {
        text.set(widgets::text_field(ui, "f", Some("Name"), 120.0, 1).or(text.take()));
        hex_id.set(ui.id().with("hex"));
        let shown = color.borrow().clone();
        let applied = widgets::hex_field(ui, "hex", &shown);
        if let Some(h) = &applied {
            color.replace(h.to_uppercase());
        }
        hex.set(applied.or(hex.take()));
        None
    };
    let mut f = Field::new();
    f.frame(vec![], &draw);
    f.frame(vec![key(Key::Tab, Modifiers::NONE)], &draw);
    f.frame(vec![key(Key::A, Modifiers::COMMAND)], &draw);
    f.frame(vec![Event::Text("Logo".into()), shift_enter()], &draw);
    assert_eq!(text.take().as_deref(), Some("Logo"));
    assert!(has_focus(&f), "the text field keeps the focus");
    f.frame(vec![key(Key::Tab, Modifiers::NONE)], &draw);
    f.frame(vec![key(Key::A, Modifiers::COMMAND)], &draw);
    f.frame(vec![Event::Text("00ff00".into()), shift_enter()], &draw);
    assert_eq!((text.take(), hex.take().as_deref()), (None, Some("00ff00")));
    assert!(f.ctx.memory(|m| m.has_focus(hex_id.get())), "the hex field keeps the focus");
    f.frame(vec![], &draw);
    assert_eq!(f.ctx.data(|d| d.get_temp::<String>(hex_id.get())).as_deref(), Some("00FF00"), "the hex field shows the color applied");
    let selected = egui::TextEdit::load_state(&f.ctx, hex_id.get()).and_then(|st| st.cursor.char_range()).map(|r| r.as_sorted_char_range());
    assert_eq!(selected.map(|r| (r.start.0, r.end.0)), Some((0, 6)), "the hex field's color is selected");
    let lines = Cell::new(None);
    let draw = |ui: &mut egui::Ui| {
        lines.set(widgets::text_field(ui, "f", Some(""), 120.0, 3).or(lines.take()));
        None
    };
    let mut f = Field::new();
    f.frame(vec![], &draw);
    f.frame(f.click(1), &draw);
    f.frame(vec![Event::Text("a".into()), shift_enter(), Event::Text("b".into())], &draw);
    assert!(has_focus(&f) && lines.take().is_none(), "a line break, nothing applied");
    f.frame(vec![key(Key::Escape, Modifiers::NONE)], &draw);
    // In a dialog, Shift+Enter applies the value and ends the editing.
    let mut f = Field::new();
    widgets::set_dialog_open(&f.ctx, true);
    px.set(100.0);
    f.frame(vec![], &num);
    f.frame(f.click(1), &num);
    f.frame(vec![Event::Text("150".into())], &num);
    assert_eq!(f.frame(vec![shift_enter()], &num), Some(150.0));
    assert!(!has_focus(&f), "a dialog's field ends its editing");
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
        (Key::ArrowDown, Modifiers::NONE, 14.0),
    ] {
        assert!(f.frame(vec![key(k, mods)], &draw).is_some(), "{k:?} {mods:?}");
        assert!((mm.get() - want).abs() < 1e-6, "{k:?} {mods:?}: {}", mm.get());
    }
    assert_eq!(f.selection(), Some((0, "14 mm".len())), "the new value is selected");
    // Enter after the steps commits nothing more.
    assert_eq!(f.frame(vec![enter()], &draw), None);
}

/// #2881, #2793: ↑/↓ and the wheel step a fractional value to the next whole unit in their
/// direction (5.1 mm: up to 6, down to 5), and Ctrl/Cmd steps to the next tenth; Shift adds or takes
/// ten and keeps the fraction. In each field's own unit: millimeters, points, percent, degrees.
#[test]
fn steps_go_to_the_next_whole_unit_or_tenth() {
    use egui::MouseWheelUnit::Line;
    let u = Unit::Millimeters;
    let mm = Cell::new(0.0);
    let mm_field = |ui: &mut egui::Ui| widgets::num_field(ui, "f", Some(u.to_pt(mm.get())), u, 80.0).inspect(|&v| mm.set(u.from_pt(v)));
    let mut f = Field::new();
    f.frame(vec![], &mm_field);
    f.frame(f.click(1), &mm_field);
    f.frame(vec![], &mm_field);
    let at = f.rect.get().center();
    // From `from` (set from outside, which the focused field follows), the value `events` give.
    let mut step = |from: f64, events: Vec<Event>| {
        mm.set(from);
        f.frame(vec![], &mm_field);
        assert!(f.frame(events, &mm_field).is_some(), "from {from}");
        // The field shows the new value before the next one is set.
        f.frame(vec![], &mm_field);
        mm.get()
    };
    let arrow = |k, mods| vec![key(k, mods)];
    let turn = |dy, mods| wheel(at, Line, dy, mods);
    for (from, events, want) in [
        (5.1, arrow(Key::ArrowUp, Modifiers::NONE), 6.0),
        (5.1, arrow(Key::ArrowDown, Modifiers::NONE), 5.0),
        (5.0, arrow(Key::ArrowUp, Modifiers::NONE), 6.0),
        (5.0, arrow(Key::ArrowDown, Modifiers::NONE), 4.0),
        (-2.5, arrow(Key::ArrowUp, Modifiers::NONE), -2.0),
        (-2.5, arrow(Key::ArrowDown, Modifiers::NONE), -3.0),
        (5.13, arrow(Key::ArrowUp, Modifiers::COMMAND), 5.2),
        (5.13, arrow(Key::ArrowDown, Modifiers::COMMAND), 5.1),
        (5.1, arrow(Key::ArrowUp, Modifiers::SHIFT), 15.1),
        (5.1, arrow(Key::ArrowDown, Modifiers::SHIFT), -4.9),
        (5.1, vec![key(Key::ArrowUp, Modifiers::NONE), key(Key::ArrowUp, Modifiers::NONE)], 7.0),
        // Several presses in one frame step in the order they came.
        (5.1, vec![key(Key::ArrowUp, Modifiers::NONE), key(Key::ArrowDown, Modifiers::NONE)], 5.0),
        (5.13, vec![key(Key::ArrowUp, Modifiers::NONE), key(Key::ArrowUp, Modifiers::COMMAND)], 6.1),
        (24.8, turn(1.0, Modifiers::NONE), 25.0),
        (24.8, turn(-1.0, Modifiers::NONE), 24.0),
        (24.8, turn(2.0, Modifiers::NONE), 26.0),
        (24.8, turn(-2.0, Modifiers::NONE), 23.0),
        (24.0, turn(1.0, Modifiers::NONE), 25.0),
        (24.83, turn(1.0, Modifiers::COMMAND), 24.9),
        (24.8, turn(1.0, Modifiers::SHIFT), 34.8),
    ] {
        let got = step(from, events);
        assert!((got - want).abs() < 1e-6, "{from} mm to {got}, not {want}");
    }
    // A points field steps whole points.
    let pt = Cell::new(12.25);
    let pt_field = |ui: &mut egui::Ui| widgets::num_field(ui, "f", Some(pt.get()), Unit::Points, 80.0).inspect(|&v| pt.set(v));
    let mut f = Field::new();
    f.frame(vec![], &pt_field);
    f.frame(f.click(1), &pt_field);
    f.frame(vec![], &pt_field);
    assert_eq!(f.frame(vec![key(Key::ArrowUp, Modifiers::NONE)], &pt_field), Some(13.0));
    // Percent and degree fields step whole percents and degrees.
    for suffix in ["%", "°"] {
        let mut f = Field::new();
        let value = Cell::new(37.5);
        let plain = |ui: &mut egui::Ui| widgets::plain_field(ui, "f", value.get(), suffix, 2, 80.0).inspect(|&v| value.set(v));
        f.frame(vec![], &plain);
        f.frame(f.click(1), &plain);
        f.frame(vec![], &plain);
        for (k, mods, want) in [
            (Key::ArrowDown, Modifiers::NONE, 37.0),
            (Key::ArrowUp, Modifiers::NONE, 38.0),
            (Key::ArrowUp, Modifiers::COMMAND, 38.1),
            (Key::ArrowUp, Modifiers::NONE, 39.0),
        ] {
            assert_eq!(f.frame(vec![key(k, mods)], &plain), Some(want), "{suffix} {k:?} {mods:?}");
            f.frame(vec![], &plain);
        }
    }
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
        [(1.0, Modifiers::NONE, 5.0), (2.0, Modifiers::SHIFT, 25.0), (-1.0, Modifiers::COMMAND, 24.9), (-1.0, Modifiers::NONE, 24.0)]
    {
        assert!(f.frame(wheel(at, Line, dy, mods), &draw).is_some(), "{dy} {mods:?}");
        assert!((mm.get() - want).abs() < 1e-6, "{dy} {mods:?}: {}", mm.get());
    }
    assert_eq!(f.selection(), Some((0, "24 mm".len())), "the new value is selected");
    // Points: a step for each line's worth, what is left carried to the next turn.
    let line = f.ctx.options(|o| o.input_options.line_scroll_speed);
    assert_eq!(f.frame(wheel(at, Point, line * 0.6, Modifiers::NONE), &draw), None);
    assert!(f.frame(wheel(at, Point, line * 0.6, Modifiers::NONE), &draw).is_some());
    assert!((mm.get() - 25.0).abs() < 1e-6, "{}", mm.get());
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

fn numeric_fixture(ui: &mut egui::Ui, mode: usize) -> Option<f64> {
    match mode {
        0 => widgets::num_field(ui, "f", Some(123456.0), Unit::Pixels, 100.0),
        1 => widgets::num_field(ui, "f", Some(123456.0), Unit::Points, 100.0),
        2 => widgets::plain_field(ui, "f", 123456.0, "°", 0, 100.0),
        _ => widgets::opt_field(ui, "f", Some(123456.0), Unit::Pixels, 100.0).flatten(),
    }
}

fn field_text(mode: usize) -> &'static str {
    ["123456 px", "123456 pt", "123456°", "123456"][mode]
}

fn pointer_button(at: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }
}

#[test]
fn numeric_drag_keeps_its_press_position_with_or_without_prior_focus() {
    for mode in 0..4 {
        for focused in [false, true] {
            for (start, end) in [(1, 4), (4, 1)] {
                let mut f = Field::new();
                let draw = |ui: &mut egui::Ui| numeric_fixture(ui, mode);
                f.frame(vec![], &draw);
                if focused {
                    f.frame(f.click(1), &draw);
                    f.frame(vec![], &draw);
                    f.time += 1.0;
                }
                let margin = if mode == 3 { 5.0 } else { 7.0 };
                let at = f.text_point(field_text(mode), start, margin);
                let to = f.text_point(field_text(mode), end, margin);
                f.frame(vec![Event::PointerMoved(at), pointer_button(at, true)], &draw);
                assert_eq!(f.selection(), Some((start, start)), "mode {mode}, prior focus {focused}");
                f.frame(vec![], &draw); // Holding still must leave the press position alone.
                f.frame(vec![Event::PointerMoved(to)], &draw);
                f.frame(vec![pointer_button(to, false)], &draw);
                assert_eq!(f.selection(), Some((1, 4)), "mode {mode}, {start}->{end}, prior focus {focused}");
                f.frame(vec![Event::Text("7".into())], &draw);
                assert_eq!(f.frame(vec![enter()], &draw), Some(1756.0), "replace only the dragged text");
            }
        }
    }
}

#[test]
fn a_new_numeric_click_selects_all_on_release_without_moving_its_press_caret() {
    let mut f = Field::new();
    f.num(vec![]);
    let at = f.rect.get().center();
    f.num(vec![Event::PointerMoved(at), pointer_button(at, true)]);
    assert!(f.selection().is_some_and(|(a, b)| a == b), "a hold can still start a text drag");
    f.num(vec![]);
    assert!(f.selection().is_some_and(|(a, b)| a == b));
    f.num(vec![pointer_button(at, false)]);
    assert_eq!(f.selection(), Some((0, "100 px".len())));
    f.num(vec![Event::Text("7".into())]);
    assert_eq!(f.num(vec![enter()]), Some(7.0));
}

/// #1326: a short drag that the toolkit still counts as a click must keep its caret on
/// release instead of selecting the whole value.
#[test]
fn a_short_drag_released_as_a_click_keeps_its_caret() {
    let mut f = Field::new();
    let draw = |ui: &mut egui::Ui| numeric_fixture(ui, 0);
    f.frame(vec![], &draw);
    let at = f.text_point(field_text(0), 2, 7.0);
    f.frame(vec![Event::PointerMoved(at), pointer_button(at, true)], &draw);
    let pressed = f.selection();
    assert!(pressed.is_some_and(|(a, b)| a == b), "the press places a caret");
    // Moved a few points: inside one character, so egui reports a click, not a text selection.
    let nudge = egui::vec2(3.4, 0.0);
    f.frame(vec![Event::PointerMoved(at + nudge)], &draw);
    f.frame(vec![pointer_button(at + nudge, false)], &draw);
    assert_eq!(f.selection(), pressed, "the short drag keeps its caret instead of selecting all");
    assert_ne!(f.selection(), Some((0, field_text(0).chars().count())), "a clicked short drag must not select all (#1326)");
    f.frame(vec![Event::Text("7".into())], &draw);
    assert_eq!(f.frame(vec![enter()], &draw), Some(1273456.0), "typing lands where the drag left the caret");
}

/// A tap (no movement) on a fresh numeric field still selects all on release.
#[test]
fn a_tap_without_movement_still_selects_all_on_release() {
    let mut f = Field::new();
    let draw = |ui: &mut egui::Ui| numeric_fixture(ui, 0);
    f.frame(vec![], &draw);
    let at = f.text_point(field_text(0), 2, 7.0);
    f.frame(vec![Event::PointerMoved(at), pointer_button(at, true)], &draw);
    f.frame(vec![], &draw);
    f.frame(vec![pointer_button(at, false)], &draw);
    assert_eq!(f.selection(), Some((0, field_text(0).chars().count())), "a plain tap selects all");
}

#[test]
fn keyboard_focus_keeps_selecting_the_entire_numeric_field() {
    for mode in 0..4 {
        let mut f = Field::new();
        let draw = |ui: &mut egui::Ui| numeric_fixture(ui, mode);
        f.frame(vec![], &draw);
        f.frame(vec![key(Key::Tab, Modifiers::NONE)], &draw);
        f.frame(vec![], &draw);
        assert_eq!(f.selection(), Some((0, field_text(mode).chars().count())), "mode {mode}");
    }
}

/// #1637: text typed into a field is committed when the focus leaves it by a click on empty space
/// or by a press on a field drawn before it (another channel of the Color panel), whether the
/// window reports keyboard focus or not, as Enter commits it. Escape still drops it.
#[test]
fn typed_text_is_committed_when_the_focus_moves_away() {
    for focused in [true, false] {
        for onto_field_above in [false, true] {
            let mut f = Field::new();
            f.focused = focused;
            let above = Cell::new(Rect::NOTHING);
            let draw = |ui: &mut egui::Ui| {
                widgets::range_field(ui, "above", 50.0, 0.0..=1000.0, "%", 0, 80.0);
                above.set(ui.min_rect());
                widgets::range_field(ui, "f", 100.0, 0.0..=1000.0, "%", 0, 80.0)
            };
            f.frame(vec![], &draw);
            let field = Pos2::new(above.get().center().x, (above.get().bottom() + f.rect.get().bottom()) / 2.0);
            let away = if onto_field_above { above.get().center() } else { Pos2::new(field.x, 180.0) };
            let case = format!("window focus {focused}, onto the field above {onto_field_above}");
            f.frame(vec![Event::PointerMoved(field), pointer_button(field, true)], &draw);
            f.frame(vec![pointer_button(field, false)], &draw);
            f.frame(vec![Event::Text("42".into())], &draw);
            let committed = [
                f.frame(vec![Event::PointerMoved(away), pointer_button(away, true)], &draw),
                f.frame(vec![pointer_button(away, false)], &draw),
                f.frame(vec![], &draw),
            ];
            assert_eq!(committed.iter().flatten().copied().collect::<Vec<_>>(), [42.0], "{case}");

            f.frame(vec![Event::PointerMoved(field), pointer_button(field, true)], &draw);
            f.frame(vec![pointer_button(field, false)], &draw);
            f.frame(vec![Event::Text("7".into())], &draw);
            let escaped = [f.frame(vec![key(Key::Escape, Modifiers::NONE)], &draw), f.frame(vec![], &draw)];
            assert_eq!(escaped, [None, None], "{case}");
        }
    }
}

/// Text typed into a field is committed in the frame of a press elsewhere, before what is drawn
/// after the field (the canvas) acts on the press: a click that selects another object must not
/// get the typed value, even when both objects show the same one.
#[test]
fn a_press_on_the_canvas_commits_to_the_object_being_edited() {
    let mut f = Field::new();
    // The object the panel shows; both have the value 100.
    let selected = Cell::new(1);
    let committed = Cell::new(None);
    let draw = |ui: &mut egui::Ui| {
        let got = widgets::range_field(ui, "f", 100.0, 0.0..=1000.0, "%", 0, 80.0);
        if let Some(v) = got {
            committed.set(Some((selected.get(), v)));
        }
        // The canvas: a press ends text input and selects the other object.
        if ui.input(|i| i.pointer.primary_pressed() && i.pointer.interact_pos().is_some_and(|p| p.y > 100.0)) {
            ui.memory_mut(|m| m.stop_text_input());
            selected.set(2);
        }
        got
    };
    f.frame(vec![], &draw);
    let (field, canvas) = (f.rect.get().center(), Pos2::new(200.0, 180.0));
    f.frame(vec![Event::PointerMoved(field), pointer_button(field, true)], &draw);
    f.frame(vec![pointer_button(field, false)], &draw);
    f.frame(vec![Event::Text("42".into())], &draw);
    f.frame(vec![Event::PointerMoved(canvas), pointer_button(canvas, true)], &draw);
    f.frame(vec![pointer_button(canvas, false)], &draw);
    f.frame(vec![], &draw);
    assert_eq!((committed.get(), selected.get()), (Some((1, 42.0)), 2));
}
