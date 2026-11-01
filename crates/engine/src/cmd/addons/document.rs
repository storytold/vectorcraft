//! Document Tools (`addon.document.*`), a built-in add-on: Save a Timestamped Copy writes a copy of
//! the document named with the date and time through File › Save a Copy's save; New Symbols from
//! Layers makes a symbol of each layer's art as New Symbol does; Export All Open Documents writes a
//! copy of every open document to one folder as Save a Copy does (#4920). Its submenu (Object › Plug-ins ›
//! Document Tools) is the UI's.

#[cfg(test)]
mod tests;

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeId, NodeKind, SymbolRegistration};

use super::super::brushsym::{instance_in_place, normalize, place_xf, store_symbol};
use super::super::fileio::{self, SaveMode};
use super::super::{CommandSpec, bad, checked_ids_param, clock_date, cmd, date_param, has_doc, str_param, switch_or};
use crate::{Result, Session};

/// The most numbers (`-2`, `-3`…) tried after a timestamped name that is taken.
const MAX_NUMBER: u32 = 999;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.document.saveTimestampedCopy",
            "Save a Timestamped Copy",
            ["Object", "Plug-ins", "Document Tools"],
            None,
            "{folder? (default: the document's own folder; needed for a document never saved), format?: vectorcraft|pdf|svg|svgz|ai|eps (default: the document's own, else vectorcraft), time?: Unix seconds (default: now)} write a copy of the document named <name>_YYYY-MM-DD_HH-MM.<ext> (the date and time in UTC; -2, -3… added when that name is taken) as File › Save a Copy writes it: the document keeps its path, title and modified state, and no undo step is made. Off where there is no clock (the web) → {path, format, bytes, warnings}",
            has_clock,
            save_timestamped_copy
        ),
        cmd!(
            "addon.document.exportAllOpen",
            "Export All Open Documents",
            ["Object", "Plug-ins", "Document Tools"],
            None,
            "{folder, format?: vectorcraft|pdf|svg|svgz|ai|eps (default: pdf)} write a copy of every open document, in tab order, to `folder` as File › Save a Copy writes it, each named after its document (<name>.<ext>; -2, -3… added when that name is taken, so no file is replaced): every document keeps its path, title and modified state, and no undo step is made. A document that can't be written is listed in `failed` and the others are still written; the active document stays active → {files: [{title, path, warnings}], failed: [{title, error}], format}",
            has_doc,
            export_all_open
        ),
        cmd!(
            "addon.document.symbolsFromLayers",
            "New Symbols from Layers",
            ["Object", "Plug-ins", "Document Tools"],
            None,
            "{layers?: [layer id…] (top-level layers; default: those highlighted in the Layers panel when two or more are, else every one), replace?: bool (false: the art stays; true: each layer's art is replaced by an instance of its symbol in the same place)} make one symbol of each layer's art as symbol.new makes one (registration in the centre), named after the layer (with a number added when a symbol has that name), as one undo step. Layers with no art are skipped, and with replace so are locked layers; with replace the instances are selected → {symbols: [name…], ids: [instance id…] (with replace), skipped: [{layer, name, reason: \"empty\"|\"locked\"}…]}",
            has_doc,
            symbols_from_layers
        ),
    ]
}

/// A document is open and the time can be told.
fn has_clock(s: &Session) -> std::result::Result<(), String> {
    has_doc(s)?;
    vectorcraft_doc::metadata::output_time().map(|_| ()).ok_or_else(|| "there is no clock here".into())
}

/// `<base>.<ext>`, then `<base>-2.<ext>`… in `folder`: the first that `taken` doesn't hold.
fn free_name(folder: &str, base: &str, ext: &str, taken: impl Fn(&str) -> bool) -> Option<String> {
    (1..=MAX_NUMBER)
        .map(|i| {
            let name = if i == 1 { format!("{base}.{ext}") } else { format!("{base}-{i}.{ext}") };
            std::path::Path::new(folder).join(name).to_string_lossy().into_owned()
        })
        .find(|path| !taken(path))
}

fn save_timestamped_copy(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.document.saveTimestampedCopy";
    let st = s.doc()?;
    let format = match str_param(p, "format") {
        Some(f) => fileio::save_format(Some(f), None),
        // A document opened from a format Save doesn't write (a picture, a drawing) copies as native.
        None => fileio::save_format(Some(st.format), None).or_else(|_| fileio::save_format(None, None)),
    }
    .map_err(|e| bad(C, e))?;
    let folder = match str_param(p, "folder").filter(|f| !f.trim().is_empty()) {
        Some(f) => f.to_string(),
        None => st
            .path
            .as_deref()
            .and_then(|path| std::path::Path::new(path).parent())
            .map(|f| f.to_string_lossy().into_owned())
            .filter(|f| !f.is_empty())
            .ok_or_else(|| bad(C, "the document has never been saved: give `folder`"))?,
    };
    let stem = fileio::file_stem(st.path.as_deref().unwrap_or(&st.doc.title));
    let given = date_param(p, "time", C)?;
    let t = clock_date(s, "time", given).ok_or_else(|| bad(C, "there is no clock here: give `time` (Unix seconds)"))?;
    let [y, mo, d, h, mi, _] = vectorcraft_doc::metadata::civil(t);
    let base = format!("{stem}_{y:04}-{mo:02}-{d:02}_{h:02}-{mi:02}");
    // Only whether something is there: the write itself goes through the automation roots.
    let path = free_name(&folder, &base, format.extensions[0], |p| std::path::Path::new(p).exists())
        .ok_or_else(|| bad(C, "every numbered name for the copy is taken"))?;
    fileio::save_document(s, SaveMode::Copy, &json!({ "path": path, "format": format.id }))
}

