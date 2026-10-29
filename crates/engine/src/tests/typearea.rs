//! Resizing area type (#252): a bounding-box handle drag and Area Type Options size the type area
//! and the text reflows at its size; Direct Selection reshapes the frame by a corner or an edge.
//! Point type still scales. The type widget converts point type to area type and back.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeKind, TextKind, TextObject};
use vectorcraft_geom::{Affine, Point, Rect};
use vectorcraft_text::TextLayout;
use vectorcraft_tools::bbox::Handle;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

const STORY: &str = "The quick brown fox jumps over the lazy dog. A second sentence follows the first one here. Then a third one ends it.";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// 12 pt area type in a `w` × `h` frame at (`x`, `y`).
fn area(s: &mut Session, x: f64, y: f64, w: f64, h: f64, text: &str) -> NodeId {
    id_of(&s.execute("text.create", &json!({"x": x, "y": y, "size": 12, "area": {"width": w, "height": h}, "text": text})).unwrap())
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn text(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        k => panic!("not text: {k:?}"),
    }
}

/// The frame's anchors in text space.
fn corners(s: &Session, id: NodeId) -> Vec<Point> {
    match text(s, id).kind {
        TextKind::Area { frame } => frame.anchors().map(|(_, _, a)| a.p).collect(),
        k => panic!("not area type: {k:?}"),
    }
}

fn frame(s: &Session, id: NodeId) -> Rect {
    match text(s, id).kind {
        TextKind::Area { frame } => frame.bounds().unwrap(),
        k => panic!("not area type: {k:?}"),
    }
}

fn layout(s: &Session, id: NodeId) -> TextLayout {
    vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text(s, id))
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn last_step(s: &Session) -> String {
    s.doc().unwrap().history.undo.last().unwrap().label.clone()
}

/// Press, drag and release `tool` from `from` to `to`.
fn drag(s: &mut Session, tool: &str, from: Point, to: Point) {
    let v = ViewInfo::default();
    s.select_tool(tool, v).unwrap();
    for (kind, p) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
        s.pointer(&PointerEvent::new(kind, p.x, p.y).with_mods(Mods::default()), v).unwrap();
    }
    assert!(!s.in_interaction());
}

fn near(a: Point, b: Point) -> bool {
    a.distance(b) < 1e-6
}

