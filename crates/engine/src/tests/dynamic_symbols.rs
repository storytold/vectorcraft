//! Dynamic symbols (#4938): an instance changes the appearance of objects in its symbol's art and
//! stays linked; editing the symbol keeps those changes, Reset and Break Link handle them. Direct
//! Selection picks one path, compound path or type object of an instance's art, and Fill, Stroke,
//! opacity and blend change it for that instance.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Rect;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::tooling::{UiRequest, ViewInfo};
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    s
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A blue square at `x` (40 pt, no stroke).
fn square(s: &mut Session, x: f64) -> NodeId {
    let r = id(&s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 40, "height": 40})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [r.0], "color": "#0000ff"})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [r.0], "none": true})).unwrap();
    r
}

/// Symbol "Pair" of two blue squares (x 10..50 and 60..100) of `kind`, made in place: its instance.
fn pair(s: &mut Session, kind: &str) -> NodeId {
    let (a, b) = (square(s, 10.0), square(s, 60.0));
    id(&s.execute("symbol.new", &json!({"name": "Pair", "ids": [a.0, b.0], "type": kind})).unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn overrides(s: &Session, id: NodeId) -> usize {
    node(s, id).extras().symbol_overrides.len()
}

fn pixel(s: &Session, x: u32, y: u32) -> [u8; 4] {
    vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 200.0, 100.0), 1.0, true).pixel(x, y)
}

const RED: [u8; 4] = [255, 0, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// The refusal of a change that is not fill, stroke, opacity or blend on a picked path, compound
/// path or type (or of selecting any other kind of object) says so.
const REFUSED: &str = "fill, stroke, opacity and blend";

#[test]
fn a_dynamic_instance_paints_one_object_its_own_way_and_stays_linked() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    assert_eq!(s.execute("symbol.list", &json!({})).unwrap()["symbols"][0]["type"], "dynamic");
    let other = id(&s.execute("symbol.place", &json!({"name": "Pair", "x": 55, "y": 80})).unwrap());
    let r = s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "fill": "#ff0000"})).unwrap();
    assert_eq!(r["overrides"], 1);
    assert!(matches!(node(&s, inst).kind, NodeKind::SymbolInstance { .. }), "still an instance");
    assert_eq!(pixel(&s, 80, 30), RED, "the changed object");
    assert_eq!(pixel(&s, 30, 30), BLUE, "its sibling keeps the symbol's look");
    assert_eq!(overrides(&s, other), 0, "other instances keep the symbol's look");
    // The same value again records nothing.
    let undo = undo_steps(&s);
    assert_eq!(s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "fill": "#ff0000"})).unwrap()["overrides"], 1);
    assert_eq!(undo_steps(&s), undo, "no undo step");
    // A later change to the same object merges with the first; null takes one back.
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "opacity": 50})).unwrap();
    assert_eq!(node(&s, inst).extras().symbol_overrides[0].opacity, Some(0.5));
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "opacity": null})).unwrap();
    let o = node(&s, inst).extras().symbol_overrides[0].clone();
    assert!(o.opacity.is_none() && o.fill.is_some());
    // SVG export draws it too.
    let svg = s.execute("document.export", &json!({"format": "svg"})).unwrap();
    let svg = String::from_utf8(vectorcraft_format::base64_decode(svg["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(svg.to_ascii_lowercase().contains("#ff0000") || svg.contains("rgb(255,0,0)"), "{svg}");
}

#[test]
fn static_symbols_and_bad_targets_are_refused() {
    let mut s = session();
    let inst = pair(&mut s, "static");
    let set = |s: &mut Session, p: Value| s.execute("symbol.instance.setAppearance", &p);
    assert!(set(&mut s, json!({"id": inst.0, "path": [0], "fill": "#ff0000"})).is_err(), "a static symbol");
    s.execute("symbol.options", &json!({"name": "Pair", "type": "dynamic"})).unwrap();
    for bad in [
        json!({"id": inst.0, "path": [2], "fill": "#ff0000"}),
        json!({"id": inst.0, "path": [0, 0], "fill": "#ff0000"}),
        json!({"id": inst.0, "path": [-1], "fill": "#ff0000"}),
        json!({"id": inst.0, "path": "0", "fill": "#ff0000"}),
        json!({"id": inst.0, "fill": "#ff0000"}),
        json!({"id": inst.0, "path": [0]}),
        json!({"id": inst.0, "path": [0], "blend": "sparkle"}),
        json!({"id": inst.0, "path": [0], "opacity": "half"}),
        json!({"id": inst.0, "path": [0], "fill": {}}),
    ] {
        assert!(set(&mut s, bad.clone()).is_err(), "{bad}");
    }
    let plain = square(&mut s, 120.0);
    assert!(set(&mut s, json!({"id": plain.0, "path": [], "fill": "#ff0000"})).is_err(), "not an instance");
    assert!(s.execute("symbol.options", &json!({"name": "Pair", "type": "sometimes"})).is_err());
    assert_eq!(overrides(&s, inst), 0);
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// The frontmost object of the active layer.
fn top(s: &Session) -> NodeId {
    let d = &s.doc().unwrap().doc;
    d.node(d.default_layer().unwrap()).unwrap().children().unwrap().last().unwrap().id
}

/// Every pixel of the page.
fn page(s: &Session) -> Vec<[u8; 4]> {
    let r = vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 200.0, 100.0), 1.0, true);
    (0..r.height).flat_map(|y| (0..r.width).map(move |x| (x, y))).map(|(x, y)| r.pixel(x, y)).collect()
}

/// A dynamic symbol "Art" of the frontmost object, made in place: its instance.
fn art_of_top(s: &mut Session) -> NodeId {
    let t = top(s);
    id(&s.execute("symbol.new", &json!({"name": "Art", "ids": [t.0], "type": "dynamic"})).unwrap())
}

/// Put a red fill and stroke on the root of instance `inst`'s art directly, past the command.
fn paint_root_red(s: &mut Session, inst: NodeId) {
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    s.edit("paint", |d, _| {
        let o = vectorcraft_doc::symbol_overrides::SymbolOverride { path: vec![], fill: Some(red.clone()), stroke: Some(red), ..Default::default() };
        d.node_mut(inst).ok_or(EngineError::NoNode(inst))?.extras_mut().symbol_overrides.push(o);
        Ok(())
    })
    .unwrap();
}

/// A 30×30 red image placed with its frame from (50, 20) to (80, 50).
fn place_image(s: &mut Session) {
    let png = crate::tests_links::png(30, 30, [255, 0, 0]);
    let data = vectorcraft_format::base64_encode(&png);
    s.execute("file.place", &json!({"name": "red.png", "dataBase64": data, "at": [65, 35], "link": false})).unwrap();
}

fn select_top(s: &mut Session, n: usize) {
    let d = &s.doc().unwrap().doc;
    let kids = d.node(d.default_layer().unwrap()).unwrap().children().unwrap();
    let ids: Vec<u64> = kids[kids.len() - n..].iter().map(|c| c.id.0).collect();
    s.execute("select.set", &json!({ "ids": ids })).unwrap();
}

#[test]
fn objects_that_draw_no_fill_or_stroke_of_their_own_refuse_one() {
    let folder = crate::tests_links::Folder::new("dynamic-symbol-kinds");
    let source = folder.file("placed.vectorcraft");
    let mut doc = vectorcraft_doc::Document::new(40.0, 40.0);
    let layer = doc.layers[0].id;
    let look = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0);
    let rect = vectorcraft_doc::Node::path(doc.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 40.0)), look);
    doc.insert(Some(layer), 0, rect).unwrap();
    crate::tests_links::write(&source, &vectorcraft_format::save_file(&doc));
    type Make = Box<dyn Fn(&mut Session)>;
    let cases: Vec<(&str, &str, Make)> = vec![
        (
            "guide",
            "a guide",
            Box::new(|s| {
                square(s, 10.0);
                select_top(s, 1);
                s.execute("view.guides.make", &json!({})).unwrap();
            }),
        ),
        (
            "blend",
            "a blend",
            Box::new(|s| {
                let (a, b) = (square(s, 10.0), square(s, 60.0));
                s.execute("object.blend.make", &json!({"ids": [a.0, b.0], "steps": 1})).unwrap();
            }),
        ),
        (
            "envelope",
            "an envelope",
            Box::new(|s| {
                square(s, 10.0);
                select_top(s, 1);
                s.execute("object.envelope.makeWithWarp", &json!({"style": "arch", "bend": 50})).unwrap();
            }),
        ),
        ("radial Repeat", "a Repeat", Box::new(|s| repeat(s, "object.repeat.radial"))),
        ("grid Repeat", "a Repeat", Box::new(|s| repeat(s, "object.repeat.grid"))),
        ("mirror Repeat", "a Repeat", Box::new(|s| repeat(s, "object.repeat.mirror"))),
        (
            "Intertwine",
            "an Intertwine group",
            Box::new(|s| {
                square(s, 10.0);
                square(s, 30.0);
                select_top(s, 2);
                s.execute("object.intertwine.make", &json!({})).unwrap();
            }),
        ),
        (
            "gradient mesh",
            "a gradient mesh",
            Box::new(|s| {
                let r = square(s, 10.0);
                s.execute("object.mesh.create", &json!({"ids": [r.0], "rows": 1, "cols": 1})).unwrap();
            }),
        ),
        ("image", "an image", Box::new(place_image)),
        (
            "placed document",
            "a placed document",
            Box::new(move |s| {
                let r = s.execute("file.place", &json!({"path": source, "at": [40, 40]})).unwrap();
                assert_eq!(r["linked"], true, "{r}");
            }),
        ),
    ];
    // Every case runs, and the failure lists each object that fails.
    let mut failures = Vec::new();
    for (label, kind, make) in cases {
        let mut s = session();
        make(&mut s);
        let inst = art_of_top(&mut s);
        if let Err(why) = refuses_own_paint(&mut s, inst, kind) {
            failures.push(format!("{label}: {why}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Check that the root of instance `inst`'s art, an object of `kind` that draws no fill or stroke
/// of its own, refuses both, takes null, opacity and blend, and draws no stored fill or stroke.
fn refuses_own_paint(s: &mut Session, inst: NodeId, kind: &str) -> std::result::Result<(), String> {
    let set = |s: &mut Session, p: &Value| s.execute("symbol.instance.setAppearance", p);
    let undo = undo_len(s);
    for key in ["fill", "stroke"] {
        for paint in [json!("#ff0000"), json!({"none": true})] {
            // With a valid opacity too: nothing of the call is applied.
            let p = json!({"id": inst.0, "path": [], key: paint, "opacity": 50});
            match set(s, &p) {
                Ok(_) => return Err(format!("{p} was taken")),
                Err(e) if !e.to_string().contains(&format!("{key} cannot be set on {kind}")) => return Err(format!("{p}: {e}")),
                Err(_) => {}
            }
        }
    }
    if (overrides(s, inst), undo_len(s)) != (0, undo) {
        return Err("a refused call changed the instance or added an undo step".into());
    }
    // Null (taking a change back), opacity and blend are taken.
    let clear = json!({"id": inst.0, "path": [], "fill": null, "stroke": null});
    for p in [clear, json!({"id": inst.0, "path": [], "opacity": 50, "blend": "multiply"})] {
        set(s, &p).map_err(|e| format!("{p}: {e}"))?;
    }
    if overrides(s, inst) != 1 {
        return Err("opacity and blend were not stored".into());
    }
    // The renderer draws no fill or stroke of the object's own: a stored one changes nothing.
    let before = page(s);
    paint_root_red(s, inst);
    if page(s) != before {
        return Err("a stored fill and stroke changed the canvas".into());
    }
    Ok(())
}

fn repeat(s: &mut Session, cmd: &str) {
    square(s, 10.0);
    select_top(s, 1);
    s.execute(cmd, &json!({})).unwrap();
}

#[test]
fn a_symbol_instance_in_the_art_takes_a_fill_that_stains_it_and_no_stroke() {
    let mut s = session();
    pair(&mut s, "static");
    let inst = art_of_top(&mut s);
    let undo = undo_len(&s);
    let e = s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [], "stroke": "#ff0000"})).unwrap_err().to_string();
    assert!(e.contains("stroke cannot be set on a symbol instance"), "{e}");
    assert_eq!((overrides(&s, inst), undo_len(&s)), (0, undo));
    // A stroke stored past the command draws nothing.
    let before = page(&s);
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    s.edit("paint", |d, _| {
        let o = vectorcraft_doc::symbol_overrides::SymbolOverride { path: vec![], stroke: Some(red), ..Default::default() };
        d.node_mut(inst).ok_or(EngineError::NoNode(inst))?.extras_mut().symbol_overrides.push(o);
        Ok(())
    })
    .unwrap();
    assert!(page(&s) == before, "a stroke on a symbol instance draws nothing");
    // The fill stains the nested instance's art.
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [], "fill": "#ff0000"})).unwrap();
    assert_eq!(pixel(&s, 30, 30), RED, "the fill stains the art");
}

