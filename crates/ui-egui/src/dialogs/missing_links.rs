//! The linked files a document opened without, one at a time: Replace picks another file
//! (`links.relink`), and with Apply to All the other missing files are looked for by name in its
//! folder; Ignore keeps the images showing their saved preview, and with Apply to All ignores the
//! rest. Find in Folder… looks for every missing file in a folder and its subfolders (dialog
//! `findLinks`, [`super::find_links`]). Afterwards, with Preferences › Update Links: Ask When
//! Modified, the modified links are offered for update (`links.update`). Files another app changes
//! while the document is open are offered the same way ([`ask_update_changed`], from
//! [`vectorcraft_engine::link_watch`]).
//!
//! Fields: `missing` (`[{name, path, ids}]`, the one asked about first), `modified` (the ids of
//! images whose file was modified), `notes` (the open's notes, shown again after the last question
//! or Cancel), `applyToAll`, `path` (the replacement file; picked when empty), `choice` (`ignore`
//! for Ignore) and `searchFolder` (set it and confirm for Find in Folder… on that folder).
//!
//! The dialog answers through [`pump`], which relinks and asks about the next file. What the
//! questions about one open still need after a dialog closed (the open's notes, the modified
//! files) is kept in [`LinkAsk`] on the app: Cancel, Escape and the close box carry no answer.

use std::time::Duration;

// PLAN: crates/ui-egui/Cargo.toml names craft-ui, not the `craft` facade these paths use; the manifest owner adds `craft` with the host, theme and ui features.
use craft::host::{Host, PickId, PickKind, PickRequest, Picked};
use craft::theme::TypeRole;
use craft::ui::buttons::Button;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Elide, Shade, Text};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::env::Label;
use craft::ui::sections::{DialogRole, FooterButton};
use serde_json::{Value, json};

use crate::{VectorcraftApp, file_asks};

/// The dialog kind of a missing linked file.
pub const KIND: &str = "missingLinks";

/// How often a dialog waiting for a picker looks for its answer.
const POLL: Duration = Duration::from_millis(100);

/// The footer index of the accepting button (Replace…) and of Ignore, the first extra button.
const ACCEPT: usize = 0;
const IGNORE: usize = 2;

/// `text` as a label of the dialog: the interface language's wording of it.
pub(super) fn label(text: &str) -> Label {
    Label::from(text.to_string())
}

/// The ids in `rows` (`[{ids}]`).
pub(super) fn ids(rows: &[Value]) -> Vec<Value> {
    rows.iter().flat_map(|r| r["ids"].as_array().cloned().unwrap_or_default()).collect()
}

/// A string field's `value`, for `set_json_field`.
pub(super) fn text_field(field: &str, value: &Value) -> Result<String, String> {
    value.as_str().map(str::to_owned).ok_or_else(|| format!("field `{field}` takes a string, got {value}"))
}

/// A boolean field's `value`, for `set_json_field`.
pub(super) fn flag_field(field: &str, value: &Value) -> Result<bool, String> {
    value.as_bool().ok_or_else(|| format!("field `{field}` takes true or false, got {value}"))
}

/// A picker a dialog filed with the host and waits for; `W` says what the answer is for.
pub(super) struct Asked<W>(Option<(PickId, W)>);

impl<W> Default for Asked<W> {
    fn default() -> Self {
        Asked(None)
    }
}

impl<W: Copy> Asked<W> {
    /// Whether a picker is open or waiting.
    pub(super) fn pending(&self) -> bool {
        self.0.is_some()
    }

    /// Files `request` for `what`, unless a picker is already waiting.
    pub(super) fn file(&mut self, ctx: &egui::Context, request: PickRequest, what: W) {
        if self.0.is_none()
            && let Some(host) = Host::try_of_ctx(ctx)
        {
            self.0 = Some((host.files().pick(request), what));
        }
    }

    /// The answer of the waiting picker, once it came, with what it was for.
    pub(super) fn answer(&mut self, ctx: &egui::Context) -> Option<(Picked, W)> {
        let (id, what) = self.0?;
        let picked = Host::try_of_ctx(ctx)?.files().take_answer(id)?;
        self.0 = None;
        Some((picked, what))
    }

    /// Drops the waiting picker: its answer is not wanted.
    pub(super) fn forget(&mut self, ctx: &egui::Context) {
        if let (Some((id, _)), Some(host)) = (self.0.take(), Host::try_of_ctx(ctx)) {
            host.files().forget(id);
        }
    }
}

/// What the questions about one open's linked files keep between their dialogs: the files still
/// asked about, the modified images to offer for update afterwards and the open's notes.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LinkAsk {
    /// The document that owns the linked-file IDs.
    pub(crate) document: u64,
    /// The missing files still asked about (`[{name, path, ids}]`), the next one first.
    pub(crate) missing: Vec<Value>,
    /// The ids of the images whose file was modified.
    pub(crate) modified: Vec<Value>,
    /// The open's notes, shown in the status bar again after the last question.
    pub(crate) notes: Vec<String>,
    /// Whether Find Linked Files, opened from Find in Folder… here, is the open question: its Cancel
    /// shows the notes again, as this dialog's does.
    pub(crate) back: bool,
}

/// What the user answered about the first missing file.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MissingLinksAnswer {
    /// Replace it with the file at `path`; with `apply_to_all` the others by name in its folder.
    Replace {
        /// The replacement file.
        path: String,
        /// Whether the other missing files are looked for beside it.
        apply_to_all: bool,
    },
    /// Keep the preview; with `apply_to_all` for the other missing files too.
    Ignore {
        /// Whether the other missing files are ignored too.
        apply_to_all: bool,
    },
    /// Look for every missing file in this folder ([`super::find_links`]).
    FindInFolder(String),
}

/// What a waiting picker answers for.
#[derive(Clone, Copy)]
enum Wanted {
    /// The file that replaces the missing one.
    File,
    /// The folder Find in Folder… searches.
    Folder,
}

