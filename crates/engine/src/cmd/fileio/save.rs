//! Saving: `document.save`, `file.saveAs`, `file.saveCopy`, `file.saveAsTemplate`,
//! `file.newFromTemplate`, `file.revert` and `file.formatOptions`.
//!
//! Every frontend saves the same way: [`save_plan`] resolves where and how (path, format, options,
//! a suggested name and folder when there is no path), then [`save_with`] encodes, writes through
//! the caller's writer (the file system here; a save panel or a browser download in the apps) and
//! makes the file the document's own (path, format, options, saved state) for Save and Save As.

use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_doc::Document;

use super::super::*;
use super::{
    ArtboardPick, Encoded, Format, Loaded, encode_all, file_stem, format, format_for_name, load, read_file, with_compression_pref, write_encoded,
    write_file,
};
use crate::{DocState, Prefs};

/// The formats Save As offers, in menu order (append-only). The other writable formats are exports:
/// they never become the document's own file.
pub const SAVE_FORMATS: &[&str] = &["vectorcraft", "template", "pdf", "svg", "svgz", "ai"];

/// Save-panel filters `(label, [extension])` for `first` (a format id, or a file name whose
/// extension names one): when Save writes that format, one per [`SAVE_FORMATS`] entry with it
/// leading (the panel's default type), else none (an export picks its own). Only the extension a
/// save writes (never the former native name).
pub fn save_filters(first: &str) -> Vec<(&'static str, &'static [&'static str])> {
    let Some(first) = format_for_name(first).or_else(|| format(first)).filter(|f| SAVE_FORMATS.contains(&f.id)) else { return vec![] };
    let mut v: Vec<&'static Format> = SAVE_FORMATS.iter().filter_map(|id| format(id)).collect();
    v.sort_by_key(|f| f.id != first.id);
    v.into_iter().map(|f| (f.label, &f.extensions[..1])).collect()
}

/// How a save treats the document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaveMode {
    /// File → Save: the document's own path and format.
    Save,
    /// File → Save As: a new path or format, which the document takes on.
    SaveAs,
    /// File → Save a Copy: the document keeps its path, title and modified state.
    Copy,
    /// File → Save as Template: a template copy; the document keeps its path and state too.
    Template,
}

impl SaveMode {
    /// The engine command of this mode.
    pub fn command(self) -> &'static str {
        match self {
            Self::Save => "document.save",
            Self::SaveAs => "file.saveAs",
            Self::Copy => "file.saveCopy",
            Self::Template => "file.saveAsTemplate",
        }
    }

    /// The mode an engine command id runs.
    pub fn of(command: &str) -> Option<Self> {
        [Self::Save, Self::SaveAs, Self::Copy, Self::Template].into_iter().find(|m| m.command() == command)
    }
}

/// A save resolved against the active document, before anything is encoded or written.
#[derive(Clone, Debug)]
pub struct SavePlan {
    pub mode: SaveMode,
    /// Where to write. `None` when no path is known (never saved, converted from an older version,
    /// restored by Data Recovery, or a format other than the document's own): the caller asks for one or takes the bytes.
    pub path: Option<String>,
    pub format: &'static Format,
    /// The format's options (only those its encoder reads).
    pub options: Map<String, Value>,
    /// Suggested file name: `<name>.<ext>`, `<name> copy.<ext>` or `<name> template.vctemplate`.
    pub name: String,
    /// Suggested folder: the document's own, or the Templates folder for a template.
    pub folder: Option<String>,
    /// The `modified` param (as `date_param` reads it): the File Info date a Save or Save As to a file
    /// stamps; `None`: the time it is written.
    pub modified: Option<Option<i64>>,
}

impl SavePlan {
    /// Does writing this make the file the document's own (Save and Save As, except to a
    /// template)?
    pub fn retargets(&self) -> bool {
        matches!(self.mode, SaveMode::Save | SaveMode::SaveAs) && self.format.id != "template"
    }
}

/// File Info's dates for a save to a file: the modified date (and the created date, when it has
/// none) become `at`. Not an undo step.
pub fn stamp_save_dates(st: &mut DocState, at: i64) {
    let d = std::sync::Arc::make_mut(&mut st.doc);
    d.metadata.created.get_or_insert(at);
    d.metadata.modified = Some(at);
}

