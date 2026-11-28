//! Type in the script object model: `textFrames`, text ranges, their attributes and `textFonts`.

use serde_json::{Value, json};
use vectorcraft_doc::Justify;

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

/// A 400 × 300 pt document with point type holding `text`.
fn doc_with(text: &str) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = s.execute("text.create", &json!({"x": 20, "y": 100, "text": text, "size": 12})).unwrap()["id"].as_u64().unwrap();
    (s, NodeId(id))
}

fn plain(s: &Session, id: NodeId) -> String {
    match &s.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    }
}

fn sizes(s: &Session, id: NodeId) -> Vec<(String, f64)> {
    match &s.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().map(|r| (r.text.clone(), r.style.size)).collect(),
        _ => panic!("not text"),
    }
}

#[test]
fn text_frames_add_point_area_and_path_type() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 200, "height": 100})).unwrap();
    s.execute("path.create", &json!({"anchors": [{"x": 20, "y": 200}, {"x": 300, "y": 220}], "closed": false})).unwrap();
    let before = undo_steps(&s);
    let out = log(
        &mut s,
        "var d = app.activeDocument, p = d.pageItems;\
         var area = d.textFrames.areaText(p[1]); area.contents = 'Area';\
         var path = d.textFrames.pathText(p[0]); path.contents = 'Path';\
         var pt = d.textFrames.add(); pt.contents = 'Point';\
         var at = d.textFrames.pointText([30, 40]); at.contents = 'At';\
         vc.log(d.textFrames.length); vc.log([area.kind, path.kind, pt.kind]);\
         vc.log(pt.typename); vc.log(pt instanceof TextFrame && pt instanceof PageItem);\
         vc.log(d.textFrames[3].contents); vc.log(d.textFrames[9] === undefined);\
         vc.log(at.orientation); vc.log(d.textFrames.parent.typename);",
    );
    assert_eq!(out[0], json!(4));
    assert_eq!(out[1], json!(["TextType.AREATEXT", "TextType.PATHTEXT", "TextType.POINTTEXT"]));
    assert_eq!(out[2..], [json!("TextFrame"), json!(true), json!("Area"), json!(true), json!("TextOrientation.HORIZONTAL"), json!("Document")]);
    assert_eq!(undo_steps(&s), before + 1);
    // A path of another kind, or no path, is refused.
    assert!(failure(&mut s, "app.activeDocument.textFrames.areaText(app.activeDocument.textFrames[0]);").contains("PathItem"));
    assert!(failure(&mut s, "app.activeDocument.textFrames.pointText([1]);").contains("anchor"));
    assert!(failure(&mut s, "app.activeDocument.textFrames.pointText([1, 2], 'up');").contains("TextOrientation"));
    assert_eq!(undo_steps(&s), before + 1);
    // pathText takes its orientation fourth, after the start and end positions it does not take yet.
    s.execute("path.create", &json!({"anchors": [{"x": 20, "y": 250}, {"x": 300, "y": 260}], "closed": false})).unwrap();
    let find = "var d = app.activeDocument, p = d.pageItems, q; for (var i = 0; i < p.length; i++) if (p[i].typename == 'PathItem') q = p[i];";
    assert!(failure(&mut s, &format!("{find} d.textFrames.pathText(q, 0.5);")).contains("not available yet"));
    let out = log(&mut s, &format!("{find} vc.log(d.textFrames.pathText(q, undefined, undefined, TextOrientation.VERTICAL).orientation);"));
    assert_eq!(out, vec![json!("TextOrientation.VERTICAL")]);
}

#[test]
fn contents_reads_paragraph_breaks_as_r_and_accepts_every_break() {
    let (mut s, id) = doc_with("x");
    let out = log(&mut s, "var t = app.activeDocument.textFrames[0]; t.contents = 'a\\rb\\r\\nc\\nd\\u0003e'; vc.log(t.contents);");
    assert_eq!(out, vec![json!("a\rb\rc\rd\u{3}e")]);
    assert_eq!(plain(&s, id), "a\nb\nc\nd\u{2028}e");
    assert!(failure(&mut s, "app.activeDocument.textFrames[0].contents = 5;").contains("contents must be a string"));
}

