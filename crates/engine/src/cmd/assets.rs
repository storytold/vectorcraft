//! Window → Asset Export and Object → Collect for Export: art collected as assets
//! ([`vectorcraft_doc::ExportAsset`]), exported each on its own, cropped to its art, at the export
//! settings Export for Screens shares (`document.exportSettings`). File → Export Selection collects
//! the selection this way and opens Export for Screens on its Assets tab (a UI command).

use serde_json::{Map, Value, json};
use vectorcraft_doc::{ExportAsset, NodeId};

use super::edit::{roots_of, selected_roots};
use super::fileio::{SCREEN_SETTINGS_KEYS, check_screen_settings, export_crop, export_screens, store_screen_settings};
use super::*;

/// The longest asset name kept (characters).
const MAX_NAME: usize = 255;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "assets.add",
            "Collect for Export",
            ["Object", "Collect for Export"],
            None,
            "{ids?: [id…] (default: the selected objects; a layer id collects the art under it, including in its sublayers), multiple?: true (an asset per object; false: one asset of them all), name?: (the asset's name, when it makes one)} collect art as assets for Window › Asset Export and Export for Screens' Assets tab, named after the object (else Asset 1, Asset 2…); art already collected as such an asset keeps it → {assets: [asset id…] (in order), added: n}. Assets follow their art: moving it moves the crop, deleting it removes it from the asset",
            has_doc,
            add
        ),
        cmd!(
            "assets.remove",
            "Remove Asset",
            [],
            None,
            "{assets: [asset id…]} take assets out of the Asset Export panel (their art stays) → {removed: n}",
            has_doc,
            remove
        ),
        cmd!("assets.rename", "Rename Asset", [], None, "{asset: id, name} rename an asset (its files' names)", has_doc, rename),
        cmd!(
            query "assets.list",
            "Assets",
            [],
            None,
            "{} → {assets: [{id, name, nodes: [id…], bounds: [x, y, width, height] | null (the crop: the bounds the renderer measures for what its art draws, effects included)}]} the Asset Export panel's assets, in order",
            has_doc,
            list
        ),
        cmd!(
            "assets.export",
            "Export Assets",
            [],
            None,
            "{assets?: [asset id…] (default all), folder?, zip?, openLocation?, formats?, preset?, settings?, prefix?, subfolders?, antiAlias?} each asset's art alone, cropped to it, in every format row: document.exportForScreens {assets} at the document's export settings (document.exportSettings) under the ones given → what document.exportForScreens returns (no folder: the files, or one zip, as dataBase64). Leaves the remembered settings as they are (assets.settings.set changes them)",
            has_doc,
            export
        ),
        cmd!(
            "assets.settings.set",
            "Asset Export Settings",
            [],
            None,
            "{formats?, preset?: mobile|density|\"\" (none), settings?, prefix?, subfolders?, antiAlias?} change the export settings the Asset Export panel and Export for Screens share (document.exportSettings; the values of document.exportForScreens; null removes one): formats replace a preset, a preset replaces the formats → {settings}. Not an undo step",
            has_doc,
            settings_set
        ),
    ]
}

/// `e` as an error of command `cmd` (for errors of the export underneath).
fn of_cmd(cmd: &str, e: EngineError) -> EngineError {
    match e {
        EngineError::BadParams { msg, .. } => bad(cmd, msg),
        e => e,
    }
}

/// `name` trimmed and at most [`MAX_NAME`] characters long (`None` when empty).
fn clean_name(name: &str) -> Option<String> {
    let n: String = name.trim().chars().take(MAX_NAME).collect();
    (!n.is_empty()).then_some(n)
}

/// Ids of art under `id`: the id itself when it is not a layer, else every non-layer under it
/// (sublayers unfolded). Layers alone have nothing to collect.
pub(crate) fn art_under(doc: &vectorcraft_doc::Document, id: NodeId, out: &mut Vec<NodeId>) {
    let Some(n) = doc.node(id) else { return };
    if n.is_layer() {
        if let Some(kids) = n.children() {
            for c in kids {
                art_under(doc, c.id, out);
            }
        }
        return;
    }
    out.push(id);
}

fn add(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "assets.add";
    let ids = if p.get("ids").is_some_and(|v| !v.is_null()) {
        // An explicit list is authoritative: never silently drop malformed or stale ids.
        let ids = checked_ids_param(s, p, "ids", C)?;
        let d = &s.doc()?.doc;
        let mut art = Vec::new();
        for id in &ids {
            art_under(d, *id, &mut art);
        }
        roots_of(d, d.paint_order(art))
    } else {
        selected_roots(s)?
    };
    if ids.is_empty() {
        return Err(bad(C, "select the art to collect (or give its ids)"));
    }
    let name = str_param(p, "name").and_then(clean_name);
    let sets: Vec<Vec<NodeId>> = if bool_or(p, "multiple", true) { ids.iter().map(|id| vec![*id]).collect() } else { vec![ids] };
    if name.is_some() && sets.len() > 1 {
        return Err(bad(C, "a name names one asset: give multiple: false, or one object"));
    }
    let existing = |d: &vectorcraft_doc::Document, nodes: &[NodeId]| d.assets.iter().find(|a| a.nodes == nodes).map(|a| a.id);
    let d = &s.doc()?.doc;
    // Nothing new: no undo step.
    if sets.iter().all(|set| existing(d, set).is_some()) {
        return Ok(json!({ "assets": sets.iter().filter_map(|set| existing(d, set)).collect::<Vec<_>>(), "added": 0 }));
    }
    s.edit("Collect for Export", |d, _| {
        let (mut out, mut added) = (vec![], 0);
        for nodes in sets {
            if let Some(id) = existing(d, &nodes) {
                out.push(id);
                continue;
            }
            let own = match nodes.as_slice() {
                [one] => d.node(*one).and_then(|n| n.name.as_deref()).and_then(clean_name),
                _ => None,
            };
            let name = name.clone().or(own).unwrap_or_else(|| d.next_asset_name());
            let id = d.alloc_id().0;
            d.assets.push(ExportAsset { id, name, nodes });
            out.push(id);
            added += 1;
        }
        Ok(json!({ "assets": out, "added": added }))
    })
}

