//! Swatches panel commands: moving swatches between colour groups, the panel menu's Add Used
//! Colors, Select All Unused, Merge, Ungroup and Sort by Kind, and colours following the
//! document's colour mode.

use serde_json::{Value, json};
use vectorcraft_color::Paint;

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn doc(s: &Session) -> &vectorcraft_doc::Document {
    &s.doc().unwrap().doc
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// The swatch names of colour group `group`, or of the ungrouped swatches.
fn names(s: &Session, group: Option<&str>) -> Vec<String> {
    let d = doc(s);
    let list = match group {
        Some(g) => &d.swatch_groups.iter().find(|x| x.name == g).unwrap().swatches,
        None => &d.swatches,
    };
    list.iter().map(|w| w.name.clone()).collect()
}

#[test]
fn move_between_groups_is_one_undo_step() {
    let mut s = session();
    let before = doc(&s).clone();
    let undo = undo_len(&s);
    // Two ungrouped colours into Brights, before its second swatch, in the order given.
    run(&mut s, "swatch.move", json!({"names": ["Orange", "Red"], "group": "Brights", "to": 1}));
    assert_eq!(names(&s, Some("Brights"))[..4], ["Bright Red", "Orange", "Red", "Bright Yellow"]);
    assert!(!names(&s, None).contains(&"Red".to_string()));
    assert_eq!(undo_len(&s), undo + 1);
    // Out of a group to the front of the ungrouped colours (`to` counts the current entries).
    run(&mut s, "swatch.move", json!({"name": "Bright Green", "to": 1}));
    assert_eq!(names(&s, None)[..3], ["[None]", "Bright Green", "White"]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(*doc(&s), before);
    // Reordering within a list; past the end goes last.
    run(&mut s, "swatch.move", json!({"name": "White", "to": 999}));
    assert_eq!(names(&s, None).last().map(String::as_str), Some("White"));
}

/// A swatch or color group moved to place 2^32 goes last. A 32-bit build read the place as 0 and
/// moved it to the front.
#[test]
fn move_past_u32_goes_last() {
    let mut s = session();
    run(&mut s, "swatch.move", json!({"name": "White", "to": 1u64 << 32}));
    assert_eq!(names(&s, None).last().map(String::as_str), Some("White"));
    run(&mut s, "swatch.move", json!({"name": "Grays", "to": 1u64 << 32}));
    assert_eq!(doc(&s).swatch_groups.last().map(|g| g.name.as_str()), Some("Grays"));
}

#[test]
fn move_reorders_groups_and_refuses_bad_moves() {
    let mut s = session();
    run(&mut s, "swatch.move", json!({"name": "Brights", "to": 0}));
    let groups: Vec<&str> = doc(&s).swatch_groups.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(groups, ["Brights", "Grays"]);
    let undo = undo_len(&s);
    for p in [
        json!({"name": "Sunset", "group": "Grays"}),
        json!({"name": "[None]"}),
        json!({"name": "Grays", "group": "Brights"}),
        json!({"names": ["Red", "Grays"]}),
        json!({"name": "Red", "group": "Nope"}),
        json!({}),
    ] {
        assert!(s.execute("swatch.move", &p).is_err(), "{p}");
    }
    assert_eq!(undo_len(&s), undo, "failed moves leave no undo step");
    assert!(doc(&s).swatch("Sunset").is_some_and(|w| matches!(w.paint, Paint::Gradient(_))));
}

fn rect_filled(s: &mut Session, fill: Value) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10}))["id"].as_u64().unwrap());
    let mut p = fill;
    p["ids"] = json!([id.0]);
    run(s, "paint.setFill", p);
    run(s, "paint.setStroke", json!({"ids": [id.0], "none": true}));
    id
}

fn fill(s: &Session, id: NodeId) -> Paint {
    doc(s).node(id).unwrap().appearance.fill_paint()
}