#[test]
fn offsets_count_utf16_units_and_keep_surrogate_pairs_whole() {
    let (mut s, _) = doc_with("a\u{1F600}b cd");
    let out = log(
        &mut s,
        "var r = app.activeDocument.textFrames[0].textRange, c = r.characters;\
         vc.log([r.length, r.start, r.end, c.length]);\
         vc.log([c[1].start, c[1].length, c[2].start, c[2].contents]);\
         vc.log([r.words.length, r.words[1].contents, r.words[0].characters.length]);\
         vc.log(app.activeDocument.textFrames[0].contents.length === r.length);",
    );
    assert_eq!(out, vec![json!([7, 0, 7, 6]), json!([1, 2, 3, "b"]), json!([2, "cd", 3]), json!(true)]);
}

#[test]
fn the_first_character_of_each_word_grows_in_one_undo_step() {
    let (mut s, id) = doc_with("one two");
    let before = undo_steps(&s);
    log(
        &mut s,
        "var text = app.activeDocument.textFrames[0].textRange;\
         for (var j = 0; j < text.words.length; j++) { var f = text.words[j].characters[0]; f.size = f.size * 1.5; }",
    );
    assert_eq!(sizes(&s, id), vec![("o".into(), 18.0), ("ne ".into(), 12.0), ("t".into(), 18.0), ("wo".into(), 12.0)]);
    assert_eq!(undo_steps(&s), before + 1);
}

#[test]
fn ranges_follow_model_edits_and_go_stale_after_other_edits() {
    let (mut s, id) = doc_with("one two three");
    let out = log(
        &mut s,
        "var t = app.activeDocument.textFrames[0], w = t.words, second = w[1], third = w[2];\
         w[0].contents = 'first'; vc.log([second.contents, second.start, third.contents]);\
         second.contents = ''; vc.log([second.length, third.contents, t.contents]);",
    );
    assert_eq!(out, vec![json!(["two", 6, "three"]), json!([0, "three", "first  three"])]);
    let source = format!("var r = app.activeDocument.textFrames[0].words[0]; vc.run('text.setText', {{id: {}, text: 'other'}}); r.contents;", id.0);
    let e = failure(&mut s, &source);
    assert!(e.contains("Stale text range"), "{e}");
    let e = failure(&mut s, "var t = app.activeDocument.textFrames[0], r = t.textRange; t.remove(); r.contents;");
    assert!(e.contains("Stale text range"), "{e}");
    // A change reversed before the range is read leaves the text the range knew.
    let (mut s, id) = doc_with("one two");
    let source = format!(
        "var r = app.activeDocument.textFrames[0].words[1]; vc.run('text.setText', {{id: {0}, text: 'other'}});\
         vc.run('text.setText', {{id: {0}, text: 'one two'}}); vc.log([r.contents, r.start]);",
        id.0
    );
    assert_eq!(log(&mut s, &source), vec![json!(["two", 4])]);
}

#[test]
fn ranges_made_before_file_revert_go_stale_even_when_the_text_is_the_same() {
    let (mut s, _) = doc_with("one two");
    let path = std::env::temp_dir().join(format!("vc-text-revert-{}.vectorcraft", std::process::id()));
    s.execute("document.save", &json!({"path": path.to_string_lossy()})).unwrap();
    let out = log(
        &mut s,
        "var t = app.activeDocument.textFrames[0], r = t.words[1];\
         vc.run('shape.rectangle', {x: 1, y: 1, width: 5, height: 5}); vc.run('file.revert', {});\
         var now = app.activeDocument.textFrames[0];\
         vc.log(now.contents); try { vc.log(r.contents); } catch (e) { vc.log(e.message); }\
         vc.log(now.words[1].contents);",
    );
    let _ = std::fs::remove_file(&path);
    assert_eq!(out[0], json!("one two"), "the reverted file holds the same text");
    assert!(out[1].as_str().is_some_and(|m| m.contains("Stale text range")), "{out:?}");
    assert_eq!(out[2], json!("two"), "a range made after the revert reads the reverted text");
}

