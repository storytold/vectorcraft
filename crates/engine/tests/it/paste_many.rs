//! Paste of many objects: what lands in the document (nodes, ids, names, order, selection, one undo
//! step), and the shortcuts the paste takes for objects that have nothing to relink or rename.

use std::collections::HashSet;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind};
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

fn run(s: &mut Session, command: &str, params: Value) -> Value {
    s.execute(command, &params).unwrap()
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn ids_of(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect()
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    id_of(&run(s, "shape.rectangle", json!({"x": x, "y": 10, "width": 8, "height": 8})))
}

/// Three named groups of three named rectangles each, on the document's only layer.
fn named_groups() -> Session {
    let mut doc = Document::new(400.0, 400.0);
    let layer = doc.layers[0].id;
    for g in 0..3 {
        let group = doc.alloc_id();
        let mut node = Node::group(group, vec![]);
        node.name = Some(format!("group {g}"));
        doc.insert(Some(layer), usize::MAX, node).unwrap();
        for r in 0..3 {
            let x = f64::from(g * 3 + r) * 10.0;
            let shape = vectorcraft_geom::rect_from_points(Point::new(x, 0.0), Point::new(x + 8.0, 8.0));
            let appearance = Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 1.0);
            let id = doc.alloc_id();
            let mut node = Node::path(id, vectorcraft_geom::shapes::rectangle(shape), appearance);
            node.name = Some(format!("rect {g}.{r}"));
            doc.insert(Some(group), usize::MAX, node).unwrap();
        }
    }
    let mut s = Session::new();
    s.add_document(doc, None);
    s
}

/// `n` and its descendants in tree order, as (id, name).
fn tree(n: &Node) -> Vec<(NodeId, Option<String>)> {
    let mut out = vec![];
    n.walk(&mut |c| out.push((c.id, c.name.clone())));
    out
}

#[test]
fn a_paste_of_groups_gives_the_same_nodes_new_ids_in_tree_order_and_one_undo_step() {
    let mut s = named_groups();
    run(&mut s, "select.all", json!({}));
    run(&mut s, "edit.copy", json!({}));
    let st = s.doc().unwrap();
    let (first_id, steps, before) = (st.doc.peek_next_id(), st.history.undo.len(), st.doc.clone());
    let sources: Vec<NodeId> = st.selection.objects.clone();

    let pasted = run(&mut s, "edit.paste", json!({"dx": 5, "dy": 7}));
    let ids = ids_of(&pasted);
    let st = s.doc().unwrap();

    // The copies, on top of the layer in the order of their sources, are selected.
    assert_eq!(ids.len(), sources.len());
    assert_eq!(st.selection.objects, ids);
    let top: Vec<NodeId> = st.doc.layers[0].children().unwrap().iter().map(|c| c.id).collect();
    assert_eq!(top[sources.len()..], ids[..]);
    assert_eq!(top[..sources.len()], sources[..]);

    // Names and geometry match the sources; ids run on from the counter in tree order.
    let mut next = first_id;
    for (source, copy) in sources.iter().zip(&ids) {
        let (source, copy) = (st.doc.node(*source).unwrap(), st.doc.node(*copy).unwrap());
        let (a, b) = (tree(source), tree(copy));
        assert_eq!(a.len(), b.len());
        for ((_, name_a), (id_b, name_b)) in a.iter().zip(&b) {
            assert_eq!(name_a, name_b);
            assert_eq!(id_b.0, next);
            next += 1;
        }
        let (from, to) = (source.geometric_bounds().unwrap(), copy.geometric_bounds().unwrap());
        assert!((to.x0 - from.x0 - 5.0).abs() < 1e-9 && (to.y0 - from.y0 - 7.0).abs() < 1e-9, "{from:?} {to:?}");
    }
    assert_eq!(st.doc.peek_next_id(), next);

    // One undo step, back to the document as it was.
    assert_eq!(st.history.undo.len(), steps + 1);
    run(&mut s, "edit.undo", json!({}));
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), steps);
    assert_eq!(*st.doc, *before);
}

#[test]
fn pasted_type_in_a_group_gets_its_layout_bounds() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 500, "height": 200}));
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="500" height="200"><g><text x="0" y="40" font-family="Source Sans 3" font-size="20">WWW</text><text x="0" y="80" font-family="Source Sans 3" font-size="20">MMM</text></g></svg>"#;
    run(&mut s, "clipboard.importSvg", json!({"svg": svg}));
    run(&mut s, "edit.paste", json!({}));
    let mut checked = 0;
    s.doc().unwrap().doc.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            assert!(t.cached_bounds.is_some(), "{:?} has no layout bounds", n.id);
            checked += 1;
        }
    });
    assert_eq!(checked, 2);
}

