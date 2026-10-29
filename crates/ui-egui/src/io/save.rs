use super::export::file_url;
use super::open::{note_recent, open_options};
use super::{
    EngineError, FilePick, Format, SAVE_FORMATS, SaveMode, SavePlan, SavedView, Services, TEMPLATE_FORMATS, Value, VectorcraftApp, Writer,
    background, dialogs, fileio, json, svg_options,
};

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
pub(super) fn suggested(app: &VectorcraftApp, ext: &str) -> (String, Option<String>) {
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
pub(super) fn target_path_keeping(
    app: &mut VectorcraftApp,
    path: Option<String>,
    ext: &str,
    keeps: impl Fn(&Format) -> bool,
) -> Result<String, String> {
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
pub(super) fn write_encoded(write: Writer, doc: &vectorcraft_doc::Document, path: &str, enc: &fileio::Encoded) -> Result<Vec<String>, String> {
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
pub(super) fn ask_before_losing(app: &mut VectorcraftApp, path: &str, command: &str, params: &Value) -> Option<Value> {
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
pub(super) fn view_file(app: &mut VectorcraftApp, path: &str) {
    if !is_web(app) {
        app.open_url(&file_url(path));
    }
}

/// "Saved <path>", with the first of the result's warnings.
pub(crate) fn report_saved(app: &mut VectorcraftApp, path: &str, r: &Value) {
    app.status(saved_status(path, r));
}

/// [`report_saved`]'s message.
pub(super) fn saved_status(path: &str, r: &Value) -> String {
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
