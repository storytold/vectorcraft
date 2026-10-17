//! Panel-support commands (`cmd/{panelcmds,gradient,appearance,stroke,swatch,style}.rs`).

use serde_json::json;
use vectorcraft_color::{Color, GradientKind, Paint};
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

/// An artboard's background color (Properties › Artboards › Fill): `artboard.setProps {color}` sets
/// it and `{color: null}` clears it, each in one undo step. A change without `color` leaves it, a
/// duplicate keeps it, `inspect_document` reports it, `artboard.new {color}` starts with it, and a
/// bad color is refused.
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
    let before = s.doc().unwrap().doc.clone();
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
