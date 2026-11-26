//! File → File Info: the document title, author, description, rating, keywords (chips) and
//! copyright, with the created and modified dates.
//!
//! Fields: what `file.info` reports (and takes back) plus `__keyword`, the keyword being typed
//! (OK keeps it too). OK files the fields for the app, which runs `file.info` with them: one undo
//! step. On an error the dialog stays open and says why.
//!
//! The ≡ menu at the top saves the fields as a metadata template (`file.info.saveTemplate`), imports
//! one (`file.info.applyTemplate`, after a choice of how to merge it into the fields), lists the
//! templates of the metadata templates folder and shows that folder. The dialog files what needs the
//! session (listing, saving, merging, the folder) as requests that [`pump`] runs.

use std::path::PathBuf;

use craft::behavior::IconId;
use craft::host::{Host, PickId, PickKind, PickRequest};
use craft::theme::variant::Tone;
use craft::ui::buttons::{Button, IconButton};
use craft::ui::choice::{Radio, RadioGroup, Rating};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _, Trigger, clicked};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, HoldsOk, StackDialog, dialog_id};
use craft::ui::display::Divider;
use craft::ui::form::Field;
use craft::ui::layout_components::Stack;
use craft::ui::menu::{Menu, MenuItem};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::{FormCx, FormDialog};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::runtime::measure;
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::sections::GroupBox;
use craft::ui::text::{ChipField, SelectableText, TextField};
use serde_json::{Map, Value, json};
use vectorcraft_doc::CopyrightStatus;
use vectorcraft_doc::metadata::MAX_RATING;

use crate::{VectorcraftApp, file_asks};

/// The key of the File Info dialog on the host.
const KEY: &str = "fileInfo";
/// The keyword being typed, as automation sets and reads it.
const PENDING: &str = "__keyword";
/// The merge choices, as `file.info.applyTemplate` names them.
const MODES: [&str; 3] = ["replace", "keep", "append"];
/// The extension of a metadata template file.
const XMP_EXT: &str = "xmp";
/// What the host remembers the folder of a template pick under.
const PURPOSE: &str = "metadata-template";

/// What `file.info` reports and takes back: field name to value.
type Info = Map<String, Value>;

/// Open File Info on the active document's.
pub fn open(app: &mut VectorcraftApp) -> Result<Value, String> {
    let fields = app.session.execute("file.info", &json!({})).map_err(|e| e.to_string())?;
    let fields = fields.as_object().cloned().ok_or("file.info gave no fields")?;
    let document = app.session.active().ok_or("no document")?.uid;
    let ok = |app: &mut VectorcraftApp, (document, fields): &(u64, Info)| {
        if !app.session.active().is_some_and(|active| active.uid == *document) {
            return Err(tl!("File Info can only change the document it was opened for.").to_owned());
        }
        app.run("file.info", Value::Object(fields.clone())).map(|_| ())
    };
    crate::dialogs::host::open_held(app, FileInfoDialog::new(document, fields), ok).ok_or("no window")?;
    Ok(Value::Null)
}

/// The keywords in `fields`.
fn keywords(fields: &Info) -> Vec<String> {
    fields.get("keywords").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// `list` with each of `typed` appended unless the list holds it already, ignoring case: the one rule for
/// the chips the field commits and the draft OK keeps.
fn merge_keywords<'a>(mut list: Vec<String>, typed: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    for k in typed {
        if !list.iter().any(|o| o.to_lowercase() == k.to_lowercase()) {
            list.push(k.to_string());
        }
    }
    list
}

/// `fields` with the keywords typed in `draft` (comma- or semicolon-separated) added to the list.
fn with_keywords(fields: &Info, draft: &str) -> Info {
    let mut fields = fields.clone();
    let typed = draft.split([',', ';']).map(str::trim).filter(|k| !k.is_empty());
    let list = merge_keywords(keywords(&fields), typed);
    fields.insert("keywords".into(), json!(list));
    fields
}

/// The text under `key`, empty when there is none.
fn text_of(fields: &Info, key: &str) -> String {
    fields.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
}

