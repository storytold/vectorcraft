//! Window → SVG Interactivity (`svgInteractivity.*`): the events objects run JavaScript on and the
//! JavaScript files the document links, with what SVG export and the native format keep of them.

use serde_json::{Value, json};

use crate::tests_links::{Folder, save, write};
use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 10, "width": 30, "height": 30}))["id"].as_u64().unwrap())
}

fn events(s: &Session, id: NodeId) -> Vec<(String, String)> {
    s.doc().unwrap().doc.node(id).unwrap().extras().attrs.as_deref().map(|a| a.events.clone()).unwrap_or_default()
}

fn pair(e: &str, js: &str) -> (String, String) {
    (e.to_string(), js.to_string())
}

#[test]
fn events_are_added_replaced_removed_and_cleared_in_one_undo_step_each() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    assert_eq!(run(&mut s, "svgInteractivity.info", json!({}))["events"], json!([]));
    assert_eq!(run(&mut s, "svgInteractivity.setEvent", json!({"event": "onclick", "script": " alert(1) "}))["changed"], 1);
    run(&mut s, "svgInteractivity.setEvent", json!({"event": "OnMouseOver", "script": "hover()"}));
    assert_eq!(events(&s, a), [pair("onclick", "alert(1)"), pair("onmouseover", "hover()")], "in order, names in lower case, scripts trimmed");
    // Setting an event again replaces its script where it is.
    run(&mut s, "svgInteractivity.setEvent", json!({"event": "onclick", "script": "go()"}));
    assert_eq!(events(&s, a), [pair("onclick", "go()"), pair("onmouseover", "hover()")]);
    let i = run(&mut s, "svgInteractivity.info", json!({}));
    assert_eq!(i["events"], json!([{"event": "onclick", "script": "go()"}, {"event": "onmouseover", "script": "hover()"}]));
    assert_eq!(i["eventNames"].as_array().unwrap().len(), 19);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(events(&s, a)[0], pair("onclick", "alert(1)"), "one step");
    run(&mut s, "edit.redo", json!({}));
    // Remove Selected Entry, and an empty script, remove one event.
    run(&mut s, "svgInteractivity.removeEvent", json!({"event": "onclick"}));
    assert_eq!(events(&s, a), [pair("onmouseover", "hover()")]);
    run(&mut s, "svgInteractivity.setEvent", json!({"event": "onmouseover", "script": ""}));
    assert!(s.doc().unwrap().doc.node(a).unwrap().extras().attrs.is_none(), "no events left: no attributes");
    // Clear Events removes all of them.
    run(&mut s, "svgInteractivity.setEvent", json!({"event": "onload", "script": "init()"}));
    run(&mut s, "svgInteractivity.setEvent", json!({"event": "onkeyup", "script": "key()"}));
    assert_eq!(run(&mut s, "svgInteractivity.clearEvents", json!({}))["changed"], 1);
    assert!(events(&s, a).is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(events(&s, a).len(), 2, "one step");
    // Nothing to change: no undo step.
    let steps = s.doc().unwrap().history.undo.len();
    assert_eq!(run(&mut s, "svgInteractivity.removeEvent", json!({"event": "onclick"}))["changed"], 0);
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);
}

#[test]
fn info_shows_no_events_where_the_objects_differ() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 10.0), rect(&mut s, 50.0));
    run(&mut s, "svgInteractivity.setEvent", json!({"ids": [a.0], "event": "onclick", "script": "a()"}));
    assert!(events(&s, b).is_empty());
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    assert_eq!(run(&mut s, "svgInteractivity.info", json!({}))["events"], Value::Null);
    // Given to both, the event goes on both.
    assert_eq!(run(&mut s, "svgInteractivity.setEvent", json!({"event": "onclick", "script": "both()"}))["changed"], 2);
    assert_eq!(run(&mut s, "svgInteractivity.info", json!({}))["events"], json!([{"event": "onclick", "script": "both()"}]));
}

