//! The Color Guide: `color.harmony` and saving several colours as swatches at once.

use serde_json::{Value, json};
use vectorcraft_color::Color;

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn hexes(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|h| h.as_str().unwrap().to_string()).collect()
}

#[test]
fn harmony_query_returns_the_group_and_its_grid() {
    let mut s = Session::new();
    let r = s.execute("color.harmony", &json!({"color": "#ff0000", "rule": "triad", "steps": 2})).unwrap();
    assert_eq!(r["rule"], "triad");
    assert_eq!(hexes(&r["colors"]), ["#ff0000", "#00ff00", "#0000ff"], "the base comes first");
    let grid = r["grid"].as_array().unwrap();
    assert_eq!(grid.len(), 3, "a row per harmony colour");
    for (row, c) in grid.iter().zip(hexes(&r["colors"])) {
        let row = hexes(row);
        assert_eq!(row.len(), 5, "2·steps + 1 columns");
        assert_eq!(row[2], c, "step 0 is the colour itself");
    }
    // Tints and shades by default: darker on the left, lighter on the right.
    let first = hexes(&grid[0]);
    let lum = |h: &str| Color::from_hex(h).unwrap().to_rgb().iter().sum::<f32>();
    assert!(lum(&first[0]) < lum(&first[2]) && lum(&first[4]) > lum(&first[2]));
    // Labels work as rule names, other variations and amounts change the grid.
    let r2 =
        s.execute("color.harmony", &json!({"color": "#ff0000", "rule": "Split Complementary", "variation": "vividMuted", "amount": 100})).unwrap();
    assert_eq!(r2["rule"], "splitComplementary");
    assert_eq!(r2["grid"][0].as_array().unwrap().len(), 9, "4 steps by default");
    // Every rule answers, base first.
    for h in vectorcraft_color::harmony::Harmony::ALL {
        let r = s.execute("color.harmony", &json!({"color": "#3366cc", "rule": h.id()})).unwrap();
        assert_eq!(r["colors"][0], "#3366cc", "{}", h.label());
    }
    assert!(s.execute("color.harmony", &json!({"color": "#ff0000", "rule": "square"})).is_err());
    assert!(s.execute("color.harmony", &json!({"rule": "triad"})).is_err());
    assert!(s.execute("color.harmony", &json!({"color": "#ff0000", "rule": "triad", "variation": "loud"})).is_err());
}

#[test]
fn new_swatch_saves_several_colours_in_one_step() {
    let mut s = session();
    let before = s.doc().unwrap().doc.swatches.len();
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("swatch.new", &json!({"colors": ["#112233", "#445566", {"c": 10, "m": 20, "y": 30, "k": 0}]})).unwrap();
    assert_eq!(hexes(&r["names"]).len(), 3);
    let st = s.doc().unwrap();
    assert_eq!(st.doc.swatches.len(), before + 3);
    assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
    assert_eq!(r["name"], "R=17 G=34 B=51");
    assert!(st.doc.swatch("C=10 M=20 Y=30 K=0").is_some());
    // Into a colour group, as global swatches.
    s.execute("swatch.newGroup", &json!({"name": "Guide"})).unwrap();
    let r = s.execute("swatch.new", &json!({"colors": ["#010203", "#040506"], "group": "Guide", "global": true})).unwrap();
    let d = &s.doc().unwrap().doc;
    let g = d.swatch_groups.iter().find(|g| g.name == "Guide").unwrap();
    assert_eq!(g.swatches.len(), 2);
    assert!(g.swatches.iter().all(|w| w.global));
    assert_eq!(hexes(&r["names"]), g.swatches.iter().map(|w| w.name.clone()).collect::<Vec<_>>());
    assert!(s.execute("swatch.new", &json!({"colors": []})).is_err());
    assert!(s.execute("swatch.new", &json!({"colors": ["nope"]})).is_err());
}