#[test]
fn a_symbol_instance_in_the_art_refuses_a_gradient_fill() {
    let mut s = session();
    pair(&mut s, "static");
    let inst = art_of_top(&mut s);
    let undo = undo_len(&s);
    let gradient = json!({"stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#00ff00"}]});
    let p = json!({"id": inst.0, "path": [], "fill": {"gradient": gradient}, "opacity": 50});
    let e = s.execute("symbol.instance.setAppearance", &p).unwrap_err().to_string();
    assert!(e.contains("fill on a symbol instance must be a solid color or none"), "{e}");
    assert_eq!((overrides(&s, inst), undo_len(&s)), (0, undo), "nothing changes and no undo step");
    // A gradient fill stored past the command draws nothing.
    let before = page(&s);
    let paint = Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default())));
    s.edit("paint", |d, _| {
        let o = vectorcraft_doc::symbol_overrides::SymbolOverride { path: vec![], fill: Some(paint), ..Default::default() };
        d.node_mut(inst).ok_or(EngineError::NoNode(inst))?.extras_mut().symbol_overrides.push(o);
        Ok(())
    })
    .unwrap();
    assert!(page(&s) == before, "a gradient fill on a symbol instance draws nothing");
    // A solid fill and none are taken.
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [], "fill": {"none": true}})).unwrap();
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [], "fill": "#ff0000"})).unwrap();
    assert_eq!(pixel(&s, 30, 30), RED);
}

#[test]
fn a_symbol_instance_in_the_art_refuses_a_pattern_fill() {
    let mut s = session();
    let tile = id(&s.execute("shape.rectangle", &json!({"x": 150, "y": 60, "width": 10, "height": 10})).unwrap());
    s.execute("object.pattern.make", &json!({"ids": [tile.0], "name": "Dots", "edit": false})).unwrap();
    pair(&mut s, "static");
    let inst = art_of_top(&mut s);
    let undo = undo_len(&s);
    let p = json!({"id": inst.0, "path": [], "fill": {"swatch": "Dots"}, "opacity": 50});
    let e = s.execute("symbol.instance.setAppearance", &p).unwrap_err().to_string();
    assert!(e.contains("fill on a symbol instance must be a solid color or none"), "{e}");
    assert_eq!((overrides(&s, inst), undo_len(&s)), (0, undo), "nothing changes and no undo step");
    // The pattern draws on a path.
    let probe = square(&mut s, 110.0);
    let before = page(&s);
    s.execute("paint.setFill", &json!({"ids": [probe.0], "swatch": "Dots"})).unwrap();
    assert!(page(&s) != before, "the pattern draws on a path");
    // A pattern fill stored on the nested instance past the command draws nothing.
    let before = page(&s);
    let paint = s.doc().unwrap().doc.swatch("Dots").unwrap().paint.clone();
    assert!(matches!(paint, Paint::Pattern { .. }), "{paint:?}");
    s.edit("paint", |d, _| {
        let o = vectorcraft_doc::symbol_overrides::SymbolOverride { path: vec![], fill: Some(paint), ..Default::default() };
        d.node_mut(inst).ok_or(EngineError::NoNode(inst))?.extras_mut().symbol_overrides.push(o);
        Ok(())
    })
    .unwrap();
    assert!(page(&s) == before, "a pattern fill on a symbol instance draws nothing");
}

#[test]
fn null_takes_back_an_old_change_an_object_does_not_draw() {
    let mut s = session();
    let (a, b) = (square(&mut s, 10.0), square(&mut s, 60.0));
    s.execute("object.blend.make", &json!({"ids": [a.0, b.0], "steps": 1})).unwrap();
    let inst = art_of_top(&mut s);
    // A change stored before the command refused one, as an older document may hold.
    paint_root_red(&mut s, inst);
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [], "fill": null})).unwrap();
    assert!(node(&s, inst).extras().symbol_overrides[0].fill.is_none());
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [], "stroke": null})).unwrap();
    assert_eq!(overrides(&s, inst), 0);
}

#[test]
fn paths_type_groups_and_shaped_image_paint_take_a_fill_and_stroke() {
    let set = |s: &mut Session, inst: NodeId, path: Value| {
        s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": path, "fill": "#ff0000", "stroke": "#ff0000"})).unwrap()
    };
    // A group's own fill paints over its members.
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    assert_eq!(set(&mut s, inst, json!([]))["overrides"], 1);
    assert_eq!(pixel(&s, 30, 30), RED, "the group's own fill");
    // A compound path, type and a clipping path.
    let mut s = session();
    let (a, b) = (square(&mut s, 10.0), square(&mut s, 60.0));
    s.execute("object.compoundPath.make", &json!({"ids": [a.0, b.0]})).unwrap();
    let inst = art_of_top(&mut s);
    set(&mut s, inst, json!([]));
    assert_eq!(pixel(&s, 30, 30), RED, "the compound path");
    let mut s = session();
    s.execute("text.create", &json!({"x": 10, "y": 40, "text": "Hi"})).unwrap();
    let inst = art_of_top(&mut s);
    set(&mut s, inst, json!([]));
    let mut s = session();
    let (a, b) = (square(&mut s, 10.0), square(&mut s, 20.0));
    s.execute("object.clippingMask.make", &json!({"ids": [a.0, b.0]})).unwrap();
    let inst = art_of_top(&mut s);
    assert!(matches!(&node(&s, inst).kind, NodeKind::SymbolInstance { .. }));
    set(&mut s, inst, json!([0]));
    set(&mut s, inst, json!([]));
    // An image whose fill has Convert to Shape: the fill paints 9 pt past its frame.
    let mut s = session();
    place_image(&mut s);
    s.execute("appearance.addFill", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#00ff00", "item": 0})).unwrap();
    s.execute("effect.apply", &json!({"effect": "convertToShape.rectangle", "item": 0})).unwrap();
    assert_eq!(pixel(&s, 44, 35), [0, 255, 0, 255]);
    let inst = art_of_top(&mut s);
    let e = s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [], "stroke": "#ff0000"})).unwrap_err().to_string();
    assert!(e.contains("stroke cannot be set on an image without a Convert to Shape or Outline Object effect on it"), "{e}");
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [], "fill": "#ff0000"})).unwrap();
    assert_eq!(pixel(&s, 44, 35), RED, "the shaped fill takes the instance's color");
}

