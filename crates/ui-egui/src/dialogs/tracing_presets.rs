//! Edit → Tracing Presets: every tracing preset in a list (built-in ones in brackets, protected,
//! then the saved ones) with the selected one's settings under it; New… (a preset starting from
//! the selected one), Edit… and Delete (saved presets), Import… and Export… (every saved preset).
//! New… and Edit… open the preset editor (kind `tracingPreset`): its OK saves the preset with
//! `imageTrace.presets.save` and selects it here, and its Cancel saves nothing. An import whose
//! file holds presets with the names of saved ones asks whether to replace them (kind
//! `tracingPresetsReplace`); Cancel keeps the saved ones. The saved presets are the ones the Image
//! Trace panel, the Control bar and Properties list.
//!
//! Fields: `selected` (a preset's name). The editor's: `name`, `params` (the tracing settings, as
//! `imageTrace.presets.save` takes them) and `editing` (the saved preset it edits; empty for a new
//! one). The question's: `choice`. Opening a `.vctracepresets` file (File › Open, a drop, the
//! Import… pick) imports it the same way.

use std::borrow::Cow;
use std::sync::Arc;

use craft::behavior::{DisplayOrder, ItemSource, Selection, SelectionChange};
use craft::host::{PickRequest, Picked};
use craft::ui::buttons::Button;
use craft::ui::contract::SelectMode;
use craft::ui::contract::{Bind, CollectionEvent, Props as _, Show as _, ShowCollection as _, ShowContainer as _, clicked};
use craft::ui::display::Text;
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
// PLAN: assumed `craft::ui::patterns::form_dialogs::FieldValue` and `craft::ui::runtime::env::Label` paths.
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, PromptChoice, StackDialog, StandardPrompt, dialog_id};
use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue, HostedParamDialog, Param, ParamDialog, ParamKind, ParamResult};
use craft::ui::runtime::env::Label;
use craft::ui::sections::{DialogRole, FooterButton};
use serde_json::{Value, json};
use vectorcraft_engine::cmd::{TRACE_PRESET_EXTS, TRACE_PRESET_FORMAT, TRACE_PRESET_MAX_BYTES};

use super::host::open_held;
use crate::VectorcraftApp;
use crate::file_asks::{self, Owner};
use crate::panels::image_trace::rows::{self, RowsData, RowsHost};
use crate::panels::image_trace::{self, Preset, PresetList};

/// The dialog kind of Tracing Presets.
pub(crate) const KIND: &str = "tracingPresets";

/// The dialog kind of the tracing preset editor.
pub(crate) const PRESET_KIND: &str = "tracingPreset";

/// The dialog kind of the question whether an import replaces saved presets of the same names.
pub(crate) const REPLACE_KIND: &str = "tracingPresetsReplace";

/// The rows the preset list shows before it scrolls.
const LIST_ROWS: u16 = 10;

/// What a button asked for, done by [`pump`] once the dialog is drawn.
enum Action {
    Select(String),
    New,
    Edit,
    Delete,
    Import,
    Export,
}

/// The presets as list rows, keyed by position.
struct Labels<'a>(&'a [String]);

impl DisplayOrder<usize> for Labels<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|name| Cow::Borrowed(name.as_str()))
    }
}

impl ItemSource<usize> for Labels<'_> {}

/// The presets manager: the list, its buttons and the selected preset's settings.
pub(crate) struct Manager {
    /// Every preset, as the Image Trace panel lists them.
    list: Arc<PresetList>,
    /// The selected preset's name, as given (a built-in one may come in brackets).
    selected: String,
    /// What the settings rows list: libraries and color groups.
    data: RowsData,
    /// What a button asked for, taken by [`pump`].
    action: Option<Action>,
}

