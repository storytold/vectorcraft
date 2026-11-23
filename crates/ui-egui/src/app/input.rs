use super::{VectorcraftApp, menus, shortcuts};

impl VectorcraftApp {
    /// Each pass, before the UI: the rich clipboard formats a paste into a text field must find
    /// written, and the clicks of the native menu bar. egui gives a frame's events to its first
    /// pass only, so a second pass of the frame finds no new paste or click and repeats nothing.
    pub(super) fn handle_input(&mut self, ctx: &egui::Context) {
        self.write_formats_before_text_input(ctx);
        menus::native::run(self, ctx);
        // A Copy taken by the native menu is published in this same pass: an idle web host may
        // have no further frame to run it in.
        self.flush_clipboard_out();
    }

    /// Each pass, last in the UI: the window's keys that no component took. Escape ends one thing
    /// (a gesture, a mode, the screen mode), unless the menu bar is open or has the keys; then a
    /// tool held by its key goes back once what deferred its return has ended. Both read only the
    /// intents of their own pass, so a second pass of a frame applies none of the first's.
    pub(super) fn window_keys(&mut self, ctx: &egui::Context) {
        // PLAN: assumed `craft::menu::bar::{is_open, is_focused}(&egui::Context) -> bool` (menus-w6's request names them; the framework tree has neither yet).
        if !craft::menu::bar::is_open(ctx) && !craft::menu::bar::is_focused(ctx) {
            shortcuts::escape(self, ctx);
        }
        shortcuts::note_field_keys(self, ctx);
        shortcuts::follow_holds(self, ctx);
        shortcuts::settings_on_a_mac(self, ctx);
    }
}
