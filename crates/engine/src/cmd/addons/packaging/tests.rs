//! Tests for Packaging Tools: fold line roles, their swatches and the role table.

use std::collections::BTreeSet;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, Node, NodeKind};

use crate::*;

fn session(mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400, "colorMode": mode})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": 20, "width": 40, "height": 40})).unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

fn steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn set_role(s: &mut Session, p: Value) -> Value {
    s.execute("addon.packaging.setRole", &p).unwrap()
}

fn err(s: &mut Session, cmd: &str, p: Value) -> String {
    s.execute(cmd, &p).unwrap_err().to_string()
}

fn b64(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().expect("dataBase64")).unwrap()
}

/// The `roles` query's counts, in role order.
fn counts(s: &mut Session, p: Value) -> Vec<u64> {
    let r = s.execute("addon.packaging.roles", &p).unwrap();
    r["roles"].as_array().unwrap().iter().map(|r| r["count"].as_u64().unwrap()).collect()
}

#[test]
fn set_role_makes_the_spot_swatch_and_links_the_top_stroke() {
    let mut s = session("cmyk");
    let a = rect(&mut s, 20.0);
    assert!(s.doc().unwrap().doc.swatch("Cut").is_none());
    let before = steps(&s);
    let r = set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    assert_eq!((r["changed"].as_u64(), r["skipped"].as_u64()), (Some(1), Some(0)));
    assert_eq!(steps(&s), before + 1, "one undo step");
    let d = &s.doc().unwrap().doc;
    let sw = d.swatch("Cut").unwrap();
    assert!(sw.global && sw.spot, "{sw:?}");
    assert_eq!(sw.paint, Paint::solid(Color::cmyk(0.0, 1.0, 0.0, 0.0)), "100% magenta in a CMYK document");
    let st = node(&s, a).appearance.stroke().cloned().unwrap();
    assert!(matches!(&st.paint, Paint::Solid { swatch: Some(n), tint, .. } if n == "Cut" && *tint == 1.0), "{:?}", st.paint);
    assert!(st.overprint, "role strokes overprint by default");
    // Again: nothing changes, and no undo step is recorded.
    let r = set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    assert_eq!(r["changed"].as_u64(), Some(0));
    assert_eq!(steps(&s), before + 1);
    // Undo takes the swatch away with the role.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.swatch("Cut").is_none());
}

#[test]
fn each_role_gets_its_own_swatch_color_in_the_document_s_mode() {
    let mut s = session("rgb");
    for (i, role) in ["cut", "crease", "partialCut", "perforation"].into_iter().enumerate() {
        let id = rect(&mut s, 20.0 + 60.0 * i as f64);
        set_role(&mut s, json!({"role": role, "ids": [id.0]}));
    }
    let d = &s.doc().unwrap().doc;
    let colors: Vec<Color> = ["Cut", "Crease", "Partial Cut", "Perforation"].iter().map(|n| d.swatch(n).unwrap().paint.color().unwrap()).collect();
    assert!(colors.iter().all(|c| matches!(c, Color::Rgb { .. })), "an RGB document's swatches are RGB: {colors:?}");
    assert_eq!(colors.iter().map(|c| c.to_hex()).collect::<BTreeSet<_>>().len(), 4, "four colors: {colors:?}");
}

#[test]
fn a_path_without_a_stroke_gets_one_and_overprint_can_be_left_off() {
    let mut s = session("cmyk");
    let a = rect(&mut s, 20.0);
    s.edit("strip", |d, _| {
        let n = d.node_mut(a).unwrap();
        n.appearance.items.retain(AppearanceItem::is_fill);
        Ok(())
    })
    .unwrap();
    assert!(node(&s, a).appearance.stroke().is_none());
    set_role(&mut s, json!({"role": "crease", "ids": [a.0], "overprint": false}));
    let st = node(&s, a).appearance.stroke().cloned().unwrap();
    assert_eq!(st.width, 1.0, "a 1 pt stroke");
    assert!(!st.overprint);
    assert!(matches!(&st.paint, Paint::Solid { swatch: Some(n), .. } if n == "Crease"));
    assert!(err(&mut s, "addon.packaging.setRole", json!({"role": "cut", "ids": [a.0], "overprint": "yes"})).contains("overprint"));
}

