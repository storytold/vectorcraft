//! The Fill/Stroke proxies and the Appearance panel's active item: Invert / Complement and the "?"
//! state follow the active fill or stroke, objects without bounds still show their paints, and the
//! proxy focus can be set directly (M3.51).

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Node};
use vectorcraft_geom::PathData;

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

/// A selected rectangle with a red fill below its stroke and a blue fill on top: [Fill, Stroke, Fill].
fn two_fills(s: &mut Session, x: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 50, "height": 50})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    s.execute("appearance.setItem", &json!({"index": 2, "color": "#0000ff"})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn item_hex(s: &Session, id: NodeId, index: usize) -> String {
    s.doc().unwrap().doc.node(id).unwrap().appearance.items[index].paint().color().unwrap().to_hex()
}

#[test]
fn invert_and_complement_recolour_the_active_item() {
    let mut s = session();
    let id = two_fills(&mut s, 10.0);
    // Without an active item the proxy's colours go (every fill, as Edit Colors does).
    s.execute("paint.invert", &json!({})).unwrap();
    assert_eq!((item_hex(&s, id, 0), item_hex(&s, id, 2)), ("#00ffff".into(), "#ffff00".into()));
    s.execute("edit.undo", &json!({})).unwrap();
    // The bottom fill is the Appearance panel's active row: only it changes, as one undo step.
    s.execute("appearance.setActiveItem", &json!({"index": 0})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    assert_eq!(s.execute("paint.invert", &json!({})).unwrap(), json!({"changed": 1}));
    assert_eq!((item_hex(&s, id, 0), item_hex(&s, id, 2)), ("#00ffff".into(), "#0000ff".into()));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(s.proxy_paints().0.color().unwrap().to_hex(), "#00ffff", "the proxy shows the active row");
    assert_eq!(s.recent_colors[0].to_hex(), "#00ffff");
    s.execute("paint.complement", &json!({})).unwrap();
    assert_eq!((item_hex(&s, id, 0), item_hex(&s, id, 2)), ("#ff0000".into(), "#0000ff".into()));
    // The Stroke proxy isn't the active row's kind: the stroke is complemented as usual.
    s.execute("paint.complement", &json!({"stroke": true})).unwrap();
    assert_eq!(item_hex(&s, id, 1), "#000000", "black's complement is black");
    assert_eq!((item_hex(&s, id, 0), item_hex(&s, id, 2)), ("#ff0000".into(), "#0000ff".into()));
}

#[test]
fn the_mixed_proxy_compares_the_active_item() {
    let mut s = session();
    let a = two_fills(&mut s, 10.0);
    let b = two_fills(&mut s, 100.0);
    // Same top fills, different bottom fills.
    s.execute("appearance.setItem", &json!({"index": 0, "color": "#00ff00", "ids": [b.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    assert_eq!(s.proxy_mixed(), (false, false));
    s.execute("appearance.setActiveItem", &json!({"index": 0})).unwrap();
    assert_eq!(s.proxy_mixed(), (true, false), "the active bottom fills differ");
    assert_eq!(s.execute("paint.proxies", &json!({})).unwrap()["fillMixed"], true);
    // A stroke row leaves the fill proxy on the top fills.
    s.execute("appearance.setActiveItem", &json!({"index": 1})).unwrap();
    assert_eq!(s.proxy_mixed(), (false, false));
}

#[test]
fn an_object_without_bounds_shows_its_paints() {
    let mut s = session();
    let id = s
        .edit("Add", |d, _| {
            let l = d.layers[0].id;
            let id = d.alloc_id();
            let ap = Appearance::basic(Paint::solid(Color::from_hex("#123456").unwrap()), Paint::solid(Color::from_hex("#654321").unwrap()), 2.0);
            d.insert(Some(l), 0, Node::path(id, PathData::default(), ap)).map_err(|e| crate::EngineError::Other(e.to_string()))
        })
        .unwrap();
    assert!(s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().is_none());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let (f, st) = s.proxy_paints();
    assert_eq!((f.color().unwrap().to_hex(), st.color().unwrap().to_hex()), ("#123456".into(), "#654321".into()));
}

#[test]
fn toggle_active_can_name_the_proxy() {
    let mut s = Session::new();
    assert_eq!(s.execute("paint.toggleActive", &json!({"fill": false})).unwrap(), json!({"fillActive": false}));
    assert_eq!(s.execute("paint.toggleActive", &json!({"fill": false})).unwrap(), json!({"fillActive": false}), "already in front");
    assert_eq!(s.execute("paint.toggleActive", &json!({})).unwrap(), json!({"fillActive": true}));
}

#[test]
fn the_last_colour_keeps_its_model() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"c": 0.1, "m": 0.9, "y": 0.2, "k": 0.0}})).unwrap();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    s.execute("paint.lastColor", &json!({})).unwrap();
    assert_eq!(s.proxy_paints().0.color(), Some(Color::cmyk(0.1, 0.9, 0.2, 0.0)));
}
