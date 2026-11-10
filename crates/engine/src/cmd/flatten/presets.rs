use super::options::FlattenOptions;
use super::{Deserialize, Result, Serialize, Session, Value, bad, bool_or, json, str_param, unique_name};

// ---------- presets ----------

/// A named set of flattener options: a built-in preset or a saved one
/// ([`crate::Prefs::flattener_presets`]).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlattenerPreset {
    pub name: String,
    #[serde(default)]
    pub options: FlattenOptions,
}

/// What `flattener.presets.export` writes and `flattener.presets.import` reads.
#[derive(Serialize, Deserialize)]
struct PresetFile {
    format: String,
    #[serde(default)]
    presets: Vec<FlattenerPreset>,
}

/// The `format` of a presets file, also its extension.
pub const PRESET_FORMAT: &str = "vcflattener";

/// The extensions of presets files (opening one imports it).
pub const PRESET_EXTS: &[&str] = &[PRESET_FORMAT];

pub(super) fn saved_preset<'a>(saved: &'a [FlattenerPreset], name: &str) -> Option<&'a FlattenerPreset> {
    saved.iter().find(|p| p.name.eq_ignore_ascii_case(name.trim()))
}

impl Session {
    /// Every flattener preset: the built-in ones, then the saved ones.
    pub fn flattener_presets(&self) -> Vec<FlattenerPreset> {
        let mut v = FlattenOptions::builtin_presets();
        v.extend(self.prefs.flattener_presets.iter().cloned());
        v
    }

    /// The preset `name` names: a built-in one (by id or display name) or a saved one, any case.
    pub fn flattener_preset(&self, name: &str) -> Option<FlattenerPreset> {
        let key = name.trim();
        let key = FlattenOptions::preset_label(&key.to_ascii_lowercase()).unwrap_or(key);
        self.flattener_presets().into_iter().find(|p| p.name.eq_ignore_ascii_case(key))
    }

    /// The flattener options `p` asks for: a `preset` by name, built-in or saved, adjusted by the
    /// option keys ([`FlattenOptions::from_params_with`]).
    pub fn flatten_options(&self, p: &Value) -> std::result::Result<FlattenOptions, String> {
        FlattenOptions::from_params_with(p, &self.prefs.flattener_presets)
    }

    /// The first free "Flattener Preset N": the name a new preset gets.
    pub fn new_preset_name(&self) -> String {
        (1..).map(|i| format!("Flattener Preset {i}")).find(|n| !self.preset_taken(n, None)).unwrap_or_default()
    }

    /// The index of the saved preset `name`.
    fn saved_index(&self, name: &str) -> Option<usize> {
        self.prefs.flattener_presets.iter().position(|q| q.name.eq_ignore_ascii_case(name.trim()))
    }

    /// Whether `name` is taken by a built-in preset or a saved one other than the one at `except`.
    fn preset_taken(&self, name: &str, except: Option<usize>) -> bool {
        FlattenOptions::preset(name).is_some() || self.saved_index(name).is_some_and(|i| Some(i) != except)
    }
}

pub(super) fn presets_list(s: &mut Session, _: &Value) -> Result<Value> {
    let rows = FlattenOptions::builtin_presets()
        .into_iter()
        .map(|p| (p, true))
        .chain(s.prefs.flattener_presets.iter().cloned().map(|p| (p, false)))
        .map(|(p, builtin)| json!({"name": p.name, "builtIn": builtin, "options": p.options}));
    Ok(json!({ "presets": rows.collect::<Vec<_>>() }))
}

