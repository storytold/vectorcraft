//! Alt + numeric-keypad codes on Windows (#5108): holding Alt, typing a decimal code on the
//! numeric keypad (NumLock on) and releasing Alt types that character (`Alt+0176` → `°`).
//!
//! Windows does the work: when Alt comes up after keypad digits, `TranslateMessage` posts a
//! `WM_CHAR` holding the character the code stands for (in the system's ANSI code page for a code
//! with a leading 0, else its OEM code page), the convention every Windows text field follows
//! (<https://learn.microsoft.com/en-us/windows/win32/inputdev/about-keyboard-input>). winit
//! 0.30 reads a `WM_CHAR` only as part of the key press before it, so it drops this one, which
//! follows a key release, and egui never sees the text. winit's message hook ([`msg_hook`]) sees
//! the message before winit handles it: [`AltCodes`] notes keypad digits typed with Alt held, takes
//! the character that follows Alt's release, and [`raw_input_hook`] hands it to the frame as typed
//! text, so the Type tool (or a panel's text field) types it like any other character, in the
//! same undo step as the typing round it. The keypad digits themselves are taken out of the frame,
//! so they don't run a shortcut bound to Alt and a digit.
//!
//! With NumLock off the keypad keys arrive as Insert, End, Down, Page Down, Left, Clear, Right,
//! Home, Up and Page Up, and Windows still composes the code from them. They are told from the
//! separate navigation keys by the extended-key flag, which only the separate keys set
//! (<https://learn.microsoft.com/en-us/windows/win32/inputdev/about-keyboard-input#extended-key-flag>),
//! so with Alt held they count as digits and are taken out of the frame too, rather than moving
//! the caret before the character arrives.
//!
//! egui reports a keypad digit as the same key as the digit above the letters, so the other
//! platforms, whose systems have no such convention, can't tell them apart either.

#![cfg_attr(not(windows), allow(dead_code))]

use std::cell::RefCell;

use egui::{Event, Key};

const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_CHAR: u32 = 0x0102;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_SYSCHAR: u32 = 0x0106;
/// `VK_NUMPAD0` to `VK_NUMPAD9` (`winuser.h`): keypad digits with NumLock on.
const VK_NUMPAD0: usize = 0x60;
const VK_NUMPAD9: usize = 0x69;
/// The keypad's virtual keys with NumLock off, by digit (`winuser.h`): `VK_INSERT`, `VK_END`,
/// `VK_DOWN`, `VK_NEXT`, `VK_LEFT`, `VK_CLEAR`, `VK_RIGHT`, `VK_HOME`, `VK_UP` and `VK_PRIOR`.
const NUMLOCK_OFF_KEYS: [usize; 10] = [0x2D, 0x23, 0x28, 0x22, 0x25, 0x0C, 0x27, 0x24, 0x26, 0x21];
/// The keys egui reports for the keypad with NumLock off, by digit (Clear has no egui key, so it
/// comes as the keypad's own key, the digit).
const NUMLOCK_OFF_EGUI_KEYS: [Key; 10] =
    [Key::Insert, Key::End, Key::ArrowDown, Key::PageDown, Key::ArrowLeft, Key::Num5, Key::ArrowRight, Key::Home, Key::ArrowUp, Key::PageUp];
