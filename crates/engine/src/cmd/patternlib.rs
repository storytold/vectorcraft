//! Pattern libraries (Window → Swatch Libraries → Patterns, the Swatches panel's library button):
//! read-only sets of pattern swatches the swatch library panel shows. Each built-in library is a
//! document holding its patterns, drawn here from plain shapes ([`PATTERN_LIBRARIES`]).
//! `swatch.library.add` copies a pattern into the document with its swatch.

use std::sync::{Arc, LazyLock};

use vectorcraft_color::{Color, Paint, Swatch, SwatchLibrary};
use vectorcraft_doc::pattern::pattern_paint;
use vectorcraft_doc::{Appearance, Document, Node, NodeId, PatternDef, TileType};
use vectorcraft_geom::{PathData, Point, Rect, SubPath, shapes};

use super::Result;
use super::clipboard::Clipboard;
use super::clipboard::resources::SwatchChoices;
use super::swatchlib::LibraryInfo;

/// A built-in pattern library: a stable id, its menu name and the function that adds its patterns.
pub struct BuiltinPatternLibrary {
    pub id: &'static str,
    pub name: &'static str,
    make: fn(&mut Document),
}

/// The built-in pattern libraries (Window → Swatch Libraries → Patterns), in menu order.
pub const PATTERN_LIBRARIES: &[BuiltinPatternLibrary] = &[
    BuiltinPatternLibrary { id: "geometric-patterns", name: "Geometric Patterns", make: geometric_patterns },
    BuiltinPatternLibrary { id: "grit-textures", name: "Grit Textures", make: grit_textures },
    BuiltinPatternLibrary { id: "optical-effects", name: "Optical Effects", make: optical_effects },
];

/// The library category of the built-in pattern libraries.
pub const CATEGORY: &str = "patterns";

/// The built-in pattern libraries as the menus list them.
pub fn infos() -> impl Iterator<Item = LibraryInfo> {
    PATTERN_LIBRARIES.iter().map(|b| LibraryInfo { id: b.id.into(), name: b.name.into(), category: CATEGORY })
}

/// Built-in pattern library `id`: the document holding its patterns, made once.
pub fn library_doc(id: &str) -> Option<Arc<Document>> {
    static MADE: LazyLock<Vec<Arc<Document>>> = LazyLock::new(|| {
        PATTERN_LIBRARIES
            .iter()
            .map(|b| {
                let mut d = Document::new(100.0, 100.0);
                d.title = b.name.into();
                (b.make)(&mut d);
                Arc::new(d)
            })
            .collect()
    });
    PATTERN_LIBRARIES.iter().position(|b| b.id == id).and_then(|i| MADE.get(i).cloned())
}

/// Built-in pattern library `id` as the library panel shows it: a swatch for each pattern.
pub fn library(id: &str) -> Option<(LibraryInfo, Arc<SwatchLibrary>)> {
    static MADE: LazyLock<Vec<Arc<SwatchLibrary>>> = LazyLock::new(|| {
        PATTERN_LIBRARIES
            .iter()
            .map(|b| {
                let swatches = library_doc(b.id)
                    .map(|d| {
                        d.patterns
                            .iter()
                            .map(|p| Swatch { name: p.name.clone(), paint: pattern_paint(&p.name), global: false, spot: false })
                            .collect()
                    })
                    .unwrap_or_default();
                Arc::new(SwatchLibrary { name: b.name.into(), swatches, groups: vec![] })
            })
            .collect()
    });
    let i = PATTERN_LIBRARIES.iter().position(|b| b.id == id)?;
    Some((infos().nth(i)?, MADE.get(i)?.clone()))
}

/// The built-in library document that holds the pattern of swatch `w`, when `w` is one of a
/// built-in pattern library's own swatches (a library panel draws them from it). A swatch of
/// another library is not, even under the same name.
pub fn swatch_source(w: &Swatch) -> Option<Arc<Document>> {
    let Paint::Pattern { pattern, .. } = &w.paint else { return None };
    let own = |lib: &SwatchLibrary| lib.swatches.iter().any(|s| std::ptr::eq(s, w));
    let b = PATTERN_LIBRARIES.iter().find(|b| library(b.id).is_some_and(|(_, lib)| own(&lib)))?;
    library_doc(b.id).filter(|d| d.pattern(pattern).is_some())
}

