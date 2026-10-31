//! Rich text editing: range edit/style commands, the Type tools through the session, area and
//! on-path type from paths, Fit Headline, Convert To Area/Point Type.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind, TextKind, TextObject};
use vectorcraft_geom::Point;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

fn session() -> Session {
    // Type placed by the Type tools starts empty (Fill New Type Objects With Placeholder Text off).
    let mut s = Session::new();
    s.prefs.placeholder_text = false;
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn text(s: &mut Session, t: &str) -> NodeId {
    let r = s.execute("text.create", &json!({"x": 100, "y": 100, "text": t, "size": 20})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn obj(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

fn click(s: &mut Session, x: f64, y: f64) {
    let v = ViewInfo::default();
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, x, y), v).unwrap();
    }
}

fn key(s: &mut Session, k: ToolKey, m: Mods) {
    s.tool_key(k, m, ViewInfo::default()).unwrap();
}

fn undo_labels(s: &Session) -> Vec<String> {
    s.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect()
}

#[test]
fn edit_range_replaces_and_undoes() {
    let mut s = session();
    let id = text(&mut s, "Hello world");
    let r = s.execute("text.editRange", &json!({"id": id.0, "start": 6, "end": 11, "insert": "there"})).unwrap();
    assert_eq!(r["caret"], json!(11));
    assert_eq!(obj(&s, id).plain_text(), "Hello there");
    // Out-of-range and reversed offsets are clamped.
    s.execute("text.editRange", &json!({"id": id.0, "start": 99, "end": 5, "insert": "!"})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "Hello!");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "Hello there");
    // An offset of 2^32 is past the end too. A 32-bit build read it as 0 and inserted at the start.
    s.execute("text.editRange", &json!({"id": id.0, "start": 1u64 << 32, "end": 1u64 << 32, "insert": "!"})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "Hello there!");
    assert!(s.execute("text.editRange", &json!({"start": 0})).is_err(), "id required");
    let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    assert!(s.execute("text.editRange", &json!({"id": rect["id"], "insert": "x"})).is_err(), "not text");
}

#[test]
fn set_range_style_splits_runs() {
    let mut s = session();
    let id = text(&mut s, "Hello brave world");
    let w0 = obj(&s, id).cached_bounds.unwrap().width();
    let r = s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 6, "end": 11, "size": 40, "fill": "#ff0000", "tracking": 50})).unwrap();
    assert_eq!(r["runs"], json!(3));
    let t = obj(&s, id);
    assert_eq!(t.runs[1].text, "brave");
    assert_eq!((t.runs[0].style.size, t.runs[1].style.size, t.runs[2].style.size), (20.0, 40.0, 20.0));
    assert_eq!(t.runs[1].style.tracking, 50.0);
    assert!(t.cached_bounds.unwrap().width() > w0 * 1.2, "bounds refreshed");
    // Kerning / scales / baseline / caps on another range; "auto" leading.
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 0, "end": 5, "kerning": 20, "hScale": 80, "vScale": 120, "baselineShift": 3, "rotation": 370, "allCaps": true, "leading": "auto", "font": "Inter", "strokeWidth": 1})).unwrap();
    let st = obj(&s, id).runs[0].style.clone();
    assert_eq!((st.kerning, st.h_scale, st.v_scale, st.baseline_shift, st.all_caps), (Some(20.0), 80.0, 120.0, 3.0, true));
    assert!((st.rotation - 10.0).abs() < 1e-9);
    assert_eq!(st.font_family, "Inter");
    // One undo step per command.
    assert_eq!(undo_labels(&s).iter().filter(|l| *l == "Character").count(), 2);
    assert!(s.execute("text.setRangeStyle", &json!({"id": id.0})).is_err(), "nothing to change");
    assert!(s.execute("text.setRangeStyle", &json!({"id": id.0, "size": -1})).is_err());
}

/// A character style applied from byte 2^32 to 2^32 + 5 styles no character, as a range past the
/// end does. A 32-bit build read the range as bytes 0 to 5 and styled "Hello".
#[test]
fn char_style_range_past_u32_styles_nothing() {
    let mut s = session();
    let id = text(&mut s, "Hello world");
    s.execute("charStyle.new", &json!({"name": "Big", "attrs": {"size": 40.0}})).unwrap();
    s.execute("charStyle.apply", &json!({"name": "Big", "id": id.0, "start": 1u64 << 32, "end": (1u64 << 32) + 5})).unwrap();
    assert!(obj(&s, id).runs.iter().all(|r| r.style.size == 20.0 && r.style.style_name.is_none()), "{:?}", obj(&s, id).runs);
}

#[test]
fn set_range_style_defaults_to_all_text_and_get_range() {
    let mut s = session();
    let id = text(&mut s, "abc");
    s.execute("text.setRangeStyle", &json!({"id": id.0, "size": 30})).unwrap();
    assert_eq!(obj(&s, id).runs.len(), 1);
    assert_eq!(obj(&s, id).runs[0].style.size, 30.0);
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 1, "end": 2, "size": 10})).unwrap();
    let r = s.execute("text.getRange", &json!({"id": id.0, "start": 0, "end": 2})).unwrap();
    assert_eq!(r["text"], json!("ab"));
    assert_eq!(r["runs"].as_array().unwrap().len(), 2);
    assert_eq!(r["length"], json!(3));
}

/// The type commands refuse a number or an on/off option given as text or as another value of the
/// wrong kind, naming the key, and leave the document and its undo history as they were; true,
/// false, numbers and null still work.
#[test]
fn type_commands_refuse_a_number_or_boolean_given_as_text() {
    use crate::tests::{not_a_number, not_a_switch, not_refused};
    let mut s = session();
    let id = text(&mut s, "abc");
    let frame = s.execute("shape.rectangle", &json!({"x": 300, "y": 300, "width": 100, "height": 50})).unwrap()["id"].clone();
    let boxed = s.execute("text.create", &json!({"x": 0, "y": 300, "text": "a", "area": {"width": 100, "height": 50}})).unwrap()["id"].clone();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let area = json!({"x": 0, "y": 0, "text": "a", "area": {"width": "200", "height": 100}});
    let mut calls = vec![("text.create".to_string(), area, "`width` must be a number, not \"200\"".to_string())];
    let area = json!({"x": 0, "y": 0, "text": "a", "area": {"width": 200, "height": 100, "fit": "shrinkText", "fitMinPercent": "40"}});
    calls.push(("text.create".to_string(), area, "`fitMinPercent` must be a number, not \"40\"".to_string()));
    calls.extend(
        [
            ("text.create", json!({"x": 0, "y": 0, "text": "a", "size": "12"}), "size"),
            ("text.createInPath", json!({"path": frame, "text": "a", "size": [12]}), "size"),
            ("text.createInPath", json!({"path": frame, "mode": "area", "fitMinPercent": "40"}), "fitMinPercent"),
            ("text.setRangeStyle", json!({"id": id.0, "size": "30"}), "size"),
            ("text.setRangeStyle", json!({"id": id.0, "tracking": "50"}), "tracking"),
            ("text.setRangeStyle", json!({"id": id.0, "hScale": true}), "hScale"),
            ("text.setRangeStyle", json!({"id": id.0, "strokeWidth": "1"}), "strokeWidth"),
            ("text.setRangeStyle", json!({"id": id.0, "start": "1", "size": 30}), "start"),
            ("text.setRangeStyle", json!({"id": id.0, "end": [1], "size": 30}), "end"),
            ("text.editRange", json!({"id": id.0, "start": 0, "end": "1", "insert": "x"}), "end"),
            ("text.getRange", json!({"id": id.0, "start": "1"}), "start"),
            ("text.touchType", json!({"id": id.0, "start": 0, "rotation": "10"}), "rotation"),
            ("text.touchType", json!({"id": id.0, "start": "0", "rotation": 10}), "start"),
            ("type.step", json!({"attribute": "size", "by": "2"}), "by"),
            ("type.step", json!({"attribute": "size", "id": id.0, "start": "1"}), "start"),
            ("type.size.increase", json!({"by": "2"}), "by"),
            ("type.size.decrease", json!({"by": [2]}), "by"),
            ("type.ruby", json!({"text": "a", "size": "60"}), "size"),
            ("text.areaOptions", json!({"ids": [boxed], "width": "200"}), "width"),
            ("text.areaOptions", json!({"ids": [boxed], "height": [80]}), "height"),
            ("text.areaOptions", json!({"ids": [boxed], "fit": "shrinkText", "fitMinPercent": "40"}), "fitMinPercent"),
        ]
        .map(|(cmd, p, key)| not_a_number(cmd, p, key)),
    );
    calls.extend(
        [
            ("text.create", json!({"x": 0, "y": 0, "text": "a", "vertical": "true"}), "vertical"),
            ("text.create", json!({"x": 0, "y": 0, "placeholder": 1}), "placeholder"),
            ("text.createInPath", json!({"path": frame, "vertical": "true"}), "vertical"),
            ("text.createInPath", json!({"path": frame, "placeholder": "true"}), "placeholder"),
            ("text.setRangeStyle", json!({"id": id.0, "underline": "true"}), "underline"),
            ("text.setRangeStyle", json!({"id": id.0, "allCaps": 1}), "allCaps"),
            ("text.setRangeStyle", json!({"id": id.0, "noBreak": "true"}), "noBreak"),
            ("text.setRangeStyle", json!({"id": id.0, "size": 30, "tateChuYoko": 1}), "tateChuYoko"),
            ("text.getRange", json!({"id": id.0, "effective": "true"}), "effective"),
            ("type.opticalMarginAlignment", json!({"on": "true"}), "on"),
            ("type.systemLayout", json!({"on": "true"}), "on"),
            ("type.fractionalWidths", json!({"on": "false"}), "on"),
            ("type.ruby", json!({"remove": "true"}), "remove"),
            ("type.ruby", json!({"text": "a", "overhang": "false"}), "overhang"),
        ]
        .map(|(cmd, p, key)| not_a_switch(cmd, p, key)),
    );
    let wrong = not_refused(&mut s, &calls);
    assert!(wrong.is_empty(), "not refused: {wrong:#?}");
    let style = json!({"id": id.0, "size": 30, "underline": true, "tracking": null, "start": null, "noBreak": true});
    s.execute("text.setRangeStyle", &style).unwrap();
    s.execute("type.size.increase", &json!({"by": null})).unwrap();
    s.execute("type.opticalMarginAlignment", &json!({"on": true})).unwrap();
    s.execute("type.ruby", &json!({"text": "a", "size": null, "overhang": false})).unwrap();
    let t = obj(&s, id);
    let st = &t.runs[0].style;
    assert!(st.underline && st.no_break && t.optical_margin && st.size > 30.0, "{st:?}");
    assert!(!t.ruby.is_empty() && t.ruby.iter().all(|r| !r.overhang && r.size_percent == 50.0), "{:?}", t.ruby);
}

