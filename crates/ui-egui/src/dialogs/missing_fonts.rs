//! The fonts a document opened without (`text.missingFonts`), listed once the missing linked file
//! questions (dialog `missingLinks`) are answered, one document at a time ([`super::settle`]), with
//! Find Fonts (Type › Find Font on the first of them, to replace it) and Close. Neither brings the
//! dialog back for that open. Search Fonts Folder (the `Fonts` folder next to the document, as
//! File › Package writes it) and Find in Folder… look for their files (`text.findFontFiles`, on
//! separate threads), and Add Fonts copies the files the user chooses into VectorCraft's own Fonts
//! folder (`text.addFontFiles`). Type › Find Font's Find in Folder… opens it too
//! (`ui.findFontsInFolder`), and so does `ui.missingFontsDialog`.
//!
//! Fields: `document` (the document's uid), `fonts` (`text.missingFonts`'s rows),
//! `fontsNextToDocument` (or empty), `state` (empty, `searching`, `done`, `stopped` or `failed`),
//! `search` (the search's `id`), `folder`, `found` (`text.findFontFiles`'s `fonts`, with the files
//! found), `searched`, `skipped`, `unreadable`, `stopped`, `error`, `chosen` (the files to add; a
//! font's first file is chosen when it is found), `searchFolder` (set it and confirm to search that
//! folder) and `discard` (set it and confirm for Find Fonts; confirming without either adds the
//! chosen files).

use std::time::Duration;

use serde_json::{Value, json};

use super::DialogSpec;
use crate::picks::PickRequest;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};

/// The dialog kind of the fonts a document opened without.
pub const KIND: &str = "missingFonts";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Missing Fonts").into(),
    body,
    confirm,
    // Close, with Find Fonts beside it; Add Fonts is a button of the body.
    ok: None,
    discard: Some("Find Fonts"),
    min_width: 440.0,
    max_width: Some(540.0),
    inputs: |_| &["searchFolder"],
    ..DialogSpec::FORM
};

/// After a document opened (`document.open`'s result `r`): its Missing Fonts dialog waits for the
/// dialog slot. On the web the status line shows how many fonts are missing instead, unless it
/// shows the open's notes.
pub fn after_open(app: &mut VectorcraftApp, r: &Value) {
    after_open_for(app, r, cfg!(target_arch = "wasm32"));
}

fn after_open_for(app: &mut VectorcraftApp, r: &Value, web: bool) {
    let Some(st) = app.session.active() else { return };
    let uid = st.uid;
    if web {
        let n = vectorcraft_engine::cmd::fontfiles::missing_fonts(&st.doc).len();
        if n > 0 && r["warnings"].as_array().is_none_or(Vec::is_empty) {
            app.status(format!("{n} font(s) aren't available here: their text shows in another font"));
        }
        return;
    }
    if !app.ui.pending_fonts.contains(&uid) {
        app.ui.pending_fonts.push(uid);
    }
}

/// The Missing Fonts dialog of the open document `uid`, when fonts it uses aren't available.
fn dialog_for(app: &VectorcraftApp, uid: u64) -> Option<Dialog> {
    let st = app.session.documents().iter().find(|d| d.uid == uid)?;
    let r = vectorcraft_engine::cmd::fontfiles::missing_fonts_of(st, app.session.search_rules.as_ref());
    let fonts = r["fonts"].as_array().filter(|f| !f.is_empty())?;
    let next = r["fontsNextToDocument"].as_str().unwrap_or_default();
    Some(Dialog::new(KIND, json!({ "document": uid, "fonts": fonts, "fontsNextToDocument": next, "state": "", "chosen": [] })))
}

/// With no dialog open: the dialog of the first queued document that is still open and still
/// misses fonts.
pub(super) fn open_next(app: &mut VectorcraftApp) {
    while app.ui.dialog.is_none() && !app.ui.pending_fonts.is_empty() {
        let uid = app.ui.pending_fonts.remove(0);
        app.ui.dialog = dialog_for(app, uid);
        if app.ui.dialog.is_some() {
            app.ui.fonts_dialog = Some(uid);
        }
    }
}

/// Put the document `uid` at the front of the queue.
fn queue_first(app: &mut VectorcraftApp, uid: u64) {
    app.ui.pending_fonts.retain(|u| *u != uid);
    app.ui.pending_fonts.insert(0, uid);
}

