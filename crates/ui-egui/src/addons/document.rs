//! Document Tools in the interface: its submenu (Object › Plug-ins › Document Tools), which runs
//! Save a Timestamped Copy, New Symbols from Layers and Convert Layers to Symbols (the same, with
//! each layer's art replaced by an instance). Save a Timestamped Copy of a document never saved
//! asks for the folder to put it in ([`save_copy`]). Export All Open Documents as PDF or SVG asks
//! for the folder the copies go in ([`export_all`], #4920). Batch Trace Folder asks for the folder of
//! images and then the folder the results go in ([`batch_trace`], #4921).

use craft::host::{PickRequest, Picked};
use craft::menu::Node;
use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::file_asks::{Owner, ask};
use crate::menus::{Sep, cp};

/// Save a Timestamped Copy.
pub const SAVE_COPY: &str = "addon.document.saveTimestampedCopy";
/// Export All Open Documents.
pub const EXPORT_ALL: &str = "addon.document.exportAllOpen";
/// Batch Trace Folder.
pub const BATCH_TRACE: &str = "addon.document.batchTraceFolder";
/// New Symbols from Layers (Convert Layers to Symbols: with their art replaced by instances).
const SYMBOLS: &str = "addon.document.symbolsFromLayers";

/// Object › Plug-ins › Document Tools.
pub fn menu() -> Vec<Node> {
    vec![
        cp("Save a Timestamped Copy", SAVE_COPY, json!({})),
        cp("Export All Open Documents as PDF", EXPORT_ALL, json!({ "format": "pdf" })),
        cp("Export All Open Documents as SVG", EXPORT_ALL, json!({ "format": "svg" })),
        cp("Batch Trace Folder", BATCH_TRACE, json!({})),
        Sep,
        cp("New Symbols from Layers", SYMBOLS, json!({})),
        cp("Convert Layers to Symbols", SYMBOLS, json!({ "replace": true })),
    ]
}

/// The params as an object, with the picked folder under `key`.
fn with_folder(p: &Value, key: &str, picked: &Picked) -> Result<Value, String> {
    let folder = picked.first_path().ok_or("no folder picked")?;
    let mut p = if p.is_object() { p.clone() } else { json!({}) };
    p[key] = json!(folder.to_string_lossy());
    Ok(p)
}

/// Save a Timestamped Copy with params `p`: a document never saved, given no `folder`, goes in the
/// folder picked; the command then returns `{"pending": "fileDialog"}` and a declined or unavailable
/// picker runs nothing. The status bar says where the copy went.
pub(crate) fn save_copy(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if p.get("folder").is_none() && app.session.active().is_some_and(|st| st.path.is_none()) {
        let p = p.clone();
        return Ok(ask(app, PickRequest::folder("document"), Owner::Document, move |app, picked| {
            finish_save_copy(app, &with_folder(&p, "folder", &picked)?)
        }));
    }
    finish_save_copy(app, p)
}

fn finish_save_copy(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let r = app.session.execute(SAVE_COPY, p).map_err(|e| e.to_string())?;
    crate::io::report_saved(app, r["path"].as_str().unwrap_or_default(), &r);
    Ok(r)
}

/// Export All Open Documents with params `p`: without a `folder`, the copies go in the folder
/// picked; the command then returns `{"pending": "fileDialog"}` and a declined or unavailable
/// picker runs nothing. The status bar says how many were written and names the first that failed.
pub(crate) fn export_all(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    if p.get("folder").is_none() {
        let p = p.clone();
        return Ok(ask(app, PickRequest::folder("document"), Owner::Free, move |app, picked| {
            finish_export_all(app, &with_folder(&p, "folder", &picked)?)
        }));
    }
    finish_export_all(app, p)
}

fn finish_export_all(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let r = app.session.execute(EXPORT_ALL, p).map_err(|e| e.to_string())?;
    let written = r["files"].as_array().map_or(0, Vec::len);
    let failed = r["failed"].as_array().cloned().unwrap_or_default();
    let status = match failed.first() {
        Some(f) => {
            let args = [("title", f["title"].as_str().unwrap_or_default()), ("error", f["error"].as_str().unwrap_or_default())];
            let (one, other) = ("Exported {n} document; {title} wasn't written: {error}", "Exported {n} documents; {title} wasn't written: {error}");
            crate::i18n::msg_n(written as u64, one, other, &args)
        }
        None => crate::i18n::msg_n(written as u64, "Exported {n} document", "Exported {n} documents", &[]),
    };
    app.status(status);
    Ok(r)
}

/// Batch Trace Folder with params `p`: the folder of images and then the folder the results go in
/// are asked for, one after the other, when not given; the command returns
/// `{"pending": "fileDialog"}` meanwhile and a declined or unavailable picker runs nothing. The
/// status bar says how many images were traced and how many couldn't be.
pub(crate) fn batch_trace(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    batch_trace_from(app, p, &["folder", "destinationFolder"])
}

/// Asks for the first of `missing` the params lack, then continues with the rest; runs the trace when
/// every folder is known.
fn batch_trace_from(app: &mut VectorcraftApp, p: &Value, keys: &'static [&'static str]) -> Result<Value, String> {
    let Some((&key, rest)) = keys.iter().position(|k| p.get(*k).is_none()).and_then(|i| keys.get(i..)).and_then(<[_]>::split_first) else {
        return finish_batch_trace(app, p);
    };
    let p = p.clone();
    Ok(ask(app, PickRequest::folder("document"), Owner::Free, move |app, picked| batch_trace_from(app, &with_folder(&p, key, &picked)?, rest)))
}

fn finish_batch_trace(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let r = app.session.execute(BATCH_TRACE, p).map_err(|e| e.to_string())?;
    let traced = r["traced"].as_array().map_or(0, Vec::len) as u64;
    let failed = r["failed"].as_array().map_or(0, Vec::len) as u64;
    // One count per message: a count inside another counted message would read as the wrong
    // template when the status is translated.
    let status = if failed == 0 {
        crate::i18n::msg_n(traced, "Traced {n} image", "Traced {n} images", &[])
    } else if traced == 0 {
        crate::i18n::msg_n(failed, "{n} image couldn't be traced", "{n} images couldn't be traced", &[])
    } else {
        let failed = failed.to_string();
        crate::i18n::msg_n(traced, "Traced {n} image; not traced: {failed}", "Traced {n} images; not traced: {failed}", &[("failed", &failed)])
    };
    app.status(status);
    Ok(r)
}

#[cfg(test)]
mod tests;
