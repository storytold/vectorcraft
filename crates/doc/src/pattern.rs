//! Pattern swatches (Object → Pattern) and live Repeats (Object → Repeat): data and pure math.
//!
//! **Patterns.** A [`PatternDef`] is a named tile: art plus a tile rectangle and a tile type
//! (grid, brick by row/column, hex by column/row). Pattern space has the tile's top-left at the
//! origin; instance `(i, j)` is the art translated by `-tile.origin + tile_offset(i, j)`. A
//! [`Paint::Pattern`] maps pattern space into the document with its own `xf` (identity = tiles
//! anchored at the document origin, like Illustrator's ruler origin). Every tiling is periodic
//! on a rectangular *super-tile* ([`PatternDef::period`]), which is what renderers rasterize and
//! what SVG `<pattern>` elements repeat.
//!
//! **Repeats.** A [`RepeatSpec`] holds source art and a radial / grid / mirror arrangement; the
//! instances are the source under [`repeat_transforms`] (evaluated on demand like blends).

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use vectorcraft_color::Paint;
use vectorcraft_geom::{Affine, Point, Rect, Vec2};

use crate::Document;
use crate::appearance::AppearanceItem;
use crate::node::{Node, NodeId, NodeKind};

fn half() -> f64 {
    0.5
}
fn five() -> u32 {
    5
}
fn seventy() -> f32 {
    70.0
}
fn yes() -> bool {
    true
}
fn three() -> u32 {
    3
}
fn full_turn() -> f64 {
    360.0
}

// =====================================================================================
// Patterns
// =====================================================================================

/// Pattern Options → Tile Type.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TileType {
    #[default]
    Grid,
    /// Rows shifted horizontally by `offset` × tile width (0.5 = half brick).
    BrickByRow {
        #[serde(default = "half")]
        offset: f64,
    },
    /// Columns shifted vertically by `offset` × tile height.
    BrickByColumn {
        #[serde(default = "half")]
        offset: f64,
    },
    /// Hexagonal tiles in columns: columns ¾ tile width apart, odd columns shifted half a tile down.
    HexByColumn,
    /// Hexagonal tiles in rows: rows ¾ tile height apart, odd rows shifted half a tile right.
    HexByRow,
}

impl TileType {
    pub fn id(&self) -> &'static str {
        match self {
            TileType::Grid => "grid",
            TileType::BrickByRow { .. } => "brickByRow",
            TileType::BrickByColumn { .. } => "brickByColumn",
            TileType::HexByColumn => "hexByColumn",
            TileType::HexByRow => "hexByRow",
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            TileType::Grid => "Grid",
            TileType::BrickByRow { .. } => "Brick by Row",
            TileType::BrickByColumn { .. } => "Brick by Column",
            TileType::HexByColumn => "Hex by Column",
            TileType::HexByRow => "Hex by Row",
        }
    }
    /// Parse an id (`grid`, `brickByRow`, `brick-row`, …) with an optional brick offset.
    pub fn parse(s: &str, offset: Option<f64>) -> Option<Self> {
        let o = offset.unwrap_or(0.5);
        Some(match s.to_ascii_lowercase().replace(['-', '_', ' '], "").as_str() {
            "grid" => TileType::Grid,
            "brickbyrow" | "brickrow" | "brick" => TileType::BrickByRow { offset: o },
            "brickbycolumn" | "brickcolumn" | "brickcol" => TileType::BrickByColumn { offset: o },
            "hexbycolumn" | "hexcolumn" | "hexcol" | "hex" => TileType::HexByColumn,
            "hexbyrow" | "hexrow" => TileType::HexByRow,
            _ => return None,
        })
    }
    pub const IDS: [&'static str; 5] = ["grid", "brickByRow", "brickByColumn", "hexByColumn", "hexByRow"];

    /// Column / row steps for a `w`×`h` tile (the hex types pack columns/rows at ¾).
    pub fn steps(&self, w: f64, h: f64) -> (f64, f64) {
        match self {
            TileType::HexByColumn => (w * 0.75, h),
            TileType::HexByRow => (w, h * 0.75),
            _ => (w, h),
        }
    }
}

/// Position of instance (column `i`, row `j`) for a `w`×`h` tile. `(0, 0)` is at the origin.
pub fn tile_offset(tt: TileType, w: f64, h: f64, i: i64, j: i64) -> Vec2 {
    let (i_f, j_f) = (i as f64, j as f64);
    let frac = |v: f64| v - v.floor();
    match tt {
        TileType::Grid => Vec2::new(i_f * w, j_f * h),
        TileType::BrickByRow { offset } => Vec2::new(i_f * w + frac(j_f * offset) * w, j_f * h),
        TileType::BrickByColumn { offset } => Vec2::new(i_f * w, j_f * h + frac(i_f * offset) * h),
        TileType::HexByColumn => Vec2::new(i_f * w * 0.75, j_f * h + if i.rem_euclid(2) == 1 { h / 2.0 } else { 0.0 }),
        TileType::HexByRow => Vec2::new(i_f * w + if j.rem_euclid(2) == 1 { w / 2.0 } else { 0.0 }, j_f * h * 0.75),
    }
}

/// Smallest `q` in 1..=24 with `q·offset` (almost) an integer (24 when none is).
fn brick_repeat(offset: f64) -> f64 {
    for q in 1..=24 {
        let v = q as f64 * offset;
        if (v - v.round()).abs() < 1e-6 {
            return q as f64;
        }
    }
    24.0
}

/// Size of the rectangular super-tile a tiling repeats on.
pub fn tile_period(tt: TileType, w: f64, h: f64) -> (f64, f64) {
    match tt {
        TileType::Grid => (w, h),
        TileType::BrickByRow { offset } => (w, h * brick_repeat(offset)),
        TileType::BrickByColumn { offset } => (w * brick_repeat(offset), h),
        TileType::HexByColumn => (w * 1.5, h),
        TileType::HexByRow => (w, h * 1.5),
    }
}

