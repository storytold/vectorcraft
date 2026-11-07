//! Inline graphics in text drawn on the canvas and written to SVG and PDF: a symbol (a red
//! circle drawn in code) set inline in a line of type.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind, TextObject};
use vectorcraft_geom::Rect;
use vectorcraft_testkit::fixtures::{self, exec, id_of};
use vectorcraft_testkit::raster;

/// A session with a red circle symbol "Dot" (its instance deleted) and the point type
/// "Tap  now" (20 pt, baseline at y = 100) with the symbol inline after "Tap ".
fn setup(inline: bool) -> (vectorcraft_engine::Session, NodeId) {
    let mut s = fixtures::session_with(400.0, 300.0);
    fixtures::ellipse(&mut s, 300.0, 250.0, 20.0, 20.0);
    exec(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    exec(&mut s, "paint.setStroke", json!({"none": true}));
    exec(&mut s, "symbol.new", json!({"name": "Dot"}));
    exec(&mut s, "edit.clear", json!({}));
    // Type is painted with the current fill: black, not the circle's red.
    exec(&mut s, "paint.setFill", json!({"color": "#000000"}));
    let id = id_of(&exec(&mut s, "text.create", json!({"x": 20, "y": 100, "text": "Tap  now", "size": 20})));
    if inline {
        exec(&mut s, "text.insertInline", json!({"id": id.0, "at": 4, "symbol": "Dot"}));
    }
    exec(&mut s, "select.none", json!({}));
    (s, id)
}

fn doc(s: &vectorcraft_engine::Session) -> Document {
    (*s.doc().unwrap().doc).clone()
}

fn text(d: &Document, id: NodeId) -> TextObject {
    match &d.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

/// Where the inline art lands, in document space.
fn art_rect(d: &Document, id: NodeId) -> Rect {
    let t = text(d, id);
    let l = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert_eq!(l.inlines.len(), 1);
    t.xf.transform_rect_bbox(l.inlines[0].bounds)
}

fn red(p: [u8; 3]) -> bool {
    p[0] > 200 && p[1] < 80 && p[2] < 80
}

#[test]
fn canvas_draws_the_symbol_where_the_layout_puts_it() {
    let (s, id) = setup(true);
    let d = doc(&s);
    let r = art_rect(&d, id);
    // 20 pt tall at 20 pt type, on the pen after "Tap ", centred on the cap-height middle.
    assert!((r.height() - 20.0).abs() < 0.5 && (r.width() - 20.0).abs() < 0.5, "{r:?}");
    assert!(r.x0 > 50.0 && r.x0 < 70.0, "{r:?}");
    assert!(r.center().y < 100.0 && r.center().y > 90.0, "{r:?}");
    let img = raster::render_region(&d, Rect::new(0.0, 0.0, 200.0, 150.0), 2.0);
    let px = |x: f64, y: f64| img.over_white((x * 2.0) as u32, (y * 2.0) as u32);
    assert!(red(px(r.center().x, r.center().y)), "{:?} at {:?}", px(r.center().x, r.center().y), r.center());
    // Only there: just outside the circle, no red.
    assert!(!red(px(r.x0 - 3.0, r.center().y)) && !red(px(r.center().x, r.y0 - 3.0)));
    // Without the graphic nothing is red, and the text after it sits further left.
    let (plain, pid) = setup(false);
    let pd = doc(&plain);
    let img = raster::render_region(&pd, Rect::new(0.0, 0.0, 200.0, 150.0), 2.0);
    assert_eq!(img.count(|p| p[0] > 200 && p[1] < 80 && p[2] < 80 && p[3] > 200), 0);
    let w = text(&d, id).bounds().unwrap().width() - text(&pd, pid).bounds().unwrap().width();
    assert!((w - 20.0).abs() < 0.5, "{w}");
}

#[test]
fn canvas_applies_the_text_objects_opacity_to_its_inline_art() {
    let (mut s, id) = setup(true);
    exec(&mut s, "select.set", json!({"ids": [id.0]}));
    exec(&mut s, "transparency.set", json!({"opacity": 50}));
    let d = doc(&s);
    let r = art_rect(&d, id);
    let img = raster::render_region(&d, Rect::new(0.0, 0.0, 200.0, 150.0), 2.0);
    let p = img.over_white((r.center().x * 2.0) as u32, (r.center().y * 2.0) as u32);
    // Half red over white: pink.
    assert!(p[0] > 240 && (100..180).contains(&p[1]) && (100..180).contains(&p[2]), "{p:?}");
}

fn svg_of(s: &mut vectorcraft_engine::Session, params: Value) -> String {
    let v = exec(s, "document.serialize", params);
    v["text"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| String::from_utf8(vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()).unwrap())
}

#[test]
fn svg_keeps_live_text_and_draws_the_symbol() {
    let (mut s, id) = setup(true);
    let svg = svg_of(&mut s, json!({"format": "svg", "outlineText": false}));
    // Live text without the object replacement character; the text after the graphic is placed
    // after it.
    assert!(svg.contains("<text") && svg.contains(">Tap") && svg.contains("now<"), "{svg}");
    assert!(!svg.contains('\u{FFFC}'), "{svg}");
    // The symbol's art: a <symbol> def and a <use> of it.
    assert!(svg.contains("<symbol id=\"Dot\""), "{svg}");
    assert!(svg.matches("xlink:href=\"#Dot\"").count() == 1, "{svg}");
    // Re-imported, the art is a red circle where the canvas draws it.
    let back = vectorcraft_svg::import(&svg).unwrap();
    let img = raster::render_region(&back, Rect::new(0.0, 0.0, 200.0, 150.0), 2.0);
    let r = art_rect(&doc(&s), id);
    let p = img.over_white((r.center().x * 2.0) as u32, (r.center().y * 2.0) as u32);
    assert!(red(p), "{p:?}");
    // Text as outlines and fewer tspans still draw it.
    for params in [json!({"format": "svg", "outlineText": true}), json!({"format": "svg", "outlineText": false, "fewerTspans": true})] {
        let svg = svg_of(&mut s, params.clone());
        assert!(svg.contains("xlink:href=\"#Dot\""), "{params}: {svg}");
        assert!(!svg.contains('\u{FFFC}'), "{params}");
    }
}

#[test]
fn pdf_draws_the_symbol_art() {
    let (mut s, id) = setup(true);
    let v = exec(&mut s, "document.serialize", json!({"format": "pdf"}));
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    let back = vectorcraft_pdf::import(&bytes).unwrap();
    let r = art_rect(&doc(&s), id);
    let img = raster::render_region(&back, Rect::new(0.0, 0.0, 200.0, 150.0), 2.0);
    let p = img.over_white((r.center().x * 2.0) as u32, (r.center().y * 2.0) as u32);
    assert!(red(p), "{p:?} at {:?}", r.center());
    // Without it, nothing red.
    let (mut plain, _) = setup(false);
    let v = exec(&mut plain, "document.serialize", json!({"format": "pdf"}));
    let back = vectorcraft_pdf::import(&vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    let img = raster::render_region(&back, Rect::new(0.0, 0.0, 200.0, 150.0), 2.0);
    assert_eq!(img.count(|p| p[0] > 200 && p[1] < 80 && p[2] < 80 && p[3] > 200), 0);
}
