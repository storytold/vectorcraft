//! Select › Start Global Edit (`select.globalEdit`, `select.globalEditOptions`): which objects
//! count as similar, and edits made to one of them reaching the others in proportion, as one undo
//! step. Behaviour: <https://helpx.adobe.com/illustrator/desktop/manage-objects/edit-objects/edit-similar-objects.html>.

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{Affine, PathData, Rect};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    node(s, id).geometric_bounds().unwrap()
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn ids(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect()
}

fn start(s: &mut Session, key: NodeId, p: Value) -> Vec<NodeId> {
    sel(s, &[key]);
    let r = s.execute("select.globalEdit", &p).unwrap();
    assert_eq!(r["active"], true);
    ids(&r)
}

fn fill(s: &Session, id: NodeId) -> String {
    node(s, id).appearance.fill_paint().color().unwrap().to_hex()
}

fn stroke(s: &Session, id: NodeId) -> (String, f64) {
    let n = node(s, id);
    (n.appearance.stroke_paint().color().unwrap().to_hex(), n.appearance.stroke_width())
}

fn members(s: &Session) -> Option<Vec<NodeId>> {
    s.doc().unwrap().global_edit.as_ref().map(|g| g.members.iter().map(|m| m.0).collect())
}

fn close(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(p, q)| (p - q).abs() < 1e-6)
}

/// `b`'s outline is `a`'s placed by `xf`, anchor for anchor.
fn placed(s: &Session, a: NodeId, b: NodeId, xf: Affine) -> bool {
    let (pa, pb): (PathData, PathData) = (node(s, a).path_data().unwrap().transformed(xf), node(s, b).path_data().unwrap().clone());
    pa.subpaths.len() == pb.subpaths.len()
        && pa.subpaths.iter().zip(&pb.subpaths).all(|(x, y)| {
            x.closed == y.closed
                && x.anchors.len() == y.anchors.len()
                && x.anchors
                    .iter()
                    .zip(&y.anchors)
                    .all(|(m, n)| m.p.distance(n.p) < 1e-6 && m.h_in.distance(n.h_in) < 1e-6 && m.h_out.distance(n.h_out) < 1e-6)
        })
}

/// A 100 pt square, a copy of it, a 20 pt square (the same shape, smaller), a 100 × 50 rectangle
/// and a 100 pt circle (other shapes).
fn squares(s: &mut Session) -> [NodeId; 5] {
    [rect(s, 0.0, 0.0, 100.0, 100.0), rect(s, 300.0, 0.0, 100.0, 100.0), rect(s, 500.0, 0.0, 20.0, 20.0), rect(s, 0.0, 300.0, 100.0, 50.0), {
        let r = s.execute("shape.ellipse", &json!({"x": 300, "y": 300, "width": 100, "height": 100})).unwrap();
        NodeId(r["id"].as_u64().unwrap())
    }]
}

#[test]
fn start_global_edit_takes_the_objects_of_the_same_shape_at_any_place_and_size() {
    let mut s = session();
    let [a, b, c, wide, circle] = squares(&mut s);
    let members = start(&mut s, a, json!({}));
    assert_eq!(members, [a, b, c], "the copy and the smaller square; not the rectangle {wide} nor the circle {circle}");
    // The selected object stays the selection; the others are highlighted on the canvas.
    assert_eq!(s.doc().unwrap().selection.objects, [a]);
    assert_eq!(crate::cmd::globaledit::highlights(&s).len(), 2);
    let r = s.execute("select.globalEdit", &json!({})).unwrap();
    assert_eq!(r["active"], false, "the second run is Stop Global Edit");
    assert!(s.doc().unwrap().global_edit.is_none() && crate::cmd::globaledit::highlights(&s).is_empty());
}

#[test]
fn size_and_appearance_options_narrow_the_match() {
    let mut s = session();
    let [a, b, c, ..] = squares(&mut s);
    assert_eq!(start(&mut s, a, json!({"size": true})), [a, b], "the smaller square has another size");
    s.execute("select.globalEdit", &json!({})).unwrap();
    sel(&mut s, &[b]);
    s.execute("paint.setFill", &json!({"color": "#12ab34"})).unwrap();
    assert_eq!(start(&mut s, a, json!({"appearance": true})), [a, c], "the copy has another fill now");
    s.execute("select.globalEdit", &json!({})).unwrap();
    // The options are kept for the session, and report how many objects they take.
    sel(&mut s, &[a]);
    let r = s.execute("select.globalEditOptions", &json!({"size": true, "appearance": true})).unwrap();
    assert_eq!((r["size"].clone(), r["appearance"].clone(), r["count"].clone()), (json!(true), json!(true), json!(1)));
    let e = s.execute("select.globalEdit", &json!({})).unwrap_err().to_string();
    assert!(e.contains("no similar objects"), "{e}");
    s.execute("select.globalEditOptions", &json!({"size": false, "appearance": false})).unwrap();
    assert_eq!(start(&mut s, a, json!({})), [a, b, c]);
}

