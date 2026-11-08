use vectorcraft_plugins::permissions;

use super::save::suggested;
use super::{Dialog, FilePick, Format, Value, VectorcraftApp, dialogs, fileio, json};

/// Template extensions New from Template's open dialog lists first.
const TEMPLATE_EXTS: &[&str] = &["vctemplate", "ait", "vectorcraft", "drawcraft"];

/// Open bytes of any readable format as a new document (templates open untitled); swatch and
/// graphic style library files open in the library panel, Libraries panel files are added to it,
/// flattener, PDF, print, perspective grid and export presets files are imported, a shortcut set another
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
        import_presets(app, import, what, serde_json::json!({"dataBase64": vectorcraft_format::base64_encode(bytes)}))?;
        return Ok(Value::Null);
    }
    if opens == Some(OpensAs::Library) {
        return crate::panels::libraries::import(app, json!({"dataBase64": vectorcraft_format::base64_encode(bytes)})).map(|_| Value::Null);
    }
    let swatches = opens == Some(OpensAs::SwatchLibrary);
    if swatches || opens == Some(OpensAs::StyleLibrary) {
        let p = match path {
            Some(path) => serde_json::json!({ "path": path }),
            // The file's bytes as they are (`.ase` swatch libraries are binary).
            None => serde_json::json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes)}),
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
    /// Flattener, PDF, print, perspective grid or export presets: imported with command `import`, and
    /// `what` names them in the status bar.
    Presets { import: &'static str, what: &'static str },
    /// A Libraries panel file: added to the Libraries panel as a new library.
    Library,
    /// A swatch library: opened in the library panel.
    SwatchLibrary,
    /// A graphic style library: opened in the library panel.
    StyleLibrary,
}

/// What the status bar calls the presets of an export presets file.
pub(crate) const EXPORT_PRESETS: &str = "export presets";

/// Run presets command `import` with `p` (the file) and say in the status bar which presets
/// (`what`) it added → its result.
pub(crate) fn import_presets(app: &mut VectorcraftApp, import: &str, what: &str, p: Value) -> Result<Value, String> {
    let r = app.run(import, p)?;
    let names: Vec<&str> = r["imported"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    app.status(format!("Imported {what}: {}", names.join(", ")));
    Ok(r)
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
        (fileio::SCREEN_EXPORT_PRESET_EXTS, "document.exportPresets.import", EXPORT_PRESETS),
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
        app.status(opened_with_notes(w.len(), w[0].as_str().unwrap_or_default()));
    }
    crate::dialogs::missing_links::after_open(app, r);
}

