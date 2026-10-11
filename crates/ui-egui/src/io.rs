//! File I/O through the injected services (pick, read, write, download, reveal); the engine's
//! `fileio` decodes, encodes and saves every format.

use serde_json::{Value, json};
use vectorcraft_doc::SavedView;
use vectorcraft_engine::EngineError;
use vectorcraft_engine::cmd::fileio::{self, Format, SAVE_FORMATS, SaveMode, SavePlan};

use crate::background::{self, Writer};
use crate::dialogs::svg_options;
use crate::state::Dialog;
use crate::{FilePick, Services, VectorcraftApp, dialogs};

/// Template extensions New from Template's open dialog lists first.
const TEMPLATE_EXTS: &[&str] = &["vctemplate", "ait", "vectorcraft", "drawcraft"];

/// Open bytes of any readable format as a new document (templates open untitled); swatch and
/// graphic style library files open in the library panel, Libraries panel files are added to it,
/// and flattener, PDF, print and perspective grid presets files are imported. → `document.open`'s result for a document opened now (its
/// `warnings` say what didn't come in as it was), else null.
pub fn open_bytes(app: &mut VectorcraftApp, name: &str, bytes: &[u8], path: Option<String>) -> Result<Value, String> {
    let opens = opens_as(name);
    // A WebAssembly plug-in is installed.
    if opens == Some(OpensAs::Plugin) {
        let r =
            app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(bytes), "name": path.as_deref().unwrap_or(name)}))?;
        app.status(format!("Installed plug-in {}", r["name"].as_str().unwrap_or(name)));
        return Ok(Value::Null);
    }
    if let Some(OpensAs::Presets { import, what }) = opens {
        let r = app.run(import, serde_json::json!({"data": String::from_utf8_lossy(bytes)}))?;
        let names: Vec<&str> = r["imported"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
        app.status(format!("Imported {what}: {}", names.join(", ")));
        return Ok(Value::Null);
    }
    if opens == Some(OpensAs::Library) {
        return crate::panels::libraries::import(app, json!({"data": String::from_utf8_lossy(bytes)})).map(|_| Value::Null);
    }
    let swatches = opens == Some(OpensAs::SwatchLibrary);
    if swatches || opens == Some(OpensAs::StyleLibrary) {
        let p = match path {
            Some(path) => serde_json::json!({ "path": path }),
            // `.ase` swatch libraries are binary.
            None if swatches => serde_json::json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)}),
            None => serde_json::json!({"name": name, "data": String::from_utf8_lossy(bytes)}),
        };
        let load = if swatches { crate::panels::swatches::load_library } else { crate::panels::graphic_styles::load_library };
        return load(app, p).map(|_| Value::Null);
    }
    // A PDF with several pages or a password asks first (the Import PDF dialog), and so does a
    // DXF drawing (DXF Import Options).
    if crate::dialogs::import_pdf::offer(app, name, bytes, path.clone(), None)
        || crate::dialogs::dxf_import::offer(app, name, bytes, path.clone(), None)
    {
        return Ok(Value::Null);
    }
    open_document(app, name, bytes, path, &Value::Null)
}

/// The kinds of file File → Open installs, imports or shows in the library panel or the Libraries
/// panel ([`opens_as`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OpensAs {
    /// A WebAssembly plug-in: installed.
    Plugin,
    /// Flattener, PDF, print or perspective grid presets: imported with command `import`, and
    /// `what` names them in the status bar.
    Presets { import: &'static str, what: &'static str },
    /// A Libraries panel file: added to the Libraries panel as a new library.
    Library,
    /// A swatch library: opened in the library panel.
    SwatchLibrary,
    /// A graphic style library: opened in the library panel.
    StyleLibrary,
}

/// The kind of file named `name` (by its extension) that File → Open installs, imports or shows in
/// the library panel or the Libraries panel ([`open_bytes`]), else `None`. A drop opens these files
/// the same way, wherever they are dropped ([`VectorcraftApp::drop_target`]).
pub fn opens_as(name: &str) -> Option<OpensAs> {
    let ext = fileio::extension(name);
    let presets = [
        (vectorcraft_engine::cmd::flatten::PRESET_EXTS, "flattener.presets.import", "flattener presets"),
        (vectorcraft_engine::cmd::pdfcmds::PRESET_EXTS, "pdf.preset.import", "PDF presets"),
        (vectorcraft_engine::cmd::printpresets::PRESET_EXTS, "print.presets.import", "print presets"),
        (vectorcraft_engine::cmd::perspgrid::PRESET_EXTS, "perspective.presets.import", "perspective grid presets"),
    ];
    if vectorcraft_engine::cmd::plugin::EXTS.contains(&ext.as_str()) {
        Some(OpensAs::Plugin)
    } else if let Some(&(_, import, what)) = presets.iter().find(|(exts, ..)| exts.contains(&ext.as_str())) {
        Some(OpensAs::Presets { import, what })
    } else if vectorcraft_engine::cmd::library::LIBRARY_EXTS.contains(&ext.as_str()) {
        Some(OpensAs::Library)
    } else if vectorcraft_engine::cmd::swatchlib::LIBRARY_EXTS.contains(&ext.as_str()) {
        Some(OpensAs::SwatchLibrary)
    } else if ext == vectorcraft_doc::style_libs::STYLES_EXT {
        Some(OpensAs::StyleLibrary)
    } else {
        None
    }
}