/// A source for `applyTemplate`: `{key: value}`.
fn source(key: &str, value: String) -> Info {
    let mut source = Info::new();
    source.insert(key.into(), json!(value));
    source
}

/// What the dialog asks the session for, run by [`pump`].
enum Request {
    /// List the templates of the metadata templates folder.
    List,
    /// Show the metadata templates folder.
    Folder,
    /// Save the fields as a template at the path the save pick answered.
    SaveTo(PathBuf),
    /// Merge the template `source` into the fields as `mode` says.
    Import { source: Info, mode: &'static str },
}

/// The kind of file pick the Templates menu asks for.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pick {
    Save,
    Open,
}

/// What a click in the Templates menu chose.
enum Chosen {
    Pick(Pick),
    Template(String),
    Folder,
}

/// The Templates menu's state: what the app told the dialog about templates, the pick in flight and the requests for
/// [`pump`].
#[derive(Default)]
struct Templates {
    can_save: bool,
    can_open: bool,
    has_folder: bool,
    /// The metadata templates folder, where the save pick starts, once listed.
    folder: Option<String>,
    /// The listed templates: `(label, file)`, the label being the name, or the file name where two templates share a name.
    listed: Vec<(String, String)>,
    ticket: Option<(PickId, Pick)>,
    requests: Vec<Request>,
}

impl Templates {
    /// Takes the answer of the pick in flight: an imported template waits for its merge choice, a save path goes to the pump.
    fn answer(&mut self, cx: &FormCx<'_, '_>, import: &mut Option<Info>) {
        if let Some((id, kind)) = self.ticket
            && let Some(answer) = cx.take_pick_answer(id)
        {
            self.ticket = None;
            let Some(path) = answer.first_path() else { return };
            match kind {
                Pick::Open => *import = Some(source("path", path.to_string_lossy().into_owned())),
                Pick::Save => self.requests.push(Request::SaveTo(path)),
            }
        }
    }

    /// Files a pick of `kind`; a save suggests `title` as the name.
    fn pick(&mut self, cx: &FormCx<'_, '_>, kind: Pick, title: &str) {
        let Some(host) = cx.env.host().cloned() else { return };
        if let Some((old, _)) = self.ticket.take() {
            host.files().forget(old);
        }
        let request = match (kind, &self.folder) {
            (Pick::Open, _) => PickRequest::open(PURPOSE),
            (Pick::Save, folder) => {
                let request = PickRequest::save(PURPOSE).file_name(format!("{}.{XMP_EXT}", title.trim())).suggested_extension(XMP_EXT);
                match folder {
                    Some(folder) => request.directory(folder),
                    None => request,
                }
            }
        };
        let request = request.filter(tl!("XMP metadata templates"), &[XMP_EXT]);
        self.ticket = Some((host.files().pick(request), kind));
    }

    /// The ≡ menu at the top: Save Metadata Template…, Import…, the listed templates and Templates Folder.
    fn menu(&mut self, ui: &mut egui::Ui, cx: &FormCx<'_, '_>, title: &str, import: &mut Option<Info>) {
        let button = IconButton::new(IconId::new("menu")).label(tl!("Templates")).show(ui);
        // Listed once as the menu opens, not on every frame it shows.
        if clicked(&button) {
            self.requests.push(Request::List);
        }
        let mut chosen = None;
        let _ = Menu::new(tl!("Templates")).opened_by(&button, Trigger::Click).show(ui, |ui, _| {
            if self.can_save && MenuItem::new(tl!("Save Metadata Template…")).show(ui).ran {
                chosen = Some(Chosen::Pick(Pick::Save));
            }
            if self.can_open && MenuItem::new(tl!("Import…")).show(ui).ran {
                chosen = Some(Chosen::Pick(Pick::Open));
            }
            if !self.listed.is_empty() {
                Divider::horizontal().show(ui);
                for (label, file) in &self.listed {
                    if MenuItem::new(label.clone()).show(ui).ran {
                        chosen = Some(Chosen::Template(file.clone()));
                    }
                }
            }
            if self.has_folder {
                Divider::horizontal().show(ui);
                if MenuItem::new(tl!("Templates Folder")).show(ui).ran {
                    chosen = Some(Chosen::Folder);
                }
            }
        });
        match chosen {
            Some(Chosen::Pick(kind)) => self.pick(cx, kind, title),
            Some(Chosen::Template(file)) => wait_for_template(import, file),
            Some(Chosen::Folder) => self.requests.push(Request::Folder),
            None => {}
        }
    }
}

