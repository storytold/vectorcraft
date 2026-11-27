//! File → Save as PDF: the Save PDF dialog, a hosted `HostedPdfExport` that the window's dialog host
//! owns. The app keeps one open record ([`SavePdfOpen`]) for it; [`pump`] runs once per frame and
//! carries out what the dialog asked for: preset changes, Custom…, Save Summary… and OK.
//!
//! The settings are the `document.exportPdf` options, so agents fill the dialog through the
//! host's JSON fields (`ui.dialog.set`). OK runs `document.exportPdf` through [`io::export_pdf`];
//! the Summary is `document.pdfSettings`, and its Save Summary… writes it to a text file
//! (`ui.pdfSummary.save`). Opened from Export As with Use Artboards off, the dialog has no Artboards
//! group, and OK writes one page of the visible art's bounds through [`io::export`]; opened from
//! Export Selection, the page is the selection's crop.
//!
//! The same dialog edits a preset (from Edit → PDF Presets): a name and description instead of the
//! preset and artboards, and OK saves the preset.

use std::sync::Arc;

use craft::ui::dialog::{DialogHost, DialogId};
use craft::ui::patterns::form_dialogs::{
    HostedPdfExport, ParamResult, PdfArtboards, PdfExportAnswer, PdfExportMode, PdfExportWords, PdfPresetEvent, PresetDraft,
};
use craft::ui::patterns::presets::PresetError;
use craft::ui::runtime::env::{Label, UnitKind};
use serde_json::{Map, Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_engine::cmd::FlattenOptions;
use vectorcraft_engine::cmd::fileio::{SaveMode, pdf};
use vectorcraft_pdf::{APP, ActiveProfiles, PdfSettings, default_preset_name};

use super::host::{HeldOptions, open_held_with};
use crate::{VectorcraftApp, io};

/// The `pending` reply of a save that waits for the Save PDF dialog.
const PENDING: &str = "savePdf";
/// Why a Summary cannot be written without the dialog.
const NOT_OPEN: &str = "the Save PDF dialog is not open";
const WRONG_DOCUMENT: &str = "Return to the original drawing to save the PDF or its summary";

/// What the Save PDF dialog was opened for; the preset editor has none.
pub(crate) struct SavePdfJob {
    /// The drawing this job reads and writes.
    document: u64,
    /// The file the PDF goes to; none: the write asks.
    pub(crate) path: Option<String>,
    /// The save command (`file.saveAs`, `file.saveCopy`) this PDF finishes; none: a plain PDF copy.
    pub(crate) save_mode: Option<String>,
    /// The Custom… flattener options over the settings' flattener preset.
    pub(crate) flattener: Option<FlattenOptions>,
    /// Opened from Export Selection: the page is the selection's crop.
    pub(crate) selection: bool,
    // PLAN: the passwords live in the dialog's settings (a preset change clears them there), so the job holds no copy.
}

/// The open Save PDF dialog (Save PDF or the preset editor) and what the app remembers of it.
pub(crate) struct SavePdfOpen {
    /// The dialog on the window's host.
    pub(crate) id: DialogId,
    /// The save job; `None` in the preset editor.
    pub(crate) job: Option<SavePdfJob>,
    /// The saved preset the editor edits (empty: a new one).
    editing: String,
    /// The document check, with the key of everything it was computed from.
    checked: Option<(egui::Id, Result<Vec<String>, String>)>,
    /// The flattener preset the dialog showed at the last pump: choosing another drops the Custom… options.
    flattener_preset: Option<String>,
}

/// Open the dialog with `params` (`document.exportPdf` options and `path?`) applied over their
/// preset. `useArtboards: false` makes the page the bounds of the visible art.
pub fn open(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    open_with(app, params, false)
}

/// [`open`], for Export Selection when `selection` (the page is the selection's crop).
pub(crate) fn open_with(app: &mut VectorcraftApp, params: &Value, selection: bool) -> Result<Value, String> {
    let document = app.session.active().ok_or("no document")?.uid;
    let art_bounds = match params.get("useArtboards") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => !b,
        Some(v) => return Err(format!("useArtboards must be true or false, not {v}")),
    };
    // The PDF settings keep no flattener options: those asked for go over the flattener preset.
    let flattener = match params.get("flattener") {
        None | Some(Value::Null) => None,
        Some(f @ Value::Object(o)) => {
            let known = serde_json::to_value(FlattenOptions::default()).unwrap_or_default();
            if let Some(k) = o.keys().find(|k| known.get(k.as_str()).is_none()) {
                return Err(format!("flattener.{k} is not a flattener option"));
            }
            Some(FlattenOptions::for_export(None, Some(f), &[]).map_err(|e| format!("flattener: {e}"))?)
        }
        Some(v) => return Err(format!("flattener must be an object of flattener options, not {v}")),
    };
    let s = |k: &str| params.get(k).and_then(Value::as_str);
    let mut settings = pdf::resolve("ui.savePdfDialog", params, &app.session.prefs.pdf_presets).map_err(|e| e.to_string())?;
    // The flattener preset shows as the list names it: the export finds a name in any case.
    if !settings.flattener_preset.trim().is_empty() {
        let name = flattener_name(&settings.flattener_preset);
        if let Some(listed) = app.session.flattener_presets().into_iter().find(|p| p.name.eq_ignore_ascii_case(&name)) {
            settings.flattener_preset = listed.name;
        }
    }
    let artboards = match (art_bounds, s("range")) {
        (true, _) => PdfArtboards::ArtBounds,
        (false, Some(range)) => PdfArtboards::Range(range.to_string()),
        (false, None) => PdfArtboards::All,
    };
    let preset = s("preset").map_or_else(|| default_preset_name(APP).to_string(), str::to_string);
    let dialog = base_dialog(app, settings).preset(preset).artboards(artboards);
    let job = SavePdfJob { document, path: s("path").map(str::to_string), save_mode: None, flattener, selection };
    open_dialog(app, dialog, Some(job), String::new())
}

