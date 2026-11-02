//! Symbol Options › Enable Guides for 9-Slice Scaling: two horizontal and two vertical guides cut
//! a symbol into nine sections, and when an instance is scaled the parts of the symbol outside the
//! guides are not scaled
//! ([helpx: maintain proportions while scaling symbols](https://helpx.adobe.com/illustrator/desktop/manage-objects/traces-mockups-symbols/maintain-proportions-while-scaling-symbols.html)).
//! New guides sit a quarter of the symbol's width and height in from each edge.
//!
//! VectorCraft's own rules: the corners keep their size, the top and bottom edges stretch only in
//! width, the left and right edges only in height, and the center in both. When the instance is
//! smaller than its two outer cells on an axis, both shrink in proportion and the center is empty.
//! The cells are sized along the symbol's own axes, and the instance's rotation, shear, reflection
//! and position then apply to them.
//!
//! The guides are stored as fractions (0..1) of the symbol's natural size, measured from the left
//! and top edges and from the right and bottom edges, so they survive resizing the art. Stored
//! under [`SYMBOL_NINE_SLICE`] keyed by symbol name; symbols without 9-slice have no entry.
//!
//! [`NineSliceMap`] places such an instance's art for the canvas, bounds, Break Link, Expand and
//! the exporters ([`Document::place_symbol_art`]).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_geom::{Affine, Anchor, CubicBez, FillRule, ParamCurve, PathData, Point, Rect, SubPath, shapes};

use crate::{Appearance, Document, MAX_SYMBOL_NODES, Node, NodeId, NodeKind, SymbolBoxes};

/// Document unknown key: `{ symbolName: [left, top, right, bottom] }`.
pub const SYMBOL_NINE_SLICE: &str = "symbolNineSlice";

/// The four 9-slice guides as fractions of the symbol's natural size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SymbolNineSlice {
    /// Left guide, from the left edge.
    pub left: f64,
    /// Top guide, from the top edge.
    pub top: f64,
    /// Right guide, from the right edge.
    pub right: f64,
    /// Bottom guide, from the bottom edge.
    pub bottom: f64,
}

impl Default for SymbolNineSlice {
    /// Each guide sits a quarter of the symbol's width or height in from its edge.
    fn default() -> Self {
        let q = 0.25;
        Self { left: q, top: q, right: q, bottom: q }
    }
}

impl SymbolNineSlice {
    /// Guides from `[left, top, right, bottom]`; `None` unless each is finite, within 0..1, and the
    /// two on each axis leave a gap between them.
    pub fn new(left: f64, top: f64, right: f64, bottom: f64) -> Option<Self> {
        let ok = |v: f64| v.is_finite() && (0.0..=1.0).contains(&v);
        (ok(left) && ok(top) && ok(right) && ok(bottom) && left + right < 1.0 && top + bottom < 1.0).then_some(Self { left, top, right, bottom })
    }

    pub fn to_value(self) -> Value {
        json!([self.left, self.top, self.right, self.bottom])
    }

    pub fn from_value(v: &Value) -> Option<Self> {
        let a = v.as_array()?;
        let n = |i: usize| a.get(i).and_then(Value::as_f64);
        if a.len() != 4 {
            return None;
        }
        Self::new(n(0)?, n(1)?, n(2)?, n(3)?)
    }
}

/// The most objects [`NineSliceMap::place`] makes for each object of the art it places: an
/// object drawn in nine cells becomes a group of nine clip groups, each a clip path and a copy.
pub const MAX_SLICE_GROWTH: u64 = 28;

/// Places a symbol instance's art within [`MAX_SYMBOL_NODES`] objects, of which `used` are already
/// spent. `place` runs only when the budget has room for `most`, the most objects placing can make
/// (the art's object count, or [`NineSliceMap::most_placed`] when it is placed through 9-slice
/// guides), and returns the placed art and its object count, which `used` then grows by. `None`,
/// with `used` as it was, when the instance does not fit.
pub fn charge_symbol_art<T>(used: &mut u64, most: u64, place: impl FnOnce() -> (T, u64)) -> Option<T> {
    if used.saturating_add(most) > MAX_SYMBOL_NODES {
        return None;
    }
    let (art, placed) = place();
    let total = used.saturating_add(placed);
    if total > MAX_SYMBOL_NODES {
        return None;
    }
    *used = total;
    Some(art)
}

/// The effects that paint past the edges of what an object draws without reshaping it: drop
/// shadow, outer glow and Gaussian blur. [`NineSliceMap::place`] applies them once to all the
/// cells of art that crosses the guides.
pub const PAINTED_EFFECTS: [&str; 3] = ["stylize.dropShadow", "stylize.outerGlow", "blur.gaussian"];

/// How far an instance's scale on an axis may be from 1 and still leave its art unchanged.
const SCALE_EPS: f64 = 1e-9;

/// How deep [`NineSliceMap::place`] follows groups and opacity masks. Art deeper than this is
/// placed as one piece ([`NineSliceMap::place`] says how).
const MAX_SLICE_DEPTH: u32 = 64;

/// One axis of a 9-slice map: the three cells between the cuts and where each one goes.
#[derive(Clone, Copy, Debug, PartialEq)]
struct AxisMap {
    /// The cuts in the art at its natural size: its start, the two guides, its end.
    src: [f64; 4],
    /// Where the cuts go. The outer cells keep their length unless the scaled length is shorter
    /// than both together; then both shrink in proportion and the middle cell is empty.
    dst: [f64; 4],
}

impl AxisMap {
    /// The axis from `lo` to `hi` with guides `near` and `far` (fractions of its length in from
    /// each end), scaled by `k` (above 0).
    fn new(lo: f64, hi: f64, near: f64, far: f64, k: f64) -> Self {
        let len = hi - lo;
        let (a, b) = (near * len, far * len);
        let src = [lo, lo + a, hi - b, hi];
        let (t0, t3) = (k * lo, k * hi);
        let room = t3 - t0;
        let (a, b) = if a + b <= room {
            (a, b)
        } else {
            let f = (room / (a + b)).max(0.0);
            (a * f, b * f)
        };
        let t1 = t0 + a;
        let t2 = (t3 - b).max(t1);
        Self { src, dst: [t0, t1, t2, t3] }
    }

    /// The cell `v` lies in; values outside the art belong to the outer cells.
    fn cell(&self, v: f64) -> usize {
        if v < self.src[1] {
            0
        } else if v < self.src[2] {
            1
        } else {
            2
        }
    }

    /// The first and last cells that the span `lo..hi` reaches. A span that ends on a guide does
    /// not reach the cell past it.
    fn cells(&self, lo: f64, hi: f64) -> [usize; 2] {
        let eps = 1e-9 * (self.src[3] - self.src[0]).abs().max(1.0);
        let first = if lo >= self.src[2] - eps {
            2
        } else if lo >= self.src[1] - eps {
            1
        } else {
            0
        };
        let last = if hi <= self.src[1] + eps {
            0
        } else if hi <= self.src[2] + eps {
            1
        } else {
            2
        };
        [first, last.max(first)]
    }

    /// The part of `lo..hi` in cell `i`, the outer cells reaching past the art; `None` when it is
    /// empty.
    fn part(&self, i: usize, lo: f64, hi: f64) -> Option<(f64, f64)> {
        let start = if i == 0 { lo } else { lo.max(self.src[i]) };
        let end = if i == 2 { hi } else { hi.min(self.src[i + 1]) };
        (start < end).then_some((start, end))
    }

    /// Cell `i`'s map, as (scale, offset).
    fn linear(&self, i: usize) -> (f64, f64) {
        let (s0, s1, d0, d1) = (self.src[i], self.src[i + 1], self.dst[i], self.dst[i + 1]);
        let k = if (s1 - s0).abs() > 1e-12 { (d1 - d0) / (s1 - s0) } else { 1.0 };
        (k, d0 - s0 * k)
    }

    fn map(&self, v: f64) -> f64 {
        let (k, o) = self.linear(self.cell(v));
        v * k + o
    }
}

/// How an instance of a symbol with 9-slice guides places its art: a map from symbol space to
/// document space that is affine within each of the nine cells and continuous across the guides.
/// The guides cut the art along the symbol's own axes; the instance's rotation, shear, reflection
/// and position apply after the cells are sized (VectorCraft's own rule).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NineSliceMap {
    /// Symbol space → the art at its natural size (an axis-aligned scale).
    natural: Affine,
    x: AxisMap,
    y: AxisMap,
    /// The instance's transform without its scale along the symbol's axes.
    outer: Affine,
    /// Whether [`Self::place`] draws art that crosses a guide in clip groups, one per cell.
    clips: bool,
}

