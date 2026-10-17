//! VectorCraft document model: artboards, layers and objects, appearance, swatches (pure data + serde).
//!
//! Documents are persistent trees: children are `Arc<Node>`, and edits go through
//! [`Document::node_mut`], which clones only the nodes on the path from the root to the edited node.
//! Keeping the previous `Document` value around is therefore a cheap undo snapshot.
#![forbid(unsafe_code)]

pub mod appearance;
pub mod assets;
pub mod blend;
pub mod cancel;
pub mod clipnest;
pub mod cmyk;
pub mod corners;
pub mod depth;
pub mod dimension;
pub mod graph;
pub mod hit;
mod id_index;
pub mod inks;
mod inline;
pub mod intertwine;
pub mod shared;
mod symbol_export_type;
mod symbol_nine_slice;
mod symbol_registration;
pub use inline::{SYMBOL_HALF, SYMBOL_SIZES};
pub use shared::Shared;
pub use symbol_export_type::{SYMBOL_EXPORT_TYPE, SymbolExportType};
pub use symbol_nine_slice::{SYMBOL_NINE_SLICE, SymbolNineSlice};
pub use symbol_registration::{SYMBOL_REGISTRATION, SymbolRegistration};
pub mod links;
pub mod live;
pub mod live_paint;
pub mod marks;
mod mem;
pub mod metadata;
pub mod node;
pub mod orient;
pub mod overprint;
pub mod pattern;
pub mod perspective;
mod pixels;
pub use pixels::MAX_RASTER_ALLOC;
pub mod placed_document;
pub mod profiles;
pub mod puppet;
pub mod range;
pub mod rastersettings;
mod reach;
pub mod recolor;
pub mod selection;
pub mod setup;
pub mod shaper;
pub mod slices;
pub mod style_libs;
pub mod swatches;
mod symbols;
pub use symbols::SymbolBoxes;
pub mod text;
pub mod trace;
pub mod variables;
pub mod xml_id;

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
    pub fn is_empty_vec<T>(v: &[T]) -> bool {
        v.is_empty()
    }
    pub fn is_empty_map(v: &serde_json::Map<String, serde_json::Value>) -> bool {
        v.is_empty()
    }
}

pub use appearance::StrokeGradientMode;
pub use appearance::{
    Appearance, AppearanceItem, ArrowAlign, Arrowhead, Dash, Effect, FillLayer, LineCap, LineJoin, PressureProfile, ProfilePreset, SavedProfile,
    StrokeAlign, StrokeLayer, WidthProfile,
};
pub use assets::ExportAsset;
pub use corners::LiveCorners;
pub use depth::MAX_DEPTH;
pub use graph::{
    ColumnDesign, ColumnType, Fractions, GraphDesign, GraphKind, GraphSpec, LabelStyles, MAX_GRAPH_CATEGORIES, MAX_GRAPH_SERIES, PiePosition,
    PieSort, SeriesPaint, TickLength, ValueAxisSide,
};
pub use hit::{Hit, HitKind};
pub use links::{LinkInfo, PlacementOptions};
pub use live::{BlendOrientation, BlendSpacing, BlendSpec, EnvelopeKind, GradientMesh, MeshPoint};
pub use live_paint::{GapOptions, GapSize};
pub use metadata::{CopyrightStatus, DocMetadata};
pub use node::Knockout;
pub use node::Scaling;
pub use node::{Extras, ExtrasMut, ImageObject, LAYER_COLORS, LayerColor, LiveShape, Node, NodeExtras, NodeId, NodeKind, OpacityMask};
pub use node::{ImageMap, MAX_SVG_SCRIPTS, ObjectAttributes, SVG_EVENTS};
pub use orient::OrientedBox;
pub use pattern::{Overlap, PatternDef, PatternEdit, RepeatKind, RepeatSpec, TileType};
pub use perspective::PerspectiveAttachment;
pub use placed_document::PlacedDocument;
pub use profiles::ColorProfiles;
pub use puppet::{PuppetPin, PuppetPins};
pub use rastersettings::{RasterColorModel, RasterEffectsSettings};
pub use selection::{AnchorRef, SegmentRef, Selection};
pub use setup::{Background, DocSetup, ExportText, GridSize, Quotes};
pub use slices::{CellAlign, CellVAlign, Slice, SliceArea, SliceKind, SliceOptions, SliceSource};
pub use style_libs::StyleLibrary;
pub use text::{
    AreaFit, AreaOptions, Burasagari, CharAlign, CharPosition, CharStyle, Composer, Digits, FirstBaseline, HyphenationOptions, InlineArt, Justify,
    Kinsoku, LeadingModel, Mojikumi, ParaDirection, ParaStyle, PathAlign, PathEffect, ScriptMetrics, TabAlign, TabStop, TextKind, TextObject,
    TextRun, TextStyleDef, TextWrap, VerticalAlign, WrapShape,
};
pub use trace::TraceView;
pub use variables::{DataSet, DataValue, Variable, VariableKind, Variables};
pub use vectorcraft_color as color;
pub use vectorcraft_geom as geom;

use serde::{Deserialize, Serialize};
use vectorcraft_color::{Swatch, SwatchGroup};
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