/// Open a document through the engine loader with the `document.open` options in `p` →
/// `document.open`'s result.
pub fn open_document(app: &mut VectorcraftApp, name: &str, bytes: &[u8], path: Option<String>, p: &Value) -> Result<Value, String> {
    let r = fileio::open_bytes_with(&mut app.session, name, bytes, path, p).map_err(|e| e.to_string())?;
    app.sync_views();
    if let Some(w) = r["warnings"].as_array().filter(|w| !w.is_empty()) {
        app.status(format!("Opened with {} note(s): {}", w.len(), w[0].as_str().unwrap_or_default()));
    }
    crate::dialogs::missing_links::after_open(app, &r);
    Ok(r)
}

/// A path from the open dialog, or "cancelled".
fn pick_open(app: &mut VectorcraftApp, pick: &FilePick) -> Result<String, String> {
    crate::picks::open(app, pick).ok_or_else(|| "cancelled".into())
}

/// Object › Plug-ins › Install Plug-in…: installs the `.wasm` at `path`, else a picked one (the
/// web's file picker hands the file to [`open_bytes`], which installs it).
pub fn install_plugin(app: &mut VectorcraftApp, path: Option<String>) -> Result<Value, String> {
    let path = match path {
        Some(p) => p,
        None if app.services.open_async.is_some() => return open_dialog(app).map(|_| Value::Null),
        None => pick_open(app, &FilePick { filters: vec![("Plug-ins", vectorcraft_engine::cmd::plugin::EXTS)], ..Default::default() })?,
    };
    let r = app.run("plugin.install", json!({ "path": path }))?;
    app.status(format!("Installed plug-in {}", r["name"].as_str().unwrap_or_default()));
    Ok(r)
}

/// File → Open…
pub fn open_dialog(app: &mut VectorcraftApp) -> Result<(), String> {
    if let Some(f) = app.services.open_async.as_mut() {
        f();
        return Ok(());
    }
    let path = pick_open(app, &FilePick { filters: fileio::open_filters().collect(), ..Default::default() })?;
    open_reporting(app, &path).map(|_| ())
}

/// A file the user asked to open (File › Open, Open Recent, the Home screen, a drop, the Finder)
/// couldn't be: say so in a dialog as well as the status bar, which is easily missed (#861).
pub fn report_open_error(app: &mut VectorcraftApp, name: &str, e: &str) {
    app.status(format!("Couldn't open {name}: {e}"));
    let message = crate::i18n::fmt(tl!("Can't open “{name}”."), &[("name", name)]);
    dialogs::confirm::tell(app, &message, &crate::i18n::message(crate::i18n::current(), e));
}

/// [`open_path`] for a file the user asked for, a failure reported ([`report_open_error`]).
pub fn open_reporting(app: &mut VectorcraftApp, path: &str) -> Result<Value, String> {
    let r = open_path(app, path);
    if let Err(e) = &r {
        report_open_error(app, &fileio::file_name(path), e);
    }
    r
}

fn read(app: &VectorcraftApp, path: &str) -> Result<Vec<u8>, String> {
    app.services.read.as_ref().ok_or("no file reader")?(path)
}

/// Open the file at `path` (see [`open_bytes`]) → `document.open`'s result, or null.
pub fn open_path(app: &mut VectorcraftApp, path: &str) -> Result<Value, String> {
    let bytes = read(app, path)?;
    let r = open_bytes(app, path, &bytes, Some(path.to_string()))?;
    note_recent(app, path);
    Ok(r)
}

/// File → New from Template…: a template (or any readable file) as a new untitled document. Without
/// a path the open dialog starts in the Templates folder. On the web the browser's file picker
/// opens it (`.vctemplate`, `.ait` and template files open untitled there too).
pub fn new_from_template(app: &mut VectorcraftApp, path: Option<String>) -> Result<Value, String> {
    let path = match path {
        Some(p) => p,
        None if app.services.open_async.is_some() => return open_dialog(app).map(|_| Value::Null),
        None => {
            let filters = std::iter::once(("Templates", TEMPLATE_EXTS)).chain(fileio::open_filters()).collect();
            pick_open(app, &FilePick { folder: fileio::templates_dialog_folder(&app.session.prefs), filters, ..Default::default() })?
        }
    };
    let bytes = read(app, &path)?;
    let r = fileio::open_template(&mut app.session, &path, &bytes, Some(&path)).map_err(|e| e.to_string())?;
    app.sync_views();
    crate::dialogs::missing_links::after_open(app, &r);
    Ok(r)
}

/// Web: download `bytes` under `path`'s file name; desktop: write them to `path`.
pub(crate) fn write_to(services: &mut Services, path: &str, bytes: &[u8]) -> Result<(), String> {
    if let Some(dl) = services.download.as_mut() {
        dl(&fileio::file_name(path), bytes);
        return Ok(());
    }
    services.write.as_mut().ok_or("no file writer")?(path, bytes)
}

/// The web saves by downloading: no save panel, no folders.
pub(crate) fn is_web(app: &VectorcraftApp) -> bool {
    app.services.download.is_some()
}

/// Write `bytes` to `path`, else to a file picked with `name` suggested next to the document (the
/// web downloads them as `name`) → where they went.
pub(crate) fn write_named(app: &mut VectorcraftApp, path: Option<String>, name: &str, bytes: &[u8]) -> Result<String, String> {
    let path = match path {
        Some(p) => p,
        None => {
            let (_, folder) = suggested(app, "");
            pick_path(app, &FilePick { name: name.to_string(), folder, ..Default::default() })?
        }
    };
    write_to(&mut app.services, &path, bytes)?;
    Ok(path)
}

