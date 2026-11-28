//! The fonts a document opened without (`text.missingFonts`), listed once the missing linked file
//! questions are answered, one document at a time ([`open_next`]), with Find Fonts (Type › Find Font
//! on the first of them, to replace it) and Close. Neither brings the dialog back for that open.
//! Search Fonts Folder (the `Fonts` folder next to the document, as File › Package writes it) and
//! Find in Folder… look for their files (`text.findFontFiles`, on separate threads), and Add Fonts
//! copies the files the user chooses into VectorCraft's own Fonts folder (`text.addFontFiles`).
//! Type › Find Font's Find in Folder… opens it too (`ui.findFontsInFolder`), and so does
//! `ui.missingFontsDialog`.
//!
//! Fields: `document` (the document's uid), `fonts` (`text.missingFonts`'s rows),
//! `fontsNextToDocument` (or empty), `state` (empty, `searching`, `done`, `stopped` or `failed`),
//! `search` (the search's `id`), `folder`, `found` (`text.findFontFiles`'s `fonts`, with the files
//! found), `searched`, `skipped`, `unreadable`, `stopped`, `error`, `chosen` (the files to add; a
//! font's first file is chosen when it is found), `searchFolder` (set it and confirm to search that
//! folder) and `discard` (set it and confirm for Find Fonts; confirming without either adds the
//! chosen files).
//!
//! The dialog has no app, so what needs one (a search, Find Font, adding the files) is the answer
//! it finishes with; [`open_next`] runs it, and opens the dialog again when it stays open.
// PLAN: assumed `crate::file_asks::Picked::first_path(&self) -> Option<PathBuf>` and `Owner::Document` (files-w5).

use std::time::Duration;

use craft::host::{PickKind, PickRequest};
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _, clicked};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::Label;
use craft::ui::sections::{DialogRole, FooterButton};
use egui::{Context, Ui};
use serde_json::{Map, Value, json};

use crate::VectorcraftApp;
use crate::file_asks::{self, Owner};

/// The dialog's key and kind: the fonts a document opened without.
pub const KIND: &str = "missingFonts";

/// The error of a document that misses no font.
const NOTHING_MISSING: &str = "No fonts are missing in this document";

/// How often a running search is looked at.
const POLL: Duration = Duration::from_millis(100);

/// The keys of a search's answer the dialog keeps under `state`.
const PROGRESS_KEYS: [&str; 6] = ["state", "searched", "skipped", "unreadable", "stopped", "error"];

/// What the dialog shows and edits.
#[derive(Clone)]
struct State {
    /// The uid of the document the dialog is about.
    uid: u64,
    /// `text.missingFonts`'s rows.
    fonts: Vec<Value>,
    /// The `Fonts` folder next to the document, or empty.
    next: String,
    /// Whether a folder can be picked here (`file_asks::can`): a property of the build target, so it is read once.
    can_pick: bool,
    /// The id of the search the dialog started.
    search: Option<u64>,
    /// `text.findFontFiles`'s `fonts`, with the files found.
    found: Vec<Value>,
    /// The files to add.
    chosen: Vec<Value>,
    /// The search's `folder` and the keys of [`PROGRESS_KEYS`] its answers set.
    status: Map<String, Value>,
    /// A folder automation asked to search.
    search_folder: String,
    /// Automation asked for Find Fonts.
    discard: bool,
}

impl State {
    fn new(uid: u64, fonts: Vec<Value>, next: String, can_pick: bool) -> Self {
        State {
            uid,
            fonts,
            next,
            can_pick,
            search: None,
            found: Vec::new(),
            chosen: Vec::new(),
            status: Map::new(),
            search_folder: String::new(),
            discard: false,
        }
    }

    /// The search's state: empty, `searching`, `done`, `stopped` or `failed`.
    fn name(&self) -> &str {
        self.status.get("state").and_then(Value::as_str).unwrap_or_default()
    }

    fn searching(&self) -> bool {
        self.name() == "searching"
    }

    /// Can Add Fonts run: files are chosen and no search is still finding more?
    fn can_add(&self) -> bool {
        !self.searching() && !self.chosen.is_empty()
    }

