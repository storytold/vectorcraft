//! Panel-support commands (`cmd/{panelcmds,gradient,appearance,stroke,swatch,style}.rs`).

use serde_json::json;
use vectorcraft_color::{Color, GradientKind, Paint};
use vectorcraft_doc::{AppearanceItem, NodeKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 100, "height": 50})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

#[test]
fn edit_gradient_converts_and_keeps_stops() {
    let mut s = session();
    let id = rect(&mut s);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute(
        "paint.editGradient",
        &json!({"stroke": false, "kind": "radial", "stops": [
            {"offset": 1.0, "color": "#0000ff"},
            {"offset": 0.0, "color": "#ff0000", "opacity": 50, "midpoint": 0.3},
            {"offset": 0.5, "color": "#00ff00"}]}),
    )
    .unwrap();
    let Paint::Gradient(g) = node(&s, id).appearance.fill_paint() else { panic!("not a gradient") };
    assert_eq!(g.gradient.kind, GradientKind::Radial);
    assert_eq!(g.gradient.stops.len(), 3);
    assert_eq!(g.gradient.stops[0].color.to_hex(), "#ff0000");
    assert!((g.gradient.stops[0].opacity - 0.5).abs() < 1e-6);
    assert!((g.gradient.stops[0].midpoint - 0.3).abs() < 1e-6);
    // One undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(node(&s, id).appearance.fill_paint().color().is_some());
}

#[test]
fn edit_gradient_angle_reverse_and_errors() {
    let mut s = session();
    let id = rect(&mut s);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("paint.editGradient", &json!({"stroke": false, "angle": 450})).unwrap();
    let Paint::Gradient(g) = node(&s, id).appearance.fill_paint() else { panic!() };
    assert!((g.angle - 90.0).abs() < 1e-9);
    s.execute("paint.editGradient", &json!({"stroke": false, "reverse": true})).unwrap();
    let Paint::Gradient(g) = node(&s, id).appearance.fill_paint() else { panic!() };
    assert_eq!(g.gradient.stops[0].color.to_hex(), "#000000");
    // #1007: an off-centre midpoint is mirrored, not reset.
    let stops = json!([{"offset": 0, "color": "#ff0000", "midpoint": 0.2}, {"offset": 1, "color": "#0000ff"}]);
    s.execute("paint.editGradient", &json!({"stroke": false, "stops": stops})).unwrap();
    s.execute("paint.editGradient", &json!({"stroke": false, "reverse": true})).unwrap();
    let Paint::Gradient(g) = node(&s, id).appearance.fill_paint() else { panic!() };
    assert_eq!(g.gradient.stops[0].color.to_hex(), "#0000ff");
    assert!((g.gradient.stops[0].midpoint - 0.8).abs() < 1e-6, "{:?}", g.gradient.stops);
    assert!(s.execute("paint.editGradient", &json!({"stops": [{"offset": 0, "color": "#fff"}]})).is_err());
    assert!(s.execute("paint.editGradient", &json!({"kind": "conic"})).is_err());
}

#[test]
fn appearance_duplicate_and_move() {
    let mut s = session();
    let id = rect(&mut s);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let n0 = node(&s, id).appearance.items.len();
    s.execute("appearance.duplicateItem", &json!({"index": 0})).unwrap();
    assert_eq!(node(&s, id).appearance.items.len(), n0 + 1);
    let last = node(&s, id).appearance.items.len() - 1;
    s.execute("appearance.moveItem", &json!({"from": last, "to": 0})).unwrap();
    assert!(matches!(node(&s, id).appearance.items[0], AppearanceItem::Stroke(_)));
    assert!(s.execute("appearance.duplicateItem", &json!({"index": 99})).is_err());
}

/// The appearance and effect commands given an item index of 2^32 + k, where k names one of the
/// object's items, leave the object's appearance and the undo history as they were, as for any index
/// past the last item: `appearance.removeItem` removes nothing, and the others fail. A 32-bit build
/// read the index as k.
#[test]
fn appearance_commands_leave_an_item_past_u32_alone() {
    let mut s = session();
    let id = rect(&mut s);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let before = node(&s, id).appearance;
    let undo = s.doc().unwrap().history.undo.len();
    let past = |k: u64| (1u64 << 32) + k;
    for (cmd, p, fails) in [
        ("appearance.removeItem", json!({"index": past(0)}), false),
        ("appearance.removeItem", json!({"indices": [past(0)]}), false),
        ("appearance.duplicateItem", json!({"index": past(0)}), true),
        ("appearance.moveItem", json!({"from": past(1), "to": 0}), true),
        ("effect.apply", json!({"effect": "distort.roughen", "item": past(1)}), true),
    ] {
        assert_eq!(s.execute(cmd, &p).is_err(), fails, "{cmd} {p}");
        assert_eq!(node(&s, id).appearance, before, "{cmd} {p}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "{cmd} {p}");
    }
}

#[test]
fn artboards_reorder_and_duplicate() {
    let mut s = session();
    s.execute("artboard.new", &json!({"name": "Second"})).unwrap();
    let r = s.execute("artboard.duplicate", &json!({"index": 0})).unwrap();
    assert_eq!(r["index"], 2);
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.artboards.len(), 3);
    assert!(d.artboards[2].rect.x0 >= d.artboards[1].rect.x1);
    s.execute("artboard.reorder", &json!({"index": 1, "to": 0})).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards[0].name, "Second");
}

/// An artboard's background color (Properties › Artboards › Fill): `artboard.setProps {color}` sets
/// it and `{color: null}` clears it, each in one undo step. A change without `color` leaves it, a
/// duplicate keeps it, `inspect_document` reports it, `artboard.new {color}` starts with it, an
/// `artboards` list gives each listed artboard its own color in one undo step, and a bad color is
/// refused.
#[test]
fn artboard_color_is_set_and_cleared_in_one_undo_step() {
    let mut s = session();
    s.execute("artboard.new", &json!({})).unwrap();
    let color = |s: &Session, i: usize| s.doc().unwrap().doc.artboards[i].color;
    let blue = Color::from_hex("#336699");
    let black = Some(Color::cmyk(0.0, 0.0, 0.0, 1.0));
    s.execute("artboard.setProps", &json!({"index": 1, "color": "#336699"})).unwrap();
    assert_eq!((color(&s, 0), color(&s, 1)), (None, blue), "only that artboard");
    s.execute("artboard.setProps", &json!({"index": 1, "name": "Dark"})).unwrap();
    assert_eq!(color(&s, 1), blue, "a change without a color leaves it");
    s.execute("artboard.setProps", &json!({"index": 1, "color": {"c": 0, "m": 0, "y": 0, "k": 100}})).unwrap();
    assert_eq!(color(&s, 1), black);
    s.execute("artboard.setProps", &json!({"index": 1, "color": null})).unwrap();
    assert_eq!(color(&s, 1), None);
    for was in [black, blue, blue, None] {
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(color(&s, 1), was);
    }
    s.execute("edit.redo", &json!({})).unwrap();
    let doc = crate::inspect::document(&s);
    assert_eq!(doc["artboards"][1]["color"], "#336699");
    assert!(doc["artboards"][0].get("color").is_none(), "{doc}");
    let copy = s.execute("artboard.duplicate", &json!({"index": 1})).unwrap()["index"].as_u64().unwrap() as usize;
    assert_eq!(color(&s, copy), blue, "a duplicate keeps the color");
    let made = s.execute("artboard.new", &json!({"color": "#336699"})).unwrap()["index"].as_u64().unwrap() as usize;
    assert_eq!(color(&s, made), blue, "a new artboard takes `color` too");
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("artboard.setProps", &json!({"artboards": [{"index": 0, "color": "#336699"}, {"index": 1, "color": null}]})).unwrap();
    assert_eq!((color(&s, 0), color(&s, 1)), (blue, None), "each listed artboard takes its own color");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "in one undo step");
    let before = s.doc().unwrap().doc.clone();
    assert!(s.execute("artboard.setProps", &json!({"artboards": [{"index": 0, "color": null}, {"index": 1, "color": "nope"}]})).is_err());
    assert!(s.execute("artboard.setProps", &json!({"index": 1, "color": "nope"})).is_err());
    assert!(s.execute("artboard.new", &json!({"color": "nope"})).is_err());
    assert_eq!(*s.doc().unwrap().doc, *before, "a bad color changes nothing");
}

/// The native format writes an artboard's background color and reads it back. An artboard without
/// one is written as before, with no `color` key.
#[test]
fn artboard_color_round_trips_through_the_native_format() {
    let mut s = session();
    s.execute("artboard.new", &json!({})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "color": {"c": 10, "m": 20, "y": 30, "k": 40}})).unwrap();
    let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let keys = |i: usize| saved["document"]["artboards"][i].as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys(0), ["id", "name", "rect", "show_center_mark", "show_cross_hairs"]);
    assert!(keys(1).contains(&"color".to_string()), "{:?}", keys(1));
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(back.artboards[0].color, None);
    assert_eq!(back.artboards[1].color, Some(Color::cmyk(0.1, 0.2, 0.3, 0.4)));
    assert_eq!(back.artboards, s.doc().unwrap().doc.artboards);
}