#[test]
fn redefining_the_symbol_keeps_the_changes_whose_objects_remain() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "fill": "#ff0000"})).unwrap();
    // New geometry with the same structure: two squares, now 10..40 and 50..80 tall.
    let (a, b) = (square(&mut s, 10.0), square(&mut s, 120.0));
    s.execute("symbol.update", &json!({"name": "Pair", "ids": [a.0, b.0]})).unwrap();
    assert_eq!(overrides(&s, inst), 1, "the change follows the edited symbol");
    // Art of one object (no children) leaves path [1] behind.
    let c = square(&mut s, 10.0);
    s.execute("symbol.update", &json!({"name": "Pair", "ids": [c.0]})).unwrap();
    assert_eq!(overrides(&s, inst), 0);
}

#[test]
fn replace_symbol_drops_the_changes_made_for_the_old_symbol() {
    let mut s = session();
    // "Other": two blue squares in the same places, the instance taken off the canvas.
    let (a, b) = (square(&mut s, 10.0), square(&mut s, 60.0));
    let first = id(&s.execute("symbol.new", &json!({"name": "Other", "ids": [a.0, b.0], "type": "dynamic"})).unwrap());
    s.execute("edit.clear", &json!({"ids": [first.0]})).unwrap();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "fill": "#ff0000"})).unwrap();
    // The same symbol: nothing changes.
    s.execute("symbol.replace", &json!({"name": "Pair", "ids": [inst.0]})).unwrap();
    assert_eq!(overrides(&s, inst), 1);
    assert_eq!(pixel(&s, 80, 30), RED);
    s.execute("symbol.replace", &json!({"name": "Other", "ids": [inst.0]})).unwrap();
    assert_eq!(overrides(&s, inst), 0, "the change named an object of Pair's art");
    assert_eq!(pixel(&s, 80, 30), BLUE, "Other's art draws as the symbol has it");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(&node(&s, inst).kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Pair"));
    assert_eq!(overrides(&s, inst), 1);
    assert_eq!(pixel(&s, 80, 30), RED);
}

#[test]
fn reset_and_static_drop_the_changes_and_undo_brings_them_back() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [0], "fill": "#ff0000", "blend": "multiply"})).unwrap();
    let r = s.execute("symbol.instance.reset", &json!({"ids": [inst.0]})).unwrap();
    assert_eq!(r["ids"], json!([inst.0]));
    assert_eq!(overrides(&s, inst), 0);
    assert_eq!(pixel(&s, 30, 30), BLUE);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(overrides(&s, inst), 1);
    // Nothing to reset: no undo step.
    let other = id(&s.execute("symbol.place", &json!({"name": "Pair"})).unwrap());
    let undo = s.doc().unwrap().history.undo.len();
    assert_eq!(s.execute("symbol.instance.reset", &json!({"ids": [other.0]})).unwrap()["ids"], json!([]));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    // Static again: the instances show the symbol's appearance.
    s.execute("symbol.options", &json!({"name": "Pair", "type": "static"})).unwrap();
    assert_eq!(overrides(&s, inst), 0);
}

#[test]
fn break_link_bakes_the_changes_and_edit_symbol_shows_the_symbols_own_art() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "fill": "#ff0000"})).unwrap();
    let other = id(&s.execute("symbol.place", &json!({"name": "Pair", "x": 55, "y": 80})).unwrap());
    s.execute("symbol.instance.setAppearance", &json!({"id": other.0, "path": [1], "fill": "#ff0000"})).unwrap();
    let fills = |s: &Session, art: NodeId| -> Vec<String> {
        node(s, art).children().unwrap().iter().map(|c| c.appearance.fill_paint().color().unwrap().to_hex()).collect()
    };
    let art = NodeId(s.execute("symbol.breakLink", &json!({"ids": [inst.0]})).unwrap()["ids"][0].as_u64().unwrap());
    assert_eq!(fills(&s, art), ["#0000ff", "#ff0000"]);
    // Symbol editing mode shows the symbol's art, without this instance's changes.
    s.execute("symbol.edit", &json!({"ids": [other.0]})).unwrap();
    let st = s.doc().unwrap();
    let layer = st.doc.symbol_edit.as_ref().unwrap().layer;
    let parts: Vec<String> =
        st.doc.node(layer).unwrap().children().unwrap().iter().map(|c| c.appearance.fill_paint().color().unwrap().to_hex()).collect();
    assert_eq!(parts, ["#0000ff", "#0000ff"], "the symbol's art, without this instance's changes");
    assert_eq!(overrides(&s, other), 1, "the instance keeps its changes");
}

/// Press the Direct Selection tool at (`x`, `y`), drag to `to` and release, with `m` held: what the
/// drag asked of the UI.
fn direct(s: &mut Session, (x, y): (f64, f64), to: (f64, f64), m: Mods) -> Vec<UiRequest> {
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    s.select_tool("directSelection", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, x, y).with_mods(m), v).unwrap();
    let mut out = vec![];
    if to != (x, y) {
        out = s.pointer(&PointerEvent::new(PointerKind::Drag, to.0, to.1).with_mods(m), v).unwrap();
    }
    s.pointer(&PointerEvent::new(PointerKind::Up, to.0, to.1).with_mods(m), v).unwrap();
    out
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// The fills of the symbol's own art.
fn symbol_fills(s: &Session) -> Vec<String> {
    let art = s.doc().unwrap().doc.symbols[0].art.clone();
    art.children().unwrap().iter().map(|c| c.appearance.fill_paint().color().unwrap().to_hex()).collect()
}

#[test]
fn direct_selection_paints_one_object_of_one_dynamic_instance() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    let other = id(&s.execute("symbol.place", &json!({"name": "Pair", "x": 55, "y": 80})).unwrap());
    direct(&mut s, (80.0, 30.0), (80.0, 30.0), Mods::default());
    let sel = s.doc().unwrap().selection.clone();
    assert_eq!(sel.objects, [inst]);
    assert_eq!(sel.symbol_part, Some(vectorcraft_doc::SymbolPart { instance: inst, path: vec![1] }));
    let undo = undo_steps(&s);
    // The Swatches panel, the Color panel and the Control bar set the fill with paint.setFill.
    assert_eq!(s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap()["objects"], 1);
    assert_eq!(undo_steps(&s), undo + 1, "one undo step");
    assert_eq!(pixel(&s, 80, 30), RED, "the picked object");
    assert_eq!(pixel(&s, 30, 30), BLUE, "its sibling keeps the symbol's look");
    assert_eq!(overrides(&s, other), 0, "the other instance keeps the symbol's look");
    assert_eq!(symbol_fills(&s), ["#0000ff", "#0000ff"], "the symbol's art is unchanged");
    assert_eq!(s.paint.fill.color().map(|c| c.to_hex()), Some("#ff0000".into()), "new art takes the fill");
    // The same paint again records nothing.
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    assert_eq!(undo_steps(&s), undo + 1);
    // Stroke goes to the same object.
    s.execute("paint.setStroke", &json!({"color": "#00ff00"})).unwrap();
    assert_eq!(s.paint.stroke.color().map(|c| c.to_hex()), Some("#00ff00".into()), "new art takes the stroke");
    assert!(s.paint.stroke_width > 0.0, "a painted stroke has a weight");
    let o = node(&s, inst).extras().symbol_overrides[0].clone();
    assert_eq!(o.path, vec![1]);
    assert!(o.fill.is_some() && o.stroke.is_some());
    // The Eyedropper's click on another object gives the selected object its paint.
    let green = square(&mut s, 150.0);
    s.execute("paint.setFill", &json!({"ids": [green.0], "color": "#00ff00"})).unwrap();
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    assert_eq!(s.execute("appearance.copyFrom", &json!({"source": green.0})).unwrap()["ids"], json!([inst.0]));
    assert_eq!(pixel(&s, 80, 30), [0, 255, 0, 255], "the eyedropper's paint");
    assert_eq!(symbol_fills(&s), ["#0000ff", "#0000ff"]);
    for _ in 0..7 {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert_eq!(overrides(&s, inst), 0, "undo takes each change back");
    assert_eq!(pixel(&s, 80, 30), BLUE);
}

