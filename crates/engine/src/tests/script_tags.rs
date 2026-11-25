//! Tags on page items: `PageItem.tags`, `Document.tags`, `Tag`, and the `object.tag.*` commands
//! they run.

use serde_json::{Value, json};
use vectorcraft_doc::{MAX_TAG_TEXT, MAX_TAGS, ObjectTag};

use crate::*;

fn run(s: &mut Session, source: &str) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "t.js"}))
}

/// What the run logged.
fn log(s: &mut Session, source: &str) -> Vec<Value> {
    let r = run(s, source).unwrap_or_else(|e| panic!("{source}: {e}"));
    r["log"].as_array().cloned().unwrap_or_default()
}

/// The message of the run's failure.
fn failure(s: &mut Session, source: &str) -> String {
    run(s, source).map(|r| panic!("{source} ran: {r}")).unwrap_err().to_string()
}

fn rect(s: &mut Session, x: f64, y: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap())
}

/// A document with two rectangles.
fn doc() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let a = rect(&mut s, 10.0, 20.0);
    let b = rect(&mut s, 100.0, 100.0);
    (s, a, b)
}

fn undo_steps(s: &Session) -> usize {
    s.active().unwrap().history.undo.len()
}

fn tags(s: &Session, id: NodeId) -> Vec<(String, String)> {
    let d = &s.active().unwrap().doc;
    crate::cmd::tags::tags_of(d.node(id).unwrap()).iter().map(|t| (t.name.clone(), t.value.clone())).collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(n, v)| (n.to_string(), v.to_string())).collect()
}