/// Where a file goes when no path was given: the suggested name on the web (a download), else the
/// save panel's choice, given the suggested name's extension when it names no file type (a name
/// typed without one).
fn pick_path(app: &mut VectorcraftApp, pick: &FilePick) -> Result<String, String> {
    if is_web(app) {
        return Ok(pick.name.clone());
    }
    let picked = crate::picks::save(app, pick).ok_or("cancelled")?;
    Ok(with_extension(&picked, &fileio::extension(&pick.name), |_| true))
}

/// `path` as typed in a save panel: kept when it ends in `ext` or its extension names a format
/// `keeps` takes, else with `.ext` added.
fn with_extension(path: &str, ext: &str, keeps: impl Fn(&Format) -> bool) -> String {
    let has = fileio::extension(path);
    if ext.is_empty() || has.eq_ignore_ascii_case(ext) || fileio::format_for_name(path).is_some_and(keeps) {
        path.to_string()
    } else {
        format!("{path}.{ext}")
    }
}

/// The active document's file name with extension `ext`, and its folder.
fn suggested(app: &VectorcraftApp, ext: &str) -> (String, Option<String>) {
    let st = app.session.active();
    let t = st.map(|d| d.path.clone().unwrap_or_else(|| d.doc.title.clone())).unwrap_or_else(|| "Untitled".into());
    let folder = st.and_then(|d| d.path.as_deref()).and_then(|p| std::path::Path::new(p).parent()).map(|p| p.to_string_lossy().to_string());
    (format!("{}.{ext}", fileio::file_stem(&t)), folder.filter(|f| !f.is_empty()))
}

/// `path`, else one picked in a save panel (the web: the suggested name) for the active document
/// with extension `ext`, filtered to that format when it is one.
pub(crate) fn target_path(app: &mut VectorcraftApp, path: Option<String>, ext: &str) -> Result<String, String> {
    if let Some(p) = path {
        return Ok(p);
    }
    let (name, folder) = suggested(app, ext);
    let filters = fileio::format(ext).map(|f| vec![(f.label, f.extensions)]).unwrap_or_default();
    pick_path(app, &FilePick { name, folder, filters })
}

/// Keep the active document's saved view current before a save (native files reopen at it).
pub fn remember_view(app: &mut VectorcraftApp) {
    let Some(v) = app.view().copied().filter(|v| v.fitted) else { return };
    if let Some(st) = app.session.active_mut() {
        st.view = Some(SavedView { name: String::new(), center: v.center, zoom: v.zoom, rotation: v.rotation });
    }
}

/// A path typed in a save panel: kept when its extension names a save format (the panel's file
/// type), else `f`'s extension is added.
fn with_save_extension(path: &str, f: &Format) -> String {
    with_extension(path, f.extensions[0], |g| SAVE_FORMATS.contains(&g.id))
}

/// Write every file of an export (one per artboard, linked images) for the destination `path`.
/// → the export's own files (without the linked images).
fn write_encoded(write: Writer, doc: &vectorcraft_doc::Document, path: &str, enc: &fileio::Encoded) -> Result<Vec<String>, String> {
    let named = enc.named(doc, path);
    for (p, bytes) in &named {
        write(p, bytes)?;
    }
    Ok(named.into_iter().take(enc.files.len()).map(|(p, _)| p).collect())
}

fn plan(app: &VectorcraftApp, mode: SaveMode, p: &Value) -> Result<SavePlan, String> {
    fileio::save_plan(&app.session, mode, p).map_err(|e| e.to_string())
}

/// File → Save, Save As…, Save a Copy…, Save as Template… and the options dialogs' OK. `p` is the
/// engine command's `{path?, format?, options?, svg?}`.
///
/// When no path is known, desktop asks with a save panel (one file type per save format) and the
/// web opens the Save As dialog (file name and format). With `ask_options` and no options in `p`,
/// a format picked that way that has options asks for them before anything is written (SVG
/// Options, the Save PDF dialog, or the save options dialog: native and `.ai` files on Save As
/// only), and so does Save As or Save a Copy to a given SVG path. → `{path, format, warnings…}` once written, or `{pending: <dialog kind>,
/// path?}` while a dialog is open.
pub fn save(app: &mut VectorcraftApp, mode: SaveMode, p: &Value, ask_options: bool) -> Result<Value, String> {
    remember_view(app);
    // Writing over the file the document was read from is asked about here, not refused.
    let first = plan(app, mode, &acknowledged(p))?;
    let ask = ask_options && !has_options(p);
    if let Some(path) = first.path.clone() {
        if let Some(r) = ask_before_losing(app, &path, save_command(mode), p) {
            return Ok(r);
        }
        if ask && matches!(mode, SaveMode::SaveAs | SaveMode::Copy) && matches!(first.format.id, "svg" | "svgz") {
            return ask_format_options(app, mode, first.format, &path);
        }
        return write_plan(app, first);
    }
    if is_web(app) && ask_options {
        return Ok(open_options(app, mode.command(), first.format, &first.name, true));
    }
    let filters = match mode {
        SaveMode::Template => vec![(first.format.label, first.format.extensions)],
        _ => fileio::save_filters(first.format.id),
    };
    let picked = pick_path(app, &FilePick { name: first.name.clone(), folder: first.folder.clone(), filters })?;
    // The picked file type wins over a `format` param.
    let mut q = if p.is_object() { p.clone() } else { json!({}) };
    if let Some(o) = q.as_object_mut() {
        o.remove("format");
        o.insert("path".into(), json!(with_save_extension(&picked, first.format)));
    }
    let chosen = plan(app, mode, &acknowledged(&q))?;
    if let Some(r) = chosen.path.as_deref().and_then(|path| ask_before_losing(app, path, save_command(mode), &q)) {
        return Ok(r);
    }
    if ask && asks_options(mode, chosen.format) {
        let path = chosen.path.clone().unwrap_or_default();
        return ask_format_options(app, mode, chosen.format, &path);
    }
    write_plan(app, chosen)
}