#[test]
fn an_edit_reaches_every_member_in_proportion_as_one_undo_step() {
    let mut s = session();
    let [a, b, c, wide, circle] = squares(&mut s);
    start(&mut s, a, json!({}));
    let before = (*s.doc().unwrap().doc).clone();
    let steps = s.doc().unwrap().history.undo.len();
    let (wide_was, circle_was) = (node(&s, wide), node(&s, circle));
    // Height 100 → 50 from the top left (the Transform panel): the 20 pt square goes to 10 pt.
    s.execute("object.setBounds", &json!({"height": 50, "reference": 0})).unwrap();
    assert!(close(bounds(&s, a), Rect::new(0.0, 0.0, 100.0, 50.0)), "{:?}", bounds(&s, a));
    assert!(close(bounds(&s, b), Rect::new(300.0, 0.0, 400.0, 50.0)), "{:?}", bounds(&s, b));
    assert!(close(bounds(&s, c), Rect::new(500.0, 0.0, 520.0, 10.0)), "{:?}", bounds(&s, c));
    assert_eq!((node(&s, wide), node(&s, circle)), (wide_was, circle_was), "objects of other shapes are left alone");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
    crate::tests_large_edits::undo_gives_back(&mut s, &before);
}

#[test]
fn an_anchor_moved_on_one_member_moves_on_the_others() {
    let mut s = session();
    let [a, b, c, ..] = squares(&mut s);
    start(&mut s, a, json!({}));
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0]]})).unwrap();
    s.execute("path.moveAnchors", &json!({"dx": 10, "dy": 30})).unwrap();
    assert!(placed(&s, a, b, Affine::translate((300.0, 0.0))));
    assert!(placed(&s, a, c, Affine::translate((500.0, 0.0)) * Affine::scale(0.2)));
    assert!(!placed(&s, a, c, Affine::translate((500.0, 0.0))), "the smaller square's anchor moves a fifth as far");
}

#[test]
fn a_drag_moves_the_members_with_each_preview_and_commits_one_step() {
    let mut s = session();
    let [a, b, c, ..] = squares(&mut s);
    start(&mut s, a, json!({}));
    let before = (*s.doc().unwrap().doc).clone();
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 30, "dy": 0})).unwrap();
    s.preview("object.move", &json!({"dx": 40, "dy": 0})).unwrap();
    assert!(close(bounds(&s, b), Rect::new(340.0, 0.0, 440.0, 100.0)), "the second preview replaces the first: {:?}", bounds(&s, b));
    s.commit_interaction().unwrap();
    assert!(close(bounds(&s, c), Rect::new(508.0, 0.0, 528.0, 20.0)), "{:?}", bounds(&s, c));
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Move");
    crate::tests_large_edits::undo_gives_back(&mut s, &before);
}

#[test]
fn any_selected_member_leads_and_fills_reach_the_others_only_when_they_change() {
    let mut s = session();
    let [a, b, c, ..] = squares(&mut s);
    sel(&mut s, &[c]);
    s.execute("paint.setFill", &json!({"color": "#123456"})).unwrap();
    start(&mut s, a, json!({}));
    let c_fill = fill(&s, c);
    // Moving the copy (selected during Global Edit) moves the others in proportion and keeps
    // their own fills.
    sel(&mut s, &[b]);
    s.execute("object.move", &json!({"dx": 50, "dy": 0})).unwrap();
    assert!(close(bounds(&s, a), Rect::new(50.0, 0.0, 150.0, 100.0)), "{:?}", bounds(&s, a));
    assert!(close(bounds(&s, c), Rect::new(510.0, 0.0, 530.0, 20.0)), "{:?}", bounds(&s, c));
    assert_eq!(fill(&s, c), c_fill);
    // A new fill reaches all of them, and the stroke weight stays the model's, not scaled.
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 4})).unwrap();
    for id in [a, b, c] {
        assert_eq!(fill(&s, id), "#ff0000");
        assert_eq!(node(&s, id).appearance.stroke_width(), 4.0);
    }
    // Selection changes alone edit nothing; after Stop Global Edit an edit stays on its object.
    s.execute("select.globalEdit", &json!({})).unwrap();
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    assert!(close(bounds(&s, a), Rect::new(50.0, 0.0, 150.0, 100.0)));
}

