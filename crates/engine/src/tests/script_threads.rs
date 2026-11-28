//! Threaded type in the script object model: `nextFrame`, `previousFrame`, `story`,
//! `document.stories`, and the text of a story of several frames.

use serde_json::{Value, json};

use crate::*;

fn run(s: &mut Session, source: &str) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "t.js"}))
}

/// What the run logged.
fn log(s: &mut Session, source: &str) -> Vec<Value> {
    let r = run(s, source).unwrap_or_else(|e| panic!("{source}: {e}"));
    r["log"].as_array().cloned().unwrap_or_default()
}

/// The message of the run's failure.
fn failure(s: &mut Session, source: &str) -> String {
    run(s, source).map(|r| panic!("{source} ran: {r}")).unwrap_err().to_string()
}

fn undo_steps(s: &Session) -> usize {
    s.active().unwrap().history.undo.len()
}

fn new_doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    s
}

/// Area type `width` × `height` at `x` holding `text`.
fn area(s: &mut Session, x: f64, width: f64, height: f64, text: &str) -> NodeId {
    let r = s.execute("text.create", &json!({"x": x, "y": 10, "text": text, "size": 12, "area": {"width": width, "height": height}})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn plain(s: &Session, id: NodeId) -> String {
    match &s.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    }
}

fn threads(s: &Session) -> Vec<Vec<NodeId>> {
    s.active().unwrap().doc.text_threads.clone()
}

/// The text of every frame of the document's threads, in thread order.
fn story(s: &Session, frames: &[NodeId]) -> String {
    frames.iter().map(|f| plain(s, *f)).collect()
}

/// Two area frames threaded into one story whose text starts in the first, one line high, and
/// flows on into the second.
fn threaded(text: &str) -> (Session, NodeId, NodeId) {
    let mut s = new_doc();
    let a = area(&mut s, 10.0, 90.0, 18.0, text);
    let b = area(&mut s, 150.0, 200.0, 200.0, "");
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(threads(&s), vec![vec![a, b]]);
    (s, a, b)
}

const STORY: &str = "alpha beta gamma\ndelta \u{1F600}";

#[test]
fn stories_list_one_story_for_each_thread_and_each_frame_in_none() {
    let (mut s, _, _) = threaded(STORY);
    area(&mut s, 400.0, 120.0, 40.0, "Alone");
    let out = log(
        &mut s,
        "var d = app.activeDocument, st = d.stories[1];\
         vc.log(d.stories.length); vc.log(st.typename); vc.log(st.textFrames.length);\
         vc.log(st.textFrames[0].nextFrame.previousFrame.contents === st.textFrames[0].contents);\
         vc.log(st.textFrames[1].nextFrame === null && st.textFrames[0].previousFrame === null);\
         vc.log(d.stories[0].textFrames[0].contents); vc.log(d.stories[0].textFrames.length);\
         d.textFrames.add(); vc.log(d.stories.length);\
         vc.log(d.stories.parent.typename); vc.log(st.parent.typename); vc.log(d.stories.typename);\
         vc.log(d.textFrames[1].story.textFrames[0].contents);",
    );
    assert_eq!(
        out,
        vec![
            json!(2),
            json!("Story"),
            json!(2),
            json!(true),
            json!(true),
            json!("Alone"),
            json!(1),
            json!(3),
            json!("Document"),
            json!("Document"),
            json!("Stories"),
            json!("Alone")
        ]
    );
}

#[test]
fn a_story_range_counts_utf16_units_across_its_frames() {
    let (mut s, a, b) = threaded(STORY);
    let (first, second) = (plain(&s, a), plain(&s, b));
    assert!(!first.is_empty() && !second.is_empty(), "the story flows through both frames: {first:?} {second:?}");
    assert_eq!(first.clone() + &second, STORY);
    let out = log(
        &mut s,
        "var st = app.activeDocument.textFrames[0].story, r = st.textRange;\
         vc.log(st.length); vc.log(r.length); vc.log(r.contents); vc.log(r.typename);\
         vc.log(st.characters.length); vc.log(st.words.length); vc.log(st.paragraphs.length);\
         vc.log(st.paragraphs[0].contents); vc.log(st.words[4].contents); vc.log(st.words[4].start);\
         vc.log(r.paragraphs[1].getParagraphLength()); vc.log(r.story.length);\
         vc.log(app.activeDocument.textFrames[0].textRange.length < r.length);",
    );
    assert_eq!(
        out,
        vec![
            json!(25),
            json!(25),
            json!("alpha beta gamma\rdelta \u{1F600}"),
            json!("TextRange"),
            json!(24),
            json!(5),
            json!(2),
            json!("alpha beta gamma\r"),
            json!("\u{1F600}"),
            json!(23),
            json!(8),
            json!(25),
            json!(true)
        ]
    );
}

/// The members a long story's range does not have yet throw that they are not available, and
/// first that a range that went stale is stale.
#[test]
fn a_story_range_names_the_members_it_does_not_have_yet() {
    let (mut s, _, b) = threaded(STORY);
    let out = log(
        &mut s,
        "var r = app.activeDocument.textFrames[0].story.textRange;\
         try { r.characterAttributes; } catch (e) { vc.log(String(e.message)); }\
         try { r.size = 20; } catch (e) { vc.log(String(e.message)); }\
         try { app.activeDocument.textFrames[0].story.lines; } catch (e) { vc.log(String(e.message)); }",
    );
    assert_eq!(
        out,
        vec![
            json!("TextRange.characterAttributes is not available yet"),
            json!("TextRange.size is not available yet"),
            json!("Story.lines is not available yet")
        ]
    );
    let message = failure(
        &mut s,
        &format!(
            "var r = app.activeDocument.textFrames[0].story.textRange; vc.run('text.editRange', {{id: {}, start: 0, end: 0, insert: 'z'}}); r.characterAttributes;",
            b.0
        ),
    );
    assert!(message.contains("Stale text range"), "{message}");
    let message = failure(
        &mut s,
        &format!(
            "var r = app.activeDocument.textFrames[0].story.textRange; vc.run('text.editRange', {{id: {}, start: 0, end: 0, insert: 'z'}}); r.typename;",
            b.0
        ),
    );
    assert!(message.contains("Stale text range"), "{message}");
}

/// Ranges of one story read from different frames keep their places through each other's edits.
#[test]
fn story_ranges_read_from_different_frames_follow_each_others_edits() {
    let (mut s, _, _) = threaded(STORY);
    let out = log(
        &mut s,
        "var t = app.activeDocument.textFrames, x = t[0].story.words[0], y = t[1].story.words[1];\
         x.contents = 'Q'; vc.log(y.contents); vc.log(y.start); y.contents = 'B'; vc.log(x.contents);\
         vc.log(t[1].story.textRange.contents);",
    );
    assert_eq!(out, vec![json!("beta"), json!(2), json!("Q"), json!("Q B gamma\rdelta \u{1F600}")]);
}

/// A story range that an edit covers holds the edit's new text, as a frame's range does.
#[test]
fn a_story_range_an_edit_covers_holds_the_new_text() {
    let (mut s, _, _) = threaded(STORY);
    let out = log(
        &mut s,
        "var st = app.activeDocument.textFrames[0].story, w = st.words[1], p = st.paragraphs[1];\
         st.textRange.contents = 'Z'; vc.log(w.contents); vc.log(p.contents); vc.log(w.typename); vc.log(st.words.typename);",
    );
    assert_eq!(out, vec![json!("Z"), json!("Z"), json!("TextRange"), json!("Words")]);
}

#[test]
fn a_story_edit_across_frames_reflows_in_one_undo_step() {
    let (mut s, a, b) = threaded(STORY);
    let before = undo_steps(&s);
    let out = log(
        &mut s,
        "var st = app.activeDocument.textFrames[0].story, frame = app.activeDocument.textFrames[1].textRange;\
         var p = st.paragraphs[0], tail = st.paragraphs[1];\
         p.contents = 'X\\r'; vc.log(st.textRange.contents); vc.log(tail.contents); vc.log(tail.start);\
         try { frame.contents; } catch (e) { vc.log(String(e.message)); }\
         st.words[2].remove(); vc.log(st.textRange.contents);",
    );
    assert_eq!(out, vec![json!("X\rdelta \u{1F600}"), json!("delta \u{1F600}"), json!(2), json!("Stale text range"), json!("X\rdelta ")]);
    assert_eq!(story(&s, &[a, b]), "X\ndelta ");
    assert_eq!(undo_steps(&s), before + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(story(&s, &[a, b]), STORY);
}

#[test]
fn a_story_range_is_stale_after_another_change_to_the_story() {
    let (mut s, _, b) = threaded(STORY);
    let message = failure(
        &mut s,
        &format!(
            "var r = app.activeDocument.textFrames[0].story.textRange; vc.run('text.editRange', {{id: {}, start: 0, end: 0, insert: 'z'}}); r.contents;",
            b.0
        ),
    );
    assert!(message.contains("Stale text range"), "{message}");
}

#[test]
fn a_story_range_made_before_file_revert_is_stale_even_when_the_text_is_the_same() {
    let (mut s, _, _) = threaded(STORY);
    let path = std::env::temp_dir().join(format!("vc-story-revert-{}.vectorcraft", std::process::id()));
    s.execute("document.save", &json!({"path": path.to_string_lossy()})).unwrap();
    let out = log(
        &mut s,
        "var r = app.activeDocument.textFrames[0].story.words[1];\
         vc.run('shape.rectangle', {x: 1, y: 1, width: 5, height: 5}); vc.run('file.revert', {});\
         var st = app.activeDocument.textFrames[0].story;\
         vc.log(st.textFrames.length); vc.log(st.textRange.contents);\
         try { vc.log(r.contents); } catch (e) { vc.log(String(e.message)); }\
         vc.log(st.words[1].contents);",
    );
    let _ = std::fs::remove_file(&path);
    assert_eq!(out[0], json!(2), "the reverted file keeps the thread");
    assert_eq!(out[1], json!("alpha beta gamma\rdelta \u{1F600}"), "the reverted file holds the same text");
    assert!(out[2].as_str().is_some_and(|m| m.contains("Stale text range")), "{out:?}");
    assert_eq!(out[3], json!("beta"), "a range made after the revert reads the reverted text");
}

#[test]
fn next_frame_joins_two_stories_in_one_undo_step_keeping_the_selection() {
    let mut s = new_doc();
    let a = area(&mut s, 10.0, 120.0, 60.0, "First story.");
    let b = area(&mut s, 150.0, 120.0, 60.0, "Second.");
    let p = NodeId(s.execute("text.create", &json!({"x": 20, "y": 300, "text": "Point"})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [p.0]})).unwrap();
    let before = undo_steps(&s);
    let out = log(
        &mut s,
        "var t = app.activeDocument.textFrames; t[2].nextFrame = t[1];\
         vc.log(app.activeDocument.stories.length); vc.log(t[1].previousFrame.contents); vc.log(t[2].story.textRange.contents);\
         t[2].nextFrame = t[1];",
    );
    assert_eq!(out, vec![json!(2), json!("First story.Second."), json!("First story.Second.")]);
    assert_eq!(threads(&s), vec![vec![a, b]]);
    assert_eq!(story(&s, &[a, b]), "First story.Second.");
    assert_eq!(undo_steps(&s), before + 1);
    assert_eq!(s.active().unwrap().selection.objects.to_vec(), vec![p]);
    // previousFrame links the other way.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(threads(&s).is_empty());
    assert_eq!((plain(&s, a), plain(&s, b)), ("First story.".into(), "Second.".into()));
    log(&mut s, "var t = app.activeDocument.textFrames; t[2].previousFrame = t[1];");
    assert_eq!(threads(&s), vec![vec![b, a]]);
    assert_eq!(story(&s, &[b, a]), "Second.First story.");
}

#[test]
fn null_breaks_the_link_and_each_frame_keeps_its_text() {
    let (mut s, a, b) = threaded(STORY);
    let (first, second) = (plain(&s, a), plain(&s, b));
    let before = undo_steps(&s);
    let out = log(
        &mut s,
        "var t = app.activeDocument.textFrames, st = t[0].story; t[0].previousFrame = null;\
         vc.log(app.activeDocument.stories.length); vc.log(st.textFrames.length); vc.log(t[1].nextFrame);\
         t[1].nextFrame = null;",
    );
    assert_eq!(out, vec![json!(2), json!(1), json!(null)]);
    assert!(threads(&s).is_empty());
    assert_eq!((plain(&s, a), plain(&s, b)), (first, second));
    assert_eq!(undo_steps(&s), before + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(threads(&s), vec![vec![a, b]]);
}

#[test]
fn area_and_path_text_thread_the_new_frame_to_a_base_frame() {
    let mut s = new_doc();
    let base = area(&mut s, 10.0, 120.0, 40.0, "Base text that is long enough to flow on into the frames made after it.");
    s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 120, "height": 60})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 350, "y": 10, "width": 120, "height": 60})).unwrap();
    s.execute("path.create", &json!({"anchors": [{"x": 20, "y": 300}, {"x": 400, "y": 320}], "closed": false})).unwrap();
    let before = undo_steps(&s);
    let out = log(
        &mut s,
        "var d = app.activeDocument, p = d.pageItems, base = d.textFrames[0];\
         var after = d.textFrames.areaText(p[2], undefined, base);\
         var first = d.textFrames.areaText(p[1], TextOrientation.HORIZONTAL, base, false);\
         var last = d.textFrames.pathText(p[0], undefined, undefined, undefined, after, true);\
         vc.log(base.story.textFrames.length); vc.log(base.previousFrame.kind); vc.log(after.nextFrame.kind); vc.log(d.stories.length);",
    );
    assert_eq!(out, vec![json!(4), json!("TextType.AREATEXT"), json!("TextType.PATHTEXT"), json!(1)]);
    let t = threads(&s);
    assert_eq!(t.len(), 1);
    assert_eq!(t[0][1], base, "the frame made with postFix false comes before the base frame: {t:?}");
    assert_eq!(undo_steps(&s), before + 1);
}