/// What the form edits.
#[derive(Clone, Default, PartialEq)]
struct Edit {
    apply_to_all: bool,
    path: String,
}

/// Linked File Not Found: the first of the missing files, with Replace…, Ignore and Find in Folder….
pub(crate) struct MissingLinksDialog {
    ask: LinkAsk,
    edit: Edit,
    ignore: bool,
    search_folder: String,
    asked: Asked<Wanted>,
    can_pick: bool,
}

impl MissingLinksDialog {
    /// The dialog over `ask`; Find in Folder… shows when the app can ask for a folder.
    fn new(app: &VectorcraftApp, ask: LinkAsk) -> Self {
        MissingLinksDialog {
            ask,
            edit: Edit::default(),
            ignore: false,
            search_folder: String::new(),
            asked: Asked::default(),
            can_pick: file_asks::can(app, PickKind::Folder),
        }
    }

    /// Replace…: the answer when a file is given, else files the picker for one and waits.
    fn accept(&mut self, ctx: &egui::Context) -> Option<MissingLinksAnswer> {
        if self.edit.path.is_empty() {
            let request = vectorcraft_engine::cmd::fileio::place_filters().fold(PickRequest::open("links"), |r, (name, exts)| r.filter(name, exts));
            self.asked.file(ctx, request, Wanted::File);
            return None;
        }
        Some(MissingLinksAnswer::Replace { path: self.edit.path.clone(), apply_to_all: self.edit.apply_to_all })
    }
}

impl StackDialog for MissingLinksDialog {
    type Result = MissingLinksAnswer;

    fn id(&self) -> DialogId {
        dialog_id(KIND)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<MissingLinksAnswer>> {
        let ctx = ui.ctx().clone();
        if let Some((picked, wanted)) = self.asked.answer(&ctx)
            && let Some(path) = picked.first_path()
        {
            let path = path.to_string_lossy().into_owned();
            return Some(DialogResult::Ok(match wanted {
                Wanted::File => MissingLinksAnswer::Replace { path, apply_to_all: self.edit.apply_to_all },
                Wanted::Folder => MissingLinksAnswer::FindInFolder(path),
            }));
        }
        if self.asked.pending() {
            ctx.request_repaint_after(POLL);
        }
        let Some(current) = self.ask.missing.first() else { return Some(DialogResult::Cancel) };
        let rest = self.ask.missing.len() - 1;
        let (name, path) = (current["name"].as_str().unwrap_or_default(), current["path"].as_str().unwrap_or_default());
        let (can_pick, mut find) = (self.can_pick, false);
        let shown = FormDialog::new(label(tl!("Linked File Not Found")), Bind::bound(&mut self.edit))
            .ok_label(label(tl!("Replace…")))
            .accept_stays_open(true)
            .extra_buttons([FooterButton::new(label(tl!("Ignore")), DialogRole::Other)])
            .show(ui, |ui, _, edit| {
                Text::new(label(name)).role(TypeRole::BodyStrong).show(ui);
                Text::new(label(path)).shade(Shade::Muted).elide(Elide::Path).tooltip(path.to_string()).show(ui);
                Text::new(label(tl!("Replace it with another file, or ignore it: its images keep showing the preview saved with the document.")))
                    .shade(Shade::Muted)
                    .show(ui);
                if can_pick && Button::new(label(tl!("Find in Folder…"))).show(ui).clicked() {
                    find = true;
                }
                if rest > 0 {
                    let all = crate::i18n::tn(rest as u64, "Apply to All ({n} more missing)", "Apply to All ({n} more missing)");
                    Checkbox::new(Bind::bound(&mut edit.apply_to_all)).label(label(&all)).show(ui);
                }
                true
            });
        if find {
            self.asked.file(&ctx, PickRequest::folder("links"), Wanted::Folder);
        }
        if shown.picked == Some(IGNORE) {
            return Some(DialogResult::Ok(MissingLinksAnswer::Ignore { apply_to_all: self.edit.apply_to_all }));
        }
        if shown.picked == Some(ACCEPT) || shown.accepted.is_some() {
            return self.accept(&ctx).map(DialogResult::Ok);
        }
        if shown.closed.is_some() {
            self.asked.forget(&ctx);
            return Some(DialogResult::Cancel);
        }
        None
    }

    fn confirm(&mut self, ctx: &egui::Context) -> Option<MissingLinksAnswer> {
        if !self.search_folder.is_empty() {
            return Some(MissingLinksAnswer::FindInFolder(std::mem::take(&mut self.search_folder)));
        }
        if self.ignore {
            return Some(MissingLinksAnswer::Ignore { apply_to_all: self.edit.apply_to_all });
        }
        self.accept(ctx)
    }

    fn cancelled(&mut self, ctx: &egui::Context) {
        self.asked.forget(ctx);
    }

    fn json_fields(&self) -> Option<Value> {
        let LinkAsk { missing, modified, notes, .. } = &self.ask;
        Some(json!({ "missing": missing, "modified": modified, "notes": notes, "applyToAll": self.edit.apply_to_all, "path": self.edit.path }))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "applyToAll" => self.edit.apply_to_all = flag_field(field, &value)?,
            "path" => self.edit.path = text_field(field, &value)?,
            "searchFolder" => self.search_folder = text_field(field, &value)?,
            "choice" => match value.as_str() {
                Some("ignore") => self.ignore = true,
                Some("replace") => self.ignore = false,
                _ => return Err(format!("field `choice` takes `replace` or `ignore`, got {value}")),
            },
            _ => return Err(format!("unknown field `{field}`")),
        }
        ctx.request_repaint();
        Ok(())
    }
}

