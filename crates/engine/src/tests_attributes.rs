//! Window → Attributes (`attributes.set`, `attributes.info`, `path.setFillRule`, `path.reverse`),
//! the links SVG export writes for objects with a URL, and what objects keep for other software
//! (their `data-*` attributes and their attributes in other applications' namespaces).

use serde_json::{Value, json};
use vectorcraft_doc::{ImageMap, NodeKind};
use vectorcraft_geom::{Affine, FillRule};
use vectorcraft_render::{RenderOptions, Renderer};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 30}))["id"].as_u64().unwrap())
}

fn info(s: &mut Session, p: Value) -> Value {
    run(s, "attributes.info", p)
}

#[test]
fn each_attribute_sets_reads_back_and_undoes_in_one_step() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    let i = info(&mut s, json!({}));
    assert_eq!((i["overprintFill"].clone(), i["showCenter"].clone(), i["imageMap"].clone()), (json!(false), json!(true), json!("none")));
    assert_eq!((i["url"].clone(), i["note"].clone(), i["fillRule"].clone()), (json!(""), json!(""), json!("nonZero")));
    let changed = run(
        &mut s,
        "attributes.set",
        json!({"overprintFill": true, "overprintStroke": true, "showCenter": false, "imageMap": "Rectangle", "url": " https://example.com ", "note": "hello"}),
    );
    assert_eq!(changed["changed"], 1);
    let i = info(&mut s, json!({}));
    for (k, v) in [
        ("overprintFill", json!(true)),
        ("overprintStroke", json!(true)),
        ("showCenter", json!(false)),
        ("imageMap", json!("rectangle")),
        ("url", json!("https://example.com")),
        ("note", json!("hello")),
        ("recentUrls", json!(["https://example.com"])),
    ] {
        assert_eq!(i[k], v, "{k}");
    }
    let n = s.doc().unwrap().doc.node(a).unwrap().clone();
    let at = n.extras().attrs.as_deref().unwrap();
    assert_eq!((at.show_center, at.image_map, n.url()), (Some(false), ImageMap::Rectangle, Some("https://example.com")));
    assert!(!n.shows_center());
    // One undo step takes everything back, and the attributes go away entirely.
    run(&mut s, "edit.undo", json!({}));
    let n = s.doc().unwrap().doc.node(a).unwrap().clone();
    assert!(n.extras().attrs.is_none() && !n.has_overprint());
    // Nothing to change: no undo step; bad values are refused.
    let before = s.doc().unwrap().revision;
    assert_eq!(run(&mut s, "attributes.set", json!({"showCenter": true}))["changed"], 0);
    assert_eq!(s.doc().unwrap().revision, before);
    assert!(s.execute("attributes.set", &json!({"imageMap": "circle"})).is_err());
    assert!(s.execute("attributes.set", &json!({})).is_err());
    // Clearing the URL and note drops the attributes.
    run(&mut s, "attributes.set", json!({"url": "u", "note": "n"}));
    run(&mut s, "attributes.set", json!({"url": "", "note": ""}));
    assert!(s.doc().unwrap().doc.node(a).unwrap().extras().attrs.is_none());
}

#[test]
fn values_that_differ_read_null_and_recent_urls_keep_the_newest_first() {
    let mut s = session();
    let a = rect(&mut s, 0.0);
    let b = rect(&mut s, 50.0);
    run(&mut s, "attributes.set", json!({"ids": [a.0], "url": "https://a.example", "note": "a"}));
    run(&mut s, "attributes.set", json!({"ids": [b.0], "url": "https://b.example"}));
    run(&mut s, "attributes.set", json!({"ids": [a.0], "url": "https://a.example"}));
    let i = info(&mut s, json!({"ids": [a.0, b.0]}));
    assert_eq!((i["url"].clone(), i["note"].clone(), i["imageMap"].clone()), (Value::Null, Value::Null, json!("none")));
    assert_eq!(i["recentUrls"], json!(["https://a.example", "https://b.example"]));
    // A pen path doesn't show its centre by default; a rectangle does.
    let p = NodeId(run(&mut s, "path.create", json!({"anchors": [{"x": 0, "y": 0}, {"x": 10, "y": 10}]}))["id"].as_u64().unwrap());
    assert_eq!(info(&mut s, json!({"ids": [p.0]}))["showCenter"], false);
    assert_eq!(info(&mut s, json!({"ids": [p.0, a.0]}))["showCenter"], Value::Null);
}