/// Where each glyph of type object `id` is drawn from (its origin in font units), document space.
fn glyph_pens(s: &Session, id: NodeId) -> Vec<(usize, Point)> {
    let t = obj(s, id);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    lay.glyphs.iter().map(|g| (g.byte, t.xf * (g.xf * Point::ZERO))).collect()
}

fn moved_by(before: &[(usize, Point)], after: &[(usize, Point)], byte: usize) -> (f64, f64) {
    let at = |v: &[(usize, Point)]| v.iter().find(|(b, _)| *b == byte).map(|(_, p)| *p).unwrap_or_else(|| panic!("no glyph at byte {byte}"));
    let d = at(after) - at(before);
    (d.x, d.y)
}

fn near(a: (f64, f64), b: (f64, f64)) -> bool {
    (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6
}

#[test]
fn touch_type_moves_one_character_and_leaves_the_others_where_they_are() {
    let mut s = session();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type", "size": 40})).unwrap()["id"].as_u64().unwrap());
    let before = glyph_pens(&s, id);
    let r = s.execute("text.touchType", &json!({"id": id.0, "start": 1, "dx": 10, "dy": 4})).unwrap();
    assert_eq!((r["start"].clone(), r["end"].clone()), (json!(1), json!(2)));
    let after = glyph_pens(&s, id);
    assert!(near(moved_by(&before, &after, 1), (10.0, -4.0)), "the y moves 10 pt right and 4 pt up: {:?}", moved_by(&before, &after, 1));
    for byte in [0, 2, 3] {
        assert!(near(moved_by(&before, &after, byte), (0.0, 0.0)), "byte {byte} stays: {:?}", moved_by(&before, &after, byte));
    }
    // Kerning on the T and the y, and a baseline shift on the y: live attributes, one undo step.
    let t = obj(&s, id);
    let style = |byte: usize| vectorcraft_text::edit::slice_runs(&t.runs, byte, byte + 1)[0].style.clone();
    let (k0, k1) = (style(0).kerning.unwrap(), style(1).kerning.unwrap());
    assert!(k0 > 0.0 && k1 < 0.0, "{k0} {k1}");
    assert!((style(1).baseline_shift - 4.0).abs() < 1e-9 && style(2).baseline_shift == 0.0);
    assert_eq!(undo_labels(&s).iter().filter(|l| *l == "Touch Type").count(), 1);
    // The type stays editable, and the moved character keeps its attributes.
    s.execute("text.editRange", &json!({"id": id.0, "start": 4, "end": 4, "insert": "!"})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "Type!");
    assert!(near(moved_by(&before, &glyph_pens(&s, id), 1), (10.0, -4.0)));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(near(moved_by(&before, &glyph_pens(&s, id), 1), (0.0, 0.0)), "undone");
    // The first character of a line moves only up and down.
    assert!(s.execute("text.touchType", &json!({"id": id.0, "start": 0, "dx": 5})).is_err());
    s.execute("text.touchType", &json!({"id": id.0, "start": 0, "dy": -2})).unwrap();
    assert!(near(moved_by(&before, &glyph_pens(&s, id), 0), (0.0, 2.0)));
    assert!(s.execute("text.touchType", &json!({"id": id.0, "start": 1})).is_err(), "nothing to change");
    assert!(s.execute("text.touchType", &json!({"id": id.0, "start": 9, "dy": 1})).is_err(), "no character there");
    // A 32-bit build read 2^32 + 1 as character 1.
    assert!(s.execute("text.touchType", &json!({"id": id.0, "start": (1u64 << 32) + 1, "dy": 1})).is_err(), "no character there");
}

/// Touch Type on the first character of a list item moves that character and leaves the bullet
/// where it is. The bullet stands for no character, so the first character still moves only up
/// and down.
#[test]
fn touch_type_moves_the_first_character_of_a_list_item_and_not_its_bullet() {
    let mut s = session();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type", "size": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("text.setFormat", &json!({"ids": [id.0], "list": "bullet"})).unwrap();
    let pens = |s: &Session| {
        let t = obj(s, id);
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
        let bullet = lay.glyphs.iter().find(|g| g.len == 0).map(|g| t.xf * (g.xf * Point::ZERO)).unwrap();
        (bullet, lay.glyphs.iter().filter(|g| g.len > 0).map(|g| (g.byte, t.xf * (g.xf * Point::ZERO))).collect::<Vec<_>>())
    };
    let (bullet, before) = pens(&s);
    assert!(s.execute("text.touchType", &json!({"id": id.0, "start": 0, "dx": 5})).is_err(), "no character before it to kern");
    s.execute("text.touchType", &json!({"id": id.0, "start": 0, "dy": 3})).unwrap();
    let (bullet_after, after) = pens(&s);
    assert!(near(moved_by(&before, &after, 0), (0.0, -3.0)), "the T moves 3 pt up: {:?}", moved_by(&before, &after, 0));
    assert!(near(moved_by(&before, &after, 1), (0.0, 0.0)));
    assert!((bullet_after - bullet).hypot() < 1e-6, "the bullet stays: {bullet:?} {bullet_after:?}");
}

/// Raising, scaling or rotating a character leaves it and the characters after it where they
/// were along the line, also where the font kerns it with its neighbors (the T and the y).
#[test]
fn touch_type_keeps_the_kerning_of_a_raised_or_scaled_character() {
    let mut s = session();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type", "size": 40})).unwrap()["id"].as_u64().unwrap());
    let before = glyph_pens(&s, id);
    s.execute("text.touchType", &json!({"id": id.0, "start": 1, "dy": 5})).unwrap();
    let after = glyph_pens(&s, id);
    assert!(near(moved_by(&before, &after, 1), (0.0, -5.0)), "{:?}", moved_by(&before, &after, 1));
    for byte in [0, 2, 3] {
        assert!(near(moved_by(&before, &after, byte), (0.0, 0.0)), "byte {byte}: {:?}", moved_by(&before, &after, byte));
    }
    // A wider y pushes the p and e on by what it widens, and no further.
    s.execute("edit.undo", &json!({})).unwrap();
    let width = |s: &Session| {
        let t = obj(s, id);
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
        let y = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
        let face = vectorcraft_text::FontDb::global().face_by_id(y.font_id).unwrap();
        let [a, b, ..] = y.xf.as_coeffs();
        face.advance(y.gid) * a.hypot(b)
    };
    let w0 = width(&s);
    s.execute("text.touchType", &json!({"id": id.0, "start": 1, "hScale": 150})).unwrap();
    let after = glyph_pens(&s, id);
    assert!(near(moved_by(&before, &after, 1), (0.0, 0.0)), "{:?}", moved_by(&before, &after, 1));
    let wider = width(&s) - w0;
    assert!((wider - w0 / 2.0).abs() < 1e-6, "{wider} {w0}");
    for byte in [2, 3] {
        assert!(near(moved_by(&before, &after, byte), (wider, 0.0)), "byte {byte}: {:?}", moved_by(&before, &after, byte));
    }
    // A move is limited to what kerning can give (-1000/1000 em on the y): the p does not move
    // by more than the font's kerning pair with the y.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("text.touchType", &json!({"id": id.0, "start": 1, "dx": 80})).unwrap();
    let after = glyph_pens(&s, id);
    let (y, _) = moved_by(&before, &after, 1);
    assert!((39.0..=41.0).contains(&y), "the y stops at about 40 pt (1 em): {y}");
    assert!(moved_by(&before, &after, 2).0.abs() < 1.0, "{:?}", moved_by(&before, &after, 2));
}

/// A character drawn with two glyphs (ß in All Caps is SS) widens by both when it is scaled.
#[test]
fn touch_type_scales_a_character_drawn_with_two_glyphs() {
    let mut s = session();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "aße", "size": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("text.setRangeStyle", &json!({"id": id.0, "allCaps": true})).unwrap();
    let t = obj(&s, id);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let esses: Vec<_> = lay.glyphs.iter().filter(|g| g.byte == 1).collect();
    assert_eq!(esses.len(), 2, "ß in All Caps is two glyphs");
    let width: f64 = esses
        .iter()
        .map(|g| {
            let face = vectorcraft_text::FontDb::global().face_by_id(g.font_id).unwrap();
            let [a, b, ..] = g.xf.as_coeffs();
            face.advance(g.gid) * a.hypot(b)
        })
        .sum();
    let before = glyph_pens(&s, id);
    s.execute("text.touchType", &json!({"id": id.0, "start": 1, "hScale": 150})).unwrap();
    // Half the width of SS (the font's kerning between the two S glyphs scales with them).
    let (x, _) = moved_by(&before, &glyph_pens(&s, id), 3);
    assert!((x - width / 2.0).abs() < 1.0, "the E moves on by half the width of SS: {x} {width}");
}

/// In a centered or right-aligned line, the kerning still moves only the dragged character.
#[test]
fn touch_type_moves_one_character_in_centered_and_right_aligned_lines() {
    for justify in ["center", "right"] {
        let mut s = session();
        let r = s.execute("text.create", &json!({"x": 200, "y": 100, "text": "Type", "size": 40, "justify": justify})).unwrap();
        let id = NodeId(r["id"].as_u64().unwrap());
        let before = glyph_pens(&s, id);
        s.execute("text.touchType", &json!({"id": id.0, "start": 1, "dx": 10, "dy": 4})).unwrap();
        let after = glyph_pens(&s, id);
        assert!(near(moved_by(&before, &after, 1), (10.0, -4.0)), "{justify}: {:?}", moved_by(&before, &after, 1));
        for byte in [2, 3] {
            assert!(near(moved_by(&before, &after, byte), (0.0, 0.0)), "{justify}, byte {byte}: {:?}", moved_by(&before, &after, byte));
        }
    }
}

/// `dy` moves a rotated character that many document points along its own up axis, also in type
/// scaled unevenly.
#[test]
fn touch_type_raises_a_rotated_character_in_unevenly_scaled_type() {
    let mut s = session();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type", "size": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.transform", &json!({"ids": [id.0], "matrix": [2, 0, 0, 1, 0, 0]})).unwrap();
    s.execute("text.touchType", &json!({"id": id.0, "start": 1, "rotation": 90})).unwrap();
    let before = glyph_pens(&s, id);
    s.execute("text.touchType", &json!({"id": id.0, "start": 1, "dy": 10})).unwrap();
    let (x, y) = moved_by(&before, &glyph_pens(&s, id), 1);
    assert!(((x * x + y * y).sqrt() - 10.0).abs() < 1e-6, "({x}, {y})");
}

