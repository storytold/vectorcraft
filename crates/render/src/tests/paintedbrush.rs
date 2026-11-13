//! A path whose live-effect plug-in paints its pieces draws each piece's brushed stroke along that
//! piece, at the piece's stroke width. Culling leaves room for that brush art past each piece, in a
//! symbol instance and a live object too, and Object › Rasterize measures it.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Node, NodeKind, Symbol};
use vectorcraft_geom::{Affine, PathData, Point, SubPath};

use crate::{RenderOptions, Renderer};

const BRUSH: &str = "3 pt. Round";

/// A 20 × 20 square at `x`, 40.
fn square_at(x: f64) -> PathData {
    PathData::single(SubPath::polyline(&[Point::new(x, 40.0), Point::new(x + 20.0, 40.0), Point::new(x + 20.0, 60.0), Point::new(x, 60.0)], true))
}

/// Installs live-effect plug-in `id`, which declares `paintOutput` and returns one piece: the
/// square at 120, 40 with a black stroke 10 pt wide.
fn install(id: &str) {
    use vectorcraft_plugins::wat as tpl;
    let paint = serde_json::to_value(Paint::solid(Color::BLACK)).unwrap();
    let output = json!({"objects": [{"path": square_at(120.0), "strokes": [{"paint": paint, "width": 10.0}]}]}).to_string();
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"effect","api":2,"paintOutput":true,"params":{{}}}}"#);
    let module = wat::parse_str(tpl::returning(&manifest, &output)).unwrap();
    vectorcraft_plugins::registry::install_bytes(&module).unwrap();
}

/// A document whose brush `3 pt. Round` paints a round nib 10 pt across for each point of stroke
/// weight: 100 pt across on a piece's 10 pt stroke, reaching from 70 to 190 across the piece.
fn document() -> Document {
    let mut d = Document::new(300.0, 100.0);
    let mut lib = vectorcraft_brush::library(&d);
    let Some(vectorcraft_brush::BrushKind::Calligraphic(c)) = lib.iter_mut().find(|b| b.name == BRUSH).map(|b| &mut b.kind) else {
        panic!("the {BRUSH} brush")
    };
    c.size = 10.0;
    vectorcraft_brush::store(&mut d, &lib);
    d
}

/// The square at 0, 40 with a black 1 pt stroke, painted by the brush when `brushed`, with
/// live-effect plug-in `plugin` as its own effect, or as its stroke's when `on_stroke`.
fn source(d: &mut Document, plugin: &str, on_stroke: bool, brushed: bool) -> Node {
    let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    let effect = Effect { id: format!("plugin.{plugin}"), params: json!({}), visible: true };
    let stroke = ap.stroke_mut().unwrap();
    stroke.brush = brushed.then(|| BRUSH.to_string());
    if on_stroke {
        stroke.effects = vec![effect];
    } else {
        ap.effects = vec![effect];
    }
    Node::path(d.alloc_id(), square_at(0.0), ap)
}

