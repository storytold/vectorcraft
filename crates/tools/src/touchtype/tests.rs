use vectorcraft_doc::{Document, Node, Selection};

use super::*;
use crate::testutil::{cx, paint};

/// 40 pt point type "Type" at (100, 200), selected.
fn doc_with_type() -> (Document, NodeId, Selection) {
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let t = TextObject::point(Point::new(100.0, 200.0), "Type", CharStyle { size: 40.0, ..Default::default() });
    d.insert(Some(l), 0, Node::new(id, NodeKind::Text(Box::new(t)))).unwrap();
    let mut s = Selection::default();
    s.add(id);
    (d, id, s)
}

/// The selected character's box in document space.
fn quad(t: &TouchTypeTool, cx: &ToolContext) -> [Point; 4] {
    let (_, text, b) = t.current(cx).expect("a character is selected");
    b.quad.map(|p| text.xf * p)
}

/// A press at `from`, a drag to `to` and the release: the actions of each.
fn drag(t: &mut TouchTypeTool, cx: &ToolContext, from: Point, to: Point) -> Vec<Vec<Action>> {
    [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)]
        .into_iter()
        .map(|(kind, p)| t.pointer(cx, &PointerEvent::new(kind, p.x, p.y)))
        .collect()
}

fn preview(acts: &[Action]) -> &Value {
    acts.iter()
        .find_map(|a| match a {
            Action::Preview(c, p) if c == "text.touchType" => Some(p),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no text.touchType preview: {acts:?}"))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn a_click_selects_one_character_and_shows_its_box() {
    let (d, id, s) = doc_with_type();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = TouchTypeTool::default();
    assert!(t.overlays(&cx).is_empty(), "no character yet");
    // Somewhere in the second character, the "y".
    let t_object = text_of(&cx, id).unwrap().clone();
    let lay = vectorcraft_text::layout(FontDb::global(), &t_object);
    let y = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
    let at = t_object.xf * (y.origin + Vec2::new(y.advance / 2.0, -10.0));
    let acts = drag(&mut t, &cx, at, at);
    assert!(acts.iter().all(Vec::is_empty), "a click changes nothing: {acts:?}");
    assert_eq!(t.options(), json!({"id": id.0, "start": 1}));
    // Its box: four sides and five handles, the rotate handle in the middle of the top edge.
    let ov = t.overlays(&cx);
    assert_eq!(ov.iter().filter(|o| matches!(o, Overlay::Line { .. })).count(), 4);
    assert_eq!(ov.iter().filter(|o| matches!(o, Overlay::Anchor { .. })).count(), 4);
    assert_eq!(ov.iter().filter(|o| matches!(o, Overlay::CurvePoint { .. })).count(), 1);
    let q = quad(&t, &cx);
    assert!(q[0].x <= at.x && at.x <= q[1].x && q[0].y < at.y && at.y < q[3].y, "{q:?} holds {at:?}");
    assert_eq!(t.cursor(&cx, q[0].midpoint(q[1]), Mods::default()), Cursor::rotate_about(t_object.xf * y.origin, q[0].midpoint(q[1])));
    assert_eq!(t.cursor(&cx, q[2], Mods::default()), Cursor::ResizeH);
    assert_eq!(t.cursor(&cx, q[0], Mods::default()), Cursor::ResizeV);
    assert_eq!(t.cursor(&cx, q[1], Mods::default()), Cursor::ResizeNeSw);
    assert_eq!(t.cursor(&cx, q[0].midpoint(q[2]), Mods::default()), Cursor::Move);
    // Escape lets go of it.
    assert!(t.claims_key(&cx, ToolKey::Escape));
    t.key(&cx, ToolKey::Escape, Mods::default());
    assert!(t.overlays(&cx).is_empty() && t.options() == json!({}));
    // A click on type that isn't selected selects it too.
    let none = Selection::default();
    let cx = crate::testutil::cx(&d, &none, &p);
    let acts = t.pointer(&cx, &PointerEvent::new(PointerKind::Down, at.x, at.y));
    assert_eq!(acts, vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
}

/// In a list item, the first character's box holds that character and not the bullet before
/// it, and a drag of the box moves that character.
#[test]
fn the_first_character_of_a_list_item_has_its_own_box_and_drags() {
    let (mut d, id, s) = doc_with_type();
    if let Some(NodeKind::Text(t)) = d.node_mut(id).map(|n| &mut n.kind) {
        t.para.list = Some(vectorcraft_doc::text::ParaList::bulleted(vectorcraft_doc::text::Bullet::Dot));
        t.para.left_indent = 18.0;
        t.para.first_line_indent = -18.0;
    }
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = TouchTypeTool::default();
    let t_object = text_of(&cx, id).unwrap().clone();
    let lay = vectorcraft_text::layout(FontDb::global(), &t_object);
    let first = lay.glyphs.iter().find(|g| g.byte == 0 && g.len > 0).unwrap();
    let at = t_object.xf * (first.origin + Vec2::new(first.advance / 2.0, -10.0));
    drag(&mut t, &cx, at, at);
    assert_eq!(t.options(), json!({"id": id.0, "start": 0}));
    let (q, pen) = (quad(&t, &cx), t_object.xf * first.origin);
    assert!(q[0].x > pen.x - 5.0, "the box starts at the T, 18 pt past the bullet: {q:?}, the T at {pen:?}");
    let acts = drag(&mut t, &cx, at, at + Vec2::new(0.0, -4.0));
    assert_eq!(preview(&acts[1])["start"].as_u64(), Some(0));
    // A drag on the character once the type is deselected selects the type and moves it.
    let none = Selection::default();
    let cx = crate::testutil::cx(&d, &none, &p);
    let acts = drag(&mut t, &cx, at, at + Vec2::new(0.0, -4.0));
    assert_eq!(acts[0], vec![Action::Exec("select.set".into(), json!({"ids": [id.0]}))]);
    assert_eq!(preview(&acts[1])["start"].as_u64(), Some(0));
}

#[test]
fn dragging_the_box_moves_the_character_along_its_line_and_up() {
    let (d, id, s) = doc_with_type();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = TouchTypeTool::default();
    let lay = vectorcraft_text::layout(FontDb::global(), text_of(&cx, id).unwrap());
    let y = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
    let at = Point::new(100.0, 200.0) + (y.origin.to_vec2() + Vec2::new(y.advance / 2.0, -10.0));
    // 10 pt right and 4 pt up: one preview, one undo step.
    let acts = drag(&mut t, &cx, at, at + Vec2::new(10.0, -4.0));
    assert_eq!(acts[1][0], Action::Begin("Touch Type".into()));
    let p1 = preview(&acts[1]);
    assert_eq!((p1["id"].as_u64(), p1["start"].as_u64()), (Some(id.0), Some(1)));
    assert!(close(p1["dx"].as_f64().unwrap(), 10.0) && close(p1["dy"].as_f64().unwrap(), 4.0), "{p1}");
    assert_eq!(acts[2], vec![Action::Commit]);
    // The lower-left handle moves it too.
    let bl = quad(&t, &cx)[3];
    let acts = drag(&mut t, &cx, bl, bl + Vec2::new(-3.0, 2.0));
    let p2 = preview(&acts[1]);
    assert!(close(p2["dx"].as_f64().unwrap(), -3.0) && close(p2["dy"].as_f64().unwrap(), -2.0), "{p2}");
    // The first character has no character before it on its line: it moves only up and down.
    let first = Point::new(100.0, 200.0) + Vec2::new(lay.glyphs[0].advance / 2.0, -10.0);
    let acts = drag(&mut t, &cx, first, first + Vec2::new(10.0, -4.0));
    let p3 = preview(&acts[1]);
    assert_eq!(p3["start"], json!(0));
    assert!(close(p3.get("dx").and_then(|v| v.as_f64()).unwrap_or(0.0), 0.0) && close(p3["dy"].as_f64().unwrap(), 4.0), "{p3}");
}

#[test]
fn arrow_keys_nudge_the_selected_character() {
    let (d, id, s) = doc_with_type();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = TouchTypeTool::default();
    let lay = vectorcraft_text::layout(FontDb::global(), text_of(&cx, id).unwrap());
    let y = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
    let at = Point::new(100.0, 200.0) + (y.origin.to_vec2() + Vec2::new(y.advance / 2.0, -10.0));
    drag(&mut t, &cx, at, at);
    assert!(t.claims_key(&cx, ToolKey::Right));
    let acts = t.key(&cx, ToolKey::Right, Mods::default());
    let p1 = match &acts[0] {
        Action::Exec(c, v) if c == "text.touchType" => v,
        other => panic!("{other:?}"),
    };
    assert!(close(p1["dx"].as_f64().unwrap(), 1.0) && p1.get("dy").is_none(), "{p1}");
    // The first character can't kern: a horizontal nudge does nothing.
    let first = Point::new(100.0, 200.0) + Vec2::new(lay.glyphs[0].advance / 2.0, -10.0);
    drag(&mut t, &cx, first, first);
    assert!(t.key(&cx, ToolKey::Left, Mods::default()).is_empty());
    let acts = t.key(&cx, ToolKey::Up, Mods::default());
    let p2 = match &acts[0] {
        Action::Exec(_, v) => v,
        other => panic!("{other:?}"),
    };
    assert!(close(p2["dy"].as_f64().unwrap(), 1.0), "{p2}");
    // Shift multiplies the increment (on a character that can kern).
    drag(&mut t, &cx, at, at);
    let acts = t.key(&cx, ToolKey::Right, Mods { shift: true, ..Mods::default() });
    let p3 = match &acts[0] {
        Action::Exec(_, v) => v,
        other => panic!("{other:?}"),
    };
    assert!(close(p3["dx"].as_f64().unwrap(), 10.0), "{p3}");
}

#[test]
fn the_handles_scale_and_rotate_the_character() {
    let (d, id, s) = doc_with_type();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let mut t = TouchTypeTool::default();
    let lay = vectorcraft_text::layout(FontDb::global(), text_of(&cx, id).unwrap());
    let y = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
    let at = Point::new(100.0, 200.0) + (y.origin.to_vec2() + Vec2::new(y.advance / 2.0, -10.0));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, at.x, at.y));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, at.x, at.y));
    let [tl, tr, br, _] = quad(&t, &cx);
    let (w, h) = (tr.x - tl.x, br.y - tr.y);
    let above = 200.0 - tl.y;
    // The lower-right handle: horizontal scale; half the width again is 150 %.
    let acts = drag(&mut t, &cx, br, br + Vec2::new(w / 2.0, 7.0));
    let ps = preview(&acts[1]);
    assert!(close(ps["hScale"].as_f64().unwrap(), 150.0) && ps.get("vScale").is_none(), "{ps}");
    // The upper-left handle: vertical scale, measured from the baseline.
    let acts = drag(&mut t, &cx, tl, tl + Vec2::new(5.0, -above));
    let ps = preview(&acts[1]);
    assert!(close(ps["vScale"].as_f64().unwrap(), 200.0) && ps.get("hScale").is_none(), "{ps}");
    // The upper-right handle: both alike.
    let pen = Point::new(tl.x, 200.0);
    let acts = drag(&mut t, &cx, tr, pen + (tr - pen) * 0.5);
    let ps = preview(&acts[1]);
    assert!(close(ps["hScale"].as_f64().unwrap(), 50.0) && close(ps["vScale"].as_f64().unwrap(), 50.0), "{ps}");
    // The top handle: a quarter turn counter-clockwise about its pen position on the baseline.
    let top = tl.midpoint(tr);
    let pivot = Point::new(100.0, 200.0) + y.origin.to_vec2();
    let r = top - pivot;
    let acts = drag(&mut t, &cx, top, pivot + Vec2::new(r.y, -r.x));
    let ps = preview(&acts[1]);
    assert!(close(ps["rotation"].as_f64().unwrap(), 90.0), "{ps}");
    assert!(h > 0.0);
    // Escape mid-drag cancels the drag and keeps the character.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, top.x, top.y));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, top.x + 20.0, top.y));
    assert!(t.claims_key(&cx, ToolKey::Escape));
    assert_eq!(t.key(&cx, ToolKey::Escape, Mods::default()), vec![Action::Cancel]);
    assert_eq!(t.pointer(&cx, &PointerEvent::new(PointerKind::Up, top.x + 20.0, top.y)), vec![]);
    assert_eq!(t.options(), json!({"id": id.0, "start": 1}));
    // Switching tools mid-drag drops the drag.
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, top.x, top.y));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Drag, top.x + 20.0, top.y));
    assert_eq!(t.deactivate(&cx), vec![Action::Cancel]);
}

