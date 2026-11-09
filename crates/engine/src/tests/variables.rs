//! Variables (data merge): definitions with kinds, bindings to objects, datasets
//! applied in one undo step, and validation that rejects instead of reinterpreting.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use crate::tests_links::{BLUE, Folder, RED, bounds, image, place, png, write};
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn text(s: &mut Session, content: &str) -> NodeId {
    let r = s.execute("text.create", &json!({"x": 10, "y": 10, "text": content})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn rect(s: &mut Session) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

/// A group of the objects `ids` → the group's id.
fn group(s: &mut Session, ids: &[NodeId]) -> NodeId {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    let r = s.execute("object.group", &json!({})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn plain(s: &Session, id: NodeId) -> String {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    }
}

#[test]
fn text_and_visibility_datasets_apply_in_one_undo_step() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "B", "values": {"Name": "Bob", "Show": false}})).unwrap();

    let v = s.execute("dataset.select", &json!({"name": "B"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(2), Some(0)));
    assert_eq!(plain(&s, t), "Bob");
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible);

    // One undo step covers the whole dataset application.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "Alice");
    assert!(s.doc().unwrap().doc.node(r).unwrap().visible);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "Bob");
}

#[test]
fn dataset_next_and_prev_wrap_around() {
    let mut s = session();
    let t = text(&mut s, "x");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    for name in ["A", "B"] {
        s.execute("dataset.new", &json!({"name": name, "values": {"Name": name}})).unwrap();
    }
    s.execute("dataset.next", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "A");
    s.execute("dataset.next", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "B");
    s.execute("dataset.next", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "A");
    s.execute("dataset.prev", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "B");
    let v = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(v["active"], "B");
}

#[test]
fn applying_a_dataset_reports_the_area_type_it_overflows() {
    let mut s = session();
    let area = |s: &mut Session, y: u32, content: &str| {
        let r = s.execute("text.create", &json!({"x": 10, "y": y, "text": content, "size": 12, "area": {"width": 120, "height": 40}})).unwrap();
        NodeId(r["id"].as_u64().unwrap())
    };
    let long = "word ".repeat(60);
    let a = area(&mut s, 10, "x");
    let p = text(&mut s, "x");
    // Area type that already overflows, bound to no variable, isn't named.
    area(&mut s, 200, &long);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [a.0, p.0]})).unwrap();
    // `variable.bind` refuses a second text variable on one type object; a document saved
    // before it did can hold two.
    Arc::make_mut(&mut s.doc_mut().unwrap().doc).variables.bind(a, "Title");
    s.execute("dataset.new", &json!({"name": "Short", "values": {"Name": "Ann", "Title": "Ann"}})).unwrap();
    s.execute("dataset.new", &json!({"name": "Long", "values": {"Name": long, "Title": long}})).unwrap();

    // The area type is named once though two variables rewrote it; the point type is never named.
    let v = s.execute("dataset.select", &json!({"name": "Long"})).unwrap();
    assert_eq!((v["applied"].as_u64(), &v["overflow"]), (Some(3), &json!([a.0])), "{v}");
    let v = s.execute("dataset.select", &json!({"name": "Short"})).unwrap();
    assert!(v.get("overflow").is_none(), "{v}");
    let v = s.execute("dataset.next", &json!({})).unwrap();
    assert_eq!((&v["name"], &v["overflow"]), (&json!("Long"), &json!([a.0])), "{v}");
    let v = s.execute("dataset.prev", &json!({})).unwrap();
    assert!(v.get("overflow").is_none(), "{v}");
    let v = s.execute("dataset.prev", &json!({})).unwrap();
    assert_eq!((&v["name"], &v["overflow"]), (&json!("Long"), &json!([a.0])), "{v}");
}

/// Area type threaded into a rectangle → (session, first frame, second frame, the story).
fn threaded(second_height: u32) -> (Session, NodeId, NodeId, String) {
    let mut s = session();
    let story = "The quick brown fox jumps over the lazy dog and keeps running. ".repeat(3) + "\nSecond paragraph.\nThird.";
    let r = s.execute("text.create", &json!({"x": 10, "y": 10, "text": story, "area": {"width": 150, "height": 60}})).unwrap();
    let a = NodeId(r["id"].as_u64().unwrap());
    let r = s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": second_height})).unwrap();
    let b = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    (s, a, b, story)
}

/// Whether `dataset.list` says the data set `name` matches the art.
fn matches(s: &mut Session, name: &str) -> bool {
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    let set = sets["datasets"].as_array().unwrap().iter().find(|d| d["name"] == name).unwrap().clone();
    set["matches"].as_bool().unwrap()
}

#[test]
fn a_text_value_on_a_frame_of_threaded_type_replaces_the_whole_story() {
    let (mut s, a, b, story) = threaded(200);
    assert!(!plain(&s, a).is_empty() && !plain(&s, b).is_empty());
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [a.0]})).unwrap();
    // A capture reads the whole story, every frame's text in thread order.
    s.execute("dataset.capture", &json!({"name": "Orig"})).unwrap();
    let orig = s.doc().unwrap().doc.variables.dataset("Orig").unwrap().values.get("Name").cloned();
    assert_eq!(orig, Some(vectorcraft_doc::DataValue::Text(story.clone())));
    s.execute("dataset.new", &json!({"name": "B", "values": {"Name": "Bob"}})).unwrap();

    let v = s.execute("dataset.select", &json!({"name": "B"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(0)));
    assert_eq!((plain(&s, a), plain(&s, b)), ("Bob".to_string(), String::new()));
    assert!(matches(&mut s, "B"));
    // The captured row applied back gives the story back, every time.
    for _ in 0..2 {
        s.execute("dataset.select", &json!({"name": "Orig"})).unwrap();
        assert_eq!(format!("{}{}", plain(&s, a), plain(&s, b)), story);
        assert!(!plain(&s, b).is_empty());
        assert!(matches(&mut s, "Orig"));
        s.execute("dataset.select", &json!({"name": "B"})).unwrap();
        assert_eq!(format!("{}{}", plain(&s, a), plain(&s, b)), "Bob");
    }
}

#[test]
fn a_text_value_on_a_later_frame_of_threaded_type_replaces_the_whole_story() {
    let (mut s, a, b, story) = threaded(200);
    s.execute("variable.define", &json!({"name": "Tail", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Tail", "ids": [b.0]})).unwrap();
    s.execute("dataset.capture", &json!({"name": "Orig"})).unwrap();
    s.execute("dataset.new", &json!({"name": "T", "values": {"Tail": "END"}})).unwrap();

    let v = s.execute("dataset.select", &json!({"name": "T"})).unwrap();
    assert_eq!(v["applied"].as_u64(), Some(1));
    assert_eq!((plain(&s, a), plain(&s, b)), ("END".to_string(), String::new()));
    assert!(matches(&mut s, "T"));
    s.execute("dataset.select", &json!({"name": "Orig"})).unwrap();
    assert_eq!(format!("{}{}", plain(&s, a), plain(&s, b)), story);
    assert!(matches(&mut s, "Orig"));
}

#[test]
fn a_story_with_a_locked_frame_is_neither_read_nor_rewritten() {
    let (mut s, a, b, story) = threaded(200);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [a.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "B", "values": {"Name": "Bob"}})).unwrap();
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();

    let v = s.execute("dataset.select", &json!({"name": "B"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(0), Some(1)));
    assert_eq!(format!("{}{}", plain(&s, a), plain(&s, b)), story);
    assert!(s.doc().unwrap().doc.captured_values().is_empty());
}

#[test]
fn two_text_variables_in_one_story_apply_in_definition_order() {
    let (mut s, a, b, _) = threaded(200);
    s.execute("variable.define", &json!({"name": "Head", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Tail", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Tail", "ids": [a.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Head", "ids": [b.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "R", "values": {"Head": "one", "Tail": "two"}})).unwrap();

    let v = s.execute("dataset.select", &json!({"name": "R"})).unwrap();
    assert_eq!(v["applied"].as_u64(), Some(2));
    assert_eq!(format!("{}{}", plain(&s, a), plain(&s, b)), "two");
}

#[test]
fn applying_a_dataset_to_threaded_type_reports_overflow_on_the_last_frame() {
    let (mut s, a, b, _) = threaded(40);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [a.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "Long", "values": {"Name": "word ".repeat(120)}})).unwrap();
    s.execute("dataset.new", &json!({"name": "Short", "values": {"Name": "Ann"}})).unwrap();

    let v = s.execute("dataset.select", &json!({"name": "Long"})).unwrap();
    assert_eq!(&v["overflow"], &json!([b.0]), "{v}");
    let v = s.execute("dataset.select", &json!({"name": "Short"})).unwrap();
    assert!(v.get("overflow").is_none(), "{v}");
}

#[test]
fn mismatched_and_missing_bindings_are_skipped_and_counted() {
    let mut s = session();
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    // A text variable bound to a rectangle applies to nothing; a dataset value for an
    // unbound variable is ignored.
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "y", "Ghost": "z"}})).unwrap_err();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "y"}})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [r.0]})).unwrap_err();
    let v = s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(0)));
    assert_eq!(plain(&s, t), "y");
}

/// A row with no value for any bound variable, such as one from Import Data Sets… before anything
/// is bound, applies nothing, and the reply's `warning` names it. A bound variable with a value in
/// the row gives no warning, even when its object is locked and skipped.
#[test]
fn a_row_with_no_value_for_a_bound_variable_warns_that_nothing_was_applied() {
    let mut s = session();
    let t = text(&mut s, "x");
    s.execute("dataset.import", &json!({"csv": "Name,Title\nAnn,Boss\nBob,Chief\n"})).unwrap();
    let (a, b) = ("Data Set 1", "Data Set 2");
    let warning = |row: &str| format!("nothing in data set {row} was applied: no object is bound to a variable it has a value for");
    let v = s.execute("dataset.select", &json!({"name": a})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64(), &v["warning"]), (Some(0), Some(0), &json!(warning(a))), "{v}");
    let v = s.execute("dataset.next", &json!({})).unwrap();
    assert_eq!(v["warning"], json!(warning(b)), "{v}");
    let v = s.execute("dataset.prev", &json!({})).unwrap();
    assert_eq!(v["warning"], json!(warning(a)), "{v}");
    assert_eq!(plain(&s, t), "x");

    // A bound variable that the row has no value for still applies nothing.
    s.execute("variable.define", &json!({"name": "Other", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Other", "ids": [t.0]})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": b})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64(), &v["warning"]), (Some(0), Some(1), &json!(warning(b))), "{v}");

    // Bound to a variable with a value, the row applies it and gives no warning.
    s.execute("variable.unbind", &json!({"variable": "Other", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Title", "ids": [t.0]})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": a})).unwrap();
    assert_eq!(v["applied"].as_u64(), Some(1), "{v}");
    assert!(v.get("warning").is_none(), "{v}");
    assert_eq!(plain(&s, t), "Boss");

    // Locked art is skipped, and its binding still counts as bound.
    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": b})).unwrap();
    assert_eq!(v["applied"].as_u64(), Some(0), "{v}");
    assert!(v.get("warning").is_none(), "{v}");
}

#[test]
fn deleting_a_variable_cleans_bindings_values_and_leaves_datasets_alone() {
    let mut s = session();
    let t = text(&mut s, "x");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "y"}})).unwrap();
    s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    s.execute("variable.delete", &json!({"names": ["Name"]})).unwrap();
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"].as_array().map(Vec::len), Some(0));
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert!(sets["datasets"][0].get("values").is_none_or(|v| v == &json!({})));
    // The dataset named "D" is still the active one: `variable.delete` prunes variables, and
    // the dataset's own name is its own namespace.
    assert_eq!(sets["active"], "D");
    s.execute("dataset.delete", &json!({"names": ["D"]})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"].as_array().map(Vec::len), Some(0));
    assert!(sets["active"].is_null());
    let v: Value = serde_json::from_str(&serde_json::to_string(&s.doc().unwrap().doc).unwrap()).unwrap();
    assert!(v["variables"].get("bindings").is_none_or(|v| v == &json!({})));
}

/// A dataset may share a variable's name: they are separate namespaces, and deleting the
/// variable must not silently unselect the dataset that happens to be called the same.
#[test]
fn a_dataset_survives_a_variable_of_the_same_name_being_deleted() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Row", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "Row", "values": {"Row": "y"}})).unwrap();
    s.execute("dataset.select", &json!({"name": "Row"})).unwrap();
    s.execute("variable.delete", &json!({"names": ["Row"]})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][0]["name"], "Row", "the dataset is not a variable");
    assert_eq!(sets["active"], "Row", "the active dataset keeps its name");
}

/// Deleting art lets its bindings go, as it does for Asset Export's assets: a binding that
/// outlived its object would count as a skip in every dataset application and grow the file.
#[test]
fn deleted_art_lets_its_bindings_go() {
    let mut s = session();
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "y", "Show": false}})).unwrap();

    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    let vars = s.execute("variable.list", &json!({})).unwrap();
    let of = |name: &str| vars["variables"].as_array().map(|a| a.iter().find(|v| v["name"] == name).unwrap()["bindings"].as_u64().unwrap()).unwrap();
    assert_eq!(of("Name"), 0, "the deleted text object took its binding with it");
    assert_eq!(of("Show"), 1, "the rectangle is still bound");
    // Nothing stale is left for the application to skip.
    let v = s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(0)));
}

/// `deleted` counts what was deleted, not what was asked for.
#[test]
fn delete_reports_what_it_actually_removed() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "A", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "B", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "D"})).unwrap();
    assert_eq!(s.execute("variable.delete", &json!({"names": ["A"]})).unwrap()["deleted"], 1);
    assert_eq!(s.execute("variable.delete", &json!({"names": ["A"]})).unwrap()["deleted"], 0, "already gone");
    assert_eq!(s.execute("variable.delete", &json!({"names": ["A", "B"]})).unwrap()["deleted"], 1, "only B was left");
    assert_eq!(s.execute("dataset.delete", &json!({"names": ["D"]})).unwrap()["deleted"], 1);
    assert_eq!(s.execute("dataset.delete", &json!({"names": ["D"]})).unwrap()["deleted"], 0);
}

/// An object may be driven by several variables, so binding it again adds to what it has
/// instead of quietly dropping the earlier one.
#[test]
fn one_object_can_be_driven_by_several_variables() {
    let mut s = session();
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Shown", "kind": "visibility"})).unwrap();
    s.execute("variable.define", &json!({"name": "Badge", "kind": "visibility"})).unwrap();

    // A text object driven by a text variable and a visibility variable keeps both.
    assert_eq!(s.execute("variable.bind", &json!({"variable": "Title", "ids": [t.0]})).unwrap()["bound"], 1);
    let v = s.execute("variable.bind", &json!({"variable": "Shown", "ids": [t.0]})).unwrap();
    assert_eq!((v["bound"].clone(), v["already"].clone()), (json!(1), json!([])), "Title is untouched");
    // A rectangle driven by a visibility variable as well as nothing else yet.
    s.execute("variable.bind", &json!({"variable": "Badge", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Title": "Hat", "Shown": true, "Badge": false}})).unwrap();

    let v = s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(3), Some(0)), "every binding applied");
    assert_eq!(plain(&s, t), "Hat");
    assert!(s.doc().unwrap().doc.node(t).unwrap().visible);
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible);

    let vars = s.execute("variable.list", &json!({})).unwrap();
    let of = |n: &str| vars["variables"].as_array().unwrap().iter().find(|v| v["name"] == n).unwrap()["bindings"].as_u64();
    assert_eq!((of("Title"), of("Shown"), of("Badge")), (Some(1), Some(1), Some(1)));

    // Binding again to one it already has is not a change, and says so.
    let v = s.execute("variable.bind", &json!({"variable": "Shown", "ids": [t.0]})).unwrap();
    assert_eq!((v["bound"].clone(), v["already"].clone()), (json!(0), json!([t.0])));
    // Unbinding one variable leaves the object's others alone.
    assert_eq!(s.execute("variable.unbind", &json!({"ids": [t.0], "variable": "Shown"})).unwrap()["unbound"], 1);
    s.execute("edit.undo", &json!({})).unwrap();
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"][1]["bindings"], 1, "the other binding is still there");
    assert!(s.execute("variable.unbind", &json!({"ids": [t.0], "variable": "Ghost"})).is_err());

    // Deleting a variable drops only its own binding.
    s.execute("variable.delete", &json!({"names": ["Shown"]})).unwrap();
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"][0]["bindings"], 1, "Title still drives the text");
}

