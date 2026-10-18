//! File → Place (`file.place`, `file.place.info`, `image.info`) and the place cursor
//! (`file.place.queue` and the `place` tool).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{AppearanceItem, Node, NodeKind};
use vectorcraft_geom::Rect;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

/// A `w`×`h` PNG of one colour (`rgba`), declaring `ppi` when given.
fn png(w: u32, h: u32, rgba: [u8; 4], ppi: Option<f64>) -> Vec<u8> {
    let mut out = vec![];
    image::RgbaImage::from_pixel(w, h, image::Rgba(rgba)).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
    match ppi {
        Some(r) => cmd::fileio::ppi::with_png_resolution(&out, (r, r)),
        None => out,
    }
}

fn file(name: &str, bytes: &[u8]) -> Value {
    json!({ "name": name, "dataBase64": vectorcraft_format::base64_encode(bytes) })
}

/// `file.place` of `bytes` named `name`, with `extra` params.
fn place(s: &mut Session, name: &str, bytes: &[u8], extra: Value) -> Value {
    let mut p = file(name, bytes);
    if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
        o.extend(e);
    }
    s.execute("file.place", &p).unwrap()
}

fn node(s: &Session, id: &Value) -> Node {
    s.doc().unwrap().doc.node(NodeId(id.as_u64().unwrap())).unwrap().clone()
}

fn top_children(s: &Session) -> Vec<Arc<Node>> {
    s.doc().unwrap().doc.layers.last().unwrap().children().unwrap().clone()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

fn serialize(s: &mut Session, format: &str) -> Vec<u8> {
    let v = s.execute("document.serialize", &json!({"format": format})).unwrap();
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
}

fn eps_of(s: &mut Session) -> Vec<u8> {
    let v = s.execute("document.exportEps", &json!({})).unwrap();
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
}

/// `file.place.info`'s width and height for `bytes` named `name`, with `extra` params.
fn info_size(s: &mut Session, name: &str, bytes: &[u8], extra: Value) -> (f64, f64) {
    let mut p = file(name, bytes);
    if let (Some(o), Value::Object(e)) = (p.as_object_mut(), extra) {
        o.extend(e);
    }
    let r = s.execute("file.place.info", &p).unwrap();
    (r["width"].as_f64().unwrap(), r["height"].as_f64().unwrap())
}

/// The bounds of what the active document's art draws ([`vectorcraft_render::encode::art_bounds`])
/// lie within a millionth of a point of `want`.
fn drawn_at(s: &Session, want: Rect) -> bool {
    let b = vectorcraft_render::encode::art_bounds(&s.doc().unwrap().doc).unwrap_or_default();
    [b.x0 - want.x0, b.y0 - want.y0, b.x1 - want.x1, b.y1 - want.y1].iter().all(|d| d.abs() < 1e-6)
}

/// A 400 × 300 pt document holding one group: a 40 × 40 pt square at (20, 20) and, hidden, a
/// 170 × 42 pt rectangle at (100, 100). Returns it with the square's visual bounds.
fn group_with_a_hidden_member() -> (Session, Rect) {
    let mut s = session();
    let square = s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap()["id"].clone();
    let wide = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 170, "height": 42})).unwrap()["id"].clone();
    s.execute("object.group", &json!({"ids": [square, wide]})).unwrap();
    s.execute("object.hide", &json!({"ids": [wide]})).unwrap();
    assert!(!node(&s, &wide).visible);
    let shown = node(&s, &square).visual_bounds().unwrap();
    (s, shown)
}

/// How many of `n` and the objects in it are hidden.
fn hidden_in(n: &Node) -> usize {
    let mut hidden = 0;
    n.walk(&mut |c| hidden += usize::from(!c.visible));
    hidden
}

