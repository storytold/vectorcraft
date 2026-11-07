use super::{Key, Modifiers, all_shortcuts};

/// Keys that paste with Cmd, or alone. (Shift+Insert is left out: Ctrl+Insert copies.)
const PASTE_KEYS: [Key; 2] = [Key::V, Key::Paste];

/// The paste command whose V chord `held` holds (Paste in Place for Cmd+Shift+V by default), else
/// Paste: egui reports every Cmd+V chord as a paste, never as its key.
pub(crate) fn paste_command(held: Modifiers) -> &'static str {
    all_shortcuts()
        .into_iter()
        .find(|(sc, id, _)| sc.logical_key == Key::V && id.starts_with("edit.paste") && held.matches_logically(sc.modifiers))
        .map_or("edit.paste", |(_, id, _)| id)
}

/// Give a copy or cut chord held with more modifiers its key back when it has a shortcut of its own:
/// egui reports every Cmd+C and Cmd+X chord as Copy or Cut, Shift and Alt held too, so Cmd+Shift+C
/// (Align center) copied instead (#1940).
pub(super) fn unclip_chords(i: &mut egui::InputState) {
    if !i.events.iter().any(|e| matches!(e, egui::Event::Copy | egui::Event::Cut)) {
        return;
    }
    let (held, shortcuts) = (i.modifiers, all_shortcuts());
    for e in &mut i.events {
        let (key, own) = match e {
            egui::Event::Copy => (Key::C, "edit.copy"),
            egui::Event::Cut => (Key::X, "edit.cut"),
            _ => continue,
        };
        if shortcuts.iter().any(|(sc, id, _)| sc.logical_key == key && *id != own && held.matches_exact(sc.modifiers)) {
            *e = egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: held };
        }
    }
}

/// Keyboard pastes of something other than text. egui swallows a paste chord's key press and
/// sends a Paste event only when the clipboard holds text, so a paste key released without its
/// press (or a Paste event) reported was a paste of a bitmap or a PDF.
#[derive(Clone, Default)]
pub(crate) struct PasteChord {
    /// Paste keys whose press was reported (typed, not pasting).
    pressed: Vec<Key>,
    /// A Paste event came since the last paste key was released.
    pasted: bool,
}

impl PasteChord {
    /// Follow one frame's `events` → the modifiers of a paste egui sent no event for (the chord's,
    /// as its key is released).
    pub(crate) fn textless_paste(&mut self, events: &[egui::Event]) -> Option<Modifiers> {
        let mut fire = None;
        for e in events {
            match e {
                egui::Event::Paste(_) => self.pasted = true,
                // A release that comes while the window is away is never seen.
                egui::Event::WindowFocused(false) => *self = Self::default(),
                egui::Event::Key { key, pressed: true, .. } if PASTE_KEYS.contains(key) && !self.pressed.contains(key) => self.pressed.push(*key),
                egui::Event::Key { key, pressed: false, modifiers, .. } if PASTE_KEYS.contains(key) => {
                    match self.pressed.iter().position(|k| k == key) {
                        Some(i) => {
                            self.pressed.swap_remove(i);
                        }
                        // Unless a Paste event came (the guard takes it either way).
                        None if !std::mem::take(&mut self.pasted) => fire = Some(*modifiers),
                        None => {}
                    }
                }
                _ => {}
            }
        }
        fire
    }
}
