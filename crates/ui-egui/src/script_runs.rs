//! A running script, advanced one slice per frame: `script.run` starts it and [`poll`] takes it to
//! its end, while the app goes on drawing the documents as they were before the run. The status bar
//! shows it with a Stop button; a run that has added a great deal, or that opened a dialog with
//! `alert`, `confirm`, `prompt` or a `File` or `Folder` file dialog, waits for the person's answer.

use serde_json::json;
use vectorcraft_engine::WindowView;

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
    let shared = share_views(app);
    let pumped = app.session.script_pump();
    take_views(app, &shared);
    let Some(result) = pumped else {
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

/// Gives each document whose window has the canvas's size its view, which a running script reads
/// as `Document.views`: every document's while the documents are under the tabs, and only the
/// current document's, whose window holds the canvas, while they are tiled or a document floats in
/// a window of its own. A document without one gets none. → the views given, for [`take_views`].
pub(crate) fn share_views(app: &mut VectorcraftApp) -> Vec<(u64, WindowView)> {
    let ui_zoom = f64::from(app.canvas_ui_zoom);
    let size = app.canvas_rect.map(|r| [f64::from(r.width()) * ui_zoom, f64::from(r.height()) * ui_zoom]);
    let mode = app.ui.screen_mode;
    let tabbed = app.canvas.panes.last.is_empty() && app.ui.floating.is_empty();
    let current = app.session.active().map(|d| d.uid);
    let shared: Vec<(u64, WindowView)> = match size {
        Some(size) => app
            .view_uids
            .iter()
            .zip(&app.views)
            .filter(|(uid, v)| v.fitted && (tabbed || current == Some(**uid)))
            .map(|(uid, v)| (*uid, WindowView { center: v.center, zoom: v.zoom, rotation: v.rotation, size, screen_mode: mode }))
            .collect(),
        None => vec![],
    };
    app.session.window_views.clone_from(&shared);
    shared
}

/// Takes the zoom, centre and screen mode a script gave the views [`share_views`] gave.
pub(crate) fn take_views(app: &mut VectorcraftApp, shared: &[(u64, WindowView)]) {
    let (min, max) = WindowView::ZOOM_RANGE;
    for (uid, before) in shared {
        let Some(now) = app.session.window_views.iter().find(|(u, _)| u == uid).map(|(_, v)| *v) else { continue };
        if now == *before {
            continue;
        }
        if let Some(v) = app.view_uids.iter().position(|u| u == uid).and_then(|k| app.views.get_mut(k)) {
            v.zoom = now.zoom.clamp(min, max);
            v.center = now.center;
        }
    }
    // The screen mode is the app's: a script that sets it on any view sets it on every view.
    if let (Some((_, before)), Some((_, now))) = (shared.first(), app.session.window_views.first())
        && now.screen_mode != before.screen_mode
    {
        app.ui.screen_mode = now.screen_mode.min(3);
    }
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