#[test]
fn none_takes_the_role_away_and_keeps_the_color() {
    let mut s = session("cmyk");
    let (a, b) = (rect(&mut s, 20.0), rect(&mut s, 100.0));
    set_role(&mut s, json!({"role": "perforation", "ids": [a.0]}));
    let shown = node(&s, a).appearance.stroke().unwrap().paint.color().unwrap();
    let before = steps(&s);
    let r = set_role(&mut s, json!({"role": "none", "ids": [a.0, b.0]}));
    assert_eq!(r["changed"].as_u64(), Some(1), "the plain rectangle has no role to take away");
    assert_eq!(steps(&s), before + 1);
    let st = node(&s, a).appearance.stroke().cloned().unwrap();
    assert_eq!(st.paint, Paint::solid(shown));
    assert!(!st.overprint);
    assert_eq!(counts(&mut s, json!({"ids": [a.0, b.0]})), [0, 0, 0, 0]);
    // Nothing left with a role: no undo step.
    set_role(&mut s, json!({"role": "none", "ids": [a.0, b.0]}));
    assert_eq!(steps(&s), before + 1);
}

#[test]
fn bad_roles_and_targets_are_refused() {
    let mut s = session("cmyk");
    let a = rect(&mut s, 20.0);
    assert!(err(&mut s, "addon.packaging.setRole", json!({"role": "fold", "ids": [a.0]})).contains("unknown role"));
    assert!(err(&mut s, "addon.packaging.setRole", json!({"ids": [a.0]})).contains("role"));
    assert!(err(&mut s, "addon.packaging.setRole", json!({"role": 3, "ids": [a.0]})).contains("role"));
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute("addon.packaging.setRole", &json!({"role": "cut"})).is_err(), "nothing selected");
    let t = id_of(&s.execute("text.create", &json!({"x": 20, "y": 200, "text": "Lid", "size": 20})).unwrap());
    let before = steps(&s);
    assert!(err(&mut s, "addon.packaging.setRole", json!({"role": "cut", "ids": [t.0]})).contains("select a path"));
    assert_eq!(steps(&s), before);
    assert!(s.doc().unwrap().doc.swatch("Cut").is_none(), "a refused run makes no swatch");
    // Text beside a path is skipped and counted.
    let r = set_role(&mut s, json!({"role": "cut", "ids": [a.0, t.0]}));
    assert_eq!((r["changed"].as_u64(), r["skipped"].as_u64()), (Some(1), Some(1)));
}

#[test]
fn groups_are_walked_and_their_locked_and_hidden_members_left_out() {
    let mut s = session("cmyk");
    let (a, b, c) = (rect(&mut s, 20.0), rect(&mut s, 100.0), rect(&mut s, 180.0));
    s.execute("select.set", &json!({"ids": [a.0, b.0, c.0]})).unwrap();
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    s.edit("lock", |d, _| {
        d.node_mut(b).unwrap().locked = true;
        d.node_mut(c).unwrap().visible = false;
        Ok(())
    })
    .unwrap();
    let r = set_role(&mut s, json!({"role": "crease", "ids": [g.0]}));
    assert_eq!(r["ids"], json!([a.0]));
    assert_eq!(r["skipped"].as_u64(), Some(2), "the locked and the hidden member");
    assert_eq!(counts(&mut s, json!({"ids": [g.0]})), [0, 1, 0, 0]);
    // Named directly, a locked or hidden path is left out too.
    let r = set_role(&mut s, json!({"role": "cut", "ids": [a.0, b.0, c.0]}));
    assert_eq!((r["ids"].clone(), r["skipped"].as_u64()), (json!([a.0]), Some(2)));
    let before = steps(&s);
    assert!(err(&mut s, "addon.packaging.setRole", json!({"role": "cut", "ids": [b.0]})).contains("locked"));
    assert_eq!(steps(&s), before);
    assert_eq!(counts(&mut s, json!({"ids": [b.0, c.0]})), [0, 0, 0, 0]);
}

