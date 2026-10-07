//! VectorCraft document model: artboards, layers and objects, appearance, swatches (pure data + serde).
//!
//! Documents are persistent trees: children are `Arc<Node>`, and edits go through
//! [`Document::node_mut`], which clones only the nodes on the path from the root to the edited node.
//! Keeping the previous `Document` value around is therefore a cheap undo snapshot.
#![forbid(unsafe_code)]

pub mod appearance;
pub mod assets;
pub mod blend;
pub mod clipnest;
pub mod graph;
pub mod hit;
pub mod inks;
pub mod links;
pub mod live;
pub mod marks;
pub mod metadata;
pub mod node;
pub mod orient;
pub mod overprint;
pub mod pattern;
pub mod perspective;
mod pixels;
pub mod profiles;
pub mod puppet;
pub mod range;
pub mod rastersettings;
mod reach;
pub mod recolor;
pub mod selection;
pub mod setup;
pub mod slices;
pub mod style_libs;
pub mod swatches;
pub mod text;

use std::collections::BTreeMap;
use std::sync::Arc;

/// `skip_serializing_if` predicates: fields equal to their serde default are not written.
pub(crate) mod skip {
    pub fn is_default<T: Default + PartialEq>(v: &T) -> bool {
        *v == T::default()
    }
    pub fn is_true(v: &bool) -> bool {
        *v
    }
    pub fn is_one(v: &f32) -> bool {
        *v == 1.0
    }
}

pub use appearance::StrokeGradientMode;
pub use appearance::{
    Appearance, AppearanceItem, ArrowAlign, Arrowhead, Dash, Effect, FillLayer, LineCap, LineJoin, ProfilePreset, SavedProfile, StrokeAlign,
    StrokeLayer, WidthProfile,
};
pub use assets::ExportAsset;
pub use graph::{GraphKind, GraphSpec};
pub use hit::{Hit, HitKind};
pub use links::{LinkInfo, PlacementOptions};
pub use live::{BlendOrientation, BlendSpacing, BlendSpec, EnvelopeKind, GradientMesh, MeshPoint};
pub use metadata::{CopyrightStatus, DocMetadata};
pub use node::Knockout;
pub use node::Scaling;
pub use node::{ImageMap, ObjectAttributes};
pub use node::{ImageObject, LAYER_COLORS, LayerColor, LiveShape, Node, NodeId, NodeKind, OpacityMask};
pub use orient::OrientedBox;
pub use pattern::{Overlap, PatternDef, PatternEdit, RepeatKind, RepeatSpec, TileType};
pub use perspective::PerspectiveAttachment;
pub use profiles::ColorProfiles;
pub use puppet::{PuppetPin, PuppetPins};
pub use rastersettings::{RasterColorModel, RasterEffectsSettings};
pub use selection::{AnchorRef, Selection};
pub use setup::{Background, DocSetup, ExportText, GridSize, Quotes};
pub use slices::{CellAlign, CellVAlign, Slice, SliceArea, SliceKind, SliceOptions, SliceSource};
pub use style_libs::StyleLibrary;
pub use text::{
    AreaOptions, CharPosition, CharStyle, FirstBaseline, Justify, ParaStyle, PathEffect, ScriptMetrics, TabAlign, TabStop, TextKind, TextObject,
    TextRun, TextStyleDef, TextWrap, WrapShape,
};
pub use vectorcraft_color as color;
pub use vectorcraft_geom as geom;

use serde::{Deserialize, Serialize};
use vectorcraft_color::{Swatch, SwatchGroup};
use vectorcraft_geom::{Point, Rect};

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum DocError {
    #[error("no such object {0}")]
    NoNode(NodeId),
    #[error("object {0} cannot have children")]
    NotContainer(NodeId),
    #[error("{0}")]
    Invalid(String),
}

/// Measurement units (display only; the model is always in points).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unit {
    #[default]
    Points,
    Picas,
    Inches,
    Millimeters,
    Centimeters,
    Pixels,
    FeetInches,
    Meters,
    Yards,
    Feet,
}