/// Add pattern `name` of library `lib` to `d` with the resources its art uses, as pasting art
/// painted with it does: the same pattern there already is used, and a name another pattern or
/// swatch has gets a number → the pattern's name in `d`.
pub(crate) fn import(d: &mut Document, lib: &Document, name: &str) -> Result<String> {
    let user = Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::basic(pattern_paint(name), Paint::None, 0.0));
    let imported = Clipboard::gather(lib, vec![user]).import_into(d, &SwatchChoices::default(), false)?;
    let renamed = imported.renamed.iter().find(|r| r["kind"] == "pattern" && r["from"] == name).and_then(|r| r["to"].as_str());
    Ok(renamed.unwrap_or(name).to_string())
}

/// The pattern of `d` that is pattern `name` of library `lib`: the same tile options and art under
/// that name or a numbered copy of it ("Polka Dots 2"), the name itself first.
pub(crate) fn existing_pattern(d: &Document, lib: &Document, name: &str) -> Option<String> {
    let def = lib.pattern(name)?;
    let bare = |p: &PatternDef| PatternDef { name: String::new(), art: vec![], ..p.clone() };
    let copy = |n: &str| {
        n == name || n.strip_prefix(name).and_then(|r| r.strip_prefix(' ')).is_some_and(|r| !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit()))
    };
    let mut found: Vec<&PatternDef> = d.patterns.iter().filter(|p| copy(&p.name) && bare(p) == bare(def) && p.art == def.art).collect();
    found.sort_by_key(|p| p.name != name);
    found.first().map(|p| p.name.clone())
}

/// The name of the first swatch of `d` that paints with pattern `pattern` as placed in its own
/// space, as a pattern's own swatch does.
pub(crate) fn pattern_swatch(d: &Document, pattern: &str) -> Option<String> {
    let paint = pattern_paint(pattern);
    d.swatches_iter().find(|w| w.paint == paint).map(|w| w.name.clone())
}

/// A pattern of `shapes` filled with black on the tile `w` × `h` from the origin (`tile_type`:
/// how the tiles are laid out).
fn add(d: &mut Document, name: &str, w: f64, h: f64, tile_type: TileType, shapes: Vec<PathData>) {
    let art =
        shapes.into_iter().map(|p| Arc::new(Node::path(d.alloc_id(), p, Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0)))).collect();
    let def = PatternDef { tile: Rect::new(0.0, 0.0, w, h), tile_type, ..PatternDef::new(name, art) };
    d.patterns.push(def);
}

fn polygon(points: &[(f64, f64)]) -> PathData {
    let points: Vec<Point> = points.iter().map(|&(x, y)| Point::new(x, y)).collect();
    PathData::single(SubPath::polyline(&points, true))
}

