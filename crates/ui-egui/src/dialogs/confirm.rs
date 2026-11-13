//! A question answered with OK or Cancel; OK runs a command ([`ask`]). Used for confirmations such
//! as deleting swatches.
//!
//! Fields: `message` (the question, shown as the heading), `detail` (an optional line under it),
//! `__command` and `__params` (the command OK runs). Both texts are shown as given: callers
//! translate their template before filling in a name, so the name is never translated.

use serde_json::{Value, json};

use super::DialogSpec;
use crate::VectorcraftApp;
use crate::state::Dialog;
use crate::theme::Tokens;

/// The dialog kind of a confirmation.
pub const KIND: &str = "confirm";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |d| d.str("message"),
    body: |_, ui, d| {
        let detail = d.str("detail");
        if !detail.is_empty() {
            ui.label(egui::RichText::new(detail).color(Tokens::get(ui.ctx()).text_dim));
        }
        false
    },
    confirm,
    max_width: Some(420.0),
    ..DialogSpec::FORM
};

/// Ask `message` (with an optional `detail` line), both in the UI language; OK runs `command`
/// with `params`.
pub fn ask(app: &mut VectorcraftApp, message: &str, detail: &str, command: &str, params: Value) {
    let fields = json!({"message": message, "detail": detail, "__command": command, "__params": params});
    app.ui.dialog = Some(Dialog::new(KIND, fields));
}

/// The dialog kind of a message with a Close button only ([`tell`]).
pub const MESSAGE: &str = "message";

pub(super) const MESSAGE_SPEC: DialogSpec = DialogSpec { ok: None, ..SPEC };

/// The dialog kind of the question whether to replace a file that a save panel's name names once
/// the app gave it an extension ([`ask_replace`]). Fields: `message`, `detail`, `path` (the file).
pub const REPLACE: &str = "replaceFile";

pub(super) const REPLACE_SPEC: DialogSpec = DialogSpec { confirm: |app, d| crate::picks::replace(app, &d.str("path")), ..SPEC };

/// Ask whether to replace the file at `path`, which the name `typed` in a save panel became when
/// the app gave it an extension, over the dialog open now (Cancel shows it again). OK runs
/// [`crate::picks::replace`]; the Enter that closed the save panel does not press it.
pub(crate) fn ask_replace(app: &mut VectorcraftApp, path: &str, typed: &str) {
    let name = vectorcraft_engine::cmd::fileio::file_name(path);
    let typed = vectorcraft_engine::cmd::fileio::file_name(typed);
    let message = crate::i18n::fmt(tl!("“{name}” already exists. Replace it?"), &[("name", &name)]);
    let detail = crate::i18n::fmt(
        tl!("The name in the save panel was “{typed}”; VectorCraft added the file type's extension. Replacing the file overwrites its contents."),
        &[("typed", &typed)],
    );
    let under = app.ui.dialog.take().and_then(|d| serde_json::to_value(d).ok());
    let fields = json!({"message": message, "detail": detail, "path": path, super::PARENT: under, super::AFTER_PANEL: true});
    app.ui.dialog = Some(Dialog::new(REPLACE, fields));
}

/// The dialog kind of the question whether a save or export may replace the files it writes beside
/// the one picked in the save panel ([`ask_replace_files`]). Fields: `message`, `detail`, `files`
/// (their paths), `allowed` (files an earlier OK allowed), `doc` (the document's uid), `__command`
/// and `__params`.
pub const REPLACE_FILES: &str = "replaceFiles";

pub(super) const REPLACE_FILES_SPEC: DialogSpec = DialogSpec { confirm: replace_files, ..SPEC };

