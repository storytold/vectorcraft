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
    ("Tracing presets", vectorcraft_engine::cmd::TRACE_PRESET_EXTS),
    ("Action sets", &[crate::panels::actions::ACTIONS_EXT]),
    ("Export presets", fileio::SCREEN_EXPORT_PRESET_EXTS),
    ("Composite fonts", vectorcraft_engine::cmd::compositefonts::EXTS),
    ("CSS", &["css"]),
    ("PostScript", &["ps"]),
    ("Preferences", &["json"]),
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

/// Keep the active document's saved view current before a save (native files reopen at it), and
/// each document's active artboard ([`remember_active_artboard`]).
pub fn remember_view(app: &mut VectorcraftApp) {
    remember_active_artboard(app);
    let Some(v) = app.view().copied().filter(|v| v.fitted) else { return };
    if let Some(st) = app.session.active_mut() {
        st.view = Some(SavedView { name: String::new(), center: v.center, zoom: v.zoom, rotation: v.rotation });
    }
}

/// Keep every open document's active artboard current (a `.ai` file saved or exported names it as
/// its active one, also in Export All Open Documents): its own view's, an index past the last
/// artboard being the last one as in the Artboards panel; `None` without a view.
pub fn remember_active_artboard(app: &mut VectorcraftApp) {
    let uids: Vec<u64> = app.session.documents().iter().map(|d| d.uid).collect();
    for uid in uids {
        let index = app.view_uids.iter().position(|u| *u == uid).and_then(|k| app.views.get(k)).map(|v| v.artboard);
        if let Some(st) = app.session.document_mut(uid) {
            let last = st.doc.artboards.len().saturating_sub(1);
            st.active_artboard = index.and_then(|i| st.doc.artboards.get(i.min(last))).map(|a| a.id);
        }
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

/// Ordinary File > Save alone offers another destination when its own location refuses writes.
/// Save Changes during Close/Quit continues to use [`save`].
pub(crate) fn save_current(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    save_current_in(app, p, fileio::documents_dialog_folder())
}

/// The same entry with a resolved Documents folder (also permits deterministic host-free tests).
pub(super) fn save_current_in(app: &mut VectorcraftApp, p: &Value, folder: Option<String>) -> Result<Value, String> {
    remember_view(app);
    let first = plan(app, SaveMode::Save, &acknowledged(p))?;
    let own = !app.automation_asked()
        && !is_web(app)
        && p.get("path").is_none_or(Value::is_null)
        && first.path.is_some()
        && app.session.active().is_some_and(|st| !st.converted && !st.recovered && st.path == first.path && st.format == first.format.id);
    if !own {
        return save(app, SaveMode::Save, p, true);
    }
    if let Some(r) = first.path.as_deref().and_then(|path| ask_before_losing(app, path, "file.save", p)) {
        return Ok(r);
    }
    let retry = super::read_only_save::Retry::new(&first, folder);
    write_plan(app, first, ("file.save", p.clone()), Some(retry))
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
    save_suggesting(app, mode, p, ask_options, None)
}

/// Reuse Save As with a captured name and folder, without changing the document before writing.
pub(super) fn save_suggesting(
    app: &mut VectorcraftApp,
    mode: SaveMode,
    p: &Value,
    ask_options: bool,
    suggestion: Option<&FilePick>,
) -> Result<Value, String> {
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
        // Only `path` was asked about: the files written beside it are checked against the file
        // the document was read from when the save writes them.
        return write_plan(app, plan(app, mode, p)?, (save_command(mode), with_path(p, &path)), None);
    }
    if is_web(app) && ask_options {
        return Ok(open_options(app, mode.command(), first.format, &first.name, true));
    }
    let filters = FilePick::filters(match mode {
        SaveMode::Template => fileio::template_filters(first.format.id),
        _ => fileio::save_filters(first.format.id),
    });
    let pick = FilePick {
        name: suggestion.map_or_else(|| first.name.clone(), |s| s.name.clone()),
        folder: suggestion.map_or_else(|| first.folder.clone(), |s| s.folder.clone()),
        filters,
    };
    let picked = pick_finished(app, &pick, |picked, file_type| with_picked_type(picked, &pick.name, file_type, first.format, mode))?;
    // Only the read-only retry carries options for the original format through Save As.
    // A different format must use its own normal options flow instead.
    let changed_retry_format = suggestion.is_some() && fileio::format_for_name(&picked).is_some_and(|f| f.id != first.format.id);
    // The picked file type wins over a `format` param.
    let mut q = if p.is_object() { p.clone() } else { json!({}) };
    if let Some(o) = q.as_object_mut() {
        o.remove("format");
        if changed_retry_format {
            o.remove("options");
        }
        o.insert("path".into(), json!(picked));
    }
    let chosen = plan(app, mode, &acknowledged(&q))?;
    if let Some(r) = chosen.path.as_deref().and_then(|path| ask_before_losing(app, path, save_command(mode), &q)) {
        return Ok(r);
    }
    if (ask || changed_retry_format) && asks_options(mode, chosen.format) {
        let path = chosen.path.clone().unwrap_or_default();
        let r = ask_format_options(app, mode, chosen.format, &path);
        crate::dialogs::opened_by_panel(app);
        return r;
    }
    let chosen = plan(app, mode, &q)?;
    write_plan(app, chosen, (save_command(mode), q), None)
}

/// `p` (an object, else none) with `path`.
pub(super) fn with_path(p: &Value, path: &str) -> Value {
    let mut p = if p.is_object() { p.clone() } else { json!({}) };
    p["path"] = json!(path);
    p
}

/// The files among `paths` that a write to `path` replaces although the save panel asked only
/// about `path`: every other one that `exists` finds, but those in `known`. None without `exists`.
pub(super) fn replaced_files<'a>(
    exists: Option<&crate::ExistsFn>,
    path: &str,
    known: &[String],
    paths: impl IntoIterator<Item = &'a str>,
) -> Vec<String> {
    let Some(exists) = exists else { return Vec::new() };
    paths.into_iter().filter(|p| *p != path && !known.iter().any(|k| k == p) && exists(p)).map(str::to_string).collect()
}