/// The File Info dialog: a form over the fields `file.info` reports, whose OK is held for the app.
pub(crate) struct FileInfoDialog {
    id: DialogId,
    document: u64,
    open: bool,
    /// OK files its answer for the app instead of closing ([`HoldsOk::held_ok`]).
    hold: bool,
    value: Info,
    /// The keyword being typed that automation set; OK keeps it with the chips.
    keyword: String,
    /// The template waiting for a merge choice: `{path}` or `{name}`.
    import: Option<Info>,
    /// The merge choice for [`import`](Self::import).
    import_mode: &'static str,
    templates: Templates,
    held: HeldAnswer<Info>,
}

impl FileInfoDialog {
    fn new(document: u64, value: Info) -> Self {
        FileInfoDialog {
            id: dialog_id(KEY),
            document,
            open: true,
            hold: false,
            value,
            keyword: String::new(),
            import: None,
            import_mode: "keep",
            templates: Templates::default(),
            held: HeldAnswer::new(),
        }
    }

    /// The fields OK files: the form's, with the typed keyword kept.
    fn filed_fields(&self) -> Info {
        with_keywords(&self.value, &self.keyword)
    }

    /// The requests filed since the last call.
    fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.templates.requests)
    }

    /// The rows of the form.
    fn rows(ui: &mut egui::Ui, fields: &mut Info) {
        for (key, label) in [("title", tl!("Document Title:")), ("author", tl!("Author:")), ("authorTitle", tl!("Author Title:"))] {
            text_row(ui, fields, key, label, 1);
        }
        text_row(ui, fields, "description", tl!("Description:"), 3);
        let mut stars = fields.get("rating").and_then(Value::as_u64).and_then(|r| u8::try_from(r).ok()).unwrap_or(0);
        let shown = stars;
        Field::new(tl!("Rating:")).show(ui, |ui, _| {
            Rating::new(Bind::bound(&mut stars), MAX_RATING).show(ui);
        });
        if stars != shown {
            fields.insert("rating".into(), json!(stars));
        }
        let mut list = keywords(fields);
        Field::new(tl!("Keywords:")).show(ui, |ui, _| {
            ChipField::new(tl!("Keywords:"), Bind::bound(&mut list)).show(ui);
        });
        let list = merge_keywords(Vec::new(), list.iter().map(String::as_str));
        if list != keywords(fields) {
            fields.insert("keywords".into(), json!(list));
        }
        GroupBox::new(tl!("Copyright"), None).show(ui, |ui, _| {
            let entries: Vec<Entry<String>> = CopyrightStatus::ALL.iter().map(|c| Entry::new(c.id().to_string(), c.label())).collect();
            let mut status = text_of(fields, "copyrightStatus");
            let shown = status.clone();
            Field::new(tl!("Copyright Status:")).show(ui, |ui, _| {
                Select::new(tl!("Copyright Status:"), Bind::bound(&mut status), &entries).show(ui);
            });
            if status != shown {
                fields.insert("copyrightStatus".into(), json!(status));
            }
            text_row(ui, fields, "copyrightNotice", tl!("Copyright Notice:"), 2);
            text_row(ui, fields, "copyrightUrl", tl!("Copyright Info URL:"), 1);
        });
        GroupBox::new(tl!("Dates"), None).show(ui, |ui, _| {
            for (key, label) in [("created", tl!("Created:")), ("modified", tl!("Modified:"))] {
                // ISO 8601 UTC from the engine, shown as "2026-10-05 14:03:00 UTC".
                let shown = fields
                    .get(key)
                    .and_then(Value::as_str)
                    .map_or_else(|| "—".to_string(), |s| format!("{} UTC", s.replace('T', " ").trim_end_matches('Z')));
                Field::new(label).show(ui, |ui, _| {
                    SelectableText::new(label, &shown).show(ui);
                });
            }
        });
    }
}

