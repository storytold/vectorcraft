//! File → New: the settings `file.new` takes, the New Document presets (generated in code, by
//! category), the user's saved presets and the recently used sizes (both kept in the
//! preferences), and the artboard grid layout shared with Rearrange All Artboards.
//!
//! New Document profiles: each built-in category has a profile, a native document in the
//! `New Document Profiles` folder next to the preferences ([`Session::attach_new_doc_profiles`]).
//! `file.new` starts a document of that category with the profile's swatches, graphic styles,
//! brushes, symbols, character and paragraph styles, rulers, document setup and print settings
//! ([`DocSettings::document_from_profile`]). The grid stays that of Preferences › Guides & Grid,
//! which applies to every open document. A category without a profile file starts with the
//! defaults, and a profile that can't be read leaves them in place and is reported in `file.new`'s
//! result.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::{Artboard, Background, ColorMode, Document, LayerColor, Node, Unit};
use vectorcraft_geom::{Point, Rect};

use super::{CommandSpec, Result, Session, always, bad, clock_date, cmd, date_param, str_param, switch_or};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "file.newPresets",
            "New Document Presets",
            [],
            None,
            "{category?: \"Recent\"|\"Saved\"|\"Mobile\"|\"Web\"|\"Print\"|\"Film & Video\"|\"Art & Illustration\"|\"Branding\"|\"Social\"} the New Document presets by category (Recent: the last sizes used; Saved: the user's presets) → {categories: [{name, presets: [{name, size: \"1920 × 1080 px\", width: pt, height: pt, units, orientation, artboards, artboardLayout: {layout, columns, spacing, rightToLeft}, bleed, backgroundContents, colorMode, rasterEffectsPpi, previewMode, alignNewObjectsToPixelGrid (on in the Mobile and Web presets)}]}]}; file.new {preset: name} starts from one",
            always,
            list
        ),
        cmd!(
            "file.newPresets.save",
            "Save Preset",
            [],
            None,
            "{name, preset?: start from this preset, …settings as file.new (default: Letter in prefs unitsGeneral)} save New Document settings as a user preset (the Saved category; same name = replace) in the preferences → {name, count}",
            always,
            save
        ),
        cmd!(
            "file.newPresets.delete",
            "Delete Preset",
            [],
            None,
            "{name} delete a saved New Document preset → {deleted}",
            |s| if s.prefs.new_doc_presets.is_empty() { Err("no saved presets".into()) } else { Ok(()) },
            delete
        ),
        cmd!(
            query "file.newProfiles",
            "New Document Profiles",
            [],
            None,
            "{} the New Document profiles, one per built-in category of file.newPresets: native documents in the New Document Profiles folder next to the preferences, whose swatches, swatch groups, graphic styles, brushes, symbols, patterns, character and paragraph styles, composite fonts, SVG filters, graph designs, ruler origin and mode, transparency grid, document setup (other than bleed, background and Align New Objects to Pixel Grid), assigned color profiles, Document Raster Effects Settings (other than the resolution) and print settings a new document of that category starts with → {profiles: [{name, customized?: a profile file exists (else the shipped profile, today's defaults, applies), path?; both left out for a request that may not read the folder}], folder?: the folder (none in runs without preferences and on the web)}",
            always,
            list_profiles
        ),
        cmd!(
            "file.newProfiles.edit",
            "Edit Profile",
            [],
            None,
            "{name: a category of file.newPresets} open the category's New Document profile as a document to edit (File › Save keeps the changes for the next file.new); a category without a profile file gets the shipped profile written first; under automation roots it needs write and read access to the file, and an error names the file by its file name outside the read roots → document.open's result",
            always,
            edit_profile
        ),
        cmd!(
            "file.newProfiles.reset",
            "Reset Profile",
            [],
            None,
            "{name: a category of file.newPresets} delete the category's profile file, so the shipped profile (today's defaults) applies again; an open copy of it stays open; under automation roots it needs write access to the file, and an error names the file by its file name outside the read roots → {name, reset: whether a file was deleted}",
            always,
            reset_profile
        ),
    ]
}

/// How New Document lays out several artboards.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ArtboardLayout {
    /// Rows of `columns` artboards, filled left to right then down.
    #[default]
    GridByRow,
    /// `columns` columns, filled top to bottom then across.
    GridByColumn,
    /// One row.
    Row,
    /// One column.
    Column,
}

impl ArtboardLayout {
    pub const ALL: [Self; 4] = [Self::GridByRow, Self::GridByColumn, Self::Row, Self::Column];
    pub fn id(self) -> &'static str {
        match self {
            Self::GridByRow => "gridByRow",
            Self::GridByColumn => "gridByColumn",
            Self::Row => "row",
            Self::Column => "column",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::GridByRow => "Grid by Row",
            Self::GridByColumn => "Grid by Column",
            Self::Row => "Arrange by Row",
            Self::Column => "Arrange by Column",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.id().eq_ignore_ascii_case(s) || l.label().eq_ignore_ascii_case(s))
    }
}

/// How a new document previews: normally, as pixels (Pixel Preview) or with Overprint Preview.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PreviewMode {
    #[default]
    Default,
    Pixel,
    Overprint,
}

impl PreviewMode {
    pub const ALL: [Self; 3] = [Self::Default, Self::Pixel, Self::Overprint];
    pub fn id(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Pixel => "pixel",
            Self::Overprint => "overprint",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.id().eq_ignore_ascii_case(s))
    }
}

/// A saved preset's raster effects resolution, within 1 to 2400 ppi (72 when it isn't finite).
fn de_ppi<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<f64, D::Error> {
    Ok(Some(f64::deserialize(d)?).filter(|v| v.is_finite()).unwrap_or(72.0).clamp(1.0, 2400.0))
}