#[test]
fn malformed_place_coordinates_are_rejected_instead_of_silently_changed() {
    let mut s = session();
    let png = png(4, 2, [20, 40, 60, 255], None);
    let original = s.doc().unwrap().doc.layers.clone();
    // Previously filter_map dropped the string from a five-element rectangle and
    // treated the remaining four values as a different, valid placement.
    for rect in [json!([0, "ignored", 10, 40, 50]), json!([10, 20, 40, null, 50]), json!([10, 20, 40, 50, 60])] {
        let mut params = file("tiny.png", &png);
        params["rect"] = rect;
        assert!(s.execute("file.place", &params).is_err(), "{params}");
        assert_eq!(s.doc().unwrap().doc.layers, original, "failed Place must not add art");
    }
    // An explicitly supplied invalid position formerly behaved as though no
    // position were supplied and centred the image on the artboard.
    for at in [json!(["not-a-number", 20]), json!([10, null]), json!([10, 20, 30]), json!("invalid")] {
        let mut params = file("tiny.png", &png);
        params["at"] = at;
        assert!(s.execute("file.place", &params).is_err(), "{params}");
        assert_eq!(s.doc().unwrap().doc.layers, original);
    }
    let mut params = file("tiny.png", &png);
    params["rect"] = json!([10, 20, 80, 60]);
    let placed = s.execute("file.place", &params).unwrap();
    assert_eq!(placed["ids"].as_array().unwrap().len(), 1);
    // A null position is no position, as for the other optional params.
    let mut params = file("tiny.png", &png);
    params["at"] = Value::Null;
    assert_eq!(s.execute("file.place", &params).unwrap()["ids"].as_array().unwrap().len(), 1);
}

#[test]
fn a_300_px_png_at_300_ppi_places_as_72_pt_centred_selected_and_undoable() {
    let mut s = session();
    let r = place(&mut s, "photo.png", &png(300, 150, [255, 0, 0, 255], Some(300.0)), json!({}));
    assert_eq!((r["format"].as_str(), r["linked"].as_bool()), (Some("png"), Some(false)), "bytes have no path to link");
    assert!(close(r["width"].as_f64().unwrap(), 72.0) && close(r["height"].as_f64().unwrap(), 36.0), "{r}");
    let id = &r["ids"][0];
    let n = node(&s, id);
    assert_eq!(n.name.as_deref(), Some("photo.png"));
    let b = n.geometric_bounds().unwrap();
    assert!(close(b.center().x, 200.0) && close(b.center().y, 150.0), "centred on the artboard: {b:?}");
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, [NodeId(id.as_u64().unwrap())]);
    assert_eq!((st.history.undo.len(), st.history.undo[0].label.as_str()), (1, "Place"));
    // No resolution: 72 ppi, one point per pixel; `at` centres it there.
    let r = place(&mut s, "plain.png", &png(30, 20, [0, 0, 255, 255], None), json!({"at": [50, 60]}));
    let b = node(&s, &r["ids"][0]).geometric_bounds().unwrap();
    assert!(close(b.width(), 30.0) && close(b.center().x, 50.0) && close(b.center().y, 60.0), "{b:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(top_children(&s).is_empty(), "each Place is one undo step");
}

#[test]
fn rect_fits_the_art_inside_keeping_its_aspect() {
    let mut s = session();
    let r = place(&mut s, "wide.png", &png(40, 20, [0, 0, 0, 255], None), json!({"rect": [10, 10, 100, 100]}));
    let b = node(&s, &r["ids"][0]).geometric_bounds().unwrap();
    assert!(close(b.width(), 100.0) && close(b.height(), 50.0) && close(b.center().y, 60.0), "{b:?}");
}