#[test]
fn a_range_an_edit_covers_holds_the_edits_new_text() {
    let (mut s, _) = doc_with("abc");
    let out = log(
        &mut s,
        "var t = app.activeDocument.textFrames[0], first = t.characters[0], r = t.characters[1];\
         t.characters[2].contents = 'ZZ'; vc.log([first.contents, r.contents]);\
         t.contents = 'XY'; vc.log([r.contents, r.start, r.length, first.contents]);",
    );
    // A range that ends where an edit starts keeps its end; one inside the edit takes the new text.
    assert_eq!(out, vec![json!(["a", "b"]), json!(["XY", 0, 2, "XY"])]);
}

#[test]
fn bad_attribute_values_are_refused_without_a_change() {
    let (mut s, id) = doc_with("abc");
    let before = undo_steps(&s);
    for (src, want) in [
        ("r.size = 0;", "size must be from 0.1"),
        ("r.size = 'big';", "size must be a finite number"),
        ("r.size = NaN;", "not a finite number"),
        ("r.leading = 0;", "leading must be from 0.1"),
        ("r.characterAttributes.tracking = 1e9;", "tracking"),
        ("r.underline = 1;", "underline must be true or false"),
        ("r.textFont = 'Arial';", "textFont must be a TextFont"),
        ("r.paragraphAttributes.justification = 'LEFT';", "Justification value"),
        ("r.paragraphAttributes.leftIndent = 5000;", "leftIndent"),
        ("r.length = 1;", "cannot be set yet"),
    ] {
        let e = failure(&mut s, &format!("var r = app.activeDocument.textFrames[0].textRange; {src}"));
        assert!(e.contains(want), "{src}: {e}");
    }
    assert_eq!(undo_steps(&s), before);
    assert_eq!(sizes(&s, id), vec![("abc".into(), 12.0)]);
}

#[test]
fn setting_the_value_a_range_has_records_no_undo_step() {
    let (mut s, _) = doc_with("abc");
    let before = undo_steps(&s);
    log(
        &mut s,
        "var t = app.activeDocument.textFrames[0], r = t.textRange; r.size = r.size; r.underline = false;\
         t.contents = t.contents; r.paragraphAttributes.leftIndent = 0; r.characters[0].characterAttributes.size = 12;",
    );
    assert_eq!(undo_steps(&s), before);
}

#[test]
fn a_leading_equal_to_the_auto_leading_turns_auto_leading_off() {
    let (mut s, id) = doc_with("abc");
    let before = undo_steps(&s);
    let out = log(&mut s, "var r = app.activeDocument.textFrames[0].textRange; r.leading = r.leading; vc.log([r.autoLeading, r.leading]);");
    assert_eq!(out, vec![json!([false, 14.4])]);
    assert_eq!(undo_steps(&s), before + 1);
    match &s.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => assert!(t.runs.iter().all(|r| r.style.leading.is_some_and(|l| (l - 14.4).abs() < 1e-9)), "{:?}", t.runs),
        _ => panic!("not text"),
    }
    // A fixed leading set to its own value is still no change.
    let out = log(&mut s, "var r = app.activeDocument.textFrames[0].textRange; r.leading = 14.4; vc.log(r.autoLeading);");
    assert_eq!(out, vec![json!(false)]);
    assert_eq!(undo_steps(&s), before + 1);
}