/// The `exists` service with which a save or export checks the files it writes beside the one
/// picked ([`replaced_files`]), none on the web (downloads), and the files it replaces without
/// asking: those the OK of the question about them allowed while it runs that again.
pub(super) fn checks_replaced(app: &VectorcraftApp) -> (Option<crate::ExistsFn>, Vec<String>) {
    (app.services.exists.clone().filter(|_| !is_web(app)), app.picks.replace_files_ok.clone())
}

/// Ask before a save or export of the document `doc` (its uid) replaces `files`, files that are
/// there beside the one picked in the save panel (a file per artboard, a linked image), which the
/// panel did not ask about. OK runs `command` with `params` again, replacing them and `allowed`
/// (those an earlier OK allowed); Cancel writes nothing → `{pending}`.
pub(super) fn ask_to_replace_files(app: &mut VectorcraftApp, doc: u64, files: &[String], allowed: &[String], command: &str, params: &Value) -> Value {
    dialogs::confirm::ask_replace_files(app, doc, files, allowed, command, params.clone());
    json!({ "pending": dialogs::confirm::REPLACE_FILES })
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
    // The viewer is another app: an export automation asked for is written but not opened, also
    // when it goes on from a save panel automation opened.
    if !is_web(app) && !app.automation_asked() {
        app.open_url(&file_url(path));
    }
}

/// "Saved <path>", with the first of the result's warnings and how many there are; hovering the
/// message lists up to 30 of them, fewer when the window is small, with how many were left out
/// ([`show_with_notes`]).
pub(crate) fn report_saved(app: &mut VectorcraftApp, path: &str, r: &Value) {
    show_with_notes(app, saved_status(path, r), notes_of(r));
}

/// The warnings of a reply `r`, in the reply's order (a save's notes start with what the format
/// loses).
pub(crate) fn notes_of(r: &Value) -> Vec<String> {
    r["warnings"].as_array().into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect()
}

/// Show `status`, which names the first of `notes`, in the status bar and keep `notes` for its
/// hover ([`crate::state::UiState::status_notes`]). Without notes nothing is kept.
pub(crate) fn show_with_notes(app: &mut VectorcraftApp, status: String, notes: Vec<String>) {
    app.status(status);
    app.ui.status_notes = (!notes.is_empty()).then(|| (app.ui.status.clone(), notes));
}

/// [`report_saved`]'s message.
pub(crate) fn saved_status(path: &str, r: &Value) -> String {
    let warnings = notes_of(r);
    match warnings.first() {
        Some(first) => crate::i18n::msg_n(
            warnings.len() as u64,
            "Saved {path} with {n} note: {first}",
            "Saved {path} with {n} notes: {first}",
            &[("path", path), ("first", first.as_str())],
        ),
        None => format!("Saved {path}"),
    }
}