#[test]
fn a_clip_group_s_clipping_path_is_skipped_and_counted() {
    let mut s = session("cmyk");
    let (art, mask) = (rect(&mut s, 20.0), rect(&mut s, 30.0));
    s.execute("select.set", &json!({"ids": [art.0, mask.0]})).unwrap();
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    let r = set_role(&mut s, json!({"role": "cut", "ids": [g.0]}));
    assert_eq!((r["ids"].clone(), r["skipped"].as_u64()), (json!([art.0]), Some(1)), "the clipping path is left out and counted");
}

#[test]
fn a_group_and_its_member_count_once_and_locked_lines_count_in_every_scope() {
    let mut s = session("cmyk");
    let (a, b) = (rect(&mut s, 20.0), rect(&mut s, 100.0));
    set_role(&mut s, json!({"role": "cut", "ids": [b.0]}));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    let r = set_role(&mut s, json!({"role": "crease", "ids": [g.0, a.0]}));
    assert_eq!(r["ids"], json!([a.0, b.0]), "each path once");
    assert_eq!(counts(&mut s, json!({"ids": [g.0, a.0]})), [0, 2, 0, 0]);
    set_role(&mut s, json!({"role": "cut", "ids": [b.0]}));
    s.edit("lock", |d, _| {
        d.node_mut(b).unwrap().locked = true;
        Ok(())
    })
    .unwrap();
    // A locked line keeps its role and is counted, whether its group is selected or not.
    assert_eq!(counts(&mut s, json!({"ids": [g.0]})), [1, 1, 0, 0]);
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(counts(&mut s, json!({})), [1, 1, 0, 0]);
}

#[test]
fn a_process_swatch_named_like_a_role_swatch_keeps_its_name() {
    let mut s = session("cmyk");
    let a = rect(&mut s, 20.0);
    s.execute("swatch.new", &json!({"name": "Cut", "color": "#ff0000"})).unwrap();
    set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.unknown.get("packagingRoles").unwrap()["cut"], json!("Cut 2"));
    assert!(d.swatch("Cut 2").is_some_and(|w| w.spot), "the role swatch is made beside it");
    assert!(d.swatch("Cut").is_some_and(|w| !w.global), "the process swatch is left as it was");
    assert_eq!(counts(&mut s, json!({"ids": [a.0]})), [1, 0, 0, 0]);
}

#[test]
fn the_free_role_swatch_name_is_searched_past_ten_thousand() {
    let mut s = session("cmyk");
    let a = rect(&mut s, 20.0);
    let taken = std::iter::once("Cut".to_string()).chain((2..10_000).map(|i| format!("Cut {i}"))).chain(["Cut 10004".to_string()]);
    let process = |name: String| vectorcraft_color::Swatch { name, paint: Paint::solid(Color::rgb(1.0, 0.0, 0.0)), global: false, spot: false };
    Arc::make_mut(&mut s.doc_mut().unwrap().doc).swatches.extend(taken.map(process));
    set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.unknown.get("packagingRoles").unwrap()["cut"], json!("Cut 10000"));
    assert!(d.swatch("Cut 10000").is_some_and(|w| w.spot), "the role swatch is made under the first free name");
    assert!(d.swatch("Cut 10004").is_some_and(|w| !w.global), "a taken name is left as it was");
}