/// Once the Missing Fonts dialog on show is gone: when another dialog took its place (Import PDF
/// for a file opened after the document, for example), its document goes back to the front of the
/// queue, and the dialog opens again once no dialog is open. A dialog closed by Cancel, Add Fonts
/// or Escape stays closed.
pub(super) fn requeue_replaced(app: &mut VectorcraftApp) {
    let Some(uid) = app.ui.fonts_dialog else { return };
    let Some(d) = &app.ui.dialog else {
        app.ui.fonts_dialog = None;
        return;
    };
    if d.kind != KIND || d.fields.get("document").and_then(Value::as_u64) != Some(uid) {
        app.ui.fonts_dialog = None;
        queue_first(app, uid);
    }
}

/// Stop the search the dialog started once the dialog is gone (closed, or another in its place).
pub(super) fn stop_when_closed(app: &mut VectorcraftApp) {
    let Some(id) = app.ui.dialog_search else { return };
    let open = app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND && d.fields.get("search").and_then(Value::as_u64) == Some(id));
    if !open {
        app.ui.dialog_search = None;
        vectorcraft_engine::cmd::fontfiles::stop(&mut app.session, id);
    }
}

/// `ui.missingFontsDialog`: the dialog for the active document; with `folder`, searching it.
pub(crate) fn open_command(app: &mut VectorcraftApp, folder: Option<String>) -> Result<Value, String> {
    let uid = app.session.active().map(|d| d.uid).ok_or("no document open")?;
    let mut d = dialog_for(app, uid).ok_or("No fonts are missing in this document")?;
    if let Some(folder) = folder {
        start_search(app, &mut d, &folder)?;
    }
    show_for(app, uid, d);
    Ok(Value::Null)
}

/// Show `d`, the dialog of the document `uid`, in place of the open dialog (another document's
/// Missing Fonts dialog asks again later).
fn show_for(app: &mut VectorcraftApp, uid: u64, d: Dialog) {
    requeue_replaced(app);
    if let Some(shown) = app.ui.fonts_dialog.filter(|shown| *shown != uid) {
        queue_first(app, shown);
    }
    app.ui.pending_fonts.retain(|u| *u != uid);
    app.ui.fonts_dialog = Some(uid);
    app.ui.dialog = Some(d);
}

/// `ui.findFontsInFolder` (Type › Find Font's Find in Folder…): ask for a folder (or take
/// `folder`), then open the dialog for the active document, searching it.
pub(crate) fn find_in_folder_command(app: &mut VectorcraftApp, folder: Option<String>) -> Result<Value, String> {
    let uid = app.session.active().map(|d| d.uid).ok_or("no document open")?;
    // Nothing to look for: no folder is asked for.
    let mut d = dialog_for(app, uid).ok_or("No fonts are missing in this document")?;
    let folder = match folder {
        Some(f) => f,
        None => crate::picks::folder(app).ok_or("cancelled")?,
    };
    start_search(app, &mut d, &folder)?;
    show_for(app, uid, d);
    Ok(Value::Null)
}

/// Start looking in `folder` for the files of the dialog's fonts; what an earlier search found is
/// dropped.
fn start_search(app: &mut VectorcraftApp, d: &mut Dialog, folder: &str) -> Result<(), String> {
    let fonts: Vec<Value> = d
        .fields
        .get("fonts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|f| json!({ "family": f["family"], "style": f["style"] }))
        .collect();
    let r = app.run("text.findFontFiles", json!({ "folder": folder, "fonts": fonts }))?;
    for k in ["found", "searched", "skipped", "unreadable", "stopped", "error"] {
        d.fields.remove(k);
    }
    d.fields.insert("chosen".into(), json!([]));
    d.fields.insert("search".into(), r["id"].clone());
    d.fields.insert("folder".into(), json!(folder));
    update(d, &r);
    app.ui.dialog_search = r["id"].as_u64();
    Ok(())
}

/// The files a search found for the font `f` (`{family, style}`): the answer's `fonts` row with
/// the same family and style.
fn files_for<'a>(found: &'a [Value], f: &Value) -> &'a [Value] {
    found.iter().find(|r| r["family"] == f["family"] && r["style"] == f["style"]).and_then(|r| r["files"].as_array()).map_or(&[], Vec::as_slice)
}

/// A search's answer `r` into the dialog. A font that gets its first file has that file chosen; a
/// file the user unchecked stays unchecked.
fn update(d: &mut Dialog, r: &Value) {
    let before = d.fields.get("found").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut chosen = d.fields.get("chosen").and_then(Value::as_array).cloned().unwrap_or_default();
    for font in r["fonts"].as_array().into_iter().flatten() {
        let had = !files_for(&before, font).is_empty();
        if let Some(first) = font["files"].as_array().and_then(|f| f.first()).filter(|f| !had && !chosen.contains(f)) {
            chosen.push(first.clone());
        }
    }
    d.fields.insert("chosen".into(), json!(chosen));
    d.fields.insert("found".into(), r["fonts"].clone());
    for k in ["state", "searched", "skipped", "unreadable", "stopped", "error"] {
        match r.get(k) {
            Some(v) => d.fields.insert(k.into(), v.clone()),
            None => d.fields.remove(k),
        };
    }
}

