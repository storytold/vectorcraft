//! The split command modules (`cmd/{appearance,gradient,stroke,style,swatch}.rs`) and the shared
//! helpers in `cmd/mod.rs` (`leaf_targets`, `paint_targets`, `unique_name`).

use serde_json::json;

use super::*;
use crate::cmd::{leaf_targets, paint_targets, unique_name};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn id_of(v: &serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

#[test]
fn moved_commands_keep_their_ids() {
    let ids = [
        "appearance.addFill",
        "appearance.addStroke",
        "appearance.clear",
        "appearance.reduceToBasic",
        "appearance.setItem",
        "appearance.removeItem",
        "appearance.addEffect",
        "appearance.duplicateItem",
        "appearance.moveItem",
        "appearance.copyFrom",
        "paint.editGradient",
        "paint.setGradientGeom",
        "stroke.set",
        "stroke.setAdvanced",
        "graphicStyle.apply",
        "graphicStyle.new",
        "graphicStyle.delete",
        "graphicStyle.duplicate",
        "swatch.new",
        "swatch.delete",
        "swatch.newGroup",
        "swatch.duplicate",
        "swatch.sortByName",
    ];
    let pos = |id: &str| command_specs().iter().position(|c| c.id == id).unwrap_or_else(|| panic!("{id}"));
    let (paint_end, after_paint) = (pos("transparency.set"), pos("transparency.makeOpacityMask"));
    for id in ids {
        assert_eq!(command_specs().iter().filter(|c| c.id == id).count(), 1, "{id}");
        // Registered right after `paint` (where they lived), so the command palette, which lists
        // the first matches in registry order, still shows them (e.g. Stroke Options for "stroke").
        assert!((paint_end..after_paint).contains(&pos(id)), "{id}");
    }
}

#[test]
fn unique_name_appends_the_first_free_number() {
    let taken = ["A", "A 2", "A 3"];
    assert_eq!(unique_name("B", |n| taken.contains(&n)), "B");
    assert_eq!(unique_name("A", |n| taken.contains(&n)), "A 4");
    assert_eq!(unique_name("A 2", |n| taken.contains(&n)), "A 2 2");
}

#[test]
fn leaf_targets_expand_groups_and_stop_at_compound_paths() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0), rect(&mut s, 30.0));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    assert_eq!(leaf_targets(&s, &[g]).unwrap(), vec![a, b]);
    let (c, d) = (rect(&mut s, 60.0), rect(&mut s, 90.0));
    s.execute("select.set", &json!({"ids": [c.0, d.0]})).unwrap();
    let comp = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    assert_eq!(leaf_targets(&s, &[comp]).unwrap(), vec![comp]);
    // A member stands for its compound path, listed once (#3692).
    let members: Vec<NodeId> = s.doc().unwrap().doc.node(comp).unwrap().children().into_iter().flatten().map(|m| m.id).collect();
    assert_eq!(members.len(), 2);
    assert_eq!(leaf_targets(&s, &[members[0]]).unwrap(), vec![comp]);
    assert_eq!(leaf_targets(&s, &[members[0], comp, members[1]]).unwrap(), vec![comp]);
    assert_eq!(leaf_targets(&s, &[members[1], members[0]]).unwrap(), vec![comp]);
    // `paint_targets` reads `ids`, then `id`, then the selection.
    assert_eq!(paint_targets(&s, &json!({"ids": [g.0]})).unwrap(), vec![a, b]);
    assert_eq!(paint_targets(&s, &json!({"id": comp.0})).unwrap(), vec![comp]);
    s.execute("select.set", &json!({"ids": [g.0, comp.0]})).unwrap();
    assert_eq!(paint_targets(&s, &json!({})).unwrap(), vec![a, b, comp]);
}

#[test]
fn duplicate_names_stay_unique() {
    let mut s = session();
    s.execute("swatch.new", &json!({"name": "Sky", "color": "#3399ff"})).unwrap();
    assert_eq!(s.execute("swatch.duplicate", &json!({"name": "Sky"})).unwrap()["name"], "Sky copy");
    assert_eq!(s.execute("swatch.duplicate", &json!({"name": "Sky"})).unwrap()["name"], "Sky copy 2");
    // Colour-group names share the swatch namespace.
    assert_eq!(s.execute("swatch.newGroup", &json!({"name": "Sky"})).unwrap()["name"], "Sky 2");
    // New swatches from a colour are named by their RGB values (also inside new groups).
    assert_eq!(s.execute("swatch.new", &json!({"color": "#ff8000"})).unwrap()["name"], "R=255 G=128 B=0");
    s.execute("swatch.newGroup", &json!({"name": "G", "colors": ["#00ff00"]})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.swatch_groups.iter().find(|g| g.name == "G").unwrap().swatches[0].name, "R=0 G=255 B=0");

    let id = rect(&mut s, 0.0);
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("graphicStyle.new", &json!({"name": "Look"})).unwrap();
    assert_eq!(s.execute("graphicStyle.duplicate", &json!({"name": "Look"})).unwrap()["name"], "Look copy");
    assert_eq!(s.execute("graphicStyle.duplicate", &json!({"name": "Look"})).unwrap()["name"], "Look copy 2");
}
