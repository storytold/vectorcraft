//! SVG Interactivity: event attributes (https://www.w3.org/TR/SVG11/script.html#EventAttributes)
//! and `<script xlink:href>` links (https://www.w3.org/TR/SVG11/script.html#ScriptElement), written
//! by export and read back by import as text that nothing runs.

use std::sync::Mutex;

use proptest::prelude::*;
use usvg::roxmltree;
use vectorcraft_doc::{Document, Node};
use vectorcraft_svg::{Encoding, ExportOptions, ImportOptions, Profile, export, export_full, import, import_with};

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="100" height="100">{body}</svg>"#)
}

fn named<'a>(doc: &'a Document, name: &str) -> &'a Node {
    let mut found = None;
    doc.walk(|n| {
        if n.name.as_deref() == Some(name) {
            found = Some(n);
        }
    });
    found.unwrap()
}

fn events(doc: &Document, name: &str) -> Vec<(String, String)> {
    named(doc, name).extras().attrs.as_deref().map(|a| a.events.clone()).unwrap_or_default()
}

fn pair(e: &str, js: &str) -> (String, String) {
    (e.to_string(), js.to_string())
}

/// A rectangle named `shape` with `events`, in a document linking `files`.
fn document(events: &[(&str, &str)], files: &[&str]) -> Document {
    let mut doc = import(&svg(r#"<rect id="shape" width="20" height="20"/>"#)).unwrap();
    let id = named(&doc, "shape").id;
    doc.node_mut(id).unwrap().edit_attrs(|a| a.events = events.iter().map(|(e, js)| pair(e, js)).collect());
    doc.svg_scripts = files.iter().map(|f| f.to_string()).collect();
    doc
}

#[test]
fn events_are_written_as_event_attributes() {
    let doc = document(&[("onclick", "show(\"menu\") && x < 1"), ("onmouseover", "a\nb")], &[]);
    let text = export(&doc, &ExportOptions::default());
    let xml = roxmltree::Document::parse(&text).unwrap();
    let rect = xml.descendants().find(|n| n.attribute("id") == Some("shape")).unwrap();
    assert_eq!(rect.attribute("onclick"), Some("show(\"menu\") && x < 1"), "{text}");
    assert_eq!(rect.attribute("onmouseover"), Some("a\nb"), "a line break stays one: {text}");
}

#[test]
fn unknown_and_repeated_events_and_the_tiny_profile_write_none() {
    let doc = document(&[("onclick", "a()"), ("onclick", "b()"), ("style", "x"), ("onfoo", "y()")], &[]);
    let out = export_full(&doc, &ExportOptions::default(), None);
    assert_eq!(out.svg.matches("onclick=").count(), 1, "{}", out.svg);
    assert!(out.svg.contains("onclick=\"a()\"") && !out.svg.contains("onfoo") && !out.svg.contains("style=\"x\""), "{}", out.svg);
    assert_eq!(out.warnings.len(), 2, "{:?}", out.warnings);
    let tiny = export_full(&doc, &ExportOptions { profile: Profile::Tiny12, ..Default::default() }, None);
    assert!(!tiny.svg.contains("onclick"), "{}", tiny.svg);
    assert!(tiny.warnings.iter().any(|w| w.contains("SVG Tiny 1.2 has no event attributes")), "{:?}", tiny.warnings);
}

#[test]
fn linked_files_are_written_as_script_elements_in_order() {
    let doc = document(&[], &["menu.js", "https://example.com/a.js?x=1&y=2"]);
    let text = export(&doc, &ExportOptions::default());
    let xml = roxmltree::Document::parse(&text).unwrap();
    let xlink = "http://www.w3.org/1999/xlink";
    let links: Vec<_> = xml.descendants().filter(|n| n.tag_name().name() == "script").filter_map(|n| n.attribute((xlink, "href"))).collect();
    assert_eq!(links, ["menu.js", "https://example.com/a.js?x=1&y=2"], "{text}");
    for profile in [Profile::Svg11, Profile::Tiny12] {
        assert!(export(&doc, &ExportOptions { profile, minify: true, ..Default::default() }).contains("<script xlink:href=\"menu.js\"/>"));
    }
}

#[test]
fn import_reads_back_events_and_linked_files() {
    let doc = document(&[("onclick", "show(\"menu\") && x < 1"), ("onload", "init()")], &["menu.js", "lib/b.js"]);
    for minify in [false, true] {
        let back = import(&export(&doc, &ExportOptions { minify, ..Default::default() })).unwrap();
        assert_eq!(events(&back, "shape"), events(&doc, "shape"));
        assert_eq!(back.svg_scripts, doc.svg_scripts);
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, failure_persistence: None, ..ProptestConfig::default() })]

    /// Any script and any file name come back from an export as they went in, in UTF-8 and in
    /// ISO 8859-1 (other characters written as references), minified or not.
    #[test]
    fn any_script_and_file_name_survive_the_round_trip(script in "[\\PC\t\n]{1,40}", file in "\\PC{1,30}", minify: bool, latin1: bool) {
        let doc = document(&[("onclick", script.as_str())], &[file.as_str()]);
        let encoding = if latin1 { Encoding::Latin1 } else { Encoding::Utf8 };
        let out = export_full(&doc, &ExportOptions { minify, encoding, ..Default::default() }, None);
        let back = import(&out.svg).map_err(|e| TestCaseError::fail(format!("{e}\n{}", out.svg)))?;
        prop_assert_eq!(events(&back, "shape"), vec![pair("onclick", &script)]);
        prop_assert_eq!(back.svg_scripts, vec![file]);
    }
}

