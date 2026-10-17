//! Type on a path (#429): Type on a Path Options keep its brackets, flip it, align it to the path
//! and space it; the selection tools drag its brackets to move it along the path and across it.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeKind, PathAlign, TextKind, TextObject};
use vectorcraft_geom::{Point, Rect, Shape};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

/// 12 pt type on a path along `d` (SVG path data), starting where the path is nearest `at`.
fn path_type(s: &mut Session, d: &str, at: [f64; 2], text: &str) -> NodeId {
    let pid = s.execute("path.create", &json!({ "d": d })).unwrap()["id"].as_u64().unwrap();
    let r = s.execute("text.createInPath", &json!({"path": pid, "mode": "onPath", "text": text, "size": 12, "at": at})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    id
}

/// Type on the 400 pt line from (100, 300) to (500, 300), from its middle on.
fn on_line(s: &mut Session) -> NodeId {
    path_type(s, "M100 300 L500 300", [300.0, 310.0], "Slide")
}

fn text(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        k => panic!("not text: {k:?}"),
    }
}

/// The type's start and end brackets as stored.
fn brackets(s: &Session, id: NodeId) -> (f64, Option<f64>) {
    match text(s, id).kind {
        TextKind::OnPath { start, end, .. } => (start, end),
        k => panic!("not type on a path: {k:?}"),
    }
}

/// The glyphs' ink bounds in the document.
fn ink(s: &Session, id: NodeId) -> Rect {
    let t = text(s, id);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    lay.glyphs
        .iter()
        .filter(|g| !g.outline.elements().is_empty())
        .map(|g| t.xf.transform_rect_bbox(g.outline.bounding_box()))
        .reduce(|a, b| a.union(b))
        .unwrap()
}

/// The glyph origins along the path, in reading order.
fn origins(s: &Session, id: NodeId) -> Vec<Point> {
    let t = text(s, id);
    vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).glyphs.iter().map(|g| t.xf * g.origin).collect()
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn last_step(s: &Session) -> String {
    s.doc().unwrap().history.undo.last().unwrap().label.clone()
}

