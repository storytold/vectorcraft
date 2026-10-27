use super::*;

impl VectorcraftApp {
    /// Hand a queued text copy to egui. Called after `tick`, after the UI phase's input and at the
    /// end of the pass; the later calls are what an input- or menu-driven fallback Copy needs, and
    /// `take` keeps it single-shot.
    pub(super) fn flush_clipboard_out(&mut self, ctx: &egui::Context) {
        if let Some(t) = self.clipboard_out.take() {
            ctx.copy_text(t);
        }
    }
}