impl NineSliceMap {
    /// The map for an instance placed by `xf` that shows art with natural-size transform
    /// `natural` and geometric bounds `art` (in symbol space). `None` when the art or the
    /// transform is degenerate, or when the instance is at its natural size on both axes (then
    /// `xf` places the art as it is).
    pub fn new(guides: SymbolNineSlice, art: Rect, natural: Affine, xf: Affine) -> Option<Self> {
        let b = natural.transform_rect_bbox(art);
        if ![b.x0, b.y0, b.x1, b.y1].into_iter().all(f64::is_finite) || b.width() <= 1e-9 || b.height() <= 1e-9 {
            return None;
        }
        if natural.determinant().abs() < 1e-18 {
            return None;
        }
        let m = (xf * natural.inverse()).as_coeffs();
        if !m.into_iter().all(f64::is_finite) {
            return None;
        }
        // The linear part as (rotation or reflection) · (shear along the x axis) · (scale along the
        // symbol's axes): x scales by the length of the first column, y by the extent of the second
        // column across it, so a shear along x alone scales neither axis. A shear along y alone
        // keeps the second column's length and the area: it is the natural size too.
        let kx = m[0].hypot(m[1]);
        if kx <= 1e-12 {
            return None;
        }
        let (u, det) = ((m[0] / kx, m[1] / kx), m[0] * m[3] - m[1] * m[2]);
        if (m[2].hypot(m[3]) - 1.0).abs() < SCALE_EPS && (det.abs() - 1.0).abs() < SCALE_EPS {
            return None;
        }
        let v = if det < 0.0 { (u.1, -u.0) } else { (-u.1, u.0) };
        let ky = m[2] * v.0 + m[3] * v.1;
        if ky <= 1e-12 || !ky.is_finite() || ((kx - 1.0).abs() < SCALE_EPS && (ky - 1.0).abs() < SCALE_EPS) {
            return None;
        }
        let h = (m[2] * u.0 + m[3] * u.1) / ky;
        let outer = Affine::new([u.0, u.1, v.0 + h * u.0, v.1 + h * u.1, m[4], m[5]]);
        let x = AxisMap::new(b.x0, b.x1, guides.left, guides.right, kx);
        let y = AxisMap::new(b.y0, b.y1, guides.top, guides.bottom, ky);
        Some(Self { natural, x, y, outer, clips: true })
    }

    /// Where a point of the symbol's art draws.
    pub fn map_point(&self, p: Point) -> Point {
        let q = self.natural * p;
        self.outer * Point::new(self.x.map(q.x), self.y.map(q.y))
    }

    /// The affine map of the cell in column `i` and row `j`, from the art at its natural size to
    /// document space.
    fn cell_local(&self, i: usize, j: usize) -> Affine {
        let (kx, ox) = self.x.linear(i);
        let (ky, oy) = self.y.linear(j);
        self.outer * Affine::new([kx, 0.0, 0.0, ky, ox, oy])
    }

    /// The affine map of the cell in column `i` and row `j`: symbol space → document space.
    fn cell_xf(&self, i: usize, j: usize) -> Affine {
        self.cell_local(i, j) * self.natural
    }

    /// The affine map of the cell that symbol-space point `p` lies in.
    pub fn cell_at(&self, p: Point) -> Affine {
        let q = self.natural * p;
        self.cell_xf(self.x.cell(q.x), self.y.cell(q.y))
    }

    /// The guides in symbol space: the two x values and the two y values.
    fn cuts(&self) -> ([f64; 2], [f64; 2]) {
        let inv = self.natural.inverse();
        let a = inv * Point::new(self.x.src[1], self.y.src[1]);
        let b = inv * Point::new(self.x.src[2], self.y.src[2]);
        ([a.x, b.x], [a.y, b.y])
    }

    /// `path` (in symbol space) in document space. Each segment is cut where it crosses a guide
    /// (at most 12 cuts a segment) and each piece is placed by its cell's map, so the path's
    /// anchors and curves follow the map exactly. The result is still one path: a stroke along it
    /// is drawn once, without joins or caps at the guides.
    pub fn map_path(&self, path: &PathData) -> PathData {
        let (xs, ys) = self.cuts();
        let mut subs = Vec::with_capacity(path.subpaths.len());
        for sp in &path.subpaths {
            let n = sp.anchors.len();
            let Some(first) = sp.anchors.first() else { continue };
            let a0 = self.cell_at(first.p);
            let mut res = vec![Anchor { p: a0 * first.p, h_in: a0 * first.h_in, h_out: a0 * first.h_out, kind: first.kind }];
            for i in 0..sp.segment_count() {
                let line = sp.segment_is_line(i);
                let pieces = split(sp.segment(i), &xs, &ys);
                let count = pieces.len();
                let mut end_in = None;
                for (j, piece) in pieces.into_iter().enumerate() {
                    let a = self.cell_at(piece.eval(0.5));
                    let (p1, p2, p3) = (a * piece.p1, a * piece.p2, a * piece.p3);
                    if let Some(prev) = res.last_mut() {
                        prev.h_out = if line { prev.p } else { p1 };
                    }
                    let h_in = if line { p3 } else { p2 };
                    if j + 1 < count {
                        res.push(Anchor { p: p3, h_in, h_out: p3, kind: Default::default() });
                    } else {
                        end_in = Some(h_in);
                    }
                }
                let end = (i + 1) % n;
                let b = &sp.anchors[end];
                let ab = self.cell_at(b.p);
                let h_in = end_in.unwrap_or(ab * b.p);
                if end == 0 {
                    res[0].h_in = h_in;
                } else {
                    res.push(Anchor { p: ab * b.p, h_in, h_out: ab * b.h_out, kind: b.kind });
                }
            }
            subs.push(SubPath::new(res, sp.closed));
        }
        PathData::new(subs)
    }

    /// Symbol art `art` (in symbol space) placed in document space, its instances measured with
    /// the symbols of `doc`. VectorCraft's own rules:
    ///
    /// - An object whose geometric box lies in one cell is placed by that cell's map, so an
    ///   object in a corner keeps its size and stays as it is (live shapes, type and images
    ///   included). A stroke that reaches past a guide is drawn with its path, at its weight.
    /// - A path with solid or no paint that crosses a guide is cut at the guides
    ///   ([`Self::map_path`]); a live shape among them becomes a plain path.
    /// - Any other object that crosses a guide (type, images, live objects, paths painted with a
    ///   gradient or pattern) is drawn once in each cell it reaches, placed by that cell's map and
    ///   clipped to the cell, so its paint lines up across the guides.
    /// - Such an object that draws symbol art ([`draws_symbol_art`]: a symbol instance, type with
    ///   an inline graphic, a placed document, a brushed stroke, or a pattern whose tile draws any
    ///   of these, in it or in its opacity masks) is placed by the map of the cell its center is
    ///   in, and is not copied into each cell. So is art nested more than 64 levels deep that
    ///   draws symbol art; other such art is clipped to the cells as above. A path with solid or
    ///   no paint is cut at the guides whatever its strokes' brushes, and stays one path.
    ///
    /// Strokes keep their weight, as they do for any instance.
    pub fn place(&self, art: &Node, doc: &Document) -> Node {
        let boxes = doc.symbol_boxes().quick();
        self.place_at(art, &boxes, 0)
    }

    /// [`Self::place`] without clip groups, for writers that do not apply clipping (DXF, WMF):
    /// every path that crosses a guide is cut at the guides whatever its paint, and any other
    /// object that crosses one is placed by the map of the cell its center is in.
    pub fn place_unclipped(&self, art: &Node, doc: &Document) -> Node {
        Self { clips: false, ..*self }.place(art, doc)
    }

    /// The most objects [`Self::place`] makes of `art`, as [`Node::count_with_masks`] counts them
    /// (the art of opacity masks included), found without placing it: an object whose box lies in
    /// one cell counts its own objects, a path cut at the guides counts 1 and its mask, and an
    /// object drawn in clip groups counts one group plus a clip path and a copy for each cell its
    /// box reaches. That is never more than [`MAX_SLICE_GROWTH`] for each object of `art`.
    pub fn most_placed(&self, art: &Node, doc: &Document) -> u64 {
        let boxes = doc.symbol_boxes().quick();
        self.most_at(art, &boxes, 0)
    }

    /// [`Self::most_placed`] for `n` at nesting depth `depth`, following [`Self::place_at`].
    fn most_at(&self, n: &Node, boxes: &SymbolBoxes, depth: u32) -> u64 {
        let whole = n.count_with_masks() as u64;
        let Some(b) = n.geometric(Some(boxes)).map(|b| self.natural.transform_rect_bbox(b)).filter(|b| finite(*b)) else {
            return whole;
        };
        let (cols, rows) = (self.x.cells(b.x0, b.x1), self.y.cells(b.y0, b.y1));
        if cols[0] == cols[1] && rows[0] == rows[1] {
            return whole;
        }
        if depth >= MAX_SLICE_DEPTH {
            return self.most_clipped(n, boxes, b);
        }
        match &n.kind {
            NodeKind::Group { .. } | NodeKind::Layer { .. } => self.most_container(n, boxes, depth),
            NodeKind::Compound { .. } if !self.clips || plain_paint(n) => self.most_container(n, boxes, depth),
            NodeKind::Path { .. } if !self.clips || plain_paint(n) => self.most_mask(n, boxes, depth).saturating_add(1),
            _ => self.most_clipped(n, boxes, b),
        }
    }