#[test]
fn a_merged_role_swatch_hands_its_role_to_the_kept_swatch() {
    let mut s = session("cmyk");
    let (a, b) = (rect(&mut s, 20.0), rect(&mut s, 100.0));
    set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    set_role(&mut s, json!({"role": "crease", "ids": [b.0]}));
    s.execute("swatch.new", &json!({"name": "Die", "color": {"model": "cmyk", "c": 0, "m": 0.8, "y": 0.2, "k": 0}, "spot": true})).unwrap();
    s.execute("swatch.merge", &json!({"names": ["Die", "Cut"]})).unwrap();
    assert_eq!(s.doc().unwrap().doc.unknown.get("packagingRoles").unwrap()["cut"], json!("Die"));
    assert_eq!(counts(&mut s, json!({"ids": [a.0]})), [1, 0, 0, 0], "the cut line is still a cut line");
    // A kept swatch that already carries a role keeps it; the merged role's lines take it.
    s.execute("swatch.merge", &json!({"names": ["Crease", "Die"]})).unwrap();
    let table = s.doc().unwrap().doc.unknown.get("packagingRoles").cloned().unwrap();
    assert_eq!((&table["cut"], &table["crease"]), (&json!("Die"), &json!("Crease")), "{table}");
    assert_eq!(counts(&mut s, json!({"ids": [a.0, b.0]})), [0, 2, 0, 0]);
    // The next Make Cut Line makes the Cut role's swatch again.
    set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    assert!(s.doc().unwrap().doc.swatch("Die").is_some_and(|w| w.spot));
    assert_eq!(counts(&mut s, json!({"ids": [a.0, b.0]})), [1, 1, 0, 0]);
}

#[test]
fn a_global_process_swatch_can_carry_a_role() {
    let mut s = session("cmyk");
    let a = rect(&mut s, 20.0);
    s.execute("swatch.new", &json!({"name": "Die", "color": {"model": "cmyk", "c": 0, "m": 0.8, "y": 0.2, "k": 0}, "global": true})).unwrap();
    s.execute("addon.packaging.setRoleSwatch", &json!({"role": "partialCut", "swatch": "Die"})).unwrap();
    set_role(&mut s, json!({"role": "partialCut", "ids": [a.0]}));
    assert!(matches!(&node(&s, a).appearance.stroke().unwrap().paint, Paint::Solid { swatch: Some(n), .. } if n == "Die"));
    assert_eq!(counts(&mut s, json!({"ids": [a.0]})), [0, 0, 1, 0]);
}

#[test]
fn roles_counts_the_selection_or_the_whole_document() {
    let mut s = session("cmyk");
    let (a, b, c) = (rect(&mut s, 20.0), rect(&mut s, 100.0), rect(&mut s, 180.0));
    set_role(&mut s, json!({"role": "cut", "ids": [a.0, b.0]}));
    set_role(&mut s, json!({"role": "partialCut", "ids": [c.0]}));
    s.execute("select.set", &json!({"ids": [a.0, c.0]})).unwrap();
    let r = s.execute("addon.packaging.roles", &json!({})).unwrap();
    assert_eq!((r["scope"].as_str(), r["selected"].as_u64()), (Some("selection"), Some(2)));
    assert_eq!(counts(&mut s, json!({})), [1, 0, 1, 0]);
    s.execute("select.none", &json!({})).unwrap();
    let r = s.execute("addon.packaging.roles", &json!({})).unwrap();
    assert_eq!((r["scope"].as_str(), r["selected"].as_u64()), (Some("document"), Some(0)));
    assert_eq!(counts(&mut s, json!({})), [2, 0, 1, 0]);
    let rows = r["roles"].as_array().unwrap();
    assert_eq!(rows[0], json!({"role": "cut", "name": "Cut", "swatch": "Cut", "exists": true, "count": 2}));
    assert_eq!(rows[1]["exists"], json!(false), "no Crease swatch yet");
}

