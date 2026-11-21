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

// PLAN: assumed paths `craft_ui::patterns::{HostedPdfExport, PdfArtboards, PdfExportAnswer, PdfExportMode, PdfPresetEvent,
// PresetDraft}`, `craft_ui::patterns::presets::PresetError`, `craft_ui::dialog::DialogId`,
// `craft_ui::runtime::env::{Label, UnitKind}`, and `vectorcraft_pdf::{APP, ActiveProfiles, default_preset_name}`.
use std::sync::Arc;

use craft_ui::dialog::DialogId;
use craft_ui::patterns::presets::PresetError;
use craft_ui::patterns::{HostedPdfExport, PdfArtboards, PdfExportAnswer, PdfExportMode, PdfPresetEvent, PresetDraft};
use craft_ui::runtime::env::{Label, UnitKind};
use serde_json::{Map, Value, json};
use vectorcraft_doc::Unit;
use vectorcraft_engine::cmd::FlattenOptions;
use vectorcraft_engine::cmd::fileio::{SaveMode, pdf};
use vectorcraft_pdf::{APP, ActiveProfiles, PdfSettings, default_preset_name};

use crate::{VectorcraftApp, io};

/// The `pending` reply of a save that waits for the Save PDF dialog.
const PENDING: &str = "savePdf";
/// Why a Summary cannot be written without the dialog.
const NOT_OPEN: &str = "the Save PDF dialog is not open";

/// What the Save PDF dialog was opened for; the preset editor has none.
pub(crate) struct SavePdfJob {
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
}

/// Open the dialog with `params` (`document.exportPdf` options and `path?`) applied over their
/// preset. `useArtboards: false` makes the page the bounds of the visible art.
pub fn open(app: &mut VectorcraftApp, params: &Value) -> Result<Value, String> {
    open_with(app, params, false)
}

/// [`open`], for Export Selection when `selection` (the page is the selection's crop).
pub(crate) fn open_with(app: &mut VectorcraftApp, params: &Value, selection: bool) -> Result<Value, String> {
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
    let job = SavePdfJob { path: s("path").map(str::to_string), save_mode: None, flattener, selection };
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
    let (base, name, description, editing) = match s("name") {
        Some(name) => {
            let p = app.session.prefs.pdf_presets.iter().find(|p| p.name.eq_ignore_ascii_case(name.trim()));
            let p = p.ok_or_else(|| format!("no saved PDF preset named `{name}` (built-in presets are read-only: start a new one from them)"))?;
            (p.name.clone(), p.name.clone(), p.description.clone(), p.name.clone())
        }
        None => {
            let base = s("preset").map_or_else(|| default_preset_name(APP).to_string(), str::to_string);
            (base, app.session.new_pdf_preset_name(), String::new(), String::new())
        }
    };
    let settings = pdf::resolve("ui.savePdfDialog", &json!({ "preset": base }), &app.session.prefs.pdf_presets).map_err(|e| e.to_string())?;
    // A name may not be taken by a built-in preset or by another saved one (case and spaces ignored).
    let taken: Vec<String> = vectorcraft_pdf::builtin_presets()
        .into_iter()
        .map(|p| p.name)
        .chain(app.session.prefs.pdf_presets.iter().map(|p| p.name.clone()))
        .filter(|n| editing.is_empty() || !n.eq_ignore_ascii_case(editing.trim()))
        .collect();
    let dialog = base_dialog(app, settings)
        .preset(base)
        .mode(PdfExportMode::PresetEditor)
        .draft(PresetDraft { name, description })
        .check_name(move |n| {
            let n = n.trim();
            if n.is_empty() {
                Some(PresetError::EmptyName)
            } else if taken.iter().any(|t| t.eq_ignore_ascii_case(n)) {
                Some(PresetError::NameTaken)
            } else {
                None
            }
        });
    open_dialog(app, dialog, None, editing)
}

