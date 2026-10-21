//! Variables (data merge): definitions with kinds, bindings to objects, datasets
//! applied in one undo step, and validation that rejects instead of reinterpreting.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use super::tests_links::{BLUE, Folder, RED, bounds, image, place, png, write};
use super::*;

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
    s.execute("variable.bind", &json!({"variable": "Title", "ids": [a.0]})).unwrap();
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
    s.execute("dataset.import", &json!({"csv": "name,Name,Title\nA,Ann,Boss\nB,Bob,Chief\n"})).unwrap();
    let warning = |row: &str| format!("nothing in data set {row} was applied: no object is bound to a variable it has a value for");
    let v = s.execute("dataset.select", &json!({"name": "A"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64(), &v["warning"]), (Some(0), Some(0), &json!(warning("A"))), "{v}");
    let v = s.execute("dataset.next", &json!({})).unwrap();
    assert_eq!(v["warning"], json!(warning("B")), "{v}");
    let v = s.execute("dataset.prev", &json!({})).unwrap();
    assert_eq!(v["warning"], json!(warning("A")), "{v}");
    assert_eq!(plain(&s, t), "x");

    // A bound variable that the row has no value for still applies nothing.
    s.execute("variable.define", &json!({"name": "Other", "kind": "text"})).unwrap();
    s.execute("variable.bind", &json!({"variable": "Other", "ids": [t.0]})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": "B"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64(), &v["warning"]), (Some(0), Some(1), &json!(warning("B"))), "{v}");

    // Bound to a variable with a value, the row applies it and gives no warning.
    s.execute("variable.bind", &json!({"variable": "Title", "ids": [t.0]})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": "A"})).unwrap();
    assert_eq!(v["applied"].as_u64(), Some(1), "{v}");
    assert!(v.get("warning").is_none(), "{v}");
    assert_eq!(plain(&s, t), "Boss");

    // Locked art is skipped, and its binding still counts as bound.
    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": "B"})).unwrap();
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
    s.execute("variable.define", &json!({"name": "Price", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Badge", "kind": "visibility"})).unwrap();

    // A text object driven by two text variables keeps both.
    assert_eq!(s.execute("variable.bind", &json!({"variable": "Title", "ids": [t.0]})).unwrap()["bound"], 1);
    let v = s.execute("variable.bind", &json!({"variable": "Price", "ids": [t.0]})).unwrap();
    assert_eq!((v["bound"].clone(), v["already"].clone()), (json!(1), json!([])), "Title is untouched");
    // A rectangle driven by a visibility variable as well as nothing else yet.
    s.execute("variable.bind", &json!({"variable": "Badge", "ids": [r.0]})).unwrap();
    s.execute("dataset.new", &json!({"name": "D", "values": {"Title": "Hat", "Price": "9.99", "Badge": false}})).unwrap();

    let v = s.execute("dataset.select", &json!({"name": "D"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(3), Some(0)), "every binding applied");
    // Both text variables are counted; the later one in definition order is what is left.
    assert_eq!(plain(&s, t), "9.99");
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible);

    let vars = s.execute("variable.list", &json!({})).unwrap();
    let of = |n: &str| vars["variables"].as_array().unwrap().iter().find(|v| v["name"] == n).unwrap()["bindings"].as_u64();
    assert_eq!((of("Title"), of("Price"), of("Badge")), (Some(1), Some(1), Some(1)));

    // Binding again to one it already has is not a change, and says so.
    let v = s.execute("variable.bind", &json!({"variable": "Price", "ids": [t.0]})).unwrap();
    assert_eq!((v["bound"].clone(), v["already"].clone()), (json!(0), json!([t.0])));
    // Unbinding one variable leaves the object's others alone.
    assert_eq!(s.execute("variable.unbind", &json!({"ids": [t.0], "variable": "Price"})).unwrap()["unbound"], 1);
    s.execute("edit.undo", &json!({})).unwrap();
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"][1]["bindings"], 1, "the other binding is still there");
    assert!(s.execute("variable.unbind", &json!({"ids": [t.0], "variable": "Ghost"})).is_err());

    // Deleting a variable drops only its own binding.
    s.execute("variable.delete", &json!({"names": ["Price"]})).unwrap();
    let vars = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(vars["variables"][0]["bindings"], 1, "Title still drives the text");
}

/// `unbound` counts what was actually dropped, like `variable.delete` does.
#[test]
fn unbind_reports_what_it_actually_removed() {
    let mut s = session();
    let t = text(&mut s, "x");
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Price", "kind": "text"})).unwrap();
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

/// Locked art is the document's to rewrite only while it isn't locked.
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

    // The same object twice takes a number rather than colliding with the first variable.
    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
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
/// while binding, unbinding and the dataset commands still run.
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
    s.execute("dataset.import", &json!({"csv": "name,Name\nCarol,Carol\n"})).unwrap();
    s.execute("dataset.select", &json!({"name": "Carol"})).unwrap();
    assert_eq!(plain(&s, t), "Carol");

    // Unlocking lets a variable be defined again; `locked` sets the state rather than toggling.
    assert_eq!(s.execute("variable.lock", &json!({"locked": false})).unwrap()["locked"], false);
    assert_eq!(s.execute("variable.lock", &json!({"locked": false})).unwrap()["locked"], false);
    s.execute("variable.define", &json!({"name": "Other", "kind": "text"})).unwrap();
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

/// A variable whose object is locked reports no value, so Update keeps the row's value for it
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
/// and leaves the layer hidden: showing the layer shows that row's text. A layer that is locked
/// as well as hidden is still left alone.
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

    // Locked and hidden: the row skips both objects and the type keeps its text.
    s.execute("layer.setProps", &json!({"id": layer.0, "visible": false, "locked": true})).unwrap();
    let v = s.execute("dataset.select", &json!({"name": "A"})).unwrap();
    assert_eq!((v["applied"].as_u64(), v["skipped"].as_u64()), (Some(0), Some(2)), "{v}");
    assert_eq!(plain(&s, t), "Bob", "the type on the locked layer keeps its text");
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible);
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

/// Of two visibility variables on one object, the one defined later decides whether it shows,
/// as with two text variables.
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
    s.execute("dataset.delete", &json!({"names": ["A"]})).unwrap();
    s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    let sets = &s.doc().unwrap().doc.variables.datasets;
    assert_eq!(sets.len(), 1);
    assert_eq!(sets[0].values.get("Name"), Some(&vectorcraft_doc::DataValue::Text(value.into())));
    assert!(s.execute("dataset.import", &json!({"csv": "name,Name\nX,\"open"})).is_err(), "unclosed quote");
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
    s.execute("dataset.delete", &json!({"names": ["A"]})).unwrap();
    s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    s.execute("dataset.import", &json!({"csv": "name,Name\nB,  typed  \nC,\" quoted \"  \n"})).unwrap();
    let value = |name: &str| s.doc().unwrap().doc.variables.dataset(name).and_then(|d| d.values.get("Name")).cloned();
    let text_value = |v: &str| Some(vectorcraft_doc::DataValue::Text(v.into()));
    assert_eq!(value("A"), text_value("  padded "));
    assert_eq!(value("B"), text_value("typed"));
    assert_eq!(value("C"), text_value(" quoted "));
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
    let tsv = "name\tTitle\tShow\r\nA\tTwo words\ttrue\r\nB\t\"a, b\tc\"\tfalse\r\n";
    let imp = s.execute("dataset.import", &json!({"csv": tsv})).unwrap();
    assert_eq!(imp["names"], json!(["A", "B"]));
    s.execute("dataset.import", &json!({"csv": "name,Title\nC,one\ttwo"})).unwrap();
    s.execute("dataset.import", &json!({"csv": "name\t\"Last, First\"\nD\tSmith, Jo"})).unwrap();
    let value = |name: &str, var: &str| s.doc().unwrap().doc.variables.dataset(name).and_then(|d| d.values.get(var)).cloned();
    let text_value = |v: &str| Some(vectorcraft_doc::DataValue::Text(v.into()));
    assert_eq!(value("A", "Title"), text_value("Two words"));
    assert_eq!(value("A", "Show"), Some(vectorcraft_doc::DataValue::Visible(true)));
    assert_eq!(value("B", "Title"), text_value("a, b\tc"));
    assert_eq!(value("B", "Show"), Some(vectorcraft_doc::DataValue::Visible(false)));
    assert_eq!(value("C", "Title"), text_value("one\ttwo"));
    assert_eq!(value("D", "Last, First"), text_value("Smith, Jo"));
    let unclosed = s.execute("dataset.import", &json!({"csv": "name\tTitle\t\"Show\nA\tx\ttrue"})).unwrap_err().to_string();
    assert!(unclosed.contains("unclosed quote"), "{unclosed}");
}

/// Without a `name` column every header cell is a variable, and the rows take the first free
/// names Data Set 1, 2 and so on. A first header cell `name`, in any case, still holds the names.
#[test]
fn datasets_csv_without_a_name_column_numbers_the_rows() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "Data Set 2", "values": {"Title": "kept"}})).unwrap();
    let imp = s.execute("dataset.import", &json!({"csv": "Title\nx\ny\n"})).unwrap();
    assert_eq!((imp["imported"].as_u64(), imp["updated"].as_u64()), (Some(2), Some(0)));
    assert_eq!(imp["names"], json!(["Data Set 1", "Data Set 3"]));
    let value = |s: &Session, name: &str| s.doc().unwrap().doc.variables.dataset(name).and_then(|d| d.values.get("Title")).cloned();
    let text_value = |v: &str| Some(vectorcraft_doc::DataValue::Text(v.into()));
    assert_eq!(value(&s, "Data Set 1"), text_value("x"));
    assert_eq!(value(&s, "Data Set 2"), text_value("kept"));
    assert_eq!(value(&s, "Data Set 3"), text_value("y"));
    let named = s.execute("dataset.import", &json!({"csv": "NAME,Title\nData Set 1,z"})).unwrap();
    assert_eq!((named["imported"].as_u64(), named["updated"].as_u64()), (Some(0), Some(1)));
    assert_eq!(value(&s, "Data Set 1"), text_value("z"));
}