/// While searching: the search's progress into the dialog. A search started elsewhere since
/// stopped this one.
fn follow(app: &mut VectorcraftApp, d: &mut Dialog) {
    // Polled through `session.execute`.
    let Ok(r) = app.session.execute("text.findFontFiles", &json!({})) else { return };
    if d.fields.get("search") != Some(&r["id"]) {
        d.fields.insert("state".into(), json!("stopped"));
        return;
    }
    update(d, &r);
    if let Some(e) = r["error"].as_str() {
        app.status(e);
    }
}

/// A button of the dialog's.
enum Act {
    Search(String),
    Pick,
    Stop,
    Add,
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    if d.str("state") == "searching" {
        follow(app, d);
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    let state = d.str("state");
    let searching = state == "searching";
    widgets::dim_label(ui, tl!("These fonts aren't available. Their text shows in another font."));
    ui.add_space(8.0);
    let fonts = d.fields.get("fonts").and_then(Value::as_array).cloned().unwrap_or_default();
    let found = d.fields.get("found").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut chosen = d.fields.get("chosen").and_then(Value::as_array).cloned().unwrap_or_default();
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for f in &fonts {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.add_space(3.0);
                        let (note, color) = crate::find_font::font_note(f);
                        let name = format!("{} {}{note}", f["family"].as_str().unwrap_or_default(), f["style"].as_str().unwrap_or_default());
                        ui.label(egui::RichText::new(name).font(theme::semibold(12.0)).color(color.unwrap_or(t.text_strong)));
                        let files = files_for(&found, f);
                        for file in files {
                            let path = file.as_str().unwrap_or_default();
                            let name = std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
                            let on = chosen.contains(file);
                            if widgets::check(ui, &name, on, true) {
                                if on {
                                    chosen.retain(|c| c != file);
                                } else {
                                    chosen.push(file.clone());
                                }
                            }
                            ui.add(egui::Label::new(egui::RichText::new(path).size(11.0).color(t.text_dim)).truncate()).on_hover_text(path);
                        }
                        // A failed search (a folder it refuses) didn't look: `progress` says why.
                        if files.is_empty() && !state.is_empty() && state != "failed" {
                            widgets::dim_label(ui, if searching { tl!("Searching…") } else { tl!("Not found") });
                        }
                        ui.add_space(3.0);
                    });
                });
            }
        });
    });
    d.fields.insert("chosen".into(), json!(chosen));
    ui.add_space(10.0);
    let mut act = None;
    let can_pick = crate::picks::can(app, &PickRequest::Folder);
    ui.horizontal(|ui| {
        let next = d.str("fontsNextToDocument");
        if !next.is_empty() && ui.add_enabled(!searching, egui::Button::new(tl!("Search Fonts Folder"))).on_hover_text(&next).clicked() {
            act = Some(Act::Search(next));
        }
        if can_pick && ui.add_enabled(!searching, egui::Button::new(tl!("Find in Folder…"))).clicked() {
            act = Some(Act::Pick);
        }
        if searching {
            if ui.button(tl!("Stop")).clicked() {
                act = Some(Act::Stop);
            }
            ui.add(egui::Spinner::new().size(12.0).color(t.accent));
        }
    });
    progress(ui, d);
    // Once files are found: what copying them means, and Add Fonts.
    if found.iter().any(|f| f["files"].as_array().is_some_and(|files| !files.is_empty())) {
        ui.add_space(8.0);
        widgets::dim_label(ui, tl!("Copy only fonts you own or are licensed to install."));
        if let Some(dir) = vectorcraft_text::app_font_dir() {
            widgets::dim_name(ui, &crate::i18n::fmt(tl!("VectorCraft copies the fonts to {folder}."), &[("folder", &dir.to_string_lossy())]));
        }
        ui.add_space(6.0);
        if ui.add_enabled(!searching && !chosen.is_empty(), egui::Button::new(tl!("Add Fonts"))).clicked() {
            act = Some(Act::Add);
        }
    }
    let r = match act {
        Some(Act::Search(folder)) => start_search(app, d, &folder),
        Some(Act::Pick) => crate::picks::in_dialog(app, d, |app, d| match crate::picks::folder(app) {
            Some(folder) => start_search(app, d, &folder),
            None => Ok(()),
        }),
        Some(Act::Stop) => app.run("text.findFontFiles", json!({ "stop": true })).map(|_| follow(app, d)),
        // Added: the dialog closes.
        Some(Act::Add) => match add_fonts(app, d) {
            Ok(_) => return true,
            Err(e) => Err(e),
        },
        None => Ok(()),
    };
    if let Err(e) = r {
        app.status(e);
    }
    false
}