#[test]
fn a_handle_drag_resizes_the_type_area_and_the_text_reflows() {
    let mut s = session();
    let id = area(&mut s, 40.0, 40.0, 120.0, 40.0, STORY);
    let before = layout(&s, id);
    assert!(before.overflow, "the story overflows 120 × 40");
    sel(&mut s, &[id]);
    let n = undo_len(&s);
    // The bounding box's bottom-right handle, from (160, 80) to (200, 140): 160 × 100.
    drag(&mut s, "selection", Point::new(160.0, 80.0), Point::new(200.0, 140.0));
    assert_eq!(undo_len(&s), n + 1, "one step");
    assert_eq!(last_step(&s), "Resize Type Area");
    let r = frame(&s, id);
    assert!(near(r.origin(), Point::ZERO) && (r.width() - 160.0).abs() < 1e-6 && (r.height() - 100.0).abs() < 1e-6, "{r:?}");
    let t = text(&s, id);
    assert_eq!(t.xf, Affine::translate((40.0, 40.0)), "the type isn't scaled");
    assert!(t.runs.iter().all(|r| r.style.size == 12.0));
    let after = layout(&s, id);
    assert!(after.lines.len() > before.lines.len(), "more lines fit: {} → {}", before.lines.len(), after.lines.len());
    assert!(after.glyphs.len() > before.glyphs.len());
    assert!(!after.overflow, "the whole story fits 160 × 100");
    // The bounding box hugs the new frame.
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().geometric_bounds(), Some(Rect::new(40.0, 40.0, 200.0, 140.0)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(frame(&s, id), Rect::new(0.0, 0.0, 120.0, 40.0));
    assert!(layout(&s, id).overflow);
}

/// Transform Again after a handle drag on area type repeats the drag: it resizes the type area
/// again, and the type keeps its size.
#[test]
fn transform_again_after_a_handle_drag_resizes_the_type_area_again() {
    let mut s = session();
    let id = area(&mut s, 40.0, 40.0, 120.0, 40.0, STORY);
    sel(&mut s, &[id]);
    // The bottom-right handle, from (160, 80) to (200, 140): 4/3 as wide and 2.5 times as tall.
    drag(&mut s, "selection", Point::new(160.0, 80.0), Point::new(200.0, 140.0));
    s.execute("object.transformAgain", &json!({})).unwrap();
    assert_eq!(last_step(&s), "Transform Again");
    let r = frame(&s, id);
    assert!(near(r.origin(), Point::ZERO) && (r.width() - 640.0 / 3.0).abs() < 1e-6 && (r.height() - 250.0).abs() < 1e-6, "{r:?}");
    let t = text(&s, id);
    assert_eq!(t.xf, Affine::translate((40.0, 40.0)), "the type isn't scaled");
    assert!(t.runs.iter().all(|r| r.style.size == 12.0));
}

#[test]
fn point_type_still_scales_with_its_handles() {
    let mut s = session();
    let id = id_of(&s.execute("text.create", &json!({"x": 40, "y": 40, "size": 12, "text": "Hello"})).unwrap());
    sel(&mut s, &[id]);
    let b = s.transform_box(&s.doc().unwrap().selection.objects).unwrap();
    let (tl, br) = (b.to_doc() * Handle::TopLeft.pos(b.rect), b.to_doc() * Handle::BottomRight.pos(b.rect));
    drag(&mut s, "selection", br, tl + (br - tl) * 2.0);
    assert_eq!(last_step(&s), "Scale");
    let c = text(&s, id).xf.as_coeffs();
    assert!((c[0] - 2.0).abs() < 1e-6 && (c[3] - 2.0).abs() < 1e-6, "the type doubles: {c:?}");
}

#[test]
fn with_other_objects_the_type_area_resizes_and_the_others_scale() {
    let mut s = session();
    let id = area(&mut s, 40.0, 40.0, 120.0, 40.0, STORY);
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 40, "y": 100, "width": 120, "height": 20})).unwrap());
    sel(&mut s, &[id, r]);
    // The box spans (40, 40)–(160, 120); its right handle doubles the width.
    drag(&mut s, "selection", Point::new(160.0, 80.0), Point::new(280.0, 80.0));
    assert_eq!(last_step(&s), "Scale");
    assert_eq!(s.doc().unwrap().doc.node(r).unwrap().geometric_bounds(), Some(Rect::new(40.0, 100.0, 280.0, 120.0)));
    assert_eq!(frame(&s, id), Rect::new(0.0, 0.0, 240.0, 40.0));
    assert_eq!(text(&s, id).xf, Affine::translate((40.0, 40.0)));
}

#[test]
fn a_rotated_type_area_resizes_along_its_own_axes() {
    let mut s = session();
    let id = area(&mut s, 40.0, 40.0, 120.0, 40.0, STORY);
    sel(&mut s, &[id]);
    s.execute("object.rotate", &json!({"angle": 30, "absolute": true})).unwrap();
    let xf = text(&s, id).xf;
    let b = s.transform_box(&[id]).unwrap();
    let right = b.to_doc() * Handle::Right.pos(b.rect);
    let along = (b.to_doc() * Handle::Right.pos(b.rect)) - (b.to_doc() * Handle::Left.pos(b.rect));
    drag(&mut s, "selection", right, right + along * 0.5);
    assert_eq!(text(&s, id).xf, xf, "the type keeps its size and angle");
    let r = frame(&s, id);
    assert!((r.width() - 180.0).abs() < 1e-6 && (r.height() - 40.0).abs() < 1e-6, "still a rectangle, 180 × 40: {r:?}");
    assert_eq!(corners(&s, id).len(), 4);
}

