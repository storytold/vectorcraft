//! Paragraph attributes per paragraph: ranged Paragraph panel commands, paragraph styles on one
//! paragraph, splitting and merging paragraphs while editing, undo and threaded text.

use serde_json::json;
use vectorcraft_doc::{Justify, NodeId, NodeKind, TextObject};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn area(s: &mut Session, text: &str) -> u64 {
    s.execute("text.create", &json!({"x": 10, "y": 10, "text": text, "area": {"width": 300, "height": 300}})).unwrap()["id"].as_u64().unwrap()
}

fn text(s: &Session, id: u64) -> TextObject {
    match &s.doc().unwrap().doc.node(NodeId(id)).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

fn justs(s: &Session, id: u64) -> Vec<Justify> {
    text(s, id).paragraph_styles().iter().map(|p| p.justify).collect()
}

#[test]
fn ranged_paragraph_commands_touch_only_their_paragraphs() {
    let mut s = session();
    let id = area(&mut s, "one\ntwo\nthree");
    // A caret in "two" (byte 5): only the second paragraph.
    s.execute("text.setStyle", &json!({"id": id, "justify": "center", "start": 5, "end": 5})).unwrap();
    assert_eq!(justs(&s, id), [Justify::Auto, Justify::Center, Justify::Auto]);
    // A selection from "one" into "two": both.
    s.execute("text.setFormat", &json!({"ids": [id], "spaceBefore": 9, "leftIndent": 4, "start": 1, "end": 5})).unwrap();
    let t = text(&s, id);
    let sb: Vec<f64> = t.paragraph_styles().iter().map(|p| p.space_before).collect();
    assert_eq!(sb, [9.0, 9.0, 0.0]);
    assert_eq!(t.para.left_indent, 4.0, "paragraph 0 is `para`");
    // Character attributes of setFormat follow the range too.
    s.execute("text.setFormat", &json!({"ids": [id], "underline": true, "start": 4, "end": 7})).unwrap();
    let t = text(&s, id);
    assert!(t.runs.iter().any(|r| r.text == "two" && r.style.underline));
    assert!(t.runs.iter().any(|r| r.text.starts_with("one") && !r.style.underline));
    // The layout sees each paragraph's alignment.
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert!(lay.lines[1].x0 > lay.lines[0].x0 + 20.0, "the centred line sits right of the left one");
    // No range: every paragraph (as before).
    s.execute("text.setStyle", &json!({"id": id, "justify": "right"})).unwrap();
    assert_eq!(justs(&s, id), [Justify::Right; 3]);
    // Undo steps back through the ranged edits.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(justs(&s, id), [Justify::Auto, Justify::Center, Justify::Auto]);
    // Tabs per paragraph; a query reads the paragraph at `start`.
    s.execute("text.tabs.set", &json!({"ids": [id], "stops": [{"position": 40}], "start": 9, "end": 9})).unwrap();
    assert_eq!(s.execute("text.tabs.get", &json!({"ids": [id], "start": 9})).unwrap()["stops"].as_array().unwrap().len(), 1);
    assert_eq!(s.execute("text.tabs.get", &json!({"ids": [id]})).unwrap()["stops"], json!([]));
    // Junk ranges are errors, not panics.
    assert!(s.execute("text.setFormat", &json!({"ids": [id], "spaceAfter": 1, "start": -3})).is_err());
    assert!(s.execute("text.setStyle", &json!({"id": id, "justify": "left", "end": "x"})).is_err());
    s.execute("text.setFormat", &json!({"ids": [id], "spaceAfter": 1, "start": 1_000_000, "end": u64::MAX})).unwrap();
    assert_eq!(text(&s, id).para_at(2).space_after, 1.0, "past the end: the last paragraph");
}

/// A file whose paragraphs hold an indent or a space before or after past `MAX_COORD`, or a tab
/// stop outside 0 to 100,000 pt, opens with the nearer end of each range, in the type and in
/// paragraph styles. Values past the Paragraph panel's 1296 pt and within those ranges stay.
#[test]
fn a_file_with_paragraph_spacing_out_of_range_opens_with_it_in_range() {
    let mut s = session();
    let id = s.execute("text.create", &json!({"x": 40, "y": 60, "size": 20, "text": "Line\tone\nLine two"})).unwrap()["id"].as_u64().unwrap();
    let mut d = (*s.doc().unwrap().doc).clone();
    let Some(NodeKind::Text(t)) = d.node_mut(NodeId(id)).map(|n| &mut n.kind) else { panic!("not text") };
    let tab = |position| vectorcraft_doc::TabStop { position, align: vectorcraft_doc::TabAlign::Left, leader: String::new(), align_on: '.' };
    let mut first = t.para.clone();
    (first.left_indent, first.first_line_indent, first.space_after) = (f64::MAX, -1e7, 5000.0);
    first.tabs = vec![tab(-5.0), tab(f64::MAX)];
    let mut second = t.para.clone();
    (second.right_indent, second.space_before) = (-f64::MAX, 1e7);
    t.set_paragraph_styles(vec![first, second]);
    let attrs = json!({"left_indent": 1e9, "space_after": 2000, "tabs": [serde_json::to_value(tab(-5.0)).unwrap()]});
    d.para_styles.push(vectorcraft_doc::TextStyleDef { name: "Wide".into(), attrs: attrs.as_object().unwrap().clone() });
    let data = vectorcraft_format::base64_encode(&vectorcraft_format::save_file(&d));
    s.execute("document.open", &json!({"name": "spacing.vectorcraft", "dataBase64": data})).unwrap();
    let max = vectorcraft_doc::MAX_COORD;
    let t = text(&s, id);
    let (p0, p1) = (t.para_at(0), t.para_at(1));
    assert_eq!((p0.left_indent, p0.first_line_indent, p0.space_after), (max, -max, 5000.0));
    assert_eq!(p0.tabs.iter().map(|t| t.position).collect::<Vec<_>>(), [0.0, 100_000.0]);
    assert_eq!((p1.right_indent, p1.space_before), (-max, max));
    let def = &s.doc().unwrap().doc.para_styles.iter().find(|d| d.name == "Wide").unwrap().attrs;
    assert_eq!((&def["left_indent"], &def["space_after"], &def["tabs"][0]["position"]), (&json!(max), &json!(2000), &json!(0.0)));
    // With the spacing kept as saved, the type's bounds were infinite.
    let b = s.doc().unwrap().doc.bounds_of(&[NodeId(id)], false).unwrap();
    assert!(b.width().is_finite() && b.height().is_finite(), "{b:?}");
}

#[test]
fn editing_splits_and_merges_paragraph_styles() {
    let mut s = session();
    let id = area(&mut s, "one\ntwo\nthree");
    s.execute("text.setStyle", &json!({"id": id, "justify": "center", "start": 5, "end": 5})).unwrap();
    // A break inside "two": both halves keep its style.
    s.execute("text.editRange", &json!({"id": id, "start": 5, "end": 5, "insert": "\n"})).unwrap();
    assert_eq!(text(&s, id).plain_text(), "one\nt\nwo\nthree");
    assert_eq!(justs(&s, id), [Justify::Auto, Justify::Center, Justify::Center, Justify::Auto]);
    // Deleting the break after "one": the first paragraph's style wins.
    s.execute("text.editRange", &json!({"id": id, "start": 3, "end": 4})).unwrap();
    assert_eq!(text(&s, id).plain_text(), "onet\nwo\nthree");
    assert_eq!(justs(&s, id), [Justify::Auto, Justify::Center, Justify::Auto]);
    // Pasting styled runs with breaks in "wo": the new paragraphs continue it.
    let runs = json!([{"text": "A\nB\n", "style": text(&s, id).runs[0].style}]);
    s.execute("text.editRange", &json!({"id": id, "start": 6, "end": 6, "runs": runs})).unwrap();
    assert_eq!(justs(&s, id), [Justify::Auto, Justify::Center, Justify::Center, Justify::Center, Justify::Auto]);
    // Undo restores the styles with the text.
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(justs(&s, id), [Justify::Auto, Justify::Center, Justify::Center, Justify::Auto]);
    // Replacing all the text: every paragraph takes the first one's attributes.
    s.execute("text.setText", &json!({"ids": [id], "text": "x\ny"})).unwrap();
    assert_eq!(justs(&s, id), [Justify::Auto, Justify::Auto]);
    assert!(text(&s, id).paras.is_empty());
}

#[test]
fn return_in_the_type_tool_continues_the_paragraph_it_splits() {
    let mut s = session();
    let v = ViewInfo::default();
    let id = s.execute("text.create", &json!({"x": 100, "y": 100, "text": "one\ntwo", "size": 20})).unwrap()["id"].as_u64().unwrap();
    s.execute("text.setStyle", &json!({"id": id, "justify": "right", "start": 0, "end": 0})).unwrap();
    s.execute("text.setStyle", &json!({"id": id, "justify": "center", "start": 5, "end": 5})).unwrap();
    s.select_tool("type", v).unwrap();
    // Click into the first line (right-aligned: it ends at the origin), then End and Return.
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &text(&s, id));
    let l0 = &lay.lines[0];
    let (x, y) = (100.0 + (l0.x0 + l0.x1) / 2.0, 100.0 + l0.baseline - 5.0);
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, x, y), v).unwrap();
    }
    s.tool_key(ToolKey::End, Mods::default(), v).unwrap();
    s.tool_key(ToolKey::Enter, Mods::default(), v).unwrap();
    s.tool_text("new", v).unwrap();
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    assert_eq!(text(&s, id).plain_text(), "one\nnew\ntwo");
    assert_eq!(justs(&s, id), [Justify::Right, Justify::Right, Justify::Center]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(justs(&s, id), [Justify::Right, Justify::Center]);
}

