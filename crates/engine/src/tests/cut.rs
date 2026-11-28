//! Tests for the cutting commands: Mirror & Cut, Line Cut and Rectangle Cut.

use serde_json::{Value, json};
use vectorcraft_doc::NodeKind;
use vectorcraft_geom::{FillRule, PathData, Rect};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn path(s: &mut Session, pts: &[(f64, f64)], closed: bool) -> NodeId {
    let anchors: Vec<Value> = pts.iter().map(|(x, y)| json!({"x": x, "y": y})).collect();
    id_of(&s.execute("path.create", &json!({"anchors": anchors, "closed": closed})).unwrap())
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn ids(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|x| NodeId(x.as_u64().unwrap())).collect()
}

fn geometry(s: &Session, id: NodeId) -> (PathData, FillRule) {
    let n = s.doc().unwrap().doc.node(id).cloned().unwrap();
    match &n.kind {
        NodeKind::Path { path, rule, .. } => (path.clone(), *rule),
        NodeKind::Compound { children, rule } => {
            (PathData::new(children.iter().flat_map(|c| c.path_data().unwrap().subpaths.clone()).collect()), *rule)
        }
        _ => panic!("not a path"),
    }
}

fn area(s: &Session, id: NodeId) -> f64 {
    let (p, r) = geometry(s, id);
    vectorcraft_pathops::area(&p, r)
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    geometry(s, id).0.bounds().unwrap()
}

fn near(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(p, q)| (p - q).abs() < 0.05)
}

fn undo_depth(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

#[test]
fn line_cut_splits_a_shape_in_one_undo_step() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    select(&mut s, &[r]);
    let before = undo_depth(&s);
    let out = s.execute("path.lineCut", &json!({"from": [30, -10], "to": [30, 10]})).unwrap();
    let pieces = ids(&out);
    assert_eq!(pieces.len(), 2);
    let mut areas: Vec<f64> = pieces.iter().map(|id| area(&s, *id)).collect();
    areas.sort_by(f64::total_cmp);
    assert!((areas[0] - 3000.0).abs() < 1.0 && (areas[1] - 7000.0).abs() < 1.0, "{areas:?}");
    // The pieces are selected, and the cut is one undo step.
    assert_eq!(s.doc().unwrap().selection.len(), 2);
    assert_eq!(undo_depth(&s), before + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((area(&s, r) - 10_000.0).abs() < 1.0);
}

#[test]
fn line_cut_keeps_compound_holes_and_splits_open_paths() {
    let mut s = session();
    let outer = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    // Wound against the outer rectangle, so the non-zero compound has a hole.
    let hole = path(&mut s, &[(40.0, 40.0), (40.0, 60.0), (60.0, 60.0), (60.0, 40.0)], true);
    select(&mut s, &[outer, hole]);
    let c = id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap());
    assert!((area(&s, c) - 9600.0).abs() < 1.0);
    let line = path(&mut s, &[(0.0, 200.0), (100.0, 200.0)], false);
    select(&mut s, &[c, line]);
    // A vertical line at x = 20 misses the hole: the right piece keeps it.
    let out = s.execute("path.lineCut", &json!({"from": [20, 0], "to": [20, 300]})).unwrap();
    let pieces = ids(&out);
    assert_eq!(pieces.len(), 4, "two compound pieces and two line pieces");
    let doc = &s.doc().unwrap().doc;
    let compounds: Vec<NodeId> = pieces.iter().copied().filter(|id| matches!(doc.node(*id).unwrap().kind, NodeKind::Compound { .. })).collect();
    assert_eq!(compounds.len(), 2, "both pieces of a compound path stay compound paths");
    let mut areas: Vec<f64> = compounds.iter().map(|id| area(&s, *id)).collect();
    areas.sort_by(f64::total_cmp);
    assert!((areas[0] - 2000.0).abs() < 1.0 && (areas[1] - 7600.0).abs() < 1.0, "{areas:?}");
    let open: Vec<Rect> = pieces.iter().filter(|id| !compounds.contains(id)).map(|id| bounds(&s, *id)).collect();
    assert!(open.iter().any(|b| (b.x1 - 20.0).abs() < 0.01) && open.iter().any(|b| (b.x0 - 20.0).abs() < 0.01), "{open:?}");
}

#[test]
fn line_cut_that_misses_changes_nothing() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    select(&mut s, &[r]);
    let before = undo_depth(&s);
    let out = s.execute("path.lineCut", &json!({"from": [200, 0], "to": [200, 10]})).unwrap();
    assert!(ids(&out).is_empty());
    assert_eq!(undo_depth(&s), before);
    assert!(s.execute("path.lineCut", &json!({"from": [5, 5], "to": [5, 5]})).is_err());
    assert!(s.execute("path.lineCut", &json!({"from": [5, 5]})).is_err());
}

