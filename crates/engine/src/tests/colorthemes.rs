//! Color Themes: saving, editing and deleting themes in the local theme library (the preferences),
//! and adding one to the Swatches panel as a colour group.

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_color::harmony::Harmony;

use crate::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn names(s: &mut Session) -> Vec<String> {
    run(s, "colorTheme.list", json!({}))["themes"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect()
}

#[test]
fn themes_save_list_edit_and_delete() {
    let mut s = Session::new();
    assert!(names(&mut s).is_empty());
    assert!(s.execute("colorTheme.delete", &json!({"name": "Theme 1"})).is_err(), "nothing to delete");
    // From colours, keeping each colour's model.
    let r = run(&mut s, "colorTheme.save", json!({"colors": ["#ff0000", "cmyk 0 100 100 0", {"gray": 0.4}]}));
    assert_eq!(r, json!({"name": "Theme 1", "count": 1}));
    let t = &run(&mut s, "colorTheme.list", json!({}))["themes"][0];
    assert_eq!(t["colors"], json!(["#ff0000", Color::cmyk(0.0, 1.0, 1.0, 0.0).to_hex(), Color::gray(0.4).to_hex()]));
    assert_eq!(t["keys"], json!(["rgb 255 0 0", "cmyk 0 100 100 0", "gray 40"]));
    assert!(t.get("rule").is_none());
    // From a base colour and a harmony rule: its five colours, base first.
    let r = run(&mut s, "colorTheme.save", json!({"color": "#3366cc", "rule": "Triad", "name": " Sea "}));
    assert_eq!(r["name"], "Sea");
    let t = &run(&mut s, "colorTheme.list", json!({}))["themes"][1];
    let want: Vec<String> = Harmony::Triad.theme(Color::from_hex("#3366cc").unwrap()).iter().map(Color::to_hex).collect();
    assert_eq!(t["colors"], json!(want));
    assert_eq!(t["rule"], "triad");
    // Names stay unique (ignoring case); replace edits a theme in place, renaming it if asked.
    assert!(s.execute("colorTheme.save", &json!({"colors": ["#000000"], "name": "sea"})).is_err());
    run(&mut s, "colorTheme.save", json!({"colors": ["#111111", "#222222"], "replace": "Sea"}));
    assert_eq!(names(&mut s), ["Theme 1", "Sea"]);
    assert_eq!(run(&mut s, "colorTheme.list", json!({}))["themes"][1]["colors"], json!(["#111111", "#222222"]));
    run(&mut s, "colorTheme.save", json!({"colors": ["#111111"], "replace": "Theme 1", "name": "Dark"}));
    assert_eq!(names(&mut s), ["Dark", "Sea"]);
    assert!(s.execute("colorTheme.save", &json!({"colors": ["#111111"], "replace": "Dark", "name": "Sea"})).is_err());
    assert!(s.execute("colorTheme.save", &json!({"colors": ["#111111"], "replace": "Nope"})).is_err());
    assert_eq!(run(&mut s, "colorTheme.save", json!({"colors": ["#123456"]}))["name"], "Theme 1", "the first free name");
    // Bad colours, counts and rules.
    for bad in [
        json!({"colors": []}),
        json!({"colors": ["#000000", "#111111", "#222222", "#333333", "#444444", "#555555"]}),
        json!({"colors": ["nope"]}),
        json!({"color": "#ff0000"}),
        json!({"color": "#ff0000", "rule": "square"}),
        json!({"colors": ["#000000"], "name": "  "}),
        json!({}),
    ] {
        assert!(s.execute("colorTheme.save", &bad).is_err(), "{bad}");
    }
    assert_eq!(run(&mut s, "colorTheme.delete", json!({"name": "Sea"}))["deleted"], "Sea");
    assert_eq!(names(&mut s), ["Dark", "Theme 1"]);
    assert!(s.execute("colorTheme.delete", &json!({"name": "Sea"})).is_err());
}

#[test]
fn themes_round_trip_with_the_preferences() {
    let mut s = Session::new();
    run(&mut s, "colorTheme.save", json!({"colors": ["#ff8800", "cmyk 10 20 30 40", "lab 55 60 -40"], "rule": "analogous", "name": "Warm"}));
    let saved = s.prefs.to_json();
    assert_eq!(saved["colorThemes"][0]["name"], "Warm");
    let back: Prefs = serde_json::from_value(saved).unwrap();
    assert_eq!(back, s.prefs, "every colour comes back in its model");
    // Without themes nothing is written, and older preference files load with none.
    assert!(Prefs::default().to_json().get("colorThemes").is_none());
    let old: Prefs = serde_json::from_value(json!({"keyboardIncrement": 2})).unwrap();
    assert!(old.color_themes.is_empty());
    // Resetting the preferences keeps the library, and a restarted session lists it again.
    s.execute("prefs.reset", &json!({})).unwrap();
    assert_eq!(s.prefs.color_themes.len(), 1);
    let mut fresh = Session::new();
    fresh.apply_prefs(back);
    assert_eq!(names(&mut fresh), ["Warm"]);
}

#[test]
fn add_to_swatches_makes_a_colour_group_in_one_step() {
    let mut s = Session::new();
    run(&mut s, "colorTheme.save", json!({"color": "#cc3300", "rule": "splitComplementary", "name": "Tide"}));
    assert!(s.execute("colorTheme.addToSwatches", &json!({"name": "Tide"})).is_err(), "no document");
    run(&mut s, "file.new", json!({"width": 100, "height": 100}));
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "colorTheme.addToSwatches", json!({"name": "Tide"}));
    assert_eq!(r["name"], "Tide");
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
    let g = st.doc.swatch_groups.iter().find(|g| g.name == "Tide").expect("the group");
    let theme = &s.prefs.color_themes[0].colors;
    assert_eq!(g.swatches.len(), 5);
    assert_eq!(
        g.swatches.iter().filter_map(|w| w.paint.color()).map(|c| c.to_hex()).collect::<Vec<_>>(),
        theme.iter().map(Color::to_hex).collect::<Vec<_>>()
    );
    assert_eq!(r["swatches"].as_array().unwrap().len(), 5);
    // Again: a second group with a free name.
    assert_eq!(run(&mut s, "colorTheme.addToSwatches", json!({"name": "Tide"}))["name"], "Tide 2");
    assert!(s.execute("colorTheme.addToSwatches", &json!({"name": "Nope"})).is_err());
    // In a CMYK document the theme's colors keep their model.
    run(&mut s, "file.new", json!({"width": 100, "height": 100, "colorMode": "cmyk"}));
    run(&mut s, "colorTheme.addToSwatches", json!({"name": "Tide"}));
    let g = s.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "Tide").expect("the group");
    assert_eq!(g.swatches.iter().filter_map(|w| w.paint.color()).collect::<Vec<_>>(), s.prefs.color_themes[0].colors);
}
