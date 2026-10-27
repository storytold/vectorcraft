use super::*;
use crate::ime;

impl VectorcraftApp {
    /// Did the Type tool end an IME composition on its own this frame? Interrupting the IME
    /// through egui (`should_interrupt_composition`) doesn't reach the macOS input context, which
    /// keeps the marked text and types it again into the next composition: the host discards it.
    pub fn take_ime_discard(&mut self) -> bool {
        std::mem::take(&mut self.ime_discard)
    }
}

impl VectorcraftApp {
    /// Install once during host Context creation, before its first run_ui.
    pub fn install_ime_reset_hooks(ctx: &egui::Context) {
        ime::install_hooks(ctx);
    }

    /// Host logic wrappers call this after their last application mutation. Full-pass logic
    /// leaves final output observation to UI; standalone logic retires unavailable publication.
    pub fn finish_ime_logic(&mut self, ctx: &egui::Context) {
        ime::finish_logic(self, ctx);
    }
}
