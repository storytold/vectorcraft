//! Find Linked Files: the files of a document's missing linked images and placed documents (from
//! the Links panel, of the selected ones whatever their status, when any are selected), looked for
//! in a folder the user picks and its subfolders (`links.findFiles`, on separate threads). Each
//! file it looks for lists the files found with its name, best first, with Don't Relink: a file is
//! chosen at the start only when it is the only file found, or the only one that is the same file
//! (the size and hash saved with the link). Relink relinks each choice (`links.relink`, one undo
//! step each); with Copy into the document's folder (a saved document), an image's file is copied
//! there first, never replacing a file, and a placed document is relinked where its file is. Opened
//! from Find in Folder… in Linked File Not Found (dialog `missingLinks`), the files left without a
//! choice and those that fail to relink go back to that dialog; opened from the Links panel's Find
//! in Folder… (`ui.findLinksInFolder`), they stay as they are.
//!
//! Fields: `document` (the document's uid), `missing` (`[{name, path, ids}]`), `modified` (the ids
//! of the images whose file was modified, offered for update afterwards), `back` (the files left
//! without a choice and those that fail to relink go back to dialog `missingLinks`), `state`
//! (empty, `searching`, `done`, `stopped` or `failed`), `search` (the search's `id`), `folder`,
//! `found` (`links.findFiles`'s `links`, with the files found), `searched`, `skipped`,
//! `unreadable`, `stopped`, `error`, `picks` (the file chosen for each row of `missing`, or null),
//! `touched` (the rows whose file was chosen in the dialog: new search results leave their pick as
//! it is), `copy` and `searchFolder` (set it and confirm to search that folder).

use std::time::Duration;

use serde_json::{Value, json};

use super::DialogSpec;
use crate::picks::PickRequest;
use crate::state::Dialog;
use crate::theme::{self, Tokens};
use crate::{VectorcraftApp, widgets};

/// The dialog kind of Find Linked Files.
pub const KIND: &str = "findLinks";

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Find Linked Files").into(),
    body,
    confirm,
    ok: Some("Relink"),
    min_width: 440.0,
    max_width: Some(560.0),
    inputs: |_| &["searchFolder"],
    ..DialogSpec::FORM
};