#[test]
fn a_drag_of_a_picked_object_moves_nothing_and_says_why() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    direct(&mut s, (80.0, 30.0), (80.0, 30.0), Mods::default());
    let (before, undo) = (node(&s, inst).kind.clone(), undo_steps(&s));
    let ui = direct(&mut s, (80.0, 30.0), (120.0, 60.0), Mods::default());
    assert!(ui.iter().any(|r| matches!(r, UiRequest::Status(m) if m.contains("symbol instance"))), "{ui:?}");
    assert_eq!(node(&s, inst).kind, before, "the instance stays put");
    assert_eq!(undo_steps(&s), undo, "no undo step");
    assert!(s.execute("object.nudge", &json!({"dx": 1})).is_err(), "the arrow keys too");
    assert!(s.execute("object.rotate", &json!({"angle": 10})).is_err());
    // The Selection tool picks the instance whole again, and it moves.
    s.select_tool("selection", ViewInfo::default()).unwrap();
    assert!(s.doc().unwrap().selection.symbol_part.is_none());
    s.execute("object.nudge", &json!({"dx": 1})).unwrap();
    assert_ne!(node(&s, inst).kind, before);
}

/// The Stroke panel and the Control bar show the stroke of the object picked in a dynamic instance.
#[test]
fn the_stroke_panel_shows_a_picked_objects_stroke() {
    let mut s = session();
    let (a, b) = (square(&mut s, 10.0), square(&mut s, 60.0));
    s.execute("paint.setStroke", &json!({"ids": [a.0, b.0], "color": "#00ff00"})).unwrap();
    s.execute("stroke.set", &json!({"ids": [a.0], "weight": 2})).unwrap();
    s.execute("stroke.set", &json!({"ids": [b.0], "weight": 7})).unwrap();
    let inst = id(&s.execute("symbol.new", &json!({"name": "Pair", "ids": [a.0, b.0], "type": "dynamic"})).unwrap());
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    assert_eq!(s.shown_stroke().map(|st| st.width), Some(7.0), "the picked object's weight");
    assert!(!s.doc().unwrap().stroke_mixed().weight, "the instance's art has two weights, the object one");
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [0]})).unwrap();
    assert_eq!(s.shown_stroke().map(|st| st.width), Some(2.0));
}

#[test]
fn delete_and_cut_of_a_picked_object_remove_nothing() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    let undo = undo_steps(&s);
    for cmd in ["edit.cut", "edit.clear"] {
        let e = s.execute(cmd, &json!({})).unwrap_err().to_string();
        assert!(e.contains("symbol instance"), "{cmd}: {e}");
        assert!(s.doc().unwrap().doc.node(inst).is_some(), "{cmd} keeps the instance");
    }
    assert_eq!(undo_steps(&s), undo, "no undo step");
    // Named by id, the instance is removed.
    s.execute("edit.clear", &json!({"ids": [inst.0]})).unwrap();
    assert!(s.doc().unwrap().doc.node(inst).is_none());
}