/// A type object holds one text variable: binding another text variable to it is refused, the
/// error names the one it has, and nothing is bound, not even the other objects of the call.
/// Variables of other kinds still share the object.
#[test]
fn a_type_object_holds_one_text_variable() {
    let mut s = session();
    let t = text(&mut s, "x");
    let u = text(&mut s, "y");
    for (name, kind) in [("Title", "text"), ("Price", "text"), ("Shown", "visibility")] {
        s.execute("variable.define", &json!({"name": name, "kind": kind})).unwrap();
    }
    s.execute("variable.bind", &json!({"variable": "Title", "ids": [t.0]})).unwrap();
    let bindings = |s: &Session, name: &str| s.doc().unwrap().doc.variables.objects_of(name);

    let e = s.execute("variable.bind", &json!({"variable": "Price", "ids": [u.0, t.0]})).unwrap_err().to_string();
    assert!(e.contains("`variable.bind`") && e.contains(&format!("node {} already has the text variable `Title`", t.0)), "{e}");
    assert_eq!((bindings(&s, "Title"), bindings(&s, "Price")), (vec![t], vec![]), "nothing was bound");
    // The same variable again is not a second one, and another type object takes Price.
    assert_eq!(s.execute("variable.bind", &json!({"variable": "Title", "ids": [t.0]})).unwrap()["already"], json!([t.0]));
    s.execute("variable.bind", &json!({"variable": "Price", "ids": [u.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Shown", "ids": [t.0, u.0]})).unwrap();
    assert_eq!(bindings(&s, "Shown"), vec![t, u]);
    // After Title is unbound from it, the object takes Price.
    s.execute("variable.unbind", &json!({"ids": [t.0], "variable": "Title"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Price", "ids": [t.0]})).unwrap();
    assert_eq!(bindings(&s, "Price"), vec![t, u]);
}

/// Make Text Dynamic refuses type that already has a text variable and defines no variable;
/// Make Visibility Dynamic still binds it.
#[test]
fn make_text_dynamic_refuses_type_that_has_a_text_variable() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    s.execute("variable.makeTextDynamic", &json!({})).unwrap();

    let e = s.execute("variable.makeTextDynamic", &json!({})).unwrap_err().to_string();
    assert!(e.contains("`variable.makeTextDynamic`") && e.contains(&format!("node {} already has the text variable `Alice`", t.0)), "{e}");
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"].as_array().map(Vec::len), Some(1), "{vars}");
    let v = s.execute("variable.makeVisibilityDynamic", &json!({})).unwrap();
    assert_eq!(v["ids"], json!([t.0]));
}

/// With unbound type selected before type that has a text variable, Make Text Dynamic is still
/// refused: it defines no variable and binds neither object.
#[test]
fn make_text_dynamic_refuses_a_selection_where_a_later_object_has_a_text_variable() {
    let mut s = session();
    let u = text(&mut s, "Bob");
    let t = text(&mut s, "Alice");
    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    s.execute("variable.makeTextDynamic", &json!({})).unwrap();

    s.execute("select.set", &json!({"ids": [u.0, t.0]})).unwrap();
    let e = s.execute("variable.makeTextDynamic", &json!({})).unwrap_err().to_string();
    assert!(e.contains(&format!("node {} already has the text variable `Alice`", t.0)), "{e}");
    let doc = &s.doc().unwrap().doc;
    assert_eq!(doc.variables.variables.len(), 1, "no variable was defined");
    assert!(!doc.variables.bindings.contains_key(&u), "the unbound object stays unbound");
    assert_eq!(doc.variables.bindings[&t].iter().collect::<Vec<_>>(), ["Alice"]);
}

/// A document saved with two text variables on one type object opens with both bindings, and
/// applying a row still writes both, the later one defined last. Binding either again changes
/// nothing, and binding a third is refused with the first one named.
#[test]
fn a_document_with_two_text_variables_on_one_type_object_opens_unchanged() {
    let mut s = session();
    let t = text(&mut s, "x");
    let u = text(&mut s, "y");
    for name in ["Title", "Price", "Note"] {
        s.execute("variable.define", &json!({"name": name, "kind": "text"})).unwrap();
    }
    s.execute("variable.bind", &json!({"variable": "Title", "ids": [t.0]})).unwrap();
    Arc::make_mut(&mut s.doc_mut().unwrap().doc).variables.bind(t, "Price");
    s.execute("dataset.new", &json!({"name": "D", "values": {"Title": "Hat", "Price": "9.99"}})).unwrap();
    let saved = s.execute("document.export", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"].clone();

    let mut o = Session::new();
    o.execute("document.open", &json!({"name": "two.vectorcraft", "dataBase64": saved})).unwrap();
    let held: Vec<String> = o.doc().unwrap().doc.variables.bindings[&t].iter().cloned().collect();
    assert_eq!(held, ["Price", "Title"]);
    let v = o.execute("dataset.select", &json!({"name": "D"})).unwrap();
    assert_eq!(v["applied"], json!(2), "{v}");
    assert_eq!(plain(&o, t), "9.99");
    for name in ["Title", "Price"] {
        let v = o.execute("variable.bind", &json!({"variable": name, "ids": [t.0, u.0]})).unwrap();
        assert_eq!((&v["bound"], &v["already"]), (&json!(1), &json!([t.0])), "{name}: {v}");
        o.execute("edit.undo", &json!({})).unwrap();
    }
    let e = o.execute("variable.bind", &json!({"variable": "Note", "ids": [t.0]})).unwrap_err().to_string();
    assert!(e.contains(&format!("node {} already has the text variable `Title`", t.0)), "{e}");
}

/// `unbound` counts what was actually dropped, like `variable.delete` does.
#[test]
fn unbind_reports_what_it_actually_removed() {
    let mut s = session();
    let t = text(&mut s, "x");
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Price", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Title", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Price", "ids": [t.0]})).unwrap();
    assert_eq!(s.execute("variable.unbind", &json!({"ids": [t.0], "variable": "Title"})).unwrap()["unbound"], 1);
    assert_eq!(s.execute("variable.unbind", &json!({"ids": [t.0], "variable": "Title"})).unwrap()["unbound"], 0, "already gone");
    // Without `variable` the object's last binding goes and the entry with it.
    assert_eq!(s.execute("variable.unbind", &json!({"ids": [t.0]})).unwrap()["unbound"], 1);
    assert_eq!(s.execute("variable.unbind", &json!({"ids": [t.0]})).unwrap()["unbound"], 0);
    // An object with no binding left is not kept as an empty set.
    let v: Value = serde_json::from_str(&serde_json::to_string(&s.doc().unwrap().doc).unwrap()).unwrap();
    assert!(v["variables"].get("bindings").is_none_or(|v| v == &json!({})), "{v}");
}

/// Locked type takes no new text variable binding, and a row leaves its text alone.
#[test]
fn locked_art_is_neither_bound_nor_rewritten() {
    let mut s = session();
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "y", "Show": false}})).unwrap();

    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    assert!(s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).is_err(), "locked art takes no new binding");
    // The bindings made before the lock still count, and the application skips the locked art.
    let v = s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(1)));
    assert_eq!(plain(&s, t), "x", "the locked text was left alone");
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible, "the unlocked rectangle still applied");
}

/// The art on a locked artboard acts as locked art: a data set skips it, Update Data Set reads
/// nothing from it, and it takes no new binding. Unlocking the artboard lets the data set rewrite it.
#[test]
fn art_on_a_locked_artboard_is_neither_bound_nor_rewritten() {
    let mut s = session();
    let t = NodeId(s.execute("text.create", &json!({"x": 2050, "y": 50, "text": "cat"})).unwrap()["id"].as_u64().unwrap());
    let fresh = NodeId(s.execute("text.create", &json!({"x": 2100, "y": 100, "text": "new"})).unwrap()["id"].as_u64().unwrap());
    let r = rect(&mut s);
    s.execute("artboard.new", &json!({"x": 2000, "y": 0, "width": 400, "height": 400})).unwrap();
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.define", &json!({"name": "Box", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Box", "ids": [r.0]})).unwrap();
    s.execute("dataset.capture", &json!({"name": "A"})).unwrap();
    s.execute("dataset.new", &json!({"name": "B", "values": {"Name": "Bob", "Show": false, "Box": false}})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();

    let e = s.execute("variable.bind", &json!({"variable": "Name", "ids": [fresh.0]})).unwrap_err().to_string();
    assert!(e.contains(&format!("node {} is on a locked artboard", fresh.0)), "{e}");
    let v = s.execute("dataset.select", &json!({"name": "B"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(2)), "{v}");
    assert_eq!(plain(&s, t), "cat", "the text on the locked artboard was left alone");
    assert!(s.doc().unwrap().doc.node(t).unwrap().visible, "and still shows");
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible, "the rectangle off it applied");
    // Update Data Set keeps the row's values for the art it can't read.
    let v = s.execute("dataset.update", &json!({})).unwrap();
    assert_eq!(v["values"], 1, "{v}");
    let row = s.doc().unwrap().doc.variables.dataset("B").unwrap().values.clone();
    assert_eq!(row.get("Name"), Some(&vectorcraft_doc::DataValue::Text("Bob".into())));
    assert_eq!(row.get("Show"), Some(&vectorcraft_doc::DataValue::Visible(false)));

    s.execute("artboard.setProps", &json!({"index": 1, "locked": false})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": "B"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(3), Some(0)), "{v}");
    assert_eq!(plain(&s, t), "Bob");
}

/// A locked artboard elsewhere in the document does not stop threaded type on an unlocked artboard
/// from having its whole story rewritten, and type on the locked artboard is still left alone.
#[test]
fn threaded_type_on_an_unlocked_artboard_is_rewritten_beside_a_locked_artboard() {
    let (mut s, a, b, story) = threaded(200);
    let far = NodeId(s.execute("text.create", &json!({"x": 2050, "y": 50, "text": "cat"})).unwrap()["id"].as_u64().unwrap());
    s.execute("artboard.new", &json!({"x": 2000, "y": 0, "width": 400, "height": 400})).unwrap();
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Tail", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [a.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Tail", "ids": [far.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "B", "values": {"Name": "Bob", "Tail": "dog"}})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.execute("artboard.setProps", &json!({"index": 1, "locked": true})).unwrap();

    let v = s.execute("dataset.select", &json!({"name": "B"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(1)), "{v}");
    assert_eq!((plain(&s, a), plain(&s, b)), ("Bob".to_string(), String::new()), "the whole story was rewritten");
    assert_eq!(plain(&s, far), "cat", "the text on the locked artboard was left alone");
    assert!(!story.is_empty());
}

/// Binding fails for locked art and for art in a hidden or locked group or layer, and the
/// error names which of the three applies.
#[test]
fn binding_errors_say_whether_a_layer_or_group_is_hidden_or_locked() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    let layer = NodeId(s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    let bind = |s: &mut Session| s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap_err().to_string();

    s.execute("layer.setProps", &json!({"id": layer.0, "visible": false})).unwrap();
    let e = bind(&mut s);
    assert!(e.contains(&format!("node {} is in a hidden group or layer", t.0)), "{e}");
    s.execute("layer.setProps", &json!({"id": layer.0, "visible": true})).unwrap();

    s.execute("select.set", &json!({"ids": [t.0, r.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();
    let e = bind(&mut s);
    assert!(e.contains(&format!("node {} is in a hidden group or layer", t.0)), "{e}");
    s.execute("object.showAll", &json!({})).unwrap();

    s.execute("layer.setProps", &json!({"id": layer.0, "locked": true})).unwrap();
    let e = bind(&mut s);
    assert!(e.contains(&format!("node {} is in a locked group or layer", t.0)), "{e}");
    s.execute("layer.setProps", &json!({"id": layer.0, "locked": false})).unwrap();

    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let e = bind(&mut s);
    assert!(e.contains(&format!("node {} is locked", t.0)), "{e}");
}

/// A visibility variable binds a locked layer and a locked object inside a locked group, and a row
/// hides and shows them in one undo step without unlocking them. List, Capture and Update Data Set
/// read whether they show.
#[test]
fn a_visibility_variable_shows_and_hides_locked_art() {
    let mut s = session();
    let badge = rect(&mut s);
    let other = rect(&mut s);
    let g = group(&mut s, &[badge, other]);
    let layer = NodeId(s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
    rect(&mut s);
    s.execute("object.setProps", &json!({"ids": [badge.0], "locked": true})).unwrap();
    s.execute("object.setProps", &json!({"ids": [g.0], "locked": true})).unwrap();
    s.execute("layer.setProps", &json!({"id": layer.0, "locked": true})).unwrap();
    s.execute("variable.define", &json!({"name": "Layer", "kind": "visibility"})).unwrap();
    s.execute("variable.define", &json!({"name": "Badge", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Layer", "ids": [layer.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Badge", "ids": [badge.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "Shown", "values": {"Layer": true, "Badge": true}})).unwrap();
    s.execute("dataset.new", &json!({"name": "Hidden", "values": {"Layer": false, "Badge": false}})).unwrap();
    let shows = |s: &Session, id: NodeId| s.doc().unwrap().doc.node(id).unwrap().visible;
    let locked = |s: &Session| [layer, g, badge].iter().all(|id| s.doc().unwrap().doc.node(*id).unwrap().locked);

    let v = s.execute("dataset.select", &json!({"name": "Hidden"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(2), Some(0)), "{v}");
    assert!(!shows(&s, layer) && !shows(&s, badge), "the row hides the locked layer and the locked object");
    assert!(locked(&s), "the row leaves the locks as they were");
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    let matching: Vec<&Value> = sets["datasets"].as_array().unwrap().iter().filter(|d| d["matches"] == json!(true)).map(|d| &d["name"]).collect();
    assert_eq!(matching, [&json!("Hidden")], "List reads the locked art: {sets}");
    assert_eq!(s.execute("dataset.capture", &json!({"name": "Captured"})).unwrap()["values"], 2);
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][2]["values"], json!({"Badge": {"visible": false}, "Layer": {"visible": false}}), "{sets}");

    // Undo puts back what the row hid.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(shows(&s, layer) && shows(&s, badge), "undo shows them again");

    let v = s.execute("dataset.select", &json!({"name": "Hidden"})).unwrap();
    assert_eq!(v["applied"], 2, "{v}");
    let v = s.execute("dataset.select", &json!({"name": "Shown"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(2), Some(0)), "{v}");
    assert!(shows(&s, layer) && shows(&s, badge), "the row shows them");
    assert!(locked(&s));

    // Update writes the locked layer's visibility into the active row.
    s.execute("layer.setProps", &json!({"id": layer.0, "visible": false})).unwrap();
    let v = s.execute("dataset.update", &json!({})).unwrap();
    assert_eq!((v["values"].clone(), v["left"].clone()), (json!(2), json!([])), "{v}");
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][0]["values"], json!({"Badge": {"visible": true}, "Layer": {"visible": false}}), "{sets}");
}

/// Text and graph variables refuse locked art and art in a locked layer. A visibility variable is
/// refused for art on a hidden layer that is also locked, and the error says the art is in a hidden
/// group or layer.
#[test]
fn locked_art_refuses_text_and_graph_variables_and_a_hidden_layer_refuses_visibility() {
    let mut s = session();
    let layer = NodeId(s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    s.execute("object.setProps", &json!({"ids": [r.0], "locked": true})).unwrap();
    s.execute("layer.setProps", &json!({"id": layer.0, "locked": true})).unwrap();
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Chart", "kind": "graph"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    let bind = |s: &mut Session, var: &str, id: NodeId| s.execute("variable.bind", &json!({"variable": var, "ids": [id.0]}));

    let e = bind(&mut s, "Name", t).unwrap_err().to_string();
    assert!(e.contains(&format!("node {} is in a locked group or layer", t.0)), "{e}");
    let e = bind(&mut s, "Chart", r).unwrap_err().to_string();
    assert!(e.contains(&format!("node {} is locked", r.0)), "{e}");
    bind(&mut s, "Show", t).unwrap();
    assert!(s.doc().unwrap().doc.variables.objects_of("Name").is_empty());

    s.execute("layer.setProps", &json!({"id": layer.0, "visible": false})).unwrap();
    let e = bind(&mut s, "Show", r).unwrap_err().to_string();
    assert!(e.contains(&format!("node {} is in a hidden group or layer", r.0)), "{e}");
    // The hidden layer's own visibility is what the variable drives.
    bind(&mut s, "Show", layer).unwrap();
}

/// Make Text Dynamic / Make Visibility Dynamic: what the panel's buttons run. One step from a
/// selection to a bound variable, named after what it drives.
#[test]
fn making_a_selection_dynamic_names_and_binds_it() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);

    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    let v = s.execute("variable.makeTextDynamic", &json!({})).unwrap();
    assert_eq!(v["name"], "Alice", "named after the text it drives");
    assert_eq!(v["ids"], json!([t.0]));

    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let v = s.execute("variable.makeVisibilityDynamic", &json!({})).unwrap();
    assert_eq!(v["kind"], "visibility");

    // Type with the same name takes a number rather than colliding with the first variable.
    let twin = text(&mut s, "Alice");
    s.execute("select.set", &json!({"ids": [twin.0]})).unwrap();
    let again = s.execute("variable.makeTextDynamic", &json!({})).unwrap();
    assert_eq!(again["name"], "Alice 2");
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"].as_array().map(Vec::len), Some(3));

    // Nothing selected, art that can't take the kind, and locked art are errors.
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute("variable.makeTextDynamic", &json!({})).is_err());
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    assert!(s.execute("variable.makeTextDynamic", &json!({})).is_err(), "a rectangle is not type");
}

/// Make Text Dynamic reports its errors under its own command id.
#[test]
fn make_dynamic_errors_name_the_command() {
    let mut s = session();
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    s.execute("select.set", &json!({"ids": [t.0, r.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let e = s.execute("variable.makeTextDynamic", &json!({})).unwrap_err().to_string();
    assert!(e.contains("`variable.makeTextDynamic`") && e.contains("is not type"), "{e}");
}

/// Lock Variables refuses defining, deleting and renaming variables, Make … Dynamic included,
/// while binding, unbinding and the dataset commands still run (an import that would define a
/// variable is refused; see `locked_variables_refuse_header_cells_that_define_variables`).
#[test]
fn locked_variables_refuse_new_deleted_and_renamed_variables() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    assert_eq!(s.execute("variable.list", &json!({})).unwrap()["locked"], false);
    assert_eq!(s.execute("variable.lock", &json!({})).unwrap()["locked"], true, "without `locked` it toggles");
    assert_eq!(s.execute("variable.list", &json!({})).unwrap()["locked"], true);

    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    for (cmd, p) in [
        ("variable.define", json!({"name": "Other", "kind": "text"})),
        ("variable.delete", json!({"names": ["Name"]})),
        ("variable.rename", json!({"name": "Name", "newName": "Renamed"})),
        ("variable.makeTextDynamic", json!({})),
        ("variable.makeVisibilityDynamic", json!({})),
    ] {
        let e = s.execute(cmd, &p).unwrap_err();
        assert!(matches!(&e, EngineError::Disabled(_, why) if why == "variables are locked"), "{cmd}: {e}");
    }
    let names = |s: &mut Session| {
        s.execute("variable.list", &json!({})).unwrap()["variables"].as_array().unwrap().iter().map(|v| v["name"].clone()).collect::<Vec<_>>()
    };
    assert_eq!(names(&mut s), [json!("Name"), json!("Show")], "the variables are as they were");

    // Binding and unbinding still work, and so do the datasets.
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("variable.unbind", &json!({"ids": [r.0], "variable": "Show"})).unwrap();
    s.execute("dataset.capture", &json!({})).unwrap();
    s.execute("dataset.new", &json!({"name": "Bob", "values": {"Name": "Bob"}})).unwrap();
    s.execute("dataset.import", &json!({"csv": "Name\nCarol\n"})).unwrap();
    s.execute("dataset.select", &json!({"name": "Data Set 1"})).unwrap();
    assert_eq!(plain(&s, t), "Carol");

    // Unlocking lets a variable be defined again; `locked` sets the state rather than toggling.
    assert_eq!(s.execute("variable.lock", &json!({"locked": false})).unwrap()["locked"], false);
    assert_eq!(s.execute("variable.lock", &json!({"locked": false})).unwrap()["locked"], false);
    s.execute("variable.define", &json!({"name": "Other", "kind": "text"})).unwrap();
}

/// `variable.lock`'s params text names the dataset command it refuses: an import whose header
/// would define a variable.
#[test]
fn the_lock_params_text_names_the_refused_import() {
    let params = cmd::find_command("variable.lock").unwrap().params;
    assert!(params.contains("the dataset commands still run, except a dataset.import whose header would define a variable"), "{params}");
}

/// Locking is an undo step only when it changes the state, and the lock is saved with the
/// document: a file saved locked opens locked, and a file without the key opens unlocked.
#[test]
fn the_variables_lock_is_an_undo_step_and_is_saved() {
    let mut s = session();
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let before = steps(&s);
    s.execute("variable.lock", &json!({"locked": true})).unwrap();
    assert_eq!(steps(&s), before + 1);
    s.execute("variable.lock", &json!({"locked": true})).unwrap();
    assert_eq!(steps(&s), before + 1, "locking locked variables is not a step");
    assert!(s.doc().unwrap().is_dirty());

    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    assert!(vectorcraft_format::load(&saved).unwrap().variables.locked, "a document saved locked opens locked");
    let mut v: Value = serde_json::from_slice(&saved).unwrap();
    assert_eq!(v["document"]["variables"]["locked"], true);
    v["document"]["variables"].as_object_mut().unwrap().remove("locked");
    assert!(!vectorcraft_format::load(&serde_json::to_vec(&v).unwrap()).unwrap().variables.locked, "a file without the key opens unlocked");

    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!s.doc().unwrap().doc.variables.locked, "undo unlocks");
    let v: Value = serde_json::from_slice(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert!(v["document"].get("variables").is_none_or(|v| v.get("locked").is_none()), "unlocked variables write no key: {v}");
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.variables.locked, "redo locks again");
}

/// Capture Data Set records what the art holds now, which is how rows are made — the art is
/// edited and captured, rather than the values typed.
#[test]
fn capturing_records_what_the_art_holds_now() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();

    let v = s.execute("dataset.capture", &json!({})).unwrap();
    assert_eq!((v["name"].clone(), v["values"].clone()), (json!("Data Set 1"), json!(2)));
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][0]["values"]["Name"], json!({"text": "Alice"}));
    assert_eq!(sets["datasets"][0]["values"]["Show"], json!({"visible": true}));
    assert_eq!(sets["active"], "Data Set 1");

    // Change the art, capture again: a second row, and the first still matches its own text.
    s.execute("text.editRange", &json!({"id": t.0, "insert": "Bob"})).unwrap();
    assert_eq!(s.execute("dataset.list", &json!({})).unwrap()["datasets"][0]["matches"], json!(false), "the art moved on");
    s.execute("dataset.capture", &json!({})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][1]["name"], "Data Set 2");
    assert_eq!(sets["datasets"][1]["values"]["Name"], json!({"text": "Bob"}));
    assert_eq!(sets["datasets"][0]["matches"], json!(false), "row 1 still describes the old text");

    // A named capture, and a name that is taken is an error rather than a second row.
    s.execute("dataset.capture", &json!({"name": "Winter"})).unwrap();
    assert!(s.execute("dataset.capture", &json!({"name": "Winter"})).is_err());

    // Nothing bound is nothing to capture.
    let mut empty = session();
    empty.execute("variable.define", &json!({"name": "V", "kind": "text"})).unwrap();
    assert!(empty.execute("dataset.capture", &json!({})).is_err());
}

/// Update Data Set writes the art's current values into the row that is active.
#[test]
fn updating_writes_the_art_into_the_active_row() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();

    // No active row yet: say so rather than quietly writing into some other one.
    assert!(s.execute("dataset.update", &json!({})).is_err());
    s.execute("dataset.capture", &json!({})).unwrap();
    s.execute("text.editRange", &json!({"id": t.0, "insert": "Bob"})).unwrap();
    let v = s.execute("dataset.update", &json!({})).unwrap();
    assert_eq!((v["name"].clone(), v["values"].clone()), (json!("Data Set 1"), json!(1)));
    assert_eq!(s.execute("dataset.list", &json!({})).unwrap()["datasets"][0]["matches"], json!(true));
    // One undo step puts the row back as it was.
    s.execute("text.editRange", &json!({"id": t.0, "insert": "Caz"})).unwrap();
    s.execute("dataset.update", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "Caz");
}

