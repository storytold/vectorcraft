//! File → Export → Export for Screens (`document.exportForScreens`): the chosen artboards (or the
//! whole document, the chosen assets of the Asset Export panel, layers, or the selection) in every
//! format row at each row's size, into a folder and its sub-folders, or returned for download (each
//! file, or one ZIP); the presets the dialog offers; and the settings the document remembers
//! (`document.exportSettings`), which the Asset Export panel shares ([`SHARED_KEYS`]).
//!
//! A batch export names every file from one pattern (`name`, see [`super::naming`]: the document's
//! and the item's names, a counter, the date), and sorts the files into folders by row, by format,
//! by item, or by item and format (`folders`).

use std::collections::HashSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use vectorcraft_doc::Document;
use vectorcraft_geom::Rect;

use super::super::*;
use super::encode::single_artboard;
use super::export::{isolated, selected_artwork, without_artboards};
use super::naming::{self, NameParts};
use super::{
    ARTBOARD_PARAMS, ArtboardPick, Format, artboard_file_names, create_dir, encode_with_warnings, unique_file_names, with_black_output, writable,
    write_file,
};

const C: &str = "document.exportForScreens";

/// How big a raster row's files are.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScreenSize {
    /// Pixels per point (`2x`).
    Scale(f64),
    /// Pixels per inch, stored in the file (`144ppi`: twice 72, so `2x`).
    Ppi(f64),
    /// The width in pixels; the height follows the artboard's proportions (`100w`).
    Width(f64),
    /// The height in pixels (`100h`).
    Height(f64),
}

impl ScreenSize {
    /// `2x` (or `2`), `100w`, `100h` or `72ppi`, in any case; positive numbers only.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        let num = |t: &str| t.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v > 0.0);
        if let Some(n) = s.strip_suffix("ppi") {
            return num(n).map(Self::Ppi);
        }
        if let Some(n) = s.strip_suffix('w') {
            return num(n).map(Self::Width);
        }
        if let Some(n) = s.strip_suffix('h') {
            return num(n).map(Self::Height);
        }
        num(s.strip_suffix('x').unwrap_or(&s)).map(Self::Scale)
    }

    /// The size a format row asks for: `width`, else `height`, else `ppi`, else `scale` (a number,
    /// or text such as `"2x"`, `"100w"`, `"72ppi"`); 1x when it names none.
    fn of_row(row: &Value) -> Result<Self> {
        let positive = |key: &str, make: fn(f64) -> Self| -> Option<Result<Self>> {
            let v = row.get(key).filter(|v| !v.is_null())?;
            Some(
                v.as_f64().filter(|n| n.is_finite() && *n > 0.0).map(make).ok_or_else(|| bad(C, format!("{key} must be a positive number, not {v}"))),
            )
        };
        if let Some(r) = positive("width", Self::Width).or_else(|| positive("height", Self::Height)).or_else(|| positive("ppi", Self::Ppi)) {
            return r;
        }
        match row.get("scale") {
            None | Some(Value::Null) => Ok(Self::Scale(1.0)),
            Some(Value::String(s)) => Self::parse(s)
                .ok_or_else(|| bad(C, format!("scale `{s}`: a factor such as 2x, a width (100w), a height (100h) or a resolution (72ppi)"))),
            Some(_) => positive("scale", Self::Scale).unwrap_or(Ok(Self::Scale(1.0))),
        }
    }

    /// The scale factor, for the sizes that have one of their own.
    fn factor(self) -> Option<f64> {
        match self {
            Self::Scale(s) => Some(s),
            Self::Ppi(p) => Some(p / 72.0),
            Self::Width(_) | Self::Height(_) => None,
        }
    }

    /// Pixels per point for `region`.
    fn scale_for(self, region: Rect) -> Result<f64> {
        let per = |px: f64, pt: f64| (pt > 0.0).then(|| px / pt).ok_or_else(|| bad(C, "an empty artboard has no width or height to scale to"));
        match self {
            Self::Width(w) => per(w, region.width()),
            Self::Height(h) => per(h, region.height()),
            _ => Ok(self.factor().unwrap_or(1.0)),
        }
    }

    /// The sub-folder files of this size go into: `1x`, `2x`, `100w`, `100h` (numbers to 3
    /// decimals: 80ppi is `1.111x`).
    pub fn label(self) -> String {
        let n = |v: f64| (v * 1000.0).round() / 1000.0;
        match self {
            Self::Width(w) => format!("{}w", n(w)),
            Self::Height(h) => format!("{}h", n(h)),
            _ => format!("{}x", n(self.factor().unwrap_or(1.0))),
        }
    }

    /// The file-name suffix by default: none at 1x, else `@2x`, `@100w`, `@100h` (a resolution
    /// names its scale: 144ppi is `@2x`).
    pub fn suffix(self) -> String {
        match self.factor() {
            Some(s) if (s - 1.0).abs() < 1e-9 => String::new(),
            _ => format!("@{}", self.label()),
        }
    }
}