#[test]
fn commands_on_the_selection_leave_a_picked_objects_instance_alone() {
    // A dynamic instance with an opacity mask, a plain square and the Desaturate plug-in, with an
    // object of the instance picked.
    let setup = || {
        let mut s = session();
        let inst = pair(&mut s, "dynamic");
        let other = square(&mut s, 150.0);
        let desaturate = include_bytes!("../../../plugins/tests/fixtures/desaturate.wasm");
        let plugin = s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(desaturate)})).unwrap();
        let mask = square(&mut s, 20.0);
        s.execute("transparency.makeOpacityMask", &json!({"ids": [inst.0, mask.0]})).unwrap();
        assert!(node(&s, inst).extras().mask.is_some());
        s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
        (s, inst, other, plugin["id"].clone())
    };
    let (mut s, inst, other, plugin) = setup();
    let doc = s.doc().unwrap().doc.clone();
    // Each command runs on its own copy of the setup, compared with that copy as it was, so every
    // command that fails is reported.
    let mut failures = Vec::new();
    for (cmd, p) in [
        ("object.puppetWarp", json!({"pins": [[80, 30]], "moved": [[90, 40]], "rest": true})),
        ("recolor.apply", json!({"map": {"#0000ff": "#00ff00"}})),
        ("plugin.run", json!({"id": plugin, "params": {"amount": 100}})),
        ("object.group", json!({})),
        ("object.ungroup", json!({})),
        ("object.hide", json!({})),
        ("object.lock", json!({})),
        ("object.arrange.bringToFront", json!({})),
        ("object.expand", json!({})),
        ("object.rotate", json!({"angle": 45})),
        ("symbol.instance.reset", json!({})),
        ("symbol.instance.setAppearance", json!({"path": [1], "opacity": 50})),
        ("symbol.adjust", json!({"tool": "shift", "points": [[80, 30], [95, 30]], "radius": 200})),
        ("symbol.edit", json!({})),
        ("select.globalEdit", json!({})),
        ("object.blend.release", json!({})),
        ("object.repeat.release", json!({})),
        ("object.intertwine.release", json!({})),
        ("object.objectsOnPath.expand", json!({})),
        ("transparency.set", json!({"opacity": 50, "isolate": true})),
        ("transparency.releaseOpacityMask", json!({})),
        ("transparency.setOpacityMask", json!({"invert": true, "clip": false})),
        ("transparency.disableOpacityMask", json!({})),
        ("transparency.unlinkOpacityMask", json!({})),
        ("transparency.editOpacityMask", json!({})),
        ("transparency.viewOpacityMask", json!({"on": true})),
        ("paint.editGradient", json!({"angle": 30})),
        ("paint.setGradientGeom", json!({"start": [62, 30], "end": [98, 30]})),
        ("paint.swap", json!({})),
        ("stroke.set", json!({"weight": 3})),
        ("path.moveAnchors", json!({"dx": 5, "dy": 5})),
        (
            "file.place",
            json!({"name": "a.svg", "dataBase64": vectorcraft_format::base64_encode(b"<svg xmlns='http://www.w3.org/2000/svg' width='9' height='9'><rect width='9' height='9'/></svg>"), "replace": true}),
        ),
        ("object.intertwine.arrange", json!({"at": [80, 30]})),
        ("transparency.editOpacityMask", json!({"id": null})),
        ("transparency.viewOpacityMask", json!({"on": true, "id": null})),
    ] {
        let (mut t, ..) = setup();
        let (doc, undo) = (t.doc().unwrap().doc.clone(), undo_steps(&t));
        match t.execute(cmd, &p) {
            Ok(r) => failures.push(format!("{cmd}: ran ({r})")),
            Err(e) if !e.to_string().contains("symbol instance") => failures.push(format!("{cmd}: {e}")),
            Err(_) => {}
        }
        if t.doc().unwrap().selection.symbol_part.is_none() {
            failures.push(format!("{cmd}: the object is no longer picked"));
        }
        if t.doc().unwrap().doc != doc {
            failures.push(format!("{cmd}: the document changed"));
        }
        if undo_steps(&t) != undo {
            failures.push(format!("{cmd}: an undo step was added"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    // Copy copies the instance whole.
    s.execute("edit.copy", &json!({})).unwrap();
    assert!(matches!(&s.clipboard.nodes[..], [n] if matches!(n.kind, NodeKind::SymbolInstance { .. })));
    // The picked object's paint still changes.
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    assert!(node(&s, inst).extras().symbol_overrides[0].fill.is_some());
    // A command given ids acts on them.
    s.execute("object.rotate", &json!({"ids": [other.0], "angle": 45})).unwrap();
    assert_ne!(node(&s, other).kind, doc.node(other).unwrap().kind, "the named object turns");
    s.execute("transparency.setOpacityMask", &json!({"id": inst.0, "invert": true})).unwrap();
    assert!(node(&s, inst).extras().mask.as_ref().is_some_and(|m| m.invert), "the named instance's mask changes");
}

#[test]
fn the_commands_without_a_check_for_a_picked_object_use_its_instance() {
    let setup = || {
        let mut s = session();
        let inst = pair(&mut s, "dynamic");
        s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "fill": "#ff0000"})).unwrap();
        s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
        (s, inst)
    };
    // Export Selection: the whole instance, x 10..100, and not the object at 60..100.
    let (mut s, _) = setup();
    let r = s.execute("document.exportSelection", &json!({"format": "svg"})).unwrap();
    assert_eq!(r["bounds"], json!([10.0, 10.0, 90.0, 40.0]));
    // Fit to Selection: the whole instance too.
    let (mut s, _) = setup();
    s.execute("artboard.fitToSelection", &json!({})).unwrap();
    let r = s.doc().unwrap().doc.artboards[0].rect;
    assert_eq!((r.x0, r.x1), (10.0, 100.0));
    // Liquify: the instance is its target, skipped and left as it is.
    let (mut s, inst) = setup();
    let doc = s.doc().unwrap().doc.clone();
    let r = s.execute("object.liquify", &json!({"tool": "pucker", "points": [[80, 30]], "diameter": 150, "intensity": 1})).unwrap();
    assert_eq!((&r["ids"], &r["skipped"]), (&json!([]), &json!([inst.0])));
    assert!(s.doc().unwrap().doc == doc);
    // Isolate: the instance, with the selection cleared.
    let (mut s, inst) = setup();
    s.execute("object.isolate", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().isolation, Some(inst));
    assert!(s.doc().unwrap().selection.is_empty());
    // Add to Library: a copy of the whole instance, 90 pt wide, and not the object's paint.
    let (mut s, _) = setup();
    let lib = s.execute("library.add", &json!({"kind": "graphic"})).unwrap()["library"].clone();
    let got = s.execute("library.get", &json!({"library": lib})).unwrap();
    assert_eq!(got["graphics"][0]["width"], 90.0);
    assert_ne!(s.execute("library.add", &json!({"kind": "fillColor"})).unwrap()["name"], "#ff0000");
    // Save Selection: the instance, which Recall selects whole.
    let (mut s, inst) = setup();
    s.execute("select.save", &json!({"name": "Saved"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.saved_selections[0].objects, vec![inst]);
    s.execute("select.recall", &json!({"name": "Saved"})).unwrap();
    let sel = &s.doc().unwrap().selection;
    assert_eq!((&sel.objects, &sel.symbol_part), (&vec![inst], &None));
}

#[test]
fn a_blend_around_a_picked_objects_instance_is_left_alone() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    let other = square(&mut s, 150.0);
    let blend = id(&s.execute("object.blend.make", &json!({"ids": [inst.0, other.0], "steps": 1})).unwrap());
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    let (doc, undo) = (s.doc().unwrap().doc.clone(), undo_steps(&s));
    let e = s.execute("object.blend.release", &json!({})).unwrap_err().to_string();
    assert!(e.contains("symbol instance"), "{e}");
    assert!(s.doc().unwrap().doc == doc, "the blend stays");
    assert_eq!(undo_steps(&s), undo, "no undo step");
    // With the blend selected, it is released.
    s.execute("select.set", &json!({"ids": [blend.0]})).unwrap();
    s.execute("object.blend.release", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(blend).is_none());
}

/// Dynamic symbol "Photo" of a gray 2 × 1 image and a blue square, made in place: its instance and
/// the image's path in the art.
fn image_symbol(s: &mut Session) -> (NodeId, Value) {
    let mut png = Vec::new();
    let pixels = image::DynamicImage::ImageLuma8(image::GrayImage::from_raw(2, 1, vec![0, 255]).unwrap());
    pixels.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let im = s
        .edit("Place", |d, _| {
            d.images.insert("gray".into(), vectorcraft_doc::ImageBlob::png(png));
            let id = d.alloc_id();
            let im = vectorcraft_doc::ImageObject {
                key: "gray".into(),
                width: 40,
                height: 40,
                xf: vectorcraft_geom::Affine::translate((10.0, 10.0)),
                link: None,
                placement: Default::default(),
                pixels: None,
            };
            d.insert(d.default_layer(), 0, Node::new(id, NodeKind::Image(Box::new(im))))?;
            Ok(id)
        })
        .unwrap();
    let sq = square(s, 60.0);
    let inst = id(&s.execute("symbol.new", &json!({"name": "Photo", "ids": [im.0, sq.0], "type": "dynamic"})).unwrap());
    let art = s.doc().unwrap().doc.symbols[0].art.clone();
    let i = art.children().unwrap().iter().position(|c| matches!(c.kind, NodeKind::Image(_))).unwrap();
    (inst, json!([i]))
}

#[test]
fn only_paths_compound_paths_and_type_of_the_art_are_picked() {
    let mut s = session();
    // A compound path, type, a blend and a static symbol's instance beside a path, in a group.
    let (a, b) = (square(&mut s, 10.0), square(&mut s, 20.0));
    let compound = id(&s.execute("object.compoundPath.make", &json!({"ids": [a.0, b.0]})).unwrap());
    let text = id(&s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hi"})).unwrap());
    let (c, d) = (square(&mut s, 60.0), square(&mut s, 150.0));
    let blend = id(&s.execute("object.blend.make", &json!({"ids": [c.0, d.0], "steps": 1})).unwrap());
    let inner = pair(&mut s, "static");
    let plain = square(&mut s, 100.0);
    let group = id(&s.execute("object.group", &json!({"ids": [plain.0]})).unwrap());
    let ids = [compound.0, text.0, blend.0, inner.0, group.0];
    let inst = id(&s.execute("symbol.new", &json!({"name": "Mixed", "ids": ids, "type": "dynamic"})).unwrap());
    let art = s.doc().unwrap().doc.symbols.iter().find(|y| y.name == "Mixed").unwrap().art.clone();
    let at = |kind: &dyn Fn(&NodeKind) -> bool| art.children().unwrap().iter().position(|c| kind(&c.kind)).unwrap();
    let compound = at(&|k| matches!(k, NodeKind::Compound { .. }));
    let text = at(&|k| matches!(k, NodeKind::Text(_)));
    let blend = at(&|k| matches!(k, NodeKind::Blend { .. }));
    let inner = at(&|k| matches!(k, NodeKind::SymbolInstance { .. }));
    let group = at(&|k| matches!(k, NodeKind::Group { .. }));
    for path in [json!([compound]), json!([text]), json!([group, 0])] {
        s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": path})).unwrap();
        s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
        s.execute("paint.setStroke", &json!({"color": "#00ff00"})).unwrap();
    }
    assert_eq!(overrides(&s, inst), 3);
    let undo = undo_steps(&s);
    s.execute("select.none", &json!({})).unwrap();
    for path in [json!([]), json!([group]), json!([blend]), json!([inner])] {
        let e = s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": path})).unwrap_err().to_string();
        assert!(e.contains(REFUSED), "{path}: {e}");
    }
    assert!(s.doc().unwrap().selection.symbol_part.is_none(), "nothing is picked");
    assert_eq!((overrides(&s, inst), undo_steps(&s)), (3, undo), "nothing changes");
}

/// A Direct Selection press on an image of the art is refused with the status message, and a drag
/// from it does not move what was selected.
#[test]
fn a_press_on_an_image_of_the_art_is_refused_and_moves_nothing() {
    let mut s = session();
    let (inst, path) = image_symbol(&mut s);
    let other = square(&mut s, 150.0);
    let e = s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": path})).unwrap_err().to_string();
    assert!(e.contains(REFUSED), "{e}");
    s.execute("select.set", &json!({"ids": [other.0]})).unwrap();
    let before = node(&s, other).kind.clone();
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    s.select_tool("directSelection", v).unwrap();
    let e = s.pointer(&PointerEvent::new(PointerKind::Down, 30.0, 30.0), v).unwrap_err().to_string();
    assert!(e.contains(REFUSED), "{e}");
    s.pointer(&PointerEvent::new(PointerKind::Drag, 60.0, 60.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 60.0, 60.0), v).unwrap();
    assert_eq!(node(&s, other).kind, before, "what was selected stays put");
    assert_eq!(s.doc().unwrap().selection.objects, [other], "and stays selected");
    assert_eq!(overrides(&s, inst), 0);
}

/// A gradient or pattern for a picked object is stored on the instance; Invert, Complement and the
/// gradient annotator stay refused. An object that shows a gradient takes a solid color.
#[test]
fn gradients_and_patterns_on_a_picked_object_are_taken() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    let g = json!({"kind": "linear", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]});
    s.execute("swatch.new", &json!({"name": "Ramp", "gradient": g})).unwrap();
    let tile = square(&mut s, 150.0);
    s.execute("object.pattern.make", &json!({"ids": [tile.0], "name": "Dots", "edit": false})).unwrap();
    let far = square(&mut s, 150.0);
    s.execute("paint.setFill", &json!({"ids": [far.0], "gradient": g})).unwrap();
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    s.execute("paint.setFill", &json!({"gradient": g})).unwrap();
    assert!(matches!(node(&s, inst).extras().symbol_overrides[0].fill, Some(Paint::Gradient(_))));
    s.execute("paint.setStroke", &json!({"gradient": g})).unwrap();
    assert!(matches!(node(&s, inst).extras().symbol_overrides[0].stroke, Some(Paint::Gradient(_))));
    s.execute("paint.setFill", &json!({"swatch": "Ramp"})).unwrap();
    assert!(matches!(node(&s, inst).extras().symbol_overrides[0].fill, Some(Paint::Gradient(_))));
    s.execute("paint.setFill", &json!({"swatch": "Dots"})).unwrap();
    assert!(matches!(node(&s, inst).extras().symbol_overrides[0].fill, Some(Paint::Pattern { .. })));
    s.execute("appearance.copyFrom", &json!({"source": far.0})).unwrap();
    assert!(matches!(node(&s, inst).extras().symbol_overrides[0].fill, Some(Paint::Gradient(_))));
    // Invert, Complement and selecting a stop are refused on a gradient.
    for cmd in ["paint.invert", "paint.complement"] {
        let e = s.execute(cmd, &json!({"stroke": false})).unwrap_err().to_string();
        assert!(e.contains(REFUSED), "{cmd}: {e}");
    }
    s.fill_active = true;
    for index in [json!(1), Value::Null] {
        let e = s.execute("gradient.selectStop", &json!({"index": index})).unwrap_err().to_string();
        assert!(e.contains(REFUSED), "gradient.selectStop {index}: {e}");
    }
    assert_eq!(s.selected_stop(), None);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    assert_eq!(pixel(&s, 80, 30), RED);
    s.execute("appearance.copyFrom", &json!({"source": far.0, "pickUp": {"appearance": {"fill": {"color": false}}}})).unwrap();
    assert_eq!(pixel(&s, 80, 30), RED);
    let (undo, fill) = (undo_steps(&s), s.paint.fill.clone());
    let neither = json!({"source": far.0, "pickUp": {"appearance": {"fill": {"color": false}, "stroke": {"color": false}}}});
    let e = s.execute("appearance.copyFrom", &neither).unwrap_err().to_string();
    assert!(e.contains(REFUSED), "{e}");
    assert_eq!((undo_steps(&s), &s.paint.fill), (undo, &fill), "nothing changes");
}