/// Waits for the merge choice of the listed template `file`; `file.info.applyTemplate` takes a listed template's
/// file name as its `name`.
fn wait_for_template(import: &mut Option<Info>, file: String) {
    *import = Some(source("name", file));
}

/// Picks the merge choice `name` names into `mode`; `false` when it is none of [`MODES`].
fn choose_mode(mode: &mut &'static str, name: &str) -> bool {
    let Some(chosen) = MODES.into_iter().find(|m| *m == name) else { return false };
    *mode = chosen;
    true
}

/// The merge choice's OK: asks [`pump`] to merge the waiting template into the fields.
fn ask_import(templates: &mut Templates, import: &Option<Info>, mode: &'static str) {
    if let Some(source) = import.clone() {
        templates.requests.push(Request::Import { source, mode });
    }
}

/// A text row `label` over the string under `key`, `lines` lines tall.
fn text_row(ui: &mut egui::Ui, fields: &mut Info, key: &str, label: &'static str, lines: usize) {
    let mut text = text_of(fields, key);
    let shown = text.clone();
    Field::new(label).show(ui, |ui, _| {
        let field = TextField::new(label, Bind::bound(&mut text));
        if lines > 1 {
            field.lines(lines).show(ui)
        } else {
            field.show(ui)
        };
    });
    if text != shown {
        fields.insert(key.into(), json!(text));
    }
}

/// The merge choice for a template waiting to be imported: `Some(true)` for OK, which merges it into the dialog's
/// fields (nothing changes in the document until the dialog's own OK), `Some(false)` for Cancel, which drops it.
fn import_options(ui: &mut egui::Ui, mode: &mut &'static str) -> Option<bool> {
    let labels = [
        tl!("Clear existing fields and use the template's"),
        tl!("Keep existing fields, but replace those the template has"),
        tl!("Keep existing fields, but append those the template has"),
    ];
    let mut picked = mode.to_string();
    let mut answer = None;
    GroupBox::new(tl!("Import Options"), None).show(ui, |ui, _| {
        RadioGroup::new(tl!("Import Options"), Bind::bound(&mut picked)).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                for (value, label) in MODES.into_iter().zip(labels) {
                    Radio::new(value.to_string()).label(label).show(ui);
                }
            });
        });
        if clicked(&Button::new(tl!("OK")).id_salt("import-ok").show(ui)) {
            answer = Some(true);
        }
        if clicked(&Button::new(tl!("Cancel")).id_salt("import-cancel").show(ui)) {
            answer = Some(false);
        }
    });
    choose_mode(mode, &picked);
    answer
}

impl HoldsOk for FileInfoDialog {
    type Answer = (u64, Info);

    fn held_ok(mut self) -> Self {
        self.hold = true;
        self
    }

    fn take_ok(&mut self) -> Option<Self::Answer> {
        self.held.take().map(|fields| (self.document, fields))
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }
}