impl Unit {
    pub const ALL: [Unit; 10] = [
        Unit::Points,
        Unit::Picas,
        Unit::Inches,
        Unit::Millimeters,
        Unit::Centimeters,
        Unit::Pixels,
        Unit::FeetInches,
        Unit::Meters,
        Unit::Yards,
        Unit::Feet,
    ];
    /// Points per unit.
    pub fn points(self) -> f64 {
        match self {
            Unit::Points | Unit::Pixels => 1.0,
            Unit::Picas => 12.0,
            Unit::Inches => 72.0,
            Unit::Millimeters => 72.0 / 25.4,
            Unit::Centimeters => 72.0 / 2.54,
            Unit::Meters => 72.0 / 0.0254,
            Unit::Yards => 72.0 * 36.0,
            Unit::Feet | Unit::FeetInches => 72.0 * 12.0,
        }
    }
    pub fn suffix(self) -> &'static str {
        match self {
            Unit::Points => "pt",
            Unit::Picas => "p",
            Unit::Inches => "in",
            Unit::Millimeters => "mm",
            Unit::Centimeters => "cm",
            Unit::Pixels => "px",
            Unit::FeetInches | Unit::Feet => "ft",
            Unit::Meters => "m",
            Unit::Yards => "yd",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Unit::Points => "Points",
            Unit::Picas => "Picas",
            Unit::Inches => "Inches",
            Unit::Millimeters => "Millimeters",
            Unit::Centimeters => "Centimeters",
            Unit::Pixels => "Pixels",
            Unit::FeetInches => "Feet & Inches",
            Unit::Meters => "Meters",
            Unit::Yards => "Yards",
            Unit::Feet => "Feet",
        }
    }
    pub fn from_pt(self, v: f64) -> f64 {
        v / self.points()
    }
    pub fn to_pt(self, v: f64) -> f64 {
        v * self.points()
    }
    /// Format a point value in this unit the way fields show it (e.g. `12.5 pt`, `3 in`).
    pub fn format(self, pt: f64) -> String {
        format!("{} {}", self.number(pt), self.suffix())
    }
    /// [`Unit::format`] without the suffix (`12.5`), for narrow fields.
    pub fn number(self, pt: f64) -> String {
        let s = format!("{:.3}", self.from_pt(pt));
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" { "0".into() } else { s.into() }
    }
    /// Parse `12`, `12pt`, `1in`, `3 mm`, `2p6` (picas+points), simple `+ - * /` arithmetic.
    ///
    /// A unit after a `*` or `/` operand measures the whole expression (`1080/2 px` is 540 px), so
    /// math typed before a field's unit suffix works.
    pub fn parse(self, s: &str) -> Option<f64> {
        parse_measure(s, self).filter(|v| v.is_finite())
    }
    /// The value naming this unit in the Units preferences (`points`, `millimeters`,
    /// `feetInches`).
    pub fn key(self) -> &'static str {
        match self {
            Unit::Points => "points",
            Unit::Picas => "picas",
            Unit::Inches => "inches",
            Unit::Millimeters => "millimeters",
            Unit::Centimeters => "centimeters",
            Unit::Pixels => "pixels",
            Unit::FeetInches => "feetInches",
            Unit::Meters => "meters",
            Unit::Yards => "yards",
            Unit::Feet => "feet",
        }
    }
    /// The unit `name` names: its label, suffix or [`Unit::key`] in any case, ignoring spaces and
    /// punctuation (`Millimeters`, `mm`, `Feet & Inches`, `feetInches`).
    pub fn named(name: &str) -> Option<Unit> {
        let letters = |s: &'static str| s.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase());
        let given = || name.chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase());
        Unit::ALL.into_iter().find(|u| letters(u.label()).eq(given()) || u.suffix().chars().eq(given()))
    }
    /// A point value as the canvas measurement labels show it: two decimals and the suffix
    /// (`12.50 mm`).
    pub fn readout(self, pt: f64) -> String {
        format!("{:.2} {}", self.from_pt(pt), self.suffix())
    }
}

fn parse_measure(s: &str, default: Unit) -> Option<f64> {
    let s = s.trim();
    // Simple arithmetic on the right (Illustrator fields accept "10+5", "100/2", "3in*2").
    for op in ['+', '-', '*', '/'] {
        // Past the first character, so a leading sign isn't an operator.
        let skip = s.chars().next().map_or(0, char::len_utf8);
        if let Some(i) = s[skip..].rfind(op).map(|i| i + skip) {
            let (l, r) = (&s[..i], &s[i + 1..]);
            if l.trim().is_empty() {
                continue;
            }
            return match op {
                '+' => Some(parse_measure(l, default)? + parse_measure(r, default)?),
                '-' => Some(parse_measure(l, default)? - parse_measure(r, default)?),
                _ => {
                    // A factor or divisor is a plain number; a unit after it is the expression's.
                    let (k, unit) = number_unit(r)?;
                    let a = parse_measure(l, unit.unwrap_or(default))?;
                    if op == '*' {
                        Some(a * k)
                    } else if k == 0.0 {
                        None
                    } else {
                        Some(a / k)
                    }
                }
            };
        }
    }
    if let Some((p, pt)) = s.split_once('p')
        && !p.is_empty()
        && p.trim().parse::<f64>().is_ok()
        && (pt.is_empty() || pt.trim().parse::<f64>().is_ok())
        && !s.ends_with("pt")
        && !s.ends_with("px")
    {
        return Some(p.trim().parse::<f64>().ok()? * 12.0 + pt.trim().parse::<f64>().unwrap_or(0.0));
    }
    let (v, unit) = number_unit(s)?;
    Some(unit.unwrap_or(default).to_pt(v))
}

/// A number with an optional unit suffix (`12`, `3 mm`, `2in`) → (the number, its unit).
fn number_unit(s: &str) -> Option<(f64, Option<Unit>)> {
    let s = s.trim();
    let num_end = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(s.len());
    let v: f64 = s.get(..num_end)?.trim().parse().ok()?;
    let unit = match s.get(num_end..)?.trim() {
        "" => None,
        "pt" => Some(Unit::Points),
        "px" => Some(Unit::Pixels),
        "in" | "\"" => Some(Unit::Inches),
        "mm" => Some(Unit::Millimeters),
        "cm" => Some(Unit::Centimeters),
        "m" => Some(Unit::Meters),
        "ft" | "'" => Some(Unit::Feet),
        "yd" => Some(Unit::Yards),
        "p" | "pc" => Some(Unit::Picas),
        _ => return None,
    };
    Some((v, unit))
}