/// The dialog for `settings` with what the document and the app tell it.
fn base_dialog(app: &VectorcraftApp, settings: PdfSettings) -> HostedPdfExport {
    let session = &app.session;
    let prefs = &session.prefs;
    let doc = session.active().map(|s| &s.doc);
    // PLAN: assumed `vectorcraft_pdf::ColorMode` is the framework's, and `UnitKind` the dialog's unit kind.
    let color_mode = match doc.map(|d| d.color_mode) {
        Some(vectorcraft_doc::ColorMode::Cmyk) => vectorcraft_pdf::ColorMode::Cmyk,
        _ => vectorcraft_pdf::ColorMode::Rgb,
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
    app.save_pdf = Some(SavePdfOpen { id, job, editing, checked: None });
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
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(mut open) = app.save_pdf.take() else { return };
    let closed = drive(app, &mut open);
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
    // PLAN: assumed `HostedPdfExport::take_ok(&mut self) -> Option<PdfExportAnswer>`, the held OK as the other hosted dialogs have it.
    ok: Option<PdfExportAnswer>,
}

/// One frame of [`pump`] for `open`. True once the dialog has closed.
fn drive(app: &mut VectorcraftApp, open: &mut SavePdfOpen) -> bool {
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
        dialogs.with(|host| {
            if let Some(parent) = host.get_mut_by_id::<HostedPdfExport>(id) {
                let child = super::flatten::custom_dialog(parent, job, &saved);
                host.open_child(child, id);
            }
        });
    }
    if let Some(text) = filed.summary
        && let Err(e) = write_summary(app, None, &text)
        && e != "cancelled"
    {
        app.status(e);
    }
    let warnings = worded(check(app, open, &filed.settings));
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
            Some(job) => write(app, job, &answer),
            None => save_preset(app, &open.editing, &answer),
        };
        // PLAN: a write that asked a question (replace the file the document was opened from?) leaves OK waiting; its
        // continuation must answer the dialog, which needs a hook in io.
        if reply.as_ref().is_ok_and(|r| r.get("pending").is_some()) {
            return false;
        }
        let answer = reply.map(|_| ()).map_err(|e| Label::from(crate::i18n::message(crate::i18n::current(), &e)));
        dialogs.with(|host| {
            if let Some(d) = host.get_mut_by_id::<HostedPdfExport>(id) {
                d.answer_ok(answer);
            }
        });
    }
    false
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
        PdfPresetEvent::Renamed { from, to } => ("pdf.preset.rename", json!({ "name": from, "newName": to })),
        PdfPresetEvent::Deleted(name) => ("pdf.preset.delete", json!({ "name": name })),
        _ => return Ok(()),
    };
    app.run(command, params).map(|_| ())
}

/// The preset editor's OK: `pdf.preset.save`, then back to Edit → PDF Presets on it.
fn save_preset(app: &mut VectorcraftApp, editing: &str, answer: &PdfExportAnswer) -> Result<Value, String> {
    let draft = answer.draft.as_ref().ok_or("the preset editor gave no name")?;
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
            let warnings = summary_settings(app, settings).map(|v| {
                v["warnings"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
            });
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
    egui::Id::new((settings.to_string(), document, presets, shared))
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
    write_summary(app, path, &text?)
}

/// The Summary text of the open dialog `open`, after the document check.
fn summary_of(app: &mut VectorcraftApp, open: &mut SavePdfOpen) -> Result<String, String> {
    let (Some(dialogs), Some(ctx)) = (app.dialogs(), app.ui_ctx.clone()) else { return Err(NOT_OPEN.into()) };
    let id = open.id;
    let not_open = || NOT_OPEN.to_string();
    let settings = dialogs
        .with(|host| host.get_mut_by_id::<HostedPdfExport>(id).map(|d| Value::Object(settings_json(d.settings()))))
        .ok_or_else(not_open)?;
    let checked = check(app, open, &settings);
    if let Err(e) = checked {
        return Err(e.clone());
    }
    let warnings = worded(checked);
    dialogs
        .with(|host| {
            host.get_mut_by_id::<HostedPdfExport>(id).map(|d| {
                d.set_host_warnings(warnings);
                d.summary_text(&ctx)
            })
        })
        .ok_or_else(not_open)
}

/// Write the Summary's `text` (UTF-8) to `path`, else to a picked `<document> PDF Summary.txt`
/// next to the document → `{path, bytes}`.
fn write_summary(app: &mut VectorcraftApp, path: Option<String>, text: &str) -> Result<Value, String> {
    let title = app.session.active().map(|d| d.title()).unwrap_or_default();
    let stem = std::path::Path::new(&title).file_stem().map_or_else(|| "Untitled".into(), |s| s.to_string_lossy().into_owned());
    let path = io::write_named(app, path, &format!("{stem} PDF Summary.txt"), text.as_bytes())?;
    app.status(format!("Saved {path}"));
    Ok(json!({ "path": path, "bytes": text.len() }))
}

#[cfg(test)]
mod tests;