impl StackDialog for FileInfoDialog {
    type Result = Info;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<Info>> {
        let FileInfoDialog { open, hold, value, keyword, import, import_mode, templates, held, .. } = self;
        // The app accepted the held OK: this pass closes the dialog with it. A measuring run closes nothing.
        let finish = held.is_accepted() && !measure::active(ui.ctx());
        let refusal = held.refusal().map(|(reason, refused)| (reason.clone(), refused.clone()));
        let waiting = import.is_some();
        let mut form = FormDialog::new(tl!("File Info"), Bind::bound(&mut *value))
            .opened(Bind::bound(open))
            .accept_stays_open(*hold)
            // OK and Enter wait while the app judges the last OK and while a template waits for its merge choice.
            .ok_enabled(!held.is_waiting() && !waiting);
        if waiting {
            form = form.ok_reason(tl!("Choose how to merge the template first."));
        }
        let mut answered = None;
        let shown = form.show(ui, |ui, cx, fields| {
            templates.answer(cx, import);
            templates.menu(ui, cx, &text_of(fields, "title"), import);
            Self::rows(ui, fields);
            if import.is_some() {
                answered = import_options(ui, import_mode);
            }
            let filed = with_keywords(fields, keyword);
            if let Some((reason, _)) = refusal.as_ref().filter(|(_, refused)| *refused == filed) {
                Notice::new(Tone::Bad, reason.clone()).layout(NoticeLayout::Inline).show(ui);
            }
            if finish {
                cx.request_close_as(CloseReason::Activated);
            }
            true
        });
        match answered {
            Some(true) => ask_import(templates, import, import_mode),
            Some(false) => *import = None,
            None => {}
        }
        let filed = with_keywords(value, keyword);
        if held.refusal().is_some_and(|(_, refused)| *refused != filed) {
            held.clear_refusal();
        }
        // The pass reports a press only in the pass that took the input, so a second pass of the frame files nothing.
        if *hold && shown.picked == Some(0) && !held.is_waiting() && import.is_none() {
            held.file(filed);
        }
        if finish && shown.closed == Some(CloseReason::Activated) {
            return Some(held.finish().map_or(DialogResult::Cancel, DialogResult::Ok));
        }
        let accepted = shown.accepted.filter(|_| !*hold).map(|fields| DialogResult::Ok(with_keywords(&fields, keyword)));
        accepted.or(shown.closed.map(|_| DialogResult::Cancel))
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    /// OK as the button: a held dialog files the fields and answers none, and answers the held fields once the app
    /// accepted them. None while a template waits for its merge choice.
    fn confirm(&mut self, _: &egui::Context) -> Option<Info> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if self.held.is_waiting() || self.import.is_some() {
            return None;
        }
        let fields = self.filed_fields();
        if self.hold {
            self.held.file(fields);
            return None;
        }
        Some(fields)
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    fn json_fields(&self) -> Option<Value> {
        let mut fields = self.value.clone();
        fields.insert(PENDING.into(), json!(self.keyword));
        Some(Value::Object(fields))
    }

    /// Sets a field of the form, or the keyword being typed. The dates are read only; `file.info` judges the values at OK.
    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        if field == PENDING {
            self.keyword = value.as_str().ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?.to_string();
            return Ok(());
        }
        if field.starts_with("__") {
            return Err(FieldRefusal::Unknown.message(field));
        }
        if matches!(field, "created" | "modified") {
            return Err(FieldRefusal::ReadOnly.message(field));
        }
        self.value.insert(field.to_string(), value);
        Ok(())
    }

    fn cancelled(&mut self, ctx: &egui::Context) {
        if let Some((id, _)) = self.templates.ticket.take()
            && let Some(host) = Host::try_of_ctx(ctx)
        {
            host.files().forget(id);
        }
    }
}

/// Whether the session has a metadata templates folder for Templates Folder to show (the web build
/// has none).
fn has_templates_folder(app: &VectorcraftApp) -> bool {
    app.session.metadata_templates_dir().is_some()
}

/// Templates Folder: the metadata templates folder (made when it is missing) in the file manager.
fn show_folder(app: &mut VectorcraftApp) -> Result<(), String> {
    let r = app.session.execute("file.info.templates", &json!({ "create": true })).map_err(|e| e.to_string())?;
    let folder = r["folder"].as_str().ok_or("no metadata templates folder here")?.to_string();
    crate::io::open_in_app(app, &folder)
}

/// The templates `file.info.templates` lists as `(label, file)`, and the folder they are in.
fn list_templates(app: &mut VectorcraftApp) -> (Vec<(String, String)>, Option<String>) {
    let r = app.session.execute("file.info.templates", &json!({})).unwrap_or_default();
    let listed: Vec<(&str, &str)> =
        r["templates"].as_array().into_iter().flatten().filter_map(|t| Some((t.get("name")?.as_str()?, t.get("file")?.as_str()?))).collect();
    let entries = listed
        .iter()
        .map(|&(name, file)| {
            let shared = listed.iter().filter(|(n, _)| *n == name).count() > 1;
            ((if shared { file } else { name }).to_string(), file.to_string())
        })
        .collect();
    (entries, r["folder"].as_str().map(str::to_string))
}

