use std::collections::HashMap;
use std::path::PathBuf;
// The save panel's flow is the desktop's (craft-dialogs is native only); the web downloads.
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

#[cfg(not(target_arch = "wasm32"))]
use craft::dialogs::save::{SAVE_IN_PROGRESS, SaveDocument, SaveFlow, SaveStep, SaveTarget};
#[cfg(not(target_arch = "wasm32"))]
use craft::host::Files;
use craft::host::{FileFilter, PickRequest, Picked, Wake};
use craft::platform::watch::FileWatch;
use craft::ui::dialog::DialogId;
#[cfg(not(target_arch = "wasm32"))]
use craft::ui::dialog::{DialogResult, PromptChoice, StandardPrompt};
use vectorcraft_engine::file_access::ScopeHandle;

use super::export::file_url;
use super::open::note_recent;
use super::{
    EngineError, Format, Progress, SAVE_FORMATS, SaveMode, SavePlan, SavedView, Services, TEMPLATE_FORMATS, Value, VectorcraftApp, Writer,
    background, dialogs, fileio, json, progress, svg_options,
};
use crate::file_asks::{self, Owner};

/// What runs once a save is written, failed or cancelled.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) type Done = Box<dyn FnOnce(&mut VectorcraftApp, Result<Value, String>)>;

/// What [`pump`] needs to go on with a document's save once its flow answers.
#[cfg(not(target_arch = "wasm32"))]
struct SaveDone {
    mode: SaveMode,
    params: Value,
    ask_options: bool,
    /// The name the picker suggested: a typed extension is compared with it.
    suggested: String,
    /// The picker's filters: the index of the picked type names one of them.
    filters: Vec<FileFilter>,
    /// The read-only fallback's Save As: options of the original format carry through it.
    retrying: bool,
    retry: Option<super::read_only_save::Retry>,
    /// The changed-on-disk question, while it is open.
    question: Option<DialogId>,
    /// The file-access scope and automation flag of the request that started the save: the steps
    /// [`pump`] runs once the flow answers run under them, not under the desktop's access of that frame.
    asked: (ScopeHandle, bool),
    then: Option<Done>,
}

/// The saves of the app: one [`SaveFlow`] per document, and what the flows need to ask and write.
pub(crate) struct Saves {
    #[cfg(not(target_arch = "wasm32"))]
    flows: HashMap<u64, SaveFlow<u64>>,
    /// The version of each document's file the app last read or wrote.
    pub(crate) watch: FileWatch<u64>,
    /// The files written beside each document's file, which its next save replaces.
    pub(crate) written_beside: HashMap<u64, Vec<String>>,
    /// The files the OK of the replace question allows the save it runs again to replace.
    pub(crate) replace_files_ok: Vec<String>,
    #[cfg(not(target_arch = "wasm32"))]
    done: HashMap<u64, SaveDone>,
}