/// A file-name suffix that names a pixel size (`@2x`, `@0.5x`, `@100w`, `@100h`): vector rows
/// leave it out.
fn is_size_suffix(s: &str) -> bool {
    s.strip_prefix('@').is_some_and(|t| t.ends_with(|c: char| c.is_ascii_alphabetic()) && ScreenSize::parse(t).is_some())
}

/// The presets of the dialog's format rows: (id, label).
pub const PRESETS: [(&str, &str); 2] = [("mobile", "Mobile 1x/2x/3x"), ("density", "Density buckets (0.75x–4x)")];

/// Screen density buckets: (sub-folder, scale).
const DENSITY: [(&str, f64); 6] = [("ldpi", 0.75), ("mdpi", 1.0), ("hdpi", 1.5), ("xhdpi", 2.0), ("xxhdpi", 3.0), ("xxxhdpi", 4.0)];

/// The format rows of preset `id` (see [`PRESETS`]): PNG at 1x, 2x and 3x (`@2x`, `@3x`), or PNG
/// at every density bucket, each in its bucket's sub-folder under the same name.
pub fn preset_rows(id: &str) -> Option<Vec<Value>> {
    let png = |s: f64| ScreenSize::Scale(s);
    match id {
        "mobile" => Some([1.0, 2.0, 3.0].iter().map(|s| json!({"format": "png", "scale": png(*s).label(), "suffix": png(*s).suffix()})).collect()),
        "density" => {
            Some(DENSITY.iter().map(|(folder, s)| json!({"format": "png", "scale": png(*s).label(), "suffix": "", "folder": folder})).collect())
        }
        _ => None,
    }
}

/// A name safe as one file or folder name: letters, digits, `-`, `_` and `.` kept, anything else
/// `-` (`None` when nothing but dots is left: `.` and `..` name other folders).
fn safe_name(s: &str) -> Option<String> {
    let n: String = s.trim().chars().map(|c| if c.is_alphanumeric() || "-_.".contains(c) { c } else { '-' }).collect();
    (!n.trim_matches('.').is_empty()).then_some(n)
}

/// Text that goes into a file name (prefix, suffix, a data set's name in Batch): path separators and
/// `:` become `-`, so the name cannot make a subfolder or point outside the folder the file is written to.
pub(super) fn file_part(s: &str) -> String {
    s.chars().map(|c| if matches!(c, '/' | '\\' | ':') { '-' } else { c }).collect()
}

/// Keys of a format row that aren't encoder options.
const ROW_KEYS: [&str; 7] = ["format", "scale", "ppi", "width", "height", "suffix", "folder"];

/// One format row: what to write, at which size, with which suffix, into which sub-folder.
struct Row {
    format: &'static Format,
    size: ScreenSize,
    suffix: String,
    /// The sub-folder with Create Sub-folders: the row's `folder`, else its size (raster) or format.
    folder: String,
    /// The encoder options: the format's settings, then the row's own.
    options: Value,
    /// What makes two rows the same file: format, size, suffix and options.
    key: String,
}