const DIGIT_KEYS: [Key; 10] = [Key::Num0, Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
/// Bit 24 of a key message's `lParam`: an extended key, such as the navigation keys beside the
/// keypad (the keypad's own keys clear it).
const EXTENDED_KEY: isize = 1 << 24;
/// `VK_MENU`, `VK_LMENU` and `VK_RMENU`: Alt.
const ALT_KEYS: [usize; 3] = [0x12, 0xA4, 0xA5];

thread_local! {
    /// The UI thread's sequence: [`msg_hook`] and [`raw_input_hook`] both run on it.
    static CODES: RefCell<AltCodes> = RefCell::new(AltCodes::default());
}

/// Where an Alt + keypad code stands, from the keyboard messages of the thread's queue.
#[derive(Default)]
struct AltCodes {
    /// Keypad digits typed while Alt has been held.
    digits: usize,
    /// Alt came up after keypad digits: the next character message is the code's character.
    armed: bool,
    /// A high surrogate waiting for its low half.
    high: Option<u16>,
    /// Characters the codes typed, for the next frame.
    typed: String,
    /// The digits of the keypad presses and releases with Alt held since the last frame, which
    /// egui reports as Alt and a digit (or, with NumLock off, Alt and a navigation key): taken
    /// out of the frame.
    keypad_events: Vec<usize>,
}

/// The digit a keypad key stands for in an Alt code: `wparam` is its virtual-key code and
/// `lparam` the message's flags.
fn keypad_digit(wparam: usize, lparam: isize) -> Option<usize> {
    if (VK_NUMPAD0..=VK_NUMPAD9).contains(&wparam) {
        return Some(wparam - VK_NUMPAD0);
    }
    if lparam & EXTENDED_KEY != 0 {
        return None;
    }
    NUMLOCK_OFF_KEYS.iter().position(|&k| k == wparam)
}

impl AltCodes {
    /// Note a keyboard message `message` with `wparam` (the virtual-key code, or a UTF-16 unit)
    /// and `lparam` (its flags).
    fn message(&mut self, message: u32, wparam: usize, lparam: isize) {
        let keypad = match message {
            WM_SYSKEYDOWN | WM_SYSKEYUP => keypad_digit(wparam, lparam),
            _ => None,
        };
        if let Some(d) = keypad {
            // Windows sends a key pressed with Alt held as a system key.
            if message == WM_SYSKEYDOWN {
                self.digits += 1;
                self.armed = false;
            }
            self.keypad_events.push(d);
            return;
        }
        match message {
            WM_KEYDOWN | WM_SYSKEYDOWN if ALT_KEYS.contains(&wparam) => {}
            // Any other key ends the sequence and keeps its own meaning.
            WM_KEYDOWN | WM_SYSKEYDOWN => self.reset(),
            WM_KEYUP | WM_SYSKEYUP if ALT_KEYS.contains(&wparam) => {
                self.armed = self.digits > 0;
                self.digits = 0;
            }
            WM_CHAR | WM_SYSCHAR if self.armed => {
                let unit = (wparam & 0xffff) as u16;
                if (0xd800..0xdc00).contains(&unit) {
                    self.high = Some(unit);
                    return;
                }
                let units: Vec<u16> = self.high.take().into_iter().chain([unit]).collect();
                self.typed.extend(char::decode_utf16(units).filter_map(Result::ok).filter(|c| !c.is_control()));
                self.armed = false;
            }
            _ => {}
        }
    }

    fn reset(&mut self) {
        self.digits = 0;
        self.armed = false;
        self.high = None;
    }

    /// Hand the frame `events` the characters typed since the last frame, as typed text, and take
    /// out the keypad digits typed with Alt held.
    fn drain(&mut self, events: &mut Vec<Event>) {
        let mut keypad = std::mem::take(&mut self.keypad_events);
        if !keypad.is_empty() {
            events.retain(|e| {
                let Event::Key { key, modifiers, .. } = e else {
                    return true;
                };
                if !modifiers.alt {
                    return true;
                }
                // The first keypad key still to take out that egui reports as `key`.
                let stands_for = |d: &usize| DIGIT_KEYS.get(*d) == Some(key) || NUMLOCK_OFF_EGUI_KEYS.get(*d) == Some(key);
                match keypad.iter().position(stands_for) {
                    Some(i) => {
                        keypad.remove(i);
                        false
                    }
                    None => true,
                }
            });
        }
        if !self.typed.is_empty() {
            events.push(Event::Text(std::mem::take(&mut self.typed)));
        }
    }
}

/// winit's message hook for the keyboard: notes Alt + keypad codes and the characters they type.
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
    if (WM_KEYDOWN..=WM_SYSCHAR).contains(&msg.message) {
        CODES.with(|c| {
            if let Ok(mut c) = c.try_borrow_mut() {
                c.message(msg.message, msg.wParam.0, msg.lParam.0);
            }
        });
    }
}

