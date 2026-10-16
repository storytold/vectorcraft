//! Recolor Artwork (M3.78): colour reduction, preserve rules, recolour methods, Limit to Library,
//! randomize; Edit Color Group (M3.79).

use serde_json::{Value, json};
use vectorcraft_color::Color;

use super::*;
use crate::cmd::color_value;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// A rectangle filled with `fill` (any colour the paint commands take).
fn rect(s: &mut Session, fill: Value) -> NodeId {
    let n = s.doc().map_or(0, |st| st.doc.layers[0].children().map_or(0, |c| c.len())) as f64;
    let id = NodeId(run(s, "shape.rectangle", json!({"x": 10.0 + 20.0 * n, "y": 10, "width": 15, "height": 15}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"ids": [id.0], "color": fill}));
    run(s, "paint.setStroke", json!({"ids": [id.0], "none": true}));
    id
}

fn fill(s: &Session, id: NodeId) -> Color {
    s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint().color().unwrap()
}

fn select_all(s: &mut Session) {
    run(s, "select.all", json!({}));
}

fn keys(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|k| k.as_str().unwrap().to_string()).collect()
}

fn close(a: Color, b: Color) -> bool {
    let v = |c: Color| match c {
        Color::Cmyk { c, m, y, k } => vec![c, m, y, k],
        Color::Rgb { r, g, b } => vec![r, g, b],
        Color::Gray { k } => vec![k],
        Color::Lab { l, a, b } => vec![l / 100.0, a / 100.0, b / 100.0],
    };
    a.model() == b.model() && v(a).iter().zip(v(b)).all(|(x, y)| (x - y).abs() < 2e-3)
}

#[test]
fn five_colours_reduce_to_two() {
    let mut s = session();
    for c in ["#ff0000", "#e01010", "#0000ff", "#1010e0", "#ff3030"] {
        rect(&mut s, json!(c));
    }
    select_all(&mut s);
    assert_eq!(run(&mut s, "recolor.colors", json!({}))["colors"].as_array().unwrap().len(), 5);
    let r = run(&mut s, "recolor.reduce", json!({"colors": 2}));
    let map = r["map"].as_array().unwrap();
    assert_eq!(map.len(), 2);
    let mut sizes: Vec<usize> = map.iter().map(|row| row["from"].as_array().unwrap().len()).collect();
    sizes.sort();
    assert_eq!(sizes, [2, 3], "blues and reds");
    assert_eq!(r["method"], "scaleTints");
    // Exact: each row becomes one colour.
    run(&mut s, "recolor.apply", json!({"map": r["map"], "method": "exact"}));
    let after = run(&mut s, "recolor.colors", json!({}));
    assert_eq!(after["colors"].as_array().unwrap().len(), 2, "{after}");
    // One undo step.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(run(&mut s, "recolor.colors", json!({}))["colors"].as_array().unwrap().len(), 5);
    // Without a count every colour is its own row.
    assert_eq!(run(&mut s, "recolor.reduce", json!({}))["map"].as_array().unwrap().len(), 5);
}

#[test]
fn preserve_flags_leave_neutrals_out() {
    let mut s = session();
    for c in [json!("#ffffff"), json!("#000000"), json!({"gray": 50}), json!("#ff0000")] {
        rect(&mut s, c);
    }
    select_all(&mut s);
    let r = run(&mut s, "recolor.reduce", json!({}));
    assert_eq!(keys(&r["preserved"]), ["rgb 0 0 0", "rgb 255 255 255"]);
    assert_eq!(r["map"].as_array().unwrap().len(), 2, "the grey and the red");
    let r = run(&mut s, "recolor.reduce", json!({"preserve": {"white": false, "black": false, "grays": true}}));
    assert_eq!(keys(&r["preserved"]), ["gray 50"]);
    assert_eq!(r["map"].as_array().unwrap().len(), 3);
}