/// An artboard moved to place 2^32 goes last. A 32-bit build read the place as 0 and moved it to the
/// front.
#[test]
fn artboard_reorder_past_u32_goes_last() {
    let mut s = session();
    s.execute("artboard.new", &json!({"name": "Second"})).unwrap();
    s.execute("artboard.new", &json!({"name": "Third"})).unwrap();
    s.execute("artboard.reorder", &json!({"index": 1, "to": 1u64 << 32})).unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards.last().map(|a| a.name.as_str()), Some("Second"));
}

/// The artboards' left edges and names.
fn boards(s: &Session) -> Vec<(f64, String)> {
    s.doc().unwrap().doc.artboards.iter().map(|a| (a.rect.x0, a.name.clone())).collect()
}

/// The left edge of object `id`.
fn left(s: &Session, id: u64) -> f64 {
    s.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap().x0
}

fn ids(v: &serde_json::Value) -> Vec<u64> {
    v.as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect()
}

/// Duplicate Artboards (#446) copies the art on the artboard too, as the Artboard tool's Move/Copy
/// Artwork with Artboard says, in one undo step.
#[test]
fn duplicate_artboard_copies_its_art() {
    let mut s = session();
    let r = rect(&mut s);
    let out = s.execute("artboard.duplicate", &json!({"index": 0})).unwrap();
    assert_eq!(out["index"], 1);
    let copies = ids(&out["ids"]);
    assert_eq!(copies.len(), 1);
    assert_ne!(copies[0], r.0);
    assert_eq!(boards(&s), vec![(0.0, "Artboard 1".into()), (820.0, "Artboard 1 copy".into())]);
    assert_eq!(left(&s, copies[0]), 830.0, "the copy sits on the new artboard as the original on its own");
    s.execute("edit.undo", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.artboards.len(), d.layers[0].children().unwrap().len()), (1, 1), "one undo step");
    // Off (the tool's option or the param): the artboard alone.
    s.set_tool_options(Some("artboard"), json!({"moveArt": false}).as_object().unwrap());
    assert_eq!(s.execute("artboard.duplicate", &json!({})).unwrap()["ids"], json!([]));
    assert_eq!(ids(&s.execute("artboard.duplicate", &json!({"index": 0, "art": true})).unwrap()["ids"]).len(), 1);
}

/// New Artboard and Artboard Options trim the name they are given, as the Artboards panel's rename
/// does, and a blank name gives the artboard its default name, Artboard N for the Nth artboard.
#[test]
fn new_and_renamed_artboards_trim_their_names() {
    let mut s = session();
    let new = |s: &mut Session, name: &str| s.execute("artboard.new", &json!({"name": name})).unwrap();
    new(&mut s, "  Cover \t");
    new(&mut s, " \t ");
    new(&mut s, "");
    let names = |s: &Session| boards(s).into_iter().map(|(_, n)| n).collect::<Vec<_>>();
    assert_eq!(names(&s), ["Artboard 1", "Cover", "Artboard 3", "Artboard 4"]);
    s.execute("artboard.setProps", &json!({"index": 0, "name": " Back\t"})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "name": "  "})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 3, "name": ""})).unwrap();
    assert_eq!(names(&s), ["Back", "Artboard 2", "Artboard 3", "Artboard 4"]);
}

/// Duplicate, Copy and Cut Artboard and Move Artboard Up/Down fail with `no such artboard` on an
/// `index` that names no artboard (past the last, negative, fractional, a string or null), and the
/// artboards, the art and the clipboard stay as they were.
#[test]
fn artboard_copies_refuse_an_index_that_names_no_artboard() {
    let mut s = session();
    rect(&mut s);
    s.execute("artboard.new", &json!({})).unwrap();
    s.set_tool_options(Some("artboard"), json!({"active": 1}).as_object().unwrap());
    let (was, n) = (boards(&s), s.doc().unwrap().history.undo.len());
    for index in [json!(99), json!(-1), json!(1.5), json!("1"), serde_json::Value::Null] {
        for cmd in ["artboard.duplicate", "artboard.copy", "artboard.cut", "artboard.reorder"] {
            let p = if cmd == "artboard.reorder" { json!({"index": index, "to": 0}) } else { json!({"index": index}) };
            assert_eq!(s.execute(cmd, &p).unwrap_err().to_string(), "no such artboard", "{cmd} {p}");
        }
    }
    let st = s.doc().unwrap();
    assert_eq!((boards(&s), st.doc.layers[0].children().unwrap().len(), st.history.undo.len()), (was, 1, n));
    assert!(s.clipboard.nodes.is_empty() && s.clipboard.artboards.is_empty(), "nothing copied");
}

/// Artboard Options, Move Artboard, Duplicate Artboards, Copy Artboard and Cut Artboard refuse an
/// option given as text or as another value that is not true or false, such as "true", and
/// Artboard Options and Move Artboard a position, size or move given as text, such as "10". The
/// artboards, the art, the undo history and the clipboard stay as they were; a null reads as the
/// option left out.
#[test]
fn artboard_commands_refuse_a_boolean_or_number_given_as_text() {
    use crate::tests::{not_a_number, not_a_switch, not_refused};
    let mut s = session();
    let r = rect(&mut s);
    s.execute("artboard.new", &json!({})).unwrap();
    let mut calls: Vec<_> = [
        ("artboard.setProps", "scaleArt", json!({"index": 0, "width": 400, "scaleArt": "true"})),
        ("artboard.setProps", "strokes", json!({"index": 0, "width": 400, "scaleArt": true, "strokes": "false"})),
        ("artboard.setProps", "corners", json!({"index": 0, "width": 400, "scaleArt": true, "corners": 1})),
        ("artboard.setProps", "patterns", json!({"index": 0, "x": 50, "moveArt": true, "patterns": "true"})),
        ("artboard.setProps", "moveArt", json!({"index": 0, "x": 50, "moveArt": "true"})),
        ("artboard.setProps", "lockedAndHidden", json!({"index": 0, "x": 50, "moveArt": true, "lockedAndHidden": [true]})),
        ("artboard.setProps", "locked", json!({"artboards": [{"index": 1, "locked": "true"}]})),
        ("artboard.move", "moveArt", json!({"index": 0, "dx": 10, "moveArt": "true"})),
        ("artboard.move", "copy", json!({"index": 0, "dx": 10, "copy": 1})),
        ("artboard.move", "patterns", json!({"index": 0, "dx": 10, "moveArt": true, "patterns": "false"})),
        ("artboard.duplicate", "patterns", json!({"index": 0, "patterns": "true"})),
        ("artboard.duplicate", "art", json!({"index": 0, "art": "false"})),
        ("artboard.copy", "art", json!({"index": 0, "art": "true"})),
        ("artboard.cut", "art", json!({"index": 1, "art": 0})),
    ]
    .into_iter()
    .map(|(cmd, key, p)| not_a_switch(cmd, p, key))
    .collect();
    for (cmd, key, p) in [
        ("artboard.setProps", "x", json!({"index": 0, "x": "50"})),
        ("artboard.setProps", "height", json!({"index": 1, "height": [300]})),
        ("artboard.move", "dx", json!({"index": 0, "dx": "10"})),
        ("artboard.move", "dy", json!({"index": 0, "dx": 10, "dy": "5"})),
    ] {
        calls.push(not_a_number(cmd, p, key));
    }
    let wrong = not_refused(&mut s, &calls);
    assert!(wrong.is_empty(), "not refused: {wrong:#?}");
    assert!(s.clipboard.nodes.is_empty() && s.clipboard.artboards.is_empty(), "nothing copied");
    let p = json!({"index": 0, "dx": 10, "dy": null, "moveArt": true, "copy": null, "patterns": false});
    s.execute("artboard.move", &p).unwrap();
    assert_eq!(node(&s, r).geometric_bounds().map(|b| b.x0), Some(20.0));
}

/// Artboards 1 and 2, each with a square on it, and an artboard 3 below artboard 1 with a square on
/// it, with the Artboard tool selecting artboards 1 and 2 → the squares' ids, artboard by artboard.
fn two_selected_artboards(s: &mut Session) -> [u64; 3] {
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 100, "height": 100})).unwrap();
    s.execute("artboard.new", &json!({"x": 0, "y": 700, "width": 100, "height": 100, "name": "Small"})).unwrap();
    let sq = |s: &mut Session, x: u64, y: u64| {
        s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap()
    };
    let ids = [sq(s, 10, 10), sq(s, 1010, 10), sq(s, 10, 710)];
    s.execute("select.none", &json!({})).unwrap();
    s.select_tool("artboard", ViewInfo::default()).unwrap();
    s.set_tool_option("selected", &json!([0, 1]));
    ids
}

/// The objects on the document's layers, in paint order.
fn objects(s: &Session) -> Vec<u64> {
    s.doc().unwrap().doc.layers.iter().flat_map(|l| l.children().into_iter().flatten().map(|n| n.id.0)).collect()
}