/// Geometric Patterns: black dots, stripes, checks, grids and shapes on a clear background.
fn geometric_patterns(d: &mut Document) {
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| shapes::rectangle(Rect::new(x0, y0, x1, y1));
    add(d, "Polka Dots", 12.0, 12.0, TileType::BrickByRow { offset: 0.5 }, vec![shapes::ellipse(Rect::new(3.0, 3.0, 9.0, 9.0))]);
    add(d, "Fine Dots", 6.0, 6.0, TileType::Grid, vec![shapes::ellipse(Rect::new(2.0, 2.0, 4.0, 4.0))]);
    add(d, "Vertical Stripes", 8.0, 8.0, TileType::Grid, vec![rect(0.0, 0.0, 3.0, 8.0)]);
    add(d, "Horizontal Stripes", 8.0, 8.0, TileType::Grid, vec![rect(0.0, 0.0, 8.0, 3.0)]);
    // A band along the tile's diagonal and the corners of the two bands beside it.
    add(
        d,
        "Diagonal Stripes",
        10.0,
        10.0,
        TileType::Grid,
        vec![
            polygon(&[(0.0, 0.0), (1.5, 0.0), (10.0, 8.5), (10.0, 10.0), (8.5, 10.0), (0.0, 1.5)]),
            polygon(&[(8.5, 0.0), (10.0, 0.0), (10.0, 1.5)]),
            polygon(&[(0.0, 8.5), (1.5, 10.0), (0.0, 10.0)]),
        ],
    );
    add(d, "Checkerboard", 10.0, 10.0, TileType::Grid, vec![rect(0.0, 0.0, 5.0, 5.0), rect(5.0, 5.0, 10.0, 10.0)]);
    add(d, "Grid", 10.0, 10.0, TileType::Grid, vec![rect(0.0, 0.0, 10.0, 0.75), rect(0.0, 0.75, 0.75, 10.0)]);
    add(d, "Diamonds", 10.0, 10.0, TileType::Grid, vec![polygon(&[(5.0, 1.0), (9.0, 5.0), (5.0, 9.0), (1.0, 5.0)])]);
    add(d, "Triangles", 10.0, 10.0, TileType::BrickByRow { offset: 0.5 }, vec![polygon(&[(1.0, 9.0), (5.0, 1.0), (9.0, 9.0)])]);
    add(d, "Crosses", 10.0, 10.0, TileType::Grid, vec![rect(4.0, 1.5, 6.0, 8.5), rect(1.5, 4.0, 8.5, 6.0)]);
    add(d, "Zigzag", 10.0, 8.0, TileType::Grid, vec![polygon(&[(0.0, 5.0), (5.0, 1.0), (10.0, 5.0), (10.0, 7.0), (5.0, 3.0), (0.0, 7.0)])]);
}

/// SplitMix64, a small generator with a fixed seed per pattern, so a grit pattern draws the same
/// marks every time it is made.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number from `lo` up to, but not including, `hi`.
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (self.next() >> 11) as f64 / (1u64 << 53) as f64 * (hi - lo)
    }
}

/// A round dot of radius `r` at (`x`, `y`).
fn dot(x: f64, y: f64, r: f64) -> PathData {
    shapes::ellipse(Rect::new(x - r, y - r, x + r, y + r))
}

/// An irregular fleck of four to seven corners about (`x`, `y`), at most `r` from it.
fn fleck(rng: &mut SplitMix64, x: f64, y: f64, r: f64) -> PathData {
    use std::f64::consts::TAU;
    let n = 4 + (rng.next() % 4) as usize;
    let turn = rng.range(0.0, TAU);
    let points: Vec<(f64, f64)> = (0..n)
        .map(|k| {
            let a = turn + TAU * (k as f64 + rng.range(-0.3, 0.3)) / n as f64;
            let reach = r * rng.range(0.55, 1.0);
            (x + reach * a.cos(), y + reach * a.sin())
        })
        .collect();
    polygon(&points)
}

/// A thin straight scratch `len` long and `width` wide, centered on (`x`, `y`) at a random angle.
fn scratch(rng: &mut SplitMix64, x: f64, y: f64, len: f64, width: f64) -> PathData {
    let (s, c) = rng.range(0.0, std::f64::consts::PI).sin_cos();
    let (hx, hy, wx, wy) = (c * len / 2.0, s * len / 2.0, -s * width / 2.0, c * width / 2.0);
    polygon(&[(x - hx - wx, y - hy - wy), (x + hx - wx, y + hy - wy), (x + hx + wx, y + hy + wy), (x - hx + wx, y - hy + wy)])
}

/// A grit pattern of `count` marks scattered over a square grid tile of side `size`, drawn from
/// `seed` (`mark` draws one at a point). A mark that reaches past an edge is kept whole: the tiling
/// draws the part outside the tile on the next tile, so the tiles join without a seam.
fn grit(d: &mut Document, name: &str, size: f64, seed: u64, count: usize, mark: impl Fn(&mut SplitMix64, f64, f64) -> PathData) {
    let mut rng = SplitMix64(seed);
    let marks = (0..count)
        .map(|_| {
            let (x, y) = (rng.range(0.0, size), rng.range(0.0, size));
            mark(&mut rng, x, y)
        })
        .collect();
    add(d, name, size, size, TileType::Grid, marks);
}

