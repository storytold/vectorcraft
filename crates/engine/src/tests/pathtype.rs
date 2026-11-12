//! Type on a path (#429): Type on a Path Options keep its brackets, flip it, align it to the path
//! and space it; the selection tools drag its brackets to move it along the path and across it.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeKind, PathAlign, TextKind, TextObject};
use vectorcraft_geom::{Point, Rect, Shape};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::*;

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

/// A circle of radius 100 round (200, 200), starting at 9 o'clock and running clockwise, with
/// type on it from its start.
fn on_circle(s: &mut Session, text: &str) -> NodeId {
    path_type(s, "M100 200 A100 100 0 1 1 300 200 A100 100 0 1 1 100 200 Z", [90.0, 200.0], text)
}

#[test]
fn center_on_top_and_bottom_of_a_circle() {
    let mut s = session();
    let id = on_circle(&mut s, "OOOO");
    assert!(near(brackets(&s, id).0, 0.0), "{:?}", brackets(&s, id));
    let steps = undo_len(&s);
    let r = s.execute("type.pathCenter.top", &json!({})).unwrap();
    // Once round, its middle at 12 o'clock, a quarter of the way round: the type sits on the top, centered.
    let (start, end) = brackets(&s, id);
    assert!(near(start, 0.75) && end.is_none(), "{start} {end:?}");
    assert_eq!((r["ids"].clone(), r["end"].clone()), (json!([id.0]), Value::Null));
    assert_eq!(text(&s, id).para_at(0).justify, vectorcraft_doc::Justify::Center);
    let top = ink(&s, id);
    assert!((top.center().x - 200.0).abs() < 0.5 && top.y1 < 101.0, "{top:?}");
    assert_eq!((undo_len(&s), last_step(&s)), (steps + 1, "Center on Top".to_string()));
    // Again: nothing moves, and no undo step is recorded.
    s.execute("type.pathCenter.top", &json!({})).unwrap();
    assert_eq!(undo_len(&s), steps + 1);
    // The bottom: 6 o'clock, outside the circle (the type is not flipped).
    s.execute("type.pathCenter.bottom", &json!({})).unwrap();
    let (start, end) = brackets(&s, id);
    assert!(near(start, 0.25) && end.is_none(), "{start} {end:?}");
    let bottom = ink(&s, id);
    assert!((bottom.center().x - 200.0).abs() < 0.5 && bottom.y0 > 299.0, "{bottom:?}");
    assert_eq!((undo_len(&s), last_step(&s)), (steps + 2, "Center on Bottom".to_string()));
    // Justify Center stays as it is.
    s.execute("text.setStyle", &json!({"justify": "justifyCenter"})).unwrap();
    s.execute("type.pathCenter.top", &json!({})).unwrap();
    assert_eq!(text(&s, id).para_at(0).justify, vectorcraft_doc::Justify::JustifyCenter);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(brackets(&s, id).0, 0.25));
    // Brackets already in place with another alignment: only the alignment changes, in one step.
    s.execute("text.setStyle", &json!({"justify": "left"})).unwrap();
    let (steps, before) = (undo_len(&s), brackets(&s, id));
    s.execute("type.pathCenter.bottom", &json!({})).unwrap();
    assert_eq!((undo_len(&s), brackets(&s, id)), (steps + 1, before));
    assert_eq!(text(&s, id).para_at(0).justify, vectorcraft_doc::Justify::Center);
}

#[test]
fn center_on_top_twice_records_one_step_for_any_span() {
    let mut s = session();
    let id = on_circle(&mut s, "OOOO");
    for (start, end) in [(0.0, 0.2), (0.13, 0.41), (0.37, 0.52), (0.61, 0.97), (0.89, 0.07), (0.3, 0.3001)] {
        s.execute("type.pathOptions", &json!({"start": start, "end": end})).unwrap();
        for c in ["type.pathCenter.top", "type.pathCenter.bottom"] {
            s.execute(c, &json!({})).unwrap();
            let (steps, placed) = (undo_len(&s), brackets(&s, id));
            // Again: the brackets stay exactly where they are, and no undo step is recorded.
            s.execute(c, &json!({})).unwrap();
            assert_eq!((undo_len(&s), brackets(&s, id)), (steps, placed), "{c} {start} {end}");
        }
    }
}

