//! File › Scripts › Other Script…: choosing a JavaScript file and the warning before it runs
//! (`script.run` runs it), the question a run that keeps adding art asks, the question quitting
//! asks while a script runs, and the dialogs a script opens with `alert`, `confirm` and `prompt`.
//!
//! `ui.runScript {path? | source?, name?}` shows the warning for that script; without either it
//! asks for a `.js` or `.jsx` file (the web opens the browser's file picker). With Preferences ›
//! General › Show External Script Warning off, the chosen script runs at once. Warning fields:
//! `name`, `script` (`script.run`'s params) and `dontShow` (true: turn the warning off).
//!
//! A script's dialog (kind `scriptDialog`) has the fields `kind` (alert, confirm or prompt),
//! `message`, `title` (null: Script Alert), `errorIcon`, `noAsDefault` and `value` (the prompt's
//! text), and takes `answer`: `ok` (alert, prompt), `yes` or `no` (confirm), `cancel` (prompt) or
//! `stop`. OK, Enter or `ui.dialog.confirm` without an `answer` gives the default button's answer.
//! Escape or `ui.dialog.cancel` gives Cancel for a prompt, No for a confirm and Stop for an alert.
//! Stop ends the run and puts back everything it did.
//!
//! A script's file dialog (kind `scriptFiles`: `File.openDialog`, `File.saveDialog`,
//! `Folder.selectDialog` and the objects' `openDlg`, `saveDlg` and `selectDlg`) has the fields
//! `kind` (open, save or folder), `prompt` and `canAlways`, and takes `answer`: `once`, `always`
//! (only with `canAlways`), `deny` or `stop`. Only the person at the keyboard answers `once` or
//! `always`: from `ui.dialog.set`, `ui.dialog.confirm` or synthetic input they are refused, and the
//! dialog stays open; `deny` and `stop` are taken from automation too. The paths it allows are the
//! ones Choose… picked in the host's file dialog, at most 2,048; `ui.dialog.set` cannot set them,
//! and the dialog's own fields change neither what it allows nor what it says it allows. Escape,
//! `ui.dialog.cancel`, Enter, and OK or `ui.dialog.confirm` without an `answer` give Deny.
//! `selectDlg` opens the host's folder picker in its own folder.

use serde_json::{Value, json};
use vectorcraft_engine::cmd::scripts::{RUN_EXTS, TOO_LARGE, runs};
use vectorcraft_engine::script::dialogs::{MAX_PROMPT_BYTES, cut_to, shown_text};
use vectorcraft_engine::script::files::{MAX_ENTRIES, PickAnswer, PickerKind, ScriptPicker};
use vectorcraft_engine::script::{DialogAnswer, ScriptDialog};

use super::{DialogAction, DialogSpec};
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{FilePick, VectorcraftApp, widgets};

/// The dialog kind of the warning before a script runs.
pub const KIND: &str = "scriptRun";
/// The dialog kind of the question a run asks when it has added a great deal.
pub const GROWTH_KIND: &str = "scriptGrowth";
/// The dialog kind of the question quitting asks while a script runs.
pub const CLOSE_KIND: &str = "scriptClose";
/// The dialog kind of a script's `alert`, `confirm` and `prompt`.
pub const NOTICE_KIND: &str = "scriptDialog";
/// The dialog kind of a script's file dialogs.
pub const PICKER_KIND: &str = "scriptFiles";

pub(super) const PICKER_SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Script File Access").into(),
    body_with_actions: Some(picker_body),
    confirm: picker_confirm,
    ok: None,
    inputs: |_| &["answer"],
    min_width: 420.0,
    max_width: Some(560.0),
    ..DialogSpec::FORM
};

pub(super) const NOTICE_SPEC: DialogSpec = DialogSpec {
    heading: notice_heading,
    body_with_actions: Some(notice_body),
    confirm: notice_confirm,
    ok: None,
    inputs: |_| &["answer"],
    min_width: 360.0,
    max_width: Some(560.0),
    ..DialogSpec::FORM
};

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

/// Quitting while a script runs asks whether to stop it, also over the script's own dialog, which
/// Cancel shows again as it was. Any other open dialog stays, and quitting asks nothing yet.
pub(crate) fn ask_to_close(app: &mut VectorcraftApp) {
    let notice = app.ui.dialog.take_if(|d| d.kind == NOTICE_KIND || d.kind == PICKER_KIND);
    if app.ui.dialog.is_some() {
        return;
    }
    let mut fields = json!({});
    if let Some(notice) = notice {
        fields[super::PARENT] = serde_json::to_value(notice).unwrap_or_default();
    }
    app.ui.dialog = Some(Dialog::new(CLOSE_KIND, fields));
}