#[test]
fn add_used_colors_skips_colours_that_have_swatches() {
    let mut s = session();
    rect_filled(&mut s, json!({"color": "#123456"}));
    rect_filled(&mut s, json!({"color": "#ed1c24"}));
    rect_filled(&mut s, json!({"swatch": "Orange"}));
    let undo = undo_len(&s);
    let r = run(&mut s, "swatch.addUsedColors", json!({}));
    assert_eq!(r, json!({"added": ["R=18 G=52 B=86"], "linked": 0}), "Red and Orange are swatches already");
    assert_eq!(undo_len(&s), undo + 1);
    assert_eq!(run(&mut s, "swatch.addUsedColors", json!({}))["added"], json!([]));
    // Only the selection's colours, as global swatches its paints link to.
    let d = rect_filled(&mut s, json!({"color": "#abcdef"}));
    rect_filled(&mut s, json!({"color": "#fedcba"}));
    run(&mut s, "select.set", json!({"ids": [d.0]}));
    let r = run(&mut s, "swatch.addUsedColors", json!({"selection": true, "global": true}));
    assert_eq!(r, json!({"added": ["R=171 G=205 B=239"], "linked": 1}));
    assert_eq!(
        fill(&s, d),
        Paint::Solid { color: vectorcraft_color::Color::from_hex("#abcdef").unwrap(), swatch: Some("R=171 G=205 B=239".into()), tint: 1.0 }
    );
    assert!(doc(&s).swatch("R=171 G=205 B=239").unwrap().global);
}

#[test]
fn unused_leaves_out_linked_colours_gradients_and_patterns_in_use() {
    let mut s = session();
    let all = run(&mut s, "swatch.unused", json!({}));
    assert!(!all["names"].as_array().unwrap().contains(&json!("[None]")));
    assert!(all["names"].as_array().unwrap().contains(&json!("Sunset")));
    run(&mut s, "swatch.edit", json!({"name": "Teal", "global": true}));
    rect_filled(&mut s, json!({"swatch": "Teal"}));
    run(&mut s, "swatch.edit", json!({"name": "Teal", "color": "#010203"}));
    rect_filled(&mut s, json!({"swatch": "Sunset"}));
    rect_filled(&mut s, json!({"color": "#ed1c24"}));
    let r = rect_filled(&mut s, json!({"color": "#ff0000"}));
    run(&mut s, "select.set", json!({"ids": [r.0]}));
    run(&mut s, "object.pattern.make", json!({"name": "Dots", "width": 20, "height": 20}));
    run(&mut s, "object.pattern.done", json!({}));
    rect_filled(&mut s, json!({"swatch": "Dots"}));
    let names = run(&mut s, "swatch.unused", json!({}))["names"].clone();
    let names: Vec<&str> = names.as_array().unwrap().iter().filter_map(Value::as_str).collect();
    for used in ["Teal", "Sunset", "Red", "Dots", "White"] {
        assert!(!names.contains(&used), "{used} is used: {names:?}");
    }
    for unused in ["Orange", "Ocean", "Bright Red", "K=50"] {
        assert!(names.contains(&unused), "{unused} is unused: {names:?}");
    }
}

