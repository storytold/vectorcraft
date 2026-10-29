use super::{VectorcraftApp, fileio, prefs_dir};

/// Where UI preferences live: ~/Library/Application Support/VectorCraft (macOS),
/// %APPDATA%\VectorCraft (Windows), $XDG_CONFIG_HOME or ~/.config/vectorcraft (Linux).
pub(crate) fn prefs_path() -> Option<std::path::PathBuf> {
    prefs_dir::prefs_path_for("VectorCraft", "vectorcraft")
}

/// The same place under the project's former name (DrawCraft): read once if there are no
/// VectorCraft preferences yet, so settings survive the rename.
fn legacy_prefs_path() -> Option<std::path::PathBuf> {
    prefs_dir::prefs_path_for("DrawCraft", "drawcraft")
}

/// Where the log files live: `logs` in the preferences folder (see `logging`).
pub(crate) fn log_dir() -> Option<std::path::PathBuf> {
    Some(prefs_path()?.parent()?.join("logs"))
}

/// Runs without preferences (`VECTORCRAFT_NO_PREFS`, agents' test runs) neither read nor write them.
pub(crate) fn prefs_enabled() -> bool {
    std::env::var_os("VECTORCRAFT_NO_PREFS").is_none()
}

/// The saved UI preferences, read before the window opens (they hold its size and position).
/// A file read only in part, or with layouts converted from an older version, is copied beside
/// itself before anything saves over it.
pub(crate) fn read_prefs() -> Option<vectorcraft_ui_egui::UiState> {
    if !prefs_enabled() {
        return None;
    }
    if let Some(p) = prefs_path()
        && let Ok(bytes) = std::fs::read(&p)
    {
        let read = vectorcraft_ui_egui::UiState::read_preferences(&bytes);
        if read.as_ref().is_none_or(|read| !read.intact) {
            keep_prefs_copy(&p, &bytes, "ui.unreadable");
        }
        if read.as_ref().is_some_and(|read| read.converted) {
            keep_prefs_copy(&p, &bytes, "ui.before-layout-migration");
        }
        return read.map(|read| read.ui);
    }
    let bytes = legacy_prefs_path().and_then(|p| std::fs::read(p).ok())?;
    decode_saved_prefs(&bytes)
}

/// Saved preferences, with what this version cannot read of their docking data dropped and
/// older layouts converted (`vectorcraft_ui_egui::layout_migration`).
pub(crate) fn decode_saved_prefs(bytes: &[u8]) -> Option<vectorcraft_ui_egui::UiState> {
    vectorcraft_ui_egui::UiState::read_preferences(bytes).map(|read| read.ui)
}

/// Set when `ui.json` needed a copy kept ([`keep_prefs_copy`]) and none could be: the file is
/// then left as it is rather than overwritten.
pub(crate) static KEEP_UI_PREFS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Copy `ui.json` beside it as `<stem>.json` or the first free `<stem>-N.json`, before the next
/// save replaces it: `ui.unreadable` when it could not be read in full (a newer VectorCraft may
/// have written it), `ui.before-layout-migration` when its layouts were converted.
pub(crate) fn keep_prefs_copy(path: &std::path::Path, bytes: &[u8], stem: &str) {
    use std::io::Write as _;
    for n in 1..100u32 {
        let name = if n == 1 { format!("{stem}.json") } else { format!("{stem}-{n}.json") };
        match std::fs::OpenOptions::new().write(true).create_new(true).open(path.with_file_name(name)) {
            Ok(mut file) => {
                if file.write_all(bytes).is_ok() {
                    return;
                }
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => break,
        }
    }
    KEEP_UI_PREFS.store(true, std::sync::atomic::Ordering::Relaxed);
}

pub(crate) fn load_prefs(app: &mut VectorcraftApp, saved: Option<vectorcraft_ui_egui::UiState>) {
    if !prefs_enabled() {
        return;
    }
    if let Some(ui) = saved {
        app.ui = ui.sanitized();
    }
    vectorcraft_ui_egui::prefs_dialog::restore(app);
}

pub(crate) fn save_prefs(app: &VectorcraftApp) {
    if !prefs_enabled() || KEEP_UI_PREFS.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    if let Some(p) = prefs_path() {
        let _ = std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new(".")));
        if let Ok(bytes) = prefs_bytes(app) {
            // Preferences are best effort: a failed write keeps the previous file.
            let _ = fileio::write_atomic(&p, &bytes);
        }
    }
}

/// The preferences file's contents: the UI state with the engine preferences.
pub(crate) fn prefs_bytes(app: &VectorcraftApp) -> serde_json::Result<Vec<u8>> {
    let mut ui = app.ui.clone();
    ui.engine_prefs = app.session.prefs.to_json();
    serde_json::to_vec_pretty(&ui)
}