/// A stop selected on the whole instance's gradient is not selected once a part of its art is
/// picked, so the Gradient tool does not take Delete for it, and is selected again when the whole
/// instance is.
#[test]
fn picking_a_part_leaves_no_gradient_stop_selected() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    let g = json!({"kind": "linear", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]});
    s.execute("select.set", &json!({"ids": [inst.0]})).unwrap();
    s.execute("paint.setFill", &json!({"gradient": g})).unwrap();
    s.fill_active = true;
    let view = ViewInfo::default();
    s.select_tool("gradient", view).unwrap();
    s.execute("gradient.selectStop", &json!({"index": 1})).unwrap();
    assert_eq!(s.selected_stop(), Some(1));
    assert!(s.tool_claims_key(vectorcraft_tools::ToolKey::Delete, view));
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    assert_eq!(s.selected_stop(), None);
    assert!(!s.tool_claims_key(vectorcraft_tools::ToolKey::Delete, view));
    s.execute("select.set", &json!({"ids": [inst.0]})).unwrap();
    assert_eq!(s.selected_stop(), Some(1));
}

#[test]
fn opacity_and_blend_on_a_picked_object_go_through_transparency_set() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    assert_eq!(node(&s, inst).extras().symbol_overrides[0].opacity, Some(0.5));
    s.execute("transparency.set", &json!({"blend": "multiply"})).unwrap();
    let o = node(&s, inst).extras().symbol_overrides[0].clone();
    assert_eq!(o.opacity, Some(0.5));
    assert_eq!(o.blend, Some(vectorcraft_color::BlendMode::Multiply));
    // A static symbol's instance refuses selectPart, so transparency.set acts on the instance whole.
    let mut t = session();
    let static_inst = pair(&mut t, "static");
    t.execute("select.set", &json!({"ids": [static_inst.0]})).unwrap();
    t.execute("transparency.set", &json!({"opacity": 40})).unwrap();
    assert!(node(&t, static_inst).extras().symbol_overrides.is_empty());
    assert!((node(&t, static_inst).opacity - 0.4).abs() < 1e-6);
}

#[test]
fn select_part_refuses_hidden_and_locked_objects_of_the_art() {
    let mut s = session();
    let (a, b, c) = (square(&mut s, 10.0), square(&mut s, 60.0), square(&mut s, 110.0));
    let group = id(&s.execute("object.group", &json!({"ids": [b.0, c.0]})).unwrap());
    let inst = id(&s.execute("symbol.new", &json!({"name": "Three", "ids": [a.0, group.0], "type": "dynamic"})).unwrap());
    // The art's first square is hidden and its group locked.
    let mut art = (*s.doc().unwrap().doc.symbols[0].art).clone();
    let children = art.children_mut().unwrap();
    std::sync::Arc::make_mut(&mut children[0]).visible = false;
    std::sync::Arc::make_mut(&mut children[1]).locked = true;
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).symbols[0].art = std::sync::Arc::new(art);
    for path in [json!([0]), json!([1]), json!([1, 0])] {
        let e = s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": path})).unwrap_err().to_string();
        assert!(e.contains("hidden or locked"), "{path}: {e}");
    }
    assert!(s.doc().unwrap().selection.symbol_part.is_none(), "nothing is picked");
    // With the art's group unlocked, a locked instance's objects are still not picked.
    let mut art = (*s.doc().unwrap().doc.symbols[0].art).clone();
    std::sync::Arc::make_mut(&mut art.children_mut().unwrap()[1]).locked = false;
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).symbols[0].art = std::sync::Arc::new(art);
    s.execute("object.lock", &json!({"ids": [inst.0]})).unwrap();
    let e = s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1, 1]})).unwrap_err().to_string();
    assert!(e.contains("hidden or locked"), "locked instance: {e}");
    s.execute("object.unlockAll", &json!({})).unwrap();
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1, 1]})).unwrap();
}

#[test]
fn choosing_the_selection_tool_again_picks_the_instance_whole() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.select_tool("selection", ViewInfo::default()).unwrap();
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    assert!(s.doc().unwrap().selection.symbol_part.is_some());
    let before = node(&s, inst).kind.clone();
    let revision = s.doc().unwrap().revision;
    s.select_tool("selection", ViewInfo::default()).unwrap();
    assert!(s.doc().unwrap().selection.symbol_part.is_none());
    assert_ne!(s.doc().unwrap().revision, revision, "panels cached by revision read the instance");
    s.execute("object.nudge", &json!({"dx": 1})).unwrap();
    assert_ne!(node(&s, inst).kind, before, "the instance moves");
}

#[test]
fn the_eyedropper_sample_taken_for_a_picked_object_is_kept() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    let green = square(&mut s, 150.0);
    s.execute("paint.setFill", &json!({"ids": [green.0], "color": "#00ff00"})).unwrap();
    let target = square(&mut s, 150.0);
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": green.0})).unwrap();
    assert_eq!(s.paint.fill.color().map(|c| c.to_hex()), Some("#00ff00".into()), "new art takes the sample's fill");
    // A reverse Eyedropper click with nothing selected applies that sample.
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(s.execute("appearance.copyFrom", &json!({"source": target.0, "reverse": true})).unwrap()["ids"], json!([target.0]));
    assert_eq!(node(&s, target).appearance.fill_paint().color().unwrap().to_hex(), "#00ff00");
}

#[test]
fn invert_and_complement_recolor_a_picked_object_for_its_instance_only() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    let other = id(&s.execute("symbol.place", &json!({"name": "Pair", "x": 55, "y": 80})).unwrap());
    let art = s.doc().unwrap().doc.symbols[0].art.clone();
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    let undo = undo_steps(&s);
    let r = s.execute("paint.invert", &json!({"stroke": false})).unwrap();
    assert_eq!(r["changed"], 1, "{r}");
    assert_eq!(undo_steps(&s), undo + 1, "one undo step");
    assert_eq!(symbol_part_fill(&s, inst), "#ffff00", "the picked object's own blue, inverted");
    assert_eq!(overrides(&s, inst), 1, "only the picked object changes");
    assert_eq!(overrides(&s, other), 0, "the other instance keeps the symbol's look");
    assert!(*s.doc().unwrap().doc.symbols[0].art == *art, "the symbol's art stays");
    assert_eq!(pixel(&s, 80, 30), [255, 255, 0, 255], "the changed object");
    assert_eq!(pixel(&s, 30, 30), BLUE, "its sibling keeps the symbol's look");
    s.execute("paint.complement", &json!({"stroke": false})).unwrap();
    assert_eq!(symbol_part_fill(&s, inst), "#0000ff", "the complement of yellow");
    assert_eq!(undo_steps(&s), undo + 2, "one undo step");
}

/// One change holds one paint, so Invert and Complement are refused for type whose characters
/// differ in color, and each character's color stays. A fill gives all of them one color.
#[test]
fn invert_and_complement_are_refused_for_picked_type_whose_characters_differ() {
    let mut s = session();
    let t = id(&s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hi"})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [t.0], "color": "#ff0000"})).unwrap();
    let inst = id(&s.execute("symbol.new", &json!({"name": "Word", "ids": [t.0], "type": "dynamic"})).unwrap());
    let mut art = (*s.doc().unwrap().doc.symbols[0].art).clone();
    let NodeKind::Text(tx) = &mut art.kind else { panic!("type: {art:?}") };
    let mut second = tx.runs[0].clone();
    second.style.fill = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 1.0));
    tx.runs.push(second);
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).symbols[0].art = std::sync::Arc::new(art);
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": []})).unwrap();
    let undo = undo_steps(&s);
    for cmd in ["paint.invert", "paint.complement"] {
        let e = s.execute(cmd, &json!({"stroke": false})).unwrap_err().to_string();
        assert!(e.contains(REFUSED), "{cmd}: {e}");
    }
    assert_eq!((overrides(&s, inst), undo_steps(&s)), (0, undo), "nothing changes");
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    s.execute("paint.invert", &json!({"stroke": false})).unwrap();
    assert_eq!(symbol_part_fill(&s, inst), "#ff00ff", "one color inverts");
}

/// The color of the fill instance `inst`'s first change sets.
fn symbol_part_fill(s: &Session, inst: NodeId) -> String {
    let o = node(s, inst).extras().symbol_overrides[0].clone();
    o.fill.as_ref().and_then(|p| p.color()).map(|c| c.to_hex()).unwrap_or_else(|| panic!("a solid fill: {o:?}"))
}