#[test]
fn a_bad_base_frame_is_refused_before_a_frame_is_made() {
    let mut s = new_doc();
    s.execute("text.create", &json!({"x": 20, "y": 300, "text": "Point"})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 120, "height": 60})).unwrap();
    let count = |s: &Session| s.active().unwrap().doc.node_count();
    let (n, before) = (count(&s), undo_steps(&s));
    let d = "var d = app.activeDocument, p = d.pageItems;";
    assert!(failure(&mut s, &format!("{d} d.textFrames.areaText(p[0], undefined, d.textFrames[0]);")).contains("only area type"));
    assert!(failure(&mut s, &format!("{d} d.textFrames.areaText(p[0], undefined, p[0]);")).contains("baseFrame must be a TextFrame"));
    assert!(failure(&mut s, &format!("{d} d.textFrames.areaText(p[0], undefined, d.textFrames[0], 'yes');")).contains("postFix"));
    assert_eq!((count(&s), undo_steps(&s)), (n, before));
}

/// A base frame given with a path inside a compound path is refused before the path is turned
/// into type, so a script that catches the error finds the path as it was.
#[test]
fn a_base_frame_for_a_path_in_a_compound_path_leaves_the_path() {
    let mut s = new_doc();
    area(&mut s, 10.0, 120.0, 60.0, "Base.");
    let r1 = s.execute("shape.rectangle", &json!({"x": 200, "y": 10, "width": 80, "height": 80})).unwrap()["id"].as_u64().unwrap();
    let r2 = s.execute("shape.rectangle", &json!({"x": 300, "y": 10, "width": 80, "height": 80})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [r1, r2]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let before = undo_steps(&s);
    let out = log(
        &mut s,
        "var d = app.activeDocument, p = null;\
         for (var i = 0; i < d.pageItems.length; i++) { if (d.pageItems[i].typename == 'PathItem') { p = d.pageItems[i]; break; } }\
         try { d.textFrames.areaText(p, undefined, d.textFrames[0]); } catch (e) { vc.log(String(e.message)); }\
         vc.log(p.typename); vc.log(d.textFrames.length);",
    );
    assert_eq!(out, vec![json!("baseFrame: type made from a path inside a compound path cannot be threaded"), json!("PathItem"), json!(1)]);
    assert_eq!(undo_steps(&s), before);
}

/// A thread of `a` and `b`, a third area frame `c`, and point type `p`; what the document holds
/// now, to check that a refused change left it alone.
fn three() -> (Session, [NodeId; 4]) {
    let (mut s, a, b) = threaded(STORY);
    let c = area(&mut s, 400.0, 120.0, 40.0, "Third");
    let p = NodeId(s.execute("text.create", &json!({"x": 20, "y": 300, "text": "Point"})).unwrap()["id"].as_u64().unwrap());
    (s, [a, b, c, p])
}

fn unchanged(s: &Session, ids: &[NodeId], before: &(Vec<Vec<NodeId>>, Vec<String>, usize)) {
    assert_eq!((threads(s), ids.iter().map(|i| plain(s, *i)).collect::<Vec<_>>(), undo_steps(s)), *before);
}

fn state(s: &Session, ids: &[NodeId]) -> (Vec<Vec<NodeId>>, Vec<String>, usize) {
    (threads(s), ids.iter().map(|i| plain(s, *i)).collect(), undo_steps(s))
}

#[test]
fn a_link_to_the_frame_itself_is_refused() {
    let (mut s, ids) = three();
    let before = state(&s, &ids);
    let m = failure(&mut s, "var t = app.activeDocument.textFrames; t[2].nextFrame = t[2];");
    assert!(m.contains("nextFrame") && m.contains("itself"), "{m}");
    unchanged(&s, &ids, &before);
}

#[test]
fn a_link_that_makes_a_loop_is_refused() {
    let (mut s, ids) = three();
    let before = state(&s, &ids);
    let m = failure(&mut s, "var t = app.activeDocument.textFrames; t[2].nextFrame = t[3];");
    assert!(m.contains("loop"), "{m}");
    let m = failure(&mut s, "var t = app.activeDocument.textFrames; t[3].previousFrame = t[2];");
    assert!(m.contains("loop"), "{m}");
    unchanged(&s, &ids, &before);
}

#[test]
fn a_link_into_the_middle_of_a_story_is_refused() {
    let (mut s, ids) = three();
    let before = state(&s, &ids);
    let m = failure(&mut s, "var t = app.activeDocument.textFrames; t[1].nextFrame = t[2];");
    assert!(m.contains("already has a previous frame"), "{m}");
    let m = failure(&mut s, "var t = app.activeDocument.textFrames; t[1].previousFrame = t[3];");
    assert!(m.contains("already has a next frame"), "{m}");
    unchanged(&s, &ids, &before);
}

#[test]
fn point_type_and_other_values_cannot_be_linked() {
    let (mut s, ids) = three();
    let before = state(&s, &ids);
    let m = failure(&mut s, "var t = app.activeDocument.textFrames; t[1].nextFrame = t[0];");
    assert!(m.contains("only area type"), "{m}");
    let m = failure(&mut s, "var t = app.activeDocument.textFrames; t[0].previousFrame = t[1];");
    assert!(m.contains("only area type"), "{m}");
    let m = failure(&mut s, "app.activeDocument.textFrames[1].nextFrame = 5;");
    assert!(m.contains("TypeError") && m.contains("TextFrame or null"), "{m}");
    let m = failure(&mut s, "app.activeDocument.textFrames[1].nextFrame = undefined;");
    assert!(m.contains("TextFrame or null"), "{m}");
    unchanged(&s, &ids, &before);
}

#[test]
fn a_hidden_or_locked_frame_in_either_story_stops_a_link() {
    let (mut s, ids) = three();
    log(&mut s, "app.activeDocument.textFrames[2].locked = true;");
    let before = state(&s, &ids);
    let m = failure(&mut s, "var t = app.activeDocument.textFrames; t[2].nextFrame = t[1];");
    assert!(m.contains("hidden or locked"), "{m}");
    let m = failure(&mut s, "var t = app.activeDocument.textFrames; t[3].nextFrame = null;");
    assert!(m.contains("hidden or locked"), "{m}");
    unchanged(&s, &ids, &before);
}

#[test]
fn a_frame_of_another_document_cannot_be_linked() {
    let (mut s, ids) = three();
    let before = state(&s, &ids);
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    area(&mut s, 10.0, 100.0, 40.0, "Elsewhere");
    s.execute("shape.rectangle", &json!({"x": 150, "y": 10, "width": 80, "height": 60})).unwrap();
    let m = failure(&mut s, "app.documents[1].textFrames[1].nextFrame = app.documents[0].textFrames[0];");
    assert!(m.contains("different documents"), "{m}");
    let out = log(
        &mut s,
        "var d = app.documents[0], p = d.pageItems[0];\
         try { d.textFrames.areaText(p, undefined, app.documents[1].textFrames[1]); } catch (e) { vc.log(String(e.message)); }\
         vc.log(p.typename); vc.log(d.textFrames.length);",
    );
    assert_eq!(out, vec![json!("baseFrame: the frame is in another document"), json!("PathItem"), json!(1)]);
    s.execute("document.activate", &json!({"index": 0})).unwrap();
    unchanged(&s, &ids, &before);
}

#[test]
fn a_deleted_frame_makes_its_story_and_links_stale() {
    let (mut s, _, _) = threaded(STORY);
    let m = failure(&mut s, "var f = app.activeDocument.textFrames[0], st = f.story; f.remove(); st.length;");
    assert!(m.contains("Stale story"), "{m}");
    let m = failure(&mut s, "var f = app.activeDocument.textFrames[0], st = f.story; f.remove(); st.lines;");
    assert!(m.contains("Stale story"), "{m}");
    let m = failure(&mut s, "var f = app.activeDocument.textFrames[1], g = app.activeDocument.textFrames[0]; g.remove(); f.nextFrame = g;");
    assert!(m.contains("Stale page item"), "{m}");
    let m = failure(&mut s, "var f = app.activeDocument.textFrames[1], r = f.story.textRange; f.remove(); r.contents;");
    assert!(m.contains("Stale text range"), "{m}");
    // A story read from a frame that stays follows it when its link is broken.
    let out = log(&mut s, "var t = app.activeDocument.textFrames, st = t[0].story; t[1].nextFrame = null; vc.log(st.textFrames.length);");
    assert_eq!(out, vec![json!(1)]);
}

#[test]
fn stories_refuse_bad_keys_and_huge_numbers() {
    let (mut s, _, _) = threaded(STORY);
    let out = log(
        &mut s,
        "var st = app.activeDocument.stories; vc.log(st[1e300] === undefined); vc.log(st[-1] === undefined); vc.log(st[0.5] === undefined);",
    );
    assert_eq!(out, vec![json!(true), json!(true), json!(true)]);
    assert!(failure(&mut s, "app.activeDocument.stories.index(1e20);").contains("Not found"));
    assert!(failure(&mut s, "app.activeDocument.stories.index('nobody');").contains("Not found"));
    assert!(failure(&mut s, "app.activeDocument.stories.index({});").contains("TypeError"));
    log(&mut s, "app.activeDocument.textFrames[1].name = 'Lead';");
    assert_eq!(log(&mut s, "vc.log(app.activeDocument.stories.index('Lead').textFrames.length);"), vec![json!(2)]);
}

#[test]
fn a_long_story_walks_to_its_end_and_refuses_a_loop() {
    let mut s = new_doc();
    let ids: Vec<NodeId> = (0..120).map(|i| area(&mut s, f64::from(i % 12) * 45.0, 40.0, 20.0, if i == 0 { STORY } else { "" })).collect();
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(threads(&s), vec![ids.clone()]);
    let out = log(
        &mut s,
        "var d = app.activeDocument, f = d.stories[0].textFrames[0], n = 1; while (f.nextFrame) { f = f.nextFrame; n++; } \
         vc.log(n); vc.log(f.story.textFrames.length); vc.log(d.stories.length);",
    );
    assert_eq!(out, vec![json!(120), json!(120), json!(1)]);
    let before = state(&s, &ids);
    let m = failure(&mut s, "var t = app.activeDocument.stories[0].textFrames; t[t.length - 1].nextFrame = t[0];");
    assert!(m.contains("loop"), "{m}");
    let m = failure(&mut s, "var t = app.activeDocument.stories[0].textFrames; t[0].previousFrame = t[t.length - 1];");
    assert!(m.contains("loop"), "{m}");
    let m = failure(&mut s, "var t = app.activeDocument.stories[0].textFrames; t[60].nextFrame = t[30];");
    assert!(m.contains("loop"), "{m}");
    unchanged(&s, &ids, &before);
}

/// A stored thread that lists a frame twice, as a file can, is a story that visits each frame
/// once, so a walk along `nextFrame` ends.
#[test]
fn a_thread_listing_a_frame_twice_is_walked_once() {
    let mut s = new_doc();
    let a = area(&mut s, 10.0, 120.0, 60.0, "One.");
    let b = area(&mut s, 150.0, 120.0, 60.0, "Two.");
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).text_threads = vec![vec![a, b, a]];
    let out = log(
        &mut s,
        "var d = app.activeDocument, f = d.stories[0].textFrames[0], n = 1; while (f.nextFrame && n < 10) { f = f.nextFrame; n++; } \
         vc.log(n); vc.log(d.stories[0].textFrames.length); vc.log(d.stories.length);",
    );
    assert_eq!(out, vec![json!(2), json!(2), json!(1)]);
    let r = s.execute("text.thread.setNeighbor", &json!({"id": b.0, "side": "previous", "neighbor": null})).unwrap();
    assert_eq!(r, json!({"changed": true, "threads": [[a.0], [b.0]]}));
    assert!(threads(&s).is_empty());
}