#[test]
fn paragraph_style_applies_to_one_paragraph() {
    let mut s = session();
    let id = area(&mut s, "Title\nBody text");
    // The second paragraph keeps new type's attributes, whatever they are.
    let body = text(&s, id).para_at(1).clone();
    s.execute("paraStyle.new", &json!({"name": "Head", "attrs": {"justify": "Center", "space_after": 8.0}})).unwrap();
    s.execute("paraStyle.apply", &json!({"name": "Head", "id": id, "start": 2, "end": 2})).unwrap();
    let t = text(&s, id);
    assert_eq!(t.para_at(0).style_name.as_deref(), Some("Head"));
    assert_eq!((t.para_at(0).justify, t.para_at(0).space_after), (Justify::Center, 8.0));
    assert_eq!(t.para_at(1), &body, "new type's");
    let list = s.execute("paraStyle.list", &json!({})).unwrap();
    assert_eq!(list["styles"][1]["uses"], 1, "one paragraph uses it");
    assert_eq!(list["styles"][0]["uses"], 1, "the other is Normal");
    // Redefining updates that paragraph only; renaming follows it.
    s.execute("paraStyle.setAttrs", &json!({"name": "Head", "attrs": {"justify": "Right"}})).unwrap();
    assert_eq!(justs(&s, id), [Justify::Right, Justify::Auto]);
    s.execute("paraStyle.rename", &json!({"name": "Head", "to": "Heading"})).unwrap();
    assert_eq!(text(&s, id).para_at(0).style_name.as_deref(), Some("Heading"));
    // New from the selection reads the caret's paragraph.
    s.execute("paraStyle.new", &json!({"name": "From body", "id": id, "start": 8})).unwrap();
    let defs = &s.doc().unwrap().doc.para_styles;
    assert_eq!(defs.iter().find(|d| d.name == "From body").unwrap().attrs["justify"], "Auto");
}