#[test]
fn scale_and_preserve_tints_keep_tint_ratios() {
    let mut s = session();
    let full = rect(&mut s, json!({"c": 0, "m": 100, "y": 100, "k": 0}));
    let tint = rect(&mut s, json!({"c": 0, "m": 40, "y": 40, "k": 0}));
    select_all(&mut s);
    let r = run(&mut s, "recolor.reduce", json!({"colors": 1}));
    let row = &r["map"][0];
    assert_eq!(keys(&row["from"]), ["cmyk 0 100 100 0", "cmyk 0 40 40 0"], "the darkest colour leads the row");
    for method in ["scaleTints", "preserveTints"] {
        run(&mut s, "recolor.apply", json!({"map": [{"from": row["from"], "to": "cmyk 100 50 0 0"}], "method": method}));
        assert!(close(fill(&s, full), Color::cmyk(1.0, 0.5, 0.0, 0.0)), "{method}");
        assert!(close(fill(&s, tint), Color::cmyk(0.4, 0.2, 0.0, 0.0)), "{method}: {:?}", fill(&s, tint));
        run(&mut s, "edit.undo", json!({}));
    }
    // Exact makes both the new colour.
    run(&mut s, "recolor.apply", json!({"map": [{"from": row["from"], "to": "cmyk 100 50 0 0"}]}));
    assert!(close(fill(&s, tint), Color::cmyk(1.0, 0.5, 0.0, 0.0)));
}

#[test]
fn cmyk_colours_keep_their_identity_and_model() {
    let mut s = session();
    let cmyk = rect(&mut s, json!({"c": 0, "m": 100, "y": 100, "k": 0}));
    let shown = Color::cmyk(0.0, 1.0, 1.0, 0.0).to_hex();
    // An RGB colour that looks the same is a different colour.
    let rgb = rect(&mut s, json!(shown));
    select_all(&mut s);
    let listed: Vec<String> =
        run(&mut s, "recolor.colors", json!({}))["colors"].as_array().unwrap().iter().map(|c| c["key"].as_str().unwrap().to_string()).collect();
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(listed.contains(&"cmyk 0 100 100 0".to_string()));
    run(&mut s, "recolor.apply", json!({"map": {"cmyk 0 100 100 0": "cmyk 100 0 0 0"}}));
    assert_eq!(fill(&s, cmyk), Color::cmyk(1.0, 0.0, 0.0, 0.0), "exact CMYK values survive");
    assert!(matches!(fill(&s, rgb), Color::Rgb { .. }) && fill(&s, rgb).to_hex() == shown, "the RGB lookalike is untouched");
    // A new colour given in RGB still lands in CMYK.
    run(&mut s, "recolor.apply", json!({"map": {"cmyk 100 0 0 0": "#00ff00"}, "method": "scaleTints"}));
    assert!(matches!(fill(&s, cmyk), Color::Cmyk { .. }));
    // Hex keys still match every colour shown as that hex.
    run(&mut s, "recolor.apply", json!({"map": {shown.as_str(): "#0000ff"}}));
    assert_eq!(fill(&s, rgb).to_hex(), "#0000ff");
}

#[test]
fn lab_colours_keep_their_identity_and_model() {
    let mut s = session();
    let lab = rect(&mut s, json!({"l": 55, "a": 60, "b": 40}));
    let rgb = rect(&mut s, json!(Color::lab(55.0, 60.0, 40.0).to_hex()));
    select_all(&mut s);
    let r = run(&mut s, "recolor.reduce", json!({}));
    let rows: Vec<Vec<String>> = r["map"].as_array().unwrap().iter().map(|row| keys(&row["from"])).collect();
    assert!(rows.contains(&vec!["lab 55 60 40".to_string()]) && rows.len() == 2, "{rows:?}");
    run(&mut s, "recolor.apply", json!({"map": {"lab 55 60 40": "#00ff00"}, "method": "scaleTints"}));
    assert!(matches!(fill(&s, lab), Color::Lab { .. }), "the Lab colour stays Lab");
    assert_eq!(fill(&s, lab).to_hex(), "#00ff00");
    assert!(matches!(fill(&s, rgb), Color::Rgb { .. }) && fill(&s, rgb).to_hex() != "#00ff00", "its RGB lookalike is untouched");
}

