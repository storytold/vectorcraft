use serde_json::json;

use super::VectorcraftApp;

/// The most fonts Type › Recent Fonts lists (Preferences › Type › Number of Recent Fonts).
pub const MAX_RECENT_FONTS: usize = 15;

impl VectorcraftApp {
    /// Type › Recent Fonts: the fonts used last, newest first, as many as Preferences › Type ›
    /// Number of Recent Fonts says.
    pub fn recent_fonts(&self) -> &[String] {
        let n = usize::try_from(self.session.prefs.recent_fonts_count).unwrap_or(MAX_RECENT_FONTS).clamp(1, MAX_RECENT_FONTS);
        self.ui.recent_fonts.get(..n).unwrap_or(&self.ui.recent_fonts)
    }
}

// PLAN: hub-w1 calls `watch_fonts` once in `VectorcraftApp::new` (beside `scan_in_background`, app.rs:49) and `follow_fonts` once per frame in `tick` (app.rs:354-359), adds the fields `font_watch: Option<craft::font_catalog::Watch>` and `fonts_seen: u64` (lib.rs:280-285), makes `FONT_INSTALL_POLL` a `Duration` (state.rs:1585) and drops the `WindowFocused` refresh (app/input.rs:210).
impl VectorcraftApp {
    /// Fonts installed or removed while the app runs are listed without Refresh Font List: the
    /// font catalog looks at the font folders every [`FONT_INSTALL_POLL`] on a thread of its own
    /// (asking the platform for the fonts font services loaded meanwhile takes tens of milliseconds
    /// on Windows, #579), rescans only when they changed, and wakes the app for
    /// [`Self::follow_fonts`]. Called once, at start; the watch stops with the app. Not on wasm,
    /// which has no installed fonts.
    ///
    /// [`FONT_INSTALL_POLL`]: crate::state::FONT_INSTALL_POLL
    pub(super) fn watch_fonts(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let ctx = ctx.clone();
            let watch = vectorcraft_text::FontDb::global().catalog().watch(crate::state::FONT_INSTALL_POLL, move |_| ctx.request_repaint());
            self.font_watch = Some(watch);
        }
        #[cfg(target_arch = "wasm32")]
        let _ = ctx;
    }

    /// Once per frame: when the catalog changed since the last look, runs `text.rescanFonts`, so
    /// documents and lists take up the fonts the watch ([`Self::watch_fonts`]) found. It reads no
    /// input, so a second pass of a frame changes nothing.
    // PLAN: keyed on the catalog's generation, as contract row ty.S2 says, not `FontDb::generation` (the settled note): that one also moves each time layout loads an installed family, which would rescan on every font first used.
    pub(super) fn follow_fonts(&mut self) {
        let catalog = vectorcraft_text::FontDb::global().catalog();
        // Until the first scan ends there is nothing to follow, and its end changes no document.
        if !catalog.is_ready() {
            return;
        }
        let generation = catalog.generation();
        if generation == self.fonts_seen {
            return;
        }
        // 0: the first look after the first scan (the generation is at least 1 once it ran).
        if self.fonts_seen != 0 {
            // A failure shows in the status bar, as the menu item's does.
            let _ = self.run("text.rescanFonts", json!({}));
        }
        // The command's own rescan moves the generation too.
        self.fonts_seen = catalog.generation();
    }
}
