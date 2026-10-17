//! Panel-support commands (`cmd/{panelcmds,gradient,appearance,stroke,swatch,style}.rs`).

use serde_json::json;
use vectorcraft_color::{GradientKind, Paint};
use vectorcraft_doc::{AppearanceItem, NodeKind};

use super::*;

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
    // Isolation mode starts with nothing selected, so leaving it doesn't select the art again.
    s.execute("object.isolate", &json!({"id": pasted[0]})).unwrap();
    assert!(!editable(&s), "in isolation mode");
    s.execute("object.exitIsolation", &json!({})).unwrap();
    assert!(selected(&s).is_empty(), "after isolation mode");
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