/// A five-pointed star drawn as one self-intersecting path (its centre is wound twice).
fn star(s: &mut Session) -> NodeId {
    let (c, r) = (50.0f64, 45.0f64);
    let anchors: Vec<Value> = (0..5)
        .map(|i| {
            let a = (-90.0 + 144.0 * i as f64).to_radians();
            json!({"x": c + r * a.cos(), "y": c + r * a.sin()})
        })
        .collect();
    let id = NodeId(run(s, "path.create", json!({"anchors": anchors, "closed": true}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"ids": [id.0], "color": "#ff0000"}));
    run(s, "paint.setStroke", json!({"ids": [id.0], "none": true}));
    id
}

fn red_at_centre(s: &Session) -> bool {
    let doc = s.doc().unwrap().doc.clone();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let p = Renderer::new().render(&doc, 100, 100, Affine::IDENTITY, &opts).pixel(50, 52);
    p[0] > 200 && p[1] < 80
}

#[test]
fn an_even_odd_star_has_a_hollow_centre() {
    let mut s = session();
    let id = star(&mut s);
    assert!(red_at_centre(&s), "non-zero fills the centre");
    assert_eq!(run(&mut s, "path.setFillRule", json!({"rule": "evenOdd"}))["changed"], 1);
    assert!(matches!(s.doc().unwrap().doc.node(id).unwrap().kind, NodeKind::Path { rule: FillRule::EvenOdd, .. }));
    assert_eq!(info(&mut s, json!({}))["fillRule"], "evenOdd");
    assert!(!red_at_centre(&s), "even-odd leaves it empty");
    assert_eq!(run(&mut s, "path.setFillRule", json!({"rule": "evenOdd"}))["changed"], 0);
    run(&mut s, "edit.undo", json!({}));
    assert!(red_at_centre(&s));
    assert!(s.execute("path.setFillRule", &json!({"rule": "odd"})).is_err());
}

#[test]
fn fill_rule_reaches_compound_paths_and_the_paths_in_groups() {
    let mut s = session();
    let a = rect(&mut s, 0.0);
    let b = rect(&mut s, 50.0);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    run(&mut s, "object.compoundPath.make", json!({}));
    let compound = s.doc().unwrap().selection.objects[0];
    let d = rect(&mut s, 0.0);
    run(&mut s, "select.set", json!({"ids": [compound.0, d.0]}));
    run(&mut s, "object.group", json!({}));
    assert_eq!(run(&mut s, "path.setFillRule", json!({"rule": "evenOdd"}))["changed"], 2, "the compound path and the rectangle");
    let doc = &s.doc().unwrap().doc;
    assert!(matches!(doc.node(compound).unwrap().kind, NodeKind::Compound { rule: FillRule::EvenOdd, .. }));
    assert!(matches!(doc.node(d).unwrap().kind, NodeKind::Path { rule: FillRule::EvenOdd, .. }));
}

#[test]
fn reverse_path_direction_on_and_off_set_the_winding() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    let ccw = |s: &Session| s.doc().unwrap().doc.node(a).unwrap().path_data().unwrap().subpaths[0].signed_area() < 0.0;
    let start = ccw(&s);
    assert_eq!(info(&mut s, json!({}))["reversed"], start);
    // Off / On set it; asking again changes nothing.
    assert_eq!(run(&mut s, "path.reverse", json!({"reversed": !start}))["changed"], 1);
    assert_eq!(ccw(&s), !start);
    assert_eq!(info(&mut s, json!({}))["reversed"], !start);
    assert_eq!(run(&mut s, "path.reverse", json!({"reversed": !start}))["changed"], 0);
    // Without `reversed` it flips.
    run(&mut s, "path.reverse", json!({}));
    assert_eq!(ccw(&s), start);
    run(&mut s, "select.none", json!({}));
    assert!(s.execute("path.reverse", &json!({})).is_err(), "nothing to reverse");
}

#[test]
fn attributes_survive_a_native_round_trip() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    run(&mut s, "attributes.set", json!({"showCenter": false, "imageMap": "polygon", "url": "https://example.com/x", "note": "keep"}));
    let doc = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(back.node(a).unwrap().extras().attrs, doc.node(a).unwrap().extras().attrs);
    assert!(back.node(a).unwrap().extras().attrs.is_some());
}