/// Write a planned save through the services and report it in the status bar.
/// With Background Save on, the file is encoded and written from a snapshot off the UI thread
/// ([`background`]): this returns `{path, background: true, status, format, warnings}` at once
/// (`warnings`: what the format loses, then the save's notes on its params, such as an option the format
/// doesn't read), and the document takes the file (and counts as saved, as of the snapshot) once it is written.
/// A file it writes beside the one at its path that is there already, and that the document's last
/// save did not write, is asked about before anything is written; `again` (a command and its params)
/// runs the save again once the question's OK allows it ([`ask_to_replace_files`]).
fn write_plan(
    app: &mut VectorcraftApp,
    plan: SavePlan,
    again: (&'static str, Value),
    retry: Option<super::read_only_save::Retry>,
) -> Result<Value, String> {
    let uid = app.session.active().ok_or("no document")?.uid;
    // Saves of one document finish in order.
    if app.background.busy_with(uid) {
        background::wait_all(app);
    }
    let refused = plan.path.clone().unwrap_or_else(|| plan.name.clone());
    // Refused before writing (#4933): told as a failed write is, in the reason's own words.
    let job = fileio::save_job(&mut app.session, plan).map_err(|e| {
        let reason = match &e {
            EngineError::BadParams { msg, .. } => msg.clone(),
            e => e.to_string(),
        };
        report_write_error(app, false, &refused, &reason);
        e.to_string()
    })?;
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
    // Capture the caller before background work leaves the request: it must never open a panel
    // or error dialog later, when answering_control has been reset.
    let by_control = app.automation_asked();
    let location_refused = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let refused_by_work = location_refused.clone();
    let own_path = target.clone();
    let save_write = app.services.save_write.clone().filter(|_| !is_web(app));
    let save_stream = app.services.save_stream.clone().filter(|_| job.streams() && !is_web(app));
    // A native file streams into the file as it is encoded (desktop; not to a download).
    let stream = app.services.stream_shared.clone().filter(|_| job.streams() && !is_web(app));
    let (exists, allowed) = checks_replaced(app);
    let known: Vec<String> = allowed.iter().chain(app.picks.written_beside.get(&uid).into_iter().flatten()).cloned().collect();
    let work = move |write: Writer| {
        let mut own_refused = false;
        let mut observed = |p: &str, r: vectorcraft_engine::Result<()>| {
            own_refused = own_path.as_deref() == Some(p) && matches!(&r, Err(EngineError::ReadOnlyLocation(_)));
            r
        };
        let check = |paths: &[&str]| {
            let path = job.plan().path.as_deref().unwrap_or_default();
            let replaced = replaced_files(exists.as_ref(), path, &known, paths.iter().copied());
            (!replaced.is_empty()).then(|| json!({ "replaces": replaced }))
        };
        let r = match (save_stream, save_write, stream) {
            (Some(stream), _, _) => job.write_streamed(&mut |p, fill| observed(p, stream(p, fill))),
            (_, Some(write), _) => job.write_unless(check, |p, b| observed(p, write(p, b))),
            (_, _, Some(stream)) => job.write_streamed(&mut |p, fill| stream(p, fill).map_err(EngineError::Other)),
            _ => job.write_unless(check, |p, b| write(p, b).map_err(EngineError::Other)),
        };
        r.map_err(|e| {
            if let EngineError::ReadOnlyLocation(reason) = &e {
                refused_by_work.store(own_refused, std::sync::atomic::Ordering::Relaxed);
                if !by_control {
                    return reason.clone();
                }
            }
            e.to_string()
        })
    };
    let failed = target.clone().unwrap_or_default();
    let then = move |app: &mut VectorcraftApp, r: Result<Value, String>| {
        if r.is_err()
            && location_refused.load(std::sync::atomic::Ordering::Relaxed)
            && let Some(retry) = retry
            && let Some(r) = retry.instead(app, uid)
        {
            return r;
        }
        // Not written: the document keeps its file, format and modified state, and the user is told.
        let r = r.inspect_err(|e| {
            if !by_control {
                report_write_error(app, false, &failed, e);
            }
        })?;
        // Nothing written either: asked first.
        if let Some(files) = r.get("replaces").and_then(Value::as_array) {
            let files: Vec<String> = files.iter().filter_map(Value::as_str).map(str::to_string).collect();
            return Ok(ask_to_replace_files(app, uid, &files, &allowed, again.0, &again.1));
        }
        // The document may have closed meanwhile: the file is written all the same.
        done.complete(&mut app.session, uid);
        let path = r["path"].as_str().unwrap_or_default().to_string();
        if retargets {
            note_recent(app, &path);
            // The files written beside the document's file, which its next save replaces.
            let beside = ["files", "linked"].iter().flat_map(|k| r[*k].as_array().into_iter().flatten()).filter_map(Value::as_str);
            let beside: Vec<String> = beside.filter(|p| *p != path).map(str::to_string).collect();
            app.picks.written_beside.insert(uid, beside);
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