/// `d` with `n` added to its first layer.
fn with(mut d: Document, n: Node) -> Document {
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

/// Does a new renderer draw `d` dark at (`x` + 5, 50), in a view of 10 × 20 pt from (`x`, 40)?
fn dark_at(d: &Document, x: f64) -> bool {
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let img = Renderer::new().render(d, 10, 20, Affine::translate((-x, -40.0)), &opts);
    let p = img.pixel(5, 10);
    (p[0] as u32 + p[1] as u32 + p[2] as u32) < 200
}

/// The brush art of the piece reaches 50 pt past it, to 190; the room left for the source's 1 pt
/// brushed stroke ends at 169. A view from 175 shows that art whether the plug-in is the path's own
/// effect or its stroke's, and culling leaves `16 × 10 + 8` pt past the piece's 10 pt stroke. A
/// renderer that draws the whole document draws it there too.
#[test]
fn a_painted_piece_is_culled_by_its_own_brush_art() {
    for (id, on_stroke) in [("test.render.paintedbrush.object", false), ("test.render.paintedbrush.stroke", true)] {
        install(id);
        let mut d = document();
        let n = source(&mut d, id, on_stroke, true);
        let d = with(d, n.clone());
        assert!(dark_at(&d, 175.0), "on the stroke: {on_stroke}; the brush art past the source's room is drawn");
        let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let whole = Renderer::new().render(&d, 300, 100, Affine::IDENTITY, &opts).pixel(180, 50);
        assert!((whole[0] as u32 + whole[1] as u32 + whole[2] as u32) < 200, "on the stroke: {on_stroke}; {whole:?}");
        let b = crate::brush_fx::cull_bounds(&d.symbol_boxes(), &n).unwrap();
        assert!(b.x0 <= -52.9 && b.x1 >= 312.9 && b.y0 <= -132.9 && b.y1 >= 232.9, "on the stroke: {on_stroke}; {b:?}");
        vectorcraft_plugins::registry::remove(id);
    }
}

/// Object › Rasterize measures the brush art drawn along the piece, from 70 to 190, and not the
/// brush art of the source, which is not drawn.
#[test]
fn a_painted_piece_s_brush_art_is_in_its_drawn_bounds() {
    const ID: &str = "test.render.paintedbrush.crop";
    install(ID);
    let mut d = document();
    let n = source(&mut d, ID, false, true);
    let d = with(d, n.clone());
    let b = crate::drawn_bounds(&d, &n).unwrap();
    assert!(b.x0 > 65.0 && b.x0 < 75.0 && b.x1 > 185.0 && b.x1 < 195.0, "{b:?}");
    vectorcraft_plugins::registry::remove(ID);
}

/// An instance of a symbol whose art is the path, and a one-by-one grid repeat of it, are drawn
/// where the piece's brush art reaches.
#[test]
fn a_symbol_and_a_repeat_holding_a_painted_path_are_culled_by_its_pieces_brush_art() {
    const ID: &str = "test.render.paintedbrush.held";
    install(ID);
    let mut d = document();
    let art = source(&mut d, ID, false, true);
    d.symbols.push(Symbol { dynamic: false, name: "Painted".into(), art: Arc::new(art) });
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Painted".into(), xf: Affine::IDENTITY });
    let d = with(d, instance);
    assert!(dark_at(&d, 175.0), "an instance");
    let mut d = document();
    let held = source(&mut d, ID, false, true);
    let kind = vectorcraft_doc::RepeatKind::Grid {
        h_spacing: 0.0,
        v_spacing: 0.0,
        rows: 1,
        cols: 1,
        grid_type: vectorcraft_doc::TileType::Grid,
        flip_rows: false,
        flip_cols: false,
    };
    let repeat = Node::new(d.alloc_id(), NodeKind::Repeat(vectorcraft_doc::RepeatSpec { kind, source: vec![Arc::new(held)] }));
    let d = with(d, repeat);
    assert!(dark_at(&d, 175.0), "a repeat");
    vectorcraft_plugins::registry::remove(ID);
}

/// The room an instance leaves for the pieces' brush art is measured once for a symbol's art: the
/// second instance measured with the same boxes measures no art.
#[test]
fn the_pieces_brush_width_is_measured_once_for_a_symbol() {
    const ID: &str = "test.render.paintedbrush.once";
    install(ID);
    let mut d = document();
    let art = source(&mut d, ID, false, true);
    d.symbols.push(Symbol { dynamic: false, name: "Painted".into(), art: Arc::new(art) });
    let instance =
        |d: &mut Document, x: f64| Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Painted".into(), xf: Affine::translate((x, 0.0)) });
    let (a, b) = (instance(&mut d, 0.0), instance(&mut d, 30.0));
    let symbols = d.symbol_boxes();
    assert_eq!(crate::brush_fx::pad(&symbols, &a), 168.0, "16 × the piece's 10 pt + 8");
    let measured = symbols.art_measured();
    assert_eq!(crate::brush_fx::pad(&symbols, &b), 168.0);
    assert_eq!(symbols.art_measured(), measured);
    vectorcraft_plugins::registry::remove(ID);
}

/// The square at 0, 40 with a black 1 pt stroke painted by the brush, without effects.
fn brushed_path(d: &mut Document) -> Node {
    let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    ap.stroke_mut().unwrap().brush = Some(BRUSH.to_string());
    Node::path(d.alloc_id(), square_at(0.0), ap)
}

/// A group leaves the room each member's brushed strokes leave for brush art, as the renderer
/// measures the group by its members: a plain brushed path (`16 × 1 + 8` pt past its 1 pt stroke),
/// a painted path (the room past its piece), and the brushed path in a group with an effect of its
/// own.
#[test]
fn a_group_is_culled_by_its_members_brush_art() {
    const ID: &str = "test.render.paintedbrush.group";
    install(ID);
    for painted in [false, true] {
        let mut d = document();
        let member = if painted { source(&mut d, ID, false, true) } else { brushed_path(&mut d) };
        let g = Node::group(d.alloc_id(), vec![Arc::new(member.clone())]);
        let d = with(d, g);
        let g = d.layers[0].children().unwrap()[0].clone();
        let symbols = d.symbol_boxes();
        let own = crate::brush_fx::cull_bounds(&symbols, &member);
        assert!(own.is_some_and(|b| b.x0 < -24.0), "painted: {painted}; {own:?}");
        assert_eq!(crate::brush_fx::cull_bounds(&symbols, &g), own, "painted: {painted}");
        assert_eq!(crate::painted_bounds(&d, &g), own, "painted: {painted}");
        assert_eq!(Renderer::new().bounds_of(&symbols, &g), own, "painted: {painted}");
    }
    vectorcraft_plugins::registry::remove(ID);
    // A 4 pt stroke leaves `16 × 4 + 8` pt, past the crop marks. An empty effect id stands for a
    // stroke of the group's own.
    for effect in ["adjust.hueSaturation", "cropMarks", ""] {
        let mut d = document();
        let mut member = brushed_path(&mut d);
        member.appearance.stroke_mut().unwrap().width = 4.0;
        let mut g = Node::group(d.alloc_id(), vec![Arc::new(member.clone())]);
        if effect.is_empty() {
            g.appearance = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0).into();
        } else {
            g.appearance.effects = vec![Effect { id: effect.into(), params: json!({}), visible: true }];
        }
        let d = with(d, g);
        let g = d.layers[0].children().unwrap()[0].clone();
        assert!(crate::fx::has_object_fx(&g), "{effect}: an effect of the group's own");
        let own = crate::brush_fx::cull_bounds(&d.symbol_boxes(), &member).unwrap();
        assert!(own.x0 <= -73.9, "{own:?}");
        let b = crate::painted_bounds(&d, &g);
        assert!(b.is_some_and(|b| b.union(own) == b), "{effect}: {b:?} holds {own:?}");
    }
}