impl StackDialog for Manager {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id(KIND)
    }

    fn key(&self) -> Option<&str> {
        Some(KIND)
    }

    fn json_fields(&self) -> Option<Value> {
        Some(json!({ "selected": self.selected }))
    }

    fn set_json_field(&mut self, ctx: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        if field != "selected" {
            return Err(FieldRefusal::Unknown.message(field));
        }
        self.selected = value.as_str().map(str::to_owned).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
        ctx.request_repaint();
        Ok(())
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let list = Arc::clone(&self.list);
        let at = shown(&list.presets, &self.selected);
        let Some(preset) = list.presets.get(at) else { return Some(DialogResult::Cancel) };
        let (builtin, any_saved) = (preset.3, list.presets.iter().any(|p| !p.3));
        let labels: Vec<String> = list.presets.iter().map(list_label).collect();
        let (data, action) = (&self.data, &mut self.action);
        let dialog = Dialog::new(tl!("Tracing Presets")).buttons([FooterButton::new(tl!("Close"), DialogRole::Cancel)]);
        let shown = dialog.show(ui, |ui, _| {
            Text::new(tl!("Presets:")).show(ui);
            let out = List::new(tl!("Presets"))
                .select(SelectMode::Single)
                .max_rows(LIST_ROWS)
                .selection(Bind::given(Selection::single(at)))
                .show_items(ui, &Labels(&labels), &mut |&k, _| ListRow::new(labels[k].clone()));
            for event in out.events {
                if let CollectionEvent::Selection { change: SelectionChange::Set(keys), .. } = event
                    && let Some(p) = keys.last().and_then(|&k| list.presets.get(k))
                {
                    *action = Some(Action::Select(p.0.clone()));
                }
            }
            Row::new().show(ui, |ui, _| {
                let buttons = [
                    (Button::new(tl!("New…")).tooltip(tl!("A new preset starting from the selected one")), Action::New, true),
                    (Button::new(tl!("Edit…")), Action::Edit, !builtin),
                    (Button::new(tl!("Delete")), Action::Delete, !builtin),
                    (Button::new(tl!("Import…")), Action::Import, true),
                    (Button::new(tl!("Export…")), Action::Export, any_saved),
                ];
                for (button, act, enabled) in buttons {
                    if clicked(&button.enabled(enabled).show(ui)) {
                        *action = Some(act);
                    }
                }
            });
            // PLAN: the sheet's two columns and a strong name need layout and text variants that are not in the bundle; stacked and plain here.
            Text::new(list_label(preset)).show(ui);
            // The selected preset's settings, as the editor shows them, to read only: edits land in a copy.
            let mut params = preset.1.clone();
            // PLAN: the rows components take no `enabled` prop; the scope's `Ui` is disabled for them.
            ui.scope(|ui| {
                ui.disable();
                rows::main_rows(ui, &mut params, data, RowsHost::Dialog);
                rows::advanced_rows(ui, &mut params, data, RowsHost::Dialog, None);
            });
            if builtin {
                Text::new(tl!("Built-in presets don't change: New makes a copy you can edit.")).show(ui);
            }
        });
        shown.output.result(|| ())
    }
}

/// Open the presets manager on `selected` (default: the first preset, Default).
pub(crate) fn open(app: &mut VectorcraftApp, selected: Option<&str>) {
    let manager =
        Manager { list: image_trace::presets(app), selected: selected.unwrap_or("Default").to_string(), data: RowsData::of(app), action: None };
    match app.dialogs() {
        Some(dialogs) => {
            dialogs.with(|host| host.open(manager));
        }
        None => app.status("no window".to_string()),
    }
}

/// Runs `f` on the open manager; `None` while it is closed.
fn with_manager<R>(app: &VectorcraftApp, f: impl FnOnce(&mut Manager) -> R) -> Option<R> {
    app.dialogs()?.with(|host| host.get_mut::<Manager>().map(f))
}

/// Selects the preset `name` in the manager, when it is open.
fn select(app: &VectorcraftApp, name: &str) {
    // A closed manager has nothing to select in.
    let _ = with_manager(app, |m| m.selected = name.to_string());
}

/// The index in `all` of the preset `selected` names, found as the engine finds presets (the first
/// when none).
fn shown(all: &[Preset], selected: &str) -> usize {
    image_trace::find_preset(all, selected).and_then(|p| all.iter().position(|q| std::ptr::eq(p, q))).unwrap_or(0)
}

