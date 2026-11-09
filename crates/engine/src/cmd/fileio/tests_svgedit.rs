//! SVG files that reopen as they were: Preserve Editing restores the document exactly (unless the
//! SVG was edited elsewhere), Save keeps hidden layers, symbols share one def, and linked images
//! stay linked.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeKind};

use super::tests_svg::{image_session, svg};
use super::*;

fn new_id(v: &Value) -> u64 {
    v["id"].as_u64().or_else(|| v["ids"][0].as_u64()).unwrap_or_else(|| panic!("no id in {v}"))
}

pub(super) fn rect(s: &mut Session, x: f64, y: f64) -> u64 {
    new_id(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 20, "height": 20})).unwrap())
}

fn select(s: &mut Session, ids: &[u64]) {
    s.execute("select.set", &json!({ "ids": ids })).unwrap();
}

/// A roughened square, three instances of a symbol and a square filled with a pattern.
pub(super) fn rich() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 200})).unwrap();
    let rough = rect(&mut s, 10.0, 10.0);
    s.execute("effect.apply", &json!({"effect": "distort.roughen", "ids": [rough]})).unwrap();
    let mark = rect(&mut s, 60.0, 10.0);
    s.execute("symbol.new", &json!({"name": "Mark", "ids": [mark]})).unwrap();
    for (x, y) in [(150.0, 50.0), (200.0, 120.0)] {
        s.execute("symbol.place", &json!({"name": "Mark", "x": x, "y": y})).unwrap();
    }
    let dot = rect(&mut s, 0.0, 150.0);
    s.execute("paint.setFill", &json!({"ids": [dot], "color": "#ff0000"})).unwrap();
    select(&mut s, &[dot]);
    s.execute("object.pattern.make", &json!({"name": "Dots", "width": 30, "height": 30})).unwrap();
    s.execute("object.pattern.done", &json!({})).unwrap();
    let filled = rect(&mut s, 100.0, 150.0);
    s.execute("paint.setFill", &json!({"ids": [filled], "swatch": "Dots"})).unwrap();
    s
}

fn open(s: &mut Session, name: &str, text: &str) -> Value {
    s.execute("document.open", &json!({"name": name, "dataBase64": vectorcraft_format::base64_encode(text.as_bytes())})).unwrap()
}

/// `d` without what opening a file sets anew.
pub(super) fn comparable(d: &Document) -> Document {
    let mut d = d.clone();
    d.title.clear();
    d
}

#[test]
fn preserve_editing_restores_effects_symbols_and_patterns_exactly() {
    let mut s = rich();
    let before = comparable(&s.doc().unwrap().doc);
    assert!(before.symbols.iter().any(|x| x.name == "Mark") && before.pattern("Dots").is_some());
    let text = svg(&mut s, json!({"preserveEditing": true, "styling": "css"}));
    assert_eq!(text.matches("<use ").count(), 3, "{text}");
    let r = open(&mut s, "rich.svg", &text);
    assert_eq!(r["warnings"], json!([]), "{r}");
    let after = comparable(&s.doc().unwrap().doc);
    assert_eq!(after, before, "the document came back exactly");
    let mut effects = 0;
    after.walk(|n| effects += n.appearance.effects.len());
    assert_eq!(effects, 1, "the live effect is still live");
}

#[test]
fn an_svg_edited_elsewhere_opens_as_plain_svg_with_a_warning() {
    let mut s = rich();
    let text = svg(&mut s, json!({"preserveEditing": true}));
    // Another app changes a colour (the pattern's red).
    let edited = text.replacen("#ff0000", "#00ff00", 1);
    assert_ne!(edited, text);
    let r = open(&mut s, "edited.svg", &edited);
    assert_eq!(r["warnings"][0], load::EDITING_STALE, "{r}");
    let d = &s.doc().unwrap().doc;
    // (SVG import keeps symbols since M4.41, so the roughen's result tells the plain import.)
    let mut effects = 0;
    d.walk(|n| effects += n.appearance.effects.len());
    assert!(effects == 0 && d.patterns.iter().all(|p| p.name != "Dots"), "plain import: the roughen as its result, no named pattern");
    // Unreadable editing data (the hash only covers the markup around it) falls back too.
    let damaged = text.replacen("<![CDATA[", "<![CDATA[!!", 1);
    let r = open(&mut s, "damaged.svg", &damaged);
    assert_eq!(r["warnings"][0], load::EDITING_DAMAGED, "{r}");
}

#[test]
fn save_keeps_hidden_layers_and_export_leaves_them_out() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    rect(&mut s, 10.0, 10.0);
    let notes = new_id(&s.execute("layer.new", &json!({"name": "Notes"})).unwrap());
    rect(&mut s, 50.0, 50.0);
    s.execute("layer.setProps", &json!({"id": notes, "visible": false})).unwrap();

    let exported = svg(&mut s, json!({}));
    assert!(!exported.contains("Notes"), "{exported}");
    let saved = s.execute("document.save", &json!({"format": "svg"})).unwrap();
    let saved = String::from_utf8(vectorcraft_format::base64_decode(saved["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(saved.contains("id=\"Notes\"") && saved.contains("display=\"none\""), "{saved}");
    // Asked not to, Save leaves them out too.
    let r = s.execute("document.save", &json!({"format": "svg", "svg": {"hiddenLayers": false}})).unwrap();
    assert!(!String::from_utf8(vectorcraft_format::base64_decode(r["dataBase64"].as_str().unwrap()).unwrap()).unwrap().contains("Notes"));

    open(&mut s, "saved.svg", &saved);
    let d = &s.doc().unwrap().doc;
    let layers: Vec<(Option<&str>, bool, usize)> =
        d.layers.iter().map(|l| (l.name.as_deref(), l.visible, l.children().map_or(0, Vec::len))).collect();
    assert_eq!(layers, [(Some("Layer 1"), true, 1), (Some("Notes"), false, 1)]);
}

#[test]
fn link_mode_writes_no_data_uri() {
    let mut s = image_session();
    // An image placed linked: it has both its bytes and its file.
    s.edit("Link", |d, _| {
        let id = d.layers[0].children().and_then(|c| c.first()).map(|n| n.id).unwrap();
        if let Some(NodeKind::Image(im)) = d.node_mut(id).map(|n| &mut n.kind) {
            im.link = Some("art/dot.png".into());
        }
        Ok(())
    })
    .unwrap();
    let linked = svg(&mut s, json!({"images": "link"}));
    assert!(linked.contains("xlink:href=\"art/dot.png\"") && !linked.contains("data:"), "{linked}");
    let embedded = svg(&mut s, json!({"images": "embed"}));
    assert!(embedded.contains("href=\"data:image/png;base64,") && !embedded.contains("art/dot.png"), "{embedded}");
}