/// A unitless field value (percent, degrees, counts) with the same `+ - * /` arithmetic as
/// [`Unit::parse`] (`45*2`, `100/3`); text other than digits, `.` and operators reads as nothing.
pub fn parse_number(s: &str) -> Option<f64> {
    if !s.chars().all(|c| c.is_ascii_digit() || " .+-*/".contains(c)) {
        return None;
    }
    // A leading `+` is a sign (`+5`), as a plain number read it.
    let s = s.trim();
    Unit::Points.parse(s.strip_prefix('+').unwrap_or(s))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorMode {
    #[default]
    Rgb,
    Cmyk,
}

impl ColorMode {
    /// The colour model new colours take in a document of this mode.
    pub fn model(self) -> vectorcraft_color::cms::Model {
        match self {
            ColorMode::Rgb => vectorcraft_color::cms::Model::Rgb,
            ColorMode::Cmyk => vectorcraft_color::cms::Model::Cmyk,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Artboard {
    pub id: u32,
    pub name: String,
    /// Artboard rectangle in document points.
    pub rect: Rect,
    #[serde(default)]
    pub show_center_mark: bool,
    #[serde(default)]
    pub show_cross_hairs: bool,
    /// The artboard's own background colour, painted behind its art on screen and in export
    /// (`None`: the paper, transparent in export).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<vectorcraft_color::Color>,
    /// Locked: the Artboard tool can't move or resize it, and its art was locked with it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// The objects locking the artboard locked (unlocking it unlocks just these, so art that
    /// was locked before stays locked).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub locked_art: Vec<NodeId>,
}

/// Opacity-mask editing mode: `object`'s mask art lives on the temporary `layer` while editing.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskEdit {
    pub object: NodeId,
    pub layer: NodeId,
}

/// A saved view (View → New View…): zoom, centre and rotation, listed at the bottom of the View menu.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedView {
    pub name: String,
    pub center: Point,
    pub zoom: f64,
    #[serde(default)]
    pub rotation: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    /// true = vertical guide at `pos` (x), false = horizontal at `pos` (y).
    pub vertical: bool,
    pub pos: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GridPrefs {
    pub spacing: f64,
    pub subdivisions: u32,
}

impl Default for GridPrefs {
    fn default() -> Self {
        Self { spacing: 72.0, subdivisions: 8 }
    }
}

/// A named graphic style (Graphic Styles panel): an appearance plus the object's transparency.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GraphicStyle {
    pub name: String,
    pub appearance: Appearance,
    /// Stable id linked objects refer to ([`Node::graphic_style`]); 0 until first needed (styles
    /// from older files get one when an object is first linked to them).
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub id: u32,
    #[serde(default = "one", skip_serializing_if = "skip::is_one")]
    pub opacity: f32,
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub blend: vectorcraft_color::BlendMode,
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub isolate: bool,
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub knockout: Knockout,
    /// Placed gradients are stored relative to the unit box (0, 0)–(1, 1), so applying the style
    /// places them on each object's own bounds. Styles saved before this kept document
    /// coordinates (false).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unit_box: bool,
}

fn one() -> f32 {
    1.0
}

/// The name of the style new documents list first (Sort by Name keeps it first).
pub const DEFAULT_GRAPHIC_STYLE: &str = "Default Graphic Style";

impl GraphicStyle {
    /// The box a style's placed gradients are stored relative to.
    pub const UNIT_BOX: Rect = Rect::new(0.0, 0.0, 1.0, 1.0);

    /// A style with `appearance` (its placed gradients, if any, in unit-box space) and default
    /// transparency (no id yet).
    pub fn new(name: impl Into<String>, appearance: Appearance) -> Self {
        Self {
            name: name.into(),
            appearance,
            id: 0,
            opacity: 1.0,
            blend: Default::default(),
            isolate: false,
            knockout: Knockout::Neutral,
            unit_box: true,
        }
    }
    /// A style capturing `n`'s transparency and `appearance` (in unit-box space).
    pub fn of(name: impl Into<String>, appearance: Appearance, n: &Node) -> Self {
        Self { opacity: n.opacity, blend: n.blend, isolate: n.isolate, knockout: n.knockout, ..Self::new(name, appearance) }
    }
    /// The style's appearance as object `n` takes it: placed gradients stored in unit-box space
    /// land at the same place relative to `n`'s geometric bounds.
    pub fn appearance_on(&self, n: &Node) -> std::borrow::Cow<'_, Appearance> {
        let placed = self.unit_box && self.appearance.has_placed_gradient();
        match n.geometric_bounds().filter(|_| placed) {
            Some(b) => {
                let mut ap = self.appearance.clone();
                ap.rebase_gradients(Self::UNIT_BOX, b);
                std::borrow::Cow::Owned(ap)
            }
            None => std::borrow::Cow::Borrowed(&self.appearance),
        }
    }
    /// Give `n` this style's transparency.
    pub fn apply_transparency(&self, n: &mut Node) {
        (n.opacity, n.blend, n.isolate, n.knockout) = (self.opacity, self.blend, self.isolate, self.knockout);
    }
    /// Does `n` have this style's transparency?
    pub fn transparency_matches(&self, n: &Node) -> bool {
        (n.opacity, n.blend, n.isolate, n.knockout) == (self.opacity, self.blend, self.isolate, self.knockout)
    }
}

/// A symbol definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub art: Arc<Node>,
}

