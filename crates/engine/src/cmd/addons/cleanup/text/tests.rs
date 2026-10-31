//! Artwork Cleanup's text tools: pieces stay where their glyphs were drawn and keep their styles,
//! each command is one undo step and refuses cleanly when there is nothing to do.

use serde_json::{Value, json};
use vectorcraft_doc::{Justify, NodeId, NodeKind, TextKind, TextObject};
use vectorcraft_geom::Point;

use crate::{EngineError, Session};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    s
}

fn text(s: &mut Session, p: Value) -> NodeId {
    NodeId(s.execute("text.create", &p).unwrap()["id"].as_u64().unwrap())
}

fn obj(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.select(|_, sel| sel.set(ids.iter().copied())).unwrap();
}

fn ids(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect()
}

/// Each drawn character of `ids` (spaces left out) with where its glyph starts on the page, in
/// the order of the objects and their text.
fn glyphs(s: &Session, ids: &[NodeId]) -> Vec<(char, Point)> {
    let mut out = vec![];
    for id in ids {
        let t = obj(s, *id);
        let text = t.plain_text();
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
        let mut gs: Vec<_> = lay.glyphs.iter().filter(|g| g.len > 0).collect();
        gs.sort_by_key(|g| g.byte);
        gs.dedup_by_key(|g| g.byte);
        for g in gs {
            let c = text[g.byte..].chars().next().unwrap();
            if !c.is_whitespace() {
                out.push((c, t.xf * g.origin));
            }
        }
    }
    out
}

fn assert_same_places(was: &[(char, Point)], now: &[(char, Point)]) {
    assert_eq!(was.iter().map(|g| g.0).collect::<String>(), now.iter().map(|g| g.0).collect::<String>());
    for (a, b) in was.iter().zip(now) {
        assert!((a.1 - b.1).hypot() < 0.02, "{:?} moved to {:?}", a, b);
    }
}

fn undo_steps(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn doc_json(s: &Session) -> String {
    serde_json::to_string(&s.doc().unwrap().doc.layers).unwrap()
}

fn other(r: Result<Value, EngineError>) -> String {
    match r {
        Err(EngineError::Other(m)) => m,
        r => panic!("expected a refusal, got {r:?}"),
    }
}

#[test]
fn break_into_words_keeps_places_styles_and_is_one_step() {
    let mut s = session();
    let id = text(&mut s, json!({"x": 40, "y": 100, "text": "Hello big World", "size": 24}));
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 6, "end": 9, "size": 36, "tracking": 50})).unwrap();
    let rotated = text(&mut s, json!({"x": 300, "y": 200, "text": "Turned words", "size": 18, "rotation": 30}));
    let was = glyphs(&s, &[id, rotated]);
    let before = doc_json(&s);
    let steps = undo_steps(&s);
    select(&mut s, &[id, rotated]);
    let r = s.execute("addon.cleanup.breakText", &json!({"by": "words"})).unwrap();
    let pieces = ids(&r);
    assert_eq!(pieces.len(), 5);
    assert_eq!(undo_steps(&s), steps + 1);
    assert_eq!(s.doc().unwrap().selection.objects.len(), 5, "the pieces are selected");
    let texts: Vec<String> = pieces.iter().map(|p| obj(&s, *p).plain_text()).collect();
    assert_eq!(texts, ["Hello", "big", "World", "Turned", "words"]);
    assert_same_places(&was, &glyphs(&s, &pieces));
    let big = obj(&s, pieces[1]);
    assert!(matches!(big.kind, TextKind::Point));
    assert_eq!((big.runs[0].style.size, big.runs[0].style.tracking), (36.0, 50.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(doc_json(&s), before);
}

#[test]
fn break_area_type_into_lines_paragraphs_and_glyphs() {
    for (by, count) in [("lines", None), ("paragraphs", Some(2)), ("glyphs", None)] {
        let mut s = session();
        let id = text(
            &mut s,
            json!({"x": 40, "y": 40, "text": "A paragraph long enough to wrap in its frame\nSecond one", "size": 14, "area": {"width": 140, "height": 200}}),
        );
        let t = obj(&s, id);
        let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t);
        assert!(lay.lines.len() >= 3, "the first paragraph wraps");
        let was = glyphs(&s, &[id]);
        select(&mut s, &[id]);
        let r = s.execute("addon.cleanup.breakText", &json!({"by": by})).unwrap();
        let pieces = ids(&r);
        let want = count.unwrap_or(if by == "lines" { lay.lines.len() } else { was.len() });
        assert_eq!(pieces.len(), want, "{by}");
        assert_same_places(&was, &glyphs(&s, &pieces));
        if by == "paragraphs" {
            // The wrapped lines end in line breaks.
            assert!(obj(&s, pieces[0]).plain_text().contains('\u{2028}'));
        }
    }
}

#[test]
fn break_text_refuses_what_it_cannot_break() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    assert!(other(s.execute("addon.cleanup.breakText", &json!({}))).contains("point or area type"));
    let r = s.execute("addon.cleanup.breakText", &json!({"by": "sentences"}));
    assert!(matches!(r, Err(EngineError::BadParams { .. })), "{r:?}");
}