/// A save as PDF (`mode`) asks with the Save PDF dialog; its OK finishes the save ([`io::save_pdf`]).
pub(crate) fn open_for_save(app: &mut VectorcraftApp, params: &Value, mode: SaveMode) -> Result<Value, String> {
    open_with(app, params, false)?;
    if let Some(job) = app.save_pdf.as_mut().and_then(|o| o.job.as_mut()) {
        job.save_mode = Some(mode.command().to_string());
    }
    Ok(json!({ "pending": PENDING }))
}

/// Open the preset editor on the saved preset `name`, or on a new preset starting from `preset`
/// (default: the app default).
pub fn open_preset(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    let s = |k: &str| params.get(k).and_then(Value::as_str);
    let mut draft = PresetDraft::default();
    let (base, editing) = match s("name") {
        Some(name) => {
            let p = app.session.prefs.pdf_presets.iter().find(|p| p.name.eq_ignore_ascii_case(name.trim()));
            let p = p.ok_or_else(|| format!("no saved PDF preset named `{name}` (built-in presets are read-only: start a new one from them)"))?;
            (draft.name, draft.description) = (p.name.clone(), p.description.clone());
            (p.name.clone(), p.name.clone())
        }
        None => {
            let base = s("preset").map_or_else(|| default_preset_name(APP).to_string(), str::to_string);
            draft.name = app.session.new_pdf_preset_name();
            (base, String::new())
        }
    };
    let settings = pdf::resolve("ui.savePdfDialog", &json!({ "preset": base }), &app.session.prefs.pdf_presets).map_err(|e| e.to_string())?;
    let taken = taken_names(app, &editing);
    let dialog = base_dialog(app, settings).preset(base).mode(PdfExportMode::PresetEditor).draft(draft).check_name(move |n| name_error(&taken, n));
    open_dialog(app, dialog, None, editing)
}

/// The names a preset other than `editing` (empty: a new one) has: a built-in preset's or another saved one's.
fn taken_names(app: &VectorcraftApp, editing: &str) -> Vec<String> {
    vectorcraft_pdf::builtin_presets(APP)
        .into_iter()
        .map(|p| p.name)
        .chain(app.session.prefs.pdf_presets.iter().map(|p| p.name.clone()))
        .filter(|n| editing.is_empty() || !n.eq_ignore_ascii_case(editing.trim()))
        .collect()
}

/// Why the preset name `n` can't be used: empty, or one of `taken` (case and spaces ignored).
fn name_error(taken: &[String], n: &str) -> Option<PresetError> {
    let n = n.trim();
    if n.is_empty() {
        Some(PresetError::EmptyName)
    } else if taken.iter().any(|t| t.eq_ignore_ascii_case(n)) {
        Some(PresetError::NameTaken)
    } else {
        None
    }
}

