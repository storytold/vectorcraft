//! Recolor Artwork (M3.78): colour reduction, preserve rules, recolour methods, Limit to Library,
//! randomize; Edit Color Group (M3.79).

use serde_json::{Value, json};
use vectorcraft_color::Color;

use crate::cmd::color_value;
use crate::*;

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
    // A count larger than the number of colors also keeps every color in its own row. A 32-bit build read 2^32 + 2 as 2.
    assert_eq!(run(&mut s, "recolor.reduce", json!({"colors": (1u64 << 32) + 2}))["map"].as_array().unwrap().len(), 5);
}

#[test]
fn colors_with_nothing_selected_come_from_all_the_art() {
    let mut s = session();
    let red = rect(&mut s, json!("#ff0000"));
    let blue = rect(&mut s, json!("#0000ff"));
    run(&mut s, "select.set", json!({"ids": []}));
    let hexes = |v: &Value| v["colors"].as_array().unwrap().iter().map(|c| c["hex"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    let mut all = hexes(&run(&mut s, "recolor.colors", json!({})));
    all.sort();
    assert_eq!(all, ["#0000ff", "#ff0000"]);
    assert!(s.doc().unwrap().selection.objects.is_empty(), "the selection is left as it was");
    assert_eq!(run(&mut s, "recolor.reduce", json!({}))["map"].as_array().unwrap().len(), 2);
    // `ids` names the objects; otherwise the selection does.
    assert_eq!(hexes(&run(&mut s, "recolor.colors", json!({"ids": [blue.0]}))), ["#0000ff"]);
    // An id listed twice counts once.
    let once = run(&mut s, "recolor.colors", json!({"ids": [red.0]}))["colors"].clone();
    assert_eq!(run(&mut s, "recolor.colors", json!({"ids": [red.0, red.0]}))["colors"], once);
    assert_eq!(run(&mut s, "recolor.reduce", json!({"ids": [red.0]}))["map"][0]["from"], json!(["rgb 255 0 0"]));
    run(&mut s, "select.set", json!({"ids": [red.0]}));
    assert_eq!(hexes(&run(&mut s, "recolor.colors", json!({}))), ["#ff0000"]);
    let e = s.execute("recolor.colors", &json!({"ids": [9999]})).unwrap_err().to_string();
    assert!(e.contains("no such object id 9999"), "{e}");
    // Recolor Artwork still recolors only what is selected.
    run(&mut s, "select.set", json!({"ids": []}));
    assert!(s.execute("recolor.apply", &json!({"map": {"#ff0000": "#00ff00"}})).is_err());
    assert_eq!(fill(&s, red).to_hex(), "#ff0000");
}

#[test]
fn colors_with_nothing_selected_leave_out_what_select_all_leaves_out() {
    let mut s = session();
    rect(&mut s, json!("#ff0000"));
    let hidden = rect(&mut s, json!("#0000ff"));
    let locked = rect(&mut s, json!("#00ff00"));
    run(&mut s, "object.hide", json!({"ids": [hidden.0]}));
    run(&mut s, "object.lock", json!({"ids": [locked.0]}));
    // A hidden layer and a locked layer, each holding a rectangle.
    for (fill, props) in [("#ffff00", json!({"visible": false})), ("#ff00ff", json!({"locked": true}))] {
        let layer = run(&mut s, "layer.new", json!({}))["id"].as_u64().unwrap();
        let id = run(&mut s, "shape.rectangle", json!({"x": 300, "y": 10, "width": 15, "height": 15}))["id"].as_u64().unwrap();
        run(&mut s, "paint.setFill", json!({"ids": [id], "color": fill}));
        assert_eq!(s.doc().unwrap().doc.layer_containing(NodeId(id)), Some(NodeId(layer)));
        let mut p = props;
        p["id"] = json!(layer);
        run(&mut s, "layer.setProps", p);
    }
    let hexes = |v: &Value| v["colors"].as_array().unwrap().iter().map(|c| c["hex"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    run(&mut s, "select.set", json!({"ids": []}));
    let none = hexes(&run(&mut s, "recolor.colors", json!({})));
    assert_eq!(none, ["#ff0000"]);
    assert_eq!(run(&mut s, "recolor.reduce", json!({}))["map"], json!([{"from": ["rgb 255 0 0"], "to": "rgb 255 0 0"}]));
    select_all(&mut s);
    assert_eq!(hexes(&run(&mut s, "recolor.colors", json!({}))), none, "the same colors as after Select All");
    // `ids` still reads hidden and locked objects.
    assert_eq!(hexes(&run(&mut s, "recolor.colors", json!({"ids": [hidden.0]}))), ["#0000ff"]);
}

#[test]
fn colors_with_nothing_selected_leave_out_hidden_and_locked_objects_in_groups() {
    let mut s = session();
    rect(&mut s, json!("#ff0000"));
    let hidden = rect(&mut s, json!("#0000ff"));
    let locked = rect(&mut s, json!("#00ff00"));
    // A group inside the group, holding a hidden rectangle.
    let inner_hidden = rect(&mut s, json!("#ffff00"));
    run(&mut s, "select.set", json!({"ids": [inner_hidden.0]}));
    run(&mut s, "object.group", json!({}));
    select_all(&mut s);
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().len(), 1, "one group holds the art");
    run(&mut s, "object.hide", json!({"ids": [hidden.0, inner_hidden.0]}));
    run(&mut s, "object.lock", json!({"ids": [locked.0]}));
    let hexes = |v: &Value| {
        let mut h = v["colors"].as_array().unwrap().iter().map(|c| c["hex"].as_str().unwrap().to_string()).collect::<Vec<_>>();
        h.sort();
        h
    };
    run(&mut s, "select.set", json!({"ids": []}));
    assert_eq!(hexes(&run(&mut s, "recolor.colors", json!({}))), ["#ff0000"]);
    assert_eq!(run(&mut s, "recolor.reduce", json!({}))["map"], json!([{"from": ["rgb 255 0 0"], "to": "rgb 255 0 0"}]));
    // Isolation mode reads the same objects of the group.
    run(&mut s, "object.isolate", json!({"id": g.0}));
    assert_eq!(hexes(&run(&mut s, "recolor.colors", json!({}))), ["#ff0000"]);
    run(&mut s, "object.exitIsolation", json!({}));
    // The group named in `ids`, or selected, is read with everything it holds.
    let all = ["#0000ff", "#00ff00", "#ff0000", "#ffff00"];
    assert_eq!(hexes(&run(&mut s, "recolor.colors", json!({"ids": [g.0]}))), all);
    run(&mut s, "select.set", json!({"ids": [g.0]}));
    assert_eq!(hexes(&run(&mut s, "recolor.colors", json!({}))), all);
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

/// Art linked to a global swatch of the group that `recolor.apply` rewrites stays linked when the
/// swatch takes its row's new color: it shows that color at its tint and follows later edits.
#[test]
fn apply_with_a_group_keeps_art_linked_to_its_global_swatches() {
    let mut s = session();
    run(&mut s, "swatch.newGroup", json!({"name": "Brand", "colors": ["#2878f0"]}));
    let name = s.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "Brand").unwrap().swatches[0].name.clone();
    run(&mut s, "swatch.edit", json!({"name": name, "global": true}));
    let full = rect(&mut s, json!("#000000"));
    let tint = rect(&mut s, json!("#000000"));
    run(&mut s, "paint.setFill", json!({"ids": [full.0], "swatch": name}));
    run(&mut s, "paint.setFill", json!({"ids": [tint.0], "swatch": name, "tint": 50}));
    let link = |s: &Session, id: NodeId| match s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint() {
        vectorcraft_color::Paint::Solid { color, swatch, tint } => (color, swatch, tint),
        p => panic!("{p:?}"),
    };
    let swatch_hex = |s: &Session| s.doc().unwrap().doc.swatch(&name).unwrap().paint.color().unwrap().to_hex();
    let orange = Color::from_hex("#ff6a00").unwrap();
    select_all(&mut s);
    let undo = s.doc().unwrap().history.undo.len();
    // The 50% tint is not in the row: it follows the swatch.
    let r = run(&mut s, "recolor.apply", json!({"map": {"#2878f0": "#ff6a00"}, "group": "Brand"}));
    assert_eq!(r["relinked"], 1, "{r}");
    assert_eq!(link(&s, full), (orange, Some(name.clone()), 1.0), "the recolored art stays linked");
    let (c, w, t) = link(&s, tint);
    assert!(w.as_deref() == Some(name.as_str()) && t == 0.5 && close(c, orange.tinted(0.5)), "{c:?} {w:?} {t}");
    assert_eq!(swatch_hex(&s), "#ff6a00");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    // A later edit of the swatch reaches the art.
    let r = run(&mut s, "swatch.edit", json!({"name": name, "color": "#00ff00"}));
    assert_eq!(r["relinked"], 2, "{r}");
    assert_eq!(fill(&s, full).to_hex(), "#00ff00");
    assert!(close(fill(&s, tint), Color::from_hex("#00ff00").unwrap().tinted(0.5)));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(swatch_hex(&s), "#2878f0");
    assert_eq!(link(&s, full), (Color::from_hex("#2878f0").unwrap(), Some(name.clone()), 1.0), "undo restores the art");
    // A row holding the tint too (as recolor.reduce gives it): the tint keeps its tint and its link.
    select_all(&mut s);
    let tint_key = vectorcraft_color::recolor::ColorKey::of(&fill(&s, tint)).to_string();
    let rows = json!([{"from": ["rgb 40 120 240", tint_key], "to": "#ff6a00"}]);
    run(&mut s, "recolor.apply", json!({"map": rows, "method": "scaleTints", "group": "Brand"}));
    let (c, w, t) = link(&s, tint);
    assert!(w.as_deref() == Some(name.as_str()) && (t - 0.5).abs() < 0.01 && close(c, orange.tinted(t)), "{c:?} {w:?} {t}");
    assert_eq!(link(&s, full), (orange, Some(name.clone()), 1.0));
    // A swatch rewritten with another color than its art's row: that art takes the row's color, unlinked.
    run(&mut s, "edit.undo", json!({}));
    select_all(&mut s);
    run(&mut s, "recolor.apply", json!({"map": {"#2878f0": "#ff6a00"}, "group": "Brand", "groupColors": ["#123456"]}));
    assert_eq!(link(&s, full), (orange, None, 1.0));
    assert_eq!(swatch_hex(&s), "#123456");
    // A row that leaves the art's color as it is while the swatch is rewritten with another color:
    // the art keeps its color, unlinked.
    run(&mut s, "edit.undo", json!({}));
    select_all(&mut s);
    let blue = Color::from_hex("#2878f0").unwrap();
    run(&mut s, "recolor.apply", json!({"map": {"#2878f0": "#2878f0"}, "group": "Brand", "groupColors": ["#123456"]}));
    assert_eq!(link(&s, full), (blue, None, 1.0));
    assert_eq!(swatch_hex(&s), "#123456");
    // A CMYK new color for RGB art keeps the art's model (model: "keep"), so the art takes the
    // color in RGB, unlinked, while the swatch takes it in CMYK.
    run(&mut s, "edit.undo", json!({}));
    select_all(&mut s);
    let r = run(&mut s, "recolor.apply", json!({"map": {"#2878f0": {"c": 0, "m": 60, "y": 100, "k": 0}}, "group": "Brand"}));
    assert!(r["note"].is_string(), "{r}");
    let (c, w, _) = link(&s, full);
    assert!(matches!(c, Color::Rgb { .. }) && w.is_none(), "{c:?} {w:?}");
    assert!(matches!(s.doc().unwrap().doc.swatch(&name).unwrap().paint.color(), Some(Color::Cmyk { .. })));
    // Without a group the swatch is unchanged, so the art takes the new color unlinked.
    run(&mut s, "edit.undo", json!({}));
    select_all(&mut s);
    run(&mut s, "recolor.apply", json!({"map": {"#2878f0": "#ff6a00"}}));
    assert_eq!(link(&s, full), (orange, None, 1.0));
    assert_eq!(swatch_hex(&s), "#2878f0");
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

#[test]
fn model_param_chooses_the_model_of_the_new_colour() {
    // RGB art in a CMYK document: by default a CMYK replacement comes out RGB (#2170).
    let mut s = session();
    run(&mut s, "file.new", json!({"width": 400, "height": 200, "colorMode": "cmyk"}));
    let art = NodeId(run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 15, "height": 15}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setFill", json!({"ids": [art.0], "color": "#1a1a1a", "keepModel": true}));
    assert_eq!(fill(&s, art).model(), vectorcraft_color::cms::Model::Rgb);
    select_all(&mut s);
    let map = json!({"#1a1a1a": {"c": 0, "m": 0, "y": 0, "k": 100}});
    let r = run(&mut s, "recolor.apply", json!({"map": map}));
    assert_eq!(fill(&s, art).model(), vectorcraft_color::cms::Model::Rgb, "kept RGB");
    assert!(r["note"].as_str().is_some_and(|n| n.contains("model")), "warns the model was kept: {r}");
    run(&mut s, "edit.undo", json!({}));
    let r = run(&mut s, "recolor.apply", json!({"map": map, "model": "new"}));
    assert_eq!(fill(&s, art), Color::cmyk(0.0, 0.0, 0.0, 1.0));
    assert!(r.get("note").is_none(), "no note when the model follows the new colour: {r}");
    run(&mut s, "edit.undo", json!({}));
    // "document" converts a new RGB colour into the document's CMYK.
    run(&mut s, "recolor.apply", json!({"map": {"#1a1a1a": "#ff0000"}, "model": "document"}));
    assert_eq!(fill(&s, art).model(), vectorcraft_color::cms::Model::Cmyk);
    assert!(s.execute("recolor.apply", &json!({"map": map, "model": "nope"})).is_err());
}

#[test]
fn model_param_must_be_a_string_when_given() {
    let mut s = session();
    let art = rect(&mut s, json!("#1a1a1a"));
    select_all(&mut s);
    for bad in [json!(42), json!(null), json!(true)] {
        assert!(s.execute("recolor.apply", &json!({"map": {"#1a1a1a": "#ff0000"}, "model": bad})).is_err(), "{bad}");
    }
    assert_eq!(fill(&s, art).to_hex(), "#1a1a1a", "nothing changed");
}

#[test]
fn document_model_converts_the_colours_of_a_group_too() {
    let mut s = session();
    run(&mut s, "file.new", json!({"width": 400, "height": 200, "colorMode": "cmyk"}));
    run(&mut s, "swatch.newGroup", json!({"name": "Theme", "colors": ["cmyk 0 0 0 50"]}));
    let group = |s: &Session| -> Vec<Color> {
        let g = s.doc().unwrap().doc.swatch_groups.iter().find(|g| g.name == "Theme").unwrap();
        g.swatches.iter().filter_map(|w| w.paint.color()).collect()
    };
    // Group-only edit from the rows, and with explicit group colours.
    run(&mut s, "recolor.apply", json!({"map": {"cmyk 0 0 0 50": "#ff0000"}, "group": "Theme", "model": "document"}));
    assert!(group(&s).iter().all(|c| c.model() == vectorcraft_color::cms::Model::Cmyk), "{:?}", group(&s));
    run(&mut s, "recolor.apply", json!({"map": [], "group": "Theme", "groupColors": ["#00ff00"], "model": "document"}));
    assert!(group(&s).iter().all(|c| c.model() == vectorcraft_color::cms::Model::Cmyk), "{:?}", group(&s));
}