#[test]
fn role_swatches_maps_a_role_to_another_swatch() {
    let mut s = session("cmyk");
    let a = rect(&mut s, 20.0);
    set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    s.execute("swatch.new", &json!({"name": "Die", "color": {"model": "cmyk", "c": 0, "m": 0.8, "y": 0.2, "k": 0}, "spot": true})).unwrap();
    let before = steps(&s);
    let r = s.execute("addon.packaging.setRoleSwatch", &json!({"role": "cut", "swatch": "Die"})).unwrap();
    assert_eq!(r["changed"], json!(true));
    assert_eq!(steps(&s), before + 1);
    assert_eq!(s.doc().unwrap().doc.unknown.get("packagingRoles").unwrap()["cut"], json!("Die"));
    // The old swatch's objects keep their color and lose the role.
    assert_eq!(counts(&mut s, json!({"ids": [a.0]})), [0, 0, 0, 0]);
    set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    assert!(matches!(&node(&s, a).appearance.stroke().unwrap().paint, Paint::Solid { swatch: Some(n), .. } if n == "Die"));
    // The same mapping again changes nothing.
    let steps_now = steps(&s);
    let r = s.execute("addon.packaging.setRoleSwatch", &json!({"role": "cut", "swatch": "Die"})).unwrap();
    assert_eq!((r["changed"].clone(), steps(&s)), (json!(false), steps_now));
}