#[test]
fn threaded_frames_keep_each_paragraphs_style() {
    let mut s = session();
    let story = "The quick brown fox jumps over the lazy dog and keeps running. ".repeat(3) + "\nSecond paragraph.\nThird.";
    let a =
        s.execute("text.create", &json!({"x": 10, "y": 10, "text": story, "area": {"width": 150, "height": 60}})).unwrap()["id"].as_u64().unwrap();
    let b = s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 150, "height": 200})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    let joined = |s: &Session| format!("{}{}", text(s, a).plain_text(), text(s, b).plain_text());
    assert_eq!(joined(&s), story);
    // Centre the second paragraph (in frame b) and right-align the third.
    let tb = text(&s, b).plain_text();
    let second = tb.find("Second").unwrap();
    s.execute("text.setStyle", &json!({"id": b, "justify": "center", "start": second, "end": second})).unwrap();
    let third = text(&s, b).plain_text().find("Third").unwrap();
    s.execute("text.setStyle", &json!({"id": b, "justify": "right", "start": third, "end": third})).unwrap();
    assert_eq!(justs(&s, b), [Justify::Auto, Justify::Center, Justify::Right]);
    // Typing in the first frame re-flows the story: the styles move with their paragraphs.
    s.execute("text.editRange", &json!({"id": a, "start": 0, "end": 0, "insert": "More words at the start of the story. "})).unwrap();
    let (ja, jb) = (justs(&s, a), justs(&s, b));
    assert_eq!(ja, [Justify::Auto]);
    assert_eq!(jb, [Justify::Auto, Justify::Center, Justify::Right]);
    // A break in the first frame makes a new paragraph there; the later ones keep their styles.
    s.execute("text.editRange", &json!({"id": a, "start": 4, "end": 4, "insert": "\n"})).unwrap();
    let all: Vec<Justify> = [justs(&s, a), justs(&s, b)].concat();
    assert_eq!(all.iter().filter(|j| **j == Justify::Center).count(), 1);
    assert_eq!(all.last(), Some(&Justify::Right));
    // Restyling the paragraph that continues into frame b (its first) sticks after the re-flow.
    s.execute("text.setStyle", &json!({"id": b, "justify": "justifyAll", "start": 0, "end": 0})).unwrap();
    assert_eq!(justs(&s, b)[0], Justify::JustifyAll);
    assert_eq!(*justs(&s, a).last().unwrap(), Justify::JustifyAll, "the paragraph's part in frame a follows");
}