/// The status bar's line for a document opened with `n` notes, the first of them `first`.
pub(crate) fn opened_with_notes(n: usize, first: &str) -> String {
    crate::i18n::msg_n(n as u64, "Opened with {n} note: {first}", "Opened with {n} notes: {first}", &[("first", first)])
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

/// What permission `name` lets a plug-in do, in plain words in the UI language; a name the app
/// doesn't know is shown as it is.
fn permission_words(name: &str) -> String {
    let words = match permissions::Permission::parse(name) {
        Some(permissions::Permission::DocRead) => tl!("Read the objects it is given"),
        Some(permissions::Permission::DocWrite) => tl!("Change, add and delete objects in the document"),
        Some(permissions::Permission::State) => tl!("Keep data of its own with the document"),
        Some(permissions::Permission::EffectRender) => tl!("Change how the objects its effect is applied to are drawn"),
        Some(permissions::Permission::ExportWrite) => tl!("Write the document as a file in its own format"),
        Some(permissions::Permission::ImportRead) => tl!("Read files you open in its format and make documents from them"),
        Some(permissions::Permission::Progress) => tl!("Show its progress, see Cancel and write log lines"),
        None => return permissions::display_text(name, permissions::MAX_SHOWN_FIELD),
    };
    words.to_string()
}

/// Installs the plug-in module that `module` names (`plugin.install`'s `path`, or `dataBase64`
/// with `name`) → `plugin.install`'s result. A plug-in that lists permissions the user hasn't
/// granted it yet (`grant`) asks them first, showing its name, id, version, file hash and what each
/// permission lets it do, and so does a filter that asks for a heavy budget the user hasn't allowed
/// yet (`allowHeavy`); OK installs it if the module still has the hash the prompt showed, Cancel
/// installs nothing → `{pending}` while it asks. Every
/// text from the manifest is shown with its hidden characters escaped and its length capped
/// ([`permissions::display_text`]).
pub fn install_module(app: &mut VectorcraftApp, module: Value) -> Result<Value, String> {
    let heavy_allowed = module.get("allowHeavy").and_then(Value::as_bool) == Some(true);
    let granted = module.get("grant").is_some_and(|g| !g.is_null());
    if !heavy_allowed || !granted {
        let given = |k: &str| module.get(k).map(|v| (k.to_string(), v.clone()));
        let source = Value::Object(["path", "dataBase64", "name"].into_iter().filter_map(given).collect());
        // What OK installs: the same module, with what the user has agreed to so far, pinned to
        // the bytes the prompt shows (`hash`).
        let mut answered =
            Value::Object(["path", "dataBase64", "name", "replace", "grant", "allowHeavy", "hash"].into_iter().filter_map(given).collect());
        let info = app.run("plugin.inspect", source)?;
        if answered.get("hash").is_none() {
            answered["hash"] = info["hash"].clone();
        }
        let shown = |k: &str, max: usize| permissions::display_text(info[k].as_str().unwrap_or_default(), max);
        let name = shown("name", permissions::MAX_SHOWN_NAME);
        if !granted && info["needsApproval"] == true {
            let message = crate::i18n::fmt(tl!("Install “{name}”, which asks for these permissions?"), &[("name", &name)]);
            let mut lines = vec![crate::i18n::fmt(tl!("ID: {id}"), &[("id", &shown("id", permissions::MAX_SHOWN_FIELD))])];
            let version = shown("version", permissions::MAX_SHOWN_FIELD);
            if !version.is_empty() {
                lines.push(crate::i18n::fmt(tl!("Version: {version}"), &[("version", &version)]));
            }
            lines.push(crate::i18n::fmt(tl!("File SHA-256: {hash}"), &[("hash", &shown("hash", 64))]));
            lines.push(String::new());
            let asked = info["permissions"].as_array().cloned().unwrap_or_default();
            lines.extend(asked.iter().filter_map(Value::as_str).map(|p| format!("• {}", permission_words(p))));
            let mut again = answered;
            again["grant"] = Value::Array(asked);
            dialogs::confirm::ask(app, &message, &lines.join("\n"), "ui.installPlugin", again);
            return Ok(json!({ "pending": dialogs::confirm::KIND }));
        }
        if !heavy_allowed && info["budget"] == "heavy" {
            let (minutes, times) = vectorcraft_plugins::heavy_terms();
            let message = crate::i18n::fmt(tl!("Install “{name}”, which asks for a heavy budget?"), &[("name", &name)]);
            let detail = crate::i18n::fmt(
                tl!(
                    "Each run may take up to {minutes} minutes and do {times} times the work other plug-ins may. It runs in the background, and you can cancel it."
                ),
                &[("minutes", &minutes.to_string()), ("times", &times.to_string())],
            );
            let mut allowed = answered;
            allowed["allowHeavy"] = json!(true);
            dialogs::confirm::ask(app, &message, &detail, "ui.installPlugin", allowed);
            return Ok(json!({ "pending": dialogs::confirm::KIND }));
        }
    }
    let r = app.run("plugin.install", module)?;
    app.status(format!("Installed plug-in {}", permissions::display_text(r["name"].as_str().unwrap_or_default(), permissions::MAX_SHOWN_NAME)));
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

/// File › Open With: opens the file `p` names (`path`), else one picked among the files with the
/// extensions the importer plug-in `importer` lists, with that importer, also when a built-in
/// format reads the extension (`document.open {importer, params}`), and adds it to Open Recent; a
/// failure is reported as File › Open reports it. An importer with parameters, given no `params`,
/// asks for them first in dialog `pluginImport`, at their defaults. The web's file picker can't
/// carry the importer: there File › Open's picker opens, and the file opens as File › Open opens
/// it.
pub fn open_with(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let importer = p.get("importer").and_then(Value::as_str).ok_or("missing `importer` (see document.formats)")?;
    let plugin = vectorcraft_plugins::import::named(importer).map_err(|e| e.to_string())?;
    let path = match p.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        Some(path) => path.to_string(),
        None if app.services.open_async.is_some() => return open_dialog(app).map(|_| Value::Null),
        None => {
            // The importer's format: its name and extensions, made once for each installation.
            let filters = fileio::format(&vectorcraft_plugins::export::format_id(plugin.id()))
                .filter(|f| f.read)
                .map(|f| FilePick::filter(f.label, f.extensions))
                .into_iter()
                .collect();
            pick_open(app, &FilePick { filters, folder: open_dialog_folder(app), ..Default::default() })?
        }
    };
    let importer = vectorcraft_plugins::export::format_id(plugin.id());
    let given = p.get("params").filter(|v| !v.is_null());
    let opened = vectorcraft_engine::file_access::check_read(&path).and_then(|()| check_import_size(&path, true)).and_then(|()| {
        if given.is_none() && !plugin.manifest().params.is_empty() {
            dialogs::plugin::open_import(app, &plugin, json!({ "path": path, "importer": importer }));
            return Ok(Value::Null);
        }
        let mut o = json!({ "importer": importer });
        if let Some(params) = given {
            o["params"] = params.clone();
        }
        let bytes = read(app, &path)?;
        let r = open_document(app, &path, &bytes, Some(path.clone()), &o)?;
        note_recent(app, &path);
        Ok(r)
    });
    if let Err(e) = &opened {
        report_open_error(app, &fileio::file_name(&path), e);
    }
    opened
}

/// A file at `path` that only an importer plug-in reads (with `explicit`, the one named for it),
/// larger than the importer's input limit: an error before the file is read. A file whose start
/// shows a built-in format passes when no importer was named, since a built-in reader opens it.
#[cfg(not(target_arch = "wasm32"))]
fn check_import_size(path: &str, explicit: bool) -> Result<(), String> {
    use std::io::Read as _;
    use vectorcraft_plugins::import;
    let ext = fileio::extension(path);
    if !explicit && (fileio::readable_format(&ext).is_some() || import::for_extension(&ext).is_empty()) {
        return Ok(());
    }
    if std::fs::metadata(path).map_or(true, |m| m.len() <= import::MAX_INPUT_BYTES as u64) {
        return Ok(());
    }
    if !explicit {
        let mut head = Vec::new();
        if let Ok(f) = std::fs::File::open(path) {
            let _ = f.take(64 << 10).read_to_end(&mut head);
        }
        if fileio::detect(path, &head).is_some() {
            return Ok(());
        }
    }
    Err(format!("can't open `{}`: {}", fileio::file_name(path), import::too_large()))
}

#[cfg(target_arch = "wasm32")]
fn check_import_size(_: &str, _: bool) -> Result<(), String> {
    Ok(())
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

/// A file already open in a tab is not opened again: its tab comes to the front, unsaved edits
/// and all (#3455), without the questions asked after an open (missing links and fonts).
/// → `{activated: true, …}` as `document.open`'s result, or None.
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
    app.sync_views();
    // Home, shown over the document that was already active, gives way to it too.
    app.ui.home = None;
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
    // Automation confined to some folders reaches no file outside them, open in a tab or not.
    vectorcraft_engine::file_access::check_read(path)?;
    // File › Open, Open Recent, the Home screen, a drop and the Finder all focus a document that
    // is already open, or still being read, instead of opening a second copy. While a dialog or
    // a drag acts on the active document, its tab comes to the front once that ends
    // ([`crate::opening::poll`]), as a file that finishes opening then does.
    if document_index_for_path(app, path).is_some() && !crate::opening::may_switch(app) {
        if !app.opening.activate.iter().any(|(p, _)| same_open_path(p, path)) {
            let scope = vectorcraft_engine::file_access::ScopeHandle::capture();
            app.opening.activate.push((path.to_string(), scope));
        }
        return Ok(Value::Null);
    }
    if let Some(r) = try_activate_open_document(app, path)? {
        return Ok(r);
    }
    if app.opening.pending.iter().any(|p| same_open_path(&p.path, path)) {
        return Ok(Value::Null);
    }
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
    // A file only an importer plug-in reads is not read past the importer's input limit.
    check_import_size(path, false)?;
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