/// Duplicate Artboards copies every selected artboard with its art, in one undo step (the public
/// Use the artboard context menu page). The copies keep their places relative to each other, right
/// of the last artboard.
#[test]
fn duplicate_artboards_copies_every_selected_artboard_in_one_step() {
    let mut s = session();
    let [a, b, _] = two_selected_artboards(&mut s);
    let before = (boards(&s), objects(&s));
    for p in [json!({"indices": [0, 1]}), json!({})] {
        let out = s.execute("artboard.duplicate", &p).unwrap();
        assert_eq!((out["index"].clone(), out["indices"].clone()), (json!(3), json!([3, 4])), "{p}");
        let copies = ids(&out["ids"]);
        assert_eq!(copies.len(), 2, "{p}");
        assert_eq!(boards(&s)[3..], [(1120.0, "Artboard 1 copy".to_string()), (2120.0, "Artboard 2 copy".to_string())], "{p}");
        assert_eq!((left(&s, copies[0]), left(&s, copies[1])), (1130.0, 2130.0), "{p}: each square on its artboard's copy");
        assert!(left(&s, a) == 10.0 && left(&s, b) == 1010.0, "{p}: the originals stay");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!((boards(&s), objects(&s)), before, "{p}: one undo step");
    }
}

/// #2103: with the Artboard tool, an arrow key moves the selected artboards and the art on them by
/// the keyboard increment (Shift: ten times as far) in one undo step each; the selected art
/// doesn't move on its own.
#[test]
fn artboard_tool_arrow_keys_move_the_selected_artboards_with_their_art() {
    use vectorcraft_tools::{Mods, ToolKey};
    let mut s = session();
    let [a, b, c] = two_selected_artboards(&mut s);
    s.execute("prefs.set", &json!({"key": "keyboardIncrement", "value": 4})).unwrap();
    let v = ViewInfo::default();
    assert!(s.tool_claims_key(ToolKey::Right, v));
    let undo = s.doc().unwrap().history.undo.len();
    s.tool_key(ToolKey::Right, Mods::default(), v).unwrap();
    s.tool_key(ToolKey::Right, Mods { shift: true, ..Mods::default() }, v).unwrap();
    let xs = |s: &Session| boards(s).into_iter().map(|(x, _)| x).collect::<Vec<_>>();
    assert_eq!(xs(&s), [44.0, 1044.0, 0.0]);
    assert_eq!([left(&s, a), left(&s, b), left(&s, c)], [54.0, 1054.0, 10.0], "the art moves with its artboard");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 2, "one step per press");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((xs(&s), left(&s, a)), (vec![4.0, 1004.0, 0.0], 14.0));
}

/// With the Artboard tool, Copy takes every selected artboard and the art on each (the public Cut,
/// copy, and paste artboards page). Paste adds a copy of each with its art in one undo step,
/// together right of the last artboard; Paste in Place puts them where they were. The tool then
/// selects the pasted artboards.
#[test]
fn artboard_tool_copy_and_paste_bring_every_selected_artboard() {
    let mut s = session();
    two_selected_artboards(&mut s);
    let before = (boards(&s), objects(&s));
    assert_eq!(s.execute("edit.copy", &json!({})).unwrap()["copied"], 2, "the squares on artboards 1 and 2");
    let out = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!((out["artboard"].clone(), out["artboards"].clone()), (json!(3), json!([3, 4])));
    assert_eq!(boards(&s)[3..], [(1120.0, "Artboard 1 copy".to_string()), (2120.0, "Artboard 2 copy".to_string())]);
    let pasted = ids(&out["ids"]);
    assert_eq!((left(&s, pasted[0]), left(&s, pasted[1])), (1130.0, 2130.0));
    assert_eq!(s.tool_options()["selected"], json!([3, 4]), "the tool selects the pasted artboards");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((boards(&s), objects(&s)), before, "one undo step");
    let out = s.execute("edit.pasteInPlace", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.artboards[3].rect, d.artboards[4].rect), (d.artboards[0].rect, d.artboards[1].rect));
    let pasted = ids(&out["ids"]);
    assert_eq!((left(&s, pasted[0]), left(&s, pasted[1])), (10.0, 1010.0));
}

/// With the Artboard tool, Cut takes every selected artboard and the art on each away in one undo
/// step, and leaves the other artboards and their art; it fails when every artboard is selected.
#[test]
fn artboard_tool_cut_removes_every_selected_artboard() {
    let mut s = session();
    let [a, b, c] = two_selected_artboards(&mut s);
    let before = (boards(&s), objects(&s));
    assert_eq!(s.execute("edit.cut", &json!({})).unwrap()["copied"], 2);
    assert_eq!((boards(&s), objects(&s)), (vec![(0.0, "Small".into())], vec![c]));
    s.execute("edit.pasteInPlace", &json!({})).unwrap();
    assert_eq!(boards(&s), vec![(0.0, "Small".into()), (0.0, "Artboard 1".into()), (1000.0, "Artboard 2".into())]);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((boards(&s), objects(&s)), before, "the cut is one undo step");
    assert!(objects(&s).contains(&a) && objects(&s).contains(&b));
    s.set_tool_option("selected", &json!([0, 1, 2]));
    assert!(s.execute("edit.cut", &json!({})).is_err(), "every artboard");
    assert_eq!(boards(&s), before.0);
}

