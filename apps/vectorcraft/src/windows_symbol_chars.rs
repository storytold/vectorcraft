//! Characters a keyboard tool types in a symbol font's encoding on Windows, such as NHM Writer's
//! legacy Tamil keymaps for SR-Tamil fonts.
//!
//! Such a tool posts each byte of the font's 8-bit encoding as a `WM_CHAR` holding U+F000 plus the
//! byte, Windows' convention for symbol fonts. Those are Private Use Area characters, which
//! egui-winit drops as unprintable (`is_printable_char`), so the Type tool gets nothing (the tool's
//! Backspace presses, which it sends to rewrite a syllable, still arrive). [`message`] notes the
//! characters in order with the other characters and key presses the window gets (it is fed by
//! `windows_alt_codes::msg_hook`, which already reads each `MSG`), and [`raw_input_hook`] hands
//! them to the frame unchanged, each in its place among the frame's text and key events. A symbol
//! font maps exactly these code points (its Windows cmap is (3,0) at U+F020..=U+F0FF), so they draw
//! as typed; turning them into the bytes' Latin-1 characters instead would make byte 0xAD
//! (SR-Tamil's ஷ) a soft hyphen, which the text engine never draws, and bytes 0x80..=0x9F control
//! characters.

#![cfg_attr(not(windows), allow(dead_code))]

use std::cell::RefCell;
use std::collections::VecDeque;

use egui::Event;

const WM_KEYDOWN: u32 = 0x0100;
const WM_CHAR: u32 = 0x0102;
const WM_SYSKEYDOWN: u32 = 0x0104;
/// `VK_PACKET` (`winuser.h`): the key press a keyboard tool's injected character comes with, which
/// egui reports as no key.
const VK_PACKET: usize = 0xE7;
/// U+F020..=U+F0FF: a symbol font's bytes 0x20..=0xFF.
const SYMBOL_FIRST: u32 = 0xF020;
const SYMBOL_LAST: u32 = 0xF0FF;

thread_local! {
    /// The UI thread's sequence: [`message`] and [`raw_input_hook`] both run on it.
    static TYPED: RefCell<SymbolChars> = RefCell::new(SymbolChars::default());
}

/// A keyboard message that becomes an egui event, in the order the window got it.
#[derive(Debug, PartialEq)]
enum Typed {
    /// A symbol-font character (U+F020..=U+F0FF), which egui drops.
    Symbol(char),
    /// Another character, which egui reports as text.
    Char(char),
    /// A key press, which egui reports as a key event.
    Key,
}

#[derive(Default)]
struct SymbolChars {
    typed: VecDeque<Typed>,
    /// A high surrogate waiting for its low half.
    high: Option<u16>,
}

/// The symbol-font character `unit` is, or `None` for any other code point.
fn symbol_char(unit: u32) -> Option<char> {
    (SYMBOL_FIRST..=SYMBOL_LAST).contains(&unit).then(|| char::from_u32(unit)).flatten()
}

fn is_symbol(c: char) -> bool {
    (SYMBOL_FIRST..=SYMBOL_LAST).contains(&u32::from(c))
}

/// A character egui reports as text: not a control character and not in a Private Use Area.
fn is_text(c: char) -> bool {
    let private = matches!(c, '\u{E000}'..='\u{F8FF}' | '\u{F0000}'..='\u{FFFFD}' | '\u{100000}'..='\u{10FFFD}');
    !private && !c.is_control()
}

/// Take `typed` up to and including the first entry `is_match` accepts, and return the symbols
/// before it. When no entry matches, nothing is taken.
fn symbols_before(typed: &mut VecDeque<Typed>, is_match: impl Fn(&Typed) -> bool) -> String {
    let Some(i) = typed.iter().position(is_match) else {
        return String::new();
    };
    typed
        .drain(..=i)
        .filter_map(|t| match t {
            Typed::Symbol(c) => Some(c),
            _ => None,
        })
        .collect()
}