/// After a document opened (`document.open`'s result `r`): ask about its missing linked files,
/// then about its modified ones, then about the fonts it opened without
/// ([`super::missing_fonts`]). The web reads no linked files, so missing ones just show their
/// previews there. The status bar shows what it says about the linked files followed by the open's
/// `notes` ([`crate::io::show_opened`]). On the web, with both missing and modified files and
/// Update Links not asking, the line about the missing files comes first and the line about the
/// modified ones is the first of the notes.
pub fn after_open(app: &mut VectorcraftApp, r: &Value, notes: Vec<String>) {
    after_open_for(app, r, notes, cfg!(target_arch = "wasm32"));
}

fn after_open_for(app: &mut VectorcraftApp, r: &Value, mut notes: Vec<String>, web: bool) {
    // The fonts' dialog waits for these questions (one dialog at a time).
    super::missing_fonts::after_open(app, r);
    let rows = |k: &str| r[k].as_array().cloned().unwrap_or_default();
    let (missing, modified) = (rows("missingLinks"), ids(&rows("modifiedLinks")));
    let notice = if missing.is_empty() || web {
        let unreadable = (!missing.is_empty()).then(|| unreadable_status(missing.len()));
        match (ask_update(app, modified), unreadable) {
            (Some(changed), Some(unreadable)) => {
                notes.insert(0, changed);
                Some(unreadable)
            }
            (changed, unreadable) => changed.or(unreadable),
        }
    } else {
        open(app, missing, modified, &notes);
        None
    };
    crate::io::show_opened(app, notice, notes);
    // PLAN: the registry's `super::settle(app)` that ended this function is the frame pump's now (fl.A9).
}

/// The status bar's line for `n` linked files the web can't read.
fn unreadable_status(n: usize) -> String {
    crate::i18n::msg_n(
        n as u64,
        "{n} linked file can't be read here: its preview shows",
        "{n} linked files can't be read here: their previews show",
        &[],
    )
}

/// What the linked files of a document that opened (`document.open`'s result `r`) need, without
/// asking yet → the status bar's lines about them, the missing files to ask about and the ids of
/// the modified images to offer for update ([`ask_next`]). The web reads no linked files, so the
/// missing ones get a line there; without Update Links: Ask When Modified the modified ones get a
/// line instead of the offer.
pub(crate) fn questions(app: &VectorcraftApp, r: &Value) -> (Vec<String>, Vec<Value>, Vec<Value>) {
    questions_for(app, r, cfg!(target_arch = "wasm32"))
}

fn questions_for(app: &VectorcraftApp, r: &Value, web: bool) -> (Vec<String>, Vec<Value>, Vec<Value>) {
    let rows = |k: &str| r[k].as_array().cloned().unwrap_or_default();
    let (mut missing, mut modified) = (rows("missingLinks"), ids(&rows("modifiedLinks")));
    let mut lines = vec![];
    if web && !missing.is_empty() {
        lines.push(unreadable_status(missing.len()));
        missing.clear();
    }
    if !modified.is_empty() && app.session.prefs.update_links != "askWhenModified" {
        lines.push(changed_on_disk_status(modified.len()));
        modified.clear();
    }
    (lines, missing, modified)
}

/// Opens the dialog about the first of `missing`, keeping what the questions after it need
/// ([`LinkAsk`]). A window that isn't up has nothing to show a question in.
fn open(app: &mut VectorcraftApp, missing: Vec<Value>, modified: Vec<Value>, notes: &[String]) {
    let Some(document) = app.session.active().map(|st| st.uid) else { return };
    open_ask(app, LinkAsk { document, missing, modified, notes: notes.to_vec(), back: false });
}

fn open_ask(app: &mut VectorcraftApp, ask: LinkAsk) {
    let Some(dialogs) = app.dialogs() else { return };
    let dialog = MissingLinksDialog::new(app, ask.clone());
    app.link_asks = Some(ask);
    dialogs.with(|h| h.open(dialog));
}

/// The status bar shows the open's notes again, after the line about the modified files when
/// Update Links doesn't ask about them ([`crate::io::show_opened`]): the rest of the questions are
/// not asked.
pub(super) fn show_notes_again(app: &mut VectorcraftApp, ask: &LinkAsk) {
    let modified = ask.modified.len();
    let notice = (modified > 0 && app.session.prefs.update_links != "askWhenModified").then(|| changed_on_disk_status(modified));
    crate::io::show_opened(app, notice, ask.notes.clone());
}

/// Ask about the missing files `rest`, one at a time, then offer the modified ones `modified` for
/// update. After the last question the status bar shows the open's `notes` again, after the line
/// about the modified files when there is one ([`crate::io::show_opened`]), so their hover lists
/// them whatever message the answers showed.
pub(crate) fn ask_next(app: &mut VectorcraftApp, rest: Vec<Value>, modified: Vec<Value>, notes: Vec<String>) {
    if rest.is_empty() {
        app.link_asks = None;
        let notice = ask_update(app, modified);
        crate::io::show_opened(app, notice, notes);
    } else {
        open(app, rest, modified, &notes);
    }
}

/// With Update Links: Ask When Modified, offer to read the modified linked files `ids` again;
/// otherwise → the status bar's line for them ([`changed_on_disk_status`]).
fn ask_update(app: &mut VectorcraftApp, ids: Vec<Value>) -> Option<String> {
    ask(
        app,
        ids,
        "{n} linked image changed since this document was saved. Show the new version?",
        "{n} linked images changed since this document was saved. Show the new versions?",
    )
}

/// The same offer for linked files another app changed while the document is open
/// ([`vectorcraft_engine::link_watch`]).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn ask_update_changed(app: &mut VectorcraftApp, ids: Vec<Value>) {
    let status =
        ask(app, ids, "{n} linked image changed on disk. Show the new version?", "{n} linked images changed on disk. Show the new versions?");
    if let Some(status) = status {
        app.status(status);
    }
}