#[test]
fn tags_are_added_named_read_and_removed_in_one_undo_step() {
    let (mut s, a, _) = doc();
    let before = undo_steps(&s);
    let out = log(
        &mut s,
        "var item = app.activeDocument.pageItems[0], tags = item.tags;\
         vc.log(tags.typename); vc.log(tags.length);\
         var t = tags.add(); vc.log(t.typename); vc.log(t.name === '' && t.value === '');\
         t.name = 'size'; t.value = 'XL';\
         var u = tags.add(); u.name = 'size'; u.value = 'S';\
         var w = tags.add(); w.name = '../a\\\\b'; w.value = 'kept';\
         vc.log(tags.length); vc.log(tags.getByName('size').value); vc.log(tags[1].value); vc.log(tags.index(2).name);\
         vc.log(tags.index('../a\\\\b').value); vc.log(tags[3] === undefined); vc.log(t.parent.typename); vc.log(tags.parent.typename);\
         u.remove(); vc.log(tags.length); vc.log(tags[1].name);",
    );
    let expected = [json!("Tags"), json!(0), json!("Tag"), json!(true), json!(3), json!("XL"), json!("S"), json!("../a\\b")];
    assert_eq!(out[..8], expected);
    assert_eq!(out[8..], [json!("kept"), json!(true), json!("PathItem"), json!("PathItem"), json!(2), json!("../a\\b")]);
    assert_eq!(tags(&s, a), pairs(&[("size", "XL"), ("../a\\b", "kept")]));
    assert_eq!(undo_steps(&s), before + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(tags(&s, a).is_empty());
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(tags(&s, a).len(), 2);
    log(&mut s, "app.activeDocument.pageItems[0].tags.removeAll();");
    assert!(tags(&s, a).is_empty());
}

#[test]
fn document_tags_list_every_items_tags_and_add_none() {
    let (mut s, a, b) = doc();
    s.execute("object.tag.add", &json!({"id": b.0, "name": "b1", "value": "2"})).unwrap();
    s.execute("object.tag.add", &json!({"id": a.0, "name": "a1", "value": "1"})).unwrap();
    let out = log(
        &mut s,
        "var d = app.activeDocument; vc.log(d.tags.length); vc.log(d.tags[0].name); vc.log(d.tags[1].name);\
         vc.log(d.tags.parent.typename); vc.log(d.tags.getByName('b1').parent.typename);",
    );
    assert_eq!(out, [json!(2), json!("a1"), json!("b1"), json!("Document"), json!("PathItem")]);
    // A copy keeps its source's tags.
    let copy = s.execute("edit.duplicate", &json!({"ids": [a.0]})).unwrap()["ids"][0].as_u64().map(NodeId).unwrap();
    assert_eq!(tags(&s, copy), pairs(&[("a1", "1")]));
    s.execute("edit.undo", &json!({})).unwrap();
    let before = undo_steps(&s);
    assert!(failure(&mut s, "app.activeDocument.tags.add();").contains("added to a page item's tags"));
    assert_eq!(undo_steps(&s), before);
    log(&mut s, "app.activeDocument.tags.removeAll();");
    assert!(tags(&s, a).is_empty() && tags(&s, b).is_empty());
    assert_eq!(undo_steps(&s), before + 1);
}

#[test]
fn bad_tag_values_throw_and_change_nothing() {
    let (mut s, a, _) = doc();
    s.execute("object.tag.add", &json!({"id": a.0, "name": "n", "value": "v"})).unwrap();
    let before = undo_steps(&s);
    let tag = "app.activeDocument.pageItems[0].tags[0]";
    assert!(failure(&mut s, &format!("{tag}.name = 5;")).contains("TypeError: name must be a string"));
    assert!(failure(&mut s, &format!("{tag}.value = null;")).contains("TypeError: value must be a string"));
    let long = format!("{tag}.value = new Array({}).join('x');", MAX_TAG_TEXT + 2);
    assert!(failure(&mut s, &long).contains("RangeError: a tag's value is longer than 65,536 bytes"));
    assert!(failure(&mut s, "app.activeDocument.pageItems[0].tags.getByName('none');").contains("Not found"));
    assert!(failure(&mut s, "app.activeDocument.pageItems[0].tags.index(true);").contains("TypeError"));
    // A run that fails after changing a tag puts it back.
    assert!(failure(&mut s, &format!("{tag}.value = 'w'; throw new Error('stop');")).contains("stop"));
    assert_eq!(tags(&s, a), pairs(&[("n", "v")]));
    // Setting what a tag already has runs no command.
    log(&mut s, &format!("{tag}.name = 'n'; {tag}.value = 'v';"));
    assert_eq!(undo_steps(&s), before);
}

#[test]
fn tags_work_on_an_item_and_with_tag_ids_above_2_to_the_53() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let big = (1u64 << 53) + 1;
    s.active_mut().unwrap().change_unsaved(|d| std::sync::Arc::make_mut(d).raise_next_id(big));
    let a = rect(&mut s, 10.0, 20.0);
    assert_eq!(a.0, big);
    let out = log(
        &mut s,
        "var all = app.activeDocument.pageItems[0].tags, t = all.add(); t.name = 'n'; t.value = 'v';\
         var u = all.add(); u.name = 'm'; vc.log(all.length); vc.log(t.name); vc.log(all[1].name); vc.log(t.parent.typename);\
         t.remove(); vc.log(all.length); vc.log(all[0].name);",
    );
    assert_eq!(out, [json!(2), json!("n"), json!("m"), json!("PathItem"), json!(1), json!("m")]);
    assert_eq!(tags(&s, a), pairs(&[("m", "")]));
}

#[test]
fn an_object_takes_at_most_64_tags() {
    let (mut s, a, _) = doc();
    let out = log(&mut s, "var t = app.activeDocument.pageItems[0].tags; for (var i = 0; i < 64; i++) t.add(); vc.log(t.length);");
    assert_eq!(out, [json!(MAX_TAGS)]);
    let e = failure(&mut s, "app.activeDocument.pageItems[0].tags.add();");
    assert!(e.contains("64 tags"), "{e}");
    assert_eq!(tags(&s, a).len(), MAX_TAGS);
}