#[test]
fn rect_cut_rejects_non_numeric_rectangle_elements_without_modifying_art() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    select(&mut s, &[id]);
    let original = bounds(&s, id);
    let history = undo_depth(&s);
    // The old parser dropped the string and used the four remaining values.
    for params in [json!([0, "ignored", 10, 50, 50]), json!([0, 10, 50, null, 50]), json!([0, 10, 50, 50, 5])] {
        assert!(s.execute("path.rectCut", &json!({"rect": params})).is_err());
        assert!(near(bounds(&s, id), original), "failed cut must not change geometry");
        assert_eq!(undo_depth(&s), history, "failed cut must not add undo history");
    }
    // Proper rectangular cuts still work.
    s.execute("path.rectCut", &json!({"rect": [10, 10, 70, 70]})).unwrap();
    assert!(near(bounds(&s, id), Rect::new(10.0, 10.0, 80.0, 80.0)));
}

#[test]
fn rect_cut_crops_to_real_geometry() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let outside = rect(&mut s, 300.0, 300.0, 10.0, 10.0);
    let inside = rect(&mut s, 60.0, 20.0, 10.0, 10.0);
    let line = path(&mut s, &[(0.0, 50.0), (200.0, 50.0)], false);
    select(&mut s, &[a, outside, inside, line]);
    let out = s.execute("path.rectCut", &json!({"rect": [50, 0, 100, 200]})).unwrap();
    assert_eq!(out["removed"], 1);
    let doc = &s.doc().unwrap().doc;
    assert!(doc.node(outside).is_none(), "art wholly outside is deleted");
    assert!(matches!(doc.node(a).unwrap().kind, NodeKind::Path { .. }), "no clipping mask");
    assert!(near(bounds(&s, a), Rect::new(50.0, 0.0, 100.0, 100.0)));
    assert!((area(&s, a) - 5000.0).abs() < 1.0);
    assert!(near(bounds(&s, inside), Rect::new(60.0, 20.0, 70.0, 30.0)), "art inside is untouched");
    assert!(near(bounds(&s, line), Rect::new(50.0, 50.0, 150.0, 50.0)));
    // Two corners work too.
    select(&mut s, &[a]);
    s.execute("path.rectCut", &json!({"from": [100, 100], "to": [75, 50]})).unwrap();
    assert!(near(bounds(&s, a), Rect::new(75.0, 50.0, 100.0, 100.0)));
    assert!(s.execute("path.rectCut", &json!({"rect": [0, 0, 0, 10]})).is_err());
    assert!(s.execute("path.rectCut", &json!({"rect": [0, 0, 10]})).is_err());
}

#[test]
fn rect_cut_keeps_a_long_line_through_a_narrow_rectangle() {
    // A 200 pt line used to be tested every 200 / 24 pt (at 400 and 408.33 near here), so a
    // rectangle between two of those points kept nothing of it.
    let mut s = session();
    let line = path(&mut s, &[(300.0, 150.0), (500.0, 150.0)], false);
    select(&mut s, &[line]);
    s.execute("path.rectCut", &json!({"rect": [402, 100, 4, 100]})).unwrap();
    assert!(near(bounds(&s, line), Rect::new(402.0, 150.0, 406.0, 150.0)));
}

#[test]
fn mirror_cut_makes_a_symmetric_shape() {
    let mut s = session();
    // A right triangle: mirrored at x = 50 keeping the left, it becomes a house-like pentagon.
    let t = path(&mut s, &[(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)], true);
    select(&mut s, &[t]);
    let out = s.execute("path.mirrorCut", &json!({"axis": "vertical", "from": [50, 0]})).unwrap();
    assert_eq!(ids(&out), vec![t]);
    let (pd, _) = geometry(&s, t);
    assert_eq!(pd.subpaths.len(), 1, "both halves are joined into one closed path");
    assert!(pd.subpaths[0].closed);
    assert!((area(&s, t) - 7500.0).abs() < 1.0);
    assert!(near(bounds(&s, t), Rect::new(0.0, 0.0, 100.0, 100.0)));
    // Keeping the right half instead.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("path.mirrorCut", &json!({"axis": "vertical", "from": [50, 0], "keep": "right"})).unwrap();
    assert!((area(&s, t) - 2500.0).abs() < 1.0);
    assert!(near(bounds(&s, t), Rect::new(0.0, 0.0, 100.0, 50.0)));
}