impl Row {
    fn parse(s: &Session, row: &Value, settings: &Value) -> Result<Self> {
        if !row.is_object() {
            return Err(bad(C, "each format is an object {format, scale?, suffix?}"));
        }
        let format = writable(C, Some(str_param(row, "format").unwrap_or("png")), None)?;
        if matches!(format.id, "template" | "txt" | "dxf" | "emf" | "wmf") {
            return Err(bad(C, "Export for Screens writes png, png8, jpg, webp, gif, tiff, bmp, tga, psd, svg, svgz, pdf, eps, ai or vectorcraft"));
        }
        // Vector formats have no pixel size: it doesn't apply and adds no @2x suffix (not even one
        // left over from a raster row switched to SVG or PDF).
        let size = if format.raster { ScreenSize::of_row(row)? } else { ScreenSize::Scale(1.0) };
        let suffix = match str_param(row, "suffix") {
            Some(s) if !format.raster && is_size_suffix(s) => String::new(),
            Some(s) => file_part(s),
            None => size.suffix(),
        };
        let folder = match str_param(row, "folder").and_then(safe_name) {
            Some(f) => f,
            None if format.raster => size.label(),
            None => format.id.to_uppercase(),
        };
        // The format's settings, without anything that picks artboards or sizes (rows do that).
        let mut options = Map::new();
        if let Some(o) = settings.get(format.id).and_then(Value::as_object) {
            options.extend(
                o.iter().filter(|(k, _)| !ROW_KEYS.contains(&k.as_str()) && k.as_str() != "useArtboards").map(|(k, v)| (k.clone(), v.clone())),
            );
        }
        if let Some(o) = without_artboards(row).as_object() {
            options.extend(o.iter().filter(|(k, _)| !ROW_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())));
        }
        for k in ARTBOARD_PARAMS {
            options.remove(k);
        }
        // A file per artboard is for viewing: it doesn't carry the whole document to edit.
        options.entry("preserveEditing").or_insert(json!(false));
        let options = Value::Object(options);
        // A PDF preset (built-in or saved) becomes its settings.
        let options = if format.id == "pdf" { super::pdf::expand_preset(s, C, &options)?.into_owned() } else { options };
        // Two rows that would write the same bytes under the same name are one file.
        let key = format!("{}|{}|{suffix}|{options}", format.id, size.label());
        Ok(Self { format, size, suffix, folder, options, key })
    }

    /// Formats that hold a whole document to edit (native, Illustrator, EPS): written from a
    /// document of the item alone, one artboard, rather than with an `artboard` option.
    fn whole_document(&self) -> bool {
        matches!(self.format.id, "vectorcraft" | "ai" | "eps")
    }

    /// The encoder options for `region`, `artboard` of the document written.
    fn options_for(&self, region: Rect, artboard: Option<usize>) -> Result<Value> {
        let mut o = self.options.clone();
        if let Some(b) = artboard.filter(|_| !self.whole_document()) {
            o["artboard"] = json!(b);
        }
        if self.format.raster {
            match self.size {
                // The resolution is stored in the file too.
                ScreenSize::Ppi(ppi) => o["ppi"] = json!(ppi),
                size => {
                    o["scale"] = json!(size.scale_for(region)?);
                    if let Some(m) = o.as_object_mut() {
                        m.remove("ppi");
                    }
                }
            }
        }
        Ok(o)
    }
}

/// The settings the Asset Export panel shares with the dialog: what is exported, not which art
/// or where to.
pub(crate) const SHARED_KEYS: [&str; 6] = ["formats", "preset", "settings", "prefix", "subfolders", "antiAlias"];

/// What `document.exportForScreens` reads besides the formats.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ScreenParams {
    #[serde(flatten)]
    boards: ArtboardPick,
    folder: Option<String>,
    zip: bool,
    full_document: bool,
    include_bleed: bool,
    subfolders: bool,
    preset: Option<String>,
    prefix: Option<String>,
    /// Asset ids (Asset Export) to write instead of artboards.
    assets: Option<Vec<u64>>,
    /// Layer ids to write instead of artboards: each layer's art alone.
    layers: Option<Vec<u64>>,
    /// The selection instead of artboards: `one` file of it all, or one for `each` object.
    selection: Option<String>,
    /// The file name pattern ([`naming`]); without it, `prefix` + name + suffix.
    name: Option<String>,
    /// The counter's first value (`####` in `name`).
    counter_start: Option<u64>,
    /// The counter counts `item`s (every file of an artboard has its number) or `file`s.
    counter_per: Option<String>,
    /// The date the pattern's date tokens use (`YYYY-MM-DD`); default today (UTC).
    date: Option<String>,
    /// Where files go: `none` (all in the folder), `rows` (the rows' sub-folders, as
    /// `subfolders`), `format` (a folder per extension), `item`, or `itemFormat`.
    folders: Option<String>,
}

/// How exported files are sorted into folders.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Folders {
    None,
    Rows,
    Format,
    Item,
    ItemFormat,
}

impl Folders {
    fn of(o: &ScreenParams) -> Result<Self> {
        Ok(match o.folders.as_deref() {
            None if o.subfolders || o.preset.is_some() => Self::Rows,
            None | Some("none" | "") => Self::None,
            Some("rows") => Self::Rows,
            Some("format") => Self::Format,
            Some("item") => Self::Item,
            Some("itemFormat") => Self::ItemFormat,
            Some(f) => return Err(bad(C, format!("folders `{f}`: none, rows, format, item or itemFormat"))),
        })
    }
}