impl SymbolChars {
    /// Note a keyboard message `message` with `wparam` (the virtual-key code, or a UTF-16 unit).
    fn message(&mut self, message: u32, wparam: usize) {
        match message {
            WM_KEYDOWN | WM_SYSKEYDOWN if wparam != VK_PACKET => self.typed.push_back(Typed::Key),
            WM_CHAR => {
                let unit = u16::try_from(wparam & 0xffff).unwrap_or_default();
                if (0xd800..0xdc00).contains(&unit) {
                    self.high = Some(unit);
                    return;
                }
                let units: Vec<u16> = self.high.take().into_iter().chain([unit]).collect();
                for c in char::decode_utf16(units).filter_map(Result::ok) {
                    if let Some(s) = symbol_char(u32::from(c)) {
                        self.typed.push_back(Typed::Symbol(s));
                    } else if is_text(c) {
                        self.typed.push_back(Typed::Char(c));
                    }
                }
            }
            _ => {}
        }
    }

    /// Hand the frame `events` the symbol-font characters typed since the last frame, as typed
    /// text, each before the text or key press that followed it.
    fn drain(&mut self, events: &mut Vec<Event>) {
        let mut typed = std::mem::take(&mut self.typed);
        if !typed.iter().any(|t| matches!(t, Typed::Symbol(_))) {
            return;
        }
        // Should egui ever pass these characters on itself, leave them to it.
        if events.iter().any(|e| matches!(e, Event::Text(t) if t.chars().any(is_symbol))) {
            return;
        }
        let mut out = Vec::with_capacity(events.len() + 1);
        for e in events.drain(..) {
            match e {
                Event::Key { pressed: true, .. } => {
                    let symbols = symbols_before(&mut typed, |t| *t == Typed::Key);
                    if !symbols.is_empty() {
                        out.push(Event::Text(symbols));
                    }
                    out.push(e);
                }
                Event::Text(text) => {
                    // Each character of the text was typed on its own: the symbols typed before
                    // it go first, splitting the text where they fall.
                    let mut run = String::new();
                    for c in text.chars() {
                        let symbols = symbols_before(&mut typed, |t| *t == Typed::Char(c));
                        if !symbols.is_empty() {
                            if !run.is_empty() {
                                out.push(Event::Text(std::mem::take(&mut run)));
                            }
                            out.push(Event::Text(symbols));
                        }
                        run.push(c);
                    }
                    if !run.is_empty() {
                        out.push(Event::Text(run));
                    }
                }
                e => out.push(e),
            }
        }
        let rest: String = typed
            .into_iter()
            .filter_map(|t| match t {
                Typed::Symbol(c) => Some(c),
                _ => None,
            })
            .collect();
        if !rest.is_empty() {
            out.push(Event::Text(rest));
        }
        *events = out;
    }
}

/// Note a keyboard message of the UI thread's queue. `windows_alt_codes::msg_hook` calls this for
/// each message winit's message hook gets.
pub fn message(message: u32, wparam: usize) {
    TYPED.with(|t| {
        if let Ok(mut t) = t.try_borrow_mut() {
            t.message(message, wparam);
        }
    });
}

