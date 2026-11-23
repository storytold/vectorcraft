//! Alt (Option on macOS) pressed and let go with no other key, click or wheel turn in between: the
//! active tool gets [`ToolKey::AltTap`](vectorcraft_tools::ToolKey::AltTap) when it claims it (the
//! Quick Pen tool switches to its next action).

/// Alt held down since it went down, and whether anything else happened since.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct AltTap {
    down: bool,
    spoiled: bool,
}

impl AltTap {
    /// The modifiers changed: `alt` is whether Alt is held, `alone` whether it is held without
    /// Shift, Ctrl or Cmd. True when Alt was let go after a press with nothing else in between.
    pub(crate) fn modifiers(&mut self, alt: bool, alone: bool) -> bool {
        match (self.down, alt) {
            (false, true) => {
                (self.down, self.spoiled) = (true, !alone);
                false
            }
            (true, true) => {
                self.spoiled |= !alone;
                false
            }
            (true, false) => {
                let tap = !self.spoiled;
                *self = Self::default();
                tap
            }
            (false, false) => false,
        }
    }

    /// A key press, a click or a wheel turn: Alt held now is no tap.
    pub(crate) fn other(&mut self) {
        self.spoiled = true;
    }
}

fn alt_alone(m: egui::Modifiers) -> (bool, bool) {
    (m.alt, !(m.shift || m.ctrl || m.command || m.mac_cmd))
}

/// Follows the frame's events in order, then its modifiers at its end; true when Alt was tapped,
/// also when it went down and up within the frame. A frame without the keyboard focus drops a
/// press under way.
pub(super) fn follow(tap: &mut AltTap, ctx: &egui::Context, focused: bool) -> bool {
    if !focused {
        *tap = AltTap::default();
        return false;
    }
    ctx.input(|i| {
        let mut tapped = false;
        for e in &i.events {
            match e {
                egui::Event::ModifiersChanged(m) => {
                    let (alt, alone) = alt_alone(*m);
                    tapped |= tap.modifiers(alt, alone);
                }
                egui::Event::Key { pressed: true, .. }
                | egui::Event::PointerButton { .. }
                | egui::Event::MouseWheel { .. }
                | egui::Event::Touch { .. } => tap.other(),
                _ => {}
            }
        }
        let (alt, alone) = alt_alone(i.modifiers);
        tapped | tap.modifiers(alt, alone)
    })
}

#[cfg(test)]
mod tests {
    use super::{AltTap, follow};

    #[test]
    fn a_press_and_release_alone_is_a_tap() {
        let mut t = AltTap::default();
        assert!(!t.modifiers(true, true));
        assert!(!t.modifiers(true, true));
        assert!(t.modifiers(false, true));
        assert!(!t.modifiers(false, true), "once");
    }

    #[test]
    fn anything_else_while_alt_is_down_is_no_tap() {
        // A key (Alt+P), a click or a wheel turn while held.
        let mut t = AltTap::default();
        t.modifiers(true, true);
        t.other();
        assert!(!t.modifiers(false, true));
        // With Shift held too.
        t.modifiers(true, false);
        assert!(!t.modifiers(false, true));
        // Something before Alt went down does not count, and the next press starts afresh.
        t.other();
        t.modifiers(true, true);
        assert!(t.modifiers(false, true));
    }

    /// Alt down and up between two frames: both changes come in one frame's events, whose
    /// modifiers at its end hold no Alt.
    #[test]
    fn a_tap_within_one_frame_counts() {
        let ctx = egui::Context::default();
        let mut t = AltTap::default();
        let mut tapped = false;
        let events = vec![egui::Event::ModifiersChanged(egui::Modifiers::ALT), egui::Event::ModifiersChanged(egui::Modifiers::NONE)];
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| tapped = follow(&mut t, ui.ctx(), true));
        out.textures_delta.clear();
        assert!(tapped);
        assert_eq!(t, AltTap::default());
    }
}
