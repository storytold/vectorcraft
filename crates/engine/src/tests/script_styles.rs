//! Character and paragraph styles in the script object model.

use serde_json::{Value, json};
use vectorcraft_doc::{TextObject, TextStyleDef};

use crate::cmd::textstyles::{NORMAL_CHAR, NORMAL_PARA};
use crate::script::TEST_RESULT_BYTES;
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

fn text_of(s: &Session, id: NodeId) -> TextObject {
    match &s.active().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

/// Each run's text, style name, size and underline.
fn runs(s: &Session, id: NodeId) -> Vec<(String, Option<String>, f64, bool)> {
    text_of(s, id).runs.iter().map(|r| (r.text.clone(), r.style.style_name.clone(), r.style.size, r.style.underline)).collect()
}

fn char_names(s: &Session) -> Vec<String> {
    s.active().unwrap().doc.char_styles.iter().map(|d| d.name.clone()).collect()
}

const DOC: &str = "var d = app.activeDocument, cs = d.characterStyles, ps = d.paragraphStyles, tf = d.textFrames[0];";

#[test]
fn style_collections_list_the_normal_style_first_and_add_find_and_remove() {
    let (mut s, _) = doc_with("Hello");
    let before = undo_steps(&s);
    let out = log(
        &mut s,
        &format!(
            "{DOC} vc.log([cs.length, ps.length, cs[0].name, ps[0].name, cs.typename, cs[0].typename, ps[0].typename]);\
             var b = cs.add('Body'); var p = ps.add('Lead');\
             vc.log([cs.length, cs[1].name, cs.getByName('Body').name, cs.index(1).name, cs.index('Body').name, b.parent.typename]);\
             vc.log([ps.length, ps.getByName('Lead').name, cs[5] === undefined]);\
             cs.add('Other'); cs.removeAll(); ps.removeAll();\
             vc.log([cs.length, ps.length, cs[0].name]);"
        ),
    );
    assert_eq!(out[0], json!([1, 1, NORMAL_CHAR, NORMAL_PARA, "CharacterStyles", "CharacterStyle", "ParagraphStyle"]));
    assert_eq!(out[1], json!([2, "Body", "Body", "Body", "Body", "Document"]));
    assert_eq!(out[2], json!([2, "Lead", true]));
    assert_eq!(out[3], json!([1, 1, NORMAL_CHAR]));
    assert_eq!(undo_steps(&s), before + 1);
    let undo = undo_steps(&s);
    for (src, says) in [
        ("cs.getByName('Nope');", "Not found"),
        ("cs.index(4);", "Not found"),
        ("cs.add('Body'); cs.add('Body');", "already exists"),
        ("cs[0].remove();", "cannot be removed"),
        ("ps[0].name = 'Mine';", "cannot be renamed"),
        ("cs.getByName(5);", "string"),
    ] {
        assert!(failure(&mut s, &format!("{DOC} {src}")).contains(says), "{src}");
    }
    assert_eq!((undo_steps(&s), char_names(&s)), (undo, Vec::<String>::new()));
}

/// Names are trimmed; an empty name, one past 256 characters and one with a control character
/// are refused with nothing changed.
#[test]
fn style_names_are_trimmed_and_checked() {
    let (mut s, _) = doc_with("Hello");
    let out = log(&mut s, &format!("{DOC} vc.log(cs.add('  Lead  ').name); vc.log(cs.add('{}').name.length);", "x".repeat(256)));
    assert_eq!(out, [json!("Lead"), json!(256)]);
    let undo = undo_steps(&s);
    let long = "x".repeat(257);
    for (src, says) in [
        ("cs.add('   ');".to_string(), "empty"),
        (format!("cs.add('{long}');"), "256 characters"),
        ("cs.add('a\\u0007b');".to_string(), "control characters"),
        ("ps.add('tab\\there');".to_string(), "control characters"),
        ("cs.getByName('Lead').name = 'x\\ny';".to_string(), "control characters"),
        ("cs.add(5);".to_string(), "string"),
        (format!("cs.getByName('{}');", "y".repeat(5000)), "too long"),
    ] {
        assert!(failure(&mut s, &format!("{DOC} {src}")).contains(says), "{src}");
    }
    assert_eq!(undo_steps(&s), undo);
    assert_eq!(char_names(&s).len(), 2);
    // Path characters are kept as part of the name; a style name never names a file.
    let out = log(&mut s, &format!("{DOC} vc.log(cs.add('../a/b\\\\c:*?').name);"));
    assert_eq!(out, [json!("../a/b\\c:*?")]);
    assert_eq!(char_names(&s).last().map(String::as_str), Some("../a/b\\c:*?"));
}

/// A style attribute that is not a finite number in range is refused with nothing changed.
#[test]
fn style_attributes_refuse_values_that_are_not_finite_or_in_range() {
    let (mut s, _) = doc_with("Hello");
    log(&mut s, &format!("{DOC} cs.add('X'); ps.add('P');"));
    let undo = undo_steps(&s);
    for (src, says) in [
        ("cs[1].characterAttributes.size = NaN;", "finite number"),
        ("cs[1].characterAttributes.size = Infinity;", "finite number"),
        ("cs[1].characterAttributes.size = 1e9;", "0.1 to 1296"),
        ("cs[1].characterAttributes.leading = 1e300;", "0.1 to 5000"),
        ("cs[1].characterAttributes.tracking = -1e9;", "tracking"),
        ("ps[1].paragraphAttributes.leftIndent = NaN;", "finite"),
        ("ps[1].paragraphAttributes.spaceBefore = 1e12;", "1296"),
    ] {
        assert!(failure(&mut s, &format!("{DOC} {src}")).contains(says), "{src}");
    }
    assert_eq!(undo_steps(&s), undo);
    let attrs = |s: &Session| {
        s.active().unwrap().doc.char_styles.iter().chain(s.active().unwrap().doc.para_styles.iter()).map(|d| d.attrs.len()).sum::<usize>()
    };
    assert_eq!(attrs(&s), 0);
}

/// A style object follows its style through a rename made with it; after the style is removed,
/// with the object or with a command, every member throws, and a new style of that name does not
/// bring it back.
#[test]
fn a_renamed_style_keeps_its_object_and_a_removed_style_is_stale() {
    let (mut s, id) = doc_with("Hello");
    let out = log(
        &mut s,
        &format!(
            "{DOC} var a = cs.add('A'); a.applyTo(tf.textRange); a.name = '  B '; vc.log([a.name, cs[1].name, a === cs.getByName('B')]);\
             a.name = 'B'; vc.log(cs.length);"
        ),
    );
    assert_eq!(out, [json!(["B", "B", true]), json!(2)]);
    assert_eq!(runs(&s, id)[0].1.as_deref(), Some("B"));
    for remove in ["a.remove();", "vc.run('charStyle.delete', {name: 'B'});", "cs.removeAll();"] {
        let e = failure(&mut s, &format!("{DOC} var a = cs.getByName('B'); {remove} cs.add('B'); a.name;"));
        assert!(e.contains("Stale style"), "{remove}: {e}");
    }
    // A style removed and made again by commands is another style.
    let e = failure(
        &mut s,
        &format!(
            "{DOC} var a = cs.getByName('B'); vc.run('charStyle.delete', {{name: 'B'}}); vc.run('charStyle.new', {{name: 'B', attrs: {{}}}}); a.name;"
        ),
    );
    assert!(e.contains("Stale style"), "{e}");
    // So is a style removed and made again by the steps of one batch.
    let e = failure(
        &mut s,
        &format!(
            "{DOC} var a = cs.getByName('B'); vc.run('command.batch', {{commands: [{{command: 'charStyle.delete', params: {{name: 'B'}}}}, \
             {{command: 'charStyle.new', params: {{name: 'B', attrs: {{}}}}}}]}}); a.name;"
        ),
    );
    assert!(e.contains("Stale style"), "{e}");
    // And a style a batch renames and then names back.
    let e = failure(
        &mut s,
        &format!(
            "{DOC} var a = cs.getByName('B'); vc.run('command.batch', {{commands: [{{command: 'charStyle.rename', params: {{name: 'B', to: 'C'}}}}, \
             {{command: 'charStyle.rename', params: {{name: 'C', to: 'B'}}}}]}}); a.name;"
        ),
    );
    assert!(e.contains("Stale style"), "{e}");
    // The attributes object of a removed style throws for its typename too.
    let e =
        failure(&mut s, &format!("{DOC} var t = cs.getByName('B').characterAttributes; vc.log(t.typename); cs.getByName('B').remove(); t.typename;"));
    assert!(e.contains("Stale style"), "{e}");
    // A style renamed by a command is stale too, and its text keeps its look after a removal.
    let e = failure(&mut s, &format!("{DOC} var a = cs.getByName('B'); vc.run('charStyle.rename', {{name: 'B', to: 'C'}}); a.name;"));
    assert!(e.contains("Stale style"), "{e}");
    let e =
        failure(&mut s, &format!("{DOC} var a = cs.getByName('B'); vc.run('charStyle.rename', {{name: 'B', to: 'C'}}); a.characterAttributes.size;"));
    assert!(e.contains("Stale style"), "{e}");
    let size = runs(&s, id)[0].2;
    log(&mut s, &format!("{DOC} var a = cs.getByName('B'); a.characterAttributes.size = 40; a.remove();"));
    assert_eq!(runs(&s, id)[0].1, None);
    assert_ne!(runs(&s, id)[0].2, size);
    assert_eq!(runs(&s, id)[0].2, 40.0);
}

/// The steps of one batch a script runs keep one copy of a document's style lists, however many of
/// them change the lists.
#[test]
fn the_steps_of_a_batch_keep_one_copy_of_the_style_lists() {
    let (mut s, _) = doc_with("Hello");
    let commands: Vec<Value> = (0..20).map(|i| json!({"command": "charStyle.new", "params": {"name": format!("S{i}"), "attrs": {}}})).collect();
    s.script_in_run = true;
    let r = s.execute("command.batch", &json!({"commands": commands}));
    s.script_in_run = false;
    r.unwrap();
    assert_eq!(char_names(&s).len(), 20);
    assert_eq!(crate::script::retained_style_lists(&s), 1);
}

/// A removal or rename whose result cannot cross to the script is put back, and the style's object
/// still names the style after it.
#[test]
fn a_style_change_that_is_put_back_keeps_the_style_object() {
    let (mut s, _) = doc_with("Hello");
    s.execute("charStyle.new", &json!({"name": "B", "attrs": {}})).unwrap();
    TEST_RESULT_BYTES.with(|b| b.set(Some(1)));
    let r = run(
        &mut s,
        &format!(
            "{DOC} var a = cs.getByName('B');\
             try {{ a.remove(); }} catch (e) {{ vc.log(e.name); }}\
             try {{ vc.run('charStyle.delete', {{name: 'B'}}); }} catch (e) {{ vc.log(e.name); }}\
             try {{ a.name = 'C'; }} catch (e) {{ vc.log(e.name); }}\
             vc.log([a.name, a === cs.getByName('B'), cs.length]);"
        ),
    );
    TEST_RESULT_BYTES.with(|b| b.set(None));
    let r = r.unwrap();
    assert_eq!(r["log"], json!(["RangeError", "RangeError", "RangeError", ["B", true, 2]]), "{r}");
    assert!(char_names(&s).contains(&"B".to_string()), "{:?}", char_names(&s));
}

/// A character style styles exactly the range; without clearing overrides the range keeps its
/// own underline, and with them it goes back to the Normal style first.
#[test]
fn apply_to_styles_a_range_and_clearing_overrides_resets_it() {
    let (mut s, id) = doc_with("Hello world");
    let before = undo_steps(&s);
    log(
        &mut s,
        &format!("{DOC} var w = tf.words[0]; w.underline = true; var big = cs.add('Big'); big.characterAttributes.size = 30; big.applyTo(w);"),
    );
    assert_eq!(runs(&s, id), [("Hello".into(), Some("Big".into()), 30.0, true), (" world".into(), None, 12.0, false)]);
    assert_eq!(undo_steps(&s), before + 1);
    log(&mut s, &format!("{DOC} cs.getByName('Big').applyTo(tf.words[0], true);"));
    assert_eq!(runs(&s, id)[0], ("Hello".into(), Some("Big".into()), 30.0, false));
    // A whole frame, and an empty range, which changes nothing.
    log(&mut s, &format!("{DOC} cs.getByName('Big').applyTo(tf); cs[0].applyTo(tf.characters[0]);"));
    assert_eq!(runs(&s, id), [("H".into(), None, 30.0, false), ("ello world".into(), Some("Big".into()), 30.0, false)]);
}

/// A character style applied to an empty frame styles the text set in it later.
#[test]
fn a_character_style_applied_to_an_empty_frame_styles_text_set_later() {
    for target in ["tf", "tf.textRange"] {
        let (mut s, id) = doc_with("");
        log(&mut s, &format!("{DOC} var big = cs.add('Big'); big.characterAttributes.size = 30; big.applyTo({target}); tf.contents = 'Hi';"));
        assert_eq!(runs(&s, id), [("Hi".into(), Some("Big".into()), 30.0, false)], "{target}");
    }
}

/// A paragraph style styles every paragraph the text touches, and only those.
#[test]
fn a_paragraph_style_styles_the_paragraphs_the_text_touches() {
    let (mut s, id) = doc_with("one\ntwo\nthree");
    let out = log(
        &mut s,
        &format!(
            "{DOC} var c = ps.add('Centered'); var a = c.paragraphAttributes;\
             vc.log([a.justification === undefined, a.spaceBefore === undefined, a.typename]);\
             a.justification = Justification.CENTER; a.spaceBefore = 6;\
             vc.log([a.justification, a.spaceBefore]); c.applyTo(tf.paragraphs[1]);\
             vc.log(tf.paragraphs[1].paragraphAttributes.justification);"
        ),
    );
    assert_eq!(out, [json!([true, true, "ParagraphAttributes"]), json!(["Justification.CENTER", 6]), json!("Justification.CENTER")]);
    let t = text_of(&s, id);
    let names: Vec<_> = (0..3).map(|i| t.para_at(i).style_name.clone()).collect();
    assert_eq!(names, [None, Some("Centered".to_string()), None]);
    assert_eq!(t.para_at(1).space_before, 6.0);
    assert_eq!(t.para_at(2).space_before, 0.0);
    let e = failure(&mut s, &format!("{DOC} ps.getByName('Centered').paragraphAttributes.justification = 'middle';"));
    assert!(e.contains("Justification"), "{e}");
    let e = failure(&mut s, &format!("{DOC} ps.getByName('Centered').paragraphAttributes.spaceAfter = 5000;"));
    assert!(e.contains("1296"), "{e}");
}

/// A style's character attributes read `undefined` until set; setting one restyles the text in
/// the style but not a value the text changed itself.
#[test]
fn style_attributes_redefine_the_text_in_the_style() {
    let (mut s, id) = doc_with("ab cd");
    let out = log(
        &mut s,
        &format!(
            "{DOC} var x = cs.add('X'), a = x.characterAttributes;\
             vc.log([a.size === undefined, a.autoLeading === undefined, a.textFont === undefined, a.typename]);\
             a.size = 20; x.applyTo(tf.textRange); tf.words[1].size = 9; a.size = 24; a.autoLeading = true; a.underline = true;\
             vc.log([a.size, a.autoLeading, a.underline]);"
        ),
    );
    assert_eq!(out, [json!([true, true, true, "CharacterAttributes"]), json!([24, true, true])]);
    assert_eq!(runs(&s, id), [("ab ".into(), Some("X".into()), 24.0, true), ("cd".into(), Some("X".into()), 9.0, true)]);
    // Turning auto leading off keeps the leading auto leading gave the style's size.
    let out = log(
        &mut s,
        &format!("{DOC} var a = cs.getByName('X').characterAttributes; a.autoLeading = false; vc.log([a.autoLeading, Math.round(a.leading * 10)]);"),
    );
    assert_eq!(out, [json!([false, 288])]);
    let e = failure(&mut s, &format!("{DOC} cs.getByName('X').characterAttributes.size = 0;"));
    assert!(e.contains("0.1 to 1296"), "{e}");
    let e = failure(&mut s, &format!("{DOC} cs.getByName('X').characterAttributes.textFont = 'Arial';"));
    assert!(e.contains("TextFont"), "{e}");
    let e = failure(&mut s, &format!("{DOC} ps.add('P').characterAttributes;"));
    assert!(e.contains("not available yet"), "{e}");
}

/// A range lists each style its text uses once, in order, with the Normal style for unstyled
/// text, and its lists cannot add or remove styles.
#[test]
fn a_range_lists_the_styles_its_text_uses() {
    let (mut s, _) = doc_with("aa bb cc\ndd");
    let out = log(
        &mut s,
        &format!(
            "{DOC} var x = cs.add('X'), y = cs.add('Y'); y.applyTo(tf.words[2]); x.applyTo(tf.words[1]); x.applyTo(tf.words[3]);\
             ps.add('P').applyTo(tf.paragraphs[1]);\
             function names(l) {{ var o = []; for (var i = 0; i < l.length; i++) o.push(l[i].name); return o; }}\
             vc.log(names(tf.textRange.characterStyles)); vc.log(names(tf.words[1].characterStyles));\
             vc.log(names(tf.textRange.paragraphStyles)); vc.log(names(tf.paragraphs[0].paragraphStyles));\
             vc.log([tf.words[1].characterStyles[0] === x, tf.words[1].characterStyles.getByName('X').name,\
                     tf.textRange.characterStyles.parent.typename]);"
        ),
    );
    assert_eq!(out[0], json!([NORMAL_CHAR, "X", "Y"]));
    assert_eq!(out[1], json!(["X"]));
    assert_eq!(out[2], json!([NORMAL_PARA, "P"]));
    assert_eq!(out[3], json!([NORMAL_PARA]));
    assert_eq!(out[4], json!([true, "X", "TextRange"]));
    for src in ["tf.textRange.characterStyles.add('Z');", "tf.textRange.paragraphStyles.removeAll();"] {
        assert!(failure(&mut s, &format!("{DOC} {src}")).contains("cannot be added or removed"), "{src}");
    }
}

/// `applyTo` refuses what is not text, a `clearingOverrides` that is not a boolean, text of another
/// document, and a range whose text changed, with nothing changed.
#[test]
fn apply_to_refuses_bad_arguments_with_nothing_changed() {
    let (mut s, id) = doc_with("Hello");
    log(&mut s, &format!("{DOC} cs.add('X');"));
    let runs_before = runs(&s, id);
    for (src, says) in [
        ("cs[1].applyTo(5);".to_string(), "TextRange"),
        ("cs[1].applyTo(tf.textRange, 'yes');".to_string(), "clearingOverrides"),
        ("cs[1].applyTo(cs);".to_string(), "TextRange"),
        (format!("var r = tf.textRange; vc.run('text.setText', {{id: {}, text: 'Help'}}); cs[1].applyTo(r);", id.0), "Stale text range"),
        (
            "var o = app.documents.add(); var t = o.textFrames.add(); t.contents = 'Other'; cs[1].applyTo(t.textRange);".to_string(),
            "another document",
        ),
    ] {
        assert!(failure(&mut s, &format!("{DOC} {src}")).contains(says), "{src}");
    }
    assert_eq!(runs(&s, id), runs_before);
}

/// A run names at most 4,096 styles; reading past that throws instead of growing without bound.
#[test]
fn style_objects_made_before_file_revert_go_stale_even_when_the_style_is_back() {
    let (mut s, _) = doc_with("Hello");
    s.execute("charStyle.new", &json!({"name": "B", "attrs": {"size": 30}})).unwrap();
    let path = std::env::temp_dir().join(format!("vc-styles-revert-{}.vectorcraft", std::process::id()));
    s.execute("document.save", &json!({"path": path.to_string_lossy()})).unwrap();
    let out = log(
        &mut s,
        &format!(
            "{DOC} var a = cs.getByName('B');\
             vc.run('shape.rectangle', {{x: 1, y: 1, width: 5, height: 5}}); vc.run('file.revert', {{}});\
             var cs2 = app.activeDocument.characterStyles;\
             try {{ vc.log(a.name); }} catch (e) {{ vc.log(e.message); }}\
             vc.log(cs2.getByName('B').name); vc.log(cs2.getByName('B') === a);"
        ),
    );
    let _ = std::fs::remove_file(&path);
    assert!(out[0].as_str().is_some_and(|m| m.contains("Stale style")), "{out:?}");
    assert_eq!(out[1], json!("B"), "a style object made after the revert names the reverted style");
    assert_eq!(out[2], json!(false));
}

#[test]
fn a_run_names_at_most_4096_styles() {
    let (mut s, _) = doc_with("Hello");
    let mut d = (*s.active().unwrap().doc).clone();
    let defs: Vec<TextStyleDef> = (0..4100).map(|i| TextStyleDef { name: format!("S{i}"), attrs: Default::default() }).collect();
    d.char_styles = defs.into();
    s.doc_mut().unwrap().doc = std::sync::Arc::new(d);
    let out = log(&mut s, &format!("{DOC} for (var i = 0; i < 4096; i++) cs[i]; vc.log(cs[0] === undefined); vc.log(cs.length);"));
    assert_eq!(out, [json!(false), json!(4101)]);
    let e = failure(&mut s, &format!("{DOC} for (var i = 0; i < 4097; i++) cs[i].name;"));
    assert!(e.contains("4096 styles"), "{e}");
    // `add` with every slot taken adds no style; with one slot left it adds the style.
    let out = log(
        &mut s,
        &format!("{DOC} for (var i = 0; i < 4096; i++) cs[i]; try {{ cs.add('New'); }} catch (e) {{ vc.log(e.message); }} vc.log(cs.length);"),
    );
    assert_eq!(out, [json!("a script can name at most 4096 styles in one run"), json!(4101)]);
    let out = log(&mut s, &format!("{DOC} for (var i = 0; i < 4095; i++) cs[i]; vc.log(cs.add('New').name); vc.log(cs.length);"));
    assert_eq!(out, [json!("New"), json!(4102)]);
}

/// A script loads styles from a file only where it may read files; a file outside that is refused.
#[test]
fn a_script_loads_styles_only_from_a_file_it_may_read() {
    let (mut s, _) = doc_with("Hello");
    let dir = std::env::temp_dir().join(format!("vc-script-styles-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Other.vectorcraft");
    let mut other = Session::new();
    other.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    other.execute("charStyle.new", &json!({"name": "Loaded", "attrs": {"size": 30}})).unwrap();
    std::fs::write(&path, vectorcraft_format::save(&other.active().unwrap().doc, false)).unwrap();
    let p = path.to_string_lossy().replace('\\', "/");
    let e = failure(&mut s, &format!("vc.run('charStyle.load', {{path: '{p}'}});"));
    std::fs::remove_dir_all(&dir).ok();
    assert!(e.contains("access denied"), "{e}");
    assert!(char_names(&s).is_empty());
}