/// When a frame is listed in two threads, a story read from a frame of the second thread reads
/// that thread's text, not the text of the story read before it from a shared frame.
#[test]
fn stories_of_threads_that_share_a_frame_keep_their_own_text() {
    let mut s = new_doc();
    let a = area(&mut s, 10.0, 120.0, 60.0, "One.");
    let b = area(&mut s, 150.0, 120.0, 60.0, "Two.");
    let c = area(&mut s, 300.0, 120.0, 60.0, "Three!");
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).text_threads = vec![vec![a, b], vec![b, c]];
    let out = log(&mut s, "var t = app.activeDocument.textFrames; vc.log(t[1].story.length); vc.log(t[0].story.length);");
    let (ab, bc) = (story(&s, &[a, b]).encode_utf16().count(), story(&s, &[b, c]).encode_utf16().count());
    assert_ne!(ab, bc);
    assert_eq!(out, vec![json!(ab), json!(bc)]);
}

/// A story and its range keep the exact id of the frame they were read from, also past 2^53.
#[test]
fn a_story_read_from_a_frame_with_a_large_id_stays_with_that_frame() {
    let mut s = new_doc();
    // The second frame's id, 2^53 + 1, is not exact as a number.
    let big = 1u64 << 53;
    s.active_mut().unwrap().change_unsaved(|d| std::sync::Arc::make_mut(d).raise_next_id(big));
    let a = area(&mut s, 10.0, 120.0, 60.0, "One.");
    let b = area(&mut s, 150.0, 120.0, 60.0, "Two.");
    assert_eq!((a.0, b.0), (big, big + 1));
    s.execute("text.thread.setNeighbor", &json!({"id": a.0, "side": "next", "neighbor": b.0})).unwrap();
    let out = log(
        &mut s,
        "var st = app.activeDocument.stories[0].textFrames[1].story, r = st.textRange; vc.log(r.contents);\
         st.textFrames[0].nextFrame = null; vc.log(st.textFrames.length); vc.log(st.textFrames[0].contents); vc.log(st.typename);",
    );
    // The whole story flows into the first frame, so the second is empty once the link is broken.
    assert_eq!(out, vec![json!("One.Two."), json!(1), json!(""), json!("Story")]);
}