/// Encoded image bytes (PNG/JPEG/WebP…) shared by image objects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageBlob {
    pub mime: String,
    #[serde(skip)]
    pub bytes: Arc<Vec<u8>>,
    /// A linked image's low-resolution preview (PNG, see [`links`]): what a save writes when only
    /// linked images show this blob, and what `bytes` hold while the linked file can't be read.
    #[serde(skip)]
    pub proxy: Option<Arc<Vec<u8>>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub version: u32,
    #[serde(default)]
    pub title: String,
    /// Saved with File → Save as Template: opening it starts a new untitled document.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub template: bool,
    #[serde(default)]
    pub units: Unit,
    #[serde(default)]
    pub color_mode: ColorMode,
    pub artboards: Vec<Artboard>,
    /// Top-level layers, bottom first.
    pub layers: Vec<Arc<Node>>,
    #[serde(default)]
    pub swatches: Vec<Swatch>,
    #[serde(default)]
    pub swatch_groups: Vec<SwatchGroup>,
    #[serde(default)]
    pub graphic_styles: Vec<GraphicStyle>,
    /// Character styles (besides the built-in [Normal Character Style], which may be redefined here).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub char_styles: Vec<TextStyleDef>,
    /// Paragraph styles (besides the built-in [Normal Paragraph Style]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub para_styles: Vec<TextStyleDef>,
    /// Threaded text: area-type frames one story flows through, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_threads: Vec<Vec<NodeId>>,
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    #[serde(default)]
    pub guides: Vec<Guide>,
    /// View → New View… (up to 25, like Illustrator).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<SavedView>,
    #[serde(default)]
    pub grid: GridPrefs,
    #[serde(default = "ppi72")]
    pub raster_effects_ppi: f64,
    #[serde(default)]
    pub images: BTreeMap<String, ImageBlob>,
    /// Pattern swatch definitions (Object → Pattern).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub patterns: Vec<PatternDef>,
    /// Pattern editing mode, while active.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern_edit: Option<PatternEdit>,
    /// Opacity-mask editing mode, while active. Never saved (see [`Document::without_edit_modes`]);
    /// still read so files saved mid-edit by older versions load without the editing layer.
    #[serde(default, skip_serializing)]
    pub mask_edit: Option<MaskEdit>,
    next_id: u64,
    /// Foreign data preserved on round-trip.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub unknown: serde_json::Map<String, serde_json::Value>,
    /// Transparency panel → Page Isolated Blending: the page is an isolated transparency group,
    /// so top-level blend modes don't blend with what lies under the page.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub page_isolate: bool,
    /// Transparency panel → Page Knockout Group: the page's elements (its layers; neutral layers
    /// pass their contents through) knock each other out.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub page_knockout: bool,
    /// Swatches panel → Spot Colors: spot colours defined in Lab show and separate from their Lab
    /// values (on, the default) or from their working-CMYK equivalents (off)
    /// ([`Document::linked_color`]).
    #[serde(default = "yes", skip_serializing_if = "skip::is_true")]
    pub spot_use_lab: bool,
    /// File → Document Setup (bleed, transparency grid, paper, type options).
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub setup: DocSetup,
    /// Layers panel → Paste Remembers Layers: pasted objects go back into the layers (by name)
    /// they were copied from instead of the current layer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paste_remembers_layers: bool,
    /// File → File Info (the title is [`Document::title`]).
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub metadata: DocMetadata,
    /// Effect → Document Raster Effects Settings besides the resolution
    /// ([`Document::raster_effects_ppi`]).
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub raster_effects: RasterEffectsSettings,
    /// The view (zoom, centre, rotation) the document was saved with; it reopens there. Written at
    /// save time only, so changing the view never marks the document modified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_view: Option<SavedView>,
    /// Edit → Assign Profile: the profiles the document is tagged with (files before format v3
    /// kept them in `unknown`, see [`Document::migrate_color_profiles`]).
    #[serde(default, skip_serializing_if = "ColorProfiles::is_empty")]
    pub color_profiles: ColorProfiles,
    /// File → Export for Screens: the settings it last exported with (`document.exportForScreens`
    /// params), so the dialog reopens on them.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub export_settings: serde_json::Map<String, serde_json::Value>,
    /// Top-level keys this version doesn't know (written by a newer one), kept so saving doesn't
    /// lose them. Separate from [`Document::unknown`], which holds foreign data by design.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
    /// User slices (Slice tool, Object → Slice); object slices are [`Node::slice`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub slices: Vec<Slice>,
    /// Object → Slice → Clip to Artboard: slices are clipped to the artboards and auto slices fill
    /// them (on, the default); off, auto slices cover the art and the slices.
    #[serde(default = "yes", skip_serializing_if = "skip::is_true")]
    pub slices_clip_to_artboard: bool,
    /// File → Print: the print settings saved with the document (`vectorcraft_pdf::PrintSettings`
    /// as JSON: the print engine sits above this crate); `None` until they are set up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub print_setup: Option<serde_json::Value>,
    /// Window → Asset Export: art collected for export ([`ExportAsset`]), in panel order. Their
    /// export settings are Export for Screens' ([`Document::export_settings`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<ExportAsset>,
    /// Puppet Warp pins on the selected artwork while the tool edits it ([`PuppetPins`]): editing
    /// state, never saved.
    #[serde(skip)]
    pub puppet: Option<Arc<PuppetPins>>,
}