#[test]
fn bad_events_and_scripts_are_refused() {
    let mut s = session();
    rect(&mut s, 10.0);
    for p in [
        json!({"script": "x()"}),
        json!({"event": "onfoo", "script": "x()"}),
        json!({"event": "onclick"}),
        json!({"event": "onclick", "script": 3}),
        json!({"event": "onclick", "script": "x()", "ids": "1"}),
    ] {
        assert!(s.execute("svgInteractivity.setEvent", &p).is_err(), "{p}");
    }
    run(&mut s, "select.none", json!({}));
    assert!(s.execute("svgInteractivity.setEvent", &json!({"event": "onclick", "script": "x()"})).is_err(), "nothing selected");
    assert!(s.execute("svgInteractivity.clearEvents", &json!({})).is_err(), "nothing selected");
}

#[test]
fn javascript_files_are_linked_once_and_unlinked() {
    let mut s = session();
    let files = |s: &mut Session| run(s, "svgInteractivity.info", json!({}))["files"].clone();
    assert_eq!(run(&mut s, "svgInteractivity.addFile", json!({"file": " menu.js "}))["files"], json!(["menu.js"]));
    run(&mut s, "svgInteractivity.addFile", json!({"file": "https://example.com/a.js"}));
    let steps = s.doc().unwrap().history.undo.len();
    run(&mut s, "svgInteractivity.addFile", json!({"file": "menu.js"}));
    assert_eq!(files(&mut s), json!(["menu.js", "https://example.com/a.js"]), "a file already linked stays where it is");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "and makes no undo step");
    assert!(s.execute("svgInteractivity.addFile", &json!({"file": "  "})).is_err());
    assert!(s.execute("svgInteractivity.removeFile", &json!({"file": "other.js"})).is_err());
    assert_eq!(run(&mut s, "svgInteractivity.removeFile", json!({"file": "menu.js"}))["files"], json!(["https://example.com/a.js"]));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(files(&mut s), json!(["menu.js", "https://example.com/a.js"]));
}

/// `cmd` with `p` for a caller whose read roots are `read` and whose write roots are `write`.
fn confined(s: &mut Session, read: &[&Folder], write: &[&Folder], cmd: &str, p: Value) -> std::result::Result<Value, String> {
    let read: Vec<&std::path::Path> = read.iter().map(|f| f.0.as_path()).collect();
    let write: Vec<&std::path::Path> = write.iter().map(|f| f.0.as_path()).collect();
    let roots = crate::file_access::AutomationRoots::new(&read, &write).unwrap();
    crate::file_access::confine(roots.as_ref(), || s.execute(cmd, &p)).map_err(|e| e.to_string())
}

/// `info`, `addFile` and `removeFile` for a caller with read roots name a linked file outside them
/// by its file name, a relative one taken from the document's folder; a file inside them and a URL
/// are named as they are, and the document keeps every path.
#[test]
fn javascript_file_replies_hide_paths_outside_read_roots() {
    let (away, home) = (Folder::new("js-replies-away"), Folder::new("js-replies-home"));
    let (outside, inside) = (away.file("menu.js"), home.file("in.js"));
    for f in [&outside, &inside, &away.file("lib/rel.js")] {
        write(f, b"void 0;");
    }
    let url = "https://example.com/a.js";
    let mut s = session();
    for f in [outside.as_str(), inside.as_str(), url, "lib/rel.js"] {
        run(&mut s, "svgInteractivity.addFile", json!({ "file": f }));
    }
    save(&mut s, &away.file("poster.vectorcraft"));
    let stored = s.doc().unwrap().doc.svg_scripts.clone();
    let shown = json!(["menu.js", inside, url, "rel.js"]);
    let info = confined(&mut s, &[&home], &[], "svgInteractivity.info", json!({})).unwrap();
    assert_eq!(info["files"], shown, "{info}");
    let added = confined(&mut s, &[&home], &[], "svgInteractivity.addFile", json!({"file": "https://example.com/b.js"})).unwrap();
    assert_eq!(added["files"], json!(["menu.js", inside, url, "rel.js", "https://example.com/b.js"]), "{added}");
    let removed = confined(&mut s, &[&home], &[], "svgInteractivity.removeFile", json!({"file": "https://example.com/b.js"})).unwrap();
    assert_eq!(removed["files"], shown, "{removed}");
    assert!(!format!("{info}{added}{removed}").contains("js-replies-away"));
    // A caller with only write roots reads none of the paths.
    let info = confined(&mut s, &[], &[&home], "svgInteractivity.info", json!({})).unwrap();
    assert_eq!(info["files"], json!(["menu.js", "in.js", url, "rel.js"]), "{info}");
    // A caller whose read roots hold the files, and one without roots, sees every path.
    assert_eq!(confined(&mut s, &[&away, &home], &[], "svgInteractivity.info", json!({})).unwrap()["files"], json!(stored));
    assert_eq!(run(&mut s, "svgInteractivity.info", json!({}))["files"], json!(stored));
    assert_eq!(s.doc().unwrap().doc.svg_scripts, stored, "the document keeps its paths");
}