    /// [`Self::most_placed`] for a container placed by [`Self::container`].
    fn most_container(&self, n: &Node, boxes: &SymbolBoxes, depth: u32) -> u64 {
        let own = self.most_mask(n, boxes, depth).saturating_add(1);
        n.children().into_iter().flatten().fold(own, |sum, c| sum.saturating_add(self.most_at(c, boxes, depth + 1)))
    }

    /// [`Self::most_placed`] for the opacity mask of `n` as [`Self::place_mask`] places it: a
    /// linked one through the map, an unlinked one as it is.
    fn most_mask(&self, n: &Node, boxes: &SymbolBoxes, depth: u32) -> u64 {
        match n.extras().mask.as_ref() {
            Some(m) if m.linked => self.most_at(&m.art, boxes, depth + 1),
            Some(m) => m.art.count_with_masks() as u64,
            None => 0,
        }
    }

    /// [`Self::most_placed`] for an object placed by [`Self::clipped`], whose box at the natural
    /// size is `b`.
    fn most_clipped(&self, n: &Node, boxes: &SymbolBoxes, b: Rect) -> u64 {
        let whole = n.count_with_masks() as u64;
        if !self.clips || draws_symbol_art(n, boxes.document()) {
            return whole;
        }
        let (cols, rows) = self.clip_cells(n, boxes, b).0;
        let cells = ((cols[1] - cols[0] + 1) * (rows[1] - rows[0] + 1)) as u64;
        cells.saturating_mul(whole.saturating_add(2)).saturating_add(1)
    }

    /// The columns and rows that [`Self::clipped`] draws `n` in, and the box it clips: `n`'s visual
    /// box at the natural size joined with its geometric box `b`.
    fn clip_cells(&self, n: &Node, boxes: &SymbolBoxes, b: Rect) -> (([usize; 2], [usize; 2]), Rect) {
        let v = n.visual(Some(boxes)).map(|v| self.natural.transform_rect_bbox(v)).filter(|v| finite(*v)).map_or(b, |v| v.union(b));
        ((self.x.cells(v.x0, v.x1), self.y.cells(v.y0, v.y1)), v)
    }

    fn place_at(&self, n: &Node, boxes: &SymbolBoxes, depth: u32) -> Node {
        let Some(b) = n.geometric(Some(boxes)).map(|b| self.natural.transform_rect_bbox(b)).filter(|b| finite(*b)) else {
            return transformed(n, self.cell_at(Point::ZERO));
        };
        let (cols, rows) = (self.x.cells(b.x0, b.x1), self.y.cells(b.y0, b.y1));
        if cols[0] == cols[1] && rows[0] == rows[1] {
            return transformed(n, self.cell_xf(cols[0], rows[0]));
        }
        let center = self.cell_xf(self.x.cell(b.center().x), self.y.cell(b.center().y));
        if depth >= MAX_SLICE_DEPTH {
            return self.clipped(n, boxes, b, center);
        }
        match &n.kind {
            NodeKind::Group { .. } | NodeKind::Layer { .. } => self.container(n, boxes, depth, center),
            NodeKind::Compound { .. } if !self.clips || plain_paint(n) => self.container(n, boxes, depth, center),
            NodeKind::Path { path: src, .. } if !self.clips || plain_paint(n) => {
                let mut out = n.clone();
                out.transform(center, false);
                if let NodeKind::Path { path, live, .. } = &mut out.kind {
                    *path = self.map_path(src);
                    *live = None;
                }
                self.place_mask(n, &mut out, boxes, depth);
                out
            }
            _ => self.clipped(n, boxes, b, center),
        }
    }

    /// Group, layer or compound path `n`: its members placed one by one, the container itself
    /// (its gradients and frames) by `center`.
    fn container(&self, n: &Node, boxes: &SymbolBoxes, depth: u32, center: Affine) -> Node {
        let mut out = n.clone();
        let kids = out.children_mut().map(std::mem::take).unwrap_or_default();
        out.transform(center, false);
        if let Some(children) = out.children_mut() {
            *children = kids.iter().map(|c| Arc::new(self.place_at(c, boxes, depth + 1))).collect();
        }
        self.place_mask(n, &mut out, boxes, depth);
        out
    }

    /// The linked opacity mask of `src` placed through the map into `out`; an unlinked one stays
    /// where it is, as [`Node::transform`] leaves it.
    fn place_mask(&self, src: &Node, out: &mut Node, boxes: &SymbolBoxes, depth: u32) {
        if let Some(m) = src.extras().mask.as_ref().filter(|m| m.linked)
            && let Some(dst) = out.extras_if_any().and_then(|e| e.mask.as_mut())
        {
            dst.art = Arc::new(self.place_at(&m.art, boxes, depth + 1));
        }
    }

    /// `n`, whose box at the natural size is `b`, drawn in each cell it reaches: a clip group of
    /// the cell around a copy placed by the cell's map, all in a group with `n`'s id. `n` placed
    /// by `center` when it draws symbol art ([`draws_symbol_art`]), and in [`Self::place_unclipped`].
    ///
    /// The drop shadows, outer glows and blurs ([`PAINTED_EFFECTS`]) at the end of `n`'s own
    /// effects, after its last other visible effect, go on that group with its opacity, blending
    /// mode and isolation, and the group applies them to the copies together: they reach past the
    /// outer cells instead of being clipped to them. Its other effects stay on each copy, in their
    /// order before the moved ones, so effects that reshape it do not reshape the cells' clip
    /// paths, and effects that follow its outline follow the copy's.
    fn clipped(&self, n: &Node, boxes: &SymbolBoxes, b: Rect, center: Affine) -> Node {
        if !self.clips || draws_symbol_art(n, boxes.document()) {
            return transformed(n, center);
        }
        let mut piece = n.clone();
        let own = &mut piece.appearance.effects;
        let keep = own.iter().rposition(|e| e.visible && !PAINTED_EFFECTS.contains(&e.id.as_str())).map_or(0, |i| i + 1);
        let effects = own.split_off(keep);
        let (opacity, blend, isolate) = (piece.opacity, piece.blend, piece.isolate);
        if !effects.is_empty() {
            (piece.opacity, piece.blend, piece.isolate) = (1.0, vectorcraft_color::BlendMode::Normal, false);
        }
        let n = &piece;
        let ((cols, rows), v) = self.clip_cells(n, boxes, b);
        let mut cells = Vec::with_capacity(9);
        for j in rows[0]..=rows[1] {
            for i in cols[0]..=cols[1] {
                let (Some((x0, x1)), Some((y0, y1))) = (self.x.part(i, v.x0, v.x1), self.y.part(j, v.y0, v.y1)) else { continue };
                let local = self.cell_local(i, j);
                if local.determinant().abs() < 1e-18 {
                    continue;
                }
                let mut area = shapes::rectangle(Rect::new(x0, y0, x1, y1));
                area.transform(local);
                let mut clip = Node::new(NodeId(0), NodeKind::Path { path: area, rule: FillRule::NonZero, live: None, clipping: true, guide: false });
                clip.appearance = Appearance::default().into();
                let copy = transformed(n, local * self.natural);
                cells.push(Arc::new(Node::new(NodeId(0), NodeKind::Group { children: vec![Arc::new(clip), Arc::new(copy)], clip: true })));
            }
        }
        let mut out = Node::group(n.id, cells);
        out.visible = n.visible;
        out.locked = n.locked;
        out.name = n.name.clone();
        out.knockout_shape = n.knockout_shape;
        if !effects.is_empty() {
            out.appearance.effects = effects;
            (out.opacity, out.blend, out.isolate) = (opacity, blend, isolate);
        }
        out
    }
}

/// `n` moved by `a`, its strokes at their own weight.
fn transformed(n: &Node, a: Affine) -> Node {
    let mut out = n.clone();
    out.transform(a, false);
    out
}

fn finite(b: Rect) -> bool {
    [b.x0, b.y0, b.x1, b.y1].into_iter().all(f64::is_finite)
}

/// Whether `n`'s own fills and strokes are solid or empty (no gradient or pattern).
fn plain_paint(n: &Node) -> bool {
    n.appearance.items.iter().all(|i| matches!(i.paint(), Paint::None | Paint::Solid { .. }))
}

