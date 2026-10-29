//! Graphic styles: transparency, capture from groups and type, add-apply, unique names, links
//! (redefine, break link, delete, rename), Select All Unused, Sort by Name and Select > Same.

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, Paint};
use vectorcraft_doc::{GraphicStyle, Knockout, Node};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// A selected 50×50 rectangle at `x` with the default appearance.
fn rect(s: &mut Session, x: f64) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 10, "width": 50, "height": 50}))["id"].as_u64().unwrap());
    run(s, "select.set", json!({"ids": [id.0]}));
    id
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn style(s: &Session, name: &str) -> GraphicStyle {
    s.doc().unwrap().doc.graphic_style(name).unwrap_or_else(|| panic!("no style {name}")).clone()
}

fn fill(s: &Session, id: NodeId) -> Paint {
    node(s, id).appearance.fill_paint()
}

fn set_fill(s: &mut Session, id: NodeId, hex: &str) {
    run(s, "paint.setFill", json!({"color": hex, "ids": [id.0]}));
}

fn linked(s: &mut Session, name: &str) -> Vec<u64> {
    let l = run(s, "graphicStyle.list", json!({}));
    let st = l["styles"].as_array().unwrap().iter().find(|g| g["name"] == name).unwrap_or_else(|| panic!("{name} not listed")).clone();
    st["linked"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect()
}

#[test]
fn styles_carry_opacity_and_blend_through_a_save() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    run(&mut s, "transparency.set", json!({"item": null, "opacity": 50, "blend": "multiply", "knockout": true}));
    run(&mut s, "graphicStyle.new", json!({"name": "Half"}));
    let g = style(&s, "Half");
    assert_eq!((g.opacity, g.blend, g.knockout), (0.5, BlendMode::Multiply, Knockout::On));
    let b = rect(&mut s, 100.0);
    run(&mut s, "graphicStyle.apply", json!({"name": "Half"}));
    let nb = node(&s, b);
    assert_eq!((nb.opacity, nb.blend, nb.knockout), (0.5, BlendMode::Multiply, Knockout::On));
    let l = run(&mut s, "graphicStyle.list", json!({}));
    let half = l["styles"].as_array().unwrap().iter().find(|g| g["name"] == "Half").unwrap();
    assert_eq!((half["opacity"].as_f64(), half["blend"].as_str()), (Some(50.0), Some("Multiply")));
    assert_eq!(linked(&mut s, "Half"), vec![a.0, b.0]);
    assert_eq!(l["selected"], "Half");

    // Native round trip: the style's transparency, its id and the objects' links survive.
    let doc = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(back.graphic_styles, doc.graphic_styles);
    assert_eq!(back.node(b).unwrap().extras().graphic_style, Some(g.id));
    // Styles and objects from files without links still load (and get an id when first linked).
    let old: GraphicStyle = serde_json::from_value(json!({"name": "Old", "appearance": {"items": []}})).unwrap();
    assert_eq!((old.id, old.opacity, old.blend, old.isolate), (0, 1.0, BlendMode::Normal, false));
    let mut d = (*doc).clone();
    d.graphic_styles.push(old);
    let i = d.graphic_styles.len() - 1;
    assert_eq!(d.graphic_style_id(i), d.graphic_styles.iter().map(|g| g.id).max().unwrap());
    assert!(!serde_json::to_string(&GraphicStyle::new("Plain", Default::default())).unwrap().contains("opacity"));
}

#[test]
fn a_style_from_a_group_or_type_is_not_empty() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    let b = rect(&mut s, 100.0);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    assert!(node(&s, g).appearance.items.is_empty());
    run(&mut s, "graphicStyle.new", json!({"name": "Group Look"}));
    let gs = style(&s, "Group Look");
    assert_eq!(gs.appearance.fill_paint(), Paint::solid(Color::from_hex("#ff0000").unwrap()));
    // Both members share the look, so the group is linked.
    assert_eq!(linked(&mut s, "Group Look"), vec![g.0]);

    let t = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 200, "text": "Hi"}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "graphicStyle.new", json!({"name": "Type Look"}));
    assert!(!style(&s, "Type Look").appearance.fill_paint().is_none());

    // Applying to a group gives the group itself the style (its fills paint the members, which
    // keep their own look) and links it; `target: "contents"` styles the members instead.
    let c = rect(&mut s, 200.0);
    let before = node(&s, a).appearance;
    run(&mut s, "graphicStyle.apply", json!({"name": "Black Outline", "ids": [g.0]}));
    assert_eq!(node(&s, g).appearance, style(&s, "Black Outline").appearance);
    assert_eq!(node(&s, a).appearance, before);
    assert_eq!(node(&s, g).extras().graphic_style, Some(style(&s, "Black Outline").id));
    assert_eq!(linked(&mut s, "Black Outline"), vec![g.0]);
    assert_ne!(node(&s, c).appearance, style(&s, "Black Outline").appearance);
    run(&mut s, "graphicStyle.apply", json!({"name": "Black Outline", "ids": [g.0], "target": "contents"}));
    assert_eq!(node(&s, a).appearance, style(&s, "Black Outline").appearance);
}