/// Closes the question quitting asks once the run it asks about has ended, and forgets the
/// script's dialog it was asked over once the run no longer waits at that dialog, so Cancel does
/// not show it again.
pub(crate) fn settle_close(app: &mut VectorcraftApp) {
    if !app.ui.dialog.as_ref().is_some_and(|d| d.kind == CLOSE_KIND) {
        return;
    }
    if !app.session.script_running() {
        app.ui.dialog = None;
    } else if app.session.script_dialog().is_none()
        && app.session.script_picker().is_none()
        && let Some(d) = app.ui.dialog.as_mut()
    {
        d.fields.remove(super::PARENT);
    }
}

/// Shows the dialog the running script waits at.
pub(crate) fn open_notice(app: &mut VectorcraftApp, d: &ScriptDialog) {
    let fields = json!({
        "kind": d.kind.id(),
        "message": d.message,
        "title": d.title,
        "errorIcon": d.error_icon,
        "noAsDefault": d.no_as_default,
        "value": d.preset,
    });
    app.ui.dialog = Some(Dialog::new(NOTICE_KIND, fields));
}

/// The dialog's heading: the script's title on one line, or Script Alert.
pub(crate) fn notice_heading(d: &Dialog) -> String {
    match d.fields.get("title").and_then(Value::as_str) {
        Some(title) => shown_text(title, false),
        None => tl!("Script Alert").into(),
    }
}

/// The answer of the default button: OK, or for a confirm Yes, or No with `noAsDefault`.
fn default_answer(d: &Dialog) -> &'static str {
    match (d.str("kind").as_str(), d.bool("noAsDefault")) {
        ("confirm", true) => "no",
        ("confirm", false) => "yes",
        _ => "ok",
    }
}

/// A button that is the primary one when it is the default.
fn answer_button(ui: &mut egui::Ui, label: &str, default: bool) -> bool {
    if default { widgets::primary_button_enabled(ui, label, true).clicked() } else { widgets::secondary_button(ui, label).clicked() }
}

fn notice_body(_: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> DialogAction {
    let t = Tokens::get(ui.ctx());
    let kind = d.str("kind");
    let message = shown_text(&d.str("message"), true);
    ui.horizontal_top(|ui| {
        if d.bool("errorIcon") {
            ui.label(egui::RichText::new("!").font(theme::semibold(22.0)).color(t.error));
            ui.add_space(8.0);
        }
        egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| ui.label(&message));
    });
    if kind == "prompt" {
        ui.add_space(8.0);
        let name = cut_to(&shown_text(&message, false), 200).to_string();
        super::form::text_edit_named(ui, d, "value", &name, ui.available_width());
        let value = d.str("value");
        if value.len() > MAX_PROMPT_BYTES {
            d.fields.insert("value".into(), json!(cut_to(&value, MAX_PROMPT_BYTES)));
        }
    }
    ui.add_space(16.0);
    let default = default_answer(d);
    let mut answer = None;
    let row = egui::vec2(ui.min_rect().width(), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        let buttons: &[(&str, &str)] = match kind.as_str() {
            "confirm" => &[("yes", "Yes"), ("no", "No")],
            "prompt" => &[("ok", "OK"), ("cancel", "Cancel")],
            _ => &[("ok", "OK")],
        };
        for (id, label) in buttons {
            if answer_button(ui, tl!(label), *id == default) {
                answer = Some(*id);
            }
            ui.add_space(8.0);
        }
        // The row runs right to left: Stop goes at its left end.
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            if widgets::secondary_button(ui, tl!("Stop")).clicked() {
                answer = Some("stop");
            }
        });
    });
    if answer.is_none() && crate::shortcuts::pressed_anew(ui.ctx(), egui::Key::Enter) {
        answer = Some(default);
    }
    let Some(answer) = answer else { return DialogAction::None };
    d.fields.insert("answer".into(), json!(answer));
    DialogAction::Confirm
}

/// Gives the running script the dialog's `answer`, or the default button's.
fn notice_confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let answer = d.fields.get("answer").and_then(Value::as_str).unwrap_or(default_answer(d));
    give(app, d, answer)
}

/// Escape or Cancel: Cancel for a prompt, No for a confirm and Stop for an alert.
pub(super) fn notice_cancel(app: &mut VectorcraftApp, d: &Dialog) {
    let answer = match d.str("kind").as_str() {
        "prompt" => "cancel",
        "confirm" => "no",
        _ => "stop",
    };
    let _ = give(app, d, answer);
}