/// Whether drawing `n` draws the art of a symbol: `n`, a member of it (the members and sources of
/// live objects included) or the art of an opacity mask in it is a symbol instance, type with an
/// inline graphic, a placed document (whose art may hold instances), an object with a visible
/// brushed stroke (whose brush art may hold instances), or is painted with a pattern of `doc`
/// whose tile draws symbol art (by a visible fill or stroke, or by its characters). Each
/// pattern's tile is looked at once, without recursion.
fn draws_symbol_art(n: &Node, doc: &Document) -> bool {
    let mut seen = std::collections::HashSet::new();
    let mut stack = vec![n];
    while let Some(n) = stack.pop() {
        let draws = match &n.kind {
            NodeKind::SymbolInstance { .. } | NodeKind::PlacedDocument(_) => true,
            NodeKind::Text(t) => t.runs.iter().any(|r| r.inline.is_some()),
            _ => false,
        };
        let brushed = n.appearance.items.iter().any(|i| matches!(i, crate::AppearanceItem::Stroke(s) if s.visible && s.brush.is_some()));
        if draws || brushed {
            return true;
        }
        let runs = match &n.kind {
            NodeKind::Text(t) => t.runs.as_slice(),
            _ => &[],
        };
        let items = n.appearance.items.iter().filter(|i| i.visible()).map(|i| i.paint());
        for p in items.chain(runs.iter().flat_map(|r| [&r.style.fill, &r.style.stroke])) {
            if let Paint::Pattern { pattern, .. } = p
                && seen.insert(pattern.as_str())
                && let Some(def) = doc.pattern(pattern)
            {
                stack.extend(def.art.iter().map(|a| &**a));
            }
        }
        stack.extend(n.children().into_iter().flatten().map(|c| &**c));
        stack.extend(n.extras().mask.as_ref().map(|m| &*m.art));
    }
    false
}

/// The parameters in (0, 1) where `c` crosses one of the lines x = `xs` or y = `ys`, in order.
fn crossings(c: CubicBez, xs: &[f64; 2], ys: &[f64; 2]) -> Vec<f64> {
    // Power-basis coefficients of one coordinate of the curve.
    let coeffs = |p0: f64, p1: f64, p2: f64, p3: f64| [p0, 3.0 * (p1 - p0), 3.0 * (p2 - 2.0 * p1 + p0), p3 - 3.0 * p2 + 3.0 * p1 - p0];
    let cx = coeffs(c.p0.x, c.p1.x, c.p2.x, c.p3.x);
    let cy = coeffs(c.p0.y, c.p1.y, c.p2.y, c.p3.y);
    let mut ts = vec![];
    for (k, cuts) in [(cx, xs), (cy, ys)] {
        for v in cuts {
            for t in vectorcraft_geom::kurbo::common::solve_cubic(k[0] - v, k[1], k[2], k[3]) {
                if t.is_finite() && t > 1e-9 && t < 1.0 - 1e-9 {
                    ts.push(t);
                }
            }
        }
    }
    ts.sort_by(f64::total_cmp);
    ts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    ts
}

/// `c` cut at every crossing of a guide.
fn split(c: CubicBez, xs: &[f64; 2], ys: &[f64; 2]) -> Vec<CubicBez> {
    let ts = crossings(c, xs, ys);
    if ts.is_empty() {
        return vec![c];
    }
    let mut out = Vec::with_capacity(ts.len() + 1);
    let mut t0 = 0.0;
    for t in ts.into_iter().chain([1.0]) {
        out.push(c.subsegment(t0..t));
        t0 = t;
    }
    out
}

impl Document {
    /// The 9-slice map for an instance of symbol `name` placed by `xf` (see [`NineSliceMap::new`]):
    /// `None` when the symbol has no guides or the instance is at its natural size. The guides cut
    /// the art's box with the instances in it measured by their own art.
    pub fn symbol_slice_map(&self, name: &str, xf: Affine) -> Option<NineSliceMap> {
        let guides = self.symbol_nine_slice(name)?;
        let sym = self.symbols.iter().find(|s| s.name == name)?;
        let art = self.symbol_boxes().quick().art_bounds(name).or_else(|| sym.art.geometric_bounds())?;
        NineSliceMap::new(guides, art, self.symbol_natural_xf(name), xf)
    }

    /// Art `art` of symbol `name` (stained as the caller draws it) placed by an instance's
    /// transform `xf`, with strokes at their own weight: through the symbol's 9-slice guides when
    /// it has them ([`NineSliceMap::place`]), otherwise by `xf`. `None` when placing it through
    /// the guides could make more than [`MAX_SYMBOL_NODES`] objects ([`NineSliceMap::most_placed`]).
    pub fn place_symbol_art(&self, name: &str, art: &Node, xf: Affine) -> Option<Node> {
        match self.symbol_slice_map(name, xf) {
            Some(m) if m.most_placed(art, self) > MAX_SYMBOL_NODES => None,
            Some(m) => Some(m.place(art, self)),
            None => Some(transformed(art, xf)),
        }
    }

    /// [`Self::place_symbol_art`] for writers that do not apply clipping
    /// ([`NineSliceMap::place_unclipped`]).
    pub fn place_symbol_art_unclipped(&self, name: &str, art: &Node, xf: Affine) -> Option<Node> {
        match self.symbol_slice_map(name, xf) {
            Some(m) => {
                let m = NineSliceMap { clips: false, ..m };
                (m.most_placed(art, self) <= MAX_SYMBOL_NODES).then(|| m.place(art, self))
            }
            None => Some(transformed(art, xf)),
        }
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_color::{Color, Gradient, GradientPaint, Paint};
    use vectorcraft_geom::{Vec2, shapes};

    use super::*;
    use crate::{Appearance, ImageObject, OpacityMask, PatternDef, PlacedDocument, SYMBOL_SIZES, Symbol};

    fn square(d: &mut Document, r: Rect, stroke: f64) -> Node {
        let s = if stroke > 0.0 { Paint::solid(Color::BLACK) } else { Paint::None };
        Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(Paint::solid(Color::BLACK), s, stroke))
    }

    /// A 100 × 100 symbol "S": a 10 × 10 square in each corner, one in the center and one at the
    /// middle of the top edge, inside a frame with no paint that gives the art its bounds.
    fn markers(guides: Option<SymbolNineSlice>) -> Document {
        let mut d = Document::new(400.0, 400.0);
        let rects = [
            Rect::new(0.0, 0.0, 100.0, 100.0),
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(90.0, 0.0, 100.0, 10.0),
            Rect::new(0.0, 90.0, 10.0, 100.0),
            Rect::new(90.0, 90.0, 100.0, 100.0),
            Rect::new(45.0, 45.0, 55.0, 55.0),
            Rect::new(45.0, 0.0, 55.0, 10.0),
        ];
        let art: Vec<Arc<Node>> = rects.iter().map(|r| Arc::new(square(&mut d, *r, 0.0))).collect();
        let group = Node::group(d.alloc_id(), art);
        d.symbols.push(Symbol { dynamic: false, name: "S".into(), art: Arc::new(group) });
        d.set_symbol_nine_slice("S", guides);
        d
    }

    fn placed(d: &Document, xf: Affine) -> Node {
        d.place_symbol_art("S", &d.symbols[0].art, xf).unwrap()
    }

    /// The geometric boxes of the placed art's members.
    fn boxes(d: &Document, xf: Affine) -> Vec<Rect> {
        placed(d, xf).children().map(|c| c.iter().filter_map(|c| c.geometric_bounds()).collect()).unwrap_or_default()
    }

    fn near(a: Rect, b: Rect) -> bool {
        [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|v| v.abs() < 1e-9)
    }

    fn assert_boxes(got: &[Rect], want: &[Rect]) {
        assert_eq!(got.len(), want.len(), "{got:?}");
        for (g, w) in got.iter().zip(want) {
            assert!(near(*g, *w), "{g:?} vs {w:?} in {got:?}");
        }
    }

    /// The guides sit a quarter in by default. At 200 × 150 the corners stay 10 × 10, the center
    /// marker becomes 30 × 20 and the top marker stretches in width only.
    #[test]
    fn a_scaled_instance_keeps_its_corners_and_stretches_between_the_guides() {
        assert_eq!(SymbolNineSlice::default(), SymbolNineSlice { left: 0.25, top: 0.25, right: 0.25, bottom: 0.25 });
        let d = markers(Some(SymbolNineSlice::default()));
        let got = boxes(&d, Affine::translate((20.0, 30.0)) * Affine::scale_non_uniform(2.0, 1.5));
        let want = [
            Rect::new(0.0, 0.0, 200.0, 150.0),
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(190.0, 0.0, 200.0, 10.0),
            Rect::new(0.0, 140.0, 10.0, 150.0),
            Rect::new(190.0, 140.0, 200.0, 150.0),
            Rect::new(85.0, 65.0, 115.0, 85.0),
            Rect::new(85.0, 0.0, 115.0, 10.0),
        ]
        .map(|r| r + Vec2::new(20.0, 30.0));
        assert_boxes(&got, &want);
        // Asymmetric guides: 10 % from the left, 40 % from the right.
        let d = markers(SymbolNineSlice::new(0.1, 0.25, 0.4, 0.25));
        let got = boxes(&d, Affine::scale_non_uniform(2.0, 1.0));
        // The middle column runs from 10 to 60 and is stretched to 10..160 (3 times).
        assert!(near(got[5], Rect::new(10.0 + 35.0 * 3.0, 45.0, 10.0 + 45.0 * 3.0, 55.0)), "{:?}", got[5]);
        assert!(near(got[2], Rect::new(190.0, 0.0, 200.0, 10.0)), "{:?}", got[2]);
    }