#[test]
fn direct_selection_drags_a_frame_corner_or_edge() {
    let mut s = session();
    let id = area(&mut s, 40.0, 40.0, 120.0, 40.0, STORY);
    let glyphs = layout(&s, id).glyphs.len();
    let n = undo_len(&s);
    // The bottom-right corner, not selected yet: the corner moves, the others stay.
    drag(&mut s, "directSelection", Point::new(160.0, 80.0), Point::new(200.0, 140.0));
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    assert_eq!(undo_len(&s), n + 1, "one step");
    assert_eq!(last_step(&s), "Reshape Type Area");
    let c = corners(&s, id);
    assert!(near(c[0], Point::ZERO) && near(c[1], Point::new(120.0, 0.0)) && near(c[3], Point::new(0.0, 40.0)), "{c:?}");
    assert!(near(c[2], Point::new(160.0, 100.0)), "{c:?}");
    assert!(layout(&s, id).glyphs.len() > glyphs, "more text fits the larger area");
    assert_eq!(text(&s, id).xf, Affine::translate((40.0, 40.0)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(frame(&s, id), Rect::new(0.0, 0.0, 120.0, 40.0));
    // The right edge: both its ends move, and the area stays a rectangle.
    drag(&mut s, "directSelection", Point::new(160.0, 60.0), Point::new(180.0, 60.0));
    assert_eq!(frame(&s, id), Rect::new(0.0, 0.0, 140.0, 40.0));
    assert_eq!(text(&s, id).runs[0].style.size, 12.0);
}

/// Direct Selection's marquee selects the frame anchors of area type inside it: a drag of one moves
/// only those, the arrow keys nudge them, and Shift-click takes one out.
#[test]
fn direct_selection_marquee_selects_frame_anchors() {
    let mut s = session();
    let id = area(&mut s, 40.0, 40.0, 120.0, 40.0, STORY);
    let other = area(&mut s, 300.0, 40.0, 120.0, 40.0, STORY);
    s.execute("select.none", &json!({})).unwrap();
    let v = ViewInfo::default();
    let shift = Mods { shift: true, ..Mods::default() };
    // A marquee round the right-hand corners, (160, 40) and (160, 80).
    drag(&mut s, "directSelection", Point::new(150.0, 20.0), Point::new(180.0, 100.0));
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    let n = undo_len(&s);
    drag(&mut s, "directSelection", Point::new(160.0, 80.0), Point::new(180.0, 100.0));
    assert_eq!(undo_len(&s), n + 1, "one step");
    assert_eq!(last_step(&s), "Reshape Type Area");
    assert_eq!(corners(&s, id), vec![Point::ZERO, Point::new(140.0, 20.0), Point::new(140.0, 60.0), Point::new(0.0, 40.0)]);
    // The arrow keys nudge them, each press one step, and Shift-click takes (179, 60) out.
    assert!(s.tool_claims_key(ToolKey::Left, v));
    s.tool_key(ToolKey::Left, Mods::default(), v).unwrap();
    assert_eq!(undo_len(&s), n + 2);
    assert_eq!(corners(&s, id), vec![Point::ZERO, Point::new(139.0, 20.0), Point::new(139.0, 60.0), Point::new(0.0, 40.0)]);
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, 179.0, 60.0).with_mods(shift), v).unwrap();
    }
    s.tool_key(ToolKey::Down, shift, v).unwrap();
    assert_eq!(corners(&s, id), vec![Point::ZERO, Point::new(139.0, 20.0), Point::new(139.0, 70.0), Point::new(0.0, 40.0)]);
    // A marquee across both frames' facing corners moves both frames in one step.
    drag(&mut s, "directSelection", Point::new(170.0, 10.0), Point::new(310.0, 65.0));
    assert_eq!(s.doc().unwrap().selection.objects.len(), 2);
    let n = undo_len(&s);
    drag(&mut s, "directSelection", Point::new(300.0, 40.0), Point::new(300.0, 30.0));
    assert_eq!(undo_len(&s), n + 1, "one step");
    assert_eq!(corners(&s, id)[1], Point::new(139.0, 10.0));
    assert_eq!(corners(&s, other), vec![Point::new(0.0, -10.0), Point::new(120.0, 0.0), Point::new(120.0, 40.0), Point::new(0.0, 40.0)]);
    // Shift-click on the other frame's only selected corner deselects that type; the corner of the
    // first frame still nudges.
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, 300.0, 30.0).with_mods(shift), v).unwrap();
    }
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    s.tool_key(ToolKey::Up, Mods::default(), v).unwrap();
    assert_eq!(corners(&s, id)[1], Point::new(139.0, 9.0));
    // Deselect, then the same two selected again: they are selected whole, so the arrow keys move
    // them as objects.
    s.execute("select.none", &json!({})).unwrap();
    sel(&mut s, &[id, other]);
    assert!(!s.tool_claims_key(ToolKey::Left, v));
    // A click on empty canvas deselects; the arrow keys are then no longer the tool's.
    drag(&mut s, "directSelection", Point::new(600.0, 500.0), Point::new(600.0, 500.0));
    assert!(!s.tool_claims_key(ToolKey::Left, v));
}

