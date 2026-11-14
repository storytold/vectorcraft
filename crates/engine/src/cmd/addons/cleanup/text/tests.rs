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

/// Tracked area type broken into lines or paragraphs stays where it was, however it is aligned:
/// a wrap after a hyphen or at a space becomes a line break or a paragraph's end, and the line
/// leaves the tracking after its last character out of its width there as it did at the wrap.
/// (Words chosen without a kerning pair across the wrap, which the pieces don't keep.)
#[test]
fn break_tracked_area_type_keeps_its_aligned_lines_in_place() {
    for justify in ["center", "right"] {
        for text_of in ["Mode-Mode-Mode", "Wave Wave Wave"] {
            for by in ["paragraphs", "lines"] {
                let mut s = session();
                let id = text(&mut s, json!({"x": 40, "y": 40, "text": text_of, "size": 20, "area": {"width": 90, "height": 200}}));
                s.execute("text.setRangeStyle", &json!({"id": id.0, "tracking": 200})).unwrap();
                s.execute("text.setStyle", &json!({"ids": [id.0], "justify": justify})).unwrap();
                let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &obj(&s, id));
                assert!(lay.lines.len() > 1, "{text_of}: it wraps");
                let was = glyphs(&s, &[id]);
                select(&mut s, &[id]);
                let pieces = ids(&s.execute("addon.cleanup.breakText", &json!({"by": by})).unwrap());
                assert_same_places(&was, &glyphs(&s, &pieces));
            }
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

/// Two area frames threaded together, with `story` flowing through them → (first, second).
fn threaded(s: &mut Session, story: &str) -> (NodeId, NodeId) {
    let a = text(s, json!({"x": 10, "y": 300, "text": story, "size": 12, "area": {"width": 150, "height": 60}}));
    let b = NodeId(s.execute("shape.rectangle", &json!({"x": 200, "y": 300, "width": 150, "height": 60})).unwrap()["id"].as_u64().unwrap());
    select(s, &[a, b]);
    s.execute("text.thread.create", &json!({})).unwrap();
    (a, b)
}

fn thread_scans() -> usize {
    super::THREAD_SCANS.with(|c| c.get())
}

/// Threaded frames in the selection stay as they are and are reported as skipped; the other type
/// is broken. The document's threads are gone through once for the whole selection, not once
/// per object.
#[test]
fn break_text_skips_threaded_frames_and_reads_the_threads_once() {
    let mut s = session();
    let (a, b) = threaded(&mut s, "Threaded story text");
    let mut plain = vec![];
    for i in 0..6 {
        plain.push(text(&mut s, json!({"x": 20, "y": 30 + 30 * i, "text": "Two words", "size": 14})));
    }
    let was = glyphs(&s, &plain);
    let frames = (obj(&s, a), obj(&s, b));
    let before = doc_json(&s);
    let steps = undo_steps(&s);
    let mut all = vec![a, b];
    all.extend(&plain);
    select(&mut s, &all);
    let scans = thread_scans();
    let r = s.execute("addon.cleanup.breakText", &json!({"by": "words"})).unwrap();
    assert_eq!(thread_scans() - scans, 1, "the threads are gone through once");
    assert_eq!(ids(&json!({"ids": r["skipped"]})), [a, b]);
    assert_eq!((obj(&s, a), obj(&s, b)), frames, "the threaded frames stay as they are");
    let pieces = ids(&r);
    assert_eq!(pieces.len(), 12);
    assert_eq!(s.doc().unwrap().selection.objects.len(), 12, "the pieces are selected");
    assert_same_places(&was, &glyphs(&s, &pieces));
    assert_eq!(undo_steps(&s), steps + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(doc_json(&s), before);
}

#[test]
fn split_text_refuses_a_threaded_frame() {
    let mut s = session();
    let (a, _) = threaded(&mut s, "Threaded story text");
    select(&mut s, &[a]);
    edit_text(&mut s, a, 9, 9);
    let before = doc_json(&s);
    assert!(other(s.execute("addon.cleanup.splitText", &json!({}))).contains("point or area type"));
    assert_eq!(doc_json(&s), before);
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

/// Pieces in horizontally scaled type: the tracking that puts each back where it was is measured in
/// the em as the Horizontal Scale draws it (`vectorcraft_text::spacing_scale`), as layout spaces it.
#[test]
fn combine_puts_horizontally_scaled_pieces_back_where_they_were() {
    let mut s = session();
    let a = text(&mut s, json!({"x": 50, "y": 100, "text": "Left", "size": 20}));
    let b = text(&mut s, json!({"x": 160, "y": 100, "text": "Right", "size": 20}));
    for id in [a, b] {
        s.execute("text.setRangeStyle", &json!({"id": id.0, "hScale": 200})).unwrap();
    }
    let was = glyphs(&s, &[a, b]);
    select(&mut s, &[a, b]);
    let r = s.execute("addon.cleanup.combineText", &json!({})).unwrap();
    assert_eq!(ids(&r), [a]);
    assert_eq!(obj(&s, a).plain_text(), "Left Right");
    assert_same_places(&was, &glyphs(&s, &[a]));
}

/// Join Point Text makes one object of stacked one-line pieces: lines top to bottom as paragraphs,
/// pieces on a line left to right, each line's baseline where it was (its leading is the distance
/// from the line above), in one undo step. A piece turned unlike the top-left one stays.
#[test]
fn join_stacks_lines_into_one_object_with_their_spacing() {
    let mut s = session();
    let a = text(&mut s, json!({"x": 50, "y": 100, "text": "First", "size": 20}));
    let b = text(&mut s, json!({"x": 120, "y": 101, "text": "line", "size": 20}));
    let c = text(&mut s, json!({"x": 50, "y": 130, "text": "Second", "size": 20}));
    let d = text(&mut s, json!({"x": 50, "y": 166, "text": "Third", "size": 20}));
    let turned = text(&mut s, json!({"x": 300, "y": 300, "text": "Turned", "size": 20, "rotation": 45}));
    let was = glyphs(&s, &[a, c, d]);
    select(&mut s, &[d, b, turned, c, a]);
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.joinText", &json!({})).unwrap();
    assert_eq!(undo_steps(&s), steps + 1);
    assert_eq!((r["id"].as_u64(), r["joined"].as_u64(), r["lines"].as_u64()), (Some(a.0), Some(4), Some(3)));
    let t = obj(&s, a);
    assert_eq!(t.plain_text(), "First line\nSecond\nThird");
    assert_eq!(t.runs.iter().filter(|r| r.text.contains("Third")).map(|r| r.style.leading).collect::<Vec<_>>(), [Some(36.0)]);
    // Each line's first glyph is where it was.
    let now = glyphs(&s, &[a]);
    for ch in ['F', 'S', 'T'] {
        let (p, q) = (was.iter().find(|g| g.0 == ch).unwrap().1, now.iter().find(|g| g.0 == ch).unwrap().1);
        assert!((p - q).hypot() < 0.05, "{ch}: {p:?} moved to {q:?}");
    }
    let doc = &s.doc().unwrap().doc;
    assert!(doc.node(b).is_none() && doc.node(c).is_none() && doc.node(d).is_none() && doc.node(turned).is_some());
    select(&mut s, &[turned]);
    assert!(other(s.execute("addon.cleanup.joinText", &json!({}))).contains("join"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(c).is_some(), "undo brings the pieces back");
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

/// `ids: null` is `ids` left out: the characters the Type tool selected lose their breaks, and
/// the rest of the text keeps them.
#[test]
fn remove_breaks_with_ids_null_in_the_characters_a_type_tool_selected() {
    let mut s = session();
    let id = text(&mut s, json!({"x": 40, "y": 60, "text": "a\nb\nc", "size": 12}));
    select(&mut s, &[id]);
    edit_text(&mut s, id, 0, 2);
    s.execute("addon.cleanup.removeBreaks", &json!({"kind": "paragraph", "ids": null})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), "a b\nc");
}

/// Remove Breaks reads a threaded story whose frames are all selected as one text and each other
/// object on its own, going through the document's threads once for the whole selection.
#[test]
fn remove_breaks_reads_the_threads_once() {
    let mut s = session();
    let (a, b) = threaded(&mut s, "one\ntwo");
    let mut plain = vec![];
    for i in 0..6 {
        plain.push(text(&mut s, json!({"x": 20, "y": 30 + 30 * i, "text": "a\nb", "size": 12})));
    }
    let mut all = vec![a, b];
    all.extend(&plain);
    select(&mut s, &all);
    let scans = thread_scans();
    let r = s.execute("addon.cleanup.removeBreaks", &json!({"kind": "paragraph"})).unwrap();
    assert_eq!(thread_scans() - scans, 1, "the threads are gone through once");
    assert_eq!(r["removed"], 7);
    assert_eq!(obj(&s, a).plain_text(), "one two");
    for id in plain {
        assert_eq!(obj(&s, id).plain_text(), "a b");
    }
}

/// Lock `id` (`locked`), else hide it, as an undo step of its own.
fn lock_or_hide(s: &mut Session, id: NodeId, locked: bool) {
    s.edit("Lock or Hide", |d, _| {
        let n = d.node_mut(id).unwrap();
        if locked {
            n.locked = true;
        } else {
            n.visible = false;
        }
        Ok(())
    })
    .unwrap();
}

/// Remove Breaks takes the locked and hidden type that `ids` names, and the locked member of a
/// group it names, as one undo step; from the selection it takes only type that can be edited, and
/// nothing changed makes no undo step.
#[test]
fn remove_breaks_named_locked_and_hidden_type() {
    let mut s = session();
    let locked = text(&mut s, json!({"x": 40, "y": 60, "text": "a\nb", "size": 12}));
    let hidden = text(&mut s, json!({"x": 40, "y": 120, "text": "c\nd", "size": 12}));
    let grouped = text(&mut s, json!({"x": 40, "y": 180, "text": "e\nf", "size": 12}));
    let free = text(&mut s, json!({"x": 40, "y": 240, "text": "g\nh", "size": 12}));
    select(&mut s, &[grouped, free]);
    let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    lock_or_hide(&mut s, locked, true);
    lock_or_hide(&mut s, hidden, false);
    lock_or_hide(&mut s, grouped, true);
    select(&mut s, &[locked, hidden]);
    let steps = undo_steps(&s);
    assert_eq!(other(s.execute("addon.cleanup.removeBreaks", &json!({"kind": "paragraph"}))), "No paragraph breaks to remove");
    assert_eq!(undo_steps(&s), steps, "the selected locked and hidden type stays as it is");
    let r = s.execute("addon.cleanup.removeBreaks", &json!({"kind": "paragraph", "ids": [locked.0, hidden.0, group.0]})).unwrap();
    assert_eq!(undo_steps(&s), steps + 1);
    assert_eq!(r["removed"], 4);
    for (id, now) in [(locked, "a b"), (hidden, "c d"), (grouped, "e f"), (free, "g h")] {
        assert_eq!(obj(&s, id).plain_text(), now);
    }
    assert_eq!(other(s.execute("addon.cleanup.removeBreaks", &json!({"kind": "paragraph", "ids": [locked.0]}))), "No paragraph breaks to remove");
    assert_eq!(undo_steps(&s), steps + 1, "nothing changed, no undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(obj(&s, locked).plain_text(), "a\nb");
}

/// A threaded story whose frames `ids` all names is read as one text even when a frame is locked.
#[test]
fn remove_breaks_named_story_with_a_locked_frame() {
    let mut s = session();
    let (a, b) = threaded(&mut s, "one\ntwo");
    lock_or_hide(&mut s, a, true);
    let r = s.execute("addon.cleanup.removeBreaks", &json!({"kind": "paragraph", "ids": [a.0, b.0]})).unwrap();
    assert_eq!(r["removed"], 1);
    assert_eq!(obj(&s, a).plain_text(), "one two");
}

/// Type on a locked artboard stays as it is even when `ids` names it, with no undo step.
#[test]
fn remove_breaks_named_type_on_a_locked_artboard() {
    let mut s = session();
    let on_board = text(&mut s, json!({"x": 40, "y": 60, "text": "a\nb", "size": 12}));
    s.edit("Lock Artboard", |d, _| {
        d.artboards[0].locked = true;
        Ok(())
    })
    .unwrap();
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.removeBreaks", &json!({"kind": "paragraph", "ids": [on_board.0]}));
    assert_eq!(other(r), "No paragraph breaks to remove");
    assert_eq!(undo_steps(&s), steps);
    assert_eq!(obj(&s, on_board).plain_text(), "a\nb");
}

/// Break Text, Combine and Join Point Text, Set Alignment, Keep Position and Reset Text Transform
/// act on the locked and hidden type that `ids` names (type in a locked layer included), each as
/// one undo step, and leave it as it is when it is only selected.
#[test]
fn named_locked_and_hidden_type() {
    let mut s = session();
    let words = text(&mut s, json!({"x": 40, "y": 100, "text": "Two words", "size": 14}));
    lock_or_hide(&mut s, words, true);
    select(&mut s, &[words]);
    assert!(s.execute("addon.cleanup.breakText", &json!({"by": "words"})).is_err());
    let steps = undo_steps(&s);
    let r = s.execute("addon.cleanup.breakText", &json!({"by": "words", "ids": [words.0]})).unwrap();
    assert_eq!((ids(&r).len(), undo_steps(&s)), (2, steps + 1));
    assert!(s.doc().unwrap().selection.objects.is_empty(), "the locked pieces stay unselected");

    let mut s = session();
    let a = text(&mut s, json!({"x": 50, "y": 100, "text": "Left", "size": 20}));
    let b = text(&mut s, json!({"x": 110, "y": 100, "text": "Right", "size": 20}));
    lock_or_hide(&mut s, a, false);
    lock_or_hide(&mut s, b, false);
    select(&mut s, &[a, b]);
    assert!(s.execute("addon.cleanup.combineText", &json!({})).is_err());
    let r = s.execute("addon.cleanup.combineText", &json!({"ids": [a.0, b.0]})).unwrap();
    assert_eq!(ids(&r), [a]);
    assert_eq!(obj(&s, a).plain_text(), "Left Right");
    assert!(s.doc().unwrap().selection.objects.is_empty(), "the hidden object stays unselected");

    let mut s = session();
    let first = text(&mut s, json!({"x": 50, "y": 100, "text": "First", "size": 20}));
    let second = text(&mut s, json!({"x": 50, "y": 130, "text": "Second", "size": 20}));
    let layer = s.doc().unwrap().doc.layers[0].id;
    lock_or_hide(&mut s, layer, true);
    select(&mut s, &[first, second]);
    assert!(s.execute("addon.cleanup.joinText", &json!({})).is_err());
    let r = s.execute("addon.cleanup.joinText", &json!({"ids": [first.0, second.0]})).unwrap();
    assert_eq!(r["id"].as_u64(), Some(first.0));
    assert_eq!(obj(&s, first).plain_text(), "First\nSecond");
    assert!(s.doc().unwrap().selection.objects.is_empty(), "the object in the locked layer stays unselected");

    let mut s = session();
    let lines = text(&mut s, json!({"x": 50, "y": 100, "text": "a\nlonger line", "size": 20}));
    lock_or_hide(&mut s, lines, false);
    select(&mut s, &[lines]);
    assert!(s.execute("addon.cleanup.alignInPlace", &json!({"align": "center"})).is_err());
    s.execute("addon.cleanup.alignInPlace", &json!({"align": "center", "ids": [lines.0]})).unwrap();
    assert!(obj(&s, lines).para_styles().all(|p| p.justify == Justify::Center));

    let mut s = session();
    let turned = text(&mut s, json!({"x": 300, "y": 200, "text": "Turned", "size": 18, "rotation": 30}));
    lock_or_hide(&mut s, turned, true);
    select(&mut s, &[turned]);
    assert!(s.execute("addon.cleanup.resetTextTransform", &json!({})).is_err());
    let steps = undo_steps(&s);
    s.execute("addon.cleanup.resetTextTransform", &json!({"ids": [turned.0]})).unwrap();
    assert_eq!(undo_steps(&s), steps + 1);
    let [_, b, c, _, _, _] = obj(&s, turned).xf.as_coeffs();
    assert!(b.abs() < 1e-9 && c.abs() < 1e-9, "the rotation is gone");
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

#[test]
fn replace_missing_fonts_reaches_the_type_in_symbols() {
    let mut s = session();
    let gone = text(&mut s, json!({"x": 40, "y": 60, "text": "Lost", "size": 20, "font": "No Such Font Family"}));
    s.execute("symbol.new", &json!({"name": "Mark", "ids": [gone.0]})).unwrap();
    let family = |s: &Session| crate::cmd::fonts::symbol_texts(&s.doc().unwrap().doc, None)[0].0.runs[0].style.font_family.clone();
    assert_eq!(family(&s), "No Such Font Family");
    let r = s.execute("addon.cleanup.replaceFonts", &json!({})).unwrap();
    assert_eq!((r["runs"].as_u64(), r["font"].as_str()), (Some(1), Some("Source Sans 3")), "{r}");
    assert_eq!(family(&s), "Source Sans 3");
    assert_eq!(crate::cmd::fonts::substitution_warning(&s.doc().unwrap().doc), None);
    assert_eq!(other(s.execute("addon.cleanup.replaceFonts", &json!({}))), "No fonts are missing");
    // A symbol being edited: its type on the editing layer is replaced once.
    let gone = text(&mut s, json!({"x": 40, "y": 60, "text": "Lost", "size": 20, "font": "No Such Font Family"}));
    s.execute("symbol.new", &json!({"name": "Mark 2", "ids": [gone.0]})).unwrap();
    s.execute("symbol.edit", &json!({"name": "Mark 2"})).unwrap();
    let r = s.execute("addon.cleanup.replaceFonts", &json!({})).unwrap();
    assert_eq!(r["runs"].as_u64(), Some(1), "{r}");
    s.execute("symbol.exitEditing", &json!({})).unwrap();
    assert_eq!(crate::cmd::fonts::substitution_warning(&s.doc().unwrap().doc), None);
}

/// Replace Missing Fonts reaches the type in opacity masks (of a hidden object, disabled, in a
/// symbol's art) and in a pattern swatch nothing paints with, and the export warning, the missing
/// fonts list and Find Font name that type's font until it is replaced.
#[test]
fn replace_missing_fonts_reaches_the_type_in_masks_and_patterns() {
    let mut s = session();
    let mask = |s: &mut Session, owner: NodeId, art: NodeId| {
        s.edit("Mask", |d, _| {
            let art = d.remove(art)?;
            d.node_mut(owner).unwrap().extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new((*art).clone(), true)));
            Ok(())
        })
        .unwrap();
    };
    let lost = |s: &mut Session| text(s, json!({"x": 40, "y": 60, "text": "Lost", "size": 20, "font": "No Such Font Family"}));
    let rect =
        |s: &mut Session| NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 20})).unwrap()["id"].as_u64().unwrap());
    let (hidden, disabled, in_symbol) = (rect(&mut s), rect(&mut s), rect(&mut s));
    for owner in [hidden, disabled, in_symbol] {
        let t = lost(&mut s);
        mask(&mut s, owner, t);
    }
    s.execute("select.set", &json!({"ids": [hidden.0]})).unwrap();
    s.execute("object.hide", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [disabled.0]})).unwrap();
    s.execute("transparency.disableOpacityMask", &json!({})).unwrap();
    s.execute("symbol.new", &json!({"name": "Mark", "ids": [in_symbol.0]})).unwrap();
    let t = lost(&mut s);
    s.execute("object.pattern.make", &json!({"ids": [t.0], "name": "Unused", "edit": false})).unwrap();
    select(&mut s, &[t]);
    s.execute("edit.clear", &json!({})).unwrap();
    let mut on_layers = 0;
    s.doc().unwrap().doc.walk(|n| on_layers += usize::from(matches!(n.kind, NodeKind::Text(_))));
    assert_eq!(on_layers, 0, "the type is only in masks, a symbol and a pattern");
    let missing = |s: &Session| crate::cmd::fontfiles::missing_fonts(&s.doc().unwrap().doc).into_iter().map(|f| f.family).collect::<Vec<_>>();
    assert_eq!(missing(&s), ["No Such Font Family"]);
    assert!(crate::cmd::fonts::substitution_warning(&s.doc().unwrap().doc).is_some_and(|w| w.contains("No Such Font Family")));
    let listed = s.execute("text.fonts", &json!({})).unwrap();
    assert_eq!((listed[0]["family"].as_str(), listed[0]["objects"].as_u64()), (Some("No Such Font Family"), Some(4)), "{listed}");
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let before = steps(&s);
    let r = s.execute("addon.cleanup.replaceFonts", &json!({})).unwrap();
    assert_eq!(r["runs"].as_u64(), Some(4), "{r}");
    assert_eq!(steps(&s), before + 1);
    assert!(missing(&s).is_empty());
    assert_eq!(crate::cmd::fonts::substitution_warning(&s.doc().unwrap().doc), None);
    assert_eq!(other(s.execute("addon.cleanup.replaceFonts", &json!({}))), "No fonts are missing");
    assert_eq!(steps(&s), before + 1);
}
