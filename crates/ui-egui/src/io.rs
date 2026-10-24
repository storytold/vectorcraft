//! File I/O through the injected services (pick, read, write, download, reveal); the engine's
//! `fileio` decodes, encodes and saves every format.

use serde_json::{Value, json};
use vectorcraft_doc::SavedView;
use vectorcraft_doc::cancel;
use vectorcraft_engine::EngineError;
use vectorcraft_engine::cmd::fileio::progress::{self, Progress};
use vectorcraft_engine::cmd::fileio::{self, Format, SAVE_FORMATS, SaveMode, SavePlan, TEMPLATE_FORMATS};

use crate::background::{self, Writer};
use crate::dialogs::svg_options;
use crate::state::Dialog;
use crate::{FilePick, Services, VectorcraftApp, dialogs};

/// Template extensions New from Template's open dialog lists first.
const TEMPLATE_EXTS: &[&str] = &["vctemplate", "ait", "vectorcraft", "drawcraft"];

/// Open bytes of any readable format as a new document (templates open untitled); swatch and
/// graphic style library files open in the library panel, Libraries panel files are added to it,
/// flattener, PDF, print and perspective grid presets files are imported, a shortcut set another
/// app saved (`.kys`) opens Import Shortcut Set, and a script written for the reference app
/// (`.jsx`, `.jsxinc`, `.jsxbin`) shows its check (Check Script…). → `document.open`'s result for
/// a document opened now (its `warnings` say what didn't come in as it was), else null.
pub fn open_bytes(app: &mut VectorcraftApp, name: &str, bytes: &[u8], path: Option<String>) -> Result<Value, String> {
    let opens = opens_as(name);
    // A WebAssembly plug-in is installed.
    if opens == Some(OpensAs::Plugin) {
        let module = json!({"dataBase64": vectorcraft_format::base64_encode(bytes), "name": path.as_deref().unwrap_or(name)});
        return install_module(app, module).map(|_| Value::Null);
    }
    if opens == Some(OpensAs::ShortcutSet) {
        return dialogs::shortcut_import::open(app, name, bytes).map(|_| Value::Null);
    }
    if opens == Some(OpensAs::Script) {
        let p = match path {
            Some(path) => json!({ "path": path }),
            None => json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)}),
        };
        return dialogs::script_check::command(app, &p).map(|_| Value::Null);
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

/// The kinds of file File → Open installs, imports, checks, shows in the library panel or the
/// Libraries panel, or reviews in Import Shortcut Set ([`opens_as`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OpensAs {
    /// A WebAssembly plug-in: installed.
    Plugin,
    /// A shortcut set another app saved (`.kys`): reviewed in Import Shortcut Set.
    ShortcutSet,
    /// A script written for the reference app: checked without running it (Check Script…).
    Script,
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

/// The kind of file named `name` (by its extension) that File → Open installs, imports, checks,
/// shows in the library panel or the Libraries panel, or reviews in Import Shortcut Set
/// ([`open_bytes`]), else `None`. A drop opens these files the same way, wherever they are dropped
/// ([`VectorcraftApp::drop_target`]).
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
    } else if vectorcraft_engine::cmd::shortcutset::EXTS.contains(&ext.as_str()) {
        Some(OpensAs::ShortcutSet)
    } else if vectorcraft_engine::cmd::scripts::EXTS.contains(&ext.as_str()) {
        Some(OpensAs::Script)
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
    opened(app, &r);
    Ok(r)
}

/// A document just opened (`r`: `document.open`'s result): the views follow, the status bar says
/// what didn't come in as it was, and missing links are asked about.
pub(crate) fn opened(app: &mut VectorcraftApp, r: &Value) {
    app.sync_views();
    if let Some(w) = r["warnings"].as_array().filter(|w| !w.is_empty()) {
        app.status(format!("Opened with {} note(s): {}", w.len(), w[0].as_str().unwrap_or_default()));
    }
    crate::dialogs::missing_links::after_open(app, r);
}

/// A path from the open dialog, or "cancelled".
fn pick_open(app: &mut VectorcraftApp, pick: &FilePick) -> Result<String, String> {
    crate::picks::open(app, pick).ok_or_else(|| "cancelled".into())
}

/// Object › Plug-ins › Install Plug-in…: installs the module `p` gives (`path`, or `dataBase64`
/// with `name`), else a picked `.wasm` (the web's file picker hands the file to [`open_bytes`],
/// which installs it) → [`install_module`]'s result.
pub fn install_plugin(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let given = |k: &str| p.get(k).and_then(Value::as_str).is_some_and(|v| !v.is_empty());
    if given("path") || given("dataBase64") {
        return install_module(app, p.clone());
    }
    if app.services.open_async.is_some() {
        return open_dialog(app).map(|_| Value::Null);
    }
    let path =
        pick_open(app, &FilePick { filters: vec![FilePick::filter("Plug-ins", vectorcraft_engine::cmd::plugin::EXTS)], ..Default::default() })?;
    install_module(app, json!({ "path": path }))
}

