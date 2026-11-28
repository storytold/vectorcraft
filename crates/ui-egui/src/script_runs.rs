//! A running script, advanced one slice per frame: `script.run` starts it and [`poll`] takes it to
//! its end, while the app goes on drawing the documents as they were before the run. The status bar
//! shows it with a Stop button; a run that has added a great deal, or that opened a dialog with
//! `alert`, `confirm`, `prompt`, a ScriptUI dialog's `show()` or a `File` or `Folder` file dialog,
//! waits for the person's answer.

use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::dialogs::script_run::{self, GrowthQuestion, ScriptFiles, ScriptNotice};
use crate::dialogs::script_window;

/// Advances the running script by one slice, or asks what its growth needs asked.
pub fn poll(app: &mut VectorcraftApp) {
    startup_failures(app);
    if !app.session.script_running() {
        return;
    }
    script_run::settle_close(app);
    if ask(app) {
        return;
    }
    // Between two queued runs, the next one waits for the answer to the quit question.
    if app.session.script_name().is_none() && (script_run::asking_to_close(app) || script_run::answer_close_between_runs(app)) {
        return;
    }
    let name = app.session.script_name().unwrap_or_default();
    let startup = app.session.startup_script_running();
    let Some(result) = app.session.script_pump() else {
        // A pump that started the next queued run shows the runs before it that could not start
        // in this frame: with none left running, the app may draw no next one.
        startup_failures(app);
        // The app draws no more frames while the run waits: its question opens in this one.
        ask(app);
        return;
    };
    script_run::forget::<GrowthQuestion>(app);
    script_run::settle_close(app);
    script_window::sync(app);
    match result {
        // The commands it ran are journaled now: a recording they no longer fit in names its limit.
        Ok(_) => {
            if !crate::panels::actions::after_journaled_edit(app) {
                app.status(format!("Ran {name}"));
            }
        }
        Err(e) => ended_with_error(app, startup, e.to_string()),
    }
    startup_failures(app);
    app.sync_views();
}

/// Shows each run `script.startup.run` queued that could not start ([`add_notes`]), and once the
/// round has no run left, the round's notes again ([`end_round`]).
fn startup_failures(app: &mut VectorcraftApp) {
    let failures: Vec<String> = app.session.take_startup_failures().into_iter().map(|(name, error)| format!("{name}: {error}")).collect();
    if app.session.script_running() {
        add_notes(app, failures);
    } else {
        app.ui.startup_notes.extend(failures);
        end_round(app);
    }
}

/// Adds `notes` to those of the startup scripts' round under way and shows them all
/// ([`show_notes`]); a round with no run left keeps none.
fn add_notes(app: &mut VectorcraftApp, notes: Vec<String>) {
    if notes.is_empty() {
        return;
    }
    app.ui.startup_notes.extend(notes);
    show_notes(app, app.ui.startup_notes.clone());
    if !app.session.script_running() {
        app.ui.startup_notes.clear();
    }
}

/// When the startup scripts' round has notes and its last run has ended: shows them again, before
/// the message that run ended with and its own notes, so a later message does not hide them.
fn end_round(app: &mut VectorcraftApp) {
    if app.ui.startup_notes.is_empty() {
        return;
    }
    let mut notes = std::mem::take(&mut app.ui.startup_notes);
    match app.ui.status_notes.take() {
        Some((status, more)) if status == app.ui.status && more == notes => {}
        Some((status, more)) if status == app.ui.status && !more.is_empty() => notes.extend(more),
        _ if app.ui.status.is_empty() || notes.contains(&app.ui.status) => {}
        _ => notes.push(app.ui.status.clone()),
    }
    show_notes(app, notes);
}

/// Shows the first of `notes` in the status bar, and keeps them all for its hover, which lists
/// them when there are several. Without notes the status bar keeps its message.
fn show_notes(app: &mut VectorcraftApp, notes: Vec<String>) {
    if let Some(first) = notes.first().cloned() {
        crate::io::show_with_notes(app, first, notes);
    }
}

/// Opens what the run waits at, once no other dialog is open, or over the script's ScriptUI dialog:
/// the script's own dialog, or the question about its growth. Closes the script's dialog when the
/// run no longer waits at it, and the ScriptUI dialog when its `show()` ends. → true while the run
/// waits.
fn ask(app: &mut VectorcraftApp) -> bool {
    script_window::sync(app);
    if let Some(picker) = app.session.script_picker() {
        if script_run::can_ask(app) {
            script_run::open_picker(app, picker);
        }
        return true;
    }
    script_run::forget::<ScriptFiles>(app);
    if let Some(dialog) = app.session.script_dialog() {
        if script_run::can_ask(app) {
            script_run::open_notice(app, &dialog);
        }
        return true;
    }
    // A run asked to stop no longer waits at its dialog.
    script_run::forget::<ScriptNotice>(app);
    // A run past its growth allowance asks about it first, also when its ScriptUI dialog waits
    // again.
    if app.session.script_waiting() {
        if script_run::can_ask(app) {
            script_run::open_growth(app);
        }
        return true;
    }
    app.session.script_window().is_some()
}

