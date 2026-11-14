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
    assert!(s.doc().unwrap().global_edit.is_none());
    assert_eq!(crate::cmd::globaledit::highlights(&s).len(), 2, "with the square still selected, the objects it would take are highlighted again");
}

#[test]
fn selecting_one_object_highlights_the_similar_objects_before_global_edit_starts() {
    use crate::cmd::globaledit::highlights;
    let mut s = session();
    let [a, b, _, wide, _] = squares(&mut s);
    assert!(highlights(&s).is_empty(), "nothing selected");
    sel(&mut s, &[a]);
    assert_eq!(members(&s), None, "Global Edit has not started");
    assert_eq!(highlights(&s).len(), 2, "the copy and the smaller square");
    s.execute("select.globalEditOptions", &json!({"size": true})).unwrap();
    assert_eq!(highlights(&s).len(), 1, "the options narrow them: the copy only");
    s.execute("select.globalEditOptions", &json!({"size": false})).unwrap();
    assert_eq!(highlights(&s).len(), 2);
    sel(&mut s, &[b]);
    s.execute("edit.clear", &json!({})).unwrap();
    sel(&mut s, &[a]);
    assert_eq!(highlights(&s).len(), 1, "a deleted copy is no longer highlighted");
    sel(&mut s, &[wide]);
    assert!(highlights(&s).is_empty(), "the rectangle has no similar object");
    sel(&mut s, &[a, wide]);
    assert!(highlights(&s).is_empty(), "two objects selected");
}

#[test]
fn the_canvas_highlights_similar_objects_before_the_start_with_the_selection_tool_only() {
    let mut s = session();
    let [a, b, ..] = squares(&mut s);
    let outline = node(&s, b).stroke_path().unwrap();
    let shown = |s: &mut Session| {
        s.overlays(ViewInfo::default())
            .iter()
            .any(|o| matches!(o, vectorcraft_tools::Overlay::Path { path, width, .. } if *path == outline && *width == 1.0))
    };
    sel(&mut s, &[a]);
    s.select_tool("selection", ViewInfo::default()).unwrap();
    assert!(shown(&mut s), "the copy is highlighted with the Selection tool");
    s.select_tool("directSelection", ViewInfo::default()).unwrap();
    assert!(!shown(&mut s), "not with another tool before Global Edit starts");
    s.execute("select.globalEdit", &json!({})).unwrap();
    assert!(shown(&mut s), "while Global Edit is on, with any tool");
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
    // A group holding type is not offered, though another such group exists.
    let group = |s: &mut Session, x: f64| {
        let p = rect(s, x, 100.0, 10.0, 10.0);
        let t = NodeId(s.execute("text.create", &json!({"x": x, "y": 150, "text": "Hi"})).unwrap()["id"].as_u64().unwrap());
        group_of(s, &[p, t])
    };
    let (g, _) = (group(&mut s, 600.0), group(&mut s, 700.0));
    assert_not_offered(&mut s, g, "a group holding type");
}

fn enabled(s: &Session, id: &str) -> bool {
    command_specs().iter().find(|c| c.id == id).is_some_and(|c| (c.enabled)(s).is_ok())
}

/// With `id` selected, Start Global Edit is disabled, and running it changes nothing.
fn assert_not_offered(s: &mut Session, id: NodeId, what: &str) {
    sel(s, &[id]);
    assert!(!enabled(s, "select.globalEdit"), "{what}");
    let r = s.execute("select.globalEdit", &json!({}));
    assert!(matches!(&r, Err(EngineError::Disabled(_, why)) if why.contains("symbol instances and groups of them")), "{what}: {r:?}");
    assert!(s.doc().unwrap().global_edit.is_none(), "{what}");
}

