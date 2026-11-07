//! `edit.findReplace` keeps per-paragraph styles lined up with the text when a find or replace
//! string holds a line break, as `text.editRange` does.

use serde_json::json;
use vectorcraft_doc::{Justify, NodeId, NodeKind};
use vectorcraft_testkit::fixtures::{self, exec, id_of};

/// "A\nB\nC" with B centred and C right-aligned.
fn setup() -> (vectorcraft_engine::Session, NodeId) {
    let mut s = fixtures::session_with(400.0, 300.0);
    let id = id_of(&exec(&mut s, "text.create", json!({"x": 20, "y": 100, "text": "A\nB\nC", "size": 20})));
    exec(&mut s, "text.setStyle", json!({"id": id.0, "start": 2, "end": 3, "justify": "center"}));
    exec(&mut s, "text.setStyle", json!({"id": id.0, "start": 4, "end": 5, "justify": "right"}));
    exec(&mut s, "select.none", json!({}));
    (s, id)
}

fn state(s: &vectorcraft_engine::Session, id: NodeId) -> (String, Vec<Justify>) {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (t.plain_text(), t.paragraph_styles().iter().map(|p| p.justify).collect()),
        _ => panic!("not text"),
    }
}

#[test]
fn replacement_adding_a_line_break_keeps_later_paragraph_styles() {
    let (mut s, id) = setup();
    assert_eq!(state(&s, id).1, [Justify::Auto, Justify::Center, Justify::Right]);
    exec(&mut s, "edit.findReplace", json!({"find": "A", "replace": "A\nA2", "ids": [id.0]}));
    let (text, styles) = state(&s, id);
    assert_eq!(text, "A\nA2\nB\nC");
    assert_eq!(styles, [Justify::Auto, Justify::Auto, Justify::Center, Justify::Right]);
}

#[test]
fn find_text_with_a_line_break_merges_paragraphs_like_edit_range() {
    let (mut s, id) = setup();
    exec(&mut s, "edit.findReplace", json!({"find": "A\nB", "replace": "AB", "ids": [id.0]}));
    let (text, styles) = state(&s, id);
    assert_eq!(text, "AB\nC");
    assert_eq!(styles, [Justify::Auto, Justify::Right]);
}