#[test]
fn a_stroke_on_a_picked_type_object_without_a_weight_draws() {
    let mut s = session();
    let t = id(&s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hi"})).unwrap());
    s.execute("paint.setStroke", &json!({"ids": [t.0], "none": true})).unwrap();
    let inst = id(&s.execute("symbol.new", &json!({"name": "Word", "ids": [t.0], "type": "dynamic"})).unwrap());
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": []})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    let part = s.doc().unwrap().selected_symbol_part().unwrap();
    let NodeKind::Text(tx) = &part.kind else { panic!("type: {part:?}") };
    assert!(tx.runs.iter().all(|r| r.style.stroke_width == 1.0 && !r.style.stroke.is_none()), "{:?}", tx.runs);
}

/// The colors the Fill and Stroke proxies show, as `paint.proxies` replies them.
fn proxy_colors(s: &mut Session) -> (Option<String>, Option<String>) {
    let r = s.execute("paint.proxies", &json!({})).unwrap();
    assert_eq!((r["fillMixed"].clone(), r["strokeMixed"].clone()), (json!(false), json!(false)), "{r}");
    let (fill, stroke) = s.proxy_paints();
    assert_eq!((r["fill"].clone(), r["stroke"].clone()), (json!(fill), json!(stroke)));
    (fill.color().map(|c| c.to_hex()), stroke.color().map(|c| c.to_hex()))
}

#[test]
fn the_fill_and_stroke_proxies_show_the_picked_object() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    assert_eq!(proxy_colors(&mut s), (Some("#0000ff".into()), None), "the symbol's own paint");
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#00ff00"})).unwrap();
    assert_eq!(proxy_colors(&mut s), (Some("#ff0000".into()), Some("#00ff00".into())), "the object's own paint");
    // The other object of the same instance shows the symbol's paint.
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [0]})).unwrap();
    assert_eq!(proxy_colors(&mut s), (Some("#0000ff".into()), None));
}

#[test]
fn selecting_a_guide_too_lets_go_of_the_picked_object() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    let g = s.execute("guide.add", &json!({"vertical": true, "pos": 150})).unwrap()["index"].as_u64().unwrap();
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    s.execute("guide.select", &json!({"indexes": [g], "toggle": true})).unwrap();
    let sel = s.doc().unwrap().selection.clone();
    assert_eq!((sel.objects, sel.guides.len(), sel.symbol_part), (vec![inst], 1, None));
    // Paint now goes to the instance, not to the object.
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    assert_eq!(overrides(&s, inst), 0);
    // Pruning a selection with a guide in it lets go of the object as well.
    let mut sel = s.doc().unwrap().selection.clone();
    sel.symbol_part = Some(vectorcraft_doc::SymbolPart { instance: inst, path: vec![1] });
    sel.prune(&s.doc().unwrap().doc);
    assert_eq!(sel.symbol_part, None);
}

/// A dynamic symbol with 9-slice guides at 25%: a 100 × 100 frame at (50, 50), a 10 × 10 square
/// in its top-left corner and one in its center; its instance resized to 300 × 100.
fn sliced_panel() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let rect =
        |s: &mut Session, x: f64, y: f64, w: f64| id(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": w})).unwrap()).0;
    let ids = [rect(&mut s, 50.0, 50.0, 100.0), rect(&mut s, 50.0, 50.0, 10.0), rect(&mut s, 95.0, 95.0, 10.0)];
    let inst = id(&s.execute("symbol.new", &json!({"name": "Panel", "ids": ids, "type": "dynamic", "nineSlice": true})).unwrap());
    s.execute("object.setBounds", &json!({"x": 50, "y": 50, "width": 300, "height": 100, "reference": 0})).unwrap();
    (s, inst)
}

#[test]
fn direct_selection_picks_and_outlines_the_object_where_a_9_slice_instance_draws_it() {
    let (mut s, inst) = sliced_panel();
    let part_box = |s: &Session, path: &[u32]| {
        let d = &s.doc().unwrap().doc;
        let o = vectorcraft_doc::hit::symbol_part_outline(d, d.node(inst).unwrap(), path).unwrap();
        vectorcraft_geom::kurbo::Shape::bounding_box(&o)
    };
    let close = |a: Rect, b: Rect| [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() < 1e-6);
    // The center cell (75..125 of the art) is drawn at 75..325, so the center square at 175..225;
    // placed by the instance's transform alone it would be at 185..215.
    assert!(close(part_box(&s, &[2]), Rect::new(175.0, 95.0, 225.0, 105.0)), "{:?}", part_box(&s, &[2]));
    assert!(close(part_box(&s, &[1]), Rect::new(50.0, 50.0, 60.0, 60.0)), "the corner keeps its size: {:?}", part_box(&s, &[1]));
    direct(&mut s, (177.0, 100.0), (177.0, 100.0), Mods::default());
    assert_eq!(s.doc().unwrap().selection.symbol_part, Some(vectorcraft_doc::SymbolPart { instance: inst, path: vec![2] }));
    direct(&mut s, (165.0, 100.0), (165.0, 100.0), Mods::default());
    let picked = s.doc().unwrap().selection.symbol_part.clone().map(|p| p.path);
    assert_ne!(picked, Some(vec![2]), "where the square is not drawn");
}

/// An object that is drawn once in each 9-slice cell it reaches, rather than cut at the guides
/// (here area type), is outlined where its copies are drawn.
#[test]
fn the_outline_of_an_object_drawn_in_each_9_slice_cell_matches_what_is_drawn() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let frame = id(&s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 100, "height": 100})).unwrap()).0;
    let area = json!({"width": 80, "height": 10});
    let text = id(&s.execute("text.create", &json!({"x": 60, "y": 95, "area": area, "size": 6, "text": "Hi"})).unwrap()).0;
    let ids = [frame, text];
    let inst = id(&s.execute("symbol.new", &json!({"name": "Bar", "ids": ids, "type": "dynamic", "nineSlice": true})).unwrap());
    s.execute("object.setBounds", &json!({"x": 50, "y": 50, "width": 300, "height": 100, "reference": 0})).unwrap();
    let d = &s.doc().unwrap().doc;
    let o = vectorcraft_doc::hit::symbol_part_outline(d, d.node(inst).unwrap(), &[1]).unwrap();
    let b = vectorcraft_geom::kurbo::Shape::bounding_box(&o);
    // The left copy is drawn at 60..75, the center one (placed whole it reaches 0..400) at 75..325
    // and the right one at 325..340.
    let want = Rect::new(60.0, 95.0, 340.0, 105.0);
    assert!([(b.x0, want.x0), (b.y0, want.y0), (b.x1, want.x1), (b.y1, want.y1)].iter().all(|(x, y)| (x - y).abs() < 1e-6), "{b:?}");
}

#[test]
fn direct_selection_picks_a_static_instance_whole_as_before() {
    let mut s = session();
    let inst = pair(&mut s, "static");
    direct(&mut s, (80.0, 30.0), (80.0, 30.0), Mods::default());
    let sel = s.doc().unwrap().selection.clone();
    assert_eq!((sel.objects, sel.symbol_part), (vec![inst], None));
    assert!(s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).is_err(), "a static symbol");
    // A dynamic one: Alt picks the instance whole, and a path naming no object is refused.
    s.execute("symbol.options", &json!({"name": "Pair", "type": "dynamic"})).unwrap();
    direct(&mut s, (80.0, 30.0), (80.0, 30.0), Mods { alt: true, ..Mods::default() });
    assert!(s.doc().unwrap().selection.symbol_part.is_none());
    assert!(s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [2]})).is_err());
    assert!(s.execute("symbol.instance.selectPart", &json!({"path": [1]})).is_err(), "no id");
    // Making the symbol static again lets go of the picked object.
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    s.execute("symbol.options", &json!({"name": "Pair", "type": "static"})).unwrap();
    assert!(s.doc().unwrap().selection.symbol_part.is_none());
}

#[test]
fn dynamic_symbols_and_their_changes_round_trip_and_old_documents_save_as_before() {
    let mut s = session();
    pair(&mut s, "static");
    let json = serde_json::to_string(&s.doc().unwrap().doc).unwrap();
    assert!(!json.contains("\"dynamic\"") && !json.contains("symbol_overrides"), "nothing new is written for static symbols");
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.setAppearance", &json!({"id": inst.0, "path": [1], "fill": "#ff0000", "opacity": 25})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
    assert!(back.symbols[0].dynamic);
    assert_eq!(back.node(inst).unwrap().extras().symbol_overrides, doc.node(inst).unwrap().extras().symbol_overrides);
    assert_eq!(back.symbols, doc.symbols);
}

#[test]
fn a_swatch_edit_recolors_a_picked_objects_own_paints() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#ff0000", "global": true})).unwrap();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    s.execute("paint.setFill", &json!({"swatch": "Brand"})).unwrap();
    assert_eq!(pixel(&s, 80, 30), RED);
    s.execute("swatch.edit", &json!({"name": "Brand", "color": "#00ff00"})).unwrap();
    let fill = s.doc().unwrap().selected_symbol_part().unwrap().appearance.fill_paint();
    assert_eq!(fill.color().map(|c| c.to_hex()), Some("#00ff00".into()), "{fill:?}");
    assert_eq!(pixel(&s, 80, 30), [0, 255, 0, 255]);
}

