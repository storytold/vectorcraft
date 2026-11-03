//! Pattern fills are written as tiling patterns whose cells overlap their neighbours a little
//! (#2823): a viewer paints every cell clipped to its own box with anti-aliased edges, so cells
//! that only touch let the paper show along their shared edge, a grid of hairlines.

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, PatternDef};
use vectorcraft_geom::{Affine, Rect, shapes};
use vectorcraft_testkit::raster::render_region;

use crate::*;

fn rect_node(r: Rect, paint: Paint) -> Node {
    Node::path(NodeId(0), shapes::rectangle(r), Appearance::basic(paint, Paint::None, 0.0))
}

/// `d` exported uncompressed.
fn uncompressed(d: &Document) -> String {
    String::from_utf8_lossy(&export(d, &PdfOptions::uncompressed()).expect("export")).into_owned()
}

/// `d` exported and imported back.
fn roundtrip(d: &Document) -> Document {
    import(&export(d, &PdfOptions::default()).expect("export")).expect("import")
}

/// A cell of the tiling pattern `/PatternType 1` dictionaries of a PDF write.
#[derive(Debug)]
struct Cell {
    bbox: [f64; 4],
    step: (f64, f64),
}

/// The numbers after `key` in `dict` (an array's, or one).
fn numbers(dict: &str, key: &str) -> Vec<f64> {
    let Some(at) = dict.find(key) else { return vec![] };
    let rest = dict[at + key.len()..].trim_start();
    let value = match rest.strip_prefix('[') {
        Some(array) => array.split(']').next().unwrap_or_default(),
        None => rest.split(['/', '>']).next().unwrap_or_default(),
    };
    value.split_whitespace().filter_map(|n| n.parse().ok()).collect()
}

/// The tiling patterns of an uncompressed PDF.
fn cells(pdf: &str) -> Vec<Cell> {
    pdf.match_indices("/PatternType 1")
        .filter_map(|(at, _)| {
            let dict = pdf[at..].split("stream").next()?;
            let (bbox, x, y) = (numbers(dict, "/BBox"), numbers(dict, "/XStep"), numbers(dict, "/YStep"));
            Some(Cell { bbox: bbox.try_into().ok()?, step: (*x.first()?, *y.first()?) })
        })
        .collect()
}

/// A 100 × 100 pt page filled from (10, 10) to (90, 90) with a 20 × 20 pt two-colour mosaic tile.
fn mosaic() -> Document {
    let mut d = Document::new(100.0, 100.0);
    let red = rect_node(Rect::new(0.0, 0.0, 20.0, 10.0), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    let blue = rect_node(Rect::new(0.0, 10.0, 20.0, 20.0), Paint::solid(Color::rgb(0.0, 0.0, 1.0)));
    d.patterns.push(PatternDef::new("Mosaic", vec![Arc::new(red), Arc::new(blue)]));
    let mut fill = rect_node(Rect::new(10.0, 10.0, 90.0, 90.0), Paint::Pattern { pattern: "Mosaic".into(), xf: Affine::IDENTITY });
    fill.id = d.alloc_id();
    let layer = d.default_layer().expect("layer");
    d.insert(Some(layer), 0, fill).expect("insert");
    d
}

#[test]
fn a_tiling_pattern_cell_reaches_past_its_repeat_on_every_side() {
    let cells = cells(&uncompressed(&mosaic()));
    let [cell] = cells.as_slice() else { panic!("one tiling pattern, got {cells:?}") };
    // The repeat is the tile: a larger step would distort the pattern.
    assert_eq!(cell.step, (20.0, 20.0), "{cell:?}");
    let [x0, y0, x1, y1] = cell.bbox;
    // Overlapping the neighbours by a hairline at least and by a fraction of the tile at most.
    assert!(x0 < -0.1 && y0 < -0.1 && x1 > 20.1 && y1 > 20.1, "the cell must overlap its neighbours: {cell:?}");
    assert!(x0 >= -5.0 && y0 >= -5.0 && x1 <= 25.0 && y1 <= 25.0, "the overlap must stay small: {cell:?}");
    assert!(((-x0) - (x1 - 20.0)).abs() < 1e-3 && ((-y0) - (y1 - 20.0)).abs() < 1e-3, "the overlap must be even: {cell:?}");
}

#[test]
fn a_pattern_fill_read_back_repeats_as_drawn_with_no_paper_along_the_tile_edges() {
    let back = roundtrip(&mosaic());
    let [def] = back.patterns.as_slice() else { panic!("one pattern swatch, got {}", back.patterns.len()) };
    assert_eq!((def.tile.x0, def.tile.y0, def.tile.width(), def.tile.height()), (0.0, 0.0, 20.0, 20.0), "the overlap isn't part of the tile");
    let img = render_region(&back, Rect::new(0.0, 0.0, 100.0, 100.0), 4.0);
    let paper = |x: u32, y: u32| img.over_white(x, y).iter().all(|c| *c > 200);
    // Inside the filled square, across every tile edge (the vertical ones at 20, 40, 60, 80 pt,
    // the horizontal ones at the same places, and the red/blue edge inside each tile).
    for edge in [20u32, 40, 60, 80] {
        for along in (44..=356u32).step_by(3) {
            for across in edge * 4 - 2..=edge * 4 + 2 {
                assert!(!paper(across, along), "paper at x = {across}, y = {along}");
                assert!(!paper(along, across), "paper at x = {along}, y = {across}");
            }
        }
    }
    // The tiles repeat with their own phase: in every tile a red band (its top half) over a blue one.
    for tile in 0..4u32 {
        let x = (15 + 20 * tile) * 4;
        let (red, blue) = ((25 + 20 * tile) * 4, (15 + 20 * tile) * 4);
        assert_eq!(img.over_white(x, red), [255, 0, 0], "red at {x}, {red}");
        assert_eq!(img.over_white(x, blue), [0, 0, 255], "blue at {x}, {blue}");
    }
}

#[test]
fn reading_a_written_pattern_back_and_writing_it_again_adds_no_art() {
    let once = roundtrip(&mosaic());
    let twice = roundtrip(&once);
    let thrice = roundtrip(&twice);
    let (n1, n2, n3) = (once.patterns[0].art.len(), twice.patterns[0].art.len(), thrice.patterns[0].art.len());
    assert!(n1 == n2 && n2 == n3, "a pattern's art must not grow every time it goes through a PDF: {n1}, {n2}, {n3}");
}