/// Everything New Document sets: a preset (built-in, saved or recent) or `file.new`'s params.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DocSettings {
    /// The preset's name.
    pub name: String,
    /// Artboard size in points, more than 0 and up to `MAX_SIDE` (a saved or Recent preset's
    /// value outside that range is read as the nearer end, and one of 0 or less is read as 1).
    #[serde(deserialize_with = "de_side")]
    pub width: f64,
    #[serde(deserialize_with = "de_side")]
    pub height: f64,
    pub units: Unit,
    /// 1 to `MAX_ARTBOARDS` (a saved or Recent preset's value outside that range is read as the
    /// nearer end).
    #[serde(deserialize_with = "de_artboards")]
    pub artboards: u32,
    pub layout: ArtboardLayout,
    /// Columns of a grid layout.
    pub columns: u32,
    /// Space between artboards, points: 0 to `MAX_SPACING` (a saved or Recent preset's value
    /// outside that range is read as the nearer end).
    #[serde(deserialize_with = "de_spacing")]
    pub spacing: f64,
    /// Change to Right-to-Left Layout: the first artboard at the right.
    pub right_to_left: bool,
    /// Top, bottom, left, right, points: 0 to `MAX_BLEED` each (a saved or Recent preset's value
    /// outside that range is read as the nearer end).
    #[serde(deserialize_with = "de_bleed")]
    pub bleed: [f64; 4],
    pub background: Background,
    pub color_mode: ColorMode,
    /// 1 to 2400 ppi (a saved preset's value outside that range is read as the nearer end).
    #[serde(deserialize_with = "de_ppi")]
    pub raster_effects_ppi: f64,
    pub preview_mode: PreviewMode,
    /// Align New Objects to Pixel Grid ([`vectorcraft_doc::DocSetup::align_new_objects_to_pixel_grid`]):
    /// on in the Mobile and Web presets, off in the others and in a saved or Recent preset from
    /// before it.
    pub align_new_objects_to_pixel_grid: bool,
    /// The New Document profile the document starts from (a built-in category's name): the
    /// category of a built-in preset, kept by the saved and Recent presets made from it; `None`:
    /// the defaults.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

impl Default for DocSettings {
    fn default() -> Self {
        Self {
            name: "Letter".into(),
            width: 612.0,
            height: 792.0,
            units: Unit::Points,
            artboards: 1,
            layout: ArtboardLayout::GridByRow,
            columns: 0,
            spacing: 20.0,
            right_to_left: false,
            bleed: [0.0; 4],
            background: Background::Transparent,
            color_mode: ColorMode::Rgb,
            raster_effects_ppi: 72.0,
            preview_mode: PreviewMode::Default,
            align_new_objects_to_pixel_grid: false,
            profile: None,
        }
    }
}

/// Most artboards a new document can have (as the reference app).
pub const MAX_ARTBOARDS: u32 = 1000;
/// Raster effects resolutions New Document offers: (ppi, label).
pub const RASTER_PPI: [(f64, &str); 3] = [(72.0, "Screen (72 ppi)"), (150.0, "Medium (150 ppi)"), (300.0, "High (300 ppi)")];

impl DocSettings {
    /// The columns a grid actually uses (0 or more than the artboards: one row).
    pub fn grid_columns(&self) -> usize {
        let n = self.artboards.max(1) as usize;
        match self.layout {
            ArtboardLayout::Row => n,
            ArtboardLayout::Column => 1,
            _ if self.columns == 0 => n,
            _ => (self.columns as usize).min(n),
        }
    }