#[test]
fn a_renamed_role_swatch_keeps_its_role() {
    let mut s = session("cmyk");
    let a = rect(&mut s, 20.0);
    set_role(&mut s, json!({"role": "crease", "ids": [a.0]}));
    s.execute("swatch.edit", &json!({"name": "Crease", "newName": "Score"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.unknown.get("packagingRoles").unwrap()["crease"], json!("Score"));
    assert_eq!(counts(&mut s, json!({"ids": [a.0]})), [0, 1, 0, 0]);
    // A swatch that carries no role leaves the table alone.
    s.execute("swatch.new", &json!({"name": "Ink", "color": {"model": "cmyk", "c": 0, "m": 1, "y": 1, "k": 0}, "spot": true})).unwrap();
    s.execute("swatch.edit", &json!({"name": "Ink", "newName": "Red Ink"})).unwrap();
    let table = s.doc().unwrap().doc.unknown.get("packagingRoles").cloned().unwrap();
    assert_eq!(table, json!({"cut": "Cut", "crease": "Score", "partialCut": "Partial Cut", "perforation": "Perforation"}));
    // Undo puts the old name back in the table with the swatch.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.unknown.get("packagingRoles").is_none());
    assert_eq!(counts(&mut s, json!({"ids": [a.0]})), [0, 1, 0, 0]);
}

#[test]
fn a_role_swatch_renamed_to_another_role_s_name_leaves_each_role_its_own_swatch() {
    let mut s = session("cmyk");
    let a = rect(&mut s, 20.0);
    set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    // No Crease swatch exists yet, so the name is free for the Cut swatch.
    s.execute("swatch.edit", &json!({"name": "Cut", "newName": "Crease"})).unwrap();
    let table = s.doc().unwrap().doc.unknown.get("packagingRoles").cloned().unwrap();
    assert_eq!((&table["cut"], &table["crease"]), (&json!("Crease"), &json!("Crease 2")), "{table}");
    assert_eq!(counts(&mut s, json!({"ids": [a.0]})), [1, 0, 0, 0], "the line is still a cut line");
}

#[test]
fn a_role_swatch_that_stops_being_global_hands_its_role_to_a_new_swatch() {
    let mut s = session("cmyk");
    let (a, b) = (rect(&mut s, 20.0), rect(&mut s, 100.0));
    set_role(&mut s, json!({"role": "perforation", "ids": [a.0]}));
    s.execute("swatch.edit", &json!({"name": "Perforation", "spot": false, "global": false})).unwrap();
    assert_eq!(s.doc().unwrap().doc.unknown.get("packagingRoles").unwrap()["perforation"], json!("Perforation 2"));
    assert_eq!(counts(&mut s, json!({"ids": [a.0]})), [0, 0, 0, 0], "the unlinked stroke has no role");
    set_role(&mut s, json!({"role": "perforation", "ids": [b.0]}));
    assert!(s.doc().unwrap().doc.swatch("Perforation 2").is_some_and(|w| w.spot));
    assert_eq!(counts(&mut s, json!({"ids": [b.0]})), [0, 0, 0, 1]);
}

#[test]
fn a_color_group_named_like_a_role_swatch_keeps_its_name() {
    let mut s = session("cmyk");
    let a = rect(&mut s, 20.0);
    s.execute("swatch.newGroup", &json!({"name": "Cut", "colors": ["#ff0000"]})).unwrap();
    set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.unknown.get("packagingRoles").unwrap()["cut"], json!("Cut 2"));
    assert!(d.swatch("Cut 2").is_some_and(|w| w.spot) && d.swatch("Cut").is_none());
    assert_eq!(counts(&mut s, json!({"ids": [a.0]})), [1, 0, 0, 0]);
}

#[test]
fn set_role_makes_a_deleted_role_swatch_again_for_strokes_that_still_name_it() {
    let mut s = session("cmyk");
    let a = rect(&mut s, 20.0);
    set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    s.execute("swatch.delete", &json!({"name": "Cut", "unlink": false})).unwrap();
    assert!(s.doc().unwrap().doc.swatch("Cut").is_none());
    let before = steps(&s);
    let r = set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    assert_eq!(r["changed"].as_u64(), Some(0), "the stroke already names the swatch");
    assert!(s.doc().unwrap().doc.swatch("Cut").is_some_and(|w| w.spot), "the swatch is back");
    assert_eq!(steps(&s), before + 1, "as one undo step");
}

#[test]
fn a_hidden_stroke_is_shown_when_it_takes_a_role() {
    let mut s = session("cmyk");
    let a = rect(&mut s, 20.0);
    s.edit("hide", |d, _| {
        if let Some(AppearanceItem::Stroke(st)) = d.node_mut(a).unwrap().appearance.items.iter_mut().find(|i| !i.is_fill()) {
            st.visible = false;
        }
        Ok(())
    })
    .unwrap();
    set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    assert!(node(&s, a).appearance.stroke().unwrap().visible);
}

#[test]
fn role_swatches_refuses_swatches_that_cannot_carry_a_role() {
    let mut s = session("cmyk");
    let ramp = json!({"type": "linear", "stops": [{"offset": 0, "color": "#000000"}, {"offset": 1, "color": "#ffffff"}]});
    s.execute("swatch.new", &json!({"name": "Ramp", "gradient": ramp})).unwrap();
    s.execute("swatch.new", &json!({"name": "Plain", "color": "#336699"})).unwrap();
    s.execute("swatch.new", &json!({"name": "Ink", "color": {"model": "cmyk", "c": 0, "m": 1, "y": 1, "k": 0}, "spot": true})).unwrap();
    s.execute("swatch.new", &json!({"name": "Ink Tint", "swatch": "Ink", "tint": 0.5})).unwrap();
    s.execute("swatch.new", &json!({"name": "Crease", "color": {"model": "cmyk", "c": 1, "m": 0, "y": 0, "k": 0}, "spot": true})).unwrap();
    let before = steps(&s);
    for (swatch, says) in [
        ("Nope", "no swatch"),
        ("Ramp", "isn't a color swatch"),
        ("Plain", "process color"),
        ("Ink Tint", "tint swatch"),
        ("[None]", "can't carry a role"),
        ("[Registration]", "can't carry a role"),
        ("Crease", "already carries the Crease role"),
    ] {
        let e = err(&mut s, "addon.packaging.setRoleSwatch", json!({"role": "cut", "swatch": swatch}));
        assert!(e.contains(says), "{swatch}: {e}");
    }
    assert!(err(&mut s, "addon.packaging.setRoleSwatch", json!({"role": "none", "swatch": "Ink"})).contains("unknown role"));
    assert!(err(&mut s, "addon.packaging.setRoleSwatch", json!({"role": "cut"})).contains("swatch"));
    assert_eq!(steps(&s), before, "no refusal records an undo step");
}

#[test]
fn a_damaged_role_table_reads_as_the_defaults() {
    let mut s = session("cmyk");
    s.edit("damage", |d, _| {
        d.unknown.insert("packagingRoles".into(), json!({"cut": 5, "crease": "", "partialCut": null, "extra": "x"}));
        Ok(())
    })
    .unwrap();
    let r = s.execute("addon.packaging.roles", &json!({})).unwrap();
    let swatches: Vec<&str> = r["roles"].as_array().unwrap().iter().map(|r| r["swatch"].as_str().unwrap()).collect();
    assert_eq!(swatches, ["Cut", "Crease", "Partial Cut", "Perforation"]);
    s.edit("not an object", |d, _| {
        d.unknown.insert("packagingRoles".into(), json!("Cut"));
        Ok(())
    })
    .unwrap();
    let a = rect(&mut s, 20.0);
    set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    assert_eq!(counts(&mut s, json!({"ids": [a.0]})), [1, 0, 0, 0]);
    // One swatch named for two roles: the later role takes its own default.
    s.edit("duplicate", |d, _| {
        d.unknown.insert("packagingRoles".into(), json!({"cut": "Crease", "crease": "Crease", "partialCut": "Cut"}));
        Ok(())
    })
    .unwrap();
    let r = s.execute("addon.packaging.roles", &json!({})).unwrap();
    let swatches: Vec<&str> = r["roles"].as_array().unwrap().iter().map(|r| r["swatch"].as_str().unwrap()).collect();
    assert_eq!(swatches, ["Crease", "Crease 2", "Cut", "Perforation"]);
    let b = rect(&mut s, 100.0);
    set_role(&mut s, json!({"role": "crease", "ids": [b.0]}));
    assert_eq!(counts(&mut s, json!({"ids": [b.0]})), [0, 1, 0, 0], "the crease line is a crease line");
}

#[test]
fn native_save_and_open_keep_the_role_table_and_the_roles() {
    let mut s = session("cmyk");
    let a = rect(&mut s, 20.0);
    s.execute("swatch.new", &json!({"name": "Die", "color": {"model": "cmyk", "c": 0, "m": 0.8, "y": 0.2, "k": 0}, "spot": true})).unwrap();
    s.execute("addon.packaging.setRoleSwatch", &json!({"role": "cut", "swatch": "Die"})).unwrap();
    set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    let saved = s.execute("document.export", &json!({"format": "vectorcraft"})).unwrap();
    s.execute("document.open", &json!({"name": "box.vectorcraft", "dataBase64": saved["dataBase64"]})).unwrap();
    assert_eq!(s.doc().unwrap().doc.unknown.get("packagingRoles").unwrap()["cut"], json!("Die"));
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(counts(&mut s, json!({})), [1, 0, 0, 0]);
}

/// A document with one rectangle of each of Cut and Crease.
fn two_roles() -> Session {
    let mut s = session("cmyk");
    let (a, b) = (rect(&mut s, 20.0), rect(&mut s, 100.0));
    set_role(&mut s, json!({"role": "cut", "ids": [a.0]}));
    set_role(&mut s, json!({"role": "crease", "ids": [b.0]}));
    s
}

fn export(s: &mut Session, format: &str) -> String {
    String::from_utf8_lossy(&b64(&s.execute("document.export", &json!({"format": format})).unwrap())).into_owned()
}

#[test]
fn pdf_prints_each_role_on_its_swatch_s_separation_and_overprints_it() {
    let mut s = two_roles();
    let c = rect(&mut s, 180.0);
    set_role(&mut s, json!({"role": "partialCut", "ids": [c.0]}));
    let bytes = vectorcraft_pdf::export(
        &s.doc().unwrap().doc,
        &vectorcraft_pdf::PdfOptions { created: Some(0), ..vectorcraft_pdf::PdfOptions::uncompressed() },
    )
    .unwrap();
    let pdf = String::from_utf8_lossy(&bytes);
    assert!(pdf.contains("/Separation/Cut"), "the Cut line prints on the Cut plate");
    assert!(pdf.contains("/Separation/Crease"));
    assert!(pdf.contains("/Separation/Partial#20Cut"), "a name with a space");
    assert!(pdf.contains("/OP true"), "the role strokes overprint");
    // Without overprint, nothing in the file overprints.
    let mut plain = session("cmyk");
    let (a, b) = (rect(&mut plain, 20.0), rect(&mut plain, 100.0));
    set_role(&mut plain, json!({"role": "cut", "ids": [a.0], "overprint": false}));
    set_role(&mut plain, json!({"role": "crease", "ids": [b.0], "overprint": false}));
    let bytes = vectorcraft_pdf::export(
        &plain.doc().unwrap().doc,
        &vectorcraft_pdf::PdfOptions { created: Some(0), ..vectorcraft_pdf::PdfOptions::uncompressed() },
    )
    .unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("/OP true"));
    assert!(!export(&mut s, "pdf").is_empty());
}