    /// Without guides, or at the natural size, the transform places the art as it is.
    #[test]
    fn without_guides_or_scale_the_art_is_placed_by_the_transform() {
        let xf = Affine::scale_non_uniform(2.0, 1.5);
        let d = markers(None);
        assert!(d.symbol_slice_map("S", xf).is_none());
        assert!(near(boxes(&d, xf)[1], Rect::new(0.0, 0.0, 20.0, 15.0)));
        let d = markers(Some(SymbolNineSlice::default()));
        let moved = Affine::translate((5.0, 6.0)) * Affine::rotate(0.5);
        assert!(d.symbol_slice_map("S", moved).is_none());
        assert!(d.symbol_slice_map("S", Affine::scale(0.0)).is_none());
        assert!(d.symbol_slice_map("S", Affine::new([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0])).is_none());
        assert!(d.symbol_slice_map("S", Affine::new([f64::INFINITY, 0.0, 0.0, 1.0, 0.0, 0.0])).is_none());
        assert!(d.symbol_slice_map("Missing", xf).is_none());
    }

    /// At 30 × 30 the 25 + 25 of the outer cells do not fit: both shrink in proportion (to 15) and
    /// the center is empty, never turned inside out.
    #[test]
    fn a_target_smaller_than_the_outer_cells_shrinks_them() {
        let d = markers(Some(SymbolNineSlice::default()));
        let got = boxes(&d, Affine::scale(0.3));
        assert!(near(got[1], Rect::new(0.0, 0.0, 6.0, 6.0)), "{:?}", got[1]);
        assert!(near(got[4], Rect::new(24.0, 24.0, 30.0, 30.0)), "{:?}", got[4]);
        let c = got[5];
        assert!(c.width().abs() < 1e-9 && c.height().abs() < 1e-9 && near(c, Rect::new(15.0, 15.0, 15.0, 15.0)), "{c:?}");
        for b in boxes(&d, Affine::scale(1e-6)) {
            assert!([b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) && b.width() >= 0.0 && b.height() >= 0.0, "{b:?}");
        }
        // Zero-width guides on one side leave that side's corners out of the outer cells.
        let d = markers(SymbolNineSlice::new(0.0, 0.25, 0.25, 0.0));
        let got = boxes(&d, Affine::scale(2.0));
        assert!(near(got[0], Rect::new(0.0, 0.0, 200.0, 200.0)), "{:?}", got[0]);
        assert!(got.iter().all(|b| finite(*b)), "{got:?}");
    }

    /// A negative scale reflects the sized cells; a rotation turns them, the corners keeping
    /// their size along the symbol's own axes.
    #[test]
    fn reflection_and_rotation_apply_after_the_cells_are_sized() {
        let d = markers(Some(SymbolNineSlice::default()));
        let got = boxes(&d, Affine::scale_non_uniform(-2.0, 1.0));
        // The top-left corner square is now at the right, still 10 wide; the center is 30 wide.
        assert!(near(got[1], Rect::new(-10.0, 0.0, 0.0, 10.0)), "{:?}", got[1]);
        assert!(near(got[5], Rect::new(-115.0, 45.0, -85.0, 55.0)), "{:?}", got[5]);
        // Reflected top to bottom (a negative determinant): the corner square stays 10 × 10.
        let got = boxes(&d, Affine::scale_non_uniform(2.0, -1.5));
        assert!(near(got[1], Rect::new(0.0, -10.0, 10.0, 0.0)), "{:?}", got[1]);
        assert!(near(got[5], Rect::new(85.0, -85.0, 115.0, -65.0)), "{:?}", got[5]);
        let turn = Affine::translate((50.0, 50.0)) * Affine::rotate(std::f64::consts::FRAC_PI_6);
        let m = d.symbol_slice_map("S", turn * Affine::scale_non_uniform(2.0, 1.5)).unwrap();
        let o = m.map_point(Point::ORIGIN);
        assert!((o - Point::new(50.0, 50.0)).hypot() < 1e-9);
        assert!(((m.map_point(Point::new(10.0, 0.0)) - o).hypot() - 10.0).abs() < 1e-9);
        assert!(((m.map_point(Point::new(0.0, 10.0)) - o).hypot() - 10.0).abs() < 1e-9);
        assert!(((m.map_point(Point::new(100.0, 0.0)) - o).hypot() - 200.0).abs() < 1e-9);
        assert!(((m.map_point(Point::new(0.0, 100.0)) - o).hypot() - 150.0).abs() < 1e-9);
    }

    /// A sheared instance sizes the cells along the symbol's axes before the shear: the corner is
    /// the unscaled corner sheared, the art's outer corners land where the instance's transform
    /// puts them, and the map is continuous at the guides. A shear alone scales nothing.
    #[test]
    fn a_sheared_instance_keeps_its_corners_along_the_symbols_axes() {
        let d = markers(Some(SymbolNineSlice::default()));
        let shear = Affine::new([1.0, 0.0, 0.5, 1.0, 10.0, 20.0]);
        assert!(d.symbol_slice_map("S", shear).is_none());
        assert!(d.symbol_slice_map("S", Affine::new([1.0, 1.0, 0.0, 1.0, 0.0, 0.0])).is_none(), "a shear along y");
        // A turn by -60° of the shear along y by √3: (1, 0) goes to (2, 0), (0, 1) to (√0.75, 0.5).
        let turned = Affine::rotate(-std::f64::consts::FRAC_PI_3) * Affine::new([1.0, 3f64.sqrt(), 0.0, 1.0, 0.0, 0.0]);
        assert!((turned * Point::new(1.0, 0.0) - Point::new(2.0, 0.0)).hypot() < 1e-9, "{turned:?}");
        assert!(d.symbol_slice_map("S", turned).is_none());
        let xf = shear * Affine::scale_non_uniform(2.0, 1.5);
        let m = d.symbol_slice_map("S", xf).unwrap();
        for p in [Point::new(10.0, 0.0), Point::new(0.0, 10.0), Point::new(10.0, 10.0)] {
            assert!((m.map_point(p) - shear * p).hypot() < 1e-9, "{p:?}");
        }
        for p in [Point::new(100.0, 0.0), Point::new(0.0, 100.0), Point::new(100.0, 100.0)] {
            assert!((m.map_point(p) - xf * p).hypot() < 1e-9, "{p:?}");
        }
        for g in [25.0, 75.0] {
            let before = m.map_point(Point::new(g - 1e-7, 40.0));
            let after = m.map_point(Point::new(g + 1e-7, 40.0));
            assert!((before - after).hypot() < 1e-5, "{g}");
        }
    }

