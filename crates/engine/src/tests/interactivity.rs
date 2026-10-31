//! Window → SVG Interactivity (`svgInteractivity.*`): the events objects run JavaScript on and the
//! JavaScript files the document links, with what SVG export and the native format keep of them.

use serde_json::{Value, json};

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