#[test]
fn mirror_cut_axes_and_sides() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 40.0, 100.0);
    let gone = rect(&mut s, 300.0, 0.0, 10.0, 10.0);
    select(&mut s, &[a, gone]);
    // A free axis at x = 50 (drawn upward): art on the discarded side is deleted.
    let out = s.execute("path.mirrorCut", &json!({"axis": "free", "from": [50, 100], "to": [50, 0], "keep": "left"})).unwrap();
    assert_eq!(out["removed"], 1);
    assert!(s.doc().unwrap().doc.node(gone).is_none());
    // Wholly on the kept side: the shape and its reflection, one object.
    let (pd, _) = geometry(&s, a);
    assert_eq!(pd.subpaths.len(), 2);
    assert!(near(bounds(&s, a), Rect::new(0.0, 0.0, 100.0, 100.0)));
    // A horizontal axis through the selection's centre keeps the top by default.
    let b = rect(&mut s, 0.0, 200.0, 100.0, 60.0);
    select(&mut s, &[b]);
    s.execute("path.mirrorCut", &json!({"axis": "horizontal"})).unwrap();
    assert!(near(bounds(&s, b), Rect::new(0.0, 200.0, 100.0, 260.0)));
    assert!(s.execute("path.mirrorCut", &json!({"axis": "vertical", "keep": "top"})).is_err());
    assert!(s.execute("path.mirrorCut", &json!({"axis": "diagonal"})).is_err());
    assert!(s.execute("path.mirrorCut", &json!({"axis": "free", "from": [1, 1], "to": [1, 1]})).is_err());
}

#[test]
fn mirror_cut_closes_an_open_half() {
    let mut s = session();
    let p = path(&mut s, &[(50.0, 0.0), (0.0, 50.0), (50.0, 100.0)], false);
    select(&mut s, &[p]);
    s.execute("path.mirrorCut", &json!({"axis": "vertical", "from": [50, 0]})).unwrap();
    let (pd, _) = geometry(&s, p);
    assert_eq!(pd.subpaths.len(), 1);
    assert!(pd.subpaths[0].closed);
    assert_eq!(pd.subpaths[0].anchors.len(), 4);
    assert!(near(bounds(&s, p), Rect::new(0.0, 0.0, 100.0, 100.0)));
}

#[test]
fn cut_commands_need_paths() {
    let mut s = session();
    assert!(s.execute("path.lineCut", &json!({"from": [0, 0], "to": [1, 1]})).is_err(), "nothing selected");
    let t = id_of(&s.execute("text.create", &json!({"x": 10, "y": 10, "text": "A"})).unwrap());
    select(&mut s, &[t]);
    assert!(s.execute("path.lineCut", &json!({"from": [0, 0], "to": [1, 1]})).is_err());
}

mod tools {
    use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

    use super::*;
    use crate::tooling::ViewInfo;

    fn view() -> ViewInfo {
        ViewInfo { smart_guides: false, ..Default::default() }
    }

    fn press(s: &mut Session, kind: PointerKind, x: f64, y: f64) {
        s.pointer(&PointerEvent::new(kind, x, y).with_mods(Mods::default()), view()).unwrap();
    }

    #[test]
    fn line_cut_tool_previews_live_and_commits_one_step() {
        let mut s = session();
        let r = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
        select(&mut s, &[r]);
        let before = undo_depth(&s);
        s.select_tool("lineCut", view()).unwrap();
        press(&mut s, PointerKind::Down, 50.0, -20.0);
        press(&mut s, PointerKind::Drag, 50.0, 40.0);
        // The preview already shows the cut.
        assert!(s.in_interaction());
        assert_eq!(s.doc().unwrap().selection.len(), 2);
        press(&mut s, PointerKind::Drag, 50.0, 120.0);
        press(&mut s, PointerKind::Up, 50.0, 120.0);
        assert!(!s.in_interaction());
        assert_eq!(undo_depth(&s), before + 1);
        assert_eq!(s.journal.last().map(|j| j.0.as_str()), Some("path.lineCut"));
        assert!((area(&s, r) - 5000.0).abs() < 1.0);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!((area(&s, r) - 10_000.0).abs() < 1.0);
    }

    #[test]
    fn rect_and_mirror_tools_cut_and_escape_restores() {
        let mut s = session();
        let r = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
        select(&mut s, &[r]);
        s.select_tool("rectCut", view()).unwrap();
        press(&mut s, PointerKind::Down, 50.0, 50.0);
        press(&mut s, PointerKind::Drag, 150.0, 150.0);
        assert!(near(bounds(&s, r), Rect::new(50.0, 50.0, 100.0, 100.0)));
        s.tool_key(ToolKey::Escape, Mods::default(), view()).unwrap();
        assert!(!s.in_interaction());
        assert!(near(bounds(&s, r), Rect::new(0.0, 0.0, 100.0, 100.0)));
        // Mirror & Cut with a vertical axis placed by a click at x = 80.
        s.select_tool("mirrorCut", view()).unwrap();
        s.set_tool_option("axis", &json!("vertical"));
        press(&mut s, PointerKind::Down, 80.0, 50.0);
        press(&mut s, PointerKind::Up, 80.0, 50.0);
        assert!(near(bounds(&s, r), Rect::new(0.0, 0.0, 160.0, 100.0)));
        assert_eq!(s.journal.last().map(|j| j.0.as_str()), Some("path.mirrorCut"));
    }
}