#[test]
fn tints_of_a_global_swatch_share_its_row() {
    let mut s = session();
    let brand = Color::cmyk(0.0, 0.8, 0.9, 0.0);
    run(&mut s, "swatch.new", json!({"name": "Brand", "color": {"c": 0, "m": 80, "y": 90, "k": 0}, "global": true}));
    let full = rect(&mut s, json!("#00ff00"));
    let tint = rect(&mut s, json!("#00ff00"));
    let other = rect(&mut s, json!("#0000ff"));
    // Both linked to the swatch; the second as a 50% tint.
    run(&mut s, "paint.setFill", json!({"ids": [full.0], "swatch": "Brand"}));
    run(&mut s, "paint.setFill", json!({"ids": [tint.0], "swatch": "Brand", "tint": 50}));
    assert!(close(fill(&s, tint), brand.tinted(0.5)));
    select_all(&mut s);
    let colors = run(&mut s, "recolor.colors", json!({}));
    assert!(colors["colors"].as_array().unwrap().iter().filter(|c| c["swatch"] == "Brand").count() == 2, "{colors}");
    let r = run(&mut s, "recolor.reduce", json!({}));
    let map = r["map"].as_array().unwrap();
    assert_eq!(map.len(), 2, "the swatch and its tint in one row, the blue in another");
    let brand_row = map.iter().find(|row| row["from"].as_array().unwrap().len() == 2).unwrap();
    assert_eq!(brand_row["to"], "cmyk 0 80 90 0", "the swatch's colour leads its row");
    let blue_row = map.iter().find(|row| row["from"][0] == "rgb 0 0 255").unwrap();
    let rows = json!([{"from": brand_row["from"], "to": "cmyk 100 0 0 0"}, {"from": blue_row["from"], "to": null}]);
    run(&mut s, "recolor.apply", json!({"map": rows, "method": "scaleTints"}));
    assert!(close(fill(&s, tint), Color::cmyk(0.5, 0.0, 0.0, 0.0)), "{:?}", fill(&s, tint));
    assert_eq!(fill(&s, other).to_hex(), "#0000ff", "a row without a new colour is left alone");
}

#[test]
fn limit_to_library_snaps_new_colours() {
    let mut s = session();
    rect(&mut s, json!("#f01020"));
    rect(&mut s, json!("#2040e0"));
    select_all(&mut s);
    let r = run(&mut s, "recolor.reduce", json!({"limitTo": "web-safe-216"}));
    let web = |k: &Value| {
        let c = color_value(k).unwrap();
        c.to_rgb().iter().all(|v| ((v * 5.0).round() - v * 5.0).abs() < 1e-3)
    };
    assert!(r["map"].as_array().unwrap().iter().all(|row| web(&row["to"])), "{r}");
    run(&mut s, "recolor.apply", json!({"map": {"#f01020": "#f01020"}, "limitTo": "Web Safe 216"}));
    let hexes: Vec<Value> = run(&mut s, "recolor.colors", json!({}))["colors"].as_array().unwrap().iter().map(|c| c["hex"].clone()).collect();
    assert!(hexes.contains(&json!("#ff0033")) && !hexes.contains(&json!("#f01020")), "{hexes:?}");
    assert!(s.execute("recolor.reduce", &json!({"limitTo": "nope"})).is_err());
}