/// Save Metadata Template…: the fields as they are in the dialog, at the path a save pick answered. A name
/// typed without `.xmp` takes it, so the templates list shows the file; a file already there under the
/// completed name is replaced only after a question.
fn save_template(app: &mut VectorcraftApp, picked: PathBuf, fields: Info) -> Result<(), String> {
    let picked = picked.to_string_lossy().into_owned();
    let path = if picked.to_ascii_lowercase().ends_with(&format!(".{XMP_EXT}")) { picked.clone() } else { format!("{picked}.{XMP_EXT}") };
    // The save pick does not ask about the completed name.
    app.session.execute("file.info.templates", &json!({ "create": true })).map_err(|e| e.to_string())?;
    let save = {
        let path = path.clone();
        move |app: &mut VectorcraftApp| app.run("file.info.saveTemplate", json!({ "path": path, "fields": fields }))
    };
    if path != picked && std::path::Path::new(&path).exists() {
        file_asks::ask_replace(app, &path, &picked, save);
        return Ok(());
    }
    save(app).map(|_| ())
}

/// Merge the template `source` names into the dialog's fields as `mode` says.
fn import(app: &mut VectorcraftApp, d: &mut FileInfoDialog, source: Info, mode: &str) -> Result<(), String> {
    let mut p = Value::Object(source);
    p["mode"] = json!(mode);
    p["apply"] = json!(false);
    p["base"] = Value::Object(d.filed_fields());
    let merged = app.session.execute("file.info.applyTemplate", &p).map_err(|e| e.to_string())?;
    for (k, v) in merged.as_object().into_iter().flatten() {
        d.value.insert(k.clone(), v.clone());
    }
    d.keyword.clear();
    d.import = None;
    Ok(())
}

/// Runs what the open File Info dialog asked the session for, and tells it what the app can do with templates.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    // PLAN: fl.A9 lists no `file_info::pump`; the Templates menu needs the session, so hub-w2 adds this line to `dialogs::host::pump`.
    let Some(dialogs) = app.dialogs() else { return };
    let id = dialog_id(KEY);
    let (can_save, can_open, has_folder) = (file_asks::can(app, PickKind::Save), file_asks::can(app, PickKind::Open), has_templates_folder(app));
    let requests = dialogs.with(|host| {
        host.get_mut_by_id::<FileInfoDialog>(id).map(|d| {
            d.templates.can_save = can_save;
            d.templates.can_open = can_open;
            d.templates.has_folder = has_folder;
            d.take_requests()
        })
    });
    for request in requests.into_iter().flatten() {
        let done = match request {
            Request::List => {
                let (listed, folder) = list_templates(app);
                dialogs.with(|host| {
                    if let Some(d) = host.get_mut_by_id::<FileInfoDialog>(id) {
                        d.templates.listed = listed;
                        d.templates.folder = folder;
                    }
                });
                Ok(())
            }
            Request::Folder => show_folder(app),
            Request::SaveTo(path) => match dialogs.with(|host| host.get_by_id::<FileInfoDialog>(id).map(FileInfoDialog::filed_fields)) {
                Some(fields) => save_template(app, path, fields),
                None => Ok(()),
            },
            Request::Import { source, mode } => {
                dialogs.with(|host| host.get_mut_by_id::<FileInfoDialog>(id).map(|d| import(app, d, source, mode))).unwrap_or(Ok(()))
            }
        };
        if let Err(e) = done {
            app.status(e);
        }
    }
}

#[cfg(test)]
mod tests {
    use craft::ui::dialog::{HeldOk, dialog_id};
    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::{FileInfoDialog, KEY, PENDING};
    use crate::VectorcraftApp;

    /// An app with a window, so its dialogs have a host.
    fn windowed() -> VectorcraftApp {
        let mut app = VectorcraftApp::new(Session::new(), Default::default());
        // PLAN: files-w7's `scripted` gives the app its window context (`ui_ctx`) and a held file service.
        let _ = crate::tests::picks::scripted(&mut app);
        app
    }