/// The Templates folder: the `templatesFolder` preference, else `VectorCraft Templates` in the
/// user's documents folder (none where there is no home folder, as on the web).
pub fn templates_folder(prefs: &Prefs) -> Option<String> {
    if !prefs.templates_folder.is_empty() {
        return Some(prefs.templates_folder.clone());
    }
    Some(documents_folder()?.join("VectorCraft Templates").to_string_lossy().to_string())
}

/// The Templates folder for a file dialog to start in (#703): made when it's missing; where it
/// can't be, the nearest folder above it that exists, else the home folder. A dialog asked to
/// start in a folder that isn't there shows an error on some desktops.
pub fn templates_dialog_folder(prefs: &Prefs) -> Option<String> {
    let folder = std::path::PathBuf::from(templates_folder(prefs)?);
    if !folder.is_dir() {
        // Best effort: a folder that can't be made leaves the nearest one that exists.
        let _ = std::fs::create_dir_all(&folder);
    }
    folder.ancestors().find(|a| a.is_dir()).map(|a| a.to_string_lossy().into_owned()).or_else(home_folder)
}

/// The user's documents folder: on Linux and the BSDs the one the desktop names
/// (`XDG_DOCUMENTS_DIR` in `user-dirs.dirs`: `~/Documenti`, `~/Dokumente`…), elsewhere `Documents`
/// in the home folder.
fn documents_folder() -> Option<std::path::PathBuf> {
    let home = std::path::PathBuf::from(home_folder()?);
    if cfg!(all(unix, not(target_os = "macos"))) {
        let config =
            std::env::var_os("XDG_CONFIG_HOME").map(std::path::PathBuf::from).filter(|p| p.is_absolute()).unwrap_or_else(|| home.join(".config"));
        if let Some(dir) = std::fs::read_to_string(config.join("user-dirs.dirs")).ok().and_then(|t| xdg_documents(&t, &home)) {
            return Some(dir);
        }
    }
    Some(home.join("Documents"))
}

/// `XDG_DOCUMENTS_DIR` in a `user-dirs.dirs` file's `text`, `$HOME` being `home`. None when it isn't
/// set, is relative, or is the home folder itself (the spec's way of turning it off).
fn xdg_documents(text: &str, home: &std::path::Path) -> Option<std::path::PathBuf> {
    let line = text.lines().map(str::trim).find(|l| l.starts_with("XDG_DOCUMENTS_DIR="))?;
    let value = line.split_once('=')?.1.trim().trim_matches('"');
    let dir = match value.strip_prefix("$HOME") {
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => std::path::PathBuf::from(value),
    };
    (dir.has_root() && dir != home).then_some(dir)
}

/// The user's home folder (`HOME`, else `USERPROFILE` on Windows; none on the web).
fn home_folder() -> Option<String> {
    std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")).ok().filter(|h| !h.is_empty())
}

/// Where exports go by default (Export for Screens): the user's Desktop when there is one, else
/// their home folder (none on the web, which downloads).
pub fn export_folder() -> Option<String> {
    let home = home_folder()?;
    let desktop = std::path::Path::new(&home).join("Desktop");
    Some(if desktop.is_dir() { desktop.to_string_lossy().into_owned() } else { home })
}

/// The folder of a path (`None` for a bare file name).
fn parent_folder(path: &str) -> Option<String> {
    std::path::Path::new(path).parent().map(|p| p.to_string_lossy().to_string()).filter(|p| !p.is_empty())
}

/// A save format by id or extension, or why it isn't one.
fn save_format_of(f: &str) -> std::result::Result<&'static Format, String> {
    match format(f) {
        Some(f) if SAVE_FORMATS.contains(&f.id) => Ok(f),
        Some(f) => Err(format!("Save writes {}, not {}: use document.export for other formats", SAVE_FORMATS.join(", "), f.label)),
        None => Err(format!("unknown format `{f}` (Save writes {})", SAVE_FORMATS.join(", "))),
    }
}

/// The format Save writes for `format` (an id or extension), else the format `path`'s extension
/// names, else native (see [`SAVE_FORMATS`]); other formats are exports.
pub fn save_format(format: Option<&str>, path: Option<&str>) -> std::result::Result<&'static Format, String> {
    save_format_of(format.or_else(|| path.and_then(format_for_name).map(|f| f.id)).unwrap_or("vectorcraft"))
}