#[test]
fn locked_and_hidden_objects_are_left_out() {
    let mut s = session();
    let [a, b, c, ..] = squares(&mut s);
    let locked = rect(&mut s, 0.0, 450.0, 100.0, 100.0);
    let hidden = rect(&mut s, 150.0, 450.0, 100.0, 100.0);
    s.execute("object.lock", &json!({"ids": [locked.0]})).unwrap();
    s.execute("object.hide", &json!({"ids": [hidden.0]})).unwrap();
    assert_eq!(start(&mut s, a, json!({})), [a, b, c], "locked and hidden squares are not found");
    // A member locked after the start is not edited either.
    s.execute("object.lock", &json!({"ids": [b.0]})).unwrap();
    let b_was = node(&s, b);
    sel(&mut s, &[a]);
    s.execute("object.move", &json!({"dx": 0, "dy": 10})).unwrap();
    assert_eq!(node(&s, b), b_was);
    assert!(close(bounds(&s, c), Rect::new(500.0, 2.0, 520.0, 22.0)), "{:?}", bounds(&s, c));
    assert_eq!(bounds(&s, locked), Rect::new(0.0, 450.0, 100.0, 550.0));
    assert_eq!(bounds(&s, hidden), Rect::new(150.0, 450.0, 250.0, 550.0));
}

#[test]
fn artboard_options_limit_where_similar_objects_are_found() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 400, "height": 400})).unwrap();
    let key = rect(&mut s, 10.0, 10.0, 50.0, 50.0);
    let same_board = rect(&mut s, 200.0, 10.0, 50.0, 50.0);
    let second = rect(&mut s, 1100.0, 10.0, 50.0, 50.0);
    let canvas = rect(&mut s, 2000.0, 2000.0, 50.0, 50.0);
    assert_eq!(start(&mut s, key, json!({})), [key, same_board, second, canvas], "all artboards and the canvas by default");
    s.execute("select.globalEdit", &json!({})).unwrap();
    assert_eq!(start(&mut s, key, json!({"includeCanvas": false})), [key, same_board, second]);
    s.execute("select.globalEdit", &json!({})).unwrap();
    assert_eq!(start(&mut s, key, json!({"artboards": "2"})), [key, second, canvas]);
    s.execute("select.globalEdit", &json!({})).unwrap();
    assert_eq!(start(&mut s, key, json!({"artboards": "2", "includeCanvas": false})), [key, second]);
    s.execute("select.globalEdit", &json!({})).unwrap();
    let e = s.execute("select.globalEdit", &json!({"artboards": "3"})).unwrap_err().to_string();
    assert!(e.contains("1 to 2"), "{e}");
}

#[test]
fn compound_paths_match_whole_and_keep_their_own_member_ids() {
    let mut s = session();
    let compound = |s: &mut Session, x: f64, k: f64| {
        let (p, q) = (rect(s, x, 0.0, 100.0 * k, 100.0 * k), rect(s, x + 25.0 * k, 25.0 * k, 50.0 * k, 50.0 * k));
        sel(s, &[p, q]);
        NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap())
    };
    let (one, two) = (compound(&mut s, 0.0, 1.0), compound(&mut s, 300.0, 0.5));
    let lone = rect(&mut s, 600.0, 0.0, 100.0, 100.0);
    assert_eq!(start(&mut s, one, json!({})), [one, two], "not the square {lone} alone");
    let kids = |s: &Session, id: NodeId| node(s, id).children().unwrap().iter().map(|c| c.id).collect::<Vec<_>>();
    let two_kids = kids(&s, two);
    s.execute("object.setBounds", &json!({"width": 200, "reference": 0})).unwrap();
    assert!(close(bounds(&s, two), Rect::new(300.0, 0.0, 400.0, 50.0)), "{:?}", bounds(&s, two));
    assert_eq!(kids(&s, two), two_kids);
    assert!(matches!(node(&s, two).kind, NodeKind::Compound { .. }));
}

#[test]
fn symbol_instances_of_one_symbol_match() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 40.0, 40.0);
    let made = s.execute("symbol.new", &json!({"ids": [r.0], "name": "Tile"})).unwrap();
    let first = NodeId(made["id"].as_u64().unwrap());
    let placed = s.execute("symbol.place", &json!({"name": "Tile", "x": 300, "y": 300})).unwrap();
    let second = NodeId(placed["id"].as_u64().unwrap());
    let other = rect(&mut s, 500.0, 500.0, 40.0, 40.0);
    assert_eq!(start(&mut s, first, json!({})), [first, second], "not the plain square {other}");
    let xf = |s: &Session, id: NodeId| match node(s, id).kind {
        NodeKind::SymbolInstance { xf, .. } => xf,
        _ => panic!("not an instance"),
    };
    let was = xf(&s, second);
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    let (now, want) = (xf(&s, second).as_coeffs(), (Affine::translate((10.0, 0.0)) * was).as_coeffs());
    assert!(now.iter().zip(want).all(|(p, q)| (p - q).abs() < 1e-6), "{now:?} vs {want:?}");
}