/// Pattern Options → Overlap.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Overlap {
    /// Right in front (default: left in front).
    #[serde(default)]
    pub right_in_front: bool,
    /// Bottom in front (default: top in front).
    #[serde(default)]
    pub bottom_in_front: bool,
}

/// A pattern swatch definition.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternDef {
    pub name: String,
    /// Tile rectangle in the art's coordinates.
    pub tile: Rect,
    #[serde(default)]
    pub tile_type: TileType,
    pub art: Vec<Arc<Node>>,
    #[serde(default)]
    pub overlap: Overlap,
    /// Size Tile to Art: the tile is the art bounds grown by the spacing.
    #[serde(default)]
    pub size_tile_to_art: bool,
    #[serde(default)]
    pub h_spacing: f64,
    #[serde(default)]
    pub v_spacing: f64,
    /// Pattern editing preview: copies per side (3, 5, 7, 9).
    #[serde(default = "five")]
    pub copies: u32,
    /// Dim Copies to (percent opacity of the preview copies).
    #[serde(default = "seventy")]
    pub dim_copies: f32,
    #[serde(default = "yes")]
    pub show_tile_edge: bool,
    /// Show Swatch Bounds: pattern editing mode outlines the part of the tiling the swatch repeats
    /// (the tile, or the period of a brick or hex tiling), dashed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub show_swatch_bounds: bool,
}

impl PatternDef {
    /// A pattern of `art` whose tile is the art's bounds.
    pub fn new(name: &str, art: Vec<Arc<Node>>) -> Self {
        let tile = crate::live::nodes_bounds(&art).unwrap_or(Rect::new(0.0, 0.0, 10.0, 10.0));
        Self {
            name: name.to_string(),
            tile,
            tile_type: TileType::Grid,
            art,
            overlap: Overlap::default(),
            size_tile_to_art: false,
            h_spacing: 0.0,
            v_spacing: 0.0,
            copies: 5,
            dim_copies: 70.0,
            show_tile_edge: true,
            show_swatch_bounds: false,
        }
    }
    pub fn width(&self) -> f64 {
        self.tile.width().max(1e-3)
    }
    pub fn height(&self) -> f64 {
        self.tile.height().max(1e-3)
    }
    /// Visual bounds of the art (stroke included).
    pub fn art_bounds(&self) -> Option<Rect> {
        self.art.iter().fold(None, |acc, n| vectorcraft_geom::union_opt(acc, n.visual_bounds()))
    }
    /// Re-derive the tile from the art (Size Tile to Art).
    pub fn fit_tile_to_art(&mut self) {
        if let Some(b) = crate::live::nodes_bounds(&self.art) {
            self.tile = Rect::new(b.x0 - self.h_spacing / 2.0, b.y0 - self.v_spacing / 2.0, b.x1 + self.h_spacing / 2.0, b.y1 + self.v_spacing / 2.0);
        }
    }
    /// Offset of instance (column `i`, row `j`) in pattern space.
    pub fn offset(&self, i: i64, j: i64) -> Vec2 {
        tile_offset(self.tile_type, self.width(), self.height(), i, j)
    }
    /// The rectangular period of the tiling (the super-tile).
    pub fn period(&self) -> (f64, f64) {
        tile_period(self.tile_type, self.width(), self.height())
    }
    /// The swatch bounds (Show Swatch Bounds): the period of the tiling from the tile's origin.
    pub fn swatch_bounds(&self) -> Rect {
        let (w, h) = self.period();
        Rect::from_origin_size(self.tile.origin(), (w, h))
    }
    /// Maps the art's coordinates into pattern space for instance offset `o`.
    pub fn instance_xf(&self, o: Vec2) -> Affine {
        Affine::translate(o - self.tile.origin().to_vec2())
    }
    /// Offsets (pattern space) of every instance whose art touches `region` (pattern space), in
    /// paint order: the instances "in front" per [`Overlap`] come last.
    pub fn offsets_covering(&self, region: Rect) -> Vec<Vec2> {
        let (w, h) = (self.width(), self.height());
        // Art extent relative to the tile's origin (at least the tile itself).
        let ab = self.art_bounds().unwrap_or(self.tile).union(self.tile) - self.tile.origin().to_vec2();
        let (sx, sy) = self.tile_type.steps(w, h);
        // Shifts of brick/hex rows or columns are < one tile.
        // A region whose tile indices do not fit in integers (or are not finite) gives none, as does
        // one with more than 250,000 positions or none.
        let index = |v: f64| (v.abs() < 9.0e18).then_some(v as i64);
        let (Some(i0), Some(i1), Some(j0), Some(j1)) = (
            index(((region.x0 - ab.x1 - w) / sx).floor()),
            index(((region.x1 - ab.x0 + w) / sx).ceil()),
            index(((region.y0 - ab.y1 - h) / sy).floor()),
            index(((region.y1 - ab.y0 + h) / sy).ceil()),
        ) else {
            return vec![];
        };
        let (i0, i1, j0, j1) = (i0 - 1, i1 + 1, j0 - 1, j1 + 1);
        let (ni, nj) = (i1.saturating_sub(i0), j1.saturating_sub(j0));
        if ni < 1 || nj < 1 || ni.saturating_mul(nj) > 250_000 {
            return vec![];
        }
        let mut out = vec![];
        let rows: Vec<i64> = if self.overlap.bottom_in_front { (j0..=j1).collect() } else { (j0..=j1).rev().collect() };
        for &j in &rows {
            let cols: Vec<i64> = if self.overlap.right_in_front { (i0..=i1).collect() } else { (i0..=i1).rev().collect() };
            for i in cols {
                let o = self.offset(i, j);
                let r = ab + o;
                if r.x0 < region.x1 && r.x1 > region.x0 && r.y0 < region.y1 && r.y1 > region.y0 {
                    out.push(o);
                }
            }
        }
        out
    }
    /// Offsets of the pattern-editing preview copies (`copies`×`copies` around the tile, the
    /// centre `(0, 0)` excluded), in row-major order (not paint order).
    pub fn preview_offsets(&self) -> Vec<Vec2> {
        let k = (self.copies.clamp(1, 15) / 2) as i64;
        let mut out = vec![];
        for j in -k..=k {
            for i in -k..=k {
                if (i, j) != (0, 0) {
                    out.push(self.offset(i, j));
                }
            }
        }
        out
    }
    /// Offsets of every pattern-editing preview copy, including the centre tile, in the same paint
    /// order [`offsets_covering`] uses for [`Overlap`].
    pub fn preview_offsets_in_paint_order(&self) -> Vec<Vec2> {
        let k = (self.copies.clamp(1, 15) / 2) as i64;
        let (i0, i1, j0, j1) = (-k, k, -k, k);
        let mut out = vec![];
        let rows: Vec<i64> = if self.overlap.bottom_in_front { (j0..=j1).collect() } else { (j0..=j1).rev().collect() };
        for &j in &rows {
            let cols: Vec<i64> = if self.overlap.right_in_front { (i0..=i1).collect() } else { (i0..=i1).rev().collect() };
            for i in cols {
                out.push(self.offset(i, j));
            }
        }
        out
    }
    /// Preview copies drawn before and after the centre tile in [`Self::preview_offsets_in_paint_order`],
    /// for one dimmed composite each (the centre tile is drawn at full strength between them).
    pub fn preview_offsets_dimmed_sides(&self) -> (Vec<Vec2>, Vec<Vec2>) {
        let mut before = vec![];
        let mut after = vec![];
        let mut past_center = false;
        for o in self.preview_offsets_in_paint_order() {
            if o.x == 0.0 && o.y == 0.0 {
                past_center = true;
            } else if past_center {
                after.push(o);
            } else {
                before.push(o);
            }
        }
        (before, after)
    }
    /// Instances covering `region` (document space) as plain nodes for a pattern painted with
    /// `paint_xf` (pattern space → document), fresh-id-less (id 0). Used by exporters and Distort
    /// Pattern Fills; Expand uses [`Self::instances_in_with`].
    pub fn instances_in(&self, paint_xf: Affine, region: Rect) -> Vec<Node> {
        self.instances_in_with(paint_xf, region, false)
    }
    /// [`Self::instances_in`], each instance placed with `scaling` (what scales with `paint_xf`
    /// besides geometry).
    pub fn instances_in_with(&self, paint_xf: Affine, region: Rect, scaling: impl Into<crate::node::Scaling>) -> Vec<Node> {
        let scaling = scaling.into();
        let inv = paint_xf.inverse();
        let pr = inv.transform_rect_bbox(region);
        let mut out = vec![];
        for o in self.offsets_covering(pr) {
            let m = paint_xf * self.instance_xf(o);
            for a in &self.art {
                let mut n = (**a).clone();
                place_art(&mut n, m, scaling);
                zero_ids(&mut n);
                out.push(n);
            }
        }
        out
    }
}