/// A folder search's progress under the dialog's buttons (fields `state`, `folder`, `searched`,
/// `skipped`, `unreadable`, `stopped` and `error`), in Find Linked Files too.
pub(super) fn progress(ui: &mut egui::Ui, d: &Dialog) {
    if d.str("state") == "searching" {
        widgets::dim_name(ui, &d.str("folder"));
    }
    if let Some(e) = d.fields.get("error").and_then(Value::as_str) {
        widgets::dim_name(ui, &crate::i18n::message(crate::i18n::current(), e));
    }
    if let Some(s) = d.fields.get("searched") {
        let (folders, files) = (s["folders"].to_string(), s["files"].to_string());
        widgets::dim_name(
            ui,
            &crate::i18n::fmt(tl!("{folders} folder(s) and {files} file(s) searched"), &[("folders", &folders), ("files", &files)]),
        );
    }
    let count = |k: &str| d.fields.get(k).and_then(Value::as_u64).filter(|n| *n > 0).map(|n| n.to_string());
    if let Some(n) = count("skipped") {
        let text = tl!("{count} folder(s) skipped, such as other apps' and the system's folders and hidden ones");
        widgets::dim_name(ui, &crate::i18n::fmt(text, &[("count", &n)]));
    }
    if let Some(n) = count("unreadable") {
        widgets::dim_name(ui, &crate::i18n::fmt(tl!("{count} folder(s) or file(s) couldn't be read"), &[("count", &n)]));
    }
    if matches!(d.fields.get("stopped").and_then(Value::as_str), Some("time" | "limit")) {
        widgets::dim_label(ui, tl!("The search stopped at its limit: search a smaller folder."));
    }
}

/// Find Fonts (`discard`): Type › Find Font on the first font the document misses, to replace it;
/// this Missing Fonts dialog doesn't come back. Otherwise (agents), with `searchFolder`: search that
/// folder, and stay open; else Add Fonts.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    if d.bool("discard") {
        app.ui.fonts_dialog = None;
        // Find Font works on the active document; the dialog's may be another tab.
        let uid = d.fields.get("document").and_then(Value::as_u64);
        let index = app.session.documents().iter().position(|doc| Some(doc.uid) == uid).ok_or("that document is no longer open")?;
        app.run("document.activate", json!({ "index": index }))?;
        crate::find_font::open_on_missing(app);
        return Ok(Value::Null);
    }
    let folder = d.str("searchFolder");
    if !folder.is_empty() {
        let mut d = d.clone();
        d.fields.remove("searchFolder");
        // The folder is searched once: a search that doesn't start leaves the dialog without it.
        if let Some(open) = app.ui.dialog.as_mut().filter(|open| open.kind == KIND) {
            open.fields.remove("searchFolder");
        }
        start_search(app, &mut d, &folder)?;
        let state = d.fields.get("state").cloned().unwrap_or(Value::Null);
        app.ui.dialog = Some(d);
        return Ok(json!({ "state": state }));
    }
    add_fonts(app, d)
}

/// Add Fonts: copy the chosen files into VectorCraft's Fonts folder and close.
fn add_fonts(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let chosen = d.fields.get("chosen").and_then(Value::as_array).cloned().unwrap_or_default();
    if chosen.is_empty() {
        return Err("choose the font files to add".into());
    }
    let r = app.run("text.addFontFiles", json!({ "files": chosen }))?;
    app.ui.dialog = None;
    let len = |k: &str| r[k].as_array().map_or(0, Vec::len);
    let n = len("copied") + len("kept");
    let folder = r["folder"].as_str().unwrap_or_default();
    match r["skipped"].as_array().and_then(|s| s.first()) {
        None => app.status(format!("{n} font file(s) are now in {folder}")),
        Some(first) => {
            let failed = len("skipped");
            let file =
                first["file"].as_str().map(std::path::Path::new).and_then(std::path::Path::file_name).map(|f| f.to_string_lossy().into_owned());
            let reason = format!("{}: {}", file.unwrap_or_default(), first["reason"].as_str().unwrap_or_default());
            app.status(format!("{n} font file(s) are now in {folder}; {failed} couldn't be added: {reason}"));
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests;