/// One thing a batch export writes a file of per row: an artboard, an asset, a layer, the
/// selection or one selected object, or the whole document.
struct Item {
    /// Its name as the user gave it (`<name>`).
    name: String,
    /// Its name made safe and unique among the items (the default file name, the item folder).
    stem: String,
    /// The document written: the whole document with `artboard`, or the item alone.
    doc: Arc<Document>,
    artboard: Option<usize>,
    region: Rect,
    /// Full Document: what a PDF row writes (every artboard of the document).
    every_artboard: Option<Arc<Document>>,
}

impl ScreenParams {
    fn of(p: &Value) -> Result<Self> {
        if p.is_object() { Self::deserialize(p).map_err(|e| bad(C, e.to_string())) } else { Ok(Self::default()) }
    }
}

/// The format rows `p` asks for (`formats`, else `preset`'s, else one PNG at 1x), each with the
/// format's `settings` and the top-level `antiAlias`.
fn rows_of(s: &Session, p: &Value, preset: Option<&str>) -> Result<Vec<Row>> {
    let settings = p.get("settings").cloned().unwrap_or(Value::Null);
    let rows: Vec<Value> = match (preset, p.get("formats")) {
        (Some(_), Some(f)) if !f.is_null() => return Err(bad(C, "give formats or a preset, not both")),
        (Some(id), _) => preset_rows(id).ok_or_else(|| bad(C, format!("preset `{id}`: mobile or density")))?,
        (None, Some(Value::Array(rows))) => rows.clone(),
        (None, None | Some(Value::Null)) => vec![json!({})],
        (None, Some(v)) => return Err(bad(C, format!("formats must be an array of rows, not {v}"))),
    };
    let mut rows: Vec<Row> = rows.iter().map(|r| Row::parse(s, r, &settings)).collect::<Result<_>>()?;
    if rows.is_empty() {
        return Err(bad(C, "no formats to export"));
    }
    // A top-level anti-aliasing mode applies to every row that doesn't name its own.
    if let Some(aa) = p.get("antiAlias") {
        for r in &mut rows {
            if let Some(m) = r.options.as_object_mut() {
                m.entry("antiAlias").or_insert_with(|| aa.clone());
            }
        }
    }
    Ok(rows)
}

/// Check the export settings in `p` (formats, preset, settings…) as an export would read them.
pub(crate) fn check_settings(s: &Session, p: &Value) -> Result<()> {
    rows_of(s, p, ScreenParams::of(p)?.preset.as_deref()).map(|_| ())
}

/// The file names (and bytes) an export wrote, or the files it returns.
#[derive(Default)]
struct Output {
    /// Lower-case relative names already used, with what wrote each (two identical rows write
    /// one file; different files that would share a name get `-2`, `-3`…).
    taken: std::collections::HashMap<String, String>,
    /// Sub-folders already made.
    made: HashSet<String>,
    /// `(relative name, bytes)` kept to return or zip (empty when written to a folder).
    kept: Vec<(String, Vec<u8>)>,
    /// The paths written.
    written: Vec<String>,
}

impl Output {
    /// The name the file `key` writes as `name` takes: `name`, else (with `number`) `name-2`,
    /// `name-3`… when another file has it; `None` when the same file (`key`) was written as `name`
    /// already, or, without `number`, when any file was (the first row wins, as Export for Screens
    /// always did when no name pattern is given).
    fn claim(&mut self, name: &str, key: &str, number: bool) -> Option<String> {
        let (base, ext) = name.rsplit_once('.').map_or((name, ""), |(b, e)| (b, e));
        for n in 1.. {
            let candidate = if n == 1 {
                name.to_string()
            } else if ext.is_empty() {
                format!("{base}-{n}")
            } else {
                format!("{base}-{n}.{ext}")
            };
            match self.taken.get(&candidate.to_lowercase()) {
                Some(k) if k == key || !number => return None,
                Some(_) => continue,
                None => {
                    self.taken.insert(candidate.to_lowercase(), key.to_string());
                    return Some(candidate);
                }
            }
        }
        None
    }

    /// Write `bytes` as `name` (relative) under `folder`, or keep them.
    fn put(&mut self, folder: Option<&str>, keep: bool, name: String, bytes: Vec<u8>) -> Result<()> {
        match folder {
            Some(dir) if !keep => {
                if let Some((sub, _)) = name.rsplit_once('/')
                    && self.made.insert(sub.to_lowercase())
                {
                    create_dir(&format!("{dir}/{sub}"))?;
                }
                let path = format!("{dir}/{name}");
                write_file(&path, &bytes)?;
                self.written.push(path);
            }
            _ => self.kept.push((name, bytes)),
        }
        Ok(())
    }
}