/// Installs the plug-in module that `module` names (`plugin.install`'s `path`, or `dataBase64`
/// with `name`) → `plugin.install`'s result. A filter that asks for a heavy budget the user hasn't
/// allowed yet (`allowHeavy`) asks them first, and OK installs it → `{pending}` while it asks.
pub fn install_module(app: &mut VectorcraftApp, module: Value) -> Result<Value, String> {
    if module.get("allowHeavy").and_then(Value::as_bool) != Some(true) {
        let given = |k: &str| module.get(k).map(|v| (k.to_string(), v.clone()));
        let source = Value::Object(["path", "dataBase64", "name"].into_iter().filter_map(given).collect());
        let info = app.run("plugin.inspect", source.clone())?;
        if info["budget"] == "heavy" {
            let name = info["name"].as_str().unwrap_or_default();
            let (minutes, times) = vectorcraft_plugins::heavy_terms();
            let message = crate::i18n::fmt(tl!("Install “{name}”, which asks for a heavy budget?"), &[("name", name)]);
            let detail = crate::i18n::fmt(
                tl!(
                    "Each run may take up to {minutes} minutes and do {times} times the work other plug-ins may. It runs in the background, and you can cancel it."
                ),
                &[("minutes", &minutes.to_string()), ("times", &times.to_string())],
            );
            let mut allowed = source;
            allowed["allowHeavy"] = json!(true);
            dialogs::confirm::ask(app, &message, &detail, "ui.installPlugin", allowed);
            return Ok(json!({ "pending": dialogs::confirm::KIND }));
        }
    }
    let r = app.run("plugin.install", module)?;
    app.status(format!("Installed plug-in {}", r["name"].as_str().unwrap_or_default()));
    Ok(r)
}

/// File → Open…
pub fn open_dialog(app: &mut VectorcraftApp) -> Result<(), String> {
    if let Some(f) = app.services.open_async.as_mut() {
        f();
        return Ok(());
    }
    let path = pick_open(app, &FilePick { filters: FilePick::filters(fileio::open_filters()), ..Default::default() })?;
    open_reporting(app, &path).map(|_| ())
}

/// A file the user asked to open (File › Open, Open Recent, the Home screen, a drop, the Finder)
/// couldn't be: say so in a dialog as well as the status bar, which is easily missed (#861).
pub fn report_open_error(app: &mut VectorcraftApp, name: &str, e: &str) {
    app.status(format!("Couldn't open {name}: {e}"));
    let message = crate::i18n::fmt(tl!("Can't open “{name}”."), &[("name", name)]);
    dialogs::confirm::tell(app, &message, &crate::i18n::message(crate::i18n::current(), e));
}

/// A save or an export that didn't write `path`: say so in a dialog that names the file and the
/// reason and stays until it is closed, as an editor does (#1685). The status bar's line, which
/// the caller's error becomes, is replaced by the next message, and a document that wasn't saved
/// must not look saved: it stays modified, and what was at `path` is as it was. Not for a request
/// an agent sent and waits on: the error is its reply.
pub(crate) fn report_write_error(app: &mut VectorcraftApp, export: bool, path: &str, e: &str) {
    if app.ui.answering_control {
        return;
    }
    let name = fileio::file_name(path);
    let message = if export { tl!("Can't export “{name}”.") } else { tl!("Can't save “{name}”.") };
    dialogs::confirm::tell(app, &crate::i18n::fmt(message, &[("name", &name)]), &crate::i18n::message(crate::i18n::current(), e));
}

/// [`write_to`], a failure reported ([`report_write_error`]).
pub(crate) fn write_reporting(app: &mut VectorcraftApp, path: &str, bytes: &[u8]) -> Result<(), String> {
    let r = write_to(&mut app.services, path, bytes);
    if let Err(e) = &r {
        report_write_error(app, false, path, e);
    }
    r
}

/// [`open_path`] for a file the user asked for, a failure reported ([`report_open_error`]). On
/// the desktop a document is decoded off the UI thread ([`crate::opening`]): the result is then
/// `{background: true, status}`, and the document opens in its tab (or the failure is reported)
/// once it is ready.
pub fn open_reporting(app: &mut VectorcraftApp, path: &str) -> Result<Value, String> {
    let r = open_file(app, path, crate::opening::available());
    if let Err(e) = &r {
        report_open_error(app, &fileio::file_name(path), e);
    }
    r
}

/// Home › Recent Files and File › Open Recent: focus an already-open tab for that path instead of
/// calling `document.open` again (#3455).
pub fn open_recent_reporting(app: &mut VectorcraftApp, path: &str) -> Result<Value, String> {
    if let Some(r) = try_activate_open_document(app, path)? {
        return Ok(r);
    }
    open_reporting(app, path)
}