/// Grit Textures: seamless black grain, grit, sand, speckle, stipple, scratch and fleck textures on
/// a clear background, each drawn from its own fixed seed.
fn grit_textures(d: &mut Document) {
    grit(d, "Fine Grain", 24.0, 1, 320, |rng, x, y| dot(x, y, rng.range(0.15, 0.35)));
    grit(d, "Coarse Grit", 30.0, 2, 150, |rng, x, y| {
        let r = rng.range(0.45, 1.1);
        fleck(rng, x, y, r)
    });
    grit(d, "Sand", 24.0, 3, 280, |rng, x, y| {
        let r = rng.range(0.2, 0.45);
        fleck(rng, x, y, r)
    });
    grit(d, "Speckle", 30.0, 4, 90, |rng, x, y| dot(x, y, rng.range(0.3, 0.9)));
    // Stipple: one dot in each cell of a 12 × 12 grid, moved a little off the cell's center.
    let mut rng = SplitMix64(5);
    let (size, cells) = (24.0, 12);
    let step = size / f64::from(cells);
    let dots = (0..cells)
        .flat_map(|row| (0..cells).map(move |col| (col, row)))
        .map(|(col, row)| {
            let x = (f64::from(col) + 0.5) * step + rng.range(-0.3, 0.3) * step;
            let y = (f64::from(row) + 0.5) * step + rng.range(-0.3, 0.3) * step;
            dot(x, y, rng.range(0.3, 0.45))
        })
        .collect();
    add(d, "Stipple", size, size, TileType::Grid, dots);
    grit(d, "Scratches", 36.0, 6, 45, |rng, x, y| {
        let (len, width) = (rng.range(2.0, 7.0), rng.range(0.15, 0.35));
        scratch(rng, x, y, len, width)
    });
    grit(d, "Sparse Flecks", 36.0, 7, 32, |rng, x, y| {
        let r = rng.range(0.35, 0.8);
        fleck(rng, x, y, r)
    });
    grit(d, "Dense Flecks", 18.0, 8, 220, |rng, x, y| {
        let r = rng.range(0.2, 0.5);
        fleck(rng, x, y, r)
    });
}

/// `n` + 1 points of the curve `(t, f(t))` for `t` from `t0` to `t1`, evenly spaced.
fn curve(t0: f64, t1: f64, n: u32, f: impl Fn(f64) -> f64) -> Vec<(f64, f64)> {
    (0..=n)
        .map(|i| {
            let t = t0 + (t1 - t0) * f64::from(i) / f64::from(n);
            (t, f(t))
        })
        .collect()
}

/// The closed band between two curves drawn from the same side to the same side: `upper` forward,
/// then `lower` back.
fn band(upper: &[(f64, f64)], lower: &[(f64, f64)]) -> PathData {
    let points: Vec<(f64, f64)> = upper.iter().chain(lower.iter().rev()).copied().collect();
    polygon(&points)
}

/// 72 corners of the circle of radius `r` about (`cx`, `cy`), turning the other way with `reverse`.
fn circle(cx: f64, cy: f64, r: f64, reverse: bool) -> SubPath {
    const CORNERS: u32 = 72;
    let turn = if reverse { -std::f64::consts::TAU } else { std::f64::consts::TAU };
    let points: Vec<Point> = (0..CORNERS)
        .map(|i| {
            let a = turn * f64::from(i) / f64::from(CORNERS);
            Point::new(cx + r * a.cos(), cy + r * a.sin())
        })
        .collect();
    SubPath::polyline(&points, true)
}

/// The ring from radius `inner` to `outer` about (`cx`, `cy`): the outer circle and the inner one
/// turning the other way, so the middle stays clear under either fill rule. An `inner` of 0 gives
/// a disk.
fn ring(cx: f64, cy: f64, inner: f64, outer: f64) -> PathData {
    let mut subpaths = vec![circle(cx, cy, outer, false)];
    if inner > 0.0 {
        subpaths.push(circle(cx, cy, inner, true));
    }
    PathData::new(subpaths)
}