#[test]
fn svg_dxf_and_hpgl_keep_the_default_role_swatches_apart() {
    let mut s = two_roles();
    let svg = export(&mut s, "svg");
    let strokes: BTreeSet<&str> = svg.split("stroke=\"").skip(1).filter_map(|t| t.split('"').next()).filter(|c| c.starts_with('#')).collect();
    let strokes = if strokes.is_empty() {
        svg.split("stroke:").skip(1).filter_map(|t| t.split([';', '"', '}']).next()).filter(|c| c.starts_with('#')).collect()
    } else {
        strokes
    };
    assert_eq!(strokes.len(), 2, "two stroke colors: {svg}");
    let dxf = export(&mut s, "dxf");
    let lines: Vec<&str> = dxf.lines().map(str::trim).collect();
    // The rectangles' white fills come out as true colors too.
    let colors: BTreeSet<&str> = lines.windows(2).filter(|w| w[0] == "420" && w[1] != "16777215").map(|w| w[1]).collect();
    assert_eq!(colors.len(), 2, "two stroke true colors: {colors:?}");
    let plot = s.execute("document.export", &json!({"format": "hpgl", "hpglVersion": "hpgl2"})).unwrap();
    let plot = String::from_utf8_lossy(&b64(&plot)).into_owned();
    assert!(plot.contains("SP1") && plot.contains("SP2"), "a pen per role swatch color: {plot}");
}

#[test]
fn compound_paths_carry_a_role() {
    let mut s = session("cmyk");
    let (a, b) = (rect(&mut s, 20.0), rect(&mut s, 30.0));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let c = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    assert!(matches!(node(&s, c).kind, NodeKind::Compound { .. }));
    let r = set_role(&mut s, json!({"role": "cut", "ids": [c.0]}));
    assert_eq!(r["ids"], json!([c.0]));
    assert_eq!(counts(&mut s, json!({"ids": [c.0]})), [1, 0, 0, 0]);
    // A member named by id gives its compound the role.
    let r = set_role(&mut s, json!({"role": "crease", "ids": [a.0, b.0]}));
    assert_eq!((r["ids"].clone(), r["changed"].as_u64()), (json!([c.0]), Some(1)));
    assert_eq!(counts(&mut s, json!({"ids": [c.0]})), [0, 1, 0, 0]);
}