fn give(app: &mut VectorcraftApp, d: &Dialog, answer: &str) -> Result<Value, String> {
    let given = match (d.str("kind").as_str(), answer) {
        (_, "stop") => None,
        ("alert", "ok") => Some(DialogAnswer::Ok),
        ("confirm", "yes") => Some(DialogAnswer::Confirm(true)),
        ("confirm", "no") => Some(DialogAnswer::Confirm(false)),
        ("prompt", "ok") => Some(DialogAnswer::Prompt(Some(d.str("value")))),
        ("prompt", "cancel") => Some(DialogAnswer::Prompt(None)),
        (kind, _) => return Err(format!("`answer` for this {kind} dialog must be one of: {}", answers_for(kind))),
    };
    match given {
        None => {
            app.session.script_request_stop();
        }
        Some(a) => {
            // Refused while the dialog still waits: the text is too long, and the dialog stays open.
            if !app.session.script_answer(a) && app.session.script_dialog().is_some() {
                return Err(format!("the prompt's text is longer than {MAX_PROMPT_BYTES} bytes"));
            }
        }
    }
    if app.ui.dialog.as_ref().is_some_and(|open| open.kind == NOTICE_KIND) {
        app.ui.dialog = None;
    }
    Ok(Value::Null)
}

/// Shows the file dialog the running script waits at.
pub(crate) fn open_picker(app: &mut VectorcraftApp, p: &ScriptPicker) {
    let fields = json!({
        "kind": p.kind.id(),
        "prompt": p.prompt,
        "multiple": p.multiple,
        "start": p.start,
        "filters": p.filters,
        "canAlways": p.can_always,
    });
    app.script_picked.clear();
    app.script_picked_left_out = false;
    app.ui.dialog = Some(Dialog::new(PICKER_KIND, fields));
}

/// Shows the host's file dialog of the kind the running script asked for, starting at the script's
/// file or folder, with the script's file types, and keeps what it picked in
/// [`VectorcraftApp::script_picked`]: in place of the paths before, or after them when the script
/// asked for several files. At most [`MAX_ENTRIES`] paths are kept, the number of files the script
/// is given; the dialog tells the person when it left out more.
pub(crate) fn choose(app: &mut VectorcraftApp, d: &mut Dialog) {
    let r = crate::picks::in_app_dialog(app, d, |app, _| {
        let Some(p) = app.session.script_picker() else { return Ok(()) };
        let start = p.start.as_deref().map(std::path::Path::new);
        let name = start.and_then(|s| s.file_name()).map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let folder = start.and_then(|s| s.parent()).map(|s| s.to_string_lossy().into_owned());
        let pick = FilePick { name, folder, filters: p.filters.clone() };
        let several = p.multiple && p.kind == PickerKind::Open;
        let chosen: Vec<String> = match p.kind {
            PickerKind::Open if several => crate::picks::open_files(app, &pick),
            PickerKind::Open => crate::picks::open(app, &pick).into_iter().collect(),
            PickerKind::Save => crate::picks::save(app, &pick).into_iter().collect(),
            // `selectDlg` starts in its folder; `Folder.selectDialog` gives none.
            PickerKind::Folder => match p.start.clone() {
                Some(start) => crate::picks::folder_at(app, &start).into_iter().collect(),
                None => crate::picks::folder(app).into_iter().collect(),
            },
        };
        if !chosen.is_empty() {
            let mut paths = if several { std::mem::take(&mut app.script_picked) } else { Vec::new() };
            let mut seen: std::collections::HashSet<String> = paths.iter().cloned().collect();
            for path in chosen {
                if seen.contains(&path) {
                    continue;
                }
                if paths.len() >= MAX_ENTRIES {
                    app.script_picked_left_out = true;
                    break;
                }
                seen.insert(path.clone());
                paths.push(path);
            }
            app.script_picked = paths;
        }
        Ok(())
    });
    // Picking a path doesn't fail.
    let _ = r;
}

