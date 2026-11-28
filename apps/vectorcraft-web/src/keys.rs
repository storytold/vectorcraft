//! Shortcuts the browser would also act on. Many of the app's Cmd and Ctrl chords are the
//! browser's too: the View menu's `Cmd+=`, `Cmd+-` and `Cmd+0` zoom the page, Edit's `Cmd+Z` and
//! `Cmd+Shift+Z` undo and redo in the browser (in Safari, Undo Close Tab when the page has nothing
//! to undo), `Cmd+[` and `Cmd+]` go back and forward, `Cmd+F` finds. eframe lets them through, so
//! the browser acted on them as well. For a press the keymap in effect binds (the user's shortcuts
//! included), the page's default action is cancelled; the key still reaches egui, which runs the
//! command (or a text field its own undo). A few chords never reach a page at all (Safari's
//! `Cmd+L`, Chrome's `Cmd+T`, `Cmd+W` and `Cmd+N`): the browser keeps those.

use craft::keymap::Chord;
// PLAN: assumed path `craft::ui::runtime::commands` for the ui contract's `commands` module.
#[cfg(target_arch = "wasm32")]
use craft::ui::runtime::commands;
use egui::{Key, Modifiers};

/// The chord of a key press the browser's own action may be cancelled for. `key` is the event's
/// `key` (the character typed), `code` its `code` (the key's position), `modifiers` as eframe
/// reports them, and `alt_graph` whether AltGr is held.
///
/// Only Cmd and Ctrl presses: a plain or Shift press types (cancelling it would drop the
/// character it types), and so does AltGr, which Windows reports as Ctrl and Alt held. Cut, Copy
/// and Paste (with any other modifiers) are left to the browser, whose clipboard events carry
/// their text, and Reload too, as eframe leaves them.
pub(crate) fn browser_chord(key: &str, code: &str, modifiers: Modifiers, alt_graph: bool) -> Option<Chord> {
    if !(modifiers.ctrl || modifiers.mac_cmd) || alt_graph {
        return None;
    }
    let key = Key::from_name(key).or(Key::from_name(code))?;
    if matches!(key, Key::C | Key::V | Key::X | Key::R) {
        return None;
    }
    vectorcraft_ui_egui::keymap::chord_of_press(key, modifiers, vectorcraft_ui_egui::keymap::platform())
}

/// Whether the browser's own action for a key press is cancelled: the keymap in effect, the
/// user's shortcuts included, binds its chord. Before the first frame nothing is bound.
#[cfg(target_arch = "wasm32")]
pub(crate) fn kept_from_browser(ctx: &egui::Context, key: &str, code: &str, modifiers: Modifiers, alt_graph: bool) -> bool {
    browser_chord(key, code, modifiers, alt_graph).is_some_and(|chord| commands::holder(ctx, &chord).is_some())
}

/// The egui context of the running app; empty until the first start fills it, and filled again by
/// every restart after the graphics were lost.
#[cfg(target_arch = "wasm32")]
pub(crate) type ContextSlot = std::rc::Rc<std::cell::RefCell<Option<egui::Context>>>;

/// The modifiers of a key event as eframe reports them to egui (its `modifiers_from_kb_event`).
#[cfg(target_arch = "wasm32")]
fn modifiers(e: &web_sys::KeyboardEvent) -> Modifiers {
    Modifiers { alt: e.alt_key(), ctrl: e.ctrl_key(), shift: e.shift_key(), mac_cmd: e.meta_key(), command: e.ctrl_key() || e.meta_key() }
}

/// Whether the app has the keyboard for this event: it went to the canvas, or to the text input
/// eframe places right after it while a text field has the focus. Elsewhere (the page's body, a
/// file input) eframe ignores the key, so the browser keeps it.
#[cfg(target_arch = "wasm32")]
fn to_the_app(e: &web_sys::KeyboardEvent) -> bool {
    use wasm_bindgen::JsCast as _;
    let Some(target) = e.target() else { return false };
    if target.has_type::<web_sys::HtmlCanvasElement>() {
        return true;
    }
    target
        .dyn_ref::<web_sys::HtmlInputElement>()
        .and_then(|input| input.previous_element_sibling())
        .is_some_and(|before| before.has_type::<web_sys::HtmlCanvasElement>())
}

