use super::analysis::draws_image;
use super::apply::{apply, kept_work, plan};
use super::presets::{FlattenerPreset, saved_preset};
use super::report::target_roots;
use super::{BTreeSet, Deserialize, EngineError, Node, Result, Serialize, Session, Value, bad, json, str_param};

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
    pub(super) fn draws_see_through_image(&self, n: &Node) -> bool {
        self.no_transparency.as_ref().is_some_and(|keys| draws_image(n, keys))
    }

    /// Whether a group that splits into `regions` atomic regions is rasterized whole.
    pub(super) fn rasterize_all(&self, regions: usize) -> bool {
        self.balance <= 0.0 || (self.balance < 100.0 && regions as f64 > 10f64.powf(self.balance / 20.0))
    }
}

pub(super) fn flatten(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "object.flattenTransparency";
    let o = s.flatten_options(p).map_err(|m| bad(C, m))?;
    let roots = target_roots(s, p)?;
    if roots.is_empty() {
        return Err(EngineError::Other("Flatten Transparency: select objects to flatten".into()));
    }
    let src = s.doc()?.doc.clone();
    let failed = vectorcraft_render::decode_failures();
    let plan = plan(&src, &roots, &o, true)?;
    let warning = crate::cmd::rasterfx::undecoded_warning(failed);
    let options = serde_json::to_value(&o).unwrap_or_default();
    if plan.flat.is_empty() && !plan.kept.iter().any(|id| src.node(*id).is_some_and(|n| kept_work(n, &o))) {
        return Ok(json!({ "ids": roots.iter().map(|i| i.0).collect::<Vec<_>>(), "rasterized": 0, "vector": 0, "options": options }));
    }
    let (ids, vector, rasterized) = s.edit("Flatten Transparency", |d, sel| {
        let threaded = crate::cmd::threads::frames(d);
        let out = apply(d, sel, plan, &o)?;
        // Frames of threaded text that were outlined or flattened leave their threads with the
        // text they show.
        crate::cmd::threads::leave_threads(d, &threaded);
        Ok(out)
    })?;
    let mut out = json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "rasterized": rasterized, "vector": vector, "options": options });
    if let Some(w) = warning {
        out["warning"] = json!(w);
    }
    Ok(out)
}
