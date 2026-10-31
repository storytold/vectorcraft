//! Symbol instances measured by their symbol's art: Align, the Transform panel and the bounding
//! box, Use Preview Bounds, Fit to Artwork Bounds, Rearrange, Convert to Artboards, All on Active
//! Artboard, Transform Each, Lock and Hide › All Artwork Above, EPS export of the art, an
//! artboard's SVG, Export Selection, copy and paste, library graphics and the raster effect images
//! of exports.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Node, NodeId, NodeKind, Symbol};
use vectorcraft_geom::{Affine, Point, Rect, shapes};

use crate::*;

/// The symbol of the SVG import tests: a circle of radius 4 around (5, 5) with a 2 pt stroke, and
/// an instance of it at x 80.
const DOT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><defs><symbol id="Dot"><circle cx="5" cy="5" r="4" stroke="#000" stroke-width="2"/></symbol></defs><use href="#Dot" x="80" fill="red"/></svg>"##;

/// A symbol whose art, a red rectangle from (100, 60) to (140, 80), lies outside the 20 × 20
/// square around the origin, and an instance of it.
const FAR: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><defs><symbol id="Far"><rect x="100" y="60" width="40" height="20" fill="#f00"/></symbol></defs><use href="#Far"/></svg>"##;

/// The same symbol with its instance moved by (-50, -50): the art, (50, 10) to (90, 30), lies on
/// the artboard, and the 20 × 20 square, (-60, -60) to (-40, -40), lies off it.
const FAR_MOVED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><defs><symbol id="Far"><rect x="100" y="60" width="40" height="20" fill="#f00"/></symbol></defs><use href="#Far" x="-50" y="-50"/></svg>"##;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// `svg` opened, and its symbol instance.
fn open(svg: &str) -> (Session, NodeId) {
    let mut s = Session::new();
    run(&mut s, "document.open", json!({"name": "symbols.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())}));
    let mut found = None;
    s.doc().unwrap().doc.walk(|n| {
        if matches!(n.kind, NodeKind::SymbolInstance { .. }) {
            found = Some(n.id);
        }
    });
    (s, found.expect("a symbol instance"))
}

fn close(a: Option<Rect>, b: Rect) -> bool {
    a.is_some_and(|a| [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() < 1e-9))
}

#[test]
fn align_left_puts_the_dot_instances_circle_on_the_rectangles_edge() {
    let (mut s, dot) = open(DOT);
    // The Transform panel and the bounding box measure the circle, (81, 1) to (89, 9), not the
    // 20 × 20 square, (70, -10) to (90, 10).
    assert!(close(s.transform_bounds(&[dot]), Rect::new(81.0, 1.0, 89.0, 9.0)), "{:?}", s.transform_bounds(&[dot]));
    assert!(close(s.transform_box(&[dot]).map(|b| b.rect), Rect::new(81.0, 1.0, 89.0, 9.0)));
    run(&mut s, "shape.rectangle", json!({"x": 0, "y": 40, "width": 20, "height": 10}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "object.align", json!({"horizontal": "left"}));
    let NodeKind::SymbolInstance { xf, .. } = &s.doc().unwrap().doc.node(dot).unwrap().kind else { panic!("the instance") };
    // The circle's left edge, 1 pt right of where the instance draws the art's (0, 0), is at x 0.
    assert!(((*xf * Point::ZERO).x + 1.0).abs() < 1e-9, "{xf:?}");
}

#[test]
fn document_inspect_reports_the_instances_art_bounds() {
    let (mut s, dot) = open(DOT);
    let rect = |v: &Value| {
        Some(Rect::new(v["x"].as_f64()?, v["y"].as_f64()?, v["x"].as_f64()? + v["width"].as_f64()?, v["y"].as_f64()? + v["height"].as_f64()?))
    };
    let node = run(&mut s, "document.node", json!({"id": dot.0, "summary": true}));
    assert!(close(rect(&node["bounds"]), Rect::new(81.0, 1.0, 89.0, 9.0)), "{node}");
    run(&mut s, "select.all", json!({}));
    let doc = run(&mut s, "document.inspect", json!({}));
    assert!(close(rect(&doc["selectionBounds"]), Rect::new(81.0, 1.0, 89.0, 9.0)), "{doc}");
    let layer = &doc["layers"][0];
    assert!(close(rect(&layer["children"][0]["bounds"]), Rect::new(81.0, 1.0, 89.0, 9.0)), "{layer}");
}

#[test]
fn use_preview_bounds_takes_in_the_strokes_inside_the_symbol() {
    let (mut s, dot) = open(DOT);
    run(&mut s, "prefs.set", json!({"key": "usePreviewBounds", "value": true}));
    // The 2 pt stroke paints 1 pt around the circle.
    assert!(close(s.transform_bounds(&[dot]), Rect::new(80.0, 0.0, 90.0, 10.0)), "{:?}", s.transform_bounds(&[dot]));
}

#[test]
fn set_bounds_with_use_preview_bounds_keeps_the_strokes_in_the_symbol_at_their_weight() {
    // The circle's visual box, (80, 0) to (90, 10), set to 20 × 20 around its center and from its
    // top-left corner: the stroke still paints 1 pt around the circle, whether strokes scale or not.
    for strokes in [true, false] {
        for (reference, want) in [(4, Rect::new(75.0, -5.0, 95.0, 15.0)), (0, Rect::new(80.0, 0.0, 100.0, 20.0))] {
            let (mut s, dot) = open(DOT);
            run(&mut s, "prefs.set", json!({"key": "usePreviewBounds", "value": true}));
            run(&mut s, "select.set", json!({"ids": [dot.0]}));
            run(&mut s, "object.setBounds", json!({"width": 20, "height": 20, "strokes": strokes, "reference": reference}));
            let b = s.transform_bounds(&[dot]);
            assert!(close(b, want), "strokes {strokes}, reference {reference}: {b:?}");
        }
    }
    // A width of 2 pt, what the stroke adds, or less, with strokes that scale: the circle gets no
    // width, and the visual box is the stroke's 2 pt. The instance does not turn over.
    for width in [2.0, 1.0, 0.0] {
        let (mut s, dot) = open(DOT);
        run(&mut s, "prefs.set", json!({"key": "usePreviewBounds", "value": true}));
        run(&mut s, "select.set", json!({"ids": [dot.0]}));
        run(&mut s, "object.setBounds", json!({"width": width, "strokes": true}));
        let NodeKind::SymbolInstance { xf, .. } = &s.doc().unwrap().doc.node(dot).unwrap().kind else { panic!("the instance") };
        assert_eq!(xf.as_coeffs()[0], 0.0, "width {width}: {xf:?}");
        let b = s.transform_bounds(&[dot]).unwrap();
        assert!((b.width() - 2.0).abs() < 1e-9, "width {width}: {b:?}");
    }
    // An instance with a 2 pt stroke of its own, which scales, around art with a 2 pt stroke, which
    // keeps its weight: doubled, the visual box is twice as wide and high.
    let mut d = Document::new(200.0, 120.0);
    let stroked = Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::BLACK), 2.0);
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 8.0, 8.0)), stroked);
    d.symbols.push(Symbol { name: "Box".into(), art: Arc::new(art) });
    let mut instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Box".into(), xf: Affine::translate((50.0, 50.0)) });
    *instance.appearance = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0);
    let (layer, id) = (d.layers[0].id, instance.id);
    d.insert(Some(layer), 0, instance).unwrap();
    let mut s = Session::new();
    s.add_document(d, None);
    run(&mut s, "prefs.set", json!({"key": "usePreviewBounds", "value": true}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let before = s.transform_bounds(&[id]).unwrap();
    run(&mut s, "object.setBounds", json!({"width": 2.0 * before.width(), "height": 2.0 * before.height(), "strokes": true}));
    let after = s.transform_bounds(&[id]).unwrap();
    assert!((after.width() - 2.0 * before.width()).abs() < 1e-9 && (after.height() - 2.0 * before.height()).abs() < 1e-9, "{before:?} {after:?}");
    // A symbol of an 8 × 8 square with a 2 pt round-joined stroke, and an instance of it at
    // (50, 50) as the one copy of a one-by-one grid repeat. With strokes that scale,
    // `object.setBounds` sets the repeat's visual box, (49, 49) to (59, 59), to 20 × 20 around its
    // center, (44, 44) to (64, 64), and the stroke in the symbol keeps its weight.
    let mut d = Document::new(200.0, 120.0);
    let mut round = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0);
    round.stroke_mut().unwrap().join = vectorcraft_doc::LineJoin::Round;
    let art = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 8.0, 8.0)), round);
    d.symbols.push(Symbol { name: "Ring".into(), art: Arc::new(art) });
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Ring".into(), xf: Affine::translate((50.0, 50.0)) });
    let (layer, id) = (d.layers[0].id, instance.id);
    d.insert(Some(layer), 0, instance).unwrap();
    let mut s = Session::new();
    s.add_document(d, None);
    run(&mut s, "prefs.set", json!({"key": "usePreviewBounds", "value": true}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let repeat = NodeId(run(&mut s, "object.repeat.grid", json!({"rows": 1, "cols": 1}))["id"].as_u64().expect("the repeat"));
    run(&mut s, "select.set", json!({"ids": [repeat.0]}));
    assert!(close(s.transform_bounds(&[repeat]), Rect::new(49.0, 49.0, 59.0, 59.0)), "{:?}", s.transform_bounds(&[repeat]));
    run(&mut s, "object.setBounds", json!({"width": 20, "height": 20, "strokes": true}));
    let b = s.transform_bounds(&[repeat]);
    assert!(close(b, Rect::new(44.0, 44.0, 64.0, 64.0)), "the repeat: {b:?}");
}

#[test]
fn set_bounds_sizes_an_instance_and_a_path_whose_strokes_differ() {
    // An instance at (50, 50) of a 10 × 10 square with a 10 pt stroke, which keeps its weight, and a
    // path of the same square there with a 2 pt stroke, which scales. Setting their visual box,
    // (45, 45) to (65, 65), to 40 × 40 around its center gives (35, 35) to (75, 75): the instance's
    // stroke still sets every edge.
    let mut d = Document::new(200.0, 120.0);
    let square = |d: &mut Document, at: f64, weight: f64| {
        let stroked = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), weight);
        Node::path(d.alloc_id(), shapes::rectangle(Rect::new(at, at, at + 10.0, at + 10.0)), stroked)
    };
    let art = square(&mut d, 0.0, 10.0);
    d.symbols.push(Symbol { name: "Thick".into(), art: Arc::new(art) });
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Thick".into(), xf: Affine::translate((50.0, 50.0)) });
    let path = square(&mut d, 50.0, 2.0);
    let (layer, ids) = (d.layers[0].id, [instance.id, path.id]);
    d.insert(Some(layer), 0, instance).unwrap();
    d.insert(Some(layer), 1, path).unwrap();
    let mut s = Session::new();
    s.add_document(d, None);
    run(&mut s, "prefs.set", json!({"key": "usePreviewBounds", "value": true}));
    run(&mut s, "select.set", json!({"ids": [ids[0].0, ids[1].0]}));
    assert!(close(s.transform_bounds(&ids), Rect::new(45.0, 45.0, 65.0, 65.0)), "{:?}", s.transform_bounds(&ids));
    run(&mut s, "object.setBounds", json!({"width": 40, "height": 40, "strokes": true}));
    let b = s.transform_bounds(&ids);
    assert!(close(b, Rect::new(35.0, 35.0, 75.0, 75.0)), "{b:?}");
}