/// Transform pattern art `n` by `m` with `scaling`, as a pattern places its art: the opacity masks
/// in it move with it, unlinked ones included (the canvas draws a tile's art and masks together).
pub fn place_art(n: &mut Node, m: Affine, scaling: impl Into<crate::node::Scaling>) {
    let scaling = scaling.into();
    n.transform(m, scaling);
    place_unlinked_masks(n, m, scaling);
}

/// Transform the art of the unlinked opacity masks of `n` and its descendants by `m`, the masks
/// inside mask art included.
fn place_unlinked_masks(n: &mut Node, m: Affine, scaling: crate::node::Scaling) {
    if let Some(mask) = n.extras_if_any().and_then(|e| e.mask.as_mut()) {
        let linked = mask.linked;
        let art = Arc::make_mut(&mut mask.art);
        // A linked mask's art moved with `n`; an unlinked one's moves here.
        if !linked {
            art.transform(m, scaling);
        }
        place_unlinked_masks(art, m, scaling);
    }
    for c in n.children_mut().into_iter().flatten() {
        place_unlinked_masks(Arc::make_mut(c), m, scaling);
    }
}

fn zero_ids(n: &mut Node) {
    n.id = NodeId(0);
    if let Some(ch) = n.children_mut() {
        for c in ch.iter_mut() {
            zero_ids(Arc::make_mut(c));
        }
    }
}

/// Pattern editing mode state (lives in the document so undo and the renderer see it).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PatternEdit {
    /// Pattern being edited.
    pub pattern: String,
    /// The temporary layer holding the tile art while editing.
    pub layer: NodeId,
    /// The definition before editing (restored by Cancel); `None` when Make created it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original: Option<PatternDef>,
}

/// Name of the temporary pattern-editing layer.
pub const PATTERN_EDIT_LAYER: &str = "Pattern Editing Mode";

/// A pattern paint (identity placement).
pub fn pattern_paint(name: &str) -> Paint {
    Paint::Pattern { pattern: name.to_string(), xf: Affine::IDENTITY }
}

/// Does `n` (not its children) paint any fill/stroke with pattern `name` (any pattern for `None`)?
pub fn uses_pattern(n: &Node, name: Option<&str>) -> bool {
    n.appearance.items.iter().any(|it| {
        let p = match it {
            AppearanceItem::Fill(f) => &f.paint,
            AppearanceItem::Stroke(s) => &s.paint,
        };
        matches!(p, Paint::Pattern { pattern, .. } if name.is_none_or(|nm| nm == pattern))
    })
}

/// Transform Patterns without Transform Objects: `m` moves the pattern tiles of `n` and of
/// everything inside it, and the art stays where it is.
pub fn transform_patterns_only(n: &mut Node, m: Affine) {
    transform_pattern_paints(n, m);
    for c in n.children_mut().into_iter().flatten() {
        transform_patterns_only(Arc::make_mut(c), m);
    }
}

