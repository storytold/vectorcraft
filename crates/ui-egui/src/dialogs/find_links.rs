//! Find Linked Files: the files of a document's missing linked images and placed documents (from
//! the Links panel, of the selected ones whatever their status, when any are selected), looked for
//! in a folder the user picks and its subfolders (`links.findFiles`, on separate threads). Each
//! file it looks for lists the files found with its name, best first, with Don't Relink: a file is
//! chosen at the start only when it is the only file found, or the only one that is the same file
//! (the size and hash saved with the link). Relink relinks each choice (`links.relink`, one undo
//! step each); with Copy into the document's folder (a saved document), an image's file is copied
//! there first, never replacing a file, and a placed document is relinked where its file is. Opened
//! from Find in Folder… in Linked File Not Found ([`super::missing_links`]), the files left without a
//! choice and those that fail to relink go back to that dialog; opened from the Links panel's Find
//! in Folder… (`ui.findLinksInFolder`), they stay as they are.
//!
//! Fields: `document` (the document's uid), `missing` (`[{name, path, ids}]`), `modified` (the ids
//! of the images whose file was modified, offered for update afterwards), `notes` (from
//! `missingLinks`: the open's notes, shown with the line about the modified files), `back` (the
//! files left without a choice and those that fail to relink go back to dialog `missingLinks`), `state`
//! (empty, `searching`, `done`, `stopped` or `failed`), `search` (the search's id), `folder`,
//! `found` (`links.findFiles`'s `links`, with the files found), `searched`, `skipped`,
//! `unreadable`, `stopped`, `error`, `picks` (the file chosen for each row of `missing`, or null),
//! `touched` (the rows whose file was chosen in the dialog: new search results leave their pick as
//! it is), `copy` and `searchFolder` (set it and confirm to search that folder).
//!
//! The dialog keeps those fields as its state ([`Fields`]) and files what needs the app (starting,
//! stopping and following the search) as a [`Request`]; [`pump`] does it each frame, since the
//! dialog has no app to run commands on.

use std::time::Duration;

use craft::host::{PickKind, PickRequest};
use craft::theme::TypeRole;
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Radio, RadioGroup};
use craft::ui::contract::{Bind, Props as _, Scroll, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Badge, Elide, Shade, Spinner, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::FormDialog;
use serde_json::{Map, Value, json};

use super::missing_links::{Asked, LinkAsk, flag_field, ids, label, show_notes_again, text_field};
use crate::VectorcraftApp;
use crate::file_asks::{self, Owner};

/// The dialog kind of Find Linked Files.
pub const KIND: &str = "findLinks";

/// How often the dialog follows a running search, and a picker it waits for.
const POLL: Duration = Duration::from_millis(100);

/// The dialog's state: the fields in the module's header.
type Fields = Map<String, Value>;

/// The rows of field `key` of `d` (`missing`, `modified`, `picks`…).
fn rows(d: &Fields, key: &str) -> Vec<Value> {
    d.get(key).and_then(Value::as_array).cloned().unwrap_or_default()
}

/// The string field `key` of `d`; empty when it has none.
fn text<'a>(d: &'a Fields, key: &str) -> &'a str {
    d.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// The boolean field `key` of `d`.