/// With the Artboard tool, Copy and Paste copy its artboard with the art on it (#446): Paste places
/// it right of the last artboard, Paste in Place where it was, also in another document; each paste
/// is one undo step, and the tool takes the pasted artboard.
#[test]
fn artboard_tool_copy_and_paste_artboards_with_their_art() {
    let mut s = session();
    let r = rect(&mut s);
    s.execute("artboard.new", &json!({"x": 0, "y": 700, "width": 100, "height": 100, "name": "Small"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    // Nothing selected: Copy is off, until the Artboard tool is chosen.
    let copy_enabled = |s: &Session| (crate::find_command("edit.copy").unwrap().enabled)(s).is_ok();
    assert!(!copy_enabled(&s));
    s.select_tool("artboard", ViewInfo::default()).unwrap();
    assert!(copy_enabled(&s));
    assert_eq!(s.execute("edit.copy", &json!({})).unwrap()["copied"], 1, "artboard 1 and its rectangle");
    let out = s.execute("edit.paste", &json!({"center": [5, 5]})).unwrap();
    assert_eq!(out["artboard"], 2);
    let pasted = ids(&out["ids"]);
    assert_eq!(left(&s, pasted[0]), 830.0, "beside the artboards, not at the view centre");
    assert_eq!(boards(&s)[2], (820.0, "Artboard 1 copy".into()));
    assert_eq!(s.tool_options()["active"], 2);
    let st = s.doc().unwrap();
    assert_eq!(st.selection.in_paint_order(&st.doc), vec![NodeId(pasted[0])]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(boards(&s).len(), 2);
    assert!(s.doc().unwrap().doc.node(NodeId(pasted[0])).is_none(), "one undo step");
    // In place: on top of the original.
    let out = s.execute("edit.pasteInPlace", &json!({})).unwrap();
    assert_eq!(left(&s, ids(&out["ids"])[0]), 10.0);
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.artboards[2].rect, d.artboards[0].rect);
    // Into another document.
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    let out = s.execute("edit.paste", &json!({})).unwrap();
    assert_eq!(boards(&s), vec![(0.0, "Artboard 1".into()), (320.0, "Artboard 1 copy".into())]);
    let n = s.doc().unwrap().doc.node(NodeId(ids(&out["ids"])[0])).unwrap().geometric_bounds().unwrap();
    assert_eq!((n.x0, n.width()), (330.0, 100.0));
    // Paste on All Artboards pastes the art alone.
    let out = s.execute("edit.pasteOnAllArtboards", &json!({})).unwrap();
    assert_eq!((out["artboard"].clone(), ids(&out["ids"]).len(), boards(&s).len()), (json!(null), 2, 2));
    // The clipboard still holds the artboard for the next document, and the original is untouched.
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    assert!(s.doc().unwrap().doc.node(r).is_some());
}

/// With the Artboard tool, Cut takes its artboard and the art on it away in one undo step (#446),
/// never the only artboard; pasting it back keeps its name.
#[test]
fn artboard_tool_cut_removes_the_artboard_and_its_art() {
    let mut s = session();
    let r = rect(&mut s);
    let outside = s.execute("shape.rectangle", &json!({"x": 2000, "y": 0, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.select_tool("artboard", ViewInfo::default()).unwrap();
    assert!(s.execute("edit.cut", &json!({})).is_err(), "the only artboard");
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 100, "height": 100})).unwrap();
    s.set_tool_option("active", &json!(0));
    s.execute("edit.cut", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.artboards.len(), d.node(r).is_none(), d.node(NodeId(outside)).is_some()), (1, true, true));
    s.execute("edit.pasteInPlace", &json!({})).unwrap();
    assert_eq!(boards(&s), vec![(1000.0, "Artboard 2".into()), (0.0, "Artboard 1".into())]);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!((d.artboards.len(), d.node(r).is_some()), (2, true), "the cut is one undo step");
}

/// A copy of an artboard whose id is the largest there is (only a hand-made file has one, #1718)
/// takes an id that no other artboard has, as a new artboard does, whether Duplicate Artboards, the
/// Artboard tool's Alt-drag (`artboard.move` with `copy`) or Paste makes it. Deleting either
/// artboard then leaves the other's guides.
#[test]
fn copied_artboards_take_a_free_id_and_keep_their_own_guides() {
    let mut got = vec![];
    for (cmd, p) in [
        ("artboard.duplicate", json!({"index": 0})),
        ("artboard.move", json!({"index": 0, "dx": 0, "dy": 900, "copy": true})),
        ("edit.paste", json!({})),
    ] {
        let mut s = Session::new();
        let mut d = vectorcraft_doc::Document::new(800.0, 600.0);
        d.artboards[0].id = u32::MAX;
        s.add_document(d, None);
        s.execute("guide.add", &json!({"vertical": true, "pos": 100, "artboard": 0})).unwrap();
        if cmd == "edit.paste" {
            s.execute("artboard.copy", &json!({"index": 0})).unwrap();
        }
        s.execute(cmd, &p).unwrap();
        // Each pass deletes one artboard (the copy, then the original), adds the artboard ids of
        // the remaining guides to `kept`, and undoes the delete.
        let mut kept = vec![];
        for i in [1, 0] {
            s.execute("artboard.delete", &json!({"index": i})).unwrap();
            kept.push(s.doc().unwrap().doc.guides.iter().map(|g| g.artboard).collect::<Vec<_>>());
            s.execute("edit.undo", &json!({})).unwrap();
        }
        got.push((cmd, s.doc().unwrap().doc.artboards.iter().map(|a| a.id).collect::<Vec<_>>(), kept));
    }
    let max = Some(u32::MAX);
    assert_eq!(
        got,
        [
            ("artboard.duplicate", vec![u32::MAX, 1], vec![vec![max], vec![Some(1)]]),
            ("artboard.move", vec![u32::MAX, 1], vec![vec![max], vec![Some(1)]]),
            ("edit.paste", vec![u32::MAX, 1], vec![vec![max], vec![]]),
        ]
    );
}

/// `artboard.setProps {locked}` locks an artboard in one undo step and deselects the art on it.
/// Select All, `select.set` and Select › Inverse then pass over that art, a command that takes the
/// selection leaves it where it is, and the document summary reports `locked: true`. Unlocking
/// gives the art back. The art of pattern editing mode is on no artboard.
#[test]
fn a_locked_artboard_protects_the_art_on_it() {
    let mut s = session();
    let a = rect(&mut s);
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 200, "height": 200})).unwrap();
    let b = NodeId(s.execute("shape.rectangle", &json!({"x": 1010, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    let selected = |s: &Session| s.doc().unwrap().selection.objects.clone();
    let locked = |s: &Session| s.doc().unwrap().doc.artboards[1].locked;
    s.execute("select.all", &json!({})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    assert_eq!(selected(&s), [a], "locking deselects b");
    let doc = crate::inspect::document(&s);
    assert_eq!(doc["artboards"][1]["locked"], true);
    assert!(doc["artboards"][0].get("locked").is_none(), "{doc}");
    for (cmd, p) in [("select.all", json!({})), ("select.set", json!({"ids": [a.0, b.0]}))] {
        s.execute(cmd, &p).unwrap();
        assert_eq!(selected(&s), [a], "{cmd}");
    }
    s.execute("select.none", &json!({})).unwrap();
    s.execute("select.inverse", &json!({})).unwrap();
    assert_eq!(selected(&s), [a]);
    s.execute("select.all", &json!({})).unwrap();
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    assert_eq!((left(&s, a.0), left(&s, b.0)), (15.0, 1010.0), "b stays");
    // Art drawn on it stays selected until the selection leaves it; art moved onto it is
    // deselected by the edit.
    let drawn = NodeId(s.execute("shape.rectangle", &json!({"x": 1100, "y": 100, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    assert_eq!(selected(&s), [drawn], "drawn on it");
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("object.move", &json!({"dx": 1000, "dy": 0})).unwrap();
    assert!(selected(&s).is_empty(), "moved onto it");
    s.execute("select.set", &json!({"ids": [drawn.0]})).unwrap();
    assert!(selected(&s).is_empty(), "drawn on it, then left out of the selection");
    s.execute("artboard.setProps", &json!({"index": 1, "locked": false})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    assert_eq!(selected(&s), [a, b, drawn]);
    for was in [true, true, true, true, false] {
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(locked(&s), was);
    }
    // The art of pattern editing mode is on no artboard: Select All selects it on a locked one.
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "P", "edit": false})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 0, "locked": true})).unwrap();
    s.execute("object.pattern.edit", &json!({"name": "P"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let tile = selected(&s);
    assert!(!tile.is_empty() && !tile.contains(&a), "{tile:?}");
}

/// New art on a locked artboard stays selected and editable until the selection leaves it: a
/// second Pen click goes on the path the first one started, and pasted art moves. Once another
/// object is clicked, a click, a marquee, `select.set` and Select All pass over that art. Undo, in
/// a batch too, and a failed batch bring back the selection the step had, and the new art with
/// it. Isolation mode starts with nothing selected, so the art is protected in it, and leaving it
/// doesn't select that art. Art drawn on an unlocked artboard, or
/// new art that leaves the locked artboard, is deselected when it is moved onto one. Selection
/// changes leave the document as it is, and once one has left the new art out, an unlocked
/// artboard over it moves without it. A member of a new group can be selected, and ungrouping it
/// keeps the members selected.
#[test]
fn new_art_on_a_locked_artboard_stays_editable_until_the_selection_leaves_it() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut s = session();
    let a = rect(&mut s);
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 400, "height": 400})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    // An unlocked artboard over part of the locked one.
    s.execute("artboard.new", &json!({"x": 1100, "y": 200, "width": 200, "height": 200})).unwrap();
    let selected = |s: &Session| s.doc().unwrap().selection.objects.iter().map(|id| id.0).collect::<Vec<_>>();
    let v = ViewInfo { zoom: 1.0, smart_guides: false, snap_to_grid: false, snap_to_pixel: false, snap_to_point: false, ..Default::default() };
    let click = |s: &mut Session, x, y| {
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
        }
    };
    // Art drawn on the unlocked artboard and moved onto the locked one isn't new art there.
    let off = s.execute("shape.rectangle", &json!({"x": 300, "y": 300, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    assert_eq!(selected(&s), [off]);
    s.execute("object.move", &json!({"dx": 1000, "dy": 0})).unwrap();
    assert!(selected(&s).is_empty(), "moved onto it");
    s.select_tool("pen", v).unwrap();
    click(&mut s, 1050.0, 50.0);
    click(&mut s, 1150.0, 50.0);
    let [path] = selected(&s)[..] else { panic!("one path selected: {:?}", selected(&s)) };
    assert_eq!(node(&s, NodeId(path)).path_data().unwrap().anchor_count(), 2, "the second click goes on the same path");
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("edit.copy", &json!({})).unwrap();
    let pasted = ids(&s.execute("edit.paste", &json!({"center": [1200, 300]})).unwrap()["ids"]);
    assert_eq!(selected(&s), pasted);
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    assert_eq!((selected(&s), left(&s, pasted[0])), (pasted.clone(), 1160.0), "pasted art moves");
    // Moved off the locked artboard and back, it is no longer new art.
    s.execute("object.move", &json!({"dx": -1000, "dy": 0})).unwrap();
    assert_eq!(selected(&s), pasted);
    s.execute("object.move", &json!({"dx": 1000, "dy": 0})).unwrap();
    assert!(selected(&s).is_empty(), "moved off and back");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(selected(&s), pasted, "undone twice");
    s.select_tool("selection", v).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    click(&mut s, 10.0, 30.0);
    assert_eq!(selected(&s), [a.0], "a click on other art");
    assert!(Arc::ptr_eq(&doc, &s.doc().unwrap().doc), "a selection change leaves the document as it is");
    let moved = s.execute("artboard.move", &json!({"index": 2, "dx": 0, "dy": 1000, "moveArt": true})).unwrap();
    assert!(!ids(&moved["moved"]).contains(&pasted[0]), "the artboard over it moves without it");
    s.execute("edit.undo", &json!({})).unwrap();
    click(&mut s, 1160.0, 300.0);
    assert!(selected(&s).is_empty(), "a click on the pasted art's edge");
    s.pointer(&PointerEvent::new(PointerKind::Down, 1010.0, 10.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 1390.0, 390.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 1390.0, 390.0), v).unwrap();
    assert!(selected(&s).is_empty(), "a marquee");
    s.execute("select.set", &json!({"ids": [path, pasted[0]]})).unwrap();
    assert!(selected(&s).is_empty(), "select.set");
    s.execute("select.all", &json!({})).unwrap();
    assert_eq!(selected(&s), [a.0], "Select All");
    // Undoing the move brings back the pasted art selected, as it was before the move.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(selected(&s), pasted);
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    assert_eq!(left(&s, pasted[0]), 1160.0);
    let editable = |s: &Session| s.doc().unwrap().doc.is_editable(NodeId(pasted[0]));
    // A batch that deselects it and fails brings it back selected and editable, and so does
    // undoing a batch that deselects it and draws.
    let failed = json!({"commands": [{"command": "select.none"}, {"command": "no.such.command"}]});
    assert!(s.execute("command.batch", &failed).is_err());
    assert_eq!((selected(&s), editable(&s)), (pasted.clone(), true), "after a failed batch");
    let drawn =
        json!({"commands": [{"command": "select.none"}, {"command": "shape.rectangle", "params": {"x": 300, "y": 300, "width": 20, "height": 20}}]});
    s.execute("command.batch", &drawn).unwrap();
    assert!(!editable(&s), "deselected by the batch");
    // Undo inside a later batch gives the document the new art of the selection it brings back:
    // the unlocked artboard over that art then moves it along.
    let up = json!({"command": "artboard.move", "params": {"index": 2, "dx": 0, "dy": 1000, "moveArt": true}});
    let r = s.execute("command.batch", &json!({"commands": [{"command": "edit.undo"}, up.clone()]})).unwrap();
    assert!(ids(&r["results"][1]["moved"]).contains(&pasted[0]), "undone inside a batch: {r}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((selected(&s), editable(&s)), (pasted.clone(), true), "the batch undone");
    // Isolation mode inside a batch protects it before the next step.
    let isolated = json!({"commands": [{"command": "object.isolate", "params": {"id": pasted[0]}}, up]});
    let r = s.execute("command.batch", &isolated).unwrap();
    assert!(!ids(&r["results"][1]["moved"]).contains(&pasted[0]), "isolated inside a batch: {r}");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("object.exitIsolation", &json!({})).unwrap();
    // Isolation mode starts with nothing selected, so leaving it doesn't select the art again,
    // and the art, protected again, isn't isolated, as locked art isn't.
    assert!(selected(&s).is_empty(), "after isolation mode");
    assert!(!editable(&s), "after isolation mode");
    assert!(s.execute("object.isolate", &json!({"id": pasted[0]})).is_err(), "isolating it again");
    // A member of a new group can be selected while the group is new art.
    let copy = ids(&s.execute("edit.paste", &json!({"center": [1200, 150]})).unwrap()["ids"]);
    s.execute("object.group", &json!({})).unwrap();
    let group = selected(&s);
    s.execute("select.set", &json!({"ids": copy})).unwrap();
    assert_eq!(selected(&s), copy, "a member of a new group");
    s.execute("select.set", &json!({"ids": group})).unwrap();
    assert_eq!(selected(&s), group, "from the member back to the group");
    s.execute("object.ungroup", &json!({})).unwrap();
    assert_eq!(selected(&s), copy, "ungrouped");
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    assert_eq!(selected(&s), copy, "the ungrouped art moves");
}

/// When an artboard moves, scales or is cut with its art, the art inside it that is also on a
/// locked artboard stays where it is, unless prefs moveLockedWithArtboard is on, as locked art
/// does; a copy of the artboard copies that art.
#[test]
fn art_also_on_a_locked_artboard_stays_when_another_artboard_moves() {
    let mut s = session();
    // A locked artboard over the right edge of artboard 1, and `held` inside artboard 1 and on it.
    s.execute("artboard.new", &json!({"x": 700, "y": 0, "width": 300, "height": 300})).unwrap();
    let free = rect(&mut s);
    let held = NodeId(s.execute("shape.rectangle", &json!({"x": 710, "y": 10, "width": 50, "height": 20})).unwrap()["id"].as_u64().unwrap());
    s.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    let undo = |s: &mut Session| s.execute("edit.undo", &json!({})).unwrap();
    let moved = s.execute("artboard.move", &json!({"index": 0, "dx": 0, "dy": 1000, "moveArt": true})).unwrap();
    assert_eq!(ids(&moved["moved"]), [free.0]);
    undo(&mut s);
    let scaled = s.execute("artboard.setProps", &json!({"index": 0, "width": 1600, "scaleArt": true})).unwrap();
    assert_eq!(ids(&scaled["scaled"]), [free.0]);
    undo(&mut s);
    s.execute("artboard.cut", &json!({"index": 0, "art": true})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.node(held).is_some() && d.node(free).is_none(), "the cut leaves it");
    undo(&mut s);
    assert_eq!(ids(&s.execute("artboard.duplicate", &json!({"index": 0, "art": true})).unwrap()["ids"]).len(), 2, "a copy takes it");
    undo(&mut s);
    s.prefs.move_locked_with_artboard = true;
    let moved = s.execute("artboard.move", &json!({"index": 0, "dx": 0, "dy": 1000, "moveArt": true})).unwrap();
    assert_eq!(ids(&moved["moved"]), [free.0, held.0], "with the preference on, it moves");
}

/// A locked artboard keeps its place and size: `artboard.setProps` refuses a new rectangle but
/// still renames it, `artboard.move`, `artboard.delete`, `artboard.cut` and `artboard.fitToArt`
/// refuse it, and Rearrange leaves it and the art on it where they are. A copy of it starts
/// unlocked, and unlocked it moves again.
#[test]
fn a_locked_artboard_keeps_its_place_and_size() {
    let mut s = session();
    // Three artboards in a row, 800 pt wide: at 0, 820 and 1640.
    for _ in 0..2 {
        s.execute("artboard.new", &json!({})).unwrap();
    }
    // On the second and third artboards, its center on the third.
    let shape = |s: &mut Session, x: u32, y: u32| {
        s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 80, "height": 20})).unwrap()["id"].as_u64().unwrap()
    };
    let c = shape(&mut s, 1610, 10);
    // In a sublayer, one like it and one on the third artboard alone.
    s.execute("layer.newSublayer", &json!({"name": "Sub"})).unwrap();
    let (c2, d2) = (shape(&mut s, 1610, 100), shape(&mut s, 2000, 100));
    s.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    let rects = |s: &Session| s.doc().unwrap().doc.artboards.iter().map(|a| a.rect).collect::<Vec<_>>();
    let was = rects(&s);
    let refused = |r: Result<serde_json::Value>| matches!(r, Err(EngineError::Other(m)) if m == "the artboard is locked");
    assert!(refused(s.execute("artboard.setProps", &json!({"index": 1, "width": 300}))));
    assert!(refused(s.execute("artboard.move", &json!({"index": 1, "dx": 5, "dy": 0}))));
    assert!(refused(s.execute("artboard.delete", &json!({"index": 1}))));
    assert!(refused(s.execute("artboard.cut", &json!({"index": 1}))));
    assert!(refused(s.execute("artboard.fitToArt", &json!({"index": 1}))));
    assert_eq!(rects(&s), was);
    s.execute("artboard.setProps", &json!({"index": 1, "name": "Cover", "x": 820, "y": 0, "width": 800, "height": 600})).unwrap();
    assert_eq!(boards(&s)[1], (820.0, "Cover".into()), "Artboard Options renames it");
    // Its slot in the row is at 850; the third artboard's is at 1700.
    s.execute("artboard.rearrange", &json!({"layout": "row", "spacing": 50})).unwrap();
    assert_eq!(rects(&s)[1], was[1]);
    assert_eq!(rects(&s)[2].x0, 1700.0, "the third artboard moves");
    assert_eq!((left(&s, c), left(&s, c2)), (1610.0, 1610.0), "the art on the locked artboard stays, in a sublayer too");
    assert_eq!(left(&s, d2), 2060.0, "the sublayer's other art moves with the third artboard");
    let copy = s.execute("artboard.duplicate", &json!({"index": 1, "art": false})).unwrap()["index"].as_u64().unwrap() as usize;
    assert!(!s.doc().unwrap().doc.artboards[copy].locked, "a copy starts unlocked");
    s.execute("artboard.setProps", &json!({"index": 1, "locked": false, "x": 5})).unwrap();
    assert_eq!(rects(&s)[1].x0, 5.0, "unlocking it lets the same change move it");
}

/// A locked artboard keeps its exact rectangle when `artboard.setProps` passes values within
/// 1e-6 pt of it, as Artboard Options does when it renames the artboard: repeated calls don't
/// move or resize it by small steps.
#[test]
fn a_locked_artboard_keeps_its_exact_rectangle() {
    let mut s = session();
    s.execute("artboard.setProps", &json!({"index": 0, "locked": true})).unwrap();
    let rect = |s: &Session| s.doc().unwrap().doc.artboards[0].rect;
    let was = rect(&s);
    for i in 0..10 {
        let r = rect(&s);
        let p =
            json!({"index": 0, "name": format!("Cover {i}"), "x": r.x0 + 5e-7, "y": r.y0 - 9e-7, "width": r.width() + 4e-7, "height": r.height()});
        s.execute("artboard.setProps", &p).unwrap();
    }
    assert_eq!(rect(&s), was);
    assert_eq!(boards(&s)[0].1, "Cover 9", "the name still changes");
}

/// `artboard.setProps {artboards}` takes `locked` from each entry, and a list that gives a locked
/// artboard a new rectangle is refused and changes no artboard.
#[test]
fn a_list_of_artboards_takes_each_entrys_lock() {
    let mut s = session();
    s.execute("artboard.new", &json!({})).unwrap();
    s.execute("artboard.setProps", &json!({"artboards": [{"index": 0, "locked": true}, {"index": 1, "name": "Back"}]})).unwrap();
    let locks = |s: &Session| s.doc().unwrap().doc.artboards.iter().map(|a| a.locked).collect::<Vec<_>>();
    let widths = |s: &Session| s.doc().unwrap().doc.artboards.iter().map(|a| a.rect.width()).collect::<Vec<_>>();
    assert_eq!(locks(&s), [true, false]);
    let was = widths(&s);
    let r = s.execute("artboard.setProps", &json!({"artboards": [{"index": 1, "width": 300}, {"index": 0, "width": 300}]}));
    assert!(matches!(r, Err(EngineError::Other(m)) if m == "the artboard is locked"));
    assert_eq!(widths(&s), was, "neither artboard changes");
    s.execute("artboard.setProps", &json!({"artboards": [{"index": 1, "width": 300}, {"index": 0, "width": 300, "locked": false}]})).unwrap();
    assert_eq!((locks(&s), widths(&s)), (vec![false, false], vec![300.0, 300.0]));
}

/// The native format writes an artboard's lock and reads it back. An unlocked artboard is written
/// as before, with no `locked` key: unlocked again, the document saves to the bytes it had before.
#[test]
fn artboard_lock_round_trips_through_the_native_format() {
    let mut s = session();
    s.execute("artboard.new", &json!({})).unwrap();
    let save = |s: &Session| vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let before = save(&s);
    s.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    let bytes = save(&s);
    let saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let keys = saved["document"]["artboards"][0].as_object().unwrap().keys().cloned().collect::<Vec<_>>();
    assert_eq!(keys, ["id", "name", "rect", "show_center_mark", "show_cross_hairs"]);
    assert_eq!(saved["document"]["artboards"][1]["locked"], true);
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert!(back.artboards[1].locked && !back.artboards[0].locked);
    assert_eq!(back.artboards, s.doc().unwrap().doc.artboards);
    s.execute("artboard.setProps", &json!({"index": 1, "locked": false})).unwrap();
    assert_eq!(save(&s), before);
}

/// Artboard Options' Show Center Mark and Show Cross Hairs: `artboard.setProps {showCenterMark,
/// showCrossHairs}` sets and clears each on that artboard only, in one undo step, also for a
/// locked artboard and in an `artboards` list. A change without them leaves them, a duplicate
/// keeps them, `artboard.new` takes them, `inspect_document` reports them, and a value that isn't
/// true or false is refused.
#[test]
fn artboard_display_marks_are_set_and_cleared_in_one_undo_step() {
    let mut s = session();
    s.execute("artboard.new", &json!({})).unwrap();
    let marks = |s: &Session, i: usize| {
        let a = &s.doc().unwrap().doc.artboards[i];
        (a.show_center_mark, a.show_cross_hairs)
    };
    let undo = |s: &Session| s.doc().unwrap().history.undo.len();
    let u0 = undo(&s);
    s.execute("artboard.setProps", &json!({"index": 1, "showCenterMark": true})).unwrap();
    assert_eq!((marks(&s, 0), marks(&s, 1)), ((false, false), (true, false)), "only that artboard");
    s.execute("artboard.setProps", &json!({"index": 1, "showCrossHairs": true, "showCenterMark": false})).unwrap();
    assert_eq!(marks(&s, 1), (false, true));
    s.execute("artboard.setProps", &json!({"index": 1, "name": "Marked"})).unwrap();
    assert_eq!(marks(&s, 1), (false, true), "a change without them leaves them");
    assert_eq!(undo(&s), u0 + 3, "one undo step each");
    for was in [(false, true), (true, false), (false, false)] {
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(marks(&s, 1), was);
    }
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(marks(&s, 1), (true, false));
    // A locked artboard keeps its rectangle, not its display marks.
    s.execute("artboard.setProps", &json!({"index": 0, "locked": true})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 0, "showCrossHairs": true})).unwrap();
    assert_eq!(marks(&s, 0), (false, true));
    let u = undo(&s);
    s.execute(
        "artboard.setProps",
        &json!({"artboards": [{"index": 0, "showCenterMark": true}, {"index": 1, "showCenterMark": false, "showCrossHairs": true}]}),
    )
    .unwrap();
    assert_eq!((marks(&s, 0), marks(&s, 1)), ((true, true), (false, true)), "each listed artboard takes its own");
    assert_eq!(undo(&s), u + 1, "in one undo step");
    let doc = crate::inspect::document(&s);
    assert_eq!((doc["artboards"][0]["showCenterMark"].clone(), doc["artboards"][1]["showCrossHairs"].clone()), (json!(true), json!(true)));
    assert!(doc["artboards"][1].get("showCenterMark").is_none(), "{doc}");
    let copy = s.execute("artboard.duplicate", &json!({"index": 1})).unwrap()["index"].as_u64().unwrap() as usize;
    assert_eq!(marks(&s, copy), (false, true), "a duplicate keeps them");
    let made = s.execute("artboard.new", &json!({"showCenterMark": true})).unwrap()["index"].as_u64().unwrap() as usize;
    assert_eq!(marks(&s, made), (true, false), "a new artboard takes them too");
    let plain = s.execute("artboard.new", &json!({})).unwrap()["index"].as_u64().unwrap() as usize;
    assert_eq!(marks(&s, plain), (false, false));
    let before = s.doc().unwrap().doc.clone();
    assert!(s.execute("artboard.setProps", &json!({"index": 1, "showCenterMark": "yes"})).is_err());
    assert!(
        s.execute("artboard.setProps", &json!({"artboards": [{"index": 0, "showCenterMark": false}, {"index": 1, "showCrossHairs": 1}]})).is_err()
    );
    assert!(s.execute("artboard.new", &json!({"showCrossHairs": "yes"})).is_err());
    assert_eq!(*s.doc().unwrap().doc, *before, "a bad value changes nothing");
}

/// The native format keeps Show Center Mark and Show Cross Hairs. A document that never shows them
/// is written byte for byte as before they could be set, and a file written without the two keys
/// opens with both off.
#[test]
fn artboard_display_marks_round_trip_through_the_native_format() {
    let mut s = session();
    s.execute("artboard.new", &json!({})).unwrap();
    let save = |s: &Session| vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let before = save(&s);
    s.execute("artboard.setProps", &json!({"artboards": [{"index": 0, "showCenterMark": true}, {"index": 1, "showCrossHairs": true}]})).unwrap();
    let bytes = save(&s);
    let saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let ab = |i: usize| saved["document"]["artboards"][i].clone();
    assert_eq!((ab(0)["show_center_mark"].clone(), ab(0)["show_cross_hairs"].clone()), (json!(true), json!(false)));
    assert_eq!((ab(1)["show_center_mark"].clone(), ab(1)["show_cross_hairs"].clone()), (json!(false), json!(true)));
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(back.artboards, s.doc().unwrap().doc.artboards);
    s.execute("artboard.setProps", &json!({"artboards": [{"index": 0, "showCenterMark": false}, {"index": 1, "showCrossHairs": false}]})).unwrap();
    assert_eq!(save(&s), before, "off again, the file is as it was");
    // A file whose artboards have no such keys (older writers) opens with both off.
    let mut old: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for a in old["document"]["artboards"].as_array_mut().unwrap() {
        let a = a.as_object_mut().unwrap();
        a.remove("show_center_mark");
        a.remove("show_cross_hairs");
    }
    let back = vectorcraft_format::load(&serde_json::to_vec(&old).unwrap()).unwrap();
    assert!(back.artboards.iter().all(|a| !a.show_center_mark && !a.show_cross_hairs));
}

/// Show Center Mark, Show Cross Hairs, Show Video Safe Areas and the Video Ruler Pixel Aspect Ratio
/// are screen-only: SVG and PDF exports (and print, which prints the PDF) come out the same with
/// them on as with them off.
#[test]
fn artboard_display_marks_stay_out_of_exports() {
    let mut s = session();
    rect(&mut s);
    let doc = |s: &Session| (*s.doc().unwrap().doc).clone();
    let off = doc(&s);
    s.execute(
        "artboard.setProps",
        &json!({"index": 0, "showCenterMark": true, "showCrossHairs": true, "showVideoSafeAreas": true, "videoRulerPixelAspectRatio": 2}),
    )
    .unwrap();
    let on = doc(&s);
    assert_ne!(on.artboards, off.artboards);
    assert_eq!(vectorcraft_svg::export(&on, &Default::default()), vectorcraft_svg::export(&off, &Default::default()));
    assert_eq!(crate::export_pdf(&on, &Default::default()).unwrap(), crate::export_pdf(&off, &Default::default()).unwrap());
}

/// Artboard Options' video display options: `artboard.setProps {showVideoSafeAreas,
/// videoRulerPixelAspectRatio}` sets each on that artboard only, in one undo step, also for a
/// locked artboard and in an `artboards` list. A change without them leaves them, a duplicate
/// keeps them, `artboard.new` takes them, `inspect_document` reports them, and a value that isn't
/// true or false, or a ratio outside 0.1–10, is refused and changes nothing.
#[test]
fn artboard_video_display_options_are_set_in_one_undo_step() {
    let mut s = session();
    s.execute("artboard.new", &json!({})).unwrap();
    let video = |s: &Session, i: usize| {
        let a = &s.doc().unwrap().doc.artboards[i];
        (a.show_video_safe_areas, a.video_ruler_pixel_aspect_ratio)
    };
    let undo = |s: &Session| s.doc().unwrap().history.undo.len();
    assert_eq!((video(&s, 0), video(&s, 1)), ((false, 1.0), (false, 1.0)), "off and square by default");
    let u0 = undo(&s);
    s.execute("artboard.setProps", &json!({"index": 1, "showVideoSafeAreas": true})).unwrap();
    assert_eq!((video(&s, 0), video(&s, 1)), ((false, 1.0), (true, 1.0)), "only that artboard");
    s.execute("artboard.setProps", &json!({"index": 1, "videoRulerPixelAspectRatio": 0.9})).unwrap();
    assert_eq!(video(&s, 1), (true, 0.9));
    s.execute("artboard.setProps", &json!({"index": 1, "name": "Video"})).unwrap();
    assert_eq!(video(&s, 1), (true, 0.9), "a change without them leaves them");
    assert_eq!(undo(&s), u0 + 3, "one undo step each");
    for was in [(true, 0.9), (true, 1.0), (false, 1.0)] {
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(video(&s, 1), was);
    }
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(video(&s, 1), (true, 1.0));
    // A locked artboard keeps its rectangle, not its video display options.
    s.execute("artboard.setProps", &json!({"index": 0, "locked": true})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 0, "videoRulerPixelAspectRatio": 2})).unwrap();
    assert_eq!(video(&s, 0), (false, 2.0));
    let u = undo(&s);
    s.execute(
        "artboard.setProps",
        &json!({"artboards": [{"index": 0, "showVideoSafeAreas": true}, {"index": 1, "showVideoSafeAreas": false, "videoRulerPixelAspectRatio": 1.2}]}),
    )
    .unwrap();
    assert_eq!((video(&s, 0), video(&s, 1)), ((true, 2.0), (false, 1.2)), "each listed artboard takes its own");
    assert_eq!(undo(&s), u + 1, "in one undo step");
    let doc = crate::inspect::document(&s);
    assert_eq!(doc["artboards"][0]["showVideoSafeAreas"], json!(true));
    assert_eq!(
        (doc["artboards"][0]["videoRulerPixelAspectRatio"].clone(), doc["artboards"][1]["videoRulerPixelAspectRatio"].clone()),
        (json!(2.0), json!(1.2))
    );
    assert!(doc["artboards"][1].get("showVideoSafeAreas").is_none(), "{doc}");
    s.execute("artboard.setProps", &json!({"index": 1, "videoRulerPixelAspectRatio": 1})).unwrap();
    assert!(crate::inspect::document(&s)["artboards"][1].get("videoRulerPixelAspectRatio").is_none(), "square pixels aren't reported");
    let copy = s.execute("artboard.duplicate", &json!({"index": 0})).unwrap()["index"].as_u64().unwrap() as usize;
    assert_eq!(video(&s, copy), (true, 2.0), "a duplicate keeps them");
    let made = s.execute("artboard.new", &json!({"showVideoSafeAreas": true, "videoRulerPixelAspectRatio": 0.1})).unwrap()["index"].as_u64().unwrap()
        as usize;
    assert_eq!(video(&s, made), (true, 0.1), "a new artboard takes them too");
    let before = s.doc().unwrap().doc.clone();
    for bad in [json!("yes"), json!(1)] {
        assert!(s.execute("artboard.setProps", &json!({"index": 1, "showVideoSafeAreas": bad})).is_err());
        assert!(s.execute("artboard.new", &json!({"showVideoSafeAreas": bad})).is_err());
    }
    for bad in [json!(0.09), json!(10.5), json!(0), json!(-1), json!("2"), json!(null)] {
        assert!(s.execute("artboard.setProps", &json!({"index": 1, "videoRulerPixelAspectRatio": bad})).is_err(), "{bad}");
        assert!(s.execute("artboard.new", &json!({"videoRulerPixelAspectRatio": bad})).is_err(), "{bad}");
    }
    assert!(
        s.execute(
            "artboard.setProps",
            &json!({"artboards": [{"index": 0, "showVideoSafeAreas": false}, {"index": 1, "videoRulerPixelAspectRatio": 11}]})
        )
        .is_err()
    );
    assert_eq!(*s.doc().unwrap().doc, *before, "a bad value changes nothing");
    s.execute("artboard.setProps", &json!({"index": 1, "videoRulerPixelAspectRatio": 10})).unwrap();
    assert_eq!(video(&s, 1), (false, 10.0), "10 is in range");
}