#[test]
fn paragraph_attributes_set_the_paragraphs_a_range_touches() {
    let (mut s, _) = doc_with("first\nsecond");
    let out = log(
        &mut s,
        "var t = app.activeDocument.textFrames[0], p = t.paragraphs;\
         vc.log([p.length, p[0].contents, p[1].contents]);\
         p[1].paragraphAttributes.justification = Justification.CENTER; p[1].paragraphAttributes.leftIndent = 9;\
         vc.log([p[0].paragraphAttributes.justification, p[1].paragraphAttributes.justification]);\
         vc.log([p[0].paragraphAttributes.leftIndent, p[1].paragraphAttributes.leftIndent, p[1].getParagraphLength()]);",
    );
    assert_eq!(out[0], json!([2, "first\r", "second"]));
    assert_eq!(out[1], json!(["Justification.LEFT", "Justification.CENTER"]));
    assert_eq!(out[2], json!([0, 9, 6]));
    // A paragraph whose range ends after its break sets only itself.
    let out = log(
        &mut s,
        "var p = app.activeDocument.textFrames[0].paragraphs; p[0].paragraphAttributes.justification = Justification.RIGHT;\
         p[0].paragraphAttributes.spaceAfter = 4;\
         vc.log([p[0].justification === undefined, p[0].paragraphAttributes.justification, p[1].paragraphAttributes.justification]);\
         vc.log([p[0].paragraphAttributes.spaceAfter, p[1].paragraphAttributes.spaceAfter]);",
    );
    assert_eq!(out, vec![json!([true, "Justification.RIGHT", "Justification.CENTER"]), json!([4, 0])]);
    // The empty paragraph after a final break belongs only to an empty range at the end.
    let (mut s, id) = doc_with("a\n");
    let out = log(
        &mut s,
        "var t = app.activeDocument.textFrames[0], p = t.paragraphs; p[0].paragraphAttributes.spaceAfter = 4;\
         vc.log([p.length, p[0].paragraphs.length, p[0].paragraphAttributes.spaceAfter]);",
    );
    assert_eq!(out, vec![json!([1, 1, 4])]);
    let after: Vec<f64> = match &s.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (0..2).map(|i| t.para_at(i).space_after).collect(),
        _ => panic!("not text"),
    };
    assert_eq!(after, vec![4.0, 0.0]);
}

#[test]
fn auto_leading_reads_the_paragraph_of_the_range_inside_one_run() {
    let (mut s, id) = doc_with("first\nsecond");
    s.execute("text.setFormat", &json!({"id": id.0, "start": 6, "end": 6, "autoLeading": 200})).unwrap();
    let out = log(
        &mut s,
        "var p = app.activeDocument.textFrames[0].paragraphs; vc.log([p[0].leading, p[1].leading, p[1].characters[0].leading]);\
         p[1].autoLeading = false; vc.log([p[0].autoLeading, p[1].autoLeading, p[1].leading]);",
    );
    assert_eq!(out, vec![json!([14.4, 24, 24]), json!([true, false, 24])]);
    let leadings: Vec<(String, Option<f64>)> = match &s.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().map(|r| (r.text.clone(), r.style.leading)).collect(),
        _ => panic!("not text"),
    };
    assert_eq!(leadings, vec![("first\n".into(), None), ("second".into(), Some(24.0))]);
}

#[test]
fn a_font_that_missing_glyph_protection_keeps_off_some_characters_throws() {
    let (from, to, c) = ("Source Sans 3", "JetBrains Mono", '\u{180}');
    let db = vectorcraft_text::FontDb::global();
    let covers = |f: &str| db.face(f, "Regular").is_some_and(|face| face.covers(c));
    if !covers(from) || covers(to) {
        return; // Installed fonts of these names cover other characters.
    }
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 50, "text": "a\u{180}c", "font": from})).unwrap();
    let pick = format!("var r = app.activeDocument.textFrames[0].textRange, f = app.textFonts.getByName('{to} Regular');");
    let e = failure(&mut s, &format!("{pick} r.textFont = f;"));
    assert!(e.contains("textFont: the font has no glyph for 1 character, which keeps its font"), "{e}");
    // Characters the font can draw take it without an error.
    let out = log(&mut s, &format!("{pick} r.characters[0].textFont = f; vc.log(r.characters[0].textFont.family);"));
    assert_eq!(out, vec![json!(to)]);
}