    /// "portrait" or "landscape" (a square is portrait).
    pub fn orientation(&self) -> &'static str {
        if self.width > self.height { "landscape" } else { "portrait" }
    }

    /// The size as New Document shows it, e.g. "1920 × 1080 px".
    pub fn size_label(&self) -> String {
        let u = self.units;
        format!("{} × {} {}", u.number(self.width), u.number(self.height), u.suffix())
    }

    /// As `file.newPresets` lists it (and `file.new` takes it back).
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "size": self.size_label(),
            "width": self.width,
            "height": self.height,
            "units": self.units.label(),
            "orientation": self.orientation(),
            "artboards": self.artboards,
            "artboardLayout": {"layout": self.layout.id(), "columns": self.grid_columns(), "spacing": self.spacing, "rightToLeft": self.right_to_left},
            "bleed": self.bleed,
            "backgroundContents": self.background.id(),
            "colorMode": color_mode_id(self.color_mode),
            "rasterEffectsPpi": self.raster_effects_ppi,
            "previewMode": self.preview_mode.id(),
            "alignNewObjectsToPixelGrid": self.align_new_objects_to_pixel_grid,
            "profile": self.profile,
        })
    }

    /// These settings with `p`'s changes (the params of `file.new`) → validated.
    pub fn with_params(mut self, p: &Value, cmd: &str) -> Result<Self> {
        if let Some(u) = p.get("units").filter(|v| !v.is_null()) {
            let u = u.as_str().unwrap_or("");
            self.units = Unit::named(u).ok_or_else(|| bad(cmd, format!("unknown units `{u}`")))?;
        }
        // Numbers are points; strings may carry their unit ("210 mm"), else the document's.
        let length = |k: &str, units: Unit| -> Result<Option<f64>> {
            match p.get(k) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::Number(n)) => Ok(n.as_f64()),
                Some(Value::String(s)) => units.parse(s).map(Some).ok_or_else(|| bad(cmd, format!("{k}: can't read `{s}` as a length"))),
                Some(_) => Err(bad(cmd, format!("{k} must be a number (pt) or a length such as \"210 mm\""))),
            }
        };
        if let Some(w) = length("width", self.units)? {
            self.width = w;
        }
        if let Some(h) = length("height", self.units)? {
            self.height = h;
        }
        if !side_in_range(self.width) || !side_in_range(self.height) {
            return Err(side_error(cmd, self.units));
        }
        match p.get("orientation").and_then(Value::as_str) {
            None => {}
            Some(o) if o.eq_ignore_ascii_case("portrait") => (self.width, self.height) = (self.width.min(self.height), self.width.max(self.height)),
            Some(o) if o.eq_ignore_ascii_case("landscape") => (self.width, self.height) = (self.width.max(self.height), self.width.min(self.height)),
            Some(o) => return Err(bad(cmd, format!("orientation must be portrait or landscape, not `{o}`"))),
        }
        if let Some(n) = p.get("artboards").filter(|v| !v.is_null()) {
            let n = n.as_f64().filter(|n| n.is_finite()).ok_or_else(|| bad(cmd, "artboards must be a number"))?;
            self.artboards = n.round().clamp(1.0, MAX_ARTBOARDS as f64) as u32;
        }
        if let Some(l) = p.get("artboardLayout").filter(|v| !v.is_null()) {
            self.layout_params(l, cmd)?;
        }
        if let Some(b) = p.get("bleed").filter(|v| !v.is_null()) {
            self.bleed = super::docsetup::bleed_param(b, self.bleed, cmd)?;
        }
        if let Some(b) = p.get("backgroundContents").filter(|v| !v.is_null()) {
            self.background = super::docsetup::background_param(b, cmd)?;
        }
        if let Some(m) = str_param(p, "colorMode") {
            self.color_mode = match m.to_ascii_lowercase().as_str() {
                "rgb" | "rgb color" => ColorMode::Rgb,
                "cmyk" | "cmyk color" => ColorMode::Cmyk,
                _ => return Err(bad(cmd, format!("colorMode must be rgb or cmyk, not `{m}`"))),
            };
        }
        if let Some(v) = p.get("rasterEffectsPpi").filter(|v| !v.is_null()) {
            self.raster_effects_ppi = v
                .as_f64()
                .filter(|r| (1.0..=2400.0).contains(r))
                .ok_or_else(|| bad(cmd, "rasterEffectsPpi must be 1–2400 (72 screen, 150 medium, 300 high)"))?;
        }
        if let Some(m) = p.get("previewMode").filter(|v| !v.is_null()) {
            self.preview_mode = m.as_str().and_then(PreviewMode::parse).ok_or_else(|| bad(cmd, "previewMode must be default, pixel or overprint"))?;
        }
        self.align_new_objects_to_pixel_grid = switch_or(p, "alignNewObjectsToPixelGrid", self.align_new_objects_to_pixel_grid, cmd)?;
        match p.get("profile") {
            None => {}
            Some(Value::Null) => self.profile = None,
            Some(v) => self.profile = Some(profile_name(v, cmd)?.to_string()),
        }
        let extent = self.boards().iter().fold(Rect::ZERO, |a, r| a.union(*r));
        if extent.width().max(extent.height()) + self.bleed.iter().fold(0.0f64, |a, b| a.max(*b)) * 2.0 > crate::MAX_COORD {
            return Err(bad(cmd, "the artboards don't fit on the canvas: fewer artboards, less spacing or a smaller size"));
        }
        Ok(self)
    }

    /// `artboardLayout: {layout?, columns?, spacing?, rightToLeft?}` or just a layout id.
    fn layout_params(&mut self, l: &Value, cmd: &str) -> Result<()> {
        let bad_layout = || bad(cmd, "artboardLayout.layout must be gridByRow, gridByColumn, row or column");
        if let Some(id) = l.as_str() {
            self.layout = ArtboardLayout::parse(id).ok_or_else(bad_layout)?;
            return Ok(());
        }
        let o = l.as_object().ok_or_else(|| bad(cmd, "artboardLayout must be {layout?, columns?, spacing?, rightToLeft?}"))?;
        for (k, v) in o {
            match k.as_str() {
                "layout" => self.layout = v.as_str().and_then(ArtboardLayout::parse).ok_or_else(bad_layout)?,
                "columns" => {
                    self.columns = v
                        .as_f64()
                        .filter(|c| c.is_finite() && *c >= 1.0)
                        .ok_or_else(|| bad(cmd, "artboardLayout.columns must be 1 or more"))?
                        .round()
                        .min(MAX_ARTBOARDS as f64) as u32
                }
                "spacing" => {
                    self.spacing = v
                        .as_f64()
                        .filter(|s| (0.0..=MAX_SPACING).contains(s))
                        .ok_or_else(|| bad(cmd, format!("artboardLayout.spacing must be 0–{MAX_SPACING} pt")))?
                }
                "rightToLeft" => self.right_to_left = v.as_bool().ok_or_else(|| bad(cmd, "artboardLayout.rightToLeft must be true or false"))?,
                _ => return Err(bad(cmd, format!("unknown artboardLayout key `{k}` (layout, columns, spacing, rightToLeft)"))),
            }
        }
        Ok(())
    }

    /// The artboard rectangles, the first at the origin's side.
    pub fn boards(&self) -> Vec<Rect> {
        let n = self.artboards.max(1) as usize;
        let sizes = vec![(self.width, self.height); n];
        let by_column = self.layout == ArtboardLayout::GridByColumn;
        grid_origins(&sizes, Point::ZERO, self.grid_columns(), self.spacing, by_column, self.right_to_left)
            .into_iter()
            .map(|o| Rect::from_origin_size(o, (self.width, self.height)))
            .collect()
    }

    /// The new document (title and swatches set by `file.new`).
    pub fn document(&self) -> Document {
        let mut d = Document::new_with_mode(self.width, self.height, self.color_mode);
        d.units = self.units;
        d.raster_effects_ppi = self.raster_effects_ppi;
        d.setup.bleed = self.bleed;
        d.setup.background = self.background;
        d.setup.align_new_objects_to_pixel_grid = self.align_new_objects_to_pixel_grid;
        d.artboards = self
            .boards()
            .into_iter()
            .enumerate()
            .map(|(i, rect)| Artboard {
                id: i as u32 + 1,
                name: format!("Artboard {}", i + 1),
                rect,
                locked: false,
                show_center_mark: false,
                show_cross_hairs: false,
                show_video_safe_areas: false,
                video_ruler_pixel_aspect_ratio: 1.0,
                color: None,
                guide_label: None,
            })
            .collect();
        d
    }

    /// The new document started from New Document profile `profile`: [`Self::document`] with the
    /// profile's swatches, swatch groups, graphic styles, brushes, symbols, patterns and the images
    /// they use, character and paragraph styles, composite fonts, SVG filters, graph designs, ruler
    /// origin and mode, transparency grid, document setup (bleed, background and Align New Objects
    /// to Pixel Grid stay these settings'), assigned color profiles, raster effects settings (the
    /// resolution stays these settings') and print settings, in these settings' color mode (the
    /// profile's colors converted as Document Color Mode converts them). The profile's art,
    /// layers and artboards are left out: the document has one empty layer.
    pub fn document_from_profile(&self, mut profile: Document) -> Document {
        if profile.color_mode != self.color_mode {
            super::colormgmt::set_color_mode(&mut profile, self.color_mode, true, None, super::colormgmt::Grays::Profile);
        }
        let profile_next_id = profile.peek_next_id();
        let mut d = self.document();
        d.swatches = profile.swatches;
        d.swatch_groups = profile.swatch_groups;
        d.graphic_styles = profile.graphic_styles;
        d.char_styles = profile.char_styles;
        d.para_styles = profile.para_styles;
        d.composite_fonts = profile.composite_fonts;
        d.symbols = profile.symbols;
        d.graph_designs = profile.graph_designs;
        d.patterns = profile.patterns;
        d.images = profile.images;
        d.svg_filters = profile.svg_filters;
        d.spot_use_lab = profile.spot_use_lab;
        d.ruler_origin = profile.ruler_origin;
        d.rulers = profile.rulers;
        d.artboard_ruler_origins = profile.artboard_ruler_origins.into_iter().filter(|(id, _)| d.artboards.iter().any(|a| a.id == *id)).collect();
        d.transparency_grid = profile.transparency_grid;
        d.print_setup = profile.print_setup;
        d.color_profiles = profile.color_profiles;
        // The resolution stays the preset's (raster_effects_ppi); the other raster effects settings
        // are the profile's.
        d.raster_effects = profile.raster_effects;
        let setup = std::mem::replace(&mut d.setup, profile.setup);
        d.setup.bleed = setup.bleed;
        d.setup.background = setup.background;
        d.setup.align_new_objects_to_pixel_grid = setup.align_new_objects_to_pixel_grid;
        if let Some(brushes) = profile.unknown.get("brushes") {
            d.unknown.insert("brushes".into(), brushes.clone());
        }
        // Symbol, pattern and graph design art keeps the profile's ids, which a damaged file can
        // have above its counter: the counter goes past all of them, as it does for a loaded
        // document's layers, before the new layer takes an id.
        d.raise_next_id(profile_next_id);
        d.layers = d
            .symbols
            .iter()
            .map(|s| s.art.clone())
            .chain(d.graph_designs.iter().map(|g| g.art.clone()))
            .chain(d.patterns.iter().flat_map(|p| p.art.iter().cloned()))
            .collect();
        d.fix_next_id();
        let layer = d.alloc_id();
        d.layers = vec![Arc::new(Node::layer(layer, "Layer 1", LayerColor::Preset(0)))];
        // Images only the profile's art used stay behind: an image is kept when an object of the new
        // document names it in its `key`, as a native save keeps only the images its objects name.
        let images = std::mem::take(&mut d.images);
        if !images.is_empty() {
            d.images = match serde_json::to_value(&d) {
                Ok(v) => {
                    let used = image_keys(&v);
                    images.into_iter().filter(|(k, _)| used.contains(k.as_str())).collect()
                }
                Err(_) => images,
            };
        }
        d
    }
}

