use super::save::suggested;
use super::*;

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

/// Where File → Open… starts: the folder of the most recently opened file, else the active
/// document's (#4122).
fn open_dialog_folder(app: &VectorcraftApp) -> Option<String> {
    app.ui.recent_files.iter().find_map(|p| fileio::dialog_folder(p)).or_else(|| suggested(app, "").1)
}

/// File → Open…
pub fn open_dialog(app: &mut VectorcraftApp) -> Result<(), String> {
    if let Some(f) = app.services.open_async.as_mut() {
        f();
        return Ok(());
    }
    let path =
        pick_open(app, &FilePick { filters: FilePick::filters(fileio::open_filters()), folder: open_dialog_folder(app), ..Default::default() })?;
    open_reporting(app, &path).map(|_| ())
}

/// A file the user asked to open (File › Open, Open Recent, the Home screen, a drop, the Finder)
/// couldn't be: say so in a dialog as well as the status bar, which is easily missed (#861).
pub fn report_open_error(app: &mut VectorcraftApp, name: &str, e: &str) {
    app.status(format!("Couldn't open {name}: {e}"));
    let message = crate::i18n::fmt(tl!("Can't open “{name}”."), &[("name", name)]);
    dialogs::confirm::tell(app, &message, &crate::i18n::message(crate::i18n::current(), e));
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

/// Open the save options dialog of format `f` before writing `path`: `action` is the save command
/// it finishes, and `pick` (the web's Save As) also names the file and chooses the format.
pub(super) fn open_options(app: &mut VectorcraftApp, action: &str, f: &Format, path: &str, pick: bool) -> Value {
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

/// File → Revert: ask first; OK runs `file.revert` with `confirmed`, which goes to the engine.
pub fn ask_revert(app: &mut VectorcraftApp) -> Result<Value, String> {
    let c = vectorcraft_engine::find_command("file.revert").ok_or("no revert command")?;
    (c.enabled)(&app.session)?;
    let name = app.session.active().map(|d| d.title()).unwrap_or_default();
    let message = crate::i18n::fmt(tl!("Revert to the saved version of “{name}”?"), &[("name", &name)]);
    dialogs::confirm::ask(app, &message, tl!("Changes made since it was last saved will be lost."), "file.revert", json!({ "confirmed": true }));
    Ok(json!({ "pending": dialogs::confirm::KIND }))
}

/// Place a file's bytes (no path, so embedded) centered in the view: `file.place`, read off the UI
/// thread on the desktop ([`crate::place::run_for_user`]).
pub fn place_bytes(app: &mut VectorcraftApp, name: &str, bytes: &[u8]) -> Result<(), String> {
    crate::place::run_for_user(app, &serde_json::json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) })).map(|_| ())
}