/// [`save_format_of`] for command `cmd`.
fn checked(cmd: &str, f: &str) -> Result<&'static Format> {
    save_format_of(f).map_err(|e| bad(cmd, e))
}

/// Resolve a save of the active document from `{path?, format?, options?}` (see the commands'
/// params docs).
pub fn save_plan(s: &Session, mode: SaveMode, p: &Value) -> Result<SavePlan> {
    let cmd = mode.command();
    let st = s.doc()?;
    let path = str_param(p, "path").map(str::to_string);
    let format = match (mode, str_param(p, "format"), path.as_deref().and_then(format_for_name)) {
        (SaveMode::Template, ..) => checked(cmd, "template")?,
        (_, Some(f), _) => checked(cmd, f)?,
        (_, None, Some(f)) => checked(cmd, f.id)?,
        (_, None, None) => checked(cmd, st.format)?,
    };
    let mut given: Map<String, Value> = match p.get("options") {
        Some(Value::Object(o)) => o.iter().filter(|(k, _)| reads_option(format, k)).map(|(k, v)| (k.clone(), v.clone())).collect(),
        None | Some(Value::Null) => Map::new(),
        Some(_) => return Err(bad(cmd, "options must be an object (see file.formatOptions)")),
    };
    if is_svg(format) {
        // SVG options also come flat or as `svg: {…}` beside the path; kept flat.
        let mut svg = super::svg_options(&Value::Object(given)).map_err(|e| bad(cmd, e))?;
        svg.extend(super::svg_options(p).map_err(|e| bad(cmd, e))?);
        given = svg;
    } else {
        // So do PDF options (as the Save PDF dialog and document.exportPdf name them) and the
        // native ones (compress, version, preview).
        if let Some(o) = p.as_object() {
            given.extend(
                o.iter().filter(|(k, v)| !v.is_null() && k.as_str() != "path" && reads_option(format, k)).map(|(k, v)| (k.clone(), v.clone())),
            );
        }
    }
    // None given: the ones the document was last saved with in this format.
    let options = if given.is_empty() && format.id == st.format { st.save_options.clone() } else { given };
    // Save writes the document's own file; the other modes only where they are told to.
    let path =
        path.or_else(|| (mode == SaveMode::Save && !st.converted && !st.recovered && format.id == st.format).then(|| st.path.clone()).flatten());
    let stem = file_stem(st.path.as_deref().unwrap_or(&st.doc.title));
    let ext = format.extensions[0];
    let name = match mode {
        SaveMode::Copy => format!("{stem} copy.{ext}"),
        SaveMode::Template => format!("{stem} template.{ext}"),
        SaveMode::Save | SaveMode::SaveAs => format!("{stem}.{ext}"),
    };
    let folder = match mode {
        SaveMode::Template => templates_dialog_folder(&s.prefs),
        _ => st.path.as_deref().and_then(parent_folder),
    };
    let modified = date_param(p, "modified", cmd)?;
    if let Some(path) = &path {
        super::check_not_lossy_overwrite(st, path, p, cmd)?;
    }
    Ok(SavePlan { mode, path, format, options, name, folder, modified })
}

/// Does a save in `f` read option `key`? PDF (and a .ai file, a PDF) also reads its General
/// settings (`createLayers`…) and presets.
fn reads_option(f: &Format, key: &str) -> bool {
    f.options.iter().any(|o| o.name == key) || (is_pdf(f) && (super::pdf::OPTIONS.iter().any(|o| o.name == key) || super::pdf::is_setting(key)))
}

/// Formats that carry the whole native document: they lose nothing, and record links relative to
/// where they are written. A `.ai` file is a PDF carrying it.
fn is_native(f: &Format) -> bool {
    matches!(f.id, "vectorcraft" | "template" | "ai")
}

fn is_pdf(f: &Format) -> bool {
    matches!(f.id, "pdf" | "ai")
}

fn is_svg(f: &Format) -> bool {
    matches!(f.id, "svg" | "svgz")
}

/// The document as written: native files carry the view to reopen at and the Layers panel's open
/// rows.
fn doc_to_save(st: &DocState, f: &Format) -> Arc<Document> {
    if !is_native(f) {
        return st.doc.clone();
    }
    let open = st.layers_open.saved(&st.doc);
    if st.doc.last_view == st.view && st.doc.layers_open == open {
        return st.doc.clone();
    }
    let mut d = (*st.doc).clone();
    d.last_view = st.view.clone();
    d.layers_open = open;
    Arc::new(d)
}