/// Cancel the page's default action for [`kept_from_browser`] presses, before eframe's listener
/// sees the event (it doesn't stop it). The framework cancels no browser default itself, so this
/// listener stays; it reads the context from `ctx` because it is installed before the first frame.
#[cfg(target_arch = "wasm32")]
pub(crate) fn keep_app_keys(ctx: ContextSlot) {
    use wasm_bindgen::JsCast as _;
    let Some(window) = web_sys::window() else { return };
    let on_key = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| {
        // An IME's keys are its own until it commits.
        if e.is_composing() || !to_the_app(&e) {
            return;
        }
        let Some(ctx) = ctx.borrow().clone() else { return };
        if kept_from_browser(&ctx, &e.key(), &e.code(), modifiers(&e), e.get_modifier_state("AltGraph")) {
            e.prevent_default();
        }
    });
    if let Err(e) = window.add_event_listener_with_callback_and_bool("keydown", on_key.as_ref().unchecked_ref(), true) {
        log::error!("couldn't follow keydown events: {e:?}");
    }
    // The listener lives as long as the page.
    on_key.forget();
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::keymap::{Key as ChordKey, Mods};

    /// Cmd as egui reports it on this platform: the Command key on macOS, Ctrl elsewhere.
    fn cmd() -> Modifiers {
        if cfg!(target_os = "macos") { Modifiers::MAC_CMD | Modifiers::COMMAND } else { Modifiers::CTRL | Modifiers::COMMAND }
    }

    #[test]
    fn the_keymaps_chords_are_kept_from_the_browser() {
        for (key, code, mods, want) in [
            ("=", "Equal", cmd(), Chord::new(Mods::CMD, ChordKey::Equals)),
            ("-", "Minus", cmd(), Chord::parse("Cmd+-").unwrap()),
            ("0", "Digit0", cmd(), Chord::parse("Cmd+0").unwrap()),
            ("z", "KeyZ", cmd(), Chord::parse("Cmd+Z").unwrap()),
            ("Z", "KeyZ", cmd() | Modifiers::SHIFT, Chord::parse("Cmd+Shift+Z").unwrap()),
            ("[", "BracketLeft", cmd(), Chord::parse("Cmd+[").unwrap()),
        ] {
            assert_eq!(browser_chord(key, code, mods, false), Some(want), "{key} ({code}) {mods:?}");
        }
    }

    #[test]
    fn typing_the_clipboard_and_reload_stay_with_the_browser() {
        for (key, code, mods) in [
            // Typing: plain and Shift presses.
            ("z", "KeyZ", Modifiers::NONE),
            ("Z", "KeyZ", Modifiers::SHIFT),
            ("=", "Equal", Modifiers::NONE),
            // The clipboard's events carry the text; Paste and Match Style is Shift+Cmd+V.
            ("c", "KeyC", cmd()),
            ("x", "KeyX", cmd()),
            ("v", "KeyV", cmd()),
            ("V", "KeyV", cmd() | Modifiers::SHIFT),
            ("r", "KeyR", cmd()),
        ] {
            assert!(browser_chord(key, code, mods, false).is_none(), "{key} ({code}) {mods:?}");
        }
        // AltGr+Z types ż on a Polish layout, reported as Ctrl and Alt held on Windows.
        let ctrl_alt = Modifiers::CTRL | Modifiers::COMMAND | Modifiers::ALT;
        assert!(browser_chord("ż", "KeyZ", ctrl_alt, true).is_none());
        assert!(browser_chord("}", "Digit0", ctrl_alt, true).is_none());
    }
}
