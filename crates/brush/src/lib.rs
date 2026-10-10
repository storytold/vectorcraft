//! VectorCraft brushes: definitions (calligraphic, scatter, art, pattern, bristle), the default
//! library, and the geometry that turns a brushed stroke into filled art.
//!
//! Brush definitions live in the document under `Document::unknown["brushes"]` (a JSON array of
//! [`Brush`]). A document without that key uses the default library ([`defaults`]); the first
//! edit to the library materialises the full list into the document. Strokes refer to brushes by
//! name (`StrokeLayer::brush`).
//!
//! [`stroke_pieces`] is the single source of brush geometry: the renderer paints the pieces it
//! returns (caching them per node) and `object.expandBrush` inserts them as real objects.
#![forbid(unsafe_code)]

mod calli;
mod colorize;
mod defaults;
pub mod track;
mod warp;

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, Node, NodeId, NodeKind, StrokeLayer};
use vectorcraft_geom::{BezPath, Rect};

pub use colorize::{colorize, instance_art, stain, tint_node};
pub use defaults::defaults;
pub use serde_json;

/// Key of the brush library in `Document::unknown`.
pub const DOC_KEY: &str = "brushes";
/// Key of the current brush name (used by the Paintbrush tool) in `Document::unknown`.
pub const CURRENT_KEY: &str = "currentBrush";

/// A named brush.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Brush {
    pub name: String,
    #[serde(flatten)]
    pub kind: BrushKind,
}

// Brushes are few and long-lived; boxing the art-carrying variants buys nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum BrushKind {
    Calligraphic(Calligraphic),
    Scatter(Scatter),
    Art(ArtBrush),
    Pattern(PatternBrush),
    Bristle(Bristle),
}

impl BrushKind {
    pub fn label(&self) -> &'static str {
        match self {
            BrushKind::Calligraphic(_) => "Calligraphic",
            BrushKind::Scatter(_) => "Scatter",
            BrushKind::Art(_) => "Art",
            BrushKind::Pattern(_) => "Pattern",
            BrushKind::Bristle(_) => "Bristle",
        }
    }
    pub fn type_id(&self) -> &'static str {
        match self {
            BrushKind::Calligraphic(_) => "calligraphic",
            BrushKind::Scatter(_) => "scatter",
            BrushKind::Art(_) => "art",
            BrushKind::Pattern(_) => "pattern",
            BrushKind::Bristle(_) => "bristle",
        }
    }
}

/// How brush art takes the stroke colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "method")]
pub enum Colorization {
    /// Keep the art's own colours.
    #[default]
    None,
    /// Black becomes the stroke colour, lighter colours become tints of it, white stays white.
    Tints,
    /// Black and white stay; mid tones become the stroke colour, darker ones shades of it.
    TintsAndShades,
    /// The key colour becomes the stroke colour; other colours rotate by the same hue offset.
    HueShift { key: vectorcraft_color::Color },
}

/// Calligraphic brush: an elliptical nib swept along the path. Sizes are at 1 pt stroke weight
/// (the stroke weight scales the brush).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calligraphic {
    /// Nib angle in degrees, counter-clockwise from horizontal.
    pub angle: f64,
    /// Minor/major axis ratio in percent (1..=100).
    pub roundness: f64,
    /// Nib diameter in points.
    pub size: f64,
    /// How far angle (°), roundness (%) and size (pt) vary each way, as [`Self::modes`] say.
    pub variation: [f64; 3],
    /// How angle, roundness and size vary. `None` (brushes saved before there were modes): Random
    /// where the variation is non-zero, else Fixed — see [`Calligraphic::modes`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modes: Option<[Variation; 3]>,
}

impl Default for Calligraphic {
    fn default() -> Self {
        Self { angle: 0.0, roundness: 100.0, size: 3.0, variation: [0.0; 3], modes: None }
    }
}

impl Calligraphic {
    /// The ranges of angle (°), roundness (%) and size (pt), and the most each varies.
    pub const RANGES: [(f64, f64); 3] = [(-180.0, 180.0), (0.0, 100.0), (0.0, 1296.0)];
    pub const MAX_VARIATION: [f64; 3] = [180.0, 100.0, 1296.0];