/// `dy` moves a character that many document points in type scaled up 2000 times, where baseline
/// shift is limited to 0.648 pt of text space.
#[test]
fn touch_type_raises_a_character_in_type_scaled_up_2000_times() {
    let mut s = session();
    let id = NodeId(s.execute("text.create", &json!({"x": 0, "y": 0, "text": "Type", "size": 1})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.transform", &json!({"ids": [id.0], "matrix": [2000, 0, 0, 2000, 0, 0]})).unwrap();
    let before = glyph_pens(&s, id);
    s.execute("text.touchType", &json!({"id": id.0, "start": 1, "dy": 10})).unwrap();
    let (x, y) = moved_by(&before, &glyph_pens(&s, id), 1);
    assert!(near((x, y), (0.0, -10.0)), "({x}, {y})");
}

/// A character whose baseline shift is beyond the range that the object's scale leaves keeps its
/// shift through a change that does not raise it, and a `dy` moves it back into the range.
#[test]
fn touch_type_moves_a_character_shifted_beyond_the_range() {
    let mut s = session();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type", "size": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 1, "end": 2, "baselineShift": 1000})).unwrap();
    s.execute("object.transform", &json!({"ids": [id.0], "matrix": [2, 0, 0, 2, 0, 0]})).unwrap();
    let before = glyph_pens(&s, id);
    s.execute("text.touchType", &json!({"id": id.0, "start": 1, "hScale": 150})).unwrap();
    let (_, up) = moved_by(&before, &glyph_pens(&s, id), 1);
    assert!(up.abs() < 1e-6, "the shift stays: {up}");
    let before = glyph_pens(&s, id);
    s.execute("text.touchType", &json!({"id": id.0, "start": 1, "dy": -10})).unwrap();
    let (_, down) = moved_by(&before, &glyph_pens(&s, id), 1);
    assert!(down >= 10.0 - 1e-6, "{down}");
}

/// `dy` moves a rotated character on a path with the Skew effect that many document points too,
/// where the effect stretches its baseline shift.
#[test]
fn touch_type_raises_a_rotated_character_on_a_skewed_path() {
    let mut s = session();
    let pid = s.execute("path.create", &json!({"d": "M100 400 L400 100"})).unwrap()["id"].as_u64().unwrap();
    let r = s.execute("text.createInPath", &json!({"path": pid, "mode": "onPath", "text": "Type", "size": 40, "at": [100, 400]})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("type.pathOptions", &json!({"effect": "skew"})).unwrap();
    s.execute("text.touchType", &json!({"id": id.0, "start": 1, "rotation": 45})).unwrap();
    let before = glyph_pens(&s, id);
    s.execute("text.touchType", &json!({"id": id.0, "start": 1, "dy": 10})).unwrap();
    let (x, y) = moved_by(&before, &glyph_pens(&s, id), 1);
    assert!(((x * x + y * y).sqrt() - 10.0).abs() < 1e-6, "({x}, {y})");
}

/// A right-to-left character moves only up and down.
#[test]
fn touch_type_moves_a_right_to_left_character_only_up_and_down() {
    let mut s = session();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "אבג", "size": 40})).unwrap()["id"].as_u64().unwrap());
    assert!(s.execute("text.touchType", &json!({"id": id.0, "start": 2, "dx": 5})).is_err());
    let before = glyph_pens(&s, id);
    s.execute("text.touchType", &json!({"id": id.0, "start": 2, "dy": 3})).unwrap();
    let (_, up) = moved_by(&before, &glyph_pens(&s, id), 2);
    assert!((up + 3.0).abs() < 1e-9, "{up}");
}

#[test]
fn touch_type_scales_and_rotates_one_character() {
    let mut s = session();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type", "size": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("text.touchType", &json!({"id": id.0, "start": 2, "hScale": 150, "vScale": 50, "rotation": 30})).unwrap();
    let t = obj(&s, id);
    let st = |byte: usize| vectorcraft_text::edit::slice_runs(&t.runs, byte, byte + 1)[0].style.clone();
    assert_eq!((st(2).h_scale, st(2).v_scale, st(2).rotation), (150.0, 50.0, 30.0));
    for byte in [1, 3] {
        assert_eq!((st(byte).h_scale, st(byte).v_scale, st(byte).rotation), (100.0, 100.0, 0.0));
    }
}

/// The Touch Type tool through the session: a click on a character selects it, and dragging its
/// box sends `text.touchType` as one undo step.
#[test]
fn the_touch_type_tool_drags_a_character() {
    let mut s = session();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type", "size": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let before = glyph_pens(&s, id);
    let v = ViewInfo::default();
    s.select_tool("touchType", v).unwrap();
    let t = obj(&s, id);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let y = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
    let at = t.xf * (y.origin + vectorcraft_geom::Vec2::new(y.advance / 2.0, -10.0));
    click(&mut s, at.x, at.y);
    assert_eq!(s.doc().unwrap().selection.objects, vec![id], "the click selects the type");
    assert_eq!(s.tool_options(), json!({"id": id.0, "start": 1}));
    for (k, x, y) in [(PointerKind::Down, at.x, at.y), (PointerKind::Drag, at.x + 10.0, at.y - 4.0), (PointerKind::Up, at.x + 10.0, at.y - 4.0)] {
        s.pointer(&PointerEvent::new(k, x, y), v).unwrap();
    }
    let after = glyph_pens(&s, id);
    assert!(near(moved_by(&before, &after, 1), (10.0, -4.0)), "{:?}", moved_by(&before, &after, 1));
    assert!(near(moved_by(&before, &after, 2), (0.0, 0.0)));
    assert_eq!(undo_labels(&s).iter().filter(|l| *l == "Touch Type").count(), 1);
    // Turned 30°, the y still follows the pointer: an upward drag of its box moves it straight up.
    s.execute("text.touchType", &json!({"id": id.0, "start": 1, "rotation": 30})).unwrap();
    let before = glyph_pens(&s, id);
    let t = obj(&s, id);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let y = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
    let face = vectorcraft_text::FontDb::global().face_by_id(y.font_id).unwrap();
    let (ascent, descent) = face.vertical_metrics();
    let at = t.xf * (y.xf * Point::new(face.advance(y.gid) / 2.0, (descent - ascent) / 2.0));
    for (k, x, y) in [(PointerKind::Down, at.x, at.y), (PointerKind::Drag, at.x, at.y - 4.0), (PointerKind::Up, at.x, at.y - 4.0)] {
        s.pointer(&PointerEvent::new(k, x, y), v).unwrap();
    }
    let after = glyph_pens(&s, id);
    assert!(near(moved_by(&before, &after, 1), (0.0, -4.0)), "{:?}", moved_by(&before, &after, 1));
    assert!(near(moved_by(&before, &after, 2), (0.0, 0.0)), "{:?}", moved_by(&before, &after, 2));
}

/// Arrow keys with the Touch Type tool nudge the selected character by the keyboard increment.
#[test]
fn touch_type_arrow_keys_nudge_the_selected_character() {
    let mut s = session();
    let id = NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": "Type", "size": 40})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("touchType", v).unwrap();
    let t = obj(&s, id);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let y = lay.glyphs.iter().find(|g| g.byte == 1).unwrap();
    let at = t.xf * (y.origin + vectorcraft_geom::Vec2::new(y.advance / 2.0, -10.0));
    click(&mut s, at.x, at.y);
    let before = glyph_pens(&s, id);
    s.tool_key(vectorcraft_tools::ToolKey::Right, Mods::default(), v).unwrap();
    let after = glyph_pens(&s, id);
    assert!(near(moved_by(&before, &after, 1), (1.0, 0.0)), "{:?}", moved_by(&before, &after, 1));
    assert_eq!(undo_labels(&s).iter().filter(|l| *l == "Touch Type").count(), 1);
}