#[test]
fn a_tag_goes_stale_when_it_or_its_item_is_deleted() {
    let (mut s, a, b) = doc();
    s.execute("object.tag.add", &json!({"id": a.0, "name": "x"})).unwrap();
    s.execute("object.tag.add", &json!({"id": b.0, "name": "y"})).unwrap();
    let e = failure(&mut s, "var t = app.activeDocument.pageItems[0].tags[0]; t.remove(); t.name;");
    assert!(e.contains("Stale tag"), "{e}");
    // `typename` throws too, on a deleted tag and on the tags of a deleted item (each failed run
    // put the document back).
    let e = failure(&mut s, "var t = app.activeDocument.pageItems[0].tags[0]; t.remove(); t.typename;");
    assert!(e.contains("Stale tag"), "{e}");
    let e = failure(&mut s, "var i = app.activeDocument.pageItems[1], all = i.tags; i.remove(); all.typename;");
    assert!(e.contains("Stale page item"), "{e}");
    let e = failure(&mut s, "var i = app.activeDocument.pageItems[1], t = i.tags[0], all = i.tags; i.remove(); all.length;");
    assert!(e.contains("Stale page item"), "{e}");
    let e = failure(&mut s, "var i = app.activeDocument.pageItems[1], t = i.tags[0]; i.remove(); t.value;");
    assert!(e.contains("Stale page item"), "{e}");
    // A tag added after another was deleted does not take the deleted one's place.
    let e = failure(&mut s, "var all = app.activeDocument.pageItems[0].tags, t = all[0]; t.remove(); all.add(); t.name;");
    assert!(e.contains("Stale tag"), "{e}");
    // Nor after the run undoes the add: the undone document's next id does not give the tag's id
    // out again.
    let e = failure(&mut s, "var all = app.activeDocument.pageItems[0].tags, t = all.add(); vc.run('edit.undo'); all.add().name = 'u'; t.name;");
    assert!(e.contains("Stale tag"), "{e}");
    // Nor when the deleted tag's id was ahead of the document's next id (a tag pasted from
    // another document).
    let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let ahead = d.peek_next_id() + 2;
    d.node_mut(a).unwrap().edit_attrs(|at| at.tags = vec![ObjectTag { id: ahead, name: "p".into(), value: String::new() }]);
    let e = failure(&mut s, "var all = app.activeDocument.pageItems[0].tags, t = all[0]; t.remove(); all.add(); all.add(); all.add(); t.name;");
    assert!(e.contains("Stale tag"), "{e}");
}

#[test]
fn tag_commands_check_their_params_and_skip_unchanged_edits() {
    let (mut s, a, _) = doc();
    let tag = s.execute("object.tag.add", &json!({"id": a.0, "name": "k"})).unwrap()["tag"].as_u64().unwrap();
    let before = undo_steps(&s);
    assert_eq!(s.execute("object.tag.set", &json!({"id": a.0, "tag": tag, "name": "k"})).unwrap()["changed"], false);
    assert_eq!(s.execute("object.tag.removeAll", &json!({"ids": []})).unwrap()["changed"], 0);
    assert_eq!(undo_steps(&s), before);
    let too_long = "x".repeat(MAX_TAG_TEXT + 1);
    for (id, p) in [
        ("object.tag.add", json!({"id": a.0, "name": too_long})),
        ("object.tag.add", json!({"id": a.0, "value": 3})),
        ("object.tag.add", json!({"id": a.0, "name": null})),
        ("object.tag.set", json!({"id": a.0, "tag": tag, "name": null, "value": "updated"})),
        ("object.tag.set", json!({"id": a.0, "tag": tag, "value": null})),
        ("object.tag.add", json!({"id": 999_999})),
        ("object.tag.add", json!({"id": "1"})),
        ("object.tag.set", json!({"id": a.0, "tag": tag})),
        ("object.tag.set", json!({"id": a.0, "tag": tag + 1, "name": "n"})),
        ("object.tag.remove", json!({"id": a.0, "tag": -1})),
        ("object.tag.removeAll", json!({"ids": [999_999]})),
        ("object.tags", json!({})),
    ] {
        assert!(s.execute(id, &p).is_err(), "{id} {p}");
    }
    let layer = s.active().unwrap().doc.layers[0].id;
    assert!(s.execute("object.tag.add", &json!({"id": layer.0})).is_err());
    assert_eq!(undo_steps(&s), before);
    assert_eq!(s.execute("object.tags", &json!({"id": a.0})).unwrap(), json!({"tags": [{"tag": tag, "name": "k", "value": ""}]}));
}