#[test]
fn methods_and_bad_params() {
    let mut s = session();
    rect(&mut s, json!("#ff0000"));
    select_all(&mut s);
    assert!(s.execute("recolor.apply", &json!({"map": {"#ff0000": "#00ff00"}, "method": "nope"})).is_err());
    assert!(s.execute("recolor.apply", &json!({"map": {"#ff0000": "nonsense"}})).is_err());
    assert!(s.execute("recolor.apply", &json!({})).is_err());
    run(&mut s, "select.none", json!({}));
    assert!(s.execute("recolor.apply", &json!({"map": {}})).is_err(), "needs art or a group");
}

#[test]
fn randomize_shuffles_new_colours_and_skips_excluded_rows() {
    let mut s = session();
    let map = json!([
        {"from": ["rgb 255 0 0"], "to": "rgb 255 0 0"},
        {"from": ["rgb 0 255 0"], "to": "rgb 0 255 0"},
        {"from": ["rgb 0 0 255"], "to": "rgb 0 0 255", "exclude": true},
        {"from": ["rgb 255 255 0"], "to": "rgb 255 255 0"},
    ]);
    let a = run(&mut s, "recolor.randomize", json!({"map": map, "seed": 4}));
    let b = run(&mut s, "recolor.randomize", json!({"map": map, "seed": 4}));
    assert_eq!(a, b, "deterministic");
    assert_eq!(a["map"][2], map[2], "an excluded row keeps its colour");
    let mut tos: Vec<String> = [0, 1, 3].iter().map(|i| a["map"][*i]["to"].as_str().unwrap().to_string()).collect();
    tos.sort();
    assert_eq!(tos, ["rgb 0 255 0", "rgb 255 0 0", "rgb 255 255 0"], "a permutation");
    let sb = run(&mut s, "recolor.randomize", json!({"map": map, "order": false, "saturationBrightness": true}));
    let hue = |v: &Value| color_value(v).unwrap().to_hsb()[0];
    assert!((hue(&sb["map"][0]["to"]) - 0.0).abs() < 1.0 && (hue(&sb["map"][1]["to"]) - 120.0).abs() < 1.0, "hue kept");
}

#[test]
fn edit_group_rewrites_in_order_as_one_undo_step() {
    let mut s = session();
    let g = run(&mut s, "swatch.newGroup", json!({"name": "Theme", "colors": ["#ff0000", "#00ff00", "#0000ff"]}));
    let names = keys(&g["swatches"]);
    let group = |s: &Session| {
        let d = &s.doc().unwrap().doc;
        let g = d.swatch_groups.iter().find(|g| g.name == "Theme" || g.name == "Brand Theme").unwrap();
        (g.name.clone(), g.swatches.iter().map(|w| (w.name.clone(), w.paint.color().unwrap().to_hex())).collect::<Vec<_>>())
    };
    let before = group(&s);
    let r = run(
        &mut s,
        "swatch.editGroup",
        json!({"group": "Theme", "colors": ["#111111", "cmyk 0 0 0 50", "#333333", "#444444"], "rename": "Brand Theme"}),
    );
    assert_eq!(r["name"], "Brand Theme");
    let (name, sw) = group(&s);
    assert_eq!(name, "Brand Theme");
    assert_eq!(sw.iter().map(|w| w.1.as_str()).collect::<Vec<_>>()[..1], ["#111111"]);
    assert_eq!(sw.len(), 4, "an extra colour becomes a new swatch");
    assert_eq!(sw[..3].iter().map(|w| w.0.clone()).collect::<Vec<_>>(), names, "the swatches keep their names, in order");
    assert!(matches!(s.doc().unwrap().doc.swatch(&names[1]).unwrap().paint.color(), Some(Color::Cmyk { .. })));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(group(&s), before, "one undo step");
    // Fewer colours drop the swatches past the end.
    run(&mut s, "swatch.editGroup", json!({"group": "Theme", "colors": ["#ababab"]}));
    assert_eq!(group(&s).1.len(), 1);
    assert!(s.execute("swatch.editGroup", &json!({"group": "Nope", "colors": []})).is_err());
}

