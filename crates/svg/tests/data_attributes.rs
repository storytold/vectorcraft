//! Synthetic XML 1.0/SVG round trips: https://www.w3.org/TR/xml/#AVNormalize
//! https://www.w3.org/TR/xml/#NT-NameChar https://www.w3.org/TR/xml/#charsets
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use std::collections::HashSet;
use usvg::roxmltree;
use vectorcraft_doc::{Document, Node};
use vectorcraft_svg::{Encoding, ExportOptions, ObjectIds, Output, Styling, export, export_full, import};

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">{body}</svg>"#)
}
fn styles() -> [Styling; 4] {
    [Styling::PresentationAttributes, Styling::InlineStyle, Styling::InternalCss, Styling::StyleEntities]
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
fn xml(text: &str) -> roxmltree::Document<'_> {
    roxmltree::Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() }).unwrap()
}
fn native_data(raw: &str) -> Document {
    let mut doc = import(&svg(r#"<rect id="shape" width="20" height="20"/>"#)).unwrap();
    let attrs = serde_json::from_str::<vectorcraft_doc::node::ObjectAttributes>(raw).unwrap();
    let id = named(&doc, "shape").id;
    doc.node_mut(id).unwrap().edit_attrs(|a| *a = attrs);
    doc
}
#[test]
fn imported_decimal_whitespace_references_survive_export() {
    let doc = import(&svg(r#"<rect id="shape" width="20" height="20" data-value="a&#10;b" data-tab="a&#9;b" data-cr="a&#13;b"/>"#)).unwrap();
    assert_eq!(
        named(&doc, "shape").attrs.as_deref().unwrap().data,
        [("value".into(), "a\nb".into()), ("tab".into(), "a\tb".into()), ("cr".into(), "a\rb".into())]
    );
    for styling in styles() {
        for minify in [false, true] {
            let text = export(&doc, &ExportOptions { styling, minify, ..Default::default() });
            let parsed = xml(&text);
            let n = parsed.descendants().find(|n| n.attribute("data-value").is_some()).unwrap();
            for (key, value) in [("data-value", "a\nb"), ("data-tab", "a\tb"), ("data-cr", "a\rb")] {
                assert_eq!(n.attribute(key), Some(value));
            }
            let back = import(&text).unwrap();
            assert_eq!(named(&back, "shape").attrs.as_deref().unwrap().data, named(&doc, "shape").attrs.as_deref().unwrap().data);
        }
    }
}
fn encoded_text(out: &Output) -> String {
    let bytes = out.bytes();
    match out.encoding {
        Encoding::Utf8 => String::from_utf8(bytes).unwrap(),
        Encoding::Utf16 => {
            assert!(bytes.starts_with(&[0xfe, 0xff]));
            assert_eq!(bytes.len() % 2, 0);
            let chars: Vec<_> = bytes[2..].as_chunks::<2>().0.iter().copied().map(u16::from_be_bytes).collect();
            String::from_utf16(&chars).unwrap()
        }
        Encoding::Latin1 => bytes.into_iter().map(char::from).collect(),
    }
}
#[test]
fn unicode_data_names_survive_each_encoding() {
    let doc = import(&svg(
        r#"<rect id="shape" width="20" height="20" data-ok="safe" data-配方="blend" data-á="accent" data-𐀀="supplementary" data-·="suffix"/>"#,
    ))
    .unwrap();
    assert_eq!(named(&doc, "shape").attrs.as_deref().unwrap().data.len(), 5);
    for styling in styles() {
        for encoding in [Encoding::Utf8, Encoding::Utf16, Encoding::Latin1] {
            for minify in [false, true] {
                let out = export_full(&doc, &ExportOptions { styling, encoding, minify, ..Default::default() }, None);
                assert_eq!(encoded_text(&out), out.svg);
                let parsed = xml(&out.svg);
                let n = parsed.descendants().find(|n| n.attribute("data-ok").is_some()).unwrap();
                for (key, value) in [("data-配方", "blend"), ("data-á", "accent"), ("data-𐀀", "supplementary"), ("data-·", "suffix")] {
                    assert_eq!(n.attribute(key), Some(value));
                }
                if encoding == Encoding::Latin1 {
                    assert_eq!(out.encoding, Encoding::Utf8, "XML names cannot use character references");
                    assert!(!out.warnings.is_empty());
                    assert!(!out.svg.contains("ISO-8859-1"));
                } else {
                    assert_eq!(out.encoding, encoding);
                }
                let back = import(&out.svg).unwrap();
                assert_eq!(named(&back, "shape").attrs.as_deref().unwrap().data, named(&doc, "shape").attrs.as_deref().unwrap().data);
            }
        }
    }
}
#[test]
fn latin1_preserves_representable_unicode_names() {
    let doc = import(&svg(r#"<rect id="shape" width="20" height="20" data-é="配方"/>"#)).unwrap();
    for styling in styles() {
        let out = export_full(&doc, &ExportOptions { styling, encoding: Encoding::Latin1, ..Default::default() }, None);
        assert_eq!(out.encoding, Encoding::Latin1);
        assert!(out.warnings.is_empty());
        assert_eq!(encoded_text(&out), out.svg);
        let parsed = xml(&out.svg);
        assert!(parsed.descendants().any(|n| n.attribute("data-é") == Some("配方")));
    }
}
#[test]
fn native_json_invalid_xml_data_cannot_break_export() {
    let doc = native_data(
        r#"{"data":[["ok","safe"],["control","a\u0000b"],["bad key","ordinary"],["colon:k","ordinary"],["noncharacter","\uffff"],["othercontrol","\u0001\u000b\u001f"],["bad\u0000key","ordinary"],["outside\udb80\udc00","ordinary"]]}"#,
    );
    for styling in styles() {
        let text = export(&doc, &ExportOptions { styling, ..Default::default() });
        let parsed = xml(&text);
        let n = parsed.descendants().find(|n| n.attribute("data-ok").is_some()).unwrap();
        assert_eq!(n.attribute("data-ok"), Some("safe"));
        assert!(n.attribute("data-othercontrol").is_none());
        assert!(n.attribute("data-control").is_none() && n.attribute("data-noncharacter").is_none());
    }
}
#[test]
fn native_json_repeated_data_keys_emit_one_valid_attribute() {
    let doc = native_data(
        r#"{"data":[["ok","safe"],["repeat","first"],["repeat","second"],["fallback","\u0000"],["fallback","good"],["name","reserved"],["vc-private","reserved"]]}"#,
    );
    for styling in styles() {
        let text = export(&doc, &ExportOptions { styling, ..Default::default() });
        let parsed = xml(&text);
        let n = parsed.descendants().find(|n| n.attribute("data-ok").is_some()).unwrap();
        assert_eq!(n.attribute("data-repeat"), Some("first"));
        assert_eq!(n.attribute("data-fallback"), Some("good"));
        assert!(n.attribute("data-vc-private").is_none());
        assert!(n.attribute("data-name").is_none());
    }
}
#[test]
fn name_aliases_preserve_decoded_whitespace() {
    let name = "name\na\tb\rc";
    let doc = import(&svg(r#"<rect id="shape" data-name="name&#10;a&#9;b&#13;c" data-ok="safe" width="20" height="20"/>"#)).unwrap();
    for styling in styles() {
        let text = export(&doc, &ExportOptions { styling, ..Default::default() });
        let parsed = xml(&text);
        let n = parsed.descendants().find(|n| n.attribute("data-ok").is_some()).unwrap();
        assert_eq!(n.attribute("data-name"), Some(name));
        assert_eq!(named(&import(&text).unwrap(), name).name.as_deref(), Some(name));
    }
}
#[test]
fn native_json_name_controls_do_not_break_xml() {
    let mut doc = import(&svg(r#"<rect id="shape" data-ok="safe" width="20" height="20"/>"#)).unwrap();
    let id = named(&doc, "shape").id;
    doc.node_mut(id).unwrap().name = Some(serde_json::from_str::<String>(r#""bad\u0000name""#).unwrap());
    let text = export(&doc, &ExportOptions::default());
    let parsed = xml(&text);
    let n = parsed.descendants().find(|n| n.attribute("data-ok").is_some()).unwrap();
    assert!(n.attribute("data-name").is_none());
}
#[test]
fn reid_imported_nodes_keeps_unique_ids_references_and_data() {
    let source = svg(
        r##"<defs><clipPath id="clip"><rect width="60" height="60"/></clipPath><linearGradient id="paint"><stop stop-color="blue"/><stop offset="1" stop-color="red"/></linearGradient><mask id="opacity"><rect width="100" height="100" fill="white"/></mask></defs><g id="layer"><g id="copy" clip-path="url(#clip)" data-copy="root"><rect id="clip-path-1" width="50" height="50" fill="url(#paint)" data-value="safe" data-配方="a&#10;b&#x9;c&#13;d"/><rect id="gradient-1" x="10" width="40" height="40" fill="url(#paint)"/><rect id="masked" width="30" height="30" fill="green" mask="url(#opacity)"/></g></g>"##,
    );
    let mut doc = import(&source).unwrap();
    let original = named(&doc, "copy").clone();
    let layer = doc.layers[0].id;
    for _ in 0..2 {
        let copy = doc.reid(&original);
        doc.insert(Some(layer), usize::MAX, copy).unwrap();
    }
    let mut nodes = Vec::new();
    doc.walk(|n| nodes.push(n.id));
    assert_eq!(nodes.len(), nodes.iter().collect::<HashSet<_>>().len());
    for object_ids in [ObjectIds::LayerNames, ObjectIds::Unique, ObjectIds::Minimal] {
        for styling in styles() {
            let text = export(&doc, &ExportOptions { object_ids, styling, ..Default::default() });
            let parsed = xml(&text);
            let ids: Vec<_> = parsed.descendants().filter_map(|n| n.attribute("id")).collect();
            let set = ids.iter().copied().collect::<HashSet<_>>();
            assert_eq!(ids.len(), set.len());
            for id in &ids {
                assert!(
                    id.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                        && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
                );
            }
            let mut count = 0;
            for suffix in text.split("url(#").skip(1) {
                let id = suffix.split(')').next().unwrap();
                assert!(set.contains(id));
                count += 1;
            }
            assert!(count > 0);
            assert_eq!(parsed.descendants().filter(|n| n.attribute("data-copy") == Some("root")).count(), 3);
            assert_eq!(parsed.descendants().filter(|n| n.attribute("data-value") == Some("safe")).count(), 3);
            let value = "a\nb\tc\rd";
            assert_eq!(parsed.descendants().filter(|n| n.attribute("data-配方") == Some(value)).count(), 3);
            let back = import(&text).unwrap();
            let mut copies = 0;
            back.walk(|n| {
                if n.attrs.as_deref().is_some_and(|a| a.data.iter().any(|(k, v)| k == "配方" && v == value)) {
                    copies += 1;
                }
            });
            assert_eq!(copies, 3);
        }
    }
}

fn symbol_document(name: &str) -> Document {
    let mut doc = import(&svg(r#"<rect id="instance" width="20" height="20"/>"#)).unwrap();
    let id = named(&doc, "instance").id;
    let art = doc.node(id).unwrap().clone();
    doc.symbols.push(vectorcraft_doc::Symbol { name: name.into(), art: std::sync::Arc::new(art) });
    let instance = doc.node_mut(id).unwrap();
    instance.appearance = vectorcraft_doc::Appearance::default();
    instance.kind = vectorcraft_doc::NodeKind::SymbolInstance { symbol: name.into(), xf: vectorcraft_geom::Affine::IDENTITY };
    doc
}

#[test]
fn symbol_name_aliases_preserve_decoded_whitespace() {
    let name = "symbol\nwith\ttab\rand é配方";
    let doc = symbol_document(name);
    for styling in styles() {
        for encoding in [Encoding::Utf8, Encoding::Utf16, Encoding::Latin1] {
            for minify in [false, true] {
                let out = export_full(&doc, &ExportOptions { styling, encoding, minify, ..Default::default() }, None);
                assert_eq!(out.encoding, encoding);
                assert_eq!(encoded_text(&out), out.svg);
                let parsed = xml(&out.svg);
                let symbol = parsed.descendants().find(|n| n.has_tag_name("symbol")).unwrap();
                assert_eq!(symbol.attribute("data-name"), Some(name));
                let back = import(&out.svg).unwrap();
                assert_eq!(back.symbols.len(), 1);
                assert_eq!(back.symbols[0].name, name);
            }
        }
    }
}

#[test]
fn invalid_symbol_name_alias_does_not_break_xml() {
    for name in ["bad\0symbol", "bad\u{1}symbol", "bad\u{ffff}symbol"] {
        let doc = symbol_document(name);
        for styling in styles() {
            for encoding in [Encoding::Utf8, Encoding::Utf16, Encoding::Latin1] {
                let out = export_full(&doc, &ExportOptions { styling, encoding, ..Default::default() }, None);
                assert_eq!(encoded_text(&out), out.svg);
                let parsed = xml(&out.svg);
                let symbol = parsed.descendants().find(|n| n.has_tag_name("symbol")).unwrap();
                assert!(symbol.attribute("data-name").is_none());
                let id = symbol.attribute("id").unwrap();
                assert!(parsed.descendants().any(|n| n.attribute(("http://www.w3.org/1999/xlink", "href")) == Some(&format!("#{id}"))));
                assert!(out.warnings.iter().any(|w| w.contains("Symbol names")));
                assert_eq!(import(&out.svg).unwrap().symbols.len(), 1);
            }
        }
    }
}