#[test]
fn tags_survive_native_save_and_open_and_hostile_ones_are_trimmed() {
    let (mut s, a, b) = doc();
    let tag = s.execute("object.tag.add", &json!({"id": a.0, "name": "size", "value": "XL"})).unwrap()["tag"].as_u64().unwrap();
    let mut doc = (*s.active().unwrap().doc).clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    let t = crate::cmd::tags::tags_of(back.node(a).unwrap());
    assert_eq!(t.iter().map(|t| (t.name.as_str(), t.value.as_str())).collect::<Vec<_>>(), [("size", "XL")]);
    // The tag keeps its id, so `object.tag.set` and `object.tag.remove` still find it.
    assert_eq!(t[0].id, tag);
    assert!(tag < back.peek_next_id());
    // A file with too many tags, repeated ids and over-long text opens with them within the limits.
    doc.node_mut(b).unwrap().edit_attrs(|at| {
        at.tags = (0..MAX_TAGS + 10).map(|i| ObjectTag { id: 7, name: format!("t{i}"), value: String::new() }).collect();
        at.tags.insert(0, ObjectTag { id: 1, name: "x".repeat(MAX_TAG_TEXT + 1), value: String::new() });
    });
    // The repeated ids get new ids from the next node id on.
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    let t = crate::cmd::tags::tags_of(back.node(b).unwrap());
    assert_eq!(t[0].id, 7);
    assert!(t[1..].iter().all(|t| t.id > tag && t.id < back.peek_next_id()));
    // An id at the top of the node id range, and one past it: the new ids are the smallest the
    // object's other tags do not have.
    let top = (1u64 << 62) - 1;
    doc.node_mut(a).unwrap().edit_attrs(|at| {
        at.tags[0].id = u64::MAX;
        at.tags.insert(0, ObjectTag { id: top, name: "top".into(), value: String::new() });
    });
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    // Opening gives both new ids below the next node id, which stays within the node id range.
    let fixed: Vec<u64> = crate::cmd::tags::tags_of(back.node(a).unwrap()).iter().map(|t| t.id).collect();
    assert_eq!(fixed.len(), 2);
    assert!(fixed.iter().all(|id| *id < back.peek_next_id()) && fixed[0] != fixed[1], "{fixed:?}");
    assert_eq!(fixed, [top, 1]);
    assert!(back.peek_next_id() <= 1 << 62, "{}", back.peek_next_id());
    // No tag id is left for a new tag: one past the range would not be kept on opening, and one
    // below it could be a deleted tag's.
    assert_eq!(back.clone().alloc_tag_id(a), None);
    let t = crate::cmd::tags::tags_of(back.node(b).unwrap());
    assert_eq!(t.len(), MAX_TAGS);
    assert_eq!(t[0].name, "t0");
    // The first tag with the repeated id keeps it; the others get new ones.
    assert_eq!(t[0].id, 7);
    let mut ids: Vec<u64> = t.iter().map(|t| t.id).collect();
    assert!(ids.iter().all(|id| *id < back.peek_next_id()));
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), MAX_TAGS);
}

#[test]
fn a_file_with_a_tag_id_that_is_not_a_whole_number_opens_and_the_tag_gets_a_new_id() {
    let (s, a, _) = doc();
    let mut doc = (*s.active().unwrap().doc).clone();
    doc.node_mut(a).unwrap().edit_attrs(|at| {
        at.tags =
            ["k0", "k1", "k2"].iter().enumerate().map(|(i, n)| ObjectTag { id: 424_240 + i as u64, name: (*n).into(), value: "v".into() }).collect();
    });
    let mut hostile = String::from_utf8(vectorcraft_format::save(&doc, false)).unwrap();
    for (from, to) in [("\"id\":424240", "\"id\":-1"), ("\"id\":424241", "\"id\":1.5"), ("\"id\":424242", "\"id\":\"x\"")] {
        assert!(hostile.contains(from), "{from}");
        hostile = hostile.replace(from, to);
    }
    let back = vectorcraft_format::load(hostile.as_bytes()).unwrap();
    let t = crate::cmd::tags::tags_of(back.node(a).unwrap());
    assert_eq!(t.iter().map(|t| (t.name.as_str(), t.value.as_str())).collect::<Vec<_>>(), [("k0", "v"), ("k1", "v"), ("k2", "v")]);
    let mut ids: Vec<u64> = t.iter().map(|t| t.id).collect();
    assert!(ids.iter().all(|id| *id >= 1 && *id < back.peek_next_id()), "{ids:?}");
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3);
}
