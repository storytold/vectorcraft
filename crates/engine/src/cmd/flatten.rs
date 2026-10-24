//! Object → Flatten Transparency: transparent art becomes opaque art that looks the same.
//!
//! The targets (`ids` or the selection) split into groups of overlapping objects. Groups without
//! transparency stay as they are (their type and strokes are outlined on request). The others are
//! cut into atomic regions: the faces of the planar arrangement of every fill, outlined stroke and
//! clipping path in them ([`po::regions`]). The same paints cover each whole region, so its colour
//! is exact: those paints composited as the renderer composites them ([`Composer`], blend modes by
//! [`composite`]), over white (or over nothing with Preserve Alpha); in CMYK documents their inks
//! composite, plane by plane ([`cmyk_planes`]), and the colour is CMYK. Each region becomes a path
//! filled with that flat colour. Where gradients, patterns, images, opacity masks or raster
//! effects reach there is no single colour: those regions are rendered into one image per group,
//! clipped to them with Clip Complex Regions (else the image is a rectangle that takes in the
//! regions it overlaps). The raster/vector balance rasterizes whole groups that split into more
//! regions than it allows (all of them at 0). For files without transparency (PDF/X-1a, PDF/X-3)
//! a CMYK document's images are opaque CMYK: its colours keep their inks there too.
//!
//! Edit → Transparency Flattener Presets: the built-in presets plus the user's, saved with the
//! preferences ([`crate::Prefs::flattener_presets`]); a `preset` named anywhere options are taken
//! finds both.
//!
//! Window → Flattener Preview: the same plan, worked out without rendering, reports what flattening
//! the document would touch ([`FlattenReport`], `flattener.preview`).

use std::collections::BTreeSet;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_color::blend::{cmyk_planes, composite, planes_cmyk};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::appearance::{AppearanceItem, FillLayer};
use vectorcraft_doc::cmyk::Inks;
use vectorcraft_doc::{Appearance, ColorMode, Document, Effect, ImageBlob, ImageObject, Knockout, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, FillRule, PathData, Rect, shapes};
use vectorcraft_pathops as po;
use vectorcraft_render::effects;

use super::edit::roots_of;
use super::menucmds::{MAX_PIXELS, isolated_doc, unique_key};
use super::pathops::{node_path, outline_strokes, outline_strokes_under, separate, shape_node};
use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "object.flattenTransparency",
            "Flatten Transparency…",
            ["Object"],
            None,
            "{preset?: \"high\"|\"medium\"|\"low\" or a saved preset's name (see flattener.presets.list; default medium), balance?: 0..100 (raster/vector balance; 0 rasterizes everything), lineArtPpi?: 1..2400, gradientPpi?: 1..2400 (areas only gradients and meshes reach), textToOutlines?, strokesToOutlines?, clipComplexRegions? (clip images to the region outlines, else rectangles), antiAlias?, preserveAlpha? (composite over nothing instead of white), preserveOverprints? (areas showing one paint keep its colour and overprint; false clears overprints), options?: {the same keys}, ids?|id?} overlapping transparent objects become one group of flat-colour regions (CMYK colours composited ink by ink in CMYK documents), plus an image where gradients, patterns, images, masks or raster effects reach; objects without transparency stay; a frame of threaded text that is outlined or flattened leaves its thread with the text it shows → {ids, rasterized: images made, vector: regions made, options, warning? (an image in the art couldn't be decoded, as object.rasterize says)}",
            has_doc,
            flatten
        ),
        cmd!(
            query "flattener.presets.list",
            "Transparency Flattener Presets",
            [],
            None,
            "{} → {presets: [{name, builtIn, options}]} the built-in presets (High, Medium and Low Resolution), then the saved ones; any of these names works as `preset` wherever flattener options are taken",
            always,
            presets_list
        ),
        cmd!(
            query "flattener.presets.save",
            "Save Transparency Flattener Preset",
            [],
            None,
            "{name?: (default: a new \"Flattener Preset N\"), newName?: rename it, preset?: the preset to start from (default: the saved preset `name`, else medium), …options (the keys of object.flattenTransparency, at the top level or in `options`)} create or change a saved preset (built-in ones can't change) → {name, options, created}",
            always,
            presets_save
        ),
        cmd!(
            query "flattener.presets.delete",
            "Delete Transparency Flattener Preset",
            [],
            None,
            "{name} delete a saved preset (built-in ones stay) → {deleted: name}",
            always,
            presets_delete
        ),
        cmd!(
            query "flattener.presets.import",
            "Import Transparency Flattener Presets",
            [],
            None,
            "{path? | data?: file text | dataBase64?, replace?: false (replace saved presets of the same names; else the imported ones get a number)} add the presets of a .vcflattener file (as flattener.presets.export writes) to the saved ones → {imported: [names]}",
            always,
            presets_import
        ),
        cmd!(
            query "flattener.presets.export",
            "Export Transparency Flattener Presets",
            [],
            None,
            "{names?: [preset names, built-in ones too] (default: every saved preset), path?} write the presets as a .vcflattener file (JSON) → {path, count}; without path → {data: the file's text, count}",
            always,
            presets_export
        ),
        cmd!(
            query "flattener.preview",
            "Flattener Preview",
            [],
            None,
            "{highlight?: \"none\"|\"rasterizedRegions\" (areas the raster/vector balance rasterizes whole)|\"transparentObjects\"|\"allAffected\"|\"expandedPatterns\" (pattern art taking part)|\"outlinedStrokes\"|\"outlinedText\"|\"allRasterized\" (default none), overprints?: \"preserve\"|\"simulate\"|\"discard\" (simulate and discard flatten without preserveOverprints), preset?, …options as object.flattenTransparency, ids?|id?: (default: the whole document)} what flattening would do, without changing anything → {highlight, regions: [{bounds: {x, y, width, height}, id?}] (the highlighted objects, or areas), counts: {transparentObjects, allAffected, expandedPatterns, outlinedStrokes, outlinedText, rasterizedRegions, allRasterized, vectorRegions}, options}",
            has_doc,
            preview
        ),
    ]
}

/// Object → Flatten Transparency settings: a preset's, adjusted by the command's params.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FlattenOptions {
    /// Raster/vector balance, 0–100: 0 rasterizes all flattened art; 100 keeps everything vector
    /// that can be; in between, groups that split into more regions than it allows are rasterized.
    pub balance: f64,
    /// Resolution (ppi) of rasterized art.
    pub line_art_ppi: f64,
    /// Resolution (ppi) of rasterized areas that only gradients and meshes reach.
    pub gradient_ppi: f64,
    /// Outline all type, also where it isn't transparent.
    pub text_to_outlines: bool,
    /// Outline all strokes, also where they aren't transparent.
    pub strokes_to_outlines: bool,
    /// Clip rasterized areas to their region outlines (else they are rectangles).
    pub clip_complex_regions: bool,
    /// Smooth the edges of rasterized areas.
    pub anti_alias: bool,
    /// Keep the art's own alpha: composite over nothing instead of over white.
    pub preserve_alpha: bool,
    /// Areas showing a single paint keep it (spot and process colours, swatches) with its
    /// overprint; off, the flattened objects lose their overprints.
    pub preserve_overprints: bool,
    /// Flattening for files without any transparency (PDF/X-1a, PDF/X-3): images with
    /// see-through pixels (these keys, [`see_through_images`]) count as transparency, and
    /// rasterized areas clipped to their regions are opaque (in a CMYK document, images of ink
    /// amounts). `None` for ordinary flattening. Not a preset option.
    #[serde(skip)]
    pub no_transparency: Option<BTreeSet<String>>,
}

impl Default for FlattenOptions {
    fn default() -> Self {
        Self::builtin("medium")
    }
}