/// The dialog for `settings` with what the document and the app tell it.
fn base_dialog(app: &VectorcraftApp, settings: PdfSettings) -> HostedPdfExport {
    let session = &app.session;
    let prefs = &session.prefs;
    let doc = session.active().map(|s| &s.doc);
    // `vectorcraft_pdf` leaves the framework's `ColorMode` out: the app names its own colour modes so.
    let color_mode = match doc.map(|d| d.color_mode) {
        Some(vectorcraft_doc::ColorMode::Cmyk) => craft::pdf::settings::ColorMode::Cmyk,
        _ => craft::pdf::settings::ColorMode::Rgb,
    };
    HostedPdfExport::new(APP, settings)
        .saved_presets(prefs.pdf_presets.clone())
        .flattener_presets(prefs.flattener_presets.iter().map(|p| p.name.clone()).collect())
        .profiles(vectorcraft_color::cms::profiles().into_iter().map(|p| p.name).collect())
        .catalog(Arc::new(ActiveProfiles::default()))
        .color_mode(color_mode)
        .length_unit(unit_kind(session.general_unit()))
        .document_bleed(doc.map_or([0.0; 4], |d| d.setup.bleed))
        .artboard_count(doc.map_or(0, |d| d.artboards.len()))
}

/// The dialog's unit kind for a document unit.
fn unit_kind(unit: Unit) -> UnitKind {
    match unit {
        Unit::Picas => UnitKind::Picas,
        Unit::Inches => UnitKind::Inches,
        Unit::Millimeters => UnitKind::Millimetres,
        Unit::Centimeters => UnitKind::Centimetres,
        Unit::Pixels => UnitKind::Pixels,
        Unit::Meters => UnitKind::Metres,
        Unit::Yards => UnitKind::Yards,
        Unit::Feet => UnitKind::Feet,
        _ => UnitKind::Points,
    }
}

/// Open `dialog` on the window's host, replacing an earlier Save PDF, and record it.
fn open_dialog(app: &mut VectorcraftApp, dialog: HostedPdfExport, job: Option<SavePdfJob>, editing: String) -> Result<Value, String> {
    let dialogs = app.dialogs().ok_or("no window to show the dialog in")?;
    let id = dialogs.with(|host| host.open(dialog));
    app.save_pdf = Some(SavePdfOpen { id, job, editing, checked: None, flattener_preset: None });
    Ok(Value::Null)
}

/// The settings as `document.exportPdf` params. Serialization omits the passwords, which the dialog
/// holds and the export needs.
fn settings_json(s: &PdfSettings) -> Map<String, Value> {
    // A plain settings struct always serializes; an empty object only stands in for the impossible.
    let Ok(Value::Object(mut m)) = serde_json::to_value(s) else { return Map::new() };
    if let Some(Value::Object(security)) = m.get_mut("security") {
        for (key, password) in [("openPassword", &s.security.open_password), ("permissionsPassword", &s.security.permissions_password)] {
            if !password.is_empty() {
                security.insert(key.into(), json!(password));
            }
        }
    }
    m
}

/// The name of the flattener preset `name` stands for: a built-in one named by id (`high`) as its name.
fn flattener_name(name: &str) -> String {
    let name = name.trim();
    FlattenOptions::preset_label(&name.to_ascii_lowercase()).unwrap_or(name).to_string()
}

/// Carry out what the open Save PDF dialog asked for this frame.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    let Some(mut open) = app.save_pdf.take() else { return };
    let closed = drive(app, ctx, &mut open);
    if !closed && app.save_pdf.is_none() {
        app.save_pdf = Some(open);
    }
}

/// What the dialog filed since the last frame.
struct Filed {
    settings: Value,
    events: Vec<PdfPresetEvent>,
    custom: bool,
    summary: Option<String>,
    ok: Option<PdfExportAnswer>,
}