#[test]
fn the_box_of_a_character_drawn_with_two_glyphs_holds_both() {
    // An All Caps ß is two glyphs (SS) with one byte range.
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let text = TextObject::point(Point::new(100.0, 200.0), "aße", CharStyle { size: 40.0, all_caps: true, ..Default::default() });
    d.insert(Some(l), 0, Node::new(id, NodeKind::Text(Box::new(text.clone())))).unwrap();
    let mut s = Selection::default();
    s.add(id);
    let p = paint();
    let cx = cx(&d, &s, &p);
    let lay = vectorcraft_text::layout(FontDb::global(), &text);
    let esses: Vec<_> = lay.glyphs.iter().filter(|g| g.byte == 1).collect();
    assert_eq!(esses.len(), 2);
    // A click on the second S selects the ß, and its box reaches from the first S to the end of the second.
    let second = esses[1];
    let at = text.xf * (second.origin + Vec2::new(second.advance / 2.0, -10.0));
    let mut t = TouchTypeTool::default();
    drag(&mut t, &cx, at, at);
    assert_eq!(t.options(), json!({"id": id.0, "start": 1}));
    let q = quad(&t, &cx);
    let ends = |g: &PositionedGlyph| (text.xf * glyph_quad(g).unwrap()[1]).x;
    assert!((q[1].x - ends(second)).abs() < 1e-9 && q[0].x < ends(esses[0]), "{q:?}");
    // Dragging the lower-right handle by half the box's width is 150 %.
    let w = q[1].x - q[0].x;
    let acts = drag(&mut t, &cx, q[2], q[2] + Vec2::new(w / 2.0, 0.0));
    assert!(close(preview(&acts[1])["hScale"].as_f64().unwrap(), 150.0));
    // Rotated, each S turns about its own origin: the box still holds both, square to the first.
    let mut d = Document::new(500.0, 500.0);
    let l = d.layers[0].id;
    let id = d.alloc_id();
    let text = TextObject::point(Point::new(100.0, 200.0), "aße", CharStyle { size: 40.0, all_caps: true, rotation: 30.0, ..Default::default() });
    d.insert(Some(l), 0, Node::new(id, NodeKind::Text(Box::new(text.clone())))).unwrap();
    let mut s = Selection::default();
    s.add(id);
    let cx = crate::testutil::cx(&d, &s, &p);
    let t = TouchTypeTool { target: Some((id, 1)), ..Default::default() };
    let q = quad(&t, &cx);
    let (across, down) = (q[1] - q[0], q[3] - q[0]);
    assert!(across.dot(down).abs() < 1e-9, "a rectangle: {q:?}");
    let lay = vectorcraft_text::layout(FontDb::global(), &text);
    for g in lay.glyphs.iter().filter(|g| g.byte == 1) {
        for c in glyph_quad(g).unwrap() {
            let v = text.xf * c - q[0];
            let (a, b) = (v.dot(across) / across.hypot2(), v.dot(down) / down.hypot2());
            assert!((-1e-9..=1.0 + 1e-9).contains(&a) && (-1e-9..=1.0 + 1e-9).contains(&b), "{c:?} outside {q:?}");
        }
    }
}