/// Ask whether to replace `files`, files that are there beside the one a save or export of the
/// document `doc` (its uid) writes, which the save panel did not ask about (a file per artboard, a
/// linked image). OK runs `command` with `params` again on that document, now replacing those
/// files and `allowed` (those an earlier OK allowed); Cancel writes nothing.
pub(crate) fn ask_replace_files(app: &mut VectorcraftApp, doc: u64, files: &[String], allowed: &[String], command: &str, params: Value) {
    let name = files.first().map(|f| vectorcraft_engine::cmd::fileio::file_name(f)).unwrap_or_default();
    let message = crate::i18n::fmt(tl!("“{name}” already exists. Replace it?"), &[("name", &name)]);
    // Several: every one of them, by name.
    let detail = match files {
        [_, _, ..] => files.iter().map(|f| vectorcraft_engine::cmd::fileio::file_name(f)).collect::<Vec<_>>().join("\n"),
        _ => String::new(),
    };
    let fields =
        json!({"message": message, "detail": detail, "files": files, "allowed": allowed, "doc": doc, "__command": command, "__params": params});
    let d = Dialog::new(REPLACE_FILES, fields);
    // Asked when a Background Save or Export ends: a dialog open then stays, and this waits its turn
    // (however many messages wait: the save or export needs the answer).
    match app.ui.dialog {
        None => app.ui.dialog = Some(d),
        Some(_) => app.ui.told.push(d),
    }
}

/// OK on [`ask_replace_files`]: what asked runs again on its document, and the files the question
/// named are replaced; one it writes now that was not named is asked about again. The save or export
/// reads the allowance when it starts, before a background one hands its work to the worker.
fn replace_files(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    if !crate::picks::on_document(app, d.fields.get("doc").and_then(Value::as_u64)) {
        return Ok(Value::Null);
    }
    let paths = |k: &str| d.fields.get(k).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string);
    app.picks.replace_files_ok = paths("allowed").chain(paths("files")).collect();
    let r = confirm(app, d);
    app.picks.replace_files_ok.clear();
    r
}

/// The dialog kind of the question Reset Preferences asks over the Preferences dialog
/// ([`ask_reset_prefs`]). Fields: `message`, `detail`.
pub const RESET_PREFS: &str = "resetPreferences";

pub(super) const RESET_PREFS_SPEC: DialogSpec = DialogSpec { confirm: reset_prefs, ..SPEC };

/// Ask whether to reset the preferences, over the Preferences dialog open now. Cancel and Escape
/// show Preferences again as it was; OK shows it again with every preference at its default value
/// ([`crate::prefs_dialog::reset_fields`]). Nothing is applied until OK in Preferences.
pub(crate) fn ask_reset_prefs(app: &mut VectorcraftApp) {
    let under = app.ui.dialog.take().and_then(|d| serde_json::to_value(d).ok());
    let message = tl!("Reset all preferences to their defaults?");
    let detail = tl!("The defaults are applied when you click OK in Preferences. Cancel there keeps your current preferences.");
    let fields = json!({"message": message, "detail": detail, super::PARENT: under});
    app.ui.dialog = Some(Dialog::new(RESET_PREFS, fields));
}

/// OK on [`ask_reset_prefs`]: Preferences again, its fields at their defaults.
fn reset_prefs(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut prefs = super::parent(d).filter(|p| p.kind == "preferences").ok_or("the dialog it was opened over is no longer open")?;
    crate::prefs_dialog::reset_fields(&mut prefs);
    app.ui.dialog = Some(prefs);
    Ok(Value::Null)
}

/// Most messages kept for later while a dialog is open ([`tell`]).
const MAX_WAITING: usize = 16;

/// Tell the user `message` (with an optional `detail` line), both in the UI language, in a dialog
/// they close. While another dialog is open (the message comes from work in the background) it
/// waits its turn instead of replacing it: neither what the user was doing there nor the message
/// is lost.
pub fn tell(app: &mut VectorcraftApp, message: &str, detail: &str) {
    let d = Dialog::new(MESSAGE, json!({"message": message, "detail": detail}));
    if app.ui.dialog.is_none() {
        app.ui.dialog = Some(d);
    } else if app.ui.told.len() < MAX_WAITING && !app.ui.dialog.iter().chain(&app.ui.told).any(|t| t.kind == MESSAGE && t.fields == d.fields) {
        app.ui.told.push(d);
    }
}

/// Show the next message that waited for a dialog to close (each frame).
pub(super) fn show_waiting(app: &mut VectorcraftApp) {
    if app.ui.dialog.is_none() && !app.ui.told.is_empty() {
        app.ui.dialog = Some(app.ui.told.remove(0));
    }
}

/// Closes before running, so a dialog the command opens stays open.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let params = d.fields.get("__params").cloned().unwrap_or_else(|| json!({}));
    app.ui.dialog = None;
    app.run(&d.str("__command"), params)
}