/// A caller with read roots unlinks a file a reply names by its file name alone by that name, as
/// one undo step; the name is not taken for a file it may read, nor without read roots.
#[test]
fn hidden_javascript_file_can_be_removed_by_unique_shown_name() {
    let (away, home) = (Folder::new("js-remove-away"), Folder::new("js-remove-home"));
    let (outside, inside) = (away.file("menu.js"), home.file("other.js"));
    let mut s = session();
    for f in [&outside, &inside] {
        run(&mut s, "svgInteractivity.addFile", json!({ "file": f }));
    }
    assert!(s.execute("svgInteractivity.removeFile", &json!({"file": "menu.js"})).is_err(), "no name without read roots");
    let e = confined(&mut s, &[&home], &[], "svgInteractivity.removeFile", json!({"file": "other.js"})).unwrap_err();
    assert!(e.contains("\"other.js\" isn't linked"), "a file inside the roots is unlinked by its path: {e}");
    let steps = s.doc().unwrap().history.undo.len();
    let r = confined(&mut s, &[&home], &[], "svgInteractivity.removeFile", json!({"file": "menu.js"})).unwrap();
    assert_eq!(r["files"], json!([inside]), "{r}");
    assert_eq!(s.doc().unwrap().doc.svg_scripts, [inside.as_str()]);
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().doc.svg_scripts, [outside.as_str(), inside.as_str()]);
    // A caller that knows the whole path unlinks the file by it.
    let r = confined(&mut s, &[&home], &[], "svgInteractivity.removeFile", json!({ "file": outside })).unwrap();
    assert_eq!(r["files"], json!([inside]), "{r}");
}

/// Two linked files outside the read roots with the same file name: that name unlinks neither,
/// and the error names no path.
#[test]
fn ambiguous_hidden_javascript_file_name_is_refused_without_edit() {
    let (away, home) = (Folder::new("js-ambiguous-away"), Folder::new("js-ambiguous-home"));
    let (a, b) = (away.file("a/menu.js"), away.file("b/menu.js"));
    let mut s = session();
    for f in [&a, &b] {
        run(&mut s, "svgInteractivity.addFile", json!({ "file": f }));
    }
    let steps = s.doc().unwrap().history.undo.len();
    let e = confined(&mut s, &[&home], &[], "svgInteractivity.removeFile", json!({"file": "menu.js"})).unwrap_err();
    assert!(e.contains("\"menu.js\" names more than one linked JavaScript file") && !e.contains("js-ambiguous-away"), "{e}");
    assert_eq!(s.doc().unwrap().doc.svg_scripts, [a.as_str(), b.as_str()]);
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "no undo step");
    let e = confined(&mut s, &[&home], &[], "svgInteractivity.removeFile", json!({"file": "nav.js"})).unwrap_err();
    assert!(e.contains("\"nav.js\" isn't linked") && !e.contains("js-ambiguous-away"), "{e}");
    // A file linked as the name itself, beside one outside the roots shown by that name: neither.
    run(&mut s, "svgInteractivity.removeFile", json!({ "file": b }));
    run(&mut s, "svgInteractivity.addFile", json!({"file": "menu.js"}));
    let e = confined(&mut s, &[&home], &[], "svgInteractivity.removeFile", json!({"file": "menu.js"})).unwrap_err();
    assert!(e.contains("names more than one"), "{e}");
    assert_eq!(s.doc().unwrap().doc.svg_scripts, [a.as_str(), "menu.js"]);
    // Without read roots the name is the file linked as it.
    run(&mut s, "svgInteractivity.removeFile", json!({"file": "menu.js"}));
    assert_eq!(s.doc().unwrap().doc.svg_scripts, [a.as_str()]);
}

