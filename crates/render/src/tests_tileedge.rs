//! Pattern editing mode: the tile edge in the Tile Edge Color, and Show Swatch Bounds.

use super::*;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::pattern::{PatternDef, PatternEdit, TileType};
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::shapes;

/// A document editing a 20×20 pattern tile of type `tt` (a red 10×10 square in its corner, no
/// preview copies), its tile edge and swatch bounds shown as asked.
fn editing(tt: TileType, edge: bool, bounds: bool) -> Document {
    let mut d = Document::new(200.0, 200.0);
    let id = d.alloc_id();
    let red = Node::path(
        id,
        shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)),
        Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0),
    );
    let mut def = PatternDef::new("Checks", vec![Arc::new(red.clone())]);
    def.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
    (def.tile_type, def.copies, def.show_tile_edge, def.show_swatch_bounds) = (tt, 1, edge, bounds);
    d.patterns.push(def);
    let layer = d.add_layer(Some("Pattern Editing Mode"));
    d.insert(Some(layer), 0, red).unwrap();
    d.pattern_edit = Some(PatternEdit { pattern: "Checks".into(), layer, original: None });
    d
}

/// `d` at 1:1 with the tile's corner at the centre of pixel (50, 50), on white.
fn render(d: &Document, tile_edge: Option<[u8; 3]>) -> Rendered {
    let opts = RenderOptions { background: Some([255; 4]), tile_edge: tile_edge.unwrap_or(RenderOptions::default().tile_edge), ..Default::default() };
    Renderer::new().render(d, 120, 120, Affine::translate((50.5, 50.5)), &opts)
}

fn is_white(p: [u8; 4]) -> bool {
    p[0] > 245 && p[1] > 245 && p[2] > 245
}

#[test]
fn the_tile_edge_takes_the_tile_edge_color() {
    let green = render(&editing(TileType::Grid, true, false), Some([0, 200, 0]));
    // The left edge below the red square, and the bottom edge.
    for (x, y) in [(50, 65), (60, 70), (70, 55)] {
        let p = green.pixel(x, y);
        assert!(p[1] > 150 && p[0] < 100 && p[2] < 100, "({x},{y}) {p:?}");
    }
    assert!(is_white(green.pixel(65, 65)), "inside the tile");
    // The default is the first layer colour (light blue).
    let [r, g, b] = RenderOptions::default().tile_edge;
    let p = render(&editing(TileType::Grid, true, false), None).pixel(50, 65);
    assert!(p[0].abs_diff(r) < 30 && p[1].abs_diff(g) < 30 && p[2].abs_diff(b) < 30, "{p:?}");
    // Hidden: nothing there.
    assert!(is_white(render(&editing(TileType::Grid, false, false), Some([0, 200, 0])).pixel(50, 65)));
}

#[test]
fn swatch_bounds_outline_the_period_dashed() {
    // Brick by row repeats every two rows: the swatch is 20×40 from the tile's corner.
    let tt = TileType::BrickByRow { offset: 0.5 };
    let on = render(&editing(tt, false, true), Some([0, 200, 0]));
    let green = |p: [u8; 4]| p[1] > 150 && p[0] < 100 && p[2] < 100;
    let bottom: Vec<bool> = (51..70).map(|x| green(on.pixel(x, 90))).collect();
    assert!(bottom.iter().any(|g| *g) && bottom.iter().any(|g| !*g), "dashed: {bottom:?}");
    assert!((51..90).any(|y| green(on.pixel(70, y))), "right side");
    assert!(!(51..70).any(|x| green(on.pixel(x, 70))), "the tile edge itself stays hidden");
    // Off: no outline.
    let off = render(&editing(tt, false, false), Some([0, 200, 0]));
    assert!((51..70).all(|x| is_white(off.pixel(x, 90))));
}

/// #3261: exports (which leave templates and guides out) don't draw the tile edge or the swatch
/// bounds, but do draw the tile's art.
#[test]
fn exports_leave_the_tile_edge_and_swatch_bounds_out() {
    let d = editing(TileType::BrickByRow { offset: 0.5 }, true, true);
    let opts = RenderOptions { background: Some([255; 4]), tile_edge: [0, 200, 0], skip_templates: true, ..Default::default() };
    let out = Renderer::new().render(&d, 120, 120, Affine::translate((50.5, 50.5)), &opts);
    assert!(is_white(out.pixel(50, 65)), "tile edge: {:?}", out.pixel(50, 65));
    assert!((51..70).all(|x| is_white(out.pixel(x, 90))), "swatch bounds");
    let p = out.pixel(55, 55);
    assert!(p[0] > 200 && p[1] < 60 && p[2] < 60, "the red square is still drawn: {p:?}");
}

/// #1534: a path made into a guide is drawn in the guide colour and style (Preferences › Guides &
/// Grid), as ruler guides are: a line, or dots with gaps between them.
#[test]
fn guide_paths_take_the_guide_color_and_style() {
    let mut d = Document::new(200.0, 200.0);
    let (layer, id) = (d.layers[0].id, d.alloc_id());
    let mut n = Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 60.0, 60.0)), Appearance::basic(Paint::None, Paint::None, 0.0));
    if let NodeKind::Path { guide, .. } = &mut n.kind {
        *guide = true;
    }
    d.insert(Some(layer), 0, n).unwrap();
    let draw = |guide| {
        let opts = RenderOptions { background: Some([255; 4]), guide, ..Default::default() };
        Renderer::new().render(&d, 120, 120, Affine::translate((50.5, 50.5)), &opts)
    };
    // Along the top edge of the square (y = 50), between its corners: the pixels the red covers.
    let inked = |r: &Rendered| (55..105).filter(|x| r.pixel(*x, 50)[1] < 64).count();
    let line = draw(([200, 0, 0], false));
    let p = line.pixel(80, 50);
    assert!(p[0] > 150 && p[1] < 100 && p[2] < 100, "{p:?}");
    assert_eq!(inked(&line), 50);
    let dotted = draw(([200, 0, 0], true));
    let dots = inked(&dotted);
    assert!((10..40).contains(&dots), "{dots}: {:?}", (45..70).map(|x| dotted.pixel(x, 50)[1]).collect::<Vec<_>>());
    let default = draw(RenderOptions::default().guide);
    assert!((55..105).all(|x| !is_white(default.pixel(x, 50))), "a line by default");
}