    /// Runs `f` on the open File Info dialog.
    fn with_dialog<R>(app: &VectorcraftApp, f: impl FnOnce(&mut FileInfoDialog) -> R) -> Option<R> {
        app.dialogs()?.with(|host| host.get_mut_by_id::<FileInfoDialog>(dialog_id(KEY)).map(f))
    }

    /// Sets the dialog's field `field`, as automation does.
    fn set(app: &VectorcraftApp, field: &str, value: serde_json::Value) -> Result<(), String> {
        let ctx = app.ui_ctx.clone().unwrap_or_default();
        app.dialogs().unwrap().with(|host| host.set_json_field(&ctx, dialog_id(KEY), field, value))
    }

    /// Presses OK: the dialog files its fields, and the frames after it run the app's OK and close an accepted dialog
    /// (a second OK would file a refused answer again). Whether it closed.
    fn press_ok(app: &mut VectorcraftApp) -> bool {
        crate::dialogs::tests::confirm(app)
    }

    fn held_state(app: &VectorcraftApp) -> Option<HeldOk> {
        app.dialogs().unwrap().with(|host| host.held_state(dialog_id(KEY)))
    }

    #[test]
    fn file_info_draws_and_ok_is_one_undo_step() {
        let mut app = windowed();
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        // The menu item and its shortcut open the dialog; agents use ui.fileInfoDialog.
        crate::menus::invoke(&mut app, "file.info", json!({}));
        assert!(with_dialog(&app, |d| d.value["created"].is_string()).unwrap(), "a new document has its creation date");
        for (k, v) in [
            ("author", json!("Ada")),
            ("keywords", json!(["poster"])),
            ("rating", json!(4)),
            ("copyrightStatus", json!("copyrighted")),
            (PENDING, json!("print, Poster; blue")),
        ] {
            set(&app, k, v).unwrap();
        }
        assert!(set(&app, "__import", json!({})).is_err(), "the template wait is not an automation field");
        assert!(press_ok(&mut app));
        assert!(with_dialog(&app, |_| ()).is_none(), "OK closes the dialog");
        let st = app.session.active().unwrap();
        let m = &st.doc.metadata;
        assert_eq!((m.author.as_str(), m.rating, m.copyright_status), ("Ada", 4, vectorcraft_doc::CopyrightStatus::Copyrighted));
        assert_eq!(m.keywords, ["poster", "print", "blue"], "typed keywords join the chips, each once");
        assert_eq!(st.history.undo.len(), 1);
        // A bad value keeps the dialog open and says why.
        app.run("ui.fileInfoDialog", json!({})).unwrap();
        set(&app, "rating", json!(9)).unwrap();
        assert!(!press_ok(&mut app));
        assert!(matches!(held_state(&app), Some(HeldOk::Refused(_))), "{:?}", held_state(&app));
        assert!(with_dialog(&app, |_| ()).is_some());
    }

    #[test]
    fn a_file_info_confirmation_stays_with_its_document() {
        let mut app = windowed();
        for (title, author) in [("Source", "Ada"), ("Destination", "Grace")] {
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            app.run("file.info", json!({"title": title, "author": author})).unwrap();
        }
        let activate = |app: &mut VectorcraftApp, index| app.run("document.activate", json!({"index": index})).unwrap();
        activate(&mut app, 0);
        let a_undo = app.session.active().unwrap().history.undo.len();
        app.run("ui.fileInfoDialog", json!({})).unwrap();
        set(&app, "title", json!("Revised Source")).unwrap();
        set(&app, PENDING, json!("print")).unwrap();
        let ctx = app.ui_ctx.clone().unwrap();
        assert!(!app.dialogs().unwrap().with(|host| host.confirm_by_id(&ctx, dialog_id(KEY))), "the edit waits for the application");

        activate(&mut app, 1);
        let before = app.session.active().unwrap();
        let document = before.doc.clone();
        let b_undo = before.history.undo.len();
        crate::dialogs::host::pump(&mut app, &ctx);
        let after = app.session.active().unwrap();
        assert_eq!(after.doc.info_title(), document.info_title(), "the destination's title stays untouched");
        assert_eq!(after.doc.metadata, document.metadata, "the destination's metadata stays untouched");
        assert_eq!(after.history.undo.len(), b_undo);
        assert!(matches!(held_state(&app), Some(HeldOk::Refused(_))));

        activate(&mut app, 0);
        assert!(press_ok(&mut app), "a fresh confirmation can apply the original draft");
        let source = app.session.active().unwrap();
        assert_eq!(source.doc.info_title(), "Revised Source");
        assert_eq!(source.doc.metadata.author, "Ada");
        assert_eq!(source.doc.metadata.keywords, ["print"]);
        assert_eq!(source.history.undo.len(), a_undo + 1);
    }