/// An embedded 4 × 4 image at `x`, in `parent` (the default layer when `None`) → its id. The
/// selection stays as it is.
fn image(s: &mut Session, x: f64, parent: Option<NodeId>) -> NodeId {
    s.edit("Place", |d, _| {
        let id = d.alloc_id();
        let im = vectorcraft_doc::ImageObject {
            key: "pic".into(),
            width: 4,
            height: 4,
            xf: Affine::translate((x, 0.0)) * Affine::scale(10.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(parent.or(d.default_layer()), 0, Node::new(id, NodeKind::Image(Box::new(im))))?;
        Ok(id)
    })
    .unwrap()
}

#[test]
fn the_enabled_check_follows_edits_to_the_selected_group() {
    let mut s = session();
    let (g, _) = card(&mut s, 0.0, 1.0);
    card(&mut s, 300.0, 1.0);
    sel(&mut s, &[g]);
    assert!(enabled(&s, "select.globalEdit"));
    // The answer is kept between frames, and an edit of the selected group changes it.
    assert!(enabled(&s, "select.globalEdit"));
    image(&mut s, 0.0, Some(g));
    assert_eq!(s.doc().unwrap().selection.objects, [g]);
    assert!(!enabled(&s, "select.globalEdit"), "an image put into the selected group");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, [g]);
    assert!(enabled(&s, "select.globalEdit"), "after Undo");
}

#[test]
fn start_global_edit_is_offered_only_for_a_group_of_supported_objects() {
    let mut s = session();
    // Groups holding an image, groups holding a group that holds type, and empty groups; two of
    // each, so that each would have a group like it.
    for x in [0.0, 300.0] {
        let p = rect(&mut s, x, 100.0, 10.0, 10.0);
        let i = image(&mut s, x, None);
        let g = group_of(&mut s, &[p, i]);
        assert_not_offered(&mut s, g, "a group holding an image");
        let (p, q) = (rect(&mut s, x, 200.0, 10.0, 10.0), rect(&mut s, x + 20.0, 200.0, 10.0, 10.0));
        let t = NodeId(s.execute("text.create", &json!({"x": x, "y": 250, "text": "Hi"})).unwrap()["id"].as_u64().unwrap());
        let inner = group_of(&mut s, &[q, t]);
        let g = group_of(&mut s, &[p, inner]);
        assert_not_offered(&mut s, g, "a group holding a group that holds type");
        let empty = s
            .edit("Group", |d, _| {
                let id = d.alloc_id();
                d.insert(d.default_layer(), 0, Node::group(id, vec![]))?;
                Ok(id)
            })
            .unwrap();
        assert_not_offered(&mut s, empty, "an empty group");
    }
    // A group of supported objects is offered and starts; while on, Stop is offered whatever is
    // selected.
    let p = rect(&mut s, 600.0, 400.0, 10.0, 10.0);
    let t = NodeId(s.execute("text.create", &json!({"x": 600, "y": 450, "text": "Hi"})).unwrap()["id"].as_u64().unwrap());
    let typed = group_of(&mut s, &[p, t]);
    let (g, _) = card(&mut s, 0.0, 1.0);
    let (copy, _) = card(&mut s, 300.0, 1.0);
    sel(&mut s, &[g]);
    assert!(enabled(&s, "select.globalEdit"));
    assert_eq!(start(&mut s, g, json!({})), [g, copy]);
    sel(&mut s, &[typed]);
    assert!(enabled(&s, "select.globalEdit"), "Stop");
    assert_eq!(s.execute("select.globalEdit", &json!({})).unwrap()["active"], false);
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

#[test]
fn an_object_left_out_keeps_its_own_shape_while_the_others_follow() {
    let mut s = session();
    let [a, b, c, ..] = squares(&mut s);
    assert_eq!(start(&mut s, a, json!({})), [a, b, c]);
    let steps = s.doc().unwrap().history.undo.len();
    let r = s.execute("select.globalEditExclude", &json!({"id": b.0})).unwrap();
    assert_eq!(r, json!({"active": true, "count": 2, "ids": [a.0, c.0]}));
    assert_eq!(members(&s), Some(vec![a, c]));
    assert_eq!(crate::cmd::globaledit::highlights(&s).len(), 1, "the copy is no longer highlighted");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "not an undo step");
    assert_eq!(s.doc().unwrap().selection.objects, [a], "the selection stays");
    let b_was = node(&s, b);
    s.execute("object.setBounds", &json!({"height": 50, "reference": 0})).unwrap();
    assert!(close(bounds(&s, c), Rect::new(500.0, 0.0, 520.0, 10.0)), "{:?}", bounds(&s, c));
    assert_eq!(node(&s, b), b_was, "the copy left out keeps its shape");
}

/// A Shift-drag with the Selection tool from `from` to `to`.
fn shift_drag(s: &mut Session, from: (f64, f64), to: (f64, f64)) {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind};
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let steps = if from == to {
        vec![(PointerKind::Down, from), (PointerKind::Up, to)]
    } else {
        vec![(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)]
    };
    for (kind, (x, y)) in steps {
        s.pointer(&PointerEvent::new(kind, x, y).with_mods(Mods { shift: true, ..Mods::default() }), v).unwrap();
    }
}

/// A Shift-click with the Selection tool at (`x`, `y`).
fn shift_click(s: &mut Session, x: f64, y: f64) {
    shift_drag(s, (x, y), (x, y));
}