/// An error about a row's values or name gives the file line the row starts on, counting blank
/// lines and line breaks inside quoted cells.
#[test]
fn datasets_csv_row_errors_name_the_file_line() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.define", &json!({"name": "Show", "kind": "visibility"})).unwrap();
    let error = |s: &mut Session, csv: &str| s.execute("dataset.import", &json!({"csv": csv})).unwrap_err().to_string();
    let blank_line = error(&mut s, "name,Show\n\nX,maybe");
    assert!(blank_line.contains("line 3:"), "{blank_line}");
    let line_break = error(&mut s, "name,Title,Show\nA,\"one\ntwo\",true\nB,x,maybe");
    assert!(line_break.contains("line 4:"), "{line_break}");
    let twice = error(&mut s, "name,Title\nA,x\n\nA,y");
    assert!(twice.contains("line 4:"), "{twice}");
    let long = error(&mut s, &format!("name,Title\n\n{},x", "n".repeat(256)));
    assert!(long.contains("line 3:") && long.contains("longer than 255"), "{long}");
}

/// A header cell that names no variable defines one, unbound, in the import's undo step: `#`
/// defines a visibility variable, `@` defines an image variable, and a cell without a prefix
/// defines a text variable. Importing the file again finds them; a prefix of the other kind, a
/// bare prefix and a long name fail and define nothing.
#[test]
fn datasets_csv_header_prefixes_define_variables() {
    let mut s = session();
    let csv = "name,#Shown,@Photo,Title\nA,true,a.png,x";
    let imp = s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    assert_eq!(imp["defined"], json!(["Shown", "Photo", "Title"]));
    assert_eq!((imp["imported"].as_u64(), imp["names"].clone()), (Some(1), json!(["A"])));
    let listed = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(
        listed["variables"],
        json!([
            {"name": "Shown", "kind": "visibility", "bindings": 0},
            {"name": "Photo", "kind": "image", "bindings": 0},
            {"name": "Title", "kind": "text", "bindings": 0},
        ])
    );
    let value = |s: &Session, var: &str| s.doc().unwrap().doc.variables.dataset("A").and_then(|d| d.values.get(var)).cloned();
    assert_eq!(value(&s, "Shown"), Some(vectorcraft_doc::DataValue::Visible(true)));
    assert_eq!(value(&s, "Photo"), Some(vectorcraft_doc::DataValue::Image("a.png".into())));
    assert_eq!(value(&s, "Title"), Some(vectorcraft_doc::DataValue::Text("x".into())));

    let again = s.execute("dataset.import", &json!({"csv": "name\t#Shown\t@Photo\tTitle\nA\tfalse\tb.png\ty"})).unwrap();
    assert_eq!((again["updated"].as_u64(), again["defined"].clone()), (Some(1), json!([])));
    assert_eq!(value(&s, "Shown"), Some(vectorcraft_doc::DataValue::Visible(false)));

    let error = |s: &mut Session, csv: &str| s.execute("dataset.import", &json!({"csv": csv})).unwrap_err().to_string();
    let other_kind = error(&mut s, "name,New,#Title\nB,n,true");
    assert!(other_kind.contains("column 3:") && other_kind.contains("text variable"), "{other_kind}");
    let twice = error(&mut s, "Fresh,@Fresh\nn,f.png");
    assert!(twice.contains("column 2:") && twice.contains("text variable"), "{twice}");
    let bare = error(&mut s, "name,#\nB,true");
    assert!(bare.contains("column 2:") && bare.contains("names no variable"), "{bare}");
    let long = error(&mut s, &format!("name,@{}\nB,b.png", "n".repeat(256)));
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
    for (name, kind) in [("#Shown", "text"), ("@Photo", "visibility"), ("%Chart", "graph"), ("Shown", "visibility")] {
        s.execute("variable.define", &json!({"name": name, "kind": kind})).unwrap();
    }
    let values = json!({"#Shown": "hello", "@Photo": true, "%Chart": ",Q\nX,3", "Shown": false});
    s.execute("dataset.new", &json!({"name": "A", "values": values})).unwrap();
    let csv = s.execute("dataset.export", &json!({})).unwrap()["csv"].as_str().unwrap().to_string();
    assert!(csv.starts_with("name,#Shown,@Photo,%Chart,Shown\n"), "{csv}");
    s.execute("dataset.delete", &json!({"names": ["A"]})).unwrap();
    let imp = s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    assert_eq!((imp["imported"].as_u64(), imp["defined"].clone()), (Some(1), json!([])));
    let value = |var: &str| s.doc().unwrap().doc.variables.dataset("A").and_then(|d| d.values.get(var)).cloned();
    assert_eq!(value("#Shown"), Some(vectorcraft_doc::DataValue::Text("hello".into())));
    assert_eq!(value("@Photo"), Some(vectorcraft_doc::DataValue::Visible(true)));
    assert_eq!(value("%Chart"), Some(vectorcraft_doc::DataValue::Graph(",Q\nX,3".into())));
    assert_eq!(value("Shown"), Some(vectorcraft_doc::DataValue::Visible(false)));
    assert_eq!(s.doc().unwrap().doc.variables.variables.len(), 4);
}

