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

/// The Color Guide refuses `steps` or `amount` that is not a number, which it would otherwise
/// replace with its default.
#[test]
fn harmony_refuses_steps_or_amount_that_is_not_a_number() {
    let mut s = Session::new();
    for p in [json!({"color": "#ff0000", "rule": "triad", "steps": "2"}), json!({"color": "#ff0000", "rule": "triad", "amount": "100"})] {
        let e = s.execute("color.harmony", &p).unwrap_err().to_string();
        assert!(e.contains("must be a number"), "{p}: {e}");
    }
    let r = s.execute("color.harmony", &json!({"color": "#ff0000", "rule": "triad", "steps": null, "amount": null})).unwrap();
    assert_eq!(r["grid"][0].as_array().unwrap().len(), 9, "a null is read as a key left out");
}

/// The Color Guide refuses a `rule` or `variation` that is not a string; a `variation` such as 5
/// would otherwise be dropped and the default variation used.
#[test]
fn harmony_refuses_rule_or_variation_that_is_not_a_string() {
    let mut s = Session::new();
    for p in [json!({"color": "#ff0000", "rule": 5}), json!({"color": "#ff0000", "rule": "triad", "variation": 5})] {
        let e = s.execute("color.harmony", &p).unwrap_err().to_string();
        assert!(e.contains("must be a string"), "{p}: {e}");
    }
    s.execute("color.harmony", &json!({"color": "#ff0000", "rule": "triad", "variation": null})).unwrap();
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

/// The tint swatch a swatch named `name` is: (base, tint 0..1); none for any other swatch.
fn tint_of(s: &Session, name: &str) -> Option<(String, f32)> {
    s.doc().unwrap().doc.swatch(name)?.tint_of().map(|(b, t)| (b.to_string(), t))
}

/// `{swatch, tint}` entries of `swatch.new {colors}` and `swatch.newGroup {colors}` make tint
/// swatches linked to their spot color, beside plain colors, in one undo step; the tints follow an
/// edit of the spot color; a bad entry saves nothing.
#[test]
fn colors_entries_naming_a_spot_color_save_tint_swatches() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Ink", "color": "#0066cc", "spot": true})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("swatch.new", &json!({"colors": [{"swatch": "Ink", "tint": 60}, "#112233", {"swatch": "Ink", "tint": 30}]})).unwrap();
    let names = hexes(&r["names"]);
    assert_eq!(names, ["Ink 60%", "R=17 G=34 B=51", "Ink 30%"]);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    let (base, tint) = tint_of(&s, "Ink 60%").expect("a tint swatch");
    assert_eq!(base, "Ink");
    assert!((tint - 0.6).abs() < 1e-6, "{tint}");
    assert!(tint_of(&s, "R=17 G=34 B=51").is_none());
    let r = s.execute("swatch.newGroup", &json!({"name": "Guide", "colors": [{"swatch": "Ink", "tint": 45}, "#445566", {"swatch": "Ink"}]})).unwrap();
    let members = hexes(&r["swatches"]);
    assert_eq!(members[0], "Ink 45%");
    assert_eq!(tint_of(&s, "Ink 45%").map(|(b, _)| b).as_deref(), Some("Ink"));
    assert!(members[1..].iter().all(|m| tint_of(&s, m).is_none()), "a color and the full spot color are process copies: {members:?}");
    let d = &s.doc().unwrap().doc;
    let full = d.swatch(&members[2]).unwrap();
    assert!(!full.spot && !full.global, "{full:?}");
    // The tint swatches follow an edit of the spot color.
    s.execute("swatch.edit", &json!({"name": "Ink", "color": "#cc0000"})).unwrap();
    let d = &s.doc().unwrap().doc;
    let ink = d.global_color("Ink").unwrap();
    for name in ["Ink 60%", "Ink 45%"] {
        let w = d.swatch(name).unwrap();
        let t = w.tint_of().unwrap().1;
        assert_eq!(w.paint.color().map(|c| c.to_hex()), Some(ink.tinted(t).to_hex()), "{name}");
    }
    // A missing swatch, a tint of a process color or a tint that is not a number saves nothing.
    let count = s.doc().unwrap().doc.swatches_iter().count();
    for bad in [json!({"swatch": "Nope", "tint": 50}), json!({"swatch": "R=17 G=34 B=51", "tint": 50}), json!({"swatch": "Ink", "tint": "half"})] {
        assert!(s.execute("swatch.new", &json!({"colors": ["#010203", bad]})).is_err(), "{bad}");
        assert!(s.execute("swatch.newGroup", &json!({"colors": ["#010203", bad]})).is_err(), "{bad}");
    }
    assert_eq!(s.doc().unwrap().doc.swatches_iter().count(), count);
}