/// The rows of field `key` of `d` (`missing`, `modified`, `picks`…).
fn rows(d: &Dialog, key: &str) -> Vec<Value> {
    d.fields.get(key).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The ids in `rows` (`[{ids}]`).
fn ids(rows: &[Value]) -> Vec<Value> {
    rows.iter().flat_map(|r| r["ids"].as_array().cloned().unwrap_or_default()).collect()
}

/// A Find Linked Files dialog for the `missing` files of the active document.
fn dialog(app: &VectorcraftApp, missing: Vec<Value>, modified: Vec<Value>, back: bool) -> Result<Dialog, String> {
    let uid = app.session.active().map(|d| d.uid).ok_or("no document open")?;
    let picks = vec![Value::Null; missing.len()];
    let fields =
        json!({ "document": uid, "missing": missing, "modified": modified, "back": back, "state": "", "picks": picks, "touched": [], "copy": false });
    Ok(Dialog::new(KIND, fields))
}

/// Linked File Not Found's Find in Folder… (`d`, the `missingLinks` dialog): `d` becomes Find
/// Linked Files, searching `folder` for every file `d` still asks about.
pub(super) fn from_missing_links(app: &mut VectorcraftApp, d: &mut Dialog, folder: &str) -> Result<(), String> {
    let mut find = dialog(app, rows(d, "missing"), rows(d, "modified"), true)?;
    start_search(app, &mut find, folder)?;
    *d = find;
    Ok(())
}

/// The selected linked images and placed documents of the active document.
fn selected_linked(app: &VectorcraftApp) -> Vec<Value> {
    let Some(st) = app.session.active() else { return vec![] };
    let linked = |id: &vectorcraft_doc::NodeId| {
        st.doc.node(*id).is_some_and(|n| match &n.kind {
            vectorcraft_doc::NodeKind::Image(im) => im.link.is_some(),
            vectorcraft_doc::NodeKind::PlacedDocument(_) => true,
            _ => false,
        })
    };
    st.selection.objects.iter().filter(|id| linked(id)).map(|id| json!(id.0)).collect()
}

/// `ui.findLinksInFolder` (the Links panel's Find in Folder…): ask for a folder (or take `folder`),
/// then open Find Linked Files searching it for the files of linked objects `ids` (default: the
/// selected linked images and placed documents, else the files that are missing).
pub(crate) fn find_in_folder_command(app: &mut VectorcraftApp, folder: Option<String>, ids: Option<Vec<Value>>) -> Result<Value, String> {
    let ids = ids.filter(|ids| !ids.is_empty()).or_else(|| Some(selected_linked(app)).filter(|ids| !ids.is_empty()));
    let all = ids.is_some();
    let r = app.session.execute("links.check", &json!({ "ids": ids })).map_err(|e| e.to_string())?;
    let missing: Vec<Value> = r["links"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| all || l["status"] == "missing")
        .map(|l| json!({ "name": l["name"], "path": l["path"], "ids": l["ids"] }))
        .collect();
    // Nothing to look for: no folder is asked for.
    if missing.is_empty() {
        return Err("No linked files to look for".into());
    }
    let folder = match folder {
        Some(f) => f,
        None => crate::picks::folder(app).ok_or("cancelled")?,
    };
    let mut d = dialog(app, missing, vec![], false)?;
    start_search(app, &mut d, &folder)?;
    app.ui.dialog = Some(d);
    Ok(Value::Null)
}

/// Whether object `id` of the active document is an image (and not a placed document).
fn is_image(app: &VectorcraftApp, id: &Value) -> bool {
    let Some(id) = id.as_u64() else { return false };
    let st = app.session.active();
    st.and_then(|st| st.doc.node(vectorcraft_doc::NodeId(id))).is_some_and(|n| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_)))
}

/// Make the dialog's document the active one: it may be another tab's.
fn activate(app: &mut VectorcraftApp, d: &Dialog) -> Result<(), String> {
    let uid = d.fields.get("document").and_then(Value::as_u64);
    if app.session.active().map(|st| st.uid) == uid {
        return Ok(());
    }
    let index = app.session.documents().iter().position(|doc| Some(doc.uid) == uid).ok_or("that document is no longer open")?;
    app.run("document.activate", json!({ "index": index })).map(|_| ())
}

/// Start looking in `folder` for the files of the dialog's rows; what an earlier search found, and
/// the files chosen from it, are dropped.
fn start_search(app: &mut VectorcraftApp, d: &mut Dialog, folder: &str) -> Result<(), String> {
    activate(app, d)?;
    let r = app.run("links.findFiles", json!({ "folder": folder, "ids": ids(&rows(d, "missing")) }))?;
    for k in ["found", "searched", "skipped", "unreadable", "stopped", "error"] {
        d.fields.remove(k);
    }
    d.fields.insert("picks".into(), json!(vec![Value::Null; rows(d, "missing").len()]));
    d.fields.insert("touched".into(), json!([]));
    d.fields.insert("search".into(), r["id"].clone());
    d.fields.insert("folder".into(), json!(folder));
    update(d, &r);
    app.ui.links_search = r["id"].as_u64();
    Ok(())
}

/// The files a search found for `row` (`{path, ids}`), best first: the answer's `links` row with
/// the same path and ids.
fn files_for<'a>(found: &'a [Value], row: &Value) -> &'a [Value] {
    found.iter().find(|r| r["path"] == row["path"] && r["ids"] == row["ids"]).and_then(|r| r["files"].as_array()).map_or(&[], Vec::as_slice)
}

/// The file chosen at the start among `files`: the only one that is the same file, else the only
/// file found.
fn first_pick(files: &[Value]) -> Option<&Value> {
    let mut same = files.iter().filter(|f| f["match"] == "same");
    match (same.next(), same.next(), files) {
        (Some(f), None, _) | (None, _, [f]) => Some(&f["path"]),
        _ => None,
    }
}