/// A `%` header cell that isn't a variable's name (graph data files) is refused with its column,
/// and the import defines nothing.
#[test]
fn datasets_csv_refuses_graph_data_file_columns() {
    let mut s = session();
    let error = |s: &mut Session, csv: &str| s.execute("dataset.import", &json!({"csv": csv})).unwrap_err().to_string();
    let named = error(&mut s, "name,Title,%Sales\nA,x,sales.txt");
    assert!(named.contains("column 3:") && named.contains("`%Sales`"), "{named}");
    let unnamed = error(&mut s, "%Sales\tTitle\nsales.txt\tx");
    assert!(unnamed.contains("column 1:") && unnamed.contains("`%Sales`"), "{unnamed}");
    let vars = &s.doc().unwrap().doc.variables;
    assert!(vars.variables.is_empty() && vars.datasets.is_empty());
}

/// While the variables are locked, an import is refused when a header cell would define a
/// variable, as New Variable is, and a file whose headers all name variables imports.
#[test]
fn locked_variables_refuse_header_cells_that_define_variables() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Shown", "kind": "visibility"})).unwrap();
    s.execute("variable.define", &json!({"name": "Title", "kind": "text"})).unwrap();
    s.execute("variable.lock", &json!({"locked": true})).unwrap();
    for csv in ["name,#Shown,Title,New\nA,true,x,n", "@Photo\nb.png", "name,#Fresh\nA,true"] {
        let e = s.execute("dataset.import", &json!({"csv": csv})).unwrap_err();
        assert!(matches!(&e, EngineError::Disabled(cmd, why) if cmd == "dataset.import" && why == "variables are locked"), "{csv}: {e}");
    }
    let vars = &s.doc().unwrap().doc.variables;
    assert!(vars.variables.len() == 2 && vars.datasets.is_empty(), "a refused import defines and imports nothing");

    let imp = s.execute("dataset.import", &json!({"csv": "name,#Shown,Title\nA,true,x"})).unwrap();
    assert_eq!((imp["imported"].as_u64(), imp["defined"].clone()), (Some(1), json!([])));
    s.execute("variable.lock", &json!({"locked": false})).unwrap();
    let imp = s.execute("dataset.import", &json!({"csv": "name,New\nB,n"})).unwrap();
    assert_eq!(imp["defined"], json!(["New"]));
}