#[test]
fn set_neighbor_links_and_breaks_frames_by_id_without_a_selection() {
    let mut s = new_doc();
    let a = area(&mut s, 10.0, 120.0, 60.0, "One.");
    let b = area(&mut s, 150.0, 120.0, 60.0, "Two.");
    s.execute("select.none", &json!({})).unwrap();
    let before = undo_steps(&s);
    let r = s.execute("text.thread.setNeighbor", &json!({"id": a.0, "side": "next", "neighbor": b.0})).unwrap();
    assert_eq!(r, json!({"changed": true, "threads": [[a.0, b.0]]}));
    let r = s.execute("text.thread.setNeighbor", &json!({"id": b.0, "side": "previous", "neighbor": a.0})).unwrap();
    assert_eq!(r["changed"], json!(false));
    assert_eq!(undo_steps(&s), before + 1);
    assert!(s.active().unwrap().selection.objects.is_empty());
    let r = s.execute("text.thread.setNeighbor", &json!({"id": a.0, "side": "next", "neighbor": null})).unwrap();
    assert_eq!(r, json!({"changed": true, "threads": [[a.0], [b.0]]}));
    let r = s.execute("text.thread.setNeighbor", &json!({"id": a.0, "side": "next", "neighbor": null})).unwrap();
    assert_eq!(r["changed"], json!(false));
    assert_eq!(undo_steps(&s), before + 2);
    for bad in [
        json!({"id": a.0, "side": "up", "neighbor": b.0}),
        json!({"id": a.0, "side": "next"}),
        json!({"id": a.0, "side": "next", "neighbor": 999_999}),
    ] {
        assert!(s.execute("text.thread.setNeighbor", &bad).is_err(), "{bad}");
    }
    assert_eq!(undo_steps(&s), before + 2);
}

