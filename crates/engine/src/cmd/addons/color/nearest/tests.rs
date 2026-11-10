//! Find Nearest Swatch.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::NodeId;

use crate::Session;

const FIND: &str = "addon.color.nearestSwatch";
const APPLY: &str = "addon.color.applyNearestSwatch";

/// A document with a global swatch "Brand" and a selected rectangle filled `fill`, stroked `stroke`.
fn doc(fill: &str, stroke: Option<&str>) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s.execute("swatch.new", &json!({"name": "Brand", "color": "#ff8000", "global": true})).unwrap();
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color": fill})).unwrap();
    match stroke {
        Some(c) => s.execute("paint.setStroke", &json!({"color": c})).unwrap(),
        None => s.execute("paint.setStroke", &json!({"none": true})).unwrap(),
    };
    (s, id)
}

fn paints(s: &Session, id: NodeId) -> (Paint, Paint) {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    (n.appearance.fill_paint(), n.appearance.items.iter().find(|i| !i.is_fill()).unwrap().paint().clone())
}

fn names(matches: &Value) -> Vec<&str> {
    matches.as_array().unwrap().iter().map(|m| m["name"].as_str().unwrap()).collect()
}

#[test]
fn the_closest_swatches_of_a_library_are_listed_closest_first() {
    let (mut s, _) = doc("#ff6634", None);
    let v = s.execute(FIND, &json!({"library": "web-safe-216", "count": 3})).unwrap();
    assert_eq!(v["library"], "web-safe-216");
    let colors = v["colors"].as_array().unwrap();
    assert_eq!(colors.len(), 1, "the fill; no stroke");
    assert_eq!((colors[0]["hex"].as_str(), colors[0]["values"].as_str()), (Some("#ff6634"), Some("R=255 G=102 B=52")));
    let m = &colors[0]["matches"];
    assert_eq!(names(m)[0], "#FF6633");
    let de: Vec<f64> = m.as_array().unwrap().iter().map(|x| x["deltaE"].as_f64().unwrap()).collect();
    assert_eq!(de.len(), 3);
    assert!(de[0] < 1.0 && de.windows(2).all(|w| w[0] <= w[1]), "{de:?}");
    // A colour given is looked up instead of the selection's.
    let v = s.execute(FIND, &json!({"library": "web-safe-216", "color": "#010203", "count": 1})).unwrap();
    assert_eq!(names(&v["colors"][0]["matches"]), ["#000000"]);
}

#[test]
fn the_document_swatches_are_the_default_library() {
    let (mut s, _) = doc("#ff8010", Some("#ff8010"));
    let v = s.execute(FIND, &json!({})).unwrap();
    assert_eq!(v["library"], "document");
    assert_eq!(v["colors"].as_array().unwrap().len(), 1, "fill and stroke share a colour: looked up once");
    assert_eq!(names(&v["colors"][0]["matches"])[0], "Brand");
    assert!(!names(&v["colors"][0]["matches"]).contains(&"[Registration]"));
}

#[test]
fn applying_links_each_colour_to_its_nearest_document_swatch_as_one_undo_step() {
    let (mut s, id) = doc("#ff8010", Some("#ff8010"));
    let undo = s.doc().unwrap().history.undo.len();
    let v = s.execute(APPLY, &json!({"stroke": false})).unwrap();
    assert_eq!((v["changed"].as_u64(), v["swatches"].clone(), v["added"].clone()), (Some(1), json!(["Brand"]), json!([])));
    let (fill, stroke) = paints(&s, id);
    assert!(matches!(&fill, Paint::Solid { swatch: Some(n), color, .. } if n == "Brand" && color.to_hex() == "#ff8000"), "{fill:?}");
    assert_eq!(stroke.color().unwrap().to_hex(), "#ff8010", "strokes left alone");
    let st = s.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.history.undo.last().unwrap().label.as_str()), (undo + 1, "Apply Nearest Swatch"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(paints(&s, id).0.color().unwrap().to_hex(), "#ff8010");
}

#[test]
fn a_library_spot_colour_joins_the_document_and_stays_spot() {
    let (mut s, id) = doc("#e01525", None);
    // A colour book of one spot colour, loaded as a user would load one they own.
    s.execute("swatch.new", &json!({"name": "Ink Red", "color": "#e01020", "spot": true})).unwrap();
    let data = s.execute("swatch.library.save", &json!({"names": ["Ink Red"]})).unwrap()["data"].as_str().unwrap().to_string();
    s.execute("swatch.delete", &json!({"names": ["Ink Red"]})).unwrap();
    let lib = s.execute("swatch.library.load", &json!({"data": data, "name": "Book.vcswatches"})).unwrap()["library"].as_str().unwrap().to_string();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    let v = s.execute(APPLY, &json!({"library": lib})).unwrap();
    assert_eq!((v["swatches"].clone(), v["added"].clone()), (json!(["Ink Red"]), json!(["Ink Red"])));
    let d = &s.doc().unwrap().doc;
    assert!(d.swatch("Ink Red").is_some_and(|w| w.spot && w.global));
    assert!(matches!(paints(&s, id).0, Paint::Solid { swatch: Some(n), .. } if n == "Ink Red"));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "adding and applying undo together");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.swatch("Ink Red").is_none());
    assert_eq!(paints(&s, id).0, Paint::solid(Color::from_hex("#e01525").unwrap()));
}

#[test]
fn bad_requests_are_refused() {
    let (mut s, _) = doc("#ff8010", None);
    for (p, why) in [
        (json!({"library": "nope"}), "no swatch library `nope`"),
        (json!({"library": "spectrum-gradients"}), "has no colours"),
        (json!({"count": 0}), "`count` must be from 1 to 50"),
        (json!({"color": "zzz"}), "`color` is not a colour"),
        (json!({"fill": "yes"}), "`fill` must be true or false"),
    ] {
        let e = s.execute(FIND, &p).unwrap_err().to_string();
        assert!(e.contains(why), "{p}: {e}");
    }
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute(FIND, &json!({})).unwrap_err().to_string().contains("select objects or give `color`"));
    assert!(s.execute(APPLY, &json!({})).is_err(), "nothing selected");
}