impl FlattenOptions {
    /// Preset ids, finest first.
    pub const PRESETS: [&'static str; 3] = ["high", "medium", "low"];

    /// Display name of a preset id.
    pub fn preset_label(id: &str) -> Option<&'static str> {
        Some(match id {
            "high" => "High Resolution",
            "medium" => "Medium Resolution",
            "low" => "Low Resolution",
            _ => return None,
        })
    }

    /// A preset by id or display name, any case.
    pub fn preset(name: &str) -> Option<Self> {
        let key = name.trim().to_ascii_lowercase();
        let id = Self::PRESETS.into_iter().find(|id| key == *id || Self::preset_label(id).is_some_and(|l| l.eq_ignore_ascii_case(&key)))?;
        Some(Self::builtin(id))
    }

    /// The built-in preset `id` (one of [`Self::PRESETS`]; anything else is low).
    fn builtin(id: &str) -> Self {
        let (balance, line_art_ppi, gradient_ppi, clip_complex_regions, anti_alias) = match id {
            "high" => (100.0, 1200.0, 300.0, true, false),
            "medium" => (75.0, 300.0, 300.0, false, true),
            _ => (75.0, 300.0, 150.0, false, true),
        };
        Self {
            balance,
            line_art_ppi,
            gradient_ppi,
            text_to_outlines: false,
            strokes_to_outlines: false,
            clip_complex_regions,
            anti_alias,
            preserve_alpha: false,
            preserve_overprints: true,
            no_transparency: None,
        }
    }

    /// The built-in presets, finest first.
    pub fn builtin_presets() -> Vec<FlattenerPreset> {
        Self::PRESETS
            .into_iter()
            .filter_map(|id| Some(FlattenerPreset { name: Self::preset_label(id)?.into(), options: Self::preset(id)? }))
            .collect()
    }

    /// A built-in preset (by id or display name) or one of `saved` (by name), any case.
    pub fn find_preset(name: &str, saved: &[FlattenerPreset]) -> Option<Self> {
        Self::preset(name).or_else(|| saved_preset(saved, name).map(|p| p.options.clone()))
    }

    /// The options `p` asks for, with the built-in presets only ([`Self::from_params_with`]).
    pub fn from_params(p: &Value) -> std::result::Result<Self, String> {
        Self::from_params_with(p, &[])
    }

    /// The options an export flattens with: preset `name` (built-in or one of `saved`; default
    /// medium) with the export's `flattener` `options` over it.
    pub fn for_export(name: Option<&str>, options: Option<&Value>, saved: &[FlattenerPreset]) -> std::result::Result<Self, String> {
        let mut q = json!({ "options": options.cloned().unwrap_or(Value::Null) });
        if let Some(name) = name {
            q["preset"] = json!(name);
        }
        Self::from_params_with(&q, saved)
    }

    /// The options `p` asks for: `preset` (built-in or one of `saved`; default medium) adjusted by
    /// the option keys given at the top level or in `options`.
    pub fn from_params_with(p: &Value, saved: &[FlattenerPreset]) -> std::result::Result<Self, String> {
        let base = match str_param(p, "preset") {
            Some(name) => Self::find_preset(name, saved)
                .ok_or_else(|| format!("unknown preset `{name}` (high, medium, low or a saved one: see flattener.presets.list)"))?,
            None => Self::default(),
        };
        let mut v = serde_json::to_value(&base).map_err(|e| e.to_string())?;
        for src in [Some(p), p.get("options")].into_iter().flatten().filter_map(Value::as_object) {
            for (k, val) in src {
                if let Some(slot) = v.get_mut(k) {
                    *slot = val.clone();
                }
            }
        }
        let o: Self = serde_json::from_value(v).map_err(|e| e.to_string())?;
        if !(0.0..=100.0).contains(&o.balance) {
            return Err("balance must be between 0 and 100".into());
        }
        if ![o.line_art_ppi, o.gradient_ppi].iter().all(|r| (1.0..=2400.0).contains(r)) {
            return Err("resolutions must be between 1 and 2400 ppi".into());
        }
        Ok(o)
    }

    /// `n` shows an image whose see-through pixels count as transparency.
    fn draws_see_through_image(&self, n: &Node) -> bool {
        self.no_transparency.as_ref().is_some_and(|keys| draws_image(n, keys))
    }

    /// Whether a group that splits into `regions` atomic regions is rasterized whole.
    fn rasterize_all(&self, regions: usize) -> bool {
        self.balance <= 0.0 || (self.balance < 100.0 && regions as f64 > 10f64.powf(self.balance / 20.0))
    }
}

fn flatten(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.flattenTransparency";
    let o = s.flatten_options(p).map_err(|m| bad(C, m))?;
    let roots = target_roots(s, p)?;
    if roots.is_empty() {
        return Err(EngineError::Other("Flatten Transparency: select objects to flatten".into()));
    }
    let src = s.doc()?.doc.clone();
    let failed = vectorcraft_render::decode_failures();
    let plan = plan(&src, &roots, &o, true)?;
    let warning = super::rasterfx::undecoded_warning(failed);
    let options = serde_json::to_value(&o).unwrap_or_default();
    if plan.flat.is_empty() && !plan.kept.iter().any(|id| src.node(*id).is_some_and(|n| kept_work(n, &o))) {
        return Ok(json!({ "ids": roots.iter().map(|i| i.0).collect::<Vec<_>>(), "rasterized": 0, "vector": 0, "options": options }));
    }
    let (ids, vector, rasterized) = s.edit("Flatten Transparency", |d, sel| {
        let threaded = super::threads::frames(d);
        let out = apply(d, sel, plan, &o)?;
        // Frames of threaded text that were outlined or flattened leave their threads with the
        // text they show.
        super::threads::leave_threads(d, &threaded);
        Ok(out)
    })?;
    let mut out = json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "rasterized": rasterized, "vector": vector, "options": options });
    if let Some(w) = warning {
        out["warning"] = json!(w);
    }
    Ok(out)
}

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

fn saved_preset<'a>(saved: &'a [FlattenerPreset], name: &str) -> Option<&'a FlattenerPreset> {
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

fn presets_list(s: &mut Session, _: &Value) -> Result<Value> {
    let rows = FlattenOptions::builtin_presets()
        .into_iter()
        .map(|p| (p, true))
        .chain(s.prefs.flattener_presets.iter().cloned().map(|p| (p, false)))
        .map(|(p, builtin)| json!({"name": p.name, "builtIn": builtin, "options": p.options}));
    Ok(json!({ "presets": rows.collect::<Vec<_>>() }))
}

fn presets_save(s: &mut Session, p: &Value) -> Result<Value> {
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

fn presets_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "flattener.presets.delete";
    let name = str_param(p, "name").ok_or_else(|| bad(C, "missing `name`"))?;
    if FlattenOptions::preset(name).is_some() {
        return Err(bad(C, format!("`{name}` is a built-in preset and stays")));
    }
    let i = s.saved_index(name).ok_or_else(|| bad(C, format!("no saved preset named `{name}`")))?;
    Ok(json!({ "deleted": s.prefs.flattener_presets.remove(i).name }))
}

fn presets_export(s: &mut Session, p: &Value) -> Result<Value> {
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
            super::fileio::write_file(path, text.as_bytes())?;
            Ok(json!({"path": path, "count": count}))
        }
        None => Ok(json!({"data": text, "count": count})),
    }
}

