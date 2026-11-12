use super::save::{
    ask_before_losing, ask_to_replace_files, checks_replaced, export_path, is_web, notes_of, remember_active_artboard, replaced_files, report_saved,
    report_write_error, saved_status, show_with_notes, target_path, target_path_keeping, view_file, with_path, write_encoded, write_named,
    write_reporting,
};
use super::{EngineError, Progress, Value, VectorcraftApp, Writer, background, cancel, fileio, json, progress};

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
    // A Shape Builder merge still being computed for a click is exported with the document.
    let r = app.session.finish_tool_edit();
    crate::canvas::apply_requests(app, r);
    // A `.ai` file names the view's active artboard.
    remember_active_artboard(app);
    let st = app.session.active().ok_or("no document")?;
    let doc = match fileio::export_doc(st, f, params).map_err(|e| e.to_string())? {
        std::borrow::Cow::Borrowed(_) => st.doc.clone(),
        std::borrow::Cow::Owned(d) => std::sync::Arc::new(d),
    };
    // A PDF shows its title in the viewer's title bar only when it is the document's own, not the
    // file name it was titled after, as `document.export` writes it.
    let own_title = fileio::has_own_title(&doc);
    let doc = fileio::titled_for_export(&doc, st.path.as_deref(), f.id).map_or(doc, std::sync::Arc::new);
    let uid = st.uid;
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
    // A PDF's View PDF after saving: the first file written opens in the system viewer, unless
    // automation asked for the export (checked now: a background export ends after the request).
    let view = params.get("viewAfterSaving").and_then(Value::as_bool).unwrap_or(false) && app.refuse_automation("viewAfterSaving").is_ok();
    let (mut params, target) = (params.clone(), path.clone());
    if let Some(o) = params.as_object_mut() {
        o.insert("path".into(), json!(path));
        if f.id == "pdf" {
            o.insert("_displayDocTitle".into(), json!(own_title));
        }
    }
    // The progress dialog's counts and Cancel: a cancelled export stops drawing and never starts
    // writing.
    let progress = Progress::new();
    let (reporting, asked) = (progress.clone(), progress.clone());
    let (exists, allowed) = checks_replaced(app);
    let allowed_before = allowed.clone();
    let rerun = with_path(&again, &path);
    let work = move |write: Writer| {
        progress::scope(&reporting, || {
            cancel::check().map_err(|e| e.to_string())?;
            let enc = fileio::encode_all(&doc, f.id, &params).map_err(|e| e.to_string())?;
            cancel::check().map_err(|e| e.to_string())?;
            let replaced = replaced_files(exists.as_ref(), &path, &allowed, enc.named(&doc, &path).iter().map(|(p, _)| p.as_str()));
            if !replaced.is_empty() {
                return Ok(json!({ "replaces": replaced }));
            }
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
        // Nothing written: asked first.
        if let Some(files) = r.get("replaces").and_then(Value::as_array) {
            let files: Vec<String> = files.iter().filter_map(Value::as_str).map(str::to_string).collect();
            return Ok(ask_to_replace_files(app, uid, &files, &allowed_before, "file.exportAs", &rerun));
        }
        let what = match r["files"].as_array() {
            Some(files) => format!("{} files", files.len()),
            None => r["path"].as_str().unwrap_or_default().to_string(),
        };
        let warnings = notes_of(&r);
        let status = match warnings.first() {
            Some(first) => exported_with_notes(&what, warnings.len(), first),
            None => format!("Exported {what}"),
        };
        if view && let Some(path) = r["path"].as_str() {
            view_file(app, path);
        }
        show_with_notes(app, status, warnings);
        Ok(r)
    })?;
    if r.get("path").is_none() {
        r["path"] = json!(target);
    }
    Ok(r)
}

/// The status bar's line for an export of `what` with `n` notes, the first of them `first`.
pub(crate) fn exported_with_notes(what: &str, n: usize, first: &str) -> String {
    let (one, other) = ("Exported {what} with {n} note: {first}", "Exported {what} with {n} notes: {first}");
    crate::i18n::msg_n(n as u64, one, other, &[("what", what), ("first", first)])
}