#[test]
fn merge_keeps_the_first_and_relinks_the_others_art() {
    let mut s = session();
    for n in ["Red", "Orange"] {
        run(&mut s, "swatch.edit", json!({"name": n, "global": true}));
    }
    let a = rect_filled(&mut s, json!({"swatch": "Red"}));
    let b = rect_filled(&mut s, json!({"swatch": "Orange"}));
    let red = doc(&s).swatch("Red").unwrap().paint.color().unwrap();
    let before = doc(&s).clone();
    let undo = undo_len(&s);
    let r = run(&mut s, "swatch.merge", json!({"names": ["Red", "Orange"]}));
    assert_eq!(r, json!({"name": "Red", "merged": ["Orange"], "relinked": 1}));
    assert!(doc(&s).swatch("Orange").is_none());
    assert_eq!(fill(&s, b), Paint::Solid { color: red, swatch: Some("Red".into()), tint: 1.0 });
    assert_eq!(undo_len(&s), undo + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(*doc(&s), before);
    // A kept swatch that isn't global leaves the merged swatches' art unlinked, in its colour.
    run(&mut s, "swatch.merge", json!({"names": ["Lime", "Red", "Bright Red"]}));
    let lime = doc(&s).swatch("Lime").unwrap().paint.color().unwrap();
    assert_eq!(fill(&s, a), Paint::solid(lime));
    assert!(doc(&s).swatch("Red").is_none() && doc(&s).swatch("Bright Red").is_none());
    for bad in [json!({"names": ["Lime"]}), json!({"names": ["Lime", "Sunset"]}), json!({"names": ["Lime", "Nope"]})] {
        assert!(s.execute("swatch.merge", &bad).is_err(), "{bad}");
    }
}

#[test]
fn ungroup_and_sort_by_kind_are_one_undo_step_each() {
    let mut s = session();
    let undo = undo_len(&s);
    assert_eq!(run(&mut s, "swatch.ungroup", json!({"name": "Brights"}))["swatches"].as_array().unwrap().len(), 5);
    assert!(doc(&s).swatch_groups.iter().all(|g| g.name != "Brights"));
    assert_eq!(names(&s, None).last().map(String::as_str), Some("Bright Violet"));
    assert!(s.execute("swatch.ungroup", &json!({"name": "Red"})).is_err());
    // A gradient first, a spot colour among the process ones.
    run(&mut s, "swatch.move", json!({"name": "Ocean", "to": 0}));
    run(&mut s, "swatch.edit", json!({"name": "Orange", "spot": true}));
    run(&mut s, "swatch.sortByKind", json!({}));
    assert_eq!(undo_len(&s), undo + 4);
    let rank = |w: &vectorcraft_color::Swatch| match &w.paint {
        Paint::None => 0,
        Paint::Solid { .. } if !w.spot => 1,
        Paint::Solid { .. } => 2,
        Paint::Gradient(_) => 3,
        Paint::Pattern { .. } => 4,
    };
    let ranks: Vec<u8> = doc(&s).swatches.iter().map(rank).collect();
    assert!(ranks.is_sorted(), "{ranks:?}");
    let n = names(&s, None);
    assert_eq!((n[0].as_str(), n[1].as_str()), ("[None]", "White"), "the order within a kind is kept");
    assert_eq!(n[n.iter().position(|x| x == "Orange").unwrap() + 1], "Ocean", "the spot colour comes after the process ones, then the gradients");
}

#[test]
fn a_cmyk_document_starts_with_cmyk_swatches_and_stores_new_colours_as_cmyk() {
    use vectorcraft_color::Color;
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 100, "height": 100, "colorMode": "cmyk"}));
    assert_eq!(doc(&s).color_mode, vectorcraft_doc::ColorMode::Cmyk);
    assert!(doc(&s).swatches_iter().filter_map(|w| w.paint.color()).all(|c| !matches!(c, Color::Rgb { .. })));
    assert_eq!(doc(&s).swatch("Black").and_then(|w| w.paint.color()), Some(Color::cmyk(0.0, 0.0, 0.0, 1.0)));
    // RGB colors separate through the color settings (Generic CMYK: red is C0 M97 Y100 K0).
    let cms = vectorcraft_color::cms::active();
    let managed = |c: Color| cms.convert(&c, vectorcraft_color::cms::Model::Cmyk, cms.settings().intent);
    let a = rect_filled(&mut s, json!({"color": "#ff0000"}));
    assert_eq!(fill(&s, a), Paint::solid(managed(Color::rgb(1.0, 0.0, 0.0))));
    // Gray stays Gray; keepModel keeps RGB; gradient stops follow the document too.
    run(&mut s, "paint.setFill", json!({"ids": [a.0], "color": {"gray": 0.4}}));
    assert_eq!(fill(&s, a), Paint::solid(Color::gray(0.4)));
    run(&mut s, "paint.setFill", json!({"ids": [a.0], "color": "#ff0000", "keepModel": true}));
    assert_eq!(fill(&s, a), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    run(
        &mut s,
        "paint.setFill",
        json!({"ids": [a.0], "gradient": {"stops": [{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#0000ff"}]}}),
    );
    let Paint::Gradient(g) = fill(&s, a) else { panic!("a gradient") };
    assert!(g.gradient.stops.iter().all(|st| matches!(st.color, Color::Cmyk { .. })));
    // A swatch made from a hex colour is CMYK as well; the document round-trips natively.
    let name = run(&mut s, "swatch.new", json!({"color": "#00ff00"}))["name"].as_str().unwrap().to_string();
    assert_eq!(doc(&s).swatch(&name).and_then(|w| w.paint.color()), Some(managed(Color::rgb(0.0, 1.0, 0.0))));
    assert_eq!(name, "C=62 M=0 Y=100 K=0");
    let json = serde_json::to_string(doc(&s)).unwrap();
    assert_eq!(serde_json::from_str::<vectorcraft_doc::Document>(&json).unwrap(), *doc(&s));
}

/// The other swatch commands store RGB colors in a CMYK document as `swatch.new {color}` does:
/// `swatch.new {colors}`, `swatch.newGroup {colors}`, `swatch.editGroup {colors}` and
/// `swatch.edit {color}`. A color a swatch already has, and a color given with `mode`, keep their
/// model.
#[test]
fn a_cmyk_document_stores_rgb_colors_given_to_the_swatch_commands_as_cmyk() {
    use vectorcraft_color::Color;
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 100, "height": 100, "colorMode": "cmyk"}));
    let cms = vectorcraft_color::cms::active();
    let managed = |c: Color| cms.convert(&c, vectorcraft_color::cms::Model::Cmyk, cms.settings().intent);
    let (red, green) = (Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 1.0, 0.0));
    let color = |s: &Session, n: &Value| doc(s).swatch(n.as_str().unwrap()).and_then(|w| w.paint.color());
    let r = run(&mut s, "swatch.new", json!({"colors": ["#ff0000", {"c": 0, "m": 0, "y": 100, "k": 0}]}));
    assert_eq!((color(&s, &r["names"][0]), color(&s, &r["names"][1])), (Some(managed(red)), Some(Color::cmyk(0.0, 0.0, 1.0, 0.0))));
    let r = run(&mut s, "swatch.newGroup", json!({"name": "G", "colors": ["#00ff00"]}));
    assert_eq!(color(&s, &r["swatches"][0]), Some(managed(green)));
    let kept = run(&mut s, "swatch.newGroup", json!({"name": "K", "colors": ["#00ff00"], "keepModel": true}));
    assert_eq!(color(&s, &kept["swatches"][0]), Some(green), "keepModel keeps RGB");
    let r = run(&mut s, "swatch.editGroup", json!({"group": "G", "colors": ["#ff0000", "#00ff00"]}));
    assert_eq!((color(&s, &r["swatches"][0]), color(&s, &r["swatches"][1])), (Some(managed(red)), Some(managed(green))));
    let name = run(&mut s, "swatch.new", json!({"color": "#000000", "keepModel": true}))["name"].clone();
    assert_eq!(color(&s, &name), Some(Color::BLACK));
    run(&mut s, "swatch.edit", json!({"name": name, "color": "#ff0000"}));
    assert_eq!(color(&s, &name), Some(managed(red)));
    // An RGB swatch sent back unchanged stays RGB.
    let rgb = run(&mut s, "swatch.new", json!({"color": "#00ff00", "keepModel": true}))["name"].clone();
    run(&mut s, "swatch.edit", json!({"name": rgb, "color": "#00ff00", "global": true}));
    assert_eq!(color(&s, &rgb), Some(green));
    run(&mut s, "swatch.newGroup", json!({"name": "H", "swatches": [rgb]}));
    let r = run(&mut s, "swatch.editGroup", json!({"group": "H", "colors": ["#00ff00", "#ff0000"]}));
    assert_eq!((color(&s, &r["swatches"][0]), color(&s, &r["swatches"][1])), (Some(green), Some(managed(red))));
    // With `mode`, the color is converted once, into that mode.
    let r = run(&mut s, "swatch.new", json!({"color": "#00ff00", "mode": "rgb"}));
    assert_eq!(color(&s, &r["name"]), Some(green));
    let r = run(&mut s, "swatch.new", json!({"colors": ["#00ff00"], "mode": "rgb"}));
    assert_eq!(color(&s, &r["names"][0]), Some(green));
}

/// In a CMYK document `swatch.new` (with `color`, `colors` or `mode`), `swatch.newGroup`,
/// `swatch.edit` (its `keepModel` and its `paint`'s) and `swatch.editGroup` refuse a `keepModel` given
/// as text or as another value that is not true or false, and the swatches and the undo history stay
/// as they were. Null is read as `keepModel` left out.
#[test]
fn the_swatch_commands_refuse_a_keep_model_given_as_text() {
    use vectorcraft_color::Color;
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 100, "height": 100, "colorMode": "cmyk"}));
    let cms = vectorcraft_color::cms::active();
    let managed = |c: Color| cms.convert(&c, vectorcraft_color::cms::Model::Cmyk, cms.settings().intent);
    let name = run(&mut s, "swatch.new", json!({"color": "#000000", "keepModel": true}))["name"].clone();
    run(&mut s, "swatch.newGroup", json!({"name": "G", "colors": ["#00ff00"]}));
    let before = (doc(&s).clone(), undo_len(&s));
    let calls = [
        ("swatch.new", json!({"color": "#ff0000", "keepModel": "true"})),
        ("swatch.new", json!({"colors": ["#ff0000"], "keepModel": 1})),
        ("swatch.new", json!({"color": "#ff0000", "mode": "rgb", "keepModel": "false"})),
        ("swatch.newGroup", json!({"name": "K", "colors": ["#00ff00"], "keepModel": "true"})),
        ("swatch.edit", json!({"name": name, "color": "#ff0000", "keepModel": [true]})),
        ("swatch.edit", json!({"name": name, "paint": {"color": "#ff0000", "keepModel": "true"}, "mode": "cmyk"})),
        ("swatch.editGroup", json!({"group": "G", "colors": ["#ff0000"], "keepModel": "true"})),
    ];
    for (cmd, p) in &calls {
        let r = s.execute(cmd, p);
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg == "`keepModel` must be true or false"), "{cmd} {p}: {r:?}");
        assert_eq!((doc(&s).clone(), undo_len(&s)), before, "{cmd} {p}");
    }
    let r = run(&mut s, "swatch.new", json!({"color": "#ff0000", "keepModel": null}));
    assert_eq!(doc(&s).swatch(r["name"].as_str().unwrap()).and_then(|w| w.paint.color()), Some(managed(Color::rgb(1.0, 0.0, 0.0))));
}