#[test]
fn center_on_top_keeps_the_span_and_measures_the_path_as_drawn() {
    let mut s = session();
    let id = on_circle(&mut s, "OOOO");
    // A span of a fifth of the path keeps its length.
    s.execute("type.pathOptions", &json!({"end": 0.2})).unwrap();
    s.execute("type.pathCenter.top", &json!({})).unwrap();
    let (start, end) = brackets(&s, id);
    assert!(near(start, 0.15) && end.is_some_and(|e| near(e, 0.35)), "{start} {end:?}");
    // Turned a quarter turn counter-clockwise, its 3 o'clock point, halfway round, is at the top.
    s.execute("type.pathOptions", &json!({"start": 0, "end": null})).unwrap();
    s.execute("object.rotate", &json!({"angle": 90})).unwrap();
    s.execute("type.pathCenter.top", &json!({})).unwrap();
    let (start, _) = brackets(&s, id);
    assert!(near(start.rem_euclid(1.0), 0.0) || near(start, 1.0), "{start}");
    let bounds = text(&s, id).bounds().unwrap();
    let top = ink(&s, id);
    assert!((top.center().x - bounds.center().x).abs() < 0.5 && top.y1 < bounds.y0 + 1.0, "{top:?} {bounds:?}");
    // A rectangle: the middle of its top edge.
    let id = path_type(&mut s, "M100 100 L300 100 L300 300 L100 300 Z", [90.0, 100.0], "OOOO");
    s.execute("type.pathCenter.top", &json!({})).unwrap();
    let (start, end) = brackets(&s, id);
    assert!(near(start, 0.625) && end.is_none(), "{start} {end:?}");
    assert!((ink(&s, id).center().x - 200.0).abs() < 0.5);
}

#[test]
fn center_on_top_moves_every_selected_type_on_a_path_in_one_step() {
    let mut s = session();
    let a = on_circle(&mut s, "OOOO");
    let b = path_type(&mut s, "M100 100 L300 100 L300 300 L100 300 Z", [90.0, 100.0], "OOOO");
    let p = s.execute("text.create", &json!({"x": 40, "y": 40, "text": "Point"})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0, p]})).unwrap();
    let steps = undo_len(&s);
    let r = s.execute("type.pathCenter.top", &json!({})).unwrap();
    assert_eq!(r["ids"], json!([a.0, b.0]));
    assert!(near(brackets(&s, a).0, 0.75) && near(brackets(&s, b).0, 0.625));
    assert_eq!(undo_len(&s), steps + 1);
    // Dynamic Text stays on, and the layout fits the text to the span where it now runs: a fifth of
    // the circle, at its bottom.
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("type.pathOptions", &json!({"start": 0, "end": 0.2})).unwrap();
    s.execute("type.dynamicText", &json!({"on": true})).unwrap();
    s.execute("type.pathCenter.bottom", &json!({})).unwrap();
    assert!(text(&s, a).dynamic_text && near(brackets(&s, a).0, 0.65), "{:?}", brackets(&s, a));
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text(&s, a));
    assert!(lay.fit_scale > 1.5 && !lay.overflow, "{}", lay.fit_scale);
    let bottom = ink(&s, a);
    assert!((bottom.center().x - 200.0).abs() < 1.0 && bottom.y1 > 300.0, "{bottom:?}");
    // Vertical type on a path moves the same way.
    s.execute("type.pathOptions", &json!({"start": 0, "end": null})).unwrap();
    s.execute("type.orientation.vertical", &json!({})).unwrap();
    s.execute("type.pathCenter.top", &json!({})).unwrap();
    assert!(near(brackets(&s, a).0, 0.75));
}