#[test]
fn a_shift_click_on_a_highlighted_object_leaves_it_out() {
    let mut s = session();
    let [a, b, c, ..] = squares(&mut s);
    start(&mut s, a, json!({}));
    // A Shift-drag that starts on the copy selects it and moves it with the selection, and leaves
    // the copy in.
    shift_drag(&mut s, (350.0, 50.0), (390.0, 50.0));
    assert_eq!(members(&s), Some(vec![a, b, c]), "a drag leaves nothing out");
    assert_eq!(s.doc().unwrap().selection.objects, [a, b]);
    assert!(bounds(&s, a).x0 > 0.0, "the selection moved: {:?}", bounds(&s, a));
    assert!(bounds(&s, b).x0 > 300.0, "the copy moved with it: {:?}", bounds(&s, b));
    s.execute("edit.undo", &json!({})).unwrap();
    sel(&mut s, &[a]);
    shift_click(&mut s, 350.0, 50.0);
    assert_eq!(members(&s), Some(vec![a, c]), "the copy is left out");
    assert_eq!(s.doc().unwrap().selection.objects, [a], "and not selected");
    // A Shift-click on the object the edit started from deselects it, as before.
    shift_click(&mut s, 50.0, 50.0);
    assert_eq!(members(&s), Some(vec![a, c]));
    assert!(s.doc().unwrap().selection.objects.is_empty());
    shift_click(&mut s, 510.0, 10.0);
    assert_eq!(members(&s), None, "one object left: Global Edit ends");
    assert!(s.doc().unwrap().selection.objects.is_empty());
    // With Global Edit off, a Shift-click selects, as before.
    shift_click(&mut s, 350.0, 50.0);
    assert_eq!(s.doc().unwrap().selection.objects, [b]);
}

#[test]
fn a_shift_click_inside_a_matched_group_leaves_the_whole_group_out() {
    let mut s = session();
    let (g, _) = card(&mut s, 0.0, 1.0);
    let (copy, _) = card(&mut s, 300.0, 1.0);
    let (small, _) = card(&mut s, 500.0, 0.5);
    assert_eq!(start(&mut s, g, json!({})), [g, copy, small]);
    // On the circle in the copy.
    shift_click(&mut s, 370.0, 20.0);
    assert_eq!(members(&s), Some(vec![g, small]));
    assert_eq!(s.doc().unwrap().selection.objects, [g]);
    // An object inside a member that is selected: a Shift-click deselects it, as before.
    let inner = kids(&s, small)[0];
    sel(&mut s, &[inner]);
    shift_click(&mut s, 505.0, 5.0);
    assert_eq!(members(&s), Some(vec![g, small]));
}

#[test]
fn a_shift_click_on_a_member_inside_a_selected_group_deselects_the_group() {
    let mut s = session();
    let [a, b, c, wide, _] = squares(&mut s);
    let pair = group_of(&mut s, &[b, wide]);
    let all = start(&mut s, a, json!({}));
    assert_eq!(all, [a, c, b], "the copy in the group comes after the small square");
    sel(&mut s, &[pair]);
    assert_eq!(crate::cmd::globaledit::highlights(&s).len(), 2, "the copy inside the selected group is not highlighted");
    shift_click(&mut s, 350.0, 50.0);
    assert_eq!(members(&s), Some(all), "nothing is left out");
    assert!(s.doc().unwrap().selection.objects.is_empty(), "the group is deselected");
}

#[test]
fn leaving_out_all_but_one_ends_global_edit_and_the_first_object_stays() {
    let mut s = session();
    let [a, b, c, wide, _] = squares(&mut s);
    let e = s.execute("select.globalEditExclude", &json!({"id": b.0})).unwrap_err().to_string();
    assert!(e.contains("Global Edit is off"), "{e}");
    start(&mut s, a, json!({}));
    for id in [a, wide] {
        let e = s.execute("select.globalEditExclude", &json!({"id": id.0})).unwrap_err().to_string();
        assert!(e.contains("other than the one it started from"), "{id}: {e}");
    }
    assert_eq!(members(&s), Some(vec![a, b, c]), "nothing left out");
    s.execute("select.globalEditExclude", &json!({"id": c.0})).unwrap();
    let r = s.execute("select.globalEditExclude", &json!({"id": b.0})).unwrap();
    assert_eq!(r, json!({"active": false, "count": 0, "ids": []}));
    assert_eq!(members(&s), None, "one object left: Global Edit ends");
    sel(&mut s, &[wide]);
    assert!(crate::cmd::globaledit::highlights(&s).is_empty());
}

