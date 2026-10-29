//! Edit → Print Presets: `print.presets.list`, `save`, `delete`, `export` and `import`. The
//! built-in [Default] (the default print settings) is generated in code and protected; the user's
//! presets live in [`crate::Prefs::print_presets`]. The Print dialog's preset list loads them.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_pdf::PrintSettings;

use super::fileio::pdf::changed;
use super::print::settings_over;
use super::{CommandSpec, EngineError, Result, Session, always, bad, bool_or, cmd, str_param, unique_name};

/// The built-in preset's name.
pub const DEFAULT_PRESET: &str = "[Default]";

/// The `format` of a print presets file, also its extension.
pub const PRESET_FORMAT: &str = "vcprintpresets";

/// The extensions of print presets files (opening one imports it).
pub const PRESET_EXTS: &[&str] = &[PRESET_FORMAT];

/// A named set of print settings: [Default] or a saved one ([`crate::Prefs::print_presets`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrintPreset {
    pub name: String,
    #[serde(default)]
    pub settings: PrintSettings,
}

/// What `print.presets.export` writes and `print.presets.import` reads.
#[derive(Serialize, Deserialize)]
struct PresetFile {
    format: String,
    #[serde(default)]
    presets: Vec<PrintPreset>,
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "print.presets.list",
            "Print Presets",
            [],
            None,
            "{} → {presets: [{name, builtIn, settings (as print.setup's), changed: [{option, value}] (how it differs from [Default])}]} [Default] (the default print settings, protected) first, then the saved presets",
            always,
            list
        ),
        cmd!(
            query "print.presets.save",
            "Save Print Preset",
            [],
            None,
            "{name?: (default: a new \"Print Preset N\"), newName?: rename it, preset?: the preset to start from (default: the saved preset `name`, else [Default]), settings?: {…settings of print.setup} (over it)} create or change a saved preset ([Default] is protected) → {name, created, settings}",
            always,
            save
        ),
        cmd!(query "print.presets.delete", "Delete Print Preset", [], None, "{name} delete a saved preset ([Default] stays) → {deleted: name}", always, delete),
        cmd!(
            query "print.presets.export",
            "Export Print Presets",
            [],
            None,
            "{names?: [preset names, [Default] too] (default: every saved preset), path?} write the presets as a .vcprintpresets file (JSON) → {path, count}; without path → {data: the file's text, count}",
            always,
            export
        ),
        cmd!(
            query "print.presets.import",
            "Import Print Presets",
            [],
            None,
            "{path? | data?: file text | dataBase64?, replace?: false (replace saved presets of the same names; else imported ones whose name is taken get a number)} add the presets of a .vcprintpresets file (as print.presets.export writes) to the saved ones → {imported: [names]}",
            always,
            import
        ),
    ]
}

/// Is `name` the built-in [Default] (also as "Default")?
pub fn is_default(name: &str) -> bool {
    let n = name.trim();
    n.eq_ignore_ascii_case(DEFAULT_PRESET) || n.eq_ignore_ascii_case("Default")
}

/// `[{option, value}]` for every setting of `set` that differs from `base` (`marks.trim`).
pub fn changes(set: &PrintSettings, base: &PrintSettings) -> Vec<Value> {
    let (Ok(v), Ok(base)) = (serde_json::to_value(set), serde_json::to_value(base)) else { return vec![] };
    let mut diff = vec![];
    changed("", &v, &base, &mut diff);
    diff
}

impl Session {
    /// Every print preset: [Default], then the saved ones.
    pub fn print_presets(&self) -> Vec<PrintPreset> {
        let mut v = vec![PrintPreset { name: DEFAULT_PRESET.into(), settings: PrintSettings::default() }];
        v.extend(self.prefs.print_presets.iter().cloned());
        v
    }

    /// The settings of the print preset `name` ([Default] or a saved one, any case).
    pub fn print_preset_settings(&self, name: &str) -> Option<PrintSettings> {
        if is_default(name) {
            return Some(PrintSettings::default());
        }
        self.print_saved_index(name).and_then(|i| self.prefs.print_presets.get(i)).map(|p| p.settings.clone())
    }

    /// The first free "Print Preset N": the name a new preset gets.
    pub fn new_print_preset_name(&self) -> String {
        (1..).map(|i| format!("Print Preset {i}")).find(|n| !self.print_preset_taken(n, None)).unwrap_or_default()
    }