/// The native format keeps Show Video Safe Areas and the Video Ruler Pixel Aspect Ratio. A
/// document whose artboards hide the safe areas and have square pixels is written byte for byte as
/// before they could be set, a file written without the two keys opens with the safe areas off
/// and square pixels, and a file's ratio outside 0.1–10 opens brought into that range.
#[test]
fn artboard_video_display_options_round_trip_through_the_native_format() {
    let mut s = session();
    s.execute("artboard.new", &json!({})).unwrap();
    let save = |s: &Session| vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let before = save(&s);
    s.execute(
        "artboard.setProps",
        &json!({"artboards": [{"index": 0, "showVideoSafeAreas": true}, {"index": 1, "videoRulerPixelAspectRatio": 0.9}]}),
    )
    .unwrap();
    let bytes = save(&s);
    let saved: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let ab = |i: usize| saved["document"]["artboards"][i].clone();
    assert_eq!(ab(0)["show_video_safe_areas"], json!(true));
    assert!(ab(0).get("video_ruler_pixel_aspect_ratio").is_none(), "square pixels aren't written: {}", ab(0));
    assert_eq!(ab(1)["video_ruler_pixel_aspect_ratio"], json!(0.9));
    assert!(ab(1).get("show_video_safe_areas").is_none(), "hidden safe areas aren't written: {}", ab(1));
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(back.artboards, s.doc().unwrap().doc.artboards);
    assert_eq!((back.artboards[0].show_video_safe_areas, back.artboards[1].video_ruler_pixel_aspect_ratio), (true, 0.9));
    s.execute("artboard.setProps", &json!({"artboards": [{"index": 0, "showVideoSafeAreas": false}, {"index": 1, "videoRulerPixelAspectRatio": 1}]}))
        .unwrap();
    assert_eq!(save(&s), before, "off and square again, the file is as it was");
    // A file whose artboards have no such keys (older writers) opens with them off and square.
    let mut old: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for a in old["document"]["artboards"].as_array_mut().unwrap() {
        let a = a.as_object_mut().unwrap();
        a.remove("show_video_safe_areas");
        a.remove("video_ruler_pixel_aspect_ratio");
    }
    let back = vectorcraft_format::load(&serde_json::to_vec(&old).unwrap()).unwrap();
    assert!(back.artboards.iter().all(|a| !a.show_video_safe_areas && a.video_ruler_pixel_aspect_ratio == 1.0));
    for (written, opens) in [(json!(0), 0.1), (json!(-3), 0.1), (json!(1e9), 10.0), (json!(2.5), 2.5)] {
        let mut file: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        file["document"]["artboards"][1]["video_ruler_pixel_aspect_ratio"] = written.clone();
        let back = vectorcraft_format::load(&serde_json::to_vec(&file).unwrap()).unwrap();
        assert_eq!(back.artboards[1].video_ruler_pixel_aspect_ratio, opens, "{written}");
    }
}