    /// The fields automation sees.
    fn json(&self) -> Value {
        let mut f = json!({
            "document": self.uid,
            "fonts": self.fonts,
            "fontsNextToDocument": self.next,
            "state": "",
            "chosen": self.chosen,
        });
        if let Some(id) = self.search {
            f["search"] = json!(id);
            f["found"] = json!(self.found);
        }
        for (k, v) in &self.status {
            f[k.as_str()] = v.clone();
        }
        if !self.search_folder.is_empty() {
            f["searchFolder"] = json!(self.search_folder);
        }
        if self.discard {
            f["discard"] = json!(true);
        }
        f
    }

    /// Set the field automation names: `searchFolder`, `discard` or `chosen`; the others are read only.
    fn set(&mut self, field: &str, value: Value) -> Result<(), String> {
        match (field, &value) {
            ("searchFolder", Value::String(folder)) => self.search_folder = folder.clone(),
            ("discard", Value::Bool(on)) => self.discard = *on,
            ("chosen", Value::Array(files)) => self.chosen = files.clone(),
            ("searchFolder" | "discard" | "chosen", _) => return Err(FieldRefusal::Rejects(&value).message(field)),
            _ if self.json().get(field).is_some() => return Err(FieldRefusal::ReadOnly.message(field)),
            _ => return Err(FieldRefusal::Unknown.message(field)),
        }
        Ok(())
    }
}

/// What the user chose, which needs the app.
#[derive(Clone, Copy)]
enum Act {
    /// Copy the chosen files into the Fonts folder and close.
    Add,
    /// Type › Find Font on the first missing font; the dialog closes.
    Find,
    /// Search `search_folder`, and stay open.
    Search,
    /// Stop the running search, and stay open.
    Stop,
    /// Ask for a folder to search: the dialog closes, and comes back searching it.
    Pick,
}

/// How the dialog finished: what to do, and the state to open the dialog again with.
pub(crate) struct Answer {
    act: Act,
    state: State,
}

/// The Missing Fonts dialog on the host.
pub(crate) struct MissingFontsDialog {
    id: DialogId,
    state: State,
}

impl MissingFontsDialog {
    fn new(state: State) -> Self {
        MissingFontsDialog { id: dialog_id(KIND), state }
    }
}

impl StackDialog for MissingFontsDialog {
    type Result = Answer;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    /// Find Fonts (`discard`): Type › Find Font on the first font the document misses, to replace it;
    /// this Missing Fonts dialog doesn't come back. Otherwise (agents), with `searchFolder`: search
    /// that folder, and stay open; else Add Fonts, which needs chosen files.
    fn confirm(&mut self, _: &Context) -> Option<Answer> {
        let state = &self.state;
        let act = if state.discard {
            Act::Find
        } else if !state.search_folder.is_empty() {
            Act::Search
        } else if state.can_add() {
            Act::Add
        } else {
            return None;
        };
        Some(Answer { act, state: state.clone() })
    }

    fn json_fields(&self) -> Option<Value> {
        Some(self.state.json())
    }