/// A search's answer `r` into the dialog. A row whose file wasn't chosen in the dialog takes the
/// file chosen at the start ([`first_pick`]).
fn update(d: &mut Dialog, r: &Value) {
    let missing = rows(d, "missing");
    let touched = rows(d, "touched");
    let mut picks = rows(d, "picks");
    picks.resize(missing.len(), Value::Null);
    let found = r["links"].as_array().map_or(&[][..], Vec::as_slice);
    for (i, (row, pick)) in missing.iter().zip(picks.iter_mut()).enumerate() {
        if !touched.contains(&json!(i)) {
            *pick = first_pick(files_for(found, row)).cloned().unwrap_or(Value::Null);
        }
    }
    d.fields.insert("picks".into(), json!(picks));
    d.fields.insert("found".into(), r["links"].clone());
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
    let Ok(r) = app.session.execute("links.findFiles", &json!({})) else { return };
    if d.fields.get("search") != Some(&r["id"]) {
        d.fields.insert("state".into(), json!("stopped"));
        return;
    }
    update(d, &r);
    if let Some(e) = r["error"].as_str() {
        app.status(e);
    }
}

/// Stop the search the dialog started once the dialog is gone (closed, or another in its place).
pub(super) fn stop_when_closed(app: &mut VectorcraftApp) {
    let Some(id) = app.ui.links_search else { return };
    let open = app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND && d.fields.get("search").and_then(Value::as_u64) == Some(id));
    if !open {
        app.ui.links_search = None;
        vectorcraft_engine::cmd::linkfiles::stop(&mut app.session, id);
    }
}

/// The folder of the dialog's document, when it was saved.
fn document_folder(app: &VectorcraftApp, d: &Dialog) -> Option<String> {
    let uid = d.fields.get("document").and_then(Value::as_u64)?;
    let path = app.session.documents().iter().find(|doc| doc.uid == uid)?.path.clone()?;
    std::path::Path::new(&path).parent().map(|p| p.to_string_lossy().into_owned())
}