#[test]
fn type_tool_typing_is_one_undo_step() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 200.0, 200.0);
    let id = s.doc().unwrap().selection.objects[0];
    s.tool_text("Hello", v).unwrap();
    s.tool_text(" world", v).unwrap();
    assert!(s.tool_wants_text());
    assert_eq!(obj(&s, id).plain_text(), "Hello world");
    key(&mut s, ToolKey::Left, Mods::default());
    assert!(!s.in_interaction(), "moving the caret commits");
    assert_eq!(undo_labels(&s).last().map(String::as_str), Some("Typing"));
    s.tool_text("!", v).unwrap();
    // #1686: Escape is the tool's while editing, and asks the UI for the Selection tool.
    assert!(s.tool_claims_key(ToolKey::Escape, v));
    let r = s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    assert!(r.iter().any(|r| matches!(r, UiRequest::SwitchTool(t) if t == "selection")), "{r:?}");
    assert!(!s.tool_claims_key(ToolKey::Escape, v));
    assert_eq!(obj(&s, id).plain_text(), "Hello worl!d");
    // #1686: Escape ends editing, the type still selected; the UI goes on to the Selection tool.
    assert!(!s.tool_wants_text());
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "Hello world");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "");
}

#[test]
fn undo_while_typing_takes_back_only_the_typing_and_redo_returns_it() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 200.0, 200.0);
    let id = s.doc().unwrap().selection.objects[0];
    s.tool_text("abc", v).unwrap();
    key(&mut s, ToolKey::Left, Mods::default());
    key(&mut s, ToolKey::Right, Mods::default());
    s.tool_text("def", v).unwrap();
    assert!(s.in_interaction(), "still typing");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "abc", "only the typing in progress is undone");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "abcdef", "and Redo brings it back");
    // Typing on after an undo starts a new session from the restored text.
    s.execute("edit.undo", &json!({})).unwrap();
    s.tool_text("!", v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "abc!");
}

fn list_markers(s: &Session, id: NodeId) -> Vec<String> {
    vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &obj(s, id)).markers.iter().map(|m| m.text.clone()).collect()
}

/// Typed list detection: at the start of a paragraph, "1.", "1)", "A." or "A)" and a space start
/// a numbered list, and "*", "#", ">" or "+" and a space start a bulleted list with the filled dot.
/// The typed characters go, the paragraph takes the hanging indent, and the text typed next is the
/// list item's. One Undo puts the typed characters back with no list, and typing goes on after
/// them, also after a Backspace that deleted nothing.
#[test]
fn typing_a_list_start_and_a_space_starts_a_list() {
    use vectorcraft_doc::text::{Bullet, NumberStyle, ParaList};
    let v = ViewInfo::default();
    let (arabic, letters, dot) =
        (ParaList::numbered(NumberStyle::Arabic), ParaList::numbered(NumberStyle::UpperLetters), ParaList::bulleted(Bullet::Dot));
    let cases = [("1.", arabic, "1."), ("1)", arabic, "1."), ("A.", letters, "A."), ("A)", letters, "A.")];
    let bullets = ["*", "#", ">", "+"].map(|b| (b, dot, "\u{2022}"));
    for (typed, list, marker) in cases.into_iter().chain(bullets) {
        let mut s = session();
        s.select_tool("type", v).unwrap();
        click(&mut s, 200.0, 200.0);
        let id = s.doc().unwrap().selection.objects[0];
        for c in typed.chars() {
            s.tool_text(&c.to_string(), v).unwrap();
        }
        s.tool_text(" ", v).unwrap();
        let t = obj(&s, id);
        assert_eq!((t.plain_text().as_str(), t.para_at(0).list), ("", Some(list)), "{typed}");
        assert_eq!((t.para_at(0).left_indent, t.para_at(0).first_line_indent), (18.0, -18.0), "{typed}");
        assert_eq!(undo_labels(&s).last().map(String::as_str), Some("Bullets and Numbering"), "{typed}");
        s.tool_text("Apples", v).unwrap();
        assert_eq!((obj(&s, id).plain_text().as_str(), obj(&s, id).para_at(0).list), ("Apples", Some(list)), "{typed}");
        assert_eq!(list_markers(&s, id), [marker], "{typed}");
        key(&mut s, ToolKey::Enter, Mods::default());
        s.tool_text("Pears", v).unwrap();
        assert_eq!(obj(&s, id).para_at(1).list, Some(list), "Return continues the list: {typed}");
        // One Undo after the space puts the characters back with no list; typing goes on after them.
        let mut s = session();
        s.select_tool("type", v).unwrap();
        click(&mut s, 200.0, 200.0);
        let id = s.doc().unwrap().selection.objects[0];
        for c in typed.chars() {
            s.tool_text(&c.to_string(), v).unwrap();
        }
        s.tool_text(" ", v).unwrap();
        // A Backspace at the start of the empty item deletes nothing and leaves that as it is.
        key(&mut s, ToolKey::Backspace, Mods::default());
        s.execute("edit.undo", &json!({})).unwrap();
        let t = obj(&s, id);
        assert_eq!((t.plain_text(), t.para_at(0).list), (format!("{typed} "), None), "{typed}");
        assert_eq!((t.para_at(0).left_indent, t.para_at(0).first_line_indent), (0.0, 0.0), "{typed}");
        s.tool_text("x", v).unwrap();
        assert_eq!((obj(&s, id).plain_text(), obj(&s, id).para_at(0).list), (format!("{typed} x"), None), "{typed}");
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(obj(&s, id).plain_text(), "", "the typing before the list is a step of its own: {typed}");
    }
}

/// A list typed before a paragraph's text: the caret stays before the text, goes after the typed
/// characters when Undo puts them back, also after undoing what was typed after the list, and
/// before the text again after Redo, also when the text starts with the same characters.
#[test]
fn the_caret_follows_undo_and_redo_of_a_typed_list() {
    let v = ViewInfo::default();
    for start in ["Apples", "* Apples"] {
        let mut s = session();
        let id = text(&mut s, start);
        s.select_tool("type", v).unwrap();
        click(&mut s, 101.0, 95.0);
        s.set_tool_option("select", &json!({"start": 0, "end": 0}));
        for c in "* ".chars() {
            s.tool_text(&c.to_string(), v).unwrap();
        }
        let state = |s: &Session| (obj(s, id).plain_text(), s.tool_options()["caret"].clone());
        assert_eq!(state(&s), (start.to_string(), json!(0)));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(state(&s), (format!("* {start}"), json!(2)));
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(state(&s), (start.to_string(), json!(0)));
        s.tool_text("x", v).unwrap();
        assert_eq!(obj(&s, id).plain_text(), format!("x{start}"));
        // Undo of the typing and then of the list: typing goes on after the characters again.
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        s.tool_text("y", v).unwrap();
        assert_eq!(obj(&s, id).plain_text(), format!("* y{start}"));
    }
}

/// The same characters later in a paragraph, without the space, typed in one piece with the space
/// (a paste), or at the start of a paragraph that is already in a list start nothing.
#[test]
fn list_starts_typed_elsewhere_or_pasted_start_no_list() {
    use vectorcraft_doc::text::{NumberStyle, ParaList};
    let v = ViewInfo::default();
    let mut s = session();
    s.select_tool("type", v).unwrap();
    click(&mut s, 200.0, 200.0);
    let id = s.doc().unwrap().selection.objects[0];
    for c in "Fruit 1. * A) x".chars() {
        s.tool_text(&c.to_string(), v).unwrap();
    }
    key(&mut s, ToolKey::Enter, Mods::default());
    s.tool_text("1. Apples", v).unwrap();
    key(&mut s, ToolKey::Enter, Mods::default());
    for c in "1.Pears".chars() {
        s.tool_text(&c.to_string(), v).unwrap();
    }
    key(&mut s, ToolKey::Enter, Mods::default());
    s.tool_text("* ", v).unwrap();
    let t = obj(&s, id);
    assert_eq!(t.plain_text(), "Fruit 1. * A) x\n1. Apples\n1.Pears\n* ");
    assert_eq!((0..4).map(|i| t.para_at(i).list).collect::<Vec<_>>(), [None, None, None, None]);
    assert!(!undo_labels(&s).iter().any(|l| l == "Bullets and Numbering"));
    // A paragraph in a list keeps its list, and the typed characters stay as text.
    let id = text(&mut s, "Item");
    s.execute("text.setFormat", &json!({"ids": [id.0], "list": "number"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    click(&mut s, 101.0, 95.0);
    s.set_tool_option("select", &json!({"start": 0, "end": 0}));
    for c in "* ".chars() {
        s.tool_text(&c.to_string(), v).unwrap();
    }
    let t = obj(&s, id);
    assert_eq!((t.plain_text().as_str(), t.para_at(0).list), ("* Item", Some(ParaList::numbered(NumberStyle::Arabic))));
}

/// `text.startList`, which the Type tool runs, works from scripts too: by default it reads the
/// text up to the end, and it changes nothing (and records no step) when the text there starts no
/// list.
#[test]
fn start_list_command_removes_the_typed_characters_and_starts_the_list() {
    use vectorcraft_doc::text::{Bullet, NumberStyle, ParaList};
    let mut s = session();
    let id = text(&mut s, "Fruit\n1) Apples");
    let r = s.execute("text.startList", &json!({"id": id.0, "at": 9})).unwrap();
    assert_eq!(r, json!({"id": id.0, "started": true, "caret": 6}));
    let t = obj(&s, id);
    assert_eq!(
        (t.plain_text().as_str(), t.para_at(0).list, t.para_at(1).list),
        ("Fruit\nApples", None, Some(ParaList::numbered(NumberStyle::Arabic)))
    );
    assert_eq!(list_markers(&s, id), ["1."]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((obj(&s, id).plain_text().as_str(), obj(&s, id).para_at(1).list), ("Fruit\n1) Apples", None));
    let id = text(&mut s, "x\n# ");
    let r = s.execute("text.startList", &json!({"id": id.0})).unwrap();
    assert_eq!((r["started"].clone(), obj(&s, id).plain_text()), (json!(true), "x\n".to_string()));
    assert_eq!(obj(&s, id).para_at(1).list, Some(ParaList::bulleted(Bullet::Dot)));
    let steps = undo_labels(&s).len();
    let id = text(&mut s, "1.x");
    assert_eq!(s.execute("text.startList", &json!({"id": id.0, "at": 3})).unwrap()["started"], json!(false));
    assert_eq!(undo_labels(&s).len(), steps + 1, "only text.create's step");
    assert!(s.execute("text.startList", &json!({"id": id.0, "at": "3"})).is_err(), "`at` is a number");
    assert!(s.execute("text.startList", &json!({"at": 3})).is_err(), "id required");
}

/// #1881: a Cmd/Ctrl drag while typing (the Selection tool lent to the Type tool) moves the type
/// and keeps what was typed, and typing goes on after it.
#[test]
fn a_cmd_drag_while_typing_moves_the_type_and_keeps_the_typing() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 200.0, 200.0);
    let id = s.doc().unwrap().selection.objects[0];
    s.tool_text("Hello", v).unwrap();
    let before = s.doc().unwrap().doc.bounds_of(&[id], false).unwrap();
    let (at, cmd) = (before.center(), Mods { cmd: true, ..Default::default() });
    for (kind, dx) in [(PointerKind::Down, 0.0), (PointerKind::Drag, 100.0), (PointerKind::Up, 100.0)] {
        s.pointer(&PointerEvent::new(kind, at.x + dx, at.y).with_mods(cmd), v).unwrap();
    }
    assert_eq!(obj(&s, id).plain_text(), "Hello", "the typing is kept");
    let moved = s.doc().unwrap().doc.bounds_of(&[id], false).unwrap();
    assert!(moved.x0 - before.x0 > 50.0, "the type moved: {before:?} → {moved:?}");
    s.tool_text("!", v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "Hello!");
    let after = s.doc().unwrap().doc.bounds_of(&[id], false).unwrap();
    assert!((after.x0 - moved.x0).abs() < 1e-6, "and stays where it was moved: {after:?}");
}

fn cmd_click(s: &mut Session, x: f64, y: f64) {
    let (v, cmd) = (ViewInfo::default(), Mods { cmd: true, ..Default::default() });
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, x, y).with_mods(cmd), v).unwrap();
    }
}