#[test]
fn edit_group_relinks_art_and_apply_rewrites_the_group_with_the_art() {
    let mut s = session();
    run(&mut s, "swatch.newGroup", json!({"name": "Theme", "colors": ["#ff0000"]}));
    let name = s.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "Theme").unwrap().swatches[0].name.clone();
    run(&mut s, "swatch.edit", json!({"name": name, "global": true}));
    let linked = rect(&mut s, json!("#000000"));
    run(&mut s, "paint.setFill", json!({"ids": [linked.0], "swatch": name}));
    let r = run(&mut s, "swatch.editGroup", json!({"group": "Theme", "colors": ["#00ff00"]}));
    assert_eq!(r["relinked"], 1);
    assert_eq!(fill(&s, linked).to_hex(), "#00ff00", "linked art follows its swatch");
    // Recolor with a group: the art and the group change in one undo step.
    let plain = rect(&mut s, json!("#0000ff"));
    run(&mut s, "select.set", json!({"ids": [plain.0]}));
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "recolor.apply", json!({"map": [{"from": ["#0000ff"], "to": "#ffff00"}], "group": "Theme"}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(fill(&s, plain).to_hex(), "#ffff00");
    assert_eq!(s.doc().unwrap().doc.swatch(&name).unwrap().paint.color().unwrap().to_hex(), "#ffff00");
    // Without a selection, only the group.
    run(&mut s, "select.none", json!({}));
    run(&mut s, "recolor.apply", json!({"map": [{"from": ["#ffff00"], "to": "#ff00ff"}], "group": "Theme"}));
    assert_eq!(s.doc().unwrap().doc.swatch(&name).unwrap().paint.color().unwrap().to_hex(), "#ff00ff");
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Edit Color Group");
    // Explicit group colours.
    run(&mut s, "recolor.apply", json!({"map": [], "group": "Theme", "groupColors": ["#010203", "cmyk 0 0 0 20"]}));
    let g = s.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "Theme").unwrap().clone();
    assert_eq!(g.swatches.iter().map(|w| w.paint.color().unwrap()).collect::<Vec<_>>(), [Color::rgb8(1, 2, 3), Color::cmyk(0.0, 0.0, 0.0, 0.2)]);
}

#[test]
fn edit_group_refuses_to_change_built_in_swatches() {
    let mut s = session();
    run(&mut s, "swatch.newGroup", json!({"name": "Marks", "colors": ["#ff0000"]}));
    // A group holding the built-in Registration swatch (as an older file might).
    s.edit("test", |d, _| {
        d.swatch_groups.iter_mut().find(|g| g.name == "Marks").unwrap().swatches.insert(0, vectorcraft_color::swatch::registration().clone());
        Ok(())
    })
    .unwrap();
    let reg = vectorcraft_color::swatch::registration().paint.color().unwrap();
    assert!(s.execute("swatch.editGroup", &json!({"group": "Marks", "colors": ["#00ff00", "#0000ff"]})).is_err(), "changed");
    assert!(s.execute("swatch.editGroup", &json!({"group": "Marks", "colors": []})).is_err(), "removed");
    let key = vectorcraft_color::recolor::ColorKey::of(&reg).to_string();
    let ok = run(&mut s, "swatch.editGroup", json!({"group": "Marks", "colors": [key, "#0000ff"]}));
    assert_eq!(ok["swatches"][0], vectorcraft_color::swatch::REGISTRATION, "kept as it is");
}

/// A `{c, m, y, k}` object takes one scale for the whole colour: percent as soon as any component is
/// above 1, else fractions. Reading each component on its own turned K 0.79 % into 79 %.
#[test]
fn cmyk_object_reads_one_scale_for_every_component() {
    let k = |v: Value| color_value(&v).unwrap().to_cmyk();
    let close = |a: [f32; 4], b: [f32; 4]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5);
    assert!(close(k(json!({"c": 18.16, "m": 13.39, "y": 13.9, "k": 0.79})), [0.1816, 0.1339, 0.139, 0.0079]));
    assert!(close(k(json!({"c": 100, "m": 0.5, "y": 0, "k": 1})), [1.0, 0.005, 0.0, 0.01]));
    // All components 0..1: fractions, as before.
    assert!(close(k(json!({"c": 0.1, "m": 0.5, "y": 0.9, "k": 0.2})), [0.1, 0.5, 0.9, 0.2]));
    assert!(close(k(json!({"c": 0, "m": 1, "y": 1, "k": 0})), [0.0, 1.0, 1.0, 0.0]));
}