/// Apply `m` to the pattern placement of every pattern paint of `n` (Transform Patterns).
pub fn transform_pattern_paints(n: &mut Node, m: Affine) {
    // Without pattern paints there is nothing to change: don't copy the shared appearance.
    let patterned = |it: &AppearanceItem| {
        matches!(it, AppearanceItem::Fill(f) if matches!(f.paint, Paint::Pattern { .. }))
            || matches!(it, AppearanceItem::Stroke(s) if matches!(s.paint, Paint::Pattern { .. }))
    };
    if !n.appearance.items.iter().any(patterned) {
        return;
    }
    for it in n.appearance.items.iter_mut() {
        let p = match it {
            AppearanceItem::Fill(f) => &mut f.paint,
            AppearanceItem::Stroke(s) => &mut s.paint,
        };
        if let Paint::Pattern { xf, .. } = p {
            *xf = m * *xf;
        }
    }
}

// =====================================================================================
// Repeat
// =====================================================================================

/// The arrangement of a Repeat object.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RepeatKind {
    /// Copies rotated around `center`; the source's centre is kept `radius` from it.
    #[serde(rename_all = "camelCase")]
    Radial {
        instances: u32,
        radius: f64,
        center: Point,
        #[serde(default)]
        reverse_overlap: bool,
        #[serde(default)]
        start_angle: f64,
        #[serde(default = "full_turn")]
        end_angle: f64,
    },
    /// Copies on a grid (tile = source bounds + spacing), optionally brick/hex offset and with
    /// alternate rows/columns flipped.
    #[serde(rename_all = "camelCase")]
    Grid {
        h_spacing: f64,
        v_spacing: f64,
        #[serde(default = "three")]
        rows: u32,
        #[serde(default = "three")]
        cols: u32,
        #[serde(default)]
        grid_type: TileType,
        /// Flip alternate rows vertically.
        #[serde(default)]
        flip_rows: bool,
        /// Flip alternate columns horizontally.
        #[serde(default)]
        flip_cols: bool,
    },
    /// The source plus its reflection across the axis through `center` at `angle` degrees
    /// (90 = vertical axis).
    Mirror { angle: f64, center: Point },
}

