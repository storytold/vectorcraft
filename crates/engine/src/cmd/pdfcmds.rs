//! Edit → PDF Presets: `pdf.preset.list`, `save`, `delete`, `export` and `import`. The built-in
//! presets are generated in code ([`vectorcraft_pdf::builtin_presets`]) and read-only; the user's
//! live in [`crate::Prefs::pdf_presets`]. Any of their names works as `preset` wherever PDF
//! options are taken (`document.exportPdf`, `document.export`, a `.ai` save…).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_pdf::{PdfPreset, builtin_preset, builtin_presets};

use super::fileio::pdf::{find_preset, resolve};
use super::{CommandSpec, Result, Session, always, bad, bool_or, cmd, str_param, unique_name};

/// The `format` of a PDF presets file, also its extension.
pub const PRESET_FORMAT: &str = "vcpdfpresets";

/// The extensions of PDF presets files (opening one imports it).
pub const PRESET_EXTS: &[&str] = &[PRESET_FORMAT];

/// What `pdf.preset.export` writes and `pdf.preset.import` reads.
#[derive(Serialize, Deserialize)]
pub(crate) struct PresetFile {
    pub(crate) format: String,
    #[serde(default)]
    pub(crate) presets: Vec<PdfPreset>,
}

impl PresetFile {
    pub(crate) fn new(presets: Vec<PdfPreset>) -> Self {
        Self { format: PRESET_FORMAT.into(), presets }
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "pdf.preset.list",
            "PDF Presets",
            [],
            None,
            "{} → {presets: [{name, builtIn, supported (the writer produces its standard), description, settings (as document.pdfSettings)}]} the built-in presets (VectorCraft Default first), then the saved ones; any of these names works as `preset` wherever PDF options are taken",
            always,
            list
        ),
        cmd!(
            query "pdf.preset.save",
            "Save PDF Preset",
            [],
            None,
            "{name?: (default: a new \"PDF Preset N\"), newName?: rename it, description?, preset?: the preset to start from (default: the saved preset `name`, else VectorCraft Default), …options of document.exportPdf} create or change a saved preset (built-in presets are read-only; passwords are never stored) → {name, created, settings}",
            always,
            save
        ),
        cmd!(query "pdf.preset.delete", "Delete PDF Preset", [], None, "{name} delete a saved preset (built-in ones stay) → {deleted: name}", always, delete),
        cmd!(
            query "pdf.preset.export",
            "Export PDF Presets",
            [],
            None,
            "{names?: [preset names, built-in ones too] (default: every saved preset), path?} write the presets as a .vcpdfpresets file (JSON) → {path, count}; without path → {data: the file's text, count}",
            always,
            export
        ),
        cmd!(
            query "pdf.preset.import",
            "Import PDF Presets",
            [],
            None,
            "{path? | data?: file text | dataBase64?, replace?: false (replace saved presets of the same names; else imported ones whose name is taken get a number)} add the presets of a .vcpdfpresets file (as pdf.preset.export writes) to the saved ones → {imported: [names]}",
            always,
            import
        ),
    ]
}

impl Session {
    /// Every PDF preset: the built-in ones, then the saved ones.
    pub fn pdf_presets(&self) -> Vec<PdfPreset> {
        let mut v = builtin_presets();
        v.extend(self.prefs.pdf_presets.iter().cloned());
        v
    }

    /// The first free "PDF Preset N": the name a new preset gets.
    pub fn new_pdf_preset_name(&self) -> String {
        (1..).map(|i| format!("PDF Preset {i}")).find(|n| !self.pdf_preset_taken(n, None)).unwrap_or_default()
    }