/// A preset's name as the list shows it: a built-in one translated and in brackets, a saved one as
/// the user named it.
fn list_label(p: &Preset) -> String {
    if p.3 { format!("[{}]", image_trace::preset_label(&p.0)) } else { p.0.clone() }
}

/// Once per frame: does what a manager button asked for, keeps the manager's list current, and
/// acts on the answer of the Replace question.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    answer_replace(app);
    let Some((action, current)) = with_manager(app, |m| (m.action.take(), m.current())) else { return };
    if let Some(action) = action {
        if let Err(e) = run(app, action, &current) {
            app.status(e);
        }
        let data = RowsData::of(app);
        let _ = with_manager(app, |m| m.data = data);
    }
    let list = image_trace::presets(app);
    let _ = with_manager(app, |m| m.list = list);
}

impl Manager {
    /// The name of the selected preset.
    fn current(&self) -> String {
        self.list.presets.get(shown(&self.list.presets, &self.selected)).map(|p| p.0.clone()).unwrap_or_default()
    }
}

/// Does `act` on the preset `current`.
fn run(app: &mut VectorcraftApp, act: Action, current: &str) -> Result<(), String> {
    match act {
        Action::Select(name) => select(app, &name),
        Action::New => open_editor(app, current, false)?,
        Action::Edit => open_editor(app, current, true)?,
        Action::Delete => {
            let i = app.session.prefs.trace_presets.iter().position(|p| p.name == current);
            app.run("imageTrace.presets.delete", json!({ "name": current }))?;
            // The saved preset below it takes its place, else the one above it, else Default.
            let saved = &app.session.prefs.trace_presets;
            let next = i.and_then(|i| saved.get(i).or_else(|| i.checked_sub(1).and_then(|k| saved.get(k)))).map(|p| p.name.clone());
            select(app, next.as_deref().unwrap_or("Default"));
        }
        Action::Import => {
            let request = PickRequest::open("tracingPresets").filter("Tracing presets", TRACE_PRESET_EXTS);
            // The answer, or its failure, reaches the status bar from the continuation; nothing is pending here.
            let _ = file_asks::ask(app, request, Owner::Free, picked);
        }
        Action::Export => {
            crate::io::save_command_output(app, "imageTrace.presets.export", TRACE_PRESET_FORMAT, json!({}), |_, _| Ok(Value::Null))?;
        }
    }
    Ok(())
}

/// The file Import… picked: a path on the desktop, the file's bytes on the web.
fn picked(app: &mut VectorcraftApp, picked: Picked) -> Result<Value, String> {
    match picked {
        Picked::Paths(paths) => {
            if let Some(path) = paths.first() {
                open_path(app, &path.to_string_lossy())?;
            }
        }
        Picked::Contents(files) => {
            if let Some((_, bytes)) = files.first() {
                open_file(app, bytes)?;
            }
        }
        _ => {}
    }
    Ok(Value::Null)
}

/// Import the tracing presets file `file` (`{path}` or `{dataBase64}`) into the saved presets,
/// keeping saved presets of the same names → the presets the file added and the names it skipped.
fn import_file(app: &mut VectorcraftApp, file: &Value) -> Result<(Value, Vec<Value>), String> {
    let r = crate::io::import_presets(app, "imageTrace.presets.import", "tracing presets", file.clone())?;
    let skipped = r["skipped"].as_array().cloned().unwrap_or_default();
    Ok((r["imported"].clone(), skipped))
}

/// What the Replace question carries back: the file it was asked about, so Replace acts on the
/// contents the first import checked, and the preset to select afterwards.
struct ReplaceFile {
    file: Value,
    first: Option<String>,
}

/// The Replace question. Its own type, so the host keeps its answer apart from every other prompt's.
pub(crate) struct ReplacePresets(pub(crate) StandardPrompt);

crate::dialogs::confirm::wrapped_dialog!(ReplacePresets, StandardPrompt, REPLACE_KIND);

