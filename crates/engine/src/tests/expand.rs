//! Object → Expand Appearance: fills and strokes as objects, baked geometry effects, raster
//! effects as images, brushes as art, and the enable rule.

use serde_json::{Value, json};
use vectorcraft_color::BlendMode;
use vectorcraft_doc::{Node, NodeKind};

use crate::tests_menucmds::{place_undecodable_png, warns_undecoded};
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

/// A selected 100×100 square at (50, 50) with a red fill and a 4 pt blue stroke.
fn square(s: &mut Session) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"color": "#ff0000"}));
    run(s, "paint.setStroke", json!({"color": "#0000ff"}));
    run(s, "stroke.set", json!({"weight": 4}));
    id
}

fn render(s: &Session) -> vectorcraft_render::Rendered {
    let doc = s.doc().unwrap().doc.clone();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    vectorcraft_render::Renderer::new().render(&doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts)
}

/// Pixels (of 200×200) whose colour differs by more than `tol` in any channel.
fn differing(a: &vectorcraft_render::Rendered, b: &vectorcraft_render::Rendered, tol: u8) -> usize {
    a.pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.pixels.as_chunks::<4>().0)
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > tol))
        .count()
}

fn expandable(s: &Session) -> bool {
    s.commands().iter().find(|c| c.id == "effect.expandAppearance").unwrap().enabled
}

#[test]
fn symbol_instance_tint_survives_expanding_an_object_effect() {
    let mut s = session();
    square(&mut s);
    let id = NodeId(run(&mut s, "symbol.new", json!({"name": "Tinted Square"}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addFill", json!({"ids": [id.0]}));
    let fill = node(&s, id).appearance.items.len() - 1;
    run(&mut s, "appearance.setItem", json!({"ids": [id.0], "index": fill, "color": "#00ff00", "opacity": 50}));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "item": null, "effect": "path.offsetPath", "params": {"offset": 6}}));
    assert!(matches!(node(&s, id).kind, NodeKind::SymbolInstance { .. }));
    let before = render(&s);
    assert_ne!(before.pixel(100, 100), [255, 255, 255, 255], "the symbol is visible");
    assert_ne!(before.pixel(100, 100), [255, 0, 0, 255], "the instance fill tints the red symbol art");
    assert!(expandable(&s));

    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    assert!(!matches!(n.kind, NodeKind::SymbolInstance { .. }));
    n.walk(&mut |part| assert!(part.appearance.effects.is_empty()));
    let after = render(&s);
    assert!(differing(&before, &after, 8) < 200, "expanding changed {} pixels", differing(&before, &after, 8));
}

/// The canvas tints a symbol's art with the instance fill before the instance's color adjustment,
/// and expanding gives the same colors.
#[test]
fn symbol_instance_tint_comes_before_a_color_adjustment_when_expanded() {
    let mut s = session();
    square(&mut s);
    let id = NodeId(run(&mut s, "symbol.new", json!({"name": "Tinted Square"}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addFill", json!({"ids": [id.0]}));
    let fill = node(&s, id).appearance.items.len() - 1;
    run(&mut s, "appearance.setItem", json!({"ids": [id.0], "index": fill, "color": "#00ff00", "opacity": 50}));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "item": null, "effect": "adjust.hueSaturation", "params": {"hue": 120}}));
    let before = render(&s);
    assert_ne!(before.pixel(100, 100), [255, 255, 255, 255], "the symbol is visible");
    assert!(expandable(&s));

    run(&mut s, "effect.expandAppearance", json!({}));
    assert!(!matches!(node(&s, id).kind, NodeKind::SymbolInstance { .. }));
    let after = render(&s);
    let (b, a) = (before.pixel(100, 100), after.pixel(100, 100));
    assert!(b.iter().zip(a.iter()).all(|(x, y)| x.abs_diff(*y) <= 8), "expanding changed the color from {b:?} to {a:?}");
    assert!(differing(&before, &after, 8) < 200, "expanding changed {} pixels", differing(&before, &after, 8));
}

/// An instance fill with an opacity of 0 does not tint the symbol's art, so expanding keeps the
/// art's global swatch link.
#[test]
fn a_symbol_instance_fill_with_no_opacity_keeps_swatch_links_when_expanded() {
    let mut s = session();
    let sq = square(&mut s);
    run(&mut s, "swatch.new", json!({"name": "Brand", "color": "#2a6fb0", "global": true}));
    run(&mut s, "paint.setFill", json!({"ids": [sq.0], "swatch": "Brand"}));
    let id = NodeId(run(&mut s, "symbol.new", json!({"name": "Linked Square"}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addFill", json!({"ids": [id.0]}));
    let fill = node(&s, id).appearance.items.len() - 1;
    run(&mut s, "appearance.setItem", json!({"ids": [id.0], "index": fill, "color": "#00ff00", "opacity": 0}));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "item": null, "effect": "path.offsetPath", "params": {"offset": 6}}));
    assert!(expandable(&s));

    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    assert!(!matches!(n.kind, NodeKind::SymbolInstance { .. }));
    let mut linked = 0;
    n.walk(&mut |part| {
        if let vectorcraft_color::Paint::Solid { swatch: Some(name), .. } = part.appearance.fill_paint() {
            assert_eq!(name, "Brand");
            linked += 1;
        }
    });
    assert!(linked > 0, "the expanded art keeps the Brand swatch link");
}

/// The square of [`square`] made a symbol, its instance selected, with a new fill (`fill`) or
/// stroke on top that has no paint: it neither stains nor strokes the art. Returns the instance and
/// the new row's index.
fn instance_with_row(s: &mut Session, name: &str, fill: bool) -> (NodeId, usize) {
    square(s);
    let id = NodeId(run(s, "symbol.new", json!({"name": name}))["id"].as_u64().unwrap());
    run(s, if fill { "appearance.addFill" } else { "appearance.addStroke" }, json!({"ids": [id.0]}));
    let row = node(s, id).appearance.items.len() - 1;
    run(s, "appearance.setItem", json!({"ids": [id.0], "index": row, "none": true}));
    (id, row)
}

fn offset_row(s: &mut Session, id: NodeId, row: usize, offset: f64) {
    run(s, "effect.apply", json!({"ids": [id.0], "item": row, "effect": "path.offsetPath", "params": {"offset": offset}}));
}

fn near(p: [u8; 4], q: [u8; 4]) -> bool {
    p.iter().zip(q.iter()).all(|(x, y)| x.abs_diff(*y) <= 24)
}