    /// The index of the saved PDF preset `name` (any case).
    fn pdf_saved_index(&self, name: &str) -> Option<usize> {
        self.prefs.pdf_presets.iter().position(|q| q.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Whether `name` is taken by a built-in PDF preset or a saved one other than the one at
    /// `except`.
    fn pdf_preset_taken(&self, name: &str, except: Option<usize>) -> bool {
        builtin_preset(name).is_some() || self.pdf_saved_index(name).is_some_and(|i| Some(i) != except)
    }
}

fn row(p: PdfPreset, built_in: bool) -> Value {
    json!({ "name": p.name, "builtIn": built_in, "supported": p.settings.check().is_ok(), "description": p.description, "settings": p.settings })
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let rows = builtin_presets().into_iter().map(|p| row(p, true)).chain(s.prefs.pdf_presets.iter().cloned().map(|p| row(p, false)));
    Ok(json!({ "presets": rows.collect::<Vec<_>>() }))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "pdf.preset.save";
    let name = match str_param(p, "name").map(str::trim) {
        Some("") => return Err(bad(C, "`name` is empty")),
        Some(n) => n.to_string(),
        None => s.new_pdf_preset_name(),
    };
    if builtin_preset(&name).is_some() {
        return Err(bad(C, format!("`{name}` is a built-in preset and is read-only: save it under another name")));
    }
    let at = s.pdf_saved_index(&name);
    let saved = at.and_then(|i| s.prefs.pdf_presets.get(i)).cloned();
    let old_name = saved.as_ref().map(|old| old.name.clone());
    // A saved preset changes from its own settings unless `preset` says where to start.
    let mut q = if p.is_object() { p.clone() } else { json!({}) };
    if let (Some(old), None) = (&saved, str_param(p, "preset")) {
        q["preset"] = json!(old.name);
    }
    let mut settings = resolve(C, &q, &s.prefs.pdf_presets)?;
    settings.clear_passwords();
    let name = match str_param(p, "newName").map(str::trim) {
        Some("") => return Err(bad(C, "`newName` is empty")),
        Some(n) if s.pdf_preset_taken(n, at) => return Err(bad(C, format!("a preset named `{n}` exists"))),
        Some(n) => n.to_string(),
        None => saved.as_ref().map_or(name, |old| old.name.clone()),
    };
    let description = str_param(p, "description").map(str::to_string).or_else(|| saved.map(|old| old.description)).unwrap_or_default();
    let preset = PdfPreset { name: name.clone(), description, settings: settings.clone() };
    s.store_pdf_preset(old_name.as_deref(), &preset);
    match at.and_then(|i| s.prefs.pdf_presets.get_mut(i)) {
        Some(slot) => *slot = preset,
        None => s.prefs.pdf_presets.push(preset),
    }
    Ok(json!({ "name": name, "created": at.is_none(), "settings": settings }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "pdf.preset.delete";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name`"))?;
    if builtin_preset(name).is_some() {
        return Err(bad(C, format!("`{name}` is a built-in preset and stays")));
    }
    let i = s.pdf_saved_index(name).ok_or_else(|| bad(C, format!("no saved PDF preset named `{name}`")))?;
    let deleted = s.prefs.pdf_presets.remove(i).name;
    s.unstore_pdf_preset(&deleted);
    Ok(json!({ "deleted": deleted }))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "pdf.preset.export";
    let presets = match p.get("names").and_then(Value::as_array) {
        Some(names) => {
            let find = |n: &Value| {
                let n = n.as_str().unwrap_or_default();
                find_preset(n, &s.prefs.pdf_presets).ok_or_else(|| bad(C, format!("no PDF preset named `{n}`")))
            };
            names.iter().map(find).collect::<Result<Vec<_>>>()?
        }
        None => s.prefs.pdf_presets.clone(),
    };
    if presets.is_empty() {
        return Err(bad(C, "no saved presets to export (name built-in ones in `names`)"));
    }
    let count = presets.len();
    let text = serde_json::to_string_pretty(&PresetFile { format: PRESET_FORMAT.into(), presets }).map_err(|e| bad(C, e.to_string()))?;
    match str_param(p, "path") {
        Some(path) => {
            super::fileio::write_file(path, text.as_bytes())?;
            Ok(json!({"path": path, "count": count}))
        }
        None => Ok(json!({"data": text, "count": count})),
    }
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "pdf.preset.import";
    let bytes = match (str_param(p, "path"), str_param(p, "data"), str_param(p, "dataBase64")) {
        (Some(path), ..) => super::fileio::read_file(path)?,
        (None, Some(text), _) => text.as_bytes().to_vec(),
        (None, None, Some(b64)) => vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(C, "bad dataBase64"))?,
        _ => return Err(bad(C, "give `path`, `data` or `dataBase64`")),
    };
    let file: PresetFile = serde_json::from_slice(&bytes).map_err(|e| bad(C, format!("not a PDF presets file: {e}")))?;
    if file.format != PRESET_FORMAT {
        return Err(bad(C, format!("not a PDF presets file (format `{}`)", file.format)));
    }
    // Values a hand-edited file may carry are refused as the export options refuse them.
    for q in &file.presets {
        q.settings.check_values().map_err(|e| bad(C, format!("`{}`: {e}", q.name)))?;
    }
    let replace = bool_or(p, "replace", false);
    let mut imported = vec![];
    for mut preset in file.presets {
        preset.settings.clear_passwords();
        let base = Some(preset.name.trim()).filter(|n| !n.is_empty()).unwrap_or("PDF Preset").to_string();
        match s.pdf_saved_index(&base).filter(|_| replace) {
            Some(i) => {
                if let Some(slot) = s.prefs.pdf_presets.get_mut(i) {
                    preset.name = slot.name.clone();
                    imported.push(preset.name.clone());
                    *slot = preset.clone();
                    s.store_pdf_preset(Some(&preset.name), &preset);
                }
            }
            None => {
                preset.name = unique_name(&base, |n| s.pdf_preset_taken(n, None));
                imported.push(preset.name.clone());
                s.store_pdf_preset(None, &preset);
                s.prefs.pdf_presets.push(preset);
            }
        }
    }
    Ok(json!({ "imported": imported }))
}