#[test]
fn turning_auto_leading_off_keeps_each_characters_leading() {
    let (mut s, id) = doc_with("ab\ncd");
    s.execute("text.setFormat", &json!({"id": id.0, "start": 3, "end": 3, "autoLeading": 200})).unwrap();
    s.execute("text.setRangeStyle", &json!({"id": id.0, "start": 1, "end": 2, "size": 24})).unwrap();
    let before = undo_steps(&s);
    let out = log(&mut s, "var r = app.activeDocument.textFrames[0].textRange; r.autoLeading = false; vc.log(r.autoLeading);");
    assert_eq!(out, vec![json!(false)]);
    assert_eq!(undo_steps(&s), before + 1);
    let leadings: Vec<(String, Option<i64>)> = match &s.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => t.runs.iter().map(|r| (r.text.clone(), r.style.leading.map(|l| (l * 1000.0).round() as i64))).collect(),
        _ => panic!("not text"),
    };
    let want = [("a", 14_400), ("b", 28_800), ("\n", 14_400), ("cd", 24_000)];
    assert_eq!(leadings, want.iter().map(|(t, l)| (t.to_string(), Some(*l))).collect::<Vec<_>>());
    // Paragraphs and runs with the same leading take one command.
    let (mut s, _) = doc_with("one\ntwo\nthree");
    let r = run(&mut s, "app.activeDocument.textFrames[0].textRange.autoLeading = false;").unwrap();
    let calls: Vec<(Value, Value)> = r["slowestCommands"].as_array().unwrap().iter().map(|c| (c["id"].clone(), c["calls"].clone())).collect();
    assert_eq!(calls, vec![(json!("text.setRangeStyle"), json!(1))]);
    // Auto leading that a fixed leading cannot hold is refused before anything changes.
    let (mut s, id) = doc_with("ab\ncd");
    s.execute("text.setFormat", &json!({"id": id.0, "start": 3, "end": 3, "autoLeading": 0})).unwrap();
    let before = undo_steps(&s);
    let e = failure(&mut s, "app.activeDocument.textFrames[0].textRange.autoLeading = false;");
    assert!(e.contains("autoLeading cannot be turned off"), "{e}");
    assert_eq!(undo_steps(&s), before);
    match &s.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => assert!(t.runs.iter().all(|r| r.style.leading.is_none())),
        _ => panic!("not text"),
    }
}

#[test]
fn an_explicit_alignment_replaces_auto_alignment() {
    let justify = |s: &Session, id: NodeId| match &s.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => t.para_at(0).justify,
        _ => panic!("not text"),
    };
    for (text, reads) in [("abc", "Justification.LEFT"), ("\u{5e9}\u{5dc}\u{5d5}\u{5dd}", "Justification.RIGHT")] {
        let (mut s, id) = doc_with(text);
        assert_eq!(justify(&s, id), Justify::Auto);
        let before = undo_steps(&s);
        let out = log(
            &mut s,
            "var a = app.activeDocument.textFrames[0].textRange.paragraphAttributes; vc.log(a.justification);\
             a.justification = Justification.LEFT; vc.log(a.justification);",
        );
        assert_eq!(out, vec![json!(reads), json!("Justification.LEFT")], "{text}");
        assert_eq!(justify(&s, id), Justify::Left, "{text}");
        assert_eq!(undo_steps(&s), before + 1, "{text}");
    }
    // Each paragraph on Auto reads by its own text.
    let (mut s, _) = doc_with("abc\n\u{5e9}\u{5dc}\u{5d5}\u{5dd}\nd");
    let out = log(
        &mut s,
        "var p = app.activeDocument.textFrames[0].paragraphs; var j = function (i) { return p[i].paragraphAttributes.justification; }; vc.log([j(0), j(1), j(2)]);",
    );
    assert_eq!(out, vec![json!(["Justification.LEFT", "Justification.RIGHT", "Justification.LEFT"])]);
}