#[test]
fn a_group_left_out_keeps_its_objects_while_the_others_follow() {
    let mut s = session();
    let (g, [_, dot, _]) = card(&mut s, 0.0, 1.0);
    let (copy, [_, cdot, _]) = card(&mut s, 300.0, 1.0);
    let (small, [_, sdot, _]) = card(&mut s, 500.0, 0.5);
    assert_eq!(start(&mut s, g, json!({})), [g, copy, small]);
    s.execute("select.globalEditExclude", &json!({"id": copy.0})).unwrap();
    let was = fill(&s, cdot);
    sel(&mut s, &[dot]);
    s.execute("paint.setFill", &json!({"color": "#12ab34"})).unwrap();
    assert_eq!((fill(&s, sdot), fill(&s, cdot)), ("#12ab34".to_string(), was));
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

fn group_of(s: &mut Session, objects: &[NodeId]) -> NodeId {
    sel(s, objects);
    NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap())
}

/// A group of a square, a circle and a bar, `k` times the size, at `x` → the group and its objects.
fn card(s: &mut Session, x: f64, k: f64) -> (NodeId, [NodeId; 3]) {
    let sq = rect(s, x, 0.0, 40.0 * k, 40.0 * k);
    let dot = NodeId(
        s.execute("shape.ellipse", &json!({"x": x + 50.0 * k, "y": 0, "width": 40.0 * k, "height": 40.0 * k})).unwrap()["id"].as_u64().unwrap(),
    );
    let bar = rect(s, x, 50.0 * k, 90.0 * k, 20.0 * k);
    (group_of(s, &[sq, dot, bar]), [sq, dot, bar])
}

fn kids(s: &Session, g: NodeId) -> Vec<NodeId> {
    node(s, g).children().unwrap().iter().map(|c| c.id).collect()
}

#[test]
fn start_global_edit_on_a_group_takes_the_groups_like_it() {
    let mut s = session();
    let (g, [sq, ..]) = card(&mut s, 0.0, 1.0);
    let (copy, _) = card(&mut s, 300.0, 1.0);
    let (small, _) = card(&mut s, 500.0, 0.5);
    // Two of the three objects; the same three with the bar moved; a lone square like the first.
    let (p, q) = (rect(&mut s, 0.0, 300.0, 40.0, 40.0), rect(&mut s, 0.0, 400.0, 90.0, 20.0));
    let pair = group_of(&mut s, &[p, q]);
    let (moved, [.., bar]) = card(&mut s, 300.0, 1.0);
    sel(&mut s, &[bar]);
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    rect(&mut s, 600.0, 300.0, 40.0, 40.0);
    assert_eq!(start(&mut s, g, json!({})), [g, copy, small], "not the pair {pair} nor the group with the bar moved {moved}");
    // A group alone among groups of other contents: nothing to edit with it.
    sel(&mut s, &[pair]);
    s.execute("select.globalEdit", &json!({})).unwrap();
    let e = s.execute("select.globalEdit", &json!({})).unwrap_err().to_string();
    assert!(e.contains("no similar objects"), "{e}");
    // Size leaves out the smaller group; Appearance looks at each object in the groups.
    assert_eq!(start(&mut s, g, json!({"size": true})), [g, copy]);
    s.execute("select.globalEdit", &json!({})).unwrap();
    sel(&mut s, &[sq]);
    s.execute("paint.setFill", &json!({"color": "#12ab34"})).unwrap();
    sel(&mut s, &[g]);
    let e = s.execute("select.globalEdit", &json!({"appearance": true})).unwrap_err().to_string();
    assert!(e.contains("no similar objects"), "the square in the first group has another fill now: {e}");
}

