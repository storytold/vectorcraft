//! Kerning at the Type tool's caret at the start of a line (#5111): with no character on the line
//! before the caret, `type.step`, `type.resetTracking` and the Kerning field set the space before
//! the character after it (`kerningBefore`), as one undo step, and the character moves.

use serde_json::json;
use vectorcraft_doc::{CharStyle, NodeId, NodeKind, TextObject};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn text(s: &mut Session, t: &str) -> NodeId {
    NodeId(s.execute("text.create", &json!({"x": 100, "y": 100, "text": t, "size": 20})).unwrap()["id"].as_u64().unwrap())
}

fn obj(s: &Session, id: NodeId) -> TextObject {
    let Some(NodeKind::Text(t)) = s.doc().unwrap().doc.node(id).map(|n| &n.kind) else { panic!("not text") };
    (**t).clone()
}

fn style_at(s: &Session, id: NodeId, byte: usize) -> CharStyle {
    vectorcraft_text::edit::style_at(&obj(s, id).runs, byte + 1)
}

/// Where glyph `byte` is drawn along its line, in text space.
fn drawn_x(s: &Session, id: NodeId, byte: usize) -> f64 {
    let l = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &obj(s, id));
    l.glyphs.iter().find(|g| g.byte == byte).unwrap().xf.as_coeffs()[4]
}

/// The Type tool editing `id` (made by [`text`]) with its caret at byte `caret`.
fn caret_in(s: &mut Session, id: NodeId, caret: usize) -> ViewInfo {
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, 102.0, 95.0), v).unwrap();
    }
    assert_eq!(s.tool_options()["editing"], json!(id.0), "editing the text");
    s.set_tool_option("select", &json!({"start": caret, "end": caret}));
    v
}

fn step_at(s: &mut Session, id: NodeId, caret: usize, by: i32) -> serde_json::Value {
    s.execute("type.step", &json!({"id": id.0, "start": caret, "end": caret, "attribute": "kerning", "by": by})).unwrap()
}

#[test]
fn a_step_at_the_start_of_the_text_moves_the_first_character() {
    let mut s = session();
    let id = text(&mut s, "AV");
    let inc = s.prefs.tracking_increment;
    let (a, v) = (drawn_x(&s, id, 0), drawn_x(&s, id, 1));
    assert_eq!(step_at(&mut s, id, 0, 1)["ids"], json!([id.0]));
    let st = style_at(&s, id, 0);
    assert_eq!((st.kerning_before, st.kerning), (inc, None), "the space before A, not after it");
    // A step of `inc` thousandths of the 20 pt em moves both letters along the line.
    let moved = inc / 1000.0 * 20.0;
    assert!((drawn_x(&s, id, 0) - a - moved).abs() < 1e-6 && (drawn_x(&s, id, 1) - v - moved).abs() < 1e-6);
    // A negative step goes back towards the line's start, past it.
    step_at(&mut s, id, 0, -3);
    assert_eq!(style_at(&s, id, 0).kerning_before, -2.0 * inc);
    assert!((drawn_x(&s, id, 0) - a + 2.0 * moved).abs() < 1e-6);
    // One undo step each.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(style_at(&s, id, 0).kerning_before, inc);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(style_at(&s, id, 0).kerning_before, 0.0);
    assert!((drawn_x(&s, id, 0) - a).abs() < 1e-6);
}

#[test]
fn inside_a_line_a_step_still_kerns_the_character_before_the_caret() {
    let mut s = session();
    let id = text(&mut s, "AV");
    step_at(&mut s, id, 1, 1);
    let (a, v) = (style_at(&s, id, 0), style_at(&s, id, 1));
    assert_eq!((a.kerning_before, v.kerning_before, v.kerning), (0.0, 0.0, None));
    assert!(a.kerning.is_some(), "the pair after A");
}

#[test]
fn the_start_of_a_paragraph_or_after_a_forced_line_break_kerns_the_character_after_it() {
    let mut s = session();
    let inc = s.prefs.tracking_increment;
    let id = text(&mut s, "One\nTwo");
    let t = drawn_x(&s, id, 4);
    step_at(&mut s, id, 4, 1);
    assert_eq!((style_at(&s, id, 3).kerning, style_at(&s, id, 4).kerning_before), (None, inc), "not the line break's kerning");
    assert!(drawn_x(&s, id, 4) > t, "T moves");
    let forced = text(&mut s, "One\u{2028}Two");
    let after = "One\u{2028}".len();
    step_at(&mut s, forced, after, 1);
    assert_eq!((style_at(&s, forced, 3).kerning, style_at(&s, forced, after).kerning_before), (None, inc));
}