/// File → Export for Screens: [`export`] (over the settings of `exportPreset`, when given), with
/// the params remembered in the document (`document.exportSettings`).
pub(super) fn export_for_screens(s: &mut Session, p: &Value) -> Result<Value> {
    let p = with_preset(s, p)?;
    let r = export(s, &p)?;
    remember(s, &p);
    Ok(r)
}

// ---------- export presets ----------

/// A saved batch export: what to write and how to name and sort it, to use again
/// (`document.exportPresets.*`, `exportPreset`). Kept with the preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScreenExportPreset {
    pub name: String,
    /// The [`PRESET_KEYS`] of `document.exportForScreens` params.
    pub settings: Map<String, Value>,
}

/// What a preset keeps: the formats, their settings, the naming and the folders; not which art
/// (it changes with the document) nor where to (unless given).
pub const PRESET_KEYS: [&str; 13] = [
    "formats",
    "settings",
    "preset",
    "name",
    "prefix",
    "counterStart",
    "counterPer",
    "folders",
    "subfolders",
    "includeBleed",
    "antiAlias",
    "folder",
    "fullDocument",
];

/// The built-in export presets: (name, settings).
pub fn builtin_presets() -> Vec<ScreenExportPreset> {
    let marks = json!({"trim": true, "registration": true, "colorBars": true, "pageInfo": true});
    let preset = |name: &str, settings: Value| ScreenExportPreset { name: name.into(), settings: settings.as_object().cloned().unwrap_or_default() };
    vec![
        preset(
            "Logo Package",
            json!({
                "name": "<filename>-<name>",
                "folders": "format",
                "formats": [
                    {"format": "svg"},
                    {"format": "png", "scale": "1x", "suffix": ""},
                    {"format": "png", "scale": "2x", "suffix": "@2x"},
                    {"format": "pdf"},
                    {"format": "pdf", "suffix": "-print", "marks": marks},
                ],
            }),
        ),
        preset("Print-Ready PDF", json!({"formats": [{"format": "pdf", "suffix": "-print", "marks": marks, "bleed": {"useDocument": true}}]})),
    ]
}

/// The preset called `name`: a saved one, else a built-in one.
fn find_preset(s: &Session, name: &str) -> Option<ScreenExportPreset> {
    s.prefs.screen_export_presets.iter().find(|p| p.name == name).cloned().or_else(|| builtin_presets().into_iter().find(|p| p.name == name))
}

/// `p` over the settings of its `exportPreset`, when it names one (its own keys win).
fn with_preset(s: &Session, p: &Value) -> Result<Value> {
    let Some(name) = str_param(p, "exportPreset") else { return Ok(p.clone()) };
    let preset = find_preset(s, name).ok_or_else(|| bad(C, format!("no export preset `{name}` (see document.exportPresets.list)")))?;
    let mut merged = preset.settings;
    if let Some(o) = p.as_object() {
        merged.extend(o.iter().filter(|(k, _)| k.as_str() != "exportPreset").map(|(k, v)| (k.clone(), v.clone())));
    }
    Ok(Value::Object(merged))
}

pub(super) fn presets_list(s: &mut Session, _: &Value) -> Result<Value> {
    let row = |p: &ScreenExportPreset, built_in: bool| json!({"name": p.name, "builtIn": built_in, "settings": p.settings});
    let rows: Vec<Value> =
        builtin_presets().iter().map(|p| row(p, true)).chain(s.prefs.screen_export_presets.iter().map(|p| row(p, false))).collect();
    Ok(json!({ "presets": rows }))
}

pub(super) fn presets_save(s: &mut Session, p: &Value) -> Result<Value> {
    const SAVE: &str = "document.exportPresets.save";
    let name = str_param(p, "name").map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(SAVE, "give the preset a name"))?.to_string();
    if builtin_presets().iter().any(|b| b.name == name) {
        return Err(bad(SAVE, format!("`{name}` is a built-in preset: save under another name")));
    }
    let given = p.get("settings").and_then(Value::as_object).ok_or_else(|| bad(SAVE, "settings is an object of document.exportForScreens params"))?;
    let settings: Map<String, Value> = given.iter().filter(|(k, _)| PRESET_KEYS.contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
    // Refuse what an export would refuse, now rather than when the preset is used.
    check_settings(s, &Value::Object(settings.clone())).map_err(|e| bad(SAVE, e.to_string()))?;
    Folders::of(&ScreenParams::of(&Value::Object(settings.clone()))?).map_err(|e| bad(SAVE, e.to_string()))?;
    let preset = ScreenExportPreset { name: name.clone(), settings };
    let created = match s.prefs.screen_export_presets.iter_mut().find(|q| q.name == name) {
        Some(slot) => {
            *slot = preset.clone();
            false
        }
        None => {
            s.prefs.screen_export_presets.push(preset.clone());
            true
        }
    };
    Ok(json!({ "name": name, "created": created, "settings": preset.settings }))
}