    fn set_json_field(&mut self, _: &Context, field: &str, value: Value) -> Result<(), String> {
        self.state.set(field, value)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Answer>> {
        let searching = self.state.searching();
        let can_add = self.state.can_add();
        let state_name = self.state.name().to_string();
        let lines = progress(&Value::Object(self.state.status.clone()));
        let any_found = self.state.found.iter().any(|f| f["files"].as_array().is_some_and(|files| !files.is_empty()));
        let mut act = None;
        let MissingFontsDialog { state, .. } = &mut *self;
        let State { fonts, next, found, chosen, can_pick, search_folder, .. } = state;
        let shown = FormDialog::new(tl!("Missing Fonts"), Bind::bound(chosen))
            .ok_label(tl!("Add Fonts"))
            .cancel_label(tl!("Close"))
            .ok_enabled(can_add)
            .extra_buttons([FooterButton::new(tl!("Find Fonts"), DialogRole::Other)])
            .show(ui, |ui, _, chosen| {
                Stack::new().show(ui, |ui, _| {
                    Text::new(tl!("These fonts aren't available. Their text shows in another font.")).shade(Shade::Muted).show(ui);
                    for f in fonts.iter() {
                        let (note, tone) = crate::find_font::font_note(f);
                        let name = format!("{} {}{note}", f["family"].as_str().unwrap_or_default(), f["style"].as_str().unwrap_or_default());
                        Text::new(name).tone(tone).show(ui);
                        let files = files_for(found, f);
                        for file in files {
                            let path = file.as_str().unwrap_or_default();
                            let label = std::path::Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned());
                            let was = chosen.contains(file);
                            let mut on = was;
                            Checkbox::new(Bind::bound(&mut on)).label(label).id_salt(path.to_string()).show(ui);
                            if on != was {
                                if on {
                                    chosen.push(file.clone());
                                } else {
                                    chosen.retain(|c| c != file);
                                }
                            }
                            Text::new(path.to_string()).shade(Shade::Muted).show(ui);
                        }
                        // A failed search (a folder it refuses) didn't look: the progress lines say why.
                        if files.is_empty() && !state_name.is_empty() && state_name != "failed" {
                            let note = if searching { tl!("Searching…") } else { tl!("Not found") };
                            Text::new(note).shade(Shade::Muted).show(ui);
                        }
                    }
                    Row::new().show(ui, |ui, _| {
                        if !next.is_empty()
                            && clicked(&Button::new(tl!("Search Fonts Folder")).id_salt("next").enabled(!searching).tooltip(next.clone()).show(ui))
                        {
                            *search_folder = next.clone();
                            act = Some(Act::Search);
                        }
                        // The app asks for the folder, as Type › Find Font's Find in Folder… does.
                        if *can_pick && clicked(&Button::new(tl!("Find in Folder…")).id_salt("pick").enabled(!searching).show(ui)) {
                            act = Some(Act::Pick);
                        }
                        if searching && clicked(&Button::new(tl!("Stop")).id_salt("stop").show(ui)) {
                            act = Some(Act::Stop);
                        }
                    });
                    for line in &lines {
                        Text::new(line.clone()).shade(Shade::Muted).show(ui);
                    }
                    // Once files are found: what copying them means.
                    if any_found {
                        Text::new(tl!("Copy only fonts you own or are licensed to install.")).shade(Shade::Muted).show(ui);
                        if let Some(dir) = vectorcraft_text::app_font_dir() {
                            let text = crate::i18n::fmt(tl!("VectorCraft copies the fonts to {folder}."), &[("folder", &dir.to_string_lossy())]);
                            Text::new(text).shade(Shade::Muted).show(ui);
                        }
                    }
                });
                true
            });
        // Footer buttons: OK is Add Fonts; the extra button follows Reset/Cancel.
        if shown.accepted.is_some() {
            act = Some(Act::Add);
        } else if shown.picked == Some(2) {
            act = Some(Act::Find);
        }
        let closed = act.is_none() && shown.closed.is_some();
        match act {
            Some(act) => Some(DialogResult::Ok(Answer { act, state: self.state.clone() })),
            None if closed => Some(DialogResult::Cancel),
            None => None,
        }
    }
}

/// After a document opened (`document.open`'s result `r`): its Missing Fonts dialog waits for no other
/// dialog to be open ([`open_next`]). On the web the status line shows how many fonts are missing
/// instead, unless it shows the open's notes.
pub fn after_open(app: &mut VectorcraftApp, r: &Value) {
    after_open_for(app, r, cfg!(target_arch = "wasm32"));
}

fn after_open_for(app: &mut VectorcraftApp, r: &Value, web: bool) {
    let Some(st) = app.session.active() else { return };
    let uid = st.uid;
    if web {
        let n = vectorcraft_engine::cmd::fontfiles::missing_fonts(&st.doc).len();
        if n > 0 && r["warnings"].as_array().is_none_or(Vec::is_empty) {
            app.status(crate::i18n::msg_n(
                n as u64,
                "{n} font isn't available here: its text shows in another font",
                "{n} fonts aren't available here: their text shows in another font",
                &[],
            ));
        }
        return;
    }
    if !app.ui.pending_fonts.contains(&uid) {
        app.ui.pending_fonts.push(uid);
    }
}

/// Whether an opened document's Missing Fonts dialog waits for the frame's pump to open it.
pub(crate) fn waiting(app: &VectorcraftApp) -> bool {
    app.ui.pending_fonts.iter().any(|&uid| dialog_for(app, uid).is_some())
}