/// Before egui reads the main window's frame: the characters Alt + keypad codes typed, as text.
pub fn raw_input_hook(raw: &mut egui::RawInput) {
    CODES.with(|c| {
        if let Ok(mut c) = c.try_borrow_mut() {
            c.drain(&mut raw.events);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALT: usize = 0x12;

    /// Alt held, keypad digits `code`, Alt released, and the character Windows then posts.
    fn alt_code(c: &mut AltCodes, code: &str, posted: u16) {
        c.message(WM_SYSKEYDOWN, ALT, 0);
        for d in code.bytes() {
            let vk = VK_NUMPAD0 + usize::from(d - b'0');
            c.message(WM_SYSKEYDOWN, vk, 0);
            c.message(WM_SYSKEYUP, vk, 0);
        }
        c.message(WM_KEYUP, ALT, 0);
        c.message(WM_CHAR, usize::from(posted), 0);
    }

    fn key(key: Key, alt: bool, pressed: bool) -> Event {
        let modifiers = egui::Modifiers { alt, ..Default::default() };
        Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers }
    }

    #[test]
    fn an_alt_code_types_its_character_when_alt_comes_up() {
        let mut c = AltCodes::default();
        alt_code(&mut c, "0176", 0xB0);
        alt_code(&mut c, "0151", 0x2014);
        let mut events = vec![key(Key::Num0, true, true), key(Key::Num0, true, false), key(Key::Num1, false, true)];
        c.drain(&mut events);
        // The keypad digits typed with Alt are gone (eight presses and eight releases, two here);
        // the digit typed without Alt stays, and the characters follow.
        assert_eq!(events, vec![key(Key::Num1, false, true), Event::Text("°—".into())]);
        let mut events = vec![key(Key::Num0, true, true)];
        c.drain(&mut events);
        assert_eq!(events.len(), 1, "counts don't carry over to a later frame");
    }

    #[test]
    fn a_character_without_a_code_before_it_is_left_to_winit() {
        let mut c = AltCodes::default();
        // A plain key press's character (winit types it), and Alt pressed and released alone.
        c.message(WM_KEYDOWN, 0x41, 0);
        c.message(WM_CHAR, usize::from(b'a'), 0);
        c.message(WM_SYSKEYDOWN, ALT, 0);
        c.message(WM_KEYUP, ALT, 0);
        c.message(WM_CHAR, usize::from(b'b'), 0);
        let mut events = vec![];
        c.drain(&mut events);
        assert!(events.is_empty(), "{events:?}");
    }

    #[test]
    fn another_key_while_alt_is_held_cancels_the_code() {
        let mut c = AltCodes::default();
        c.message(WM_SYSKEYDOWN, ALT, 0);
        c.message(WM_SYSKEYDOWN, VK_NUMPAD0 + 1, 0);
        // Alt+F: a shortcut, not a code.
        c.message(WM_SYSKEYDOWN, 0x46, 0);
        c.message(WM_KEYUP, ALT, 0);
        c.message(WM_CHAR, 0x41, 0);
        let mut events = vec![];
        c.drain(&mut events);
        assert!(!events.iter().any(|e| matches!(e, Event::Text(_))), "{events:?}");
    }

    #[test]
    fn a_code_above_the_basic_plane_arrives_as_two_halves() {
        let mut c = AltCodes::default();
        c.message(WM_SYSKEYDOWN, ALT, 0);
        c.message(WM_SYSKEYDOWN, VK_NUMPAD0 + 1, 0);
        c.message(WM_KEYUP, ALT, 0);
        c.message(WM_CHAR, 0xD83D, 0);
        c.message(WM_CHAR, 0xDE00, 0);
        let mut events = vec![];
        c.drain(&mut events);
        assert_eq!(events, vec![Event::Text("😀".into())]);
    }

    /// Alt held, keypad digits `code` with NumLock off (keypad Insert, End, …, flags without the
    /// extended-key bit), Alt released, and the character Windows then posts.
    fn alt_code_numlock_off(c: &mut AltCodes, code: &str, posted: u16) {
        c.message(WM_SYSKEYDOWN, ALT, 0);
        for d in code.bytes() {
            let vk = NUMLOCK_OFF_KEYS[usize::from(d - b'0')];
            c.message(WM_SYSKEYDOWN, vk, 0);
            c.message(WM_SYSKEYUP, vk, 0);
        }
        c.message(WM_KEYUP, ALT, 0);
        c.message(WM_CHAR, usize::from(posted), 0);
    }

    #[test]
    fn with_numlock_off_the_keypad_keys_type_the_code_and_leave_the_caret_alone() {
        let mut c = AltCodes::default();
        alt_code_numlock_off(&mut c, "0176", 0xB0);
        // What egui reports for keypad 0, 1, 7 and 6 with NumLock off and Alt held, a press and a
        // release each, then a plain Home typed after Alt came up.
        let mut events = vec![];
        for k in [Key::Insert, Key::End, Key::Home, Key::ArrowRight] {
            events.push(key(k, true, true));
            events.push(key(k, true, false));
        }
        events.push(key(Key::Home, false, true));
        c.drain(&mut events);
        assert_eq!(events, vec![key(Key::Home, false, true), Event::Text("°".into())]);
    }

    #[test]
    fn with_numlock_off_keypad_five_counts_too() {
        let mut c = AltCodes::default();
        // Keypad 5 is Clear, which egui reports as the keypad's 5; 0151 uses every other kind.
        alt_code_numlock_off(&mut c, "0151", 0x2014);
        alt_code_numlock_off(&mut c, "0234", 0xEA);
        let mut events =
            vec![key(Key::Num5, true, true), key(Key::ArrowDown, true, true), key(Key::PageDown, true, true), key(Key::ArrowLeft, true, true)];
        c.drain(&mut events);
        assert_eq!(events, vec![Event::Text("—ê".into())]);
    }

    #[test]
    fn the_separate_navigation_keys_still_move_the_caret_and_cancel_the_code() {
        let mut c = AltCodes::default();
        c.message(WM_SYSKEYDOWN, ALT, 0);
        c.message(WM_SYSKEYDOWN, NUMLOCK_OFF_KEYS[1], 0);
        c.message(WM_SYSKEYUP, NUMLOCK_OFF_KEYS[1], 0);
        // Alt+Home on the separate Home key (extended): a shortcut, not a digit.
        c.message(WM_SYSKEYDOWN, NUMLOCK_OFF_KEYS[7], EXTENDED_KEY);
        c.message(WM_SYSKEYUP, NUMLOCK_OFF_KEYS[7], EXTENDED_KEY);
        c.message(WM_KEYUP, ALT, 0);
        c.message(WM_CHAR, 0x41, 0);
        let mut events = vec![key(Key::End, true, true), key(Key::End, true, false), key(Key::Home, true, true), key(Key::Home, true, false)];
        c.drain(&mut events);
        assert_eq!(events, vec![key(Key::Home, true, true), key(Key::Home, true, false)]);
    }

    #[test]
    fn numlock_off_keys_without_alt_are_left_alone() {
        let mut c = AltCodes::default();
        c.message(WM_KEYDOWN, NUMLOCK_OFF_KEYS[7], 0);
        c.message(WM_KEYUP, NUMLOCK_OFF_KEYS[7], 0);
        let mut events = vec![key(Key::Home, false, true), key(Key::Home, false, false)];
        c.drain(&mut events);
        assert_eq!(events.len(), 2);
    }
}