/// How many steps there are to undo and to redo, and whether the document has unsaved changes.
fn history(s: &Session) -> (usize, usize, bool) {
    let st = s.doc().unwrap();
    (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
}

/// Leave a step to redo (a rectangle drawn and undone, which keeps the selection) and mark the
/// document saved.
fn with_a_step_to_redo(s: &mut Session) {
    s.execute("shape.rectangle", &json!({"x": 700, "y": 500, "width": 10, "height": 10})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.doc_mut().unwrap().mark_saved();
}

/// Press, drag and release `tool` from `from` to `to`.
fn drag(s: &mut Session, tool: &str, from: Point, to: Point, mods: Mods) {
    let v = ViewInfo::default();
    s.select_tool(tool, v).unwrap();
    for (kind, p) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
        s.pointer(&PointerEvent::new(kind, p.x, p.y).with_mods(mods), v).unwrap();
    }
    assert!(!s.in_interaction());
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

#[test]
fn options_keep_the_brackets() {
    let mut s = session();
    let id = on_line(&mut s);
    assert!(near(brackets(&s, id).0, 0.5));
    // The query (what the dialog shows) has the current options.
    let mut q = s.execute("type.pathOptions", &json!({})).unwrap();
    assert!(near(q["start"].as_f64().unwrap(), 0.5), "{q}");
    q.as_object_mut().unwrap().remove("start");
    assert_eq!(q, json!({"end": null, "flip": false, "effect": "rainbow", "alignToPath": "baseline", "spacing": 0.0}));
    let start = brackets(&s, id).0;
    // OK with the dialog's values (effect, flip, align, spacing) leaves the type where it is.
    let before = origins(&s, id);
    s.execute("type.pathOptions", &json!({"effect": "rainbow", "flip": false, "alignToPath": "baseline", "spacing": 0})).unwrap();
    assert_eq!(brackets(&s, id), (start, None));
    assert_eq!(origins(&s, id), before);
    // So does a new effect.
    s.execute("type.pathOptions", &json!({"effect": "skew"})).unwrap();
    assert_eq!(brackets(&s, id), (start, None));
    assert_eq!(text(&s, id).path_effect, vectorcraft_doc::PathEffect::Skew);
    // The end bracket is set, and put back at the end of the path.
    s.execute("type.pathOptions", &json!({"end": 0.9})).unwrap();
    assert_eq!(brackets(&s, id), (start, Some(0.9)));
    s.execute("type.pathOptions", &json!({"end": null})).unwrap();
    assert_eq!(brackets(&s, id), (start, None));
    for bad in [json!({"start": "x"}), json!({"effect": "nope"}), json!({"alignToPath": "top"})] {
        assert!(s.execute("type.pathOptions", &bad).is_err(), "{bad}");
    }
}

/// Type on a Path Options given the values the type has (OK in the dialog with nothing changed, the
/// effect it has from the Type on a Path menu, the end bracket at the end of the path, or the
/// query's own reply) records no undo step: the step to redo stays and the document stays
/// unmodified. It still replies with the options. A flip is one undo step.
#[test]
fn options_the_type_has_record_no_undo_step() {
    let mut s = session();
    on_line(&mut s);
    with_a_step_to_redo(&mut s);
    let before = history(&s);
    assert_eq!((before.1, before.2), (1, false), "a step to redo, and nothing unsaved");
    let q = s.execute("type.pathOptions", &json!({})).unwrap();
    let ok = json!({"effect": "rainbow", "flip": false, "alignToPath": "baseline", "spacing": 0});
    for p in [ok, json!({"effect": "rainbow"}), json!({"end": null}), q.clone()] {
        assert_eq!(s.execute("type.pathOptions", &p).unwrap(), q, "{p}");
        assert_eq!(history(&s), before, "{p}");
    }
    s.execute("type.pathOptions", &json!({"flip": true})).unwrap();
    assert_eq!(history(&s), (before.0 + 1, 0, true));
}

#[test]
fn flip_turns_the_type_to_the_other_side_of_its_stretch() {
    let mut s = session();
    let id = on_line(&mut s);
    let up = ink(&s, id);
    assert!(up.y1 < 301.0 && up.x0 > 299.0, "above the path from its middle: {up:?}");
    let n = undo_len(&s);
    let r = s.execute("type.pathOptions", &json!({"flip": true})).unwrap();
    assert_eq!(undo_len(&s), n + 1);
    // The path runs the other way and the brackets swap ends: the type keeps the right half.
    assert!(near(r["start"].as_f64().unwrap(), 0.0) && near(r["end"].as_f64().unwrap(), 0.5), "{r}");
    let down = ink(&s, id);
    assert!(down.y0 > 299.0 && down.x0 > 299.0, "below the path, on the same half: {down:?}");
    let o = origins(&s, id);
    assert!(o.first().unwrap().x > o.last().unwrap().x, "reading right to left: {o:?}");
    // Flipped back, it is as it was.
    s.execute("type.pathOptions", &json!({"flip": true})).unwrap();
    let (start, end) = brackets(&s, id);
    assert!(near(start, 0.5) && end.is_none(), "{start} {end:?}");
    assert!(ink(&s, id).y1 < 301.0);
}

#[test]
fn a_flip_round_a_closed_path_keeps_the_type_in_place() {
    let mut s = session();
    let id = path_type(&mut s, "M100 100 L300 100 L300 300 L100 300 Z", [200.0, 90.0], "Ring");
    let (start, _) = brackets(&s, id);
    assert!(near(start, 0.125), "{start}");
    let first = origins(&s, id)[0];
    s.execute("type.pathOptions", &json!({"flip": true})).unwrap();
    // Once round from the same point, the other way.
    let (start, end) = brackets(&s, id);
    assert!(near(start, 0.875) && end.is_none(), "{start} {end:?}");
    assert!(origins(&s, id)[0].distance(first) < 8.0);
}

#[test]
fn align_to_path_and_spacing() {
    let mut s = session();
    let id = on_line(&mut s);
    s.execute("type.pathOptions", &json!({"alignToPath": "ascender"})).unwrap();
    assert_eq!(text(&s, id).path_align, PathAlign::Ascender);
    let hang = ink(&s, id);
    assert!(hang.y0 > 299.0, "hangs below the path: {hang:?}");
    s.execute("type.pathOptions", &json!({"alignToPath": "center"})).unwrap();
    let mid = ink(&s, id);
    assert!(mid.y0 < 300.0 && mid.y1 > 300.0, "the path runs through it: {mid:?}");
    // Spacing closes glyphs up round the outside of a curve.
    let id = path_type(&mut s, "M100 400 A60 60 0 0 1 220 400", [100.0, 400.0], "Round the bend");
    let spread = |s: &Session| {
        let o = origins(s, id);
        o.first().unwrap().distance(*o.last().unwrap())
    };
    let loose = spread(&s);
    s.execute("type.pathOptions", &json!({"spacing": 6})).unwrap();
    assert!(spread(&s) < loose - 1.0, "{} < {loose}", spread(&s));
    assert_eq!(s.execute("type.pathOptions", &json!({})).unwrap()["spacing"], json!(6.0));
}

#[test]
fn dragging_the_centre_bracket_slides_the_type_and_across_the_path_flips_it() {
    let mut s = session();
    let id = on_line(&mut s);
    let n = undo_len(&s);
    // The centre bracket stands midway between the start (300) and the end of the path (500).
    drag(&mut s, "selection", Point::new(400.0, 295.0), Point::new(440.0, 295.0), Mods::default());
    assert_eq!(undo_len(&s), n + 1, "one step");
    assert_eq!(last_step(&s), "Move Type on a Path");
    let (start, end) = brackets(&s, id);
    assert!(near(start, 0.6) && end.is_none(), "{start} {end:?}");
    assert!(ink(&s, id).x0 > 339.0);
    // Direct Selection drags it too; Cmd/Ctrl held below the path only slides it.
    drag(&mut s, "directSelection", Point::new(420.0, 295.0), Point::new(380.0, 330.0), Mods { cmd: true, ..Mods::default() });
    let (start, end) = brackets(&s, id);
    assert!(near(start, 0.5) && end.is_none(), "{start} {end:?}");
    assert!(ink(&s, id).y1 < 301.0, "still above the path");
    // Across the path: flipped, on the same stretch.
    drag(&mut s, "selection", Point::new(400.0, 295.0), Point::new(400.0, 330.0), Mods::default());
    assert_eq!(undo_len(&s), n + 3);
    let (start, end) = brackets(&s, id);
    assert!(near(start, 0.0) && end.is_some_and(|e| near(e, 0.5)), "{start} {end:?}");
    assert!(ink(&s, id).y0 > 299.0, "below the path");
    // The step undoes as one.
    s.execute("edit.undo", &json!({})).unwrap();
    let (start, end) = brackets(&s, id);
    assert!(near(start, 0.5) && end.is_none(), "{start} {end:?}");
}

#[test]
fn dragging_the_start_and_end_brackets_sets_where_the_type_runs() {
    let mut s = session();
    let id = on_line(&mut s);
    // The start bracket from the middle (300) to 200.
    drag(&mut s, "selection", Point::new(300.0, 290.0), Point::new(200.0, 290.0), Mods::default());
    assert!(near(brackets(&s, id).0, 0.25));
    // The end bracket from the end of the path to 400.
    drag(&mut s, "directSelection", Point::new(500.0, 290.0), Point::new(400.0, 290.0), Mods::default());
    let (start, end) = brackets(&s, id);
    assert!(near(start, 0.25) && end.is_some_and(|e| near(e, 0.75)), "{start} {end:?}");
    let v: Value = s.execute("type.pathOptions", &json!({})).unwrap();
    assert!(near(v["end"].as_f64().unwrap(), 0.75));
    let r = ink(&s, id);
    assert!(r.x0 > 199.0 && r.x1 < 401.0, "between the brackets: {r:?}");
}

/// A compound path of the two lines (100, 300)–(500, 300) and (100, 400)–(500, 400), as a GIS
/// export makes of a river's pieces.
fn two_line_compound(s: &mut Session) -> NodeId {
    let ids: Vec<u64> = ["M100 300 L500 300", "M100 400 L500 400"]
        .iter()
        .map(|d| s.execute("path.create", &json!({ "d": d })).unwrap()["id"].as_u64().unwrap())
        .collect();
    s.execute("select.set", &json!({ "ids": ids })).unwrap();
    NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap())
}

#[test]
fn type_on_a_compound_path_goes_on_the_piece_nearest_the_click() {
    let mut s = session();
    let c = two_line_compound(&mut s);
    let r = s.execute("text.createInPath", &json!({"path": c.0, "mode": "onPath", "text": "Columbia", "at": [300, 395]})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let TextKind::OnPath { path, start, .. } = text(&s, id).kind else { panic!("not type on a path") };
    let b = path.bounds().unwrap();
    assert!(near(b.y0, 400.0) && near(b.y1, 400.0), "on the lower piece: {b:?}");
    assert!(near(start, 0.5), "{start}");
    // The other piece stays a compound path, and the type sits just above it.
    let doc = &s.doc().unwrap().doc;
    let NodeKind::Compound { children, .. } = &doc.node(c).unwrap().kind else { panic!("the compound went") };
    assert_eq!(children.len(), 1);
    let (cp, ci, _) = doc.position(c).unwrap();
    assert_eq!(doc.position(id).unwrap(), (cp, ci + 1, doc.children(cp).unwrap().len()));
    // One undo step brings the piece back.
    s.execute("edit.undo", &json!({})).unwrap();
    let doc = &s.doc().unwrap().doc;
    assert!(doc.node(id).is_none());
    assert!(matches!(&doc.node(c).unwrap().kind, NodeKind::Compound { children, .. } if children.len() == 2));
}

#[test]
fn type_on_the_last_piece_of_a_compound_path_replaces_the_compound() {
    let mut s = session();
    let c = two_line_compound(&mut s);
    let (cp, ci, _) = s.doc().unwrap().doc.position(c).unwrap();
    for at in [[300, 300], [300, 400]] {
        s.execute("text.createInPath", &json!({"path": c.0, "mode": "onPath", "text": "A", "at": at})).unwrap();
    }
    let doc = &s.doc().unwrap().doc;
    assert!(doc.node(c).is_none(), "no empty compound path is left");
    let kids = doc.children(cp).unwrap();
    assert!(kids.iter().skip(ci).take(2).all(|n| matches!(n.kind, NodeKind::Text(_))), "both types where the compound was");
}

#[test]
fn area_type_still_needs_a_path_object() {
    let mut s = session();
    let c = two_line_compound(&mut s);
    assert!(s.execute("text.createInPath", &json!({"path": c.0, "mode": "area", "text": "A"})).is_err());
}