/// Ask about the modified linked files `ids`; `one` and `other` are the English forms of the
/// question. Without Update Links: Ask When Modified → the status bar's line for them instead.
fn ask(app: &mut VectorcraftApp, ids: Vec<Value>, one: &str, other: &str) -> Option<String> {
    if ids.is_empty() {
        return None;
    }
    if app.session.prefs.update_links != "askWhenModified" {
        return Some(changed_on_disk_status(ids.len()));
    }
    let detail = crate::i18n::tn(ids.len() as u64, one, other);
    super::confirm::ask(app, tl!("Update Modified Links"), &detail, "links.update", json!({ "ids": ids }));
    None
}

/// The status bar's line for `n` linked images that changed on disk while Update Links doesn't ask.
pub(crate) fn changed_on_disk_status(n: usize) -> String {
    crate::i18n::msg_n(
        n as u64,
        "{n} linked image changed on disk: Update Links shows the new version",
        "{n} linked images changed on disk: Update Links shows the new versions",
        &[],
    )
}

/// Takes the answer of the dialog that closed: Cancel shows the open's notes again; Replace relinks
/// the first file, Ignore keeps its preview, and either asks about the next. Find in Folder… opens
/// Find Linked Files over the files still asked about ([`super::find_links::from_missing_links`]).
/// A step that fails says why in the status bar and asks about the same files again.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let Some(result) = dialogs.with(|h| h.take::<MissingLinksDialog>()) else { return };
    let Some(ask) = app.link_asks.take() else { return };
    let Some(answer) = result.ok() else { return show_notes_again(app, &ask) };
    if app.session.active().is_none_or(|st| st.uid != ask.document) {
        app.status(tl!("This dialog was opened in a different document."));
        if app.session.documents().iter().any(|st| st.uid == ask.document) {
            open_ask(app, ask);
        }
        return;
    }
    let LinkAsk { document, missing, modified, notes, .. } = ask;
    let done = match answer {
        MissingLinksAnswer::FindInFolder(folder) => {
            super::find_links::from_missing_links(app, missing.clone(), modified.clone(), notes.clone(), &folder).map(|()| None)
        }
        answer => apply(app, &missing, answer).map(Some),
    };
    match done {
        Ok(Some(rest)) => ask_next(app, rest, modified, notes),
        Ok(None) => {}
        Err(e) => {
            app.status(e);
            open_ask(app, LinkAsk { document, missing, modified, notes, back: false });
        }
    }
}

/// Replace (with `path`) or Ignore the first of `missing` → the files still to ask about. Apply to
/// All: the others by name in the same folder; any not there are asked about.
fn apply(app: &mut VectorcraftApp, missing: &[Value], answer: MissingLinksAnswer) -> Result<Vec<Value>, String> {
    let Some((current, rest)) = missing.split_first() else { return Ok(vec![]) };
    let mut rest = rest.to_vec();
    match answer {
        MissingLinksAnswer::Ignore { apply_to_all } => {
            if apply_to_all {
                rest.clear();
            }
        }
        MissingLinksAnswer::Replace { path, apply_to_all } => {
            app.run("links.relink", json!({ "ids": current["ids"], "path": path }))?;
            let folder = std::path::Path::new(&path).parent().map(|p| p.to_string_lossy().into_owned());
            if let (true, false, Some(folder)) = (apply_to_all, rest.is_empty(), folder) {
                let r = app.run("links.relink", json!({ "ids": ids(&rest), "folder": folder }))?;
                let done = r["relinked"].as_array().cloned().unwrap_or_default();
                rest.retain(|row| row["ids"].as_array().is_none_or(|ids| ids.iter().any(|id| !done.contains(id))));
            }
        }
        // `pump` sends Find in Folder… elsewhere.
        MissingLinksAnswer::FindInFolder(_) => {}
    }
    Ok(rest)
}

/// What the tests of both link dialogs drive them with: the window's dialog host, as the shell
/// shows it, and the frame pump.
#[cfg(test)]
pub(super) mod drive {
    use super::*;