/// What a format loses against a native file (reported whenever a save writes it).
fn fidelity_warning(f: &Format) -> Option<String> {
    (!is_native(f)).then(|| {
        format!(
            "{} keeps the artwork but not everything a VectorCraft document holds (editable effects, symbols, swatches, styles): save as VectorCraft to keep it all editable",
            f.label
        )
    })
}

/// A save of the active document, snapshotted and ready to encode and write anywhere (Background
/// Save does both on a worker thread): [`save_job`], then [`SaveJob::write`], then
/// [`SaveJob::finish`]. Cheap to clone (the documents are shared).
#[derive(Clone)]
pub struct SaveJob {
    plan: SavePlan,
    /// The document as written (with the view to reopen at, links relative to the file).
    doc: Arc<Document>,
    /// The encoder's params.
    params: Value,
    /// The document when the job was made: what counts as saved once the file is written.
    snapshot: Arc<Document>,
    /// The artboards also saved to files of their own (`separateArtboards`).
    boards: Vec<usize>,
}

/// The artboards a native or `.ai` save also writes to files of their own: with
/// `separateArtboards`, those `range` names (default all).
fn separate_boards(cmd: &str, f: &Format, doc: &Document, p: &Value) -> Result<Vec<usize>> {
    if !is_native(f) || !bool_or(p, "separateArtboards", false) {
        return Ok(vec![]);
    }
    let range = str_param(p, "range").filter(|r| !r.trim().is_empty()).map(str::to_string);
    let n = doc.artboards.len();
    let picked = ArtboardPick { range, ..Default::default() }.resolve(n).map_err(|e| bad(cmd, e))?;
    Ok(picked.unwrap_or_else(|| (0..n).collect()))
}

/// `doc` reduced to artboard `i` and the art on it (Save each artboard to a separate file):
/// objects whose bounds touch the artboard, in their layers; resources are kept.
fn artboard_doc(doc: &Document, i: usize) -> Document {
    let mut d = doc.clone();
    let Some(a) = doc.artboards.get(i) else { return d };
    let r = a.rect;
    d.artboards = vec![a.clone()];
    // It opens fitted to its artboard, not at the master file's view.
    d.last_view = None;
    d.layers_open = None;
    for layer in &mut d.layers {
        keep_on(layer, r);
    }
    d
}

/// Keep in layer `n` (and its sublayers) only the objects touching `r`.
fn keep_on(n: &mut Arc<vectorcraft_doc::Node>, r: vectorcraft_geom::Rect) {
    let touches = |b: vectorcraft_geom::Rect| b.x0 <= r.x1 && b.x1 >= r.x0 && b.y0 <= r.y1 && b.y1 >= r.y0;
    if let Some(children) = Arc::make_mut(n).children_mut() {
        children.retain(|c| c.is_layer() || c.visual_bounds().is_some_and(touches));
        for c in children.iter_mut().filter(|c| c.is_layer()) {
            keep_on(c, r);
        }
    }
}

/// `doc` with empty layers: the pages of a `.ai` file saved without PDF content.
fn blank_pages(doc: &Document) -> Document {
    let mut d = doc.clone();
    for layer in &mut d.layers {
        if let Some(children) = Arc::make_mut(layer).children_mut() {
            children.clear();
        }
    }
    d
}

/// Why a `.ai` file saved without PDF content looks empty elsewhere.
const NOT_PDF_COMPATIBLE: &str = "saved without PDF content: VectorCraft opens it as before, other apps show empty pages";