impl Saves {
    /// No save in progress; `wake` runs a frame when the watcher has news.
    pub(crate) fn new(wake: Wake) -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            flows: HashMap::new(),
            watch: FileWatch::new(wake),
            written_beside: HashMap::new(),
            replace_files_ok: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            done: HashMap::new(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start(&mut self, files: &dyn Files, document: SaveDocument<u64>, target: SaveTarget) -> SaveStep<u64> {
        let Self { flows, watch, .. } = self;
        flows.entry(document.key).or_default().start(files, watch, document, target)
    }

    /// Frees the document's flow after a write that wrote nothing.
    #[cfg(not(target_arch = "wasm32"))]
    pub(super) fn release(&mut self, uid: u64) {
        if let Some(flow) = self.flows.get_mut(&uid) {
            flow.failed(&uid);
        }
    }
}

/// Calls the `then` of the document's save, once.
#[cfg(not(target_arch = "wasm32"))]
fn terminate(app: &mut VectorcraftApp, uid: u64, r: Result<Value, String>) {
    if let Some(then) = app.saves.done.remove(&uid).and_then(|d| d.then) {
        then(app, r);
    }
}

/// A save's end: the flow is free again (and the watcher knows the new version of a file the
/// document took), and the caller that waits for it is told, unless a question or a fallback
/// picker goes on with it.
#[cfg(not(target_arch = "wasm32"))]
fn finish(app: &mut VectorcraftApp, uid: u64, retargets: bool, out: &Result<Value, String>) {
    let written = out.as_ref().ok().filter(|v| v.get("cancelled").is_none() && v.get("pending").is_none());
    let flow = app.saves.flows.entry(uid).or_default();
    match written.and_then(|v| v["path"].as_str()) {
        Some(path) if flow.is_busy() || retargets => flow.written(&mut app.saves.watch, uid, Path::new(path)),
        _ => flow.failed(&uid),
    }
    if !matches!(out, Ok(v) if v.get("pending").is_some()) {
        terminate(app, uid, out.clone());
    }
}

/// Lets the save that runs again replace `files`, which the OK of the replace question allowed.
pub(crate) fn allow_replacing(app: &mut VectorcraftApp, files: &[String]) {
    for f in files {
        if !app.saves.replace_files_ok.contains(f) {
            app.saves.replace_files_ok.push(f.clone());
        }
    }
}

/// Takes what the flows and their questions have answered since the last pass: the pick, the
/// changed-on-disk question's answer. A save whose document closed is cancelled and forgotten.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let busy: Vec<u64> = app.saves.flows.iter().filter(|(_, f)| f.is_busy()).map(|(uid, _)| *uid).collect();
    for uid in busy {
        if !app.session.documents().iter().any(|d| d.uid == uid) {
            terminate(app, uid, Err(file_asks::closed().into()));
            app.status(file_asks::closed());
            if let Some(mut flow) = app.saves.flows.remove(&uid) {
                flow.cancel(app.host.files());
            }
            app.saves.watch.forget(&uid);
            continue;
        }
        let (scope, automated) = app.saves.done.get(&uid).map_or_else(|| (ScopeHandle::capture(), app.automation_asked()), |d| d.asked.clone());
        let r = scope.run(|| {
            file_asks::as_automated(app, automated, |app| {
                let step = next_step(app, uid)?;
                Some(run_step(app, uid, step))
            })
        });
        if let Some(Err(e)) = r
            && e != "cancelled"
        {
            app.status(e);
        }
    }
}

/// The web has no save flows: nothing answers between passes.
#[cfg(target_arch = "wasm32")]
pub(crate) fn pump(_: &mut VectorcraftApp, _: &egui::Context) {}

#[cfg(not(target_arch = "wasm32"))]
/// The step the document's flow has reached since the last pass: its question's answer, else its
/// pick.
fn next_step(app: &mut VectorcraftApp, uid: u64) -> Option<SaveStep<u64>> {
    if let Some(id) = app.saves.done.get(&uid).and_then(|d| d.question) {
        let answer = app.dialogs()?.with(|host| host.take_by_id::<StandardPrompt>(id))?;
        if let Some(done) = app.saves.done.get_mut(&uid) {
            done.question = None;
        }
        let flow = app.saves.flows.get_mut(&uid)?;
        return match answer {
            DialogResult::Ok(a) if a.choice == PromptChoice::Proceed => Some(flow.replace()),
            DialogResult::Ok(a) if a.choice == PromptChoice::Alternate => Some(flow.save_elsewhere(app.host.files())),
            _ => flow.cancel(app.host.files()),
        };
    }
    let Saves { flows, watch, .. } = &mut app.saves;
    flows.get_mut(&uid)?.take_pick(app.host.files(), watch)
}

#[cfg(not(target_arch = "wasm32"))]
/// Runs `step` of the document `uid`'s save; a failure frees the flow and tells the caller.
fn run_step(app: &mut VectorcraftApp, uid: u64, step: SaveStep<u64>) -> Result<Value, String> {
    let r = proceed(app, step);
    if r.is_err() {
        app.saves.release(uid);
        terminate(app, uid, r.clone());
    }
    r
}