#[test]
fn alt_apply_adds_the_style_on_top() {
    let mut s = session();
    rect(&mut s, 10.0);
    run(&mut s, "appearance.clear", json!({}));
    run(&mut s, "paint.setFill", json!({"color": "#00ff00"}));
    run(&mut s, "effect.apply", json!({"effect": "distort.roughen"}));
    run(&mut s, "graphicStyle.new", json!({"name": "Rough"}));
    let rough = style(&s, "Rough");
    let b = rect(&mut s, 100.0);
    let before = node(&s, b).appearance;
    run(&mut s, "graphicStyle.apply", json!({"name": "Rough", "add": true}));
    let after = node(&s, b).appearance;
    assert_eq!(after.items.len(), before.items.len() + rough.appearance.items.len());
    assert_eq!(after.items[..before.items.len()], before.items[..]);
    assert_eq!(after.items.last(), rough.appearance.items.last());
    assert_eq!(after.effects.len(), before.effects.len() + 1);
    assert_eq!(node(&s, b).extras().graphic_style, None);
    // One undo step.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, b).appearance, before);
}

#[test]
fn style_names_are_unique() {
    let mut s = session();
    rect(&mut s, 10.0);
    assert_eq!(run(&mut s, "graphicStyle.new", json!({"name": "X"}))["name"], "X");
    assert_eq!(run(&mut s, "graphicStyle.new", json!({"name": "X"}))["name"], "X 2");
    let n = s.doc().unwrap().doc.graphic_styles.len();
    assert_eq!(run(&mut s, "graphicStyle.new", json!({}))["name"], format!("Graphic Style {}", n + 1));
    assert!(s.execute("graphicStyle.rename", &json!({"name": "X 2", "to": "X"})).is_err());
    assert!(s.execute("graphicStyle.rename", &json!({"name": "X 2", "to": "  "})).is_err());
    assert_eq!(run(&mut s, "graphicStyle.rename", json!({"name": "X 2", "to": "Y"}))["name"], "Y");
    assert_eq!(run(&mut s, "graphicStyle.duplicate", json!({"name": "Y"}))["name"], "Y copy");
    assert_ne!(style(&s, "Y copy").id, style(&s, "Y").id);
}

/// Three rectangles: `a` (the source of style "S"), `b` and `c`, all linked to "S".
fn linked_three(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let a = rect(s, 10.0);
    run(s, "graphicStyle.new", json!({"name": "S"}));
    let b = rect(s, 100.0);
    let c = rect(s, 200.0);
    run(s, "graphicStyle.apply", json!({"name": "S", "ids": [b.0, c.0]}));
    assert_eq!(linked(s, "S"), vec![a.0, b.0, c.0]);
    (a, b, c)
}

#[test]
fn redefine_updates_every_linked_object() {
    let mut s = session();
    let (a, b, c) = linked_three(&mut s);
    set_fill(&mut s, a, "#0000ff");
    // Editing the source broke its link, but it remembers the style Redefine replaces.
    assert_eq!(linked(&mut s, "S"), vec![b.0, c.0]);
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    assert_eq!(s.selection_graphic_style().map(|(g, linked)| (g.name.clone(), linked)), Some(("S".into(), false)));
    run(&mut s, "graphicStyle.redefine", json!({}));
    let blue = Paint::solid(Color::from_hex("#0000ff").unwrap());
    assert_eq!((fill(&s, b), fill(&s, c)), (blue.clone(), blue));
    assert_eq!(linked(&mut s, "S"), vec![a.0, b.0, c.0]);
    // The step is named from a template with the style's name in it.
    let step = s.doc().unwrap().history.undo.last().unwrap().clone();
    let named = step.named().map(|n| (n.template, n.name.as_str(), n.built_in));
    assert_eq!((step.label.as_str(), named), ("Redefine Graphic Style “S”", Some(("Redefine Graphic Style “{name}”", "S", false))));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(fill(&s, b), Paint::solid(Color::WHITE));
}

#[test]
fn redefine_leaves_unlinked_and_edited_objects_alone() {
    let mut s = session();
    let (a, b, c) = linked_three(&mut s);
    run(&mut s, "graphicStyle.breakLink", json!({"ids": [c.0]}));
    assert_eq!(node(&s, c).extras().graphic_style, None);
    let d = rect(&mut s, 300.0);
    run(&mut s, "graphicStyle.apply", json!({"name": "S"}));
    set_fill(&mut s, d, "#00ff00");
    set_fill(&mut s, a, "#0000ff");
    run(&mut s, "graphicStyle.redefine", json!({"name": "S", "id": a.0}));
    assert_eq!(fill(&s, b), Paint::solid(Color::from_hex("#0000ff").unwrap()));
    assert_eq!(fill(&s, c), Paint::solid(Color::WHITE));
    // The edited object keeps its look and is unlinked.
    assert_eq!(fill(&s, d), Paint::solid(Color::from_hex("#00ff00").unwrap()));
    assert_eq!(node(&s, d).extras().graphic_style, None);
    assert!(s.execute("graphicStyle.redefine", &json!({"id": c.0})).is_err());
}