/// Update Data Set writes only the variables the active row holds, so a row that sets some of
/// them still leaves the others alone when it is selected again. `all: true` adds the rest.
#[test]
fn updating_writes_only_the_variables_the_row_holds() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Client", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Client", "ids": [t.0]})).unwrap();
    s.execute("variable.define", &json!({"name": "Badge", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Badge", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "Badge off", "values": {"Badge": false}})).unwrap();
    s.execute("dataset.select", &json!({"name": "Badge off"})).unwrap();

    s.execute("text.editRange", &json!({"id": t.0, "insert": "Typed by hand"})).unwrap();
    s.execute("object.setProps", &json!({"ids": [r.0], "visible": true})).unwrap();
    let v = s.execute("dataset.update", &json!({})).unwrap();
    assert_eq!((v["values"].clone(), v["left"].clone()), (json!(1), json!(["Client"])), "{v}");
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][0]["values"], json!({"Badge": {"visible": true}}), "{sets}");

    // `all: true` adds every variable the art reports a value for.
    let v = s.execute("dataset.update", &json!({"all": true})).unwrap();
    assert_eq!((v["values"].clone(), v["left"].clone()), (json!(2), json!([])), "{v}");
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][0]["values"]["Client"], json!({"text": "Typed by hand"}), "{sets}");
}

/// A text variable whose object is locked reports no value, so Update keeps the row's value for it
/// rather than dropping it from the row.
#[test]
fn updating_keeps_a_value_the_art_does_not_report() {
    let mut s = session();
    let t = text(&mut s, "First");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Label", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Label", "ids": [t.0]})).unwrap();
    s.execute("variable.define", &json!({"name": "Mark", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Mark", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "Row", "values": {"Label": "Second", "Mark": true}})).unwrap();
    s.execute("dataset.select", &json!({"name": "Row"})).unwrap();

    s.execute("object.setProps", &json!({"ids": [t.0], "locked": true})).unwrap();
    s.execute("object.setProps", &json!({"ids": [r.0], "visible": false})).unwrap();
    for params in [json!({}), json!({"all": true})] {
        let v = s.execute("dataset.update", &params).unwrap();
        assert_eq!((v["values"].clone(), v["left"].clone()), (json!(1), json!(["Label"])), "{params}: {v}");
        let sets = s.execute("dataset.list", &json!({})).unwrap();
        assert_eq!(sets["datasets"][0]["values"], json!({"Label": {"text": "Second"}, "Mark": {"visible": false}}), "{params}: {sets}");
    }
}

/// With every bound object locked, Update keeps the whole row and adds no undo step, and
/// Capture says the bound objects report no value rather than asking for a binding.
#[test]
fn updating_with_every_bound_object_locked_keeps_the_row() {
    let mut s = session();
    let t = text(&mut s, "First");
    s.execute("variable.define", &json!({"name": "Label", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Label", "ids": [t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "Row", "values": {"Label": "Second"}})).unwrap();
    s.execute("dataset.select", &json!({"name": "Row"})).unwrap();
    s.execute("object.setProps", &json!({"ids": [t.0], "locked": true})).unwrap();

    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let before = steps(&s);
    for params in [json!({}), json!({"all": true})] {
        let v = s.execute("dataset.update", &params).unwrap();
        assert_eq!((v["values"].clone(), v["left"].clone()), (json!(0), json!(["Label"])), "{params}: {v}");
        let sets = s.execute("dataset.list", &json!({})).unwrap();
        assert_eq!(sets["datasets"][0]["values"], json!({"Label": {"text": "Second"}}), "{params}: {sets}");
    }
    assert_eq!(steps(&s), before, "an update that writes nothing is not a step");

    let e = s.execute("dataset.capture", &json!({})).unwrap_err().to_string();
    assert!(e.contains("no bound object reports a value"), "{e}");
    assert_eq!(s.execute("dataset.list", &json!({})).unwrap()["datasets"].as_array().unwrap().len(), 1);

    // With nothing bound at all, both still ask for a binding.
    s.execute("variable.unbind", &json!({"ids": [t.0]})).unwrap();
    for cmd in ["dataset.update", "dataset.capture"] {
        let e = s.execute(cmd, &json!({})).unwrap_err().to_string();
        assert!(e.contains("bind something to a variable first"), "{cmd}: {e}");
    }
}

/// A variable defined after a row was captured stays out of that row on Update, and joins it
/// with `all: true`.
#[test]
fn a_variable_defined_after_a_row_joins_it_only_with_all() {
    let mut s = session();
    let name = text(&mut s, "First job");
    let note = text(&mut s, "First note");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [name.0]})).unwrap();
    s.execute("dataset.capture", &json!({"name": "Job 1"})).unwrap();
    s.execute("variable.define", &json!({"name": "Note", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Note", "ids": [note.0]})).unwrap();

    s.execute("text.editRange", &json!({"id": name.0, "insert": "Second job"})).unwrap();
    let v = s.execute("dataset.update", &json!({})).unwrap();
    assert_eq!((v["values"].clone(), v["left"].clone()), (json!(1), json!(["Note"])), "{v}");
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][0]["values"], json!({"Name": {"text": "Second job"}}), "{sets}");

    s.execute("dataset.update", &json!({"all": true})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][0]["values"], json!({"Name": {"text": "Second job"}, "Note": {"text": "First note"}}), "{sets}");
}

/// Renaming carries the variable's bindings and its values in every row with it — a name is
/// how a variable is referred to, so leaving the old one behind would orphan both.
#[test]
fn renaming_carries_the_bindings_and_values_with_it() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "Bob"}})).unwrap();
    s.execute("dataset.select", &json!({"name": "D"})).unwrap();

    s.execute("variable.rename", &json!({"name": "Name", "newName": "Title"})).unwrap();
    assert_eq!(plain(&s, t), "Bob", "the row still applied");
    assert_eq!(s.execute("dataset.list", &json!({})).unwrap()["datasets"][0]["values"]["Title"], json!({"text": "Bob"}));
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!((vars["variables"][0]["name"].clone(), vars["variables"][0]["bindings"].clone()), (json!("Title"), json!(1)));

    // Renaming a dataset keeps it active and keeps its values.
    s.execute("dataset.rename", &json!({"name": "D", "newName": "Row 1"})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!((sets["active"].clone(), sets["datasets"][0]["name"].clone()), (json!("Row 1"), json!("Row 1")));

    // Taken and missing names are errors.
    s.execute("variable.define", &json!({"name": "Other", "kind": "text"})).unwrap();
    assert!(s.execute("variable.rename", &json!({"name": "Title", "newName": "Other"})).is_err());
    assert!(s.execute("variable.rename", &json!({"name": "Ghost", "newName": "X"})).is_err());
    assert!(s.execute("dataset.rename", &json!({"name": "Ghost", "newName": "X"})).is_err());
    // A dataset and a variable are separate namespaces: a dataset may take a variable's name,
    // and a variable may take a dataset's.
    s.execute("dataset.rename", &json!({"name": "Row 1", "newName": "Title"})).unwrap();
    assert_eq!(s.execute("dataset.list", &json!({})).unwrap()["active"], "Title");
    s.execute("dataset.new", &json!({"name": "Taken"})).unwrap();
    assert!(s.execute("dataset.rename", &json!({"name": "Title", "newName": "Taken"})).is_err());
}

/// Renaming a name that does not exist onto one that does reports the missing name, the same
/// error a rename to a free name gives.
#[test]
fn renaming_a_missing_name_onto_a_taken_one_reports_the_missing_name() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "A", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "A"})).unwrap();

    let e = s.execute("variable.rename", &json!({"name": "Nope", "newName": "A"})).unwrap_err().to_string();
    assert!(e.contains("no variable named `Nope`"), "{e}");
    let e = s.execute("dataset.rename", &json!({"name": "Nope", "newName": "A"})).unwrap_err().to_string();
    assert!(e.contains("no dataset named `Nope`"), "{e}");
}

/// `dataset.set` is how a value changes after the row exists: replacing it must not need the
/// dataset to be deleted and rebuilt (which loses its place in the list and the active one).
#[test]
fn dataset_set_replaces_the_row_and_reports_what_it_dropped() {
    let mut s = session();
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "y", "Show": false}})).unwrap();

    // The dataset keeps its place and stays active while its values change.
    let v = s.execute("dataset.set", &json!({"name": "D", "values": {"Name": "z"}})).unwrap();
    assert_eq!((v["values"].clone(), v["removed"].clone()), (json!(1), json!(["Show"])));
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"].as_array().map(Vec::len), Some(1));
    assert_eq!(sets["datasets"][0]["values"], json!({"Name": {"text": "z"}}));

    s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    assert_eq!(plain(&s, t), "z");
    assert!(s.doc().unwrap().doc.node(r).unwrap().visible, "`Show` has no value in this row now");

    // Errors are errors, not a silent rewrite of some other row.
    assert!(s.execute("dataset.set", &json!({"name": "Missing"})).is_err());
    assert!(s.execute("dataset.set", &json!({"name": "D", "values": {"Ghost": "z"}})).is_err());
    assert!(s.execute("dataset.set", &json!({"name": "D", "values": {"Show": "yes"}})).is_err());
    assert!(s.execute("dataset.set", &json!({"name": "D", "values": []})).is_err());
    // One step, so one undo puts the row back as it was.
    s.execute("dataset.set", &json!({"name": "D", "values": {"Name": "w"}})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(plain(&s, t), "z");
}

/// `newName` renames the row in the same step as its values change, and keeps it active.
#[test]
fn dataset_set_with_a_new_name_renames_the_row() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "y"}})).unwrap();
    s.execute("dataset.new", &json!({"name": "E", "values": {"Name": "e"}})).unwrap();
    s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    let undo = s.doc().unwrap().history.undo.len();

    let v = s.execute("dataset.set", &json!({"name": "D", "newName": "Spring", "values": {"Name": "z"}})).unwrap();
    assert_eq!(v["name"], "Spring");
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!((sets["datasets"][0]["name"].clone(), sets["active"].clone()), (json!("Spring"), json!("Spring")));
    assert_eq!(sets["datasets"][0]["values"], json!({"Name": {"text": "z"}}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");

    // Its own name is no rename; another row's name, or an empty one, is refused and changes nothing.
    s.execute("dataset.set", &json!({"name": "Spring", "newName": "Spring", "values": {"Name": "z"}})).unwrap();
    for bad in [json!("E"), json!(""), json!(3)] {
        assert!(s.execute("dataset.set", &json!({"name": "Spring", "newName": bad, "values": {"Name": "x"}})).is_err(), "{bad}");
    }
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][0]["values"], json!({"Name": {"text": "z"}}));
    assert_eq!(sets["datasets"][1]["values"], json!({"Name": {"text": "e"}}));

    // The set that changed nothing added no step: one undo takes the rename back.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.execute("dataset.list", &json!({})).unwrap()["active"], "D", "undo puts the name back");
}

#[test]
fn bad_variables_requests_are_errors_not_panics() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "N", "kind": "text"})).unwrap();
    for (cmd, params) in [
        ("variable.define", json!({})),
        ("variable.define", json!({"name": "", "kind": "text"})),
        ("variable.define", json!({"name": "N", "kind": "colour"})),
        ("variable.define", json!({"name": "N", "kind": "text"})),
        ("variable.delete", json!({})),
        ("variable.bind", json!({})),
        ("variable.bind", json!({"variable": "N"})),
        ("variable.bind", json!({"variable": "Missing", "ids": [1]})),
        ("dataset.new", json!({})),
        ("dataset.new", json!({"name": "D", "values": []})),
        ("dataset.new", json!({"name": "D", "values": {"N": 5}})),
        ("dataset.select", json!({"name": "Missing"})),
        ("dataset.next", json!({})),
    ] {
        assert!(s.execute(cmd, &params).is_err(), "{cmd} {params}");
    }
}

#[test]
fn highlight_selects_without_unbinding_and_deletes_leave_the_rest() {
    // The reviewer's regression case: two variables, two rows.
    let mut s = session();
    let a = text(&mut s, "Alice");
    let b = text(&mut s, "Bob");
    for (name, id) in [("Name", a), ("Other", b)] {
        s.execute("variable.define", &json!({"name": name, "kind": "text"})).unwrap();
        s.execute("variable.bind", &json!({"variable": name, "ids": [id.0]})).unwrap();
    }
    s.execute("dataset.new", &json!({"name": "One", "values": {"Name": "Alice", "Other": "Bob"}})).unwrap();
    s.execute("dataset.new", &json!({"name": "Two", "values": {"Name": "A2", "Other": "B2"}})).unwrap();
    s.execute("dataset.select", &json!({"name": "One"})).unwrap();

    // What a row click runs (highlight), then what Select Bound Object runs (select).
    s.execute("variable.highlight", &json!({"variable": "Name"})).unwrap();
    let bound: Vec<u64> = s.doc().unwrap().doc.variables.objects_of("Name").iter().map(|i| i.0).collect();
    s.execute("select.set", &json!({"ids": bound})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    // …and the binding is still there: selecting never unbinds.
    assert_eq!(s.doc().unwrap().doc.variables.objects_of("Name"), vec![a]);

    // Deleting one variable leaves the other; deleting the current dataset leaves the other.
    s.execute("variable.delete", &json!({"names": ["Name"]})).unwrap();
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"].as_array().unwrap().len(), 1);
    assert_eq!(vars["variables"][0]["name"], "Other");
    s.execute("dataset.delete", &json!({"names": ["One"]})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"].as_array().unwrap().len(), 1);
    assert_eq!(sets["datasets"][0]["name"], "Two");
}

#[test]
fn highlight_follows_rename_and_clears_with_delete() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    assert!(s.execute("variable.highlight", &json!({"variable": "Missing"})).is_err(), "unknown names stay errors");
    let v = s.execute("variable.highlight", &json!({"variable": "Name"})).unwrap();
    assert_eq!(v["variable"], "Name");
    // The highlight names a variable, so the rename carries it along.
    s.execute("variable.rename", &json!({"name": "Name", "newName": "Title"})).unwrap();
    assert_eq!(s.doc().unwrap().variables_highlight.as_deref(), Some("Title"));
    // Empty clears it, and deleting the highlighted variable clears it too.
    s.execute("variable.highlight", &json!({})).unwrap();
    assert!(s.doc().unwrap().variables_highlight.is_none());
    s.execute("variable.highlight", &json!({"variable": "Title"})).unwrap();
    s.execute("variable.delete", &json!({"names": ["Title"]})).unwrap();
    assert!(s.doc().unwrap().variables_highlight.is_none(), "a highlight of a deleted variable highlights nothing");
}

/// Undo of a rename takes the new name away, so the highlight that followed the rename goes
/// too: a variable defined later under that name is not highlighted, and Delete without names
/// does not delete it.
#[test]
fn undoing_a_rename_clears_the_highlight_of_the_new_name() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "A", "kind": "text"})).unwrap();
    s.execute("variable.highlight", &json!({"variable": "A"})).unwrap();
    s.execute("variable.rename", &json!({"name": "A", "newName": "B"})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().variables_highlight.is_none());
    s.execute("variable.define", &json!({"name": "B", "kind": "visibility"})).unwrap();
    assert!(s.doc().unwrap().variables_highlight.is_none(), "B was never clicked");
    assert!(s.execute("variable.delete", &json!({})).is_err(), "no variable highlighted");
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"].as_array().unwrap().len(), 2);
}