fn presets_import(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "flattener.presets.import";
    let bytes = match (str_param(p, "path"), str_param(p, "data"), str_param(p, "dataBase64")) {
        (Some(path), ..) => super::fileio::read_file(path)?,
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

/// The top-level objects among `ids` (or the selection), in paint order: layers stand for their
/// contents; hidden and locked objects and clipping paths are left alone.
fn target_roots(s: &Session, p: &Value) -> Result<Vec<NodeId>> {
    let d = &s.doc()?.doc;
    let mut ids = vec![];
    for id in targets(s, p)? {
        match d.node(id) {
            Some(n) if n.is_layer() => ids.extend(n.children().into_iter().flatten().map(|c| c.id)),
            Some(_) => ids.push(id),
            None => {}
        }
    }
    let mut keyed: Vec<(Vec<usize>, NodeId)> = ids.into_iter().filter_map(|id| d.index_path(id).map(|p| (p, id))).collect();
    keyed.sort();
    keyed.dedup();
    let roots = roots_of(d, keyed.into_iter().map(|(_, id)| id).collect());
    // A clipping path stays where it clips.
    let clipping = |id: NodeId| {
        d.parent_of(id).and_then(|p| d.node(p)).is_some_and(|p| p.clips() && p.children().and_then(|c| c.first()).is_some_and(|c| c.id == id))
    };
    let editable = d.editable();
    Ok(roots.into_iter().filter(|id| d.is_visible(*id) && editable(*id) && !clipping(*id)).collect())
}

// ---------- preview ----------

/// What the Flattener Preview highlights.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Highlight {
    /// Nothing: the art in colour.
    #[default]
    None,
    /// Areas the raster/vector balance rasterizes whole.
    RasterizedRegions,
    /// Objects with transparency of their own.
    TransparentObjects,
    /// Every object flattening changes.
    AllAffected,
    /// Objects with pattern paint in the flattened art.
    ExpandedPatterns,
    /// Strokes that become filled outlines.
    OutlinedStrokes,
    /// Type that becomes outlines.
    OutlinedText,
    /// Every area that becomes an image.
    AllRasterized,
}

impl Highlight {
    pub const ALL: [Self; 8] = [
        Self::None,
        Self::RasterizedRegions,
        Self::TransparentObjects,
        Self::AllAffected,
        Self::ExpandedPatterns,
        Self::OutlinedStrokes,
        Self::OutlinedText,
        Self::AllRasterized,
    ];

    /// The id `flattener.preview` takes.
    pub fn id(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RasterizedRegions => "rasterizedRegions",
            Self::TransparentObjects => "transparentObjects",
            Self::AllAffected => "allAffected",
            Self::ExpandedPatterns => "expandedPatterns",
            Self::OutlinedStrokes => "outlinedStrokes",
            Self::OutlinedText => "outlinedText",
            Self::AllRasterized => "allRasterized",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None (Color Preview)",
            Self::RasterizedRegions => "Rasterized Complex Regions",
            Self::TransparentObjects => "Transparent Objects",
            Self::AllAffected => "All Affected Objects",
            Self::ExpandedPatterns => "Expanded Patterns",
            Self::OutlinedStrokes => "Outlined Strokes",
            Self::OutlinedText => "Outlined Text",
            Self::AllRasterized => "All Rasterized Regions",
        }
    }

    /// A highlight by id or label, any case.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        Self::ALL.into_iter().find(|h| h.id().eq_ignore_ascii_case(s) || h.label().eq_ignore_ascii_case(s))
    }
}

/// How the Flattener Preview treats overprints: kept as they are, simulated in the preview, or
/// dropped; only Preserve flattens with Preserve Overprints and Spot Colors.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Overprints {
    #[default]
    Preserve,
    Simulate,
    Discard,
}

impl Overprints {
    pub const ALL: [Self; 3] = [Self::Preserve, Self::Simulate, Self::Discard];

    pub fn id(self) -> &'static str {
        match self {
            Self::Preserve => "preserve",
            Self::Simulate => "simulate",
            Self::Discard => "discard",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Preserve => "Preserve",
            Self::Simulate => "Simulate",
            Self::Discard => "Discard",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|o| o.id().eq_ignore_ascii_case(s.trim()) || o.label().eq_ignore_ascii_case(s.trim()))
    }
}

/// What flattening would do (Window → Flattener Preview, `flattener.preview`).
#[derive(Clone, Debug, Default)]
pub struct FlattenReport {
    /// Top-level objects with transparency of their own.
    pub transparent: Vec<NodeId>,
    /// Top-level objects flattening changes: the overlapping groups with transparency, and those
    /// the outline and overprint options change.
    pub affected: Vec<NodeId>,
    /// Objects with pattern paint in the flattened art.
    pub patterns: Vec<NodeId>,
    /// Paths whose strokes become filled outlines.
    pub strokes: Vec<NodeId>,
    /// Type that becomes outlines.
    pub text: Vec<NodeId>,
    /// Areas the raster/vector balance rasterizes whole.
    pub complex: Vec<PathData>,
    /// Every area that becomes an image.
    pub rasterized: Vec<PathData>,
    /// Flat-colour regions made.
    pub vector: usize,
}

impl FlattenReport {
    /// The objects `h` highlights.
    pub fn objects(&self, h: Highlight) -> &[NodeId] {
        match h {
            Highlight::TransparentObjects => &self.transparent,
            Highlight::AllAffected => &self.affected,
            Highlight::ExpandedPatterns => &self.patterns,
            Highlight::OutlinedStrokes => &self.strokes,
            Highlight::OutlinedText => &self.text,
            _ => &[],
        }
    }

    /// The areas `h` highlights.
    pub fn areas(&self, h: Highlight) -> &[PathData] {
        match h {
            Highlight::RasterizedRegions => &self.complex,
            Highlight::AllRasterized => &self.rasterized,
            _ => &[],
        }
    }

    /// How many objects or areas each highlight shows.
    pub fn counts(&self) -> Value {
        json!({
            "transparentObjects": self.transparent.len(),
            "allAffected": self.affected.len(),
            "expandedPatterns": self.patterns.len(),
            "outlinedStrokes": self.strokes.len(),
            "outlinedText": self.text.len(),
            "rasterizedRegions": self.complex.len(),
            "allRasterized": self.rasterized.len(),
            "vectorRegions": self.vector,
        })
    }

    /// The report as `flattener.preview` answers: the highlighted objects (or areas) of `doc` and
    /// the counts.
    pub fn to_json(&self, doc: &Document, h: Highlight) -> Value {
        let rect = |r: Rect| json!({"x": r.x0, "y": r.y0, "width": r.width(), "height": r.height()});
        let objects = self.objects(h).iter().filter_map(|id| Some(json!({"id": id.0, "bounds": rect(doc.node(*id)?.visual_bounds()?)})));
        let areas = self.areas(h).iter().filter_map(|p| Some(json!({ "bounds": rect(p.bounds()?) })));
        json!({"highlight": h.id(), "regions": objects.chain(areas).collect::<Vec<_>>(), "counts": self.counts()})
    }

    /// The art `h` highlights, opaque, alone in a copy of `doc`: the objects and the areas (filled
    /// black). Its coverage is the highlight. `None` when nothing is highlighted.
    pub fn highlight_art(&self, doc: &Document, h: Highlight) -> Option<Document> {
        let (ids, areas) = (self.objects(h), self.areas(h));
        if ids.is_empty() && areas.is_empty() {
            return None;
        }
        let mut tmp = doc.clone();
        let mut art: Vec<Node> = ids.iter().filter_map(|id| doc.node(*id).cloned()).collect();
        art.iter_mut().for_each(opaque);
        for path in areas {
            let mut n = shape_node(&mut tmp, path.clone(), None);
            n.appearance = Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 0.0)), Paint::None, 0.0).into();
            art.push(n);
        }
        Some(isolated_doc(&tmp, art))
    }
}