/// The string values of every `key` member at any depth of `v`, which name the images a document's
/// objects use, collected in one pass.
fn image_keys(v: &Value) -> std::collections::HashSet<&str> {
    let mut keys = std::collections::HashSet::new();
    let mut stack = vec![v];
    while let Some(v) = stack.pop() {
        match v {
            Value::Object(m) => {
                for (k, v) in m {
                    match v.as_str() {
                        Some(s) if k == "key" => {
                            keys.insert(s);
                        }
                        _ => stack.push(v),
                    }
                }
            }
            Value::Array(a) => stack.extend(a),
            _ => {}
        }
    }
    keys
}

/// Largest artboard side `file.new` and `artboard.new` accept, points: the span of the canvas
/// ([`crate::MAX_COORD`] from the origin) less the largest bleed on both sides, 3,999,856 pt.
pub(crate) const MAX_SIDE: f64 = crate::MAX_COORD - 2.0 * vectorcraft_doc::setup::MAX_BLEED;

/// Is `side` an artboard side `file.new` and `artboard.new` accept: more than 0 and at most [`MAX_SIDE`]?
pub(crate) fn side_in_range(side: f64) -> bool {
    side > 0.0 && side <= MAX_SIDE
}

/// The error of an artboard side outside [`side_in_range`]: the limit in points, and in `units` as
/// well when they aren't points, rounded down to the decimals [`Unit::number`] shows so the value
/// shown is one the commands take.
pub(crate) fn side_error(cmd: &str, units: Unit) -> crate::EngineError {
    let limit = match units {
        Unit::Points => format!("{MAX_SIDE} pt"),
        u => {
            let step = if matches!(u, Unit::Pixels | Unit::Picas | Unit::Q | Unit::Ha) { 1e3 } else { 1e4 };
            let shown = (u.from_pt(MAX_SIDE) * step).floor() / step;
            format!("{MAX_SIDE} pt ({} {})", u.number(u.to_pt(shown)), u.suffix())
        }
    };
    bad(cmd, format!("width and height must be more than 0 and at most {limit}"))
}
/// Largest artboard spacing, points.
const MAX_SPACING: f64 = 1.0e5;