/// Import / export datasets as CSV: header `name` then variables; round-trip keeps values.
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
    assert!(csv.starts_with("name,Name,Show,Photo\n"), "{csv}");
    assert!(csv.contains("Alice") && csv.contains("Bob") && csv.contains("false"), "{csv}");

    // Clear rows, import again, apply B.
    s.execute("dataset.delete", &json!({"names": ["A", "B"]})).unwrap();
    let imp = s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    assert_eq!((imp["imported"].as_u64(), imp["updated"].as_u64()), (Some(2), Some(0)));
    s.execute("dataset.select", &json!({"name": "B"})).unwrap();
    assert_eq!(plain(&s, t), "Bob");
    assert!(!s.doc().unwrap().doc.node(r).unwrap().visible);
    assert_eq!(image(&s, img).link.as_ref().map(|l| l.path.as_str()), Some(blue_path.as_str()));

    // Re-import updates in place and defines nothing; a bad visibility value is an error.
    let again = s.execute("dataset.import", &json!({"csv": csv})).unwrap();
    assert_eq!((again["imported"].as_u64(), again["updated"].as_u64()), (Some(0), Some(2)));
    assert_eq!(again["defined"], json!([]));
    assert!(s.execute("dataset.import", &json!({"csv": "name,Show\nX,maybe"})).is_err());

    let out = dir.file("sets.csv");
    s.execute("dataset.export", &json!({"path": out})).unwrap();
    let from_file = s.execute("dataset.import", &json!({"path": out})).unwrap();
    assert_eq!(from_file["updated"].as_u64(), Some(2));

    // One undo undoes the last import's replacements.
    s.execute("edit.undo", &json!({})).unwrap();
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
    assert_eq!((from_base64["deleted"].clone(), from_base64["deletedDataSets"].clone()), (json!(["Name", "Note"]), json!(["Row 1", "Row 2"])));
    assert_eq!(s.doc().unwrap().doc.variables.datasets.len(), 2);
}