/// `text.reshapeArea` with `items` moves several frames' anchors in one step, and fails whole.
#[test]
fn reshape_area_moves_the_anchors_of_several_frames() {
    let mut s = session();
    let a = area(&mut s, 40.0, 40.0, 120.0, 40.0, STORY);
    let b = area(&mut s, 300.0, 40.0, 120.0, 40.0, STORY);
    let n = undo_len(&s);
    let items = json!([{"id": a.0, "anchors": [[0, 2]]}, {"id": b.0, "anchors": [[0, 0], [0, 3]]}]);
    s.execute("text.reshapeArea", &json!({"items": items, "dx": -10, "dy": 5})).unwrap();
    assert_eq!(undo_len(&s), n + 1, "one step");
    assert_eq!(corners(&s, a)[2], Point::new(110.0, 45.0));
    assert_eq!(corners(&s, b), vec![Point::new(-10.0, 5.0), Point::new(120.0, 0.0), Point::new(120.0, 40.0), Point::new(-10.0, 45.0)]);
    for bad in [
        json!({"items": [], "dx": 1, "dy": 1}),
        json!({"items": {"id": a.0}, "dx": 1, "dy": 1}),
        json!({"items": [{"id": a.0, "anchors": [[0, 2]]}], "id": a.0, "dx": 1, "dy": 1}),
        json!({"items": [{"id": a.0, "anchors": [[0, 2]]}, {"id": a.0, "anchors": [[0, 1]]}], "dx": 1, "dy": 1}),
        // A bad item fails the whole step: the good one does not move either.
        json!({"items": [{"id": a.0, "anchors": [[0, 2]]}, {"id": b.0, "anchors": [[0, 9]]}], "dx": 1, "dy": 1}),
        json!({"items": [{"id": a.0, "anchors": [[0, 2]]}, {"anchors": [[0, 1]]}], "dx": 1, "dy": 1}),
    ] {
        assert!(s.execute("text.reshapeArea", &bad).is_err(), "{bad}");
    }
    assert_eq!(undo_len(&s), n + 1, "nothing more changed");
    assert_eq!(corners(&s, a)[2], Point::new(110.0, 45.0));
}

#[test]
fn a_resized_frame_reflows_its_thread() {
    let mut s = session();
    let a = area(&mut s, 40.0, 40.0, 120.0, 40.0, STORY);
    let b = area(&mut s, 300.0, 40.0, 120.0, 200.0, "");
    sel(&mut s, &[a, b]);
    s.execute("text.thread.create", &json!({})).unwrap();
    let rest = text(&s, b).plain_text().len();
    assert!(rest > 0, "the overflow flows on");
    // Bounding-box resize of the first frame to 160 × 100, as the Selection tool sends it.
    let m = Affine::translate((40.0, 40.0)) * Affine::scale_non_uniform(160.0 / 120.0, 100.0 / 40.0) * Affine::translate((-40.0, -40.0));
    let c = m.as_coeffs();
    s.execute("object.transform", &json!({"ids": [a.0], "matrix": c, "typeAreas": true})).unwrap();
    assert!(text(&s, b).plain_text().len() < rest, "the second frame holds less");
    assert_eq!(text(&s, a).plain_text().len() + text(&s, b).plain_text().len(), STORY.len());
}