#[cfg(not(target_arch = "wasm32"))]
fn proceed(app: &mut VectorcraftApp, step: SaveStep<u64>) -> Result<Value, String> {
    match step {
        SaveStep::Picking => Ok(json!({ "pending": "fileDialog" })),
        SaveStep::AskChangedOnDisk { key, path } => {
            let name = fileio::file_name(&path.to_string_lossy());
            let prompt = StandardPrompt::changed_on_disk(name).instance(("changed-on-disk", key));
            let id = app.dialogs().ok_or("no window to ask in")?.with(|host| host.open(prompt));
            if let Some(done) = app.saves.done.get_mut(&key) {
                done.question = Some(id);
            }
            Ok(json!({ "pending": "changedOnDisk" }))
        }
        SaveStep::Write { key, path, target, file_type } => write_step(app, key, &path.to_string_lossy(), target, file_type),
        SaveStep::Cancelled { key } => {
            terminate(app, key, Err("cancelled".into()));
            Err("cancelled".into())
        }
        SaveStep::Busy => Err(SAVE_IN_PROGRESS.english.into()),
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// The flow's `Write` step for document `uid`: its own file written as it is, or a picked path
/// made into the file the picked type names, asking before it replaces a file the panel did not
/// ask about.
fn write_step(app: &mut VectorcraftApp, uid: u64, path: &str, target: SaveTarget, file_type: Option<usize>) -> Result<Value, String> {
    let index = app.session.documents().iter().position(|d| d.uid == uid).ok_or(file_asks::closed())?;
    if app.session.active_index() != Some(index) {
        app.run("document.activate", json!({ "index": index }))?;
    }
    let (mode, params, ask, suggested, retrying, retry, picked_ext) = {
        let done = app.saves.done.get_mut(&uid).ok_or(file_asks::closed())?;
        // `None` without a picked type: the typed or suggested extension decides (macOS has no type menu).
        let picked_ext = file_type.and_then(|i| done.filters.get(i)).and_then(|f| f.extensions.first().cloned());
        (done.mode, done.params.clone(), done.ask_options, done.suggested.clone(), done.retrying, done.retry.take(), picked_ext)
    };
    let own = target == SaveTarget::InPlace && app.session.active().is_some_and(|st| st.path.as_deref() == Some(path));
    if own {
        return plan(app, mode, &params).and_then(|plan| write_plan(app, plan, (save_command(mode), params), retry));
    }
    let first = plan(app, mode, &acknowledged(&params))?;
    let answered = path.to_string();
    let path = with_picked_type(&answered, &suggested, picked_ext.as_deref(), first.format, mode);
    // The flow ends here: the questions and options dialogs that follow run on the path alone.
    app.saves.release(uid);
    replaces_unasked(app, &answered, &path, move |app, path| finish_picked(app, mode, &params, ask, &path, retrying))
}

#[cfg(not(target_arch = "wasm32"))]
/// What [`start_flow`] needs besides the plan.
struct Start<'a> {
    mode: SaveMode,
    params: &'a Value,
    ask_options: bool,
    target: SaveTarget,
    retry: Option<super::read_only_save::Retry>,
    name: String,
    directory: Option<PathBuf>,
    retrying: bool,
}

#[cfg(not(target_arch = "wasm32"))]
/// Starts the active document's save on its flow: the picker, or the changed-on-disk check of
/// its own file, then the write.
fn start_flow(app: &mut VectorcraftApp, first: &SavePlan, s: Start<'_>) -> Result<Value, String> {
    let st = app.session.active().ok_or("no document")?;
    let uid = st.uid;
    let template = s.mode == SaveMode::Template;
    let id = first.format.id;
    let document = SaveDocument {
        key: uid,
        name: s.name.clone(),
        path: st.path.clone().map(PathBuf::from),
        extension: first.format.extensions.first().map_or_else(String::new, |e| (*e).to_string()),
        filters: file_asks::filters(&if template { fileio::template_filters(id) } else { fileio::save_filters(id) }),
        purpose: if template { "template" } else { "document" }.into(),
        saved: None,
        directory: s.directory,
    };
    let filters = document.filters.clone();
    let mut step = app.saves.start(app.host.files(), document.clone(), s.target);
    if matches!(step, SaveStep::Busy) && app.background.busy_with(uid) {
        background::wait_all(app);
        step = app.saves.start(app.host.files(), document, s.target);
    }
    if matches!(step, SaveStep::Busy) {
        return Err(SAVE_IN_PROGRESS.english.into());
    }
    let then = app.saves.done.remove(&uid).and_then(|d| d.then);
    let done = SaveDone {
        mode: s.mode,
        params: s.params.clone(),
        ask_options: s.ask_options,
        suggested: s.name,
        filters,
        retrying: s.retrying,
        retry: s.retry,
        question: None,
        asked: (ScopeHandle::capture(), app.automation_asked()),
        then,
    };
    app.saves.done.insert(uid, done);
    run_step(app, uid, step)
}

/// The status once a save running in the background was canceled.
pub const SAVE_CANCELED: &str = "Save canceled";

/// File › Cancel Save (the progress window's Cancel): every save running in the background stops
/// before it replaces its file. The file stays as it was, and the document keeps its file, format
/// and modified state → `{cancelled: n}`. A save whose file was already written is reported as saved.
pub fn cancel_saves(app: &mut VectorcraftApp) -> Value {
    let n = app.background.cancel_saves();
    if n > 0 {
        app.status(SAVE_CANCELED);
        app.background.save_cancel_line = Some(app.ui.status_sets);
    }
    json!({ "cancelled": n })
}

/// Does the status bar still show Cancel Save's line, or the line a save it reached set after it?
/// A line set since by another command or another job stays.
fn cancel_line_shown(app: &VectorcraftApp) -> bool {
    app.background.save_cancel_line == Some(app.ui.status_sets)
}

/// The progress window's Continue in Background for saves: they go on, shown in the status bar
/// only → `{hidden: n}`.
pub fn hide_saves(app: &mut VectorcraftApp) -> Value {
    json!({ "hidden": app.background.hide_saves() })
}

/// A save or an export that didn't write `path`: say so in a dialog that names the file and the
/// reason and stays until it is closed, as an editor does (#1685). The status bar's line, which
/// the caller's error becomes, is replaced by the next message, and a document that wasn't saved
/// must not look saved: it stays modified, and what was at `path` is as it was. Not for a request
/// an agent sent and waits on: the error is its reply.
pub(crate) fn report_write_error(app: &mut VectorcraftApp, export: bool, path: &str, e: &str) {
    report_write_error_over(app, None, export, path, e);
}

/// [`report_write_error`] over the open dialog `over` when there is one, at once: the dialog the write was asked from
/// shows again when the message closes.
fn report_write_error_over(app: &mut VectorcraftApp, over: Option<DialogId>, export: bool, path: &str, e: &str) {
    if app.ui.answering_control {
        return;
    }
    let name = fileio::file_name(path);
    let message = if export { tl!("Can't export “{name}”.") } else { tl!("Can't save “{name}”.") };
    let (message, detail) = (crate::i18n::fmt(message, &[("name", &name)]), crate::i18n::message(crate::i18n::current(), e));
    match over {
        Some(parent) => dialogs::confirm::tell_over(app, parent, &message, &detail),
        None => dialogs::confirm::tell(app, &message, &detail),
    }
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
/// document and shows its file type, [`file_types`] (the web downloads them as `name`); `then`
/// gets where they went. While the panel is open the reply is `{pending: "fileDialog"}`.
pub(crate) fn write_named(
    app: &mut VectorcraftApp,
    path: Option<String>,
    name: &str,
    bytes: Vec<u8>,
    then: impl FnOnce(&mut VectorcraftApp, String) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    write_named_as(app, path, name, file_types(&fileio::extension(name)), bytes, then)
}

/// [`write_named`] asked from the open dialog `parent`: a failed write is told over it.
pub(crate) fn write_named_over(
    app: &mut VectorcraftApp,
    parent: DialogId,
    path: Option<String>,
    name: &str,
    bytes: Vec<u8>,
    then: impl FnOnce(&mut VectorcraftApp, String) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    write_named_in(app, Some(parent), path, name, file_types(&fileio::extension(name)), bytes, then)
}

/// [`write_named`] with a save panel that shows `filters`, for a file whose extension names no one
/// type (`.json` holds shortcut sets as well as preferences).
pub(crate) fn write_named_as(
    app: &mut VectorcraftApp,
    path: Option<String>,
    name: &str,
    filters: Vec<FileFilter>,
    bytes: Vec<u8>,
    then: impl FnOnce(&mut VectorcraftApp, String) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    write_named_in(app, None, path, name, filters, bytes, then)
}

/// [`write_named_as`], a failed write told over the dialog `over` when there is one.
fn write_named_in(
    app: &mut VectorcraftApp,
    over: Option<DialogId>,
    path: Option<String>,
    name: &str,
    filters: Vec<FileFilter>,
    bytes: Vec<u8>,
    then: impl FnOnce(&mut VectorcraftApp, String) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    let ext = fileio::extension(name);
    let (_, folder) = suggested(app, "");
    ask_path(
        app,
        path,
        name.to_string(),
        folder,
        filters,
        ext,
        |_| false,
        move |app, path| {
            write_to(&mut app.services, &path, &bytes).inspect_err(|e| report_write_error_over(app, over, false, &path, e))?;
            then(app, path)
        },
    )
}

/// Where a file goes when no path was given: the suggested name on the web (a download), else the
/// save panel's choice with the suggested name's extension, [`with_written_extension`]: a name
/// typed with the extension of a format `keeps` takes stays as typed too. `then` gets the path:
/// at once for a given one, else once the panel is answered (the reply is then `{pending:
/// "fileDialog"}`; a cancelled panel runs nothing).
#[allow(clippy::too_many_arguments)]
fn ask_path(
    app: &mut VectorcraftApp,
    path: Option<String>,
    name: String,
    folder: Option<String>,
    filters: Vec<FileFilter>,
    ext: String,
    keeps: impl Fn(&Format) -> bool + 'static,
    then: impl FnOnce(&mut VectorcraftApp, String) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    if let Some(p) = path {
        return then(app, p);
    }
    if is_web(app) {
        return then(app, with_written_extension(&name, &ext, &keeps));
    }
    let mut request = PickRequest::save("export").file_name(name);
    request.filters = filters;
    if let Some(folder) = folder {
        request = request.directory(folder);
    }
    Ok(file_asks::ask(app, request, Owner::Document, move |app, picked| {
        let Picked::Saved { path: picked, .. } = picked else { return Err("cancelled".into()) };
        let picked = picked.to_string_lossy().into_owned();
        let path = with_written_extension(&picked, &ext, &keeps);
        replaces_unasked(app, &picked, &path, then)
    }))
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

/// `then` on `path`, the file the save panel's answer `picked` was made into. When that changed
/// the name (an extension added or replaced) and a file is there, which the panel did not ask
/// about, a question asks before replacing it and `then` runs on its OK (the reply is `{pending:
/// "replaceFile"}`).
fn replaces_unasked(
    app: &mut VectorcraftApp,
    picked: &str,
    path: &str,
    then: impl FnOnce(&mut VectorcraftApp, String) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    let there = path != picked && !is_web(app) && app.services.exists.as_ref().is_some_and(|exists| exists(path));
    if !there {
        return then(app, path.to_string());
    }
    let path = path.to_string();
    Ok(file_asks::ask_replace(app, &path.clone(), picked, move |app| then(app, path)))
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
pub(crate) const FILE_TYPES: &[(&str, &[&str])] = &[
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
fn file_types(ext: &str) -> Vec<FileFilter> {
    if let Some(f) = fileio::format(ext) {
        return file_asks::filters(&[(f.label, f.extensions)]);
    }
    let ext = ext.trim_start_matches('.').to_ascii_lowercase();
    let row = FILE_TYPES.iter().find(|(_, exts)| exts.contains(&ext.as_str())).copied();
    file_asks::filters(&row.into_iter().collect::<Vec<_>>())
}

/// `then` on `path`, else on one picked in a save panel (the web: the suggested name) for the
/// active document with extension `ext`, filtered to its file type when it has one
/// ([`file_types`]). A name typed with another format's extension gets `ext` instead
/// ([`with_written_extension`]). While the panel is open the reply is `{pending: "fileDialog"}`.
pub(crate) fn target_path(
    app: &mut VectorcraftApp,
    path: Option<String>,
    ext: &str,
    then: impl FnOnce(&mut VectorcraftApp, String) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    target_path_keeping(app, path, ext, |_| false, then)
}

/// [`target_path`] for an export in format `f` (Export As's OK and [`export`]): a name typed with
/// `.svgz` for an SVG stays as typed, and [`export`] writes it compressed.
pub(crate) fn export_path(
    app: &mut VectorcraftApp,
    path: Option<String>,
    f: &Format,
    then: impl FnOnce(&mut VectorcraftApp, String) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    let id = f.id;
    target_path_keeping(app, path, f.extensions.first().copied().unwrap_or_default(), move |g| id == "svg" && g.id == "svgz", then)
}

/// [`target_path`], keeping a typed extension of a format `keeps` takes.
pub(super) fn target_path_keeping(
    app: &mut VectorcraftApp,
    path: Option<String>,
    ext: &str,
    keeps: impl Fn(&Format) -> bool + 'static,
    then: impl FnOnce(&mut VectorcraftApp, String) -> Result<Value, String> + 'static,
) -> Result<Value, String> {
    let (name, folder) = suggested(app, ext);
    ask_path(app, path, name, folder, file_types(ext), ext.to_string(), keeps, then)
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
#[cfg(not(target_arch = "wasm32"))]
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
    // Only `path` was asked about: the files written beside it are checked against the file the
    // document was read from when the save writes them. The flow checks that another app did not
    // change the file since.
    let start = Start {
        mode: SaveMode::Save,
        params: p,
        ask_options: false,
        target: SaveTarget::InPlace,
        retry: Some(retry),
        name: first.name.clone(),
        directory: first.folder.clone().map(PathBuf::from),
        retrying: false,
    };
    start_flow(app, &first, start)
}

/// The web has no save panel to fall back on: Save downloads as [`save`] does.
#[cfg(target_arch = "wasm32")]
pub(super) fn save_current_in(app: &mut VectorcraftApp, p: &Value, _folder: Option<String>) -> Result<Value, String> {
    save(app, SaveMode::Save, p, true)
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

/// Reuse Save As with a captured name and folder (`suggestion`), without changing the document
/// before writing.
pub(super) fn save_suggesting(
    app: &mut VectorcraftApp,
    mode: SaveMode,
    p: &Value,
    ask_options: bool,
    suggestion: Option<(&str, Option<PathBuf>)>,
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
        return Ok(dialogs::save_options::open(app, mode.command(), first.format, &first.name, true));
    }
    let retrying = suggestion.is_some();
    let name = suggestion.as_ref().map_or_else(|| first.name.clone(), |(name, _)| (*name).to_string());
    // The desktop asks with the save panel; the web downloads under the suggested name.
    #[cfg(not(target_arch = "wasm32"))]
    if !is_web(app) {
        let directory = match suggestion {
            _ if mode == SaveMode::Template => fileio::templates_dialog_folder(&app.session.prefs).map(PathBuf::from),
            Some((_, directory)) => directory,
            None => first.folder.clone().map(PathBuf::from),
        };
        let target = match mode {
            SaveMode::Save => SaveTarget::InPlace,
            SaveMode::SaveAs => SaveTarget::As,
            SaveMode::Copy | SaveMode::Template => SaveTarget::Copy,
        };
        return start_flow(app, &first, Start { mode, params: p, ask_options, target, retry: None, name, directory, retrying });
    }
    let picked = with_picked_type(&name, &name, None, first.format, mode);
    finish_picked(app, mode, p, ask_options, &picked, retrying)
}

/// The rest of a save to the path the save panel's answer was made into: the options dialog of
/// the picked format, or the write.
fn finish_picked(app: &mut VectorcraftApp, mode: SaveMode, p: &Value, ask_options: bool, picked: &str, retrying: bool) -> Result<Value, String> {
    let first = plan(app, mode, &acknowledged(p))?;
    let ask = ask_options && !has_options(p);
    // Only the read-only retry carries options for the original format through Save As.
    // A different format must use its own normal options flow instead.
    let changed_retry_format = retrying && fileio::format_for_name(picked).is_some_and(|f| f.id != first.format.id);
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
        return ask_format_options(app, mode, chosen.format, &path);
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
/// asking: those the OK of the question about them allowed while it runs that again (taken: the
/// allowance is for that one save).
pub(super) fn checks_replaced(app: &mut VectorcraftApp) -> (Option<crate::ExistsFn>, Vec<String>) {
    (app.services.exists.clone().filter(|_| !is_web(app)), std::mem::take(&mut app.saves.replace_files_ok))
}

/// Ask before a save or export of the document `doc` (its uid) replaces `files`, files that are
/// there beside the one picked in the save panel (a file per artboard, a linked image), which the
/// panel did not ask about. OK runs `command` with `params` again, replacing them and `allowed`
/// (those an earlier OK allowed); Cancel writes nothing → `{pending}`. `asked` is the file-access
/// scope and automation flag of the request that started the save or export, captured before it
/// went to the background: OK runs `command` again under them.
pub(super) fn ask_to_replace_files(
    app: &mut VectorcraftApp,
    doc: u64,
    files: &[String],
    allowed: &[String],
    command: &str,
    params: &Value,
    asked: (ScopeHandle, bool),
) -> Value {
    let (scope, automated) = asked;
    scope.run(|| {
        file_asks::as_automated(app, automated, |app| dialogs::confirm::ask_replace_files(app, doc, files, allowed, command, params.clone()));
    });
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
    let st = app.session.active()?;
    let what = fileio::losses_summary(fileio::overwrite_losses(st, path, params)?);
    // The question's Cancel has no step of the flow to answer: the OK runs the command again.
    #[cfg(not(target_arch = "wasm32"))]
    app.saves.release(st.uid);
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
pub fn ask_format_options(app: &mut VectorcraftApp, mode: SaveMode, f: &'static Format, path: &str) -> Result<Value, String> {
    match f.id {
        "pdf" => ask_pdf_options(app, mode, path),
        "svg" | "svgz" => {
            let m = if mode == SaveMode::Copy { svg_options::Mode::SaveCopy } else { svg_options::Mode::Save };
            svg_options::open(app, m, Some(path));
            Ok(json!({ "pending": svg_options::KIND, "dialog": svg_options::KIND, "path": path }))
        }
        _ => Ok(dialogs::save_options::open(app, mode.command(), f, path, false)),
    }
}

/// A save as PDF asks with the Save PDF dialog, filled with the PDF options the document was last
/// saved with; its OK finishes the save ([`save_pdf`]).
pub fn ask_pdf_options(app: &mut VectorcraftApp, mode: SaveMode, path: &str) -> Result<Value, String> {
    let mut p = Value::Object(plan(app, mode, &json!({ "path": path, "format": "pdf" }))?.options);
    p["path"] = json!(path);
    dialogs::save_pdf::open_for_save(app, &p, mode)
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

/// Clear the status bar's message and its hover notes when `left` is still what the status bar
/// shows: a message with notes, and [`crate::state::UiState::status_sets`] when it was shown.
/// A message set since stays, one with the same text included.
pub(crate) fn clear_notes_left(app: &mut VectorcraftApp, left: Option<(String, u64)>) {
    if left.is_some_and(|(status, sets)| status == app.ui.status && sets == app.ui.status_sets) {
        app.status("");
        app.ui.status_notes = None;
    }
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

/// A string writer's error during a save: [`EngineError::Cancelled`] when the save was canceled and
/// the writer stopped on that Cancel (its message is the Cancel's own, alone or after the path), so
/// the file was not put in place. Any other message, such as one naming the copies a failed
/// replacement kept, stays [`EngineError::Other`].
fn writer_error(e: String) -> EngineError {
    use vectorcraft_engine::cancel;
    let stopped = [EngineError::Cancelled.to_string(), cancel::Cancelled.to_string()]
        .iter()
        .any(|c| e == *c || e.strip_suffix(c.as_str()).is_some_and(|rest| rest.ends_with(": ")));
    if stopped && cancel::is_cancelled() { EngineError::Cancelled } else { EngineError::Other(e) }
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
    // The question whether the save may replace files is asked under the request's scope.
    let asked_scope = ScopeHandle::capture();
    let location_refused = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let refused_by_work = location_refused.clone();
    let own_path = target.clone();
    let save_write = app.services.save_write.clone().filter(|_| !is_web(app));
    let save_stream = app.services.save_stream.clone().filter(|_| job.streams() && !is_web(app));
    // A native file streams into the file as it is encoded (desktop; not to a download).
    let stream = app.services.stream_shared.clone().filter(|_| job.streams() && !is_web(app));
    let (exists, allowed) = checks_replaced(app);
    let known: Vec<String> = allowed.iter().chain(app.saves.written_beside.get(&uid).into_iter().flatten()).cloned().collect();
    // The progress window's Cancel: a save canceled before it replaces its file writes nothing.
    let progress = Progress::new();
    let (reporting, asked) = (progress.clone(), progress.clone());
    let write_file = move |write: Writer| {
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
            (_, _, Some(stream)) => job.write_streamed(&mut |p, fill| stream(p, fill).map_err(writer_error)),
            _ => job.write_unless(check, |p, b| write(p, b).map_err(writer_error)),
        };
        match r {
            // Nothing written: the file is as it was.
            Err(EngineError::Cancelled) => Ok(json!({ "cancelled": true })),
            r => r.map_err(|e| {
                if let EngineError::ReadOnlyLocation(reason) = &e {
                    refused_by_work.store(own_refused, std::sync::atomic::Ordering::Relaxed);
                    if !by_control {
                        return reason.clone();
                    }
                }
                e.to_string()
            }),
        }
    };
    let work = move |write: Writer| progress::scope(&reporting, || write_file(write));
    let failed = target.clone().unwrap_or_default();
    // The status line before the save starts: on the worker, the save sets one line ("<label>…")
    // right after this; written right away, it sets none before `then` runs.
    let (sets, started_line) = (app.ui.status_sets, format!("{label}…"));
    // The read-only fallback's picker takes over the save: its end is the save's end.
    let handed = std::rc::Rc::new(std::cell::Cell::new(false));
    let handing = handed.clone();
    let outcome = move |app: &mut VectorcraftApp, r: Result<Value, String>| {
        if r.is_err()
            && location_refused.load(std::sync::atomic::Ordering::Relaxed)
            && let Some(retry) = retry
            && let Some(r) = retry.instead(app, uid)
        {
            handing.set(true);
            return r;
        }
        // Not written: the document keeps its file, format and modified state, and the user is told.
        let r = r.inspect_err(|e| {
            if !by_control {
                report_write_error(app, false, &failed, e);
            }
        })?;
        // Canceled before the file was replaced: the document keeps its file, format and modified
        // state.
        if r.get("cancelled").and_then(Value::as_bool) == Some(true) {
            // A line set since Cancel Save (another command's, another job's) stays.
            let owned = cancel_line_shown(app);
            if owned || background::status_unchanged(app, sets, &started_line) {
                app.status(SAVE_CANCELED);
            }
            if owned {
                app.background.save_cancel_line = Some(app.ui.status_sets);
            }
            return Ok(r);
        }
        // Nothing written either: asked first.
        if let Some(files) = r.get("replaces").and_then(Value::as_array) {
            let files: Vec<String> = files.iter().filter_map(Value::as_str).map(str::to_string).collect();
            return Ok(ask_to_replace_files(app, uid, &files, &allowed, again.0, &again.1, (asked_scope, by_control)));
        }
        // The document may have closed meanwhile: the file is written all the same.
        done.complete(&mut app.session, uid);
        let path = r["path"].as_str().unwrap_or_default().to_string();
        if retargets {
            note_recent(app, &path);
            // The files written beside the document's file, which its next save replaces.
            let beside = ["files", "linked"].iter().flat_map(|k| r[*k].as_array().into_iter().flatten()).filter_map(Value::as_str);
            let beside: Vec<String> = beside.filter(|p| *p != path).map(str::to_string).collect();
            app.saves.written_beside.insert(uid, beside);
        }
        // A line set while the file was written (another command's, another job's) stays. A Cancel
        // that came once the file was being replaced said it was canceled: the file is saved, which
        // replaces that line while it is still shown, and only then.
        let owned = asked.cancelled() && cancel_line_shown(app);
        if owned || background::status_unchanged(app, sets, &started_line) {
            report_saved(app, &path, &r);
        }
        if owned {
            app.background.save_cancel_line = Some(app.ui.status_sets);
        }
        Ok(r)
    };
    let then = move |app: &mut VectorcraftApp, r: Result<Value, String>| {
        let out = outcome(app, r);
        // The desktop's flow ends with the write, unless the read-only fallback's picker took the save over.
        #[cfg(not(target_arch = "wasm32"))]
        if !handed.get() {
            finish(app, uid, retargets, &out);
        }
        out
    };
    let mut r = background::run_saving(app, enabled, label, uid, progress, work, then)?;
    if let Some(path) = target.filter(|_| r.get("path").is_none()) {
        r["path"] = json!(path);
    }
    if r.get("background").and_then(Value::as_bool) == Some(true) {
        r["format"] = json!(format);
        r["warnings"] = json!(warnings);
    }
    Ok(r)
}