#[test]
fn swatch_groups_duplicate_sort() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Zed", "color": "#123456"})).unwrap();
    s.execute("swatch.new", &json!({"name": "Alpha", "color": "#654321"})).unwrap();
    let r = s.execute("swatch.newGroup", &json!({"name": "Mine", "swatches": ["Zed"], "colors": ["#ff0000"]})).unwrap();
    assert_eq!(r["name"], "Mine");
    let d = &s.doc().unwrap().doc;
    let g = d.swatch_groups.iter().find(|g| g.name == "Mine").unwrap();
    assert_eq!(g.swatches.len(), 2);
    assert!(!d.swatches.iter().any(|x| x.name == "Zed"));
    let r = s.execute("swatch.duplicate", &json!({"name": "Alpha"})).unwrap();
    assert_eq!(r["name"], "Alpha copy");
    s.execute("swatch.sortByName", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.swatches[0].name.starts_with('['));
    let names: Vec<_> = d.swatches.iter().skip(1).map(|x| x.name.to_lowercase()).collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
}

#[test]
fn graphic_style_delete_duplicate() {
    let mut s = session();
    let id = rect(&mut s);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graphicStyle.new", &json!({"name": "S1"})).unwrap();
    assert_eq!(s.execute("graphicStyle.duplicate", &json!({"name": "S1"})).unwrap()["name"], "S1 copy");
    s.execute("graphicStyle.delete", &json!({"name": "S1"})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.graphic_styles.iter().all(|g| g.name != "S1"));
    assert!(s.execute("graphicStyle.delete", &json!({"name": "nope"})).is_err());
}