    /// The index of the saved print preset `name` (any case).
    fn print_saved_index(&self, name: &str) -> Option<usize> {
        self.prefs.print_presets.iter().position(|q| q.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Whether `name` is [Default]'s or a saved preset's other than the one at `except`.
    fn print_preset_taken(&self, name: &str, except: Option<usize>) -> bool {
        is_default(name) || self.print_saved_index(name).is_some_and(|i| Some(i) != except)
    }
}

fn row(p: PrintPreset, built_in: bool) -> Value {
    let changed = changes(&p.settings, &PrintSettings::default());
    json!({ "name": p.name, "builtIn": built_in, "settings": p.settings, "changed": changed })
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let rows: Vec<Value> = s.print_presets().into_iter().enumerate().map(|(i, p)| row(p, i == 0)).collect();
    Ok(json!({ "presets": rows }))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "print.presets.save";
    let name = match str_param(p, "name").map(str::trim) {
        Some("") => return Err(bad(C, "`name` is empty")),
        Some(n) => n.to_string(),
        None => s.new_print_preset_name(),
    };
    if is_default(&name) {
        return Err(bad(C, format!("{DEFAULT_PRESET} is protected: save the settings under another name")));
    }
    let at = s.print_saved_index(&name);
    let saved = at.and_then(|i| s.prefs.print_presets.get(i)).cloned();
    // A saved preset changes from its own settings unless `preset` says where to start.
    let base = match str_param(p, "preset") {
        Some(b) => s.print_preset_settings(b).ok_or_else(|| bad(C, format!("no print preset named `{b}` (see print.presets.list)")))?,
        None => saved.as_ref().map(|old| old.settings.clone()).unwrap_or_default(),
    };
    let base = serde_json::to_value(base).map_err(|e| EngineError::Other(e.to_string()))?;
    let settings = settings_over(C, base, p)?;
    let name = match str_param(p, "newName").map(str::trim) {
        Some("") => return Err(bad(C, "`newName` is empty")),
        Some(n) if s.print_preset_taken(n, at) => return Err(bad(C, format!("a preset named `{n}` exists"))),
        Some(n) => n.to_string(),
        None => saved.map_or(name, |old| old.name),
    };
    let preset = PrintPreset { name: name.clone(), settings: settings.clone() };
    match at.and_then(|i| s.prefs.print_presets.get_mut(i)) {
        Some(slot) => *slot = preset,
        None => s.prefs.print_presets.push(preset),
    }
    Ok(json!({ "name": name, "created": at.is_none(), "settings": settings }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "print.presets.delete";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name`"))?;
    if is_default(name) {
        return Err(bad(C, format!("{DEFAULT_PRESET} is protected and stays")));
    }
    let i = s.print_saved_index(name).ok_or_else(|| bad(C, format!("no saved print preset named `{name}`")))?;
    Ok(json!({ "deleted": s.prefs.print_presets.remove(i).name }))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "print.presets.export";
    let presets = match p.get("names").and_then(Value::as_array) {
        Some(names) => {
            let find = |n: &Value| {
                let n = n.as_str().unwrap_or_default();
                let settings = s.print_preset_settings(n).ok_or_else(|| bad(C, format!("no print preset named `{n}`")))?;
                let name = s.print_saved_index(n).and_then(|i| s.prefs.print_presets.get(i)).map_or(DEFAULT_PRESET.to_string(), |q| q.name.clone());
                Ok(PrintPreset { name, settings })
            };
            names.iter().map(find).collect::<Result<Vec<_>>>()?
        }
        None => s.prefs.print_presets.clone(),
    };
    if presets.is_empty() {
        return Err(bad(C, "no saved presets to export (name [Default] in `names` to export it)"));
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
    const C: &str = "print.presets.import";
    let bytes = match (str_param(p, "path"), str_param(p, "data"), str_param(p, "dataBase64")) {
        (Some(path), ..) => super::fileio::read_file(path)?,
        (None, Some(text), _) => text.as_bytes().to_vec(),
        (None, None, Some(b64)) => vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(C, "bad dataBase64"))?,
        _ => return Err(bad(C, "give `path`, `data` or `dataBase64`")),
    };
    let file: PresetFile = serde_json::from_slice(&bytes).map_err(|e| bad(C, format!("not a print presets file: {e}")))?;
    if file.format != PRESET_FORMAT {
        return Err(bad(C, format!("not a print presets file (format `{}`)", file.format)));
    }
    // Values a hand-edited file may carry are refused as the print commands refuse them.
    for q in &file.presets {
        q.settings.check().map_err(|e| bad(C, format!("`{}`: {e}", q.name)))?;
    }
    let replace = bool_or(p, "replace", false);
    let mut imported = vec![];
    for mut preset in file.presets {
        // [Default] comes in as a saved copy ("Default 2"…): the built-in one stays.
        let name = preset.name.trim().trim_start_matches('[').trim_end_matches(']').trim();
        let base = Some(name).filter(|n| !n.is_empty()).unwrap_or("Print Preset").to_string();
        match s.print_saved_index(&base).filter(|_| replace) {
            Some(i) => {
                if let Some(slot) = s.prefs.print_presets.get_mut(i) {
                    preset.name = slot.name.clone();
                    imported.push(preset.name.clone());
                    *slot = preset;
                }
            }
            None => {
                preset.name = unique_name(&base, |n| s.print_preset_taken(n, None));
                imported.push(preset.name.clone());
                s.prefs.print_presets.push(preset);
            }
        }
    }
    Ok(json!({ "imported": imported }))
}
