//! A running script, advanced one slice per frame: `script.run` starts it and [`poll`] takes it to
//! its end, while the app goes on drawing the documents as they were before the run. The status bar
//! shows it with a Stop button; a run that has added a great deal, or that opened a dialog with
//! `alert`, `confirm`, `prompt` or a `File` or `Folder` file dialog, waits for the person's answer.

use vectorcraft_engine::WindowView;

use crate::VectorcraftApp;
use crate::dialogs::script_run::{self, GrowthQuestion, ScriptFiles, ScriptNotice};

/// Advances the running script by one slice, or asks what its growth needs asked.
pub fn poll(app: &mut VectorcraftApp) {
    if !app.session.script_running() {
        return;
    }
    script_run::settle_close(app);
    if ask(app) {
        return;
    }
    let name = app.session.script_name().unwrap_or_default();
    let shared = share_views(app);
    let pumped = app.session.script_pump();
    take_views(app, &shared);
    let Some(result) = pumped else {
        // The app draws no more frames while the run waits: its question opens in this one.
        ask(app);
        return;
    };
    script_run::forget::<GrowthQuestion>(app);
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

/// The running script reads each drawn window in `Document.views`, with its last drawn area.
/// The returned list contains those windows for [`take_views`].
pub(crate) fn share_views(app: &mut VectorcraftApp) -> Vec<((u64, u64), WindowView)> {
    // The view's zoom counts screen points before UI scaling; its area is in the interface's points.
    let scaling = f64::from(app.ui_scaling());
    let mode = app.ui.screen_mode;
    let shared: Vec<((u64, u64), WindowView)> = crate::doc_windows::windows(app)
        .into_iter()
        .filter_map(|window| {
            let uid = app.session.documents().get(window.0)?.uid;
            let v = crate::doc_windows::view(app, window)?;
            if !v.fitted || v.rect.width() <= 0.0 || v.rect.height() <= 0.0 {
                return None;
            }
            let size = [f64::from(v.rect.width()) * scaling, f64::from(v.rect.height()) * scaling];
            Some(((uid, v.runtime_id), WindowView { center: v.center, zoom: v.zoom, rotation: v.rotation, size, screen_mode: mode }))
        })
        .collect();
    app.session.window_views.clone_from(&shared);
    shared
}

/// Takes the zoom, centre and screen mode a script gave the views [`share_views`] gave.
pub(crate) fn take_views(app: &mut VectorcraftApp, shared: &[((u64, u64), WindowView)]) {
    let (min, max) = WindowView::ZOOM_RANGE;
    for (key, before) in shared {
        let Some(now) = app.session.window_views.iter().find(|(k, _)| k == key).map(|(_, v)| *v) else { continue };
        if now == *before {
            continue;
        }
        if let Some(v) = crate::doc_windows::view_by_id_mut(app, *key) {
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
        if !script_run::busy(app) {
            script_run::open_picker(app, picker);
        }
        return true;
    }
    script_run::forget::<ScriptFiles>(app);
    if let Some(dialog) = app.session.script_dialog() {
        if !script_run::busy(app) {
            script_run::open_notice(app, &dialog);
        }
        return true;
    }
    // A run asked to stop no longer waits at its dialog.
    script_run::forget::<ScriptNotice>(app);
    if app.session.script_waiting() {
        if !script_run::busy(app) {
            script_run::open_growth(app);
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