/// A group holding point type and a rectangle: (the group, the type).
fn group_with_type() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 500, "height": 200}));
    let text = id_of(&run(&mut s, "text.create", json!({"x": 10, "y": 40, "text": "a", "size": 12})));
    let other = rect(&mut s, 100.0);
    run(&mut s, "select.set", json!({"ids": [text.0, other.0]}));
    let group = id_of(&run(&mut s, "object.group", json!({})));
    (s, group, text)
}

#[test]
fn find_and_replace_given_a_group_and_type_in_it_changes_that_type_once() {
    let (mut s, group, text) = group_with_type();
    // Replacing "a" with "aa" twice would leave "aaaa".
    run(&mut s, "edit.findReplace", json!({"find": "a", "replace": "aa", "ids": [group.0, text.0, group.0]}));
    let doc = &s.doc().unwrap().doc;
    let NodeKind::Text(t) = &doc.node(text).unwrap().kind else { panic!("the type is gone") };
    assert_eq!(t.runs.iter().map(|r| r.text.as_str()).collect::<String>(), "aa");
}

#[test]
fn a_type_command_given_a_group_and_type_in_it_lists_that_type_once() {
    let (mut s, group, text) = group_with_type();
    let changed = run(&mut s, "type.orientation.vertical", json!({"ids": [group.0, text.0, group.0]}));
    assert_eq!(ids_of(&changed), vec![text]);
}

#[test]
fn a_pasted_objects_on_path_group_names_its_own_copies() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 500, "height": 200}));
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 40.0));
    let path = id_of(&run(&mut s, "shape.line", json!({"x1": 0, "y1": 100, "x2": 300, "y2": 100})));
    let group = id_of(&run(&mut s, "object.objectsOnPath.attach", json!({"path": path.0, "ids": [a.0, b.0]})));

    // Inside another group too: every level of a copy gets new ids.
    let plain = rect(&mut s, 200.0);
    run(&mut s, "select.set", json!({"ids": [group.0, plain.0]}));
    let outer = id_of(&run(&mut s, "object.group", json!({})));
    run(&mut s, "select.set", json!({"ids": [outer.0]}));
    run(&mut s, "edit.copy", json!({}));
    let pasted = ids_of(&run(&mut s, "edit.paste", json!({})));

    let doc = &s.doc().unwrap().doc;
    let copy = doc.node(pasted[0]).unwrap().children().unwrap().iter().find(|c| c.extras().objects_on_path.is_some()).unwrap();
    let rec = copy.extras().objects_on_path.as_deref().unwrap();
    let children: HashSet<NodeId> = copy.children().unwrap().iter().map(|c| c.id).collect();
    assert!(children.contains(&rec.path), "the path {:?} is not in the copy", rec.path);
    assert_ne!(rec.path, path);
    assert_eq!(rec.members.len(), 2);
    for m in &rec.members {
        assert!(children.contains(&m.id), "the member {:?} is not in the copy", m.id);
    }
}

#[test]
fn a_pasted_object_keeps_its_graphic_style_under_the_id_it_gets_in_the_document() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 500, "height": 200}));
    let source = rect(&mut s, 10.0);
    run(&mut s, "paint.setFill", json!({"ids": [source.0], "color": "#ff0000"}));
    run(&mut s, "graphicStyle.new", json!({"name": "Copied", "id": source.0}));
    run(&mut s, "select.set", json!({"ids": [source.0]}));
    run(&mut s, "edit.copy", json!({}));

    // A second document whose own style already has the id the copied one has.
    run(&mut s, "file.new", json!({"width": 500, "height": 200}));
    let own = rect(&mut s, 10.0);
    run(&mut s, "paint.setFill", json!({"ids": [own.0], "color": "#0000ff"}));
    run(&mut s, "graphicStyle.new", json!({"name": "Own", "id": own.0}));
    let pasted = ids_of(&run(&mut s, "edit.paste", json!({})));

    let doc = &s.doc().unwrap().doc;
    let link = doc.node(pasted[0]).unwrap().extras().graphic_style;
    let copied = doc.graphic_styles.iter().find(|g| g.name == "Copied").unwrap().id;
    assert_ne!(copied, doc.graphic_styles.iter().find(|g| g.name == "Own").unwrap().id);
    assert_eq!(link, Some(copied));
    assert_eq!(doc.node(own).unwrap().extras().graphic_style, doc.graphic_styles.iter().find(|g| g.name == "Own").map(|g| g.id));
}

#[test]
fn a_pasted_object_without_a_graphic_style_stays_without_one() {
    let mut s = named_groups();
    run(&mut s, "select.all", json!({}));
    run(&mut s, "edit.copy", json!({}));
    for id in ids_of(&run(&mut s, "edit.paste", json!({}))) {
        s.doc().unwrap().doc.node(id).unwrap().walk(&mut |n| assert_eq!(n.extras().graphic_style, None));
    }
}