#[test]
fn svg_with_a_data_png_renders_and_leaves_the_clipboard_alone() {
    let mut s = session();
    let rect = s.execute("shape.rectangle", &json!({"x": 300, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
    s.execute("select.set", &json!({"ids": [rect]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    let clipboard = s.clipboard.clone();
    let red = vectorcraft_format::base64_encode(&png(4, 2, [255, 0, 0, 255], None));
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><image width="40" height="20" href="data:image/png;base64,{red}"/><rect width="10" height="10" fill="#0000ff"/></svg>"##
    );
    let r = place(&mut s, "art.svg", svg.as_bytes(), json!({"at": [100, 100]}));
    assert_eq!(r["format"], "svg");
    let g = node(&s, &r["ids"][0]);
    let NodeKind::Group { children, clip: false } = &g.kind else { panic!("one group: {:?}", g.kind) };
    assert_eq!(children.len(), 2);
    assert_eq!(g.name.as_deref(), Some("art.svg"));
    assert_eq!(s.clipboard, clipboard, "Place never touches the clipboard");
    let doc = &s.doc().unwrap().doc;
    assert!(doc.images.len() == 1, "the embedded image joined the document");
    // The group spans 80..120 × 90..110: blue square top left, red image elsewhere.
    let img = vectorcraft_testkit::raster::render_region(doc, Rect::new(80.0, 90.0, 120.0, 110.0), 1.0);
    assert_eq!(img.over_white(30, 15), [255, 0, 0]);
    assert_eq!(img.over_white(4, 4), [0, 0, 255]);
}

#[test]
fn replace_keeps_the_transform_and_the_stacking_place() {
    let mut s = session();
    let first = place(&mut s, "a.png", &png(100, 50, [255, 0, 0, 255], None), json!({"at": [150, 150]}))["ids"][0].clone();
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    s.execute("object.scale", &json!({"sx": 50})).unwrap();
    let centre = node(&s, &first).geometric_bounds().unwrap().center();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    s.execute("select.set", &json!({"ids": [first]})).unwrap();
    assert!(s.execute("file.place", &json!({"replace": true, "at": [0, 0], "name": "b.png", "dataBase64": ""})).is_err());
    let r = place(&mut s, "b.png", &png(20, 20, [0, 255, 0, 255], Some(144.0)), json!({"replace": true}));
    let id = &r["ids"][0];
    assert_eq!(top_children(&s)[0].id.0, id.as_u64().unwrap(), "same place in the stacking order");
    assert_eq!(top_children(&s).len(), 2);
    let c = node(&s, id).geometric_bounds().unwrap().center();
    assert!(close(c.x, centre.x) && close(c.y, centre.y), "{c:?} vs {centre:?}");
    // 144 ppi at the old image's 50%: 288 ppi, rotated as it was.
    let info = s.execute("image.info", &json!({})).unwrap();
    assert!(close(info["ppi"][0].as_f64().unwrap(), 288.0) && close(info["ppi"][1].as_f64().unwrap(), 288.0), "{info}");
    let NodeKind::Image(im) = &node(&s, id).kind else { panic!() };
    let [a, b, ..] = im.xf.as_coeffs();
    assert!(close(b.atan2(a).to_degrees(), -30.0), "{:?}", im.xf);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(top_children(&s)[0].id.0, first.as_u64().unwrap(), "one undo step brings the old image back");
}

#[test]
fn replace_needs_exactly_one_selected_object() {
    let mut s = session();
    let r = s
        .execute("file.place", &json!({"replace": true, "name": "b.png", "dataBase64": vectorcraft_format::base64_encode(&png(2, 2, [0; 4], None))}));
    assert!(r.unwrap_err().to_string().contains("select the one object"));
}

#[test]
fn template_puts_the_art_on_a_locked_template_layer_below_the_current_one() {
    let mut s = session();
    s.execute("layer.new", &json!({})).unwrap();
    let current = s.doc().unwrap().current_layer().unwrap();
    let r = place(&mut s, "sketch.png", &png(10, 10, [0, 0, 0, 255], None), json!({"template": true}));
    let st = s.doc().unwrap();
    let layers = &st.doc.layers;
    assert_eq!(layers.len(), 3);
    let t = layers.iter().position(|l| l.name.as_deref() == Some("Template sketch.png")).unwrap();
    assert_eq!(layers[t + 1].id, current, "right below the current layer");
    assert!(layers[t].locked && matches!(layers[t].kind, NodeKind::Layer { template: true, .. }));
    assert_eq!(st.doc.layer_of(NodeId(r["ids"][0].as_u64().unwrap())), Some(layers[t].id));
    assert!(st.selection.is_empty(), "locked template art isn't selected");
    assert_eq!(st.history.undo.len(), 2, "layer.new, then Place as one step");
}

#[test]
fn pdf_pages_and_native_artboards_place_as_clipped_groups() {
    let mut src = Session::new();
    src.execute("file.new", &json!({"width": 100, "height": 50, "artboards": 2})).unwrap();
    src.execute("artboard.setProps", &json!({"index": 1, "width": 40})).unwrap();
    let b1 = src.doc().unwrap().doc.artboards[1].rect;
    src.execute("shape.rectangle", &json!({"x": b1.x0 + 5.0, "y": 5, "width": 20, "height": 10})).unwrap();
    let pdf = serialize(&mut src, "pdf");
    let native = serialize(&mut src, "vectorcraft");
    let mut s = session();
    for (name, bytes) in [("two.pdf", &pdf), ("two.vectorcraft", &native)] {
        let r = place(&mut s, name, bytes, json!({"page": 2}));
        assert!(close(r["width"].as_f64().unwrap(), 40.0) && close(r["height"].as_f64().unwrap(), 50.0), "{name}: {r}");
        let g = node(&s, &r["ids"][0]);
        assert!(matches!(g.kind, NodeKind::Group { clip: true, .. }), "{name}: clipped to the page");
        let r = place(&mut s, name, bytes, json!({"page": 2, "crop": "bounding"}));
        assert!((r["width"].as_f64().unwrap() - 20.0).abs() < 1.5, "{name}: the art's bounds: {r}");
        let err = s.execute("file.place", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(bytes), "page": 3})).unwrap_err();
        assert!(err.to_string().contains("has 2"), "{name}: {err}");
    }
}

/// With crop "bounding", the art's box is the box of what shows, as an export measures it: a hidden
/// member of a group does not count, and is still placed, hidden.
#[test]
fn bounding_box_placement_measures_only_the_art_that_shows() {
    let (mut src, shown) = group_with_a_hidden_member();
    let eps = eps_of(&mut src);
    let native = serialize(&mut src, "vectorcraft");
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="300"><g><rect x="20" y="20" width="40" height="40"/><rect x="100" y="100" width="170" height="42" style="display:none"/></g></svg>"#;
    // The sample's group `Pair` has a hidden member 42 pt tall; what shows is 170 × 40 pt.
    let sample = vectorcraft_testkit::ai::eps(&vectorcraft_testkit::ai::sample_data(), vectorcraft_testkit::ai::page_ps());
    let cases: [(&str, &[u8], (f64, f64)); 4] = [
        ("pair.eps", &eps, (shown.width(), shown.height())),
        ("pair.vectorcraft", &native, (shown.width(), shown.height())),
        ("pair.svg", svg, (40.0, 40.0)),
        ("art.eps", &sample, (170.0, 40.0)),
    ];
    for (name, bytes, (w, h)) in cases {
        let mut s = session();
        let (iw, ih) = info_size(&mut s, name, bytes, json!({"crop": "bounding"}));
        assert!(close(iw, w) && close(ih, h), "{name}: {iw} × {ih}, not {w} × {h}");
        // `rect` fits what shows, centered.
        let r = place(&mut s, name, bytes, json!({"crop": "bounding", "rect": [100, 100, 80, 80]}));
        let k = (80.0 / w).min(80.0 / h);
        let want = Rect::from_center_size((140.0, 140.0), (w * k, h * k));
        assert!(drawn_at(&s, want), "{name}: not {want:?}");
        assert_eq!(hidden_in(&node(&s, &r["ids"][0])), 1, "{name}: the hidden member is placed, hidden");
    }
}

/// With crop "bounding", an effect's reach counts in the art's box, as in an export of the art.
#[test]
fn bounding_box_placement_counts_a_drop_shadow() {
    let mut src = session();
    let id = src.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 40, "height": 40})).unwrap()["id"].clone();
    let square = node(&src, &id).visual_bounds().unwrap();
    src.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [id], "params": {"x": 8, "y": 0, "blur": 0}})).unwrap();
    let export = vectorcraft_render::encode::art_bounds(&src.doc().unwrap().doc).unwrap();
    assert!(export.width() > square.width() + 7.0, "{export:?}");
    let eps = eps_of(&mut src);
    let native = serialize(&mut src, "vectorcraft");
    let cases: [(&str, &[u8]); 2] = [("shadow.eps", &eps), ("shadow.vectorcraft", &native)];
    for (name, bytes) in cases {
        let mut s = session();
        let (w, h) = info_size(&mut s, name, bytes, json!({"crop": "bounding"}));
        assert!(close(w, export.width()) && close(h, export.height()), "{name}: {w} × {h}, not {export:?}");
    }
}