/// Coordinates beyond this many points exceed the supported document range.
pub const MAX_COORD: f64 = 4.0e6;

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
    /// The rulers' label step at `zoom` (screen pixels per point), in this unit: the first of
    /// 1, 2, 5 × 10ⁿ that puts labels at least 50 pixels apart. Ticks mark every tenth of it.
    pub fn ruler_step(self, zoom: f64) -> f64 {
        const STEPS: [f64; 19] =
            [0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0];
        STEPS.iter().copied().find(|s| s * self.points() * zoom >= 50.0).unwrap_or(10000.0)
    }
    /// `v` (points) on the nearest ruler tick at `zoom` ([`Self::ruler_step`]).
    pub fn snap_to_ruler_tick(self, v: f64, zoom: f64) -> f64 {
        let tick = self.ruler_step(zoom) / 10.0 * self.points();
        (v / tick).round() * tick
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
        // Three decimals for the small units (`595.276 pt`), four for the large ones (`8.2677 in`,
        // `35.2778 mm`, the `0.0078 in` stroke preset); trailing zeros are trimmed (`1 pt` is `1`).
        let decimals = if matches!(self, Unit::Points | Unit::Pixels | Unit::Picas) { 3 } else { 4 };
        let s = format!("{:.*}", decimals, self.from_pt(pt));
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" { "0".into() } else { s.into() }
    }
    /// Parse `12`, `12pt`, `1in`, `3 mm`, `2p6` (picas+points), simple `+ - * /` arithmetic.
    ///
    /// A unit after a `*` or `/` operand measures the whole expression (`1080/2 px` is 540 px), so
    /// math typed before a field's unit suffix works. After `*` or `/`, a percentage scales the term
    /// before it (`3cm*50%` is 1.5 cm, `3cm/50%` is 6 cm). After `+` or `-`, it adds or takes away
    /// that share of everything before it (`50pt+25%` is 62.5 pt, `100pt+20pt-50%` is 60 pt). A
    /// percentage on its own (`50%`) reads as nothing.
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
    // `+` and `-` share one precedence level, as do `*` and `/`, so each level splits at the last
    // of its operators (left to right).
    for ops in [&['+', '-'][..], &['*', '/'][..]] {
        // Past the first character, so a leading sign isn't an operator.
        let skip = s.chars().next().map_or(0, char::len_utf8);
        // A `+` or `-` right after another operator is the sign of its operand (`10*-2`, `5--3`).
        let is_sign = |i: usize| {
            s.get(i..).and_then(|r| r.chars().next()).is_some_and(|c| matches!(c, '+' | '-'))
                && s.get(..i).and_then(|l| l.trim_end().chars().last()).is_some_and(|c| "+-*/".contains(c))
        };
        let found = s.char_indices().rev().find(|&(i, c)| i >= skip && ops.contains(&c) && !is_sign(i));
        if let Some(i) = found.map(|(i, _)| i) {
            let (l, r) = (&s[..i], &s[i + 1..]);
            if l.trim().is_empty() {
                continue;
            }
            let Some(op) = s[i..].chars().next() else { continue };
            return match op {
                '+' | '-' => {
                    let a = parse_measure(l, default)?;
                    // A percentage added or taken away is a share of everything before it:
                    // `50pt+25%` is 62.5 pt, `100pt+20%-10pt` is 110 pt.
                    let b = match percent(r) {
                        Some(k) => a * k,
                        None => parse_measure(r, default)?,
                    };
                    Some(if op == '+' { a + b } else { a - b })
                }
                _ => {
                    // A factor or divisor is a plain number or a percentage (`3cm*50%` is 1.5 cm);
                    // a unit after it is the expression's.
                    let (k, unit) = match percent(r) {
                        Some(k) => (k, None),
                        None => number_unit(r)?,
                    };
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
        // The sign belongs to the whole value: `-2p6` is -(2 picas + 6 points).
        let sign = if p.trim_start().starts_with('-') { -1.0 } else { 1.0 };
        let picas = p.trim().parse::<f64>().ok()?.abs();
        return Some(sign * (picas * 12.0 + pt.trim().parse::<f64>().unwrap_or(0.0)));
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

/// A number followed by `%` (`50%`, `12.5 %`) → its fraction (`0.5`, `0.125`).
fn percent(s: &str) -> Option<f64> {
    match number_unit(s.trim().strip_suffix('%')?)? {
        (p, None) => Some(p / 100.0),
        _ => None,
    }
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artboard {
    pub id: u32,
    pub name: String,
    /// Artboard rectangle in document points.
    pub rect: Rect,
    /// Locked (the Artboards panel's lock column): the Artboard tool and the artboard commands
    /// don't move, resize or delete it, Rearrange leaves it where it is, and the art on it can't be
    /// selected or edited ([`Document::on_locked_artboard`]), except new art while it stays
    /// selected ([`Document::new_art`]).
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub locked: bool,
    #[serde(default)]
    pub show_center_mark: bool,
    #[serde(default)]
    pub show_cross_hairs: bool,
}

/// A document's locked artboards ([`Document::locked_artboards`]).
pub struct LockedArtboards<'a> {
    rects: Vec<Rect>,
    symbols: SymbolBoxes<'a>,
    /// The new art, which isn't on them ([`Document::new_art`]).
    new_art: Arc<std::collections::HashSet<NodeId>>,
    /// The objects of pattern editing mode's layer ([`Document::pattern_edit`]), which are on no
    /// artboard.
    pattern_art: std::collections::HashSet<NodeId>,
    /// What [`LockedArtboards::overlaps`] found for each object it measured: the members of a
    /// group are each answered with one measurement of the group.
    held: std::cell::RefCell<std::collections::HashMap<NodeId, bool>>,
}

impl LockedArtboards<'_> {
    /// No artboard is locked.
    pub fn is_empty(&self) -> bool {
        self.rects.is_empty()
    }
    /// Is `n`, an object of a layer or sublayer, on a locked artboard: shown, not a guide, and its
    /// visual bounds (with its strokes, a symbol instance by its art) overlapping the artboard? An
    /// object counts for every artboard it overlaps, as Fit to Artwork Bounds counts it. A layer or
    /// sublayer is on none, nor is the art of pattern editing mode ([`Document::pattern_edit`]) or
    /// new art while it stays selected ([`Document::new_art`]).
    pub fn holds(&self, n: &Node) -> bool {
        !self.new_art.contains(&n.id) && self.overlaps(n)
    }
    /// [`LockedArtboards::holds`] for new art too: is `n` on a locked artboard, whether or not it
    /// is new art?
    pub fn overlaps(&self, n: &Node) -> bool {
        if self.rects.is_empty()
            || !n.visible
            || n.is_layer()
            || matches!(n.kind, NodeKind::Path { guide: true, .. })
            || self.pattern_art.contains(&n.id)
        {
            return false;
        }
        if let Some(held) = self.held.borrow().get(&n.id) {
            return *held;
        }
        let held =
            n.visual_bounds_with(&self.symbols).is_some_and(|b| self.rects.iter().any(|r| b.x0 < r.x1 && b.x1 > r.x0 && b.y0 < r.y1 && b.y1 > r.y0));
        self.held.borrow_mut().insert(n.id, held);
        held
    }
    /// Is node `id` on a locked artboard: is the object of a layer or sublayer that holds it (`id`
    /// itself when it is one, a group counting as one object) on one ([`LockedArtboards::holds`])?
    pub fn holds_id(&self, id: NodeId) -> bool {
        !self.rects.is_empty() && self.symbols.document().layer_object(id).is_some_and(|n| self.holds(n))
    }
}

/// New art on a locked artboard ([`Document::new_art`]). Setting it changes the document in
/// place, so a selection change that sets it doesn't replace the document.
#[derive(Default)]
pub struct NewArtIds(std::sync::RwLock<Arc<std::collections::HashSet<NodeId>>>);

impl NewArtIds {
    /// The objects.
    pub fn get(&self) -> Arc<std::collections::HashSet<NodeId>> {
        self.0.read().map(|g| g.clone()).unwrap_or_default()
    }
    /// Make `ids` the objects.
    pub fn set(&self, ids: Arc<std::collections::HashSet<NodeId>>) {
        if let Ok(mut g) = self.0.write() {
            *g = ids;
        }
    }
}

impl Clone for NewArtIds {
    fn clone(&self) -> Self {
        NewArtIds(std::sync::RwLock::new(self.get()))
    }
}

/// Editing state, not document content: two documents are equal whatever new art they name.
impl PartialEq for NewArtIds {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for NewArtIds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("NewArtIds").field(&self.get()).finish()
    }
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

/// A saved selection (Select → Save Selection…): the objects that were selected, listed by name at
/// the bottom of the Select menu.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedSelection {
    pub name: String,
    pub objects: Vec<NodeId>,
}

impl SavedSelection {
    /// Most saved selections a document keeps (the Select menu lists every one).
    pub const MAX: usize = 25;
    /// Longest name, in characters.
    pub const MAX_NAME: usize = 255;

    /// `name` trimmed and cut to [`Self::MAX_NAME`] characters (`None` when blank).
    pub fn clean_name(name: &str) -> Option<String> {
        let name: String = name.trim().chars().take(Self::MAX_NAME).collect();
        (!name.is_empty()).then_some(name)
    }

    /// The first "Selection N" none of `saved` is named.
    pub fn default_name(saved: &[SavedSelection]) -> String {
        (1..=saved.len() + 1).map(|i| format!("Selection {i}")).find(|n| !saved.iter().any(|x| &x.name == n)).unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Guide {
    /// true = vertical guide at `pos` (x), false = horizontal at `pos` (y).
    pub vertical: bool,
    pub pos: f64,
    /// An artboard guide: the [`Artboard::id`] it belongs to. It runs across that artboard only
    /// and moves, is copied and is deleted with it. None: a canvas guide, across the whole canvas.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artboard: Option<u32>,
    /// The layer (or sublayer) it is on: it shows, hides and locks with that layer and is listed
    /// in it in the Layers panel, and goes when the layer is deleted. None: on no layer (guides
    /// from before guides had layers, and from files that don't say), always there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<NodeId>,
}

impl Guide {
    /// A canvas guide.
    pub fn new(vertical: bool, pos: f64) -> Self {
        Self { vertical, pos, artboard: None, layer: None }
    }

    /// Moved by `d` (a vertical guide across, a horizontal one down).
    pub fn moved(&self, d: Vec2) -> Self {
        Self { pos: self.pos + if self.vertical { d.x } else { d.y }, ..self.clone() }
    }
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

/// What the rulers measure from (View › Rulers): the active artboard's origin, the default of a
/// new document, or one origin for the whole document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RulerMode {
    #[default]
    Artboard,
    Global,
}

impl RulerMode {
    /// The mode of a document saved before the choice existed: global, as its rulers measured.
    pub fn legacy() -> Self {
        Self::Global
    }
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
    /// The rulers' zero point in document coordinates. Moving it changes measurements, never art.
    #[serde(default, skip_serializing_if = "skip::is_default")]
    pub ruler_origin: Point,
    /// View › Rulers › Change to Global / Artboard Rulers: whether the rulers and position fields
    /// measure from [`Self::ruler_origin`] (global) or from the active artboard's own origin.
    /// Documents saved before the choice existed measured globally and open that way.
    #[serde(default = "RulerMode::legacy")]
    pub rulers: RulerMode,
    /// Artboard rulers' zero point of each artboard (by [`Artboard::id`]), as an offset from its
    /// top-left corner; an artboard not listed measures from that corner.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub artboard_ruler_origins: BTreeMap<u32, Vec2>,
    #[serde(default)]
    pub color_mode: ColorMode,
    pub artboards: Vec<Artboard>,
    /// Top-level layers, bottom first.
    pub layers: Vec<Arc<Node>>,
    // The document-wide resources below are [`Shared`]: an edit copies the document (copy-on-write)
    // and the undo history keeps every version, so what an edit doesn't change stays one copy.
    // Reads, writes and the saved form are those of the value itself.
    #[serde(default)]
    pub swatches: Shared<Vec<Swatch>>,
    #[serde(default)]
    pub swatch_groups: Shared<Vec<SwatchGroup>>,
    #[serde(default)]
    pub graphic_styles: Shared<Vec<GraphicStyle>>,
    /// Character styles (besides the built-in [Normal Character Style], which may be redefined here).
    #[serde(default, skip_serializing_if = "skip::is_empty_vec")]
    pub char_styles: Shared<Vec<TextStyleDef>>,
    /// Paragraph styles (besides the built-in [Normal Paragraph Style]).
    #[serde(default, skip_serializing_if = "skip::is_empty_vec")]
    pub para_styles: Shared<Vec<TextStyleDef>>,
    /// Threaded text: area-type frames one story flows through, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text_threads: Vec<Vec<NodeId>>,
    #[serde(default)]
    pub symbols: Vec<Symbol>,
    /// Object › Graph › Design…: art graphs can draw as markers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub graph_designs: Vec<GraphDesign>,
    #[serde(default)]
    pub guides: Vec<Guide>,
    /// View → New View… (up to 25, like Illustrator).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<SavedView>,
    /// Select → Save Selection… (at most [`SavedSelection::MAX`]); saved with the document.
    #[serde(default, skip_serializing_if = "skip::is_empty_vec")]
    pub saved_selections: Shared<Vec<SavedSelection>>,
    /// Variables (data merge): named values bound to objects, applied per dataset.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub variables: Shared<Variables>,
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
    #[serde(default, skip_serializing_if = "skip::is_empty_map")]
    pub unknown: Shared<serde_json::Map<String, serde_json::Value>>,
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
    /// View → Show Transparency Grid when the document was saved. Written at save time only,
    /// like [`Document::last_view`]; older files open with the grid hidden.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub transparency_grid: bool,
    /// The layers, sublayers and groups open in the Layers panel when the document was saved; they
    /// reopen that way. `None`: the default, only the top-level layers open. Written at save time
    /// only, like [`Document::last_view`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layers_open: Option<Vec<NodeId>>,
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
    /// Window › SVG Interactivity › Link JavaScript Files: the JavaScript files (paths or URLs, as
    /// given) SVG export links with `<script>` elements, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub svg_scripts: Vec<String>,
    /// Puppet Warp pins on the selected artwork while the tool edits it ([`PuppetPins`]): editing
    /// state, never saved.
    #[serde(skip)]
    pub puppet: Option<Arc<PuppetPins>>,
    /// New art on a locked artboard: objects of layers on a locked artboard that an edit made and
    /// left selected (a group with selected members counts), kept while they stay selected and on
    /// one. They and their contents can be selected and edited ([`LockedArtboards::holds`]). The
    /// engine sets it from the selection's ([`Selection::new_art`]). Editing state, never saved.
    #[serde(skip)]
    pub new_art: NewArtIds,
    /// Some object is a dimension (the Dimension tool): edits refresh dimensions only while this
    /// is set, so documents without any pay nothing for them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dimensions: bool,
    /// Where each node is in the tree, for lookups by id ([`id_index`]): a cache, never saved.
    #[serde(skip)]
    index: id_index::IdIndex,
}

fn ppi72() -> f64 {
    72.0
}

fn yes() -> bool {
    true
}

/// Is point `p` on the artboard at `rect`, its edges included, within a millionth of a point? This
/// is the rule [`Document::art_on_artboard`] applies to art flush with an edge (#890), where
/// `Rect::contains` leaves out the right and bottom edges.
pub fn artboard_holds(rect: Rect, p: Point) -> bool {
    let r = rect.inflate(1e-6, 1e-6);
    r.x0 <= p.x && p.x <= r.x1 && r.y0 <= p.y && p.y <= r.y1
}

/// Listed nodes by parent: the parent's index path (empty for the top-level layers), then the index
/// of each listed child in it, in order, with the child's place in the list.
type Places = BTreeMap<Vec<usize>, Vec<(usize, usize)>>;

pub const FORMAT_VERSION: u32 = 1;

impl Document {
    /// The ruler zero point, or zero on both axes when either stored coordinate is invalid.
    /// Rulers, coordinate fields and guide snapping use the same supported range.
    pub fn ruler_origin_or_zero(&self) -> Point {
        let origin = self.ruler_origin;
        if origin.is_finite() && origin.x.abs() <= MAX_COORD && origin.y.abs() <= MAX_COORD { origin } else { Point::ZERO }
    }

    /// Where the rulers measure from with artboard `index` active: [`Self::ruler_origin_or_zero`]
    /// with global rulers; with artboard rulers that artboard's top-left corner moved by its own
    /// offset (the first artboard's when `index` is out of range, the global zero point when the
    /// document has no artboard or the result is out of the supported range).
    pub fn ruler_origin_for(&self, index: usize) -> Point {
        if self.rulers == RulerMode::Global {
            return self.ruler_origin_or_zero();
        }
        let Some(ab) = self.artboards.get(index).or_else(|| self.artboards.first()) else { return self.ruler_origin_or_zero() };
        let corner = Point::new(ab.rect.x0, ab.rect.y0);
        let offset = self.artboard_ruler_origins.get(&ab.id).copied().unwrap_or(Vec2::ZERO);
        let origin = corner + offset;
        if origin.is_finite() && origin.x.abs() <= MAX_COORD && origin.y.abs() <= MAX_COORD {
            origin
        } else if corner.is_finite() && corner.x.abs() <= MAX_COORD && corner.y.abs() <= MAX_COORD {
            corner
        } else {
            Point::ZERO
        }
    }

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
            ruler_origin: Point::ZERO,
            rulers: RulerMode::Artboard,
            artboard_ruler_origins: BTreeMap::new(),
            color_mode: mode,
            artboards: vec![Artboard {
                id: 1,
                name: "Artboard 1".into(),
                rect: Rect::new(0.0, 0.0, width, height),
                locked: false,
                show_center_mark: false,
                show_cross_hairs: false,
            }],
            layers: vec![],
            swatches: swatches.into(),
            swatch_groups: swatch_groups.into(),
            graphic_styles: default_graphic_styles().into(),
            char_styles: Default::default(),
            para_styles: Default::default(),
            text_threads: vec![],
            symbols: vec![],
            graph_designs: vec![],
            guides: vec![],
            views: vec![],
            saved_selections: Default::default(),
            variables: Default::default(),
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
            transparency_grid: false,
            layers_open: None,
            color_profiles: ColorProfiles::default(),
            export_settings: Default::default(),
            extra: Default::default(),
            slices: vec![],
            slices_clip_to_artboard: true,
            print_setup: None,
            assets: vec![],
            svg_scripts: vec![],
            puppet: None,
            new_art: Default::default(),
            dimensions: false,
            index: Default::default(),
        };
        let id = d.alloc_id();
        d.layers.push(Arc::new(Node::layer(id, "Layer 1", LayerColor::Preset(0))));
        d
    }

    /// Allocate a fresh node id.
    pub fn alloc_id(&mut self) -> NodeId {
        let id = NodeId(self.next_id);
        // Never reached from a counter [`Self::fix_next_id`] has seen (it stays under
        // [`MAX_NEXT_ID`]); saturating so that no counter can overflow.
        self.next_id = self.next_id.saturating_add(1);
        id
    }
    pub fn peek_next_id(&self) -> u64 {
        self.next_id
    }
    /// Ensure `next_id` is above every id in the tree (after deserializing foreign data).
    pub fn fix_next_id(&mut self) {
        // Ids from [`MAX_NEXT_ID`] up (a damaged or hand-made file: #1718) are left out, as is a
        // counter up there: new ids start under them and never reach them, where counting on from
        // the largest would run out and hand out ids the document already uses.
        let mut max = 0;
        let mut see = |id: u64| {
            if id < MAX_NEXT_ID {
                max = max.max(id);
            }
        };
        for l in &self.layers {
            l.walk(&mut |n| see(n.id.0));
        }
        self.slices.iter().for_each(|s| see(s.id.0));
        self.assets.iter().for_each(|a| see(a.id));
        let counter = Some(self.next_id).filter(|n| *n <= MAX_NEXT_ID).unwrap_or(0);
        self.next_id = counter.max(max.saturating_add(1));
    }

    /// Find a node anywhere in the tree.
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.index_path(id).and_then(|p| self.node_at(&p))
    }

    /// [`Document::node`] without building the lookup cache when there is none: for a look at a
    /// document kept in the undo history ([`Document::drop_lookup_cache`]).
    pub fn node_uncached(&self, id: NodeId) -> Option<&Node> {
        match self.index.is_built() {
            true => self.node(id),
            false => self.walk_index_path(id).and_then(|p| self.node_at(&p)),
        }
    }

    /// Let go of the lookup cache ([`id_index`]), as a document kept in the undo history does: a
    /// table per step would add up. Lookups still work; the first one builds the cache again.
    pub fn drop_lookup_cache(&self) {
        self.index.clear();
    }

    /// Whether the lookup cache ([`id_index`]) is built (see [`Document::drop_lookup_cache`]).
    pub fn has_lookup_cache(&self) -> bool {
        self.index.is_built()
    }

    /// Index path from the top-level layer list to `id` (e.g. `[0, 3, 1]`): the first place in
    /// tree order where `id` appears.
    pub fn index_path(&self, id: NodeId) -> Option<Vec<usize>> {
        // The cached path, while the node there still has the id ([`id_index`]).
        let cached = self.index.get();
        match cached.as_ref().and_then(|m| m.get(&id)) {
            Some(Some(p)) if self.node_at(&p).is_some_and(|n| n.id == id) => return Some(p),
            Some(None) => return self.walk_index_path(id),
            _ => {}
        }
        // A stale path (the tree changed), an id the cache doesn't have (deleted or added since),
        // or no cache yet: walk, and rebuild now and then.
        let found = self.walk_index_path(id);
        let stale = cached.as_ref().is_some_and(|m| m.contains_key(&id));
        if cached.is_none() || ((stale || found.is_some()) && self.index.missed()) {
            self.index.rebuild(&self.layers);
        }
        found
    }

    /// [`Document::index_path`] by walking the tree.
    fn walk_index_path(&self, id: NodeId) -> Option<Vec<usize>> {
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

    /// [`Document::index_path`] of every id in `ids` that is in the tree, in one walk of it: an
    /// edit of many objects looks them all up for the cost of one search instead of one each. Like
    /// `index_path`, an id that appears twice maps to its first place in tree order.
    pub fn index_paths(&self, ids: impl IntoIterator<Item = NodeId>) -> std::collections::HashMap<NodeId, Vec<usize>> {
        use std::collections::{HashMap, HashSet};
        fn walk(nodes: &[Arc<Node>], want: &HashSet<NodeId>, path: &mut Vec<usize>, out: &mut HashMap<NodeId, Vec<usize>>) -> bool {
            for (i, n) in nodes.iter().enumerate() {
                path.push(i);
                if want.contains(&n.id) && !out.contains_key(&n.id) {
                    out.insert(n.id, path.clone());
                    if out.len() == want.len() {
                        return true;
                    }
                }
                if let Some(ch) = n.children()
                    && walk(ch, want, path, out)
                {
                    return true;
                }
                path.pop();
            }
            false
        }
        let mut want: HashSet<NodeId> = ids.into_iter().collect();
        let mut out = HashMap::with_capacity(want.len());
        // Cached paths that still check out ([`id_index`]); the rest take one walk. For a good
        // share of the document, one walk is quicker than checking each cached path.
        if let Some(cache) = self.index.get().filter(|c| want.len().saturating_mul(8) < c.len()) {
            want.retain(|id| match cache.get(id) {
                Some(Some(p)) if self.node_at(&p).is_some_and(|n| n.id == *id) => {
                    out.insert(*id, p);
                    false
                }
                _ => true,
            });
        }
        if !want.is_empty() {
            let mut rest = HashMap::with_capacity(want.len());
            walk(&self.layers, &want, &mut Vec::new(), &mut rest);
            out.extend(rest);
        }
        out
    }

    /// The node at index path `path` (as [`Document::index_path`] gives it).
    pub fn node_at(&self, path: &[usize]) -> Option<&Node> {
        let (first, rest) = path.split_first()?;
        let mut cur: &Node = self.layers.get(*first)?;
        for &i in rest {
            cur = cur.children()?.get(i)?;
        }
        Some(cur)
    }

    /// Mutable access to the node at index path `path`; clones the Arc path (copy-on-write), as
    /// [`Document::node_mut`] does.
    pub fn node_mut_at(&mut self, path: &[usize]) -> Option<&mut Node> {
        let (first, rest) = path.split_first()?;
        let mut cur: &mut Arc<Node> = self.layers.get_mut(*first)?;
        for &i in rest {
            cur = Arc::make_mut(cur).children_mut()?.get_mut(i)?;
        }
        Some(Arc::make_mut(cur))
    }

    /// [`Document::ancestry`] of the node at index path `path`.
    pub fn ancestry_at(&self, path: &[usize]) -> Option<Vec<NodeId>> {
        let mut out = Vec::with_capacity(path.len());
        let mut nodes = &self.layers;
        for &i in path {
            let n = nodes.get(i)?;
            out.push(n.id);
            if let Some(ch) = n.children() {
                nodes = ch;
            }
        }
        Some(out)
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

    /// Remove the nodes `ids` names and return them in the order of `ids`, in one pass over the
    /// children of each parent. Each id is listed once and none lies inside another listed node, as
    /// in a list of top-level objects. For an id that is not in the tree, is listed twice or lies
    /// inside another listed node, it fails with `NoNode` and leaves the document as it was.
    pub fn remove_many(&mut self, ids: &[NodeId]) -> Result<Vec<Arc<Node>>, DocError> {
        let places = self.places(ids)?;
        let removed = self.take(places, ids.len())?;
        self.index.clear();
        Ok(removed)
    }

    /// Remove the nodes `ids` names, as [`Document::remove_many`] does, and put the node that `make`
    /// builds from them (in the order of `ids`) in the parent of the last of them, at that node's
    /// index less the number of listed nodes below it there. Returns the new node's id.
    pub fn wrap(&mut self, ids: &[NodeId], make: impl FnOnce(Vec<Arc<Node>>) -> Node) -> Result<NodeId, DocError> {
        let places = self.places(ids)?;
        let last = ids.len().checked_sub(1);
        let found = places.iter().find_map(|(up, kids)| kids.iter().find(|(_, k)| Some(*k) == last).map(|(i, _)| (up.clone(), *i)));
        let Some((up, i)) = found else { return Err(DocError::Invalid("nothing to wrap".into())) };
        // The parent and each node it is in move down by the listed nodes before them.
        let below = |parent: &[usize], index: usize| places.get(parent).map_or(0, |kids| kids.partition_point(|(j, _)| *j < index));
        let to: Vec<usize> = up.iter().enumerate().map(|(l, &j)| j.saturating_sub(below(&up[..l], j))).collect();
        let at = i.saturating_sub(below(&up, i));
        let node = make(self.take(places, ids.len())?);
        let id = node.id;
        let children = self.children_at(&to).ok_or_else(|| DocError::Invalid("the container of the listed objects is gone".into()))?;
        let at = at.min(children.len());
        children.insert(at, Arc::new(node));
        self.index.clear();
        Ok(id)
    }

    /// Put each list of nodes in `items` directly above its anchor (`replace`: in its place), in
    /// list order, in one pass over the children of each parent. Each anchor is listed once and none
    /// lies inside another. For an anchor that is not in the tree, is listed twice or lies inside
    /// another listed anchor, it fails with `NoNode` and leaves the document as it was.
    pub fn insert_beside(&mut self, items: Vec<(NodeId, Vec<Node>)>, replace: bool) -> Result<(), DocError> {
        let anchors: Vec<NodeId> = items.iter().map(|(id, _)| *id).collect();
        let places = self.places(&anchors)?;
        let mut lists: Vec<Vec<Node>> = items.into_iter().map(|(_, nodes)| nodes).collect();
        self.edit_children(places, |k, anchor, children| {
            if !replace {
                children.push(anchor);
            }
            let nodes = lists.get_mut(k).map(std::mem::take).unwrap_or_default();
            children.extend(nodes.into_iter().map(Arc::new));
        })?;
        self.index.clear();
        Ok(())
    }

    /// Where each of `ids` is ([`Places`]). Fails with `NoNode` for an id that is not in the tree,
    /// is listed twice or lies inside another listed node.
    fn places(&self, ids: &[NodeId]) -> Result<Places, DocError> {
        let paths = self.index_paths(ids.iter().copied());
        let mut places = Places::new();
        for (k, id) in ids.iter().enumerate() {
            let (&i, up) = paths.get(id).and_then(|p| p.split_last()).ok_or(DocError::NoNode(*id))?;
            match places.get_mut(up) {
                Some(kids) => kids.push((i, k)),
                None => {
                    places.insert(up.to_vec(), vec![(i, k)]);
                }
            }
        }
        for kids in places.values_mut() {
            kids.sort_unstable();
        }
        // An id listed twice has one path, so its parent lists the same child twice. A listed node
        // lies inside another when its parent's path, or a part of it from the top, is the path of a
        // listed node.
        let twice = places.values().flat_map(|kids| kids.windows(2)).find(|w| w[0].0 == w[1].0).map(|w| w[1].1);
        let listed = |path: &[usize]| {
            path.split_last().is_some_and(|(i, up)| places.get(up).is_some_and(|kids| kids.binary_search_by_key(i, |(j, _)| *j).is_ok()))
        };
        let nested = places.iter().find(|(up, _)| (1..=up.len()).any(|l| listed(&up[..l])));
        let inner = nested.and_then(|(_, kids)| kids.first()).map(|(_, k)| *k);
        if let Some(id) = twice.or(inner).and_then(|k| ids.get(k)) {
            return Err(DocError::NoNode(*id));
        }
        Ok(places)
    }

    /// Take the nodes `places` lists out of their parents and return them in the order of the list,
    /// which is `len` long.
    fn take(&mut self, places: Places, len: usize) -> Result<Vec<Arc<Node>>, DocError> {
        let mut taken = vec![None; len];
        self.edit_children(places, |k, node, _| {
            if let Some(t) = taken.get_mut(k) {
                *t = Some(node);
            }
        })?;
        Ok(taken.into_iter().flatten().collect())
    }

    /// Rebuild the children of each parent in `places` once, the parents later in tree order
    /// first. `each` gets each listed child, its place in the list and the parent's new children,
    /// onto which it pushes what takes the child's place.
    fn edit_children(&mut self, places: Places, mut each: impl FnMut(usize, Arc<Node>, &mut Vec<Arc<Node>>)) -> Result<(), DocError> {
        for (up, kids) in places.into_iter().rev() {
            let children = self.children_at(&up).ok_or_else(|| DocError::Invalid("a container of the listed objects is gone".into()))?;
            let old = std::mem::take(children);
            children.reserve(old.len());
            let mut kids = kids.into_iter().peekable();
            for (i, node) in old.into_iter().enumerate() {
                match kids.next_if(|(j, _)| *j == i) {
                    Some((_, k)) => each(k, node, children),
                    None => children.push(node),
                }
            }
        }
        Ok(())
    }

    /// The children of the node at index path `path`, or the top-level layers for an empty path.
    fn children_at(&mut self, path: &[usize]) -> Option<&mut Vec<Arc<Node>>> {
        match path {
            [] => Some(&mut self.layers),
            _ => self.node_mut_at(path)?.children_mut(),
        }
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
    /// The default name of a new layer or sublayer, "Layer N". Layers and sublayers share the
    /// numbering: it starts after the number of layers there are and skips any layer's name.
    pub fn next_layer_name(&self) -> String {
        let (mut count, mut taken) = (0, std::collections::HashSet::new());
        self.walk(|n| {
            if n.is_layer() {
                count += 1;
                taken.extend(n.name.as_deref());
            }
        });
        (count + 1..).map(|i| format!("Layer {i}")).find(|name| !taken.contains(name.as_str())).unwrap_or_default()
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
    /// Is the node (and every ancestor) visible and unlocked, and not on a locked artboard
    /// ([`Document::on_locked_artboard`])?
    pub fn is_editable(&self, id: NodeId) -> bool {
        self.is_editable_with(&self.locked_artboards(), id)
    }
    /// [`Document::is_editable`] with the locked artboards `locks` of this document, measured
    /// once for many ids.
    pub fn is_editable_with(&self, locks: &LockedArtboards, id: NodeId) -> bool {
        self.ancestry(id).is_some_and(|a| a.iter().all(|i| self.node(*i).is_some_and(|n| n.visible && !n.locked))) && !locks.holds_id(id)
    }
    /// [`Document::is_editable`] for many ids: the art on the locked artboards is measured once
    /// for all of them, a group once for all its members.
    pub fn editable(&self) -> impl Fn(NodeId) -> bool + '_ {
        let locks = self.locked_artboards();
        move |id| self.is_editable_with(&locks, id)
    }
    /// The locked artboards, for asking which art is on them ([`LockedArtboards::holds`]).
    pub fn locked_artboards(&self) -> LockedArtboards<'_> {
        self.locked_artboards_except(usize::MAX)
    }
    /// [`Document::locked_artboards`] leaving out artboard `index`.
    pub fn locked_artboards_except(&self, index: usize) -> LockedArtboards<'_> {
        // Art flush with an edge from outside can overlap it by a rounding error, as in
        // `art_bounds_on` (#1295).
        let rects: Vec<Rect> =
            self.artboards.iter().enumerate().filter(|(i, a)| a.locked && *i != index).map(|(_, a)| a.rect.inflate(-1e-6, -1e-6)).collect();
        // The objects of pattern editing mode's layer, a sublayer's objects included.
        fn objects(n: &Node, out: &mut std::collections::HashSet<NodeId>) {
            for c in n.children().into_iter().flatten() {
                if c.is_layer() {
                    objects(c, out);
                } else {
                    out.insert(c.id);
                }
            }
        }
        let mut pattern_art = std::collections::HashSet::new();
        if let Some(layer) = self.pattern_edit.as_ref().filter(|_| !rects.is_empty()).and_then(|e| self.layers.iter().find(|l| l.id == e.layer)) {
            objects(layer, &mut pattern_art);
        }
        LockedArtboards { rects, symbols: self.symbol_boxes(), new_art: self.new_art.get(), pattern_art, held: Default::default() }
    }
    /// Is `id` on a locked artboard ([`LockedArtboards::holds_id`])? A layer or sublayer is on none.
    pub fn on_locked_artboard(&self, id: NodeId) -> bool {
        self.locked_artboards().holds_id(id)
    }
    /// The object of a layer or sublayer that holds node `id`: `id` itself when it is one, else
    /// the group or other object around it right inside a layer. None for a layer or sublayer.
    pub fn layer_object(&self, id: NodeId) -> Option<&Node> {
        self.layer_object_at(&self.index_path(id)?)
    }
    /// [`Document::layer_object`] for the node at index path `path`.
    pub fn layer_object_at(&self, path: &[usize]) -> Option<&Node> {
        // Down the path to the first node that isn't a layer.
        let mut nodes = &self.layers;
        for &i in path {
            let n = nodes.get(i)?;
            if !n.is_layer() {
                return Some(n.as_ref());
            }
            nodes = n.children()?;
        }
        None
    }
    pub fn is_visible(&self, id: NodeId) -> bool {
        self.ancestry(id).is_some_and(|a| a.iter().all(|i| self.node(*i).is_some_and(|n| n.visible)))
    }
    /// The innermost layer or sublayer containing `id` (`id` itself when it is a layer).
    pub fn layer_containing(&self, id: NodeId) -> Option<NodeId> {
        let a = self.ancestry(id)?;
        a.iter().rev().copied().find(|i| self.node(*i).is_some_and(Node::is_layer))
    }
    /// Every art object the Selection tool can reach (Select All): the visible, unlocked objects
    /// of the visible, unlocked, non-template layers, looking through sublayers (bottom first),
    /// except those on a locked artboard.
    pub fn selectable_art(&self) -> Vec<NodeId> {
        let locks = self.locked_artboards();
        self.layers
            .iter()
            .filter(|l| l.visible && !l.locked && !l.is_template())
            .flat_map(|l| l.layer_art(true))
            .filter(|id| locks.is_empty() || !self.node(*id).is_some_and(|n| locks.holds(n)))
            .collect()
    }
    /// Colour of the innermost layer or sublayer containing `id` (selection highlight colour).
    pub fn layer_color(&self, id: NodeId) -> [u8; 3] {
        let l = self.layer_containing(id).and_then(|l| self.node(l));
        match l.map(|n| &n.kind) {
            Some(NodeKind::Layer { color, .. }) => color.rgb(),
            _ => LAYER_COLORS[0].1,
        }
    }
    /// Union of the geometric bounds of `ids` (their visual bounds when `visual`), with symbol
    /// instances measured by their art ([`Node::geometric_bounds_with`]).
    pub fn bounds_of(&self, ids: &[NodeId], visual: bool) -> Option<Rect> {
        let symbols = self.symbol_boxes();
        ids.iter().filter_map(|id| self.node(*id)).fold(None, |acc, n| {
            vectorcraft_geom::union_opt(acc, if visual { n.visual_bounds_with(&symbols) } else { n.geometric_bounds_with(&symbols) })
        })
    }
    /// Visual bounds of all art, with symbol instances measured by their art.
    pub fn art_bounds(&self) -> Option<Rect> {
        let symbols = self.symbol_boxes();
        self.layers.iter().fold(None, |acc, l| vectorcraft_geom::union_opt(acc, l.visual_bounds_with(&symbols)))
    }
    /// Artboard index containing point `p` (topmost = last), its edges included
    /// ([`Document::artboard_holding`]).
    pub fn artboard_at(&self, p: Point) -> Option<usize> {
        self.artboard_holding(p, true)
    }
    /// Index of the artboard that holds point `p`: the first (the last when `topmost`) that holds it
    /// inside or on its left or top edge, else the first (the last) that holds it by
    /// [`artboard_holds`]: on its right or bottom edge, or within a millionth of a point outside an
    /// edge. On an edge two artboards share, the artboard to the right of it or below it holds the
    /// point, except at the end of the edge that lies on that artboard's bottom or right edge, such
    /// as the point where both artboards' bottom edges meet; there the first (the last) holds it.
    pub fn artboard_holding(&self, p: Point, topmost: bool) -> Option<usize> {
        let pick = |on: &dyn Fn(Rect) -> bool| {
            let on = |i: &usize| self.artboards.get(*i).is_some_and(|a| on(a.rect));
            let mut all = 0..self.artboards.len();
            if topmost { all.rfind(on) } else { all.find(on) }
        };
        pick(&|r| r.contains(p)).or_else(|| pick(&|r| artboard_holds(r, p)))
    }
    /// The art that moves with an artboard at `rect`: top-level objects (children of layers and
    /// sublayers) lying entirely inside it, with symbol instances measured by their art. Locked and
    /// hidden objects and layers stay put unless `locked_and_hidden` (Selection & Anchor Display ›
    /// Move Locked and Hidden Artwork with Artboard).
    pub fn art_on_artboard(&self, rect: Rect, locked_and_hidden: bool) -> Vec<NodeId> {
        // Art drawn flush with the artboard can end up a rounding error outside it (a background
        // 3e-14 pt past the top edge, #1295): a millionth of a point is far below anything visible.
        let rect = rect.inflate(1e-6, 1e-6);
        fn collect(n: &Node, s: &SymbolBoxes, rect: Rect, all: bool, out: &mut Vec<NodeId>) {
            for c in n.children().into_iter().flatten().filter(|c| c.rides_with_artboard(all)) {
                if c.is_layer() {
                    collect(c, s, rect, all, out);
                } else if let Some(b) = c.geometric_bounds_with(s)
                    // Inside it, edges included: art flush with the right or bottom edge is on it too
                    // (`Rect::contains` leaves those out, #890).
                    && b.x0 >= rect.x0
                    && b.y0 >= rect.y0
                    && b.x1 <= rect.x1
                    && b.y1 <= rect.y1
                {
                    out.push(c.id);
                }
            }
        }
        let symbols = self.symbol_boxes();
        let mut art = vec![];
        for l in self.layers.iter().filter(|l| l.rides_with_artboard(locked_and_hidden)) {
            collect(l, &symbols, rect, locked_and_hidden, &mut art);
        }
        art
    }
    /// The art that moves, scales or is cut with artboard `index`: [`Document::art_on_artboard`]
    /// without the objects on another locked artboard, which stay unless `locked_and_hidden`, as
    /// locked art does.
    pub fn art_moving_with(&self, index: usize, locked_and_hidden: bool) -> Vec<NodeId> {
        let Some(rect) = self.artboards.get(index).map(|a| a.rect) else { return vec![] };
        let mut art = self.art_on_artboard(rect, locked_and_hidden);
        let locks = self.locked_artboards_except(index);
        if !locked_and_hidden && !locks.is_empty() {
            art.retain(|id| !self.node(*id).is_some_and(|n| locks.holds(n)));
        }
        art
    }
    /// Where ruler guide `g` runs along its line (the y range of a vertical guide): across its
    /// artboard for an artboard guide, None (the whole canvas) for a canvas guide or one whose
    /// artboard is gone.
    pub fn guide_span(&self, g: &Guide) -> Option<(f64, f64)> {
        let r = self.artboards.iter().find(|a| Some(a.id) == g.artboard)?.rect;
        Some(if g.vertical { (r.y0, r.y1) } else { (r.x0, r.x1) })
    }
    /// The layer (or sublayer) ruler guide `g` is on, if it is on one this document has.
    fn guide_layer(&self, g: &Guide) -> Option<NodeId> {
        g.layer.filter(|l| self.node(*l).is_some_and(Node::is_layer))
    }
    /// Is ruler guide `g` shown: on no layer, or on a layer that (with the layers around it) is
    /// visible? A guide whose layer is gone shows as one on no layer.
    pub fn guide_shown(&self, g: &Guide) -> bool {
        self.guide_layer(g).is_none_or(|l| self.is_visible(l))
    }
    /// Can ruler guide `g` be picked, moved and deleted: shown, and its layer unlocked?
    pub fn guide_editable(&self, g: &Guide) -> bool {
        self.guide_layer(g).is_none_or(|l| self.is_editable(l))
    }
    /// Put the ruler guides on no layer this document has (from files saved before guides had
    /// layers, or of layers merged into `layer`) on layer `layer`. Returns how many.
    pub fn rehome_guides(&mut self, layer: NodeId) -> usize {
        let lost: Vec<bool> = self.guides.iter().map(|g| self.guide_layer(g).is_none()).collect();
        let mut n = 0;
        for (g, lost) in self.guides.iter_mut().zip(lost) {
            if lost {
                g.layer = Some(layer);
                n += 1;
            }
        }
        n
    }
    /// Put the ruler guides on no layer (from files saved before guides had layers), or on one that
    /// is gone, on the top visible, unlocked layer ([`Self::default_layer`]). Returns how many.
    pub fn adopt_guides(&mut self) -> usize {
        self.default_layer().map_or(0, |home| self.rehome_guides(home))
    }
    /// Delete the ruler guides whose layer was deleted (they go with it), keeping `sel`'s guides in
    /// step. Guides on no layer stay. Returns how many went.
    pub fn drop_guides_of_deleted_layers(&mut self, sel: &mut Selection) -> usize {
        let gone: Vec<bool> = self.guides.iter().map(|g| g.layer.is_some() && self.guide_layer(g).is_none()).collect();
        self.retain_guides(sel, |i, _| !gone.get(i).copied().unwrap_or(false))
    }
    /// Copy the ruler guides of layer `from` and of its sublayers onto their copies in `copy` (made
    /// from `from` by [`Self::reid`]: the same tree with fresh ids), as duplicating a layer copies
    /// the objects on it. Returns how many.
    pub fn copy_layer_guides(&mut self, from: &Node, copy: &Node) -> usize {
        let layers = |n: &Node| {
            let mut out = vec![];
            n.walk(&mut |c| {
                if c.is_layer() {
                    out.push(c.id);
                }
            });
            out
        };
        let pairs: Vec<(NodeId, NodeId)> = layers(from).into_iter().zip(layers(copy)).collect();
        let copies: Vec<Guide> = self
            .guides
            .iter()
            .filter_map(|g| {
                let to = pairs.iter().find(|(a, _)| Some(*a) == g.layer)?.1;
                Some(Guide { layer: Some(to), ..g.clone() })
            })
            .collect();
        let n = copies.len();
        self.guides.extend(copies);
        n
    }
    /// The ruler guides on layer `layer` (not its sublayers'), by index.
    pub fn guides_on(&self, layer: NodeId) -> impl Iterator<Item = (usize, &Guide)> + '_ {
        self.guides.iter().enumerate().filter(move |(_, g)| g.layer == Some(layer))
    }
    /// Does ruler guide `g` run past `p` (up to `tol` beyond its ends)?
    pub fn guide_passes(&self, g: &Guide, p: Point, tol: f64) -> bool {
        let along = if g.vertical { p.y } else { p.x };
        self.guide_span(g).is_none_or(|(a, b)| along >= a - tol && along <= b + tol)
    }
    /// Keep the ruler guides `keep` accepts (by index), the selected ones left keeping their
    /// place among the rest (the selected art stays selected). Returns how many went.
    pub fn retain_guides(&mut self, sel: &mut Selection, keep: impl Fn(usize, &Guide) -> bool) -> usize {
        let kept: Vec<bool> = self.guides.iter().enumerate().map(|(i, g)| keep(i, g)).collect();
        // Where each kept guide ends up.
        let (mut to, mut n) = (Vec::with_capacity(kept.len()), 0);
        for k in &kept {
            to.push(k.then_some(n));
            n += usize::from(*k);
        }
        sel.guides = sel.guides.iter().filter_map(|i| to.get(*i).copied().flatten()).collect();
        let before = self.guides.len();
        let mut flags = kept.into_iter();
        self.guides.retain(|_| flags.next().unwrap_or(true));
        before - self.guides.len()
    }
    /// Move the guides of artboard `id` by `d` (along with their artboard).
    pub fn move_artboard_guides(&mut self, id: u32, d: Vec2) {
        for g in self.guides.iter_mut().filter(|g| g.artboard == Some(id)) {
            *g = g.moved(d);
        }
    }
    /// Map the guides of artboard `id` through `xf`, an axis-aligned scale and move (Scale Artwork
    /// with Artboard).
    pub fn map_artboard_guides(&mut self, id: u32, xf: Affine) {
        for g in self.guides.iter_mut().filter(|g| g.artboard == Some(id)) {
            g.pos = if g.vertical { (xf * Point::new(g.pos, 0.0)).x } else { (xf * Point::new(0.0, g.pos)).y };
        }
    }
    /// Copy the guides of artboard `from` onto artboard `to`, `d` away (with a copy of their
    /// artboard).
    pub fn copy_artboard_guides(&mut self, from: u32, to: u32, d: Vec2) {
        let copies: Vec<Guide> =
            self.guides.iter().filter(|g| g.artboard == Some(from)).map(|g| Guide { artboard: Some(to), ..g.moved(d) }).collect();
        self.guides.extend(copies);
    }
    pub fn next_artboard_id(&self) -> u32 {
        free_id(self.artboards.iter().map(|a| a.id))
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
        // A copied annotation must never bind its source ids to unrelated art in a different
        // document. Keep the measured geometry; new measurements can be linked after pasting.
        if let Some(e) = n.extras_if_any()
            && let Some(dimension) = &mut e.dimension
        {
            dimension.source = None;
        }
        if let Some(ch) = n.children_mut() {
            let old: Vec<Arc<Node>> = std::mem::take(ch);
            *ch = old.iter().map(|c| Arc::new(self.reid(c))).collect();
        }
        n
    }
}

impl Document {
    /// Saved selections as a file gives them, made safe to list and recall: at most
    /// [`SavedSelection::MAX`], each with a clean, unique name and naming objects this document
    /// has, each once. (An id it doesn't have would select whatever object takes that id later.)
    pub fn tidy_saved_selections(&mut self) {
        if self.saved_selections.is_empty() {
            return;
        }
        let mut ids = std::collections::HashSet::new();
        self.walk(|n| {
            ids.insert(n.id);
        });
        let mut kept: Vec<SavedSelection> = vec![];
        for s in std::mem::take(&mut self.saved_selections) {
            if kept.len() >= SavedSelection::MAX {
                break;
            }
            let Some(name) = SavedSelection::clean_name(&s.name) else { continue };
            if kept.iter().any(|k| k.name == name) {
                continue;
            }
            let mut seen = std::collections::HashSet::new();
            let objects = s.objects.into_iter().filter(|id| ids.contains(id) && seen.insert(*id)).collect();
            kept.push(SavedSelection { name, objects });
        }
        self.saved_selections = kept.into();
    }

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
        free_id(self.graphic_styles.iter().map(|g| g.id))
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
    /// `base`, or the first free "`base` N" (N from 2) when a symbol already has that name.
    pub fn unique_symbol_name(&self, base: &str) -> String {
        let taken = |n: &str| self.symbols.iter().any(|s| s.name == n);
        if !taken(base) {
            return base.to_string();
        }
        (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
    }

    /// Symbol Options › Registration for `name` (default: centre).
    pub fn symbol_registration(&self, name: &str) -> SymbolRegistration {
        self.unknown
            .get(SYMBOL_REGISTRATION)
            .and_then(|m| m.get(name))
            .and_then(|v| v.as_u64())
            .and_then(SymbolRegistration::from_u64)
            .unwrap_or_default()
    }

    /// Set or clear (when centre) the registration point stored for `name`.
    pub fn set_symbol_registration(&mut self, name: &str, reg: SymbolRegistration) {
        let m = self.unknown.entry(SYMBOL_REGISTRATION.to_string()).or_insert_with(|| serde_json::json!({}));
        let Some(o) = m.as_object_mut() else { return };
        if reg.is_center() {
            o.remove(name);
            if o.is_empty() {
                self.unknown.remove(SYMBOL_REGISTRATION);
            }
        } else {
            o.insert(name.into(), serde_json::json!(reg.0));
        }
    }

    /// Move the stored registration when a symbol is renamed.
    pub fn rename_symbol_registration(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        let reg = self.symbol_registration(from);
        self.set_symbol_registration(from, SymbolRegistration::CENTER);
        self.set_symbol_registration(to, reg);
    }

    /// Symbol Options › Export Type for `name` (default: Movie Clip).
    pub fn symbol_export_type(&self, name: &str) -> SymbolExportType {
        self.unknown.get(SYMBOL_EXPORT_TYPE).and_then(|m| m.get(name)).and_then(SymbolExportType::from_value).unwrap_or_default()
    }

    /// Set or clear (when Movie Clip, the default) the export type stored for `name`.
    pub fn set_symbol_export_type(&mut self, name: &str, kind: SymbolExportType) {
        if kind.is_default() {
            self.clear_symbol_export_type(name);
            return;
        }
        let m = self.unknown.entry(SYMBOL_EXPORT_TYPE.to_string()).or_insert_with(|| serde_json::json!({}));
        let Some(o) = m.as_object_mut() else { return };
        o.insert(name.into(), serde_json::json!(kind.id()));
    }

    /// Drop any stored export type for `name` (Movie Clip default).
    pub fn clear_symbol_export_type(&mut self, name: &str) {
        let Some(m) = self.unknown.get_mut(SYMBOL_EXPORT_TYPE) else { return };
        let Some(o) = m.as_object_mut() else { return };
        o.remove(name);
        if o.is_empty() {
            self.unknown.remove(SYMBOL_EXPORT_TYPE);
        }
    }

    /// Move the stored export type when a symbol is renamed.
    pub fn rename_symbol_export_type(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        let kind = self.symbol_export_type(from);
        self.clear_symbol_export_type(from);
        self.set_symbol_export_type(to, kind);
    }

    /// Symbol Options › 9-slice guides for `name`: `None` when 9-slice scaling is off (default).
    pub fn symbol_nine_slice(&self, name: &str) -> Option<SymbolNineSlice> {
        self.unknown.get(SYMBOL_NINE_SLICE).and_then(|m| m.get(name)).and_then(SymbolNineSlice::from_value)
    }

    /// Turn 9-slice scaling on with `guides`, or off with `None`, for `name`.
    pub fn set_symbol_nine_slice(&mut self, name: &str, guides: Option<SymbolNineSlice>) {
        let Some(g) = guides else {
            let Some(o) = self.unknown.get_mut(SYMBOL_NINE_SLICE).and_then(|m| m.as_object_mut()) else { return };
            o.remove(name);
            if o.is_empty() {
                self.unknown.remove(SYMBOL_NINE_SLICE);
            }
            return;
        };
        let m = self.unknown.entry(SYMBOL_NINE_SLICE.to_string()).or_insert_with(|| serde_json::json!({}));
        if let Some(o) = m.as_object_mut() {
            o.insert(name.into(), g.to_value());
        }
    }

    /// Move the stored 9-slice guides when a symbol is renamed.
    pub fn rename_symbol_nine_slice(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        let g = self.symbol_nine_slice(from);
        self.set_symbol_nine_slice(from, None);
        self.set_symbol_nine_slice(to, g);
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

/// Node, slice and asset ids are handed out below this: no document gets near it by counting, so
/// larger ids in a file are damage (or made by hand) and the counter never follows them (#1718).
const MAX_NEXT_ID: u64 = 1 << 62;

/// An id (never 0) none of `used` has: one past the largest, or, when that is the largest there
/// is (a damaged file), the smallest one free.
fn free_id(used: impl Iterator<Item = u32>) -> u32 {
    let mut used: Vec<u32> = used.collect();
    if let Some(next) = used.iter().max().copied().unwrap_or(0).checked_add(1) {
        return next;
    }
    used.sort_unstable();
    used.dedup();
    // Ids 1, 2, 3… as far as they are all taken: the first that isn't follows them.
    let taken = used.iter().filter(|&&id| id != 0).zip(1u32..).take_while(|&(&id, n)| id == n).count();
    u32::try_from(taken).map_or(u32::MAX, |n| n.saturating_add(1))
}

impl Document {
    /// Make the next id allocated at least `next`: ids handed out outside the layer tree (such
    /// as an importer's opacity mask and pattern art), which [`Document::fix_next_id`] doesn't see.
    pub fn reserve_ids(&mut self, next: u64) {
        self.next_id = self.next_id.max(next.min(MAX_NEXT_ID));
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
    fn art_on_artboard_takes_unlocked_objects_wholly_inside() {
        let (mut d, a, b) = doc_with_rects();
        let wide = Rect::new(-1.0, -1.0, 40.0, 15.0);
        assert_eq!(d.art_on_artboard(Rect::new(-1.0, -1.0, 15.0, 15.0), false), vec![a]);
        assert_eq!(d.art_on_artboard(Rect::new(-1.0, -1.0, 25.0, 15.0), false), vec![a], "b only half inside");
        // Flush with the artboard's edges counts as inside: the right and bottom ones too (#890).
        for board in [Rect::new(0.0, 0.0, 10.0, 10.0), Rect::new(-5.0, 0.0, 10.0, 15.0), Rect::new(0.0, -5.0, 15.0, 10.0)] {
            assert_eq!(d.art_on_artboard(board, false), vec![a], "{board:?}");
        }
        d.node_mut(a).unwrap().locked = true;
        assert_eq!(d.art_on_artboard(wide, false), vec![b]);
        // Move Locked and Hidden Artwork with Artboard (#394): hidden art stays too, unless on.
        d.node_mut(b).unwrap().visible = false;
        assert!(d.art_on_artboard(wide, false).is_empty());
        assert_eq!(d.art_on_artboard(wide, true), vec![a, b]);
        Arc::make_mut(&mut d.layers[0]).locked = true;
        assert!(d.art_on_artboard(wide, false).is_empty());
        assert_eq!(d.art_on_artboard(wide, true), vec![a, b], "a locked layer's art too");
    }

    #[test]
    fn art_on_artboard_tolerates_roundoff_at_each_edge() {
        let (d, a, _) = doc_with_rects();
        let roundoff = 1e-12;
        for board in [
            Rect::new(roundoff, 0.0, 10.0, 10.0),
            Rect::new(0.0, roundoff, 10.0, 10.0),
            Rect::new(0.0, 0.0, 10.0 - roundoff, 10.0),
            Rect::new(0.0, 0.0, 10.0, 10.0 - roundoff),
        ] {
            assert_eq!(d.art_on_artboard(board, false), vec![a], "roundoff at {board:?}");
        }
        // A real overhang remains outside.
        for board in
            [Rect::new(0.001, 0.0, 10.0, 10.0), Rect::new(0.0, 0.001, 10.0, 10.0), Rect::new(0.0, 0.0, 9.999, 10.0), Rect::new(0.0, 0.0, 10.0, 9.999)]
        {
            assert!(d.art_on_artboard(board, false).is_empty(), "overhang at {board:?}");
        }
    }

    /// The art on a locked artboard (each object of a layer whose bounds overlap it, a group as one
    /// object) can't be edited, and Select All, a click and a marquee pass over it; art flush with
    /// the artboard from outside isn't on it, a layer is on no artboard, and new art isn't on one.
    /// Unlocking the artboard gives the art back.
    #[test]
    fn art_on_a_locked_artboard_cannot_be_selected_or_edited() {
        use crate::hit::{HitOptions, hit_test, marquee, objects_at};
        let (mut d, a, b) = doc_with_rects();
        let layer = d.layers[0].id;
        d.artboards.push(Artboard {
            id: 2,
            name: "Artboard 2".into(),
            rect: Rect::new(700.0, 0.0, 900.0, 200.0),
            locked: false,
            show_center_mark: false,
            show_cross_hairs: false,
        });
        let rect = |d: &mut Document, r: Rect, appearance: Appearance| {
            let id = d.alloc_id();
            d.insert(Some(layer), usize::MAX, Node::path(id, shapes::rectangle(r), appearance)).unwrap();
            id
        };
        let c = rect(&mut d, Rect::new(710.0, 10.0, 730.0, 30.0), Appearance::default_art());
        // A group with one member on the artboard and one right of it.
        let (g, m1, m2) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        let members = [(m1, 750.0), (m2, 950.0)]
            .map(|(id, x)| Arc::new(Node::path(id, shapes::rectangle(Rect::new(x, 50.0, x + 10.0, 60.0)), Appearance::default_art())));
        d.insert(Some(layer), usize::MAX, Node::group(g, members.to_vec())).unwrap();
        // Unstroked and flush with the artboard's left edge, from outside.
        let e = rect(&mut d, Rect::new(680.0, 10.0, 700.0, 30.0), Appearance::default());
        let opt = HitOptions::default();
        let top = |d: &Document, x, y| hit_test(d, Point::new(x, y), opt).map(|h| h.top_object(None));
        let all = Rect::new(-10.0, -10.0, 1000.0, 800.0);
        assert_eq!(top(&d, 720.0, 20.0), Some(c));
        d.artboards[1].locked = true;
        for id in [c, g, m1, m2] {
            assert!(d.on_locked_artboard(id) && !d.is_editable(id), "{id:?} is on the locked artboard");
        }
        for id in [a, b, e, layer] {
            assert!(!d.on_locked_artboard(id) && d.is_editable(id), "{id:?} isn't");
        }
        assert_eq!(d.selectable_art(), [a, b, e]);
        assert_eq!(top(&d, 720.0, 20.0), None, "a click passes over c");
        assert_eq!(top(&d, 955.0, 55.0), None, "and over the group's member off the artboard");
        let isolated = HitOptions { scope: Some(g), ..opt };
        assert!(hit_test(&d, Point::new(755.0, 55.0), isolated).is_none(), "and inside the group isolated before the lock");
        assert!(objects_at(&d, Point::new(720.0, 20.0), opt, None).is_empty());
        assert_eq!(marquee(&d, all, None, false), [a, b, e]);
        assert_eq!(marquee(&d, all, None, true), [a, b, e], "the Direct Selection tool's marquee too");
        // Deselected in one pass, with the key object and the anchors.
        let mut sel = Selection::default();
        sel.set([a, c]);
        sel.key = Some(c);
        sel.anchors.insert(c, [(0, 0)].into());
        sel.deselect_on_locked_artboards(&d);
        assert_eq!((sel.objects.clone(), sel.key, sel.anchors.is_empty()), (vec![a], None, true));
        // A targeted layer stays targeted, with its art off the locked artboard selected.
        sel.set_target(&d, layer);
        sel.deselect_on_locked_artboards(&d);
        assert_eq!((sel.objects.clone(), sel.target), (vec![a, b, e], Some(layer)));
        // A sublayer with art on both artboards: its art off the locked artboard can still be clicked.
        let (sub, s1, s2) = (d.alloc_id(), d.alloc_id(), d.alloc_id());
        d.insert(Some(layer), usize::MAX, Node::layer(sub, "Sub", LayerColor::Preset(2))).unwrap();
        for (id, x, y) in [(s1, 300.0, 300.0), (s2, 800.0, 100.0)] {
            d.insert(Some(sub), usize::MAX, Node::path(id, shapes::rectangle(Rect::new(x, y, x + 10.0, y + 10.0)), Appearance::default_art()))
                .unwrap();
        }
        assert_eq!((top(&d, 305.0, 305.0), top(&d, 805.0, 105.0)), (Some(s1), None));
        assert!(!d.on_locked_artboard(sub));
        let mut both = Selection::default();
        both.set([s1, s2]);
        both.deselect_on_locked_artboards(&d);
        assert_eq!(both.objects, [s1]);
        // New art on it is editable and can be clicked while it is new art.
        d.new_art.set(Arc::new([c].into()));
        assert!(!d.on_locked_artboard(c) && d.is_editable(c), "new art");
        assert_eq!(top(&d, 720.0, 20.0), Some(c));
        d.new_art.set(Default::default());
        d.artboards[1].locked = false;
        assert_eq!(d.selectable_art(), [a, b, c, g, e, s1, s2]);
        assert_eq!(top(&d, 955.0, 55.0), Some(g));
        assert_eq!(marquee(&d, all, None, false), [a, b, c, g, e, s1, s2]);
    }

    #[test]
    fn art_on_artboard_measures_a_symbol_instance_by_its_art() {
        let mut d = Document::new(612.0, 792.0);
        let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default_art());
        d.symbols.push(Symbol { name: "Box".into(), art: Arc::new(art) });
        let (layer, id) = (d.layers[0].id, d.alloc_id());
        d.insert(Some(layer), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Box".into(), xf: Affine::translate((2.0, 2.0)) })).unwrap();
        // The art, (2, 2) to (12, 12), lies inside; the 20 × 20 square, (-8, -8) to (12, 12), doesn't.
        assert_eq!(d.art_on_artboard(Rect::new(0.0, 0.0, 15.0, 15.0), false), vec![id]);
    }

    #[test]
    fn artboard_at_counts_the_right_and_bottom_edges() {
        // Artboard 0 is (0, 0) to (100, 100), artboard 1 is to its right and artboard 2 below it.
        let mut d = Document::new(100.0, 100.0);
        for rect in [Rect::new(100.0, 0.0, 200.0, 100.0), Rect::new(0.0, 100.0, 100.0, 200.0)] {
            let mut a = d.artboards[0].clone();
            a.rect = rect;
            d.artboards.push(a);
        }
        let held = |d: &Document, x: f64, y: f64| (d.artboard_at(Point::new(x, y)), d.artboard_holding(Point::new(x, y), false));
        // Each artboard's own right and bottom edges, edges with roundoff, and points beyond them.
        assert_eq!(held(&d, 200.0, 50.0), (Some(1), Some(1)), "right edge");
        assert_eq!(held(&d, 150.0, 100.0), (Some(1), Some(1)), "bottom edge");
        assert_eq!(held(&d, 200.0, 100.0), (Some(1), Some(1)), "bottom right corner");
        assert_eq!(held(&d, 100.0, 150.0), (Some(2), Some(2)), "right edge");
        assert_eq!(held(&d, 50.0, 200.0 + 1e-9), (Some(2), Some(2)), "roundoff past the bottom edge");
        assert_eq!(held(&d, -1e-9, 50.0), (Some(0), Some(0)), "roundoff past the left edge");
        assert_eq!(held(&d, 200.001, 50.0), (None, None));
        assert_eq!(held(&d, 50.0, 200.001), (None, None));
        // On an edge two artboards share, the artboard to the right of it or below it holds the
        // point, in either list order, except at the end of the edge that lies on that artboard's
        // bottom or right edge.
        assert_eq!(held(&d, 100.0, 50.0), (Some(1), Some(1)));
        assert_eq!(held(&d, 50.0, 100.0), (Some(2), Some(2)));
        // The bottom edges of artboards 0 and 1 meet at (100, 100), which is also artboard 2's top
        // right corner: the first holds the point, or the last when `topmost`.
        assert_eq!(held(&d, 100.0, 100.0), (Some(2), Some(0)), "shared corner");
        d.artboards.swap(0, 1);
        assert_eq!(held(&d, 100.0, 50.0), (Some(0), Some(0)));
        // Where two artboards overlap, the topmost holds the point, or the first one when not
        // `topmost`.
        let mut inner = d.artboards[1].clone();
        inner.rect = Rect::new(10.0, 10.0, 90.0, 90.0);
        d.artboards.push(inner);
        assert_eq!(held(&d, 50.0, 50.0), (Some(3), Some(1)));
        assert_eq!(held(&d, 90.0, 50.0), (Some(1), Some(1)), "inside artboard 1, on the right edge of artboard 3");
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

    /// #1718: a file whose ids or counters are at their type's largest value still hands out ids
    /// nothing in it uses (and never 0), instead of overflowing.
    #[test]
    fn exhausted_ids_in_a_file_dont_run_out() {
        let (d, a, b) = doc_with_rects();
        let mut v = serde_json::to_value(&d).unwrap();
        v["next_id"] = serde_json::json!(u64::MAX);
        // Object `a` gets the largest id there is.
        fn renumber(v: &mut serde_json::Value, from: u64) {
            match v {
                serde_json::Value::Object(o) => {
                    if o.get("id").and_then(serde_json::Value::as_u64) == Some(from) {
                        o.insert("id".into(), serde_json::json!(u64::MAX));
                    }
                    o.values_mut().for_each(|c| renumber(c, from));
                }
                serde_json::Value::Array(a) => a.iter_mut().for_each(|c| renumber(c, from)),
                _ => {}
            }
        }
        renumber(&mut v["layers"], a.0);
        let mut d: Document = serde_json::from_value(v).unwrap();
        assert!(d.node(NodeId(u64::MAX)).is_some(), "the file's largest id is kept");
        d.fix_next_id();
        let fresh: Vec<NodeId> = (0..3).map(|_| d.alloc_id()).collect();
        assert!(fresh.iter().all(|id| d.node(*id).is_none() && id.0 > b.0 && id.0 < MAX_NEXT_ID), "{fresh:?}");
        assert!(fresh[0] != fresh[1] && fresh[1] != fresh[2]);
        // A counter left at the end by other means stays there rather than wrapping to 0.
        d.next_id = u64::MAX;
        assert_eq!((d.alloc_id(), d.peek_next_id()), (NodeId(u64::MAX), u64::MAX));
        d.reserve_ids(u64::MAX);
        // Artboards and graphic styles: one past the largest, else the smallest id free.
        assert_eq!(free_id([1, 2, 7].into_iter()), 8);
        assert_eq!(free_id([u32::MAX, 0, 1, 2, 4].into_iter()), 3);
        assert_eq!(free_id([u32::MAX].into_iter()), 1);
        d.artboards[0].id = u32::MAX;
        assert_eq!(d.next_artboard_id(), 1);
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

    #[test]
    fn negative_picas_and_points_sign_applies_to_whole_value() {
        assert_eq!(Unit::Points.parse("-2p6"), Some(-30.0));
        assert_eq!(Unit::Points.parse("-0p6"), Some(-6.0));
        assert_eq!(Unit::Points.parse("-2p"), Some(-24.0));
        assert_eq!(Unit::Points.parse("2p6"), Some(30.0));
    }
}

#[cfg(test)]
mod tests_lookups;
#[cfg(test)]
mod tests_slices;
#[cfg(test)]
mod tests_units;