fn flag(d: &Fields, key: &str) -> bool {
    d.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// The open's notes the dialog keeps (field `notes`).
fn notes(d: &Fields) -> Vec<String> {
    d.get("notes").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

/// What the dialog asks of the app on the next frame ([`pump`]).
enum Request {
    /// Look for the files in this folder.
    Search(String),
    /// Stop the running search.
    Stop,
}

/// What the form edits: the file chosen for each row, the rows chosen in the dialog and Copy.
#[derive(Clone, Default, PartialEq)]
struct Edit {
    picks: Vec<Value>,
    touched: Vec<Value>,
    copy: bool,
}

/// Find Linked Files: the missing files, the files found for each and the one chosen.
pub(crate) struct FindLinksDialog {
    fields: Fields,
    request: Option<Request>,
    search_folder: String,
    asked: Asked<()>,
    can_pick: bool,
    /// The folder of the document, when it was saved: where Copy copies to.
    copy_folder: Option<String>,
}

impl FindLinksDialog {
    /// The dialog over `fields` ([`dialog`]).
    fn new(app: &VectorcraftApp, fields: Fields) -> Self {
        FindLinksDialog {
            copy_folder: document_folder(app, &fields),
            fields,
            request: None,
            search_folder: String::new(),
            asked: Asked::default(),
            can_pick: file_asks::can(app, PickKind::Folder),
        }
    }

    /// The id of the search the dialog started.
    fn search(&self) -> Option<u64> {
        self.fields.get("search").and_then(Value::as_u64)
    }
}

impl StackDialog for FindLinksDialog {
    type Result = Fields;

    fn id(&self) -> DialogId {
        dialog_id(KIND)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<Fields>> {
        let ctx = ui.ctx().clone();
        if let Some((picked, ())) = self.asked.answer(&ctx)
            && let Some(folder) = picked.first_path()
        {
            self.request = Some(Request::Search(folder.to_string_lossy().into_owned()));
        }
        if self.asked.pending() || self.request.is_some() {
            ctx.request_repaint_after(POLL);
        }
        let state = text(&self.fields, "state").to_string();
        let searching = state == "searching";
        let (missing, found) = (rows(&self.fields, "missing"), rows(&self.fields, "found"));
        let progress = super::missing_fonts::progress(&Value::Object(self.fields.clone()));
        let mut edit = Edit { picks: rows(&self.fields, "picks"), touched: rows(&self.fields, "touched"), copy: flag(&self.fields, "copy") };
        edit.picks.resize(missing.len(), Value::Null);
        let (can_pick, copy_folder) = (self.can_pick, self.copy_folder.clone());
        let (mut pick, mut stop) = (false, false);
        let shown =
            FormDialog::new(label(tl!("Find Linked Files")), Bind::bound(&mut edit)).ok_label(label(tl!("Relink"))).show(ui, |ui, _, edit| {
                Text::new(label(tl!(
                    "Choose the file to relink each linked file to. Files with the size and contents saved with the link come first."
                )))
                .shade(Shade::Muted)
                .show(ui);
                Stack::new().scroll(Scroll::vertical()).show(ui, |ui, _| {
                    for (i, row) in missing.iter().enumerate() {
                        let (name, path) = (row["name"].as_str().unwrap_or_default(), row["path"].as_str().unwrap_or_default());
                        Text::new(label(name)).role(TypeRole::BodyStrong).show(ui);
                        Text::new(label(path)).shade(Shade::Muted).elide(Elide::Path).tooltip(path.to_string()).show(ui);
                        let files = files_for(&found, row);
                        let current = files.iter().position(|f| edit.picks.get(i) == Some(&f["path"]));
                        let mut chosen = current;
                        RadioGroup::new(label(name), Bind::bound(&mut chosen)).show(ui, |ui, _| {
                            for (at, f) in files.iter().enumerate() {
                                let file = f["path"].as_str().unwrap_or_default();
                                let p = std::path::Path::new(file);
                                let file_name = p.file_name().map_or_else(|| file.to_string(), |n| n.to_string_lossy().into_owned());
                                Row::new().show(ui, |ui, _| {
                                    Radio::new(Some(at)).label(label(&file_name)).show(ui);
                                    match f["match"].as_str() {
                                        Some("same") => Badge::new(label(tl!("Same file"))).show(ui),
                                        Some("size") => Badge::new(label(tl!("Same size"))).show(ui),
                                        _ => return,
                                    };
                                });
                                let folder = p.parent().map_or_else(String::new, |f| f.to_string_lossy().into_owned());
                                Text::new(label(&folder)).shade(Shade::Muted).elide(Elide::Path).tooltip(file.to_string()).show(ui);
                            }
                            if !files.is_empty() {
                                Radio::new(None::<usize>).label(label(tl!("Don't Relink"))).show(ui);
                            }
                        });
                        if chosen != current
                            && let Some(slot) = edit.picks.get_mut(i)
                        {
                            *slot = chosen.and_then(|at| files.get(at)).map_or(Value::Null, |f| f["path"].clone());
                            if !edit.touched.contains(&json!(i)) {
                                edit.touched.push(json!(i));
                            }
                        }
                        // A failed search (a folder it refuses) didn't look: `progress` says why.
                        if files.is_empty() && !state.is_empty() && state != "failed" {
                            let line = if searching { tl!("Searching…") } else { tl!("Not found") };
                            Text::new(label(line)).shade(Shade::Muted).show(ui);
                        }
                    }
                });
                Row::new().show(ui, |ui, _| {
                    if can_pick && Button::new(label(tl!("Find in Folder…"))).enabled(!searching).show(ui).clicked() {
                        pick = true;
                    }
                    if searching {
                        stop = Button::new(label(tl!("Stop"))).show(ui).clicked();
                        Spinner::new().show(ui);
                    }
                });
                for line in &progress {
                    Text::new(line.clone()).shade(Shade::Muted).show(ui);
                }
                if let Some(folder) = &copy_folder {
                    Checkbox::new(Bind::bound(&mut edit.copy)).label(label(tl!("Copy into the document's folder"))).tooltip(folder.clone()).show(ui);
                }
                true
            });
        self.fields.insert("picks".into(), json!(edit.picks));
        self.fields.insert("touched".into(), json!(edit.touched));
        self.fields.insert("copy".into(), json!(edit.copy));
        if pick {
            self.asked.file(&ctx, PickRequest::folder("links"), ());
        }
        if stop {
            self.request = Some(Request::Stop);
        }
        match (shown.accepted, shown.closed) {
            (Some(_), _) => Some(DialogResult::Ok(self.fields.clone())),
            (None, Some(_)) => {
                self.asked.forget(&ctx);
                Some(DialogResult::Cancel)
            }
            (None, None) => None,
        }
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<Fields> {
        if !self.search_folder.is_empty() {
            self.request = Some(Request::Search(std::mem::take(&mut self.search_folder)));
            return None;
        }
        Some(self.fields.clone())
    }

    fn cancelled(&mut self, ctx: &egui::Context) {
        self.asked.forget(ctx);
    }

    fn json_fields(&self) -> Option<Value> {
        Some(Value::Object(self.fields.clone()))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "searchFolder" => self.search_folder = text_field(field, &value)?,
            "copy" => {
                let copy = flag_field(field, &value)?;
                self.fields.insert("copy".into(), json!(copy));
            }
            "picks" => self.set_picks(&value)?,
            _ => return Err(format!("unknown field `{field}`")),
        }
        ctx.request_repaint();
        Ok(())
    }
}

impl FindLinksDialog {
    /// Sets the file chosen for each row (`picks`, one path or null per row) as the user choosing
    /// them would: a row whose file changes is one the next search results leave as it is.
    fn set_picks(&mut self, value: &Value) -> Result<(), String> {
        let rows_given = value.as_array().ok_or_else(|| format!("field `picks` takes a list, got {value}"))?;
        let missing = rows(&self.fields, "missing").len();
        if rows_given.len() != missing || rows_given.iter().any(|p| !p.is_null() && !p.is_string()) {
            return Err(format!("field `picks` takes {missing} paths or nulls, got {value}"));
        }
        let before = rows(&self.fields, "picks");
        let mut touched = rows(&self.fields, "touched");
        for (i, pick) in rows_given.iter().enumerate() {
            if before.get(i) != Some(pick) && !touched.contains(&json!(i)) {
                touched.push(json!(i));
            }
        }
        self.fields.insert("picks".into(), json!(rows_given));
        self.fields.insert("touched".into(), json!(touched));
        Ok(())
    }
}

/// The fields of a Find Linked Files dialog for the `missing` files of the active document.
fn dialog(app: &VectorcraftApp, missing: Vec<Value>, modified: Vec<Value>, back: bool) -> Result<Fields, String> {
    let uid = app.session.active().map(|d| d.uid).ok_or("no document open")?;
    let picks = vec![Value::Null; missing.len()];
    let fields =
        json!({ "document": uid, "missing": missing, "modified": modified, "back": back, "state": "", "picks": picks, "touched": [], "copy": false });
    match fields {
        Value::Object(fields) => Ok(fields),
        _ => Err("the dialog has no fields".into()),
    }
}

/// Shows the dialog over `fields`.
fn open(app: &mut VectorcraftApp, fields: Fields) -> Result<(), String> {
    let dialog = FindLinksDialog::new(app, fields);
    let dialogs = app.dialogs().ok_or("no window to show the dialog in")?;
    dialogs.with(|h| h.open(dialog));
    Ok(())
}

/// Linked File Not Found's Find in Folder…: Find Linked Files opens, searching `folder` for every
/// file still asked about (`missing`). The files left without a choice and those that fail to
/// relink go back to Linked File Not Found, with `modified` and the open's `notes`.
pub(super) fn from_missing_links(
    app: &mut VectorcraftApp,
    missing: Vec<Value>,
    modified: Vec<Value>,
    notes: Vec<String>,
    folder: &str,
) -> Result<(), String> {
    let mut find = dialog(app, missing.clone(), modified.clone(), true)?;
    let document = find["document"].as_u64().ok_or("no document open")?;
    find.insert("notes".into(), json!(notes));
    start_search(app, &mut find, folder)?;
    open(app, find)?;
    app.link_asks = Some(LinkAsk { document, missing, modified, notes, back: true });
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

/// The linked files of objects `ids` (all of them when given, else the files that are missing) as
/// the rows the dialog looks for. Nothing to look for is an error.
fn linked_files(app: &mut VectorcraftApp, ids: Option<&[Value]>) -> Result<Vec<Value>, String> {
    let all = ids.is_some();
    let r = app.session.execute("links.check", &json!({ "ids": ids })).map_err(|e| e.to_string())?;
    let missing: Vec<Value> = r["links"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| all || l["status"] == "missing")
        .map(|l| json!({ "name": l["name"], "path": l["path"], "ids": l["ids"] }))
        .collect();
    if missing.is_empty() {
        return Err("No linked files to look for".into());
    }
    Ok(missing)
}

/// `ui.findLinksInFolder` (the Links panel's Find in Folder…): ask for a folder (or take `folder`),
/// then open Find Linked Files searching it for the files of linked objects `ids` (default: the
/// selected linked images and placed documents, else the files that are missing).
pub(crate) fn find_in_folder_command(app: &mut VectorcraftApp, folder: Option<String>, ids: Option<Vec<Value>>) -> Result<Value, String> {
    let ids = ids.filter(|ids| !ids.is_empty()).or_else(|| Some(selected_linked(app)).filter(|ids| !ids.is_empty()));
    // Nothing to look for: no folder is asked for.
    linked_files(app, ids.as_deref())?;
    match folder {
        Some(folder) => find_in(app, ids, &folder),
        // PLAN: assumed `file_asks::Owner::Document` and `ask(app, request, owner, then) -> Value` (fl.A3, files-w5).
        None => Ok(file_asks::ask(app, PickRequest::folder("links"), Owner::Document, move |app, picked| {
            let folder = picked.first_path().ok_or("cancelled")?;
            find_in(app, ids, &folder.to_string_lossy())
        })),
    }
}

/// Opens Find Linked Files searching `folder` for the files of `ids`.
fn find_in(app: &mut VectorcraftApp, ids: Option<Vec<Value>>, folder: &str) -> Result<Value, String> {
    let missing = linked_files(app, ids.as_deref())?;
    let mut d = dialog(app, missing, vec![], false)?;
    start_search(app, &mut d, folder)?;
    open(app, d)?;
    Ok(Value::Null)
}

/// Whether object `id` of the active document is an image (and not a placed document), in the
/// layers or in opacity-mask art or the art of a symbol, graph design or pattern.
fn is_image(app: &VectorcraftApp, id: &Value) -> bool {
    let Some(id) = id.as_u64() else { return false };
    let st = app.session.active();
    st.and_then(|st| st.doc.stored_node(vectorcraft_doc::NodeId(id))).is_some_and(|(n, _)| matches!(n.kind, vectorcraft_doc::NodeKind::Image(_)))
}

/// Make the dialog's document the active one: it may be another tab's.
fn activate(app: &mut VectorcraftApp, d: &Fields) -> Result<(), String> {
    let uid = d.get("document").and_then(Value::as_u64);
    if app.session.active().map(|st| st.uid) == uid {
        return Ok(());
    }
    let index = app.session.documents().iter().position(|doc| Some(doc.uid) == uid).ok_or("that document is no longer open")?;
    app.run("document.activate", json!({ "index": index })).map(|_| ())
}

/// Start looking in `folder` for the files of the dialog's rows; what an earlier search found, and
/// the files chosen from it, are dropped.
fn start_search(app: &mut VectorcraftApp, d: &mut Fields, folder: &str) -> Result<(), String> {
    activate(app, d)?;
    let r = app.run("links.findFiles", json!({ "folder": folder, "ids": ids(&rows(d, "missing")) }))?;
    for k in ["found", "searched", "skipped", "unreadable", "stopped", "error"] {
        d.remove(k);
    }
    d.insert("picks".into(), json!(vec![Value::Null; rows(d, "missing").len()]));
    d.insert("touched".into(), json!([]));
    d.insert("search".into(), r["id"].clone());
    d.insert("folder".into(), json!(folder));
    update(d, &r);
    app.ui.links_search = r["id"].as_u64();
    Ok(())
}

/// The files a search found for `row` (`{path?, ids}`), best first: the answer's `links` row with
/// the same ids and, when both have one, the same path. A row has no `path` when its file is outside
/// an automation client's read roots.
fn files_for<'a>(found: &'a [Value], row: &Value) -> &'a [Value] {
    let same_path = |r: &Value| r["path"].is_null() || row["path"].is_null() || r["path"] == row["path"];
    found.iter().find(|r| r["ids"] == row["ids"] && same_path(r)).and_then(|r| r["files"].as_array()).map_or(&[], Vec::as_slice)
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
fn update(d: &mut Fields, r: &Value) {
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
    d.insert("picks".into(), json!(picks));
    d.insert("found".into(), r["links"].clone());
    for k in ["state", "searched", "skipped", "unreadable", "stopped", "error"] {
        match r.get(k) {
            Some(v) => d.insert(k.into(), v.clone()),
            None => d.remove(k),
        };
    }
}

/// While searching: the search's progress into the dialog. A search started elsewhere since
/// stopped this one.
fn follow(app: &mut VectorcraftApp, d: &mut Fields) {
    // Polled through `session.execute`.
    let Ok(r) = app.session.execute("links.findFiles", &json!({})) else { return };
    if d.get("search") != Some(&r["id"]) {
        d.insert("state".into(), json!("stopped"));
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
    let open = app.dialogs().is_some_and(|d| d.with(|h| h.get::<FindLinksDialog>().is_some_and(|d| d.search() == Some(id))));
    if !open {
        app.ui.links_search = None;
        vectorcraft_engine::cmd::linkfiles::stop(&mut app.session, id);
    }
}

/// The folder of the dialog's document, when it was saved.
fn document_folder(app: &VectorcraftApp, d: &Fields) -> Option<String> {
    let uid = d.get("document").and_then(Value::as_u64)?;
    let path = app.session.documents().iter().find(|doc| doc.uid == uid)?.path.clone()?;
    std::path::Path::new(&path).parent().map(|p| p.to_string_lossy().into_owned())
}

/// Each frame: stops a search whose dialog is gone, takes the answer of the dialog that closed
/// (Cancel shows the open's notes again when Linked File Not Found opened it; Relink relinks), and
/// does what the open dialog asked of the app: start or stop its search, and follow it while it runs.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    stop_when_closed(app);
    let Some(dialogs) = app.dialogs() else { return };
    if let Some(result) = dialogs.with(|h| h.take::<FindLinksDialog>()) {
        let ask = app.link_asks.take_if(|ask| ask.back);
        match (result.ok(), ask) {
            (Some(fields), _) => {
                if let Err(e) = relink(app, &fields) {
                    app.status(e);
                }
            }
            (None, Some(ask)) => show_notes_again(app, &ask),
            (None, None) => {}
        }
        return;
    }
    let Some((mut fields, request)) = dialogs.with(|h| h.get_mut::<FindLinksDialog>().map(|d| (d.fields.clone(), d.request.take()))) else {
        return;
    };
    if request.is_none() && text(&fields, "state") != "searching" {
        return;
    }
    let before = fields.clone();
    let done = match request {
        Some(Request::Search(folder)) => start_search(app, &mut fields, &folder),
        Some(Request::Stop) => app.run("links.findFiles", json!({ "stop": true })).map(|_| follow(app, &mut fields)),
        None => {
            follow(app, &mut fields);
            Ok(())
        }
    };
    if let Err(e) = done {
        app.status(e);
    }
    if text(&fields, "state") == "searching" {
        ctx.request_repaint_after(POLL);
    }
    if fields != before {
        dialogs.with(|h| {
            if let Some(d) = h.get_mut::<FindLinksDialog>() {
                d.fields = fields;
            }
        });
    }
}

/// Relink: each row's file, one `links.relink` each (copied into the document's folder with
/// `copy`). The status line names the first file that fails to relink, and the other files still
/// relink. From Linked File Not Found, the rows left without a file and the rows that failed to
/// relink go back to it, then the modified files are offered for update.
fn relink(app: &mut VectorcraftApp, d: &Fields) -> Result<Value, String> {
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
        if flag(d, "copy") && row["ids"].as_array().is_some_and(|ids| ids.iter().all(|id| is_image(app, id))) {
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
    let n = relinked.len();
    match failed.first() {
        Some(f) => {
            let (name, reason) = (f["name"].as_str().unwrap_or_default(), f["error"].as_str().unwrap_or_default());
            app.status(crate::i18n::msg_n(
                n as u64,
                "Relinked {n} image; {name} couldn't be relinked: {reason}",
                "Relinked {n} images; {name} couldn't be relinked: {reason}",
                &[("name", name), ("reason", reason)],
            ));
        }
        None if n > 0 => app.status(crate::panels::links::relinked_status(n)),
        None => {}
    }
    if flag(d, "back") {
        super::missing_links::ask_next(app, rest, rows(d, "modified"), notes(d));
    }
    Ok(json!({ "relinked": relinked, "failed": failed }))
}

#[cfg(test)]
mod tests;