/// #3779: a Cmd/Ctrl click on another object while typing selects it and ends the text edit, so
/// the keys that follow are shortcuts again and not typing.
#[test]
fn a_cmd_click_on_another_object_while_typing_ends_the_text_edit() {
    let mut s = session();
    let v = ViewInfo::default();
    let r = s.execute("shape.rectangle", &json!({"x": 500, "y": 400, "width": 50, "height": 50})).unwrap();
    let rect = NodeId(r["id"].as_u64().unwrap());
    s.select_tool("type", v).unwrap();
    click(&mut s, 200.0, 200.0);
    let id = s.doc().unwrap().selection.objects[0];
    s.tool_text("Hello", v).unwrap();
    cmd_click(&mut s, 525.0, 425.0);
    assert_eq!(s.doc().unwrap().selection.objects, vec![rect], "the rectangle is selected");
    assert!(!s.tool_wants_text(), "the text edit ended");
    s.tool_text("v", v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "Hello", "a later key is not typed into the text");
    assert_eq!(s.tool_id(), "type", "the Type tool is back");
}

/// #3779: a Cmd/Ctrl click on empty canvas while typing deselects the text and ends its edit.
#[test]
fn a_cmd_click_on_empty_canvas_while_typing_ends_the_text_edit() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 200.0, 200.0);
    let id = s.doc().unwrap().selection.objects[0];
    s.tool_text("Hello", v).unwrap();
    cmd_click(&mut s, 600.0, 450.0);
    assert!(s.doc().unwrap().selection.objects.is_empty(), "nothing is selected");
    assert!(!s.tool_wants_text(), "the text edit ended");
    s.tool_text("v", v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "Hello", "a later key is not typed into the text");
}

/// #3779: point type a click placed and nothing was typed into goes away on a Cmd/Ctrl click
/// elsewhere, as it does on Escape.
#[test]
fn a_cmd_click_away_from_empty_point_type_discards_it() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 200.0, 200.0);
    let id = s.doc().unwrap().selection.objects[0];
    cmd_click(&mut s, 600.0, 450.0);
    assert!(s.doc().unwrap().doc.node(id).is_none(), "the empty type is discarded");
}

#[test]
fn undo_during_a_drag_takes_back_only_the_drag() {
    let mut s = session();
    let v = ViewInfo::default();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 50, "height": 50})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.select_tool("selection", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 125.0, 125.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 225.0, 125.0), v).unwrap();
    assert!(s.in_interaction(), "dragging");
    s.execute("edit.undo", &json!({})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let b = doc.bounds_of(&[id], false).expect("the rectangle is still there");
    assert_eq!(b.x0, 100.0, "back where it was: {b:?}");
}

#[test]
fn ime_composition_shows_in_place_and_commits_as_one_step() {
    // Romaji → kana → conversion → commit, as the macOS Japanese IME sends it.
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 200.0, 200.0);
    let id = s.doc().unwrap().selection.objects[0];
    let created = undo_labels(&s).len();
    s.tool_text("曲:", v).unwrap();
    for (t, r) in [("g", 1..1), ("が", 1..1), ("がg", 2..2), ("ががく", 3..3)] {
        s.tool_preedit(t, Some(r), v).unwrap();
        assert_eq!(obj(&s, id).plain_text(), format!("曲:{t}"), "marked text lays out in place");
        assert!(s.tool_composing());
    }
    // Conversion: the active clause is the whole word.
    s.tool_preedit("雅楽", Some(0..2), v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "曲:雅楽");
    assert_eq!(s.tool_options()["composing"], json!(true));
    // macOS clears the marked text, then commits.
    s.tool_preedit("", None, v).unwrap();
    s.tool_text("雅楽", v).unwrap();
    assert!(!s.tool_composing());
    assert_eq!(obj(&s, id).plain_text(), "曲:雅楽");
    assert_eq!(s.tool_options()["caret"], json!("曲:雅楽".len()));
    // Typing goes on after the commit, in the same session.
    s.tool_text("。", v).unwrap();
    key(&mut s, ToolKey::Escape, Mods::default());
    assert_eq!(obj(&s, id).plain_text(), "曲:雅楽。");
    assert_eq!(undo_labels(&s).len(), created + 1, "one undo step for the session");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "");
}

#[test]
fn ime_cancel_leaves_no_undo_step_and_keeps_the_text() {
    let mut s = session();
    let v = ViewInfo::default();
    let id = text(&mut s, "雅楽");
    s.select_tool("type", v).unwrap();
    let t = obj(&s, id);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let (a, b) = vectorcraft_text::caret_position(&lay, "雅楽".len());
    let p = t.xf * a.midpoint(b);
    click(&mut s, p.x - 0.1, p.y);
    assert!(s.tool_wants_text());
    s.set_tool_option("select", &json!({"start": 6, "end": 6}));
    let before = undo_labels(&s).len();
    s.tool_preedit("えんそう", Some(4..4), v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "雅楽えんそう");
    // Escape in the IME: the marked text goes, nothing else changes.
    s.tool_preedit("", None, v).unwrap();
    assert!(!s.tool_composing());
    assert!(!s.in_interaction(), "a cancelled composition leaves no typing session");
    assert_eq!(obj(&s, id).plain_text(), "雅楽");
    assert_eq!(undo_labels(&s).len(), before);
    // A stray clear (no composition) never deletes the selection.
    s.set_tool_option("select", &json!({"start": 0, "end": 3}));
    s.tool_preedit("", None, v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "雅楽");
    // A composition over a selection replaces it.
    s.tool_preedit("が", Some(1..1), v).unwrap();
    s.tool_preedit("", None, v).unwrap();
    s.tool_text("我", v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "我楽");
}

#[test]
fn clicking_away_keeps_marked_text_as_typed_and_undo_mid_composition_is_safe() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 200.0, 200.0);
    let id = s.doc().unwrap().selection.objects[0];
    s.tool_preedit("しょうこ", Some(4..4), v).unwrap();
    // Keys belong to the IME while it composes.
    key(&mut s, ToolKey::Backspace, Mods::default());
    assert_eq!(obj(&s, id).plain_text(), "しょうこ");
    click(&mut s, 600.0, 500.0);
    assert!(!s.tool_composing());
    assert_eq!(obj(&s, id).plain_text(), "しょうこ", "the marked text stays, committed");
    // The UI holds Undo back while composing; if it comes anyway, the session is committed first.
    click(&mut s, 50.0, 50.0);
    let id2 = s.doc().unwrap().selection.objects[0];
    s.tool_preedit("ひちりき", Some(4..4), v).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!s.tool_composing());
    assert_eq!(obj(&s, id2).plain_text(), "");
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(obj(&s, id2).plain_text(), "ひちりき");
}

#[test]
fn type_tool_selection_replace_and_delete() {
    let mut s = session();
    let v = ViewInfo::default();
    let id = text(&mut s, "Hello brave world");
    s.select_tool("type", v).unwrap();
    // Click at the start of the text to edit it.
    let t = obj(&s, id);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let (a, b) = vectorcraft_text::caret_position(&lay, 6);
    let p = t.xf * a.midpoint(b);
    click(&mut s, p.x + 0.1, p.y);
    assert!(s.tool_wants_text());
    assert_eq!(s.tool_options()["caret"], json!(6));
    let shift_word = Mods { shift: true, alt: true, ..Default::default() };
    key(&mut s, ToolKey::Right, shift_word);
    assert_eq!((s.tool_options()["start"].as_u64(), s.tool_options()["end"].as_u64()), (Some(6), Some(11)));
    s.tool_text("bold", v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "Hello bold world");
    key(&mut s, ToolKey::Home, Mods { shift: true, ..Default::default() });
    key(&mut s, ToolKey::Delete, Mods::default());
    assert_eq!(obj(&s, id).plain_text(), " world");
    key(&mut s, ToolKey::End, Mods::default());
    key(&mut s, ToolKey::Backspace, Mods { alt: true, ..Default::default() });
    assert_eq!(obj(&s, id).plain_text(), " ");
}