    #[test]
    fn a_reopened_file_info_confirmation_uses_the_new_drafts_document() {
        let mut app = windowed();
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("file.new", json!({"width": 300, "height": 300})).unwrap();
        app.run("document.activate", json!({"index": 0})).unwrap();
        app.run("ui.fileInfoDialog", json!({})).unwrap();
        app.run("document.activate", json!({"index": 1})).unwrap();
        app.run("ui.fileInfoDialog", json!({})).unwrap();
        set(&app, "title", json!("Revised Destination")).unwrap();
        assert!(press_ok(&mut app));
        assert_eq!(app.session.active().unwrap().doc.info_title(), "Revised Destination");
        app.run("document.activate", json!({"index": 0})).unwrap();
        assert!(app.session.active().unwrap().history.undo.is_empty());
    }

    #[test]
    fn an_imported_template_fills_the_fields_and_ok_is_one_undo_step() {
        let dir = vectorcraft_testkit::TempDir::new("ui-file-info-template");
        let mut app = windowed();
        app.session.attach_metadata_templates_dir(dir.join("Metadata Templates"));
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("file.info.saveTemplate", json!({"name": "Studio", "fields": {"author": "Grace", "keywords": ["print"], "rating": 3}})).unwrap();
        app.run("ui.fileInfoDialog", json!({})).unwrap();
        set(&app, "author", json!("Ada")).unwrap();
        set(&app, PENDING, json!("fair")).unwrap();
        // A listed template waits for its merge choice; its OK asks the pump to merge it.
        with_dialog(&app, |d| {
            super::wait_for_template(&mut d.import, "Studio".into());
            assert!(super::choose_mode(&mut d.import_mode, "append"));
            super::ask_import(&mut d.templates, &d.import, d.import_mode);
        })
        .unwrap();
        let ctx = app.ui_ctx.clone().unwrap_or_default();
        super::pump(&mut app, &ctx);
        let (import_left, fields) = with_dialog(&app, |d| (d.import.is_some(), d.filed_fields())).unwrap();
        assert!(!import_left);
        // Appended: the typed author stays, the typed keyword is kept and the template's added.
        assert_eq!(
            (fields["author"].clone(), fields["keywords"].clone(), fields["rating"].clone()),
            (json!("Ada"), json!(["fair", "print"]), json!(3))
        );
        assert_eq!(app.session.active().unwrap().history.undo.len(), 0, "nothing changes before OK");
        assert!(press_ok(&mut app));
        let st = app.session.active().unwrap();
        assert_eq!((st.doc.metadata.author.as_str(), st.doc.metadata.rating), ("Ada", 3));
        assert_eq!(st.history.undo.len(), 1);
    }

    #[test]
    fn ok_waits_for_the_merge_choice_of_a_template_being_imported() {
        let mut app = windowed();
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.run("ui.fileInfoDialog", json!({})).unwrap();
        with_dialog(&app, |d| super::wait_for_template(&mut d.import, "Studio".into())).unwrap();
        assert!(!press_ok(&mut app));
        assert_eq!(held_state(&app), None, "OK and Enter wait while the merge choice is shown");
        with_dialog(&app, |d| d.import = None).unwrap();
        assert!(press_ok(&mut app), "OK files once the choice is answered");
    }
}
