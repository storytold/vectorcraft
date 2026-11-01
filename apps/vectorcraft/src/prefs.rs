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
    // What this window started from: only what changes after it is this window's to save.
    set_saved_base(prefs_value(app).ok());
}

/// Save what this window changed since it last read or saved the preferences, merged into the
/// file as it is now ([`merge_changes`]): another VectorCraft window open at the same time, which
/// read the file before this one saved, then keeps this window's changes when it saves or quits
/// (#4625). Nothing is written when nothing changed.
pub(crate) fn save_prefs(app: &VectorcraftApp) {
    if !prefs_enabled() || KEEP_UI_PREFS.load(std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let Some(p) = prefs_path() else { return };
    let Ok(mine) = prefs_value(app) else { return };
    let base = saved_base();
    if base.as_ref().is_some_and(|b| same_lasting(b, &mine)) {
        return;
    }
    // The file as another window may have saved it since (none, or unreadable: this one's own).
    let on_disk = std::fs::read(&p).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
    let merged = match (base.as_ref(), on_disk) {
        (Some(base), Some(on_disk)) => merge_changes(base, &mine, on_disk),
        _ => mine.clone(),
    };
    let _ = std::fs::create_dir_all(p.parent().unwrap_or(std::path::Path::new(".")));
    let Ok(bytes) = serde_json::to_vec_pretty(&merged) else { return };
    // Preferences are best effort: a failed write keeps the previous file, and the changes are
    // tried again on the next save.
    if fileio::write_atomic(&p, &bytes).is_ok() {
        set_saved_base(Some(mine));
    }
}

/// How often [`save_prefs_soon`] looks for changes to save.
const SAVE_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// Save the preferences at most every [`SAVE_EVERY`] while the app runs, when something changed:
/// the workspace, panels, toolbars and the other UI settings kept only in the UI state were saved
/// only on quitting, so a session ended any other way (the window closed by the system, the
/// process ended or crashing) started the next one from the settings before it (#4625).
pub(crate) fn save_prefs_soon(app: &VectorcraftApp) {
    static LAST: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    let Ok(mut last) = LAST.lock() else { return };
    if last.is_some_and(|t| t.elapsed() < SAVE_EVERY) {
        return;
    }
    *last = Some(std::time::Instant::now());
    drop(last);
    save_prefs(app);
}

/// The preferences as this window last read or saved them (`None`: not read yet).
static SAVED_BASE: std::sync::Mutex<Option<serde_json::Value>> = std::sync::Mutex::new(None);

fn saved_base() -> Option<serde_json::Value> {
    SAVED_BASE.lock().ok().and_then(|b| b.clone())
}

fn set_saved_base(value: Option<serde_json::Value>) {
    if let Ok(mut b) = SAVED_BASE.lock() {
        *b = value;
    }
}

/// Fields of the UI state that a start resets ([`vectorcraft_ui_egui::UiState::sanitized`]): a
/// change to them alone isn't saved, and they are never merged over another window's.
const PASSING: &[&str] = &["status", "dialog", "flyout", "palette_open", "palette_query", "about", "open_panel"];

/// Do `a` and `b` hold the same preferences, leaving out what a start resets ([`PASSING`])?
fn same_lasting(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    match (a.as_object(), b.as_object()) {
        (Some(a), Some(b)) => {
            let keys: std::collections::BTreeSet<&String> = a.keys().chain(b.keys()).filter(|k| !PASSING.contains(&k.as_str())).collect();
            keys.into_iter().all(|k| a.get(k) == b.get(k))
        }
        _ => a == b,
    }
}

/// `on_disk` with what changed from `base` to `mine` put over it: each top-level field of the UI
/// state, and each engine preference (`engine_prefs`) on its own, so two windows that change
/// different settings keep both. A field both changed takes this window's value (it saves last).
fn merge_changes(base: &serde_json::Value, mine: &serde_json::Value, on_disk: serde_json::Value) -> serde_json::Value {
    use serde_json::Value;
    let (Some(base), Some(mine), Value::Object(mut out)) = (base.as_object(), mine.as_object(), on_disk) else {
        return mine.clone();
    };
    let keys: std::collections::BTreeSet<&String> = base.keys().chain(mine.keys()).collect();
    for key in keys {
        if PASSING.contains(&key.as_str()) {
            if let Some(v) = mine.get(key) {
                out.entry(key.clone()).or_insert_with(|| v.clone());
            }
            continue;
        }
        let (was, now) = (base.get(key), mine.get(key));
        if was == now {
            continue;
        }
        if key == "engine_prefs"
            && let (Some(Value::Object(was)), Some(Value::Object(now)), Some(Value::Object(theirs))) = (was, now, out.get_mut(key))
        {
            let names: std::collections::BTreeSet<&String> = was.keys().chain(now.keys()).collect();
            for name in names {
                match (was.get(name), now.get(name)) {
                    (a, b) if a == b => {}
                    (_, Some(v)) => {
                        theirs.insert(name.clone(), v.clone());
                    }
                    (_, None) => {
                        theirs.remove(name);
                    }
                }
            }
            continue;
        }
        match now {
            Some(v) => {
                out.insert(key.clone(), v.clone());
            }
            None => {
                out.remove(key);
            }
        }
    }
    Value::Object(out)
}

/// The preferences file's contents: the UI state with the engine preferences.
#[cfg(test)]
pub(crate) fn prefs_bytes(app: &VectorcraftApp) -> serde_json::Result<Vec<u8>> {
    serde_json::to_vec_pretty(&prefs_value(app)?)
}

/// [`prefs_bytes`] as JSON.
fn prefs_value(app: &VectorcraftApp) -> serde_json::Result<serde_json::Value> {
    let mut ui = app.ui.clone();
    ui.engine_prefs = app.session.prefs.to_json();
    serde_json::to_value(&ui)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{merge_changes, same_lasting};

    /// Two windows open at once (#4625): the one that saves last keeps the other's changes and
    /// adds its own, instead of putting back the preferences it started from.
    #[test]
    fn a_window_saves_only_what_it_changed_over_another_windows_save() {
        let base = json!({"workspace": "Essentials", "toolbar": true, "engine_prefs": {"uiAccent": "blue", "uiScaling": 1.0}});
        // The other window switched the workspace and the accent, and saved.
        let on_disk = json!({"workspace": "Painting", "toolbar": true, "engine_prefs": {"uiAccent": "violet", "uiScaling": 1.0}});
        // This one changed only the UI scaling.
        let mine = json!({"workspace": "Essentials", "toolbar": true, "engine_prefs": {"uiAccent": "blue", "uiScaling": 1.25}});
        let merged = merge_changes(&base, &mine, on_disk);
        assert_eq!(merged, json!({"workspace": "Painting", "toolbar": true, "engine_prefs": {"uiAccent": "violet", "uiScaling": 1.25}}));
    }

    #[test]
    fn a_setting_both_windows_changed_takes_the_last_saved() {
        let base = json!({"workspace": "Essentials", "engine_prefs": {"uiAccent": "blue"}});
        let on_disk = json!({"workspace": "Painting", "engine_prefs": {"uiAccent": "violet"}});
        let mine = json!({"workspace": "Typography", "engine_prefs": {"uiAccent": "green"}});
        assert_eq!(merge_changes(&base, &mine, on_disk), json!({"workspace": "Typography", "engine_prefs": {"uiAccent": "green"}}));
    }

    #[test]
    fn fields_added_or_removed_by_this_window_are_saved() {
        let base = json!({"docking": {"root": 1}, "engine_prefs": {"old": 1}});
        let on_disk = json!({"docking": {"root": 1}, "recent_files": ["a.svg"], "engine_prefs": {"old": 1}});
        let mine = json!({"dock_width": 300.0, "engine_prefs": {"new": 2}});
        assert_eq!(merge_changes(&base, &mine, on_disk), json!({"dock_width": 300.0, "recent_files": ["a.svg"], "engine_prefs": {"new": 2}}));
    }

    /// The status line and open dialogs reset at start: they don't make a save, and they don't
    /// replace another window's.
    #[test]
    fn what_a_start_resets_is_left_out() {
        let base = json!({"status": "", "dialog": null, "workspace": "Essentials"});
        let mine = json!({"status": "Saved", "dialog": {"kind": "preferences"}, "workspace": "Essentials"});
        assert!(same_lasting(&base, &mine));
        let on_disk = json!({"status": "Opened", "dialog": null, "workspace": "Painting"});
        let mine = json!({"status": "Saved", "dialog": {"kind": "preferences"}, "workspace": "Essentials", "toolbar": false});
        let merged = merge_changes(&base, &mine, on_disk);
        assert_eq!(merged, json!({"status": "Opened", "dialog": null, "workspace": "Painting", "toolbar": false}));
        assert!(!same_lasting(&base, &mine));
    }

    #[test]
    fn a_file_that_is_not_an_object_is_replaced() {
        let base = json!({"workspace": "Essentials"});
        let mine = json!({"workspace": "Painting"});
        assert_eq!(merge_changes(&base, &mine, json!([1, 2])), mine);
    }
}