#[test]
fn character_panel_flow_styles_the_selected_range_while_editing() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 50.0, 300.0);
    let id = s.doc().unwrap().selection.objects[0];
    s.tool_text("Make this big", v).unwrap();
    // Select "this" (still inside the typing session).
    s.set_tool_option("select", &json!({"start": 5, "end": 9}));
    // What the panel does: end the typing session, then style the range.
    s.set_tool_option("commitTyping", &json!(true));
    s.commit_interaction().unwrap();
    let o = s.tool_options();
    s.execute("text.setRangeStyle", &json!({"id": o["editing"], "start": o["start"], "end": o["end"], "size": 48})).unwrap();
    let t = obj(&s, id);
    assert_eq!(t.runs.len(), 3);
    assert_eq!(t.runs[1].text, "this");
    assert_eq!(t.runs[1].style.size, 48.0);
    // Typing continues in a new session, over the styled selection (keeps its style).
    s.tool_text("THAT", v).unwrap();
    key(&mut s, ToolKey::Escape, Mods::default());
    let t = obj(&s, id);
    assert_eq!(t.plain_text(), "Make THAT big");
    assert_eq!(t.runs[1].text, "THAT");
    assert_eq!(t.runs[1].style.size, 48.0);
    let labels = undo_labels(&s);
    assert_eq!(&labels[labels.len() - 3..], ["Typing", "Character", "Typing"]);
}

#[test]
fn select_all_and_cut_like_ui() {
    let mut s = session();
    let v = ViewInfo::default();
    s.execute("shape.rectangle", &json!({"x": 300, "y": 300, "width": 10, "height": 10})).unwrap();
    let id = text(&mut s, "abc def");
    s.select_tool("type", v).unwrap();
    let t = obj(&s, id);
    let p = t.xf * Point::new(2.0, -5.0);
    click(&mut s, p.x, p.y);
    // Select All while editing takes the text, not the art (the rectangle stays unselected).
    assert!(s.tool_wants_text());
    let o = s.execute("select.all", &json!({})).unwrap();
    assert_eq!(o, json!({"editing": id.0, "start": 0, "end": 7}));
    assert_eq!(s.doc().unwrap().selection.objects, [id]);
    let r = s.execute("text.getRange", &json!({"id": o["editing"], "start": o["start"], "end": o["end"]})).unwrap();
    assert_eq!(r["text"], json!("abc def"));
    s.set_tool_option("copy", &r["runs"]);
    key(&mut s, ToolKey::Delete, Mods::default());
    assert_eq!(obj(&s, id).plain_text(), "");
    s.tool_text("abc def", v).unwrap();
    s.tool_text("abc def", v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "abc defabc def");
    // Not editing: Select All selects the art again.
    key(&mut s, ToolKey::Escape, Mods::default());
    assert!(!s.tool_wants_text());
    assert_eq!(s.execute("select.all", &json!({})).unwrap(), json!({"count": 2}));
}

#[test]
fn a_frame_dragged_with_the_type_tools_is_the_dragged_rectangle() {
    for tool in ["type", "verticalType"] {
        let mut s = session();
        let v = ViewInfo::default();
        s.select_tool(tool, v).unwrap();
        for (k, x, y) in [(PointerKind::Down, 100.0, 100.0), (PointerKind::Drag, 300.0, 180.0), (PointerKind::Up, 300.0, 180.0)] {
            s.pointer(&PointerEvent::new(k, x, y), v).unwrap();
        }
        let t = obj(&s, s.doc().unwrap().selection.objects[0]);
        let TextKind::Area { frame } = &t.kind else { panic!("{tool}: not area type") };
        let b = frame.bounds().unwrap();
        let (a, z) = (t.xf * Point::new(b.x0, b.y0), t.xf * Point::new(b.x1, b.y1));
        assert!((a - Point::new(100.0, 100.0)).hypot() < 1e-9 && (z - Point::new(300.0, 180.0)).hypot() < 1e-9, "{tool}: {a:?} {z:?}");
    }
}

/// Through the session, a click with Shift held and the Type tool chosen makes vertical type and
/// edits it with the Type tool still chosen, and the pointer with Shift held is the vertical
/// I-beam. A click with Alt held and the Type on a Path tool chosen turns a rectangle into area
/// type.
#[test]
fn shift_and_alt_switch_the_type_tools_through_the_session() {
    let mut s = session();
    let v = ViewInfo::default();
    let (shift, alt) = (Mods { shift: true, ..Mods::default() }, Mods { alt: true, ..Mods::default() });
    s.select_tool("type", v).unwrap();
    assert_eq!(s.cursor(Point::new(400.0, 300.0), shift, v), vectorcraft_tools::Cursor::TypeVertical);
    assert_eq!(s.cursor(Point::new(400.0, 300.0), Mods::default(), v), vectorcraft_tools::Cursor::Text);
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, 400.0, 300.0).with_mods(shift), v).unwrap();
    }
    assert_eq!(s.tool_id(), "type");
    let id = s.doc().unwrap().selection.objects[0];
    assert!(obj(&s, id).vertical);
    assert!(s.tool_wants_text());
    s.tool_text("縦", v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "縦");
    key(&mut s, ToolKey::Escape, Mods::default());

    let rect = NodeId(s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap()["id"].as_u64().unwrap());
    s.select_tool("typeOnPath", v).unwrap();
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, 100.0, 150.0).with_mods(alt), v).unwrap();
    }
    assert_eq!(s.tool_id(), "typeOnPath");
    assert!(s.doc().unwrap().doc.node(rect).is_none(), "the rectangle became the frame");
    let id = s.doc().unwrap().selection.objects[0];
    assert!(matches!(obj(&s, id).kind, TextKind::Area { .. }));
    assert!(!obj(&s, id).vertical);
}