/// What flattening `roots` of `doc` with `o` would do.
pub fn report(doc: &Document, roots: &[NodeId], o: &FlattenOptions) -> Result<FlattenReport> {
    let plan = plan(doc, roots, o, false)?;
    let mut r = FlattenReport { transparent: plan.transparent, ..Default::default() };
    // Every object under `id` (but a compound path's own pieces), visible ones only.
    fn visit(n: &Node, f: &mut dyn FnMut(&Node)) {
        if !n.visible {
            return;
        }
        f(n);
        if !matches!(n.kind, NodeKind::Compound { .. }) {
            for c in n.children().into_iter().flatten() {
                visit(c, f);
            }
        }
    }
    let pattern = |n: &Node| n.appearance.items.iter().any(|i| i.visible() && matches!(i.paint(), Paint::Pattern { .. }));
    for f in &plan.flat {
        r.affected.extend(&f.roots);
        r.vector += f.regions.len();
        for root in f.roots.iter().filter_map(|id| doc.node(*id)) {
            visit(root, &mut |n| {
                // Flattened art is outlined throughout and its patterns become part of the image.
                if matches!(n.kind, NodeKind::Text(_)) {
                    r.text.push(n.id);
                } else if stroked(n) {
                    r.strokes.push(n.id);
                }
                if pattern(n) {
                    r.patterns.push(n.id);
                }
            });
        }
        if let Some(a) = &f.area {
            if a.whole {
                r.complex.push(a.outline());
            }
            r.rasterized.push(a.outline());
        }
    }
    for root in plan.kept.iter().filter_map(|id| doc.node(*id)).filter(|n| kept_work(n, o)) {
        r.affected.push(root.id);
        visit(root, &mut |n| {
            if o.text_to_outlines && matches!(n.kind, NodeKind::Text(_)) {
                r.text.push(n.id);
            } else if o.strokes_to_outlines && stroked(n) {
                r.strokes.push(n.id);
            }
        });
    }
    r.affected.sort_by_key(|id| doc.index_path(*id));
    Ok(r)
}

impl Session {
    /// What flattening would do with the options `p` asks for (as `flattener.preview`): the
    /// options and the report.
    pub fn flattener_report(&self, p: &Value) -> Result<(FlattenOptions, FlattenReport)> {
        const C: &str = "flattener.preview";
        let mut o = self.flatten_options(p).map_err(|m| bad(C, m))?;
        if let Some(v) = str_param(p, "overprints") {
            let ov = Overprints::parse(v).ok_or_else(|| bad(C, format!("unknown overprints `{v}` (preserve, simulate or discard)")))?;
            o.preserve_overprints = ov == Overprints::Preserve;
        }
        let roots = if p.get("ids").is_some() {
            target_roots(self, p)?
        } else {
            let layers: Vec<u64> = self.doc()?.doc.layers.iter().map(|l| l.id.0).collect();
            target_roots(self, &json!({ "ids": layers }))?
        };
        let r = report(&self.doc()?.doc, &roots, &o)?;
        Ok((o, r))
    }
}

fn preview(s: &mut Session, p: &Value) -> Result<Value> {
    let h = match str_param(p, "highlight") {
        Some(v) => Highlight::parse(v).ok_or_else(|| bad("flattener.preview", format!("unknown highlight `{v}`")))?,
        None => Highlight::None,
    };
    let (o, r) = s.flattener_report(p)?;
    let mut out = r.to_json(&s.doc()?.doc, h);
    out["options"] = serde_json::to_value(&o).unwrap_or_default();
    Ok(out)
}

// ---------- plan ----------

/// What the flattening does, worked out before the document changes.
struct Plan {
    flat: Vec<Flat>,
    /// Objects without transparency (they stay, but for the outline options).
    kept: Vec<NodeId>,
    /// The objects with transparency of their own.
    transparent: Vec<NodeId>,
}

/// One group of overlapping objects with transparency and what replaces it.
struct Flat {
    /// Its objects in paint order; the result takes the place of the last.
    roots: Vec<NodeId>,
    regions: Regions,
    /// Where its image goes, and the image (once rendered).
    area: Option<Area>,
    raster: Option<Raster>,
}

/// Where a group's image goes.
struct Area {
    rect: Rect,
    ppi: f64,
    /// The complex regions it is clipped to (Clip Complex Regions).
    clip: Option<PathData>,
    /// The raster/vector balance rasterizes the whole group (it splits into too many regions).
    whole: bool,
}

impl Area {
    /// The area's outline.
    fn outline(&self) -> PathData {
        self.clip.clone().unwrap_or_else(|| shapes::rectangle(self.rect))
    }
}

/// Flat-colour regions: their outlines and paints.
type Regions = Vec<(PathData, RegionFill)>;

/// A flat-colour region's paint.
struct RegionFill {
    paint: Paint,
    alpha: f32,
    overprint: bool,
}

/// A rendered image of complex regions, clipped to them or not.
struct Raster {
    blob: ImageBlob,
    width: u32,
    height: u32,
    xf: Affine,
    clip: Option<PathData>,
}

/// The flattening of `roots` with `o`; `render` renders the images (the preview only needs where
/// they go). Fails only when an image can't be encoded.
fn plan(src: &Document, roots: &[NodeId], o: &FlattenOptions, render: bool) -> Result<Plan> {
    let tmp = isolated_doc(src, roots.iter().filter_map(|id| src.node(*id).cloned()).collect());
    // Geometry effects first: what is left on the art are raster effects.
    let baked = effects::bake_document(&tmp).unwrap_or(tmp);
    let art: Vec<Node> = baked.layers[0].children().into_iter().flatten().map(|a| (**a).clone()).collect();
    let mut sc = Scratch { brushes: vectorcraft_brush::library(&baked), doc: baked.clone() };
    let plain: Vec<Node> = art.iter().map(|n| sc.plain(n)).collect();
    let reaches: Vec<Option<Rect>> = plain.iter().map(reach).collect();
    let see_through: Vec<bool> = plain.iter().map(|n| n.shows_transparency() || o.draws_see_through_image(n)).collect();
    let cmyk = src.color_mode == ColorMode::Cmyk;
    let mut out = Plan { flat: vec![], kept: vec![], transparent: roots.iter().zip(&see_through).filter(|(_, t)| **t).map(|(id, _)| *id).collect() };
    // Baking above stopped early when cancelled; each group is a place to stop (a plan changes nothing).
    vectorcraft_doc::cancel::check()?;
    for g in overlapping(&reaches) {
        vectorcraft_doc::cancel::check()?;
        let ids: Vec<NodeId> = g.iter().map(|&i| roots[i]).collect();
        let group = g.iter().any(|&i| see_through[i]).then(|| {
            let plain: Vec<&Node> = g.iter().map(|&i| &plain[i]).collect();
            flatten_group(&plain, o, cmyk)
        });
        let Some((regions, area)) = group.flatten() else {
            out.kept.extend(ids);
            continue;
        };
        let raster = match area.as_ref().filter(|_| render) {
            Some(a) => {
                let art: Vec<Node> = g.iter().map(|&i| art[i].clone()).collect();
                Some(render_raster(&baked, &art, a.rect, a.ppi, a.clip.clone(), o)?)
            }
            None => None,
        };
        out.flat.push(Flat { roots: ids, regions, area, raster });
    }
    Ok(out)
}