/// Undo of New Variable, Redo of Delete Variables and a canceled undo group each leave the
/// highlighted name undefined, and the highlight goes with it.
#[test]
fn undo_redo_and_a_canceled_group_clear_a_highlight_of_a_variable_they_remove() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "N", "kind": "text"})).unwrap();
    s.execute("variable.highlight", &json!({"variable": "N"})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().variables_highlight.is_none(), "undo of New Variable");

    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("variable.delete", &json!({"names": ["N"]})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("variable.highlight", &json!({"variable": "N"})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(s.doc().unwrap().variables_highlight.is_none(), "redo of Delete Variables");

    s.execute("edit.undoGroup.begin", &json!({})).unwrap();
    s.execute("variable.define", &json!({"name": "G", "kind": "text"})).unwrap();
    s.execute("variable.highlight", &json!({"variable": "G"})).unwrap();
    s.execute("edit.undoGroup.end", &json!({"cancel": true})).unwrap();
    assert!(s.doc().unwrap().doc.variables.variable("G").is_none(), "the cancel took G back");
    assert!(s.doc().unwrap().variables_highlight.is_none(), "a canceled undo group");
}

/// A row that hides an object must be able to show it again, and hiding it on the artboard
/// (as the Layers panel does) is what a capture records: a visibility variable reads and
/// writes its object's own visibility, which therefore never makes the object off limits.
#[test]
fn a_hidden_object_comes_back_and_is_captured_hidden() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0, t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "Shown", "values": {"Name": "Alice", "Show": true}})).unwrap();
    s.execute("dataset.new", &json!({"name": "Hidden", "values": {"Name": "Bob", "Show": false}})).unwrap();

    s.execute("dataset.select", &json!({"name": "Hidden"})).unwrap();
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible);
    let v = s.execute("dataset.select", &json!({"name": "Shown"})).unwrap();
    assert_eq!(v["skipped"], json!(0), "{v}");
    assert!(s.doc().unwrap().doc.node(r).unwrap().visible, "the row shows it again");
    assert_eq!(plain(&s, t), "Alice", "and rewrites the text of the object it had hidden");

    // Hidden on the artboard, then captured: the row says hidden, and it matches the art.
    s.execute("select.set", &json!({"ids": [t.0, r.0]})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();
    s.execute("dataset.capture", &json!({"name": "Captured"})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    let captured = sets["datasets"].as_array().unwrap().iter().find(|d| d["name"] == "Captured").unwrap().clone();
    assert_eq!(captured["values"]["Show"], json!({"visible": false}), "{captured}");
    assert_eq!(captured["matches"], json!(true));

    // A hidden object can still be bound by id.
    s.execute("variable.define", &json!({"name": "More", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "More", "ids": [r.0]})).unwrap();
}

/// A row selected while a layer is hidden rewrites the type on it and applies its other values,
/// and leaves the layer hidden: showing the layer shows that row's text. On a layer that is locked
/// as well as hidden, the type keeps its text and the row's visibility value applies.
#[test]
fn a_row_rewrites_type_on_a_hidden_layer_but_not_on_a_locked_one() {
    let mut s = session();
    let layer = NodeId(s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": "Alice", "Show": true}})).unwrap();
    s.execute("dataset.new", &json!({"name": "B", "values": {"Name": "Bob", "Show": false}})).unwrap();
    s.execute("dataset.select", &json!({"name": "A"})).unwrap();

    s.execute("layer.setProps", &json!({"id": layer.0, "visible": false})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": "B"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(2), Some(0)), "{v}");
    let doc = &s.doc().unwrap().doc;
    assert_eq!(plain(&s, t), "Bob", "the type on the hidden layer holds the row's text");
    assert!(!doc.node(r).unwrap().visible, "the row's visibility value applies on the hidden layer");
    assert!(!doc.node(layer).unwrap().visible, "the row leaves the layer hidden");
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    let matching: Vec<&Value> = sets["datasets"].as_array().unwrap().iter().filter(|d| d["matches"] == json!(true)).map(|d| &d["name"]).collect();
    assert_eq!(matching, [&json!("B")], "only the row the hidden art holds matches: {sets}");
    s.execute("layer.setProps", &json!({"id": layer.0, "visible": true})).unwrap();
    assert_eq!(plain(&s, t), "Bob", "showing the layer shows the row's text");

    // Locked and hidden: the row skips the type, which keeps its text, and shows the rectangle.
    s.execute("layer.setProps", &json!({"id": layer.0, "visible": false, "locked": true})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": "A"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(1)), "{v}");
    assert_eq!(plain(&s, t), "Bob", "the type on the locked layer keeps its text");
    assert!(s.doc().unwrap().doc.node(r).unwrap().visible, "the row's visibility value applies on the locked layer");
}

/// Type inside a hidden group is rewritten too, and List and Capture Data Set read it.
#[test]
fn a_row_rewrites_type_in_a_hidden_group_and_capture_reads_it() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let other = rect(&mut s);
    let g = group(&mut s, &[t, other]);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": "Alice"}})).unwrap();
    s.execute("dataset.new", &json!({"name": "B", "values": {"Name": "Bob"}})).unwrap();
    s.execute("object.setProps", &json!({"ids": [g.0], "visible": false})).unwrap();

    let v = s.execute("dataset.select", &json!({"name": "B"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(0)), "{v}");
    assert_eq!(plain(&s, t), "Bob");
    assert!(!s.doc().unwrap().doc.node(g).unwrap().visible, "the group stays hidden");

    let sets = s.execute("dataset.list", &json!({})).unwrap();
    let matches: Vec<&Value> = sets["datasets"].as_array().unwrap().iter().map(|d| &d["matches"]).collect();
    assert_eq!(matches, [&json!(false), &json!(true)], "the art holds B's text, not A's: {sets}");
    let v = s.execute("dataset.capture", &json!({"name": "C"})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][2]["values"], json!({"Name": {"text": "Bob"}}), "{v}: {sets}");
}

/// A row that hides selected objects deselects them, the key object included. Undo shows them
/// and selects them again with the key object.
#[test]
fn a_row_that_hides_selected_objects_deselects_them() {
    let mut s = session();
    let (r, q) = (rect(&mut s), rect(&mut s));
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "Hidden", "values": {"Show": false}})).unwrap();
    s.execute("select.set", &json!({"ids": [r.0, q.0]})).unwrap();
    s.execute("select.key", &json!({"id": r.0})).unwrap();

    s.execute("dataset.select", &json!({"name": "Hidden"})).unwrap();
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible);
    let sel = s.doc().unwrap().selection.clone();
    assert_eq!((sel.objects, sel.key), (vec![q], None), "r, the key object, is hidden");

    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(r).unwrap().visible);
    let sel = s.doc().unwrap().selection.clone();
    assert_eq!((sel.objects, sel.key), (vec![r, q], Some(r)), "undo selects r again as the key object");
}

/// A row that shows or hides a group also sets the objects inside it in the same selection,
/// whichever variable was defined first.
#[test]
fn a_row_sets_the_objects_inside_a_group_it_shows_or_hides() {
    let mut s = session();
    let (inner, other) = (rect(&mut s), rect(&mut s));
    let g = group(&mut s, &[inner, other]);
    s.execute("variable.define", &json!({"name": "Inner", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Inner", "ids": [inner.0]})).unwrap();
    s.execute("variable.define", &json!({"name": "Set", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Set", "ids": [g.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "Hidden", "values": {"Inner": false, "Set": false}})).unwrap();
    s.execute("dataset.new", &json!({"name": "Shown", "values": {"Inner": true, "Set": true}})).unwrap();

    let v = s.execute("dataset.select", &json!({"name": "Hidden"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(2), Some(0)), "{v}");
    let v = s.execute("dataset.select", &json!({"name": "Shown"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(2), Some(0)), "{v}");
    let doc = &s.doc().unwrap().doc;
    assert!(doc.node(g).unwrap().visible && doc.node(inner).unwrap().visible, "the group and the object inside it show");
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    let matching: Vec<&Value> = sets["datasets"].as_array().unwrap().iter().filter(|d| d["matches"] == json!(true)).map(|d| &d["name"]).collect();
    assert_eq!(matching, [&json!("Shown")], "{sets}");

    // With the inner object's variable defined last, a row that hides both still hides it.
    s.execute("variable.delete", &json!({"names": ["Inner"]})).unwrap();
    s.execute("variable.define", &json!({"name": "Inner", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Inner", "ids": [inner.0]})).unwrap();
    s.execute("dataset.set", &json!({"name": "Hidden", "values": {"Inner": false, "Set": false}})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": "Hidden"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(2), Some(0)), "{v}");
    let doc = &s.doc().unwrap().doc;
    assert!(!doc.node(g).unwrap().visible && !doc.node(inner).unwrap().visible, "both are hidden");
}

/// A row writes the text of type inside a group that it shows or hides, whichever variable was
/// defined first.
#[test]
fn a_row_writes_type_inside_a_group_it_shows_or_hides() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let other = rect(&mut s);
    let g = group(&mut s, &[t, other]);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.define", &json!({"name": "Set", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Set", "ids": [g.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "Hidden", "values": {"Name": "Bob", "Set": false}})).unwrap();
    s.execute("dataset.new", &json!({"name": "Shown", "values": {"Name": "Carol", "Set": true}})).unwrap();

    s.execute("dataset.select", &json!({"name": "Hidden"})).unwrap();
    assert_eq!(plain(&s, t), "Bob");
    let v = s.execute("dataset.select", &json!({"name": "Shown"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(2), Some(0)), "{v}");
    assert_eq!(plain(&s, t), "Carol", "the text is written although the group started hidden");

    // With the text variable defined last, a row that hides the group still writes the text.
    s.execute("variable.delete", &json!({"names": ["Name"]})).unwrap();
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.set", &json!({"name": "Hidden", "values": {"Name": "Bob", "Set": false}})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": "Hidden"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(2), Some(0)), "{v}");
    assert_eq!(plain(&s, t), "Bob", "the text is written before the group is hidden");
    assert!(!s.doc().unwrap().doc.node(g).unwrap().visible);
}

/// Of two visibility variables on one object, the one defined later decides whether it shows.
#[test]
fn the_later_visibility_variable_on_an_object_wins() {
    let mut s = session();
    let r = rect(&mut s);
    for name in ["First", "Second"] {
        s.execute("variable.define", &json!({"name": name, "kind": "visibility"})).unwrap();
        s.execute("variable.bind", &json!({"variable": name, "ids": [r.0]})).unwrap();
    }
    s.execute("dataset.new", &json!({"name": "Hide", "values": {"First": true, "Second": false}})).unwrap();
    s.execute("dataset.new", &json!({"name": "Show", "values": {"First": false, "Second": true}})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": "Hide"})).unwrap();
    assert_eq!(v["applied"], json!(2), "{v}");
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible);
    s.execute("dataset.select", &json!({"name": "Show"})).unwrap();
    assert!(s.doc().unwrap().doc.node(r).unwrap().visible);
}

/// Variables and datasets are separate namespaces: renaming a variable leaves a dataset of
/// the same name, and which dataset is active, alone.
#[test]
fn renaming_a_variable_leaves_a_dataset_of_the_same_name_active() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Title", "ids": [t.0]})).unwrap();
    s.execute("dataset.capture", &json!({"name": "Title"})).unwrap();
    s.execute("variable.rename", &json!({"name": "Title", "newName": "Heading"})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!((sets["active"].clone(), sets["datasets"][0]["name"].clone()), (json!("Title"), json!("Title")));
    s.execute("dataset.update", &json!({})).unwrap();
}

/// Without `names`, the deletes act on what the panel shows as current: the highlighted
/// variable and the active dataset (Window › Variables runs them with no params).
#[test]
fn deletes_without_names_take_the_highlight_and_the_active_dataset() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    for name in ["A", "B"] {
        s.execute("variable.define", &json!({"name": name, "kind": "text"})).unwrap();
    }
    s.execute("variable.bind", &json!({"variable": "A", "ids": [t.0]})).unwrap();
    s.execute("dataset.capture", &json!({})).unwrap();
    s.execute("dataset.capture", &json!({})).unwrap();
    assert!(s.execute("variable.delete", &json!({})).is_err(), "nothing highlighted");

    s.execute("variable.highlight", &json!({"variable": "B"})).unwrap();
    assert_eq!(s.execute("variable.delete", &json!({})).unwrap()["deleted"], json!(1));
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"].as_array().map(Vec::len), Some(1));
    assert_eq!(vars["variables"][0]["name"], "A");

    assert_eq!(s.execute("dataset.delete", &json!({})).unwrap()["deleted"], json!(1));
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"].as_array().map(Vec::len), Some(1));
    assert_eq!(sets["datasets"][0]["name"], "Data Set 1");
    assert!(s.execute("dataset.delete", &json!({})).is_err(), "no active dataset left");
}

/// Ids and names from an agent are checked, never reinterpreted: an `ids` that isn't a list
/// of objects is an error rather than the selection, and names are trimmed and capped.
#[test]
fn ids_and_names_from_an_agent_are_checked() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    s.execute("variable.define", &json!({"name": "  Name  ", "kind": "text"})).unwrap();
    assert_eq!(s.execute("variable.list", &json!({})).unwrap()["variables"][0]["name"], "Name");
    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    for params in
        [json!({"variable": "Name", "ids": 5}), json!({"variable": "Name", "ids": [t.0, "x"]}), json!({"variable": "Name", "ids": [999_999]})]
    {
        assert!(s.execute("variable.bind", &params).is_err(), "{params}");
    }
    assert!(s.doc().unwrap().doc.variables.objects_of("Name").is_empty(), "nothing was bound");
    assert!(s.execute("variable.unbind", &json!({"ids": "all"})).is_err());
    assert!(s.execute("variable.define", &json!({"name": "x".repeat(256), "kind": "text"})).is_err());
    s.execute("variable.define", &json!({"name": "x".repeat(255), "kind": "text"})).unwrap();
    assert!(s.execute("variable.delete", &json!({"names": []})).is_err());
    assert!(s.execute("variable.delete", &json!({"names": [5]})).is_err());
}

/// Image variables swap the linked file and keep the object's bounds; capture records the path.
#[test]
fn image_datasets_swap_pixels_and_keep_bounds() {
    let dir = Folder::new("vars-image");
    let red_path = dir.file("red.png");
    let blue_path = dir.file("blue.png");
    write(&red_path, &png(40, 30, RED));
    write(&blue_path, &png(80, 60, BLUE));

    let mut s = session();
    let id = place(&mut s, &red_path);
    let before = bounds(&s, id);

    s.execute("variable.define", &json!({"name": "Photo", "kind": "image"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Photo", "ids": [id.0]})).unwrap();
    // A text object can't take an image variable; a path must be a non-empty string.
    let t = text(&mut s, "x");
    assert!(s.execute("variable.bind", &json!({"variable": "Photo", "ids": [t.0]})).is_err());
    assert!(s.execute("dataset.new", &json!({"name": "B", "values": {"Photo": ""}})).is_err());
    assert!(s.execute("dataset.new", &json!({"name": "B", "values": {"Photo": true}})).is_err());

    s.execute("dataset.new", &json!({"name": "Blue", "values": {"Photo": blue_path}})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": "Blue"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(0)));
    assert_eq!(image(&s, id).link.as_ref().map(|l| l.path.as_str()), Some(blue_path.as_str()));
    assert_eq!(bounds(&s, id), before, "bounds stay when the file swaps");

    // Capture records the path the art holds now; a missing file is skipped, not an abort.
    s.execute("dataset.capture", &json!({"name": "Now"})).unwrap();
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    let now = sets["datasets"].as_array().unwrap().iter().find(|d| d["name"] == "Now").unwrap();
    assert_eq!(now["values"]["Photo"], json!({"image": blue_path}));

    let gone = dir.file("gone.png");
    s.execute("dataset.new", &json!({"name": "Missing", "values": {"Photo": gone}})).unwrap();
    let skip = s.execute("dataset.select", &json!({"name": "Missing"})).unwrap();
    assert_eq!((skip["applied"].as_u64(), skip["skipped"].as_u64()), (Some(0), Some(1)));
    assert_eq!(image(&s, id).link.as_ref().map(|l| l.path.as_str()), Some(blue_path.as_str()));

    // Make Image Dynamic: one step from a selection to a bound image variable.
    let id2 = place(&mut s, &red_path);
    s.execute("select.set", &json!({"ids": [id2.0]})).unwrap();
    let made = s.execute("variable.makeImageDynamic", &json!({})).unwrap();
    assert_eq!(made["kind"], "image");
    assert_eq!(made["ids"], json!([id2.0]));
    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    assert!(s.execute("variable.makeImageDynamic", &json!({})).is_err(), "type is not an image");
}

/// A text value with commas, quotes and line breaks comes back from the exported CSV as it went out.
#[test]
fn datasets_csv_keeps_line_breaks_in_quoted_cells() {
    let mut s = session();
    let t = text(&mut s, "One");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    let value = "Line one, \"quoted\"\nLine two";
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": value}})).unwrap();
    let csv = s.execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    let sets = &s.doc().unwrap().doc.variables.datasets;
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].values.get("Name"), Some(&vectorcraft_doc::DataValue::Text(value.into())));
    assert!(s.execute("dataset.import", &json!({"csv": "Name\n\"open"})).is_err(), "unclosed quote");
}

/// An empty text value is not a missing value: it comes back from the exported CSV as an empty
/// text, so selecting the data set still clears the bound text.
#[test]
fn datasets_csv_keeps_an_empty_text_value() {
    let mut s = session();
    let t = text(&mut s, "One");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": ""}})).unwrap();
    let csv = s.execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    let value = |s: &Session, name: &str| s.doc().unwrap().doc.variables.dataset(name).and_then(|d| d.values.get("Name")).cloned();
    assert_eq!(value(&s, "Data Set 1"), Some(vectorcraft_doc::DataValue::Text(String::new())));
    s.execute("variable.define", &json!({"name": "Other", "kind": "text"})).unwrap();
    s.execute("dataset.import", &json!({"csv": "Name,Other\n,x\n\"\",y\n"})).unwrap();
    assert_eq!(value(&s, "Data Set 2"), Some(vectorcraft_doc::DataValue::Text(String::new())));
    // Compatibility rule for files written before empty text was quoted: a bare empty cell stays "no value".
    assert_eq!(value(&s, "Data Set 1"), None);
    // A data set with no value for the variable exports a bare cell and stays without a value.
    let csv = s.execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    assert_eq!(csv, "Name,Other\n,x\n\"\",y\n");
    s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    assert_eq!(value(&s, "Data Set 1"), None);
}

/// A data set with no value for any variable exports as a row that imports back: a quoted empty
/// cell in the first column that isn't a text variable reads back as no value, and with only text
/// variables the first column reads back as an empty text.
#[test]
fn datasets_csv_keeps_a_data_set_without_values() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "A", "values": {}})).unwrap();
    s.execute("dataset.new", &json!({"name": "B", "values": {"Title": "x"}})).unwrap();
    let export = |s: &mut Session| s.execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    let csv = export(&mut s);
    assert_eq!(csv, "Title\n\"\"\nx\n");
    let imp = s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    assert_eq!(imp["names"], json!(["Data Set 1", "Data Set 2"]));
    let values = |s: &Session, name: &str| s.doc().unwrap().doc.variables.dataset(name).map(|d| d.values.clone()).unwrap();
    assert_eq!(values(&s, "Data Set 1").get("Title"), Some(&vectorcraft_doc::DataValue::Text(String::new())));

    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("dataset.new", &json!({"name": "C", "values": {}})).unwrap();
    let csv = export(&mut s);
    assert!(csv.ends_with("\n,\"\"\n"), "{csv:?}");
    s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    assert_eq!(s.doc().unwrap().doc.variables.datasets.len(), 3);
    assert!(values(&s, "Data Set 3").is_empty());
}

/// A quoted text cell keeps its leading and trailing spaces, so a padded value comes back from
/// the exported CSV as it went out; an unquoted cell, and spaces after a closing quote, are trimmed.
#[test]
fn datasets_csv_keeps_spaces_in_quoted_cells() {
    let mut s = session();
    let t = text(&mut s, "One");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": "  padded "}})).unwrap();
    let csv = s.execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    let value = |s: &Session, name: &str| s.doc().unwrap().doc.variables.dataset(name).and_then(|d| d.values.get("Name")).cloned();
    let text_value = |v: &str| Some(vectorcraft_doc::DataValue::Text(v.into()));
    assert_eq!(value(&s, "Data Set 1"), text_value("  padded "));
    s.execute("dataset.import", &json!({"csv": "Name\n  typed  \n\" quoted \"  \n"})).unwrap();
    assert_eq!(value(&s, "Data Set 1"), text_value("typed"));
    assert_eq!(value(&s, "Data Set 2"), text_value(" quoted "));
}

/// Export refuses a document with more data sets or variables than `dataset.import` reads, so
/// every file it writes imports back: 1,000 data sets and 256 variables export and import, and
/// one more of either is refused. The import refuses a line of 257 columns, the last one included.
#[test]
fn datasets_export_refuses_more_than_import_reads() {
    let mut s = session();
    let header: Vec<String> = (1..=256).map(|i| format!("V{i}")).collect();
    for wide in [format!("{},V257\nx\n", header.join(",")), format!("{}\nx{}\n", header.join(","), ",".repeat(256))] {
        let err = s.execute("dataset.import", &json!({"csv": wide})).unwrap_err().to_string();
        assert!(err.contains("more than 256 columns"), "{err}");
    }
    let rows: String = (1..=1000).map(|i| format!("Set {i},x\n")).collect();
    s.execute("dataset.import", &json!({"csv": format!("{}\n{rows}", header.join(","))})).unwrap();
    let csv = s.execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    s.execute("dataset.import", &json!({"csv": csv})).unwrap();

    s.execute("dataset.new", &json!({"name": "Set 1001"})).unwrap();
    let err = s.execute("dataset.export", &json!({})).unwrap_err().to_string();
    assert!(err.contains("more than 1000 data sets"), "{err}");
    s.execute("dataset.delete", &json!({"names": ["Set 1001"]})).unwrap();
    s.execute("dataset.export", &json!({})).unwrap();

    s.execute("variable.define", &json!({"name": "V257", "kind": "text"})).unwrap();
    let err = s.execute("dataset.export", &json!({})).unwrap_err().to_string();
    assert!(err.contains("more than 256 variables"), "{err}");
    let dir = Folder::new("vars-csv-caps");
    let out = dir.file("sets.csv");
    assert!(s.execute("dataset.export", &json!({"path": out})).is_err());
    assert!(!std::path::Path::new(&out).exists(), "no file is written");
}

/// A header row with a tab and no comma outside quotes makes tabs the separator; a quoted cell
/// may then hold tabs and commas. A header row with a comma outside quotes keeps commas, even
/// when a cell holds a tab. A tab-separated header with an unclosed quote fails with the reason.
#[test]
fn datasets_csv_reads_tab_separated_files() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.define", &json!({"name": "Last, First", "kind": "text"})).unwrap();
    let tsv = "Title\tShow\r\nTwo words\ttrue\r\n\"a, b\tc\"\tfalse\r\n";
    let imp = s.execute("dataset.import", &json!({"csv": tsv})).unwrap();
    assert_eq!(imp["names"], json!(["Data Set 1", "Data Set 2"]));
    let value = |s: &Session, name: &str, var: &str| s.doc().unwrap().doc.variables.dataset(name).and_then(|d| d.values.get(var)).cloned();
    let text_value = |v: &str| Some(vectorcraft_doc::DataValue::Text(v.into()));
    assert_eq!(value(&s, "Data Set 1", "Title"), text_value("Two words"));
    assert_eq!(value(&s, "Data Set 1", "Show"), Some(vectorcraft_doc::DataValue::Visible(true)));
    assert_eq!(value(&s, "Data Set 2", "Title"), text_value("a, b\tc"));
    assert_eq!(value(&s, "Data Set 2", "Show"), Some(vectorcraft_doc::DataValue::Visible(false)));
    s.execute("dataset.import", &json!({"csv": "Title,Show\none\ttwo,true"})).unwrap();
    assert_eq!(value(&s, "Data Set 1", "Title"), text_value("one\ttwo"));
    s.execute("dataset.import", &json!({"csv": "\"Last, First\"\tShow\nSmith, Jo\ttrue"})).unwrap();
    assert_eq!(value(&s, "Data Set 1", "Last, First"), text_value("Smith, Jo"));
    // A file of one variable whose name holds a tab is exported with the name quoted, and reads back as one column.
    let mut one = session();
    one.execute("variable.define", &json!({"name": "Tab\tName", "kind": "text"})).unwrap();
    one.execute("dataset.new", &json!({"name": "A", "values": {"Tab\tName": "v"}})).unwrap();
    let csv = one.execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    assert_eq!(csv, "\"Tab\tName\"\nv\n");
    let imp = one.execute("dataset.import", &json!({"csv": csv})).unwrap();
    assert_eq!(imp["defined"], json!([]), "{imp}");
    assert_eq!(value(&one, "Data Set 1", "Tab\tName"), text_value("v"));
    let unclosed = s.execute("dataset.import", &json!({"csv": "Title\t\"Show\nx\ttrue"})).unwrap_err().to_string();
    assert!(unclosed.contains("unclosed quote"), "{unclosed}");
}

/// Every header cell is a variable, whatever its case: a file laid out as the set-up page shows
/// (`Name,Age`) defines and fills `Name`, and `name` and `NAME` are variables too. The rows are
/// named Data Set 1, 2 and so on, in file order.
#[test]
fn datasets_csv_reads_every_header_cell_as_a_variable() {
    let mut s = session();
    let t = text(&mut s, "x");
    let imp = s.execute("dataset.import", &json!({"csv": "Name,Age\nBill Tucker,36\nJo Lee,41\n"})).unwrap();
    assert_eq!((imp["imported"].as_u64(), &imp["defined"]), (Some(2), &json!(["Name", "Age"])), "{imp}");
    assert_eq!(imp["names"], json!(["Data Set 1", "Data Set 2"]));
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.select", &json!({"name": "Data Set 2"})).unwrap();
    assert_eq!(plain(&s, t), "Jo Lee");
    for header in ["name", "NAME"] {
        let imp = s.execute("dataset.import", &json!({"csv": format!("{header},Age\nx,1\n")})).unwrap();
        assert_eq!((&imp["defined"], &imp["names"]), (&json!([header]), &json!(["Data Set 1"])), "{header}: {imp}");
    }
    let value = |s: &Session, var: &str| s.doc().unwrap().doc.variables.dataset("Data Set 1").and_then(|d| d.values.get(var)).cloned();
    assert_eq!(value(&s, "NAME"), Some(vectorcraft_doc::DataValue::Text("x".into())));
}

/// An import replaces every data set of the document, as Load Variable Library does, and clears
/// the active data set. The document's variables stay with their bindings, including those the
/// file has no column for, which then have no value in any data set. Undo brings the data sets back.
#[test]
fn datasets_csv_import_replaces_the_data_sets() {
    let mut s = session();
    let t = text(&mut s, "x");
    let r = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Title", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    for (name, title) in [("First", "a"), ("Data Set 2", "b"), ("Data Set 3", "c")] {
        s.execute("dataset.new", &json!({"name": name, "values": {"Title": title, "Show": true}})).unwrap();
    }
    s.execute("dataset.select", &json!({"name": "First"})).unwrap();
    let imp = s.execute("dataset.import", &json!({"csv": "Title\ny\nz\n"})).unwrap();
    assert_eq!(imp["names"], json!(["Data Set 1", "Data Set 2"]));
    assert_eq!(imp["deletedDataSets"], json!(["First", "Data Set 2", "Data Set 3"]));
    let vars = &s.doc().unwrap().doc.variables;
    let sets: Vec<_> = vars.datasets.iter().map(|d| (d.name.as_str(), d.values.get("Title").cloned(), d.values.get("Show").cloned())).collect();
    let text_value = |v: &str| Some(vectorcraft_doc::DataValue::Text(v.into()));
    assert_eq!(sets, [("Data Set 1", text_value("y"), None), ("Data Set 2", text_value("z"), None)]);
    assert_eq!(vars.active_dataset, None);
    let listed = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(listed["variables"], json!([{"name": "Title", "kind": "text", "bindings": 1}, {"name": "Show", "kind": "visibility", "bindings": 1}]));
    s.execute("dataset.select", &json!({"name": "Data Set 2"})).unwrap();
    assert_eq!(plain(&s, t), "z");

    // The first undo takes back the selection, the second the import.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    let vars = &s.doc().unwrap().doc.variables;
    let names: Vec<&str> = vars.datasets.iter().map(|d| d.name.as_str()).collect();
    assert_eq!((names, vars.active_dataset.as_deref()), (vec!["First", "Data Set 2", "Data Set 3"], Some("First")));
}

/// An error about a row's values gives the file line the row starts on, counting blank lines and
/// line breaks inside quoted cells.
#[test]
fn datasets_csv_row_errors_name_the_file_line() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    let error = |s: &mut Session, csv: &str| s.execute("dataset.import", &json!({"csv": csv})).unwrap_err().to_string();
    let blank_line = error(&mut s, "Show\n\nmaybe");
    assert!(blank_line.contains("line 3:"), "{blank_line}");
    let line_break = error(&mut s, "Title,Show\n\"one\ntwo\",true\nx,maybe");
    assert!(line_break.contains("line 4:"), "{line_break}");
}

/// A refused cell's error gives its line and why the value was refused: a visibility value that
/// isn't true or false, or a graph value longer than the graph data limit.
#[test]
fn datasets_csv_cell_errors_say_why() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.define", &json!({"name": "Sales", "kind": "graph"})).unwrap();
    let error = |s: &mut Session, csv: &str| s.execute("dataset.import", &json!({"csv": csv})).unwrap_err();
    let maybe = error(&mut s, "Show\nmaybe");
    let want = "line 2: value for `Show` must be true or false";
    assert!(matches!(&maybe, EngineError::BadParams { msg, .. } if msg == want), "{maybe}");
    let long = error(&mut s, &format!("Sales\n\"{}\"", "1,".repeat(140_000)));
    let want = "line 2: value for `Sales` is longer than 262144 bytes";
    assert!(matches!(&long, EngineError::BadParams { msg, .. } if msg == want), "{long}");
    assert!(s.doc().unwrap().doc.variables.datasets.is_empty());
}