/// The dialog's prompt and the access it describes come from the request the run waits at, not
/// from the dialog's fields, so `ui.dialog.set` cannot change what the person reads.
fn picker_body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> DialogAction {
    let t = Tokens::get(ui.ctx());
    let Some((kind, prompt)) = app.session.script_picker().map(|p| (p.kind, shown_text(&p.prompt, true))) else { return DialogAction::None };
    if !prompt.is_empty() {
        egui::ScrollArea::vertical().max_height(160.0).show(ui, |ui| ui.label(&prompt));
        ui.add_space(8.0);
    }
    ui.label(picker_request(kind));
    ui.add_space(8.0);
    let paths = app.script_picked.clone();
    ui.horizontal(|ui| {
        if widgets::secondary_button(ui, tl!("Choose…")).clicked() {
            choose(app, d);
        }
        ui.add_space(8.0);
        if let [path] = paths.as_slice() {
            ui.label(egui::RichText::new(shown_text(path, false)).color(t.text_strong));
        }
    });
    if paths.len() > 1 {
        ui.add_space(4.0);
        egui::ScrollArea::vertical().id_salt("script-picked").max_height(120.0).show(ui, |ui| {
            for path in &paths {
                ui.label(egui::RichText::new(shown_text(path, false)).color(t.text_strong));
            }
        });
    }
    if app.script_picked_left_out {
        ui.add_space(4.0);
        ui.colored_label(t.error, crate::i18n::msg(PICKED_LEFT_OUT));
    }
    ui.add_space(16.0);
    let chosen = !app.script_picked.is_empty();
    let can_always = app.session.script_picker().is_some_and(|p| p.can_always);
    let mut answer = None;
    let row = egui::vec2(ui.min_rect().width(), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if widgets::primary_button_enabled(ui, tl!("Allow Once"), chosen).clicked() {
            answer = Some("once");
        }
        ui.add_space(8.0);
        if can_always {
            if ui.add_enabled_ui(chosen, |ui| widgets::secondary_button(ui, tl!("Always Allow This Script"))).inner.clicked() {
                answer = Some("always");
            }
            ui.add_space(8.0);
        }
        if widgets::secondary_button(ui, tl!("Deny")).clicked() {
            answer = Some("deny");
        }
        // The row runs right to left: Stop goes at its left end.
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            if widgets::secondary_button(ui, tl!("Stop")).clicked() {
                answer = Some("stop");
            }
        });
    });
    // Enter gives the dialog's default answer, Deny, as `ui.dialog.confirm` without an answer does,
    // unless it goes to the button that has the keyboard focus (Choose…).
    let unfocused = ui.ctx().memory(|m| m.focused().is_none());
    if answer.is_none() && unfocused && crate::shortcuts::pressed_anew(ui.ctx(), egui::Key::Enter) {
        answer = Some("deny");
    }
    let Some(answer) = answer else { return DialogAction::None };
    d.fields.insert("answer".into(), json!(answer));
    DialogAction::Confirm
}

/// What the file dialog of `kind` asks the person to allow.
fn picker_request(kind: PickerKind) -> &'static str {
    match kind {
        PickerKind::Open => tl!("The script asks to read a file you choose."),
        PickerKind::Save => tl!("The script asks to read and write a file you choose."),
        PickerKind::Folder => tl!("The script asks to read and write a folder you choose, with everything in it."),
    }
}

/// The note the file dialog shows when the person picked more files than a script is given.
const PICKED_LEFT_OUT: &str = "A script is given at most 2,048 files: the files picked after them were left out";

/// Gives the running script the file dialog's `answer`, or Deny.
fn picker_confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let answer = d.fields.get("answer").and_then(Value::as_str).unwrap_or("deny");
    give_pick(app, answer)
}

/// Escape or Cancel: Deny.
pub(super) fn picker_cancel(app: &mut VectorcraftApp, _: &Dialog) {
    let _ = give_pick(app, "deny");
}

/// Gives the running script `answer` with the paths Choose… picked. Only the person at the keyboard
/// allows access: Allow Once and Always Allow are refused, and the dialog stays open, while
/// automation sends the answer (a control or MCP request, or synthetic input) or opened the host's
/// file dialog; Deny and Stop are taken from anyone.
fn give_pick(app: &mut VectorcraftApp, answer: &str) -> Result<Value, String> {
    let paths = app.script_picked.clone();
    let can_always = app.session.script_picker().is_some_and(|p| p.can_always);
    let given = match answer {
        "stop" => None,
        "deny" => Some(PickAnswer::Deny),
        "once" | "always" if app.automation_asked() => return Err("Only the person at the keyboard can allow a script's file access".into()),
        "once" | "always" if paths.is_empty() => return Err("choose a file or folder first, or deny".into()),
        "always" if !can_always => return Err("this script's file access can be allowed only once".into()),
        "once" => Some(PickAnswer::Allow { paths, always: false }),
        "always" => Some(PickAnswer::Allow { paths, always: true }),
        _ => return Err("`answer` for a script's file dialog must be one of: once, always, deny, stop".into()),
    };
    app.script_picked.clear();
    app.script_picked_left_out = false;
    match given {
        None => {
            app.session.script_request_stop();
        }
        Some(a) => {
            app.session.script_pick(a);
        }
    }
    if app.ui.dialog.as_ref().is_some_and(|open| open.kind == PICKER_KIND) {
        app.ui.dialog = None;
    }
    Ok(Value::Null)
}

fn answers_for(kind: &str) -> &'static str {
    match kind {
        "confirm" => "yes, no, stop",
        "prompt" => "ok, cancel, stop",
        _ => "ok, stop",
    }
}