#[test]
fn global_edit_needs_one_supported_object() {
    let mut s = session();
    let [a, b, ..] = squares(&mut s);
    sel(&mut s, &[a, b]);
    assert!(matches!(s.execute("select.globalEdit", &json!({})), Err(EngineError::Disabled(..))), "a selection of two");
    let t = NodeId(s.execute("text.create", &json!({"x": 50, "y": 500, "text": "Hi"})).unwrap()["id"].as_u64().unwrap());
    sel(&mut s, &[t]);
    assert!(matches!(s.execute("select.globalEdit", &json!({})), Err(EngineError::Disabled(..))), "type");
    let lone = rect(&mut s, 0.0, 500.0, 30.0, 70.0);
    // A copy turned a quarter turn (70 × 30) is not taken: rotated copies are not matched, and the
    // message says so.
    let turned = rect(&mut s, 200.0, 500.0, 30.0, 70.0);
    sel(&mut s, &[turned]);
    s.execute("object.rotate", &json!({"angle": 90})).unwrap();
    sel(&mut s, &[lone]);
    let e = s.execute("select.globalEdit", &json!({})).unwrap_err().to_string();
    assert!(e.contains("no similar objects") && e.contains("rotated"), "{e}");
    assert!(s.doc().unwrap().global_edit.is_none());
    // A group that holds type is refused, though another such group exists.
    let typed = |s: &mut Session, x: f64| {
        let p = rect(s, x, 100.0, 10.0, 10.0);
        let t = NodeId(s.execute("text.create", &json!({"x": x + 20.0, "y": 110, "text": "Hi"})).unwrap()["id"].as_u64().unwrap());
        group(s, &[p, t])
    };
    let (g, _) = (typed(&mut s, 600.0), typed(&mut s, 700.0));
    sel(&mut s, &[g]);
    let e = s.execute("select.globalEdit", &json!({})).unwrap_err();
    assert!(matches!(e, EngineError::Disabled(..)) && e.to_string().contains("symbol instances and groups of them"), "{e}");
}

fn group(s: &mut Session, ids: &[NodeId]) -> NodeId {
    sel(s, ids);
    NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap())
}

fn circle(s: &mut Session, x: f64, y: f64, d: f64) -> NodeId {
    NodeId(s.execute("shape.ellipse", &json!({"x": x, "y": y, "width": d, "height": d})).unwrap()["id"].as_u64().unwrap())
}

/// A group of a square and a circle beside it, `k` times the size of a 20 pt square and a 20 pt
/// circle 40 pt to its right, from (`x`, `y`).
fn pair(s: &mut Session, x: f64, y: f64, k: f64) -> NodeId {
    let (sq, c) = (rect(s, x, y, 20.0 * k, 20.0 * k), circle(s, x + 40.0 * k, y, 20.0 * k));
    group(s, &[sq, c])
}

fn kids(s: &Session, id: NodeId) -> Vec<NodeId> {
    node(s, id).children().unwrap().iter().map(|c| c.id).collect()
}

#[test]
fn start_global_edit_on_a_group_takes_the_groups_of_the_same_hierarchy_and_shapes() {
    let mut s = session();
    let (one, two, half) = (pair(&mut s, 0.0, 0.0, 1.0), pair(&mut s, 300.0, 0.0, 1.0), pair(&mut s, 500.0, 0.0, 0.5));
    // The circle under the square: another stacking order.
    let swapped = {
        let c = circle(&mut s, 40.0, 100.0, 20.0);
        let sq = rect(&mut s, 0.0, 100.0, 20.0, 20.0);
        group(&mut s, &[c, sq])
    };
    // The circle further away: the same shapes, placed apart differently.
    let apart = {
        let (sq, c) = (rect(&mut s, 0.0, 200.0, 20.0, 20.0), circle(&mut s, 60.0, 200.0, 20.0));
        group(&mut s, &[sq, c])
    };
    // A third object, and the square in a group of its own: another hierarchy.
    let three = {
        let (sq, c, more) = (rect(&mut s, 0.0, 300.0, 20.0, 20.0), circle(&mut s, 40.0, 300.0, 20.0), rect(&mut s, 80.0, 300.0, 5.0, 5.0));
        group(&mut s, &[sq, c, more])
    };
    let nested = {
        let sq = rect(&mut s, 0.0, 400.0, 20.0, 20.0);
        let inner = group(&mut s, &[sq]);
        let c = circle(&mut s, 40.0, 400.0, 20.0);
        group(&mut s, &[inner, c])
    };
    let lone = rect(&mut s, 300.0, 300.0, 20.0, 20.0);
    let members = start(&mut s, one, json!({}));
    assert_eq!(members, [one, two, half], "not {swapped}, {apart}, {three}, {nested} nor the square {lone}");
    assert!(s.doc().unwrap().global_edit.as_ref().unwrap().groups);
    assert_eq!(crate::cmd::globaledit::highlights(&s).len(), 2);
    s.execute("select.globalEdit", &json!({})).unwrap();
    // Size leaves out the smaller group; Appearance the group whose circle has another fill.
    assert_eq!(start(&mut s, one, json!({"size": true})), [one, two]);
    s.execute("select.globalEdit", &json!({})).unwrap();
    let circle_two = kids(&s, two)[1];
    sel(&mut s, &[circle_two]);
    s.execute("paint.setFill", &json!({"color": "#12ab34"})).unwrap();
    assert_eq!(start(&mut s, one, json!({"appearance": true})), [one, half]);
    s.execute("select.globalEdit", &json!({})).unwrap();
    assert_eq!(start(&mut s, one, json!({})), [one, two, half], "Appearance off: fills may differ");
}