/// A header cell that names no variable defines one, unbound, in the import's undo step: `#`
/// defines a visibility variable, `@` defines an image variable, and a cell without a prefix
/// defines a text variable. Importing the file again finds them; a prefix of the other kind, a
/// bare prefix and a long name fail and define nothing.
#[test]
fn datasets_csv_header_prefixes_define_variables() {
    let mut s = session();
    let csv = "#Shown,@Photo,Title\ntrue,a.png,x";
    let imp = s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    assert_eq!(imp["defined"], json!(["Shown", "Photo", "Title"]));
    assert_eq!((imp["imported"].as_u64(), imp["names"].clone()), (Some(1), json!(["Data Set 1"])));
    let listed = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(
        listed["variables"],
        json!([
            {"name": "Shown", "kind": "visibility", "bindings": 0},
            {"name": "Photo", "kind": "image", "bindings": 0},
            {"name": "Title", "kind": "text", "bindings": 0},
        ])
    );
    let value = |s: &Session, var: &str| s.doc().unwrap().doc.variables.dataset("Data Set 1").and_then(|d| d.values.get(var)).cloned();
    assert_eq!(value(&s, "Shown"), Some(vectorcraft_doc::DataValue::Visible(true)));
    assert_eq!(value(&s, "Photo"), Some(vectorcraft_doc::DataValue::Image("a.png".into())));
    assert_eq!(value(&s, "Title"), Some(vectorcraft_doc::DataValue::Text("x".into())));

    let again = s.execute("dataset.import", &json!({"csv": "#Shown\t@Photo\tTitle\nfalse\tb.png\ty"})).unwrap();
    assert_eq!((again["imported"].as_u64(), again["defined"].clone()), (Some(1), json!([])));
    assert_eq!(value(&s, "Shown"), Some(vectorcraft_doc::DataValue::Visible(false)));

    let error = |s: &mut Session, csv: &str| s.execute("dataset.import", &json!({"csv": csv})).unwrap_err().to_string();
    let other_kind = error(&mut s, "Shown,New,#Title\ntrue,n,true");
    assert!(other_kind.contains("column 3:") && other_kind.contains("text variable"), "{other_kind}");
    let twice = error(&mut s, "Fresh,@Fresh\nn,f.png");
    assert!(twice.contains("column 2:") && twice.contains("text variable"), "{twice}");
    let bare = error(&mut s, "Title,#\nx,true");
    assert!(bare.contains("column 2:") && bare.contains("names no variable"), "{bare}");
    let long = error(&mut s, &format!("Title,@{}\nx,b.png", "n".repeat(256)));
    assert!(long.contains("column 2:") && long.contains("longer than 255"), "{long}");
    assert_eq!(s.doc().unwrap().doc.variables.variables.len(), 3, "a failed import defines nothing");

    // Each import is one undo step, which takes back the variables it defined with its rows.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(value(&s, "Shown"), Some(vectorcraft_doc::DataValue::Visible(true)));
    assert_eq!(s.doc().unwrap().doc.variables.variables.len(), 3);
    s.execute("edit.undo", &json!({})).unwrap();
    let vars = &s.doc().unwrap().doc.variables;
    assert!(vars.variables.is_empty() && vars.datasets.is_empty());
}

/// A header cell that is a variable's exact name names it, even when the name starts with `#`,
/// `@` or `%`, so a file `dataset.export` wrote imports back without defining anything.
#[test]
fn datasets_csv_exact_names_win_over_prefixes() {
    let mut s = session();
    for (name, kind) in [("#Shown", "text"), ("@Photo", "visibility"), ("%Chart", "graph"), ("Shown", "image")] {
        s.execute("variable.define", &json!({"name": name, "kind": kind})).unwrap();
    }
    let values = json!({"#Shown": "hello", "@Photo": true, "%Chart": ",Q\nX,3", "Shown": "s.png"});
    s.execute("dataset.new", &json!({"name": "A", "values": values})).unwrap();
    let csv = s.execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    assert!(csv.starts_with("#Shown,#@Photo,%Chart,@Shown\n"), "{csv}");
    let imp = s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    assert_eq!((imp["imported"].as_u64(), imp["defined"].clone()), (Some(1), json!([])));
    let value = |var: &str| s.doc().unwrap().doc.variables.dataset("Data Set 1").and_then(|d| d.values.get(var)).cloned();
    assert_eq!(value("#Shown"), Some(vectorcraft_doc::DataValue::Text("hello".into())));
    assert_eq!(value("@Photo"), Some(vectorcraft_doc::DataValue::Visible(true)));
    assert_eq!(value("%Chart"), Some(vectorcraft_doc::DataValue::Graph(",Q\nX,3".into())));
    assert_eq!(value("Shown"), Some(vectorcraft_doc::DataValue::Image("s.png".into())));
    assert_eq!(s.doc().unwrap().doc.variables.variables.len(), 4);
}

/// `dataset.export` writes `#` before a visibility variable's header and `@` before an image
/// variable's, so a document without the variables reads the file back with their kinds. The
/// reply's `warning` names the variables it doesn't: a text variable whose name starts with `#`,
/// `@` or `%`, and a graph variable.
#[test]
fn datasets_csv_export_marks_visibility_and_image_columns() {
    let mut s = session();
    for (name, kind) in [("Name", "text"), ("Show", "visibility"), ("Photo", "image")] {
        s.execute("variable.define", &json!({"name": name, "kind": kind})).unwrap();
    }
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": "Ann", "Show": false, "Photo": "a.png"}})).unwrap();
    let out = s.execute("dataset.export", &json!({})).unwrap();
    assert_eq!(out["csv"], "Name,#Show,@Photo\nAnn,false,a.png\n");
    assert!(out.get("warning").is_none(), "{out}");

    let mut fresh = session();
    let imp = fresh.execute("dataset.import", &json!({"csv": out["csv"]})).unwrap();
    assert_eq!(imp["defined"], json!(["Name", "Show", "Photo"]));
    let listed = fresh.execute("variable.list", &json!({})).unwrap();
    let kinds: Vec<_> = listed["variables"].as_array().unwrap().iter().map(|v| v["kind"].clone()).collect();
    assert_eq!(kinds, [json!("text"), json!("visibility"), json!("image")]);
    let value = |var: &str| fresh.doc().unwrap().doc.variables.dataset("Data Set 1").and_then(|d| d.values.get(var)).cloned();
    assert_eq!(value("Name"), Some(vectorcraft_doc::DataValue::Text("Ann".into())));
    assert_eq!(value("Show"), Some(vectorcraft_doc::DataValue::Visible(false)));
    assert_eq!(value("Photo"), Some(vectorcraft_doc::DataValue::Image("a.png".into())));

    for (name, kind) in [("#Tag", "text"), ("Chart", "graph"), ("@Note", "text")] {
        s.execute("variable.define", &json!({"name": name, "kind": kind})).unwrap();
    }
    let out = s.execute("dataset.export", &json!({})).unwrap();
    assert_eq!(out["warning"], "a new document won't read these variables back with their kind: `#Tag`, `Chart`, `@Note`");
    assert!(out["csv"].as_str().unwrap().starts_with("Name,#Show,@Photo,#Tag,Chart,@Note\n"), "{out}");
}

/// A file `dataset.export` writes has no column of data set names, and `dataset.import` reads
/// every column as a variable, so the file imports back to the same data sets: the same variables
/// with their kinds, and each data set's values in order, a data set without values included, named
/// Data Set 1, 2 and so on. In a document whose data sets already have those names and values, the
/// import changes nothing.
#[test]
fn datasets_csv_export_imports_back_to_the_same_data_sets() {
    let mut s = session();
    for (name, kind) in [("Name", "text"), ("Show", "visibility"), ("Photo", "image")] {
        s.execute("variable.define", &json!({"name": name, "kind": kind})).unwrap();
    }
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": "Ann, Jr.", "Show": true, "Photo": "a.png"}})).unwrap();
    s.execute("dataset.new", &json!({"name": "B", "values": {"Show": false}})).unwrap();
    s.execute("dataset.new", &json!({"name": "C", "values": {}})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Name": "", "Photo": "d.png"}})).unwrap();
    let csv = s.execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    assert!(csv.starts_with("Name,#Show,@Photo\n"), "{csv}");
    let before = s.doc().unwrap().doc.variables.clone();

    let mut fresh = session();
    fresh.execute("dataset.import", &json!({"csv": csv})).unwrap();
    let vars = fresh.doc().unwrap().doc.variables.clone();
    assert_eq!(vars.variables, before.variables, "the same variables, with their kinds");
    let values = |v: &vectorcraft_doc::Variables| v.datasets.iter().map(|d| d.values.clone()).collect::<Vec<_>>();
    assert_eq!(values(&vars), values(&before), "the same values in every data set");
    let names: Vec<&str> = vars.datasets.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, ["Data Set 1", "Data Set 2", "Data Set 3", "Data Set 4"]);

    // Exporting the imported data sets writes the same file, and importing it changes nothing: no
    // undo step, and the active data set stays.
    let again = fresh.execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    assert_eq!(again, csv);
    fresh.execute("dataset.select", &json!({"name": "Data Set 1"})).unwrap();
    let vars = fresh.doc().unwrap().doc.variables.clone();
    assert!(vars.active_dataset.is_some());
    let undo = fresh.doc().unwrap().history.undo.len();
    fresh.execute("dataset.import", &json!({"csv": again})).unwrap();
    assert_eq!(fresh.doc().unwrap().history.undo.len(), undo, "no undo step");
    assert_eq!(fresh.doc().unwrap().doc.variables, vars);
}