#[test]
fn set_bounds_measures_the_objects_in_an_instances_art_one_by_one() {
    // A symbol whose art is a group of a rectangle from x 0 to 10 with a 2 pt stroke and one from
    // x 3 to 10 with a 10 pt stroke, both 50 pt high, and an instance of it 100 pt down, in a group
    // when `grouped`, selected with an unstroked rectangle from x 100 to 110 when `plain`. The
    // instance's visual box runs from (-2, 95) to (15, 155). With `own`, the art's group and the
    // instance each have a 1 pt stroke of their own, which reaches 5 pt at its miter limit of 10.
    let selection = |grouped: bool, plain: bool, own: bool| {
        let mut d = Document::new(400.0, 400.0);
        let stroked = |d: &mut Document, x0: f64, weight: f64| {
            Node::path(
                d.alloc_id(),
                shapes::rectangle(Rect::new(x0, 0.0, 10.0, 50.0)),
                Appearance::basic(Paint::None, Paint::solid(Color::BLACK), weight),
            )
        };
        let mut art = Node::group(d.alloc_id(), vec![Arc::new(stroked(&mut d, 0.0, 2.0)), Arc::new(stroked(&mut d, 3.0, 10.0))]);
        let mut instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Mixed".into(), xf: Affine::translate((0.0, 100.0)) });
        if own {
            *art.appearance = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
            *instance.appearance = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
        }
        d.symbols.push(Symbol { name: "Mixed".into(), art: Arc::new(art) });
        let mut roots = vec![if grouped { Node::group(d.alloc_id(), vec![Arc::new(instance)]) } else { instance }];
        if plain {
            let filled = Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0);
            roots.push(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(100.0, 100.0, 110.0, 150.0)), filled));
        }
        let (layer, ids): (NodeId, Vec<NodeId>) = (d.layers[0].id, roots.iter().map(|n| n.id).collect());
        for (i, n) in roots.into_iter().enumerate() {
            d.insert(Some(layer), i, n).unwrap();
        }
        let mut s = Session::new();
        s.add_document(d, None);
        run(&mut s, "prefs.set", json!({"key": "usePreviewBounds", "value": true}));
        run(&mut s, "select.set", json!({"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()}));
        (s, ids)
    };
    // With the unstroked rectangle, the visual box runs from (-2, 95) to (110, 155). Set to 224 wide
    // from its left edge, it is 224 wide and its left edge stays at -2, with strokes kept or
    // scaling, and with the instance in a group. The instance alone set to 37 wide is 37 wide. With
    // the strokes of their own, which paint around each rectangle, the visual box runs from
    // (-12, 85) to (110, 165): set to 224 wide, it is 224 wide and its left edge stays at -12.
    let mut wrong = vec![];
    for (grouped, plain, own, width, strokes) in [
        (false, true, false, 224.0, false),
        (false, true, false, 224.0, true),
        (true, true, false, 224.0, false),
        (false, false, false, 37.0, false),
        (false, true, true, 224.0, false),
        (false, true, true, 224.0, true),
    ] {
        let (mut s, ids) = selection(grouped, plain, own);
        let (left, top, bottom) = if own { (-12.0, 85.0, 165.0) } else { (-2.0, 95.0, 155.0) };
        let before = s.transform_bounds(&ids);
        if !close(before, Rect::new(left, top, if plain { 110.0 } else { 15.0 }, bottom)) {
            wrong.push(format!("grouped {grouped}, plain {plain}, own {own}, before: {before:?}"));
        }
        run(&mut s, "object.setBounds", json!({"width": width, "reference": 0, "strokes": strokes}));
        let b = s.transform_bounds(&ids);
        if !close(b, Rect::new(left, top, left + width, bottom)) {
            wrong.push(format!("grouped {grouped}, plain {plain}, own {own}, strokes {strokes}: {b:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

#[test]
fn set_bounds_measures_an_instance_as_one_object_past_4096_objects_in_its_art() {
    // An instance 100 pt down of a symbol whose art is a rectangle from x 0 to 10 with a 2 pt
    // stroke, one from x 3 to 10 with a 10 pt stroke, both 50 pt high, and unstroked 1 × 1 squares
    // inside them: 4,096 objects, then 4,097. Its visual box runs from (-2, 95) to (15, 155). Set to
    // 37 wide from the left edge, it is 37 wide with 4,096 objects. With 4,097 it is measured as one
    // object: its 2 pt margin on the left is measured from its left edge, though the stroke of the
    // thick rectangle at x 3 paints it, so it is 36 wide and its left edge moves to -1.
    for (objects, want) in [(4096, Rect::new(-2.0, 95.0, 35.0, 155.0)), (4097, Rect::new(-1.0, 95.0, 35.0, 155.0))] {
        let mut d = Document::new(400.0, 400.0);
        let stroked = |d: &mut Document, x0: f64, weight: f64| {
            Arc::new(Node::path(
                d.alloc_id(),
                shapes::rectangle(Rect::new(x0, 0.0, 10.0, 50.0)),
                Appearance::basic(Paint::None, Paint::solid(Color::BLACK), weight),
            ))
        };
        let mut art = vec![stroked(&mut d, 0.0, 2.0), stroked(&mut d, 3.0, 10.0)];
        for i in 2..objects {
            let (x, y) = (f64::from(4 + i % 5), f64::from(1 + i % 47));
            let square = Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0);
            art.push(Arc::new(Node::path(d.alloc_id(), shapes::rectangle(Rect::new(x, y, x + 1.0, y + 1.0)), square)));
        }
        let art = Node::group(d.alloc_id(), art);
        d.symbols.push(Symbol { name: "Many".into(), art: Arc::new(art) });
        let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Many".into(), xf: Affine::translate((0.0, 100.0)) });
        let (layer, id) = (d.layers[0].id, instance.id);
        d.insert(Some(layer), 0, instance).unwrap();
        let mut s = Session::new();
        s.add_document(d, None);
        run(&mut s, "prefs.set", json!({"key": "usePreviewBounds", "value": true}));
        run(&mut s, "select.set", json!({"ids": [id.0]}));
        run(&mut s, "object.setBounds", json!({"width": 37, "reference": 0, "strokes": false}));
        let b = s.transform_bounds(&[id]);
        assert!(close(b, want), "{objects} objects: {b:?}");
    }
}

#[test]
fn set_bounds_measures_a_turned_instance_by_the_box_around_its_turned_art_box() {
    // A symbol whose art is a filled circle of radius 10 around the origin, and an instance of it
    // turned 30° at (100, 100) whose bounding box stands square to the page: the box around the
    // art's 20 × 20 box turned 30°, 27.32 wide. Set to 40 wide from its left edge, it is 40 wide.
    let mut d = Document::new(400.0, 400.0);
    let filled = Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0);
    let circle = Node::path(d.alloc_id(), shapes::ellipse(Rect::new(-10.0, -10.0, 10.0, 10.0)), filled);
    d.symbols.push(Symbol { name: "Disc".into(), art: Arc::new(circle) });
    let xf = Affine::translate((100.0, 100.0)) * Affine::rotate(30f64.to_radians());
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Disc".into(), xf });
    let (layer, id) = (d.layers[0].id, instance.id);
    d.insert(Some(layer), 0, instance).unwrap();
    let mut s = Session::new();
    s.add_document(d, None);
    run(&mut s, "prefs.set", json!({"key": "usePreviewBounds", "value": true}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let before = s.transform_bounds(&[id]).unwrap();
    let turned = 20.0 * (30f64.to_radians().cos() + 30f64.to_radians().sin());
    assert!((before.width() - turned).abs() < 1e-9, "{before:?}");
    run(&mut s, "object.setBounds", json!({"width": 40, "reference": 0}));
    let after = s.transform_bounds(&[id]).unwrap();
    assert!((after.width() - 40.0).abs() < 1e-9 && (after.x0 - before.x0).abs() < 1e-9, "{before:?} {after:?}");
}

#[test]
fn fit_to_artwork_bounds_fits_the_instances_art() {
    let (mut s, _) = open(DOT);
    run(&mut s, "artboard.fitToArt", json!({}));
    let board = s.doc().unwrap().doc.artboards[0].rect;
    assert!(close(Some(board), Rect::new(80.0, 0.0, 90.0, 10.0)), "{board:?}");
}

/// Returns a session whose document has artboard 1 from (0, 0) to (800, 600), artboard 2 from
/// (1000, 0) to (1400, 300), and an instance of a symbol whose art, a black rectangle from
/// (1100, 50) to (1140, 70), lies far from the symbol's origin, and returns the instance's id. With
/// the instance moved by (100, 100), the art, (1200, 150) to (1240, 170), is on artboard 2, and the
/// 20 × 20 square around the origin, (90, 90) to (110, 110), is on artboard 1.
fn far_on_artboard_2() -> (Session, NodeId) {
    let mut d = Document::new(800.0, 600.0);
    let rect = shapes::rectangle(Rect::new(1100.0, 50.0, 1140.0, 70.0));
    let art = Node::path(d.alloc_id(), rect, Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
    d.symbols.push(Symbol { name: "Far".into(), art: Arc::new(art) });
    let (layer, id) = (d.layers[0].id, d.alloc_id());
    d.insert(Some(layer), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Far".into(), xf: Affine::translate((100.0, 100.0)) })).unwrap();
    let mut s = Session::new();
    s.add_document(d, None);
    run(&mut s, "artboard.new", json!({"x": 1000, "y": 0, "width": 400, "height": 300}));
    (s, id)
}

/// Draws a 10 × 10 square at (`x`, `y`), on top, and returns its id.
fn square(s: &mut Session, x: f64, y: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": y, "width": 10, "height": 10}))["id"].as_u64().expect("the square"))
}

fn bounds(s: &Session, ids: &[NodeId]) -> Option<Rect> {
    s.doc().unwrap().doc.bounds_of(ids, false)
}

#[test]
fn rearrange_moves_an_instance_with_the_artboard_its_art_is_on() {
    let (mut s, far) = far_on_artboard_2();
    run(&mut s, "artboard.rearrange", json!({"columns": 1, "spacing": 50}));
    assert_eq!(s.doc().unwrap().doc.artboards[1].rect, Rect::new(0.0, 650.0, 400.0, 950.0));
    assert!(close(bounds(&s, &[far]), Rect::new(200.0, 800.0, 240.0, 820.0)), "{:?}", bounds(&s, &[far]));
}

#[test]
fn all_on_active_artboard_selects_an_instance_on_the_artboard_its_art_is_on() {
    let (mut s, far) = far_on_artboard_2();
    run(&mut s, "select.allOnArtboard", json!({"artboard": 1}));
    assert_eq!(s.doc().unwrap().selection.objects, [far]);
    run(&mut s, "select.allOnArtboard", json!({"artboard": 0}));
    assert!(s.doc().unwrap().selection.objects.is_empty());
}

#[test]
fn convert_to_artboards_makes_an_artboard_of_the_instances_art() {
    let (mut s, far) = far_on_artboard_2();
    run(&mut s, "select.set", json!({"ids": [far.0]}));
    run(&mut s, "artboard.convertToArtboards", json!({}));
    let board = s.doc().unwrap().doc.artboards[2].rect;
    assert!(close(Some(board), Rect::new(1200.0, 150.0, 1240.0, 170.0)), "{board:?}");
}

#[test]
fn transform_each_scales_an_instance_about_the_center_of_its_art() {
    let (mut s, far) = far_on_artboard_2();
    run(&mut s, "select.set", json!({"ids": [far.0]}));
    run(&mut s, "object.transformEach", json!({"scaleH": 200, "scaleV": 200}));
    assert!(close(bounds(&s, &[far]), Rect::new(1180.0, 140.0, 1260.0, 180.0)), "{:?}", bounds(&s, &[far]));
}

#[test]
fn lock_and_hide_all_artwork_above_measure_the_instances_art() {
    // Above the selected instance, the square on its art is locked and the square on the 20 × 20
    // square is not.
    let (mut s, far) = far_on_artboard_2();
    let (on_art, on_square) = (square(&mut s, 1205.0, 155.0), square(&mut s, 95.0, 95.0));
    run(&mut s, "select.set", json!({"ids": [far.0]}));
    run(&mut s, "object.lock.above", json!({}));
    let node = |s: &Session, id: NodeId| s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(node(&s, on_art).locked && !node(&s, on_square).locked);
    // Brought in front of both squares, the instance is hidden above the square on its art only.
    let (mut s, far) = far_on_artboard_2();
    let (on_art, on_square) = (square(&mut s, 1205.0, 155.0), square(&mut s, 95.0, 95.0));
    run(&mut s, "select.set", json!({"ids": [far.0]}));
    run(&mut s, "object.arrange.bringToFront", json!({}));
    run(&mut s, "select.set", json!({"ids": [on_square.0]}));
    run(&mut s, "object.hide.above", json!({}));
    assert!(node(&s, far).visible, "the instance's art does not overlap the selected square");
    run(&mut s, "select.set", json!({"ids": [on_art.0]}));
    run(&mut s, "object.hide.above", json!({}));
    assert!(!node(&s, far).visible);
}

#[test]
fn eps_of_the_art_bounds_and_writes_an_instances_art() {
    let (mut s, _) = open(FAR);
    let r = run(&mut s, "document.export", json!({"format": "eps", "previewFormat": "none", "cmykPostScript": false}));
    let bytes = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
    let ps = String::from_utf8(vectorcraft_eps::sections(&bytes).unwrap().0.to_vec()).unwrap();
    let bbox = ps.lines().find_map(|l| l.strip_prefix("%%BoundingBox:")).map(str::trim);
    // The art, 60 to 80 down from the artboard's top, is 40 to 60 up from its bottom (120 high).
    assert_eq!(bbox, Some("100 40 140 60"), "{ps}");
    assert!(ps.contains("1 0 0 rg"), "the art is written: {ps}");
}

#[test]
fn an_artboards_svg_keeps_an_instance_whose_art_is_on_it() {
    let (mut s, _) = open(FAR_MOVED);
    let svg = run(&mut s, "document.serialize", json!({"format": "svg", "artboard": 0}));
    let text = svg["text"].as_str().unwrap();
    assert!(text.contains("<use"), "{text}");
}

#[test]
fn an_artboards_svg_keeps_an_instance_whose_brush_art_is_on_it() {
    // The symbol's art: a line from (0, 40) to (0, 60) with a 1 pt stroke painted by a 20 pt round
    // nib. Drawn from (-5, 0), the line lies left of the artboard, and the nib paints 10 pt either
    // side of it, onto the artboard.
    let mut d = Document::new(200.0, 120.0);
    let mut lib = vectorcraft_brush::library(&d);
    let Some(vectorcraft_brush::BrushKind::Calligraphic(c)) = lib.iter_mut().find(|b| b.name == "3 pt. Round").map(|b| &mut b.kind) else {
        panic!("the 3 pt. Round brush")
    };
    c.size = 20.0;
    vectorcraft_brush::store(&mut d, &lib);
    let mut stroke = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0);
    stroke.stroke_mut().unwrap().brush = Some("3 pt. Round".into());
    let line = Node::path(d.alloc_id(), shapes::line((0.0, 40.0).into(), (0.0, 60.0).into()), stroke);
    d.symbols.push(Symbol { name: "Nib".into(), art: Arc::new(line) });
    let (layer, id) = (d.layers[0].id, d.alloc_id());
    d.insert(Some(layer), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Nib".into(), xf: Affine::translate((-5.0, 0.0)) })).unwrap();
    let mut s = Session::new();
    s.add_document(d, None);
    let svg = run(&mut s, "document.serialize", json!({"format": "svg", "artboard": 0}));
    let text = svg["text"].as_str().unwrap();
    assert!(text.contains("<use"), "{text}");
}

#[test]
fn an_artboards_svg_keeps_an_instance_whose_shadow_is_on_it() {
    // The symbol's art, a black square from (-1, -1) to (1, 1), drawn from (-5, 20): left of the
    // artboard. A drop shadow 8 pt to the right, on the art or on the instance, reaches onto it.
    for in_art in [true, false] {
        let mut d = Document::new(200.0, 120.0);
        let shadow = Effect { id: "stylize.dropShadow".into(), params: json!({"x": 8, "y": 0, "blur": 0}), visible: true };
        let square = shapes::rectangle(Rect::new(-1.0, -1.0, 1.0, 1.0));
        let mut art = Node::path(d.alloc_id(), square, Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0));
        let mut instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Dot".into(), xf: Affine::translate((-5.0, 20.0)) });
        if in_art {
            art.appearance.effects.push(shadow);
        } else {
            instance.appearance.effects.push(shadow);
        }
        d.symbols.push(Symbol { name: "Dot".into(), art: Arc::new(art) });
        let layer = d.layers[0].id;
        d.insert(Some(layer), 0, instance).unwrap();
        let mut s = Session::new();
        s.add_document(d, None);
        let svg = run(&mut s, "document.serialize", json!({"format": "svg", "artboard": 0}));
        let text = svg["text"].as_str().unwrap();
        assert!(text.contains("<use"), "the shadow on the art: {in_art}; {text}");
    }
}

#[test]
fn export_selection_crops_to_the_instances_art() {
    let (mut s, _) = open(FAR);
    run(&mut s, "select.all", json!({}));
    let v = run(&mut s, "document.exportSelection", json!({"format": "svg"}));
    let bounds: Vec<f64> = v["bounds"].as_array().unwrap().iter().filter_map(Value::as_f64).collect();
    // x, y, width and height: the art, not the 20 × 20 square from (-10, -10).
    assert_eq!(bounds, [100.0, 60.0, 40.0, 20.0], "{v}");
}

fn ids(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().filter_map(Value::as_u64).map(NodeId).collect()
}

#[test]
fn a_copied_instance_comes_from_the_artboard_its_art_is_on() {
    let (mut s, far) = far_on_artboard_2();
    run(&mut s, "select.set", json!({"ids": [far.0]}));
    run(&mut s, "edit.copy", json!({}));
    assert_eq!(s.clipboard.source_artboard, Some(Rect::new(1000.0, 0.0, 1400.0, 300.0)));
    // Pasted in place onto artboard 2, it lands where it was.
    let pasted = ids(&run(&mut s, "edit.pasteInPlace", json!({"artboard": 1})));
    assert!(close(bounds(&s, &pasted), Rect::new(1200.0, 150.0, 1240.0, 170.0)), "{:?}", bounds(&s, &pasted));
}

#[test]
fn a_copied_instances_png_is_cropped_to_its_art() {
    let (mut s, far) = far_on_artboard_2();
    run(&mut s, "select.set", json!({"ids": [far.0]}));
    run(&mut s, "edit.copy", json!({}));
    let r = run(&mut s, "clipboard.exportPng", json!({}));
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(40), Some(20)));
    let png = vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap();
    let img = image::load_from_memory(&png).unwrap().to_rgba8();
    assert_eq!(img.get_pixel(20, 10).0, [0, 0, 0, 255], "the art fills the picture");
}

#[test]
fn an_instance_pasted_at_a_point_has_its_art_centered_there() {
    let (mut s, far) = far_on_artboard_2();
    run(&mut s, "select.set", json!({"ids": [far.0]}));
    run(&mut s, "edit.copy", json!({}));
    let pasted = ids(&run(&mut s, "edit.paste", json!({"center": [300, 200]})));
    assert!(close(bounds(&s, &pasted), Rect::new(280.0, 190.0, 320.0, 210.0)), "{:?}", bounds(&s, &pasted));
}

#[test]
fn svg_markup_on_the_clipboard_is_centered_by_the_instances_art() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    run(&mut s, "clipboard.importSvg", json!({"dataBase64": vectorcraft_format::base64_encode(FAR.as_bytes()), "center": [200, 150]}));
    let pasted = ids(&run(&mut s, "edit.pasteInPlace", json!({})));
    assert!(close(bounds(&s, &pasted), Rect::new(180.0, 140.0, 220.0, 160.0)), "{:?}", bounds(&s, &pasted));
}

#[test]
fn a_library_graphic_of_an_instance_is_measured_by_its_art() {
    let (mut s, far) = far_on_artboard_2();
    run(&mut s, "select.set", json!({"ids": [far.0]}));
    let added = run(&mut s, "library.add", json!({"kind": "graphic"}));
    let (lib, item) = (added["library"].clone(), added["id"].clone());
    let g = &run(&mut s, "library.get", json!({"library": lib}))["graphics"][0];
    assert_eq!((g["width"].as_f64(), g["height"].as_f64()), (Some(40.0), Some(20.0)), "{g}");
    run(&mut s, "file.new", json!({"width": 300, "height": 300}));
    let placed = ids(&run(&mut s, "library.use", json!({"library": lib, "kind": "graphic", "item": item, "center": [150, 100]})));
    assert!(close(bounds(&s, &placed), Rect::new(130.0, 90.0, 170.0, 110.0)), "{:?}", bounds(&s, &placed));
}

#[test]
fn a_groups_shadow_image_covers_the_art_of_the_instance_in_it() {
    let (mut s, far) = open(FAR);
    run(&mut s, "select.set", json!({"ids": [far.0]}));
    let group = run(&mut s, "object.group", json!({}))["id"].as_u64().expect("the group");
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "ids": [group]}));
    let flat = crate::flatten_raster_effects(&s.doc().unwrap().doc).unwrap();
    let mut frame = None;
    flat.walk(|n| {
        if let NodeKind::Image(im) = &n.kind {
            frame = Some(im.xf.transform_rect_bbox(Rect::new(0.0, 0.0, f64::from(im.width), f64::from(im.height))));
        }
    });
    let frame = frame.expect("the shadow's image");
    assert!(frame.x0 <= 100.0 && frame.y0 <= 60.0 && frame.x1 >= 140.0 && frame.y1 >= 80.0, "{frame:?}");
}