#[test]
fn text_set_format() {
    let mut s = session();
    let r = s.execute("text.create", &json!({"x": 10, "y": 100, "text": "Hello"})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute(
        "text.setFormat",
        &json!({"ids": [id.0], "kerning": 20, "baselineShift": 3, "hScale": 120, "rotation": 370, "underline": true, "spaceBefore": 6, "hyphenate": true}),
    )
    .unwrap();
    let NodeKind::Text(t) = node(&s, id).kind else { panic!() };
    let st = t.first_style();
    assert_eq!(st.kerning, Some(20.0));
    assert_eq!(st.baseline_shift, 3.0);
    assert_eq!(st.h_scale, 120.0);
    assert!((st.rotation - 10.0).abs() < 1e-9);
    assert!(st.underline);
    assert_eq!(t.para.space_before, 6.0);
    assert!(t.para.hyphenate);
    s.execute("text.setFormat", &json!({"ids": [id.0], "kerning": "auto"})).unwrap();
    let NodeKind::Text(t) = node(&s, id).kind else { panic!() };
    assert_eq!(t.first_style().kerning, None);
    assert!(s.execute("text.setFormat", &json!({"ids": [id.0]})).is_err());
    // Paragraph composer: Every-line by default, undoable.
    assert_eq!(t.para.composer, vectorcraft_doc::Composer::EveryLine);
    s.execute("text.setFormat", &json!({"ids": [id.0], "composer": "singleLine"})).unwrap();
    let NodeKind::Text(t) = node(&s, id).kind else { panic!() };
    assert_eq!(t.para.composer, vectorcraft_doc::Composer::SingleLine);
    assert!(s.execute("text.setFormat", &json!({"ids": [id.0], "composer": "best"})).is_err());
    assert!(s.execute("text.setFormat", &json!({"ids": [id.0], "composer": 1})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    let NodeKind::Text(t) = node(&s, id).kind else { panic!() };
    assert_eq!(t.para.composer, vectorcraft_doc::Composer::EveryLine);
}

#[test]
fn stroke_advanced_scale_swap_flip() {
    let mut s = session();
    let id = rect(&mut s);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("stroke.set", &json!({"startArrow": "Triangle", "profile": "taperEnd"})).unwrap();
    s.execute("stroke.setAdvanced", &json!({"arrowScale": [50, 200]})).unwrap();
    s.execute("stroke.setAdvanced", &json!({"swapArrows": true, "flipProfile": "along"})).unwrap();
    let n = node(&s, id);
    let st = n.appearance.stroke().unwrap();
    assert_eq!(st.start_arrow, None);
    assert_eq!(st.end_arrow, Some(vectorcraft_doc::Arrowhead::Triangle));
    assert_eq!(st.arrow_scale, (200.0, 50.0));
    assert_eq!(st.profile.as_ref().unwrap().points, vectorcraft_doc::WidthProfile::taper_start().points);
    assert!(s.execute("stroke.setAdvanced", &json!({})).is_err());
    assert!(s.execute("stroke.setAdvanced", &json!({"flipProfile": "sideways"})).is_err());
}

/// A document of `n` 200 × 150 artboards in a row, 400 pt apart (`x` = 400·i), each with a
/// guide of its own.
fn row_of_artboards(n: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 150})).unwrap();
    for i in 1..n {
        s.execute("artboard.new", &json!({"x": 400 * i, "y": 0, "width": 200, "height": 150})).unwrap();
    }
    for i in 0..n {
        s.execute("guide.add", &json!({"vertical": true, "pos": 400 * i + 50, "artboard": i})).unwrap();
    }
    s
}