#[test]
fn a_stale_range_has_no_parent() {
    let (mut s, id) = doc_with("abc");
    let source = format!("var r = app.activeDocument.textFrames[0].textRange; vc.run('text.setText', {{id: {}, text: 'x'}}); r.parent;", id.0);
    let e = failure(&mut s, &source);
    assert!(e.contains("Stale text range"), "{e}");
}

#[test]
fn a_stale_range_and_its_collections_have_no_typename() {
    let (mut s, id) = doc_with("abc");
    let source = format!(
        "var r = app.activeDocument.textFrames[0].textRange, all = [r, r.characters, r.words, r.paragraphs, r.characterAttributes, r.paragraphAttributes];\
         var names = []; for (var i = 0; i < all.length; i++) names.push(all[i].typename); vc.log(names);\
         vc.run('text.setText', {{id: {}, text: 'other'}}); var m = [];\
         for (var i = 0; i < all.length; i++) {{ try {{ m.push(all[i].typename); }} catch (e) {{ m.push(e.message); }} }} vc.log(m);",
        id.0
    );
    let out = log(&mut s, &source);
    assert_eq!(out[0], json!(["TextRange", "Characters", "Words", "Paragraphs", "CharacterAttributes", "ParagraphAttributes"]));
    assert_eq!(out[1], json!(vec!["Stale text range"; 6]));
}

#[test]
fn members_not_built_yet_throw_after_the_stale_check() {
    let (mut s, id) = doc_with("abc");
    let e = failure(&mut s, "app.activeDocument.textFrames[0].lines;");
    assert!(e.contains("TextFrame.lines is not available yet"), "{e}");
    let e = failure(&mut s, "var t = app.activeDocument.textFrames[0]; t.remove(); t.lines;");
    assert!(e.contains("Stale page item"), "{e}");
    let e = failure(&mut s, "app.activeDocument.textFrames[0].textRange.lines;");
    assert!(e.contains("TextRange.lines is not available yet"), "{e}");
    for member in ["r.lines", "r.start = 1"] {
        let source =
            format!("var r = app.activeDocument.textFrames[0].textRange; vc.run('text.setText', {{id: {}, text: 'other'}}); {member};", id.0);
        let e = failure(&mut s, &source);
        assert!(e.contains("Stale text range"), "{member}: {e}");
    }
}

#[test]
fn text_fonts_list_and_assign_fonts_by_name() {
    // The last font in the menu that draws "abc", so Missing Glyph Protection keeps no character.
    let db = vectorcraft_text::FontDb::global();
    let listed: Vec<(String, String)> = db.menu_family_list().iter().flat_map(|f| db.styles(f).into_iter().map(move |st| (f.clone(), st))).collect();
    let covers = |(f, st): &&(String, String)| db.face(f, st).is_some_and(|face| "abc".chars().all(|c| face.covers(c)));
    let Some((pos, (family, style))) = listed.iter().enumerate().rev().find(|(_, fs)| covers(fs)) else {
        return; // No installed font draws "abc".
    };
    let (mut s, id) = doc_with("abc");
    let out = log(
        &mut s,
        &format!(
            "var f = app.textFonts, m = '';\
             try {{ f.getByName('No Such Font Anywhere'); }} catch (e) {{ m = e.message; }} vc.log(m);\
             try {{ f.index('No Such Font Anywhere'); }} catch (e) {{ m = e.message; }} vc.log(m);\
             vc.log(f.length > 0); var r = app.activeDocument.textFrames[0].textRange, k = f[{pos}];\
             vc.log([k.family, k.style]); r.textFont = k; vc.log(r.textFont.family === k.family && r.textFont.style === k.style);\
             vc.log(f.getByName(k.family + ' ' + k.style).family === k.family); vc.log(f[f.length] === undefined);"
        ),
    );
    assert_eq!(out, vec![json!("Not found"), json!("Not found"), json!(true), json!([family, style]), json!(true), json!(true), json!(true)]);
    let set = match &s.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (t.runs[0].style.font_family.clone(), t.runs[0].style.font_style.clone()),
        _ => panic!("not text"),
    };
    assert_eq!(&set, &(family.clone(), style.clone()));
}