#[test]
fn area_type_options_size_the_type_area() {
    let mut s = session();
    let id = area(&mut s, 40.0, 40.0, 120.0, 40.0, STORY);
    sel(&mut s, &[id]);
    let q = s.execute("text.areaOptions", &json!({})).unwrap();
    assert_eq!((q["width"].as_f64(), q["height"].as_f64()), (Some(120.0), Some(40.0)));
    let n = undo_len(&s);
    let r = s.execute("text.areaOptions", &json!({"width": 160, "height": 100})).unwrap();
    assert_eq!((r["width"].as_f64(), r["height"].as_f64()), (Some(160.0), Some(100.0)));
    assert_eq!(frame(&s, id), Rect::new(0.0, 0.0, 160.0, 100.0));
    assert_eq!(text(&s, id).xf, Affine::translate((40.0, 40.0)));
    assert!(!layout(&s, id).overflow);
    // The dialog's OK sends the size back unchanged with the other options: only those change.
    s.execute("text.areaOptions", &json!({"width": 160, "height": 100, "inset": 4})).unwrap();
    assert_eq!(frame(&s, id), Rect::new(0.0, 0.0, 160.0, 100.0));
    assert_eq!(undo_len(&s), n + 2);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(frame(&s, id), Rect::new(0.0, 0.0, 120.0, 40.0));
    // Type scaled 200%: sizes are in points on the page, along the type's axes.
    s.execute("object.scale", &json!({"sx": 200, "origin": [40, 40]})).unwrap();
    assert_eq!(s.execute("text.areaOptions", &json!({})).unwrap()["width"].as_f64(), Some(240.0));
    s.execute("text.areaOptions", &json!({"width": 300})).unwrap();
    assert_eq!(frame(&s, id), Rect::new(0.0, 0.0, 150.0, 40.0));
}