/// One frame of [`pump`] for `open`. True once the dialog has closed.
fn drive(app: &mut VectorcraftApp, ctx: &egui::Context, open: &mut SavePdfOpen) -> bool {
    let Some(dialogs) = app.dialogs() else { return false };
    let id = open.id;
    let filed = dialogs.with(|host| {
        host.get_mut_by_id::<HostedPdfExport>(id).map(|d| Filed {
            settings: Value::Object(settings_json(d.settings())),
            events: d.take_preset_events(),
            custom: d.take_custom_flattener(),
            summary: d.take_save_summary(),
            ok: d.take_ok(),
        })
    });
    let Some(filed) = filed else {
        // Its answer is carried out already; the host keeps the result until it is taken.
        dialogs.with(|host| {
            host.take_by_id::<HostedPdfExport>(id);
        });
        return true;
    };
    for event in filed.events {
        if let Err(e) = preset_event(app, event) {
            app.status(e);
        }
    }
    if filed.custom
        && let Some(job) = &open.job
    {
        let saved = super::flatten::preset_rows(app).1;
        let child = dialogs.with(|host| host.get_by_id::<HostedPdfExport>(id).map(|parent| super::flatten::custom_dialog(ctx, parent, job, &saved)));
        if let Some(child) = child {
            open_held_with(app, child, custom_ok, HeldOptions { parent: Some(id), ..HeldOptions::default() });
        }
    }
    // A failed write is told over the dialog, which shows again when the message closes.
    let source = check_source(app, open);
    if let Some(text) = filed.summary {
        let saved = source.clone().and_then(|()| write_summary(app, Some(id), None, &text));
        if let Err(e) = saved
            && e != "cancelled"
        {
            app.status(crate::i18n::message(crate::i18n::current(), &e));
        }
    }
    let preset = filed.settings.get("flattenerPreset").and_then(Value::as_str).map(str::to_owned);
    if open.flattener_preset.is_some()
        && open.flattener_preset != preset
        && let Some(job) = open.job.as_mut()
    {
        job.flattener = None;
    }
    open.flattener_preset = preset;
    let settings = with_custom(open, filed.settings);
    let warnings = match source {
        Ok(()) => worded(check(app, open, &settings)),
        Err(e) => worded(&Err(e)),
    };
    let prefs = &app.session.prefs;
    let (presets, names) = (prefs.pdf_presets.clone(), prefs.flattener_presets.iter().map(|p| p.name.clone()).collect::<Vec<_>>());
    let custom = open.job.as_ref().is_some_and(|j| j.flattener.is_some());
    dialogs.with(|host| {
        if let Some(d) = host.get_mut_by_id::<HostedPdfExport>(id) {
            d.set_saved_presets(presets);
            d.set_flattener_presets(names);
            d.set_custom_flattener(custom);
            d.set_host_warnings(warnings);
        }
    });
    if let Some(answer) = filed.ok {
        let reply = match &open.job {
            Some(job) => check_source(app, open).and_then(|()| write(app, job, &answer)),
            None => save_preset(app, &open.editing, &answer),
        };
        // A write that asks first (the save panel, or whether to replace the file the document was opened from) closes
        // the dialog as a written one does; the question stays open, as in production.
        let answer = reply.map(|_| ()).map_err(|e| Label::from(crate::i18n::message(crate::i18n::current(), &e)));
        dialogs.with(|host| {
            if let Some(d) = host.get_mut_by_id::<HostedPdfExport>(id) {
                d.answer_ok(answer);
            }
        });
    }
    false
}

fn check_source(app: &VectorcraftApp, open: &SavePdfOpen) -> Result<(), String> {
    if let Some(job) = &open.job
        && app.session.active().map(|doc| doc.uid) != Some(job.document)
    {
        return Err(WRONG_DOCUMENT.into());
    }
    Ok(())
}

/// OK of Custom…: the open save flattens with its options ([`super::flatten::apply_custom`]); when the saved preset
/// they started from is gone, Save PDF's flattener preset shows no name.
fn custom_ok(app: &mut VectorcraftApp, answer: &ParamResult) -> Result<(), String> {
    let mut open = app.save_pdf.take().ok_or(NOT_OPEN)?;
    let saved = app.session.prefs.flattener_presets.clone();
    let id = open.id;
    let dialogs = app.dialogs().ok_or(NOT_OPEN)?;
    let applied = match open.job.as_mut() {
        Some(job) => dialogs
            .with(|host| host.get_by_id::<HostedPdfExport>(id).map(|parent| super::flatten::apply_custom(job, parent, &answer.fields, &saved)))
            .unwrap_or_else(|| Err(NOT_OPEN.to_string())),
        None => Err(NOT_OPEN.to_string()),
    };
    app.save_pdf = Some(open);
    if applied? {
        let ctx = app.ui_ctx.clone().ok_or(NOT_OPEN)?;
        dialogs.with(|host| host.set_json_field(&ctx, id, "flattenerPreset", json!("")))?;
        // The blanked name is no preset choice: the options just set stay.
        if let Some(open) = app.save_pdf.as_mut() {
            open.flattener_preset = Some(String::new());
        }
    }
    Ok(())
}