/// A glyph skewed by its transform (the Skew effect of type on a path) keeps its own box.
#[test]
fn a_skewed_glyph_keeps_its_own_box() {
    let text = TextObject::point(Point::new(100.0, 200.0), "Type", CharStyle { size: 40.0, ..Default::default() });
    let mut lay = vectorcraft_text::layout(FontDb::global(), &text);
    lay.glyphs[1].xf *= Affine::skew(0.5, 0.0);
    let b = char_box(&text, &lay, 1).unwrap();
    let own = glyph_quad(&lay.glyphs[1]).unwrap();
    assert!(b.quad.iter().zip(own).all(|(a, b)| (*a - b).hypot() < 1e-9), "{:?} {own:?}", b.quad);
}

/// `create` makes the Touch Type tool: a click with it selects a character.
#[test]
fn the_tool_is_made_by_its_id() {
    let (d, id, s) = doc_with_type();
    let p = paint();
    let cx = cx(&d, &s, &p);
    let lay = vectorcraft_text::layout(FontDb::global(), text_of(&cx, id).unwrap());
    let at = Point::new(100.0, 200.0) + Vec2::new(lay.glyphs[0].advance / 2.0, -10.0);
    let mut t = crate::create(ID);
    assert_eq!(t.id(), ID);
    t.pointer(&cx, &PointerEvent::new(PointerKind::Down, at.x, at.y));
    t.pointer(&cx, &PointerEvent::new(PointerKind::Up, at.x, at.y));
    assert_eq!(t.options(), json!({"id": id.0, "start": 0}));
}