/// Before egui reads the main window's frame: the symbol-font characters typed, as text.
pub fn raw_input_hook(raw: &mut egui::RawInput) {
    TYPED.with(|t| {
        if let Ok(mut t) = t.try_borrow_mut() {
            t.drain(&mut raw.events);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Key;

    const VK_BACK: usize = 0x08;
    const VK_LEFT: usize = 0x25;
    const VK_A: usize = 0x41;

    /// A character a keyboard tool injects.
    fn injected(c: &mut SymbolChars, unit: u32) {
        c.message(WM_KEYDOWN, VK_PACKET);
        c.message(WM_CHAR, unit as usize);
    }

    fn key(key: Key, pressed: bool) -> Event {
        Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: egui::Modifiers::default() }
    }

    fn text(s: &str) -> Event {
        Event::Text(s.into())
    }

    #[test]
    fn a_symbol_font_character_is_typed_as_it_came() {
        let mut c = SymbolChars::default();
        injected(&mut c, 0xF0E3);
        injected(&mut c, 0xF05B);
        let mut events = vec![];
        c.drain(&mut events);
        assert_eq!(events, vec![text("\u{F0E3}\u{F05B}")]);
        let mut events = vec![];
        c.drain(&mut events);
        assert!(events.is_empty(), "nothing carries over to a later frame");
    }

    #[test]
    fn characters_keep_their_place_around_backspace() {
        // What NHM Writer sends while rewriting a syllable (from a real log).
        let mut c = SymbolChars::default();
        injected(&mut c, 0xF0E3);
        c.message(WM_KEYDOWN, VK_BACK);
        injected(&mut c, 0xF0C7);
        injected(&mut c, 0xF0DF);
        c.message(WM_KEYDOWN, VK_BACK);
        injected(&mut c, 0xF0C4);
        let mut events = vec![key(Key::Backspace, true), key(Key::Backspace, false), key(Key::Backspace, true), key(Key::Backspace, false)];
        c.drain(&mut events);
        assert_eq!(
            events,
            vec![
                text("\u{F0E3}"),
                key(Key::Backspace, true),
                key(Key::Backspace, false),
                text("\u{F0C7}\u{F0DF}"),
                key(Key::Backspace, true),
                key(Key::Backspace, false),
                text("\u{F0C4}"),
            ]
        );
    }

    #[test]
    fn a_symbol_goes_before_ordinary_text_typed_after_it() {
        // Two injected characters in one frame: egui reports only the second.
        let mut c = SymbolChars::default();
        injected(&mut c, 0xF0E3);
        injected(&mut c, u32::from('a'));
        let mut events = vec![text("a")];
        c.drain(&mut events);
        assert_eq!(events, vec![text("\u{F0E3}"), text("a")]);
        // A key typed after the symbol: its press and character both come after it.
        injected(&mut c, 0xF0E3);
        c.message(WM_KEYDOWN, VK_A);
        c.message(WM_CHAR, usize::from(b'a'));
        let mut events = vec![key(Key::A, true), text("a"), key(Key::A, false)];
        c.drain(&mut events);
        assert_eq!(events, vec![text("\u{F0E3}"), key(Key::A, true), text("a"), key(Key::A, false)]);
    }

    #[test]
    fn a_symbol_goes_before_a_navigation_key_pressed_after_it() {
        let mut c = SymbolChars::default();
        c.message(WM_KEYDOWN, VK_LEFT);
        injected(&mut c, 0xF0E3);
        c.message(WM_KEYDOWN, VK_LEFT);
        let mut events = vec![key(Key::ArrowLeft, true), key(Key::ArrowLeft, false), key(Key::ArrowLeft, true), key(Key::ArrowLeft, false)];
        c.drain(&mut events);
        assert_eq!(
            events,
            vec![key(Key::ArrowLeft, true), key(Key::ArrowLeft, false), text("\u{F0E3}"), key(Key::ArrowLeft, true), key(Key::ArrowLeft, false)]
        );
    }

    #[test]
    fn a_symbol_between_ordinary_characters_splits_their_text() {
        let mut c = SymbolChars::default();
        injected(&mut c, u32::from('x'));
        injected(&mut c, 0xF0AD);
        injected(&mut c, u32::from('y'));
        let mut events = vec![text("xy")];
        c.drain(&mut events);
        assert_eq!(events, vec![text("x"), text("\u{F0AD}"), text("y")]);
    }

    #[test]
    fn only_the_symbol_range_counts_and_stays_as_it_is() {
        // 0xAD is SR-Tamil's ஷ: as U+00AD it would be a soft hyphen, which never draws.
        assert_eq!(symbol_char(0xF0AD), Some('\u{F0AD}'));
        assert_eq!(symbol_char(0xF092), Some('\u{F092}'));
        assert_eq!(symbol_char(0xF020), Some('\u{F020}'));
        assert_eq!(symbol_char(0xF0FF), Some('\u{F0FF}'));
        assert_eq!(symbol_char(0xF01F), None);
        assert_eq!(symbol_char(0xF100), None);
    }

    #[test]
    fn other_characters_are_left_to_winit() {
        let mut c = SymbolChars::default();
        c.message(WM_KEYDOWN, VK_A);
        c.message(WM_CHAR, usize::from(b'a'));
        injected(&mut c, 0x0B85);
        c.message(WM_KEYDOWN, VK_BACK);
        let mut events = vec![key(Key::A, true), text("aஅ"), key(Key::Backspace, true)];
        c.drain(&mut events);
        assert_eq!(events, vec![key(Key::A, true), text("aஅ"), key(Key::Backspace, true)]);
    }
}