/// Upstream's paragraph attributes (direction, mojikumi, leading model) follow a range like the
/// others: only the touched paragraphs change, and the layout reads each paragraph's own.
#[test]
fn ranged_set_format_sets_direction_mojikumi_and_leading_model_per_paragraph() {
    use vectorcraft_doc::{LeadingModel, Mojikumi, ParaDirection};
    let mut s = session();
    let id = area(&mut s, "Hello.\nHello.");
    // A caret in the second paragraph (byte 9).
    s.execute(
        "text.setFormat",
        &json!({"ids": [id], "direction": "rightToLeft", "leadingModel": "emBoxTop", "mojikumi": "none", "start": 9, "end": 9}),
    )
    .unwrap();
    let t = text(&s, id);
    let (p0, p1) = (t.para_at(0).clone(), t.para_at(1).clone());
    assert_eq!((p0.direction, p0.leading_model, p0.mojikumi), (None, LeadingModel::RomanBaseline, Mojikumi::LineEndHalf));
    assert_eq!((p1.direction, p1.leading_model, p1.mojikumi), (Some(ParaDirection::RightToLeft), LeadingModel::EmBoxTop, Mojikumi::None));
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
    assert!(!lay.lines[0].rtl && lay.lines[1].rtl, "each paragraph's direction");
    // Return in the right-to-left paragraph continues it.
    s.execute("text.editRange", &json!({"id": id, "start": 13, "end": 13, "insert": "\nX"})).unwrap();
    let t = text(&s, id);
    assert_eq!(t.para_at(2).direction, Some(ParaDirection::RightToLeft));
    // No range: every paragraph.
    s.execute("text.setFormat", &json!({"ids": [id], "direction": "auto"})).unwrap();
    assert!(text(&s, id).paragraph_styles().iter().all(|p| p.direction.is_none()));
}
