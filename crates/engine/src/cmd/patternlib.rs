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
}