#[test]
fn create_outline_and_conversions_return_the_new_objects() {
    let (mut s, id) = doc_with("Hi");
    let out = log(
        &mut s,
        "var t = app.activeDocument.textFrames[0], m = '';\
         try { t.convertAreaObjectToPointObject(); } catch (e) { m = e.message; } vc.log(m);\
         var a = t.convertPointObjectToAreaObject(); vc.log(a.kind);\
         var p = a.convertAreaObjectToPointObject(); vc.log(p.kind); vc.log(p.contents);\
         var g = p.createOutline(); vc.log(g.typename); vc.log(app.activeDocument.textFrames.length);",
    );
    assert_eq!(out[0], json!("the text frame is not area type"));
    assert_eq!(out[1..], [json!("TextType.AREATEXT"), json!("TextType.POINTTEXT"), json!("Hi"), json!("GroupItem"), json!(0)]);
    assert!(s.active().unwrap().doc.node(id).is_none());
}

#[test]
fn outlining_type_with_nothing_to_outline_gives_null() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let out =
        log(&mut s, "var d = app.activeDocument, t = d.textFrames.add(); var g = t.createOutline(); vc.log([g === null, d.textFrames.length]);");
    assert_eq!(out, vec![json!([true, 0])]);
}

#[test]
fn type_made_with_a_large_id_is_the_new_type() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // The second frame's id, 2^53 + 1, is not exact as a number.
    let big = 1u64 << 53;
    s.active_mut().unwrap().change_unsaved(|d| std::sync::Arc::make_mut(d).raise_next_id(big));
    let out = log(
        &mut s,
        "var d = app.activeDocument, a = d.textFrames.add(), b = d.textFrames.pointText([5, 5]); b.contents = 'second';\
         vc.log([a === b, a.contents, b.createOutline().typename]);",
    );
    assert_eq!(out, vec![json!([false, "", "GroupItem"])]);
    assert_eq!(plain(&s, NodeId(big)), "");
}

#[test]
fn text_members_refuse_objects_of_other_classes() {
    let (mut s, _) = doc_with("abc");
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let e = failure(&mut s, "Object.getOwnPropertyDescriptor(TextFrame.prototype, 'contents').get.call(app.activeDocument.pageItems[0]);");
    assert!(e.contains("this is not a TextFrame"), "{e}");
    let e = failure(&mut s, "Object.getOwnPropertyDescriptor(TextRange.prototype, 'contents').get.call(app.activeDocument.textFrames[0]);");
    assert!(e.contains("this is not a TextRange"), "{e}");
    let e = failure(&mut s, "var f = {}; f['\\u0000hostRef'] = ['TextFont', 0, 0]; app.activeDocument.textFrames[0].textRange.textFont = f;");
    assert!(e.contains("not plain data"), "{e}");
    // A font from an earlier run of a named engine is stale.
    run(&mut s, "#targetengine keepfont\nvar held = app.textFonts[0];").unwrap();
    for member in ["held.family", "held.parent"] {
        let e = failure(&mut s, &format!("#targetengine keepfont\n{member};"));
        assert!(e.contains("Stale text font"), "{member}: {e}");
    }
}