fn box_at(s: &mut Session, x: f64, w: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": w, "height": 20})).unwrap()["id"].as_u64().unwrap())
}

fn artboard_names(s: &Session) -> Vec<String> {
    s.doc().unwrap().doc.artboards.iter().map(|a| a.name.clone()).collect()
}

/// Artboards panel › Delete Empty Artboards: with art only on artboard 2, artboards 1 and 3 go
/// with their guides in one undo step, the art and its ids untouched; Undo brings them back.
#[test]
fn delete_empty_artboards_deletes_those_with_no_art_in_one_step() {
    let mut s = row_of_artboards(3);
    let art = box_at(&mut s, 450.0, 20.0);
    let before = s.doc().unwrap().doc.clone();
    let steps = s.doc().unwrap().history.undo.len();
    assert_eq!(s.execute("artboard.deleteEmpty", &json!({})).unwrap(), json!({"deleted": [0, 2]}));
    let st = s.doc().unwrap();
    assert_eq!(artboard_names(&s), ["Artboard 2"]);
    assert_eq!(st.doc.guides.len(), 1, "the deleted artboards' guides go with them");
    assert_eq!(st.doc.node(art).map(|n| n.geometric_bounds()), before.node(art).map(|n| n.geometric_bounds()), "the art stays");
    assert_eq!(st.history.undo.len(), steps + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(artboard_names(&s), ["Artboard 1", "Artboard 2", "Artboard 3"]);
    assert_eq!(s.doc().unwrap().doc.guides.len(), 3);
}

/// What counts as art: hidden and locked objects, an object straddling two artboards and one
/// that only touches an artboard's edge; a group counts by its members, not by its box.
#[test]
fn delete_empty_artboards_keeps_artboards_with_hidden_locked_or_shared_art() {
    let mut s = row_of_artboards(5);
    // Hidden art on 1, locked art on 2.
    box_at(&mut s, 10.0, 20.0);
    s.execute("object.hide", &json!({})).unwrap();
    box_at(&mut s, 410.0, 20.0);
    s.execute("object.lock", &json!({})).unwrap();
    // Straddling 3 (x 800–1000) and 4 (x 1200–1400).
    box_at(&mut s, 900.0, 350.0);
    // A group of a box on 3 and one whose right edge touches 6's left edge (x 2000): its box spans
    // 5 (x 1600–1800), which stays empty, and 6 has art only by the touch.
    s.execute("artboard.new", &json!({"x": 2000, "y": 0, "width": 200, "height": 150})).unwrap();
    let a = box_at(&mut s, 850.0, 20.0);
    let b = box_at(&mut s, 1980.0, 20.0);
    // No stroke, so its visual box ends at x 2000.
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    assert_eq!(s.execute("artboard.deleteEmpty", &json!({})).unwrap(), json!({"deleted": [4]}));
    assert_eq!(artboard_names(&s), ["Artboard 1", "Artboard 2", "Artboard 3", "Artboard 4", "Artboard 6"]);
}

/// A locked empty artboard stays; with every artboard empty artboard 1 stays; with none empty
/// the command is disabled, refuses and adds no undo step.
#[test]
fn delete_empty_artboards_leaves_locked_ones_and_artboard_1() {
    let mut s = row_of_artboards(3);
    s.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();
    assert_eq!(s.execute("artboard.deleteEmpty", &json!({})).unwrap(), json!({"deleted": [0, 2]}), "all empty, one locked");
    assert_eq!(artboard_names(&s), ["Artboard 2"]);

    let mut s = row_of_artboards(3);
    assert_eq!(s.execute("artboard.deleteEmpty", &json!({})).unwrap(), json!({"deleted": [1, 2]}), "all empty: artboard 1 stays");
    assert_eq!(artboard_names(&s), ["Artboard 1"]);
    let steps = s.doc().unwrap().history.undo.len();
    let spec = crate::cmd::command_specs().iter().find(|c| c.id == "artboard.deleteEmpty").unwrap();
    assert!((spec.enabled)(&s).is_err(), "one artboard left: nothing to delete");
    assert!(s.execute("artboard.deleteEmpty", &json!({})).is_err());
    box_at(&mut s, 10.0, 20.0);
    let steps_with_art = s.doc().unwrap().history.undo.len();
    assert!(s.execute("artboard.deleteEmpty", &json!({})).is_err(), "nothing empty");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps_with_art);
    assert!(steps_with_art > steps);
}

/// Art outside a clip shows nowhere: a clip group or a clipping layer has art only where its
/// members show inside the clip, so an artboard that only the clipped-away part of a member
/// covers is empty.
#[test]
fn delete_empty_artboards_measures_clipped_art_through_the_clip() {
    // Clip group: the clip on 1, a member running on to 2. Clipping layer: the clip on 3, a member
    // running on to 4. Nothing else.
    let mut s = row_of_artboards(4);
    let long = box_at(&mut s, 50.0, 400.0);
    let clip = box_at(&mut s, 10.0, 100.0);
    // The front object clips (drawn after the member).
    s.execute("select.set", &json!({"ids": [long.0, clip.0]})).unwrap();
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    s.execute("layer.new", &json!({"name": "Clipped"})).unwrap();
    box_at(&mut s, 850.0, 400.0);
    // The layer's top object becomes its clipping path.
    let layer_clip = box_at(&mut s, 810.0, 100.0);
    let layer_of =
        |s: &Session| s.doc().unwrap().doc.layers.iter().find(|l| l.children().unwrap().iter().any(|c| c.id == layer_clip)).unwrap().clone();
    let layer = layer_of(&s);
    assert_eq!(layer.children().unwrap().len(), 2, "both in their own layer");
    s.execute("layer.clippingMask.toggle", &json!({"id": layer.id.0})).unwrap();
    let layer = layer_of(&s);
    assert!(matches!(layer.kind, vectorcraft_doc::NodeKind::Layer { clip: true, .. }));
    let first = layer.children().unwrap()[0].id;
    assert_eq!(first, layer_clip, "the layer's first object clips the rest");
    assert_eq!(s.execute("artboard.deleteEmpty", &json!({})).unwrap(), json!({"deleted": [1, 3]}));
    assert_eq!(artboard_names(&s), ["Artboard 1", "Artboard 3"]);
}