const WHITE: [u8; 4] = [255, 255, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// No object in `n` keeps an effect of its own or on a fill or stroke.
fn no_effects_left(n: &Node) {
    n.walk(&mut |part| {
        assert!(part.appearance.effects.is_empty(), "{:?} keeps object effects", part.id);
        assert!(part.appearance.items.iter().all(|i| i.effects().is_empty()), "{:?} keeps fill or stroke effects", part.id);
    });
}

/// An effect on a symbol instance's fill reshapes the filled shapes of its art, keeping their
/// colors, and leaves their strokes where they were; Expand Appearance bakes the same look into
/// editable art in one undo step.
#[test]
fn a_symbol_instance_fill_effect_reshapes_its_filled_art_and_expands() {
    let mut s = session();
    let (id, row) = instance_with_row(&mut s, "Square", true);
    assert!(!expandable(&s));
    let plain = render(&s);
    assert!(near(plain.pixel(44, 100), WHITE));
    offset_row(&mut s, id, row, 10.0);
    assert!(expandable(&s), "the fill's effect can be expanded");
    let before = render(&s);
    assert!(near(before.pixel(44, 100), RED), "the art's red fill grows by the offset: {:?}", before.pixel(44, 100));
    assert!(near(before.pixel(50, 100), BLUE), "the blue stroke stays on the square: {:?}", before.pixel(50, 100));

    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    assert!(!matches!(n.kind, NodeKind::SymbolInstance { .. }));
    no_effects_left(&n);
    let mut paints = (0, 0);
    n.walk(&mut |part| {
        if part.appearance.fill().is_some_and(|f| f.paint.color().is_some_and(|c| c.to_hex() == "#ff0000")) {
            paints.0 += 1;
        }
        if part.appearance.stroke().is_some_and(|st| st.paint.color().is_some_and(|c| c.to_hex() == "#0000ff")) {
            paints.1 += 1;
        }
    });
    assert!(paints.0 > 0 && paints.1 > 0, "the art keeps its red fill and blue stroke: {paints:?}");
    let after = render(&s);
    assert!(differing(&before, &after, 8) < 200, "expanding changed {} pixels", differing(&before, &after, 8));

    run(&mut s, "edit.undo", json!({}));
    let n = node(&s, id);
    assert!(matches!(n.kind, NodeKind::SymbolInstance { .. }), "one undo gives the instance back");
    assert!(n.appearance.items[row].effects().iter().any(|e| e.id == "path.offsetPath"));
}

/// Export bakes a symbol instance's fill effect into its art as the canvas draws it.
#[test]
fn export_bakes_a_symbol_instance_fill_effect() {
    let mut s = session();
    let (id, row) = instance_with_row(&mut s, "Square", true);
    offset_row(&mut s, id, row, 10.0);
    let doc = s.doc().unwrap().doc.clone();
    let baked = vectorcraft_render::effects::bake_document(&doc).expect("the instance is baked");
    let n = baked.node(id).expect("the baked art keeps the instance's id").clone();
    assert!(!matches!(n.kind, NodeKind::SymbolInstance { .. }));
    no_effects_left(&n);
    let b = n.geometric_bounds().expect("the baked art has bounds");
    assert!((b.x0 - 40.0).abs() < 1.0 && (b.x1 - 160.0).abs() < 1.0, "the fill grows by the offset: {b:?}");
}

/// An effect on a symbol instance's stroke reshapes the stroked shapes of its art and leaves
/// their fills where they were.
#[test]
fn a_symbol_instance_stroke_effect_reshapes_its_stroked_art_and_expands() {
    let mut s = session();
    let (id, row) = instance_with_row(&mut s, "Square", false);
    assert!(near(render(&s).pixel(40, 100), WHITE));
    offset_row(&mut s, id, row, 10.0);
    assert!(expandable(&s));
    let before = render(&s);
    assert!(near(before.pixel(40, 100), BLUE), "the stroke moves out with the offset: {:?}", before.pixel(40, 100));
    assert!(near(before.pixel(45, 100), WHITE), "the fill keeps its shape: {:?}", before.pixel(45, 100));
    assert!(near(before.pixel(100, 100), RED));

    run(&mut s, "effect.expandAppearance", json!({"ids": [id.0]}));
    let n = node(&s, id);
    assert!(!matches!(n.kind, NodeKind::SymbolInstance { .. }));
    no_effects_left(&n);
    let after = render(&s);
    assert!(differing(&before, &after, 8) < 200, "expanding changed {} pixels", differing(&before, &after, 8));
}

/// A dynamic symbol's instance that paints a stroke of its own on art without one: the instance's
/// stroke effect reshapes that stroke, and its bounds take in where the stroke is drawn.
#[test]
fn a_stroke_effect_reshapes_a_stroke_a_dynamic_instance_adds() {
    let mut s = session();
    square(&mut s);
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let id = NodeId(run(&mut s, "symbol.new", json!({"name": "Art", "type": "dynamic"}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addStroke", json!({"ids": [id.0]}));
    let row = node(&s, id).appearance.items.len() - 1;
    run(&mut s, "appearance.setItem", json!({"ids": [id.0], "index": row, "none": true}));
    let blue = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 1.0));
    s.edit("paint", |d, _| {
        let o = vectorcraft_doc::symbol_overrides::SymbolOverride { path: vec![], stroke: Some(blue), ..Default::default() };
        d.node_mut(id).ok_or(EngineError::NoNode(id))?.extras_mut().symbol_overrides.push(o);
        Ok(())
    })
    .unwrap();
    // The 1 pt stroke's center moves to x = 30.5, so it covers the pixel column from 30 to 31.
    offset_row(&mut s, id, row, 19.5);
    assert!(near(render(&s).pixel(30, 100), BLUE), "the added stroke moves out with the offset: {:?}", render(&s).pixel(30, 100));
    let doc = s.doc().unwrap().doc.clone();
    let b = vectorcraft_render::painted_bounds(&doc, doc.node(id).unwrap()).expect("the instance has bounds");
    assert!(b.x0 <= 31.0 && b.x1 >= 169.0, "the bounds take in the offset stroke: {b:?}");
    let drawn = vectorcraft_render::drawn_bounds(&doc, doc.node(id).unwrap()).expect("the instance is drawn");
    assert!(drawn.x0 <= 31.0 && drawn.x1 >= 169.0, "the drawn bounds take in the offset stroke: {drawn:?}");
    assert_eq!(hit_at(&s, 30.5, vectorcraft_doc::hit::HitOptions { tol: 1.0, ..Default::default() }), Some(id), "a click on the added stroke");
}

/// What a click at (`x`, 100) with `opt` hits.
fn hit_at(s: &Session, x: f64, opt: vectorcraft_doc::hit::HitOptions) -> Option<NodeId> {
    vectorcraft_doc::hit::hit_test(&s.doc().unwrap().doc, vectorcraft_geom::Point::new(x, 100.0), opt).map(|h| h.leaf)
}

/// [`instance_with_row`] of a symbol whose art is an instance of another symbol made of the square.
fn nested_instance_with_row(s: &mut Session) -> (NodeId, usize) {
    square(s);
    let inner = NodeId(run(s, "symbol.new", json!({"name": "Inner"}))["id"].as_u64().unwrap());
    run(s, "select.set", json!({"ids": [inner.0]}));
    let id = NodeId(run(s, "symbol.new", json!({"name": "Outer"}))["id"].as_u64().unwrap());
    run(s, "appearance.addFill", json!({"ids": [id.0]}));
    let row = node(s, id).appearance.items.len() - 1;
    run(s, "appearance.setItem", json!({"ids": [id.0], "index": row, "none": true}));
    (id, row)
}

/// Effect › Pathfinder › Add on a group holding a symbol instance combines the art the instance
/// draws: the effects on the instance's fill and stroke reshape the result as they reshape the
/// instance, in a symbol placed inside another symbol too.
#[test]
fn pathfinder_takes_a_symbol_instance_fill_and_stroke_effects() {
    for (fill, x, color) in [(true, 44, RED), (false, 40, BLUE)] {
        let mut s = session();
        let (id, row) = instance_with_row(&mut s, "Square", fill);
        offset_row(&mut s, id, row, 10.0);
        let g = NodeId(run(&mut s, "object.group", json!({"ids": [id.0]}))["id"].as_u64().unwrap());
        run(&mut s, "effect.apply", json!({"ids": [g.0], "effect": "pathfinder.add"}));
        let r = render(&s);
        assert!(near(r.pixel(x, 100), color), "fill {fill}: the result keeps the offset: {:?}", r.pixel(x, 100));
        assert!(near(r.pixel(100, 100), RED), "fill {fill}: the result keeps the art's fill: {:?}", r.pixel(100, 100));
    }
    // An effect of the instance's own reshapes the art as well.
    let mut s = session();
    let (id, row) = instance_with_row(&mut s, "Square", true);
    offset_row(&mut s, id, row, 10.0);
    run(&mut s, "effect.apply", json!({"ids": [id.0], "item": null, "effect": "path.offsetPath", "params": {"offset": 5}}));
    assert!(near(render(&s).pixel(37, 100), RED), "both offsets grow the art on the canvas: {:?}", render(&s).pixel(37, 100));
    let g = NodeId(run(&mut s, "object.group", json!({"ids": [id.0]}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [g.0], "effect": "pathfinder.add"}));
    assert!(near(render(&s).pixel(37, 100), RED), "both offsets grow the result: {:?}", render(&s).pixel(37, 100));

    let mut s = session();
    let (id, row) = nested_instance_with_row(&mut s);
    offset_row(&mut s, id, row, 10.0);
    assert!(near(render(&s).pixel(44, 100), RED), "the nested art grows on the canvas: {:?}", render(&s).pixel(44, 100));
    let g = NodeId(run(&mut s, "object.group", json!({"ids": [id.0]}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [g.0], "effect": "pathfinder.add"}));
    assert!(near(render(&s).pixel(44, 100), RED), "the nested art grows in the result: {:?}", render(&s).pixel(44, 100));
}

/// A click where the effect on a symbol instance's fill or stroke paints its art outside the
/// instance's bounds selects the instance; a click past the drawn art, a fill with Selection by
/// Path Only and outline mode do not.
#[test]
fn a_click_where_a_symbol_instance_fill_or_stroke_effect_paints_selects_it() {
    use vectorcraft_doc::hit::HitOptions;
    let opt = HitOptions { tol: 1.0, ..Default::default() };
    let mut s = session();
    let (id, row) = instance_with_row(&mut s, "Square", true);
    assert_eq!(hit_at(&s, 44.0, opt), None);
    offset_row(&mut s, id, row, 10.0);
    assert_eq!(hit_at(&s, 44.0, opt), Some(id), "a click on the grown fill");
    assert_eq!(hit_at(&s, 100.0, opt), Some(id), "a click inside the instance");
    assert_eq!(hit_at(&s, 36.0, opt), None, "a click past the grown fill");
    assert_eq!(hit_at(&s, 44.0, HitOptions { path_only: true, ..opt }), None, "Selection by Path Only leaves the fill out");
    assert_eq!(hit_at(&s, 44.0, HitOptions { outline: true, ..opt }), None, "outline mode draws the art as it is");

    let mut s = session();
    let (id, row) = instance_with_row(&mut s, "Square", false);
    offset_row(&mut s, id, row, 10.0);
    assert_eq!(hit_at(&s, 40.0, opt), Some(id), "a click on the moved stroke");
    assert_eq!(hit_at(&s, 45.0, opt), None, "a click between the moved stroke and the fill");
    assert_eq!(hit_at(&s, 40.0, HitOptions { path_only: true, ..opt }), Some(id), "Selection by Path Only takes the stroke");

    let mut s = session();
    let (id, row) = nested_instance_with_row(&mut s);
    offset_row(&mut s, id, row, 10.0);
    assert_eq!(hit_at(&s, 44.0, opt), Some(id), "a click on the grown fill of the nested art");

    // A path in the art with an Offset Path of its own: both offsets grow what it draws.
    let mut s = session();
    let art = square(&mut s);
    run(&mut s, "effect.apply", json!({"ids": [art.0], "item": null, "effect": "path.offsetPath", "params": {"offset": 5}}));
    let id = NodeId(run(&mut s, "symbol.new", json!({"name": "Offset"}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addFill", json!({"ids": [id.0]}));
    let row = node(&s, id).appearance.items.len() - 1;
    run(&mut s, "appearance.setItem", json!({"ids": [id.0], "index": row, "none": true}));
    offset_row(&mut s, id, row, 10.0);
    assert!(near(render(&s).pixel(37, 100), RED), "both offsets grow the art on the canvas: {:?}", render(&s).pixel(37, 100));
    assert_eq!(hit_at(&s, 37.0, opt), Some(id), "a click where both offsets grow the art");
}

/// A group in the symbol's art with a fill of its own: the instance's fill effect reshapes what
/// that fill paints, and the instance's bounds take it in.
#[test]
fn a_fill_effect_reshapes_the_fill_of_a_group_in_the_art() {
    let mut s = session();
    square(&mut s);
    run(&mut s, "paint.setFill", json!({"none": true}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addFill", json!({"ids": [g.0]}));
    run(&mut s, "paint.setFill", json!({"ids": [g.0], "item": 0, "color": "#ff0000"}));
    let id = NodeId(run(&mut s, "symbol.new", json!({"name": "Group"}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addFill", json!({"ids": [id.0]}));
    let row = node(&s, id).appearance.items.len() - 1;
    run(&mut s, "appearance.setItem", json!({"ids": [id.0], "index": row, "none": true}));
    offset_row(&mut s, id, row, 20.0);
    assert!(near(render(&s).pixel(35, 100), RED), "the group's fill grows by the offset: {:?}", render(&s).pixel(35, 100));
    let doc = s.doc().unwrap().doc.clone();
    let b = vectorcraft_render::painted_bounds(&doc, doc.node(id).unwrap()).expect("the instance has bounds");
    assert!(b.x0 <= 31.0 && b.x1 >= 169.0, "the bounds take in the group's grown fill: {b:?}");
    let drawn = vectorcraft_render::drawn_bounds(&doc, doc.node(id).unwrap()).expect("the instance is drawn");
    assert!(drawn.x0 <= 31.0 && drawn.x1 >= 169.0, "the drawn bounds take in the group's grown fill: {drawn:?}");
    assert_eq!(hit_at(&s, 35.0, vectorcraft_doc::hit::HitOptions { tol: 1.0, ..Default::default() }), Some(id), "a click on the group's grown fill");

    let before = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    no_effects_left(&node(&s, id));
    let after = render(&s);
    assert!(differing(&before, &after, 8) < 200, "expanding changed {} pixels", differing(&before, &after, 8));
}

/// An image in the symbol's art whose fill Convert to Shape gives a shape: the instance's fill
/// effect grows what that fill paints, and a click there selects the instance.
#[test]
fn a_click_on_the_grown_fill_of_an_image_in_the_art_selects_the_instance() {
    let mut s = session();
    let mut png = vec![];
    image::RgbaImage::from_pixel(30, 30, image::Rgba([255, 0, 0, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    // The frame runs from 85 to 115; the fill's shape is 9 pt larger on each side.
    let r = run(&mut s, "file.place", json!({"name": "red.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [100, 100]}));
    let im = NodeId(r["ids"][0].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [im.0]}));
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "paint.setFill", json!({"color": "#0000ff", "item": 0}));
    run(&mut s, "effect.apply", json!({"effect": "convertToShape.rectangle", "item": 0}));
    let id = NodeId(run(&mut s, "symbol.new", json!({"name": "Image"}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addFill", json!({"ids": [id.0]}));
    let row = node(&s, id).appearance.items.len() - 1;
    run(&mut s, "appearance.setItem", json!({"ids": [id.0], "index": row, "none": true}));
    let opt = vectorcraft_doc::hit::HitOptions { tol: 1.0, ..Default::default() };
    assert_eq!(hit_at(&s, 70.0, opt), None);
    offset_row(&mut s, id, row, 10.0);
    assert!(near(render(&s).pixel(70, 100), BLUE), "the image's fill grows by the offset: {:?}", render(&s).pixel(70, 100));
    assert_eq!(hit_at(&s, 70.0, opt), Some(id), "a click on the image's grown fill");
    assert_eq!(hit_at(&s, 60.0, opt), None, "a click past the grown fill");
}

/// Of several fills, only the topmost one's effects reach the art, and only while it is visible.
#[test]
fn only_the_topmost_visible_instance_fill_reshapes_the_art() {
    let mut s = session();
    let (id, lower) = instance_with_row(&mut s, "Square", true);
    offset_row(&mut s, id, lower, 30.0);
    run(&mut s, "appearance.addFill", json!({"ids": [id.0]}));
    let top = node(&s, id).appearance.items.len() - 1;
    run(&mut s, "appearance.setItem", json!({"ids": [id.0], "index": top, "none": true}));
    assert!(!expandable(&s), "a lower fill's effect does not reach the art");
    offset_row(&mut s, id, top, 10.0);
    let both = render(&s);
    assert!(near(both.pixel(44, 100), RED), "the top fill's offset shows");
    assert!(near(both.pixel(25, 100), WHITE), "the lower fill's offset does not: {:?}", both.pixel(25, 100));

    run(&mut s, "appearance.setItem", json!({"ids": [id.0], "index": top, "visible": false}));
    assert!(near(render(&s).pixel(44, 100), WHITE), "a hidden top fill reshapes nothing");
    assert!(!expandable(&s));

    run(&mut s, "appearance.setItem", json!({"ids": [id.0], "index": top, "visible": true}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let after = render(&s);
    assert!(differing(&both, &after, 8) < 200, "expanding changed {} pixels", differing(&both, &after, 8));
    let n = node(&s, id);
    no_effects_left(&n);
}

/// An instance inside a symbol's art passes the outer instance's fill effect on to its own art.
#[test]
fn a_nested_symbol_instance_takes_the_outer_fill_effect() {
    let mut s = session();
    square(&mut s);
    run(&mut s, "symbol.new", json!({"name": "Inner"}));
    let id = NodeId(run(&mut s, "symbol.new", json!({"name": "Outer"}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addFill", json!({"ids": [id.0]}));
    let row = node(&s, id).appearance.items.len() - 1;
    run(&mut s, "appearance.setItem", json!({"ids": [id.0], "index": row, "none": true}));
    offset_row(&mut s, id, row, 40.0);
    let before = render(&s);
    assert!(near(before.pixel(20, 100), RED), "the inner art grows by the offset: {:?}", before.pixel(20, 100));
    assert!(expandable(&s));
    // A view that shows only the grown part, right of the square: the instance is still drawn.
    let doc = s.doc().unwrap().doc.clone();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let right = vectorcraft_render::Renderer::new().render(&doc, 200, 200, vectorcraft_geom::Affine::translate((-155.0, 0.0)), &opts);
    assert!(near(right.pixel(20, 100), RED), "the grown art past the square's box is drawn: {:?}", right.pixel(20, 100));
    let b = vectorcraft_render::painted_bounds(&doc, doc.node(id).unwrap()).expect("the instance has bounds");
    assert!(b.x0 <= 11.0 && b.x1 >= 189.0, "the bounds take in the inner art's offset: {b:?}");

    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    assert!(!matches!(n.kind, NodeKind::SymbolInstance { .. }));
    n.walk(&mut |part| assert!(!matches!(part.kind, NodeKind::SymbolInstance { .. }), "the inner instance is expanded too"));
    no_effects_left(&n);
    let after = render(&s);
    assert!(differing(&before, &after, 8) < 200, "expanding changed {} pixels", differing(&before, &after, 8));
}

/// A symbol whose art holds an instance of itself draws, exports and expands without running away.
/// The engine keeps no such document after an edit, so Expand Appearance refuses it and the
/// instance stays as it was.
#[test]
fn a_fill_effect_on_an_instance_of_a_symbol_that_holds_itself_ends() {
    let mut s = session();
    let (id, row) = instance_with_row(&mut s, "Square", true);
    offset_row(&mut s, id, row, 4.0);
    {
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        let inst = d.node(id).unwrap().clone();
        let sym = d.symbols.iter_mut().find(|sy| sy.name == "Square").unwrap();
        sym.art = std::sync::Arc::new(Node::group(NodeId(90_000), vec![sym.art.clone(), std::sync::Arc::new(inst)]));
    }
    let doc = s.doc().unwrap().doc.clone();
    assert!(vectorcraft_render::effects::bake_document(&doc).is_some(), "export bakes the instance");
    assert!(vectorcraft_render::painted_bounds(&doc, doc.node(id).unwrap()).is_some(), "the instance is measured");
    render(&s);
    assert!(expandable(&s));
    let e = s.execute("effect.expandAppearance", &json!({})).expect_err("the symbol nests without end");
    assert!(e.to_string().contains(crate::TOO_DEEP), "{e}");
    assert!(matches!(node(&s, id).kind, NodeKind::SymbolInstance { .. }), "the instance stays as it was");
}

/// A clip group's clipping path in the art keeps its shape, a compound one included, while the art
/// it clips takes the instance's fill effect.
#[test]
fn a_compound_clipping_path_in_the_art_keeps_its_shape() {
    let mut s = session();
    let (id, row) = instance_with_row(&mut s, "Square", true);
    offset_row(&mut s, id, row, 10.0);
    let red = || {
        let mut ap = vectorcraft_doc::Appearance::default();
        ap.items.push(vectorcraft_doc::AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(vectorcraft_color::Paint::solid(
            vectorcraft_color::Color::rgb(1.0, 0.0, 0.0),
        ))));
        ap
    };
    let square = |id: u64, x: f64| {
        let rect = vectorcraft_geom::Rect::new(x, 50.0, x + 100.0, 150.0);
        std::sync::Arc::new(Node::path(NodeId(id), vectorcraft_geom::shapes::rectangle(rect), red()))
    };
    let mut clip = Node::new(NodeId(90_001), NodeKind::Compound { children: vec![square(90_002, 40.0)], rule: Default::default() });
    clip.appearance = red().into();
    let group = Node::new(NodeId(90_003), NodeKind::Group { children: vec![std::sync::Arc::new(clip), square(90_004, 50.0)], clip: true });
    let doc = {
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        let sym = d.symbols.iter_mut().find(|sy| sy.name == "Square").unwrap();
        sym.art = std::sync::Arc::new(group);
        d.clone()
    };
    let inst = doc.node(id).unwrap();
    let art = doc.symbols.iter().find(|sy| sy.name == "Square").unwrap().art.clone();
    let out = vectorcraft_render::effects::outline_art(Some(&doc), inst, Some(&art)).expect("the instance's art");
    let mut seen = (false, false);
    out.walk(&mut |part| {
        let offset = part.appearance.items.iter().any(|i| i.effects().iter().any(|e| e.id == "path.offsetPath"));
        match part.id.0 {
            90_001 => {
                seen.0 = true;
                assert!(!offset, "the compound clipping path keeps its shape");
            }
            90_004 => {
                seen.1 = true;
                assert!(offset, "the clipped square takes the fill effect");
            }
            _ => {}
        }
    });
    assert_eq!(seen, (true, true));
}

/// A Drop Shadow 20 pt to the right on row `row` of instance `id`, solid black with no blur.
fn shadow_row(s: &mut Session, id: NodeId, row: usize) {
    let params = json!({"x": 20, "y": 0, "blur": 0, "opacity": 100, "mode": "normal"});
    run(s, "effect.apply", json!({"ids": [id.0], "item": row, "effect": "stylize.dropShadow", "params": params}));
}

/// Is `p` dark: a shadow, not the white page or the red fill?
fn dark(p: [u8; 4]) -> bool {
    p[..3].iter().all(|c| *c < 100)
}

/// A raster effect on a symbol instance's fill draws on the filled shapes of its art: the shadow
/// falls right of the square, the bounds take it in, a hidden fill draws none, and Expand
/// Appearance keeps the look in one undo step.
#[test]
fn a_symbol_instance_fill_shadow_shadows_its_filled_art() {
    let mut s = session();
    let (id, row) = instance_with_row(&mut s, "Square", true);
    assert!(near(render(&s).pixel(160, 100), WHITE));
    shadow_row(&mut s, id, row);
    let before = render(&s);
    assert!(dark(before.pixel(165, 100)), "the fill's shadow falls right of the square: {:?}", before.pixel(165, 100));
    assert!(near(before.pixel(100, 100), RED), "the art draws over its shadow: {:?}", before.pixel(100, 100));
    let doc = s.doc().unwrap().doc.clone();
    let b = vectorcraft_render::painted_bounds(&doc, doc.node(id).unwrap()).expect("the instance has bounds");
    assert!(b.x1 >= 169.0, "the bounds take in the shadow: {b:?}");
    // A view that shows only the shadow, right of the square: the instance is still drawn.
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let right = vectorcraft_render::Renderer::new().render(&doc, 200, 200, vectorcraft_geom::Affine::translate((-155.0, 0.0)), &opts);
    assert!(dark(right.pixel(10, 100)), "the shadow past the square's box is drawn: {:?}", right.pixel(10, 100));

    assert!(expandable(&s), "the fill's shadow can be expanded");
    run(&mut s, "effect.expandAppearance", json!({}));
    let ch = node(&s, id).children().cloned().expect("the instance becomes a group");
    assert!(matches!(ch[0].kind, NodeKind::Image(_)), "the shadow is an image under the art: {}", ch[0].kind_label());
    assert!(matches!(ch[1].kind, NodeKind::SymbolInstance { .. }), "the art stays an instance: {}", ch[1].kind_label());
    no_effects_left(&ch[1]);
    let after = render(&s);
    assert!(differing(&before, &after, 24) < 400, "expanding changed {} pixels", differing(&before, &after, 24));
    run(&mut s, "edit.undo", json!({}));
    let n = node(&s, id);
    assert!(matches!(n.kind, NodeKind::SymbolInstance { .. }), "one undo gives the instance back");
    assert!(n.appearance.items[row].effects().iter().any(|e| e.id == "stylize.dropShadow"));

    run(&mut s, "appearance.setItem", json!({"ids": [id.0], "index": row, "visible": false}));
    assert!(near(render(&s).pixel(165, 100), WHITE), "a hidden fill draws no shadow");
}

/// A raster effect on a symbol instance's stroke draws on the stroked shapes of its art only: the
/// stroke's shadow falls right of the stroke, and none falls where only the fill would cast one.
#[test]
fn a_symbol_instance_stroke_shadow_shadows_only_its_stroked_art() {
    let mut s = session();
    let (id, row) = instance_with_row(&mut s, "Square", false);
    shadow_row(&mut s, id, row);
    let r = render(&s);
    assert!(dark(r.pixel(170, 100)), "the stroke's shadow falls right of the stroke: {:?}", r.pixel(170, 100));
    assert!(near(r.pixel(160, 100), WHITE), "the fill casts no shadow: {:?}", r.pixel(160, 100));
}

/// Geometry and raster effects on one fill of an instance: the shadow falls from the grown fill.
#[test]
fn a_symbol_instance_fill_shadow_follows_the_fill_offset_before_it() {
    let mut s = session();
    let (id, row) = instance_with_row(&mut s, "Square", true);
    offset_row(&mut s, id, row, 10.0);
    shadow_row(&mut s, id, row);
    let r = render(&s);
    assert!(near(r.pixel(44, 100), RED), "the fill grows by the offset: {:?}", r.pixel(44, 100));
    assert!(dark(r.pixel(175, 100)), "the shadow falls from the grown fill: {:?}", r.pixel(175, 100));
}

/// An instance inside a symbol's art passes the outer instance's fill shadow on to its own art.
#[test]
fn a_nested_symbol_instance_takes_the_outer_fill_shadow() {
    let mut s = session();
    square(&mut s);
    run(&mut s, "symbol.new", json!({"name": "Inner"}));
    let id = NodeId(run(&mut s, "symbol.new", json!({"name": "Outer"}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addFill", json!({"ids": [id.0]}));
    let row = node(&s, id).appearance.items.len() - 1;
    run(&mut s, "appearance.setItem", json!({"ids": [id.0], "index": row, "none": true}));
    shadow_row(&mut s, id, row);
    assert!(dark(render(&s).pixel(165, 100)), "the inner art casts the shadow: {:?}", render(&s).pixel(165, 100));
}

/// Export draws a symbol instance's fill shadow on the filled shapes of its art: baking leaves the
/// shadow on the art's fill, not its stroke, and PDF export makes an image of it.
#[test]
fn export_draws_a_symbol_instance_fill_shadow() {
    let mut s = session();
    let (id, row) = instance_with_row(&mut s, "Square", true);
    shadow_row(&mut s, id, row);
    let doc = s.doc().unwrap().doc.clone();
    let baked = vectorcraft_render::effects::bake_document(&doc).expect("the instance is baked");
    let n = baked.node(id).expect("the baked art keeps the instance's id").clone();
    assert!(!matches!(n.kind, NodeKind::SymbolInstance { .. }));
    let shadowed = |i: &vectorcraft_doc::AppearanceItem| i.effects().iter().any(|e| e.id == "stylize.dropShadow");
    let (mut fills, mut strokes) = (0, 0);
    n.walk(&mut |part| {
        fills += part.appearance.items.iter().filter(|i| i.is_fill() && shadowed(i)).count();
        strokes += part.appearance.items.iter().filter(|i| !i.is_fill() && shadowed(i)).count();
    });
    assert_eq!((fills, strokes), (1, 0), "the art's fill carries the shadow");

    let flat = crate::flatten_raster_effects(&doc).expect("the shadow becomes an image");
    let mut image = None;
    flat.node(id).expect("the flattened art keeps the instance's id").walk(&mut |part| {
        if matches!(part.kind, NodeKind::Image(_)) {
            image = part.geometric_bounds();
        }
    });
    let image = image.expect("the shadow is an image");
    assert!(image.x1 >= 165.0, "the image holds the shadow right of the square: {image:?}");
}

#[test]
fn two_fills_and_an_offset_stroke_become_three_paths_that_look_the_same() {
    let mut s = session();
    let id = square(&mut s);
    assert!(!expandable(&s), "a basic appearance has nothing to expand");
    // A second fill on top (half-transparent green) and an outset stroke.
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "appearance.setItem", json!({"index": 2, "color": "#00ff00", "opacity": 50}));
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 10}, "item": 1}));
    assert!(expandable(&s));
    let before = render(&s);
    let r = run(&mut s, "effect.expandAppearance", json!({}));
    assert_eq!(r["ids"], json!([id.0]));
    let n = node(&s, id);
    let ch = n.children().expect("a group of the fills and the stroke");
    assert_eq!(ch.len(), 3);
    assert!(ch.iter().all(|c| matches!(c.kind, NodeKind::Path { .. } | NodeKind::Compound { .. }) && c.appearance.items.len() == 1));
    // In paint order: red fill, the stroke on the path offset 10 pt out (still a 4 pt stroke, which
    // Object › Expand would outline, #3089), the green fill.
    assert!(ch.iter().all(|c| c.appearance.items[0].effects().is_empty()));
    assert!(ch[0].appearance.items[0].is_fill() && !ch[1].appearance.items[0].is_fill() && ch[2].appearance.items[0].is_fill());
    assert_eq!(ch[1].appearance.stroke_width(), 4.0);
    let ring = ch[1].geometric_bounds().unwrap();
    assert!((ring.width() - 120.0).abs() < 0.5, "{ring:?}");
    let ring = ch[1].visual_bounds().unwrap();
    assert!((ring.width() - 124.0).abs() < 0.5, "{ring:?}");
    assert!((ch[2].opacity - 0.5).abs() < 1e-6, "the fill's opacity is the path's own");
    assert!(ch[2].appearance.is_basic());
    let after = render(&s);
    assert!(differing(&before, &after, 8) < 200, "{} pixels differ", differing(&before, &after, 8));
    assert!(!expandable(&s));
    // One undo step.
    run(&mut s, "edit.undo", json!({}));
    assert!(matches!(node(&s, id).kind, NodeKind::Path { .. }));
    assert_eq!(node(&s, id).appearance.items.len(), 3);
}

/// The objects [`random_transforms`] makes.
pub(super) const RANDOM_CASES: [&str; 5] = ["copies", "group copies", "nested group copies", "crop marks", "group crop marks"];

/// Selected art with Random in Transform effects, from 50 pt squares at y 60 with an 8 pt black
/// stroke, as `case` says. "copies": a square at x 20 filled with a gradient, a random Transform
/// effect on its stroke and a Transform effect that copies it 80 pt right. "group copies": the
/// same square without the second effect, in a group that has it. "nested group copies": the same
/// square in a group with a random Transform effect, in a group with the second effect. "crop
/// marks": a red square at x 20 with a random Transform effect and crop marks. "group crop marks":
/// red squares at x 20 and x 110 in a group with a random Transform effect and crop marks. Returns
/// the object's id.
pub(super) fn random_transforms(s: &mut Session, case: &str) -> NodeId {
    let random = json!({"moveH": 15, "moveV": 15, "rotate": 40, "random": true});
    let square = |s: &mut Session, x: f64, fill: Value| {
        let id = run(s, "shape.rectangle", json!({"x": x, "y": 60, "width": 50, "height": 50}))["id"].as_u64().unwrap();
        run(s, "paint.setFill", fill);
        run(s, "paint.setStroke", json!({"color": "#000000"}));
        run(s, "stroke.set", json!({"weight": 8}));
        id
    };
    let red = json!({"color": "#ff0000"});
    let id = match case {
        "copies" | "group copies" | "nested group copies" => {
            let a = square(s, 20.0, json!({"gradient": {"kind": "linear"}}));
            run(s, "effect.apply", json!({"effect": "distort.transform", "params": random, "item": 1}));
            if case == "nested group copies" {
                run(s, "object.group", json!({}));
                run(s, "effect.apply", json!({"effect": "distort.transform", "params": random, "item": null}));
            }
            if case == "copies" { a } else { run(s, "object.group", json!({}))["id"].as_u64().unwrap() }
        }
        "crop marks" => square(s, 20.0, red),
        _ => {
            let a = square(s, 20.0, red.clone());
            let b = square(s, 110.0, red);
            run(s, "select.set", json!({"ids": [a, b]}));
            run(s, "object.group", json!({}))["id"].as_u64().unwrap()
        }
    };
    if case.ends_with("copies") {
        run(s, "effect.apply", json!({"effect": "distort.transform", "params": {"moveH": 80, "copies": 1}, "item": null}));
    } else {
        run(s, "effect.apply", json!({"effect": "distort.transform", "params": random, "item": null}));
        run(s, "effect.apply", json!({"effect": "cropMarks", "item": null}));
    }
    NodeId(id)
}

/// Random in a Transform effect takes its seed from the object's id. The expanded art draws as the
/// canvas drew each of [`random_transforms`]' objects.
#[test]
fn expanded_art_keeps_the_random_transforms_the_canvas_drew() {
    let mut differ = vec![];
    for case in RANDOM_CASES {
        let mut s = session();
        let id = random_transforms(&mut s, case);
        let before = render(&s);
        let r = run(&mut s, "effect.expandAppearance", json!({}));
        differ.push((case, differing(&before, &render(&s), 64)));
        let mut art = node(&s, id);
        // Crop marks: the reply names the group of the object and its marks, then the object.
        if case.ends_with("crop marks") {
            art = (*art.children().unwrap()[0]).clone();
            assert_eq!((&r["ids"][0], &r["ids"][1]), (&json!(id.0), &json!(art.id.0)), "{case}");
        }
        // No object keeps an effect, and in the copies cases every object has a basic appearance:
        // every copy is expanded (its stroke a stroked path of its own, #3089).
        art.walk(&mut |n| {
            let unexpanded = case.ends_with("copies") && !n.appearance.is_basic();
            assert!(n.appearance.effects.is_empty() && !unexpanded, "{case}: {n:?}");
        });
    }
    assert_eq!(differ, RANDOM_CASES.map(|case| (case, 0)));
}

/// The reply lists each object whose appearance expanded once, by the id it has in the document,
/// starting with the selected object. In "group copies" the group's art holds the square with its
/// stroke's effect applied, so the reply names the group alone; the two copies of the square share
/// ids until Expand Appearance gives every object in them an id of its own.
#[test]
fn the_reply_names_each_expanded_copy_by_its_own_id() {
    for case in RANDOM_CASES {
        let mut s = session();
        let id = random_transforms(&mut s, case);
        let r = run(&mut s, "effect.expandAppearance", json!({}));
        let ids: Vec<u64> = r["ids"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect();
        assert_eq!(ids.first(), Some(&id.0), "{case}: {ids:?}");
        let unique: std::collections::HashSet<_> = ids.iter().collect();
        assert_eq!(unique.len(), ids.len(), "{case}: {ids:?}");
        assert!(ids.iter().all(|&i| s.doc().unwrap().doc.node(NodeId(i)).is_some()), "{case}: {ids:?}");
        if case == "group copies" {
            let art = node(&s, id);
            assert_eq!(ids, [id.0], "{case}");
            let copies = art.children().unwrap();
            assert_eq!(copies.len(), 2, "{case}");
            let mut seen = std::collections::HashSet::new();
            let mut repeated = vec![];
            art.walk(&mut |n| {
                if !seen.insert(n.id) {
                    repeated.push(n.id);
                }
            });
            assert!(repeated.is_empty(), "{case}: {repeated:?}");
        }
    }
}

/// A square with a color adjustment and crop marks: the reply names the group of the square and its
/// marks once. The adjusted square has a basic appearance, so the reply does not name it.
#[test]
fn the_reply_names_an_adjusted_object_with_crop_marks_once() {
    let mut s = session();
    let id = square(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "adjust.hueSaturation", "params": {"hue": 120}}));
    run(&mut s, "effect.apply", json!({"effect": "cropMarks", "item": null}));
    let r = run(&mut s, "effect.expandAppearance", json!({}));
    assert!(node(&s, id).children().is_some_and(|c| c.len() > 1));
    assert_eq!(r["ids"], json!([id.0]));
}

#[test]
fn a_shadow_becomes_an_image_under_the_art_and_transparency_stays_on_the_group() {
    let mut s = session();
    let id = square(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "params": {"x": 8, "y": 8, "blur": 2, "opacity": 100}}));
    run(&mut s, "transparency.set", json!({"opacity": 60, "blend": "multiply"}));
    let before = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    assert!((n.opacity - 0.6).abs() < 1e-6 && n.blend == BlendMode::Multiply);
    assert!(n.appearance.effects.is_empty() && n.appearance.items.is_empty());
    let ch = n.children().unwrap();
    assert_eq!(ch.len(), 3, "the shadow image, the fill, the outlined stroke");
    let NodeKind::Image(im) = &ch[0].kind else { panic!("the shadow is an image: {}", ch[0].kind_label()) };
    assert!(s.doc().unwrap().doc.images.contains_key(&im.key));
    assert!(ch[1..].iter().all(|c| c.opacity == 1.0 && c.appearance.effects.is_empty()));
    let after = render(&s);
    assert!(differing(&before, &after, 24) < 400, "{} pixels differ", differing(&before, &after, 24));
    // The shadow is still there, below and right of the square.
    let p = after.pixel(155, 155);
    assert!(p[0] < 200, "{p:?}");
}

#[test]
fn a_blur_turns_the_object_into_an_image() {
    let mut s = session();
    let id = square(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "blur.gaussian", "params": {"radius": 3}}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    assert!(matches!(n.kind, NodeKind::Image(_)), "{}", n.kind_label());
    assert!(n.appearance.effects.is_empty());
}

#[test]
fn expanding_a_blur_says_when_an_image_could_not_be_decoded() {
    let mut s = session();
    let cut = place_undecodable_png(&mut s);
    run(&mut s, "effect.apply", json!({"effect": "blur.gaussian", "params": {"radius": 3}, "ids": [cut.0]}));
    let r = run(&mut s, "effect.expandAppearance", json!({"ids": [cut.0]}));
    assert!(warns_undecoded(&r), "{r}");
    assert!(matches!(node(&s, cut).kind, NodeKind::Image(_)));
}

#[test]
fn an_art_brush_becomes_a_group_of_its_art() {
    let mut s = session();
    let id = NodeId(run(&mut s, "shape.line", json!({"x1": 20, "y1": 100, "x2": 180, "y2": 100}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "paint.setFill", json!({"none": true}));
    run(&mut s, "brush.apply", json!({"name": "Charcoal"}));
    assert!(expandable(&s));
    let before = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    assert!(matches!(n.kind, NodeKind::Group { .. }), "{}", n.kind_label());
    let mut brushed = false;
    n.walk(&mut |c| brushed |= vectorcraft_brush::has_brush(c));
    assert!(!brushed, "no brush strokes are left");
    let after = render(&s);
    assert!(differing(&before, &after, 24) < 200, "{} pixels differ", differing(&before, &after, 24));
    run(&mut s, "edit.undo", json!({}));
    assert!(vectorcraft_brush::has_brush(&node(&s, id)));
}

#[test]
fn a_targeted_layer_with_a_fill_expands_into_art_among_its_members() {
    let mut s = session();
    let a = square(&mut s);
    let layer = s.doc().unwrap().doc.layers[0].id;
    run(&mut s, "layer.target", json!({"id": layer.0}));
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "stroke.set", json!({"item": 0, "weight": 6}));
    assert!(expandable(&s));
    let before = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    let l = node(&s, layer);
    assert!(l.appearance.items.is_empty());
    let ch = l.children().unwrap();
    assert_eq!(ch.len(), 2, "the square, then the layer's stroke above it");
    assert_eq!(ch[0].id, a);
    // The layer's stroke stays a stroke, on a path of its own (#3089).
    let mut six = 0;
    ch[1].walk(&mut |n| {
        assert!(n.appearance.items.iter().all(|i| !i.is_fill()), "{n:?}");
        six += usize::from(n.appearance.stroke_width() == 6.0);
    });
    assert!(six > 0, "the layer's 6 pt stroke: {:?}", ch[1]);
    let after = render(&s);
    assert!(differing(&before, &after, 8) < 200, "{} pixels differ", differing(&before, &after, 8));
}

#[test]
fn type_with_a_shadow_stays_type_and_type_with_its_own_stroke_is_outlined() {
    let mut s = session();
    let t = NodeId(run(&mut s, "text.create", json!({"x": 20, "y": 80, "text": "Hi", "size": 48}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow"}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, t);
    let ch = n.children().unwrap();
    assert!(matches!(ch[0].kind, NodeKind::Image(_)) && matches!(ch[1].kind, NodeKind::Text(_)));
    // A stroke of its own: outlined glyphs.
    let u = NodeId(run(&mut s, "text.create", json!({"x": 20, "y": 160, "text": "Yo", "size": 48}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [u.0]}));
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let mut text = false;
    node(&s, u).walk(&mut |c| text |= matches!(c.kind, NodeKind::Text(_)));
    assert!(!text, "the type is outlined");
    assert!(!expandable(&s));
}

#[test]
fn type_with_its_own_stroke_expands_with_its_underline() {
    let mut s = session();
    let t = NodeId(run(&mut s, "text.create", json!({"x": 20, "y": 160, "text": "Hi", "size": 48}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "text.setFormat", json!({"underline": true}));
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "effect.expandAppearance", json!({}));
    // "Hi" has no ink below its baseline (y 160): the pieces reach the bar under it.
    let b = node(&s, t).geometric_bounds().unwrap();
    assert!(b.y1 > 162.0, "{b:?}");
}

#[test]
fn a_blurred_layer_keeps_its_image_as_its_only_member() {
    let mut s = session();
    square(&mut s);
    let layer = s.doc().unwrap().doc.layers[0].id;
    run(&mut s, "layer.clippingMask.toggle", json!({"id": layer.0}));
    run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 200, "height": 200}));
    run(&mut s, "layer.target", json!({"id": layer.0}));
    run(&mut s, "effect.apply", json!({"effect": "blur.gaussian"}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let l = node(&s, layer);
    assert!(l.is_layer() && !l.clips() && l.appearance.effects.is_empty());
    let ch = l.children().unwrap();
    assert!(ch.len() == 1 && matches!(ch[0].kind, NodeKind::Image(_)));
}

/// A selected open curve with no fill, stroked in blue with `stroke` (stroke.set's params) and
/// given a Zig Zag (#3089).
fn zig_zag_curve(s: &mut Session, stroke: Value) -> NodeId {
    let id = NodeId(run(s, "path.create", json!({"d": "M 30 100 C 70 40 130 160 170 100"}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"none": true}));
    run(s, "paint.setStroke", json!({"color": "#0000ff"}));
    run(s, "stroke.set", stroke);
    run(s, "effect.apply", json!({"effect": "distort.zigZag", "params": {"size": 8, "ridges": 5, "relative": false}}));
    id
}

#[test]
fn a_stroke_stays_a_stroke_with_its_dashes_arrowheads_alignment_and_width_profile() {
    for stroke in [
        json!({"weight": 4, "cap": "round", "join": "round", "dash": [10, 5]}),
        json!({"weight": 3, "startArrow": "Arrow", "endArrow": "Circle"}),
        json!({"weight": 8, "profile": "lens"}),
        json!({"weight": 6, "align": "inside"}),
    ] {
        let mut s = session();
        let id = zig_zag_curve(&mut s, stroke.clone());
        let was = node(&s, id).appearance.stroke().unwrap().clone();
        let before = render(&s);
        run(&mut s, "effect.expandAppearance", json!({}));
        // The zig-zag is baked into the path, which keeps the very same stroke and no fill: no outline.
        let n = node(&s, id);
        assert!(matches!(n.kind, NodeKind::Path { .. }), "{stroke}: a path, not a group: {:?}", n.kind);
        assert!(n.appearance.effects.is_empty() && n.appearance.is_basic(), "{stroke}");
        assert_eq!(n.appearance.stroke(), Some(&was), "{stroke}: the stroke's attributes stay");
        assert!(n.appearance.fill().is_none_or(|f| f.paint.is_none()), "{stroke}: no fill");
        assert!(n.path_data().unwrap().anchors().count() > 4, "{stroke}: the zig-zag is baked");
        let after = render(&s);
        assert!(differing(&before, &after, 8) < 40, "{stroke}: {} pixels differ", differing(&before, &after, 8));
        assert!(!expandable(&s), "{stroke}: nothing left for Expand Appearance");
    }
}

#[test]
fn several_strokes_become_stroked_paths_in_appearance_order() {
    let mut s = session();
    let id = square(&mut s);
    // [red fill, 4 pt blue stroke, 12 pt green stroke] with a Zig Zag on the object.
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "stroke.set", json!({"item": 2, "weight": 12}));
    run(&mut s, "appearance.setItem", json!({"index": 2, "color": "#00ff00"}));
    run(&mut s, "effect.apply", json!({"effect": "distort.zigZag", "params": {"size": 6, "ridges": 4, "relative": false}}));
    let items = node(&s, id).appearance.items.clone();
    let before = render(&s);
    run(&mut s, "effect.expandAppearance", json!({}));
    let n = node(&s, id);
    let ch = n.children().expect("a group of the fill and the two strokes");
    assert_eq!(ch.len(), 3);
    for (c, item) in ch.iter().zip(&items) {
        assert!(matches!(c.kind, NodeKind::Path { .. }) && c.appearance.items.len() == 1, "{:?}", c.kind);
        assert_eq!(&c.appearance.items[0], item, "each piece keeps its own fill or stroke, in appearance order");
    }
    let after = render(&s);
    assert!(differing(&before, &after, 8) < 60, "{} pixels differ", differing(&before, &after, 8));
}