/// The shared Save and Export encoder for PDF-compatible `.ai` files.
pub(super) fn encode_ai(cmd: &str, f: &Format, doc: &Document, p: &Value) -> Result<Encoded> {
    let mut params = if p.is_object() { p.clone() } else { json!({}) };
    if let Some(o) = params.as_object_mut() {
        o.insert("preserveEditing".into(), json!(true));
        // Keep upstream AI PDF layers, including hidden layers, unless a standard decides.
        if o.get("standard").is_none_or(Value::is_null) {
            o.entry("createLayers").or_insert(json!(true));
        }
        if let Some(c) = o.get("compress").and_then(Value::as_bool) {
            let compression = o.entry("compression").or_insert_with(|| json!({}));
            if let Some(m) = compression.as_object_mut() {
                m.insert("compressText".into(), json!(c));
            }
        }
    }
    let native_options = super::native::ai_options(cmd, f, doc, &params)?;
    let pdf_compatible = bool_or(&params, "pdfCompatible", true);
    if !pdf_compatible {
        let picked: ArtboardPick = super::encode::options(f, &params)?;
        let picked = picked.resolve(doc.artboards.len()).map_err(|e| bad(cmd, e))?;
        if picked.is_some_and(|v| !v.into_iter().eq(0..doc.artboards.len())) {
            return Err(bad(cmd, "pdfCompatible: false needs every artboard to keep the editable artwork"));
        }
    }
    let pages = doc.without_edit_modes();
    // Resolve placed files only for the drawn pages. Missing files become preview images there;
    // the native attachment keeps the original placed objects and their relinkable metadata.
    let (pages, placed_warnings) =
        if pdf_compatible { crate::cmd::place::document::full_documents(&pages) } else { (std::borrow::Cow::Owned(blank_pages(&pages)), Vec::new()) };
    let native = || super::native::ai_native(cmd, doc, &native_options);
    let (bytes, mut warnings) = super::pdf::encode_carrying(cmd, &pages, &params, native)?;
    warnings.extend(placed_warnings);
    if !pdf_compatible {
        warnings.push(NOT_PDF_COMPATIBLE.to_string());
    }
    Ok(Encoded { warnings, ..Encoded::one(bytes) })
}

/// Snapshot the active document for `plan` (stamping File Info's dates when the file becomes the
/// document's own). Bad options fail here, before anything is encoded.
pub fn save_job(s: &mut Session, plan: SavePlan) -> Result<SaveJob> {
    // The web build, without a clock, leaves the dates unless given one.
    if plan.path.is_some()
        && plan.retargets()
        && let Some(at) = clock_date(s, "modified", plan.modified)
    {
        stamp_save_dates(s.doc_mut()?, at);
    }
    job_for(s, s.doc()?, plan)
}

/// Snapshot `st` (any open document of `s`) for `plan`, without stamping dates (Data Recovery
/// snapshots documents that aren't active).
pub(crate) fn job_for(s: &Session, st: &DocState, plan: SavePlan) -> Result<SaveJob> {
    let cmd = plan.mode.command();
    let own = doc_to_save(st, plan.format);
    // A native file records its links' paths relative to where it is written.
    let relative = plan.path.as_deref().filter(|_| is_native(plan.format)).and_then(|p| crate::cmd::links::with_relative_paths(&own, p));
    let doc = relative.map_or(own, Arc::new);
    let mut params = plan.options.clone();
    if is_svg(plan.format) {
        // A save keeps hidden layers (not displayed) unless told otherwise; exports leave them out.
        params.entry("hiddenLayers").or_insert(Value::Bool(true));
    }
    let mut params = Value::Object(params);
    if matches!(plan.format.id, "vectorcraft" | "template") {
        // Not remembered with the options: the preference decides each time it isn't given.
        params = with_compression_pref(&s.prefs, &params);
        super::native::save_options(cmd, plan.format, &doc, &params)?;
    }
    let boards = separate_boards(cmd, plan.format, &doc, &params)?;
    let mut params = super::pdf::expand_preset(s, cmd, &params)?.into_owned();
    if let (Some(o), "ai") = (params.as_object_mut(), plan.format.id) {
        // Save keeps every artboard; exports may choose a subset instead.
        o.insert("range".into(), json!("all"));
    }
    Ok(SaveJob { plan, doc, params, snapshot: st.doc.clone(), boards })
}

impl SaveJob {
    pub fn plan(&self) -> &SavePlan {
        &self.plan
    }

    /// The document when the job was made (what counts as saved once the file is written).
    pub(crate) fn snapshot(&self) -> &Arc<Document> {
        &self.snapshot
    }

    /// Encode the file → it (and, with `separateArtboards`, a file per artboard after it), with
    /// the warnings (what the format loses first, then the encoder's own notes: PDF options not
    /// applied yet…).
    pub fn encode(&self) -> Result<Encoded> {
        let mut enc = self.encode_one(&self.doc)?;
        for &b in &self.boards {
            let one = self.encode_one(&artboard_doc(&self.doc, b))?;
            enc.files.extend(one.files.into_iter().map(|(_, bytes)| (Some(b), bytes)));
        }
        enc.warnings.splice(0..0, fidelity_warning(self.plan.format));
        Ok(enc)
    }

