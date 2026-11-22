//! A running script, advanced one slice per frame: `script.run` starts it and [`poll`] takes it to
//! its end, while the app goes on drawing the documents as they were before the run. The status bar
//! shows it with a Stop button; a run that has added a great deal, or that opened a dialog with
//! `alert`, `confirm`, `prompt` or a `File` or `Folder` file dialog, waits for the person's answer.

use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::dialogs::script_run;
use crate::state::Dialog;

/// Advances the running script by one slice, or asks what its growth needs asked.
pub fn poll(app: &mut VectorcraftApp) {
    script_run::drain(app);
    // Each run `script.startup.run` queued that could not start.
    for (name, error) in app.session.take_startup_failures() {
        app.status(format!("{name}: {error}"));
    }
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
    if let Some(picker) = app.session.script_picker() {
        if app.ui.dialog.is_none() {
            script_run::open_picker(app, &picker);
        }
        return true;
    }
    if app.ui.dialog.as_ref().is_some_and(|d| d.kind == script_run::PICKER_KIND) {
        app.ui.dialog = None;
    }
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

/// Runs the script of `params` (`script.run`'s) that the person chose from File › Scripts, after
/// the startup scripts when General › Run Startup Scripts is on (`script.startup.run`).
pub(crate) fn run_chosen(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    if app.session.startup_skip().is_some() {
        return app.run("script.run", params);
    }
    app.run("script.startup.run", json!({"reason": "menu", "then": params}))
}

/// When the app has started, out of Safe Mode: the startup scripts run when General › Run Startup
/// Scripts is on (`script.startup.run`). A failure to start them shows in the status bar.
pub fn run_at_launch(app: &mut VectorcraftApp) {
    if app.session.startup_skip().is_some() {
        return;
    }
    if let Err(e) = app.run("script.startup.run", json!({"reason": "launch"})) {
        app.status(e);
    }
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