    /// An app whose window has a dialog host.
    pub(crate) fn app() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.adopt_context(&crate::testing::ui_context());
        app
    }

    /// Whether dialog `D` is open.
    pub(crate) fn is_open<D: StackDialog>(app: &VectorcraftApp) -> bool {
        app.dialogs().is_some_and(|d| d.with(|h| h.is_open::<D>()))
    }

    /// The fields of the open dialog `D` (`Null` when it is not open).
    pub(crate) fn fields<D: StackDialog>(app: &VectorcraftApp) -> Value {
        app.dialogs().and_then(|d| d.with(|h| h.get::<D>().and_then(|d| d.json_fields()))).unwrap_or(Value::Null)
    }

    /// One frame of the window's dialogs, which take the answers of the pickers they filed.
    pub(crate) fn frame(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().expect("the context the app adopted");
        let dialogs = app.dialogs().expect("a window dialog host");
        ctx.run_ui(Default::default(), |ui| dialogs.with(|h| h.run(ui))).textures_delta.clear();
    }

    /// One frame's pump.
    pub(crate) fn pump(app: &mut VectorcraftApp) {
        let ctx = app.ui_ctx.clone().expect("the context the app adopted");
        crate::dialogs::host::pump(app, &ctx);
    }

    /// Sets `fields` of the open dialog `D`, then confirms it (`ok`) or cancels it, then pumps.
    pub(crate) fn answer<D: StackDialog>(app: &mut VectorcraftApp, fields: &[(&str, Value)], ok: bool) {
        let ctx = app.ui_ctx.clone().expect("the context the app adopted");
        let dialogs = app.dialogs().expect("a window dialog host");
        dialogs.with(|h| {
            let Some(id) = h.get::<D>().map(|d| d.id()) else { return };
            for (field, value) in fields {
                h.set_json_field(&ctx, id, field, value.clone()).expect("a field the dialog takes");
            }
            if ok {
                h.confirm_by_id(&ctx, id);
            } else {
                h.cancel_by_id(&ctx, id);
            }
        });
        pump(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(rgb: [u8; 3]) -> Vec<u8> {
        let [r, g, b] = rgb;
        let img = craft_image_codecs::Image::from_raw(
            400,
            300,
            craft_image_codecs::ChannelLayout::Rgba,
            craft_image_codecs::SampleType::U8,
            [r, g, b, 255].repeat(400 * 300),
        )
        .unwrap();
        craft_image_codecs::encode(&img, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).unwrap()
    }

    /// A document with two placed linked files, saved to a fresh folder; the files are then
    /// deleted → (app, folder, document path).
    fn opened_without_links(name: &str) -> (VectorcraftApp, std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("vectorcraft-missing-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = drive::app();
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        for n in ["a.png", "b.png"] {
            let p = dir.join(n);
            std::fs::write(&p, png([200, 0, 0])).unwrap();
            app.run("file.place", json!({"path": p.to_string_lossy()})).unwrap();
        }
        let doc = dir.join("doc.vectorcraft").to_string_lossy().into_owned();
        app.run("document.save", json!({"path": doc})).unwrap();
        for n in ["a.png", "b.png"] {
            std::fs::remove_file(dir.join(n)).unwrap();
        }
        // Reopened: an open file would only come to the front (#3455).
        app.run("file.close", json!({})).unwrap();
        crate::io::open_path(&mut app, &doc).unwrap();
        (app, dir, doc)
    }

    /// The name of the file the open dialog asks about.
    fn asked_name(app: &VectorcraftApp) -> Value {
        drive::fields::<MissingLinksDialog>(app)["missing"][0]["name"].clone()
    }

    fn row(n: u64) -> Value {
        json!({ "name": format!("{n}.png"), "path": format!("/gone/{n}.png"), "ids": [n] })
    }

    fn check_replacing_a_missing_file_stays_in_the_opening_drawing(native_picker: bool) {
        let name = if native_picker { "different-drawing-picker" } else { "different-drawing-path" };
        let (mut app, dir, _) = opened_without_links(name);
        let missing = drive::fields::<MissingLinksDialog>(&app)["missing"][0].clone();
        let id = missing["ids"][0].as_u64().unwrap();
        let picker = crate::tests::picks::scripted(&mut app);
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let mut second_ids = Vec::new();
        for name in ["second-one.png", "second-two.png"] {
            let path = dir.join(name);
            std::fs::write(&path, png([0, 200, 0])).unwrap();
            let place = app.run("file.place", json!({"path": path.to_string_lossy()})).unwrap();
            second_ids.push(place["ids"][0].as_u64().unwrap());
        }
        assert!(second_ids.contains(&id), "the same ID refers to a different linked file");
        let params = json!({"ids": [id]});
        let original_path = app.run("links.check", params.clone()).unwrap()["links"][0]["path"].clone();

        app.run("document.activate", json!({"index": 0})).unwrap();
        if native_picker {
            drive::answer::<MissingLinksDialog>(&mut app, &[], true);
            assert_eq!(picker.requests().len(), 1);
        }
        app.run("document.activate", json!({"index": 1})).unwrap();
        let replacement = dir.join("replacement.png");
        std::fs::write(&replacement, png([0, 0, 200])).unwrap();
        let replacement = replacement.to_string_lossy();
        if native_picker {
            picker.answer(&[&replacement]);
            drive::frame(&mut app);
            drive::pump(&mut app);
        } else {
            drive::answer::<MissingLinksDialog>(&mut app, &[("path", json!(replacement))], true);
        }

        assert_eq!(app.session.active_index(), Some(1));
        let after = app.run("links.check", params).unwrap()["links"][0]["path"].clone();
        assert_eq!(after, original_path, "the second drawing must keep its original link");
        assert_eq!(app.ui.status, "This dialog was opened in a different document.");
        assert_eq!(drive::fields::<MissingLinksDialog>(&app)["missing"][0], missing);

        app.run("document.activate", json!({"index": 0})).unwrap();
        drive::answer::<MissingLinksDialog>(&mut app, &[("path", json!(replacement))], true);
        assert_eq!(app.run("links.check", json!({})).unwrap()["missing"], 1, "the opening drawing can still replace its file");
        assert_eq!(asked_name(&app), "b.png", "the remaining missing file is still offered");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn typed_missing_file_replacement_keeps_the_other_drawing_untouched() {
        check_replacing_a_missing_file_stays_in_the_opening_drawing(false);
    }

    #[test]
    fn delayed_missing_file_picker_keeps_the_other_drawing_untouched() {
        check_replacing_a_missing_file_stays_in_the_opening_drawing(true);
    }

    #[test]
    fn a_missing_file_folder_search_starts_only_in_the_opening_drawing() {
        use crate::dialogs::find_links::FindLinksDialog;

        let (mut app, dir, _) = opened_without_links("different-drawing-search");
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let folder = [("searchFolder", json!(dir.to_string_lossy()))];
        drive::answer::<MissingLinksDialog>(&mut app, &folder, true);
        assert!(drive::is_open::<MissingLinksDialog>(&app));
        assert!(!drive::is_open::<FindLinksDialog>(&app));
        assert_eq!(app.session.active_index(), Some(1));

        app.run("document.activate", json!({"index": 0})).unwrap();
        drive::answer::<MissingLinksDialog>(&mut app, &folder, true);
        assert!(drive::is_open::<FindLinksDialog>(&app));
        assert_eq!(app.session.active_index(), Some(0));
        drive::answer::<FindLinksDialog>(&mut app, &[], false);
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Replace relinks the first file and asks about the next; Ignore keeps its preview.
    #[test]
    fn opening_without_linked_files_asks_to_replace_or_ignore_each() {
        let (mut app, dir, _) = opened_without_links("each");
        assert_eq!(drive::fields::<MissingLinksDialog>(&app)["missing"].as_array().map(Vec::len), Some(2));
        // Replace the first with a file given by path (the file picker otherwise).
        let blue = dir.join("new/blue.png");
        std::fs::create_dir_all(blue.parent().unwrap()).unwrap();
        std::fs::write(&blue, png([0, 0, 200])).unwrap();
        drive::answer::<MissingLinksDialog>(&mut app, &[("path", json!(blue.to_string_lossy()))], true);
        assert_eq!(asked_name(&app), "b.png", "asks about the next");
        // Ignore: done, the image keeps its preview.
        drive::answer::<MissingLinksDialog>(&mut app, &[("choice", json!("ignore"))], true);
        assert!(!drive::is_open::<MissingLinksDialog>(&app));
        let c = app.run("links.check", json!({})).unwrap();
        let status: Vec<&str> = c["links"].as_array().unwrap().iter().map(|l| l["status"].as_str().unwrap()).collect();
        assert_eq!((status, c["missing"].as_u64()), (vec!["ok", "missing"], Some(1)));
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A restored document's linked files: the web gets a line about the missing ones, and
    /// without Update Links: Ask When Modified the modified ones get a line; the rest are asked
    /// about.
    #[test]
    fn the_questions_about_linked_files_wait_or_become_lines() {
        let mut app = drive::app();
        let r = json!({ "missingLinks": [{"name": "a.png", "path": "/x/a.png", "ids": [1]}], "modifiedLinks": [{"ids": [2]}, {"ids": [3]}] });
        app.session.prefs.update_links = "askWhenModified".into();
        assert_eq!(questions_for(&app, &r, false), (vec![], r["missingLinks"].as_array().unwrap().clone(), vec![json!(2), json!(3)]));
        let changed = "2 linked images changed on disk: Update Links shows the new versions".to_string();
        app.session.prefs.update_links = "manually".into();
        assert_eq!(questions_for(&app, &r, false), (vec![changed.clone()], r["missingLinks"].as_array().unwrap().clone(), vec![]));
        let unreadable = "1 linked file can't be read here: its preview shows".to_string();
        assert_eq!(questions_for(&app, &r, true), (vec![unreadable, changed], vec![], vec![]));
        assert!(!drive::is_open::<MissingLinksDialog>(&app), "nothing is asked yet");
    }

    /// On the web, an open with two notes and two linked files it can't read: the status bar says
    /// so about the files, then counts the notes, and keeps the notes for its hover. Without notes
    /// the line about the files is shown alone.
    #[test]
    fn an_open_with_unreadable_links_keeps_import_notes_reachable() {
        let mut app = drive::app();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let missing = json!([{ "name": "a.png", "path": "/gone/a.png", "ids": [1] }, { "name": "b.png", "path": "/gone/b.png", "ids": [2] }]);
        let notes = vec!["a note".to_string(), "another note".to_string()];
        let r = json!({ "missingLinks": missing, "warnings": notes });
        after_open_for(&mut app, &r, notes.clone(), true);
        let notice = "2 linked files can't be read here: their previews show";
        assert_eq!(app.ui.status, crate::io::opened_after_notice(notice, 2, "a note"));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), notes)));
        assert!(!drive::is_open::<MissingLinksDialog>(&app));
        after_open_for(&mut app, &json!({ "missingLinks": missing }), vec![], true);
        assert_eq!((app.ui.status.as_str(), &app.ui.status_notes), (notice, &None));
    }

    /// On the web, an open with linked files it can't read and modified ones, with Update Links
    /// not asking: the status bar shows the line about the unreadable files and counts the line
    /// about the modified ones as the first note, ahead of the open's notes, and keeps them all
    /// for its hover. With Ask When Modified the update question opens and the line about the
    /// unreadable files stays.
    #[test]
    fn a_web_open_with_unreadable_and_modified_links_keeps_both_lines() {
        let mut app = drive::app();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.session.prefs.update_links = "manually".into();
        let missing = json!([{ "name": "a.png", "path": "/gone/a.png", "ids": [1] }, { "name": "b.png", "path": "/gone/b.png", "ids": [2] }]);
        let r = json!({ "missingLinks": missing, "modifiedLinks": [{ "ids": [3] }] });
        let unreadable = "2 linked files can't be read here: their previews show";
        let changed = changed_on_disk_status(1);

        let notes = vec!["a note".to_string(), "another note".to_string()];
        after_open_for(&mut app, &r, notes.clone(), true);
        let kept = [vec![changed.clone()], notes].concat();
        assert_eq!(app.ui.status, crate::io::opened_after_notice(unreadable, 3, &changed));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), kept)));
        assert!(!drive::is_open::<MissingLinksDialog>(&app));

        after_open_for(&mut app, &r, vec![], true);
        assert_eq!(app.ui.status, crate::io::opened_after_notice(unreadable, 1, &changed));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), vec![changed.clone()])));

        // One kind of line alone shows as before.
        after_open_for(&mut app, &json!({ "modifiedLinks": [{ "ids": [3] }] }), vec![], true);
        assert_eq!((app.ui.status.clone(), &app.ui.status_notes), (changed, &None));

        app.session.prefs.update_links = "askWhenModified".into();
        after_open_for(&mut app, &r, vec![], true);
        // PLAN: assumed `confirm::open_question(&VectorcraftApp) -> Option<(String, String, String)>` (message, detail, command; widgets-w10).
        let (_, _, command) = super::super::confirm::open_question(&app).expect("asks");
        assert_eq!(command, "links.update");
        assert_eq!((app.ui.status.as_str(), &app.ui.status_notes), (unreadable, &None));
    }

    /// The line about modified linked files that follows the Linked File Not Found dialog keeps the
    /// notes of that open, also when another message was shown meanwhile. An open without notes
    /// shows the line alone, even while the status bar shows an earlier open's notes, and a later
    /// line about files changed on disk while the document is open doesn't take them.
    #[test]
    fn modified_links_after_the_missing_ones_keep_the_open_notes() {
        let mut app = drive::app();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.session.prefs.update_links = "manually".into();
        let notes = vec!["a note".to_string(), "another note".to_string()];
        let links = json!({
            "missingLinks": [{ "name": "a.png", "path": "/gone/a.png", "ids": [1] }],
            "modifiedLinks": [{ "ids": [2] }],
        });
        let ignore = |app: &mut VectorcraftApp| {
            drive::answer::<MissingLinksDialog>(app, &[("choice", json!("ignore"))], true);
            assert!(!drive::is_open::<MissingLinksDialog>(app));
        };
        after_open_for(&mut app, &links, notes.clone(), false);
        assert_eq!(drive::fields::<MissingLinksDialog>(&app)["notes"], json!(notes));
        assert_eq!(app.ui.status, crate::io::opened_with_notes(2, "a note"));
        app.status("Relinked 1 image");
        ignore(&mut app);
        assert_eq!(app.ui.status, crate::io::opened_after_notice(&changed_on_disk_status(1), 2, "a note"));
        assert_eq!(app.ui.status_notes, Some((app.ui.status.clone(), notes.clone())));

        crate::io::show_opened(&mut app, None, notes);
        after_open_for(&mut app, &links, vec![], false);
        ignore(&mut app);
        assert_eq!((app.ui.status.clone(), &app.ui.status_notes), (changed_on_disk_status(1), &None));

        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::io::show_opened(&mut app, None, vec!["a note".to_string(), "another note".to_string()]);
            ask_update_changed(&mut app, vec![json!(2)]);
            assert_eq!(app.ui.status, changed_on_disk_status(1));
            assert_ne!(app.ui.status_notes.as_ref().map(|(status, _)| status), Some(&app.ui.status));
        }
    }

    /// After the last question of the Linked File Not Found dialog the status bar shows the open's
    /// notes again and keeps them for its hover, also when another message was shown meanwhile:
    /// after Ignore, Ignore with Apply to All and Cancel, with either Update Links choice, and with
    /// Ask When Modified while the update question opens. An open without notes leaves the message
    /// as it is.
    #[test]
    fn the_last_missing_link_answer_keeps_the_open_notes() {
        let mut app = drive::app();
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let notes = vec!["a note".to_string(), "another note".to_string()];
        let opened = crate::io::opened_with_notes(2, "a note");
        let answer = |app: &mut VectorcraftApp, fields: &[(&str, Value)]| {
            app.status("Something else");
            let fields: Vec<(&str, Value)> = [("choice", json!("ignore"))].into_iter().chain(fields.iter().cloned()).collect();
            drive::answer::<MissingLinksDialog>(app, &fields, true);
        };
        for prefs in ["manually", "askWhenModified"] {
            app.session.prefs.update_links = prefs.into();
            after_open_for(&mut app, &json!({ "missingLinks": [row(1)] }), notes.clone(), false);
            answer(&mut app, &[]);
            assert!(!drive::is_open::<MissingLinksDialog>(&app));
            assert_eq!((&app.ui.status, &app.ui.status_notes), (&opened, &Some((opened.clone(), notes.clone()))), "Ignore, {prefs}");

            after_open_for(&mut app, &json!({ "missingLinks": [row(1), row(2)] }), notes.clone(), false);
            answer(&mut app, &[("applyToAll", json!(true))]);
            assert!(!drive::is_open::<MissingLinksDialog>(&app));
            assert_eq!((&app.ui.status, &app.ui.status_notes), (&opened, &Some((opened.clone(), notes.clone()))), "Apply to All, {prefs}");

            after_open_for(&mut app, &json!({ "missingLinks": [row(1), row(2)], "modifiedLinks": [{ "ids": [3] }] }), notes.clone(), false);
            app.status("Something else");
            drive::answer::<MissingLinksDialog>(&mut app, &[], false);
            assert!(!drive::is_open::<MissingLinksDialog>(&app), "Cancel asks nothing more");
            let cancelled =
                if prefs == "manually" { crate::io::opened_after_notice(&changed_on_disk_status(1), 2, "a note") } else { opened.clone() };
            assert_eq!((&app.ui.status, &app.ui.status_notes), (&cancelled, &Some((cancelled.clone(), notes.clone()))), "Cancel, {prefs}");
        }

        after_open_for(&mut app, &json!({ "missingLinks": [row(1)], "modifiedLinks": [{ "ids": [3] }] }), notes.clone(), false);
        answer(&mut app, &[]);
        // PLAN: assumed `confirm::open_question(&VectorcraftApp) -> Option<(String, String, String)>` (widgets-w10).
        let (_, _, command) = super::super::confirm::open_question(&app).expect("asks");
        assert_eq!(command, "links.update");
        assert_eq!((&app.ui.status, &app.ui.status_notes), (&opened, &Some((opened.clone(), notes.clone()))));

        after_open_for(&mut app, &json!({ "missingLinks": [row(1)] }), vec![], false);
        answer(&mut app, &[]);
        assert_eq!(app.ui.status, "Something else");
        after_open_for(&mut app, &json!({ "missingLinks": [row(1)] }), vec![], false);
        app.status("Something else");
        drive::answer::<MissingLinksDialog>(&mut app, &[], false);
        assert_eq!(app.ui.status, "Something else");
        app.session.prefs.update_links = "manually".into();
        after_open_for(&mut app, &json!({ "missingLinks": [row(1)], "modifiedLinks": [{ "ids": [3] }] }), vec![], false);
        app.status("Something else");
        drive::answer::<MissingLinksDialog>(&mut app, &[], false);
        assert_eq!(app.ui.status, changed_on_disk_status(1), "Cancel keeps the line about the modified files");
    }

    /// Replace with a file given by path, with a picked file and with Apply to All: after the last
    /// one the status bar shows the open's notes again. A canceled file picker leaves the question
    /// open and the message as it is.
    #[test]
    fn the_last_replace_keeps_the_open_notes() {
        let notes = vec!["a note".to_string(), "another note".to_string()];
        let opened = crate::io::opened_with_notes(2, "a note");
        let (mut app, dir, _) = opened_without_links("notes");
        let found = dir.join("found");
        std::fs::create_dir_all(&found).unwrap();
        for n in ["a.png", "b.png"] {
            std::fs::write(found.join(n), png([0, 0, 200])).unwrap();
        }
        // The open's notes are the ones the question keeps.
        app.link_asks.as_mut().unwrap().notes = notes.clone();
        drive::answer::<MissingLinksDialog>(&mut app, &[("path", json!(found.join("a.png").to_string_lossy()))], true);
        assert_eq!(asked_name(&app), "b.png", "asks about the next");

        // PLAN: assumed `tests::picks::scripted(&mut VectorcraftApp) -> Picker` with `requests()`, `answer(&[&str])` and `cancel()` (fl.A20, files-w7).
        let picker = crate::tests::picks::scripted(&mut app);
        app.status("Something else");
        // Replace… without a file files the picker; the dialog takes its answer in its next frame.
        drive::answer::<MissingLinksDialog>(&mut app, &[], true);
        assert_eq!(picker.requests().len(), 1);
        picker.cancel();
        drive::frame(&mut app);
        drive::pump(&mut app);
        assert!(drive::is_open::<MissingLinksDialog>(&app), "a canceled picker leaves the question open");
        assert_eq!(asked_name(&app), "b.png");
        assert_eq!(app.ui.status, "Something else");

        drive::answer::<MissingLinksDialog>(&mut app, &[], true);
        picker.answer(&[&found.join("b.png").to_string_lossy()]);
        drive::frame(&mut app);
        drive::pump(&mut app);
        assert!(!drive::is_open::<MissingLinksDialog>(&app));
        assert_eq!((&app.ui.status, &app.ui.status_notes), (&opened, &Some((opened.clone(), notes.clone()))), "picked");
        let _ = std::fs::remove_dir_all(dir);

        let (mut app, dir, _) = opened_without_links("notes-all");
        let found = dir.join("found");
        std::fs::create_dir_all(&found).unwrap();
        for n in ["a.png", "b.png"] {
            std::fs::write(found.join(n), png([0, 0, 200])).unwrap();
        }
        app.link_asks.as_mut().unwrap().notes = notes.clone();
        app.status("Something else");
        let fields = [("applyToAll", json!(true)), ("path", json!(found.join("a.png").to_string_lossy()))];
        drive::answer::<MissingLinksDialog>(&mut app, &fields, true);
        assert!(!drive::is_open::<MissingLinksDialog>(&app), "both relinked");
        assert_eq!((&app.ui.status, &app.ui.status_notes), (&opened, &Some((opened.clone(), notes.clone()))), "Apply to All");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn apply_to_all_replaces_the_others_from_the_same_folder() {
        let (mut app, dir, _) = opened_without_links("all");
        let found = dir.join("found");
        std::fs::create_dir_all(&found).unwrap();
        for n in ["a.png", "b.png"] {
            std::fs::write(found.join(n), png([0, 0, 200])).unwrap();
        }
        let fields = [("applyToAll", json!(true)), ("path", json!(found.join("a.png").to_string_lossy()))];
        drive::answer::<MissingLinksDialog>(&mut app, &fields, true);
        assert!(!drive::is_open::<MissingLinksDialog>(&app), "both relinked");
        let c = app.run("links.check", json!({})).unwrap();
        assert_eq!(c["missing"], 0, "{c}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn modified_links_are_offered_for_update_when_the_preference_asks() {
        let dir = std::env::temp_dir().join(format!("vectorcraft-modified-ui-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut app = drive::app();
        app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        let pic = dir.join("a.png");
        std::fs::write(&pic, png([200, 0, 0])).unwrap();
        app.run("file.place", json!({"path": pic.to_string_lossy()})).unwrap();
        let doc = dir.join("doc.vectorcraft").to_string_lossy().into_owned();
        app.run("document.save", json!({"path": doc})).unwrap();
        let original_stamp = std::fs::metadata(&pic).unwrap().modified().unwrap();
        let mark_modified = |seconds| {
            // Link stamps use milliseconds. Equal-size writes can finish within one tick;
            // advance the fixture's mtime explicitly rather than relying on a sleep or load.
            std::fs::OpenOptions::new()
                .write(true)
                .open(&pic)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(original_stamp + std::time::Duration::from_secs(seconds)))
                .unwrap();
        };
        std::fs::write(&pic, png([0, 0, 200])).unwrap();
        mark_modified(2);
        // Reopened: an open file would only come to the front (#3455).
        app.run("file.close", json!({})).unwrap();
        crate::io::open_path(&mut app, &doc).unwrap();
        let (_, _, command) = super::super::confirm::open_question(&app).expect("asks");
        assert_eq!(command, "links.update");
        // OK runs the question's command once the question has closed.
        let question = crate::tests::picks::open_dialogs(&app).pop().map(|d| d.id).expect("the question is open");
        crate::tests::picks::answer_dialog(&mut app, question, &[], true);
        assert_eq!(app.run("links.check", json!({})).unwrap()["modified"], 0);
        // Manually: no question.
        app.run("prefs.set", json!({"key": "updateLinks", "value": "manually"})).unwrap();
        std::fs::write(&pic, png([0, 200, 0])).unwrap();
        mark_modified(4);
        app.session.execute("file.close", &json!({})).unwrap();
        crate::io::open_path(&mut app, &doc).unwrap();
        assert!(super::super::confirm::open_question(&app).is_none());
        assert_eq!(app.run("links.check", json!({})).unwrap()["modified"], 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