/// The Missing Fonts dialog of the open document `uid`, when fonts it uses aren't available.
fn dialog_for(app: &VectorcraftApp, uid: u64) -> Option<MissingFontsDialog> {
    let st = app.session.documents().iter().find(|d| d.uid == uid)?;
    let r = vectorcraft_engine::cmd::fontfiles::missing_fonts_of(st, app.session.search_rules.as_ref());
    let fonts = r["fonts"].as_array().filter(|f| !f.is_empty())?.clone();
    let next = r["fontsNextToDocument"].as_str().unwrap_or_default().to_string();
    Some(MissingFontsDialog::new(State::new(uid, fonts, next, file_asks::can(app, PickKind::Folder))))
}

/// Once per frame: run what the finished dialog left to do, follow a running search, stop the search
/// of a dialog that is gone, and with no dialog open open the dialog of the first queued document
/// that is still open and still misses fonts.
pub(crate) fn open_next(app: &mut VectorcraftApp) {
    let answered = app.dialogs().and_then(|d| d.with(|h| h.take::<MissingFontsDialog>()));
    if let Some(answer) = answered.and_then(DialogResult::ok) {
        settle(app, answer);
    }
    follow_open(app);
    stop_when_closed(app);
    while !app.any_dialog_open() && !app.ui.pending_fonts.is_empty() {
        let uid = app.ui.pending_fonts.remove(0);
        let Some(dialog) = dialog_for(app, uid) else { continue };
        let Some(dialogs) = app.dialogs() else { return };
        // One dialog at a time: a document that asks while another's is open waits its turn.
        if dialogs.with(|h| h.open_unless_open(dialog).is_err()) {
            queue_first(app, uid);
        }
        break;
    }
}

/// Run what `answer` asks, and open the dialog again when it stays open (a search starts, a search
/// stops, or the action failed: the status bar says why).
fn settle(app: &mut VectorcraftApp, Answer { act, mut state }: Answer) {
    let search_folder = std::mem::take(&mut state.search_folder);
    state.discard = false;
    let done = match act {
        Act::Add => add_fonts(app, &state.chosen).map(|_| false),
        Act::Find => find_fonts(app, state.uid).map(|()| false),
        // The folder is searched once: one that doesn't start leaves the dialog without it.
        Act::Search => start_search(app, &mut state, &search_folder).map(|()| true),
        // The app asks for the folder, as Type › Find Font's Find in Folder… does; the dialog comes back
        // searching it. A question that is cancelled leaves the dialog closed.
        Act::Pick => find_in_folder_for(app, state.uid, None).map(|_| false),
        Act::Stop => app.run("text.findFontFiles", json!({ "stop": true })).map(|_| {
            if let Ok(r) = app.session.execute("text.findFontFiles", &json!({})) {
                let error = follow(&mut state, &r);
                if let Some(e) = error {
                    app.status(e);
                }
            }
            true
        }),
    };
    let reopen = done.unwrap_or_else(|e| {
        app.status(e);
        true
    });
    if reopen && let Some(dialogs) = app.dialogs() {
        dialogs.with(|h| h.open(MissingFontsDialog::new(state)));
    }
}

/// Put the document `uid` at the front of the queue.
fn queue_first(app: &mut VectorcraftApp, uid: u64) {
    app.ui.pending_fonts.retain(|u| *u != uid);
    app.ui.pending_fonts.insert(0, uid);
}

/// While the open dialog's search runs: its progress into the dialog, and a frame again soon. A search
/// started elsewhere since stopped this one.
fn follow_open(app: &mut VectorcraftApp) {
    let Some(dialogs) = app.dialogs() else { return };
    if !dialogs.with(|h| h.get::<MissingFontsDialog>().is_some_and(|d| d.state.searching())) {
        return;
    }
    if let Some(ctx) = &app.ui_ctx {
        ctx.request_repaint_after(POLL);
    }
    // Polled through `session.execute`.
    let Ok(r) = app.session.execute("text.findFontFiles", &json!({})) else { return };
    let error = dialogs.with(|h| h.get_mut::<MissingFontsDialog>().and_then(|d| follow(&mut d.state, &r)));
    if let Some(e) = error {
        app.status(e);
    }
}

/// A search's answer `r` into `state`; the search's error, when it has one, to show.
fn follow(state: &mut State, r: &Value) -> Option<String> {
    if state.search != r["id"].as_u64() {
        state.status.insert("state".into(), json!("stopped"));
        return None;
    }
    update(state, r);
    r["error"].as_str().map(str::to_string)
}