    /// Encode one file of `doc` in the job's format.
    fn encode_one(&self, doc: &Document) -> Result<Encoded> {
        let (cmd, f) = (self.plan.mode.command(), self.plan.format);
        let enc = if f.id == "ai" { encode_ai(cmd, f, doc, &self.params)? } else { encode_all(doc, f.id, &self.params)? };
        if enc.files.len() != 1 {
            return Err(bad(cmd, "Save writes one artboard: name one, or export several with document.export"));
        }
        Ok(enc)
    }

    /// Encode the file and write it with `write` (the file, then its artboards' files and the
    /// images an SVG links to, beside it). Without a path → `{dataBase64, bytes, format, name,
    /// folder?, warnings, files?: [{name, dataBase64}] (every file, when there are several),
    /// linked?: [{name, dataBase64}]}`; with one → `{path, format, bytes, warnings, files?:
    /// [path…], linked?: [path…]}`.
    pub fn write(&self, mut write: impl FnMut(&str, &[u8]) -> Result<()>) -> Result<Value> {
        let (plan, doc) = (&self.plan, &*self.doc);
        let mut enc = self.encode()?;
        let warnings = std::mem::take(&mut enc.warnings);
        let extra = json!({ "format": plan.format.id, "warnings": warnings });
        let Some(path) = &plan.path else {
            let mut out = write_encoded(None, &plan.name, doc, &enc, extra)?;
            out["name"] = json!(plan.name);
            if let Some(folder) = &plan.folder {
                out["folder"] = json!(folder);
            }
            return Ok(out);
        };
        let files = enc.named(doc, path);
        for (p, bytes) in &files {
            write(p, bytes)?;
        }
        let bytes = files.first().map_or(0, |f| f.1.len());
        let mut out = super::merge(json!({ "path": path, "bytes": bytes }), extra);
        let (main, linked) = files.split_at(enc.files.len().min(files.len()));
        if main.len() > 1 {
            out["files"] = main.iter().map(|(p, _)| json!(p)).collect();
        }
        if !linked.is_empty() {
            out["linked"] = linked.iter().map(|(p, _)| json!(p)).collect();
        }
        Ok(out)
    }

    /// Once written: Save and Save As (except to a template) make the file `st`'s own: path,
    /// title, format, options, and the snapshot as the saved state (edits made since the job was
    /// made keep the document modified).
    pub fn finish(self, st: &mut DocState) {
        let retargets = self.plan.retargets();
        if let (true, Some(path)) = (retargets, self.plan.path) {
            st.path = Some(path);
            st.format = self.plan.format.id;
            st.save_options = self.plan.options;
            st.converted = false;
            st.recovered = false;
            st.mark_saved_as(&self.snapshot);
        }
    }

    /// [`SaveJob::finish`] for document `uid` of `s`, if it is still open; a file that became
    /// its own also ends its Data Recovery copy.
    pub fn complete(self, s: &mut Session, uid: u64) {
        let saved = self.plan.retargets() && self.plan.path.is_some();
        // The other open documents that place this one show the new version.
        let written = self.plan.path.clone().filter(|_| matches!(self.plan.format.id, "vectorcraft" | "template"));
        if let Some(st) = s.document_mut(uid) {
            self.finish(st);
        }
        if saved {
            crate::cmd::recovery::forget(s, uid);
        }
        if let Some(path) = written {
            crate::cmd::links::refresh_placed(s, uid, &path);
        }
    }
}

/// Encode `plan` and write it with `write`, then make the file the document's own (see
/// [`SaveJob`]). Without a path the document is unchanged.
pub fn save_with(s: &mut Session, plan: SavePlan, write: impl FnMut(&str, &[u8]) -> Result<()>) -> Result<Value> {
    let job = save_job(s, plan)?;
    let uid = s.doc()?.uid;
    let out = job.write(write)?;
    job.complete(s, uid);
    Ok(out)
}

/// A save command: plan, then write with the file system.
fn save(s: &mut Session, mode: SaveMode, p: &Value) -> Result<Value> {
    let plan = save_plan(s, mode, p)?;
    save_with(s, plan, write_file)
}

fn can_revert(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    match (&st.path, st.is_dirty()) {
        (None, _) => Err("the document has never been saved".into()),
        (_, false) => Err("no changes since the last save".into()),
        _ => Ok(()),
    }
}

