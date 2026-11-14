//! Window → Asset Export and Object → Collect for Export: art collected as assets
//! ([`vectorcraft_doc::ExportAsset`]), exported each on its own, cropped to its art, at the export
//! settings Export for Screens shares (`document.exportSettings`). File → Export Selection collects
//! the selection this way and opens Export for Screens on its Assets tab (a UI command).

use std::collections::HashSet;

use serde_json::{Map, Value, json};
use vectorcraft_doc::{ExportAsset, NodeId};

use super::edit::{roots_of, selected_roots};
use super::fileio::{
    DEFAULT_FORMAT_3D, FORMATS_3D, SCREEN_SETTINGS_KEYS, check_screen_settings, create_dir, export_crop, export_screens, mesh_file_names, mesh_files,
    safe_file_name, store_screen_settings, with_screen_export_preset, write_file,
};
use super::{CommandSpec, EngineError, Result, Session, bad, bool_or, checked_ids_param, cmd, has_doc, ok, str_param, switch_or, text_param};

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
            "{} → {assets: [{id, name, nodes: [id…], bounds: [x, y, width, height] | null (the crop: the bounds the renderer measures for what its art draws, effects included, with the clips of the groups and layers that hold it), format3D: usda|usdz|gltf|obj | null (a 3D asset's mesh format; null for art exported as pictures)}]} the Asset Export panel's assets, in order",
            has_doc,
            list
        ),
        cmd!(
            "assets.export",
            "Export Assets",
            [],
            None,
            "{assets?: [asset id…] (default all but the 3D assets, which assets.export3D writes), folder?, zip?, openLocation?, formats?, preset?, settings?, prefix?, subfolders?, antiAlias?} each asset's art alone, cropped to it, in every format row: document.exportForScreens {assets} at the document's export settings (document.exportSettings) under the ones given; with subfolders false every file goes into the folder itself, mobile and density rows too → what document.exportForScreens returns (no folder: the files, or one zip, as dataBase64; warnings name a key in the rows or settings given that only another format takes, not one in the remembered settings). Leaves the remembered settings as they are (assets.settings.set changes them)",
            has_doc,
            export
        ),
        cmd!(
            "assets.add3D",
            "Export 3D Object",
            [],
            None,
            "{ids?: [id…] (default: the selected objects), format?: usda|usdz|gltf|obj (default usda)} collect each object with an Extrude & Bevel, Revolve, Inflate or Rotate effect as a 3D asset of Window › Asset Export, named after the object (else Asset 1, Asset 2…): assets.export3D writes it as a mesh file. An object already collected as a 3D asset keeps it; an object without such an effect is refused → {assets: [asset id…] (in order), added: n}",
            has_doc,
            add_3d
        ),
        cmd!(
            "assets.set3DFormat",
            "3D Export Format",
            [],
            None,
            "{assets: [asset id…], format: usda|usdz|gltf|obj} the mesh format 3D assets export to (Asset Export's Format for a 3D asset) → {changed: n}",
            has_doc,
            set_3d_format
        ),
        cmd!(
            "assets.export3D",
            "Export 3D Assets",
            [],
            None,
            "{assets?: [asset id…] (default every 3D asset), folder?, zip?} each 3D asset's object as a mesh file in its format (assets.set3DFormat), named after the asset: USDA or USDZ, glTF 2.0 (one .gltf, its buffer embedded) or OBJ with its .mtl (whitespace and # in an OBJ's name become _). Every solid of the object (the fill's, each stroke's, a group's members') is a mesh with a material in the color it is painted; faces turned away from the viewer are included, at the object's 3D rotation, y up, about the point it turns about, in points (glTF: meters). Files that would share a name get -2, -3…; an object with a hole that crosses its face's outline or another hole, or with more than 2,000,000 triangles, is refused → {files: [path…]} (no folder: {files: [{name, dataBase64}]}, or with zip one zip as {name, dataBase64, files})",
            has_doc,
            export_3d
        ),
        cmd!(
            "assets.settings.set",
            "Asset Export Settings",
            [],
            None,
            "{exportPreset?: (a preset of document.exportPresets.list: its formats or preset, its format settings over the saved ones, its prefix (none when it has none), its folders (or subfolders: rows or none) with subfolders on for rows, and antiAlias; the other keys go over it), formats?, preset?: mobile|density|\"\" (none), settings?, prefix?, subfolders?, antiAlias?} change the export settings the Asset Export panel and Export for Screens share (document.exportSettings; the values of document.exportForScreens; null removes one): formats replace a preset, a preset replaces the formats → {settings}. Not an undo step",
            has_doc,
            settings_set
        ),
    ]
}