/// Asks whether the import of `file` replaces the saved presets `skipped` kept, over the manager
/// when it is open.
fn ask_replace(app: &VectorcraftApp, skipped: &[Value], file: Value) {
    let names: Vec<Label> = skipped.iter().filter_map(Value::as_str).map(|n| Label::from(n.to_string())).collect();
    let first = skipped.first().and_then(Value::as_str).map(str::to_owned);
    let prompt = StandardPrompt::custom(tl!("Replace the saved presets that have these names?"))
        .lines(names)
        .button(PromptChoice::Proceed, tl!("Replace"))
        .button(PromptChoice::Cancel, tl!("Cancel"))
        .tag(ReplaceFile { file, first });
    if let Some(dialogs) = app.dialogs() {
        dialogs.with(|host| {
            if host.is_open::<Manager>() {
                host.open_child(ReplacePresets(prompt), dialog_id(KIND));
            } else {
                host.open(ReplacePresets(prompt));
            }
        });
    }
}

/// Replace: the file imports again, replacing the saved presets of its names, and the manager
/// selects the first of them.
fn answer_replace(app: &mut VectorcraftApp) {
    let Some(dialogs) = app.dialogs() else { return };
    // The host lock is released before any command runs.
    let Some(mut answer) = dialogs.with(|host| host.take::<ReplacePresets>()).and_then(|result| result.ok()) else { return };
    if answer.choice != PromptChoice::Proceed {
        return;
    }
    let Some(ReplaceFile { mut file, first }) = answer.take_tag::<ReplaceFile>() else { return };
    file["replace"] = json!(true);
    match app.run("imageTrace.presets.import", file) {
        Ok(_) => {
            if let Some(first) = first {
                select(app, &first);
            }
        }
        Err(e) => app.status(e),
    }
}

/// The error for a tracing presets file over the size limit.
fn too_big() -> String {
    format!("the tracing presets file is larger than {} MiB", TRACE_PRESET_MAX_BYTES >> 20)
}

/// The tracing presets file `file` as `{dataBase64}`: a `{path}` is read here, once, so the
/// question and its Replace act on the contents the first import checked.
fn file_contents(app: &VectorcraftApp, file: Value) -> Result<Value, String> {
    let Some(path) = file.get("path").and_then(Value::as_str) else { return Ok(file) };
    vectorcraft_engine::file_access::check_read(path)?;
    if std::fs::metadata(path).is_ok_and(|m| m.len() > TRACE_PRESET_MAX_BYTES as u64) {
        return Err(too_big());
    }
    // One byte past the limit tells a larger file apart without reading all of it.
    // PLAN: assumed the files service of files-w5/w6 offers this bounded read; `app.services` stands in until it lands.
    let bytes = match (&app.services.read_head, &app.services.read) {
        (Some(read_head), _) => read_head(path, TRACE_PRESET_MAX_BYTES + 1)?,
        (None, Some(read)) => read(path)?,
        (None, None) => return Err("no file reader".into()),
    };
    if bytes.len() > TRACE_PRESET_MAX_BYTES {
        return Err(too_big());
    }
    Ok(json!({ "dataBase64": vectorcraft_format::base64_encode(&bytes) }))
}

/// A tracing presets file opened (File › Open, a drop, or the file Import… picked): it imports as
/// Import… does, over the manager when it is open (which selects the first preset it added), and
/// asks whether to replace saved presets of the file's names.
pub(crate) fn open_file(app: &mut VectorcraftApp, bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > TRACE_PRESET_MAX_BYTES {
        return Err(too_big());
    }
    open_import(app, json!({ "dataBase64": vectorcraft_format::base64_encode(bytes) }))
}

/// [`open_file`] for the tracing presets file at `path` (File › Open, a drop, Import…): the file is
/// read no further than one byte past the size limit.
pub(crate) fn open_path(app: &mut VectorcraftApp, path: &str) -> Result<(), String> {
    let file = file_contents(app, json!({ "path": path }))?;
    open_import(app, file)
}

/// The import of a tracing presets file `file` (`{dataBase64}`), over the manager when it is open.
fn open_import(app: &mut VectorcraftApp, file: Value) -> Result<(), String> {
    let (imported, skipped) = import_file(app, &file)?;
    if let Some(first) = imported.get(0).and_then(Value::as_str) {
        select(app, first);
    }
    if !skipped.is_empty() {
        ask_replace(app, &skipped, file);
    }
    Ok(())
}