pub fn color_mode_id(m: ColorMode) -> &'static str {
    match m {
        ColorMode::Rgb => "rgb",
        ColorMode::Cmyk => "cmyk",
    }
}

/// A saved or Recent preset's artboard side: up to [`MAX_SIDE`] points, and 1 when it is 0 or less.
fn de_side<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<f64, D::Error> {
    let v = f64::deserialize(d)?;
    Ok(if v > 0.0 { v.min(MAX_SIDE) } else { 1.0 })
}

/// A saved or Recent preset's artboard count, rounded and kept to 1 to [`MAX_ARTBOARDS`] as
/// `file.new` reads its `artboards` param.
fn de_artboards<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<u32, D::Error> {
    Ok(f64::deserialize(d)?.round().clamp(1.0, MAX_ARTBOARDS as f64) as u32)
}

/// A saved or Recent preset's artboard spacing, kept to 0 to [`MAX_SPACING`] points.
fn de_spacing<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<f64, D::Error> {
    Ok(f64::deserialize(d)?.clamp(0.0, MAX_SPACING))
}

/// A saved or Recent preset's bleed, each side kept to 0 to `MAX_BLEED` points.
fn de_bleed<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<[f64; 4], D::Error> {
    Ok(vectorcraft_doc::setup::bleed_in_range(<[f64; 4]>::deserialize(d)?))
}

/// Top-left corners for artboards of `sizes` from `origin`: `columns` per row, `spacing` apart,
/// each column as wide and each row as tall as its largest artboard; `by_column` fills columns
/// top to bottom first, `rtl` puts the first column at the right.
pub(crate) fn grid_origins(sizes: &[(f64, f64)], origin: Point, columns: usize, spacing: f64, by_column: bool, rtl: bool) -> Vec<Point> {
    let n = sizes.len().max(1);
    let cols = columns.clamp(1, n);
    grid_origins_rc(sizes, origin, (n.div_ceil(cols), cols), spacing, by_column, rtl)
}

/// [`grid_origins`] on a grid of `rows` × `cols` cells (at least enough for every artboard when
/// filled the `by_column` way).
pub(crate) fn grid_origins_rc(
    sizes: &[(f64, f64)],
    origin: Point,
    (rows, cols): (usize, usize),
    spacing: f64,
    by_column: bool,
    rtl: bool,
) -> Vec<Point> {
    let (rows, cols) = (rows.max(1), cols.max(1));
    // Grid cell (row, column) of artboard i.
    let cell = |i: usize| {
        let (r, c) = if by_column { (i % rows, i / rows) } else { (i / cols, i % cols) };
        (r, if rtl { cols - 1 - c.min(cols - 1) } else { c })
    };
    let (mut col_w, mut row_h) = (vec![0.0f64; cols], vec![0.0f64; rows]);
    for (i, (w, h)) in sizes.iter().enumerate() {
        let (r, c) = cell(i);
        if let (Some(cw), Some(rh)) = (col_w.get_mut(c), row_h.get_mut(r)) {
            *cw = cw.max(*w);
            *rh = rh.max(*h);
        }
    }
    let offset = |spans: &[f64], k: usize| spans.iter().take(k).map(|s| s + spacing).sum::<f64>();
    (0..sizes.len())
        .map(|i| {
            let (r, c) = cell(i);
            Point::new(origin.x + offset(&col_w, c), origin.y + offset(&row_h, r))
        })
        .collect()
}