/// Stop the search the dialog started once the dialog is gone.
fn stop_when_closed(app: &mut VectorcraftApp) {
    let Some(id) = app.ui.dialog_search else { return };
    let open = app.dialogs().is_some_and(|d| d.with(|h| h.get::<MissingFontsDialog>().is_some_and(|d| d.state.search == Some(id))));
    if !open {
        app.ui.dialog_search = None;
        vectorcraft_engine::cmd::fontfiles::stop(&mut app.session, id);
    }
}

/// `ui.missingFontsDialog`: the dialog for the active document; with `folder`, searching it.
pub(crate) fn open_command(app: &mut VectorcraftApp, folder: Option<String>) -> Result<Value, String> {
    let uid = app.session.active().map(|d| d.uid).ok_or("no document open")?;
    match folder {
        Some(folder) => open_searching(app, uid, &folder),
        None => {
            let dialog = dialog_for(app, uid).ok_or(NOTHING_MISSING)?;
            show_for(app, uid, dialog);
            Ok(Value::Null)
        }
    }
}

/// The dialog of the document `uid`, searching `folder`.
fn open_searching(app: &mut VectorcraftApp, uid: u64, folder: &str) -> Result<Value, String> {
    let mut dialog = dialog_for(app, uid).ok_or(NOTHING_MISSING)?;
    start_search(app, &mut dialog.state, folder)?;
    show_for(app, uid, dialog);
    Ok(Value::Null)
}

/// Show `dialog`, the dialog of the document `uid`, unless one is open: another document's Missing
/// Fonts dialog asks first, and this one waits at the front of the queue.
fn show_for(app: &mut VectorcraftApp, uid: u64, dialog: MissingFontsDialog) {
    app.ui.pending_fonts.retain(|u| *u != uid);
    let Some(dialogs) = app.dialogs() else { return };
    if dialogs.with(|h| h.open_unless_open(dialog).is_err()) {
        queue_first(app, uid);
    }
}

/// `ui.findFontsInFolder` (Type › Find Font's Find in Folder…): ask for a folder (or take
/// `folder`), then open the dialog for the active document, searching it.
pub(crate) fn find_in_folder_command(app: &mut VectorcraftApp, folder: Option<String>) -> Result<Value, String> {
    let uid = app.session.active().map(|d| d.uid).ok_or("no document open")?;
    find_in_folder_for(app, uid, folder)
}

/// [`find_in_folder_command`] for the document `uid`, which a dialog left open may not be the
/// active one.
fn find_in_folder_for(app: &mut VectorcraftApp, uid: u64, folder: Option<String>) -> Result<Value, String> {
    // Nothing to look for: no folder is asked for.
    dialog_for(app, uid).ok_or(NOTHING_MISSING)?;
    match folder {
        Some(folder) => open_searching(app, uid, &folder),
        None => Ok(file_asks::ask(app, PickRequest::folder("fonts"), Owner::Document, move |app, picked| {
            let folder = picked.first_path().ok_or("cancelled")?.to_string_lossy().into_owned();
            open_searching(app, uid, &folder)
        })),
    }
}

/// Start looking in `folder` for the files of the dialog's fonts; what an earlier search found is
/// dropped.
fn start_search(app: &mut VectorcraftApp, state: &mut State, folder: &str) -> Result<(), String> {
    let fonts: Vec<Value> = state.fonts.iter().map(|f| json!({ "family": f["family"], "style": f["style"] })).collect();
    let r = app.run("text.findFontFiles", json!({ "folder": folder, "fonts": fonts }))?;
    for k in ["found", "searched", "skipped", "unreadable", "stopped", "error"] {
        state.status.remove(k);
    }
    state.found.clear();
    state.chosen.clear();
    state.search = r["id"].as_u64();
    state.status.insert("folder".into(), json!(folder));
    update(state, &r);
    app.ui.dialog_search = state.search;
    Ok(())
}

/// The files a search found for the font `f` (`{family, style}`): the answer's `fonts` row with
/// the same family and style.
fn files_for<'a>(found: &'a [Value], f: &Value) -> &'a [Value] {
    found.iter().find(|r| r["family"] == f["family"] && r["style"] == f["style"]).and_then(|r| r["files"].as_array()).map_or(&[], Vec::as_slice)
}