#[test]
fn appearance_compares_a_groups_own_fill_and_stroke() {
    let mut s = session();
    let (one, two, three) = (pair(&mut s, 0.0, 0.0, 1.0), pair(&mut s, 300.0, 0.0, 1.0), pair(&mut s, 500.0, 0.0, 1.0));
    // Only the second group's own stroke differs; its objects are painted as the others' are.
    s.execute("appearance.addStroke", &json!({"ids": [two.0]})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [two.0], "item": 0, "color": "#000000"})).unwrap();
    s.execute("stroke.set", &json!({"ids": [two.0], "item": 0, "weight": 10})).unwrap();
    assert_eq!(start(&mut s, one, json!({"appearance": true})), [one, three]);
    s.execute("select.globalEdit", &json!({})).unwrap();
    assert_eq!(start(&mut s, one, json!({})), [one, two, three], "Appearance off: group strokes may differ");
}

#[test]
fn a_member_group_that_holds_a_selected_object_is_not_highlighted() {
    let mut s = session();
    let (one, two, three) = (pair(&mut s, 0.0, 0.0, 1.0), pair(&mut s, 300.0, 0.0, 1.0), pair(&mut s, 500.0, 0.0, 1.0));
    start(&mut s, one, json!({}));
    assert_eq!(crate::cmd::globaledit::highlights(&s).len(), 2);
    // The circle of the second group selected: only the first and third groups are highlighted.
    let (round, squares) = (kids(&s, two)[1], [kids(&s, one)[0], kids(&s, three)[0]]);
    sel(&mut s, &[round]);
    assert_eq!(crate::cmd::globaledit::highlights(&s).len(), 2, "{one} and {three}, not {two}");
    // Objects of the first and third groups selected: only the second group is highlighted.
    sel(&mut s, &squares);
    assert_eq!(crate::cmd::globaledit::highlights(&s).len(), 1);
}

#[test]
fn an_object_selected_inside_a_group_before_the_start_is_matched_as_an_object() {
    let mut s = session();
    let (one, two) = (pair(&mut s, 0.0, 0.0, 1.0), pair(&mut s, 300.0, 0.0, 1.0));
    let lone = rect(&mut s, 0.0, 300.0, 40.0, 40.0);
    let square = kids(&s, one)[0];
    assert_eq!(start(&mut s, square, json!({})), [square, kids(&s, two)[0], lone]);
    assert!(!s.doc().unwrap().global_edit.as_ref().unwrap().groups);
}