/// The built-in presets: (category, presets as (name, width, height, units)), with the category's
/// colour mode and raster effects resolution. Sizes in `units` (converted to points on use).
type Category = (&'static str, ColorMode, f64, &'static [(&'static str, f64, f64, Unit)]);

const PX: Unit = Unit::Pixels;
const PT: Unit = Unit::Points;
const MM: Unit = Unit::Millimeters;
const IN: Unit = Unit::Inches;

pub const CATEGORIES: &[Category] = &[
    (
        "Mobile",
        ColorMode::Rgb,
        72.0,
        &[
            ("Phone 390×844", 390.0, 844.0, PX),
            ("Phone 393×852", 393.0, 852.0, PX),
            ("Phone 430×932", 430.0, 932.0, PX),
            ("Phone 360×800", 360.0, 800.0, PX),
            ("Phone 412×915", 412.0, 915.0, PX),
            ("Phone 375×667", 375.0, 667.0, PX),
            ("Tablet 768×1024", 768.0, 1024.0, PX),
            ("Tablet 820×1180", 820.0, 1180.0, PX),
            ("Tablet 834×1194", 834.0, 1194.0, PX),
            ("Tablet 1024×1366", 1024.0, 1366.0, PX),
            ("Tablet 800×1280", 800.0, 1280.0, PX),
            ("Watch 184×224", 184.0, 224.0, PX),
            ("Watch 198×242", 198.0, 242.0, PX),
        ],
    ),
    (
        "Web",
        ColorMode::Rgb,
        72.0,
        &[
            ("Web 1920×1080", 1920.0, 1080.0, PX),
            ("Web 1600×900", 1600.0, 900.0, PX),
            ("Web 1440×900", 1440.0, 900.0, PX),
            ("Web 1366×768", 1366.0, 768.0, PX),
            ("Web 1280×800", 1280.0, 800.0, PX),
            ("Web 1024×768", 1024.0, 768.0, PX),
        ],
    ),
    (
        "Print",
        ColorMode::Cmyk,
        300.0,
        &[
            ("Letter", 612.0, 792.0, PT),
            ("Legal", 612.0, 1008.0, PT),
            ("Tabloid", 792.0, 1224.0, PT),
            ("A3", 297.0, 420.0, MM),
            ("A4", 210.0, 297.0, MM),
            ("A5", 148.0, 210.0, MM),
            ("B4", 250.0, 353.0, MM),
            ("B5", 176.0, 250.0, MM),
        ],
    ),
    (
        "Film & Video",
        ColorMode::Rgb,
        72.0,
        &[
            ("HDTV 1080p", 1920.0, 1080.0, PX),
            ("HDTV 720p", 1280.0, 720.0, PX),
            ("4K UHD", 3840.0, 2160.0, PX),
            ("8K UHD", 7680.0, 4320.0, PX),
            ("2K Cinema", 2048.0, 1080.0, PX),
            ("4K Cinema", 4096.0, 2160.0, PX),
        ],
    ),
    (
        "Art & Illustration",
        ColorMode::Rgb,
        300.0,
        &[
            ("Postcard", 4.0, 6.0, IN),
            ("Greeting Card", 5.0, 7.0, IN),
            ("Square 8 × 8 in", 8.0, 8.0, IN),
            ("Poster 18 × 24 in", 18.0, 24.0, IN),
            ("Poster 24 × 36 in", 24.0, 36.0, IN),
        ],
    ),
    (
        "Branding",
        ColorMode::Cmyk,
        300.0,
        &[
            ("Logo 1000×1000", 1000.0, 1000.0, PX),
            ("Business Card 3.5 × 2 in", 3.5, 2.0, IN),
            ("Business Card 85 × 55 mm", 85.0, 55.0, MM),
            ("Business Card 91 × 55 mm", 91.0, 55.0, MM),
            ("Letterhead Letter", 8.5, 11.0, IN),
            ("Letterhead A4", 210.0, 297.0, MM),
            ("Envelope DL", 220.0, 110.0, MM),
            ("Envelope #10", 9.5, 4.125, IN),
        ],
    ),
    (
        "Social",
        ColorMode::Rgb,
        72.0,
        &[
            ("Social Square Post 1080×1080", 1080.0, 1080.0, PX),
            ("Social Portrait Post 1080×1350", 1080.0, 1350.0, PX),
            ("Social Landscape Post 1200×630", 1200.0, 630.0, PX),
            ("Social Story 1080×1920", 1080.0, 1920.0, PX),
            ("Social Tall Post 1000×1500", 1000.0, 1500.0, PX),
            ("Profile Cover 1500×500", 1500.0, 500.0, PX),
            ("Page Cover 1640×624", 1640.0, 624.0, PX),
            ("Video Banner 2560×1440", 2560.0, 1440.0, PX),
            ("Video Thumbnail 1280×720", 1280.0, 720.0, PX),
        ],
    ),
];

/// The built-in categories whose presets start with Align New Objects to Pixel Grid on
/// (<https://helpx.adobe.com/illustrator/desktop/get-started/learn-the-basics/pixel-preview-mode-for-web-graphics.html>).
const PIXEL_ALIGNED_CATEGORIES: [&str; 2] = ["Mobile", "Web"];

/// The category tabs in order: Recent, Saved, then the built-in categories.
pub fn category_names() -> impl Iterator<Item = &'static str> {
    ["Recent", "Saved"].into_iter().chain(CATEGORIES.iter().map(|c| c.0))
}

/// The built-in presets of one category: screen sizes in pixels, the others in `general`
/// (Preferences ▸ Units ▸ General).
fn builtin(c: &Category, general: Unit) -> impl Iterator<Item = DocSettings> + '_ {
    let (category, color_mode, ppi, presets) = *c;
    let align = PIXEL_ALIGNED_CATEGORIES.contains(&category);
    presets.iter().map(move |&(name, w, h, units)| DocSettings {
        name: name.into(),
        width: units.to_pt(w),
        height: units.to_pt(h),
        units: if units == Unit::Pixels { units } else { general },
        color_mode,
        raster_effects_ppi: ppi,
        align_new_objects_to_pixel_grid: align,
        profile: Some(category.into()),
        ..DocSettings::default()
    })
}

/// The presets of the category `name` (any case), or None for no such category.
pub fn category(s: &Session, name: &str) -> Option<Vec<DocSettings>> {
    if name.eq_ignore_ascii_case("Recent") {
        return Some(s.prefs.recent_new_docs.clone());
    }
    if name.eq_ignore_ascii_case("Saved") {
        return Some(s.prefs.new_doc_presets.clone());
    }
    CATEGORIES.iter().find(|c| c.0.eq_ignore_ascii_case(name)).map(|c| builtin(c, s.default_units()).collect())
}

/// A preset by name (any case): a saved one first, then the built-in ones.
pub fn find(s: &Session, name: &str) -> Option<DocSettings> {
    let name = name.trim();
    s.prefs
        .new_doc_presets
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .cloned()
        .or_else(|| CATEGORIES.iter().flat_map(|c| builtin(c, s.default_units())).find(|p| p.name.eq_ignore_ascii_case(name)))
}

impl Session {
    /// New Document's starting settings: Letter in Preferences ▸ Units ▸ General.
    fn default_settings(&self) -> DocSettings {
        DocSettings { units: self.default_units(), ..DocSettings::default() }
    }
}

/// Recent keeps this many sizes.
const RECENT: usize = 12;

/// `file.new`: a new document from a preset and/or explicit settings.
pub(crate) fn file_new(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "file.new";
    let base = match str_param(p, "preset") {
        Some(name) => find(s, name).ok_or_else(|| bad(C, format!("no preset called `{name}` (see file.newPresets)")))?,
        None => s.default_settings(),
    };
    // A preset keeps its name in Recent unless its size changed (turning it doesn't count).
    let sorted = |d: &DocSettings| (d.width.min(d.height), d.width.max(d.height), d.units);
    let size = str_param(p, "preset").map(|_| sorted(&base));
    let mut settings = base.with_params(p, C)?;
    if size != Some(sorted(&settings)) {
        settings.name = "Custom".into();
    }
    let (profile, profile_error) = match settings.profile.as_deref() {
        Some(name) => match s.read_profile(name) {
            Ok(p) => (p, None),
            Err(e) => (None, Some(e)),
        },
        None => (None, None),
    };
    let applied = profile.is_some();
    let (mut d, links_from, warnings) = match profile {
        Some(p) => (settings.document_from_profile(p.doc), Some(p.file), p.warnings),
        None => (settings.document(), None, vec![]),
    };
    if let Some(defaults) = &s.prefs.default_swatches {
        super::swatchdefaults::apply(&mut d, defaults)?;
    }
    d.grid = s.prefs.grid();
    d.title = match str_param(p, "name").or(str_param(p, "title")).map(str::trim).filter(|t| !t.is_empty()) {
        // New Document offers the next untitled name: taking it uses it up.
        Some(t) if t == s.peek_untitled() => s.next_untitled(),
        Some(t) => t.to_string(),
        None => s.next_untitled(),
    };
    d.metadata.created = clock_date(s, "created", date_param(p, "created", C)?);
    let i = s.add_document(d, None);
    // The profile's links keep their relative paths, which name places from the profile's file, as
    // a new document from a history state keeps its source's.
    s.doc_mut()?.links_from = links_from;
    // Overprint Preview is the engine's (the UI turns Pixel Preview on from `previewMode`). Both
    // are views of the whole app, so Default leaves them as they are.
    if settings.preview_mode == PreviewMode::Overprint {
        s.execute("view.overprintPreview", &json!({ "on": true }))?;
    }
    remember(s, &settings);
    let mut out = json!({ "index": i, "previewMode": settings.preview_mode.id() });
    if applied {
        out["profile"] = json!(settings.profile);
    }
    if !warnings.is_empty() {
        out["warnings"] = json!(warnings);
    }
    if let Some(e) = profile_error {
        out["profileError"] = json!({ "profile": settings.profile, "reason": e });
    }
    Ok(out)
}

/// Put `settings` first in Recent (one entry per size and units).
fn remember(s: &mut Session, settings: &DocSettings) {
    let recent = &mut s.prefs.recent_new_docs;
    recent.retain(|r| (r.width, r.height, r.units) != (settings.width, settings.height, settings.units));
    recent.insert(0, settings.clone());
    recent.truncate(RECENT);
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let names: Vec<&str> = match str_param(p, "category") {
        Some(c) => vec![
            category_names()
                .find(|n| n.eq_ignore_ascii_case(c))
                .ok_or_else(|| bad("file.newPresets", format!("unknown category `{c}` ({})", category_names().collect::<Vec<_>>().join(", "))))?,
        ],
        None => category_names().collect(),
    };
    let categories: Vec<Value> = names
        .into_iter()
        .map(|n| json!({ "name": n, "presets": category(s, n).unwrap_or_default().iter().map(DocSettings::to_json).collect::<Vec<_>>() }))
        .collect();
    Ok(json!({ "categories": categories }))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "file.newPresets.save";
    let name = str_param(p, "name")
        .map(str::trim)
        .filter(|n| !n.is_empty() && n.chars().count() <= 64)
        .ok_or_else(|| bad(C, "give the preset a name (1–64 characters)"))?;
    let base = match str_param(p, "preset") {
        Some(from) => find(s, from).ok_or_else(|| bad(C, format!("no preset called `{from}` (see file.newPresets)")))?,
        None => s.default_settings(),
    };
    let mut preset = base.with_params(p, C)?;
    preset.name = name.to_string();
    let saved = &mut s.prefs.new_doc_presets;
    match saved.iter_mut().find(|q| q.name.eq_ignore_ascii_case(name)) {
        Some(q) => *q = preset,
        None => saved.push(preset),
    }
    Ok(json!({ "name": name, "count": saved.len() }))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "file.newPresets.delete";
    let name = str_param(p, "name").unwrap_or("").trim();
    let saved = &mut s.prefs.new_doc_presets;
    let i = saved.iter().position(|q| q.name.eq_ignore_ascii_case(name)).ok_or_else(|| bad(C, format!("no saved preset called `{name}`")))?;
    Ok(json!({ "deleted": saved.remove(i).name }))
}

/// The largest New Document profile file `file.new` reads, bytes (VectorCraft's own limit); a
/// larger one is reported and the document starts with the defaults.
pub const MAX_PROFILE_BYTES: u64 = 64 << 20;

/// `file.new`'s reason for a profile when the request being served may not read the profiles
/// folder: the folder's path is left out.
const PROFILE_DENIED: &str = "the request may not read the New Document profiles folder";

/// A profile's file as [`Session::read_profile`] read it.
struct ReadProfile {
    doc: Document,
    /// The file's path, which its links' relative paths name places from.
    file: String,
    /// A warning for each color profile the file carries that can't be used.
    warnings: Vec<String>,
}

/// The New Document profiles' names: the built-in categories, in tab order.
pub fn profile_names() -> impl Iterator<Item = &'static str> {
    CATEGORIES.iter().map(|c| c.0)
}