/// Recolouring to a library's spot colour (Limit to Library) applies the spot colour itself, linked
/// to it (tints keep their tint), and adds it to the document's swatches; a process swatch of the
/// library gives its colour in its own model.
#[test]
fn recolor_to_a_library_spot_colour_links_it_and_adds_the_swatch() {
    let mut s = session();
    // A library with a Lab spot colour and a CMYK process colour (made here, then removed).
    run(&mut s, "swatch.new", json!({"name": "Ink 101", "color": {"l": 50.0, "a": 70.0, "b": 40.0}, "spot": true}));
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": {"c": 0.6, "m": 0.0, "y": 1.0, "k": 0.1}}));
    let data = run(&mut s, "swatch.library.save", json!({"names": ["Ink 101", "Leaf"], "name": "Inks"}))["data"].as_str().unwrap().to_string();
    let lib = run(&mut s, "swatch.library.load", json!({"data": data, "name": "inks.vcswatches"}))["library"].as_str().unwrap().to_string();
    run(&mut s, "swatch.delete", json!({"names": ["Ink 101", "Leaf"]}));
    let red = rect(&mut s, json!("#e02010"));
    let pink = rect(&mut s, json!("#f09088"));
    let green = rect(&mut s, json!("#20c040"));
    select_all(&mut s);
    let map = json!([
        {"from": ["rgb 224 32 16", "rgb 240 144 136"], "to": "#e02010"},
        {"from": ["rgb 32 192 64"], "to": "#20c040"},
    ]);
    let r = run(&mut s, "recolor.apply", json!({"map": map, "method": "scaleTints", "limitTo": lib}));
    assert_eq!(r["swatches"], json!(["Ink 101", "Leaf"]), "{r}");
    let d = &s.doc().unwrap().doc;
    let ink = d.swatch("Ink 101").unwrap();
    assert!(ink.spot && ink.global, "the spot colour joined the document's swatches");
    let paint = |id: NodeId| d.node(id).unwrap().appearance.fill_paint();
    match paint(red) {
        vectorcraft_color::Paint::Solid { color, swatch, tint } => {
            assert_eq!(swatch.as_deref(), Some("Ink 101"), "linked to the spot colour");
            assert_eq!(tint, 1.0);
            assert_eq!(color, Color::lab(50.0, 70.0, 40.0), "in the spot colour's own model");
        }
        p => panic!("{p:?}"),
    }
    match paint(pink) {
        vectorcraft_color::Paint::Solid { swatch, tint, .. } => {
            assert_eq!(swatch.as_deref(), Some("Ink 101"));
            assert!(tint > 0.1 && tint < 0.9, "a tint of the spot colour: {tint}");
        }
        p => panic!("{p:?}"),
    }
    assert_eq!(paint(green), vectorcraft_color::Paint::solid(Color::cmyk(0.6, 0.0, 1.0, 0.1)), "a process swatch's colour, in CMYK");
    // Again: the document's swatch is used, not added twice.
    run(&mut s, "edit.undo", json!({}));
    select_all(&mut s);
    run(&mut s, "swatch.library.add", json!({"library": lib, "names": ["Ink 101"]}));
    select_all(&mut s);
    let r = run(&mut s, "recolor.apply", json!({"map": map, "method": "scaleTints", "limitTo": lib}));
    assert_eq!(r["swatches"], json!(["Leaf"]));
}