#[test]
fn an_edit_inside_one_group_reaches_the_same_object_in_the_others_as_one_undo_step() {
    let mut s = session();
    let (g, [sq, dot, bar]) = card(&mut s, 0.0, 1.0);
    let (copy, [csq, cdot, cbar]) = card(&mut s, 300.0, 1.0);
    let (_, [ssq, sdot, _]) = card(&mut s, 500.0, 0.5);
    start(&mut s, g, json!({}));
    let before = (*s.doc().unwrap().doc).clone();
    let steps = s.doc().unwrap().history.undo.len();
    let (sq_fill, bar_fill) = (fill(&s, csq), fill(&s, cbar));
    // The circle inside the first group, selected on its own.
    sel(&mut s, &[dot]);
    assert_eq!(crate::cmd::globaledit::highlights(&s).len(), 2, "the other groups are highlighted, not the one being edited");
    s.execute("paint.setFill", &json!({"color": "#12ab34"})).unwrap();
    assert_eq!((fill(&s, cdot), fill(&s, sdot)), ("#12ab34".to_string(), "#12ab34".to_string()));
    assert_eq!((fill(&s, csq), fill(&s, cbar)), (sq_fill, bar_fill), "the other objects keep theirs");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "one undo step");
    // The square's height halved from the top: each copy's in proportion.
    sel(&mut s, &[sq]);
    s.execute("object.setBounds", &json!({"height": 20, "reference": 0})).unwrap();
    assert!(close(bounds(&s, csq), Rect::new(300.0, 0.0, 340.0, 20.0)), "{:?}", bounds(&s, csq));
    assert!(close(bounds(&s, ssq), Rect::new(500.0, 0.0, 520.0, 10.0)), "{:?}", bounds(&s, ssq));
    assert!(placed(&s, bar, cbar, Affine::translate((300.0, 0.0))), "the bar is left alone");
    assert_eq!(kids(&s, copy), [csq, cdot, cbar], "each object keeps its id");
    s.execute("edit.undo", &json!({})).unwrap();
    crate::tests_large_edits::undo_gives_back(&mut s, &before);
}

#[test]
fn an_object_added_to_the_group_is_added_to_the_others() {
    let mut s = session();
    let (g, [sq, ..]) = card(&mut s, 0.0, 1.0);
    let (copy, [csq, ..]) = card(&mut s, 300.0, 1.0);
    let (small, _) = card(&mut s, 500.0, 0.5);
    start(&mut s, g, json!({}));
    sel(&mut s, &[sq]);
    let added = s
        .edit("add", |d, _| {
            let id = d.alloc_id();
            let path = vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 80.0, 10.0, 90.0));
            d.insert(Some(g), usize::MAX, Node::path(id, path, vectorcraft_doc::Appearance::default_art()))
                .map_err(|e| EngineError::Other(e.to_string()))
        })
        .unwrap();
    let (theirs, smalls) = (kids(&s, copy), kids(&s, small));
    assert_eq!((theirs.len(), smalls.len()), (4, 4));
    assert_eq!(theirs.first(), Some(&csq), "the member keeps its own ids in order");
    assert!(placed(&s, added, theirs[3], Affine::translate((300.0, 0.0))));
    assert!(placed(&s, added, smalls[3], Affine::translate((500.0, 0.0)) * Affine::scale(0.5)));
}

#[test]
fn global_edit_takes_at_most_max_members() {
    use crate::cmd::globaledit::MAX_MEMBERS;
    let mut s = session();
    let first = s
        .edit("many", |d, _| {
            let layer = d.default_layer();
            let mut first = None;
            for i in 0..MAX_MEMBERS + 5 {
                let (x, y) = ((i % 70) as f64 * 10.0, (i / 70 % 50) as f64 * 10.0);
                let id = d.alloc_id();
                let path = vectorcraft_geom::shapes::rectangle(Rect::new(x, y, x + 8.0, y + 8.0));
                d.insert(layer, usize::MAX, Node::path(id, path, vectorcraft_doc::Appearance::default_art()))
                    .map_err(|e| EngineError::Other(e.to_string()))?;
                first.get_or_insert(id);
            }
            first.ok_or_else(|| EngineError::Other("none".into()))
        })
        .unwrap();
    assert_eq!(start(&mut s, first, json!({})).len(), MAX_MEMBERS);
}

#[test]
fn groups_nested_as_deep_as_a_document_allows_are_edited_together() {
    let mut s = session();
    let nest = |s: &mut Session, x: f64| {
        let inner = rect(s, x, 0.0, 10.0, 10.0);
        let side = rect(s, x + 20.0, 0.0, 10.0, 10.0);
        let mut top = group_of(s, &[inner, side]);
        // Group until the document refuses to go deeper.
        for _ in 0..2 * vectorcraft_doc::MAX_DEPTH {
            sel(s, &[top]);
            match s.execute("object.group", &json!({})) {
                Ok(r) => top = NodeId(r["id"].as_u64().unwrap()),
                Err(_) => break,
            }
        }
        (top, inner)
    };
    let ((g, inner), (other, other_inner)) = (nest(&mut s, 0.0), nest(&mut s, 300.0));
    assert_eq!(start(&mut s, g, json!({})), [g, other]);
    sel(&mut s, &[inner]);
    s.execute("paint.setFill", &json!({"color": "#12ab34"})).unwrap();
    assert_eq!(fill(&s, other_inner), "#12ab34");
}