#[test]
fn import_keeps_events_of_groups_type_and_unnamed_shapes() {
    let doc = import(&svg(
        r#"<g id="menu" onmouseover="open()"><circle r="5" onclick="pick(1)"/></g><text id="label" x="10" y="20" onclick="say()">Hi</text>"#,
    ))
    .unwrap();
    assert_eq!(events(&doc, "menu"), [pair("onmouseover", "open()")]);
    let mut circle = vec![];
    doc.walk(|n| {
        if n.path_data().is_some() {
            circle.extend(n.extras().attrs.as_deref().map(|a| a.events.clone()).unwrap_or_default());
        }
    });
    assert_eq!(circle, [pair("onclick", "pick(1)")]);
    assert_eq!(events(&doc, "label"), [pair("onclick", "say()")]);
}

/// An unnamed group around one object keeps its own events, apart from the object's, and keeps
/// them through an export.
#[test]
fn import_keeps_an_unnamed_group_with_events_apart_from_its_object() {
    let with_events = |doc: &Document| {
        let mut found = vec![];
        doc.walk(|n| {
            let events = n.extras().attrs.as_deref().map(|a| a.events.clone()).unwrap_or_default();
            if !events.is_empty() {
                found.push((n.children().is_some(), events));
            }
        });
        found
    };
    let doc =
        import(&svg(r#"<g onclick="outer()"><rect width="20" height="20" onclick="inner()"/></g><g onmouseover="hover()"><circle r="5"/></g>"#))
            .unwrap();
    let want = [(true, vec![pair("onclick", "outer()")]), (false, vec![pair("onclick", "inner()")]), (true, vec![pair("onmouseover", "hover()")])];
    assert_eq!(with_events(&doc), want);
    assert_eq!(with_events(&import(&export(&doc, &ExportOptions::default())).unwrap()), want);
}

/// The file is untrusted: event attributes and script links are kept as text only. Import reads no
/// script file, runs nothing, keeps no inline script and no attribute the panel doesn't list.
#[test]
fn import_never_reads_or_runs_scripts() {
    let asked = Mutex::new(Vec::<String>::new());
    let read = |p: &str| -> Option<(Vec<u8>, vectorcraft_doc::LinkInfo)> {
        asked.lock().unwrap().push(p.to_string());
        None
    };
    let text = svg(r#"<script xlink:href="evil.js"/><script href="evil.js"/><script type="text/ecmascript">document.title = "x";</script>
        <image xlink:href="photo.png" width="10" height="10"/>
        <rect id="shape" width="20" height="20" onclick="fetch('https://example.com')" onfoo="x()" data-a="1"/>"#);
    let (doc, warnings) = import_with(&text, &ImportOptions { folder: Some("/tmp"), read: Some(&read) }).unwrap();
    let asked = asked.lock().unwrap().clone();
    assert!(asked.iter().any(|p| p.contains("photo.png")) && !asked.iter().any(|p| p.contains("evil")), "read {asked:?}");
    assert_eq!(doc.svg_scripts, ["evil.js"], "each link once, the inline script left out");
    assert!(warnings.iter().any(|w| w.contains("inline scripts were left out")), "{warnings:?}");
    assert_eq!(events(&doc, "shape"), [pair("onclick", "fetch('https://example.com')")]);
    assert_eq!(doc.title, "Untitled-1", "the inline script didn't run");
}
/// Import keeps the first 64 different `<script>` links and warns about the ones after them.
#[test]
fn import_keeps_the_first_64_linked_files_and_warns_about_the_rest() {
    let links = |n: usize| (0..n).map(|i| format!(r#"<script xlink:href="{i}.js"/>"#)).collect::<String>();
    let first: Vec<String> = (0..64).map(|i| format!("{i}.js")).collect();
    let warned = |w: &[String]| w.iter().any(|w| w.contains("more than 64 JavaScript files are linked"));
    let (doc, warnings) = import_with(&svg(&(links(64) + r#"<script xlink:href="0.js"/>"#)), &ImportOptions::default()).unwrap();
    assert_eq!(doc.svg_scripts, first);
    assert!(!warned(&warnings), "a repeated link is not one too many: {warnings:?}");
    let (doc, warnings) = import_with(&svg(&links(65)), &ImportOptions::default()).unwrap();
    assert_eq!(doc.svg_scripts, first);
    assert!(warned(&warnings), "{warnings:?}");
}