/// The profile `v` names (a built-in category, any case).
fn profile_name(v: &Value, cmd: &str) -> Result<&'static str> {
    let name = v.as_str().map(str::trim).unwrap_or_default();
    profile_names().find(|c| c.eq_ignore_ascii_case(name)).ok_or_else(|| {
        let shown = v.as_str().map_or_else(|| v.to_string(), str::to_string);
        bad(cmd, format!("no New Document profile called `{shown}` ({})", profile_names().collect::<Vec<_>>().join(", ")))
    })
}

impl Session {
    /// Keep the New Document profiles in the folder `dir` (`New Document Profiles` next to the
    /// preferences). The folder is made when a profile is first edited.
    pub fn attach_new_doc_profiles(&mut self, dir: std::path::PathBuf) {
        self.new_doc_profile_dir = Some(dir);
    }

    /// The New Document profiles' folder ([`Session::attach_new_doc_profiles`]); none on the web.
    pub fn new_doc_profile_dir(&self) -> Option<&std::path::Path> {
        self.new_doc_profile_dir.as_deref().filter(|_| !cfg!(target_arch = "wasm32"))
    }

    /// The file of profile `name`, whether or not it exists.
    fn profile_path(&self, name: &str) -> Option<std::path::PathBuf> {
        Some(self.new_doc_profile_dir()?.join(format!("{name}.{}", vectorcraft_format::EXTENSION)))
    }