/// `ui.dialog.set` of a Custom… option, `flattener.<option>`, on Save PDF: its settings keep only the flattener
/// preset's name, so the option goes to the job, over the options it flattens with now. `None` for another request.
// shortcut: one `field` without `action`; a `fields` list goes to the framework, which refuses these names.
pub(crate) fn dialog_set(app: &mut VectorcraftApp, params: &Value) -> Option<Result<Value, String>> {
    let field = params.get("field")?.as_str()?;
    let key = field.strip_prefix("flattener.")?;
    if params.get("action").is_some() {
        return None;
    }
    let dialogs = app.dialogs()?;
    let saved = app.session.prefs.flattener_presets.clone();
    let open = app.save_pdf.as_mut()?;
    let id = open.id;
    let (top, name) =
        dialogs.with(|host| (host.open_dialogs().last().map(|d| d.id), host.get_by_id::<HostedPdfExport>(id).map(super::flatten::preset_name)));
    let named = (params["dialog"] == "top" && top == Some(id)) || params["dialog"] == id.value();
    let (true, Some(name), Some(job)) = (named, name, open.job.as_mut()) else { return None };
    let custom = job.flattener.as_ref().and_then(|f| serde_json::to_value(f).ok());
    let set = FlattenOptions::for_export(Some(&name), custom.as_ref(), &saved).and_then(|now| {
        let mut options = serde_json::to_value(now).map_err(|e| e.to_string())?;
        *options.get_mut(key).ok_or_else(|| format!("unknown field `{field}`"))? = params.get("value").cloned().unwrap_or(Value::Null);
        FlattenOptions::for_export(None, Some(&options), &[]).map_err(|e| format!("{field}: {e}"))
    });
    Some(set.map(|options| {
        job.flattener = Some(options);
        let shown = dialogs.with(|host| craft::automation::dialogs::inspect(host));
        shown.as_array().and_then(|list| list.iter().find(|d| d["id"] == id.value())).cloned().unwrap_or(Value::Null)
    }))
}

/// `ui.dialog.set` of a settings object (`bleed`, `marks`, `compression.color`…) on Save PDF or the preset editor,
/// the dialog `id`: the object sets only the options it holds, and the ones it leaves out keep the dialog's values. A
/// value that isn't an object is refused there. The fields are set in order, all or none, as the framework sets a
/// `fields` list. `None` for a request that sets no settings object on that dialog.
pub(crate) fn set_settings_objects(host: &mut DialogHost, ctx: &egui::Context, id: DialogId, params: &Value) -> Option<Result<(), String>> {
    let named = (params["dialog"] == "top" && host.open_dialogs().last().map(|d| d.id) == Some(id))
        || params["dialog"] == id.value()
        || params["dialog"].as_str() == Some(id.value().to_string().as_str());
    let known = params.as_object()?.keys().all(|k| matches!(k.as_str(), "dialog" | "field" | "value" | "fields"));
    if !named || !known {
        return None;
    }
    let item = |f: &Value| Some((f.get("field")?.as_str()?.to_owned(), f.get("value").cloned().unwrap_or(Value::Null)));
    let items: Vec<(String, Value)> = match params.get("fields") {
        Some(_) if params.get("field").is_some() || params.get("value").is_some() => return None,
        Some(list) => {
            list.as_array().filter(|l| l.len() <= craft::automation::dialogs::MAX_DIALOG_FIELDS)?.iter().map(item).collect::<Option<_>>()?
        }
        None => vec![item(params)?],
    };
    let settings = |host: &DialogHost| host.get_by_id::<HostedPdfExport>(id).and_then(|d| serde_json::to_value(d.settings()).ok());
    let now = settings(host)?;
    if !items.iter().any(|(field, _)| option_at(&now, field).is_some_and(Value::is_object)) {
        return None;
    }
    let before = dialog_fields(host, id);
    let passwords = host.get_by_id::<HostedPdfExport>(id).map(|d| {
        let security = &d.settings().security;
        [("security.openPassword", security.open_password.clone()), ("security.permissionsPassword", security.permissions_password.clone())]
    });
    let mut changed = false;
    for (field, value) in &items {
        let set = match settings(host).as_ref().and_then(|s| option_at(s, field)).filter(|v| v.is_object()) {
            Some(_) if !value.is_object() => Err(format!("`{field}` must be an object of options, not {value}")),
            Some(old) => {
                let mut merged = old.clone();
                merge_options(&mut merged, value);
                host.set_json_field(ctx, id, field, merged)
            }
            None => host.set_json_field(ctx, id, field, value.clone()),
        };
        if let Err(e) = set {
            // A refused field leaves the dialog as it was, so only the fields set before it go back.
            if changed {
                restore_fields(host, ctx, id, before, passwords);
            }
            return Some(Err(e));
        }
        changed = true;
    }
    Some(Ok(()))
}