#[test]
fn center_on_top_refuses_open_compound_and_threaded_type() {
    let mut s = session();
    let circle = on_circle(&mut s, "OOOO");
    let line = on_line(&mut s);
    s.execute("select.set", &json!({"ids": [circle.0, line.0]})).unwrap();
    let steps = undo_len(&s);
    let before = brackets(&s, circle);
    let e = s.execute("type.pathCenter.top", &json!({})).unwrap_err().to_string();
    assert!(e.contains("type on an open or compound path cannot be centered on its top or bottom") && e.ends_with(&line.0.to_string()), "{e}");
    // Nothing changes, the type on the circle included, and the same holds for an explicit list.
    assert_eq!((undo_len(&s), brackets(&s, circle)), (steps, before));
    let e = s.execute("type.pathCenter.bottom", &json!({"ids": [circle.0, line.0]})).unwrap_err().to_string();
    assert!(e.contains("open or compound path") && e.ends_with(&line.0.to_string()), "{e}");
    s.execute("select.set", &json!({"ids": [circle.0]})).unwrap();
    for bad in [json!({"ids": "all"}), json!({"ids": [999_999]})] {
        assert!(s.execute("type.pathCenter.top", &bad).is_err(), "{bad}");
    }
    assert_eq!(undo_len(&s), steps);
    // Two circles side by side in one path: a compound path.
    let pair = path_type(
        &mut s,
        "M100 450 A40 40 0 1 1 180 450 A40 40 0 1 1 100 450 Z M300 450 A40 40 0 1 1 380 450 A40 40 0 1 1 300 450 Z",
        [90.0, 450.0],
        "OOOO",
    );
    let e = s.execute("type.pathCenter.top", &json!({})).unwrap_err().to_string();
    assert!(e.contains("open or compound path") && e.ends_with(&pair.0.to_string()), "{e}");
    // A closed path that is level all round has no height.
    let flat = path_type(&mut s, "M100 400 L300 400 Z", [90.0, 400.0], "OOOO");
    let e = s.execute("type.pathCenter.top", &json!({})).unwrap_err().to_string();
    assert!(e.contains("type on a path with no length or no height cannot be centered") && e.ends_with(&flat.0.to_string()), "{e}");
    // A frame of a thread is refused, alone or with other type on a closed path.
    let threaded = on_circle(&mut s, "Threaded story");
    let frame = s.execute("shape.rectangle", &json!({"x": 400, "y": 100, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [threaded.0, frame]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [threaded.0]})).unwrap();
    let before = brackets(&s, threaded);
    for ids in [json!([threaded.0]), json!([circle.0, threaded.0])] {
        let steps = undo_len(&s);
        let e = s.execute("type.pathCenter.top", &json!({ "ids": ids })).unwrap_err().to_string();
        assert!(e.contains("type on a path in a thread cannot be centered on its top or bottom") && e.ends_with(&threaded.0.to_string()), "{e}");
        assert_eq!((undo_len(&s), brackets(&s, threaded)), (steps, before));
    }
    // Without type on one closed path selected, the commands are not available.
    let p = s.execute("text.create", &json!({"x": 40, "y": 40, "text": "Point"})).unwrap()["id"].as_u64().unwrap();
    for (ids, why) in [
        (json!([p]), "select type on a closed path"),
        (json!([line.0]), "open or compound path"),
        (json!([circle.0, line.0]), "open or compound path"),
        (json!([threaded.0]), "in a thread"),
    ] {
        s.execute("select.set", &json!({ "ids": ids })).unwrap();
        for c in ["type.pathCenter.top", "type.pathCenter.bottom"] {
            let e = s.execute(c, &json!({})).unwrap_err().to_string();
            assert!(e.contains(why), "{c} {ids}: {e}");
        }
    }
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

/// #3980: Direct Selection drags an anchor of type on a path's path, in document space through
/// the type's transform; the brackets keep their fractions of the path and the type reflows.
#[test]
fn direct_selection_drags_an_anchor_of_the_path_and_the_type_reflows() {
    let mut s = session();
    let id = path_type(&mut s, "M100 300 L300 300 L500 300", [100.0, 310.0], "Reflowing type");
    s.execute("type.pathOptions", &json!({"start": 0.0, "end": 0.75})).unwrap();
    // A quarter turn, so a document-space move isn't the path's own.
    s.execute("object.transform", &json!({"ids": [id.0], "matrix": [0, 1, -1, 0, 600, 0]})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let path = |s: &Session| text(s, id).type_path().unwrap();
    let (first, middle) = (path(&s).subpaths[0].anchors[0].p, path(&s).subpaths[0].anchors[1].p);
    let (glyphs, n) = (origins(&s, id), undo_len(&s));
    drag(&mut s, "directSelection", middle, middle + vectorcraft_geom::Vec2::new(40.0, 0.0), Mods::default());
    let after = path(&s);
    let moved = after.subpaths[0].anchors[1].p;
    assert!(near(moved.x, middle.x + 40.0) && near(moved.y, middle.y), "{middle:?} → {moved:?}");
    assert_eq!(after.subpaths[0].anchors[0].p, first, "the other anchors stay");
    assert_eq!(brackets(&s, id), (0.0, Some(0.75)));
    assert_ne!(origins(&s, id), glyphs, "the type reflows along the reshaped path");
    assert_eq!((undo_len(&s), last_step(&s).as_str()), (n + 1, "Reshape Type Path"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(path(&s).subpaths[0].anchors[1].p, middle);
}

/// Type on a path made by clicking a path with the plain Type tool (an open path) or Type on a
/// Path (a closed one) shows its brackets as soon as the Selection or Direct Selection tool is
/// chosen, and its centre bracket, dragged across the path, flips it.
#[test]
fn type_on_a_path_made_with_the_type_tool_has_brackets_at_once() {
    let v = ViewInfo::default();
    for (tool, d, mods) in [
        ("selection", "M100 300 L500 300", Mods::default()),
        ("directSelection", "M100 300 L500 300", Mods::default()),
        ("selection", "M100 300 L500 300 L500 500 L100 500 Z", Mods::default()),
    ] {
        let mut s = session();
        s.execute("path.create", &json!({ "d": d })).unwrap();
        s.select_tool(if d.ends_with('Z') { "typeOnPath" } else { "type" }, v).unwrap();
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, 300.0, 300.0).with_mods(mods), v).unwrap();
        }
        let id = *s.doc().unwrap().selection.whole().first().expect("the new type is selected");
        assert!(matches!(text(&s, id).kind, TextKind::OnPath { .. }), "{tool} {d}");
        s.execute("text.setText", &json!({"ids": [id.0], "text": "Slide"})).unwrap();
        s.select_tool(tool, v).unwrap();
        let lines = s.overlays(v).iter().filter(|o| matches!(o, vectorcraft_tools::Overlay::Line { .. })).count();
        assert!(lines >= 5, "{tool} {d}: the brackets are drawn at once ({lines} lines)");
        if d.ends_with('Z') {
            continue;
        }
        // The centre bracket stands midway between the start (300) and the end of the path (500).
        let (start, end) = brackets(&s, id);
        assert!(near(start, 0.5) && end.is_none(), "{start} {end:?}");
        let centre = Point::new(400.0, 295.0);
        assert_eq!(s.cursor(centre, Mods::default(), v), vectorcraft_tools::Cursor::PathBracket, "{tool}");
        drag(&mut s, tool, centre, Point::new(400.0, 330.0), Mods::default());
        let (start, end) = brackets(&s, id);
        assert!(near(start, 0.0) && end.is_some_and(|e| near(e, 0.5)), "{tool}: flipped, {start} {end:?}");
        assert!(ink(&s, id).y0 > 299.0, "{tool}: below the path");
    }
}