/// A symbol instance in the placed art is measured by its symbol's art, for the art's box and for
/// the size `file.place` answers.
#[test]
fn a_placed_symbol_instance_is_measured_by_its_art() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="400" height="300"><defs><symbol id="s"><rect width="100" height="10"/></symbol></defs><use xlink:href="#s" x="50" y="50"/></svg>"##;
    let mut s = session();
    assert_eq!(info_size(&mut s, "bar.svg", svg, json!({"crop": "bounding"})), (100.0, 10.0));
    let r = place(&mut s, "bar.svg", svg, json!({"crop": "bounding", "rect": [100, 100, 200, 200]}));
    assert!(close(r["width"].as_f64().unwrap(), 200.0) && close(r["height"].as_f64().unwrap(), 20.0), "{r}");
    assert!(drawn_at(&s, Rect::new(100.0, 190.0, 300.0, 210.0)));
    // Replace centers an image on it and gives a text file its size.
    let r = place(&mut s, "dot.png", &png(10, 10, [0, 0, 0, 255], None), json!({"replace": true}));
    let c = node(&s, &r["ids"][0]).geometric_bounds().unwrap().center();
    assert!(close(c.x, 200.0) && close(c.y, 200.0), "{c:?}");
    let r = place(&mut s, "bar.svg", svg, json!({"crop": "bounding", "rect": [100, 100, 200, 200]}));
    assert!(close(r["width"].as_f64().unwrap(), 200.0), "{r}");
    let r = place(&mut s, "note.txt", b"Hello", json!({"replace": true}));
    let NodeKind::Text(t) = &node(&s, &r["ids"][0]).kind else { panic!("area type") };
    let frame = t.area_frame().and_then(|f| f.bounds()).unwrap();
    assert!(close(frame.width(), 200.0) && close(frame.height(), 20.0), "{frame:?}");
}

