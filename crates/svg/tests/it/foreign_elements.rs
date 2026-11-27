//! Elements in other applications' XML namespaces (https://www.w3.org/TR/xml-names/) survive a
//! round trip through VectorCraft (#9241): kept whole on import, under the root `<svg>` or the
//! group that held them, and written back there on export with their namespaces declared.
use craft_svg::tree::usvg::roxmltree;
use vectorcraft_doc::{Document, ForeignContent, ForeignElement, Node};
use vectorcraft_svg::{ExportOptions, MAX_FOREIGN_DEPTH, export, import, import_with_report};

const APP: &str = "https://example.com/app";

/// An SVG of `body` declaring the app's namespace.
fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:app="{APP}" width="100" height="100" viewBox="0 0 100 100">{body}</svg>"#)
}

/// The object of `doc` named `name`.
fn named<'a>(doc: &'a Document, name: &str) -> &'a Node {
    let mut found = None;
    doc.walk(|n| {
        if n.name.as_deref() == Some(name) {
            found = Some(n);
        }
    });
    found.unwrap_or_else(|| panic!("no object {name}"))
}

/// The elements of other applications the object of `doc` named `name` keeps.
fn kept(doc: &Document, name: &str) -> Vec<ForeignElement> {
    named(doc, name).extras().attrs.as_deref().map(|a| a.foreign_elements.clone()).unwrap_or_default()
}

/// The text `e` holds, its elements' included.
fn text(e: roxmltree::Node) -> String {
    e.descendants().filter(roxmltree::Node::is_text).filter_map(|t| t.text()).collect()
}

const CONFIG: &str = r#"{"behavior": [{"id": "running", "type": "value"}], "render": "a < b && b > c"}"#;

/// An app's element under the root, and others in a group and a nested group, survive import,
/// export, import again and the native file; the shapes beside them are untouched.
#[test]
fn other_applications_elements_survive_a_round_trip() {
    let file = svg(&format!(
        r#"<app:config version="2"><![CDATA[{CONFIG}]]></app:config>
        <g id="body" app:tag="body"><app:hint>kept with the group</app:hint>
            <rect id="box" width="20" height="20"/>
            <g id="inner"><app:part app:role="lid"><app:note level="1">on top</app:note></app:part><rect id="lid" width="5" height="5"/></g>
        </g>"#
    ));
    let (doc, warnings) = import_with_report(&file).unwrap();
    assert_eq!(warnings, Vec::<String>::new());
    let [config] = doc.svg_foreign_elements.as_slice() else { panic!("{:?}", doc.svg_foreign_elements) };
    assert_eq!((config.ns.as_str(), config.prefix.as_str(), config.name.as_str()), (APP, "app", "config"));
    assert_eq!(config.content, [ForeignContent::Text(CONFIG.into())]);
    assert_eq!(kept(&doc, "body").len(), 1);
    let [part] = kept(&doc, "inner").try_into().unwrap();
    assert!(matches!(part.content.as_slice(), [ForeignContent::Element(note)] if note.name == "note"), "{part:?}");
    // Written back under the same parents, in the app's namespace.
    let text_out = export(&doc, &ExportOptions::default());
    let xml = roxmltree::Document::parse(&text_out).unwrap();
    let root = xml.root_element();
    let config_out = root.children().find(|c| c.has_tag_name((APP, "config"))).unwrap_or_else(|| panic!("{text_out}"));
    assert_eq!((config_out.attribute("version"), text(config_out)), (Some("2"), CONFIG.to_string()));
    let in_group = |id: &str, name: &str| {
        let g = xml.descendants().find(|n| n.attribute("id") == Some(id)).unwrap_or_else(|| panic!("no {id}: {text_out}"));
        g.children().any(|c| c.has_tag_name((APP, name)))
    };
    assert!(in_group("body", "hint") && in_group("inner", "part"), "{text_out}");
    let note = xml.descendants().find(|n| n.has_tag_name((APP, "note"))).unwrap();
    assert_eq!((note.attribute("level"), text(note).as_str()), (Some("1"), "on top"));
    assert!(xml.descendants().any(|n| n.has_tag_name("rect") && n.attribute("id") == Some("lid")));
    let back = import(&text_out).unwrap();
    assert_eq!(back.svg_foreign_elements, doc.svg_foreign_elements);
    assert_eq!((kept(&back, "body"), kept(&back, "inner")), (kept(&doc, "body"), kept(&doc, "inner")));
    // And through the native file.
    let json = serde_json::to_value(&doc).unwrap();
    let native: Document = serde_json::from_value(json).unwrap();
    assert_eq!((native.svg_foreign_elements, kept(&native, "inner")), (doc.svg_foreign_elements.clone(), kept(&doc, "inner")));
}

/// A prefix the export uses itself, and the default namespace, are written under prefixes of their
/// own; an element whose name XML can't hold is left out with a warning.
#[test]
fn prefixes_are_made_unique_and_bad_elements_left_out() {
    let file = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
        <xlink:a xmlns:xlink="urn:not-xlink" k="1"/><cfg xmlns="urn:plain"><item/></cfg></svg>"#;
    let mut doc = import(file).unwrap();
    assert_eq!(doc.svg_foreign_elements.len(), 2, "{:?}", doc.svg_foreign_elements);
    doc.svg_foreign_elements.push(ForeignElement { ns: "urn:x".into(), prefix: "x".into(), name: "bad name".into(), ..Default::default() });
    let out = vectorcraft_svg::export_full(&doc, &ExportOptions::default(), None);
    let text_out = String::from_utf8(out.bytes()).unwrap();
    let xml = roxmltree::Document::parse(&text_out).unwrap_or_else(|e| panic!("{e}: {text_out}"));
    let root = xml.root_element();
    assert!(root.children().any(|c| c.has_tag_name(("urn:not-xlink", "a")) && c.attribute("k") == Some("1")), "{text_out}");
    let cfg = root.children().find(|c| c.has_tag_name(("urn:plain", "cfg"))).unwrap_or_else(|| panic!("{text_out}"));
    assert!(cfg.children().any(|c| c.has_tag_name(("urn:plain", "item"))), "{text_out}");
    assert!(out.warnings.iter().any(|w| w.contains("Elements of other applications")), "{:?}", out.warnings);
}

/// The file is untrusted: an element nested deeper than [`MAX_FOREIGN_DEPTH`] is left out whole,
/// with a warning, and its siblings are kept.
#[test]
fn foreign_elements_are_capped() {
    let deep = MAX_FOREIGN_DEPTH + 2;
    let nested = format!("{}{}", "<app:n>".repeat(deep), "</app:n>".repeat(deep));
    let (doc, warnings) = import_with_report(&svg(&format!("<app:keep/>{nested}"))).unwrap();
    assert_eq!(doc.svg_foreign_elements.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["keep"]);
    assert!(warnings.iter().any(|w| w.contains("other applications' elements")), "{warnings:?}");
}