#[test]
fn area_type_from_a_circle_keeps_glyphs_inside() {
    let mut s = session();
    let e = s.execute("shape.ellipse", &json!({"x": 100, "y": 100, "width": 240, "height": 240})).unwrap();
    let pid = e["id"].as_u64().unwrap();
    let r = s.execute("text.createInPath", &json!({"path": pid, "mode": "area", "text": vectorcraft_engine_placeholder()})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    assert!(s.doc().unwrap().doc.node(NodeId(pid)).is_none(), "the path became the frame");
    let t = obj(&s, id);
    assert!(matches!(t.kind, TextKind::Area { .. }));
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert!(lay.lines.len() > 5);
    let c = Point::new(220.0, 220.0);
    for g in lay.glyphs.iter().filter(|g| !g.outline.elements().is_empty()) {
        let l = &lay.lines[g.line];
        for q in [Point::new(g.origin.x, l.baseline - l.ascent), Point::new(g.origin.x + g.advance, l.baseline + l.descent)] {
            assert!((t.xf * q - c).hypot() <= 120.5, "{q:?}");
        }
    }
    assert_eq!(s.doc().unwrap().selection.objects, vec![id]);
    assert!(s.execute("text.createInPath", &json!({"path": 9999})).is_err());
}

fn vectorcraft_engine_placeholder() -> String {
    "Type flows inside any closed shape, line by line, trimmed to the shape's edges at each band. ".repeat(4)
}

#[test]
fn type_on_path_starts_where_clicked() {
    let mut s = session();
    let r = s
        .execute("shape.line", &json!({"x1": 100, "y1": 300, "x2": 500, "y2": 300}))
        .or_else(|_| s.execute("path.create", &json!({"points": [[100, 300], [500, 300]], "closed": false})));
    let pid = r.unwrap()["id"].as_u64().unwrap();
    let r = s.execute("text.createInPath", &json!({"path": pid, "mode": "onPath", "text": "On the line", "at": [200, 310]})).unwrap();
    let t = obj(&s, NodeId(r["id"].as_u64().unwrap()));
    let TextKind::OnPath { start, .. } = t.kind else { panic!() };
    assert!((start - 0.25).abs() < 0.01, "{start}");
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let g0 = &lay.glyphs[0];
    assert!((g0.origin.y - 300.0).abs() < 1e-6 && (g0.origin.x - 200.0).abs() < 1.0, "{:?}", g0.origin);
}

#[test]
fn type_on_path_tool_through_session() {
    let mut s = session();
    let v = ViewInfo::default();
    let e = s.execute("shape.ellipse", &json!({"x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    s.select_tool("typeOnPath", v).unwrap();
    assert_eq!(s.tool_id(), "typeOnPath");
    click(&mut s, 300.0, 200.0);
    let id = s.doc().unwrap().selection.objects[0];
    assert_ne!(id.0, e["id"].as_u64().unwrap());
    s.tool_text("Around", v).unwrap();
    key(&mut s, ToolKey::Escape, Mods::default());
    let t = obj(&s, id);
    assert!(matches!(t.kind, TextKind::OnPath { .. }));
    assert_eq!(t.plain_text(), "Around");
}

#[test]
fn fit_headline_fills_the_frame_width() {
    let mut s = session();
    let r = s
        .execute(
            "text.create",
            &json!({"x": 50, "y": 50, "text": "Headline\nBody copy follows here", "size": 24, "area": {"width": 400, "height": 200}}),
        )
        .unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let r = s.execute("type.fitHeadline", &json!({"ids": [id.0]})).unwrap();
    assert!(r["tracking"][0].as_f64().unwrap() > 100.0, "{r}");
    let t = obj(&s, id);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    let l0 = &lay.lines[0];
    assert_eq!(&t.plain_text()[l0.start..l0.end], "Headline", "still one line");
    assert!((l0.x1 - l0.x0 - 400.0).abs() < 1.0, "{}", l0.x1 - l0.x0);
    // The body keeps its tracking.
    assert_eq!(t.runs.last().unwrap().style.tracking, 0.0);
    let p = s.execute("text.create", &json!({"x": 1, "y": 1, "text": "point"})).unwrap();
    assert!(s.execute("type.fitHeadline", &json!({"ids": [p["id"]]})).is_err(), "area type only");
}

fn first_glyph_doc_pos(t: &TextObject) -> Point {
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t);
    t.xf * lay.glyphs[0].origin
}

#[test]
fn convert_area_point_keeps_text_in_place_for_every_alignment() {
    for j in ["left", "center", "right"] {
        let mut s = session();
        let r = s
            .execute(
                "text.create",
                &json!({"x": 100, "y": 100, "text": "Centered words wrap in a frame", "size": 18, "area": {"width": 150, "height": 200}}),
            )
            .unwrap();
        let id = NodeId(r["id"].as_u64().unwrap());
        s.execute("text.setStyle", &json!({"ids": [id.0], "justify": j})).unwrap();
        let before = first_glyph_doc_pos(&obj(&s, id));
        s.execute("type.convertToPointType", &json!({"ids": [id.0]})).unwrap();
        let t = obj(&s, id);
        assert!(matches!(t.kind, TextKind::Point));
        let after = first_glyph_doc_pos(&t);
        assert!((after - before).hypot() < 0.01, "{j}: {before:?} → {after:?}");
        s.execute("type.convertToAreaType", &json!({"ids": [id.0]})).unwrap();
        let t = obj(&s, id);
        let back = first_glyph_doc_pos(&t);
        assert!((back - before).hypot() < 0.01, "{j} (area again): {before:?} → {back:?}");
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
        assert!(!lay.overflow);
    }
}

#[test]
fn commands_are_listed_with_params() {
    let s = session();
    let cmds: Vec<Value> = s.commands().iter().map(|c| serde_json::to_value(c).unwrap()).collect();
    for id in ["text.editRange", "text.setRangeStyle", "text.getRange", "text.createInPath", "type.fitHeadline"] {
        assert!(cmds.iter().any(|c| c["id"] == id), "{id}");
    }
}

#[test]
fn type_tool_state_does_not_leak_across_documents() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 200.0, 200.0);
    let id = s.doc().unwrap().selection.objects[0];
    s.tool_text("First", v).unwrap();
    // A second document whose first object reuses the same id.
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let id2 = text(&mut s, "Other");
    assert!(!s.tool_wants_text(), "switching documents ends the edit");
    s.tool_text("XYZ", v).unwrap();
    assert_eq!(obj(&s, id2).plain_text(), "Other");
    s.set_active(0);
    assert_eq!(obj(&s, id).plain_text(), "First");
    assert!(!s.in_interaction());
}

#[test]
fn vertical_text_creation_orientation_and_persistence() {
    let mut s = session();
    let r = s.execute("text.create", &json!({"x":100,"y":100,"text":"日本語", "vertical":true})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let t = obj(&s, id);
    assert!(t.vertical);
    let round: TextObject = serde_json::from_value(serde_json::to_value(&t).unwrap()).unwrap();
    assert!(round.vertical);
    s.execute("type.orientation.horizontal", &json!({"id":id.0})).unwrap();
    assert!(!obj(&s, id).vertical);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(obj(&s, id).vertical);
    let old = serde_json::to_value(TextObject::point(Point::ZERO, "old", Default::default())).unwrap();
    assert!(old.get("vertical").is_none());
    let old: TextObject = serde_json::from_value(old).unwrap();
    assert!(!old.vertical);
}

#[test]
fn rtl_edit_commands_and_undo_preserve_logical_source_and_style_ranges() {
    let mut s = session();
    let id = text(&mut s, "שלום Rust 123");
    s.execute("text.editRange", &json!({"id": id.0, "start": 0, "end": 8, "insert": "مرحبا"})).unwrap();
    let t = obj(&s, id);
    assert_eq!(t.plain_text(), "مرحبا Rust 123");
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert!(lay.glyphs.iter().any(|g| g.rtl));
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 0, "end": 10, "size": 30})).unwrap();
    assert_eq!(obj(&s, id).runs[0].text, "مرحبا");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "שלום Rust 123");
}

#[test]
fn automatic_alignment_updates_during_edits_and_can_be_overridden() {
    let mut s = session();
    let id = text(&mut s, "English");
    let db = vectorcraft_text::FontDb::global();
    assert_eq!(obj(&s, id).para.justify, vectorcraft_doc::Justify::Auto);
    for content in ["שלום", "مرحبا", "English"] {
        let len = obj(&s, id).plain_text().len();
        s.execute("text.editRange", &json!({"id": id.0, "start": 0, "end": len, "insert": content})).unwrap();
        let t = obj(&s, id);
        let l = vectorcraft_text::layout(db, &t);
        if content == "English" {
            assert_eq!(l.lines[0].x0, 0.0);
        } else {
            assert!(l.lines[0].x1.abs() < 1e-6);
        }
    }
    s.execute("text.setStyle", &json!({"id": id.0, "justify": "center"})).unwrap();
    s.execute("text.editRange", &json!({"id": id.0, "start": 0, "end": 7, "insert": "שלום"})).unwrap();
    assert_eq!(obj(&s, id).para.justify, vectorcraft_doc::Justify::Center);
    s.execute("text.setStyle", &json!({"id": id.0, "justify": "auto"})).unwrap();
    let t = obj(&s, id);
    assert!(vectorcraft_text::layout(db, &t).lines[0].x1.abs() < 1e-6);
    let restored: TextObject = serde_json::from_value(serde_json::to_value(&t).unwrap()).unwrap();
    assert_eq!(restored.para.justify, vectorcraft_doc::Justify::Auto);
    // Legacy documents with explicit alignment retain that choice.
    let mut legacy = serde_json::to_value(t).unwrap();
    legacy["para"].as_object_mut().unwrap().remove("justify_auto");
    legacy["para"]["justify"] = json!("Left");
    assert_eq!(serde_json::from_value::<TextObject>(legacy).unwrap().para.justify, vectorcraft_doc::Justify::Left);
}

/// #1063: a fill (or stroke) chosen while the Type tool has characters selected colours just
/// those characters, as one undo step; the rest of the type keeps its colour.
#[test]
fn a_fill_chosen_with_characters_selected_colours_only_them() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 50.0, 300.0);
    let id = s.doc().unwrap().selection.objects[0];
    s.tool_text("Sample Text", v).unwrap();
    let before = obj(&s, id).runs[0].style.fill.clone();
    s.set_tool_option("select", &json!({"start": 0, "end": 6}));
    s.execute("paint.setFill", &json!({"color": "#00a000"})).unwrap();
    let t = obj(&s, id);
    assert_eq!(t.runs.len(), 2);
    assert_eq!(t.runs[0].text, "Sample");
    assert_eq!(t.runs[0].style.fill, Paint::solid(vectorcraft_color::Color::rgb8(0, 160, 0)));
    assert_eq!(t.runs[1].text, " Text");
    assert_eq!(t.runs[1].style.fill, before);
    // A stroke goes on the selected characters only, with a weight to show.
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    let t = obj(&s, id);
    assert!(t.runs[0].style.stroke_width > 0.0 && !t.runs[0].style.stroke.is_none());
    assert!(t.runs[1].style.stroke.is_none());
    // One undo step each.
    let labels = undo_labels(&s);
    assert_eq!(&labels[labels.len() - 2..], ["Fill Color", "Stroke Color"]);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(obj(&s, id).runs.iter().all(|r| r.style.fill == before));
}