#[test]
fn an_edit_inside_a_group_reaches_the_corresponding_object_of_the_others_as_one_undo_step() {
    let mut s = session();
    let (one, two, half) = (pair(&mut s, 0.0, 0.0, 1.0), pair(&mut s, 300.0, 0.0, 1.0), pair(&mut s, 500.0, 0.0, 0.5));
    let lone = rect(&mut s, 0.0, 300.0, 20.0, 20.0);
    start(&mut s, one, json!({}));
    let before = (*s.doc().unwrap().doc).clone();
    let steps = s.doc().unwrap().history.undo.len();
    let ([sq1, c1], [sq2, c2], [sq3, c3]) =
        ([kids(&s, one)[0], kids(&s, one)[1]], [kids(&s, two)[0], kids(&s, two)[1]], [kids(&s, half)[0], kids(&s, half)[1]]);
    let (squares_were, lone_was) = ([node(&s, sq2), node(&s, sq3)], node(&s, lone));
    // Moving the circle of the first group moves the circle of each other group, the smaller one
    // half as far, and leaves the squares where they are.
    sel(&mut s, &[c1]);
    s.execute("object.move", &json!({"dx": 10, "dy": 4})).unwrap();
    assert!(close(bounds(&s, c2), Rect::new(350.0, 4.0, 370.0, 24.0)), "{:?}", bounds(&s, c2));
    assert!(close(bounds(&s, c3), Rect::new(525.0, 2.0, 535.0, 12.0)), "{:?}", bounds(&s, c3));
    assert_eq!([node(&s, sq2), node(&s, sq3)], squares_were, "the other objects are left as they are");
    assert_eq!(node(&s, lone), lone_was, "a square outside the groups is not a member");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
    crate::tests_large_edits::undo_gives_back(&mut s, &before);
    // A resize, a rotation, a fill and a stroke weight of the square reach the other squares.
    sel(&mut s, &[sq1]);
    s.execute("object.setBounds", &json!({"height": 10, "reference": 0})).unwrap();
    assert!(close(bounds(&s, sq3), Rect::new(500.0, 0.0, 510.0, 5.0)), "{:?}", bounds(&s, sq3));
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    assert!(placed(&s, sq1, sq2, Affine::translate((300.0, 0.0))));
    assert!(placed(&s, sq1, sq3, Affine::translate((500.0, 0.0)) * Affine::scale(0.5)));
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 4})).unwrap();
    for id in [sq1, sq2, sq3] {
        assert_eq!(fill(&s, id), "#ff0000");
        assert_eq!(node(&s, id).appearance.stroke_width(), 4.0);
    }
    assert_ne!(fill(&s, c2), "#ff0000", "the circles keep their fill");
    // The member ids are kept.
    assert_eq!((kids(&s, two), kids(&s, half)), (vec![sq2, c2], vec![sq3, c3]));
    // An anchor moved on the circle of the second group moves on the other circles.
    s.execute("select.anchors", &json!({"id": c2.0, "anchors": [[0, 0]]})).unwrap();
    s.execute("path.moveAnchors", &json!({"dx": 6, "dy": 0})).unwrap();
    assert!(placed(&s, c2, c1, Affine::translate((-300.0, 0.0))));
    assert!(placed(&s, c2, c3, Affine::translate((500.0, 0.0)) * Affine::scale(0.5) * Affine::translate((-300.0, 0.0))));
}

#[test]
fn a_drag_inside_a_group_moves_the_corresponding_objects_with_each_preview() {
    let mut s = session();
    let (one, two) = (pair(&mut s, 0.0, 0.0, 1.0), pair(&mut s, 300.0, 0.0, 2.0));
    start(&mut s, one, json!({}));
    let before = (*s.doc().unwrap().doc).clone();
    let square = kids(&s, one)[0];
    sel(&mut s, &[square]);
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 30, "dy": 0})).unwrap();
    s.preview("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    s.commit_interaction().unwrap();
    assert!(close(bounds(&s, kids(&s, two)[0]), Rect::new(310.0, 0.0, 350.0, 40.0)), "{:?}", bounds(&s, kids(&s, two)[0]));
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Move");
    crate::tests_large_edits::undo_gives_back(&mut s, &before);
}

#[test]
fn nested_groups_match_level_by_level_and_edits_reach_the_same_place() {
    let mut s = session();
    let nest = |s: &mut Session, x: f64| {
        let (a, b) = (rect(s, x, 0.0, 10.0, 10.0), circle(s, x + 20.0, 0.0, 10.0));
        let inner = group(s, &[a, b]);
        let c = rect(s, x, 40.0, 30.0, 10.0);
        (group(s, &[inner, c]), b)
    };
    let ((one, b1), (two, b2)) = (nest(&mut s, 0.0), nest(&mut s, 300.0));
    // The same objects, all in one group: another hierarchy.
    let flat = {
        let (a, b, c) = (rect(&mut s, 600.0, 0.0, 10.0, 10.0), circle(&mut s, 620.0, 0.0, 10.0), rect(&mut s, 600.0, 40.0, 30.0, 10.0));
        group(&mut s, &[a, b, c])
    };
    assert_eq!(start(&mut s, one, json!({})), [one, two], "not {flat}");
    sel(&mut s, &[b1]);
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    assert_eq!(fill(&s, b2), "#00ff00");
    // A setting of a subgroup itself reaches the subgroup of the other member.
    let (inner1, inner2) = (kids(&s, one)[0], kids(&s, two)[0]);
    sel(&mut s, &[inner1]);
    s.execute("transparency.set", &json!({"opacity": 50})).unwrap();
    assert_eq!((node(&s, inner1).opacity, node(&s, inner2).opacity), (0.5, 0.5));
}

