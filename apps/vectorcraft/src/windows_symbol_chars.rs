//! Characters a keyboard tool types in a symbol font's encoding on Windows, such as NHM Writer's
//! legacy Tamil keymaps for SR-Tamil fonts.
//!
//! Such a tool posts each byte of the font's 8-bit encoding as a `WM_CHAR` holding U+F000 plus the
//! byte, Windows' convention for symbol fonts. Those are Private Use Area characters, which
//! egui-winit drops as unprintable (`is_printable_char`), so the Type tool gets nothing (the tool's Backspace presses,
//! which it sends to rewrite a syllable, still arrive). [`msg_hook`] notes the characters in order
//! with the Backspace presses, and [`raw_input_hook`] hands them to the frame unchanged, each one
//! before the Backspace press that followed it. A symbol font maps exactly these code points (its
//! Windows cmap is (3,0) at U+F020..=U+F0FF), so they draw as typed; turning them into the bytes'
//! Latin-1 characters instead would make byte 0xAD (SR-Tamil's ஷ) a soft hyphen, which the text
//! engine never draws, and bytes 0x80..=0x9F control characters.

#![cfg_attr(not(windows), allow(dead_code))]

use std::cell::RefCell;
use std::collections::VecDeque;

use egui::{Event, Key};

const WM_KEYDOWN: u32 = 0x0100;
const WM_CHAR: u32 = 0x0102;
/// `VK_BACK` (`winuser.h`): Backspace.
const VK_BACK: usize = 0x08;
/// U+F020..=U+F0FF: a symbol font's bytes 0x20..=0xFF.
const SYMBOL_FIRST: u32 = 0xF020;
const SYMBOL_LAST: u32 = 0xF0FF;

thread_local! {
    /// The UI thread's sequence: [`msg_hook`] and [`raw_input_hook`] both run on it.
    static TYPED: RefCell<SymbolChars> = RefCell::new(SymbolChars::default());
}

/// A keyboard message that matters here, in the order the window got it.
#[derive(Debug, PartialEq)]
enum Typed {
    /// A symbol-font character (U+F020..=U+F0FF).
    Char(char),
    /// A Backspace press.
    Backspace,
}

#[derive(Default)]
struct SymbolChars {
    typed: VecDeque<Typed>,
}

/// The symbol-font character `unit` is, or `None` for any other code point.
fn symbol_char(unit: u32) -> Option<char> {
    (SYMBOL_FIRST..=SYMBOL_LAST).contains(&unit).then(|| char::from_u32(unit)).flatten()
}

fn is_symbol(c: char) -> bool {
    (SYMBOL_FIRST..=SYMBOL_LAST).contains(&u32::from(c))
}

impl SymbolChars {
    /// Note a keyboard message `message` with `wparam` (the virtual-key code, or a UTF-16 unit).
    fn message(&mut self, message: u32, wparam: usize) {
        match message {
            WM_KEYDOWN if wparam == VK_BACK => self.typed.push_back(Typed::Backspace),
            WM_CHAR => {
                if let Some(c) = u32::try_from(wparam & 0xffff).ok().and_then(symbol_char) {
                    self.typed.push_back(Typed::Char(c));
                }
            }
            _ => {}
        }
    }

    /// Hand the frame `events` the symbol-font characters typed since the last frame, as typed
    /// text, each before the Backspace press that followed it.
    fn drain(&mut self, events: &mut Vec<Event>) {
        let mut typed = std::mem::take(&mut self.typed);
        if !typed.iter().any(|t| matches!(t, Typed::Char(_))) {
            return;
        }
        // Should egui ever pass these characters on itself, leave them to it.
        if events.iter().any(|e| matches!(e, Event::Text(t) if t.chars().any(is_symbol))) {
            return;
        }
        let mut out = Vec::with_capacity(events.len() + typed.len());
        for e in events.drain(..) {
            let backspace = matches!(e, Event::Key { key: Key::Backspace, pressed: true, .. });
            if backspace && typed.contains(&Typed::Backspace) {
                // The characters typed before this Backspace press go first.
                let mut text = String::new();
                while let Some(t) = typed.pop_front() {
                    match t {
                        Typed::Char(c) => text.push(c),
                        Typed::Backspace => break,
                    }
                }
                if !text.is_empty() {
                    out.push(Event::Text(text));
                }
            }
            out.push(e);
        }
        let rest: String = typed
            .into_iter()
            .filter_map(|t| match t {
                Typed::Char(c) => Some(c),
                Typed::Backspace => None,
            })
            .collect();
        if !rest.is_empty() {
            out.push(Event::Text(rest));
        }
        *events = out;
    }
}

/// winit's message hook for the keyboard: notes symbol-font characters and Backspace presses.
/// It never consumes the message.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn msg_hook(msg: *const std::ffi::c_void) {
    use windows::Win32::UI::WindowsAndMessaging::MSG;
    // SAFETY: winit passes the `MSG` that `PeekMessageW` has just filled, which lives for this
    // call; `as_ref` gives `None` for a null pointer.
    let Some(msg) = (unsafe { msg.cast::<MSG>().as_ref() }) else {
        return;
    };
    if msg.message == WM_KEYDOWN || msg.message == WM_CHAR {
        TYPED.with(|t| {
            if let Ok(mut t) = t.try_borrow_mut() {
                t.message(msg.message, msg.wParam.0);
            }
        });
    }
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

    /// `VK_PACKET`: the virtual key a keyboard tool's injected character comes with.
    const VK_PACKET: usize = 0xE7;

    fn typed(c: &mut SymbolChars, unit: u32) {
        c.message(WM_KEYDOWN, VK_PACKET);
        c.message(WM_CHAR, unit as usize);
    }

    fn backspace(pressed: bool) -> Event {
        Event::Key { key: Key::Backspace, physical_key: Some(Key::Backspace), pressed, repeat: false, modifiers: egui::Modifiers::default() }
    }

    #[test]
    fn a_symbol_font_character_is_typed_as_it_came() {
        let mut c = SymbolChars::default();
        typed(&mut c, 0xF0E3);
        typed(&mut c, 0xF05B);
        let mut events = vec![];
        c.drain(&mut events);
        assert_eq!(events, vec![Event::Text("\u{F0E3}\u{F05B}".into())]);
        let mut events = vec![];
        c.drain(&mut events);
        assert!(events.is_empty(), "nothing carries over to a later frame");
    }

    #[test]
    fn characters_keep_their_place_around_backspace() {
        // What NHM Writer sends while rewriting a syllable (from a real log).
        let mut c = SymbolChars::default();
        typed(&mut c, 0xF0E3);
        c.message(WM_KEYDOWN, VK_BACK);
        typed(&mut c, 0xF0C7);
        typed(&mut c, 0xF0DF);
        c.message(WM_KEYDOWN, VK_BACK);
        typed(&mut c, 0xF0C4);
        let mut events = vec![backspace(true), backspace(false), backspace(true), backspace(false)];
        c.drain(&mut events);
        assert_eq!(
            events,
            vec![
                Event::Text("\u{F0E3}".into()),
                backspace(true),
                backspace(false),
                Event::Text("\u{F0C7}\u{F0DF}".into()),
                backspace(true),
                backspace(false),
                Event::Text("\u{F0C4}".into()),
            ]
        );
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
        c.message(WM_KEYDOWN, 0x41);
        c.message(WM_CHAR, usize::from(b'a'));
        typed(&mut c, 0x0B85);
        c.message(WM_KEYDOWN, VK_BACK);
        let mut events = vec![backspace(true)];
        c.drain(&mut events);
        assert_eq!(events, vec![backspace(true)]);
    }
}
