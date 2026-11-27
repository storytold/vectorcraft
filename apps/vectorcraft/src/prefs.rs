//! The preferences file: opened before the window, sampled while the app runs and saved on exit.

use std::path::PathBuf;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use craft::settings::{LegacyFile, LoadReport, Location, OpenOptions, SettingsStore, SharedSettings};
use vectorcraft_ui_egui::{AppSettings, from_app, parse_legacy};

use super::{VectorcraftApp, prefs_dir};

/// The file the settings were kept in before `config.toml`.
const LEGACY_FILE: &str = "ui.json";

/// The DrawCraft folder (the project's former name), read once if there are no VectorCraft
/// preferences yet, so settings survive the rename.
fn legacy_dir() -> Option<PathBuf> {
    prefs_dir::profile::prefs_dir(true)
}

/// Where the log files live: `logs` in the preferences folder (see `logging`).
pub(crate) fn log_dir() -> Option<PathBuf> {
    Some(prefs_dir::prefs_dir()?.join("logs"))
}

/// Runs without preferences (`VECTORCRAFT_NO_PREFS`, agents' test runs) neither read nor write them.
pub(crate) fn prefs_enabled() -> bool {
    std::env::var_os("VECTORCRAFT_NO_PREFS").is_none()
}

/// The saved preferences, opened before the window (they hold its size and position). The folder is
/// `profile.rs`'s; the old `ui.json` files are read once when there is no `config.toml` yet. A run
/// without preferences keeps them in memory.
pub(crate) fn open_store() -> (SharedSettings<AppSettings>, LoadReport) {
    let (location, legacy) = match prefs_dir::prefs_dir().filter(|_| prefs_enabled()) {
        Some(dir) => {
            let file = |dir: PathBuf| LegacyFile { path: dir.join(LEGACY_FILE), parse: parse_legacy };
            let legacy = std::iter::once(file(dir.clone())).chain(legacy_dir().map(file)).collect();
            (Location::Dir(dir), legacy)
        }
        None => (Location::Memory, Vec::new()),
    };
    let (store, report) = SettingsStore::open_with(OpenOptions::new(location).legacy(legacy));
    (SharedSettings::new(store), report)
}

pub(crate) fn load_prefs(app: &mut VectorcraftApp, store: &SharedSettings<AppSettings>) {
    if !prefs_enabled() {
        return;
    }
    let ui = store.lock().get().ui.clone();
    app.ui = ui.sanitized();
    vectorcraft_ui_egui::prefs_dialog::restore(app);
}

/// How often [`save_tick`] samples the UI state for changes.
const SAVE_EVERY: Duration = Duration::from_secs(2);

/// Once per frame: hand the store the current settings when a preference was confirmed (#1747) or
/// every [`SAVE_EVERY`] (the workspace, panels, toolbars and the other UI settings change without
/// the dialog, #4625), then let the store write what it holds. The store decides when to write and
/// merges another window's changes; a sample equal to what it holds saves nothing.
pub(crate) fn save_tick(app: &mut VectorcraftApp, store: &SharedSettings<AppSettings>, now: Instant) {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let confirmed = app.take_prefs_changed();
    let sample = {
        let mut last = LAST.lock().unwrap_or_else(PoisonError::into_inner);
        let sample = confirmed || last.is_none_or(|t| now.duration_since(t) >= SAVE_EVERY);
        if sample {
            *last = Some(now);
        }
        sample
    };
    let mut store = store.lock();
    if sample && store.is_persistent() {
        store.set(from_app(app));
    }
    store.tick(now);
}

/// On exit: save the app's last state, with what changed since the last sample.
pub(crate) fn flush(app: &VectorcraftApp, store: &SharedSettings<AppSettings>) {
    let mut store = store.lock();
    if !store.is_persistent() {
        return;
    }
    store.set(from_app(app));
    // Preferences are best effort: a failed write is logged, never fatal at exit.
    if let Err(e) = store.flush() {
        log::warn!("the preferences were not saved: {e}");
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// One VectorCraft window: its store over the folder and its app, as loaded from it.
    struct Window {
        store: SettingsStore<AppSettings>,
        app: VectorcraftApp,
    }

    impl Window {
        fn open(dir: &Path) -> Self {
            let (store, _) = SettingsStore::<AppSettings>::open_at(dir);
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
            app.ui = store.get().ui.clone();
            Window { store, app }
        }

        fn save(&mut self) {
            self.store.set(from_app(&self.app));
            self.store.flush().unwrap();
        }

        fn set_sets(&mut self, names: &[&str]) {
            self.app.ui.actions = craft::automation::actions::ActionList::new();
            for name in names {
                self.app.ui.actions.new_set(name).unwrap();
            }
        }
    }

    fn temp() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("vc-prefs-").tempdir().unwrap()
    }

    fn saved(dir: &Path) -> AppSettings {
        SettingsStore::<AppSettings>::open_at(dir).0.get().clone()
    }

    fn set_names(settings: &AppSettings) -> Vec<String> {
        settings.ui.actions.sets().into_iter().map(str::to_string).collect()
    }

    /// The status line and open palette reset at start: a window that has them up neither saves
    /// them nor writes over another window's setting it did not change.
    #[test]
    fn what_a_start_resets_is_left_out() {
        let dir = temp();
        Window::open(dir.path()).save();
        let (mut first, mut second) = (Window::open(dir.path()), Window::open(dir.path()));
        second.store.update(|s| s.ui.status_bar = false);
        second.store.flush().unwrap();
        first.app.ui.status = "Saved".to_string();
        first.app.ui.palette_query = "find".to_string();
        first.save();
        let back = saved(dir.path());
        assert!(!back.ui.status_bar, "the other window's setting survives");
        assert!(back.ui.status.is_empty() && back.ui.palette_query.is_empty(), "nothing a start resets is written");
    }

    /// The current Actions set is saved by name: another window inserting a set before it does not
    /// make a different set current.
    #[test]
    fn the_current_action_set_follows_its_set_in_the_merged_list() {
        let dir = temp();
        let mut base = Window::open(dir.path());
        base.set_sets(&["A", "B", "C"]);
        base.save();
        let (mut chooser, mut inserter) = (Window::open(dir.path()), Window::open(dir.path()));
        chooser.app.ui.action_set = Some("C".to_string());
        chooser.save();
        inserter.set_sets(&["Z", "A", "B", "C"]);
        inserter.save();
        let back = saved(dir.path());
        assert_eq!(back.ui.action_set.as_deref(), Some("C"));
        assert_eq!(set_names(&back), ["Z", "A", "B", "C"]);
    }

    /// The window that did not choose keeps the other's choice, still by name after it deleted the
    /// set before it.
    #[test]
    fn the_other_windows_current_action_set_follows_its_set() {
        let dir = temp();
        let mut base = Window::open(dir.path());
        base.set_sets(&["A", "B", "C"]);
        base.save();
        let (mut chooser, mut deleter) = (Window::open(dir.path()), Window::open(dir.path()));
        deleter.set_sets(&["B", "C"]);
        deleter.save();
        chooser.app.ui.action_set = Some("C".to_string());
        chooser.save();
        let back = saved(dir.path());
        assert_eq!(back.ui.action_set.as_deref(), Some("C"));
        assert_eq!(set_names(&back), ["B", "C"]);
    }
}