#[test]
fn an_empty_line_has_nothing_to_kern() {
    let mut s = session();
    let id = text(&mut s, "One\n\nTwo");
    let before = obj(&s, id).runs;
    assert_eq!(step_at(&mut s, id, 4, 1)["ids"], json!([]));
    assert_eq!(obj(&s, id).runs, before);
    // Nor does the end of text after a line break.
    let end = text(&mut s, "One\n");
    assert_eq!(step_at(&mut s, end, 4, 1)["ids"], json!([]));
}

#[test]
fn reset_at_the_start_of_a_line_clears_the_space_before_and_a_full_reset_clears_it_too() {
    let mut s = session();
    let id = text(&mut s, "AV");
    s.execute("text.setRangeStyle", &json!({"id": id.0, "kerning": -40, "tracking": 100})).unwrap();
    step_at(&mut s, id, 0, 2);
    assert_ne!(style_at(&s, id, 0).kerning_before, 0.0);
    // At the caret before A only the space before A goes; A keeps its kerning after it.
    caret_in(&mut s, id, 0);
    s.execute("type.resetTracking", &json!({})).unwrap();
    let st = style_at(&s, id, 0);
    assert_eq!((st.kerning_before, st.kerning, st.tracking), (0.0, Some(-40.0), 100.0));
    step_at(&mut s, id, 0, 2);
    s.execute("type.resetTracking", &json!({"ids": [id.0]})).unwrap();
    let st = style_at(&s, id, 0);
    assert_eq!((st.kerning_before, st.kerning, st.tracking), (0.0, None, 0.0), "all manual kerning");
}

#[test]
fn set_range_style_sets_the_space_before() {
    let mut s = session();
    let id = text(&mut s, "AV");
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 0, "end": 1, "kerningBefore": 50})).unwrap();
    assert_eq!(style_at(&s, id, 0).kerning_before, 50.0);
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 0, "end": 1, "kerningBefore": 20000})).unwrap();
    assert_eq!(style_at(&s, id, 0).kerning_before, 10000.0, "kerning's limits");
    assert!(s.execute("text.setRangeStyle", &json!({"id": id.0, "kerningBefore": "auto"})).is_err());
}

/// The Type tool: Alt+← at the caret before the first character kerns it, and Alt+→ back.
#[test]
fn alt_arrows_at_the_caret_before_the_first_character_kern_it() {
    let mut s = session();
    let inc = s.prefs.tracking_increment;
    let id = text(&mut s, "Hello");
    let v = caret_in(&mut s, id, 0);
    let alt = Mods { alt: true, ..Mods::default() };
    s.tool_key(ToolKey::Left, alt, v).unwrap();
    assert_eq!((style_at(&s, id, 0).kerning_before, style_at(&s, id, 0).kerning), (-inc, None));
    s.tool_key(ToolKey::Right, alt, v).unwrap();
    s.tool_key(ToolKey::Right, alt, v).unwrap();
    assert_eq!(style_at(&s, id, 0).kerning_before, inc);
}

/// The Eyedropper copies how type looks, not where a line starts: a sample from a first character
/// with a space before it leaves the target's first character where it was.
#[test]
fn the_eyedropper_keeps_the_targets_space_before() {
    let mut s = session();
    let src = text(&mut s, "Source");
    s.execute("text.setRangeStyle", &json!({"id": src.0, "start": 0, "end": 1, "kerningBefore": 300, "tracking": 50})).unwrap();
    let dst = text(&mut s, "Target");
    s.execute("appearance.copyFrom", &json!({"source": src.0})).unwrap();
    let st = style_at(&s, dst, 0);
    assert_eq!((st.tracking, st.kerning_before), (50.0, 0.0), "the tracking goes, the space before doesn't");
}

/// A character style captured from a line's first character leaves its space before out, and a
/// style can't be given one: it would space the first character of every paragraph set in it.
#[test]
fn a_character_style_leaves_the_space_before_out() {
    let mut s = session();
    let id = text(&mut s, "Source");
    s.execute("text.setRangeStyle", &json!({"id": id.0, "kerningBefore": 300, "tracking": 50})).unwrap();
    s.execute("charStyle.new", &json!({"name": "Body", "id": id.0})).unwrap();
    let attrs = s.doc().unwrap().doc.char_styles[0].attrs.clone();
    assert_eq!(attrs.get("tracking"), Some(&json!(50.0)));
    assert!(!attrs.contains_key("kerningBefore"), "{attrs:?}");
    let err = s.execute("charStyle.new", &json!({"name": "Spaced", "attrs": {"kerningBefore": 100}})).unwrap_err();
    assert!(err.to_string().contains("kerningBefore"), "{err}");
}