/// The export's `warning` names the variables whose column a document without the variables
/// refuses, so it doesn't import the file: a `%` header, a `#` header whose values aren't true or
/// false, and a header that names a variable an earlier column defined with another kind. A
/// header that names such a variable with its kind is read with that kind, and is listed as one
/// read back with another kind.
#[test]
fn datasets_csv_export_warns_of_headers_a_new_document_refuses() {
    let export = |vars: &[(&str, &str)], values: Value| {
        let mut s = session();
        for (name, kind) in vars {
            s.execute("variable.define", &json!({"name": name, "kind": kind})).unwrap();
        }
        s.execute("dataset.new", &json!({"name": "A", "values": values})).unwrap();
        let out = s.execute("dataset.export", &json!({})).unwrap();
        let refused = session().execute("dataset.import", &json!({"csv": out["csv"]})).is_err();
        (out["warning"].as_str().unwrap_or("").to_string(), refused)
    };
    let refuses = |names: &str| format!("a new document refuses this file because of these variables: {names}");
    let loses = |names: &str| format!("a new document won't read these variables back with their kind: {names}");

    assert_eq!(export(&[("%Rate", "text"), ("Chart", "graph")], json!({})), (refuses("`%Rate`"), true));
    assert_eq!(export(&[("#Tag", "text")], json!({"#Tag": "hello"})), (refuses("`#Tag`"), true));
    assert_eq!(export(&[("#Tag", "text")], json!({"#Tag": "yes"})), (loses("`#Tag`"), false));
    assert_eq!(export(&[("#Seen", "text"), ("Seen", "image")], json!({})), (refuses("`Seen`"), true));
    assert_eq!(export(&[("#Seen", "text"), ("Seen", "text")], json!({})), (refuses("`Seen`"), true));
}

/// `dataset.export` is refused, naming both variables, when a variable's header is another
/// variable's name, which an import reads as that variable; with `path`, no file is written.
#[test]
fn datasets_csv_export_refuses_a_header_that_names_another_variable() {
    let dir = Folder::new("vars-csv-header");
    for (first, second) in
        [(("Show", "visibility"), ("#Show", "text")), (("Photo", "image"), ("@Photo", "graph")), (("X", "visibility"), ("#X", "image"))]
    {
        let mut s = session();
        for (name, kind) in [first, second] {
            s.execute("variable.define", &json!({"name": name, "kind": kind})).unwrap();
        }
        let e = s.execute("dataset.export", &json!({})).unwrap_err().to_string();
        let (name, other) = (first.0, second.0);
        assert!(e.contains(&format!("for `{name}` would read back as the variable `{other}`")), "{e}");
        let out = dir.file("sets.csv");
        assert!(s.execute("dataset.export", &json!({"path": out})).is_err());
        assert!(!std::path::Path::new(&out).exists(), "{out}");
    }
}

/// A `%` header cell that isn't a variable's name (graph data files) is refused with its column,
/// and the import defines nothing.
#[test]
fn datasets_csv_refuses_graph_data_file_columns() {
    let mut s = session();
    let error = |s: &mut Session, csv: &str| s.execute("dataset.import", &json!({"csv": csv})).unwrap_err().to_string();
    let named = error(&mut s, "Name,Title,%Sales\nA,x,sales.txt");
    assert!(named.contains("column 3:") && named.contains("`%Sales`"), "{named}");
    let unnamed = error(&mut s, "%Sales\tTitle\nsales.txt\tx");
    assert!(unnamed.contains("column 1:") && unnamed.contains("`%Sales`"), "{unnamed}");
    let vars = &s.doc().unwrap().doc.variables;
    assert!(vars.variables.is_empty() && vars.datasets.is_empty());
}

/// Two header cells that name one variable, by the same name or by a name and its prefix, are
/// refused with both columns, and the import changes nothing.
#[test]
fn datasets_csv_refuses_two_columns_for_one_variable() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Shown", "kind": "visibility"})).unwrap();
    let error = |s: &mut Session, csv: &str| s.execute("dataset.import", &json!({"csv": csv})).unwrap_err().to_string();
    for (csv, first, second, var) in [
        ("Title,City,City\nR,Paris,Rome", 2, 3, "City"),
        ("Title,City,City\nR,Paris,", 2, 3, "City"),
        ("City,Title,City\nParis,x,Rome", 1, 3, "City"),
        ("Title,Shown,#Shown\nR,true,false", 2, 3, "Shown"),
        ("Title\t@Photo\t@ Photo\nx\ta.png\tb.png", 2, 3, "Photo"),
    ] {
        let err = error(&mut s, csv);
        assert!(err.contains(&format!("columns {first} and {second} both name the variable `{var}`")), "{csv}: {err}");
    }
    let vars = &s.doc().unwrap().doc.variables;
    assert_eq!(vars.variables.len(), 1);
    assert!(vars.datasets.is_empty());
}

/// While the variables are locked, an import is refused when a header cell would define a
/// variable, as New Variable is, and a file whose headers all name variables imports.
#[test]
fn locked_variables_refuse_header_cells_that_define_variables() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Shown", "kind": "visibility"})).unwrap();
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.lock", &json!({"locked": true})).unwrap();
    for csv in ["#Shown,Title,New\ntrue,x,n", "@Photo\nb.png", "Title,#Fresh\nx,true"] {
        let e = s.execute("dataset.import", &json!({"csv": csv})).unwrap_err();
        assert!(matches!(&e, EngineError::Disabled(cmd, why) if cmd == "dataset.import" && why == "variables are locked"), "{csv}: {e}");
    }
    let vars = &s.doc().unwrap().doc.variables;
    assert!(vars.variables.len() == 2 && vars.datasets.is_empty(), "a refused import defines and imports nothing");

    let imp = s.execute("dataset.import", &json!({"csv": "#Shown,Title\ntrue,x"})).unwrap();
    assert_eq!((imp["imported"].as_u64(), imp["defined"].clone()), (Some(1), json!([])));
    s.execute("variable.lock", &json!({"locked": false})).unwrap();
    let imp = s.execute("dataset.import", &json!({"csv": "New\nn"})).unwrap();
    assert_eq!(imp["defined"], json!(["New"]));
}

/// Import / export datasets as CSV: one column per variable and no dataset names; round-trip keeps
/// the values in order, and the rows come back as Data Set 1, 2.
#[test]
fn datasets_csv_round_trips_and_rejects_bad_input() {
    let dir = Folder::new("vars-csv");
    let red_path = dir.file("red.png");
    let blue_path = dir.file("blue.png");
    write(&red_path, &png(8, 8, RED));
    write(&blue_path, &png(8, 8, BLUE));

    let mut s = session();
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);
    let img = place(&mut s, &red_path);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.define", &json!({"name": "Photo", "kind": "image"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Photo", "ids": [img.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": "Alice", "Show": true, "Photo": red_path}})).unwrap();
    s.execute("dataset.new", &json!({"name": "B", "values": {"Name": "Bob", "Show": false, "Photo": blue_path}})).unwrap();

    let exported = s.execute("dataset.export", &json!({})).unwrap();
    let csv = exported["csv"].as_str().unwrap().to_string();
    assert!(csv.starts_with("Name,#Show,@Photo\n"), "{csv}");
    assert!(csv.contains("Alice") && csv.contains("Bob") && csv.contains("false"), "{csv}");

    // Import again, which replaces A and B, and apply the second row.
    let imp = s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    assert_eq!((imp["imported"].as_u64(), &imp["deletedDataSets"]), (Some(2), &json!(["A", "B"])));
    s.execute("dataset.select", &json!({"name": "Data Set 2"})).unwrap();
    assert_eq!(plain(&s, t), "Bob");
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible);
    assert_eq!(image(&s, img).link.as_ref().map(|l| l.path.as_str()), Some(blue_path.as_str()));

    // Re-import replaces the rows and defines nothing; a bad visibility value is an error.
    let again = s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    assert_eq!((again["imported"].as_u64(), &again["deletedDataSets"]), (Some(2), &json!(["Data Set 1", "Data Set 2"])));
    assert_eq!(again["defined"], json!([]));
    assert!(s.execute("dataset.import", &json!({"csv": "Show\nmaybe"})).is_err());

    let out = dir.file("sets.csv");
    s.execute("dataset.export", &json!({"path": out})).unwrap();
    let from_file = s.execute("dataset.import", &json!({"path": out})).unwrap();
    assert_eq!(from_file["names"], json!(["Data Set 1", "Data Set 2"]));

    // One undo undoes the last import's replacements.
    s.execute("edit.undo", &json!({})).unwrap();
}

/// A CSV that isn't UTF-8 is read as Windows-1252. UTF-8 files, with or without a byte-order mark,
/// read as before, and a file that starts with the UTF-8 byte-order mark but isn't UTF-8 is refused.
#[test]
fn datasets_csv_reads_windows_1252_files() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "City", "kind": "text"})).unwrap();
    let import = |s: &mut Session, bytes: &[u8]| s.execute("dataset.import", &json!({"dataBase64": vectorcraft_format::base64_encode(bytes)}));
    let value = |s: &Session, name: &str| s.doc().unwrap().doc.variables.dataset(name).and_then(|d| d.values.get("City")).cloned();
    let text_value = |v: &str| Some(vectorcraft_doc::DataValue::Text(v.into()));
    let imp = import(&mut s, b"City\r\nJos\xe9\r\n\x80 5 \x96 \x93Caf\xe9\x94\r\n").unwrap();
    assert_eq!(imp["names"], json!(["Data Set 1", "Data Set 2"]));
    assert_eq!(value(&s, "Data Set 1"), text_value("José"));
    assert_eq!(value(&s, "Data Set 2"), text_value("€ 5 – “Café”"));
    import(&mut s, "City\nJosé\n".as_bytes()).unwrap();
    assert_eq!(value(&s, "Data Set 1"), text_value("José"));
    import(&mut s, "\u{feff}City\nZoë\n".as_bytes()).unwrap();
    assert_eq!(value(&s, "Data Set 1"), text_value("Zoë"));
    let e = import(&mut s, b"\xef\xbb\xbfCity\nJos\xe9\n").unwrap_err().to_string();
    assert!(e.contains("CSV is not UTF-8"), "{e}");
    assert_eq!(value(&s, "Data Set 1"), text_value("Zoë"));
}

/// A CSV that starts with a UTF-16 byte-order mark is read as UTF-16, and
/// one that isn't valid UTF-16 is refused. A file with NUL bytes and no byte-order mark is refused.
/// A text value holding NUL imports back from the `csv` that `dataset.export` returns.
#[test]
fn datasets_csv_reads_utf16_files() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "City", "kind": "text"})).unwrap();
    let import = |s: &mut Session, bytes: &[u8]| s.execute("dataset.import", &json!({"dataBase64": vectorcraft_format::base64_encode(bytes)}));
    let value = |s: &Session, name: &str| s.doc().unwrap().doc.variables.dataset(name).and_then(|d| d.values.get("City")).cloned();
    let text_value = |v: &str| Some(vectorcraft_doc::DataValue::Text(v.into()));
    let utf16 = |bom: &[u8], text: &str, le: bool| -> Vec<u8> {
        let units = text.encode_utf16().flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() });
        bom.iter().copied().chain(units).collect()
    };
    import(&mut s, &utf16(b"\xff\xfe", "City\r\nZoë 😀\r\n", true)).unwrap();
    assert_eq!(value(&s, "Data Set 1"), text_value("Zoë 😀"));
    import(&mut s, &utf16(b"\xfe\xff", "City\nJosé\n", false)).unwrap();
    assert_eq!(value(&s, "Data Set 1"), text_value("José"));
    let mut odd = utf16(b"\xff\xfe", "City\nx\n", true);
    odd.push(b'y');
    let e = import(&mut s, &odd).unwrap_err().to_string();
    assert!(e.contains("CSV is not UTF-16"), "{e}");
    let e = import(&mut s, &[b"\xff\xfe".as_slice(), "City\nAB,".as_bytes(), b"\x00\xd8"].concat()).unwrap_err().to_string();
    assert!(e.contains("CSV is not UTF-16"), "{e}");
    let e = import(&mut s, &utf16(b"", "City\nx\n", true)).unwrap_err().to_string();
    assert!(e.contains("CSV holds NUL bytes"), "{e}");
    assert_eq!(value(&s, "Data Set 1"), text_value("José"), "a refused file changes nothing");
    s.execute("dataset.set", &json!({"name": "Data Set 1", "values": {"City": "a\u{0}b"}})).unwrap();
    let csv = s.execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    s.execute("dataset.delete", &json!({"names": ["Data Set 1"]})).unwrap();
    s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    assert_eq!(value(&s, "Data Set 1"), text_value("a\u{0}b"));
}

/// A data set of [`library`]: its name, and each variable's name with its paragraphs.
type LibrarySet<'a> = (&'a str, &'a [(&'a str, &'a [&'a str])]);

/// A variable library written from its public grammar: a document type declaration with the
/// namespace entities, `variableSets` › `variableSet` › `variables` with one `variable` element per
/// `(varName, trait)`, then `sampleDataSets` with one `sampleDataSet` per data set, holding an
/// element per variable with a `p` element per paragraph.
fn library(vars: &[(&str, &str)], sets: &[LibrarySet]) -> String {
    let mut x = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\" [\n\
         \t<!ENTITY ns_vars \"http://ns.adobe.com/Variables/1.0/\">\n\
         \t<!ENTITY ns_custom \"http://ns.adobe.com/GenericCustomNamespace/1.0/\">\n\
         \t<!ENTITY ns_flows \"http://ns.adobe.com/Flows/1.0/\">\n\
         ]>\n<svg>\n<variableSets xmlns=\"&ns_vars;\">\n<variableSet varSetName=\"binding1\" locked=\"none\">\n<variables>\n",
    );
    for (name, kind) in vars {
        x += &format!("<variable varName=\"{name}\" trait=\"{kind}\" category=\"&ns_flows;\"></variable>\n");
    }
    x += "</variables>\n<v:sampleDataSets xmlns:v=\"&ns_vars;\" xmlns=\"&ns_custom;\">\n";
    for (set, values) in sets {
        x += &format!("<v:sampleDataSet dataSetName=\"{set}\">\n");
        for (var, paras) in *values {
            let paras: String = paras.iter().map(|p| format!("<p>{p}</p>")).collect();
            x += &format!("<{var}>{paras}</{var}>\n");
        }
        x += "</v:sampleDataSet>\n";
    }
    x + "</v:sampleDataSets>\n</variableSet>\n</variableSets>\n</svg>\n"
}

/// Load Variable Library defines the library's text variables, unbound, and adds its data sets,
/// in one undo step. A value is the text of the `p` elements, one paragraph each; an element
/// without a `p` gives no value. A file, and base64 with a UTF-8 byte-order mark, load the same.
#[test]
fn a_variable_library_defines_text_variables_and_data_sets() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let xml = library(
        &[("Name", "textcontent"), ("Note", "textcontent")],
        &[("Row 1", &[("Name", &["Bravo"]), ("Note", &["One", "Two &amp; three"])]), ("Row 2", &[("Name", &["Charlie"]), ("Note", &[])])],
    );
    let r = s.execute("variable.loadLibrary", &json!({"xml": xml})).unwrap();
    assert_eq!(r["variables"], json!(["Name", "Note"]));
    assert_eq!((r["deleted"].clone(), r["deletedDataSets"].clone()), (json!([]), json!([])));
    assert_eq!(r["names"], json!(["Row 1", "Row 2"]));
    assert_eq!((r["skipped"].clone(), r.get("warning")), (json!([]), None));
    let listed = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(listed["variables"], json!([{"name": "Name", "kind": "text", "bindings": 0}, {"name": "Note", "kind": "text", "bindings": 0}]));
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"][0]["values"], json!({"Name": {"text": "Bravo"}, "Note": {"text": "One\nTwo & three"}}));
    assert_eq!(sets["datasets"][1]["values"], json!({"Name": {"text": "Charlie"}}));
    assert_eq!(sets["active"], Value::Null);
    assert_eq!(plain(&s, t), "Alice", "loading binds nothing and applies nothing");

    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.select", &json!({"name": "Row 2"})).unwrap();
    assert_eq!(plain(&s, t), "Charlie");

    // One undo step each: the select, the bind, then the load.
    for _ in 0..3 {
        s.execute("edit.undo", &json!({})).unwrap();
    }
    let vars = &s.doc().unwrap().doc.variables;
    assert!(vars.variables.is_empty() && vars.datasets.is_empty());

    let dir = Folder::new("vars-library");
    let path = dir.file("library.xml");
    write(&path, xml.as_bytes());
    let from_file = s.execute("variable.loadLibrary", &json!({"path": path})).unwrap();
    assert_eq!(from_file["names"], json!(["Row 1", "Row 2"]));
    let bom = [b"\xEF\xBB\xBF".as_slice(), xml.as_bytes()].concat();
    let from_base64 = s.execute("variable.loadLibrary", &json!({"dataBase64": vectorcraft_format::base64_encode(&bom)})).unwrap();
    // The library declares the document's text variables: they stay, and only the data sets go.
    assert_eq!((from_base64["deleted"].clone(), from_base64["deletedDataSets"].clone()), (json!([]), json!(["Row 1", "Row 2"])));
    assert_eq!(s.doc().unwrap().doc.variables.datasets.len(), 2);
}