/// The preset editor over the manager: on the saved preset `name` (`edit`), else on a new preset
/// starting from the preset `name`, under the first free "Tracing Preset N".
fn open_editor(app: &mut VectorcraftApp, name: &str, edit: bool) -> Result<(), String> {
    let (editing, new_name, params) = if edit {
        let saved = app.session.trace_preset(name).ok_or_else(|| format!("no saved preset named `{name}`"))?;
        (saved.name.clone(), saved.name.clone(), serde_json::to_value(&saved.params).map_err(|e| e.to_string())?)
    } else {
        let params = app.session.execute("imageTrace.presets", &json!({})).map_err(|e| e.to_string())?["presets"]
            .as_array()
            .and_then(|all| {
                let found = |p: &&Value| p["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name));
                all.iter().find(|p| p["builtIn"] != false && found(p)).or_else(|| all.iter().find(found)).map(|p| p["params"].clone())
            })
            .ok_or_else(|| format!("no preset named `{name}`"))?;
        (String::new(), app.session.new_trace_preset_name(), params)
    };
    let ctx = app.ui_ctx.clone().ok_or("no window")?;
    let fields = DialogFields::from_pairs([
        ("name", FieldValue::Text(new_name)),
        ("params", FieldValue::Json(params)),
        ("editing", FieldValue::Text(editing.clone())),
    ]);
    let dialog = HostedParamDialog::new(&ctx, PRESET_KIND, fields, build(Arc::new(RowsData::of(app)))).held_ok();
    // A refusal keeps the editor open with the reason under its rows.
    let ok = move |app: &mut VectorcraftApp, r: &ParamResult| {
        let params = match r.fields.get("params") {
            Some(FieldValue::Json(v)) => v.clone(),
            _ => Value::Null,
        };
        let saved = save_preset(app, r.fields.text("name").unwrap_or_default(), &editing, params)?;
        select(app, saved["name"].as_str().unwrap_or_default());
        Ok(())
    };
    open_held(app, dialog, ok).map(|_| ()).ok_or_else(|| "no window".to_string())
}

/// The editor's dialog over its values: the name and the tracing settings, as the Image Trace
/// panel shows them.
fn build(data: Arc<RowsData>) -> impl for<'b> Fn(&'b mut DialogFields) -> ParamDialog<'b> + Send + 'static {
    move |fields| {
        let named = !fields.text("name").unwrap_or_default().trim().is_empty();
        let data = Arc::clone(&data);
        let settings = Param::custom("params", "", move |ui, fields, _| {
            let mut params = match fields.get("params") {
                Some(FieldValue::Json(v)) => v.clone(),
                _ => json!({}),
            };
            let shown = ui.scope(|ui| {
                let main = rows::main_rows(ui, &mut params, &data, RowsHost::Dialog);
                let advanced = rows::advanced_rows(ui, &mut params, &data, RowsHost::Dialog, None);
                main.changed || advanced.changed
            });
            if shown.inner {
                fields.set("params", FieldValue::Json(params));
            }
            shown.response
        });
        ParamDialog::new(PRESET_KIND, tl!("Save Preset"), vec![Param::new("name", tl!("Name:"), ParamKind::text("")), settings])
            .values(Bind::bound(fields))
            .ok_enabled(named)
    }
}

/// Save Preset (the editor's OK): `imageTrace.presets.save`. A preset may not take the name of a
/// built-in preset, of Custom or of another saved preset: the editor stays open to change it.
fn save_preset(app: &mut VectorcraftApp, name: &str, editing: &str, params: Value) -> Result<Value, String> {
    let name = name.trim();
    let p = if editing.is_empty() {
        if app.session.trace_preset(name).is_some() {
            return Err(format!("a preset named `{name}` exists"));
        }
        // The settings are whole: a new preset starts from none of the saved ones.
        json!({ "name": name, "preset": "Default", "params": params })
    } else {
        json!({ "name": editing, "newName": name, "params": params })
    };
    app.run("imageTrace.presets.save", p)
}

#[cfg(test)]
mod tests;