/// File → Revert: read and decode the saved file first (on any failure the document is left as it
/// is), then replace the document in its tab.
fn revert(s: &mut Session, _: &Value) -> Result<Value> {
    let index = s.active_index().ok_or(crate::EngineError::NoDocument)?;
    let path = s.doc()?.path.clone().ok_or_else(|| bad("file.revert", "the document has never been saved"))?;
    let Loaded { mut doc, .. } = load(&path, &read_file(&path)?)?;
    doc.template = false;
    // Back to the saved file: its recovery copy has nothing left to recover.
    let uid = s.doc()?.uid;
    crate::cmd::recovery::forget(s, uid);
    s.replace_document(index, doc);
    Ok(json!({ "path": path }))
}

/// `file.formatOptions`: a writable format's options with the values Save would use.
fn format_options(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "file.formatOptions";
    let st = s.active();
    let id = str_param(p, "format").or(st.map(|d| d.format)).unwrap_or("vectorcraft");
    let f = format(id)
        .filter(|f| f.write || SAVE_FORMATS.contains(&f.id))
        .ok_or_else(|| bad(C, format!("`{id}` is no writable format (see document.formats)")))?;
    let saved = st.filter(|d| d.format == f.id).map(|d| &d.save_options);
    // A native save compresses as Use Compression says, unless told otherwise.
    let prefs = with_compression_pref(&s.prefs, &json!({}));
    let mut v = f.to_json();
    if let Some(options) = v["options"].as_object_mut() {
        for (name, o) in options.iter_mut() {
            let default = prefs.get(name).filter(|_| matches!(f.id, "vectorcraft" | "template")).unwrap_or(&o["default"]);
            o["value"] = saved.and_then(|m| m.get(name)).unwrap_or(default).clone();
        }
    }
    v["saveFormats"] =
        SAVE_FORMATS.iter().filter_map(|id| format(id)).map(|f| json!({"id": f.id, "label": f.label, "extensions": f.extensions})).collect();
    Ok(v)
}

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "document.save",
            "Save Document",
            [],
            None,
            "{path?, format?: vectorcraft|template|pdf|svg|svgz|ai (default: the path's extension, else the document's own format), options?: {…the format's options, see file.formatOptions; default: as last saved}, svg?: {…SVG options} (SVG options may also be given flat; an SVG save keeps hidden layers, display:none, unless hiddenLayers is false), native (also flat): compress?: bool (gzip; default: the useCompression preference), version?: 4 (3, 2 or 1: for older VectorCraft versions; 1 and 2 are never compressed), preview?: false (embed a PNG of the first artboard, at most 256 px), native and .ai: separateArtboards?: false (also save each artboard of range?: \"1-3, 5\"|\"all\" (default) to <name>-<artboard>.<ext> beside the file, holding that artboard and the art touching it; the result's files lists every file written), includeLinked?: false (keep linked files' own pixels, not just their previews; they stay linked), embedProfiles?: true (carry the ICC profiles loaded from files that the document is tagged with; opening the file installs them where missing), pdfCompatible?: false for native (also carry a PDF of every artboard) | true for .ai (false: blank pages, only the native document), .ai: compress?: true (compression.compressText), modified?: Unix seconds|null (the File Info modified date, and created date when there is none, a save to a file stamps; default now, recorded in the journal so a replay matches; null: leave the dates)} → {path, format, bytes, warnings, files?: [path…] (the file and its artboards' files), linked?: [path…] (images an SVG links to)}. Save writes one artboard, except a .ai file: a PDF-compatible file of every artboard carrying the native document (preserveEditing always on; PDF options flat or in options), which document.open restores exactly; separateArtboards writes more files, one per artboard. Without a path it writes the document's own file in its own format: a document opened from or saved as SVG/PDF saves as that again (warnings name what the format loses). No path known (never saved, converted from an older version, restored by Data Recovery, or another format) → {dataBase64, format, name, folder?, warnings} and the document stays modified",
            has_doc,
            |s, p| save(s, SaveMode::Save, p)
        ),
        cmd!(
            "file.saveAs",
            "Save As…",
            ["File"],
            Some("Cmd+Shift+S"),
            "{path?, format?: vectorcraft|template|pdf|svg|svgz|ai (default: the path's extension, else the document's own), options?, svg?, modified? (as document.save)} the document takes on the new path, name and format (except a template, which is always a copy) → {path, format, bytes, warnings}; no path → {dataBase64, format, name, folder?, warnings}",
            has_doc,
            |s, p| save(s, SaveMode::SaveAs, p)
        ),
        cmd!(
            "file.saveCopy",
            "Save a Copy…",
            ["File"],
            Some("Cmd+Alt+S"),
            "{path?, format?, options?} (as file.saveAs) write a copy; the document keeps its path, title and modified state → {path, format, bytes, warnings}; no path → {dataBase64, format, name: \"<name> copy.<ext>\", folder?, warnings}",
            has_doc,
            |s, p| save(s, SaveMode::Copy, p)
        ),
        cmd!(
            "file.saveAsTemplate",
            "Save as Template…",
            ["File"],
            None,
            "{path?, compress?, version?, preview? (as document.save)} a native template (.vctemplate) that opens as a new untitled document; the document is unchanged → {path, format, bytes, warnings}; no path → {dataBase64, format, name: \"<name> template.vctemplate\", folder: the Templates folder (preference templatesFolder), warnings}",
            has_doc,
            |s, p| save(s, SaveMode::Template, p)
        ),
        cmd!(
            "file.newFromTemplate",
            "New from Template…",
            ["File"],
            Some("Cmd+Shift+N"),
            "{path} or {name, dataBase64}: open a template (or any readable file) as a new untitled document → {index, title, format, warnings}",
            always,
            super::load::new_from_template
        ),
        cmd!(
            "file.revert",
            "Revert",
            ["File"],
            Some("F12"),
            "{} discard the changes: reload the saved file into the same tab (history cleared; the tab keeps its place and view). Saved, modified documents only; if the file can't be read the document is left as it is → {path}. The app asks first (a confirm dialog) unless confirmed: true",
            can_revert,
            revert
        ),
        cmd!(
            query "file.formatOptions",
            "Format Options",
            [],
            None,
            "{format?: a writable format id (default: the document's own)} → {id, label, extensions, mime, read, write, raster, options: {name: {type, default, description, value}}, saveFormats: [{id, label, extensions}]}; value = the document's option as last saved in that format, else the default",
            always,
            format_options
        ),
    ]
}