    /// How angle, roundness and size vary.
    pub fn modes(&self) -> [Variation; 3] {
        self.modes.unwrap_or_else(|| self.variation.map(|v| if v != 0.0 { Variation::Random } else { Variation::Fixed }))
    }
    /// Does the pen's pressure change the nib?
    pub fn uses_pressure(&self) -> bool {
        self.modes().contains(&Variation::Pressure)
    }
}

/// How a Calligraphic brush's angle, roundness or size varies (Brush Options). Saved by name: new
/// modes are appended, never renamed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Variation {
    /// Always the value.
    #[default]
    Fixed,
    /// A random value within the variation either side, new for each stroke.
    Random,
    /// From the value less the variation at the lightest pen pressure to the value plus the
    /// variation at the heaviest.
    Pressure,
}

impl Variation {
    pub const ALL: [Variation; 3] = [Variation::Fixed, Variation::Random, Variation::Pressure];
    /// The name brush definitions use.
    pub fn id(self) -> &'static str {
        match self {
            Variation::Fixed => "fixed",
            Variation::Random => "random",
            Variation::Pressure => "pressure",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Variation::Fixed => "Fixed",
            Variation::Random => "Random",
            Variation::Pressure => "Pressure",
        }
    }
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.id() == id)
    }
}

/// Scatter brush: copies of the art along the path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Scatter {
    pub art: Node,
    /// Size range in percent of the art.
    pub size: (f64, f64),
    /// Spacing range in percent of the (scaled) art width.
    pub spacing: (f64, f64),
    /// Perpendicular offset range in percent of the (scaled) art height.
    pub scatter: (f64, f64),
    /// Rotation range in degrees.
    pub rotation: (f64, f64),
    /// Rotation relative to the path direction (else to the page).
    pub rotation_relative_to_path: bool,
    pub colorization: Colorization,
}

impl Default for Scatter {
    fn default() -> Self {
        Self {
            art: defaults::dot_art(),
            size: (100.0, 100.0),
            spacing: (100.0, 100.0),
            scatter: (0.0, 0.0),
            rotation: (0.0, 0.0),
            rotation_relative_to_path: false,
            colorization: Colorization::None,
        }
    }
}

/// Direction the art runs in the brush definition.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    #[default]
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
}

/// How art brush art is scaled along the path.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "mode")]
pub enum ArtScale {
    /// Keep the art's proportions (length follows the path; width scales with it).
    Proportional,
    /// Stretch the art to the path length.
    #[default]
    Stretch,
    /// Keep the ends outside the guides at their size and stretch the middle.
    /// `start`/`end` are fractions (0..1) of the art length.
    BetweenGuides { start: f64, end: f64 },
}

/// Art brush: one copy of the art stretched along the whole path.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ArtBrush {
    pub art: Node,
    pub direction: Direction,
    pub scale: ArtScale,
    /// Width in percent.
    pub width: f64,
    pub flip_along: bool,
    pub flip_across: bool,
    pub colorization: Colorization,
}

impl Default for ArtBrush {
    fn default() -> Self {
        Self {
            art: defaults::taper_art(),
            direction: Direction::LeftToRight,
            scale: ArtScale::Stretch,
            width: 100.0,
            flip_along: false,
            flip_across: false,
            colorization: Colorization::Tints,
        }
    }
}

/// How pattern tiles fit the path length.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PatternFit {
    /// Stretch tiles so a whole number fits.
    #[default]
    Stretch,
    /// Keep tile size and add space between tiles.
    AddSpace,
    /// Keep tile size; the path is approximated (tiles may overrun slightly).
    Approximate,
}

/// Pattern brush: tiles repeated along the path, with corner and end tiles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PatternBrush {
    pub side: Node,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outer_corner: Option<Node>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inner_corner: Option<Node>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<Node>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end: Option<Node>,
    /// Scale in percent.
    pub scale: f64,
    /// Space between tiles in percent of the tile width.
    pub spacing: f64,
    pub fit: PatternFit,
    pub flip_along: bool,
    pub flip_across: bool,
    pub colorization: Colorization,
}

impl Default for PatternBrush {
    fn default() -> Self {
        Self {
            side: defaults::stitch_tile(),
            outer_corner: None,
            inner_corner: None,
            start: None,
            end: None,
            scale: 100.0,
            spacing: 0.0,
            fit: PatternFit::Stretch,
            flip_along: false,
            flip_across: false,
            colorization: Colorization::Tints,
        }
    }
}