/// The regions and the image area replacing one group of overlapping objects (`plain`: the art as
/// composited; `cmyk`: in a CMYK document); `None` when it paints nothing.
fn flatten_group(plain: &[&Node], o: &FlattenOptions, cmyk: bool) -> Option<(Regions, Option<Area>)> {
    let mut b = Builder { cmyk, ..Default::default() };
    let elems: Vec<Elem> = plain.iter().filter_map(|n| b.elem(n)).collect();
    if b.shapes.is_empty() {
        return None;
    }
    // An unclipped full raster needs no regions.
    let whole = o.balance <= 0.0 && !o.clip_complex_regions;
    let regions = if whole { vec![] } else { po::regions(&b.shapes) };
    // An arrangement that failed (degenerate geometry) leaves only rasterizing.
    let all = regions.is_empty() || o.rasterize_all(regions.len());
    let mut vector: Vec<(usize, RegionFill)> = vec![];
    let mut complex: Vec<usize> = vec![];
    let mut gradients_only = !all;
    for (k, r) in regions.iter().enumerate() {
        if all || r.sources.iter().any(|&i| b.kinds[i] != Kind::Vector) {
            gradients_only &= r.sources.iter().all(|&i| b.kinds[i] == Kind::Gradient);
            complex.push(k);
        } else if let Some(fill) = region_fill(&elems, &r.sources, o, cmyk) {
            vector.push((k, fill));
        }
    }
    let bounds = |ks: &[usize]| ks.iter().filter_map(|&k| regions[k].path.bounds()).reduce(|a, b| a.union(b));
    if !o.clip_complex_regions && !all {
        // A rectangle of raster takes in every region it overlaps.
        while let Some(r) = bounds(&complex) {
            let before = complex.len();
            let rect = shapes::rectangle(r);
            vector.retain(|(k, _)| {
                let path = &regions[*k].path;
                let hit = path.bounds().is_some_and(|b| b.intersect(r).area() > 0.0)
                    && po::area(&po::boolean(path, FillRule::NonZero, &rect, FillRule::NonZero, po::BoolOp::Intersect), FillRule::NonZero) > 1e-6;
                if hit {
                    complex.push(*k);
                }
                !hit
            });
            if complex.len() == before {
                break;
            }
            gradients_only = false;
        }
    }
    let raster_rect = if all { plain.iter().filter_map(|n| reach(n)).reduce(|a, b| a.union(b)) } else { bounds(&complex) };
    let area = raster_rect.map(|rect| {
        let clip = o.clip_complex_regions.then(|| PathData::new(complex.iter().flat_map(|&k| regions[k].path.subpaths.iter().cloned()).collect()));
        let ppi = if gradients_only { o.gradient_ppi } else { o.line_art_ppi };
        Area { rect, ppi, clip: clip.filter(|c| !c.is_empty()), whole: all }
    });
    let vector = vector.into_iter().map(|(k, f)| (regions[k].path.clone(), f)).collect();
    Some((vector, area))
}

/// The flat colour of a region covered by `sources` (CMYK in a `cmyk` document); `None` where
/// nothing paints it.
fn region_fill(elems: &[Elem], sources: &[usize], o: &FlattenOptions, cmyk: bool) -> Option<RegionFill> {
    let mut c = Composer { sources, top: None, plane: 0 };
    let alone = c.eval(elems, CLEAR);
    if alone[3] < 0.5 / 255.0 {
        return None;
    }
    let start = if o.preserve_alpha { CLEAR } else { WHITE };
    let px = if o.preserve_alpha {
        alone
    } else {
        c.top = None;
        c.eval(elems, start)
    };
    let rgb = [px[0], px[1], px[2]];
    // The K plane composites the same way.
    let k = cmyk.then(|| {
        c.plane = 1;
        c.eval(elems, start)[0]
    });
    // A paint showing as it is keeps its colour (spot, process, swatch) and overprint.
    let near = |a: &[f32], b: &[f32]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3);
    let same = |f: &Fill| px[3] >= 1.0 - 1e-4 && near(&f.planes[0], &rgb) && k.is_none_or(|k| near(&f.planes[1][..1], &[k]));
    let color = match k {
        Some(k) => {
            let [c, m, y, k] = planes_cmyk(rgb, k);
            Color::cmyk(c, m, y, k)
        }
        None => Color::rgb(rgb[0], rgb[1], rgb[2]),
    };
    let (paint, overprint) = match c.top {
        Some(f) if o.preserve_overprints && same(f) => (f.paint.clone(), f.overprint),
        _ => (Paint::solid(color), false),
    };
    Some(RegionFill { paint, alpha: px[3], overprint })
}

/// `art` rendered over `rect` at `ppi`: over white with the white the art doesn't cover taken out
/// again (so blend modes see the white page), or over nothing with Preserve Alpha. The image and
/// its `clip` reach a pixel further. An opaque image in a CMYK document holds the art's ink
/// amounts (a CMYK TIFF), so its CMYK colours keep their numbers (#2160); other images are PNGs.
/// Fails when the image can't be encoded.
fn render_raster(doc: &Document, art: &[Node], rect: Rect, ppi: f64, clip: Option<PathData>, o: &FlattenOptions) -> Result<Raster> {
    let mut scale = ppi / 72.0;
    scale = scale.min((MAX_PIXELS / (rect.width() * rect.height()).max(1.0)).sqrt());
    let px = 1.0 / scale;
    let rect = rect.inflate(px, px);
    let clip = clip.map(|c| Some(po::offset_path(&c, px, po::Join::Miter, 4.0)).filter(|g| !g.is_empty()).unwrap_or(c));
    let region = Rect::new(
        rect.x0,
        rect.y0,
        rect.x0 + (rect.width() * scale).ceil().max(1.0) / scale,
        rect.y0 + (rect.height() * scale).ceil().max(1.0) / scale,
    );
    let xf = Affine::translate(region.origin().to_vec2()) * Affine::scale(1.0 / scale);
    let tmp = isolated_doc(doc, art.to_vec());
    let mut r = vectorcraft_render::Renderer::new();
    let no_alpha = o.no_transparency.is_some() && clip.is_some() && !o.preserve_alpha;
    if no_alpha && doc.color_mode == ColorMode::Cmyk {
        // Drawn as inks over the paper: as screen colours, the file would separate them again,
        // and a CMYK colour comes back with other inks (K moves into C, M and Y).
        let opts = vectorcraft_render::RenderOptions { background: Some([255; 4]), skip_templates: true, ..Default::default() };
        let inks = r.render_region_inks(&tmp, region, scale, &opts);
        vectorcraft_doc::cancel::check()?;
        let (width, height) = vectorcraft_render::region_pixels(region, scale);
        // Past the size of a CMYK image: drawn in screen colours, below.
        if let Some(blob) = Inks::new(width, height, inks).and_then(|i| ImageBlob::cmyk_tiff(&i)) {
            return Ok(Raster { blob, width, height, xf, clip });
        }
    }
    let mut img = r.render_region(&tmp, region, scale, !o.preserve_alpha);
    // The drawing stops early when cancelled: this image is not whole.
    vectorcraft_doc::cancel::check()?;
    if no_alpha {
        // Without transparency the clip alone shapes the image: it is opaque, over white (its
        // edge pixels too).
        for p in img.pixels.as_chunks_mut::<4>().0 {
            p[3] = 255;
        }
    } else if !o.preserve_alpha || !o.anti_alias {
        // The art's coverage: drawn opaque, so only its own edges (and soft effects) are partial.
        let cover: Vec<Node> = art
            .iter()
            .map(|n| {
                let mut c = n.clone();
                opaque(&mut c);
                c
            })
            .collect();
        let cover = r.render_region(&isolated_doc(doc, cover), region, scale, false);
        for (px, c) in img.pixels.as_chunks_mut::<4>().0.iter_mut().zip(cover.pixels.as_chunks::<4>().0) {
            let k = c[3];
            let k2 = if o.anti_alias || k == 0 || k == 255 {
                k
            } else if k >= 128 {
                255
            } else {
                0
            };
            if o.preserve_alpha {
                let f = k2 as f32 / k.max(1) as f32;
                for v in px.iter_mut() {
                    *v = (*v as f32 * f).round().min(255.0) as u8;
                }
            } else {
                let white = 255 - k2;
                for v in &mut px[..3] {
                    *v = v.saturating_sub(white).min(k2);
                }
                px[3] = k2;
            }
        }
    }
    let png = img.to_png().map_err(|e| EngineError::Other(format!("Flatten Transparency: {e}")))?;
    Ok(Raster { blob: ImageBlob::new("image/png", png), width: img.width, height: img.height, xf, clip })
}