/// `e` as an error of command `cmd` (for errors of the export underneath).
pub(crate) fn of_cmd(cmd: &str, e: EngineError) -> EngineError {
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
            for c in kids.iter().filter(|c| !c.is_ruler_guide()) {
                art_under(doc, c.id, out);
            }
        }
        return;
    }
    if !n.is_ruler_guide() {
        out.push(id);
    }
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
    // A 3D asset of the same art is another asset.
    let existing =
        |d: &vectorcraft_doc::Document, nodes: &[NodeId]| d.assets.iter().find(|a| a.nodes == nodes && a.format_3d.is_none()).map(|a| a.id);
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
            d.assets.push(ExportAsset { id, name, nodes, format_3d: None });
            out.push(id);
            added += 1;
        }
        Ok(json!({ "assets": out, "added": added }))
    })
}

/// The format of param `format` of command `cmd` ([`FORMATS_3D`]), `default` when it is not given.
fn format_3d_param(p: &Value, cmd: &str, default: Option<&str>) -> Result<String> {
    match (p.get("format"), default) {
        (None, Some(d)) => Ok(d.to_string()),
        (Some(Value::String(f)), _) if FORMATS_3D.contains(&f.as_str()) => Ok(f.clone()),
        _ => {
            let key = "format";
            Err(bad(cmd, format!("`{key}` must be one of {}", FORMATS_3D.join(", "))))
        }
    }
}

/// The asset ids of param `key` of command `cmd`, each a 3D asset of `d`.
fn three_d_asset_ids(d: &vectorcraft_doc::Document, p: &Value, key: &str, cmd: &str) -> Result<Vec<u64>> {
    let ids = asset_ids(p, key, cmd)?;
    for id in &ids {
        match d.asset(*id) {
            None => return Err(bad(cmd, "no such assets (see assets.list)")),
            Some(a) if a.format_3d.is_none() => return Err(bad(cmd, format!("asset {id} isn't a 3D asset"))),
            _ => {}
        }
    }
    Ok(ids)
}

fn add_3d(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "assets.add3D";
    let format = format_3d_param(p, C, Some(DEFAULT_FORMAT_3D))?;
    let ids = if p.get("ids").is_some_and(|v| !v.is_null()) { checked_ids_param(s, p, "ids", C)? } else { selected_roots(s)? };
    if ids.is_empty() {
        return Err(bad(C, "select an object with a 3D effect (or give its ids)"));
    }
    let d = &s.doc()?.doc;
    let mut ids = d.paint_order(ids);
    ids.dedup();
    for id in &ids {
        let n = d.node(*id).ok_or_else(|| bad(C, format!("no such object {0}", id.0)))?;
        if let Err(e) = vectorcraft_render::effects::three_d_meshes_in(Some(d), n) {
            let what = n.name.clone().unwrap_or_else(|| format!("#{}", id.0));
            return Err(bad(C, format!("{what} can't be exported in 3D: {e}")));
        }
    }
    let existing = |d: &vectorcraft_doc::Document, id: NodeId| d.assets.iter().find(|a| a.nodes == [id] && a.format_3d.is_some()).map(|a| a.id);
    // Nothing new: no undo step.
    if ids.iter().all(|id| existing(d, *id).is_some()) {
        return Ok(json!({ "assets": ids.iter().filter_map(|id| existing(d, *id)).collect::<Vec<_>>(), "added": 0 }));
    }
    s.edit("Export 3D Object", |d, _| {
        let (mut out, mut added) = (vec![], 0);
        for node in ids {
            if let Some(id) = existing(d, node) {
                out.push(id);
                continue;
            }
            let name = d.node(node).and_then(|n| n.name.as_deref()).and_then(clean_name).unwrap_or_else(|| d.next_asset_name());
            let id = d.alloc_id().0;
            d.assets.push(ExportAsset { id, name, nodes: vec![node], format_3d: Some(format.clone()) });
            out.push(id);
            added += 1;
        }
        Ok(json!({ "assets": out, "added": added }))
    })
}

fn set_3d_format(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "assets.set3DFormat";
    let d = &s.doc()?.doc;
    let ids = three_d_asset_ids(d, p, "assets", C)?;
    let format = format_3d_param(p, C, None)?;
    let changed = d.assets.iter().filter(|a| ids.contains(&a.id) && a.format_3d.as_deref() != Some(format.as_str())).count();
    // Nothing changes: no undo step.
    if changed == 0 {
        return Ok(json!({ "changed": 0 }));
    }
    s.edit("3D Export Format", |d, _| {
        for a in d.assets.iter_mut().filter(|a| ids.contains(&a.id)) {
            a.format_3d = Some(format.clone());
        }
        Ok(json!({ "changed": changed }))
    })
}