#[test]
fn groups_with_hidden_locked_or_unsupported_objects_are_left_out() {
    let mut s = session();
    let (one, two, three, four) =
        (pair(&mut s, 0.0, 0.0, 1.0), pair(&mut s, 300.0, 0.0, 1.0), pair(&mut s, 0.0, 300.0, 1.0), pair(&mut s, 300.0, 300.0, 1.0));
    s.execute("object.lock", &json!({"ids": [kids(&s, two)[0].0]})).unwrap();
    s.execute("object.hide", &json!({"ids": [kids(&s, three)[1].0]})).unwrap();
    assert_eq!(start(&mut s, one, json!({})), [one, four], "groups with a locked or a hidden object are not found");
    // A member's object locked after the start is not edited; the rest of the member is.
    s.execute("object.lock", &json!({"ids": [kids(&s, four)[0].0]})).unwrap();
    let locked_was = node(&s, kids(&s, four)[0]);
    let [square, round] = [kids(&s, one)[0], kids(&s, one)[1]];
    sel(&mut s, &[square]);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    assert_eq!(node(&s, kids(&s, four)[0]), locked_was);
    sel(&mut s, &[round]);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    assert_eq!(fill(&s, kids(&s, four)[1]), "#ff0000");
}

#[test]
fn group_global_edit_follows_isolation_and_artboard_options() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 400, "height": 400})).unwrap();
    let (one, two, far) = (pair(&mut s, 0.0, 0.0, 1.0), pair(&mut s, 300.0, 0.0, 1.0), pair(&mut s, 1100.0, 0.0, 1.0));
    assert_eq!(start(&mut s, one, json!({"artboards": "1"})), [one, two]);
    s.execute("select.globalEdit", &json!({})).unwrap();
    // In isolation mode, only the isolated group's groups are searched.
    let (a, b) = (pair(&mut s, 0.0, 450.0, 1.0), pair(&mut s, 100.0, 450.0, 1.0));
    let outer = group(&mut s, &[a, b]);
    s.execute("object.isolate", &json!({"id": outer.0})).unwrap();
    assert_eq!(start(&mut s, a, json!({})), [a, b], "not {one}, {two} nor {far}");
}

#[test]
fn global_edit_is_not_saved() {
    let mut s = session();
    let [a, ..] = squares(&mut s);
    start(&mut s, a, json!({}));
    let saved = s.execute("document.serialize", &json!({})).unwrap().to_string();
    assert!(!saved.to_ascii_lowercase().contains("globaledit"));
}

#[test]
fn a_stroke_weight_change_leaves_each_members_own_fill_and_stroke_colour() {
    let mut s = session();
    let [a, b, c, ..] = squares(&mut s);
    sel(&mut s, &[b]);
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#00ff00"})).unwrap();
    start(&mut s, a, json!({}));
    let (a_fill, c_fill) = (fill(&s, a), fill(&s, c));
    s.execute("stroke.set", &json!({"weight": 5})).unwrap();
    assert_eq!(fill(&s, b), "#0000ff", "the copy keeps its own fill");
    assert_eq!(stroke(&s, b), ("#00ff00".to_string(), 5.0), "and its own stroke colour, with the new weight");
    assert_eq!(stroke(&s, c).1, 5.0, "the weight is the model's, not scaled");
    assert_eq!((fill(&s, a), fill(&s, c)), (a_fill, c_fill));
}

#[test]
fn a_fill_colour_change_reaches_the_fills_and_not_the_strokes() {
    let mut s = session();
    let [a, b, c, ..] = squares(&mut s);
    sel(&mut s, &[b]);
    s.execute("paint.setStroke", &json!({"color": "#00ff00"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 3})).unwrap();
    start(&mut s, a, json!({}));
    let (a_stroke, c_stroke) = (stroke(&s, a), stroke(&s, c));
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    for id in [a, b, c] {
        assert_eq!(fill(&s, id), "#ff0000");
    }
    assert_eq!(stroke(&s, b), ("#00ff00".to_string(), 3.0), "the copy keeps its own stroke");
    assert_eq!((stroke(&s, a), stroke(&s, c)), (a_stroke, c_stroke));
}