fn ppi72() -> f64 {
    72.0
}

fn yes() -> bool {
    true
}

pub const FORMAT_VERSION: u32 = 1;

impl Document {
    /// A new RGB document with one artboard of `size` and one layer ("Layer 1").
    pub fn new(width: f64, height: f64) -> Self {
        Self::new_with_mode(width, height, ColorMode::Rgb)
    }

    /// [`Document::new`] in colour `mode`, with that mode's default swatches.
    pub fn new_with_mode(width: f64, height: f64, mode: ColorMode) -> Self {
        let (swatches, swatch_groups) = vectorcraft_color::default_swatches(mode.model());
        let mut d = Self {
            version: FORMAT_VERSION,
            title: "Untitled-1".into(),
            template: false,
            units: Unit::Points,
            color_mode: mode,
            artboards: vec![Artboard {
                id: 1,
                name: "Artboard 1".into(),
                rect: Rect::new(0.0, 0.0, width, height),
                show_center_mark: false,
                show_cross_hairs: false,
                ..Default::default()
            }],
            layers: vec![],
            swatches,
            swatch_groups,
            graphic_styles: default_graphic_styles(),
            char_styles: vec![],
            para_styles: vec![],
            text_threads: vec![],
            symbols: vec![],
            guides: vec![],
            views: vec![],
            grid: GridPrefs::default(),
            raster_effects_ppi: 72.0,
            images: BTreeMap::new(),
            patterns: vec![],
            pattern_edit: None,
            mask_edit: None,
            next_id: 1,
            unknown: Default::default(),
            page_isolate: false,
            page_knockout: false,
            spot_use_lab: true,
            setup: DocSetup::default(),
            paste_remembers_layers: false,
            metadata: DocMetadata::default(),
            raster_effects: RasterEffectsSettings::default(),
            last_view: None,
            color_profiles: ColorProfiles::default(),
            export_settings: Default::default(),
            extra: Default::default(),
            slices: vec![],
            slices_clip_to_artboard: true,
            print_setup: None,
            assets: vec![],
            puppet: None,
        };
        let id = d.alloc_id();
        d.layers.push(Arc::new(Node::layer(id, "Layer 1", LayerColor::Preset(0))));
        d
    }