#[test]
fn combine_puts_glyph_pieces_back_together_where_they_were() {
    let mut s = session();
    let id = text(&mut s, json!({"x": 50, "y": 120, "text": "Water Lily", "size": 30}));
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 6, "end": 10, "fill": "#ff0000"})).unwrap();
    let was = glyphs(&s, &[id]);
    select(&mut s, &[id]);
    let pieces = ids(&s.execute("addon.cleanup.breakText", &json!({"by": "glyphs"})).unwrap());
    assert_eq!(pieces.len(), 9);
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.combineText", &json!({})).unwrap();
    assert_eq!(undo_steps(&s), steps + 1);
    let made = ids(&r);
    assert_eq!((made.len(), r["combined"].as_u64()), (1, Some(8)));
    let t = obj(&s, made[0]);
    assert_eq!(t.plain_text(), "Water Lily", "a space goes back between the words");
    assert_same_places(&was, &glyphs(&s, &made));
    let red = t.runs.iter().find(|r| r.text.contains('L')).unwrap();
    assert_ne!(red.style.fill, t.runs[0].style.fill, "styles stay");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(pieces[1]).is_some(), "undo brings the pieces back");
}

#[test]
fn combine_joins_only_pieces_on_one_line_turned_alike() {
    let mut s = session();
    let a = text(&mut s, json!({"x": 50, "y": 100, "text": "Left", "size": 20}));
    let b = text(&mut s, json!({"x": 110, "y": 100, "text": "Right", "size": 20}));
    let c = text(&mut s, json!({"x": 50, "y": 200, "text": "Below", "size": 20}));
    let d = text(&mut s, json!({"x": 300, "y": 100, "text": "Turned", "size": 20, "rotation": 45}));
    let was = glyphs(&s, &[a, b]);
    select(&mut s, &[a, b, c, d]);
    let r = s.execute("addon.cleanup.combineText", &json!({})).unwrap();
    assert_eq!(ids(&r), [a]);
    assert_eq!(obj(&s, a).plain_text(), "Left Right");
    assert_same_places(&was, &glyphs(&s, &[a]));
    let doc = &s.doc().unwrap().doc;
    assert!(doc.node(b).is_none() && doc.node(c).is_some() && doc.node(d).is_some());
    select(&mut s, &[c, d]);
    assert!(other(s.execute("addon.cleanup.combineText", &json!({}))).contains("combine"));
}

#[test]
fn remove_line_and_paragraph_breaks() {
    let mut s = session();
    let id = text(&mut s, json!({"x": 40, "y": 60, "text": "one\u{2028}two \u{2028}three\nfour\n\nfive", "size": 12}));
    select(&mut s, &[id]);
    let before = doc_json(&s);
    let r = s.execute("addon.cleanup.removeBreaks", &json!({"kind": "line"})).unwrap();
    assert_eq!(r["removed"], 2);
    assert_eq!(obj(&s, id).plain_text(), "one two three\nfour\n\nfive");
    let r = s.execute("addon.cleanup.removeBreaks", &json!({"kind": "paragraph"})).unwrap();
    assert_eq!(r["removed"], 3);
    let t = obj(&s, id);
    assert_eq!((t.plain_text().as_str(), t.paragraph_count()), ("one two three four five", 1));
    assert_eq!(other(s.execute("addon.cleanup.removeBreaks", &json!({"kind": "line"}))), "No line breaks to remove");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(doc_json(&s), before);
    assert!(matches!(s.execute("addon.cleanup.removeBreaks", &json!({})), Err(EngineError::BadParams { .. })));
}

/// The Type tool editing `id` with bytes `anchor..caret` selected.
fn edit_text(s: &mut Session, id: NodeId, anchor: usize, caret: usize) {
    s.select_tool("type", Default::default()).unwrap();
    s.set_tool_option("textSelection", &json!({"editing": id.0, "anchor": anchor, "caret": caret}));
}

#[test]
fn remove_breaks_in_the_characters_a_type_tool_selected() {
    let mut s = session();
    let id = text(&mut s, json!({"x": 40, "y": 60, "text": "a\nb\nc", "size": 12}));
    select(&mut s, &[id]);
    edit_text(&mut s, id, 0, 2);
    s.execute("addon.cleanup.removeBreaks", &json!({"kind": "paragraph"})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "a b\nc");
}

#[test]
fn split_at_the_cursor_and_around_a_selection() {
    for (anchor, caret, want) in [(6, 6, &["Split", "here please"][..]), (6, 10, &["Split", "here", "please"][..])] {
        let mut s = session();
        let id = text(&mut s, json!({"x": 40, "y": 60, "text": "Split here please", "size": 20, "justify": "center"}));
        let was = glyphs(&s, &[id]);
        select(&mut s, &[id]);
        let r = s.execute("addon.cleanup.splitText", &json!({}));
        assert!(matches!(&r, Err(EngineError::Disabled(_, why)) if why.contains("Type tool")), "{r:?}");
        edit_text(&mut s, id, anchor, caret);
        let steps = undo_steps(&s);
        let r = s.execute("addon.cleanup.splitText", &json!({})).unwrap();
        assert_eq!(undo_steps(&s), steps + 1);
        let pieces = ids(&r);
        let texts: Vec<String> = pieces.iter().map(|p| obj(&s, *p).plain_text()).collect();
        assert_eq!(texts, want);
        assert_same_places(&was, &glyphs(&s, &pieces));
        assert!(s.tool_options()["editing"].is_null(), "editing ends");
    }
}