/// Runs the script of `params` (`script.run`'s) that the person chose from File › Scripts, after
/// the startup scripts when General › Run Startup Scripts is on (`script.startup.run`). A startup
/// script that cannot start, a listing of the Startup Scripts folder that was not read in full, and
/// a folder that is there and cannot be read show in the status bar ([`reply_notes`]).
/// → `script.run`'s reply, with `startup` holding `script.startup.run`'s when the startup scripts
/// were run: the chosen script's own reply when it ran to its end at once, `{running: true, name}`
/// when it is running, or `{running: true, queued: true}` while the chosen script waits to start.
/// A chosen script that could not start gives its error, as `script.run` does.
pub(crate) fn run_chosen(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    if app.session.startup_skip().is_some() {
        return app.run("script.run", params);
    }
    let mut startup = app.run("script.startup.run", json!({"reason": "menu", "then": params}))?;
    reply_notes(app, &startup);
    let mut reply = match startup.as_object_mut().and_then(|o| o.remove("then")) {
        Some(then) => then,
        // Nothing runs and the chosen script gave no reply: it could not start, and it is the
        // last of the runs that could not start.
        None if !app.session.script_running() => {
            let last = startup["failed"].as_array().and_then(|failed| failed.last());
            return Err(last.and_then(|failed| failed["error"].as_str()).unwrap_or_default().to_string());
        }
        None if app.session.startup_script_running() || app.session.script_name().is_none() => json!({"running": true, "queued": true}),
        None => json!({"running": true, "name": app.session.script_name().unwrap_or_default()}),
    };
    if let Some(o) = reply.as_object_mut() {
        o.insert("startup".to_string(), startup);
    }
    Ok(reply)
}

/// Starts the notes of a startup scripts' round with what its `script.startup.run` reply says was
/// left out ([`add_notes`]): each startup script it names under `failed` (the queued runs that
/// could not start before the first one did), then a listing that was not read in full
/// (`truncated`) or a folder that cannot be read (`unreadable`). The notes show again when the
/// round's last run ends ([`end_round`]).
pub(crate) fn reply_notes(app: &mut VectorcraftApp, reply: &Value) {
    app.ui.startup_notes.clear();
    let mut notes: Vec<String> = reply["failed"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|failed| format!("{}: {}", failed["name"].as_str().unwrap_or_default(), failed["error"].as_str().unwrap_or_default()))
        .collect();
    if reply["truncated"] == true {
        notes.push(STARTUP_TRUNCATED.to_string());
    }
    if reply["unreadable"] == true {
        notes.push(STARTUP_UNREADABLE.to_string());
    }
    add_notes(app, notes);
}

/// The status bar's note when a listing of the Startup Scripts folder stopped at a limit, left out
/// scripts past its limit or skipped an entry it could not read.
pub(crate) const STARTUP_TRUNCATED: &str = "The Startup Scripts folder was not read in full";
/// The status bar's note when the Startup Scripts folder is there and cannot be read.
pub(crate) const STARTUP_UNREADABLE: &str = "The Startup Scripts folder cannot be read";

/// When the app has started, out of Safe Mode: the startup scripts run when General › Run Startup
/// Scripts is on (`script.startup.run`). A failure to start them, a listing of the folder that was
/// not read in full, or a Startup Scripts folder that is there and cannot be read shows in the
/// status bar ([`reply_notes`]).
pub fn run_at_launch(app: &mut VectorcraftApp) {
    if app.session.startup_skip().is_some() {
        return;
    }
    match app.run("script.startup.run", json!({"reason": "launch"})) {
        Ok(reply) => reply_notes(app, &reply),
        Err(e) => app.status(e),
    }
}

/// Shows why a run ended without its changes ([`failed`]). When the run was a startup script
/// (`startup`), the message also stays in the round's notes, so the message of the run that ends
/// the round does not hide it ([`end_round`]).
pub(crate) fn ended_with_error(app: &mut VectorcraftApp, startup: bool, error: String) {
    failed(app, error);
    if startup {
        app.ui.startup_notes.push(app.ui.status.clone());
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

/// Shows that the runs `script.startup.run` queued were stopped between two of them, before the
/// next one started. The message shows first, before the round's notes ([`end_round`]).
pub(crate) fn stopped_between_runs(app: &mut VectorcraftApp) {
    let message = "The scripts waiting to run were not run";
    let mut notes = vec![message.to_string()];
    notes.append(&mut app.ui.startup_notes);
    if notes.len() == 1 {
        // Notes an earlier round left for the same message are not this round's.
        app.ui.status_notes = None;
        app.status(message);
    } else {
        show_notes(app, notes);
    }
}
