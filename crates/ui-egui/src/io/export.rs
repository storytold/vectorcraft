use super::save::{
    ask_before_losing, export_path, is_web, report_saved, report_write_error, saved_status, target_path, target_path_keeping, view_file,
    write_encoded, write_named, write_reporting,
};
use super::*;

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