fn export_3d(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "assets.export3D";
    let d = &s.doc()?.doc;
    let ids = if p.get("assets").is_some_and(|v| !v.is_null()) {
        three_d_asset_ids(d, p, "assets", C)?
    } else {
        d.assets.iter().filter(|a| a.format_3d.is_some()).map(|a| a.id).collect()
    };
    if ids.is_empty() {
        return Err(bad(C, "no 3D assets yet: collect an object with a 3D effect first (assets.add3D)"));
    }
    let folder = text_param(p, "folder", C)?.filter(|f| !f.is_empty());
    let zip = switch_or(p, "zip", false, C)?;
    // Every file's name, compared without case: a later file that would take one gets -2, -3…
    let mut taken: HashSet<String> = HashSet::new();
    let mut files: Vec<(String, Vec<u8>)> = vec![];
    for id in ids {
        let Some(a) = d.asset(id) else { continue };
        let format = a.format_3d.as_deref().unwrap_or(DEFAULT_FORMAT_3D);
        let nodes =
            a.nodes.iter().map(|node| d.node(*node).ok_or_else(|| bad(C, format!("no such object {0}", node.0)))).collect::<Result<Vec<_>>>()?;
        // The asset's objects share one budget of triangles.
        let what = &a.name;
        let parts =
            vectorcraft_render::effects::three_d_meshes_of_in(Some(d), nodes).map_err(|e| bad(C, format!("{what} can't be exported in 3D: {e}")))?;
        let base = Some(safe_file_name(&a.name)).filter(|b| !b.is_empty()).unwrap_or_else(|| "Asset".into());
        let mut stem = base.clone();
        let mut k = 1;
        loop {
            let names = mesh_file_names(&stem, format).map_err(|e| bad(C, e))?;
            if names.iter().all(|n| !taken.contains(&n.to_lowercase())) {
                break;
            }
            k += 1;
            stem = format!("{base}-{k}");
        }
        let made = mesh_files(&stem, format, &parts).map_err(|e| bad(C, e))?;
        taken.extend(made.iter().map(|(n, _)| n.to_lowercase()));
        files.extend(made);
    }
    let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
    if zip {
        let archive = super::fileio::zip::store(&files).map_err(|e| bad(C, e))?;
        // "-3D" keeps it apart from the picture assets' ZIP, which is named after the document.
        let name = Some(safe_file_name(&d.title)).filter(|t| !t.is_empty()).map_or_else(|| "3D Assets.zip".into(), |t| format!("{t}-3D.zip"));
        return match folder {
            Some(dir) => {
                create_dir(dir)?;
                let path = format!("{dir}/{name}");
                write_file(&path, &archive)?;
                Ok(json!({ "path": path, "bytes": archive.len(), "files": names }))
            }
            None => Ok(json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(&archive), "bytes": archive.len(), "files": names })),
        };
    }
    match folder {
        Some(dir) => {
            create_dir(dir)?;
            let mut written = vec![];
            for (name, bytes) in &files {
                let path = format!("{dir}/{name}");
                write_file(&path, bytes)?;
                written.push(path);
            }
            Ok(json!({ "files": written }))
        }
        None => {
            let files: Vec<Value> =
                files.iter().map(|(name, bytes)| json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) })).collect();
            Ok(json!({ "files": files }))
        }
    }
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
                "format3D": a.format_3d,
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
    // Create Sub-folders off puts every file in the folder itself, mobile and density rows too.
    if q.get("subfolders").and_then(Value::as_bool) == Some(false) && p.get("folders").is_none() {
        q.insert("folders".into(), json!("none"));
    }
    let d = &s.doc()?.doc;
    // 3D assets are written as mesh files, by assets.export3D: the export refuses one it is given.
    if !q.contains_key("assets") {
        let all: Vec<u64> = d.assets.iter().filter(|a| a.format_3d.is_none()).map(|a| a.id).collect();
        if all.is_empty() {
            return Err(bad(C, "no assets yet: collect art for export first (assets.add)"));
        }
        q.insert("assets".into(), json!(all));
    }
    export_screens(s, &Value::Object(q), p).map_err(|e| of_cmd(C, e))
}

fn settings_set(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "assets.settings.set";
    let o = p.as_object().ok_or_else(|| bad(C, "give the settings to change, e.g. {\"formats\": [{\"format\": \"png\", \"scale\": \"2x\"}]}"))?;
    if let Some(k) = o.keys().find(|k| !SCREEN_SETTINGS_KEYS.contains(&k.as_str()) && k.as_str() != "exportPreset") {
        return Err(bad(C, format!("`{k}` isn't an export setting: {}", SCREEN_SETTINGS_KEYS.join(", "))));
    }
    let mut settings = s.doc()?.doc.export_settings.clone();
    let mut given = o.clone();
    if let Some(name) = given.remove("exportPreset") {
        let name = name.as_str().ok_or_else(|| bad(C, "`exportPreset` is the name of a preset of document.exportPresets.list"))?;
        with_screen_export_preset(s, C, &mut settings, name)?;
    }
    // Create Sub-folders is Export for Screens' folder per row: the dialog opens on the choice.
    match given.get("subfolders").and_then(Value::as_bool) {
        Some(true) => _ = settings.insert("folders".into(), json!("rows")),
        Some(false) if settings.get("folders").and_then(Value::as_str) == Some("rows") => _ = settings.insert("folders".into(), json!("none")),
        _ => {}
    }
    overlay(&mut settings, &given);
    check_screen_settings(s, &Value::Object(settings.clone())).map_err(|e| of_cmd(C, e))?;
    store_screen_settings(s, settings)?;
    Ok(json!({ "settings": shared_settings(s)? }))
}