/// `n` at full opacity with Normal blending throughout (objects, fills and strokes).
fn opaque(n: &mut Node) {
    n.opacity = 1.0;
    n.blend = BlendMode::Normal;
    for i in &mut n.appearance.items {
        match i {
            AppearanceItem::Fill(f) => (f.opacity, f.blend) = (1.0, BlendMode::Normal),
            AppearanceItem::Stroke(s) => (s.opacity, s.blend) = (1.0, BlendMode::Normal),
        }
    }
    if let Some(ch) = n.children_mut() {
        for c in ch {
            opaque(Arc::make_mut(c));
        }
    }
}

/// `doc` with everything it draws flattened with `o`, as formats without transparency (EPS)
/// write it: every visible object of the visible layers, locked ones too (a clipping layer's
/// clipping path stays). `None` when that changes nothing.
pub(crate) fn flatten_document(doc: &Document, o: &FlattenOptions) -> Result<Option<Document>> {
    let outlining = o.text_to_outlines || o.strokes_to_outlines || !o.preserve_overprints;
    if !outlining && !doc.layers.iter().any(|l| l.shows_transparency() || o.draws_see_through_image(l)) {
        return Ok(None);
    }
    let mut roots = vec![];
    for l in doc.layers.iter().filter(|l| l.visible && !matches!(l.kind, NodeKind::Layer { template: true, .. })) {
        let children = l.children().map_or(&[][..], Vec::as_slice);
        roots.extend(children.iter().skip(usize::from(l.clips())).filter(|c| c.visible).map(|c| c.id));
    }
    let plan = plan(doc, &roots, o, true)?;
    if plan.flat.is_empty() && !plan.kept.iter().any(|id| doc.node(*id).is_some_and(|n| kept_work(n, o))) {
        return Ok(None);
    }
    let mut d = doc.clone();
    apply(&mut d, &mut vectorcraft_doc::Selection::default(), plan, o)?;
    Ok(Some(d))
}

// ---------- apply ----------

fn apply(d: &mut Document, sel: &mut vectorcraft_doc::Selection, plan: Plan, o: &FlattenOptions) -> Result<(Vec<NodeId>, usize, usize)> {
    let (mut vector, mut rasterized) = (0, 0);
    let (mut ids, mut flat) = (vec![], vec![]);
    // The groups replace their objects in one pass at the end. When an object is listed twice or
    // lies inside another listed one, each group replaces its objects as it is made instead.
    let one_pass = separate(d, &plan.flat.iter().flat_map(|f| f.roots.iter().copied()).collect::<Vec<_>>());
    for f in plan.flat {
        // A group always has objects.
        let Some(&top) = f.roots.last() else { continue };
        let mut children = vec![];
        // The image goes under the regions, reaching a pixel past its own: their edges then fall on
        // matching pixels instead of on what is behind the group.
        if let Some(r) = f.raster {
            children.push(Arc::new(raster_node(d, r)));
            rasterized += 1;
        }
        for (path, fill) in f.regions {
            let mut n = shape_node(d, path, None);
            n.appearance = Appearance {
                items: vec![AppearanceItem::Fill(FillLayer { overprint: fill.overprint, ..FillLayer::new(fill.paint) })],
                ..Appearance::default()
            }
            .into();
            n.opacity = fill.alpha.min(1.0);
            children.push(Arc::new(n));
            vector += 1;
        }
        let g = Node::group(d.alloc_id(), children);
        ids.push(g.id);
        if one_pass {
            // The group replaces the topmost object, and the other objects are removed.
            flat.extend(f.roots.iter().filter(|r| **r != top).map(|r| (*r, vec![])));
            flat.push((top, vec![g]));
        } else {
            let (par, idx, _) = d.position(top).ok_or(EngineError::NoNode(top))?;
            d.insert(par, idx + 1, g)?;
            for r in &f.roots {
                d.remove(*r)?;
            }
        }
    }
    if !flat.is_empty() {
        d.insert_beside(flat, true)?;
    }
    let brushes = if o.strokes_to_outlines { vectorcraft_brush::library(d) } else { vec![] };
    for mut id in plan.kept {
        if o.text_to_outlines {
            id = outline_texts_under(d, id)?;
        }
        if o.strokes_to_outlines {
            id = outline_strokes_under(d, &brushes, id)?.0;
        }
        if !o.preserve_overprints
            && let Some(n) = d.node_mut(id)
        {
            drop_overprints(n);
        }
        ids.push(id);
    }
    ids.sort_by_key(|id| d.index_path(*id));
    sel.set(ids.iter().copied());
    Ok((ids, vector, rasterized))
}

/// An image of `r` (in a clip group when it is clipped).
fn raster_node(d: &mut Document, r: Raster) -> Node {
    let key = unique_key(d, "flattened");
    d.images.insert(key.clone(), r.blob);
    let image = Node::new(
        d.alloc_id(),
        NodeKind::Image(Box::new(ImageObject {
            key,
            width: r.width,
            height: r.height,
            xf: r.xf,
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    let Some(path) = r.clip else { return image };
    super::rasterfx::clip_group(d, path, FillRule::NonZero, image)
}

/// Replace the type under `root` by its outlines: → the id now standing for `root`.
fn outline_texts_under(d: &mut Document, root: NodeId) -> Result<NodeId> {
    let mut texts = vec![];
    if let Some(n) = d.node(root) {
        n.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Text(_)) {
                texts.push(c.clone());
            }
        });
    }
    let mut out = root;
    for t in texts {
        let Some(o) = outlined_text(&t) else { continue };
        let o = d.reid(&o);
        let nid = o.id;
        *d.node_mut(t.id).ok_or(EngineError::NoNode(t.id))? = o;
        if t.id == root {
            out = nid;
        }
    }
    Ok(out)
}

/// Whether the outline and overprint options change `n`, an object without transparency.
fn kept_work(n: &Node, o: &FlattenOptions) -> bool {
    let mut any = false;
    n.walk(&mut |c| {
        any |= (o.text_to_outlines && matches!(c.kind, NodeKind::Text(_)))
            || (o.strokes_to_outlines && stroked(c))
            || (!o.preserve_overprints && overprints(c));
    });
    any
}

/// Whether `n` is a path with a visible stroke (outlining turns it into a fill).
fn stroked(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. })
        && n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(s) if s.visible && !s.paint.is_none() && s.width > 0.0))
}

fn overprints(n: &Node) -> bool {
    n.appearance.items.iter().any(AppearanceItem::overprint)
        || matches!(&n.kind, NodeKind::Text(t) if t.runs.iter().any(|r| r.style.overprint_fill || r.style.overprint_stroke))
}

fn drop_overprints(n: &mut Node) {
    if overprints(n) {
        for i in &mut n.appearance.items {
            *i.overprint_mut() = false;
        }
        if let NodeKind::Text(t) = &mut n.kind {
            for r in &mut t.runs {
                (r.style.overprint_fill, r.style.overprint_stroke) = (false, false);
            }
        }
    }
    if let Some(ch) = n.children_mut() {
        for c in ch {
            drop_overprints(Arc::make_mut(c));
        }
    }
}

// ---------- plain art ----------

/// Converts art into what the compositor works on.
struct Scratch {
    /// A copy of the document the art comes from (outlining allocates ids in it).
    doc: Document,
    brushes: Vec<vectorcraft_brush::Brush>,
}