pub(super) fn presets_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const DELETE: &str = "document.exportPresets.delete";
    let name = str_param(p, "name").ok_or_else(|| bad(DELETE, "missing `name`"))?;
    if builtin_presets().iter().any(|b| b.name == name) {
        return Err(bad(DELETE, format!("`{name}` is built in and stays")));
    }
    let before = s.prefs.screen_export_presets.len();
    s.prefs.screen_export_presets.retain(|q| q.name != name);
    if s.prefs.screen_export_presets.len() == before {
        return Err(bad(DELETE, format!("no saved export preset `{name}`")));
    }
    Ok(json!({ "deleted": name }))
}

/// Every chosen artboard (asset, layer, selected object) in every format row, one file each (a PDF
/// holds its artboard alone), or with `fullDocument` one file per row (a PDF of every artboard,
/// else the bounds of all art). Items that share a name get `-2`, `-3`… instead of overwriting, and
/// so do different files the `name` pattern gives one name. The reply's `warnings` lists every
/// warning the encoders give for the files, each once.
pub(crate) fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let o = ScreenParams::of(p)?;
    let rows = rows_of(s, p, o.preset.as_deref())?;
    let sources = [o.full_document, o.assets.is_some(), o.layers.is_some(), o.selection.is_some()].iter().filter(|b| **b).count();
    if sources > 1 {
        return Err(bad(C, "give one of assets, layers, selection or fullDocument"));
    }
    let folders = Folders::of(&o)?;
    // A prefix names files, not folders.
    let prefix = file_part(o.prefix.as_deref().unwrap_or(""));
    let folder = o.folder.as_deref().filter(|f| !f.is_empty());
    let when = match o.date.as_deref() {
        Some(d) => naming::parse_date(d).ok_or_else(|| bad(C, format!("date `{d}`: YYYY-MM-DD")))?,
        None => vectorcraft_doc::metadata::now_unix().map_or([1970, 1, 1, 0, 0, 0], vectorcraft_doc::metadata::civil),
    };
    let per_file = match o.counter_per.as_deref() {
        None | Some("item") => false,
        Some("file") => true,
        Some(c) => return Err(bad(C, format!("counterPer `{c}`: item or file"))),
    };
    let counter_start = o.counter_start.unwrap_or(1);
    let selection = match o.selection.as_deref() {
        None => None,
        Some("one") => Some(false),
        Some("each") => Some(true),
        Some(m) => return Err(bad(C, format!("selection `{m}`: one or each"))),
    };
    let selected = if selection.is_some() { selected_artwork(s)? } else { vec![] };
    if let Some(dir) = folder {
        create_dir(dir)?;
    }
    let doc = s.doc()?.doc.clone();
    let filename = safe_name(&super::file_stem(&doc.title)).unwrap_or_else(|| "Untitled".into());
    // Include Bleed: each artboard grown by the document's bleed.
    let doc = if o.include_bleed && doc.setup.has_bleed() {
        let mut d = (*doc).clone();
        for a in &mut d.artboards {
            a.rect = doc.setup.bleed_rect(a.rect);
        }
        Arc::new(d)
    } else {
        doc
    };
    let crop = rows.iter().any(|r| r.format.id != "pdf");
    let items = items(&doc, &o, selection, &selected, &filename, crop)?;
    let mut out = Output::default();
    // The encoders' warnings for every file, each once.
    let mut warnings: Vec<String> = Vec::new();
    // Appearance of Black › Printing / Exporting for RGB raster rows.
    let mut write = |doc: &Document, format: &str, options: &Value| -> Result<Vec<u8>> {
        let (bytes, notes) = encode_with_warnings(doc, format, &with_black_output(s, options))?;
        for w in notes {
            if !warnings.contains(&w) {
                warnings.push(w);
            }
        }
        Ok(bytes)
    };
    // A whole-document copy of an artboard, for the formats that write documents to edit.
    let mut board_docs: std::collections::HashMap<usize, Arc<Document>> = std::collections::HashMap::new();
    let mut files = 0u64;
    for (index, item) in items.iter().enumerate() {
        for r in &rows {
            let counter = counter_start.saturating_add(if per_file { files } else { index as u64 });
            let ext = r.format.extensions.first().copied().unwrap_or(r.format.id);
            let stem = match o.name.as_deref().filter(|n| !n.trim().is_empty()) {
                Some(pattern) => {
                    let parts = NameParts {
                        filename: &filename,
                        name: &item.name,
                        index: index + 1,
                        counter,
                        format: ext,
                        scale: &if r.format.raster { r.size.label() } else { String::new() },
                        suffix: &r.suffix,
                        when,
                    };
                    let named = naming::render(pattern, &parts);
                    let named = if named.is_empty() { item.stem.clone() } else { named };
                    if naming::has_suffix_token(pattern) { format!("{prefix}{named}") } else { format!("{prefix}{named}{}", r.suffix) }
                }
                None => format!("{prefix}{}{}", item.stem, r.suffix),
            };
            let file = format!("{stem}.{ext}");
            let sub = match folders {
                Folders::None => None,
                Folders::Rows => Some(r.folder.clone()),
                Folders::Format => Some(ext.to_string()),
                Folders::Item => Some(item.stem.clone()),
                Folders::ItemFormat => Some(format!("{}/{ext}", item.stem)),
            };
            let name = sub.map_or_else(|| file.clone(), |d| format!("{d}/{file}"));
            let Some(name) = out.claim(&name, &format!("{}|{}", item.stem, r.key), o.name.is_some()) else { continue };
            let bytes = if let (Some(all), "pdf") = (&item.every_artboard, r.format.id) {
                // One page per artboard.
                let mut options = r.options.clone();
                options["range"] = json!("all");
                write(all, "pdf", &options)?
            } else if r.whole_document() && item.artboard.is_some() {
                let b = item.artboard.unwrap_or(0);
                let d = board_docs.entry(b).or_insert_with(|| Arc::new(single_artboard(&item.doc, item.region, &item.name))).clone();
                write(&d, r.format.id, &r.options_for(item.region, None)?)?
            } else {
                write(&item.doc, r.format.id, &r.options_for(item.region, item.artboard)?)?
            };
            out.put(folder, o.zip, name, bytes)?;
            files += 1;
        }
    }
    let names: Vec<&str> = out.kept.iter().map(|(n, _)| n.as_str()).collect();
    if o.zip {
        let zip = super::zip::store(&out.kept).map_err(|e| bad(C, e))?;
        let zip_name = format!("{prefix}{filename}.zip");
        return match folder {
            Some(dir) => {
                let path = format!("{dir}/{zip_name}");
                write_file(&path, &zip)?;
                Ok(json!({ "path": path, "bytes": zip.len(), "files": names, "warnings": warnings }))
            }
            None => Ok(
                json!({ "name": zip_name, "dataBase64": vectorcraft_format::base64_encode(&zip), "bytes": zip.len(), "files": names, "warnings": warnings }),
            ),
        };
    }
    Ok(match folder {
        Some(_) => json!({ "files": out.written, "warnings": warnings }),
        None => {
            let files: Vec<Value> =
                out.kept.iter().map(|(name, bytes)| json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) })).collect();
            json!({ "files": files, "warnings": warnings })
        }
    })
}