#[test]
fn deleting_a_style_unlinks_and_renaming_keeps_links() {
    let mut s = session();
    let (_, b, c) = linked_three(&mut s);
    run(&mut s, "graphicStyle.rename", json!({"name": "S", "to": "T"}));
    assert_eq!(linked(&mut s, "T").len(), 3);
    run(&mut s, "select.set", json!({"ids": [b.0]}));
    assert_eq!(run(&mut s, "select.same.graphicStyle", json!({}))["count"], 3);
    let look = node(&s, c).appearance;
    run(&mut s, "graphicStyle.delete", json!({"name": "T"}));
    assert_eq!((node(&s, b).extras().graphic_style, node(&s, c).extras().graphic_style), (None, None));
    assert_eq!(node(&s, c).appearance, look);
    run(&mut s, "select.set", json!({"ids": [b.0]}));
    assert!(s.execute("select.same.graphicStyle", &json!({})).is_err());
}

#[test]
fn unused_styles_and_sort_by_name() {
    let mut s = session();
    linked_three(&mut s);
    let unused = run(&mut s, "graphicStyle.unused", json!({}));
    let names: Vec<&str> = unused["names"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(names.contains(&"Sunshine") && !names.contains(&"S"));
    run(&mut s, "graphicStyle.delete", json!({"names": names}));
    assert_eq!(s.doc().unwrap().doc.graphic_styles.len(), 1);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "graphicStyle.sortByName", json!({}));
    let names: Vec<String> = s.doc().unwrap().doc.graphic_styles.iter().map(|g| g.name.clone()).collect();
    assert_eq!(names, ["Default Graphic Style", "Black Outline", "Heavy Ink", "S", "Sunshine"]);
}

#[test]
fn select_same_appearance_attribute() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    run(&mut s, "effect.apply", json!({"effect": "distort.roughen"}));
    let b = rect(&mut s, 100.0);
    set_fill(&mut s, b, "#ff0000");
    run(&mut s, "effect.apply", json!({"effect": "distort.roughen"}));
    let c = rect(&mut s, 200.0);
    // The shared effect.
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    run(&mut s, "select.same.appearanceAttribute", json!({}));
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, b]);
    // The shared stroke (item 1).
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    run(&mut s, "select.same.appearanceAttribute", json!({"item": 1}));
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, b, c]);
    // An item past the last one leaves the shared effect. A 32-bit build read 2^32 + 1 as item 1.
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    run(&mut s, "select.same.appearanceAttribute", json!({"item": (1u64 << 32) + 1}));
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, b]);
}

/// With a member path of a compound path selected first, Select > Same > Graphic Style and
/// Appearance Attribute compare the other objects with the compound path, except in isolation mode
/// inside it.
#[test]
fn select_same_style_and_attribute_from_a_compound_path_member() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    set_fill(&mut s, a, "#ff0000");
    run(&mut s, "effect.apply", json!({"effect": "distort.roughen"}));
    run(&mut s, "graphicStyle.new", json!({"name": "Rough"}));
    run(&mut s, "graphicStyle.apply", json!({"name": "Rough"}));
    let outer = rect(&mut s, 100.0);
    let inner = rect(&mut s, 120.0);
    run(&mut s, "select.set", json!({"ids": [outer.0, inner.0]}));
    let c = NodeId(run(&mut s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "graphicStyle.apply", json!({"name": "Rough"}));
    rect(&mut s, 300.0);
    for cmd in ["select.same.graphicStyle", "select.same.appearanceAttribute"] {
        for from in [c, outer] {
            run(&mut s, "select.set", json!({"ids": [from.0]}));
            run(&mut s, cmd, json!({}));
            assert_eq!(s.doc().unwrap().selection.objects, vec![a, c], "{cmd} from {from:?}");
        }
    }
    // Isolated in the compound path, its member paths are compared, and they carry no style.
    run(&mut s, "object.isolate", json!({"id": c.0}));
    run(&mut s, "select.set", json!({"ids": [outer.0]}));
    assert!(s.execute("select.same.graphicStyle", &json!({})).is_err());
}

#[test]
fn document_info_lists_style_names() {
    let mut s = session();
    let (a, ..) = linked_three(&mut s);
    let i = run(&mut s, "document.info", json!({}));
    assert_eq!(i["graphicStyleNames"].as_array().unwrap().len(), 5);
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    assert_eq!(run(&mut s, "document.info", json!({"selectionOnly": true}))["graphicStyleNames"], json!(["S"]));
}