/// Load Variable Library replaces the document's variables and data sets: every variable goes with
/// its bindings and values, a variable of another kind with a library variable's name included,
/// and every data set goes, the active one included. A text variable the library also declares
/// stays bound (#4136), and the library's data sets then change its text. The highlight of a
/// variable that went highlights nothing. Undo brings back the variables, bindings, data sets and
/// active data set as they were.
#[test]
fn a_variable_library_replaces_every_variable_and_data_set() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);
    for (name, kind, id) in [("Name", "text", t), ("Title", "visibility", r), ("Kept", "visibility", r)] {
        s.execute("variable.define", &json!({"name": name, "kind": kind})).unwrap();
        s.execute("variable.bind", &json!({"variable": name, "ids": [id.0]})).unwrap();
    }
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": "Old", "Title": true}})).unwrap();
    s.execute("dataset.new", &json!({"name": "Other", "values": {"Name": "Stays", "Title": false, "Kept": true}})).unwrap();
    s.execute("dataset.select", &json!({"name": "Other"})).unwrap();
    s.execute("variable.highlight", &json!({"variable": "Kept"})).unwrap();
    let before = s.doc().unwrap().doc.variables.clone();

    let xml = library(&[("Name", "textcontent"), ("Title", "textcontent")], &[("A", &[("Name", &["New"]), ("Title", &["Heading"])])]);
    let loaded = s.execute("variable.loadLibrary", &json!({"xml": xml})).unwrap();
    assert_eq!(loaded["deleted"], json!(["Title", "Kept"]));
    assert_eq!((loaded["deletedDataSets"].clone(), loaded["names"].clone()), (json!(["A", "Other"]), json!(["A"])));
    let listed = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(listed["variables"], json!([{"name": "Name", "kind": "text", "bindings": 1}, {"name": "Title", "kind": "text", "bindings": 0}]));
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"].as_array().map(Vec::len), Some(1));
    assert_eq!(sets["datasets"][0]["values"], json!({"Name": {"text": "New"}, "Title": {"text": "Heading"}}));
    assert_eq!(sets["active"], Value::Null);
    let bindings = &s.doc().unwrap().doc.variables.bindings;
    assert_eq!(bindings.len(), 1, "the bindings went with the variables, but for Name's");
    assert_eq!(bindings.get(&t).map(|v| v.len()), Some(1));
    assert!(s.doc().unwrap().variables_highlight.is_none(), "the highlighted variable went");
    assert_eq!(plain(&s, t), "Stays", "loading applies nothing");
    s.execute("dataset.select", &json!({"name": "A"})).unwrap();
    assert_eq!(plain(&s, t), "New", "the library's data set changes the text bound to Name");
    s.execute("edit.undo", &json!({})).unwrap();

    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.variables, before, "one undo step brings everything back");
}

/// An import or a library load that leaves the variables and data sets as they were (the same CSV
/// imported again, the same rows tab-separated, the file Export Data Sets writes, the same library
/// loaded again) records no undo step: the step to redo stays, and the document stays unmodified.
/// An import or load that changes something is still one undo step.
#[test]
fn an_import_or_library_load_that_changes_nothing_records_no_undo_step() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    let csv = "Name,#Shown\nAnn,true\nBob,false\n";
    s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    let exported = s.execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    for csv in [csv, "Name\t#Shown\nAnn\ttrue\nBob\tfalse\n", exported.as_str()] {
        super::select_with_a_step_to_redo(&mut s, &[t]);
        let before = super::history(&s);
        assert_eq!((before.1, before.2), (1, false), "a step to redo, and nothing unsaved");
        let r = s.execute("dataset.import", &json!({"csv": csv})).unwrap();
        assert_eq!((super::history(&s), r["imported"].clone()), (before, json!(2)), "{csv}");
    }
    // A file with one of the two rows replaces both data sets with one, which is a change.
    for csv in ["Name,#Shown\nAnn,true\n", "Name\nCy\n"] {
        super::select_with_a_step_to_redo(&mut s, &[t]);
        let undo = super::history(&s).0;
        s.execute("dataset.import", &json!({"csv": csv})).unwrap();
        assert_eq!(super::history(&s), (undo + 1, 0, true), "{csv}");
    }

    let xml = library(&[("Name", "textcontent")], &[("A", &[("Name", &["Ann"])]), ("B", &[("Name", &["Bob"])])]);
    s.execute("variable.loadLibrary", &json!({"xml": xml})).unwrap();
    super::select_with_a_step_to_redo(&mut s, &[t]);
    let before = super::history(&s);
    let r = s.execute("variable.loadLibrary", &json!({"xml": xml})).unwrap();
    assert_eq!((super::history(&s), r["names"].clone()), (before, json!(["A", "B"])));
    // A bound variable the library declares keeps its binding: loading it again changes nothing.
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    super::select_with_a_step_to_redo(&mut s, &[t]);
    let before = super::history(&s);
    s.execute("variable.loadLibrary", &json!({"xml": xml})).unwrap();
    assert_eq!(super::history(&s), before);
    assert_eq!(s.execute("variable.list", &json!({})).unwrap()["variables"][0]["bindings"], json!(1));
    // A library of other variables is a change: it deletes Name with its binding.
    let other = library(&[("Note", "textcontent")], &[("A", &[("Note", &["Hi"])])]);
    let undo = super::history(&s).0;
    s.execute("variable.loadLibrary", &json!({"xml": other})).unwrap();
    assert_eq!(super::history(&s), (undo + 1, 0, true));

    // While the variables are locked, a library without text variables still loads its data sets,
    // and loading it again changes nothing.
    let mut s = session();
    let t = text(&mut s, "Alice");
    s.execute("variable.lock", &json!({"locked": true})).unwrap();
    let xml = library(&[("Shown", "other")], &[("B", &[])]);
    s.execute("variable.loadLibrary", &json!({"xml": xml})).unwrap();
    super::select_with_a_step_to_redo(&mut s, &[t]);
    let before = super::history(&s);
    s.execute("variable.loadLibrary", &json!({"xml": xml})).unwrap();
    assert_eq!(super::history(&s), before);
}

/// A library that is refused leaves the document's variables and data sets as they were.
#[test]
fn a_refused_variable_library_deletes_nothing() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": "Old"}})).unwrap();
    let before = s.doc().unwrap().doc.variables.clone();
    let dup = library(&[("Name", "textcontent")], &[("B", &[]), ("B", &[])]);
    assert!(s.execute("variable.loadLibrary", &json!({"xml": dup})).is_err());
    assert_eq!(s.doc().unwrap().doc.variables, before);
}

/// A library's variables of other kinds are left out with their values, named in `skipped` and
/// in a warning that gives each kind once with its count; its text variables load.
#[test]
fn a_variable_library_leaves_out_variables_that_are_not_text() {
    let mut s = session();
    let xml = library(
        &[("Shown", "kindB"), ("Name", "textcontent"), ("Photo", "kindC"), ("Other", "kindB")],
        &[("A", &[("Shown", &["true"]), ("Name", &["Bravo"]), ("Photo", &["a.png"])])],
    );
    let r = s.execute("variable.loadLibrary", &json!({"xml": xml})).unwrap();
    assert_eq!(r["variables"], json!(["Name"]));
    assert_eq!(r["skipped"], json!([{"name": "Shown", "trait": "kindB"}, {"name": "Photo", "trait": "kindC"}, {"name": "Other", "trait": "kindB"}]));
    assert_eq!(r["warning"], "variables that aren't text were left out: kindB (2), kindC (1)");
    let vars = &s.doc().unwrap().doc.variables;
    assert_eq!(vars.variables.len(), 1);
    assert_eq!(vars.dataset("A").map(|d| d.values.len()), Some(1));
}

/// A library that isn't one, or that breaks its grammar or the caps, is refused, and the document
/// is left as it was.
#[test]
fn malformed_variable_libraries_are_refused() {
    let mut s = session();
    let ok = library(&[("Name", "textcontent")], &[("A", &[("Name", &["x"])])]);
    let deep = format!("{}<variableSets/>{}", "<a>".repeat(300), "</a>".repeat(300));
    let markup = "<!DOCTYPE svg [<!ENTITY deep \"<a><a></a></a>\">]><svg>&deep;<variableSets/></svg>".to_string();
    // A document type declaration in a comment before the real one, and markup reached through
    // another entity.
    let hidden = format!("<!-- <!DOCTYPE svg> -->{markup}");
    let nested = "<!DOCTYPE svg [<!ENTITY a \"&b;&b;\"><!ENTITY b 'x'>]><svg>&a;<variableSets/></svg>".to_string();
    let parameter = "<!DOCTYPE svg [<!ENTITY % p \"x\">]><svg><variableSets/></svg>".to_string();
    let element = "<!DOCTYPE svg [<!ELEMENT svg ANY>]><svg><variableSets/></svg>".to_string();
    // A `>` and a comment opener inside a quoted system identifier, before elements nested too deep.
    let quoted = format!("<!DOCTYPE svg SYSTEM \"x><!--\"><svg>{}<variableSets/>{}</svg>", "<a>".repeat(300), "</a>".repeat(300));
    let entities: String = (0..65).map(|i| format!("<!ENTITY e{i} \"x\">")).collect();
    let many_entities = format!("<!DOCTYPE svg [{entities}]><svg><variableSets/></svg>");
    // 1 KiB of text referenced 9,000 times.
    let expanding = format!("<!DOCTYPE svg [<!ENTITY big \"{}\">]><svg>{}<variableSets/></svg>", "x".repeat(1024), "&big;".repeat(9000));
    // References that expand to just under the cap, then a comment that takes the text past it.
    let then_literal = format!(
        "<!DOCTYPE svg [<!ENTITY big \"{}\">]><svg>{}<!--{}--><variableSets/></svg>",
        "x".repeat(1024),
        "&big;".repeat(8000),
        " ".repeat(300_000)
    );
    let cases = [
        ("not well-formed XML", "<variableSets><variableSet>".to_string()),
        ("no `variableSets` element", "<svg><variables/></svg>".to_string()),
        ("declares no variables", "<variableSets><variableSet><variables/></variableSet></variableSets>".to_string()),
        ("is declared twice", library(&[("Name", "textcontent"), ("Name", "textcontent")], &[])),
        ("has no `trait`", ok.replace(" trait=\"textcontent\"", "")),
        ("missing `varName`", ok.replace("varName=\"Name\"", "varName=\" \"")),
        ("missing `dataSetName`", ok.replace(" dataSetName=\"A\"", "")),
        ("data set `A` is declared twice", library(&[("Name", "textcontent")], &[("A", &[]), ("A", &[])])),
        ("which the library doesn't declare", library(&[("Name", "textcontent")], &[("A", &[("Nmae", &["x"])])])),
        ("holds two values for `Name`", library(&[("Name", "textcontent")], &[("A", &[("Name", &["x"]), ("Name", &["y"])])])),
        ("nests elements more than 256 deep", deep),
        ("something other than entities of plain text", markup),
        ("something other than entities of plain text", hidden),
        ("something other than entities of plain text", nested),
        ("something other than entities of plain text", parameter),
        ("something other than entities of plain text", element),
        ("nests elements more than 256 deep", quoted),
        ("more than 64 entities", many_entities),
        ("entities expand to more than", expanding),
        ("entities expand to more than", then_literal),
        ("larger than", format!("{ok}<!--{}-->", " ".repeat(8 << 20))),
    ];
    for (why, xml) in cases {
        let e = s.execute("variable.loadLibrary", &json!({"xml": xml})).unwrap_err().to_string();
        assert!(e.contains(why), "{why}: {e}");
    }
    let names: Vec<String> = (0..257).map(|i| format!("V{i}")).collect();
    let many: Vec<(&str, &str)> = names.iter().map(|n| (n.as_str(), "textcontent")).collect();
    let e = s.execute("variable.loadLibrary", &json!({"xml": library(&many, &[])})).unwrap_err().to_string();
    assert!(e.contains("more than 256 variables"), "{e}");
    // A Latin-1 é in a comment.
    let latin1 = [ok.as_bytes(), b"<!-- \xE9 -->"].concat();
    let e = s.execute("variable.loadLibrary", &json!({"dataBase64": vectorcraft_format::base64_encode(&latin1)})).unwrap_err().to_string();
    assert!(e.contains("not UTF-8"), "{e}");
    let e = s.execute("variable.loadLibrary", &json!({})).unwrap_err().to_string();
    assert!(e.contains("give `path`, `xml` or `dataBase64`"), "{e}");
    let vars = &s.doc().unwrap().doc.variables;
    assert!(vars.variables.is_empty() && vars.datasets.is_empty(), "a refused library loads nothing");
}

/// Entities whose values hold plain text expand to it, in attribute values too, and a reference in
/// a CDATA section stays as written.
#[test]
fn a_variable_library_expands_entities_of_plain_text() {
    let mut s = session();
    let xml = "<!DOCTYPE svg [<!ENTITY a \"x\"><!ENTITY b 'y \"z\"'>]><svg><variableSets><variableSet><variables>\
               <variable varName=\"Name\" trait=\"textcontent\"/></variables><sampleDataSets><sampleDataSet dataSetName=\"&b;\">\
               <Name><p>&a;-&b;-&a;<![CDATA[&a;]]></p></Name></sampleDataSet></sampleDataSets></variableSet></variableSets></svg>";
    s.execute("variable.loadLibrary", &json!({"xml": xml})).unwrap();
    let value = s.doc().unwrap().doc.variables.dataset("y \"z\"").and_then(|d| d.values.get("Name")).cloned();
    assert_eq!(value, Some(vectorcraft_doc::DataValue::Text("x-y \"z\"-x&a;".into())));
}

/// Save Variable Library writes what Load Variable Library reads back: the text variables in their
/// order, every data set by name and in order, and each text value as it was, with several lines,
/// markup characters, quotes, a carriage return and an empty value. Variables of other kinds are
/// left out with their values, named in `skipped` and in the warning. The library is written while
/// the variables are locked too, to a file with `path`.
#[test]
fn a_saved_variable_library_loads_back_the_text_variables_and_data_sets() {
    let mut s = session();
    let t = text(&mut s, "Alice");
    for (name, kind) in [("Name", "text"), ("Shown", "visibility"), ("Note", "text"), ("Photo", "image"), ("Badge", "visibility")] {
        s.execute("variable.define", &json!({"name": name, "kind": kind})).unwrap();
    }
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "First", "values": {"Name": "Bravo", "Note": "One\nTwo & <three>\r\n'four' \"five\"", "Shown": true}}))
        .unwrap();
    s.execute("dataset.new", &json!({"name": "Say \"hi\" & <go>\tnow", "values": {"Name": "", "Photo": "a.png"}})).unwrap();
    s.execute("dataset.new", &json!({"name": "Empty"})).unwrap();
    s.execute("variable.lock", &json!({"locked": true})).unwrap();
    let before = s.doc().unwrap().doc.variables.clone();

    let saved = s.execute("variable.saveLibrary", &json!({})).unwrap();
    assert_eq!(saved["variables"], json!(["Name", "Note"]));
    assert_eq!(saved["datasets"], json!(3));
    assert_eq!(
        saved["skipped"],
        json!([{"name": "Shown", "kind": "visibility"}, {"name": "Photo", "kind": "image"}, {"name": "Badge", "kind": "visibility"}])
    );
    assert_eq!(saved["warning"], "variables that aren't text were left out: visibility (2), image (1)");
    assert_eq!(s.doc().unwrap().doc.variables, before, "saving changes nothing");

    let mut other = session();
    let loaded = other.execute("variable.loadLibrary", &json!({"xml": saved["xml"]})).unwrap();
    assert_eq!((loaded["skipped"].clone(), loaded.get("warning")), (json!([]), None));
    let vars = &other.doc().unwrap().doc.variables;
    assert_eq!(vars.variables.iter().map(|v| (v.name.as_str(), v.kind.label())).collect::<Vec<_>>(), [("Name", "text"), ("Note", "text")]);
    let text_only: Vec<vectorcraft_doc::DataSet> = before
        .datasets
        .iter()
        .map(|d| {
            let mut d = d.clone();
            d.values.retain(|k, _| k == "Name" || k == "Note");
            d
        })
        .collect();
    assert_eq!(vars.datasets, text_only);

    let dir = Folder::new("vars-save-library");
    let path = dir.file("library.xml");
    let written = s.execute("variable.saveLibrary", &json!({"path": path})).unwrap();
    assert_eq!(written["path"], json!(path));
    assert!(written.get("xml").is_none());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), saved["xml"].as_str().unwrap());
    let mut from_file = session();
    from_file.execute("variable.loadLibrary", &json!({"path": path})).unwrap();
    assert_eq!(from_file.doc().unwrap().doc.variables.datasets, text_only);
}

/// The library is the XML of the public grammar: a document type declaration with the namespace
/// entities, `variableSets` › `variableSet` › `variables` with a `textcontent` variable per text
/// variable, and `sampleDataSets` with a `sampleDataSet` per data set, holding an element per text
/// variable it has a value for, with a `p` element per line. Markup characters, a carriage return,
/// and a tab or quote in a data set name are escaped. A document with text variables and no data
/// sets writes an empty `sampleDataSets`.
#[test]
fn a_saved_variable_library_has_the_library_layout() {
    let mut s = session();
    for (name, kind) in [("Name", "text"), ("Shown", "visibility"), ("Note", "text")] {
        s.execute("variable.define", &json!({"name": name, "kind": kind})).unwrap();
    }
    let head = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
                <!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\" [\n\
                \t<!ENTITY ns_vars \"http://ns.adobe.com/Variables/1.0/\">\n\
                \t<!ENTITY ns_custom \"http://ns.adobe.com/GenericCustomNamespace/1.0/\">\n\
                \t<!ENTITY ns_flows \"http://ns.adobe.com/Flows/1.0/\">\n\
                ]>\n\
                <svg>\n\
                <variableSets xmlns=\"&ns_vars;\">\n\
                \t<variableSet varSetName=\"binding1\" locked=\"none\">\n\
                \t\t<variables>\n\
                \t\t\t<variable varName=\"Name\" trait=\"textcontent\" category=\"&ns_flows;\"></variable>\n\
                \t\t\t<variable varName=\"Note\" trait=\"textcontent\" category=\"&ns_flows;\"></variable>\n\
                \t\t</variables>\n\
                \t\t<v:sampleDataSets xmlns:v=\"&ns_vars;\" xmlns=\"&ns_custom;\">\n";
    let tail = "\t\t</v:sampleDataSets>\n\t</variableSet>\n</variableSets>\n</svg>\n";
    let empty = s.execute("variable.saveLibrary", &json!({})).unwrap();
    assert_eq!(empty["xml"].as_str().unwrap(), format!("{head}{tail}"));
    assert_eq!(empty["datasets"], json!(0));

    s.execute("dataset.new", &json!({"name": "A &\t\"B\"", "values": {"Name": "x\r\ny < z", "Shown": false}})).unwrap();
    s.execute("dataset.new", &json!({"name": "C", "values": {"Note": ""}})).unwrap();
    let xml = s.execute("variable.saveLibrary", &json!({})).unwrap()["xml"].as_str().unwrap().to_string();
    let sets = "\t\t\t<v:sampleDataSet dataSetName=\"A &amp;&#9;&quot;B&quot;\">\n\
                \t\t\t\t<Name>\n\
                \t\t\t\t\t<p>x&#13;</p>\n\
                \t\t\t\t\t<p>y &lt; z</p>\n\
                \t\t\t\t</Name>\n\
                \t\t\t</v:sampleDataSet>\n\
                \t\t\t<v:sampleDataSet dataSetName=\"C\">\n\
                \t\t\t\t<Note>\n\
                \t\t\t\t\t<p></p>\n\
                \t\t\t\t</Note>\n\
                \t\t\t</v:sampleDataSet>\n";
    assert_eq!(xml, format!("{head}{sets}{tail}"));
}