    /// Allocate a fresh node id.
    pub fn alloc_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        id
    }
    pub fn peek_next_id(&self) -> u64 {
        self.next_id
    }
    /// Ensure `next_id` is above every id in the tree (after deserializing foreign data).
    pub fn fix_next_id(&mut self) {
        let mut max = 0;
        for l in &self.layers {
            l.walk(&mut |n| max = max.max(n.id.0));
        }
        max = self.slices.iter().fold(max, |m, s| m.max(s.id.0));
        max = self.assets.iter().fold(max, |m, a| m.max(a.id));
        self.next_id = self.next_id.max(max + 1);
    }

    /// Find a node anywhere in the tree.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        fn find(nodes: &[Arc<Node>], id: NodeId) -> Option<&Node> {
            for n in nodes {
                if n.id == id {
                    return Some(n);
                }
                if let Some(ch) = n.children()
                    && let Some(f) = find(ch, id)
                {
                    return Some(f);
                }
            }
            None
        }
        find(&self.layers, id)
    }

    /// Index path from the top-level layer list to `id` (e.g. `[0, 3, 1]`).
    pub fn index_path(&self, id: NodeId) -> Option<Vec<usize>> {
        fn find(nodes: &[Arc<Node>], id: NodeId, path: &mut Vec<usize>) -> bool {
            for (i, n) in nodes.iter().enumerate() {
                path.push(i);
                if n.id == id {
                    return true;
                }
                if let Some(ch) = n.children()
                    && find(ch, id, path)
                {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut p = vec![];
        find(&self.layers, id, &mut p).then_some(p)
    }

    /// Ids from the top-level layer down to `id` inclusive.
    pub fn ancestry(&self, id: NodeId) -> Option<Vec<NodeId>> {
        let path = self.index_path(id)?;
        let mut out = Vec::with_capacity(path.len());
        let mut nodes = &self.layers;
        for &i in &path {
            let n = &nodes[i];
            out.push(n.id);
            if let Some(ch) = n.children() {
                nodes = ch;
            }
        }
        Some(out)
    }

    pub fn parent_of(&self, id: NodeId) -> Option<NodeId> {
        let a = self.ancestry(id)?;
        (a.len() >= 2).then(|| a[a.len() - 2])
    }

    /// The top-level layer that contains `id`.
    pub fn layer_of(&self, id: NodeId) -> Option<NodeId> {
        self.ancestry(id).and_then(|a| a.first().copied())
    }

    /// Mutable access to a node; clones the Arc path (copy-on-write).
    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        let path = self.index_path(id)?;
        let mut cur: &mut Arc<Node> = &mut self.layers[path[0]];
        for &i in &path[1..] {
            cur = &mut Arc::make_mut(cur).children_mut()?[i];
        }
        Some(Arc::make_mut(cur))
    }

    /// Children list of a container (or the top-level layer list for `None`).
    pub fn children_mut(&mut self, parent: Option<NodeId>) -> Result<&mut Vec<Arc<Node>>, DocError> {
        match parent {
            None => Ok(&mut self.layers),
            Some(p) => self.node_mut(p).ok_or(DocError::NoNode(p))?.children_mut().ok_or(DocError::NotContainer(p)),
        }
    }
    pub fn children(&self, parent: Option<NodeId>) -> Option<&Vec<Arc<Node>>> {
        match parent {
            None => Some(&self.layers),
            Some(p) => self.node(p)?.children(),
        }
    }

    /// Insert `node` into `parent` at `index` (clamped; `usize::MAX` = top).
    pub fn insert(&mut self, parent: Option<NodeId>, index: usize, node: Node) -> Result<NodeId, DocError> {
        let id = node.id;
        let ch = self.children_mut(parent)?;
        let i = index.min(ch.len());
        ch.insert(i, Arc::new(node));
        Ok(id)
    }

    /// Remove a node and return it.
    pub fn remove(&mut self, id: NodeId) -> Result<Arc<Node>, DocError> {
        let parent = self.parent_of(id);
        let path = self.index_path(id).ok_or(DocError::NoNode(id))?;
        let idx = *path.last().ok_or(DocError::NoNode(id))?;
        let ch = self.children_mut(parent)?;
        if idx >= ch.len() {
            return Err(DocError::NoNode(id));
        }
        Ok(ch.remove(idx))
    }

    /// Move a node to `parent` at `index` (index measured after removal).
    pub fn move_node(&mut self, id: NodeId, parent: Option<NodeId>, index: usize) -> Result<(), DocError> {
        if let Some(p) = parent
            && self.ancestry(p).is_some_and(|a| a.contains(&id))
        {
            return Err(DocError::Invalid("cannot move an object into itself".into()));
        }
        let n = self.remove(id)?;
        let ch = self.children_mut(parent)?;
        let i = index.min(ch.len());
        ch.insert(i, n);
        Ok(())
    }

    /// Position of `id` within its parent: (parent, index, sibling count).
    pub fn position(&self, id: NodeId) -> Option<(Option<NodeId>, usize, usize)> {
        let parent = self.parent_of(id);
        let idx = *self.index_path(id)?.last()?;
        let n = self.children(parent)?.len();
        Some((parent, idx, n))
    }

    /// Default target layer: the topmost visible, unlocked layer.
    pub fn default_layer(&self) -> Option<NodeId> {
        self.layers.iter().rev().find(|l| l.visible && !l.locked).or(self.layers.last()).map(|l| l.id)
    }

    /// Add a new top-level layer above all others.
    pub fn add_layer(&mut self, name: Option<&str>) -> NodeId {
        let id = self.alloc_id();
        let n = self.layers.len();
        let name = name.map(str::to_string).unwrap_or_else(|| self.next_layer_name());
        self.layers.push(Arc::new(Node::layer(id, &name, LayerColor::Preset((n % LAYER_COLORS.len()) as u8))));
        id
    }
    pub fn next_layer_name(&self) -> String {
        let mut i = self.layers.len() + 1;
        loop {
            let name = format!("Layer {i}");
            if !self.layers.iter().any(|l| l.name.as_deref() == Some(&name)) {
                return name;
            }
            i += 1;
        }
    }

    /// Visit every node depth first in paint order (bottom to top).
    pub fn walk<'a>(&'a self, mut f: impl FnMut(&'a Node)) {
        for l in &self.layers {
            l.walk(&mut f);
        }
    }
    pub fn node_count(&self) -> usize {
        self.layers.iter().map(|l| l.count()).sum()
    }
    /// Is the node (and every ancestor) visible and unlocked?
    pub fn is_editable(&self, id: NodeId) -> bool {
        self.ancestry(id).is_some_and(|a| a.iter().all(|i| self.node(*i).is_some_and(|n| n.visible && !n.locked)))
    }
    pub fn is_visible(&self, id: NodeId) -> bool {
        self.ancestry(id).is_some_and(|a| a.iter().all(|i| self.node(*i).is_some_and(|n| n.visible)))
    }
    /// Colour of the layer containing `id` (selection highlight colour).
    pub fn layer_color(&self, id: NodeId) -> [u8; 3] {
        let l = self.layer_of(id).and_then(|l| self.node(l));
        match l.map(|n| &n.kind) {
            Some(NodeKind::Layer { color, .. }) => color.rgb(),
            _ => LAYER_COLORS[0].1,
        }
    }
    /// Union of the geometric bounds of `ids`.
    pub fn bounds_of(&self, ids: &[NodeId], visual: bool) -> Option<Rect> {
        ids.iter()
            .filter_map(|id| self.node(*id))
            .fold(None, |acc, n| vectorcraft_geom::union_opt(acc, if visual { n.visual_bounds() } else { n.geometric_bounds() }))
    }
    /// Bounds of all art.
    pub fn art_bounds(&self) -> Option<Rect> {
        self.layers.iter().fold(None, |acc, l| vectorcraft_geom::union_opt(acc, l.visual_bounds()))
    }
    /// Artboard index containing point `p` (topmost = last).
    pub fn artboard_at(&self, p: Point) -> Option<usize> {
        self.artboards.iter().rposition(|a| a.rect.contains(p))
    }
    pub fn next_artboard_id(&self) -> u32 {
        self.artboards.iter().map(|a| a.id).max().unwrap_or(0) + 1
    }
    pub fn pattern(&self, name: &str) -> Option<&PatternDef> {
        self.patterns.iter().find(|p| p.name == name)
    }
    pub fn pattern_mut(&mut self, name: &str) -> Option<&mut PatternDef> {
        self.patterns.iter_mut().find(|p| p.name == name)
    }
    /// Deep-clone `node` with fresh ids for it and all descendants.
    pub fn reid(&mut self, node: &Node) -> Node {
        let mut n = node.clone();
        n.id = self.alloc_id();
        if let Some(ch) = n.children_mut() {
            let old: Vec<Arc<Node>> = std::mem::take(ch);
            *ch = old.iter().map(|c| Arc::new(self.reid(c))).collect();
        }
        n
    }
}

impl Document {
    /// Leave opacity-mask editing: drop the temporary editing layer, a working copy of art the
    /// mask already holds (the engine syncs it after every edit).
    pub fn drop_edit_modes(&mut self) {
        if let Some(me) = self.mask_edit.take() {
            self.layers.retain(|l| l.id != me.layer);
        }
    }

    /// The document as saved and exported: without the opacity-mask editing layer. Borrowed when
    /// no mask is being edited. (Pattern editing is saved: its tile layer holds unapplied edits.)
    pub fn without_edit_modes(&self) -> std::borrow::Cow<'_, Document> {
        if self.mask_edit.is_none() {
            return std::borrow::Cow::Borrowed(self);
        }
        let mut d = self.clone();
        d.drop_edit_modes();
        std::borrow::Cow::Owned(d)
    }
}