    /// Whether profile `name` has a file, which `file.newProfiles.reset` deletes.
    pub fn new_doc_profile_customized(&self, name: &str) -> bool {
        self.profile_path(name).is_some_and(|p| p.is_file())
    }

    /// Profile `name`'s document, its file's path and a warning for each color profile the file
    /// carries that can't be used: `None` when it has no file (the shipped profile, the defaults),
    /// the reason when the request being served may not read it, or its file can't be read, is
    /// larger than [`MAX_PROFILE_BYTES`] or isn't a native document VectorCraft reads.
    fn read_profile(&self, name: &str) -> std::result::Result<Option<ReadProfile>, String> {
        let Some(path) = self.profile_path(name) else { return Ok(None) };
        let file = path.to_string_lossy().to_string();
        // Checked before the file is looked for, so a request that may not read the folder learns
        // nothing of its files, nor where the folder is.
        let denied = |_| PROFILE_DENIED.to_string();
        crate::file_access::check_read(&file).map_err(denied)?;
        if std::fs::symlink_metadata(&path).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound) {
            return Ok(None);
        }
        crate::file_access::check_read(&file).map_err(denied)?;
        let bytes = crate::store::read_capped(&path, MAX_PROFILE_BYTES)?;
        // The color profiles the file carries are installed where missing, as opening it installs
        // them, so the new document's assigned profiles draw and are saved with it.
        let (mut doc, _, warnings) = super::fileio::native_file(&bytes).map_err(|e| e.to_string())?;
        // Linked images in its symbols, patterns and brushes are read from their files, as placing
        // a document reads them.
        super::links::resolve(&mut doc, super::links::LinkBase::file(Some(&file)), false, false);
        Ok(Some(ReadProfile { doc, file, warnings }))
    }
}

/// The shipped profile of category `name`: the defaults, as a document of the category's first
/// preset.
fn shipped_profile(s: &Session, name: &str) -> Document {
    let first = CATEGORIES.iter().find(|c| c.0 == name).and_then(|c| builtin(c, s.default_units()).next());
    let mut d = first.unwrap_or_default().document();
    d.title = name.to_string();
    d
}

/// `path` when the request being served may read it.
fn shown(path: &std::path::Path) -> Option<String> {
    let path = path.to_string_lossy().to_string();
    crate::file_access::check_read(&path).is_ok().then_some(path)
}

fn list_profiles(s: &mut Session, _: &Value) -> Result<Value> {
    let profiles: Vec<Value> = profile_names()
        .map(|name| {
            let mut v = json!({ "name": name });
            match s.profile_path(name) {
                // A request that may not read the folder learns nothing of its files.
                Some(path) => {
                    if let Some(p) = shown(&path) {
                        v["customized"] = json!(path.is_file());
                        v["path"] = json!(p);
                    }
                }
                None => v["customized"] = json!(false),
            }
            v
        })
        .collect();
    Ok(json!({ "profiles": profiles, "folder": s.new_doc_profile_dir().and_then(shown) }))
}

/// `msg`, an error about the profile file at `path`, as the request being served may see it: the
/// file named whole inside the read roots, else by its file name.
fn profile_error(msg: String, path: &str) -> super::EngineError {
    super::fileio::naming_in_error(super::EngineError::Other(msg), path)
}

/// The file of profile `name` for `cmd` to change, checked against the request's write access.
fn writable_profile(s: &Session, name: &str, cmd: &str) -> Result<std::path::PathBuf> {
    let path = s.profile_path(name).ok_or_else(|| bad(cmd, "New Document profiles need the preferences folder, which this run doesn't have"))?;
    let file = path.to_string_lossy();
    crate::file_access::check_write(&file).map_err(|e| profile_error(e, &file))?;
    Ok(path)
}

fn edit_profile(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "file.newProfiles.edit";
    let name = profile_name(p.get("name").unwrap_or(&Value::Null), C)?;
    let file = writable_profile(s, name, C)?;
    let path = file.to_string_lossy().to_string();
    // document.open reads it next: a profile it can't open is not written either.
    crate::file_access::check_read(&path).map_err(|e| profile_error(e, &path))?;
    if !file.exists() {
        let bytes = vectorcraft_format::save(&shipped_profile(s, name), false);
        file.parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&file, bytes))
            .map_err(|e| profile_error(format!("{path}: {e}"), &path))?;
    }
    s.execute("document.open", &json!({ "path": path }))
}

fn reset_profile(s: &mut Session, p: &Value) -> Result<Value> {
    const C: &str = "file.newProfiles.reset";
    let name = profile_name(p.get("name").unwrap_or(&Value::Null), C)?;
    let file = writable_profile(s, name, C)?;
    let path = file.to_string_lossy().to_string();
    let reset = match std::fs::remove_file(&file) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(profile_error(format!("{path}: {e}"), &path)),
    };
    Ok(json!({ "name": name, "reset": reset }))
}