pub(super) fn presets_save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "flattener.presets.save";
    let name = match str_param(p, "name").map(str::trim) {
        Some("") => return Err(bad(C, "`name` is empty")),
        Some(n) => n.to_string(),
        None => s.new_preset_name(),
    };
    if FlattenOptions::preset(&name).is_some() {
        return Err(bad(C, format!("`{name}` is a built-in preset and can't change: save it under another name")));
    }
    let at = s.saved_index(&name);
    // A saved preset changes from its own options unless `preset` says where to start.
    let mut q = p.clone();
    if let (Some(i), Some(o), None) = (at, q.as_object_mut(), str_param(p, "preset")) {
        o.insert("preset".into(), json!(s.prefs.flattener_presets[i].name));
    }
    let options = s.flatten_options(&q).map_err(|m| bad(C, m))?;
    let name = match str_param(p, "newName").map(str::trim) {
        Some("") => return Err(bad(C, "`newName` is empty")),
        Some(n) if s.preset_taken(n, at) => return Err(bad(C, format!("a preset named `{n}` exists"))),
        Some(n) => n.to_string(),
        None => at.map_or(name, |i| s.prefs.flattener_presets[i].name.clone()),
    };
    let preset = FlattenerPreset { name: name.clone(), options: options.clone() };
    match at {
        Some(i) => s.prefs.flattener_presets[i] = preset,
        None => s.prefs.flattener_presets.push(preset),
    }
    Ok(json!({"name": name, "options": options, "created": at.is_none()}))
}

pub(super) fn presets_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "flattener.presets.delete";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name`"))?;
    if FlattenOptions::preset(name).is_some() {
        return Err(bad(C, format!("`{name}` is a built-in preset and stays")));
    }
    let i = s.saved_index(name).ok_or_else(|| bad(C, format!("no saved preset named `{name}`")))?;
    Ok(json!({ "deleted": s.prefs.flattener_presets.remove(i).name }))
}

pub(super) fn presets_export(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "flattener.presets.export";
    let presets = match p.get("names").and_then(Value::as_array) {
        Some(names) => {
            let find = |n: &Value| {
                let n = n.as_str().unwrap_or_default();
                s.flattener_preset(n).ok_or_else(|| bad(C, format!("no preset named `{n}`")))
            };
            names.iter().map(find).collect::<Result<Vec<_>>>()?
        }
        None => s.prefs.flattener_presets.clone(),
    };
    if presets.is_empty() {
        return Err(bad(C, "no saved presets to export (name built-in ones in `names`)"));
    }
    let count = presets.len();
    let text = serde_json::to_string_pretty(&PresetFile { format: PRESET_FORMAT.into(), presets }).map_err(|e| bad(C, e.to_string()))?;
    match str_param(p, "path") {
        Some(path) => {
            crate::cmd::fileio::write_file(path, text.as_bytes())?;
            Ok(json!({"path": path, "count": count}))
        }
        None => Ok(json!({"data": text, "count": count})),
    }
}

pub(super) fn presets_import(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "flattener.presets.import";
    let bytes = match (str_param(p, "path"), str_param(p, "data"), str_param(p, "dataBase64")) {
        (Some(path), ..) => crate::cmd::fileio::read_file(path)?,
        (None, Some(text), _) => text.as_bytes().to_vec(),
        (None, None, Some(b64)) => vectorcraft_format::base64_decode(b64).ok_or_else(|| bad(C, "bad dataBase64"))?,
        _ => return Err(bad(C, "give `path`, `data` or `dataBase64`")),
    };
    let file: PresetFile = serde_json::from_slice(&bytes).map_err(|e| bad(C, format!("not a flattener presets file: {e}")))?;
    if file.format != PRESET_FORMAT {
        return Err(bad(C, format!("not a flattener presets file (format `{}`)", file.format)));
    }
    // Values a hand-edited file may carry are rejected as the commands reject them.
    for q in &file.presets {
        FlattenOptions::from_params(&json!({ "options": q.options })).map_err(|m| bad(C, format!("`{}`: {m}", q.name)))?;
    }
    let replace = bool_or(p, "replace", false);
    let mut imported = vec![];
    for mut preset in file.presets {
        let base = Some(preset.name.trim()).filter(|n| !n.is_empty()).unwrap_or("Flattener Preset").to_string();
        match s.saved_index(&base) {
            Some(i) if replace => {
                preset.name = s.prefs.flattener_presets[i].name.clone();
                imported.push(preset.name.clone());
                s.prefs.flattener_presets[i] = preset;
            }
            _ => {
                preset.name = unique_name(&base, |n| s.preset_taken(n, None));
                imported.push(preset.name.clone());
                s.prefs.flattener_presets.push(preset);
            }
        }
    }
    Ok(json!({ "imported": imported }))
}