/// Bristle tip shapes (names only affect the strand distribution here).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BristleShape {
    #[default]
    RoundPoint,
    FlatPoint,
    RoundBlunt,
    FlatBlunt,
    RoundCurve,
    FlatCurve,
    RoundAngle,
    FlatAngle,
    RoundFan,
    FlatFan,
}

/// Bristle brush, approximated by several thin, semi-transparent calligraphic strands.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Bristle {
    pub shape: BristleShape,
    /// Brush size in points.
    pub size: f64,
    /// Bristle length in percent (25..300) — longer bristles start/end more raggedly.
    pub length: f64,
    /// Density in percent (1..100) — number of strands.
    pub density: f64,
    /// Thickness in percent (1..100) — strand width.
    pub thickness: f64,
    /// Paint opacity in percent (1..100).
    pub opacity: f64,
    /// Stiffness in percent (1..100) — how straight strands follow the path.
    pub stiffness: f64,
}

impl Default for Bristle {
    fn default() -> Self {
        Self { shape: BristleShape::RoundPoint, size: 6.0, length: 100.0, density: 50.0, thickness: 40.0, opacity: 75.0, stiffness: 50.0 }
    }
}

// ---------- library ----------

/// The brush library of `doc`: its stored brushes, or the defaults when it has none.
pub fn library(doc: &Document) -> Vec<Brush> {
    match doc.unknown.get(DOC_KEY) {
        Some(v) => parse_library(v),
        None => defaults().to_vec(),
    }
}

/// Parse a stored library value (malformed entries are skipped).
pub fn parse_library(v: &serde_json::Value) -> Vec<Brush> {
    v.as_array().map(|a| a.iter().filter_map(|b| serde_json::from_value(b.clone()).ok()).collect()).unwrap_or_default()
}

/// Store `lib` as the document's brush library.
pub fn store(doc: &mut Document, lib: &[Brush]) {
    doc.unknown.insert(DOC_KEY.into(), serde_json::to_value(lib).unwrap_or_default());
}

/// Find a brush by name.
pub fn find(doc: &Document, name: &str) -> Option<Brush> {
    library(doc).into_iter().find(|b| b.name == name)
}

/// The current brush name (Paintbrush tool, Brushes panel highlight).
pub fn current(doc: &Document) -> Option<String> {
    doc.unknown.get(CURRENT_KEY).and_then(|v| v.as_str()).map(str::to_string)
}

/// A unique name based on `base` ("Base", "Base 2", …).
pub fn unique_name(lib: &[Brush], base: &str) -> String {
    if !lib.iter().any(|b| b.name == base) {
        return base.to_string();
    }
    // At most `lib.len()` names are taken, so one of the first `lib.len() + 1` candidates is free.
    (2..=lib.len() + 2)
        .map(|i| format!("{base} {i}"))
        .find(|n| !lib.iter().any(|b| &b.name == n))
        .unwrap_or_else(|| format!("{base} {}", lib.len() + 2))
}

// ---------- geometry ----------

/// Flattening tolerance for brush geometry, relative to its size.
fn tolerance(scale: f64) -> f64 {
    (0.02 * scale).clamp(0.01, 0.25)
}

/// Filled art for one brushed stroke along `bp`. Pieces are in document space with id 0; their
/// appearances carry no brushes (so they never recurse). Stroke-level opacity/blend are the
/// caller's job (`StrokeLayer::opacity`).
pub fn stroke_pieces(brush: &Brush, bp: &BezPath, stroke: &StrokeLayer) -> Vec<Node> {
    let w = if stroke.width > 0.0 { stroke.width } else { 1.0 };
    let mut out = match &brush.kind {
        BrushKind::Calligraphic(c) => calli::calligraphic(c, bp, w, stroke, &brush.name),
        BrushKind::Bristle(b) => calli::bristle(b, bp, w, &stroke.paint, &brush.name),
        BrushKind::Art(a) => warp::art(a, bp, w),
        BrushKind::Scatter(s) => warp::scatter(s, bp, w, &brush.name),
        BrushKind::Pattern(p) => warp::pattern(p, bp, w),
    };
    let colorization = match &brush.kind {
        BrushKind::Art(a) => a.colorization,
        BrushKind::Scatter(s) => s.colorization,
        BrushKind::Pattern(p) => p.colorization,
        _ => Colorization::None,
    };
    for n in &mut out {
        colorize(n, colorization, &stroke.paint);
    }
    out
}