/// Run an engine command that returns `{dataBase64}` and write the bytes to a picked path
/// (Export Selection, Save for Office Documents, Text Export Options and the presets dialogs'
/// Export…), reported with the command's warnings ([`report_saved`]).
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
/// downloads it). Writing over the file the document was opened from, when opening it left things
/// out, asks first, for a given path and for one the save panel picked, and runs
/// `file.export.pdf` again on OK.
pub fn export_pdf(app: &mut VectorcraftApp, mut params: Value) -> Result<Value, String> {
    if let Some(path) = params.get("path").and_then(Value::as_str)
        && let Some(r) = ask_before_losing(app, path, "file.export.pdf", &params)
    {
        return Ok(r);
    }
    let view = params.get("viewAfterSaving").and_then(Value::as_bool).unwrap_or(false);
    let given = params.as_object_mut().and_then(|o| o.remove("path")).and_then(|p| p.as_str().map(str::to_string));
    let mut v = app.session.execute("document.exportPdf", &params).map_err(|e| e.to_string())?;
    let bytes = take_output(&mut v)?;
    let path = match given {
        Some(p) => p,
        None => {
            let picked = target_path(app, None, "pdf")?;
            if let Some(r) = ask_before_losing(app, &picked, "file.export.pdf", &params) {
                return Ok(r);
            }
            picked
        }
    };
    write_reporting(app, &path, &bytes)?;
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

/// Edit Original: open `path` in the system's default app for it, or with Preferences › File
/// Handling › Use System Defaults for Edit Original off, in the app that made it (#5236): PhotoCraft
/// for a raster image where it is installed. Any other file, or no PhotoCraft, uses the system's app.
pub(crate) fn open_original(app: &mut VectorcraftApp, path: &str) -> Result<(), String> {
    let ext = std::path::Path::new(path).extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let raster = vectorcraft_engine::cmd::fileio::readable_format(&ext).is_some_and(|f| f.raster);
    if !app.session.prefs.edit_original_system_defaults
        && raster
        && let Some(open) = app.services.open_in_photocraft.as_mut()
        && open(path)?
    {
        return Ok(());
    }
    open_in_app(app, path)
}

/// File → Export for Screens with `document.exportForScreens` params: the files go into `folder`;
/// without one the web downloads them (the ZIP, or each file) and the other frontends get them
/// back as base64. With `openLocation`, the file manager shows the first file written. When an
/// export into a folder or a download has warnings, the status bar shows them as [`report_saved`]
/// shows a save's notes: "Saved <folder, ZIP or first download> with n notes: …", and hovering the
/// message lists up to 30 of them, fewer when the window is small, with how many were left out.
pub fn export_for_screens(app: &mut VectorcraftApp, params: Value) -> Result<Value, String> {
    export_files(app, "document.exportForScreens", params)
}

/// [`export_for_screens`] through command `id` (`document.exportForScreens` or `assets.export`; or
/// `assets.export3D`, whose reply takes the same form and which always runs at once).
/// On the desktop with Background Export on, an export into a folder (the `folder` param, else the
/// one its `exportPreset` keeps) is checked, journaled and prepared from the document as it is now,
/// and the files are encoded and written on the background thread ([`background`]): this returns
/// `{background: true, status}` at once, with the progress dialog's Cancel, and the status bar
/// reports the files once they are written.
pub(crate) fn export_files(app: &mut VectorcraftApp, id: &str, params: Value) -> Result<Value, String> {
    let open_location = params.get("openLocation").and_then(Value::as_bool).unwrap_or(false);
    // The folder written into, and its name when the reply leaves it out.
    let (folder, hidden) = if id == "assets.export3D" {
        (params.get("folder").and_then(Value::as_str).unwrap_or_default().to_string(), None)
    } else {
        app.session.screen_export_folder(id, &params).unwrap_or_default()
    };
    // Without a folder the files come back in the reply, which the caller waits for.
    let in_background =
        app.session.prefs.background_export && app.services.write_shared.is_some() && !is_web(app) && !folder.is_empty() && id != "assets.export3D";
    if !in_background {
        // Made at once, after the exports and saves started before it.
        background::wait_all(app);
        let r = app.run(id, params)?;
        return report_exported(app, r, &folder, hidden.as_deref(), open_location);
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
        report_exported(app, r, &folder, hidden.as_deref(), open_location)
    })
}

/// Report the files `r` (the reply of `document.exportForScreens`) in the status bar: the count
/// written into `folder` (named `hidden` when the reply leaves it out), else the ZIP or the files
/// downloaded (the web); with `open_location`, the file manager shows the first file written, or
/// the folder for a ZIP whose path the reply leaves out.
fn report_exported(app: &mut VectorcraftApp, r: Value, folder: &str, hidden: Option<&str>, open_location: bool) -> Result<Value, String> {
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
        let shown = hidden.unwrap_or(folder);
        let first = match r["path"].as_str().or_else(|| r["files"][0].as_str()) {
            // The reply names the files relative to a folder it leaves out, and a ZIP's files inside it.
            Some(_) if hidden.is_some() && r.get("bytes").is_some() => Some(folder.to_string()),
            Some(name) if hidden.is_some() => Some(format!("{folder}/{name}")),
            first => first.map(str::to_string),
        };
        let mut status = crate::i18n::msg_n(count as u64, "Exported {n} file to {folder}", "Exported {n} files to {folder}", &[("folder", shown)]);
        if warned {
            status = saved_status(r["path"].as_str().unwrap_or(shown), &r);
        }
        if open_location
            && let Some(first) = first
            && let Err(e) = reveal_path(app, &first)
        {
            let e = if hidden.is_some() { e.replace(folder, shown) } else { e };
            status = format!("{status} ({e})");
        }
        show_with_notes(app, status, notes_of(&r));
    } else if let Some(download) = app.services.download.as_mut() {
        // A browser download has no folders: a file from a sub-folder (`2x/Logo@2x.png`) is named alone.
        let returned: Vec<(&str, &str)> = returned.iter().map(|(name, data)| (name.rsplit('/').next().unwrap_or(name), *data)).collect();
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