/// A fill or stroke that the characters selected with the Type tool have already records no undo
/// step: the step to redo stays and the document stays unmodified.
#[test]
fn a_fill_the_selected_characters_have_records_no_undo_step() {
    let mut s = session();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 50.0, 300.0);
    s.tool_text("Sample Text", v).unwrap();
    s.set_tool_option("select", &json!({"start": 0, "end": 6}));
    s.execute("paint.setFill", &json!({"color": "#00a000"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    // A blue fill, undone, leaves a step to redo.
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.doc_mut().unwrap().mark_saved();
    let history = |s: &Session| {
        let st = s.doc().unwrap();
        (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
    };
    let before = history(&s);
    assert_eq!((before.1, before.2), (1, false), "a step to redo, and nothing unsaved");
    s.execute("paint.setFill", &json!({"color": "#00a000"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    assert_eq!(history(&s), before);
}

fn left(s: &Session, id: NodeId) -> f64 {
    s.doc().unwrap().doc.bounds_of(&[id], false).unwrap().x0
}

/// #1881: the Transform panel moving or scaling the type while it is being typed keeps both the
/// typing and the transformation, however the editing goes on.
#[test]
fn a_transform_panel_edit_while_typing_keeps_the_typing_and_the_transform() {
    let mut s = session();
    let v = ViewInfo::default();
    let id = text(&mut s, "Hello");
    s.select_tool("type", v).unwrap();
    let b = s.doc().unwrap().doc.bounds_of(&[id], false).unwrap();
    click(&mut s, b.x1 - 1.0, b.center().y);
    s.tool_text(" world", v).unwrap();
    s.execute("object.setBounds", &json!({"x": 400, "reference": 0})).unwrap();
    assert!((left(&s, id) - 400.0).abs() < 1e-6);
    // Typing on.
    s.tool_text("!", v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "Hello world!");
    assert!((left(&s, id) - 400.0).abs() < 1e-6, "the move stays: {}", left(&s, id));
    s.execute("object.setBounds", &json!({"width": 300, "reference": 0, "proportional": true})).unwrap();
    let w = |s: &Session| s.doc().unwrap().doc.bounds_of(&[id], false).unwrap().width();
    assert!((w(&s) - 300.0).abs() < 1e-6);
    key(&mut s, ToolKey::Escape, Mods::default());
    assert_eq!(obj(&s, id).plain_text(), "Hello world!");
    assert!((w(&s) - 300.0).abs() < 1e-6, "the scale stays: {}", w(&s));
    assert!((left(&s, id) - 400.0).abs() < 1e-6);
    // Each its own step, the typing first.
    let labels = undo_labels(&s);
    let n = labels.len();
    assert_eq!(labels[n - 4], "Typing", "{labels:?}");
    assert_eq!(labels[n - 2], "Typing", "{labels:?}");
}

/// #1881: a dialog previewing a transformation while the type is being typed (Object › Transform
/// with Preview) starts from the typed text, and keeps it when it commits.
#[test]
fn a_previewed_transform_while_typing_keeps_the_typing() {
    let mut s = session();
    let v = ViewInfo::default();
    let id = text(&mut s, "Hello");
    s.select_tool("type", v).unwrap();
    let b = s.doc().unwrap().doc.bounds_of(&[id], false).unwrap();
    click(&mut s, b.x1 - 1.0, b.center().y);
    s.tool_text(" world", v).unwrap();
    s.begin_interaction("Move").unwrap();
    s.preview("object.setBounds", &json!({"x": 400, "reference": 0})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "Hello world", "the preview starts from the typed text");
    s.commit_interaction().unwrap();
    assert_eq!(obj(&s, id).plain_text(), "Hello world");
    assert!((left(&s, id) - 400.0).abs() < 1e-6);
    // Typing on starts a new session from the moved text.
    s.tool_text("!", v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "Hello world!");
    assert!((left(&s, id) - 400.0).abs() < 1e-6);
}

/// #1950: a colour previewed while typing (the Color Picker, a Color panel slider) colours the
/// typed text: its interaction used to join the typing session and preview from the document
/// before the typing, so the new type came back as its placeholder text.
#[test]
fn a_colour_previewed_while_typing_keeps_the_typing() {
    let mut s = session();
    s.prefs.placeholder_text = true;
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    click(&mut s, 50.0, 300.0);
    let id = s.doc().unwrap().selection.objects[0];
    s.tool_text("abc", v).unwrap();
    s.begin_interaction("Fill Color").unwrap();
    s.preview("paint.setFill", &json!({"color": "#00a000"})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "abc", "the preview starts from the typed text");
    s.commit_interaction().unwrap();
    let t = obj(&s, id);
    assert_eq!(t.plain_text(), "abc");
    assert!(t.runs.iter().all(|r| r.style.fill == Paint::solid(vectorcraft_color::Color::rgb8(0, 160, 0))), "{:?}", t.runs);
    // The typing and the colour are a step each, and typing goes on after them.
    let labels = undo_labels(&s);
    assert_eq!(&labels[labels.len() - 2..], ["Typing", "Fill Color"]);
    s.tool_text("d", v).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "abcd");
}

/// The red out port of selected overflowing area type is drawn with the Selection tool too, and
/// not twice with the Type tool, which draws it itself (#2323).
#[test]
fn overflow_marker_shows_with_the_selection_tool() {
    let mut s = session();
    let v = ViewInfo::default();
    let long = "Words that fill more than the frame holds. ".repeat(20);
    let r = s.execute("text.create", &json!({"x": 50, "y": 50, "text": long, "size": 20, "area": {"width": 100, "height": 40}})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.select_tool("selection", v).unwrap();
    let red = |s: &mut Session| s.overlays(v).iter().filter(|o| matches!(o, vectorcraft_tools::Overlay::Path { color: [230, 30, 30], .. })).count();
    assert_eq!(red(&mut s), 1, "the out port");
    s.select_tool("type", v).unwrap();
    assert_eq!(red(&mut s), 1, "the Type tool's own, not a second one");
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.select_tool("selection", v).unwrap();
    assert_eq!(red(&mut s), 0, "nothing selected");
}

/// #2572: with the Selection tool, a click on the out port and a click on empty canvas thread the
/// overflow into a new frame there, in one undo step; Escape, or another tool, drops the loaded text.
#[test]
fn clicking_the_out_port_then_the_canvas_threads_the_text_into_a_new_frame() {
    let mut s = session();
    let v = ViewInfo::default();
    let long = "Words that fill more than the frame holds. ".repeat(20);
    let r = s.execute("text.create", &json!({"x": 50, "y": 50, "text": long, "size": 20, "area": {"width": 100, "height": 40}})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.select_tool("selection", v).unwrap();
    let port = Point::new(150.0, 90.0 - vectorcraft_tools::text::PORT_RISE_PX);
    let loaded = |s: &mut Session| s.cursor(Point::new(400.0, 400.0), Mods::default(), v) == vectorcraft_tools::Cursor::LoadedText;
    click(&mut s, port.x, port.y);
    assert!(loaded(&mut s));
    key(&mut s, ToolKey::Escape, Mods::default());
    assert!(!loaded(&mut s), "Escape drops the text");
    click(&mut s, port.x, port.y);
    s.select_tool("directSelection", v).unwrap();
    s.select_tool("selection", v).unwrap();
    assert!(!loaded(&mut s), "another tool drops the text");
    let steps = undo_labels(&s).len();
    click(&mut s, port.x, port.y);
    click(&mut s, 400.0, 400.0);
    let doc = &s.doc().unwrap().doc;
    let [thread] = &doc.text_threads[..] else { panic!("one thread: {:?}", doc.text_threads) };
    assert_eq!(thread[0], id);
    assert_eq!(format!("{}{}", obj(&s, id).plain_text(), obj(&s, thread[1]).plain_text()), long);
    assert_eq!(&undo_labels(&s)[steps..], ["Thread Text"]);
}

/// #2572: with the Selection tool, area type whose text fits shows both ports, and a click on its
/// in port then on empty canvas makes a new frame the story starts in, flowing on into the frame
/// clicked, in one undo step; the ports then show the thread's arrows.
#[test]
fn clicking_the_in_port_then_the_canvas_threads_a_new_frame_before_it() {
    let mut s = session();
    let v = ViewInfo::default();
    let words = "Words that fit. ";
    let r = s.execute("text.create", &json!({"x": 50, "y": 50, "text": words, "size": 12, "area": {"width": 100, "height": 40}})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.select_tool("selection", v).unwrap();
    let boxes = |s: &mut Session| {
        s.overlays(v).iter().filter(|o| matches!(o, vectorcraft_tools::Overlay::Highlight { color: [255, 255, 255, 255], .. })).count()
    };
    assert_eq!(boxes(&mut s), 2, "an in port and an out port");
    let steps = undo_labels(&s).len();
    click(&mut s, 50.0, 50.0 + vectorcraft_tools::text::PORT_RISE_PX);
    assert_eq!(s.cursor(Point::new(400.0, 400.0), Mods::default(), v), vectorcraft_tools::Cursor::LoadedText);
    click(&mut s, 400.0, 400.0);
    let doc = &s.doc().unwrap().doc;
    let [thread] = &doc.text_threads[..] else { panic!("one thread: {:?}", doc.text_threads) };
    assert_eq!(thread[1], id, "the frame clicked comes second");
    assert_eq!(format!("{}{}", obj(&s, thread[0]).plain_text(), obj(&s, id).plain_text()), words);
    assert!(!obj(&s, thread[0]).plain_text().is_empty(), "the story starts in the new frame");
    assert_eq!(&undo_labels(&s)[steps..], ["Thread Text"]);
    let strokes = s.overlays(v).iter().filter(|o| matches!(o, vectorcraft_tools::Overlay::Line { .. })).count();
    assert_eq!(strokes, 6, "both frames selected: the first one's out port and the second one's in port show arrows");
}

/// Given a family without a style, text.create, text.createInPath and text.setStyle keep the style
/// when the family has it, else take the family's closest one, as text.setRangeStyle does: a family
/// whose styles are numbered has no Regular to store (#4128). A style given is kept as given.
#[test]
fn a_family_without_its_style_takes_its_closest_style() {
    use vectorcraft_text::test_fonts::{NUMBERED_FAMILY as FAMILY, numbered_font};
    let db = vectorcraft_text::FontDb::global();
    db.add_font(numbered_font("W3").unwrap());
    db.add_font(numbered_font("W6").unwrap());
    let numbered = ["W3", "W6"];
    let style = |s: &Session, id: NodeId| obj(s, id).runs[0].style.font_style.clone();
    let create = |s: &mut Session, p: Value| NodeId(s.execute("text.create", &p).unwrap()["id"].as_u64().unwrap());
    let mut s = session();
    let created = create(&mut s, json!({"x": 10, "y": 50, "text": "a", "font": FAMILY}));
    assert!(numbered.contains(&style(&s, created).as_str()), "text.create: {}", style(&s, created));
    let pid = s.execute("path.create", &json!({"d": "M100 400 L400 100"})).unwrap()["id"].as_u64().unwrap();
    let on_path = s.execute("text.createInPath", &json!({"path": pid, "mode": "onPath", "text": "a", "font": FAMILY})).unwrap();
    let on_path = NodeId(on_path["id"].as_u64().unwrap());
    assert!(numbered.contains(&style(&s, on_path).as_str()), "text.createInPath: {}", style(&s, on_path));
    let given = create(&mut s, json!({"x": 10, "y": 90, "text": "a", "font": FAMILY, "style": "Medium"}));
    assert_eq!(style(&s, given), "Medium", "a style given is kept");
    let set = text(&mut s, "a");
    s.execute("text.setStyle", &json!({"ids": [set.0], "font": FAMILY})).unwrap();
    assert!(numbered.contains(&style(&s, set).as_str()), "text.setStyle: {}", style(&s, set));
    s.execute("text.setStyle", &json!({"ids": [set.0], "style": "W6"})).unwrap();
    s.execute("text.setStyle", &json!({"ids": [set.0], "font": FAMILY})).unwrap();
    assert_eq!(style(&s, set), "W6", "a style the family has is kept");
}