#[cfg(test)]
mod tests_folders {
    use std::path::{Path, PathBuf};

    /// #703: the documents folder a desktop names in `user-dirs.dirs`, not an English `Documents`.
    #[test]
    fn the_desktops_documents_folder_is_read_from_user_dirs() {
        let home = Path::new("/home/david");
        let file = "# comment\nXDG_DESKTOP_DIR=\"$HOME/Scrivania\"\nXDG_DOCUMENTS_DIR=\"$HOME/Documenti\"\n";
        assert_eq!(super::xdg_documents(file, home), Some(PathBuf::from("/home/david/Documenti")));
        assert_eq!(super::xdg_documents("XDG_DOCUMENTS_DIR=\"/data/docs\"", home), Some(PathBuf::from("/data/docs")));
        // "$HOME/" turns it off; a relative or missing one isn't used.
        assert_eq!(super::xdg_documents("XDG_DOCUMENTS_DIR=\"$HOME/\"", home), None);
        assert_eq!(super::xdg_documents("XDG_DOCUMENTS_DIR=\"docs\"", home), None);
        assert_eq!(super::xdg_documents("XDG_MUSIC_DIR=\"$HOME/Musica\"", home), None);
    }

    /// #703: the Templates folder a dialog starts in exists: made when missing, else the nearest
    /// folder above it.
    #[test]
    fn the_templates_dialog_folder_exists() {
        let base = std::env::temp_dir().join(format!("vc-templates-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let mut prefs = crate::Prefs::default();
        let wanted = base.join("Modelli").join("VectorCraft Templates");
        prefs.templates_folder = wanted.to_string_lossy().into_owned();
        assert_eq!(super::templates_dialog_folder(&prefs).map(PathBuf::from), Some(wanted.clone()));
        assert!(wanted.is_dir(), "made");
        // A file in the way: the nearest folder that exists.
        std::fs::write(base.join("blocked"), b"x").unwrap();
        prefs.templates_folder = base.join("blocked").join("Templates").to_string_lossy().into_owned();
        assert_eq!(super::templates_dialog_folder(&prefs).map(PathBuf::from), Some(base.clone()));
        let _ = std::fs::remove_dir_all(&base);
    }
}