impl Scratch {
    /// `n` as plain filled shapes: type outlined, symbol instances replaced by their art, live
    /// objects evaluated and strokes outlined (brush strokes as their art), each keeping the
    /// object's transparency.
    fn plain(&mut self, n: &Node) -> Node {
        if !n.visible {
            return n.clone();
        }
        let made = match &n.kind {
            NodeKind::Text(_) => outlined_text(n),
            NodeKind::SymbolInstance { symbol, .. } => {
                let art = self.doc.symbols.iter().find(|s| &s.name == symbol).map(|s| vectorcraft_render::instance_art(&s.art, n));
                art.and_then(|a| effects::outline_art(Some(&self.doc), n, Some(&a))).map(|g| carry(n, g))
            }
            // A placed document stays one: its art's resources aren't the document's.
            NodeKind::PlacedDocument(_) => None,
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Repeat(_) => {
                let hook: &dyn Fn(&Node) -> Option<Node> = &effects::outline_text;
                let hooks = vectorcraft_doc::live::Hooks { outline: Some(hook), symbols: Some(&self.doc), ..Default::default() };
                Some(carry(n, vectorcraft_doc::live::expanded_group_hooks(n, hooks)))
            }
            NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. } => outline_strokes(&mut self.doc, &self.brushes, n),
            _ => None,
        };
        let mut m = match made {
            // The pieces of one object never knock each other out.
            Some(m) if m.is_container() => Node { knockout: Knockout::Off, ..m },
            Some(m) => m,
            None => n.clone(),
        };
        if !matches!(m.kind, NodeKind::Compound { .. })
            && let Some(ch) = m.children_mut()
        {
            for c in ch.iter_mut() {
                *c = Arc::new(self.plain(c));
            }
        }
        m
    }
}

/// Type as outlines painted like it ([`effects::outline_text`]), keeping its transparency.
fn outlined_text(n: &Node) -> Option<Node> {
    effects::outline_text(n).map(|g| Node { knockout: Knockout::Off, ..carry(n, g) })
}

/// `to` (art made from `from`) with `from`'s name, transparency, opacity mask and effects.
fn carry(from: &Node, mut to: Node) -> Node {
    to.name = from.name.clone();
    to.opacity = from.opacity;
    to.blend = from.blend;
    to.isolate = from.isolate;
    to.knockout_shape = from.knockout_shape;
    to.extras_mut().mask = from.extras().mask.clone();
    to.appearance.effects = from.appearance.effects.clone();
    to
}

/// The keys of `doc`'s images that have see-through pixels (an alpha channel that isn't opaque
/// everywhere).
pub(crate) fn see_through_images(doc: &Document) -> BTreeSet<String> {
    let see_through = |b: &ImageBlob| {
        b.mime != "image/jpeg" && image::load_from_memory(&b.bytes).is_ok_and(|i| i.color().has_alpha() && i.to_rgba8().pixels().any(|p| p[3] < 255))
    };
    doc.images.iter().filter(|(_, b)| see_through(b)).map(|(k, _)| k.clone()).collect()
}

/// `n` shows an image of `keys` (itself or inside).
fn draws_image(n: &Node, keys: &BTreeSet<String>) -> bool {
    !keys.is_empty()
        && n.visible
        && match &n.kind {
            NodeKind::Image(im) => keys.contains(&im.key),
            _ => n.children().is_some_and(|ch| ch.iter().any(|c| draws_image(c, keys))),
        }
}

fn visible_fx(fx: &[Effect]) -> bool {
    fx.iter().any(|e| e.visible)
}

/// Effects on the object or on one of its fills or strokes (raster effects, once baked).
fn has_effects(n: &Node) -> bool {
    visible_fx(&n.appearance.effects) || n.appearance.items.iter().any(|i| visible_fx(i.effects()))
}

fn masked(n: &Node) -> bool {
    n.extras().mask.as_ref().is_some_and(|m| !m.disabled)
}

/// Where `n` can paint: its visual bounds grown by its effects (and a margin for antialiasing).
fn reach(n: &Node) -> Option<Rect> {
    if !n.visible {
        return None;
    }
    let b = match n.children() {
        Some(ch) if !n.clips() && !matches!(n.kind, NodeKind::Compound { .. }) => ch.iter().filter_map(|c| reach(c)).reduce(|a, b| a.union(b))?,
        _ => n.visual_bounds()?,
    };
    let fx = n.appearance.items.iter().map(|i| effects::outset(i.effects(), b)).fold(effects::outset(&n.appearance.effects, b), f64::max);
    Some(b.inflate(fx + 1.0, fx + 1.0))
}

/// Indices of `bounds` grouped by overlap (transitively), each group in index order.
fn overlapping(bounds: &[Option<Rect>]) -> Vec<Vec<usize>> {
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..bounds.len()).collect();
    let mut order: Vec<(usize, Rect)> = bounds.iter().enumerate().filter_map(|(i, b)| b.map(|b| (i, b))).collect();
    order.sort_by(|a, b| a.1.x0.total_cmp(&b.1.x0));
    // Sweep left to right, comparing each box with those still open at its left edge.
    let mut open: Vec<(usize, Rect)> = vec![];
    for (i, b) in order {
        open.retain(|(_, o)| o.x1 >= b.x0);
        for (j, o) in &open {
            if o.y0 <= b.y1 && b.y0 <= o.y1 {
                let (a, c) = (find(&mut parent, i), find(&mut parent, *j));
                parent[a.max(c)] = a.min(c);
            }
        }
        open.push((i, b));
    }
    let mut groups: Vec<Vec<usize>> = vec![];
    let mut slot: Vec<Option<usize>> = vec![None; bounds.len()];
    for i in 0..bounds.len() {
        let r = find(&mut parent, i);
        match slot[r] {
            Some(g) => groups[g].push(i),
            None => {
                slot[r] = Some(groups.len());
                groups.push(vec![i]);
            }
        }
    }
    groups
}

// ---------- the compositor ----------

/// What an arrangement shape stands for.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// Solid fills and clipping paths: regions inside get a flat colour.
    Vector,
    /// A gradient or mesh: regions inside are rasterized at the gradient resolution.
    Gradient,
    /// Anything else that has no single colour (patterns, images, masks, raster effects).
    Other,
}

/// A solid fill as the compositor uses it.
struct Fill {
    /// What it composites: its screen colour (twice), or in a CMYK document its two ink planes
    /// ([`cmyk_planes`]).
    planes: [[f32; 3]; 2],
    opacity: f32,
    blend: BlendMode,
    paint: Paint,
    overprint: bool,
}

/// One object of the art being flattened, with the transparency the renderer gives it.
struct Elem {
    opacity: f32,
    blend: BlendMode,
    isolate: bool,
    knockout: Knockout,
    knockout_shape: bool,
    /// Blending inside reaches the backdrop ([`Node::blends_through`]).
    blends: bool,
    /// Inside a knockout group its children are elements of their own ([`Node::passes_knockout_through`]).
    pass_through: bool,
    body: Body,
}

enum Body {
    /// Solid fills (bottom first) of arrangement shape `shape`.
    Fills { shape: usize, fills: Vec<Fill> },
    /// A group's children, clipped to arrangement shape `clip` in a clip group.
    Group { clip: Option<usize>, children: Vec<Elem> },
    /// Art without a flat colour: its regions are rasterized, so it never composites here.
    Complex,
}

/// Builds the compositor's objects and the arrangement shapes they cover.
#[derive(Default)]
struct Builder {
    shapes: Vec<po::Shape>,
    kinds: Vec<Kind>,
    /// Building a CMYK document's art.
    cmyk: bool,
}

impl Builder {
    /// What colour `c` composites as ([`Fill::planes`]).
    fn planes(&self, c: &Color) -> [[f32; 3]; 2] {
        if self.cmyk {
            let cms = vectorcraft_color::cms::active();
            cmyk_planes(cms.to_cmyk(c, cms.settings().intent))
        } else {
            [c.to_rgb(); 2]
        }
    }

    fn shape(&mut self, path: PathData, rule: FillRule, kind: Kind) -> Option<usize> {
        path.bounds()?;
        self.shapes.push(po::Shape::new(path, rule, self.shapes.len() as u64));
        self.kinds.push(kind);
        Some(self.shapes.len() - 1)
    }

