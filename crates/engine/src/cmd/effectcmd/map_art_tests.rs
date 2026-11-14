#![allow(clippy::unwrap_used, clippy::panic)]
use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use crate::Session;

/// A 100 × 100 square with `effect` seen straight on, and a 10 × 20 red rectangle beside it kept
/// as symbol "Mark" (11 × 21 with its stroke).
fn setup(effect: &str) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":400,"height":400})).unwrap();
    let mark = s.execute("path.create", &json!({"d":"M300 300L310 300L310 320L300 320Z"})).unwrap()["id"].clone();
    s.execute("paint.setFill", &json!({"color":"#ff0000"})).unwrap();
    s.execute("threeD.addGraphic", &json!({"name": "Mark", "ids": [mark]})).unwrap();
    let id = NodeId(s.execute("path.create", &json!({"d":"M100 100L200 100L200 200L100 200Z"})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color":"#338ad6"})).unwrap();
    s.execute("effect.apply", &json!({"effect":effect,"item":null,"params":{"rotationX":0.0,"rotationY":0.0,"rotationZ":0.0}})).unwrap();
    (s, id)
}

fn params(s: &Session, id: NodeId) -> Value {
    s.doc().unwrap().doc.node(id).unwrap().appearance.effects[0].params.clone()
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn map(surface: u64, x: f64, scale: (f64, f64), rotation: f64) -> Value {
    json!({"surface": surface, "symbol": "Mark", "x": x, "y": 0.0, "scaleX": scale.0, "scaleY": scale.1, "rotation": rotation})
}

#[test]
fn map_art_sets_edits_and_removes_a_surfaces_map_one_undo_step_each() {
    let (mut s, id) = setup("threeD.classicExtrude");
    let before = s.doc().unwrap().doc.clone();
    let history = undo_len(&s);
    let r = s.execute("threeD.mapArt", &json!({"surface": 1, "symbol": "Mark", "x": 5.0, "rotation": 30.0})).unwrap();
    assert_eq!(r["ids"], json!([id.0]));
    assert_eq!(r["maps"], json!([map(1, 5.0, (100.0, 100.0), 30.0)]));
    assert_eq!(params(&s, id)["mapArt"], json!([map(1, 5.0, (100.0, 100.0), 30.0)]));
    assert_eq!(undo_len(&s), history + 1);
    // Without a symbol the surface's map keeps its own, and only the values given change.
    s.execute("threeD.mapArt", &json!({"surface": 1, "scaleX": 50.0})).unwrap();
    assert_eq!(params(&s, id)["mapArt"], json!([map(1, 5.0, (50.0, 100.0), 30.0)]));
    // Another surface takes a map of its own, kept in the order of the surfaces.
    s.execute("threeD.mapArt", &json!({"surface": 4, "symbol": "Mark"})).unwrap();
    s.execute("threeD.mapArt", &json!({"surface": 2, "symbol": "Mark"})).unwrap();
    let surfaces: Vec<u64> = params(&s, id)["mapArt"].as_array().unwrap().iter().map(|m| m["surface"].as_u64().unwrap()).collect();
    assert_eq!(surfaces, [1, 2, 4]);
    // Shade Artwork and Invisible Geometry are the effect's own flags.
    s.execute("threeD.mapArt", &json!({"shade": true, "invisible": true})).unwrap();
    assert_eq!((params(&s, id)["mapShade"].clone(), params(&s, id)["mapInvisible"].clone()), (json!(true), json!(true)));
    // A null symbol clears one surface, and clearAll every surface.
    s.execute("threeD.mapArt", &json!({"surface": 2, "symbol": null})).unwrap();
    assert_eq!(params(&s, id)["mapArt"].as_array().unwrap().len(), 2);
    let r = s.execute("threeD.mapArt", &json!({"clearAll": true})).unwrap();
    assert_eq!((r["maps"].clone(), params(&s, id)["mapArt"].clone()), (json!([]), json!([])));
    // With a surface, clearAll leaves that surface's new map alone.
    s.execute("threeD.mapArt", &json!({"surface": 1, "symbol": "Mark"})).unwrap();
    s.execute("threeD.mapArt", &json!({"surface": 3, "symbol": "Mark", "clearAll": true})).unwrap();
    assert_eq!(params(&s, id)["mapArt"], json!([map(3, 0.0, (100.0, 100.0), 0.0)]));
    // Without a symbol, clearAll keeps the surface's own map and edits it.
    s.execute("threeD.mapArt", &json!({"surface": 1, "symbol": "Mark"})).unwrap();
    s.execute("threeD.mapArt", &json!({"surface": 3, "x": 4.0, "clearAll": true})).unwrap();
    assert_eq!(params(&s, id)["mapArt"], json!([map(3, 4.0, (100.0, 100.0), 0.0)]));
    assert_eq!(undo_len(&s), history + 11);
    for _ in 0..11 {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    assert_eq!(s.doc().unwrap().doc.layers, before.layers);
}

#[test]
fn scale_to_fit_centers_the_art_and_fills_the_surface() {
    let (mut s, id) = setup("threeD.classicExtrude");
    // The cap is 100 × 100 and the symbol's art 11 × 21.
    s.execute("threeD.mapArt", &json!({"surface": 1, "symbol": "Mark", "x": 20.0, "y": 3.0, "fit": true})).unwrap();
    let m = &params(&s, id)["mapArt"][0];
    assert_eq!((m["x"].as_f64(), m["y"].as_f64()), (Some(0.0), Some(0.0)));
    assert!((m["scaleX"].as_f64().unwrap() - 100.0 / 11.0 * 100.0).abs() < 1e-6, "{m}");
    assert!((m["scaleY"].as_f64().unwrap() - 100.0 / 21.0 * 100.0).abs() < 1e-6, "{m}");
    // A side wall is 100 along the outline and as deep as the solid, 50.
    s.execute("threeD.mapArt", &json!({"surface": 3, "symbol": "Mark", "fit": true})).unwrap();
    let m = &params(&s, id)["mapArt"][1];
    assert!((m["scaleY"].as_f64().unwrap() - 50.0 / 21.0 * 100.0).abs() < 1e-6, "{m}");
}

#[test]
fn a_change_that_changes_nothing_records_no_undo_step() {
    let (mut s, id) = setup("threeD.classicExtrude");
    s.execute("threeD.mapArt", &json!({"surface": 1, "symbol": "Mark"})).unwrap();
    let history = undo_len(&s);
    let same = params(&s, id);
    for p in [
        json!({"surface": 1, "symbol": "Mark"}),
        json!({"surface": 1, "x": 0.0, "scaleX": 100.0}),
        json!({"surface": 2, "symbol": null}),
        json!({"shade": false}),
        json!({"invisible": false}),
    ] {
        let r = s.execute("threeD.mapArt", &p).unwrap();
        assert_eq!(r["ids"], json!([]), "{p}");
        assert_eq!((undo_len(&s), params(&s, id)), (history, same.clone()), "{p}");
    }
    // Clearing an object with no map records nothing either.
    let (mut s, _) = setup("threeD.classicRevolve");
    let history = undo_len(&s);
    assert_eq!(s.execute("threeD.mapArt", &json!({"clearAll": true})).unwrap()["ids"], json!([]));
    assert_eq!(undo_len(&s), history);
}

#[test]
fn bad_surfaces_symbols_and_values_are_refused() {
    let (mut s, id) = setup("threeD.classicExtrude");
    let history = undo_len(&s);
    let maps = params(&s, id)["mapArt"].clone();
    for (p, error) in [
        (json!({}), "give `surface`, `clearAll`, `shade` or `invisible`"),
        (json!({"surface": 0, "symbol": "Mark"}), "`surface` must be a whole number"),
        (json!({"surface": 1.5, "symbol": "Mark"}), "`surface` must be a whole number"),
        (json!({"surface": "front", "symbol": "Mark"}), "`surface` must be a whole number"),
        // The fill's solid has 6 surfaces and the stroke's, a square ring, 10: two caps and the
        // four walls of each of its two outlines.
        (json!({"surface": 17, "symbol": "Mark"}), "`surface` must be from 1 to 16"),
        (json!({"surface": 1, "symbol": "Gone"}), "no symbol named `Gone`"),
        (json!({"surface": 1, "symbol": 3}), "needs `symbol`"),
        (json!({"surface": 1}), "surface 1 has no map: give `symbol`"),
        (json!({"surface": 1, "symbol": "Mark", "scaleX": 0}), "scaleX must be a number from 1 to 10000"),
        (json!({"surface": 1, "symbol": "Mark", "scaleY": 10001}), "scaleY must be a number from 1 to 10000"),
        (json!({"surface": 1, "symbol": "Mark", "rotation": -361}), "rotation must be a number from -360 to 360"),
        (json!({"surface": 1, "symbol": "Mark", "x": 1e7}), "x must be a number"),
        (json!({"surface": 1, "symbol": "Mark", "y": "up"}), "y must be a number"),
        (json!({"surface": 1, "symbol": "Mark", "fit": 1}), "fit"),
        (json!({"symbol": "Mark"}), "need `surface`"),
        (json!({"fit": true}), "need `surface`"),
        (json!({"shade": "on"}), "shade"),
        (json!({"invisible": 1}), "invisible"),
        (json!({"clearAll": "yes"}), "clearAll"),
        (json!({"surface": 1, "symbol": "Mark", "face": "front"}), "unknown parameter `face`"),
        (json!({"surface": 1, "symbol": "Mark", "item": null}), "`item` needs `index`"),
        (json!({"surface": 1, "symbol": "Mark", "index": 3}), "needs a 3D effect at index 3"),
    ] {
        let e = s.execute("threeD.mapArt", &p).unwrap_err().to_string();
        assert!(e.contains(error), "{p}: {e}");
    }
    assert_eq!((undo_len(&s), params(&s, id)["mapArt"].clone()), (history, maps));
    // Revolve (Classic) has one surface, and the other 3D effects take no map.
    let (mut s, _) = setup("threeD.classicRevolve");
    s.execute("threeD.mapArt", &json!({"surface": 1, "symbol": "Mark"})).unwrap();
    let e = s.execute("threeD.mapArt", &json!({"surface": 2, "symbol": "Mark"})).unwrap_err().to_string();
    assert!(e.contains("`surface` must be from 1 to 1"), "{e}");
    for effect in ["threeD.classicRotate", "threeD.extrude", "threeD.revolve"] {
        let (mut s, _) = setup(effect);
        for p in [json!({"surface": 1, "symbol": "Mark"}), json!({"surface": 1, "symbol": "Mark", "index": 0})] {
            let e = s.execute("threeD.mapArt", &p).unwrap_err().to_string();
            assert!(e.contains("Map Art maps symbols onto Extrude & Bevel (Classic) and Revolve (Classic)"), "{effect}: {e}");
        }
        // The effect's own parameters refuse a map too.
        let e = s.execute("effect.setParams", &json!({"index": 0, "item": null, "params": {"mapArt": [{"surface": 1, "symbol": "Mark"}]}}));
        assert!(e.is_err(), "{effect}");
    }
}

#[test]
fn a_mapped_symbol_is_drawn_expanded_renamed_and_kept_in_use() {
    let (mut s, id) = setup("threeD.classicExtrude");
    s.execute("threeD.mapArt", &json!({"surface": 1, "symbol": "Mark", "scaleX": 300.0})).unwrap();
    // effect.setParams takes the same maps and refuses a symbol the document does not have.
    let e = s.execute("effect.setParams", &json!({"index": 0, "item": null, "params": {"mapArt": [{"surface": 1, "symbol": "Gone"}]}}));
    assert!(e.unwrap_err().to_string().contains("Gone"));
    // The symbol counts as used, and renaming it renames it in the map.
    assert_eq!(s.execute("symbol.unused", &json!({})).unwrap()["names"], json!([]));
    s.execute("symbol.options", &json!({"name": "Mark", "newName": "Logo"})).unwrap();
    assert_eq!(params(&s, id)["mapArt"][0]["symbol"], "Logo");
    // Expand Appearance keeps the mapped art: the red fill is among the expanded paths.
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    let mut red = 0;
    for l in &d.layers {
        l.walk(&mut |n| {
            for item in &n.appearance.items {
                if let vectorcraft_doc::AppearanceItem::Fill(f) = item
                    && f.paint.color().is_some_and(|c| c.to_rgb()[0] > 0.9 && c.to_rgb()[1] < 0.1 && c.to_rgb()[2] < 0.1)
                    && n.visual_bounds().is_some_and(|b| b.x0 < 250.0)
                {
                    red += 1;
                }
            }
        });
    }
    assert!(red > 0, "the expanded art holds the mapped symbol's red fill");
}