/// A search's answer `r` into the dialog. A font that gets its first file has that file chosen; a
/// file the user unchecked stays unchecked.
fn update(state: &mut State, r: &Value) {
    for font in r["fonts"].as_array().into_iter().flatten() {
        let had = !files_for(&state.found, font).is_empty();
        if let Some(first) = font["files"].as_array().and_then(|f| f.first()).filter(|f| !had && !state.chosen.contains(f)) {
            state.chosen.push(first.clone());
        }
    }
    state.found = r["fonts"].as_array().cloned().unwrap_or_default();
    for k in PROGRESS_KEYS {
        match r.get(k) {
            Some(v) => state.status.insert(k.into(), v.clone()),
            None => state.status.remove(k),
        };
    }
}

/// A folder search's progress under the dialog's buttons, from its `state` object (keys `state`,
/// `folder`, `searched`, `skipped`, `unreadable`, `stopped` and `error`), in Find Linked Files too.
pub(crate) fn progress(state: &Value) -> Vec<Label> {
    let mut lines: Vec<Label> = Vec::new();
    if state["state"] == "searching" {
        lines.push(state["folder"].as_str().unwrap_or_default().to_string().into());
    }
    if let Some(e) = state.get("error").and_then(Value::as_str) {
        lines.push(crate::i18n::message(crate::i18n::current(), e).into());
    }
    if let Some(s) = state.get("searched") {
        let folders = crate::i18n::tn(s["folders"].as_u64().unwrap_or(0), "{n} folder", "{n} folders");
        let files = crate::i18n::tn(s["files"].as_u64().unwrap_or(0), "{n} file", "{n} files");
        lines.push(crate::i18n::fmt(tl!("{folders} and {files} searched"), &[("folders", &folders), ("files", &files)]).into());
    }
    let count = |k: &str| state.get(k).and_then(Value::as_u64).filter(|n| *n > 0);
    if let Some(n) = count("skipped") {
        lines.push(
            crate::i18n::tn(
                n,
                "{n} folder skipped, such as another app's or the system's folder or a hidden one",
                "{n} folders skipped, such as other apps' and the system's folders and hidden ones",
            )
            .into(),
        );
    }
    if let Some(n) = count("unreadable") {
        lines.push(crate::i18n::tn(n, "{n} folder or file couldn't be read", "{n} folders or files couldn't be read").into());
    }
    if matches!(state.get("stopped").and_then(Value::as_str), Some("time" | "limit")) {
        lines.push(tl!("The search stopped at its limit: search a smaller folder.").into());
    }
    lines
}

/// Find Fonts: Type › Find Font on the first font the document misses, to replace it. Find Font
/// works on the active document; the dialog's may be another tab.
fn find_fonts(app: &mut VectorcraftApp, uid: u64) -> Result<(), String> {
    let index = app.session.documents().iter().position(|doc| doc.uid == uid).ok_or("that document is no longer open")?;
    app.run("document.activate", json!({ "index": index }))?;
    crate::find_font::open_on_missing(app);
    Ok(())
}

/// Add Fonts: copy the chosen files into VectorCraft's Fonts folder.
fn add_fonts(app: &mut VectorcraftApp, chosen: &[Value]) -> Result<Value, String> {
    if chosen.is_empty() {
        return Err("choose the font files to add".into());
    }
    let r = app.run("text.addFontFiles", json!({ "files": chosen }))?;
    let len = |k: &str| r[k].as_array().map_or(0, Vec::len);
    let n = len("copied") + len("kept");
    let folder = r["folder"].as_str().unwrap_or_default();
    match r["skipped"].as_array().and_then(|s| s.first()) {
        None => {
            app.status(crate::i18n::msg_n(n as u64, "{n} font file is now in {folder}", "{n} font files are now in {folder}", &[("folder", folder)]))
        }
        Some(first) => {
            let failed = len("skipped");
            let file =
                first["file"].as_str().map(std::path::Path::new).and_then(std::path::Path::file_name).map(|f| f.to_string_lossy().into_owned());
            let reason = format!("{}: {}", file.unwrap_or_default(), first["reason"].as_str().unwrap_or_default());
            app.status(crate::i18n::msg_n(
                n as u64,
                "{n} font file is now in {folder}; {failed} couldn't be added: {reason}",
                "{n} font files are now in {folder}; {failed} couldn't be added: {reason}",
                &[("failed", &failed.to_string()), ("folder", folder), ("reason", &reason)],
            ));
        }
    }
    Ok(r)
}

#[cfg(test)]
mod tests;