/// Save Variable Library refuses a document without variables or without text variables, a text
/// variable whose name can't name an element, a data set name or text value with a character XML
/// can't hold, a name that Load Variable Library would read differently or refuse, and a library
/// past the caps that Load Variable Library reads. A refused save writes no file.
#[test]
fn save_variable_library_refuses_what_a_library_cant_hold() {
    let dir = Folder::new("vars-save-refused");
    let path = dir.file("library.xml");
    let refused = |s: &mut Session, why: &str| {
        let e = s.execute("variable.saveLibrary", &json!({"path": path})).unwrap_err().to_string();
        assert!(e.contains(why), "{why}: {e}");
        assert!(!std::path::Path::new(&path).exists(), "{why}: a file was written");
    };

    let mut s = session();
    refused(&mut s, "define a variable first");
    s.execute("variable.define", &json!({"name": "Shown", "kind": "visibility"})).unwrap();
    refused(&mut s, "holds text variables, and the document has none");

    for name in ["First Name", "1st", "a:b", "x/y", "-dash"] {
        let mut s = session();
        s.execute("variable.define", &json!({"name": name, "kind": "text"})).unwrap();
        refused(&mut s, &format!("variable `{name}` can't be saved in a variable library"));
    }
    for name in ["Ünïcode", "_x", "a.b-c_d", "名前"] {
        let mut s = session();
        s.execute("variable.define", &json!({"name": name, "kind": "text"})).unwrap();
        s.execute("dataset.new", &json!({"name": "A", "values": {name: "v"}})).unwrap();
        let xml = s.execute("variable.saveLibrary", &json!({})).unwrap()["xml"].clone();
        let mut other = session();
        other.execute("variable.loadLibrary", &json!({"xml": xml})).unwrap();
        assert_eq!(other.doc().unwrap().doc.variables.datasets, s.doc().unwrap().doc.variables.datasets, "{name}");
    }

    let mut s = session();
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": "bell \u{7}"}})).unwrap();
    refused(&mut s, "data set `A` holds a control character");
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "B\u{1}"})).unwrap();
    refused(&mut s, "holds a control character");

    // A document opened from a file can hold names that the commands don't make, and Load Variable
    // Library would read them differently or refuse them.
    let renamed = |dataset: Option<&str>, variable: Option<&str>| {
        let mut s = session();
        s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
        s.execute("variable.define", &json!({"name": "Other", "kind": "text"})).unwrap();
        s.execute("dataset.new", &json!({"name": "A"})).unwrap();
        s.execute("dataset.new", &json!({"name": "B"})).unwrap();
        let vars = &mut std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).variables;
        if let Some(name) = dataset {
            vars.datasets[1].name = name.to_string();
        }
        if let Some(name) = variable {
            vars.variables[1].name = name.to_string();
        }
        s
    };
    for name in [" B", "B\t", "B\n"] {
        refused(&mut renamed(Some(name), None), "starts or ends with white space");
    }
    refused(&mut renamed(Some(""), None), "missing `dataSetName`");
    refused(&mut renamed(Some(&"d".repeat(256)), None), "`dataSetName` is longer than 255 characters");
    refused(&mut renamed(Some("A"), None), "dataset `A` already exists");
    refused(&mut renamed(None, Some(&"v".repeat(256))), "`varName` is longer than 255 characters");
    refused(&mut renamed(None, Some("Name")), "variable `Name` is already defined");
    let mut s = renamed(Some(&"d".repeat(255)), Some(&"v".repeat(255)));
    let xml = s.execute("variable.saveLibrary", &json!({})).unwrap()["xml"].clone();
    let mut other = session();
    other.execute("variable.loadLibrary", &json!({"xml": xml})).unwrap();
    assert_eq!(other.doc().unwrap().doc.variables.datasets, s.doc().unwrap().doc.variables.datasets);

    let mut s = session();
    for i in 0..257 {
        s.execute("variable.define", &json!({"name": format!("V{i}"), "kind": "text"})).unwrap();
    }
    refused(&mut s, "more than 256 variables");
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    for i in 0..1001 {
        s.execute("dataset.new", &json!({"name": format!("D{i}")})).unwrap();
    }
    refused(&mut s, "more than 1000 data sets");
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": "x".repeat(9 << 20)}})).unwrap();
    refused(&mut s, "larger than 8388608 bytes");
}

/// A library near the size cap is saved only when Load Variable Library reads it back, after its
/// entity references expand: each text variable's `&ns_flows;`, two `&ns_vars;` and one
/// `&ns_custom;` grow by 20, 25 and 36 bytes.
#[test]
fn a_saved_variable_library_near_the_size_cap_loads_back() {
    let mut s = session();
    for i in 0..200 {
        s.execute("variable.define", &json!({"name": format!("V{i}"), "kind": "text"})).unwrap();
    }
    s.execute("dataset.new", &json!({"name": "A", "values": {"V0": ""}})).unwrap();
    let base = s.execute("variable.saveLibrary", &json!({})).unwrap()["xml"].as_str().unwrap().len();
    let room = (8 << 20) - (200 * 20 + 2 * 25 + 36);
    s.execute("dataset.set", &json!({"name": "A", "values": {"V0": "x".repeat(room - base + 1)}})).unwrap();
    let e = s.execute("variable.saveLibrary", &json!({})).unwrap_err().to_string();
    assert!(e.contains("larger than 8388608 bytes"), "{e}");
    s.execute("dataset.set", &json!({"name": "A", "values": {"V0": "x".repeat(room - base)}})).unwrap();
    let xml = s.execute("variable.saveLibrary", &json!({})).unwrap()["xml"].clone();
    assert_eq!(xml.as_str().map(str::len), Some(room));
    let mut other = session();
    other.execute("variable.loadLibrary", &json!({"xml": xml})).unwrap();
    assert_eq!(other.doc().unwrap().doc.variables.datasets, s.doc().unwrap().doc.variables.datasets);
    // The same library with the value grown to a file of exactly 8 MiB doesn't load back.
    let full = xml.as_str().unwrap().replacen("<p>x", &format!("<p>{}x", "x".repeat((8 << 20) - room)), 1);
    assert_eq!(full.len(), 8 << 20);
    let e = other.execute("variable.loadLibrary", &json!({"xml": full})).unwrap_err().to_string();
    assert!(e.contains("entities expand to more than"), "{e}");
}

/// While the variables are locked, a library that would delete or define a variable is refused, as
/// Delete Variable and New Variable are, and the document keeps its variables and data sets. That
/// covers a library without text variables loaded into a document with variables, and one that
/// adds a text variable to the document's. A library of the document's own text variables, and
/// one without text variables loaded into a document without variables, delete and define none:
/// they load, replacing the data sets.
#[test]
fn locked_variables_refuse_a_library_that_deletes_or_defines_variables() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": "Old"}})).unwrap();
    s.execute("variable.lock", &json!({"locked": true})).unwrap();
    let before = s.doc().unwrap().doc.variables.clone();
    for vars in [&[("Shown", "other")][..], &[("Name", "textcontent"), ("Note", "textcontent")][..]] {
        let e = s.execute("variable.loadLibrary", &json!({"xml": library(vars, &[("B", &[])])})).unwrap_err();
        assert!(matches!(&e, EngineError::Disabled(cmd, why) if cmd == "variable.loadLibrary" && why == "variables are locked"), "{e}");
        assert_eq!(s.doc().unwrap().doc.variables, before);
    }
    // A library of the document's own text variable deletes and defines none: it loads its data sets.
    let r = s.execute("variable.loadLibrary", &json!({"xml": library(&[("Name", "textcontent")], &[("B", &[])])})).unwrap();
    assert_eq!((r["deleted"].clone(), r["deletedDataSets"].clone()), (json!([]), json!(["A"])));

    let mut s = session();
    s.execute("dataset.new", &json!({"name": "A"})).unwrap();
    s.execute("variable.lock", &json!({"locked": true})).unwrap();
    let e = s.execute("variable.loadLibrary", &json!({"xml": library(&[("Name", "textcontent")], &[])})).unwrap_err();
    assert!(matches!(&e, EngineError::Disabled(_, why) if why == "variables are locked"), "{e}");
    let r = s.execute("variable.loadLibrary", &json!({"xml": library(&[("Shown", "other")], &[("B", &[])])})).unwrap();
    assert_eq!((r["deletedDataSets"].clone(), r["names"].clone()), (json!(["A"]), json!(["B"])));
    let vars = &s.doc().unwrap().doc.variables;
    assert!(vars.variables.is_empty() && vars.locked);
    assert_eq!(vars.datasets.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["B"]);
}

/// A graph variable's data set applies its CSV as `graph.setData` does: a right-axis series the row's data no longer
/// has is dropped, and a row with neither a label nor a value is not drawn as an empty slot.
#[test]
fn graph_variable_applies_csv_as_set_data() {
    let mut s = session();
    let create = json!({"type": "column", "x": 10, "y": 10, "width": 120, "height": 80, "csv": ",a,b\nQ1,10,1000\nQ2,5,500"});
    let id = s.execute("graph.create", &create).unwrap()["id"].as_u64().unwrap();
    let other = s.execute("graph.create", &create).unwrap()["id"].as_u64().unwrap();
    for g in [id, other] {
        s.execute("graph.setType", &json!({"id": g, "seriesIndexes": [1], "valueAxis": "right"})).unwrap();
    }
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("variable.makeGraphDynamic", &json!({})).unwrap();
    let name = s.execute("variable.list", &json!({})).unwrap()["variables"][0]["name"].as_str().unwrap().to_string();
    let spec = |s: &Session, g: u64| s.doc().unwrap().doc.node(NodeId(g)).unwrap().extras().graph.as_deref().unwrap().clone();
    // A dropped empty row wider than the others (`,,`) adds no blank series, so the right axis keeps nothing.
    for (row, csv) in [("Wide", ",a\nQ1,10\n,,\nQ2,5"), ("One", ",a\nQ1,10\nQ2,5"), ("Blank", ",a\nQ1,10\n,\nQ2,5")] {
        s.execute("dataset.new", &json!({"name": row, "values": {name.clone(): csv}})).unwrap();
        s.execute("dataset.select", &json!({"name": row})).unwrap();
        s.execute("graph.setData", &json!({"id": other, "csv": csv})).unwrap();
        let (merged, set) = (spec(&s, id), spec(&s, other));
        assert_eq!(merged.right_series, set.right_series, "{row}");
        assert_eq!(merged.categories, set.categories, "{row}");
        assert_eq!(merged.cells(), set.cells(), "{row}");
        assert!(merged.right_series.is_empty(), "{row}");
        assert_eq!(merged.cells(), [[Some(10.0)], [Some(5.0)]], "{row}");
    }
    assert!(spec(&s, id).right_series.is_empty());
    assert_eq!(spec(&s, id).categories, ["Q1", "Q2"]);
}

#[test]
fn graph_variable_applies_csv_and_captures() {
    let mut s = session();
    let id = s
        .execute(
            "graph.create",
            &json!({
                "type": "column",
                "x": 10,
                "y": 10,
                "width": 120,
                "height": 80,
                "csv": ",Sales\nA,1\nB,2"
            }),
        )
        .unwrap()["id"]
        .as_u64()
        .unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("variable.makeGraphDynamic", &json!({})).unwrap();
    let name = s.execute("variable.list", &json!({})).unwrap()["variables"][0]["name"].as_str().unwrap().to_string();
    assert_eq!(s.execute("variable.list", &json!({})).unwrap()["variables"][0]["kind"], "graph");
    s.execute("dataset.new", &json!({"name": "Alt", "values": {name.clone(): ",Sales\nA,10\nB,20"}})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": "Alt"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(1), Some(0)));
    let csv = s.doc().unwrap().doc.variable_value(NodeId(id), vectorcraft_doc::VariableKind::Graph).unwrap();
    match csv {
        vectorcraft_doc::DataValue::Graph(c) => assert!(c.contains("10") && c.contains("20"), "{c}"),
        other => panic!("expected graph csv, got {other:?}"),
    }
    // Binding a non-graph is refused.
    let r = rect(&mut s);
    assert!(s.execute("variable.bind", &json!({"variable": name, "ids": [r.0]})).is_err());
}

/// A Variables command that leaves the document as it was (binding art to the variable it already
/// has, unbinding art that has none, deleting a variable or data set that does not exist, writing a
/// row's own values back, applying the active data set again) records no undo step: the step to
/// redo stays, and the document stays unmodified.
#[test]
fn a_variables_command_that_changes_nothing_records_no_undo_step() {
    let dir = Folder::new("vars-noop");
    let red_path = dir.file("red.png");
    write(&red_path, &png(8, 8, RED));
    let mut s = session();
    let t = text(&mut s, "Alice");
    let r = rect(&mut s);
    let img = place(&mut s, &red_path);
    let other = rect(&mut s);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.define", &json!({"name": "Photo", "kind": "image"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [t.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Photo", "ids": [img.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": "Bob", "Show": false, "Photo": red_path}})).unwrap();
    s.execute("dataset.select", &json!({"name": "A"})).unwrap();
    let a = json!({"name": "A", "values": {"Name": "Bob", "Show": false, "Photo": red_path}});
    let calls = [
        ("variable.bind", json!({"variable": "Show", "ids": [r.0]})),
        ("variable.unbind", json!({"ids": [other.0]})),
        ("variable.unbind", json!({"variable": "Name", "ids": [r.0]})),
        ("variable.delete", json!({"names": ["Nope"]})),
        ("dataset.delete", json!({"names": ["Nope"]})),
        ("dataset.set", a),
        ("dataset.update", json!({})),
        ("dataset.update", json!({"all": true})),
        ("dataset.select", json!({"name": "A"})),
        ("dataset.next", json!({})),
        ("dataset.prev", json!({})),
    ];
    for (cmd, p) in calls {
        super::select_with_a_step_to_redo(&mut s, &[other]);
        let before = super::history(&s);
        assert_eq!((before.1, before.2), (1, false), "a step to redo, and nothing unsaved");
        s.execute(cmd, &p).unwrap();
        assert_eq!(super::history(&s), before, "{cmd} {p}");
    }
    assert_eq!(plain(&s, t), "Bob");
    // Applying the row again still deselects the art it leaves hidden, without an undo step.
    s.execute("select.set", &json!({"ids": [r.0, other.0]})).unwrap();
    assert!(s.doc().unwrap().selection.objects.contains(&r), "the hidden rectangle is selected");
    let before = super::history(&s);
    s.execute("dataset.select", &json!({"name": "A"})).unwrap();
    assert_eq!((super::history(&s), s.doc().unwrap().selection.objects.clone()), (before, vec![other]));
    // A command that changes something is still one undo step, and it clears Redo.
    super::select_with_a_step_to_redo(&mut s, &[other]);
    let undo = super::history(&s).0;
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [other.0]})).unwrap();
    assert_eq!(super::history(&s), (undo + 1, 0, true));
    s.execute("dataset.new", &json!({"name": "B", "values": {"Name": "Cy"}})).unwrap();
    s.doc_mut().unwrap().mark_saved();
    let undo = super::history(&s).0;
    s.execute("dataset.next", &json!({})).unwrap();
    assert_eq!((super::history(&s), plain(&s, t)), ((undo + 1, 0, true), "Cy".to_string()));
}

/// A Variables command that changes nothing, run while the Type tool is typing, still ends the
/// typing session as one that changes something does: the typing is its own undo step, and the
/// next keystroke starts a new one.
#[test]
fn a_variables_command_that_changes_nothing_ends_the_typing() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut s = session();
    let v = ViewInfo::default();
    let t = text(&mut s, "Hello");
    let r = s.execute("shape.rectangle", &json!({"x": 200, "y": 200, "width": 50, "height": 40})).unwrap();
    let r = NodeId(r["id"].as_u64().unwrap());
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Show", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "A", "values": {"Show": true}})).unwrap();
    s.execute("dataset.select", &json!({"name": "A"})).unwrap();
    for (cmd, p) in [("variable.bind", json!({"variable": "Show", "ids": [r.0]})), ("dataset.select", json!({"name": "A"}))] {
        s.select_tool("type", v).unwrap();
        let b = s.doc().unwrap().doc.bounds_of(&[t], false).unwrap();
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, b.x1 - 1.0, b.center().y), v).unwrap();
        }
        s.tool_text("!", v).unwrap();
        assert!(crate::cmd::typing_in_progress(&s), "{cmd}");
        let steps = s.doc().unwrap().history.undo.len();
        s.execute(cmd, &p).unwrap();
        assert!(!crate::cmd::typing_in_progress(&s), "{cmd}");
        let labels: Vec<String> = s.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect();
        assert_eq!((labels.len(), labels.last().map(String::as_str)), (steps + 1, Some("Typing")), "{cmd}");
    }
}

/// Applying the active data set again to threaded type that already holds the row's story records
/// no undo step.
#[test]
fn applying_a_story_row_again_records_no_undo_step() {
    let (mut s, a, b, _) = threaded(200);
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Name", "ids": [a.0]})).unwrap();
    s.execute("dataset.capture", &json!({"name": "Orig"})).unwrap();
    s.execute("dataset.select", &json!({"name": "Orig"})).unwrap();
    assert!(!plain(&s, b).is_empty());
    super::select_with_a_step_to_redo(&mut s, &[a]);
    let before = super::history(&s);
    s.execute("dataset.select", &json!({"name": "Orig"})).unwrap();
    assert_eq!(super::history(&s), before);
}

/// Applying the active data set again to a graph whose data already matches the row records no
/// undo step, and a row with other data still rebuilds the graph in one.
#[test]
fn applying_a_graph_row_again_records_no_undo_step() {
    let mut s = session();
    let graph = json!({"type": "column", "x": 10, "y": 10, "width": 120, "height": 80, "csv": ",Sales\nA,1\nB,2"});
    let id = s.execute("graph.create", &graph).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [id]})).unwrap();
    s.execute("variable.makeGraphDynamic", &json!({})).unwrap();
    let name = s.execute("variable.list", &json!({})).unwrap()["variables"][0]["name"].as_str().unwrap().to_string();
    s.execute("dataset.new", &json!({"name": "A", "values": {name.clone(): ",Sales\nA,10\nB,20"}})).unwrap();
    s.execute("dataset.new", &json!({"name": "B", "values": {name: ",Sales\nA,3\nB,4"}})).unwrap();
    s.execute("dataset.select", &json!({"name": "A"})).unwrap();
    super::select_with_a_step_to_redo(&mut s, &[NodeId(id)]);
    let before = super::history(&s);
    let v = s.execute("dataset.select", &json!({"name": "A"})).unwrap();
    assert_eq!((v["applied"].as_u64(), super::history(&s)), (Some(1), before));
    s.execute("dataset.select", &json!({"name": "B"})).unwrap();
    assert_eq!(super::history(&s), (before.0 + 1, 0, true));
}
