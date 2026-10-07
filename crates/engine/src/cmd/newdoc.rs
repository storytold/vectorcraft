//! File → New: the settings `file.new` takes, the New Document presets (generated in code, by
//! category), the user's saved presets and the recently used sizes (both kept in the
//! preferences), and the artboard grid layout shared with Rearrange All Artboards.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use vectorcraft_doc::{Artboard, Background, ColorMode, Document, Unit};
use vectorcraft_geom::{Point, Rect};

use super::*;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "file.newPresets",
            "New Document Presets",
            [],
            None,
            "{category?: \"Recent\"|\"Saved\"|\"Mobile\"|\"Web\"|\"Print\"|\"Film & Video\"|\"Art & Illustration\"|\"Branding\"|\"Social\"} the New Document presets by category (Recent: the last sizes used; Saved: the user's presets) → {categories: [{name, presets: [{name, size: \"1920 × 1080 px\", width: pt, height: pt, units, orientation, artboards, artboardLayout: {layout, columns, spacing, rightToLeft}, bleed, backgroundContents, colorMode, rasterEffectsPpi, previewMode}]}]}; file.new {preset: name} starts from one",
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

/// Everything New Document sets: a preset (built-in, saved or recent) or `file.new`'s params.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DocSettings {
    /// The preset's name.
    pub name: String,
    /// Artboard size in points.
    pub width: f64,
    pub height: f64,
    pub units: Unit,
    pub artboards: u32,
    pub layout: ArtboardLayout,
    /// Columns of a grid layout.
    pub columns: u32,
    /// Space between artboards, points.
    pub spacing: f64,
    /// Change to Right-to-Left Layout: the first artboard at the right.
    pub right_to_left: bool,
    /// Top, bottom, left, right, points.
    pub bleed: [f64; 4],
    pub background: Background,
    pub color_mode: ColorMode,
    pub raster_effects_ppi: f64,
    pub preview_mode: PreviewMode,
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
        if !(self.width > 0.0 && self.height > 0.0) || self.width > MAX_SIDE || self.height > MAX_SIDE {
            return Err(bad(cmd, "width/height must be positive and within the canvas"));
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
        d.artboards = self
            .boards()
            .into_iter()
            .enumerate()
            .map(|(i, rect)| Artboard {
                id: i as u32 + 1,
                name: format!("Artboard {}", i + 1),
                rect,
                show_center_mark: false,
                show_cross_hairs: false,
                ..Default::default()
            })
            .collect();
        d
    }
}

/// Largest artboard side `file.new` accepts, points.
const MAX_SIDE: f64 = 16383.0 * 10.0;
/// Largest artboard spacing, points.
const MAX_SPACING: f64 = 1.0e5;

pub fn color_mode_id(m: ColorMode) -> &'static str {
    match m {
        ColorMode::Rgb => "rgb",
        ColorMode::Cmyk => "cmyk",
    }
}

/// Top-left corners for artboards of `sizes` from `origin`: `columns` per row, `spacing` apart,
/// each column as wide and each row as tall as its largest artboard; `by_column` fills columns
/// top to bottom first, `rtl` puts the first column at the right.
pub(crate) fn grid_origins(sizes: &[(f64, f64)], origin: Point, columns: usize, spacing: f64, by_column: bool, rtl: bool) -> Vec<Point> {
    let n = sizes.len();
    let cols = columns.clamp(1, n.max(1));
    let rows = n.div_ceil(cols).max(1);
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
    (0..n)
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

/// The category tabs in order: Recent, Saved, then the built-in categories.
pub fn category_names() -> impl Iterator<Item = &'static str> {
    ["Recent", "Saved"].into_iter().chain(CATEGORIES.iter().map(|c| c.0))
}

/// The built-in presets of one category: screen sizes in pixels, the others in `general`
/// (Preferences ▸ Units ▸ General).
fn builtin(c: &Category, general: Unit) -> impl Iterator<Item = DocSettings> + '_ {
    let (_, color_mode, ppi, presets) = *c;
    presets.iter().map(move |&(name, w, h, units)| DocSettings {
        name: name.into(),
        width: units.to_pt(w),
        height: units.to_pt(h),
        units: if units == Unit::Pixels { units } else { general },
        color_mode,
        raster_effects_ppi: ppi,
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
    let mut d = settings.document();
    d.title = match str_param(p, "name").or(str_param(p, "title")).map(str::trim).filter(|t| !t.is_empty()) {
        // New Document offers the next untitled name: taking it uses it up.
        Some(t) if t == s.peek_untitled() => s.next_untitled(),
        Some(t) => t.to_string(),
        None => s.next_untitled(),
    };
    d.metadata.created = clock_date(s, "created", date_param(p, "created", C)?);
    let i = s.add_document(d, None);
    // Overprint Preview is the engine's (the UI turns Pixel Preview on from `previewMode`). Both
    // are views of the whole app, so Default leaves them as they are.
    if settings.preview_mode == PreviewMode::Overprint {
        s.execute("view.overprintPreview", &json!({ "on": true }))?;
    }
    remember(s, &settings);
    Ok(json!({ "index": i, "previewMode": settings.preview_mode.id() }))
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