fn try_activate_open_document(app: &mut VectorcraftApp, path: &str) -> Result<Option<Value>, String> {
    let Some(index) = document_index_for_path(app, path) else {
        return Ok(None);
    };
    let st = &app.session.documents()[index];
    let r = json!({
        "index": index,
        "title": st.title(),
        "format": st.format,
        "warnings": [],
        "restored": false,
        "activated": true,
    });
    app.run("document.activate", json!({ "index": index }))?;
    opened(app, &r);
    Ok(Some(r))
}

fn document_index_for_path(app: &VectorcraftApp, path: &str) -> Option<usize> {
    app.session.documents().iter().position(|st| st.path.as_deref().is_some_and(|p| same_open_path(p, path)))
}

fn same_open_path(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn read(app: &VectorcraftApp, path: &str) -> Result<Vec<u8>, String> {
    app.services.read.as_ref().ok_or("no file reader")?(path)
}

/// Open the file at `path` (see [`open_bytes`]) → `document.open`'s result, or null.
pub fn open_path(app: &mut VectorcraftApp, path: &str) -> Result<Value, String> {
    open_file(app, path, false)
}

/// [`open_path`]; with `background`, a document (not a file that installs, imports or asks
/// first) is decoded off the UI thread ([`crate::opening::start`]).
pub(crate) fn open_file(app: &mut VectorcraftApp, path: &str, background: bool) -> Result<Value, String> {
    // A shortcut set is read by Import Shortcut Set, up to its reader's cap.
    if dialogs::shortcut_import::is_kys(path) {
        dialogs::shortcut_import::command(app, &json!({ "path": path }))?;
        note_recent(app, path);
        return Ok(Value::Null);
    }
    // A script is read by `script.check`, up to its limit.
    if opens_as(path) == Some(OpensAs::Script) {
        dialogs::script_check::command(app, &json!({ "path": path }))?;
        note_recent(app, path);
        return Ok(Value::Null);
    }
    let mut bytes = read(app, path)?;
    if background && opens_as(path).is_none() {
        // A PDF with several pages or a password, or a DXF drawing, asks first, as at once.
        if crate::dialogs::import_pdf::offer(app, path, &bytes, Some(path.to_string()), None)
            || crate::dialogs::dxf_import::offer(app, path, &bytes, Some(path.to_string()), None)
        {
            note_recent(app, path);
            return Ok(Value::Null);
        }
        match crate::opening::start(app, path, bytes) {
            Ok(r) => return Ok(r),
            // No thread: at once.
            Err(back) => bytes = back,
        }
    }
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
            let filters = FilePick::filters(std::iter::once(("Templates", TEMPLATE_EXTS)).chain(fileio::open_filters()));
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
        return dl(&fileio::file_name(path), bytes);
    }
    services.write.as_mut().ok_or("no file writer")?(path, bytes)
}

/// The web saves by downloading: no save panel, no folders.
pub(crate) fn is_web(app: &VectorcraftApp) -> bool {
    app.services.download.is_some()
}

/// Write `bytes` to `path`, else to a file picked in a save panel that suggests `name` next to the
/// document and shows its file type, [`file_types`] (the web downloads them as `name`) → where
/// they went.
pub(crate) fn write_named(app: &mut VectorcraftApp, path: Option<String>, name: &str, bytes: &[u8]) -> Result<String, String> {
    let path = match path {
        Some(p) => p,
        None => {
            let (_, folder) = suggested(app, "");
            let filters = file_types(&fileio::extension(name));
            pick_path(app, &FilePick { name: name.to_string(), folder, filters }, |_| false)?
        }
    };
    write_reporting(app, &path, bytes)?;
    Ok(path)
}

/// Where a file goes when no path was given: the suggested name on the web (a download), else the
/// save panel's choice with the suggested name's extension, [`with_written_extension`]: a name
/// typed with the extension of a format `keeps` takes stays as typed too.
fn pick_path(app: &mut VectorcraftApp, pick: &FilePick, keeps: impl Fn(&Format) -> bool) -> Result<String, String> {
    let ext = fileio::extension(&pick.name);
    pick_finished(app, pick, |picked, _| with_written_extension(picked, &ext, &keeps))
}

/// `path` as typed in a save panel for a file with extension `ext`: kept when its extension is
/// `ext` in any letter case, or names the format `ext` names (`photo.jpeg` for a `.jpg` file) or
/// a format `keeps` takes. An extension that names another format (`card.svg` for a `.png` file)
/// is replaced with `ext`, and `.ext` is added to any other path (`v1.2` becomes `v1.2.png`).
fn with_written_extension(path: &str, ext: &str, keeps: impl Fn(&Format) -> bool) -> String {
    let written = fileio::format(ext);
    let keeps = |g: &Format| written.is_some_and(|w| w.id == g.id) || keeps(g);
    if !ext.is_empty() && !fileio::extension(path).eq_ignore_ascii_case(ext) && fileio::format_for_name(path).is_some_and(|g| !keeps(g)) {
        return std::path::Path::new(path).with_extension(ext).to_string_lossy().into_owned();
    }
    with_extension(path, ext, keeps)
}