    /// A path that crosses the guides is cut where it crosses them, each piece placed by its cell;
    /// its stroke keeps its weight and its live shape becomes a plain path.
    #[test]
    fn paths_across_the_guides_are_cut_at_them() {
        let mut d = markers(Some(SymbolNineSlice::default()));
        let diagonal = Node::path(
            d.alloc_id(),
            shapes::line(Point::new(0.0, 0.0), Point::new(100.0, 100.0)),
            Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 4.0),
        );
        let m = d.symbol_slice_map("S", Affine::scale_non_uniform(2.0, 1.5)).unwrap();
        let placed = m.place(&diagonal, &d);
        let pts: Vec<Point> = placed.path_data().unwrap().anchors().map(|(_, _, a)| a.p).collect();
        let want = [Point::new(0.0, 0.0), Point::new(25.0, 25.0), Point::new(175.0, 125.0), Point::new(200.0, 150.0)];
        assert_eq!(pts.len(), want.len(), "{pts:?}");
        for (p, w) in pts.iter().zip(want) {
            assert!((*p - w).hypot() < 1e-9, "{pts:?}");
        }
        assert_eq!(placed.appearance.stroke().map(|s| s.width), Some(4.0));
        // A closed curve crossing a guide keeps its shape on each side of it.
        let circle = Node::path(d.alloc_id(), shapes::ellipse(Rect::new(10.0, 10.0, 50.0, 40.0)), Appearance::default());
        let placed = m.place(&circle, &d);
        let b = placed.geometric_bounds().unwrap();
        // The middle column is stretched 3 times and the middle row 2 times past the guides at 25.
        assert!(near(b, Rect::new(10.0, 10.0, 25.0 + 25.0 * 3.0, 25.0 + 15.0 * 2.0)), "{b:?}");
        // The two segments that cross the guide at x = 25 between their anchors are cut there.
        assert_eq!(placed.path_data().unwrap().anchor_count(), 6, "{placed:?}");
        for (_, _, a) in placed.path_data().unwrap().anchors() {
            assert!([a.p.x, a.p.y, a.h_in.x, a.h_out.y].iter().all(|v| v.is_finite()));
        }
        assert!(placed.path_data().unwrap().is_closed());
        // A path with points that are not finite gives a path, not a crash.
        let bad = Node::path(d.alloc_id(), shapes::line(Point::new(f64::NAN, 0.0), Point::new(100.0, f64::INFINITY)), Appearance::default());
        let _ = m.place(&bad, &d);
    }

    /// A linked opacity mask is placed through the guides with its object; an unlinked one stays
    /// where it is, as any transform leaves it.
    #[test]
    fn linked_masks_follow_the_guides_and_unlinked_ones_stay() {
        let mut d = markers(Some(SymbolNineSlice::default()));
        let m = d.symbol_slice_map("S", Affine::scale_non_uniform(2.0, 1.5)).unwrap();
        let mask_art = square(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), 0.0);
        for linked in [true, false] {
            let mut mask = crate::OpacityMask::new(mask_art.clone(), true);
            mask.linked = linked;
            let bar = square(&mut d, Rect::new(0.0, 40.0, 100.0, 60.0), 0.0).with_mask(Some(Box::new(mask)));
            let p = m.place(&bar, &d);
            let got = p.extras().mask.as_ref().unwrap().art.geometric_bounds().unwrap();
            let want = if linked { Rect::new(0.0, 0.0, 200.0, 150.0) } else { Rect::new(0.0, 0.0, 100.0, 100.0) };
            assert!(near(got, want), "linked {linked}: {got:?}");
        }
    }

    /// An object that lies in one corner keeps its size and kind: a live rectangle stays live and
    /// an image keeps its pixel scale.
    #[test]
    fn objects_in_one_cell_are_placed_by_that_cell() {
        let mut d = markers(Some(SymbolNineSlice::default()));
        let shape = crate::LiveShape::Rectangle {
            w: 6.0,
            h: 6.0,
            radii: [1.0; 4],
            kinds: Default::default(),
            radii_relative: false,
            xf: Affine::translate((92.0, 92.0)),
        };
        let mut live = Node::path(d.alloc_id(), shape.to_path(), Appearance::default());
        if let NodeKind::Path { live: l, .. } = &mut live.kind {
            *l = Some(Box::new(shape));
        }
        let image = ImageObject {
            key: "k".into(),
            width: 4,
            height: 4,
            xf: Affine::translate((2.0, 2.0)),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        let image = Node::new(d.alloc_id(), NodeKind::Image(Box::new(image)));
        let m = d.symbol_slice_map("S", Affine::scale_non_uniform(2.0, 1.5)).unwrap();
        let p = m.place(&live, &d);
        assert!(matches!(p.kind, NodeKind::Path { live: Some(_), .. }), "{p:?}");
        assert!(near(p.geometric_bounds().unwrap(), Rect::new(192.0, 142.0, 198.0, 148.0)));
        let p = m.place(&image, &d);
        let NodeKind::Image(im) = &p.kind else { panic!("{p:?}") };
        assert_eq!(im.xf, Affine::translate((2.0, 2.0)));
    }

    /// A gradient-filled path and an image that cross the guides are drawn in each cell they
    /// reach, clipped to it: the pieces' clip areas tile the stretched box with no gap.
    #[test]
    fn other_art_across_the_guides_is_clipped_to_each_cell() {
        let mut d = markers(Some(SymbolNineSlice::default()));
        let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
        let bar = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 40.0, 100.0, 60.0)), Appearance::basic(gradient, Paint::None, 0.0));
        let m = d.symbol_slice_map("S", Affine::scale_non_uniform(2.0, 1.5)).unwrap();
        let p = m.place(&bar, &d);
        assert_eq!(p.id, bar.id);
        let cells = p.children().unwrap();
        // Three columns by one row: the bar lies between the horizontal guides.
        assert_eq!(cells.len(), 3, "{p:?}");
        let mut width = 0.0;
        for c in cells {
            assert!(matches!(c.kind, NodeKind::Group { clip: true, .. }));
            let clip = c.children().unwrap()[0].geometric_bounds().unwrap();
            width += clip.width();
            let copy = &c.children().unwrap()[1];
            assert!(matches!(copy.appearance.fill().map(|f| &f.paint), Some(Paint::Gradient(_))));
        }
        // The clip areas cover the 200-point stretched bar and no more.
        assert!((width - 200.0).abs() < 1e-9, "{width}");
        assert!(near(p.geometric_bounds().unwrap(), Rect::new(0.0, 55.0, 200.0, 95.0)), "{:?}", p.geometric_bounds());
        // An image across the vertical guides: three pieces, each its own image.
        let image = ImageObject {
            key: "k".into(),
            width: 100,
            height: 10,
            xf: Affine::translate((0.0, 45.0)),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        let image = Node::new(d.alloc_id(), NodeKind::Image(Box::new(image)));
        let p = m.place(&image, &d);
        assert!(p.count() as u64 <= MAX_SLICE_GROWTH);
        let kids = p.children().unwrap();
        assert_eq!(kids.len(), 3);
        assert!(kids.iter().all(|c| matches!(c.children().unwrap()[1].kind, NodeKind::Image(_))));
        // For writers without clipping, the bar is one path cut at the guides and the image one
        // image placed by the center cell.
        let p = m.place_unclipped(&bar, &d);
        assert!(matches!(p.kind, NodeKind::Path { .. }), "{p:?}");
        assert!(near(p.geometric_bounds().unwrap(), Rect::new(0.0, 55.0, 200.0, 95.0)), "{:?}", p.geometric_bounds());
        assert!(matches!(m.place_unclipped(&image, &d).kind, NodeKind::Image(_)));
    }

    /// A shadowed, rounded, feathered, half-opaque, multiplied and isolated gradient bar across
    /// the guides: its pieces keep the round corners and the feathering and are drawn without the
    /// shadow at default transparency, and the group around them carries the shadow, the opacity,
    /// the blending mode and the isolation.
    #[test]
    fn effects_on_art_across_the_guides_apply_to_all_its_cells() {
        let mut d = markers(Some(SymbolNineSlice::default()));
        let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
        let mut bar = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 40.0, 100.0, 60.0)), Appearance::basic(gradient, Paint::None, 0.0));
        let shadow = crate::Effect { id: "stylize.dropShadow".into(), params: json!({"x": 7, "y": 7, "blur": 5}), visible: true };
        let round = crate::Effect { id: "stylize.roundCorners".into(), params: json!({"radius": 4}), visible: true };
        let feather = crate::Effect { id: "stylize.feather".into(), params: json!({"radius": 3}), visible: true };
        bar.appearance.effects = vec![round.clone(), feather.clone(), shadow.clone()];
        (bar.opacity, bar.blend, bar.isolate) = (0.5, vectorcraft_color::BlendMode::Multiply, true);
        let m = d.symbol_slice_map("S", Affine::scale_non_uniform(2.0, 1.5)).unwrap();
        let p = m.place(&bar, &d);
        assert_eq!(p.appearance.effects, vec![shadow.clone()]);
        assert_eq!((p.opacity, p.blend, p.isolate), (0.5, vectorcraft_color::BlendMode::Multiply, true));
        let cells = p.children().unwrap();
        assert_eq!(cells.len(), 3, "{p:?}");
        // The copies keep the effects that reshape them or follow their outline.
        for c in cells {
            let [clip, copy] = c.children().unwrap().as_slice() else { panic!("{c:?}") };
            assert!(clip.appearance.effects.is_empty(), "{clip:?}");
            assert_eq!(copy.appearance.effects, vec![round.clone(), feather.clone()]);
            assert!(copy.has_default_transparency(), "{copy:?}");
        }
        // A shadow followed by an effect that stays on the copies stays with it, in its order.
        bar.appearance.effects = vec![shadow.clone(), round.clone()];
        let p = m.place(&bar, &d);
        assert!(p.appearance.effects.is_empty() && p.opacity == 1.0, "{p:?}");
        assert!(p.children().unwrap().iter().all(|c| c.children().unwrap()[1].appearance.effects == vec![shadow.clone(), round.clone()]));
        // A hidden effect after it does not keep it on the copies.
        let hidden = crate::Effect { visible: false, ..round.clone() };
        bar.appearance.effects = vec![shadow.clone(), hidden.clone()];
        assert_eq!(m.place(&bar, &d).appearance.effects, vec![shadow.clone(), hidden]);
        // Without effects the pieces keep the opacity.
        bar.appearance.effects.clear();
        let p = m.place(&bar, &d);
        assert_eq!(p.opacity, 1.0);
        assert!(p.children().unwrap().iter().all(|c| c.children().unwrap()[1].opacity == 0.5));
    }

    /// A gradient bar across the guides with Knockout Shape set keeps it on the group that holds
    /// its cells, the object a knockout group reads it from.
    #[test]
    fn knockout_shape_on_art_across_the_guides_stays_on_its_group() {
        let mut d = markers(Some(SymbolNineSlice::default()));
        let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
        let mut bar = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 40.0, 100.0, 60.0)), Appearance::basic(gradient, Paint::None, 0.0));
        bar.knockout_shape = true;
        let m = d.symbol_slice_map("S", Affine::scale_non_uniform(2.0, 1.5)).unwrap();
        let p = m.place(&bar, &d);
        assert_eq!(p.children().map(|c| c.len()), Some(3), "{p:?}");
        assert!(p.knockout_shape, "{p:?}");
    }

    /// A locked gradient bar across the guides stays locked: the group that holds its cells is
    /// locked too, so Select All does not list it.
    #[test]
    fn locked_art_across_the_guides_stays_locked() {
        let mut d = markers(Some(SymbolNineSlice::default()));
        let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
        let mut bar = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 40.0, 100.0, 60.0)), Appearance::basic(gradient, Paint::None, 0.0));
        bar.locked = true;
        let m = d.symbol_slice_map("S", Affine::scale_non_uniform(2.0, 1.5)).unwrap();
        let p = m.place(&bar, &d);
        assert_eq!(p.children().map(|c| c.len()), Some(3), "{p:?}");
        assert!(p.locked, "{p:?}");
    }

    /// An instance that does not fit leaves the budget as it was: it needs room for the most
    /// objects placing can make, and a plain one after it still fits.
    #[test]
    fn a_skipped_instance_leaves_room_for_the_next() {
        let mut used = MAX_SYMBOL_NODES - 100;
        let mut placed = false;
        assert_eq!(
            charge_symbol_art(&mut used, 280, || {
                placed = true;
                ((), 10)
            }),
            None
        );
        assert!(!placed && used == MAX_SYMBOL_NODES - 100);
        // Placed, but it made more objects than are left.
        assert_eq!(charge_symbol_art(&mut used, 84, || ((), 101)), None);
        assert_eq!(used, MAX_SYMBOL_NODES - 100);
        assert_eq!(charge_symbol_art(&mut used, 100, || ("plain", 100)), Some("plain"));
        assert_eq!(used, MAX_SYMBOL_NODES);
        assert_eq!(charge_symbol_art(&mut used, 1, || ((), 1)), None);
        // An instance is charged the objects placing made.
        let mut used = 0;
        assert_eq!(charge_symbol_art(&mut used, 280, || ((), 37)), Some(()));
        assert_eq!(used, 37);
    }

    /// The bound on what placing makes counts each object in one cell once and each object across
    /// the guides by the cells it reaches, and is never below what placing makes.
    #[test]
    fn the_bound_on_placed_objects_follows_the_cells_they_reach() {
        let mut d = markers(Some(SymbolNineSlice::default()));
        let m = d.symbol_slice_map("S", Affine::scale_non_uniform(2.0, 1.5)).unwrap();
        // Six squares in one cell each and a plain frame cut at the guides: the group and its seven.
        let art = d.symbols[0].art.clone();
        assert_eq!(m.most_placed(&art, &d), 8);
        assert_eq!(m.place(&art, &d).count(), 8);
        // Many objects in one corner count one each.
        let many: Vec<Arc<Node>> = (0..1_000).map(|_| Arc::new(square(&mut d, Rect::new(1.0, 1.0, 9.0, 9.0), 0.0))).collect();
        let corner = Node::group(d.alloc_id(), many);
        assert_eq!(m.most_placed(&corner, &d), 1_001);
        assert_eq!(m.place(&corner, &d).count(), 1_001);
        // A gradient bar across three columns: a group, then a clip path and a copy in each.
        let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
        let bar = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 40.0, 100.0, 60.0)), Appearance::basic(gradient, Paint::None, 0.0));
        assert_eq!(m.most_placed(&bar, &d), 10);
        assert_eq!(m.place(&bar, &d).count(), 10);
        // Art nested past the depth limit, clipped in all nine cells.
        let mut deep = square(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), 0.0);
        for _ in 0..=MAX_SLICE_DEPTH {
            deep = Node::group(d.alloc_id(), vec![Arc::new(deep)]);
        }
        let most = m.most_placed(&deep, &d);
        assert!(most >= m.place(&deep, &d).count() as u64 && most <= deep.count() as u64 * MAX_SLICE_GROWTH, "{most}");
    }

    /// The bound counts the art of opacity masks, which placing copies into each cell (unlinked)
    /// or places through the guides (linked).
    #[test]
    fn the_bound_on_placed_objects_counts_opacity_masks() {
        let mut d = markers(Some(SymbolNineSlice::default()));
        let m = d.symbol_slice_map("S", Affine::scale_non_uniform(2.0, 1.5)).unwrap();
        let many: Vec<Arc<Node>> = (0..1_000).map(|_| Arc::new(square(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), 0.0))).collect();
        let mask = Node::group(d.alloc_id(), many);
        let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
        let mut bar = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 40.0, 100.0, 60.0)), Appearance::basic(gradient, Paint::None, 0.0));
        let mut frame = square(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0), 0.0);
        for linked in [false, true] {
            let mut om = OpacityMask::new(mask.clone(), true);
            om.linked = linked;
            bar.extras_mut().mask = Some(Box::new(om.clone()));
            frame.extras_mut().mask = Some(Box::new(om));
            let group = Node::group(d.alloc_id(), vec![Arc::new(bar.clone()), Arc::new(frame.clone())]);
            for art in [&bar, &frame, &group] {
                let most = m.most_placed(art, &d);
                let placed = m.place(art, &d).count_with_masks() as u64;
                assert!(most >= placed && placed > 1_000, "linked {linked}: {most} < {placed}");
            }
        }
    }

    /// Art that placing through the guides could make more than [`MAX_SYMBOL_NODES`] objects of
    /// is not placed. Without clips the same path is cut at the guides once, with one copy of its
    /// mask, and is placed; at the natural size it is moved as it is.
    #[test]
    fn symbol_art_past_the_object_limit_is_not_placed() {
        let mut d = markers(Some(SymbolNineSlice::default()));
        let leaf = Arc::new(square(&mut d, Rect::new(0.0, 0.0, 1.0, 1.0), 0.0));
        let inner = Arc::new(Node::group(d.alloc_id(), vec![leaf; 1_500]));
        let mut mask = OpacityMask::new(Node::group(d.alloc_id(), vec![inner; 1_500]), true);
        mask.linked = false;
        let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
        let mut art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(gradient, Paint::None, 0.0));
        art.extras_mut().mask = Some(Box::new(mask));
        let xf = Affine::scale_non_uniform(2.0, 1.5);
        assert!(d.place_symbol_art("S", &art, xf).is_none());
        assert!(d.place_symbol_art_unclipped("S", &art, xf).is_some());
        assert!(d.place_symbol_art("S", &art, Affine::translate((5.0, 5.0))).is_some());
    }

    /// A symbol instance that crosses the guides is placed as one piece, by the cell its center is
    /// in, so copies do not multiply with nesting; one in a corner is placed by the corner.
    #[test]
    fn nested_instances_are_placed_as_one_piece() {
        let mut d = markers(Some(SymbolNineSlice::default()));
        let wide = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::new([0.8, 0.0, 0.0, 0.1, 10.0, 45.0]) });
        let corner = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::new([0.05, 0.0, 0.0, 0.05, 92.0, 92.0]) });
        let m = d.symbol_slice_map("S", Affine::scale_non_uniform(2.0, 1.5)).unwrap();
        let NodeKind::SymbolInstance { xf, .. } = m.place(&wide, &d).kind else { panic!() };
        // Its center is in the center cell, which stretches 3 times across and 2 times down.
        assert!((xf.as_coeffs()[0] - 2.4).abs() < 1e-9 && (xf.as_coeffs()[3] - 0.2).abs() < 1e-9, "{xf:?}");
        let NodeKind::SymbolInstance { xf, .. } = m.place(&corner, &d).kind else { panic!() };
        assert_eq!(xf, Affine::new([0.05, 0.0, 0.0, 0.05, 192.0, 142.0]));
    }

    /// Type with an inline graphic, a placed document, a brushed stroke and a pattern whose tile
    /// holds an instance draw symbol art: across the guides each is placed by its center's cell as
    /// one piece, and counted as one, while the same art without them is drawn in each cell.
    #[test]
    fn art_that_draws_symbol_art_across_the_guides_is_placed_as_one_piece() {
        let mut d = markers(Some(SymbolNineSlice::default()));
        let m = d.symbol_slice_map("S", Affine::scale_non_uniform(2.0, 1.5)).unwrap();
        let across = Rect::new(0.0, 40.0, 100.0, 60.0);
        let one_piece = |d: &Document, n: &Node| {
            let p = m.place(n, d);
            assert_eq!(std::mem::discriminant(&p.kind), std::mem::discriminant(&n.kind), "{p:?}");
            assert_eq!(m.most_placed(n, d), n.count_with_masks() as u64);
        };
        let in_cells = |d: &Document, n: &Node| {
            let p = m.place(n, d);
            assert_eq!(p.children().map(Vec::len), Some(3), "{p:?}");
        };
        // Type.
        let mut text = crate::TextObject::point(Point::ZERO, "Label", crate::CharStyle::default());
        text.kind = crate::TextKind::Area { frame: shapes::rectangle(across) };
        text.xf = Affine::IDENTITY;
        let plain = Node::new(d.alloc_id(), NodeKind::Text(Box::new(text.clone())));
        text.runs[0].inline = Some(crate::InlineArt::new("S"));
        let inline = Node::new(d.alloc_id(), NodeKind::Text(Box::new(text)));
        in_cells(&d, &plain);
        one_piece(&d, &inline);
        // A placed document.
        let doc = PlacedDocument {
            link: crate::LinkInfo::new("/art/logo.vectorcraft"),
            key: "k".into(),
            bounding: false,
            page_box: crate::PageBox::Crop,
            width: 100.0,
            height: 20.0,
            xf: Affine::translate((0.0, 40.0)),
            placement: Default::default(),
        };
        let doc = Node::new(d.alloc_id(), NodeKind::PlacedDocument(Box::new(doc)));
        one_piece(&d, &doc);
        // A brushed stroke on a gradient bar, in a group so the group's member is the one placed.
        let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
        let bar = Node::path(d.alloc_id(), shapes::rectangle(across), Appearance::basic(gradient, Paint::solid(Color::BLACK), 1.0));
        in_cells(&d, &bar);
        let mut brushed = bar.clone();
        brushed.appearance.stroke_mut().unwrap().brush = Some("Brush".into());
        one_piece(&d, &brushed);
        let group = Node::group(d.alloc_id(), vec![Arc::new(brushed)]);
        assert!(matches!(m.place(&group, &d).children().unwrap()[0].kind, NodeKind::Path { .. }));
        // Patterns: one whose tile holds an instance (through a second pattern), one whose does not.
        let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf: Affine::scale(0.1) });
        let mut tile = square(&mut d, Rect::new(0.0, 0.0, 10.0, 10.0), 0.0);
        d.patterns.push(PatternDef::new("Plain", vec![Arc::new(tile.clone())]));
        d.patterns.push(PatternDef::new("Inner", vec![Arc::new(instance)]));
        tile.appearance = Appearance::basic(Paint::Pattern { pattern: "Inner".into(), xf: Default::default() }, Paint::None, 0.0).into();
        d.patterns.push(PatternDef::new("Outer", vec![Arc::new(tile)]));
        for (name, holds) in [("Plain", false), ("Outer", true)] {
            let fill = Paint::Pattern { pattern: name.into(), xf: Default::default() };
            let bar = Node::path(d.alloc_id(), shapes::rectangle(across), Appearance::basic(fill, Paint::None, 0.0));
            if holds { one_piece(&d, &bar) } else { in_cells(&d, &bar) }
        }
        // A hidden stroke painted with that pattern draws nothing: the gradient bar is drawn in
        // each cell.
        let outer = Paint::Pattern { pattern: "Outer".into(), xf: Default::default() };
        let gradient = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
        let mut hidden = Node::path(d.alloc_id(), shapes::rectangle(across), Appearance::basic(gradient, outer, 1.0));
        hidden.appearance.stroke_mut().unwrap().visible = false;
        in_cells(&d, &hidden);
        // A brushed path with solid paint is cut at the guides as one path, drawn once.
        let black = Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::BLACK), 1.0);
        let mut solid = Node::path(d.alloc_id(), shapes::rectangle(across), black);
        solid.appearance.stroke_mut().unwrap().brush = Some("Brush".into());
        assert!(matches!(m.place(&solid, &d).kind, NodeKind::Path { .. }));
    }

    /// Art nested deeper than [`MAX_SLICE_DEPTH`] is clipped to the cells it reaches, so it still
    /// draws with its corners unstretched; paths are not cut at the guides there.
    #[test]
    fn art_nested_past_the_limit_is_clipped_to_the_cells() {
        let mut d = Document::new(400.0, 400.0);
        let mut deep = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default());
        for _ in 0..=MAX_SLICE_DEPTH {
            deep = Node::group(d.alloc_id(), vec![Arc::new(deep)]);
        }
        d.symbols.push(Symbol { dynamic: false, name: "S".into(), art: Arc::new(deep) });
        d.set_symbol_nine_slice("S", Some(SymbolNineSlice::default()));
        let mut n = placed(&d, Affine::scale_non_uniform(2.0, 1.5));
        assert!(n.count() as u64 <= d.symbols[0].art.count() as u64 * MAX_SLICE_GROWTH);
        for _ in 0..MAX_SLICE_DEPTH {
            n = (*n.children().unwrap()[0]).clone();
        }
        let cells = n.children().unwrap();
        assert_eq!(cells.len(), 9, "{n:?}");
        assert!(cells.iter().all(|c| matches!(c.kind, NodeKind::Group { clip: true, .. })));
        // The top-left cell's copy is the path moved, at its natural size.
        let copy = &cells[0].children().unwrap()[1];
        assert!(near(copy.geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 100.0, 100.0)), "{copy:?}");
    }

    /// Symbols made in the app keep their art in a 20 × 20 square and their size apart: the
    /// guides cut the art at its natural size.
    #[test]
    fn the_guides_cut_the_art_at_its_natural_size() {
        let mut d = Document::new(400.0, 400.0);
        let corner = square(&mut d, Rect::new(-10.0, -10.0, -8.0, -8.0), 0.0);
        let frame = square(&mut d, Rect::new(-10.0, -10.0, 10.0, 10.0), 0.0);
        let group = Node::group(d.alloc_id(), vec![Arc::new(frame), Arc::new(corner)]);
        d.symbols.push(Symbol { dynamic: false, name: "S".into(), art: Arc::new(group) });
        d.unknown.insert(SYMBOL_SIZES.into(), json!({ "S": [100.0, 100.0] }));
        d.set_symbol_nine_slice("S", Some(SymbolNineSlice::default()));
        // 200 × 150 is 10 × 7.5 times the 20 × 20 art.
        let got = boxes(&d, Affine::scale_non_uniform(10.0, 7.5));
        assert!(near(got[0], Rect::new(-100.0, -75.0, 100.0, 75.0)), "{:?}", got[0]);
        assert!(near(got[1], Rect::new(-100.0, -75.0, -90.0, -65.0)), "{:?}", got[1]);
        // At its natural size the guides change nothing.
        assert!(d.symbol_slice_map("S", Affine::scale(5.0)).is_none());
    }

    /// A scaled instance with strokes in its art is measured by placing the art through the
    /// guides only while placing it stays within the limit; past it, the art is not placed and
    /// the instance is measured by its art box and the strokes' reach.
    #[test]
    fn a_stroked_instance_past_the_limit_is_measured_without_placing_its_art() {
        let mut d = markers(Some(SymbolNineSlice::default()));
        let ring = square(&mut d, Rect::new(40.0, 40.0, 60.0, 60.0), 4.0);
        let mut art = (*d.symbols[0].art).clone();
        art.children_mut().unwrap().push(Arc::new(ring));
        d.symbols[0].art = Arc::new(art);
        let xf = Affine::scale_non_uniform(2.0, 1.5);
        let most = d.symbol_slice_map("S", xf).unwrap().most_placed(&d.symbols[0].art, &d);
        let whole = Rect::new(0.0, 0.0, 200.0, 150.0);
        let boxes = d.symbol_boxes();
        let placed = boxes.placed_visual_within("S", xf, most);
        assert!(placed.is_some_and(|b| near(b, whole)), "{placed:?}");
        assert_eq!(boxes.placed_visual_within("S", xf, most - 1), None);
        // Within the real limit the instance's visual box is the placed art's.
        let inst = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "S".into(), xf });
        let v = inst.visual_bounds_with(&d.symbol_boxes());
        assert!(v.is_some_and(|b| near(b, whole)), "{v:?}");
    }
}