/// `addFile` takes a file the same text links as already linked: a file name that a reply shows for
/// a different file is linked as one more file, within the 64-file limit.
#[test]
fn adding_a_shown_name_does_not_deduplicate_a_different_hidden_path() {
    let (away, home) = (Folder::new("js-add-away"), Folder::new("js-add-home"));
    let mut s = session();
    let outside = away.file("0.js");
    run(&mut s, "svgInteractivity.addFile", json!({ "file": outside }));
    let r = confined(&mut s, &[&home], &[], "svgInteractivity.addFile", json!({"file": "0.js"})).unwrap();
    assert_eq!(r["files"], json!(["0.js", "0.js"]), "{r}");
    assert_eq!(s.doc().unwrap().doc.svg_scripts, [outside.as_str(), "0.js"]);
    run(&mut s, "edit.undo", json!({}));
    for i in 1..64 {
        run(&mut s, "svgInteractivity.addFile", json!({ "file": away.file(&format!("{i}.js")) }));
    }
    let e = confined(&mut s, &[&home], &[], "svgInteractivity.addFile", json!({"file": "0.js"})).unwrap_err();
    assert!(e.contains("at most 64"), "a 65th file is refused: {e}");
    let steps = s.doc().unwrap().history.undo.len();
    let r = confined(&mut s, &[&home], &[], "svgInteractivity.addFile", json!({ "file": outside })).unwrap();
    assert_eq!(r["files"].as_array().map(Vec::len), Some(64), "the same path is already linked");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);
}

#[test]
fn a_document_links_at_most_64_javascript_files() {
    let mut s = session();
    for i in 0..64 {
        run(&mut s, "svgInteractivity.addFile", json!({"file": format!("{i}.js")}));
    }
    assert!(s.execute("svgInteractivity.addFile", &json!({"file": "more.js"})).is_err(), "a 65th file is refused");
    assert_eq!(run(&mut s, "svgInteractivity.addFile", json!({"file": "0.js"}))["files"].as_array().unwrap().len(), 64, "a linked one isn't");
}

#[test]
fn events_and_files_survive_a_native_round_trip_and_reach_svg() {
    let mut s = session();
    let a = rect(&mut s, 10.0);
    run(&mut s, "svgInteractivity.setEvent", json!({"event": "onclick", "script": "show(\"menu\") && x < 1"}));
    run(&mut s, "svgInteractivity.addFile", json!({"file": "menu.js"}));
    let doc = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert_eq!(back.node(a).unwrap().extras().attrs, doc.node(a).unwrap().extras().attrs);
    assert_eq!(back.svg_scripts, ["menu.js"]);
    let svg = run(&mut s, "document.serialize", json!({"format": "svg"}))["text"].as_str().unwrap().to_string();
    assert!(svg.contains(" onclick=\"show(&quot;menu&quot;) &amp;&amp; x &lt; 1\""), "{svg}");
    assert!(svg.contains("<script xlink:href=\"menu.js\"/>"), "{svg}");
}