#[test]
fn reshape_area_takes_untrusted_params() {
    let mut s = session();
    let id = area(&mut s, 40.0, 40.0, 120.0, 40.0, STORY);
    let point = id_of(&s.execute("text.create", &json!({"x": 40, "y": 300, "text": "Point"})).unwrap());
    let rect = id_of(&s.execute("shape.rectangle", &json!({"x": 300, "y": 300, "width": 50, "height": 50})).unwrap());
    let n = undo_len(&s);
    for bad in [
        json!({}),
        json!({"anchors": [[0, 2]], "dx": 1, "dy": 1}),
        json!({"id": point.0, "anchors": [[0, 0]], "dx": 1, "dy": 1}),
        json!({"id": rect.0, "anchors": [[0, 0]], "dx": 1, "dy": 1}),
        json!({"id": 99_999, "anchors": [[0, 0]], "dx": 1, "dy": 1}),
        json!({"id": id.0, "dx": 1, "dy": 1}),
        json!({"id": id.0, "anchors": [], "dx": 1, "dy": 1}),
        json!({"id": id.0, "anchors": [["a", 1], [0], "x"], "dx": 1, "dy": 1}),
        // A malformed reference among good ones fails rather than moving only the good ones.
        json!({"id": id.0, "anchors": [[0, 2], [0, "3"]], "dx": 1, "dy": 1}),
        json!({"id": id.0, "anchors": [[0, 2], [0, 3, 1]], "dx": 1, "dy": 1}),
        json!({"id": id.0, "anchors": [[0, 9]], "dx": 1, "dy": 1}),
        json!({"id": id.0, "anchors": [[0, 1], [7, 0]], "dx": 1, "dy": 1}),
        json!({"id": id.0, "anchors": [[0, 2]], "dy": 1}),
        json!({"id": id.0, "anchors": [[0, 2]], "dx": "far", "dy": 1}),
        json!({"id": id.0, "anchors": [[0, 2]], "dx": 0, "dy": 1e300}),
        json!({"id": id.0, "anchors": [[0, 2]], "dx": -1e7, "dy": 0}),
    ] {
        assert!(s.execute("text.reshapeArea", &bad).is_err(), "{bad}");
    }
    assert_eq!(undo_len(&s), n, "nothing changed");
    assert_eq!(frame(&s, id), Rect::new(0.0, 0.0, 120.0, 40.0));
    // Listed twice, an anchor still moves once.
    s.execute("text.reshapeArea", &json!({"id": id.0, "anchors": [[0, 2], [0, 2]], "dx": 10, "dy": 5})).unwrap();
    assert_eq!(corners(&s, id)[2], Point::new(130.0, 45.0));
    assert_eq!(last_step(&s), "Reshape Type Area");
    // A frame collapsed to nothing lays out (everything overflows) without trouble.
    for m in [[0.0, 0.0, 0.0, 1.0, 40.0, 0.0], [1.0, 0.0, 0.0, 0.0, 0.0, 40.0], [0.0; 6]] {
        s.execute("object.transform", &json!({"ids": [id.0], "matrix": m, "typeAreas": true})).unwrap();
        assert!(layout(&s, id).overflow);
        s.execute("edit.undo", &json!({})).unwrap();
    }
}

#[test]
fn reshape_area_is_a_registered_command() {
    let spec = cmd::find_command("text.reshapeArea").unwrap();
    assert_eq!(spec.label, "Reshape Type Area");
    assert!(spec.params.contains("anchors") && spec.params.contains("dx"));
    assert!(cmd::find_command("object.transform").unwrap().params.contains("typeAreas"));
    assert!(session().commands().iter().any(|c| c.id == "text.reshapeArea" && c.enabled));
}

/// The Discord report: type in a frame dragged with the Type tool, then the Selection tool. Its
/// bounding box resizes the type area (the text reflows at its size), it doesn't scale the type.
#[test]
fn type_in_a_dragged_frame_resizes_with_the_selection_tool() {
    let mut s = session();
    drag(&mut s, "type", Point::new(40.0, 40.0), Point::new(160.0, 80.0));
    let v = ViewInfo::default();
    s.tool_text(STORY, v).unwrap();
    s.select_tool("selection", v).unwrap();
    let id = s.doc().unwrap().selection.objects[0];
    assert_eq!(frame(&s, id), Rect::new(0.0, 0.0, 120.0, 40.0));
    let lines = layout(&s, id).lines.len();
    drag(&mut s, "selection", Point::new(160.0, 80.0), Point::new(200.0, 140.0));
    assert_eq!(last_step(&s), "Resize Type Area");
    assert_eq!(frame(&s, id), Rect::new(0.0, 0.0, 160.0, 100.0));
    assert_eq!(text(&s, id).xf, Affine::translate((40.0, 40.0)), "the type isn't scaled");
    assert!(layout(&s, id).lines.len() > lines, "the text reflows into the larger area");
}

/// The type widget beside the selected type's bounding box (as the Selection tool shows it at
/// the default view).
fn widget(s: &Session) -> vectorcraft_tools::typewidget::TypeWidget {
    let st = s.doc().unwrap();
    let bx = s.transform_box(&st.selection.objects).unwrap();
    vectorcraft_tools::typewidget::TypeWidget::of(&st.doc, &st.selection, &bx, ViewInfo::default().zoom).unwrap()
}

fn double_click(s: &mut Session, p: Point) {
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    for kind in [PointerKind::Down, PointerKind::Up, PointerKind::Down, PointerKind::Up, PointerKind::DoubleClick] {
        s.pointer(&PointerEvent::new(kind, p.x, p.y).with_mods(Mods::default()), v).unwrap();
    }
}