/// The JSON fields of the open dialog `id`.
fn dialog_fields(host: &DialogHost, id: DialogId) -> Option<Value> {
    host.open_dialogs().into_iter().find(|d| d.id == id).and_then(|d| d.fields)
}

/// The option at the dotted `path` (`compression.color`) of the settings `settings`.
fn option_at<'a>(settings: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(settings, |at, key| at.get(key))
}

/// Set the options `over` holds on `base`, object by object; any other value replaces `base`.
fn merge_options(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(slot) => merge_options(slot, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// Set the dialog `id`'s fields that differ from `before` back to it. The preset goes first, since choosing it
/// replaces every setting and clears the passwords. The `passwords` the dialog had, which its fields leave out, are set
/// back last when they differ.
fn restore_fields(host: &mut DialogHost, ctx: &egui::Context, id: DialogId, before: Option<Value>, passwords: Option<[(&str, String); 2]>) {
    let Some(Value::Object(before)) = before else { return };
    let preset = before.get("preset").filter(|old| dialog_fields(host, id).as_ref().and_then(|now| now.get("preset")) != Some(*old));
    if let Some(old) = preset {
        let _ = host.set_json_field(ctx, id, "preset", old.clone());
    }
    // The standard goes next, since it enables and sets other fields. A field that another one still disables is set
    // again in a later pass, once that one is back; one that stays disabled keeps its value, as the framework's own
    // rollback leaves it.
    let rest = || before.get_key_value("standard").into_iter().chain(before.iter().filter(|(k, _)| !matches!(k.as_str(), "preset" | "standard")));
    for _ in 0..3 {
        let now = dialog_fields(host, id).unwrap_or(Value::Null);
        let differ: Vec<_> = rest().filter(|(name, old)| now.get(name.as_str()) != Some(*old)).collect();
        if differ.is_empty() {
            break;
        }
        for (name, old) in differ {
            let _ = host.set_json_field(ctx, id, name, old.clone());
        }
    }
    let now = host
        .get_by_id::<HostedPdfExport>(id)
        .map(|d| [d.settings().security.open_password.clone(), d.settings().security.permissions_password.clone()]);
    for ((name, password), now) in passwords.into_iter().flatten().zip(now.into_iter().flatten()) {
        if password != now {
            let _ = host.set_json_field(ctx, id, name, Value::String(password));
        }
    }
}

/// The dialog's `settings` with the job's Custom… flattener options, as the export gets them.
fn with_custom(open: &SavePdfOpen, mut settings: Value) -> Value {
    if let Some(options) = open.job.as_ref().and_then(|job| job.flattener.as_ref()).and_then(|f| serde_json::to_value(f).ok()) {
        settings["flattener"] = options;
    }
    settings
}

/// Carry out a change the dialog made to the saved presets.
fn preset_event(app: &mut VectorcraftApp, event: PdfPresetEvent) -> Result<(), String> {
    // PLAN: assumed `PdfPreset { name, description, settings }` and the params `name`/`newName` of `pdf.preset.rename`.
    let (command, params) = match event {
        PdfPresetEvent::Saved(preset) => {
            let mut p = settings_json(&preset.settings);
            p.insert("name".into(), json!(preset.name));
            p.insert("description".into(), json!(preset.description));
            ("pdf.preset.save", Value::Object(p))
        }
        PdfPresetEvent::Renamed { from, to, .. } => ("pdf.preset.rename", json!({ "name": from, "newName": to })),
        PdfPresetEvent::Deleted(name) => ("pdf.preset.delete", json!({ "name": name })),
        _ => return Ok(()),
    };
    app.run(command, params).map(|_| ())
}

/// The preset editor's OK: `pdf.preset.save`, then back to Edit → PDF Presets on it.
fn save_preset(app: &mut VectorcraftApp, editing: &str, answer: &PdfExportAnswer) -> Result<Value, String> {
    let draft = answer.draft.as_ref().ok_or("the preset editor gave no name")?;
    // The editor's OK is off on such a name; an OK by automation is refused here.
    match name_error(&taken_names(app, editing), &draft.name) {
        Some(PresetError::NameTaken) => return Err("A preset with this name already exists.".into()),
        Some(_) => return Err("This name can't be used.".into()),
        None => {}
    }
    let mut p = settings_json(&answer.result.options);
    p.insert("description".into(), json!(draft.description));
    if editing.is_empty() {
        p.insert("name".into(), json!(draft.name));
    } else {
        p.insert("name".into(), json!(editing));
        p.insert("newName".into(), json!(draft.name));
    }
    let r = app.run("pdf.preset.save", Value::Object(p))?;
    super::pdf_presets::open(app, r["name"].as_str());
    Ok(r)
}

/// Write the file (a PDF copy, or the PDF a Save As / Save a Copy asked for); the dialog stays open
/// when that fails (bad range, cancelled save…).
fn write(app: &mut VectorcraftApp, job: &SavePdfJob, answer: &PdfExportAnswer) -> Result<Value, String> {
    let mut p = settings_json(&answer.result.options);
    match &answer.result.artboards {
        PdfArtboards::Range(range) if !range.trim().is_empty() => {
            p.insert("range".into(), json!(range.trim()));
        }
        PdfArtboards::ArtBounds => {
            p.insert("useArtboards".into(), json!(false));
        }
        _ => {}
    }
    if let Some(path) = &job.path {
        p.insert("path".into(), json!(path));
    }
    if let Some(flattener) = &job.flattener {
        p.insert("flattener".into(), serde_json::to_value(flattener).map_err(|e| e.to_string())?);
    }
    let art_bounds = answer.result.artboards == PdfArtboards::ArtBounds;
    let p = Value::Object(p);
    match job.save_mode.as_deref().and_then(SaveMode::of) {
        Some(mode) => io::save_pdf(app, mode, p),
        None if art_bounds => export_art(app, p, job.selection),
        None => io::export_pdf(app, p),
    }
}

/// A page of the art's bounds (Export As with Use Artboards off), or of the selection's crop
/// (Export Selection): written as Export As writes other formats ([`io::export`]), with the
/// dialog's options and a saved preset written out.
fn export_art(app: &mut VectorcraftApp, mut p: Value, selection: bool) -> Result<Value, String> {
    let path = p.as_object_mut().and_then(|o| o.remove("path")).and_then(|v| v.as_str().map(str::to_string));
    let p = pdf::expand_preset(&app.session, "file.exportAs", &p).map_err(|e| e.to_string())?.into_owned();
    io::export(app, Some("pdf"), path, &super::export_as::exported(selection, p))
}

/// The document's warnings for `settings` (`document.pdfSettings` with an export in memory), kept
/// until a field, the document, the saved PDF or flattener presets, the loaded color profiles or the
/// available fonts change.
fn check<'a>(app: &mut VectorcraftApp, open: &'a mut SavePdfOpen, settings: &Value) -> &'a Result<Vec<String>, String> {
    let key = summary_key(app, settings);
    if open.checked.as_ref().is_some_and(|(k, _)| *k != key) {
        open.checked = None;
    }
    &open
        .checked
        .get_or_insert_with(|| {
            let warnings = summary_settings(app, settings)
                .map(|v| v["warnings"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default());
            (key, warnings)
        })
        .1
}

/// The stamp of what [`check`] reads: the `settings`, the active document and its revision, the saved
/// PDF presets (which `preset` can name), the saved flattener presets (which `flattenerPreset` can
/// name), the loaded color profiles (which `output.destination` and `output.outputIntent` can name)
/// and the available fonts (which decide what is embedded or outlined).
fn summary_key(app: &VectorcraftApp, settings: &Value) -> egui::Id {
    let document = app.session.active().map(|s| (s.uid, s.revision));
    let prefs = &app.session.prefs;
    let presets = serde_json::to_string(&(&prefs.pdf_presets, &prefs.flattener_presets)).unwrap_or_default();
    let shared = (vectorcraft_color::cms::profiles_generation(), vectorcraft_text::FontDb::global().generation());
    egui::Id::unique((settings.to_string(), document, presets, shared))
}

/// The host warnings of a document check: its warnings, or the error that refused it, in the
/// interface language.
fn worded(checked: &Result<Vec<String>, String>) -> Vec<String> {
    let lang = crate::i18n::current();
    match checked {
        Ok(warnings) => warnings.iter().map(|w| crate::i18n::message(lang, w)).collect(),
        Err(e) => vec![crate::i18n::message(lang, e)],
    }
}

/// `document.pdfSettings` for the dialog's `document.exportPdf` params `p`, with the document's
/// warnings (an export in memory).
fn summary_settings(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let mut q = p.clone();
    q["includeDocument"] = json!(true);
    app.session.execute("document.pdfSettings", &q).map_err(|e| e.to_string())
}

/// The UI command that writes the Summary to a text file.
pub(crate) const SAVE_SUMMARY: &str = "ui.pdfSummary.save";

/// Is a dialog with the Summary open (Save PDF, or the preset editor)?
pub(crate) fn summary_open(app: &VectorcraftApp) -> bool {
    app.dialogs().is_some_and(|d| d.with(|host| host.is_open::<HostedPdfExport>()))
}

/// `ui.pdfSummary.save {path}`: write the open Save PDF dialog's (or preset editor's) Summary as
/// text to `path` (the web downloads it under the path's file name). The check of the document is
/// the one the dialog shows, and a document that fails it refuses the write. The dialog stays open
/// and the document is not changed → `{path, bytes}`. A missing, empty or non-string `path` is an
/// error that writes nothing.
pub(crate) fn save_summary(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let key = "path";
    let path = match p.get(key) {
        None | Some(Value::Null) => return Err("missing `path`".into()),
        Some(Value::String(s)) if s.trim().is_empty() => return Err("path is empty".into()),
        Some(Value::String(s)) => Some(s.clone()),
        Some(_) => return Err(format!("`{key}` must be a string")),
    };
    let mut open = app.save_pdf.take().ok_or(NOT_OPEN)?;
    let text = summary_of(app, &mut open);
    if app.save_pdf.is_none() {
        app.save_pdf = Some(open);
    }
    write_summary(app, None, path, &text?)
}

/// The Summary text of the open dialog `open`, after the document check.
fn summary_of(app: &mut VectorcraftApp, open: &mut SavePdfOpen) -> Result<String, String> {
    check_source(app, open)?;
    let Some(dialogs) = app.dialogs() else { return Err(NOT_OPEN.into()) };
    let id = open.id;
    let not_open = || NOT_OPEN.to_string();
    let settings =
        dialogs.with(|host| host.get_mut_by_id::<HostedPdfExport>(id).map(|d| Value::Object(settings_json(d.settings())))).ok_or_else(not_open)?;
    let settings = with_custom(open, settings);
    let checked = check(app, open, &settings);
    if let Err(e) = checked {
        return Err(e.clone());
    }
    let warnings = worded(checked);
    let words = app.host.words().set::<PdfExportWords>();
    dialogs
        .with(|host| {
            host.get_mut_by_id::<HostedPdfExport>(id).map(|d| {
                d.set_host_warnings(warnings);
                d.summary_text(&words)
            })
        })
        .ok_or_else(not_open)
}

/// Write the Summary's `text` (UTF-8) to `path`, else to a picked `<document> PDF Summary.txt`
/// next to the document → `{path, bytes}`; a failed write is told over the dialog `over`, the one it was asked from.
fn write_summary(app: &mut VectorcraftApp, over: Option<DialogId>, path: Option<String>, text: &str) -> Result<Value, String> {
    let title = app.session.active().map(|d| d.title()).unwrap_or_default();
    let stem = std::path::Path::new(&title).file_stem().map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned());
    let (name, bytes) = (format!("{stem} PDF Summary.txt"), text.len());
    let saved = move |app: &mut VectorcraftApp, path: String| {
        app.status(format!("Saved {path}"));
        Ok(json!({ "path": path, "bytes": bytes }))
    };
    match over {
        Some(parent) => io::write_named_over(app, parent, path, &name, text.as_bytes().to_vec(), saved),
        None => io::write_named(app, path, &name, text.as_bytes().to_vec(), saved),
    }
}

#[cfg(test)]
mod tests;