#[test]
fn a_swatch_or_color_only_a_picked_objects_paint_uses_counts_as_used() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#ff0000", "global": true})).unwrap();
    let inst = pair(&mut s, "dynamic");
    let unused = |s: &mut Session| s.execute("swatch.unused", &json!({})).unwrap()["names"].clone();
    assert!(unused(&mut s).as_array().unwrap().contains(&json!("Brand")));
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    s.execute("paint.setFill", &json!({"swatch": "Brand"})).unwrap();
    assert!(!unused(&mut s).as_array().unwrap().contains(&json!("Brand")), "only the object's own paint uses it");
    let colors = s.execute("recolor.colors", &json!({"ids": [inst.0]})).unwrap();
    assert!(colors["colors"].as_array().unwrap().iter().any(|c| c["hex"] == "#ff0000" && c["swatch"] == "Brand"), "{colors}");
    // A swatch edit maps the same paint once.
    s.execute("swatch.edit", &json!({"name": "Brand", "color": "#00ff00"})).unwrap();
    assert_eq!(symbol_part_fill(&s, inst), "#00ff00");
}

/// The Eyedropper takes a picked object's paints only from a path, compound path or type.
#[test]
fn the_eyedropper_refuses_a_source_that_is_not_painted_art() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    let other = id(&s.execute("symbol.place", &json!({"name": "Pair", "x": 55, "y": 80})).unwrap());
    let (a, b) = (square(&mut s, 150.0), square(&mut s, 150.0));
    let group = id(&s.execute("object.group", &json!({"ids": [a.0, b.0]})).unwrap());
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    let (fill, stroke, own, undo) = (s.paint.fill.clone(), s.paint.stroke.clone(), node(&s, inst).extras().symbol_overrides.clone(), undo_steps(&s));
    s.eyedropper_sample = None;
    for source in [inst, other, group] {
        let e = s.execute("appearance.copyFrom", &json!({"source": source.0})).unwrap_err().to_string();
        assert!(e.contains(REFUSED), "{e}");
    }
    // A click with the tool on an instance, the picked one included.
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    s.select_tool("eyedropper", v).unwrap();
    let e = s.pointer(&PointerEvent::new(PointerKind::Down, 80.0, 30.0), v).unwrap_err().to_string();
    assert!(e.contains(REFUSED), "{e}");
    s.pointer(&PointerEvent::new(PointerKind::Up, 80.0, 30.0), v).unwrap();
    assert_eq!(node(&s, inst).extras().symbol_overrides, own, "the object keeps its paints");
    assert_eq!(undo_steps(&s), undo, "no undo step");
    assert_eq!((&s.paint.fill, &s.paint.stroke), (&fill, &stroke), "the defaults for new art stay");
    assert!(s.eyedropper_sample.is_none(), "nothing was sampled");
    assert_eq!(pixel(&s, 80, 30), RED);
}

/// A path of a compound path gives the paint of the compound path, which holds it.
#[test]
fn the_eyedropper_samples_a_compound_path_through_its_member() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    let (a, b) = (square(&mut s, 120.0), square(&mut s, 170.0));
    let compound = id(&s.execute("object.compoundPath.make", &json!({"ids": [a.0, b.0]})).unwrap());
    s.execute("paint.setFill", &json!({"ids": [compound.0], "color": "#00ff00"})).unwrap();
    let member = node(&s, compound).children().unwrap()[0].id;
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    s.execute("appearance.copyFrom", &json!({"source": member.0})).unwrap();
    assert_eq!(symbol_part_fill(&s, inst), "#00ff00");
    assert_eq!(pixel(&s, 80, 30), [0, 255, 0, 255]);
}

/// A Cmd-drag lends the Selection tool, or with Alt Group Selection, which act on the whole instance.
#[test]
fn a_lent_selection_tool_moves_the_instance_of_a_picked_object() {
    let cmd = Mods { cmd: true, ..Mods::default() };
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    direct(&mut s, (80.0, 30.0), (80.0, 30.0), Mods::default());
    assert!(s.doc().unwrap().selection.symbol_part.is_some());
    let before = node(&s, inst).kind.clone();
    direct(&mut s, (80.0, 30.0), (100.0, 40.0), cmd);
    assert!(s.doc().unwrap().selection.symbol_part.is_none());
    assert_ne!(node(&s, inst).kind, before, "the instance moves");

    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    direct(&mut s, (80.0, 30.0), (80.0, 30.0), Mods::default());
    direct(&mut s, (80.0, 30.0), (80.0, 30.0), Mods { alt: true, ..cmd });
    assert!(s.doc().unwrap().selection.symbol_part.is_none(), "Group Selection selects the instance whole");
    assert_eq!(s.doc().unwrap().selection.objects, [inst]);
    // A plain Direct Selection click picks the object again once the tool is back.
    direct(&mut s, (80.0, 30.0), (80.0, 30.0), Mods::default());
    assert!(s.doc().unwrap().selection.symbol_part.is_some());
}

/// The commands that change nothing in the document read the instance of a picked object, so the
/// Attributes and CSS Properties panels and the others keep showing it.
#[test]
fn read_only_commands_read_the_instance_of_a_picked_object() {
    let mut failures = Vec::new();
    for spec in crate::cmd::command_specs().iter().filter(|c| !c.journal && crate::runs_with_empty_params(c.id)) {
        let mut s = session();
        let inst = pair(&mut s, "dynamic");
        s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
        let doc = s.doc().unwrap().doc.clone();
        if let Err(e) = s.execute(spec.id, &json!({}))
            && e.to_string().contains("symbol instance")
        {
            failures.push(format!("{}: {e}", spec.id));
        }
        if s.doc().is_ok_and(|st| st.doc != doc) {
            failures.push(format!("{}: the document changed", spec.id));
        }
        // A command run after one of these is refused as before.
        if s.doc().is_ok_and(|st| st.selection.symbol_part.is_some()) && s.execute("object.hide", &json!({})).is_ok() {
            failures.push(format!("{}: the refusal stays off afterward", spec.id));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    s.execute("css.selection", &json!({})).unwrap();
    s.execute("attributes.info", &json!({})).unwrap();
}

/// Export CSS writes a file and changes nothing in the document, so it reads the instance of a
/// picked object as it reads an instance selected whole, and the next edit is refused as before.
#[test]
fn css_export_reads_the_instance_of_a_picked_object() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("select.set", &json!({"ids": [inst.0]})).unwrap();
    let whole = s.execute("css.export", &json!({})).unwrap();
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let picked = s.execute("css.export", &json!({})).unwrap();
    assert_eq!(picked["rules"], 1);
    assert_eq!(picked["data"], whole["data"]);
    assert!(s.doc().unwrap().doc == doc, "the document changed");
    assert!(s.doc().unwrap().selection.symbol_part.is_some());
    assert!(s.execute("object.hide", &json!({})).is_err());
}

/// A drag that moved nothing leaves Transform Again repeating the last real transformation.
#[test]
fn a_refused_drag_of_a_picked_object_is_not_kept_for_transform_again() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    direct(&mut s, (80.0, 30.0), (80.0, 30.0), Mods::default());
    assert!(s.doc().unwrap().selection.symbol_part.is_some());
    direct(&mut s, (80.0, 30.0), (120.0, 60.0), Mods::default());
    assert!(s.doc().unwrap().last_transform.is_none(), "{:?}", s.doc().unwrap().last_transform);
    assert_eq!(s.doc().unwrap().selection.objects, [inst]);
}

/// A bad `target` or `aligned` on a paint command is refused with an object picked too.
#[test]
fn bad_paint_parameters_leave_a_picked_object_alone() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    let undo = undo_steps(&s);
    for (cmd, p) in [
        ("paint.setFill", json!({"color": "#ff0000", "target": "bogus"})),
        ("paint.setStroke", json!({"color": "#ff0000", "target": "bogus"})),
        ("paint.invert", json!({"target": "bogus"})),
        ("paint.complement", json!({"target": "bogus"})),
        ("paint.setFill", json!({"color": "#ff0000", "aligned": "yes"})),
    ] {
        let e = s.execute(cmd, &p).unwrap_err().to_string();
        assert!(e.contains("`target` must") || e.contains("aligned"), "{cmd} {p}: {e}");
        assert_eq!(overrides(&s, inst), 0, "{cmd} {p}");
    }
    assert_eq!(undo_steps(&s), undo, "no undo step");
}

/// An object picked while Replace read its file: the instance selected when Place started is
/// selected again first, so a replay of the journal replaces it.
#[test]
fn a_replace_that_finishes_after_an_object_was_picked_selects_the_instance_again() {
    let mut s = session();
    let inst = pair(&mut s, "dynamic");
    s.execute("select.set", &json!({"ids": [inst.0]})).unwrap();
    let svg = "<svg xmlns='http://www.w3.org/2000/svg' width='9' height='9'><rect width='9' height='9'/></svg>";
    let p = json!({"name": "a.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes()), "replace": true});
    let read = s.prepare_place(&p).unwrap().read().unwrap();
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    s.place_read(read).unwrap();
    assert!(s.doc().unwrap().doc.node(inst).is_none());
    let last: Vec<_> = s.journal.iter().rev().take(2).rev().map(|(id, _)| id.clone()).collect();
    assert_eq!(last, ["select.set", "file.place"]);
}