/// Load Variable Library replaces the document's variables and data sets: every variable goes with
/// its bindings and values, a text variable of the same name included, and every data set goes,
/// the active one included. The highlight of a variable that went highlights nothing. Undo brings
/// back the variables, bindings, data sets and active data set as they were.
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
    assert_eq!(loaded["deleted"], json!(["Name", "Title", "Kept"]));
    assert_eq!((loaded["deletedDataSets"].clone(), loaded["names"].clone()), (json!(["A", "Other"]), json!(["A"])));
    let listed = s.execute("variable.list", &json!({})).unwrap();
    assert_eq!(listed["variables"], json!([{"name": "Name", "kind": "text", "bindings": 0}, {"name": "Title", "kind": "text", "bindings": 0}]));
    let sets = s.execute("dataset.list", &json!({})).unwrap();
    assert_eq!(sets["datasets"].as_array().map(Vec::len), Some(1));
    assert_eq!(sets["datasets"][0]["values"], json!({"Name": {"text": "New"}, "Title": {"text": "Heading"}}));
    assert_eq!(sets["active"], Value::Null);
    assert!(s.doc().unwrap().doc.variables.bindings.is_empty(), "the bindings went with the variables");
    assert!(s.doc().unwrap().variables_highlight.is_none(), "the highlighted variable went");
    assert_eq!(plain(&s, t), "Stays", "loading applies nothing");

    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.variables, before, "one undo step brings everything back");
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

/// While the variables are locked, a library that would delete or define a variable is refused, as
/// Delete Variable and New Variable are, and the document keeps its variables and data sets. That
/// covers a library whose only variable is a text variable the document already has, and one
/// without text variables loaded into a document with variables. A library without text variables
/// loads into a document without variables, replacing its data sets.
#[test]
fn locked_variables_refuse_a_library_that_deletes_or_defines_variables() {
    let mut s = session();
    s.execute("variable.define", &json!({"name": "Name", "kind": "text"})).unwrap();
    s.execute("dataset.new", &json!({"name": "A", "values": {"Name": "Old"}})).unwrap();
    s.execute("variable.lock", &json!({"locked": true})).unwrap();
    let before = s.doc().unwrap().doc.variables.clone();
    for vars in [&[("Name", "textcontent")][..], &[("Shown", "other")][..]] {
        let e = s.execute("variable.loadLibrary", &json!({"xml": library(vars, &[("B", &[])])})).unwrap_err();
        assert!(matches!(&e, EngineError::Disabled(cmd, why) if cmd == "variable.loadLibrary" && why == "variables are locked"), "{e}");
        assert_eq!(s.doc().unwrap().doc.variables, before);
    }

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