/// Double-clicking the type widget converts point type to area type and back, each one undo
/// step, keeping the text, its styles and where it stands; the selection stays.
#[test]
fn the_type_widget_converts_point_and_area_type() {
    let mut s = session();
    let id = id_of(&s.execute("text.create", &json!({"x": 40, "y": 100, "size": 18, "text": "Two words"})).unwrap());
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 4, "end": 9, "size": 24})).unwrap();
    sel(&mut s, &[id]);
    let before = text(&s, id);
    let glyph = |t: &TextObject| t.xf * vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).glyphs[0].origin;
    let w = widget(&s);
    assert!(!w.area);
    let n = undo_len(&s);
    double_click(&mut s, w.at);
    assert_eq!(undo_len(&s), n + 1, "one step");
    assert_eq!(last_step(&s), "Convert To Area Type");
    let t = text(&s, id);
    assert!(matches!(t.kind, TextKind::Area { .. }));
    assert_eq!((t.plain_text(), &t.runs), (before.plain_text(), &before.runs), "the text and its styles stay");
    assert!(glyph(&t).distance(glyph(&before)) < 0.01, "the type stays where it was");
    assert!(!layout(&s, id).overflow);
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    // Now area type: a filled widget, and a double-click turns it back into point type.
    let w = widget(&s);
    assert!(w.area);
    double_click(&mut s, w.at);
    assert_eq!(undo_len(&s), n + 2);
    assert_eq!(last_step(&s), "Convert To Point Type");
    let t = text(&s, id);
    assert!(matches!(t.kind, TextKind::Point));
    assert_eq!(t.runs, before.runs);
    assert!(glyph(&t).distance(glyph(&before)) < 0.01);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&s, id), before);
}

#[test]
fn area_type_options_align_the_lines_vertically() {
    use vectorcraft_doc::VerticalAlign;
    let mut s = session();
    let id = area(&mut s, 40.0, 40.0, 200.0, 300.0, "Short story");
    sel(&mut s, &[id]);
    assert_eq!(s.execute("text.areaOptions", &json!({})).unwrap()["verticalAlign"], "top");
    let top = layout(&s, id).lines[0].baseline;
    let before = text(&s, id).cached_bounds;
    let n = undo_len(&s);
    let r = s.execute("text.areaOptions", &json!({"verticalAlign": "bottom"})).unwrap();
    assert_eq!(r["verticalAlign"], "bottom");
    assert_eq!(text(&s, id).area.vertical_align, VerticalAlign::Bottom);
    let l = layout(&s, id);
    let last = l.lines.last().unwrap();
    assert!((last.baseline + last.descent - 300.0).abs() < 1e-6);
    assert!(l.lines[0].baseline > top + 200.0);
    assert_ne!(text(&s, id).cached_bounds, before, "the cached bounds follow the text");
    assert_eq!((undo_len(&s), last_step(&s).as_str()), (n + 1, "Area Type Options"));
    s.execute("text.areaOptions", &json!({"verticalAlign": "center"})).unwrap();
    assert_eq!(text(&s, id).area.vertical_align, VerticalAlign::Center);
    // Unknown values are refused and change nothing.
    for bad in [json!({"verticalAlign": "middle"}), json!({"verticalAlign": 3}), json!({"verticalAlign": null})] {
        assert!(s.execute("text.areaOptions", &bad).is_err(), "{bad}");
    }
    assert_eq!(undo_len(&s), n + 2);
    assert_eq!(text(&s, id).area.vertical_align, VerticalAlign::Center);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(text(&s, id).area.vertical_align, VerticalAlign::Top);
    assert_eq!(layout(&s, id).lines[0].baseline, top);
    assert_eq!(text(&s, id).cached_bounds, before);
    assert!(cmd::find_command("text.areaOptions").unwrap().params.contains("verticalAlign?: top|center|bottom|justify"));
}