#[test]
fn split_keeps_the_later_lines_of_a_wrapped_piece() {
    let mut s = session();
    let id = text(
        &mut s,
        json!({"x": 40, "y": 40, "text": "First words then a long run of text that wraps in the frame", "size": 14, "area": {"width": 160, "height": 200}}),
    );
    let was = glyphs(&s, &[id]);
    select(&mut s, &[id]);
    edit_text(&mut s, id, 6, 6);
    let pieces = ids(&s.execute("addon.cleanup.splitText", &json!({})).unwrap());
    assert_eq!(pieces.len(), 2);
    assert_same_places(&was, &glyphs(&s, &pieces));
}

#[test]
fn set_alignment_keeps_the_text_in_place() {
    let mut s = session();
    let id = text(&mut s, json!({"x": 100, "y": 60, "text": "Stay put", "size": 20}));
    let multi = text(&mut s, json!({"x": 100, "y": 160, "text": "Short\u{2028}The longest line", "size": 20}));
    let was = glyphs(&s, &[id]);
    let longest: Vec<_> = glyphs(&s, &[multi]).into_iter().skip(5).collect();
    select(&mut s, &[id, multi]);
    for align in ["center", "right", "left"] {
        s.execute("addon.cleanup.alignInPlace", &json!({"align": align})).unwrap();
        assert_same_places(&was, &glyphs(&s, &[id]));
        assert_same_places(&longest, &glyphs(&s, &[multi]).into_iter().skip(5).collect::<Vec<_>>());
    }
    assert_eq!(obj(&s, id).para.justify, Justify::Left);
    assert!(other(s.execute("addon.cleanup.alignInPlace", &json!({"align": "left"}))).contains("already"));
    assert!(matches!(s.execute("addon.cleanup.alignInPlace", &json!({"align": "up"})), Err(EngineError::BadParams { .. })));
}

#[test]
fn reset_text_transform_takes_off_rotation_and_with_scale_the_horizontal_scale() {
    let mut s = session();
    let id = text(&mut s, json!({"x": 100, "y": 100, "text": "Tilted", "size": 20, "rotation": 30}));
    s.execute("text.setRangeStyle", &json!({"id": id.0, "hScale": 150})).unwrap();
    let centre = |s: &Session| {
        let t = obj(s, id);
        t.xf * vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).bounds.center()
    };
    let was = centre(&s);
    let size = obj(&s, id).effective_char_style(&obj(&s, id).runs[0].style).unwrap().size;
    select(&mut s, &[id]);
    s.execute("addon.cleanup.resetTextTransform", &json!({})).unwrap();
    let t = obj(&s, id);
    let [_, b, c, _, _, _] = t.xf.as_coeffs();
    assert!(b.abs() < 1e-9 && c.abs() < 1e-9, "{:?}", t.xf);
    assert!((centre(&s) - was).hypot() < 1e-6);
    assert!((t.effective_char_style(&t.runs[0].style).unwrap().size - size).abs() < 1e-6, "the size stays");
    assert!(t.runs[0].style.h_scale > 100.0, "the horizontal scale stays");
    assert!(other(s.execute("addon.cleanup.resetTextTransform", &json!({}))).contains("no transform"));
    s.execute("addon.cleanup.resetTextTransform", &json!({"scale": true})).unwrap();
    assert_eq!(obj(&s, id).runs[0].style.h_scale, 100.0);
}

#[test]
fn replace_missing_fonts_with_the_default_or_a_chosen_font() {
    let mut s = session();
    let gone = text(&mut s, json!({"x": 40, "y": 60, "text": "Lost", "size": 20, "font": "No Such Font Family", "style": "Bold"}));
    let kept = text(&mut s, json!({"x": 40, "y": 120, "text": "Fine", "size": 20}));
    let fine = obj(&s, kept);
    let r = s.execute("addon.cleanup.replaceFonts", &json!({})).unwrap();
    assert_eq!((r["runs"].as_u64(), r["font"].as_str()), (Some(1), Some("Source Sans 3")));
    let t = obj(&s, gone);
    assert_eq!((t.runs[0].style.font_family.as_str(), t.runs[0].style.font_style.as_str()), ("Source Sans 3", "Bold"));
    assert_eq!(obj(&s, kept), fine);
    assert_eq!(other(s.execute("addon.cleanup.replaceFonts", &json!({}))), "No fonts are missing");
    let r = s.execute("addon.cleanup.replaceFonts", &json!({"font": "No Such Font Family"}));
    assert!(matches!(r, Err(EngineError::BadParams { .. })), "{r:?}");
}