#[test]
fn svg_export_links_objects_and_import_reads_the_links_back() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    rect(&mut s, 50.0);
    run(&mut s, "attributes.set", json!({"ids": [a.0], "url": "https://example.com/?a=1&b=2"}));
    let svg = vectorcraft_svg::export(&s.doc().unwrap().doc, &Default::default());
    assert_eq!(svg.matches("<a ").count(), 1, "{svg}");
    assert!(svg.contains("xlink:href=\"https://example.com/?a=1&amp;b=2\""), "{svg}");
    let back = vectorcraft_svg::import(&svg).unwrap();
    let mut urls = vec![];
    back.walk(|n| {
        if let NodeKind::Path { .. } = n.kind {
            urls.push(n.url().map(str::to_string));
        }
    });
    assert_eq!(urls, [Some("https://example.com/?a=1&b=2".to_string()), None], "the link wraps the path alone");
}

/// #522: an object's own data (SVG `data-*` attributes) is set with `object.setProps {data}`,
/// undone in one step, written to SVG and kept by the native format.
#[test]
fn object_data_is_set_saved_and_written_to_svg() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    let data = |s: &Session| s.doc().unwrap().doc.node(a).unwrap().extras().attrs.as_deref().map(|x| x.data.clone()).unwrap_or_default();
    let pair = |k: &str, v: &str| (k.to_string(), v.to_string());
    run(&mut s, "object.setProps", json!({"data": {"pivot": "60,100", "z": 3}}));
    assert_eq!(data(&s), [pair("pivot", "60,100"), pair("z", "3")], "values are text");
    run(&mut s, "object.setProps", json!({"data": {"data-pivot": "1,2", "z": null}}));
    assert_eq!(data(&s), [pair("pivot", "1,2")], "a data- prefix is optional; null removes a key");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(data(&s).len(), 2, "one step");
    for bad in [json!({"data": {"vc-x": "1"}}), json!({"data": {"name": "x"}}), json!({"data": {"a b": "1"}}), json!({"data": "pivot"})] {
        assert!(s.execute("object.setProps", &bad).is_err(), "{bad}");
    }
    let svg = run(&mut s, "document.serialize", json!({"format": "svg"}))["text"].as_str().unwrap().to_string();
    assert!(svg.contains("data-pivot=\"60,100\" data-z=\"3\""), "{svg}");
    let native = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let back = vectorcraft_format::load(&native).unwrap();
    assert_eq!(back.node(a).unwrap().extras().attrs, s.doc().unwrap().doc.node(a).unwrap().extras().attrs);
}

/// An object's attributes in other applications' XML namespaces are its own, as its `data-*`
/// attributes are: a pasted copy and a duplicate keep them, deleting the object takes them with
/// it, and the native file and SVG export keep them.
#[test]
fn other_applications_attributes_go_with_their_object() {
    let mut s = Session::new();
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:plotter="https://example.org/plotter" width="100" height="100">
        <rect width="20" height="20" plotter:pen="2"/></svg>"#;
    run(&mut s, "document.open", json!({"name": "pens.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())}));
    // The `plotter:pen` values on the document's objects.
    let pens = |d: &vectorcraft_doc::Document| {
        let mut out = vec![];
        d.walk(|n| out.extend(n.extras().attrs.iter().flat_map(|a| a.foreign.iter().map(|f| f.value.clone()))));
        out
    };
    assert_eq!(pens(&s.doc().unwrap().doc), ["2"]);
    run(&mut s, "select.all", json!({}));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(pens(&s.doc().unwrap().doc), ["2", "2"], "a pasted copy keeps them");
    run(&mut s, "edit.duplicate", json!({}));
    assert_eq!(pens(&s.doc().unwrap().doc), ["2", "2", "2"], "so does a duplicate");
    run(&mut s, "edit.clear", json!({}));
    assert_eq!(pens(&s.doc().unwrap().doc), ["2", "2"], "a deleted object leaves none behind");
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(pens(&back), ["2", "2"], "the native file keeps them");
    let out = run(&mut s, "document.serialize", json!({"format": "svg"}))["text"].as_str().unwrap().to_string();
    assert_eq!((out.matches("xmlns:plotter=\"https://example.org/plotter\"").count(), out.matches("plotter:pen=\"2\"").count()), (1, 2), "{out}");
}