/// The UI command that runs a save in `mode`.
fn save_command(mode: SaveMode) -> &'static str {
    match mode {
        SaveMode::Save => "file.save",
        SaveMode::SaveAs => "file.saveAs",
        SaveMode::Copy => "file.saveCopy",
        SaveMode::Template => "file.saveAsTemplate",
    }
}

/// `p` with `acknowledgeLoss: true`.
fn acknowledged(p: &Value) -> Value {
    let mut p = if p.is_object() { p.clone() } else { json!({}) };
    p["acknowledgeLoss"] = json!(true);
    p
}

/// Writing `path` over the file the active document was read from, when reading it left things
/// out (hidden text, art or layers VectorCraft can't read yet): asks first, and OK runs `command`
/// with `params` and `acknowledgeLoss` → `{pending}` while it asks, `None` when nothing is lost.
fn ask_before_losing(app: &mut VectorcraftApp, path: &str, command: &str, params: &Value) -> Option<Value> {
    let what = fileio::losses_summary(fileio::overwrite_losses(app.session.active()?, path, params)?);
    let name = fileio::file_name(path);
    let message = crate::i18n::fmt(tl!("Replace “{name}”, the file this document was opened from?"), &[("name", &name)]);
    let detail = crate::i18n::fmt(
        tl!("Opening it left out what VectorCraft can't read yet ({what}), so replacing it loses that for good. Save under another name to keep it."),
        &[("what", &what)],
    );
    let mut params = acknowledged(params);
    if command != "file.save" {
        params["path"] = json!(path);
    }
    dialogs::confirm::ask(app, &message, &detail, command, params);
    Some(json!({ "pending": dialogs::confirm::KIND }))
}

/// Does a save (`mode`) to a picked file of format `f` ask for its options first? Native and `.ai`
/// files only on Save As (the options dialog after the save panel); Save, a Copy and templates
/// reuse the remembered options or Use Compression.
fn asks_options(mode: SaveMode, f: &Format) -> bool {
    !f.options.is_empty()
        && match f.id {
            "vectorcraft" | "ai" => mode == SaveMode::SaveAs,
            "template" => false,
            _ => true,
        }
}

/// Does `p` give format options (`options`, `svg: {…}` or SVG options at its top level)?
fn has_options(p: &Value) -> bool {
    ["options", "svg"].iter().any(|k| p.get(*k).is_some_and(|v| !v.is_null())) || fileio::svg_options(p).is_ok_and(|m| !m.is_empty())
}

/// Ask for format `f`'s options before a save (`mode`) to `path`: SVG Options for SVG, the Save
/// PDF dialog for PDF, else the save options dialog. OK finishes the save.
pub fn ask_format_options(app: &mut VectorcraftApp, mode: SaveMode, f: &Format, path: &str) -> Result<Value, String> {
    match f.id {
        "pdf" => ask_pdf_options(app, mode, path),
        "svg" | "svgz" => {
            let m = if mode == SaveMode::Copy { svg_options::Mode::SaveCopy } else { svg_options::Mode::Save };
            svg_options::open(app, m, Some(path));
            Ok(json!({ "pending": svg_options::KIND, "dialog": svg_options::KIND, "path": path }))
        }
        _ => Ok(open_options(app, mode.command(), f, path, false)),
    }
}

/// A save as PDF asks with the Save PDF dialog, filled with the PDF options the document was last
/// saved with; its OK finishes the save ([`save_pdf`]).
pub fn ask_pdf_options(app: &mut VectorcraftApp, mode: SaveMode, path: &str) -> Result<Value, String> {
    let mut p = Value::Object(plan(app, mode, &json!({ "path": path, "format": "pdf" }))?.options);
    p["path"] = json!(path);
    dialogs::open_save_pdf(app, &p)?;
    let d = app.ui.dialog.as_mut().ok_or("the Save PDF dialog didn't open")?;
    d.fields.insert("__save".into(), json!(mode.command()));
    Ok(json!({ "pending": d.kind }))
}

/// The Save PDF dialog's OK during a save (`mode`): `params` are its `document.exportPdf` options
/// and `path`.
pub fn save_pdf(app: &mut VectorcraftApp, mode: SaveMode, mut params: Value) -> Result<Value, String> {
    let path = params.as_object_mut().and_then(|o| o.remove("path"));
    let view = params.get("viewAfterSaving").and_then(Value::as_bool).unwrap_or(false);
    let r = save(app, mode, &json!({ "path": path, "format": "pdf", "options": params }), false)?;
    if view && let Some(path) = r["path"].as_str() {
        view_file(app, path);
    }
    Ok(r)
}