/// The suggested name made into a path by `finish` on the web (a download), else the save panel's
/// choice made into one, with the extension of the file type picked in the panel when it reports
/// one ([`crate::picks::PickAnswer`]). When `finish` changed the choice (an extension added or
/// replaced) and a file is there, which the panel did not ask about, a question asks before
/// replacing it ([`crate::picks::ask_to_replace`]) and this answers "cancelled"; the question's OK
/// runs what asked again.
fn pick_finished(app: &mut VectorcraftApp, pick: &FilePick, finish: impl Fn(&str, Option<&str>) -> String) -> Result<String, String> {
    if is_web(app) {
        return Ok(finish(&pick.name, None));
    }
    let (picked, file_type) = crate::picks::save_with_type(app, pick).ok_or("cancelled")?;
    let path = finish(&picked, file_type.as_deref());
    if path != picked && crate::picks::ask_to_replace(app, crate::picks::PickRequest::Save(pick.clone()), &picked, file_type.as_deref(), &path) {
        return Err("cancelled".into());
    }
    Ok(path)
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

/// The save panel's file type, `(label, extensions)`, for the files `write_named` and
/// `target_path` write that are not a document format (`fileio::format` has those).
const FILE_TYPES: &[(&str, &[&str])] = &[
    ("Swatch libraries", &["vcswatches"]),
    ("Swatch libraries", &["gpl"]),
    ("Swatch libraries", &["ase"]),
    ("Graphic Style Libraries", &[vectorcraft_doc::style_libs::STYLES_EXT]),
    ("Libraries", vectorcraft_engine::cmd::library::LIBRARY_EXTS),
    ("Flattener presets", vectorcraft_engine::cmd::flatten::PRESET_EXTS),
    ("PDF presets", vectorcraft_engine::cmd::pdfcmds::PRESET_EXTS),
    ("Perspective grid presets", vectorcraft_engine::cmd::perspgrid::PRESET_EXTS),
    ("Print presets", vectorcraft_engine::cmd::printpresets::PRESET_EXTS),
    ("CSS", &["css"]),
    ("PostScript", &["ps"]),
];

/// The file type a save panel shows for a file with extension `ext`: its format's, else its
/// [`FILE_TYPES`] row; none for any other extension.
fn file_types(ext: &str) -> Vec<crate::FileFilter> {
    if let Some(f) = fileio::format(ext) {
        return vec![FilePick::filter(f.label, f.extensions)];
    }
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    FilePick::filters(FILE_TYPES.iter().find(|(_, exts)| exts.contains(&ext.as_str())).copied())
}

/// `path`, else one picked in a save panel (the web: the suggested name) for the active document
/// with extension `ext`, filtered to its file type when it has one ([`file_types`]). A name typed
/// with another format's extension gets `ext` instead ([`with_written_extension`]).
pub(crate) fn target_path(app: &mut VectorcraftApp, path: Option<String>, ext: &str) -> Result<String, String> {
    target_path_keeping(app, path, ext, |_| false)
}

/// [`target_path`] for an export in format `f` (Export As's OK and [`export`]): a name typed with
/// `.svgz` for an SVG stays as typed, and [`export`] writes it compressed.
pub(crate) fn export_path(app: &mut VectorcraftApp, path: Option<String>, f: &Format) -> Result<String, String> {
    target_path_keeping(app, path, f.extensions[0], |g| f.id == "svg" && g.id == "svgz")
}

/// [`target_path`], keeping a typed extension of a format `keeps` takes.
fn target_path_keeping(app: &mut VectorcraftApp, path: Option<String>, ext: &str, keeps: impl Fn(&Format) -> bool) -> Result<String, String> {
    if let Some(p) = path {
        return Ok(p);
    }
    let (name, folder) = suggested(app, ext);
    let filters = file_types(ext);
    pick_path(app, &FilePick { name, folder, filters }, keeps)
}

/// Keep the active document's saved view current before a save (native files reopen at it).
pub fn remember_view(app: &mut VectorcraftApp) {
    let Some(v) = app.view().copied().filter(|v| v.fitted) else { return };
    if let Some(st) = app.session.active_mut() {
        st.view = Some(SavedView { name: String::new(), center: v.center, zoom: v.zoom, rotation: v.rotation });
    }
}

/// A path typed in a save panel: kept when its extension names one of the panel's file types (a
/// save format, or a template format for Save as Template, `mode`). An extension that names
/// another format (`card.png`, or `card.ait` in Save As) is replaced with `f`'s, and `f`'s is
/// added to any other path (`notes.v2` becomes `notes.v2.vectorcraft`).
fn with_save_extension(path: &str, f: &Format, mode: SaveMode) -> String {
    let formats = if mode == SaveMode::Template { TEMPLATE_FORMATS } else { SAVE_FORMATS };
    if fileio::format_for_name(path).is_some_and(|g| !formats.contains(&g.id)) {
        return std::path::Path::new(path).with_extension(f.extensions[0]).to_string_lossy().into_owned();
    }
    with_extension(path, f.extensions[0], |g| formats.contains(&g.id))
}

/// [`with_save_extension`] for a save panel that reports the file type picked in it (`picked`,
/// that type's extension; the XDG portal's panel on Linux and the BSDs), which then takes `f`'s
/// place. A path that still ends in the extension of the name the panel suggested (`suggested`),
/// in any letter case, gets the picked type's extension instead, unless that extension is one of
/// the picked type's; such a path stays as typed. Without a picked type, or with one the panel did not
/// offer, [`with_save_extension`] with `f`.
fn with_picked_type(path: &str, suggested: &str, picked: Option<&str>, f: &Format, mode: SaveMode) -> String {
    let formats = if mode == SaveMode::Template { TEMPLATE_FORMATS } else { SAVE_FORMATS };
    let Some(g) = picked.and_then(fileio::format).filter(|g| formats.contains(&g.id)) else {
        return with_save_extension(path, f, mode);
    };
    let has = fileio::extension(path);
    if !has.is_empty() && has == fileio::extension(suggested) && !g.extensions.contains(&has.as_str()) {
        return std::path::Path::new(path).with_extension(g.extensions[0]).to_string_lossy().into_owned();
    }
    with_save_extension(path, g, mode)
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
/// When no path is known, desktop asks with a save panel (one file type per save format, or one
/// per template format for Save as Template) and the web opens the Save As dialog (file name and
/// format). With `ask_options` and no options in `p`,
/// a format picked that way that has options asks for them before anything is written (SVG
/// Options, the Save PDF dialog, or the save options dialog: native files on Save As only, `.ai`
/// files on Save As and Save a Copy), and so does Save As or Save a Copy to a given SVG path.
/// → `{path, format, warnings…}` once written, or `{pending: <dialog kind>,
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
    let filters = FilePick::filters(match mode {
        SaveMode::Template => fileio::template_filters(first.format.id),
        _ => fileio::save_filters(first.format.id),
    });
    let pick = FilePick { name: first.name.clone(), folder: first.folder.clone(), filters };
    let picked = pick_finished(app, &pick, |picked, file_type| with_picked_type(picked, &first.name, file_type, first.format, mode))?;
    // The picked file type wins over a `format` param.
    let mut q = if p.is_object() { p.clone() } else { json!({}) };
    if let Some(o) = q.as_object_mut() {
        o.remove("format");
        o.insert("path".into(), json!(picked));
    }
    let chosen = plan(app, mode, &acknowledged(&q))?;
    if let Some(r) = chosen.path.as_deref().and_then(|path| ask_before_losing(app, path, save_command(mode), &q)) {
        return Ok(r);
    }
    if ask && asks_options(mode, chosen.format) {
        let path = chosen.path.clone().unwrap_or_default();
        let r = ask_format_options(app, mode, chosen.format, &path);
        crate::dialogs::opened_by_panel(app);
        return r;
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
/// files on Save As (the options dialog after the save panel), and `.ai` files also on Save a Copy,
/// so the copy is not written in the version last saved whatever the panel's `.ai` type says
/// (#3141); Save and templates reuse the remembered options or Use Compression.
fn asks_options(mode: SaveMode, f: &Format) -> bool {
    !f.options.is_empty()
        && match f.id {
            "vectorcraft" => mode == SaveMode::SaveAs,
            "ai" => matches!(mode, SaveMode::SaveAs | SaveMode::Copy),
            "template" | "ait" => false,
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
    app.status(saved_status(path, r));
}

/// [`report_saved`]'s message.
fn saved_status(path: &str, r: &Value) -> String {
    let warnings: Vec<&str> = r["warnings"].as_array().map(|w| w.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
    match warnings.first() {
        Some(first) => format!("Saved {path} with {} note(s): {first}", warnings.len()),
        None => format!("Saved {path}"),
    }
}

/// Write a planned save through the services and report it in the status bar.
/// With Background Save on, the file is encoded and written from a snapshot off the UI thread
/// ([`background`]): this returns `{path, background: true, status, format, warnings}` at once
/// (`warnings`: what the format loses, then the save's notes on its params, such as an option the format
/// doesn't read), and the document takes the file (and counts as saved, as of the snapshot) once it is written.
fn write_plan(app: &mut VectorcraftApp, plan: SavePlan) -> Result<Value, String> {
    let uid = app.session.active().ok_or("no document")?.uid;
    // Saves of one document finish in order.
    if app.background.busy_with(uid) {
        background::wait_all(app);
    }
    let job = fileio::save_job(&mut app.session, plan).map_err(|e| e.to_string())?;
    let (retargets, target) = (job.plan().retargets(), job.plan().path.clone());
    let (format, warnings) = (job.plan().format.id, job.known_warnings());
    let label = match &target {
        Some(path) => format!("Saving {}", fileio::file_name(path)),
        None => "Saving".into(),
    };
    // Without a path the bytes come back to the caller: nothing to do in the background.
    let enabled = app.session.prefs.background_save && target.is_some();
    // The work writes the file; the copy makes it the document's own afterwards.
    let done = job.clone();
    // A native file streams into the file as it is encoded (desktop; not to a download).
    let stream = app.services.stream_shared.clone().filter(|_| job.streams() && !is_web(app));
    let work = move |write: Writer| {
        match stream {
            Some(stream) => job.write_streamed(&mut |p, fill| stream(p, fill).map_err(EngineError::Other)),
            None => job.write(|p, b| write(p, b).map_err(EngineError::Other)),
        }
        .map_err(|e| e.to_string())
    };
    let failed = target.clone().unwrap_or_default();
    let then = move |app: &mut VectorcraftApp, r: Result<Value, String>| {
        // Not written: the document keeps its file, format and modified state, and the user is told.
        let r = r.inspect_err(|e| report_write_error(app, false, &failed, e))?;
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
    if r.get("background").and_then(Value::as_bool) == Some(true) {
        r["format"] = json!(format);
        r["warnings"] = json!(warnings);
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

/// The status once an export was cancelled.
pub const EXPORT_CANCELLED: &str = "Export cancelled";

/// File → Cancel Export (the progress dialog's Cancel): every export running in the background
/// stops and writes nothing more → `{cancelled: n}`.
pub fn cancel_exports(app: &mut VectorcraftApp) -> Value {
    let n = app.background.cancel_exports();
    if n > 0 {
        app.status(EXPORT_CANCELLED);
    }
    json!({ "cancelled": n })
}

/// The progress dialog's Continue in Background: the exports go on, shown in the status bar only
/// → `{hidden: n}`.
pub fn hide_exports(app: &mut VectorcraftApp) -> Value {
    json!({ "hidden": app.background.hide_exports() })
}

/// Export the active document in `format` (default: the path's extension, else PNG) with the
/// `document.export` options in `params` (artboard, range, useArtboards, selectedOnly, ppi, SVG
/// options…): every file it writes (one per artboard, linked images) goes next to `path`. The
/// document keeps its path. → `{path, warnings, files?}` (`files`: every file when there are
/// several).
pub fn export(app: &mut VectorcraftApp, format: Option<&str>, path: Option<String>, params: &Value) -> Result<Value, String> {
    // Options given as anything but an object are none: the path and format still reach the
    // encoder (an exporter plug-in is told the file it writes).
    let params = &if params.is_object() { params.clone() } else { json!({}) };
    let f = fileio::writable_format(format, path.as_deref())?;
    let st = app.session.active().ok_or("no document")?;
    let doc = match fileio::export_source(st, params).map_err(|e| e.to_string())? {
        std::borrow::Cow::Borrowed(_) => st.doc.clone(),
        std::borrow::Cow::Owned(d) => std::sync::Arc::new(d),
    };
    let doc = fileio::titled_for_export(&doc, st.path.as_deref(), f.id).map_or(doc, std::sync::Arc::new);
    let path = export_path(app, path, f)?;
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
    // Background Export: encoded and written from this snapshot while editing goes on. The
    // encoder is told the file it writes (an exporter plug-in gets its name and extension).
    let label = format!("Exporting {}", fileio::file_name(&path));
    let (mut params, target) = (params.clone(), path.clone());
    if let Some(o) = params.as_object_mut() {
        o.insert("path".into(), json!(path));
    }
    // The progress dialog's counts and Cancel: a cancelled export stops drawing and never starts
    // writing.
    let progress = Progress::new();
    let (reporting, asked) = (progress.clone(), progress.clone());
    let work = move |write: Writer| {
        progress::scope(&reporting, || {
            cancel::check().map_err(|e| e.to_string())?;
            let enc = fileio::encode_all(&doc, f.id, &params).map_err(|e| e.to_string())?;
            cancel::check().map_err(|e| e.to_string())?;
            let files = write_encoded(write, &doc, &path, &enc)?;
            let mut out = json!({ "path": files.first().unwrap_or(&path), "warnings": enc.warnings });
            if files.len() > 1 {
                out["files"] = json!(files);
            }
            Ok(out)
        })
    };
    let enabled = app.session.prefs.background_export;
    let failed = target.clone();
    let mut r = background::run_with_progress(app, enabled, label, None, Some(progress), work, move |app, r| {
        // A file written before the Cancel arrived is reported as written.
        if r.is_err() && asked.cancelled() {
            app.status(EXPORT_CANCELLED);
            return Ok(json!({ "cancelled": true }));
        }
        let r = r.inspect_err(|e| report_write_error(app, true, &failed, e))?;
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
/// (Export Selection, Save for Office Documents, Text Export Options and the presets dialogs'
/// Export…), reported with the first of the command's warnings ([`report_saved`]).
pub fn save_command_output(app: &mut VectorcraftApp, id: &str, ext: &str, params: Value) -> Result<String, String> {
    let (path, v) = run_to_file(app, id, ext, params)?;
    report_saved(app, &path, &v);
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
/// for Export Selection without a `format`, which takes it from the picked path. Export Selection
/// with a `format` picks a file with that format's extension in place of `ext`.
pub(crate) fn run_to_file(app: &mut VectorcraftApp, id: &str, ext: &str, mut params: Value) -> Result<(String, Value), String> {
    let mut path = params.as_object_mut().and_then(|o| o.remove("path")).and_then(|p| p.as_str().map(str::to_string));
    let selection = id == "document.exportSelection";
    let given = params.get("format").and_then(Value::as_str).and_then(fileio::format).filter(|_| selection);
    let ext = given.map_or(ext, |f| f.extensions[0]);
    if let Some(o) = params.as_object_mut()
        && o.get("format").is_none()
        && selection
    {
        let picked = target_path_keeping(app, path, ext, |_| true)?;
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
    write_reporting(app, &path, &bytes)?;
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
/// back as base64. With `openLocation`, the file manager shows the first file written. When an
/// export into a folder or a download has warnings, the status bar shows the first as
/// [`report_saved`] shows a save's notes: "Saved <folder, ZIP or first download> with n note(s): …".
pub fn export_for_screens(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    export_files(app, "document.exportForScreens", params)
}

/// [`export_for_screens`] through command `id` (`document.exportForScreens` or `assets.export`).
/// On the desktop with Background Export on, an export into a folder is checked, journaled and
/// prepared from the document as it is now, and the files are encoded and written on the
/// background thread ([`background`]): this returns `{background: true, status}` at once, with the
/// progress dialog's Cancel, and the status bar reports the files once they are written.
pub(crate) fn export_files(app: &mut VectorcraftApp, id: &str, params: Value) -> Result<Value, String> {
    let open_location = params.get("openLocation").and_then(Value::as_bool).unwrap_or(false);
    let folder = params.get("folder").and_then(Value::as_str).unwrap_or_default().to_string();
    // Without a folder the files come back in the reply, which the caller waits for.
    let in_background = app.session.prefs.background_export && app.services.write_shared.is_some() && !is_web(app) && !folder.is_empty();
    if !in_background {
        // Made at once, after the exports and saves started before it.
        background::wait_all(app);
        let r = app.run(id, params)?;
        return report_exported(app, r, &folder, open_location);
    }
    let export = app.session.prepare_screen_export(id, &params).map_err(|e| e.to_string()).inspect_err(|e| app.ui.status = e.clone())?;
    let label = format!("Exporting {}", fileio::file_name(&folder));
    // The progress dialog counts the files, and its Cancel stops the export before the next one.
    let progress = Progress::of_files();
    let (reporting, asked) = (progress.clone(), progress.clone());
    let work = move |write: Writer| {
        progress::scope(&reporting, || {
            let mut write = |path: &str, bytes: &[u8]| write(path, bytes).map_err(|e| EngineError::Other(format!("{path}: {e}")));
            export.write(&mut write).map_err(|e| e.to_string())
        })
    };
    background::run_with_progress(app, true, label, None, Some(progress), work, move |app, r| {
        // Files written before the Cancel arrived stay in the folder.
        if r.is_err() && asked.cancelled() {
            app.status(EXPORT_CANCELLED);
            return Ok(json!({ "cancelled": true }));
        }
        let r = r.inspect_err(|e| report_write_error(app, true, &folder, e))?;
        report_exported(app, r, &folder, open_location)
    })
}

/// Report the files `r` (the reply of `document.exportForScreens`) in the status bar: the count
/// written into `folder`, else the ZIP or the files downloaded (the web); with `open_location`,
/// the file manager shows the first file written.
fn report_exported(app: &mut VectorcraftApp, r: Value, folder: &str, open_location: bool) -> Result<Value, String> {
    let count = r["files"].as_array().map_or(0, Vec::len);
    let warned = r["warnings"].as_array().is_some_and(|w| !w.is_empty());
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
        if warned {
            status = saved_status(r["path"].as_str().unwrap_or(folder), &r);
        }
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
            download(name, &bytes)?;
        }
        let what = match returned.as_slice() {
            [(name, _)] => (*name).to_string(),
            files => format!("{} files", files.len()),
        };
        match returned.first().filter(|_| warned) {
            Some((name, _)) => report_saved(app, name, &r),
            None => app.status(format!("Downloaded {what}")),
        }
    }
    Ok(r)
}

/// Place a file's bytes (no path, so embedded) centered in the view: `file.place`, read off the UI
/// thread on the desktop ([`crate::place::run_for_user`]).
pub fn place_bytes(app: &mut VectorcraftApp, name: &str, bytes: &[u8]) -> Result<(), String> {
    crate::place::run_for_user(app, &serde_json::json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) })).map(|_| ())
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

    /// #1685: a save or an export that can't write its file says so in a dialog that names the
    /// file and the reason and stays until it is closed; the document stays modified and keeps
    /// the file it had. An agent's request only gets the error, and a message that arrives while
    /// another dialog is open waits for it to close.
    #[test]
    fn a_save_that_cannot_write_is_reported_in_a_dialog_that_stays() {
        let refuse: crate::WriteFn = Box::new(|p: &str, _: &[u8]| Err(format!("{p}: Access is denied. (os error 5)")));
        let mut app = VectorcraftApp::new(Session::new(), Services { write: Some(refuse), ..Default::default() });
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let unsaved = |app: &VectorcraftApp| {
            let st = app.session.doc().unwrap();
            (st.is_dirty(), st.path.clone())
        };
        for (command, path, message) in [
            ("file.save", "/docs/Poster.vectorcraft", "Can't save “Poster.vectorcraft”."),
            ("file.saveAs", "/docs/Poster.vectorcraft", "Can't save “Poster.vectorcraft”."),
            ("file.saveCopy", "/docs/Copy.vectorcraft", "Can't save “Copy.vectorcraft”."),
            ("file.exportAs", "/docs/Poster.png", "Can't export “Poster.png”."),
            ("file.export.pdf", "/docs/Poster.pdf", "Can't save “Poster.pdf”."),
        ] {
            let e = app.run(command, json!({"path": path})).unwrap_err();
            assert!(e.contains("Access is denied"), "{command}: {e}");
            let d = app.ui.dialog.take().unwrap_or_else(|| panic!("{command}: a dialog says so"));
            assert_eq!((d.kind.as_str(), d.str("message").as_str()), (dialogs::confirm::MESSAGE, message), "{command}");
            assert_eq!(d.str("detail"), format!("{path}: Access is denied. (os error 5)"), "{command}");
            assert_eq!(unsaved(&app), (true, None), "{command}");
        }
        // An agent waiting on its request gets the error as its reply, and no dialog.
        app.ui.answering_control = true;
        assert!(app.run("file.save", json!({"path": "/docs/Poster.vectorcraft"})).is_err());
        app.ui.answering_control = false;
        assert!(app.ui.dialog.is_none() && app.ui.told.is_empty());
        // Under another dialog (a background save that fails later) the message waits its turn,
        // once, and is drawn when that dialog is closed.
        dialogs::confirm::ask(&mut app, "Delete the swatch?", "", "edit.undo", json!({}));
        for _ in 0..2 {
            report_write_error(&mut app, false, "/docs/Poster.vectorcraft", "the disk is full");
        }
        assert_eq!((app.ui.dialog.as_ref().map(|d| d.kind.as_str()), app.ui.told.len()), (Some(dialogs::confirm::KIND), 1));
        app.ui.dialog = None;
        for _ in 0..2 {
            let ctx = egui::Context::default();
            crate::theme::install_fonts(&ctx);
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| dialogs::show(&mut app, ui.ctx()));
            out.textures_delta.clear();
        }
        let d = app.ui.dialog.as_ref().expect("the message is shown, and stays");
        assert_eq!(
            (d.kind.as_str(), d.str("message").as_str(), d.str("detail").as_str()),
            (dialogs::confirm::MESSAGE, "Can't save “Poster.vectorcraft”.", "the disk is full")
        );
        assert!(app.ui.told.is_empty());
    }

    /// #1693: a browser download that can't be started is a failed save: the document stays
    /// modified and without a file, and Save Changes doesn't close it.
    #[test]
    fn a_download_that_cannot_start_leaves_the_document_unsaved_and_open() {
        let started = Rc::new(RefCell::new(false));
        let ok = started.clone();
        let download: crate::DownloadFn =
            Box::new(move |_: &str, _: &[u8]| if *ok.borrow() { Ok(()) } else { Err("the browser refused the download".into()) });
        let mut app = VectorcraftApp::new(Session::new(), Services { download: Some(download), ..Default::default() });
        app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
        app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
        let unsaved = |app: &VectorcraftApp| {
            let st = app.session.doc().unwrap();
            (st.is_dirty(), st.path.clone())
        };
        let e = save(&mut app, SaveMode::Save, &json!({"path": "Poster.vectorcraft"}), false).unwrap_err();
        assert!(e.contains("the browser refused the download"), "{e}");
        assert_eq!(unsaved(&app), (true, None));
        // Save Changes › Save: not saved, so not closed.
        let (i, uid) = (app.session.active_index().unwrap(), app.session.doc().unwrap().uid);
        app.ui.dialog = Some(crate::state::Dialog::new(crate::unsaved::KIND, json!({"index": i, "uid": uid, "name": "Poster", "then": "close"})));
        app.session.doc_mut().unwrap().path = Some("Poster.vectorcraft".into());
        assert!(crate::unsaved::confirm(&mut app).is_err());
        assert_eq!((app.session.documents().len(), app.session.doc().unwrap().is_dirty()), (1, true));
        // Once the download starts, the save counts.
        *started.borrow_mut() = true;
        save(&mut app, SaveMode::Save, &json!({}), false).unwrap();
        assert!(!app.session.doc().unwrap().is_dirty());
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