/// Pieces for every brushed stroke of `n` (a path or compound path), in appearance order, each
/// with the stroke's opacity folded into the piece opacity. `None` if nothing is brushed.
pub fn node_pieces(doc: &Document, n: &Node) -> Option<Vec<Node>> {
    let bp = node_bezpath(n)?;
    let lib = library(doc);
    let mut out = vec![];
    let mut any = false;
    for item in &n.appearance.items {
        let AppearanceItem::Stroke(st) = item else { continue };
        let Some(b) = st.brush.as_deref().and_then(|name| lib.iter().find(|x| x.name == name)) else { continue };
        if !st.visible || st.paint.is_none() {
            any = true;
            continue;
        }
        any = true;
        for mut p in stroke_pieces(b, &bp, st) {
            p.opacity *= st.opacity;
            if st.blend != vectorcraft_color::BlendMode::Normal {
                p.blend = st.blend;
            }
            out.push(p);
        }
    }
    any.then_some(out)
}

/// The outline of a path or compound path as one BezPath.
pub fn node_bezpath(n: &Node) -> Option<BezPath> {
    match &n.kind {
        NodeKind::Path { path, .. } => Some(path.to_bezpath()),
        NodeKind::Compound { children, .. } => {
            let mut bp = BezPath::new();
            for c in children {
                if let Some(p) = c.path_data() {
                    bp.extend(p.to_bezpath());
                }
            }
            Some(bp)
        }
        _ => None,
    }
}

/// Does `n` have a stroke with a brush?
pub fn has_brush(n: &Node) -> bool {
    n.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(s) if s.brush.is_some() && s.visible))
}

/// Expand a brushed path: a group of [the path with its fills and unbrushed strokes (if any
/// paint remains), the brush art…]. Ids are 0 (the caller re-ids). `None` if not brushed.
pub fn expand(doc: &Document, n: &Node) -> Option<Node> {
    let pieces = node_pieces(doc, n)?;
    let mut children: Vec<Arc<Node>> = vec![];
    let mut base = n.clone();
    base.appearance.items.retain(|i| !matches!(i, AppearanceItem::Stroke(s) if s.brush.is_some()));
    let painted = base.appearance.items.iter().any(|i| match i {
        AppearanceItem::Fill(f) => f.visible && !f.paint.is_none(),
        AppearanceItem::Stroke(s) => s.visible && !s.paint.is_none() && s.width > 0.0,
    });
    if painted {
        base.id = NodeId(0);
        children.push(Arc::new(base));
    }
    children.extend(pieces.into_iter().map(Arc::new));
    let mut g = Node::group(NodeId(0), children);
    g.opacity = n.opacity;
    g.blend = n.blend;
    g.name = n.name.clone();
    Some(g)
}

/// A filled path node painted with `paint` (brush output).
pub(crate) fn filled(path: vectorcraft_geom::PathData, paint: &Paint) -> Node {
    Node::path(NodeId(0), path, Appearance { items: vec![AppearanceItem::Fill(FillLayer::new(paint.clone()))], ..Default::default() })
}

/// A sample stroke for previews (a gentle S-curve in a `w`×`h` box).
pub fn preview_path(w: f64, h: f64) -> BezPath {
    let mut bp = BezPath::new();
    let (x0, x1) = (h * 0.5, w - h * 0.5);
    bp.move_to((x0, h * 0.5));
    bp.curve_to((x0 + (x1 - x0) * 0.35, h * 0.05), (x0 + (x1 - x0) * 0.65, h * 0.95), (x1, h * 0.5));
    bp
}

/// Union of the geometric bounds of `nodes`.
pub fn pieces_bounds(nodes: &[Node]) -> Option<Rect> {
    nodes.iter().fold(None, |acc, n| vectorcraft_geom::union_opt(acc, n.visual_bounds()))
}

#[cfg(test)]
mod tests;