/// Graphic styles.
impl Document {
    /// Index of the graphic style named `name`.
    pub fn graphic_style_index(&self, name: &str) -> Option<usize> {
        self.graphic_styles.iter().position(|g| g.name == name)
    }
    /// The graphic style named `name`.
    pub fn graphic_style(&self, name: &str) -> Option<&GraphicStyle> {
        self.graphic_styles.iter().find(|g| g.name == name)
    }
    /// The graphic style with id `id` (0 never matches).
    pub fn graphic_style_by_id(&self, id: u32) -> Option<&GraphicStyle> {
        self.graphic_styles.iter().find(|g| g.id != 0 && g.id == id)
    }
    /// An id no graphic style has.
    pub fn next_graphic_style_id(&self) -> u32 {
        self.graphic_styles.iter().map(|g| g.id).max().unwrap_or(0) + 1
    }
    /// The id of graphic style `index`, assigning one if it has none yet.
    pub fn graphic_style_id(&mut self, index: usize) -> u32 {
        if self.graphic_styles[index].id == 0 {
            self.graphic_styles[index].id = self.next_graphic_style_id();
        }
        self.graphic_styles[index].id
    }
    /// The default name of a new graphic style: the first free "Graphic Style N", N counting on
    /// from the number of styles.
    pub fn new_graphic_style_name(&self) -> String {
        (self.graphic_styles.len() + 1..).map(|i| format!("Graphic Style {i}")).find(|n| self.graphic_style(n).is_none()).unwrap_or_default()
    }
}

fn default_graphic_styles() -> Vec<GraphicStyle> {
    use vectorcraft_color::{Color, Paint};
    let solid = |hex| Paint::solid(Color::from_hex(hex).unwrap_or(Color::BLACK));
    [
        GraphicStyle::new(DEFAULT_GRAPHIC_STYLE, Appearance::default_art()),
        GraphicStyle::new("Black Outline", Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0)),
        GraphicStyle::new("Heavy Ink", Appearance::basic(solid("#1b1464"), Paint::solid(Color::BLACK), 4.0)),
        GraphicStyle::new("Sunshine", Appearance::basic(solid("#fbb03b"), solid("#f15a24"), 2.0)),
    ]
    .into_iter()
    .zip(1..)
    .map(|(g, id)| GraphicStyle { id, ..g })
    .collect()
}