/// A lens `2 × l` long and `2 × h` high about (`cx`, `cy`), lying along x: two half sine waves that
/// meet at its tips, turning the other way with `reverse`.
fn lens(cx: f64, cy: f64, l: f64, h: f64, reverse: bool) -> SubPath {
    const STEPS: u32 = 32;
    let at = |i: u32, side: f64| {
        let t = f64::from(i) / f64::from(STEPS);
        Point::new(cx - l + 2.0 * l * t, cy + side * h * (std::f64::consts::PI * t).sin())
    };
    let mut points: Vec<Point> = (0..=STEPS).map(|i| at(i, -1.0)).chain((1..STEPS).rev().map(|i| at(i, 1.0))).collect();
    if reverse {
        points.reverse();
    }
    SubPath::polyline(&points, true)
}

/// Optical Effects: black waves, rings, warped checks, pinched stripes, chevrons and lenses on a
/// clear background. Each tile is one period of its design, so a band that leaves one edge comes
/// back at the same place on the opposite edge. Art that reaches past an edge is kept whole: the
/// tiling draws the part outside the tile on the next tile.
fn optical_effects(d: &mut Document) {
    use std::f64::consts::TAU;
    // Three bands 5 pt wide, 12 pt apart, each one sine wave across the tile.
    let bands = (0..3u32)
        .map(|k| {
            let mid = move |x: f64| 6.0 + 12.0 * f64::from(k) + 3.0 * (TAU * x / 36.0).sin();
            band(&curve(0.0, 36.0, 48, |x| mid(x) - 2.5), &curve(0.0, 36.0, 48, |x| mid(x) + 2.5))
        })
        .collect();
    add(d, "Sine Bands", 36.0, 36.0, TileType::Grid, bands);
    // Nine thin waves 4 pt apart, two waves across the tile, each a ninth of a wave on from the one
    // above it, so the ninth leads into the first wave of the next tile down.
    let lines = (0..9u32)
        .map(|k| {
            let (y, phase) = (2.0 + 4.0 * f64::from(k), TAU * f64::from(k) / 9.0);
            let mid = move |x: f64| y + 2.5 * (TAU * x / 18.0 + phase).sin();
            band(&curve(0.0, 36.0, 72, |x| mid(x) - 0.6), &curve(0.0, 36.0, 72, |x| mid(x) + 0.6))
        })
        .collect();
    add(d, "Ripple Lines", 36.0, 36.0, TileType::Grid, lines);
    // Thin rings 3 pt apart about the center and rings 3.3 pt apart about a point 4 pt to its
    // right. The two sets cross, and the outer rings reach into the next tile across.
    let rings = (1..=7u32)
        .map(|k| (24.0, 3.0 * f64::from(k)))
        .chain((1..=7u32).map(|k| (28.0, 3.3 * f64::from(k))))
        .map(|(cx, r)| ring(cx, 24.0, r - 0.6, r + 0.6))
        .collect();
    add(d, "Interference Rings", 48.0, 48.0, TileType::Grid, rings);
    // Rings 2 pt and 1 pt wide by turns about the tile's corner and about its center, the center's
    // starting with the narrower one. The corner's rings are the same rings at all four corners.
    let mut rings = vec![];
    for (c, first) in [(0.0, 0), (20.0, 1)] {
        let mut r = 0.0;
        for k in 0..5 {
            let width = if (k + first) % 2 == 0 { 2.0 } else { 1.0 };
            rings.push(ring(c, c, r, r + width));
            r += width + 1.2;
        }
    }
    add(d, "Offset Rings", 40.0, 40.0, TileType::Grid, rings);
    // An 8 × 8 checkerboard of 4 pt cells whose columns are bent by a sine wave down the tile and
    // whose rows by one across it, each edge drawn through five points.
    let warp = |x: f64, y: f64| (x + 2.0 * (TAU * y / 32.0).sin(), y + 2.0 * (TAU * x / 32.0).sin());
    let cells = (0..8u32)
        .flat_map(|row| (0..8u32).map(move |col| (col, row)))
        .filter(|(col, row)| (col + row) % 2 == 0)
        .map(|(col, row)| {
            let (x, y) = (4.0 * f64::from(col), 4.0 * f64::from(row));
            let corners = [(x, y), (x + 4.0, y), (x + 4.0, y + 4.0), (x, y + 4.0), (x, y)];
            let points: Vec<(f64, f64)> = corners
                .windows(2)
                .flat_map(|e| {
                    let (a, b) = (e[0], e[1]);
                    (0..4u32).map(move |i| {
                        let t = f64::from(i) / 4.0;
                        warp(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
                    })
                })
                .collect();
            polygon(&points)
        })
        .collect();
    add(d, "Warped Checks", 32.0, 32.0, TileType::Grid, cells);
    // Six upright stripes 6 pt apart that widen from 0.8 pt to 5.6 pt and narrow again down the tile,
    // each a sixth of a wave on from the one to its left.
    let stripes = (0..6u32)
        .map(|k| {
            let (x, phase) = (3.0 + 6.0 * f64::from(k), TAU * f64::from(k) / 6.0);
            let half = move |y: f64| 1.6 + 1.2 * (TAU * y / 36.0 + phase).cos();
            let side = |s: f64| -> Vec<(f64, f64)> { curve(0.0, 36.0, 48, |y| x + s * half(y)).into_iter().map(|(y, at)| (at, y)).collect() };
            band(&side(-1.0), &side(1.0))
        })
        .collect();
    add(d, "Pinched Stripes", 36.0, 36.0, TileType::Grid, stripes);
    // Four zigzag bands 8 pt apart, two zigzags across the tile; the bend and the width of the bands
    // rise and fall from band to band.
    let chevrons = [(0.0, 3.0, 4.0), (8.0, 4.5, 2.5), (16.0, 3.0, 1.0), (24.0, 1.5, 2.5)]
        .into_iter()
        .map(|(y, bend, width)| {
            let upper: Vec<(f64, f64)> = (0..=4u32).map(|i| (8.0 * f64::from(i), if i % 2 == 1 { y + bend } else { y })).collect();
            let lower: Vec<(f64, f64)> = upper.iter().map(|&(x, y)| (x, y + width)).collect();
            band(&upper, &lower)
        })
        .collect();
    add(d, "Chevron Pulse", 32.0, 32.0, TileType::Grid, chevrons);
    // Five nested lens outlines 1.5 pt wide about the center, 14 pt to 78 pt long. The longer ones
    // reach across the side edges and cross the next tiles' lenses.
    let lenses = (0..5u32)
        .map(|k| {
            let (l, h) = (7.0 + 8.0 * f64::from(k), 4.0 + 4.0 * f64::from(k));
            PathData::new(vec![lens(24.0, 24.0, l, h, false), lens(24.0, 24.0, l - 1.5, h - 1.5, true)])
        })
        .collect();
    add(d, "Radiating Lenses", 48.0, 48.0, TileType::Grid, lenses);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plus_signed_suffix_is_not_a_copy_number() {
        let mut lib = Document::new(100.0, 100.0);
        lib.patterns.push(PatternDef::new("Dots", vec![]));
        for (other, expected) in [("Dots +2", None), ("Dots -2", None), ("Dots 2a", None), ("Dots ", None), ("Dots 2", Some("Dots 2"))] {
            let mut d = Document::new(100.0, 100.0);
            d.patterns.push(PatternDef::new(other, vec![]));
            assert_eq!(existing_pattern(&d, &lib, "Dots").as_deref(), expected, "{other}");
        }
    }

    fn grit_doc() -> Document {
        let mut d = Document::new(100.0, 100.0);
        grit_textures(&mut d);
        d
    }

    #[test]
    fn grit_textures_draw_the_same_marks_every_time() {
        let (a, b) = (grit_doc(), grit_doc());
        assert_eq!(a.patterns.len(), 8);
        assert_eq!(a.patterns, b.patterns);
    }

    /// Every grit tile is 18 to 36 pt with at most 600 marks, each with an area, touching the tile
    /// and near it. Marks reach past the edges, and the tiling draws a tile's art on the next tile
    /// across each edge a mark crosses. No mark is another mark moved by a whole tile, which the tiling would draw
    /// twice in one place.
    #[test]
    fn grit_tiles_join_without_a_seam() {
        let mut crossing = 0;
        for p in &grit_doc().patterns {
            let (w, h) = (p.width(), p.height());
            assert!((18.0..=36.0).contains(&w) && w == h && p.tile.x0 == 0.0 && p.tile.y0 == 0.0, "{}", p.name);
            assert!(!p.art.is_empty() && p.art.len() <= 600, "{}: {} marks", p.name, p.art.len());
            let bounds: Vec<Rect> = p
                .art
                .iter()
                .map(|n| match &n.kind {
                    vectorcraft_doc::NodeKind::Path { path, .. } => path.bounds().unwrap_or_else(|| panic!("{}: an empty mark", p.name)),
                    _ => panic!("{}: a mark that is not a path", p.name),
                })
                .collect();
            let near = |a: &Rect, b: Rect| {
                (a.x0 - b.x0).abs() < 1e-9 && (a.y0 - b.y0).abs() < 1e-9 && (a.x1 - b.x1).abs() < 1e-9 && (a.y1 - b.y1).abs() < 1e-9
            };
            let band = p.tile.inflate(w / 4.0, h / 4.0);
            for b in &bounds {
                assert!(b.width() > 0.0 && b.height() > 0.0, "{}: {b:?}", p.name);
                assert!(b.x1 > 0.0 && b.y1 > 0.0 && b.x0 < w && b.y0 < h, "{}: {b:?} does not touch the tile", p.name);
                assert!(b.x0 >= band.x0 && b.y0 >= band.y0 && b.x1 <= band.x1 && b.y1 <= band.y1, "{}: {b:?} is too far out", p.name);
                // The tile past each edge the mark crosses draws this tile's art.
                let (dx, dy) = (f64::from(i8::from(b.x1 > w) - i8::from(b.x0 < 0.0)), f64::from(i8::from(b.y1 > h) - i8::from(b.y0 < 0.0)));
                for (x, y) in [(dx, 0.0), (0.0, dy), (dx, dy)] {
                    let next = p.tile + kurbo::Vec2::new(x * w, y * h);
                    assert!(p.offsets_covering(next).contains(&kurbo::Vec2::ZERO), "{}: {b:?} is cut off at {next:?}", p.name);
                }
                crossing += usize::from(dx != 0.0 || dy != 0.0);
                for (dx, dy) in [(w, 0.0), (-w, 0.0), (0.0, h), (0.0, -h), (w, h), (w, -h), (-w, h), (-w, -h)] {
                    let moved = *b + kurbo::Vec2::new(dx, dy);
                    assert!(!bounds.iter().any(|o| near(o, moved)), "{}: {b:?} is drawn twice by the tiling", p.name);
                }
            }
        }
        assert!(crossing > 0, "no mark reaches past an edge");
    }

    const OPTICAL: [&str; 8] =
        ["Sine Bands", "Ripple Lines", "Interference Rings", "Offset Rings", "Warped Checks", "Pinched Stripes", "Chevron Pulse", "Radiating Lenses"];

    fn optical_doc() -> Document {
        let mut d = Document::new(100.0, 100.0);
        optical_effects(&mut d);
        d
    }

    /// The paths of pattern `p`'s art, each with its fill rule.
    fn paths(p: &PatternDef) -> Vec<(&PathData, vectorcraft_geom::FillRule)> {
        p.art
            .iter()
            .map(|n| match &n.kind {
                vectorcraft_doc::NodeKind::Path { path, rule, .. } => (path, *rule),
                _ => panic!("{}: art that is not a path", p.name),
            })
            .collect()
    }

    #[test]
    fn optical_effects_is_a_pattern_library_of_eight_bounded_tiles() {
        let b = PATTERN_LIBRARIES.iter().find(|b| b.id == "optical-effects").expect("Optical Effects is listed");
        assert_eq!(b.name, "Optical Effects");
        let names: Vec<String> = library_doc(b.id).unwrap().patterns.iter().map(|p| p.name.clone()).collect();
        assert_eq!(names, OPTICAL);
        // Made twice, the art is the same.
        let (a, again) = (optical_doc(), optical_doc());
        assert_eq!(a.patterns, again.patterns);
        for p in &a.patterns {
            let (w, h) = (p.width(), p.height());
            assert!((32.0..=48.0).contains(&w) && w == h && p.tile.x0 == 0.0 && p.tile.y0 == 0.0, "{}", p.name);
            assert_eq!(p.tile_type, TileType::Grid, "{}", p.name);
            let paths = paths(p);
            let segments: usize = paths.iter().flat_map(|(path, _)| &path.subpaths).map(SubPath::segment_count).sum();
            assert!(!paths.is_empty() && paths.len() <= 600 && segments <= 20_000, "{}: {} paths, {segments} segments", p.name, paths.len());
            for (path, _) in &paths {
                let b = path.bounds().unwrap_or_else(|| panic!("{}: an empty path", p.name));
                assert!([b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()) && b.width() > 0.0 && b.height() > 0.0, "{}: {b:?}", p.name);
                assert!(b.x1 > 0.0 && b.y1 > 0.0 && b.x0 < w && b.y0 < h, "{}: {b:?} does not touch the tile", p.name);
            }
        }
    }

    /// Just either side of a tile's right and bottom edges, the tiling paints the same points: a band
    /// cut off at an edge, or one that comes back at another height or width, paints one side only.
    /// No path is another moved by a whole tile, which the tiling would draw twice in one place.
    #[test]
    fn optical_tiles_join_without_a_seam() {
        const SAMPLES: u32 = 400;
        const STEP: f64 = 1e-4;
        for p in &optical_doc().patterns {
            let (w, h) = (p.width(), p.height());
            let shapes: Vec<(BezPathFill, Rect)> =
                paths(p).into_iter().map(|(path, rule)| ((path.to_bezpath(), rule), path.bounds().unwrap_or(Rect::ZERO))).collect();
            let paints = |q: Point| {
                let near = Rect::new(q.x - STEP, q.y - STEP, q.x + STEP, q.y + STEP);
                p.offsets_covering(near).into_iter().any(|o| {
                    let at = q - o;
                    shapes.iter().any(|((bp, rule), b)| {
                        let inside = b.x0 <= at.x && at.x <= b.x1 && b.y0 <= at.y && at.y <= b.y1;
                        let turns = if inside { kurbo::Shape::winding(bp, at) } else { 0 };
                        match rule {
                            vectorcraft_geom::FillRule::EvenOdd => turns % 2 != 0,
                            vectorcraft_geom::FillRule::NonZero => turns != 0,
                        }
                    })
                })
            };
            let (mut differ, mut painted) = (0, 0);
            for i in 0..SAMPLES {
                let t = (f64::from(i) + 0.37) / f64::from(SAMPLES);
                let (right, bottom) = (Point::new(w, t * h), Point::new(t * w, h));
                let (dx, dy) = (kurbo::Vec2::new(STEP, 0.0), kurbo::Vec2::new(0.0, STEP));
                for (a, b) in [(right - dx, right + dx), (bottom - dy, bottom + dy)] {
                    let (pa, pb) = (paints(a), paints(b));
                    differ += usize::from(pa != pb);
                    painted += usize::from(pa);
                }
            }
            assert!(differ <= 2, "{}: {differ} of {} points across the edges differ", p.name, 2 * SAMPLES);
            assert!(painted > 0, "{}: nothing crosses an edge", p.name);
            let bounds: Vec<Rect> = shapes.iter().map(|(_, b)| *b).collect();
            let same = |a: &Rect, b: Rect| {
                (a.x0 - b.x0).abs() < 1e-9 && (a.y0 - b.y0).abs() < 1e-9 && (a.x1 - b.x1).abs() < 1e-9 && (a.y1 - b.y1).abs() < 1e-9
            };
            for b in &bounds {
                for (dx, dy) in [(w, 0.0), (-w, 0.0), (0.0, h), (0.0, -h), (w, h), (w, -h), (-w, h), (-w, -h)] {
                    let moved = *b + kurbo::Vec2::new(dx, dy);
                    assert!(!bounds.iter().any(|o| same(o, moved)), "{}: {b:?} is drawn twice by the tiling", p.name);
                }
            }
        }
    }

    type BezPathFill = (vectorcraft_geom::BezPath, vectorcraft_geom::FillRule);
}
