//! A running script, advanced one slice per frame: `script.run` starts it and [`poll`] takes it to
//! its end, while the app goes on drawing the documents as they were before the run. The status bar
//! shows it with a Stop button; a run that has added a great deal, or that opened a dialog with
//! `alert`, `confirm` or `prompt`, waits for the person's answer.

use serde_json::json;

use crate::VectorcraftApp;
use crate::dialogs::script_run;
use crate::state::Dialog;

/// Advances the running script by one slice, or asks what its growth needs asked.
pub fn poll(app: &mut VectorcraftApp) {
    script_run::drain(app);
    if !app.session.script_running() {
        return;
    }
    script_run::settle_close(app);
    if ask(app) {
        return;
    }
    let name = app.session.script_name().unwrap_or_default();
    let cursor_settings = (app.session.caps_lock, app.session.prefs.use_precise_cursors);
    let Some(result) = app.session.script_pump() else {
        // The app draws no more frames while the run waits: its question opens in this one.
        ask(app);
        return;
    };
    if cursor_settings != (app.session.caps_lock, app.session.prefs.use_precise_cursors) {
        app.canvas.hover_cursor = None;
    }
    if app.ui.dialog.as_ref().is_some_and(|d| d.kind == script_run::GROWTH_KIND) {
        app.ui.dialog = None;
    }
    script_run::settle_close(app);
    match result {
        // The commands it ran are journaled now: a recording they no longer fit in names its limit.
        Ok(_) => {
            if !crate::panels::actions::after_journaled_edit(app) {
                app.status(format!("Ran {name}"));
            }
        }
        Err(e) => failed(app, e.to_string()),
    }
    app.sync_views();
}

/// Opens what the run waits at, once no other dialog is open: the script's own dialog, or the
/// question about its growth. Closes the script's dialog when the run no longer waits at it.
/// → true while the run waits.
fn ask(app: &mut VectorcraftApp) -> bool {
    if let Some(dialog) = app.session.script_dialog() {
        if app.ui.dialog.is_none() {
            script_run::open_notice(app, &dialog);
        }
        return true;
    }
    // A run asked to stop no longer waits at its dialog.
    if app.ui.dialog.as_ref().is_some_and(|d| d.kind == script_run::NOTICE_KIND) {
        app.ui.dialog = None;
    }
    if app.session.script_waiting() {
        if app.ui.dialog.is_none() {
            app.ui.dialog = Some(Dialog::new(script_run::GROWTH_KIND, json!({})));
        }
        return true;
    }
    false
}

/// Shows why a run ended without its changes. A stopped run that wrote no file says only that it
/// was stopped; any other failure shows its error, which names the files left written.
pub(crate) fn failed(app: &mut VectorcraftApp, error: String) {
    let stopped = app.session.script_report().is_some_and(|r| r["kind"] == "stopped" && r["written"].as_array().is_none_or(Vec::is_empty));
    if stopped {
        app.status("The script was stopped and its changes were undone");
    } else {
        app.status(error);
    }
}
