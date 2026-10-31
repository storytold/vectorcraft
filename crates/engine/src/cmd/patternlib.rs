//! Pattern libraries (Window → Swatch Libraries → Patterns, the Swatches panel's library button):
//! read-only sets of pattern swatches the swatch library panel shows. Each built-in library is a
//! document holding its patterns, drawn here from plain shapes ([`PATTERN_LIBRARIES`]).
//! `swatch.library.add` copies a pattern into the document with its swatch.

use std::sync::{Arc, LazyLock};

use vectorcraft_color::{Color, Paint, Swatch, SwatchLibrary};
use vectorcraft_doc::pattern::pattern_paint;
use vectorcraft_doc::{Appearance, Document, Node, NodeId, PatternDef, TileType};
use vectorcraft_geom::{PathData, Point, Rect, SubPath, shapes};

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
pub const PATTERN_LIBRARIES: &[BuiltinPatternLibrary] =
    &[BuiltinPatternLibrary { id: "geometric-patterns", name: "Geometric Patterns", make: geometric_patterns }];

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

/// The built-in library document that holds pattern `name` (a library panel draws its pattern
/// swatches from it). Pattern names are unique across the built-in libraries.
pub fn pattern_source(name: &str) -> Option<Arc<Document>> {
    PATTERN_LIBRARIES.iter().filter_map(|b| library_doc(b.id)).find(|d| d.pattern(name).is_some())
}

/// Add pattern `name` of library `lib` to `d` with the resources its art uses, as pasting art
/// painted with it does: the same pattern there already is used, and a name another pattern or
/// swatch has gets a number → the pattern's name in `d`.
pub(crate) fn import(d: &mut Document, lib: &Document, name: &str) -> String {
    let user = Node::path(NodeId(0), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::basic(pattern_paint(name), Paint::None, 0.0));
    let imported = Clipboard::gather(lib, vec![user]).import_into(d, &SwatchChoices::default(), false);
    let renamed = imported.renamed.iter().find(|r| r["kind"] == "pattern" && r["from"] == name).and_then(|r| r["to"].as_str());
    renamed.unwrap_or(name).to_string()
}

/// The pattern of `d` that is pattern `name` of library `lib`: the same tile options and art under
/// that name or a numbered copy of it ("Polka Dots 2"), the name itself first.
pub(crate) fn existing_pattern(d: &Document, lib: &Document, name: &str) -> Option<String> {
    let def = lib.pattern(name)?;
    let bare = |p: &PatternDef| PatternDef { name: String::new(), art: vec![], ..p.clone() };
    let copy = |n: &str| n == name || n.strip_prefix(name).and_then(|r| r.strip_prefix(' ')).is_some_and(|r| r.parse::<u32>().is_ok());
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