impl RepeatKind {
    pub fn label(&self) -> &'static str {
        match self {
            RepeatKind::Radial { .. } => "Radial Repeat",
            RepeatKind::Grid { .. } => "Grid Repeat",
            RepeatKind::Mirror { .. } => "Mirror Repeat",
        }
    }
    /// How many instances [`repeat_transforms`] makes (with its clamps).
    fn count(&self) -> usize {
        match self {
            RepeatKind::Radial { instances, .. } => (*instances).clamp(1, 1000) as usize,
            RepeatKind::Grid { rows, cols, .. } => (*rows).clamp(1, 500) as usize * (*cols).clamp(1, 500) as usize,
            RepeatKind::Mirror { .. } => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepeatSpec {
    pub kind: RepeatKind,
    pub source: Vec<Arc<Node>>,
}

/// Most instances a Repeat makes in all, those of the Repeats nested in its source included: as
/// many as one 500 × 500 grid, the budget [`PatternDef::offsets_covering`] also keeps.
const MAX_REPEAT_INSTANCES: usize = 250_000;

/// Counts the instances Repeats make in all, following symbol instances into their symbol's art
/// in `doc`: each symbol's art is counted once, and a missing symbol, or an instance of a symbol
/// inside its own art, counts as none.
///
/// Limitation: symbols that show each other (A's art holds an instance of B and B's holds one of
/// A) are counted along the first path that reaches them. While A is counted, the instance of A
/// in B's art counts as none, and B keeps that count; had B been counted first, A's count would
/// include B's Repeats and B's would not. So within one count, what such symbols add can depend on
/// which of them came first. A symbol whose art reaches no such loop always counts the same.
struct NestedCount<'a> {
    doc: Option<&'a Document>,
    /// The counts made so far, by symbol name (`None` while that symbol's art is being counted).
    symbols: RefCell<HashMap<String, Option<usize>>>,
}

impl<'a> NestedCount<'a> {
    fn new(doc: Option<&'a Document>) -> Self {
        Self { doc, symbols: RefCell::default() }
    }

    /// Instances the Repeats in the art of symbol `name` make in all.
    fn symbol(&self, name: &str) -> usize {
        if let Some(known) = self.symbols.borrow().get(name) {
            return known.unwrap_or(0);
        }
        let Some(sym) = self.doc.and_then(|d| d.symbols.iter().find(|s| s.name == name)) else { return 0 };
        self.symbols.borrow_mut().insert(name.to_owned(), None);
        let n = self.nodes(std::slice::from_ref(&sym.art));
        self.symbols.borrow_mut().insert(name.to_owned(), Some(n));
        n
    }

    /// Instances the Repeats in `nodes` make in all (0 when there are none).
    fn nodes(&self, nodes: &[Arc<Node>]) -> usize {
        let mut sum = 0usize;
        for n in nodes {
            sum = sum.saturating_add(match (&n.kind, n.children()) {
                (NodeKind::Repeat(r), _) => r.instances_in_all(self),
                (NodeKind::SymbolInstance { symbol, .. }, _) => self.symbol(symbol),
                (_, Some(c)) => self.nodes(c),
                _ => 0,
            });
        }
        sum
    }
}

/// Reflection across the line through `c` with direction angle `deg`.
pub fn reflect_about(c: Point, deg: f64) -> Affine {
    let t = (2.0 * deg).to_radians();
    let (s2, c2) = t.sin_cos();
    Affine::translate(c.to_vec2()) * Affine::new([c2, s2, s2, -c2, 0.0, 0.0]) * Affine::translate(-c.to_vec2())
}

/// Instance transforms of a repeat (source space → document), in paint order.
pub fn repeat_transforms(kind: &RepeatKind, source_bounds: Option<Rect>) -> Vec<Affine> {
    let b = source_bounds.unwrap_or(Rect::new(0.0, 0.0, 1.0, 1.0));
    match kind {
        RepeatKind::Radial { instances, radius, center, reverse_overlap, start_angle, end_angle } => {
            let n = (*instances).clamp(1, 1000);
            let c0 = b.center();
            let mut d = c0 - *center;
            if d.hypot() < 1e-9 {
                d = Vec2::new(0.0, -1.0);
            }
            let pre = Affine::translate(*center + d.normalize() * radius.max(0.0) - c0);
            let sweep = end_angle - start_angle;
            let full = (sweep.abs() - 360.0).abs() < 1e-9;
            let step = if full || n == 1 { sweep / n as f64 } else { sweep / (n - 1) as f64 };
            let mut v: Vec<Affine> = (0..n)
                .map(|k| {
                    let a = (start_angle + step * k as f64).to_radians();
                    Affine::translate(center.to_vec2()) * Affine::rotate(a) * Affine::translate(-center.to_vec2()) * pre
                })
                .collect();
            if *reverse_overlap {
                v.reverse();
            }
            v
        }
        RepeatKind::Grid { h_spacing, v_spacing, rows, cols, grid_type, flip_rows, flip_cols } => {
            let (w, h) = ((b.width() + h_spacing).max(1e-3), (b.height() + v_spacing).max(1e-3));
            let mut v = vec![];
            for j in 0..(*rows).clamp(1, 500) as i64 {
                for i in 0..(*cols).clamp(1, 500) as i64 {
                    let o = tile_offset(*grid_type, w, h, i, j);
                    let c = b.center();
                    let mut flip = Affine::IDENTITY;
                    if *flip_cols && i % 2 == 1 {
                        flip = reflect_about(c, 90.0) * flip;
                    }
                    if *flip_rows && j % 2 == 1 {
                        flip = reflect_about(c, 0.0) * flip;
                    }
                    v.push(Affine::translate(o) * flip);
                }
            }
            v
        }
        RepeatKind::Mirror { angle, center } => vec![Affine::IDENTITY, reflect_about(*center, *angle)],
    }
}

impl RepeatSpec {
    pub fn source_bounds(&self) -> Option<Rect> {
        crate::live::nodes_bounds(&self.source)
    }
    /// Where the instances go, counting toward the budget the Repeats in the art of the symbols
    /// of `doc` that instances in the source show (none without a document).
    pub fn transforms(&self, doc: Option<&Document>) -> Vec<Affine> {
        let mut v = repeat_transforms(&self.kind, self.source_bounds());
        // Each instance draws the Repeats in the source again, so nested ones multiply (a 500 × 500
        // grid of one is 62.5 billion instances): keep the first that fit the budget.
        v.truncate(self.kept(NestedCount::new(doc).nodes(&self.source)));
        v
    }
    /// How many of its instances this Repeat keeps when each makes `nested` instances of the
    /// Repeats in its source: as many as fit in [`MAX_REPEAT_INSTANCES`], at least one.
    fn kept(&self, nested: usize) -> usize {
        self.kind.count().min(MAX_REPEAT_INSTANCES / nested.max(1)).max(1)
    }
    /// How many instances this Repeat draws ([`Self::transforms`] for `doc`): as many as its
    /// options make, fewer when the Repeats in its source would take it past
    /// [`MAX_REPEAT_INSTANCES`].
    pub fn instances(&self, doc: Option<&Document>) -> usize {
        self.kept(NestedCount::new(doc).nodes(&self.source))
    }
    /// Instances made in all, those of the Repeats nested in the source included.
    fn instances_in_all(&self, count: &NestedCount) -> usize {
        let nested = count.nodes(&self.source);
        self.kept(nested).saturating_mul(nested.max(1))
    }
    /// Evaluated instances: one group of the (transformed) source per instance (ids 0), as many
    /// as [`Self::transforms`] keeps for `doc`.
    pub fn expand(&self, doc: Option<&Document>) -> Vec<Node> {
        self.transforms(doc)
            .into_iter()
            .map(|m| {
                let children = self
                    .source
                    .iter()
                    .map(|s| {
                        let mut n = (**s).clone();
                        n.transform(m, false);
                        zero_ids(&mut n);
                        Arc::new(n)
                    })
                    .collect();
                Node::group(NodeId(0), children)
            })
            .collect()
    }
    /// Geometric bounds of all instances ([`Self::transforms`] for `doc`).
    pub fn bounds(&self, doc: Option<&Document>) -> Option<Rect> {
        self.copies_bounds(self.source_bounds()?, doc)
    }
    /// The bounds of all instances of a source measuring `source`, each placed by its transform
    /// ([`Self::transforms`] for `doc`).
    pub(crate) fn copies_bounds(&self, source: Rect, doc: Option<&Document>) -> Option<Rect> {
        self.transforms(doc).into_iter().fold(None, |acc, m| vectorcraft_geom::union_opt(acc, Some(m.transform_rect_bbox(source))))
    }
    /// Apply a document transform (source + arrangement parameters).
    pub fn transform(&mut self, a: Affine, scaling: impl Into<crate::node::Scaling>) {
        let scaling = scaling.into();
        for c in self.source.iter_mut() {
            Arc::make_mut(c).transform(a, scaling);
        }
        let s = a.determinant().abs().sqrt();
        match &mut self.kind {
            RepeatKind::Radial { radius, center, .. } => {
                *center = a * *center;
                *radius *= s;
            }
            RepeatKind::Grid { h_spacing, v_spacing, .. } => {
                *h_spacing *= s;
                *v_spacing *= s;
            }
            RepeatKind::Mirror { angle, center } => {
                *center = a * *center;
                let d = a * Point::new(angle.to_radians().cos(), angle.to_radians().sin()) - a * Point::ZERO;
                *angle = d.y.atan2(d.x).to_degrees();
            }
        }
    }
    /// Default radial repeat around the source (centre below it, radius = its height × 1.2).
    pub fn radial(source: Vec<Arc<Node>>, instances: u32, radius: Option<f64>) -> Self {
        let b = crate::live::nodes_bounds(&source).unwrap_or(Rect::new(0.0, 0.0, 10.0, 10.0));
        let r = radius.unwrap_or((b.height().max(b.width()) * 1.2).max(10.0));
        let center = b.center() + Vec2::new(0.0, r);
        Self { kind: RepeatKind::Radial { instances, radius: r, center, reverse_overlap: false, start_angle: 0.0, end_angle: 360.0 }, source }
    }
    pub fn grid(source: Vec<Arc<Node>>, h_spacing: f64, v_spacing: f64) -> Self {
        Self {
            kind: RepeatKind::Grid { h_spacing, v_spacing, rows: 3, cols: 3, grid_type: TileType::Grid, flip_rows: false, flip_cols: false },
            source,
        }
    }
    /// Mirror across an axis at `angle` through a point `offset` beyond the source's right edge
    /// (vertical axis) or below it (other angles pass through the right-edge midpoint).
    pub fn mirror(source: Vec<Arc<Node>>, angle: f64, offset: f64) -> Self {
        let b = crate::live::nodes_bounds(&source).unwrap_or(Rect::new(0.0, 0.0, 10.0, 10.0));
        let center = Point::new(b.x1 + offset, b.center().y);
        Self { kind: RepeatKind::Mirror { angle, center }, source }
    }
}

/// Is this a Repeat object?
pub fn is_repeat(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Repeat(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Appearance;
    use vectorcraft_geom::shapes;

    fn sq(x: f64, y: f64, s: f64) -> Arc<Node> {
        Arc::new(Node::path(NodeId(1), shapes::rectangle(Rect::new(x, y, x + s, y + s)), Appearance::default()))
    }

    fn close(a: Vec2, b: Vec2) -> bool {
        (a - b).hypot() < 1e-9
    }

    #[test]
    fn grid_offsets() {
        assert!(close(tile_offset(TileType::Grid, 10.0, 20.0, 2, 3), Vec2::new(20.0, 60.0)));
        assert!(close(tile_offset(TileType::Grid, 10.0, 20.0, -1, -1), Vec2::new(-10.0, -20.0)));
        assert_eq!(tile_period(TileType::Grid, 10.0, 20.0), (10.0, 20.0));
    }

    #[test]
    fn brick_by_row_offsets() {
        let t = TileType::BrickByRow { offset: 0.5 };
        assert!(close(tile_offset(t, 10.0, 20.0, 0, 1), Vec2::new(5.0, 20.0)));
        assert!(close(tile_offset(t, 10.0, 20.0, 0, 2), Vec2::new(0.0, 40.0)));
        assert!(close(tile_offset(t, 10.0, 20.0, 1, -1), Vec2::new(15.0, -20.0)));
        assert_eq!(tile_period(t, 10.0, 20.0), (10.0, 40.0));
        let t3 = TileType::BrickByRow { offset: 1.0 / 3.0 };
        assert_eq!(tile_period(t3, 10.0, 20.0), (10.0, 60.0));
    }

    #[test]
    fn brick_by_column_offsets() {
        let t = TileType::BrickByColumn { offset: 0.25 };
        assert!(close(tile_offset(t, 10.0, 20.0, 1, 0), Vec2::new(10.0, 5.0)));
        assert!(close(tile_offset(t, 10.0, 20.0, 3, 1), Vec2::new(30.0, 35.0)));
        assert_eq!(tile_period(t, 10.0, 20.0), (40.0, 20.0));
    }

    #[test]
    fn hex_offsets() {
        let c = TileType::HexByColumn;
        assert!(close(tile_offset(c, 10.0, 20.0, 1, 0), Vec2::new(7.5, 10.0)));
        assert!(close(tile_offset(c, 10.0, 20.0, 2, 1), Vec2::new(15.0, 20.0)));
        assert!(close(tile_offset(c, 10.0, 20.0, -1, 0), Vec2::new(-7.5, 10.0)));
        assert_eq!(tile_period(c, 10.0, 20.0), (15.0, 20.0));
        let r = TileType::HexByRow;
        assert!(close(tile_offset(r, 10.0, 20.0, 0, 1), Vec2::new(5.0, 15.0)));
        assert!(close(tile_offset(r, 10.0, 20.0, 1, 2), Vec2::new(10.0, 30.0)));
        assert_eq!(tile_period(r, 10.0, 20.0), (10.0, 30.0));
    }

    #[test]
    fn tilings_are_periodic_on_their_period() {
        // Every instance shifted by the period is again an instance.
        for tt in [
            TileType::Grid,
            TileType::BrickByRow { offset: 0.5 },
            TileType::BrickByColumn { offset: 0.25 },
            TileType::HexByColumn,
            TileType::HexByRow,
        ] {
            let (w, h) = (10.0, 20.0);
            let (pw, ph) = tile_period(tt, w, h);
            let all: Vec<Vec2> = (-8..8).flat_map(|j| (-8..8).map(move |i| tile_offset(tt, w, h, i, j))).collect();
            for o in all.iter().filter(|o| o.x.abs() < 30.0 && o.y.abs() < 30.0) {
                for d in [Vec2::new(pw, 0.0), Vec2::new(0.0, ph)] {
                    assert!(all.iter().any(|p| close(*p, *o + d)), "{tt:?} {o:?}+{d:?}");
                }
            }
        }
    }

    #[test]
    fn tile_type_parse() {
        assert_eq!(TileType::parse("brickByRow", Some(0.25)), Some(TileType::BrickByRow { offset: 0.25 }));
        assert_eq!(TileType::parse("hex-by-row", None), Some(TileType::HexByRow));
        assert_eq!(TileType::parse("nope", None), None);
        for id in TileType::IDS {
            assert_eq!(TileType::parse(id, None).unwrap().id(), id);
        }
    }

    #[test]
    fn preview_paint_order_respects_overlap() {
        let mut def = PatternDef::new("p", vec![sq(0.0, 0.0, 10.0)]);
        def.copies = 3;
        assert!(close(*def.preview_offsets_in_paint_order().last().unwrap(), Vec2::new(-10.0, -10.0)));
        let mut d2 = def.clone();
        d2.overlap = Overlap { right_in_front: true, bottom_in_front: true };
        assert!(close(*d2.preview_offsets_in_paint_order().last().unwrap(), Vec2::new(10.0, 10.0)));
        assert_eq!(def.preview_offsets_in_paint_order().len(), 9);
        let (before, after) = def.preview_offsets_dimmed_sides();
        assert_eq!(before.len() + after.len(), 8);
        assert!(before.iter().all(|o| *o != Vec2::ZERO) && after.iter().all(|o| *o != Vec2::ZERO));
    }

    #[test]
    fn covering_offsets_and_overlap_order() {
        let def = PatternDef::new("p", vec![sq(0.0, 0.0, 10.0)]);
        let offs = def.offsets_covering(Rect::new(0.0, 0.0, 30.0, 20.0));
        // 3×2 tiles fully inside, plus neighbours touching the edges are excluded (strict overlap).
        assert_eq!(offs.len(), 6, "{offs:?}");
        // Left/top in front: the top-left tile is drawn last.
        assert!(close(*offs.last().unwrap(), Vec2::ZERO));
        let mut d2 = def.clone();
        d2.overlap = Overlap { right_in_front: true, bottom_in_front: true };
        assert!(close(*d2.offsets_covering(Rect::new(0.0, 0.0, 30.0, 20.0)).last().unwrap(), Vec2::new(20.0, 10.0)));
    }

    #[test]
    fn a_region_too_large_to_count_has_no_offsets() {
        // A pattern scaled by 1e-20 covers a 100 pt square with a region of 1e22 pt.
        let def = PatternDef::new("p", vec![sq(0.0, 0.0, 10.0)]);
        assert!(def.offsets_covering(Rect::new(-1e22, -1e22, 1e22, 1e22)).is_empty());
        assert!(def.offsets_covering(Rect::new(f64::NEG_INFINITY, 0.0, f64::INFINITY, 10.0)).is_empty());
        // Far out, a region one tile wide and 1e12 pt tall still counts its rows.
        assert!(def.offsets_covering(Rect::new(1e18, 0.0, 1e18, 1e12)).is_empty());
        let tiny = Affine::scale(1e-20);
        assert!(def.instances_in(tiny, Rect::new(0.0, 0.0, 100.0, 100.0)).is_empty());
    }

    #[test]
    fn instances_follow_paint_xf() {
        let def = PatternDef::new("p", vec![sq(100.0, 100.0, 10.0)]);
        let n = def.instances_in(Affine::translate((3.0, 0.0)), Rect::new(0.0, 0.0, 20.0, 10.0));
        let xs: Vec<f64> = n.iter().filter_map(|n| n.geometric_bounds()).map(|b| b.x0).collect();
        assert!(xs.contains(&3.0) && xs.contains(&-7.0) && xs.contains(&13.0), "{xs:?}");
        assert!(n.iter().all(|n| n.id == NodeId(0)));
    }

    #[test]
    fn instances_carry_unlinked_masks_with_them() {
        let mut art = (*sq(100.0, 100.0, 10.0)).clone();
        let mut mask = crate::node::OpacityMask::new((*sq(100.0, 100.0, 5.0)).clone(), true);
        mask.linked = false;
        art.extras_mut().mask = Some(Box::new(mask));
        let def = PatternDef::new("p", vec![Arc::new(art)]);
        for n in def.instances_in(Affine::translate((3.0, 0.0)), Rect::new(0.0, 0.0, 20.0, 10.0)) {
            let (b, m) = (n.geometric_bounds().unwrap(), n.extras().mask.as_ref().unwrap());
            assert_eq!(m.art.geometric_bounds().unwrap().origin(), b.origin(), "the mask moves with its art");
            assert!(!m.linked, "the mask stays unlinked");
        }
        // An unlinked mask inside a linked mask's art moves too.
        let mut inner = (*sq(100.0, 100.0, 5.0)).clone();
        let mut nested = crate::node::OpacityMask::new((*sq(100.0, 100.0, 2.0)).clone(), true);
        nested.linked = false;
        inner.extras_mut().mask = Some(Box::new(nested));
        let mut art = (*sq(100.0, 100.0, 10.0)).clone();
        art.extras_mut().mask = Some(Box::new(crate::node::OpacityMask::new(inner, true)));
        let def = PatternDef::new("p", vec![Arc::new(art)]);
        for n in def.instances_in(Affine::translate((3.0, 0.0)), Rect::new(0.0, 0.0, 20.0, 10.0)) {
            let outer = n.extras().mask.as_ref().unwrap();
            let nested = outer.art.extras().mask.as_ref().unwrap();
            assert_eq!(nested.art.geometric_bounds().unwrap().origin(), n.geometric_bounds().unwrap().origin());
        }
    }

    #[test]
    fn preview_offsets_count() {
        let mut def = PatternDef::new("p", vec![sq(0.0, 0.0, 10.0)]);
        assert_eq!(def.preview_offsets().len(), 24);
        def.copies = 3;
        assert_eq!(def.preview_offsets().len(), 8);
    }

    #[test]
    fn radial_transforms() {
        let r = RepeatSpec::radial(vec![sq(-5.0, -105.0, 10.0)], 4, Some(100.0));
        let RepeatKind::Radial { center, .. } = r.kind else { panic!() };
        assert!((center - Point::new(0.0, -0.0)).hypot() < 1e-9, "{center:?}");
        let cs: Vec<Point> = r.transforms(None).iter().map(|m| *m * Point::new(0.0, -100.0)).collect();
        assert!((cs[0] - Point::new(0.0, -100.0)).hypot() < 1e-9);
        assert!((cs[1] - Point::new(100.0, 0.0)).hypot() < 1e-9, "{cs:?}");
        assert!((cs[2] - Point::new(0.0, 100.0)).hypot() < 1e-9);
        assert_eq!(r.expand(None).len(), 4);
    }

    #[test]
    fn radial_radius_is_authoritative() {
        let mut r = RepeatSpec::radial(vec![sq(-5.0, -105.0, 10.0)], 6, Some(100.0));
        if let RepeatKind::Radial { radius, .. } = &mut r.kind {
            *radius = 50.0;
        }
        let m = r.transforms(None)[0];
        assert!((m * Point::new(0.0, -100.0) - Point::new(0.0, -50.0)).hypot() < 1e-9);
    }

    #[test]
    fn grid_repeat_transforms() {
        let g = RepeatSpec::grid(vec![sq(0.0, 0.0, 10.0)], 5.0, 2.0);
        let t = g.transforms(None);
        assert_eq!(t.len(), 9);
        assert!((t[1] * Point::ZERO - Point::new(15.0, 0.0)).hypot() < 1e-9);
        assert!((t[3] * Point::ZERO - Point::new(0.0, 12.0)).hypot() < 1e-9);
        assert_eq!(g.bounds(None), Some(Rect::new(0.0, 0.0, 40.0, 34.0)));
    }

    #[test]
    fn grid_repeat_flip() {
        let mut g = RepeatSpec::grid(vec![sq(0.0, 0.0, 10.0)], 0.0, 0.0);
        if let RepeatKind::Grid { flip_cols, .. } = &mut g.kind {
            *flip_cols = true;
        }
        let t = g.transforms(None);
        // Column 1 is mirrored about the source centre, then moved one tile right.
        assert!((t[1] * Point::new(0.0, 0.0) - Point::new(20.0, 0.0)).hypot() < 1e-9);
    }

    #[test]
    fn mirror_transforms() {
        let m = RepeatSpec::mirror(vec![sq(0.0, 0.0, 10.0)], 90.0, 5.0);
        let t = m.transforms(None);
        assert_eq!(t.len(), 2);
        assert!((t[1] * Point::new(0.0, 3.0) - Point::new(30.0, 3.0)).hypot() < 1e-9);
        let b = m.bounds(None).unwrap();
        assert!((b.x1 - 30.0).abs() < 1e-9 && b.x0.abs() < 1e-9 && (b.y1 - 10.0).abs() < 1e-9, "{b:?}");
        // Horizontal axis.
        let h = reflect_about(Point::new(0.0, 10.0), 0.0);
        assert!((h * Point::new(4.0, 0.0) - Point::new(4.0, 20.0)).hypot() < 1e-9);
    }

    #[test]
    fn repeat_transform_moves_arrangement() {
        let mut r = RepeatSpec::radial(vec![sq(-5.0, -105.0, 10.0)], 4, Some(100.0));
        let before = r.bounds(None).unwrap();
        r.transform(Affine::translate((50.0, 0.0)), false);
        let after = r.bounds(None).unwrap();
        assert!((after.x0 - before.x0 - 50.0).abs() < 1e-9);
        let mut m = RepeatSpec::mirror(vec![sq(0.0, 0.0, 10.0)], 90.0, 5.0);
        m.transform(Affine::rotate(std::f64::consts::FRAC_PI_2), false);
        let RepeatKind::Mirror { angle, .. } = m.kind else { panic!() };
        assert!((angle.rem_euclid(180.0) - 0.0).abs() < 1e-9 || (angle.rem_euclid(180.0) - 180.0).abs() < 1e-9, "{angle}");
    }

    #[test]
    fn nested_repeats_share_one_instance_budget() {
        // Instances made in all, counted from the transforms without drawing any.
        fn made(r: &RepeatSpec) -> usize {
            let each: usize = r.source.iter().map(|s| if let NodeKind::Repeat(n) = &s.kind { made(n) } else { 1 }).sum();
            r.transforms(None).len().saturating_mul(each)
        }
        let grid = |source: Arc<Node>| {
            let mut g = RepeatSpec::grid(vec![source], 0.0, 0.0);
            if let RepeatKind::Grid { rows, cols, .. } = &mut g.kind {
                *rows = 500;
                *cols = 500;
            }
            g
        };
        let repeat = |r: RepeatSpec| Arc::new(Node::new(NodeId(2), NodeKind::Repeat(r)));
        // One grid keeps all its instances.
        let inner = grid(sq(0.0, 0.0, 1.0));
        assert_eq!(made(&inner), 250_000);
        // A 500 × 500 grid of it made 62.5 billion.
        let outer = grid(repeat(inner.clone()));
        assert!(made(&outer) <= 250_000, "{} instances", made(&outer));
        assert_eq!(outer.expand(None).len(), outer.transforms(None).len());
        assert!(made(&RepeatSpec::mirror(vec![repeat(inner)], 90.0, 5.0)) <= 250_000);
        // Small nests are left as they are.
        let small = RepeatSpec::mirror(vec![repeat(RepeatSpec::grid(vec![sq(0.0, 0.0, 10.0)], 1.0, 1.0))], 90.0, 5.0);
        assert_eq!(made(&small), 18);
    }

    #[test]
    fn serde_roundtrip() {
        let mut def = PatternDef::new("Dots", vec![sq(0.0, 0.0, 10.0)]);
        def.tile_type = TileType::HexByRow;
        let s = serde_json::to_string(&def).unwrap();
        assert_eq!(serde_json::from_str::<PatternDef>(&s).unwrap(), def);
        let n = Node::new(NodeId(5), NodeKind::Repeat(RepeatSpec::grid(vec![sq(0.0, 0.0, 10.0)], 1.0, 2.0)));
        let s = serde_json::to_string(&n).unwrap();
        assert_eq!(serde_json::from_str::<Node>(&s).unwrap(), n);
        // Old pattern paints (no xf) still load.
        let p: Paint = serde_json::from_str(r#"{"type":"pattern","pattern":"Dots"}"#).unwrap();
        assert_eq!(p, pattern_paint("Dots"));
    }
}
