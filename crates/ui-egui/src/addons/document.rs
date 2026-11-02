//! Document Tools in the interface: its submenu (Object › Plug-ins › Document Tools), which runs
//! Save a Timestamped Copy, New Symbols from Layers and Convert Layers to Symbols (the same, with
//! each layer's art replaced by an instance). Save a Timestamped Copy of a document never saved
//! asks for the folder to put it in ([`save_copy`]). Export All Open Documents as PDF or SVG asks
//! for the folder the copies go in ([`export_all`], #4920).

use serde_json::{Value, json};

use crate::VectorcraftApp;
use crate::menus::Item;

/// Save a Timestamped Copy.
pub const SAVE_COPY: &str = "addon.document.saveTimestampedCopy";
/// Export All Open Documents.
pub const EXPORT_ALL: &str = "addon.document.exportAllOpen";
/// New Symbols from Layers (Convert Layers to Symbols: with their art replaced by instances).
const SYMBOLS: &str = "addon.document.symbolsFromLayers";

/// Object › Plug-ins › Document Tools.
pub fn menu() -> Vec<Item> {
    vec![
        Item::Cmd("Save a Timestamped Copy", SAVE_COPY, json!({})),
        Item::Cmd("Export All Open Documents as PDF", EXPORT_ALL, json!({ "format": "pdf" })),
        Item::Cmd("Export All Open Documents as SVG", EXPORT_ALL, json!({ "format": "svg" })),
        Item::Sep,
        Item::Cmd("New Symbols from Layers", SYMBOLS, json!({})),
        Item::Cmd("Convert Layers to Symbols", SYMBOLS, json!({ "replace": true })),
    ]
}

/// Save a Timestamped Copy with params `p`: a document never saved, given no `folder`, goes in the
/// folder picked. The status bar says where the copy went.
pub(crate) fn save_copy(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let mut p = if p.is_object() { p.clone() } else { json!({}) };
    if p.get("folder").is_none() && app.session.active().is_some_and(|st| st.path.is_none()) {
        p["folder"] = json!(crate::picks::folder(app).ok_or("cancelled")?);
    }
    let r = app.session.execute(SAVE_COPY, &p).map_err(|e| e.to_string())?;
    crate::io::report_saved(app, r["path"].as_str().unwrap_or_default(), &r);
    Ok(r)
}

/// Export All Open Documents with params `p`: without a `folder`, the copies go in the folder
/// picked. The status bar says how many were written and names the first that failed.
pub(crate) fn export_all(app: &mut VectorcraftApp, p: &Value) -> Result<Value, String> {
    let mut p = if p.is_object() { p.clone() } else { json!({}) };
    if p.get("folder").is_none() {
        p["folder"] = json!(crate::picks::folder(app).ok_or("cancelled")?);
    }
    let r = app.session.execute(EXPORT_ALL, &p).map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod tests;