/// The swatch commands refuse an on/off option given as text or as another value that is not true
/// or false, and `swatch.move` a `to` that is not a number, naming the key; the swatches and the
/// undo history stay as they were. True, false, numbers and null still work.
#[test]
fn the_swatch_commands_refuse_a_boolean_or_number_given_as_text() {
    use crate::tests::{not_a_number, not_a_switch, not_refused};
    let mut s = session();
    run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}));
    run(&mut s, "paint.setFill", json!({"color": "#3366cc"}));
    let name = run(&mut s, "swatch.new", json!({"color": "#000000"}))["name"].clone();
    let mut calls: Vec<_> = [
        ("swatch.new", json!({"color": "#ff0000", "spot": "true"}), "spot"),
        ("swatch.new", json!({"color": "#ff0000", "global": 1}), "global"),
        ("swatch.delete", json!({"name": name, "unlink": "false"}), "unlink"),
        ("swatch.newGroup", json!({"name": "K", "colors": ["#00ff00"], "includeTints": "true"}), "includeTints"),
        ("swatch.newGroup", json!({"name": "K", "fromArtwork": "true"}), "fromArtwork"),
        ("swatch.newGroup", json!({"name": "K", "fromArtwork": true, "toGlobal": "false"}), "toGlobal"),
        ("swatch.edit", json!({"name": name, "spot": "true"}), "spot"),
        ("swatch.edit", json!({"name": name, "global": [true]}), "global"),
        ("swatch.addUsedColors", json!({"global": "true"}), "global"),
        ("swatch.addUsedColors", json!({"selection": 1}), "selection"),
    ]
    .into_iter()
    .map(|(cmd, p, key)| not_a_switch(cmd, p, key))
    .collect();
    calls.push(not_a_number("swatch.move", json!({"name": name, "to": "0"}), "to"));
    let wrong = not_refused(&mut s, &calls);
    assert!(wrong.is_empty(), "not refused: {wrong:#?}");
    run(&mut s, "swatch.edit", json!({"name": name, "global": true, "spot": null}));
    run(&mut s, "swatch.move", json!({"name": name, "to": 0}));
    assert!(doc(&s).swatch(name.as_str().unwrap()).is_some_and(|w| w.global && !w.spot));
}