/// The asset ids of param `key`.
fn asset_ids(p: &Value, key: &str, cmd: &str) -> Result<Vec<u64>> {
    let ids = p.get(key).and_then(Value::as_array).ok_or_else(|| bad(cmd, format!("`{key}` must be an array of asset ids (see assets.list)")))?;
    ids.iter().map(|v| v.as_u64().ok_or_else(|| bad(cmd, format!("{v} is not an asset id")))).collect()
}

fn remove(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "assets.remove";
    let ids = asset_ids(p, "assets", C)?;
    let n = s.doc()?.doc.assets.iter().filter(|a| ids.contains(&a.id)).count();
    if n == 0 {
        return Err(bad(C, "no such assets (see assets.list)"));
    }
    s.edit("Remove Asset", |d, _| {
        d.assets.retain(|a| !ids.contains(&a.id));
        Ok(json!({ "removed": n }))
    })
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "assets.rename";
    let id = p.get("asset").and_then(Value::as_u64).ok_or_else(|| bad(C, "`asset` is the asset id (see assets.list)"))?;
    let name = str_param(p, "name").and_then(clean_name).ok_or_else(|| bad(C, "give the asset a name"))?;
    let at = s.doc()?.doc.assets.iter().position(|a| a.id == id).ok_or_else(|| bad(C, format!("no asset {id}")))?;
    s.edit("Rename Asset", |d, _| {
        if let Some(a) = d.assets.get_mut(at) {
            a.name = name;
        }
        ok()
    })
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let d = &s.doc()?.doc;
    let rows: Vec<Value> = d
        .assets
        .iter()
        .map(|a| {
            let bounds = export_crop(d, &a.nodes);
            json!({
                "id": a.id,
                "name": a.name,
                "nodes": a.nodes.iter().map(|n| n.0).collect::<Vec<_>>(),
                "bounds": bounds.map(|b| [b.x0, b.y0, b.width(), b.height()]),
            })
        })
        .collect();
    Ok(json!({ "assets": rows }))
}

/// The shared export settings of the document (the [`SCREEN_SETTINGS_KEYS`] it has).
fn shared_settings(s: &Session) -> Result<Map<String, Value>> {
    let saved = &s.doc()?.doc.export_settings;
    Ok(SCREEN_SETTINGS_KEYS.iter().filter_map(|k| Some((k.to_string(), saved.get(*k).filter(|v| !v.is_null())?.clone()))).collect())
}

/// `base` with the settings of `p` over it: a given value replaces (null removes) the saved one;
/// formats drop a saved preset and a preset (`""`: none) saved formats.
fn overlay(base: &mut Map<String, Value>, p: &Map<String, Value>) {
    let given = |k: &str| p.get(k).is_some_and(|v| !v.is_null());
    if given("formats") && !p.contains_key("preset") {
        base.remove("preset");
    }
    if given("preset") && !p.contains_key("formats") {
        base.remove("formats");
    }
    for (k, v) in p {
        match v {
            Value::Null => _ = base.remove(k),
            Value::String(id) if k == "preset" && id.is_empty() => _ = base.remove(k),
            v => _ = base.insert(k.clone(), v.clone()),
        }
    }
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "assets.export";
    let mut q = shared_settings(s)?;
    if let Some(o) = p.as_object() {
        overlay(&mut q, o);
    }
    if !q.contains_key("assets") {
        let all: Vec<u64> = s.doc()?.doc.assets.iter().map(|a| a.id).collect();
        if all.is_empty() {
            return Err(bad(C, "no assets yet: collect art for export first (assets.add)"));
        }
        q.insert("assets".into(), json!(all));
    }
    export_screens(s, &Value::Object(q)).map_err(|e| of_cmd(C, e))
}

fn settings_set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "assets.settings.set";
    let o = p.as_object().ok_or_else(|| bad(C, "give the settings to change, e.g. {\"formats\": [{\"format\": \"png\", \"scale\": \"2x\"}]}"))?;
    if let Some(k) = o.keys().find(|k| !SCREEN_SETTINGS_KEYS.contains(&k.as_str())) {
        return Err(bad(C, format!("`{k}` isn't an export setting: {}", SCREEN_SETTINGS_KEYS.join(", "))));
    }
    let mut settings = s.doc()?.doc.export_settings.clone();
    overlay(&mut settings, o);
    check_screen_settings(s, &Value::Object(settings.clone())).map_err(|e| of_cmd(C, e))?;
    store_screen_settings(s, settings)?;
    Ok(json!({ "settings": shared_settings(s)? }))
}