/// Crop "crop" still frames the art by its artboard (an SVG by its viewBox), hidden members or not.
#[test]
fn crop_placement_keeps_the_artboard_with_a_hidden_member() {
    let (mut src, _) = group_with_a_hidden_member();
    let eps = eps_of(&mut src);
    let native = serialize(&mut src, "vectorcraft");
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 300"><g><rect x="20" y="20" width="40" height="40"/><rect x="100" y="100" width="170" height="42" style="display:none"/></g></svg>"#;
    let cases: [(&str, &[u8]); 3] = [("pair.eps", &eps), ("pair.vectorcraft", &native), ("pair.svg", svg)];
    for (name, bytes) in cases {
        let mut s = session();
        assert_eq!(info_size(&mut s, name, bytes, json!({"crop": "crop"})), (400.0, 300.0), "{name}");
        let r = place(&mut s, name, bytes, json!({"crop": "crop"}));
        assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(400.0), Some(300.0)), "{name}: {r}");
    }
}

/// A page's or artboard's art is the objects whose visible art lies on it: a group whose only part
/// on the artboard is hidden stays out, and a hidden object on the artboard is still placed, hidden.
#[test]
fn a_group_whose_only_part_on_the_artboard_is_hidden_is_left_out() {
    let mut src = session();
    let off = src.execute("shape.rectangle", &json!({"x": -300, "y": 20, "width": 40, "height": 40})).unwrap()["id"].clone();
    let under = src.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 170, "height": 42})).unwrap()["id"].clone();
    src.execute("object.group", &json!({"ids": [off, under]})).unwrap();
    src.execute("object.hide", &json!({"ids": [under]})).unwrap();
    let dot = src.execute("shape.ellipse", &json!({"x": 50, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
    let shown = node(&src, &dot).visual_bounds().unwrap();
    let ghost = src.execute("shape.rectangle", &json!({"x": 300, "y": 200, "width": 20, "height": 20})).unwrap()["id"].clone();
    src.execute("object.hide", &json!({"ids": [ghost]})).unwrap();
    let eps = eps_of(&mut src);
    let native = serialize(&mut src, "vectorcraft");
    let cases: [(&str, &[u8]); 2] = [("parts.eps", &eps), ("parts.vectorcraft", &native)];
    for (name, bytes) in cases {
        let mut s = session();
        let (w, h) = info_size(&mut s, name, bytes, json!({"crop": "bounding"}));
        assert!(close(w, shown.width()) && close(h, shown.height()), "{name}: {w} × {h}");
        let r = place(&mut s, name, bytes, json!({"crop": "bounding"}));
        let g = node(&s, &r["ids"][0]);
        let kids = g.children().unwrap();
        assert_eq!(kids.len(), 2, "{name}: the circle and the hidden square");
        assert!(kids[0].visible && !kids[1].visible, "{name}");
    }
}

#[test]
fn placed_art_brings_its_symbols_and_swatches_renaming_on_conflict() {
    // The file: a global "Brand" red rectangle and a "Star" symbol instance.
    let mut src = session();
    src.execute("swatch.new", &json!({"name": "Brand", "color": "#ff0000", "global": true})).unwrap();
    src.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    src.execute("paint.setFill", &json!({"swatch": "Brand"})).unwrap();
    src.execute("shape.ellipse", &json!({"x": 50, "y": 10, "width": 20, "height": 20})).unwrap();
    src.execute("symbol.new", &json!({"name": "Star"})).unwrap();
    let bytes = serialize(&mut src, "vectorcraft");
    // The document: its own, different "Brand" and "Star".
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#00ff00", "global": true})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
    s.execute("symbol.new", &json!({"name": "Star"})).unwrap();
    let r = place(&mut s, "logo.vectorcraft", &bytes, json!({"crop": "bounding"}));
    let g = node(&s, &r["ids"][0]);
    let (mut fill, mut symbol) = (None, None);
    g.walk(&mut |n| match &n.kind {
        NodeKind::SymbolInstance { symbol: name, .. } => symbol = Some(name.clone()),
        NodeKind::Path { .. } => {
            fill = n.appearance.items.iter().find_map(|i| if let AppearanceItem::Fill(f) = i { Some(f.paint.clone()) } else { None })
        }
        _ => {}
    });
    assert_eq!(symbol.as_deref(), Some("Star 2"));
    let doc = &s.doc().unwrap().doc;
    assert!(doc.symbols.iter().any(|x| x.name == "Star 2") && doc.symbols.iter().any(|x| x.name == "Star"));
    let Some(Paint::Solid { color, swatch: None, .. }) = fill else { panic!("unlinked: {fill:?}") };
    assert_eq!(color.to_hex(), "#ff0000", "keeps its look");
    assert_eq!(doc.swatch("Brand").and_then(|w| w.paint.color()).map(|c| c.to_hex()).as_deref(), Some("#00ff00"));
    // A document without them gets them as they are.
    let mut fresh = session();
    let r = place(&mut fresh, "logo.vectorcraft", &bytes, json!({}));
    let doc = &fresh.doc().unwrap().doc;
    assert!(doc.symbols.iter().any(|x| x.name == "Star") && doc.swatch("Brand").is_some());
    let mut linked = false;
    node(&fresh, &r["ids"][0]).walk(&mut |n| {
        linked |= n
            .appearance
            .items
            .iter()
            .any(|i| matches!(i, AppearanceItem::Fill(f) if matches!(&f.paint, Paint::Solid { swatch: Some(w), .. } if w == "Brand")));
    });
    assert!(linked, "still linked to the swatch it brought");
}

#[test]
fn info_reports_size_resolution_colour_mode_and_link() {
    let mut s = session();
    let mut gray = vec![];
    image::GrayImage::from_pixel(300, 150, image::Luma([128])).write_to(&mut std::io::Cursor::new(&mut gray), image::ImageFormat::Png).unwrap();
    let gray = cmd::fileio::ppi::with_png_resolution(&gray, (300.0, 300.0));
    let mut p = file("scan.png", &gray);
    p["thumbnail"] = json!(16);
    let info = s.execute("file.place.info", &p).unwrap();
    assert_eq!((info["colorMode"].as_str(), info["pixelWidth"].as_u64()), (Some("Grayscale"), Some(300)));
    assert_eq!(info["ppi"], json!([300.0, 300.0]));
    assert!(close(info["width"].as_f64().unwrap(), 72.0));
    let thumb = image::load_from_memory(&vectorcraft_format::base64_decode(info["thumbnailBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!((thumb.width(), thumb.height()), (16, 8));
    assert!(top_children(&s).is_empty(), "info places nothing");
    // A path is linked by default.
    let path = std::env::temp_dir().join(format!("vectorcraft-place-{}.png", std::process::id()));
    std::fs::write(&path, &gray).unwrap();
    let path = path.to_string_lossy().to_string();
    let r = s.execute("file.place", &json!({"path": path})).unwrap();
    assert_eq!(r["linked"], true);
    let im = s.execute("image.info", &json!({})).unwrap();
    assert_eq!((im["linked"].as_bool(), im["link"].as_str(), im["colorMode"].as_str()), (Some(true), Some(path.as_str()), Some("Grayscale")));
    assert!(close(im["ppi"][0].as_f64().unwrap(), 300.0), "{im}");
    let r = s.execute("file.place", &json!({"path": path, "link": false})).unwrap();
    assert_eq!(r["linked"], false);
    let _ = std::fs::remove_file(&path);
}

#[test]
fn a_queue_of_3_with_click_drag_and_esc_places_2() {
    let mut s = session();
    let view = ViewInfo::default();
    let files: Vec<Value> = ["a.png", "b.png", "c.png"].iter().map(|n| file(n, &png(40, 20, [9, 9, 9, 255], None))).collect();
    let r = s.execute("file.place.queue", &json!({"files": files, "thumbnail": 8})).unwrap();
    assert_eq!(r["count"], 3);
    assert!(r["files"][0]["thumbnailBase64"].is_string());
    assert_eq!(s.tool_id(), "place");
    assert_eq!(s.tool_options()["name"], "a.png");
    let ev = |kind, x, y| PointerEvent::new(kind, x, y);
    // Click: 100% with its top-left corner at the click.
    s.pointer(&ev(PointerKind::Down, 10.0, 10.0), view).unwrap();
    s.pointer(&ev(PointerKind::Up, 10.0, 10.0), view).unwrap();
    let a = s.doc().unwrap().selection.objects[0];
    assert_eq!(s.doc().unwrap().doc.node(a).unwrap().geometric_bounds(), Some(Rect::new(10.0, 10.0, 50.0, 30.0)));
    // Drag: the dragged size, aspect kept.
    s.pointer(&ev(PointerKind::Down, 100.0, 100.0), view).unwrap();
    s.pointer(&ev(PointerKind::Drag, 180.0, 110.0), view).unwrap();
    s.pointer(&ev(PointerKind::Up, 180.0, 110.0), view).unwrap();
    let b = s.doc().unwrap().selection.objects[0];
    assert_eq!(s.doc().unwrap().doc.node(b).unwrap().name.as_deref(), Some("b.png"));
    assert_eq!(s.doc().unwrap().doc.node(b).unwrap().geometric_bounds(), Some(Rect::new(100.0, 100.0, 180.0, 140.0)));
    // Esc discards the last one and asks for the previous tool back.
    assert!(s.tool_claims_key(ToolKey::Escape, view));
    let reqs = s.tool_key(ToolKey::Escape, Mods::default(), view).unwrap();
    assert_eq!(reqs, vec![UiRequest::SwitchTool("selection".into())]);
    assert_eq!(top_children(&s).len(), 2);
    assert_eq!(s.doc().unwrap().history.undo.len(), 2, "one undo step per placement");
}

#[test]
fn bad_place_params_are_errors() {
    let mut s = session();
    let ok = vectorcraft_format::base64_encode(&png(2, 2, [0; 4], None));
    for p in [
        json!({}),
        json!({"name": "x.png", "dataBase64": "%%%"}),
        json!({"name": "notes.xyz", "dataBase64": vectorcraft_format::base64_encode(b"hello")}),
        json!({"name": "x.png", "dataBase64": ok, "rect": [0, 0, 0, 10]}),
        json!({"name": "x.png", "dataBase64": ok, "at": [1e12, 0]}),
        json!({"name": "x.png", "dataBase64": ok, "page": 0}),
        json!({"name": "x.png", "dataBase64": ok, "crop": "trim"}),
        json!({"name": "x.png", "dataBase64": ok, "template": true, "replace": true}),
    ] {
        assert!(s.execute("file.place", &p).is_err(), "{p}");
    }
    assert!(s.execute("file.place.queue", &json!({})).is_err());
    assert!(s.execute("file.place.queue", &json!({"files": [{"name": "notes.txt", "dataBase64": ""}]})).is_err());
    assert_eq!(s.tool_id(), "selection", "a failed queue leaves the tool alone");
    assert!(top_children(&s).is_empty());
}

/// An SVG placed with `crop: "crop"` given is framed and clipped by its viewBox, as a PDF by its
/// page, so art drawn on a shared canvas keeps its place and scale; by default (and with
/// "bounding") it is fitted by its art's bounds (#788).
#[test]
fn an_svg_placed_with_crop_is_framed_by_its_viewbox() {
    let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 1000"><path d="M200 300 L600 300 L600 800 L200 800 Z"/></svg>"#;
    let path_bounds = |s: &Session, id: &Value| {
        let mut b = None;
        node(s, id).walk(&mut |n| {
            if let NodeKind::Path { clipping: false, .. } = n.kind {
                b = n.visual_bounds();
            }
        });
        b.unwrap()
    };
    let near = |a: Rect, b: Rect| [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|d| d.abs() < 1e-6);
    let mut s = session();
    let r = place(&mut s, "glyph.svg", svg, json!({"rect": [100, 100, 200, 200], "crop": "crop"}));
    assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(200.0), Some(200.0)), "{r}");
    let id = &r["ids"][0];
    assert!(near(path_bounds(&s, id), Rect::new(140.0, 160.0, 220.0, 260.0)), "{:?}", path_bounds(&s, id));
    assert!(matches!(node(&s, id).kind, NodeKind::Group { clip: true, .. }), "clipped to the viewBox");
    for extra in [json!({"rect": [100, 100, 200, 200]}), json!({"rect": [100, 100, 200, 200], "crop": "bounding"})] {
        let mut s = session();
        let r = place(&mut s, "glyph.svg", svg, extra);
        assert!(near(path_bounds(&s, &r["ids"][0]), Rect::new(120.0, 100.0, 280.0, 300.0)), "{r}");
    }
}

/// An SVG whose top-level group is clipped is placed as one clip group: the clip is kept, and its
/// clipping path is not left behind as a loose path.
#[test]
fn a_clipped_top_level_group_stays_clipped() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100" viewBox="0 0 200 100">
  <defs><clipPath id="c"><path d="M0 0 L200 0 L200 100 L0 100 Z"/></clipPath></defs>
  <g id="Layer_1" clip-path="url(#c)"><path d="M100 -20 L250 -20 L250 40 L100 40 Z" fill="#f00"/></g>
</svg>"##;
    let mut s = session();
    let r = place(&mut s, "c.svg", svg, json!({"link": false}));
    assert_eq!((r["width"].as_f64().unwrap(), r["height"].as_f64().unwrap()), (200.0, 100.0), "{r}");
    let placed = node(&s, &r["ids"][0]);
    let NodeKind::Group { children, clip: false } = &placed.kind else { panic!("{:?}", placed.kind) };
    let [inner] = children.as_slice() else { panic!("one object: {children:?}") };
    assert!(matches!(&inner.kind, NodeKind::Group { children, clip: true } if children.len() == 2), "{:?}", inner.kind);
}