#[test]
fn rgb_documents_keep_colours_as_given_and_edits_keep_the_source_model() {
    use vectorcraft_color::Color;
    let mut s = session();
    assert_eq!(doc(&s).swatch("Black").and_then(|w| w.paint.color()), Some(Color::rgb(0.0, 0.0, 0.0)));
    let a = rect_filled(&mut s, json!({"color": "#ff0000"}));
    assert_eq!(fill(&s, a), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    let b = rect_filled(&mut s, json!({"color": {"c": 0.0, "m": 1.0, "y": 0.0, "k": 0.0}}));
    assert_eq!(fill(&s, b), Paint::solid(Color::cmyk(0.0, 1.0, 0.0, 0.0)), "an RGB document doesn't convert CMYK either");
    // Recolor Artwork keeps the replaced colour's model.
    run(&mut s, "select.set", json!({"ids": [b.0]}));
    let hex = Color::cmyk(0.0, 1.0, 0.0, 0.0).to_hex();
    run(&mut s, "recolor.apply", json!({"map": {hex: "#00ff00"}}));
    assert!(matches!(fill(&s, b).color(), Some(Color::Cmyk { .. })));
    // Blending ends of different models gives the document's model.
    let c = rect_filled(&mut s, json!({"color": "#808080"}));
    run(&mut s, "select.set", json!({"ids": [a.0, c.0, b.0]}));
    run(&mut s, "edit.colors.blendFrontToBack", json!({}));
    assert!(matches!(fill(&s, c).color(), Some(Color::Rgb { .. })));
}
