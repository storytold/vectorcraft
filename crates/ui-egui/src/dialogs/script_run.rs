//! File › Scripts › Other Script…: choosing a JavaScript file and the warning before it runs
//! (`script.run` runs it), the question a run that keeps adding art asks, and the question quitting
//! asks while a script runs.
//!
//! `ui.runScript {path? | source?, name?}` shows the warning for that script; without either it
//! asks for a `.js` or `.jsx` file (the web opens the browser's file picker). With Preferences ›
//! General › Show External Script Warning off, the chosen script runs at once. Warning fields:
//! `name`, `script` (`script.run`'s params) and `dontShow` (true: turn the warning off).

use serde_json::{Value, json};
use vectorcraft_engine::cmd::scripts::{RUN_EXTS, TOO_LARGE, runs};

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{FilePick, VectorcraftApp, widgets};

/// The dialog kind of the warning before a script runs.
pub const KIND: &str = "scriptRun";
/// The dialog kind of the question a run asks when it has added a great deal.
pub const GROWTH_KIND: &str = "scriptGrowth";
/// The dialog kind of the question quitting asks while a script runs.
pub const CLOSE_KIND: &str = "scriptClose";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Run Script").into(),
    body,
    confirm,
    ok: Some("Run"),
    min_width: 420.0,
    max_width: Some(520.0),
    ..DialogSpec::FORM
};

pub(super) const GROWTH_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("A Script Is Running").into(),
    body: |_, ui, _| {
        ui.label(tl!("This script has added a great many objects. Continue, or stop it and undo what it did?"));
        false
    },
    confirm: growth_confirm,
    ok: Some("Continue"),
    cancel: Some("Stop"),
    min_width: 420.0,
    max_width: Some(520.0),
    ..DialogSpec::FORM
};

pub(super) const CLOSE_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("A Script Is Running").into(),
    body: |_, ui, _| {
        ui.label(tl!("Stop the script and undo what it did? Then quit again."));
        false
    },
    confirm: close_confirm,
    ok: Some("Stop"),
    min_width: 420.0,
    max_width: Some(520.0),
    ..DialogSpec::FORM
};

/// A file the web's picker delivered: its name and bytes, or `None` for a file over
/// [`vectorcraft_engine::script::MAX_SOURCE_BYTES`], which is not read.
pub type ScriptInbox = std::sync::Arc<std::sync::Mutex<Vec<(String, Option<Vec<u8>>)>>>;

/// Shows the warning for the next script the web's picker delivered, once no dialog is open. A
/// picker that was canceled delivers nothing.
pub(crate) fn drain(app: &mut VectorcraftApp) {
    if app.ui.dialog.is_some() {
        return;
    }
    let Some((name, bytes)) = app.services.script_inbox.as_ref().and_then(|q| q.lock().unwrap_or_else(|e| e.into_inner()).pop()) else { return };
    let result = match bytes.as_deref().map(std::str::from_utf8) {
        None => Err(TOO_LARGE.to_string()),
        Some(Ok(source)) => chosen(app, json!({"source": source, "name": name})),
        Some(Err(_)) => Err("the script is not UTF-8 text".to_string()),
    };
    if let Err(e) = result {
        app.status(e);
    }
}

/// `ui.runScript`: the script at `path` or given as `source`; without either, pick a file.
pub(crate) fn command(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if p.get("path").is_some() || p.get("source").is_some() {
        return chosen(app, p.clone());
    }
    if let Some(pick) = app.services.script_async.as_mut() {
        pick();
        return Ok(Value::Null);
    }
    let pick = FilePick { filters: vec![FilePick::filter("Scripts", RUN_EXTS)], ..Default::default() };
    let path = crate::picks::open(app, &pick).ok_or("cancelled")?;
    chosen(app, json!({ "path": path }))
}

/// A script was chosen: `params` are `script.run`'s. It runs after the warning, when that is on.
pub(crate) fn chosen(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    let path = params.get("path").and_then(Value::as_str);
    if path.is_some_and(|path| !runs(path)) {
        return Err("Choose a .js or .jsx script".into());
    }
    if !app.session.prefs.show_external_script_warning {
        return app.run("script.run", params);
    }
    let name = params.get("name").and_then(Value::as_str).or(path).map_or_else(|| "script".to_string(), vectorcraft_engine::cmd::fileio::file_name);
    app.ui.dialog = Some(Dialog::new(KIND, json!({"name": name, "script": params, "dontShow": false})));
    Ok(Value::Null)
}

fn body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(d.fields.get("name").and_then(Value::as_str).unwrap_or_default()).font(theme::semibold(12.5)).color(t.text_strong));
    ui.add_space(4.0);
    ui.label(tl!("A script can change the open documents and can write files through the commands it runs. Run only scripts you trust."));
    ui.add_space(8.0);
    let dont_show = d.fields.get("dontShow").and_then(Value::as_bool).unwrap_or(false);
    if widgets::check(ui, tl!("Do not show this warning again"), dont_show, true) {
        d.fields.insert("dontShow".into(), json!(!dont_show));
    }
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if d.fields.get("dontShow").and_then(Value::as_bool).unwrap_or(false) {
        app.run("prefs.set", json!({"key": "showExternalScriptWarning", "value": false}))?;
    }
    let script = d.fields.get("script").cloned().unwrap_or(Value::Null);
    app.ui.dialog = None;
    app.run("script.run", script)
}

fn growth_confirm(app: &mut VectorcraftApp, _: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    app.session.script_continue();
    Ok(Value::Null)
}

fn close_confirm(app: &mut VectorcraftApp, _: &Dialog) -> Result<Value, String> {
    app.ui.dialog = None;
    if app.session.script_abandon() {
        let error = app.session.script_failure_text().unwrap_or_default();
        crate::script_runs::failed(app, error);
        app.sync_views();
    }
    Ok(Value::Null)
}