impl Document {
    /// Make the next id allocated at least `next`: ids handed out outside the layer tree (such
    /// as an importer's opacity mask and pattern art), which [`Document::fix_next_id`] doesn't see.
    pub fn reserve_ids(&mut self, next: u64) {
        self.next_id = self.next_id.max(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_geom::shapes;

    fn doc_with_rects() -> (Document, NodeId, NodeId) {
        let mut d = Document::new(612.0, 792.0);
        let layer = d.layers[0].id;
        let a = d.alloc_id();
        d.insert(Some(layer), usize::MAX, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art())).unwrap();
        let b = d.alloc_id();
        d.insert(Some(layer), usize::MAX, Node::path(b, shapes::rectangle(Rect::new(20.0, 0.0, 30.0, 10.0)), Appearance::default_art())).unwrap();
        (d, a, b)
    }

    #[test]
    fn new_document_has_layer_and_artboard() {
        let d = Document::new(612.0, 792.0);
        assert_eq!(d.layers.len(), 1);
        assert_eq!(d.layers[0].display_name(), "Layer 1");
        assert_eq!(d.artboards[0].rect, Rect::new(0.0, 0.0, 612.0, 792.0));
    }

    #[test]
    fn find_and_paths() {
        let (d, a, b) = doc_with_rects();
        assert_eq!(d.index_path(b), Some(vec![0, 1]));
        assert_eq!(d.parent_of(a), Some(d.layers[0].id));
        assert_eq!(d.layer_of(b), Some(d.layers[0].id));
        assert!(d.node(NodeId(999)).is_none());
        assert_eq!(d.node_count(), 3);
    }

    #[test]
    fn copy_on_write_shares_untouched() {
        let (d, a, b) = doc_with_rects();
        let mut e = d.clone();
        e.node_mut(a).unwrap().name = Some("A".into());
        // b's Arc is shared between both versions; a's is not.
        let lb = &d.layers[0].children().unwrap()[1];
        let eb = &e.layers[0].children().unwrap()[1];
        assert!(Arc::ptr_eq(lb, eb));
        assert_eq!(d.node(a).unwrap().name, None);
        assert_eq!(e.node(a).unwrap().name.as_deref(), Some("A"));
        let _ = b;
    }

    #[test]
    fn move_and_remove() {
        let (mut d, a, b) = doc_with_rects();
        let layer = d.layers[0].id;
        d.move_node(a, Some(layer), usize::MAX).unwrap();
        assert_eq!(d.index_path(a), Some(vec![0, 1]));
        assert_eq!(d.index_path(b), Some(vec![0, 0]));
        let r = d.remove(a).unwrap();
        assert_eq!(r.id, a);
        assert!(d.node(a).is_none());
        assert!(d.move_node(layer, Some(layer), 0).is_err());
    }

    #[test]
    fn layers_and_names() {
        let mut d = Document::new(100.0, 100.0);
        let l2 = d.add_layer(None);
        assert_eq!(d.node(l2).unwrap().display_name(), "Layer 2");
        assert_eq!(d.default_layer(), Some(l2));
        assert_eq!(d.layer_color(l2), LAYER_COLORS[1].1);
    }

    #[test]
    fn serde_roundtrip() {
        let (d, _, _) = doc_with_rects();
        let s = serde_json::to_string(&d).unwrap();
        let back: Document = serde_json::from_str(&s).unwrap();
        assert_eq!(back.node_count(), d.node_count());
        assert_eq!(back.peek_next_id(), d.peek_next_id());
    }

    #[test]
    fn reid_gives_fresh_ids() {
        let (mut d, a, b) = doc_with_rects();
        let g = d.alloc_id();
        let na = (*d.remove(a).unwrap()).clone();
        let nb = (*d.remove(b).unwrap()).clone();
        let group = Node::group(g, vec![Arc::new(na), Arc::new(nb)]);
        let copy = d.reid(&group);
        assert_ne!(copy.id, g);
        let ids: Vec<NodeId> = copy.children().unwrap().iter().map(|c| c.id).collect();
        assert!(!ids.contains(&a) && !ids.contains(&b));
    }

    #[test]
    fn setup_round_trips_and_old_files_load() {
        let (mut d, _, _) = doc_with_rects();
        // A document without a setup writes no `setup` key (old readers and files stay unchanged).
        assert!(!serde_json::to_string(&d).unwrap().contains("\"setup\""));
        d.setup.bleed = [9.0, 9.0, 0.0, 18.0];
        d.setup.grid_size = GridSize::Large;
        d.setup.typographers_quotes = false;
        d.setup.quotes = setup::language_quotes("German").unwrap();
        d.setup.flattener_preset = Some("High Resolution".into());
        d.setup.background = Background::White;
        d.setup.export_text = ExportText::Appearance;
        let back: Document = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(back.setup, d.setup);
        // Files written before Document Setup existed load with the defaults.
        let mut v = serde_json::to_value(&d).unwrap();
        v.as_object_mut().unwrap().remove("setup");
        let old: Document = serde_json::from_value(v).unwrap();
        assert_eq!(old.setup, DocSetup::default());
    }

    #[test]
    fn char_position_round_trips() {
        let st = CharStyle {
            position: CharPosition::Subscript(ScriptMetrics { size: 50.0, position: 20.0 }),
            small_caps: Some(75.0),
            ..CharStyle::default()
        };
        let v = serde_json::to_value(&st).unwrap();
        assert_eq!(v["position"], serde_json::json!({"kind": "subscript", "size": 50.0, "position": 20.0}));
        assert_eq!(serde_json::from_value::<CharStyle>(v).unwrap(), st);
        let plain = serde_json::to_value(CharStyle::default()).unwrap();
        assert!(plain.get("position").is_none() && plain.get("small_caps").is_none());
        let (scale, shift) = CharPosition::Subscript(ScriptMetrics::DEFAULT).scale_shift(10.0);
        assert!((scale - 0.583).abs() < 1e-9 && (shift + 3.33).abs() < 1e-9);
    }

    #[test]
    fn units() {
        assert_eq!(Unit::Inches.parse("1"), Some(72.0));
        assert_eq!(Unit::Points.parse("1in"), Some(72.0));
        assert_eq!(Unit::Points.parse("10 mm").map(|v| (v * 1000.0).round()), Some(28346.0));
        assert_eq!(Unit::Points.parse("2p6"), Some(30.0));
        assert_eq!(Unit::Points.parse("10+5"), Some(15.0));
        assert_eq!(Unit::Points.parse("100/4"), Some(25.0));
        assert_eq!(Unit::Points.parse("-5"), Some(-5.0));
        assert_eq!(Unit::Points.parse("abc"), None);
        assert_eq!(Unit::Points.format(12.5), "12.5 pt");
        assert_eq!(Unit::Inches.format(144.0), "2 in");
    }
}

#[cfg(test)]
mod tests_slices;
#[cfg(test)]
mod tests_units;