/// Breaking a link where a paragraph starts makes the frames after it a story whose numbered
/// paragraphs count from 1, also when that story keeps two frames.
#[test]
fn a_break_at_a_paragraph_start_numbers_the_later_story_from_one() {
    let mut s = new_doc();
    let items = (1..=12).map(|i| format!("Item {i}")).collect::<Vec<_>>().join("\n");
    let ids = [area(&mut s, 10.0, 150.0, 60.0, &items), area(&mut s, 200.0, 150.0, 60.0, ""), area(&mut s, 400.0, 150.0, 200.0, "")];
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    s.execute("text.thread.create", &json!({})).unwrap();
    s.execute("text.setFormat", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "list": "number"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let numbers = |s: &Session, id: NodeId| {
        let Some(NodeKind::Text(t)) = s.active().unwrap().doc.node(id).map(|n| &n.kind) else { panic!("text") };
        assert!(!t.first_para_continues, "every frame starts a paragraph");
        vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).markers.iter().map(|m| m.text.clone()).collect::<Vec<_>>()
    };
    let first = numbers(&s, ids[0]);
    assert!(!first.is_empty() && !numbers(&s, ids[1]).is_empty(), "the first two frames hold items");
    assert_ne!(numbers(&s, ids[1]).first().map(String::as_str), Some("1."));
    let r = s.execute("text.thread.setNeighbor", &json!({"id": ids[0].0, "side": "next", "neighbor": null})).unwrap();
    assert_eq!(r, json!({"changed": true, "threads": [[ids[0].0], [ids[1].0, ids[2].0]]}));
    let later = [numbers(&s, ids[1]), numbers(&s, ids[2])].concat();
    let expected: Vec<String> = (1..=later.len()).map(|n| format!("{n}.")).collect();
    assert_eq!(later, expected, "the later story counts from 1");
    assert_eq!(numbers(&s, ids[0]), first);
}