    fn elem(&mut self, n: &Node) -> Option<Elem> {
        if !n.visible {
            return None;
        }
        let body = if masked(n) || has_effects(n) {
            self.shape(shapes::rectangle(reach(n)?), FillRule::NonZero, Kind::Other)?;
            Body::Complex
        } else {
            match &n.kind {
                NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
                    let (clip, rest) = if n.clips() {
                        let (first, rest) = children.split_first()?;
                        // Nothing to clip by hides the clipped art.
                        let (bp, rule) = effects::clip_outline(first)?;
                        (Some(self.shape(PathData::from_bezpath(&bp), rule, Kind::Vector)?), rest)
                    } else {
                        (None, &children[..])
                    };
                    Body::Group { clip, children: rest.iter().filter_map(|c| self.elem(c)).collect() }
                }
                NodeKind::Path { guide: true, .. } => return None,
                NodeKind::Path { .. } | NodeKind::Compound { .. } => {
                    let (path, rule) = node_path(n)?;
                    let mut fills = vec![];
                    for item in n.appearance.items.iter().filter(|i| i.is_fill() && i.visible() && !i.paint().is_none()) {
                        match item.paint() {
                            Paint::Solid { color, .. } => fills.push(Fill {
                                planes: self.planes(color),
                                opacity: item.opacity(),
                                blend: item.blend(),
                                paint: item.paint().clone(),
                                overprint: item.overprint(),
                            }),
                            p => {
                                let kind = if matches!(p, Paint::Gradient(_)) { Kind::Gradient } else { Kind::Other };
                                self.shape(path.clone(), rule, kind);
                            }
                        }
                    }
                    if fills.is_empty() { Body::Complex } else { Body::Fills { shape: self.shape(path, rule, Kind::Vector)?, fills } }
                }
                NodeKind::Image(im) => {
                    let frame = shapes::rectangle(Rect::new(0.0, 0.0, im.width as f64, im.height as f64)).transformed(im.xf);
                    self.shape(frame, FillRule::NonZero, Kind::Other)?;
                    Body::Complex
                }
                NodeKind::Mesh(_) => {
                    self.shape(shapes::rectangle(reach(n)?), FillRule::NonZero, Kind::Gradient)?;
                    Body::Complex
                }
                _ => {
                    self.shape(shapes::rectangle(reach(n)?), FillRule::NonZero, Kind::Other)?;
                    Body::Complex
                }
            }
        };
        Some(Elem {
            opacity: n.opacity,
            blend: n.blend,
            isolate: n.isolate,
            knockout: n.knockout,
            knockout_shape: n.knockout_shape,
            blends: n.blends_through(),
            pass_through: n.passes_knockout_through(),
            body,
        })
    }
}

/// A straight (not premultiplied) RGBA colour in 0..=1.
type Px = [f32; 4];
const CLEAR: Px = [0.0; 4];
const WHITE: Px = [1.0; 4];

fn premul(p: Px) -> Px {
    [p[0] * p[3], p[1] * p[3], p[2] * p[3], p[3]]
}

fn unpremul(q: Px) -> Px {
    if q[3] <= 0.0 { CLEAR } else { [q[0] / q[3], q[1] / q[3], q[2] / q[3], q[3].min(1.0)] }
}

/// Composites the objects covering one atomic region exactly as the renderer composites pixels
/// (its transparency groups, non-isolated groups, knockout groups and clip groups): the same
/// paints cover the whole region, so one colour stands for all of it.
struct Composer<'a> {
    /// Arrangement shapes covering the region, ascending.
    sources: &'a [usize],
    /// The last solid fill composited.
    top: Option<&'a Fill>,
    /// Which of the fills' [`Fill::planes`] composites.
    plane: usize,
}

impl<'a> Composer<'a> {
    fn inside(&self, shape: usize) -> bool {
        self.sources.binary_search(&shape).is_ok()
    }

    /// `elems` (paint order) composited over `start`.
    fn eval(&mut self, elems: &'a [Elem], start: Px) -> Px {
        elems.iter().fold(start, |acc, e| self.draw(e, acc, false, false, false))
    }

    /// `e` drawn over backdrop `b` inside a group that knocks out when `enclosing`. `as_shape`
    /// draws it as its knockout shape (full object opacity); `nested` is drawing inside a layer,
    /// where a non-isolated group can't copy its backdrop and is drawn isolated.
    fn draw(&mut self, e: &'a Elem, b: Px, enclosing: bool, as_shape: bool, nested: bool) -> Px {
        let opacity = if as_shape { 1.0 } else { e.opacity };
        let ko = matches!(e.body, Body::Group { .. }) && e.knockout.resolve(enclosing);
        if opacity >= 1.0 && e.blend == BlendMode::Normal && !e.isolate && !ko {
            return self.content(e, b, ko, None, nested);
        }
        if e.blends && !e.isolate && !nested {
            // Drawn over a copy of the backdrop, so blending inside reaches it.
            let c = self.content(e, b, ko, Some(b), false);
            if e.blend == BlendMode::Normal {
                let (pb, pc) = (premul(b), premul(c));
                return unpremul([0, 1, 2, 3].map(|i| pb[i] + (pc[i] - pb[i]) * opacity));
            }
            // The group's own colour: the backdrop's share taken out by the group's coverage.
            let (pc, pa, pb) = (premul(c), premul(self.content(e, CLEAR, ko, None, false)), premul(b));
            let keep = 1.0 - pa[3];
            let g = [0, 1, 2].map(|i| (pc[i] - keep * pb[i]).clamp(0.0, pa[3]));
            let g = unpremul([g[0], g[1], g[2], pa[3]]);
            return composite(e.blend, b, [g[0], g[1], g[2], g[3] * opacity]);
        }
        let inner_nested = !(e.blends && (e.isolate || !nested));
        let g = self.content(e, CLEAR, ko, None, inner_nested);
        composite(e.blend, b, [g[0], g[1], g[2], g[3] * opacity])
    }

    /// What `e` draws over `b` inside its group (`ko`: its children knock each other out, against
    /// `backdrop` when the group was drawn over a copy of it).
    fn content(&mut self, e: &'a Elem, b: Px, ko: bool, backdrop: Option<Px>, nested: bool) -> Px {
        match &e.body {
            Body::Fills { shape, fills } if self.inside(*shape) => fills.iter().fold(b, |acc, f| {
                self.top = Some(f);
                let [r, g, b] = f.planes[self.plane];
                composite(f.blend, acc, [r, g, b, f.opacity])
            }),
            Body::Group { clip, children } if clip.is_none_or(|c| self.inside(c)) => {
                if ko {
                    self.knockout(children, b, backdrop)
                } else {
                    children.iter().fold(b, |acc, c| self.draw(c, acc, false, false, nested))
                }
            }
            _ => b,
        }
    }

    /// The children of a knockout group over `b`: each first erases what the ones below it drew
    /// by its knockout shape, then adds itself composited against the group's backdrop.
    fn knockout(&mut self, children: &'a [Elem], b: Px, backdrop: Option<Px>) -> Px {
        fn elements<'e>(children: &'e [Elem], out: &mut Vec<&'e Elem>) {
            for c in children {
                match &c.body {
                    Body::Group { children, .. } if c.pass_through => elements(children, out),
                    _ => out.push(c),
                }
            }
        }
        let mut els = vec![];
        elements(children, &mut els);
        let mut acc = premul(b);
        for c in els {
            let s = self.draw(c, CLEAR, true, !c.knockout_shape, true)[3];
            let own = match backdrop {
                Some(b0) => premul(self.draw(c, b0, true, false, true)).map(|v| v * s),
                None => premul(self.draw(c, CLEAR, true, false, true)),
            };
            acc = [0, 1, 2, 3].map(|i| acc[i] * (1.0 - s) + own[i]);
        }
        unpremul(acc)
    }
}
