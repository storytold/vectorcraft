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
            let (ctx, changed) = (ctx.clone(), std::sync::Arc::clone(&self.fonts_changed));
            let watch = vectorcraft_text::FontDb::global().catalog().watch(crate::state::FONT_INSTALL_POLL, move |_| {
                changed.store(true, std::sync::atomic::Ordering::Release);
                ctx.request_repaint();
            });
            self.font_watch = Some(watch);
        }
        #[cfg(target_arch = "wasm32")]
        let _ = ctx;
    }

    /// Once per frame: when the watch ([`Self::watch_fonts`]) found fonts installed or removed since the last look,
    /// runs `text.rescanFonts`, so documents and lists take up the fonts it found. It reads no input, so a second pass
    /// of a frame changes nothing.
    // PLAN: contract row ty.S2 keys this on the catalog's generation; that also moves when a document's own fonts are
    // added or removed and when another app in the process rescans, which production's look at the installed fonts
    // never rescanned for, so it is keyed on what the watch found.
    pub(super) fn follow_fonts(&mut self) {
        if self.fonts_changed.swap(false, std::sync::atomic::Ordering::AcqRel) {
            // A failure shows in the status bar, as the menu item's does.
            let _ = self.run("text.rescanFonts", json!({}));
        }
    }
}