fn export_all_open(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.document.exportAllOpen";
    let format = fileio::save_format(Some(str_param(p, "format").unwrap_or("pdf")), None).map_err(|e| bad(C, e))?;
    let folder = str_param(p, "folder").map(str::trim).filter(|f| !f.is_empty()).ok_or_else(|| bad(C, "give `folder`"))?.to_string();
    let ext = format.extensions.first().copied().ok_or_else(|| bad(C, "the format has no file extension"))?;
    let (mut files, mut failed, mut written) = (vec![], vec![], Vec::<String>::new());
    let active = s.active;
    for i in 0..s.docs.len() {
        // Each document in turn is the one Save a Copy writes; switching tabs this way leaves every
        // document's tool and interaction alone, and the active one comes back below.
        s.active = Some(i);
        let Ok(st) = s.doc() else { continue };
        let title = st.doc.title.clone();
        let stem = fileio::file_stem(st.path.as_deref().unwrap_or(&title));
        // Names the copies already took count as taken, as do files on disk.
        let Some(path) = free_name(&folder, &stem, ext, |p| written.iter().any(|w| w == p) || std::path::Path::new(p).exists()) else {
            failed.push(json!({ "title": title, "error": "every numbered name for the copy is taken" }));
            continue;
        };
        match fileio::save_document(s, SaveMode::Copy, &json!({ "path": path, "format": format.id })) {
            Ok(r) => {
                written.push(path.clone());
                files.push(json!({ "title": title, "path": r.get("path").cloned().unwrap_or(json!(path)), "warnings": r.get("warnings").cloned().unwrap_or(json!([])) }));
            }
            Err(e) => failed.push(json!({ "title": title, "error": e.to_string() })),
        }
    }
    s.active = active;
    Ok(json!({ "files": files, "failed": failed, "format": format.id }))
}

/// `n` with each layer in it, and itself, made a group of the same art and clipping (a symbol holds
/// no layers).
fn without_layers(n: &Node) -> Node {
    let mut n = n.clone();
    if let NodeKind::Layer { children, clip, .. } = &n.kind {
        n.kind = NodeKind::Group { children: children.clone(), clip: *clip };
    }
    if let Some(children) = n.children_mut() {
        for c in children.iter_mut().filter(|c| matches!(c.kind, NodeKind::Layer { .. })) {
            *c = Arc::new(without_layers(c));
        }
    }
    n
}

fn symbols_from_layers(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "addon.document.symbolsFromLayers";
    let replace = switch_or(p, "replace", false, C)?;
    let st = s.doc()?;
    let d = &st.doc;
    let layers: Vec<NodeId> = match p.get("layers").filter(|v| !v.is_null()) {
        // The layers highlighted in the Layers panel when two or more are (one is mostly just the
        // current layer), else every one.
        None => {
            let rows = st.highlighted_rows();
            let picked: Vec<NodeId> = d.layers.iter().map(|l| l.id).filter(|id| rows.contains(id)).collect();
            if picked.len() > 1 { picked } else { d.layers.iter().map(|l| l.id).collect() }
        }
        Some(_) => {
            let ids = checked_ids_param(s, p, "layers", C)?;
            if let Some(NodeId(id)) = ids.iter().find(|id| !d.layers.iter().any(|l| l.id == **id)) {
                return Err(bad(C, format!("{id} is no top-level layer")));
            }
            d.layers.iter().map(|l| l.id).filter(|id| ids.contains(id)).collect()
        }
    };
    // Each layer with art: its id and name, the art's roots, and the art ready for a symbol.
    let (mut made, mut skipped) = (vec![], vec![]);
    for layer in layers.iter().filter_map(|id| d.layers.iter().find(|l| l.id == *id)) {
        let name = layer.display_name();
        let roots: Vec<NodeId> = layer.children().map(|c| c.iter().map(|n| n.id).collect()).unwrap_or_default();
        let art = match layer.children().map(Vec::as_slice).unwrap_or_default() {
            [] => None,
            [one] => Some(without_layers(one)),
            many => Some(without_layers(&Node::group(NodeId(0), many.to_vec()))),
        };
        let reason = if replace && layer.locked { Some("locked") } else { None };
        let ready = art.and_then(|mut art| {
            crate::cmd::threads::keep_shown_text(d, &mut art);
            normalize(art).ok()
        });
        match (reason, ready) {
            (None, Some(ready)) => made.push((name, roots, ready)),
            (reason, _) => skipped.push(json!({ "layer": layer.id.0, "name": name, "reason": reason.unwrap_or("empty") })),
        }
    }
    if made.is_empty() {
        return Ok(json!({ "symbols": [], "ids": [], "skipped": skipped }));
    }
    let (names, ids) = s.edit("New Symbols from Layers", |d, sel| {
        let (mut names, mut ids) = (vec![], vec![]);
        for (name, roots, (art, size, center)) in &made {
            let name = d.unique_symbol_name(name);
            store_symbol(d, art, &name, *size);
            if replace {
                ids.push(instance_in_place(d, roots, &name, place_xf(*center, *size, SymbolRegistration::CENTER))?);
            }
            names.push(name);
        }
        if replace {
            sel.set(ids.iter().copied());
        }
        Ok((names, ids))
    })?;
    Ok(json!({ "symbols": names, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "skipped": skipped }))
}