#[test]
fn a_fill_or_stroke_added_or_removed_reaches_the_members_that_keep_their_others() {
    let mut s = session();
    let [a, b, c, ..] = squares(&mut s);
    sel(&mut s, &[b]);
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    // The small square has two fills: not the model's fills and strokes in order.
    sel(&mut s, &[c]);
    s.execute("appearance.addFill", &json!({})).unwrap();
    start(&mut s, a, json!({}));
    let kinds = |s: &Session, id| node(s, id).appearance.items.iter().map(|i| i.kind_name()).collect::<Vec<_>>();
    s.execute("appearance.addStroke", &json!({})).unwrap();
    assert_eq!(kinds(&s, a), ["fill", "stroke", "stroke"]);
    assert_eq!(kinds(&s, b), kinds(&s, a), "the copy gains the stroke");
    assert_eq!(fill(&s, b), "#0000ff", "and keeps its own fill");
    assert_eq!(kinds(&s, c), kinds(&s, a), "a member with other fills and strokes takes the model's list");
    s.execute("appearance.removeItem", &json!({"index": 0})).unwrap();
    assert_eq!(kinds(&s, a), ["stroke", "stroke"]);
    assert_eq!(kinds(&s, b), kinds(&s, a), "the copy loses its fill");
}

#[test]
fn undo_and_redo_drop_the_members_the_document_lacks() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 300.0, 0.0, 100.0, 100.0);
    let c = rect(&mut s, 500.0, 0.0, 20.0, 20.0);
    assert_eq!(start(&mut s, a, json!({})), [a, b, c]);
    // Undoing the making of the small square takes it out of the Global Edit, so a square made
    // under its freed id is not edited with the others.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(members(&s), Some(vec![a, b]));
    let d = rect(&mut s, 500.0, 0.0, 20.0, 20.0);
    assert_eq!(d, c, "the id is used again");
    let d_was = node(&s, d);
    sel(&mut s, &[a]);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    assert_eq!(fill(&s, b), "#ff0000", "Global Edit goes on with the members left");
    assert_eq!(node(&s, d), d_was);
    // Redoing the deletion of a member takes it out too; with one left, Global Edit ends.
    sel(&mut s, &[b]);
    s.execute("edit.clear", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(members(&s), Some(vec![a, b]), "every member is there again");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(members(&s), None);
}

/// Two 100 pt squares and a 20 pt one made after `begin`, all three in a Global Edit; `put_back`
/// takes the document back to before the small one. The small square leaves the Global Edit, and
/// a square made under its freed id is not edited with the others.
fn put_back_drops_the_member<T>(begin: impl FnOnce(&mut Session) -> T, put_back: impl FnOnce(&mut Session, T)) {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 300.0, 0.0, 100.0, 100.0);
    let t = begin(&mut s);
    let c = rect(&mut s, 500.0, 0.0, 20.0, 20.0);
    assert_eq!(start(&mut s, a, json!({})), [a, b, c]);
    put_back(&mut s, t);
    assert_eq!(members(&s), Some(vec![a, b]));
    let d = rect(&mut s, 500.0, 0.0, 20.0, 20.0);
    assert_eq!(d, c, "the id is used again");
    let d_was = node(&s, d);
    sel(&mut s, &[a]);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    assert_eq!(fill(&s, b), "#ff0000", "Global Edit goes on with the members left");
    assert_eq!(node(&s, d), d_was);
}

#[test]
fn a_cancelled_drag_scrub_or_undo_scope_drops_the_members_it_takes_away() {
    put_back_drops_the_member(|s| s.begin_interaction("Draw").unwrap(), |s, ()| s.cancel_interaction().unwrap());
    put_back_drops_the_member(Session::begin_undo_group, |s, ()| s.end_undo_group(true));
    put_back_drops_the_member(
        |s| {
            s.begin_undo_group();
            s.begin_undo_scope().unwrap()
        },
        |s, scope| {
            s.end_undo_scope(scope, true);
            s.end_undo_group(false);
        },
    );
}

#[test]
fn an_edit_that_takes_a_member_past_the_canvas_is_refused_whole() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 2.0, 2.0);
    let big = rect(&mut s, 0.0, 0.0, 2000.0, 2000.0);
    assert_eq!(start(&mut s, a, json!({})), [a, big]);
    let before = (node(&s, a), node(&s, big));
    let steps = s.doc().unwrap().history.undo.len();
    // 4000 pt keeps the small square in range; the square 1000 times its size moves 1000 times as
    // far, past `MAX_COORD`.
    let e = s.execute("object.move", &json!({"dx": 4000, "dy": 0})).unwrap_err().to_string();
    assert!(e.contains("out of range"), "{e}");
    assert_eq!((node(&s, a), node(&s, big)), before, "nothing moved");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);
    s.execute("object.move", &json!({"dx": 3000, "dy": 0})).unwrap();
    assert!(close(bounds(&s, big), Rect::new(3.0e6, 0.0, 3.0e6 + 2000.0, 2000.0)), "{:?}", bounds(&s, big));
}