/// A button of the dialog's.
enum Act {
    Pick,
    Stop,
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let t = Tokens::get(ui.ctx());
    if d.str("state") == "searching" {
        follow(app, d);
        ui.ctx().request_repaint_after(Duration::from_millis(100));
    }
    let state = d.str("state");
    let searching = state == "searching";
    widgets::dim_label(ui, tl!("Choose the file to relink each linked file to. Files with the size and contents saved with the link come first."));
    ui.add_space(8.0);
    let missing = rows(d, "missing");
    let found = rows(d, "found");
    let mut picks = rows(d, "picks");
    picks.resize(missing.len(), Value::Null);
    let mut touched = rows(d, "touched");
    widgets::list_box(ui, |ui| {
        egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
            ui.set_width(ui.available_width());
            for (i, (row, pick)) in missing.iter().zip(picks.iter_mut()).enumerate() {
                ui.horizontal(|ui| {
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.add_space(3.0);
                        ui.label(egui::RichText::new(row["name"].as_str().unwrap_or_default()).font(theme::semibold(12.0)).color(t.text_strong));
                        let path = row["path"].as_str().unwrap_or_default();
                        ui.add(egui::Label::new(egui::RichText::new(path).size(11.0).color(t.text_dim)).truncate()).on_hover_text(path);
                        let files = files_for(&found, row);
                        let mut chose = None;
                        for f in files {
                            let file = f["path"].as_str().unwrap_or_default();
                            let p = std::path::Path::new(file);
                            ui.horizontal(|ui| {
                                let name = p.file_name().map_or_else(|| file.to_string(), |n| n.to_string_lossy().into_owned());
                                if widgets::radio(ui, &name, *pick == f["path"], true) {
                                    chose = Some(f["path"].clone());
                                }
                                let tag = match f["match"].as_str() {
                                    Some("same") => tl!("Same file"),
                                    Some("size") => tl!("Same size"),
                                    _ => "",
                                };
                                if !tag.is_empty() {
                                    ui.label(egui::RichText::new(tag).size(11.0).color(t.accent));
                                }
                            });
                            let folder = p.parent().map_or_else(String::new, |f| f.to_string_lossy().into_owned());
                            ui.add(egui::Label::new(egui::RichText::new(folder).size(11.0).color(t.text_dim)).truncate()).on_hover_text(file);
                        }
                        if !files.is_empty() && widgets::radio(ui, tl!("Don't Relink"), pick.is_null(), true) {
                            chose = Some(Value::Null);
                        }
                        if let Some(c) = chose {
                            *pick = c;
                            if !touched.contains(&json!(i)) {
                                touched.push(json!(i));
                            }
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
    d.fields.insert("picks".into(), json!(picks));
    d.fields.insert("touched".into(), json!(touched));
    ui.add_space(10.0);
    let mut act = None;
    ui.horizontal(|ui| {
        if crate::picks::can(app, &PickRequest::Folder) && ui.add_enabled(!searching, egui::Button::new(tl!("Find in Folder…"))).clicked() {
            act = Some(Act::Pick);
        }
        if searching {
            if ui.button(tl!("Stop")).clicked() {
                act = Some(Act::Stop);
            }
            ui.add(egui::Spinner::new().size(12.0).color(t.accent));
        }
    });
    super::missing_fonts::progress(ui, d);
    if let Some(folder) = document_folder(app, d) {
        ui.add_space(8.0);
        let copy = d.bool("copy");
        if widgets::check_tip(ui, tl!("Copy into the document's folder"), copy, true, &folder) {
            d.fields.insert("copy".into(), json!(!copy));
        }
    }
    let r = match act {
        Some(Act::Pick) => crate::picks::in_dialog(app, d, |app, d| match crate::picks::folder(app) {
            Some(folder) => start_search(app, d, &folder),
            None => Ok(()),
        }),
        Some(Act::Stop) => app.run("links.findFiles", json!({ "stop": true })).map(|_| follow(app, d)),
        None => Ok(()),
    };
    if let Err(e) = r {
        app.status(e);
    }
    false
}

/// With `searchFolder` (agents): search that folder, and stay open. Otherwise Relink: each row's
/// file, one `links.relink` each (copied into the document's folder with `copy`). The status line
/// names the first file that fails to relink, and the other files still relink. From Linked File
/// Not Found, the rows left without a file and the rows that failed to relink go back to it, then
/// the modified files are offered for update.
fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
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
    activate(app, d)?;
    let picks = rows(d, "picks");
    let (mut relinked, mut failed, mut rest) = (vec![], vec![], vec![]);
    for (i, row) in rows(d, "missing").into_iter().enumerate() {
        let Some(path) = picks.get(i).and_then(Value::as_str) else {
            rest.push(row);
            continue;
        };
        let mut params = json!({ "ids": row["ids"], "path": path });
        // A placed document is relinked where its file is.
        if d.bool("copy") && row["ids"].as_array().is_some_and(|ids| ids.iter().all(|id| is_image(app, id))) {
            params["copy"] = json!(true);
        }
        match app.run("links.relink", params) {
            Ok(r) => relinked.extend(r["relinked"].as_array().cloned().unwrap_or_default()),
            Err(e) => {
                failed.push(json!({ "name": row["name"], "error": e }));
                rest.push(row);
            }
        }
    }
    app.ui.dialog = None;
    let n = relinked.len();
    match failed.first() {
        Some(f) => {
            let (name, reason) = (f["name"].as_str().unwrap_or_default(), f["error"].as_str().unwrap_or_default());
            app.status(format!("Relinked {n} image(s); {name} couldn't be relinked: {reason}"));
        }
        None if n > 0 => app.status(format!("Relinked {n} image(s)")),
        None => {}
    }
    if d.bool("back") {
        super::missing_links::ask_next(app, rest, rows(d, "modified"));
    }
    Ok(json!({ "relinked": relinked, "failed": failed }))
}

#[cfg(test)]
mod tests;
