use super::*;

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

impl VectorcraftApp {
    /// Fonts installed or removed while the app runs are listed without Refresh Font List: a look
    /// at the font folders and the platform's font list on a timer and when the window comes back,
    /// a scan only when they changed.
    /// The look runs on another thread (asking DirectWrite for the fonts font services loaded
    /// meanwhile takes tens of milliseconds on Windows, #579), [`Self::poll_font_check`] scans;
    /// `wake` wakes the app for the answer.
    pub(super) fn refresh_installed_fonts(&mut self, wake: bool) {
        if let Some((_, w)) = &mut self.font_check {
            *w |= wake;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let look = move || {
            // The UI gone meanwhile has nothing to refresh.
            let _ = tx.send(vectorcraft_text::FontDb::global().installed_fonts_changed());
        };
        let spawned =
            if cfg!(target_arch = "wasm32") { None } else { std::thread::Builder::new().name("font-check".into()).spawn(look.clone()).ok() };
        if spawned.is_none() {
            look();
        }
        self.font_check = Some((rx, wake));
    }

    /// Rescan the fonts once [`Self::refresh_installed_fonts`]'s look says they changed.
    pub(super) fn poll_font_check(&mut self, ctx: &egui::Context) {
        let Some((check, wake)) = &self.font_check else { return };
        match check.try_recv() {
            Ok(changed) => {
                self.font_check = None;
                if changed {
                    // A failure shows in the status bar, as the menu item's does.
                    let _ = self.run("text.rescanFonts", json!({}));
                }
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                if *wake {
                    ctx.request_repaint_after(std::time::Duration::from_millis(50));
                }
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => self.font_check = None,
        }
    }
}