/// A layer leaves the room its members' brushed strokes leave for brush art.
#[test]
fn a_layer_is_culled_by_its_members_brush_art() {
    let mut d = document();
    let member = brushed_path(&mut d);
    let d = with(d, member.clone());
    let layer = d.layers[0].clone();
    let symbols = d.symbol_boxes();
    let own = crate::brush_fx::cull_bounds(&symbols, &member);
    assert!(own.is_some_and(|b| b.x0 <= -24.4 && b.x1 >= 44.4), "{own:?}");
    assert_eq!(crate::brush_fx::cull_bounds(&symbols, &layer), own);
    assert_eq!(crate::painted_bounds(&d, &layer), own);
    assert_eq!(Renderer::new().bounds_of(&symbols, &layer), own);
}

/// Pieces without a brushed stroke are culled by the pieces alone, as before.
#[test]
fn painted_pieces_without_a_brush_take_no_room_for_one() {
    const ID: &str = "test.render.paintedbrush.plain";
    install(ID);
    let mut d = document();
    let n = source(&mut d, ID, false, false);
    let d = with(d, n.clone());
    let symbols = d.symbol_boxes();
    let b = crate::brush_fx::cull_bounds(&symbols, &n);
    assert_eq!(b, crate::fx::cull_bounds(&symbols, &n));
    assert!(b.is_some_and(|b| b.x0 > 100.0 && b.x1 < 150.0), "{b:?}");
    vectorcraft_plugins::registry::remove(ID);
}