/// Open a written file in the system viewer (not on the web, which downloads it).
fn view_file(app: &mut VectorcraftApp, path: &str) {
    if !is_web(app) {
        app.open_url(&file_url(path));
    }
}

/// "Saved <path>", with the first of the result's warnings.
pub(crate) fn report_saved(app: &mut VectorcraftApp, path: &str, r: &Value) {
    let warnings: Vec<&str> = r["warnings"].as_array().map(|w| w.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    app.status(match warnings.first() {
        Some(first) => format!("Saved {path} with {} note(s): {first}", warnings.len()),
        None => format!("Saved {path}"),
    });
}

/// Write a planned save through the services and report it in the status bar.
/// With Background Save on, the file is encoded and written from a snapshot off the UI thread
/// ([`background`]): this returns `{path, background: true, status}` at once, and the document
/// takes the file (and counts as saved, as of the snapshot) once it is written.
fn write_plan(app: &mut VectorcraftApp, plan: SavePlan) -> Result<Value, String> {
    let uid = app.session.active().ok_or("no document")?.uid;
    // Saves of one document finish in order.
    if app.background.busy_with(uid) {
        background::wait_all(app);
    }
    let job = fileio::save_job(&mut app.session, plan).map_err(|e| e.to_string())?;
    let (retargets, target) = (job.plan().retargets(), job.plan().path.clone());
    let label = match &target {
        Some(path) => format!("Saving {}", fileio::file_name(path)),
        None => "Saving".into(),
    };
    // Without a path the bytes come back to the caller: nothing to do in the background.
    let enabled = app.session.prefs.background_save && target.is_some();
    // The work writes the file; the copy makes it the document's own afterwards.
    let done = job.clone();
    let work = move |write: Writer| job.write(|p, b| write(p, b).map_err(EngineError::Other)).map_err(|e| e.to_string());
    let then = move |app: &mut VectorcraftApp, r: Result<Value, String>| {
        let r = r?;
        // The document may have closed meanwhile: the file is written all the same.
        done.complete(&mut app.session, uid);
        let path = r["path"].as_str().unwrap_or_default().to_string();
        if retargets {
            note_recent(app, &path);
        }
        report_saved(app, &path, &r);
        Ok(r)
    };
    let mut r = background::run(app, enabled, label, Some(uid), work, then)?;
    if let Some(path) = target.filter(|_| r.get("path").is_none()) {
        r["path"] = json!(path);
    }
    Ok(r)
}

/// Open the save options dialog of format `f` before writing `path`: `action` is the save command
/// it finishes, and `pick` (the web's Save As) also names the file and chooses the format.
fn open_options(app: &mut VectorcraftApp, action: &str, f: &Format, path: &str, pick: bool) -> Value {
    let mut fields = dialogs::save_options::option_values(app, f);
    fields.insert("__action".into(), json!(action));
    fields.insert("__pick".into(), json!(pick));
    fields.insert("path".into(), json!(path));
    fields.insert("format".into(), json!(f.id));
    app.ui.dialog = Some(Dialog { kind: dialogs::save_options::KIND.into(), fields });
    json!({ "pending": dialogs::save_options::KIND })
}

/// The most recent files remembered: Preferences → File Handling → Number of Recent Files to Display
/// shows 0 to this many of them.
pub const MAX_RECENT_FILES: usize = 30;

/// Put `path` at the top of File → Open Recent Files.
pub fn note_recent(app: &mut VectorcraftApp, path: &str) {
    let r = &mut app.ui.recent_files;
    r.retain(|p| p != path);
    r.insert(0, path.to_string());
    r.truncate(MAX_RECENT_FILES);
}

/// The recent files File → Open Recent Files lists.
pub fn recent_files(app: &VectorcraftApp) -> &[String] {
    let r = &app.ui.recent_files;
    &r[..r.len().min(app.session.prefs.recent_files_count as usize)]
}

/// Export the active document in `format` (default: the path's extension, else PNG) with the
/// `document.export` options in `params` (artboard, range, useArtboards, selectedOnly, ppi, SVG
/// options…): every file it writes (one per artboard, linked images) goes next to `path`. The
/// document keeps its path. → `{path, warnings, files?}` (`files`: every file when there are
/// several).
pub fn export(app: &mut VectorcraftApp, format: Option<&str>, path: Option<String>, params: &Value) -> Result<Value, String> {
    let f = fileio::writable_format(format, path.as_deref())?;
    let st = app.session.active().ok_or("no document")?;
    let doc = match fileio::export_source(st, params).map_err(|e| e.to_string())? {
        std::borrow::Cow::Borrowed(_) => st.doc.clone(),
        std::borrow::Cow::Owned(d) => std::sync::Arc::new(d),
    };
    let path = target_path(app, path, f.extensions[0])?;
    let mut again = params.clone();
    if let Some(o) = again.as_object_mut() {
        o.insert("format".into(), json!(f.id));
    }
    if let Some(r) = ask_before_losing(app, &path, "file.exportAs", &again) {
        return Ok(r);
    }
    // An SVG given a .svgz name is written compressed.
    let f = match fileio::format_for_name(&path) {
        Some(z) if f.id == "svg" && z.id == "svgz" => z,
        _ => f,
    };
    // Background Export: encoded and written from this snapshot while editing goes on.
    let label = format!("Exporting {}", fileio::file_name(&path));
    let (params, target) = (params.clone(), path.clone());
    let work = move |write: Writer| {
        let enc = fileio::encode_all(&doc, f.id, &params).map_err(|e| e.to_string())?;
        let files = write_encoded(write, &doc, &path, &enc)?;
        let mut out = json!({ "path": files.first().unwrap_or(&path), "warnings": enc.warnings });
        if files.len() > 1 {
            out["files"] = json!(files);
        }
        Ok(out)
    };
    let enabled = app.session.prefs.background_export;
    let mut r = background::run(app, enabled, label, None, work, |app, r| {
        let r = r?;
        let what = match r["files"].as_array() {
            Some(files) => format!("{} files", files.len()),
            None => r["path"].as_str().unwrap_or_default().to_string(),
        };
        let warnings: Vec<&str> = r["warnings"].as_array().map(|w| w.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        app.status(match warnings.first() {
            Some(first) => format!("Exported {what} with {} note(s): {first}", warnings.len()),
            None => format!("Exported {what}"),
        });
        Ok(r)
    })?;
    if r.get("path").is_none() {
        r["path"] = json!(target);
    }
    Ok(r)
}

/// Run an engine command that returns `{dataBase64}` and write the bytes to a picked path
/// (Export Selection, Save as Template).
pub fn save_command_output(app: &mut VectorcraftApp, id: &str, ext: &str, params: Value) -> Result<String, String> {
    let (path, _) = run_to_file(app, id, ext, params)?;
    app.status(format!("Saved {path}"));
    Ok(path)
}

/// Run an engine command that returns `{data}` or `{dataBase64}` and write the bytes to a file
/// picked with file name `name` suggested next to the document (the web downloads them as `name`)
/// → where they went.
pub(crate) fn save_command_output_named(app: &mut VectorcraftApp, id: &str, name: &str, params: Value) -> Result<String, String> {
    let mut v = app.session.execute(id, &params).map_err(|e| e.to_string())?;
    let bytes = take_output(&mut v)?;
    let path = write_named(app, None, name, &bytes)?;
    app.status(format!("Saved {path}"));
    Ok(path)
}

/// The bytes a command returned, taken out of its result `v`, which keeps the rest: binary output
/// comes as base64, text (swatch libraries) as is.
fn take_output(v: &mut Value) -> Result<Vec<u8>, String> {
    let o = v.as_object_mut().ok_or("no data")?;
    let bytes = match (o.remove("dataBase64"), o.remove("data")) {
        (Some(Value::String(b64)), _) => vectorcraft_format::base64_decode(&b64),
        (_, Some(Value::String(text))) => Some(text.into_bytes()),
        _ => None,
    };
    bytes.ok_or_else(|| "no data".into())
}

/// Run an engine command that returns `{dataBase64}` without its `path` and write the bytes to
/// that path (else a picked or suggested name) → (path, the command's result without the data).
/// The command runs before a path is asked for, so bad params never open a save dialog, except
/// for Export Selection without a `format`, which takes it from the picked path.
pub(crate) fn run_to_file(app: &mut VectorcraftApp, id: &str, ext: &str, mut params: Value) -> Result<(String, Value), String> {
    let mut path = params.as_object_mut().and_then(|o| o.remove("path")).and_then(|p| p.as_str().map(str::to_string));
    if let Some(o) = params.as_object_mut()
        && o.get("format").is_none()
        && id == "document.exportSelection"
    {
        let picked = target_path(app, path, ext)?;
        let e = Some(fileio::extension(&picked)).filter(|e| !e.is_empty()).unwrap_or_else(|| ext.into());
        o.insert("format".into(), serde_json::json!(e));
        path = Some(picked);
    }
    let mut v = app.session.execute(id, &params).map_err(|e| e.to_string())?;
    let bytes = take_output(&mut v)?;
    let path = match path {
        Some(p) => p,
        None => target_path(app, None, ext)?,
    };
    write_to(&mut app.services, &path, &bytes)?;
    Ok((path, v))
}

/// File → Save as PDF: `document.exportPdf` with `params` (the Save PDF dialog's options), written
/// to `path` (else a picked or suggested name) → `{path, bytes, warnings}`. With
/// `viewAfterSaving`, the written file opens in the system viewer (not on the web, which
/// downloads it).
pub fn export_pdf(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    let view = params.get("viewAfterSaving").and_then(Value::as_bool).unwrap_or(false);
    let (path, mut v) = run_to_file(app, "document.exportPdf", "pdf", params)?;
    if view {
        view_file(app, &path);
    }
    report_saved(app, &path, &v);
    v["path"] = Value::String(path);
    Ok(v)
}

/// `path` as an absolute `file://` URL for the system opener (bytes other than letters, digits and
/// `/-._~:` percent-encoded).
pub(crate) fn file_url(path: &str) -> String {
    let abs = std::path::absolute(path).map_or_else(|_| path.to_string(), |p| p.to_string_lossy().into_owned());
    let abs = abs.replace('\\', "/");
    let mut url = String::from(if abs.starts_with('/') { "file://" } else { "file:///" });
    for b in abs.bytes() {
        if b.is_ascii_alphanumeric() || b"/-._~:".contains(&b) {
            url.push(char::from(b));
        } else {
            url.push_str(&format!("%{b:02X}"));
        }
    }
    url
}

/// File → Revert: ask first; OK runs `file.revert` with `confirmed`, which goes to the engine.
pub fn ask_revert(app: &mut VectorcraftApp) -> Result<Value, String> {
    let c = vectorcraft_engine::find_command("file.revert").ok_or("no revert command")?;
    (c.enabled)(&app.session)?;
    let name = app.session.active().map(|d| d.title()).unwrap_or_default();
    let message = crate::i18n::fmt(tl!("Revert to the saved version of “{name}”?"), &[("name", &name)]);
    dialogs::confirm::ask(app, &message, tl!("Changes made since it was last saved will be lost."), "file.revert", json!({ "confirmed": true }));
    Ok(json!({ "pending": dialogs::confirm::KIND }))
}

/// File → Show in Folder: the document's file in the system file manager.
pub fn reveal(app: &mut VectorcraftApp) -> Result<Value, String> {
    let path = app.session.active().and_then(|d| d.path.clone()).ok_or("the document has never been saved")?;
    reveal_path(app, &path)?;
    Ok(json!({ "path": path }))
}

/// Show the file at `path` in the system file manager (desktop).
pub(crate) fn reveal_path(app: &mut VectorcraftApp, path: &str) -> Result<(), String> {
    app.services.reveal.as_mut().ok_or("no file manager here")?(path)
}

/// Open `path` (a file or a folder) in the system's default app for it (desktop).
pub(crate) fn open_in_app(app: &mut VectorcraftApp, path: &str) -> Result<(), String> {
    app.services.open_file.as_mut().ok_or("no app to open files here")?(path)
}

/// File → Export for Screens with `document.exportForScreens` params: the files go into `folder`;
/// without one the web downloads them (the ZIP, or each file) and the other frontends get them
/// back as base64. With `openLocation`, the file manager shows the first file written.
pub fn export_for_screens(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    export_files(app, "document.exportForScreens", params)
}

/// [`export_for_screens`] through command `id` (`document.exportForScreens` or `assets.export`).
pub(crate) fn export_files(app: &mut VectorcraftApp, id: &str, params: Value) -> Result<Value, String> {
    let open_location = params.get("openLocation").and_then(Value::as_bool).unwrap_or(false);
    let folder = params.get("folder").and_then(Value::as_str).unwrap_or_default().to_string();
    let r = app.run(id, params)?;
    let count = r["files"].as_array().map_or(0, Vec::len);
    // Files that came back as bytes: (name, base64).
    let returned: Vec<(&str, &str)> = match r["dataBase64"].as_str() {
        Some(zip) => vec![(r["name"].as_str().unwrap_or("Export.zip"), zip)],
        None => {
            r["files"].as_array().into_iter().flatten().filter_map(|f| Some((f.get("name")?.as_str()?, f.get("dataBase64")?.as_str()?))).collect()
        }
    };
    if returned.is_empty() {
        let first = r["path"].as_str().or_else(|| r["files"][0].as_str());
        let mut status = format!("Exported {count} file(s) to {folder}");
        if open_location
            && let Some(first) = first
            && let Err(e) = reveal_path(app, first)
        {
            status = format!("{status} ({e})");
        }
        app.status(status);
    } else if let Some(download) = app.services.download.as_mut() {
        for (name, data) in &returned {
            let bytes = vectorcraft_format::base64_decode(data).ok_or("the export returned unreadable data")?;
            download(name, &bytes);
        }
        let what = match returned.as_slice() {
            [(name, _)] => (*name).to_string(),
            files => format!("{} files", files.len()),
        };
        app.status(format!("Downloaded {what}"));
    }
    Ok(r)
}

/// Place a file's bytes (no path, so embedded) centred in the view: `file.place`.
pub fn place_bytes(app: &mut VectorcraftApp, name: &str, bytes: &[u8]) -> Result<(), String> {
    crate::place::run(app, &serde_json::json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) })).map(|_| ())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use serde_json::json;
    use vectorcraft_engine::Session;

    use super::*;
    use crate::Services;
    use vectorcraft_doc::NodeKind;

    type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

    /// An app whose writer records what it is given.
    fn app() -> (VectorcraftApp, Written) {
        let written = Written::default();
        let w = written.clone();
        let services = Services {
            write: Some(Box::new(move |p: &str, b: &[u8]| {
                w.borrow_mut().push((p.to_string(), b.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        (VectorcraftApp::new(Session::new(), services), written)
    }

    /// #861: a file the user opens that can't be opened is reported in a dialog they close, not
    /// only in the status bar; a file an agent opens by path only answers with the error.
    #[test]
    fn a_file_the_user_cant_open_is_reported_in_a_dialog() {
        let (mut app, _) = app();
        app.services.read = Some(Box::new(|_: &str| Err("no such file".into())));
        assert!(open_reporting(&mut app, "/docs/gone.ai").is_err());
        let d = app.ui.dialog.take().expect("a dialog says so");
        assert_eq!(
            (d.kind.as_str(), d.str("message"), d.str("detail")),
            (dialogs::confirm::MESSAGE, "Can't open “gone.ai”.".to_string(), "no such file".to_string())
        );
        assert!(app.ui.status.contains("gone.ai"));
        // An agent's `file.open {path}` gets the error back and no dialog.
        assert!(app.run("file.open", json!({"path": "/docs/gone.ai"})).is_err());
        assert!(app.ui.dialog.is_none());
    }

    fn bytes_of(app: &mut VectorcraftApp, cmd: &str, p: Value) -> Vec<u8> {
        let v = app.session.execute(cmd, &p).unwrap();
        vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
    }

    fn image_size(app: &VectorcraftApp, id: vectorcraft_doc::NodeId) -> (u32, u32) {
        match &app.session.doc().unwrap().doc.node(id).unwrap().kind {
            NodeKind::Image(im) => (im.width, im.height),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn webp_opens_and_places_at_its_pixel_size() {
        let (mut app, _) = app();
        app.session.execute("file.new", &json!({"width": 3, "height": 2})).unwrap();
        let webp = bytes_of(&mut app, "document.serialize", json!({"format": "webp"}));
        open_bytes(&mut app, "tiny.webp", &webp, None).unwrap();
        assert_eq!(app.session.documents().len(), 2);
        assert_eq!(app.views.len(), 2, "views follow the documents");
        let first = app.session.doc().unwrap().doc.layers[0].children().unwrap()[0].id;
        assert_eq!(image_size(&app, first), (3, 2));
        app.session.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
        place_bytes(&mut app, "tiny.webp", &webp).unwrap();
        let placed = app.session.doc().unwrap().selection.objects[0];
        assert_eq!(image_size(&app, placed), (3, 2));
        assert!(place_bytes(&mut app, "x.xyz", b"hello").is_err());
    }

    #[test]
    fn templates_open_untitled() {
        let (mut app, _) = app();
        app.session.execute("file.new", &json!({"width": 50, "height": 50})).unwrap();
        let pdf = bytes_of(&mut app, "document.serialize", json!({"format": "pdf"}));
        open_bytes(&mut app, "brochure.ait", &pdf, Some("/tmp/brochure.ait".into())).unwrap();
        let st = app.session.active().unwrap();
        assert!(st.title().starts_with("Untitled-"), "{}", st.title());
        assert_eq!(st.path, None);
    }

    #[test]
    fn export_uses_the_engine_options_and_keeps_the_path() {
        let (mut app, written) = app();
        app.session.execute("file.new", &json!({"width": 100, "height": 50, "artboards": 2})).unwrap();
        app.session.execute("artboard.setProps", &json!({"index": 1, "width": 40})).unwrap();
        app.session.doc_mut().unwrap().path = Some("/tmp/doc.vectorcraft".into());
        export(&mut app, None, Some("/tmp/b.png".into()), &json!({"artboard": 1, "scale": 2})).unwrap();
        export(&mut app, None, Some("/tmp/copy.vectorcraft".into()), &Value::Null).unwrap();
        let w = written.borrow();
        assert_eq!(&w[0].1[16..20], 80u32.to_be_bytes(), "artboard 1 (40 pt) at scale 2");
        assert!(vectorcraft_format::sniff(&w[1].1));
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/doc.vectorcraft"));
        drop(w);
        assert!(export(&mut app, None, Some("/tmp/x.dwg".into()), &Value::Null).is_err(), "DWG can't be written");
    }

    #[test]
    fn export_for_screens_with_params_runs_the_engine() {
        let (mut app, _) = app();
        app.session.execute("file.new", &json!({"width": 60, "height": 40, "artboards": 3})).unwrap();
        let r = app.run("file.exportForScreens", json!({"range": "2-3", "formats": [{"format": "pdf"}]})).unwrap();
        let files = r["files"].as_array().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(files[0]["name"], "Artboard-2.pdf");
        assert!(app.ui.dialog.is_none());
        app.run("file.exportForScreens", Value::Null).unwrap();
        assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some("exportForScreens"));
    }

    #[test]
    fn export_selection_takes_its_format_from_the_picked_path() {
        let (mut app, written) = app();
        app.session.execute("file.new", &json!({"width": 60, "height": 40})).unwrap();
        app.session.execute("shape.rectangle", &json!({"x": 5, "y": 5, "width": 20, "height": 10})).unwrap();
        app.services.pick_save = Some(Box::new(|_: &FilePick| Some("/tmp/sel.svg".into())));
        assert_eq!(app.run("document.exportSelection", json!({})).unwrap()["path"], "/tmp/sel.svg");
        app.run("document.exportSelection", json!({"path": "/tmp/sel.png"})).unwrap();
        let w = written.borrow();
        assert!(String::from_utf8_lossy(&w[0].1).contains("<svg"), "SVG from the picked name");
        assert_eq!(&w[1].1[1..4], b"PNG", "PNG from the given path");
    }

    #[test]
    fn written_files_open_as_file_urls() {
        let url = super::file_url("/tmp/My Art #1.pdf");
        assert!(url.starts_with("file:///") && url.ends_with("/tmp/My%20Art%20%231.pdf"), "{url}");
        assert!(!url.contains('\\'), "{url}");
    }

    #[test]
    fn control_export_without_path_returns_bytes() {
        let (mut app, written) = app();
        app.session.execute("file.new", &json!({"width": 30, "height": 20})).unwrap();
        let (req, _) = crate::control::ControlRequest::new("app.export", json!({"format": "png", "scale": 2}));
        let crate::control::Outcome::Done(r) = crate::control::handle(&mut app, &egui::Context::default(), &req) else { panic!("not done") };
        assert_eq!(r["ok"], true, "{r}");
        let png = vectorcraft_format::base64_decode(r["result"]["dataBase64"].as_str().unwrap()).unwrap();
        assert_eq!(&png[16..20], 60u32.to_be_bytes(), "30 pt at scale 2");
        assert!(written.borrow().is_empty(), "no file and no save dialog");
    }
}