/// What `o` exports a file of per row ([`Item`]): the whole document, the assets, the layers, the
/// selection (one item, or one per object), else the chosen artboards.
/// `crop`: some row needs the Full Document's art bounds (a format other than PDF).
fn items(doc: &Arc<Document>, o: &ScreenParams, selection: Option<bool>, selected: &[NodeId], filename: &str, crop: bool) -> Result<Vec<Item>> {
    // The art of `ids` alone, as an item named `name`.
    let alone = |ids: &[NodeId], name: &str| -> Option<(Arc<Document>, Rect)> {
        isolated(doc, &doc.paint_order(ids.iter().copied()), name).map(|(d, r)| (Arc::new(d), r))
    };
    let named = |pieces: Vec<(String, Arc<Document>, Option<usize>, Rect)>, fallback: &str| -> Vec<Item> {
        let stems = unique_file_names(pieces.iter().map(|(n, ..)| (n.as_str(), fallback.to_string())));
        pieces
            .into_iter()
            .zip(stems)
            .map(|((name, doc, artboard, region), stem)| Item { name, stem, doc, artboard, region, every_artboard: None })
            .collect()
    };
    if o.full_document {
        // The bounds of all visible art, as the one artboard of the files that aren't PDFs (a PDF
        // row writes every artboard of the document itself).
        let (whole, region) = if crop {
            let bounds = vectorcraft_render::encode::art_bounds(doc).ok_or_else(|| bad(C, "nothing to export: the document has no visible art"))?;
            (Arc::new(single_artboard(doc, bounds, "Document")), bounds)
        } else {
            (doc.clone(), Rect::ZERO)
        };
        let name = filename.to_string();
        return Ok(vec![Item { name: name.clone(), stem: name, doc: whole, artboard: Some(0), region, every_artboard: Some(doc.clone()) }]);
    }
    if let Some(ids) = &o.assets {
        let mut pieces = vec![];
        for id in ids {
            let a = doc.asset(*id).ok_or_else(|| bad(C, format!("no asset {id} (see assets.list)")))?;
            let (d, r) = alone(&a.nodes, &a.name).ok_or_else(|| bad(C, format!("asset `{}` has no art to export", a.name)))?;
            pieces.push((a.name.clone(), d, Some(0), r));
        }
        if pieces.is_empty() {
            return Err(bad(C, "no assets to export"));
        }
        return Ok(named(pieces, "Asset"));
    }
    if let Some(ids) = &o.layers {
        let mut pieces = vec![];
        for id in ids {
            let layer = doc.node(NodeId(*id)).filter(|n| n.is_layer()).ok_or_else(|| bad(C, format!("no layer {id} (see layer.list)")))?;
            let mut art = vec![];
            crate::cmd::assets::art_under(doc, layer.id, &mut art);
            let name = layer.display_name();
            let (d, r) = alone(&art, &name).ok_or_else(|| bad(C, format!("layer `{name}` has no art to export")))?;
            pieces.push((name, d, Some(0), r));
        }
        if pieces.is_empty() {
            return Err(bad(C, "no layers to export"));
        }
        return Ok(named(pieces, "Layer"));
    }
    if let Some(each) = selection {
        if selected.is_empty() {
            return Err(bad(C, "select something to export"));
        }
        let mut pieces = vec![];
        if each {
            for id in selected {
                let name = doc.node(*id).map(|n| n.display_name()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Selection".into());
                if let Some((d, r)) = alone(&[*id], &name) {
                    pieces.push((name, d, Some(0), r));
                }
            }
        } else if let Some((d, r)) = alone(selected, "Selection") {
            pieces.push(("Selection".to_string(), d, Some(0), r));
        }
        if pieces.is_empty() {
            return Err(bad(C, "the selection has no art to export"));
        }
        return Ok(named(pieces, "Selection"));
    }
    let n = doc.artboards.len();
    let boards = o.boards.resolve(n).map_err(|e| bad(C, e))?.unwrap_or_else(|| (0..n).collect());
    if boards.is_empty() {
        return Err(bad(C, "the document has no artboard"));
    }
    let stems = artboard_file_names(doc, &boards);
    Ok(boards
        .into_iter()
        .zip(stems)
        .map(|(b, stem)| {
            let a = doc.artboards.get(b);
            let name = a.map(|a| a.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| stem.clone());
            Item { name, stem, doc: doc.clone(), artboard: Some(b), region: a.map_or(Rect::ZERO, |a| a.rect), every_artboard: None }
        })
        .collect())
}

/// Keep the params of a successful export in the document (without `zip`, a delivery detail, and
/// `assets`, a choice of art), so the dialog reopens on them.
fn remember(s: &mut Session, p: &Value) {
    let Some(o) = p.as_object() else { return };
    let mut settings = o.clone();
    settings.remove("zip");
    settings.remove("assets");
    // Only after a successful export, which had a document.
    let _ = store_settings(s, settings);
}

/// Make `settings` the document's export settings. Not an undo step (like the settings of any
/// export); the document has changed only if they differ.
pub(crate) fn store_settings(s: &mut Session, settings: Map<String, Value>) -> Result<()> {
    let st = s.doc_mut()?;
    if st.doc.export_settings != settings {
        Arc::make_mut(&mut st.doc).export_settings = settings;
        st.revision += 1;
    }
    Ok(())
}

/// `document.exportSettings`: the Export for Screens settings the document last exported with.
pub(super) fn export_settings(s: &mut Session, _: &Value) -> Result<Value> {
    Ok(json!({ "settings": s.doc()?.doc.export_settings }))
}
