//! Inline graphics in text: `text.insertInline`, editing over them, undo, following their symbol.

use serde_json::json;
use vectorcraft_doc::text::INLINE_CHAR;
use vectorcraft_doc::{NodeId, NodeKind, TextObject};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

/// A symbol `name` made of a circle `d` points across (drawn in code), its instance deleted.
fn circle_symbol(s: &mut Session, name: &str, d: f64) {
    s.execute("shape.ellipse", &json!({"x": 600, "y": 500, "width": d, "height": d})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("symbol.new", &json!({"name": name})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
}

fn text(s: &mut Session, t: &str) -> NodeId {
    let r = s.execute("text.create", &json!({"x": 100, "y": 100, "text": t, "size": 20})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn obj(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        _ => panic!("not text"),
    }
}

fn inline_count(t: &TextObject) -> usize {
    t.runs.iter().filter(|r| r.inline.is_some()).count()
}

#[test]
fn insert_inline_places_a_symbol_as_one_character() {
    let mut s = session();
    circle_symbol(&mut s, "Tap", 10.0);
    let id = text(&mut s, "Tap: add");
    let before = obj(&s, id).bounds().unwrap();
    let r = s.execute("text.insertInline", &json!({"id": id.0, "at": 0, "symbol": "Tap", "scale": 1.5, "shift": 1})).unwrap();
    assert_eq!(r, json!({"id": id.0, "caret": INLINE_CHAR.len_utf8()}));
    let t = obj(&s, id);
    assert_eq!(t.plain_text(), format!("{INLINE_CHAR}Tap: add"));
    let art = t.runs[0].inline.as_ref().unwrap();
    assert_eq!((art.symbol.as_str(), art.scale, art.baseline_shift), ("Tap", 1.5, 1.0));
    // Resolved from the symbol (a 10 pt circle), and the text grew by the art (30 pt wide).
    let b = art.bounds.unwrap();
    assert!((b.width() - 10.0).abs() < 0.5 && (b.height() - 10.0).abs() < 0.5, "{b:?}");
    let after = t.bounds().unwrap();
    assert!((after.width() - before.width() - 30.0).abs() < 1.0, "{before:?} → {after:?}");
    // It takes the style of the text it sits in.
    assert_eq!(t.runs[0].style.size, 20.0);
    // text.getRange reports it.
    let g = s.execute("text.getRange", &json!({"id": id.0, "start": 0, "end": 3})).unwrap();
    assert_eq!(g["runs"][0]["inline"], json!({"symbol": "Tap", "scale": 1.5, "baseline_shift": 1.0}));
    assert_eq!(g["text"], json!(INLINE_CHAR.to_string()));
    // One undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(inline_count(&obj(&s, id)), 0);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(inline_count(&obj(&s, id)), 1);
    // Deleting any part of its character deletes it whole.
    s.execute("text.editRange", &json!({"id": id.0, "start": 1, "end": 3})).unwrap();
    let t = obj(&s, id);
    assert_eq!(t.plain_text(), "Tap: add");
    assert_eq!(inline_count(&t), 0);
}

#[test]
fn insert_inline_rejects_junk() {
    let mut s = session();
    circle_symbol(&mut s, "Dot", 8.0);
    let id = text(&mut s, "abc");
    let rect = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    for p in [
        json!({"id": id.0, "symbol": "Nope"}),
        json!({"id": id.0, "symbol": 7}),
        json!({"id": id.0, "symbol": "Dot", "scale": 0}),
        json!({"id": id.0, "symbol": "Dot", "scale": -1}),
        json!({"id": id.0, "symbol": "Dot", "scale": 1e9}),
        json!({"id": id.0, "symbol": "Dot", "shift": "up"}),
        json!({"id": id.0, "symbol": "Dot", "at": -3}),
        json!({"id": "x", "symbol": "Dot"}),
        json!({"id": rect["id"], "symbol": "Dot"}),
        json!({"id": 999999, "symbol": "Dot"}),
        json!({"symbol": "Dot"}),
    ] {
        assert!(s.execute("text.insertInline", &p).is_err(), "{p}");
    }
    assert_eq!(inline_count(&obj(&s, id)), 0);
    // Offsets past the end are clamped; mid-character offsets land on a boundary.
    let r = s.execute("text.insertInline", &json!({"id": id.0, "at": 99, "symbol": "Dot"})).unwrap();
    assert_eq!(r["caret"], json!(3 + INLINE_CHAR.len_utf8()));
    s.execute("text.insertInline", &json!({"id": id.0, "at": 4, "symbol": "Dot"})).unwrap();
    let t = obj(&s, id);
    assert_eq!(t.plain_text(), format!("abc{INLINE_CHAR}{INLINE_CHAR}"));
    assert_eq!(inline_count(&t), 2);
    // A document without symbols has nothing to insert.
    let mut s = session();
    let id = text(&mut s, "abc");
    assert!(s.execute("text.insertInline", &json!({"id": id.0})).is_err());
}

#[test]
fn type_tool_inserts_at_its_caret_and_deletes_the_graphic_whole() {
    let mut s = session();
    circle_symbol(&mut s, "Mana", 10.0);
    s.execute("symbol.setCurrent", &json!({"name": "Mana"})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, 50.0, 300.0), v).unwrap();
    }
    let id = s.doc().unwrap().selection.objects[0];
    s.tool_text("Pay ", v).unwrap();
    // The menu item: no params, inside a typing session.
    let r = s.execute("text.insertInline", &json!({})).unwrap();
    assert_eq!(r["id"], json!(id.0));
    assert_eq!(s.tool_options()["caret"], json!(4 + INLINE_CHAR.len_utf8()));
    s.tool_text(" now", v).unwrap();
    let t = obj(&s, id);
    assert_eq!(t.plain_text(), format!("Pay {INLINE_CHAR} now"));
    assert_eq!(inline_count(&t), 1);
    assert_eq!(t.runs.iter().find(|r| r.inline.is_some()).unwrap().inline.as_ref().unwrap().symbol, "Mana");
    // Back over " now", then the graphic: one Backspace removes it whole.
    for _ in 0..5 {
        s.tool_key(ToolKey::Backspace, Mods::default(), v).unwrap();
    }
    let t = obj(&s, id);
    assert_eq!(t.plain_text(), "Pay ");
    assert_eq!(inline_count(&t), 0);
    // Undo brings it back.
    s.tool_key(ToolKey::Escape, Mods::default(), v).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(inline_count(&obj(&s, id)), 1);
}

#[test]
fn inline_graphics_follow_their_symbol() {
    let mut s = session();
    circle_symbol(&mut s, "Dot", 10.0);
    let id = text(&mut s, "ab");
    s.execute("text.insertInline", &json!({"id": id.0, "at": 1, "symbol": "Dot", "scale": 1})).unwrap();
    let w0 = obj(&s, id).bounds().unwrap().width();
    // Redefine the symbol as a 2:1 ellipse: the graphic (20 pt tall) widens to 40 pt.
    let e = s.execute("shape.ellipse", &json!({"x": 300, "y": 300, "width": 40, "height": 20})).unwrap();
    s.execute("select.set", &json!({"ids": [e["id"]]})).unwrap();
    s.execute("symbol.update", &json!({"name": "Dot"})).unwrap();
    let t = obj(&s, id);
    let b = t.runs[1].inline.as_ref().unwrap().bounds.unwrap();
    assert!((b.width() / b.height() - 2.0).abs() < 0.1, "{b:?}");
    let w1 = t.bounds().unwrap().width();
    assert!((w1 - w0 - 20.0).abs() < 1.5, "{w0} → {w1}");
    // Deleting the symbol leaves an empty slot (nothing drawn), and doesn't crash rendering.
    s.execute("symbol.delete", &json!({"name": "Dot"})).unwrap();
    let t = obj(&s, id);
    assert!(t.runs[1].inline.as_ref().unwrap().bounds.is_none());
    let _ = s.execute("document.serialize", &json!({"format": "png"})).unwrap();
    let _ = s.execute("document.serialize", &json!({"format": "svg"})).unwrap();
    let _ = s.execute("document.serialize", &json!({"format": "pdf"})).unwrap();
}

/// A list paragraph that starts with an inline graphic draws its bullet before the graphic, and
/// live SVG text writes the bullet.
#[test]
fn a_list_paragraph_starting_with_an_inline_graphic_keeps_its_bullet() {
    let mut s = session();
    circle_symbol(&mut s, "Dot", 10.0);
    let id = text(&mut s, "ab");
    s.execute("text.insertInline", &json!({"id": id.0, "at": 0, "symbol": "Dot", "scale": 1})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [id.0], "list": "bullet"})).unwrap();
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &obj(&s, id));
    assert_eq!(lay.markers.len(), 1);
    let bullet = &lay.glyphs[lay.markers[0].glyphs.start];
    assert!(bullet.origin.x < lay.inlines[0].bounds.x0, "the bullet comes before the graphic");
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap()["text"].as_str().unwrap().to_string();
    assert!(svg.contains('\u{2022}'), "{svg}");
}

#[test]
fn saved_documents_keep_inline_graphics() {
    let mut s = session();
    circle_symbol(&mut s, "Dot", 10.0);
    let id = text(&mut s, "ab");
    s.execute("text.insertInline", &json!({"id": id.0, "at": 1, "symbol": "Dot", "scale": 2})).unwrap();
    let doc = (*s.doc().unwrap().doc).clone();
    let bytes = vectorcraft_format::save(&doc, false);
    assert!(String::from_utf8_lossy(&bytes).contains("\"inline\""));
    let mut back = vectorcraft_format::load(&bytes).unwrap();
    // Bounds aren't saved: opening resolves them.
    let mut s2 = Session::new();
    back.title = "copy".into();
    s2.add_document(back, None);
    let t = obj(&s2, id);
    let art = t.runs[1].inline.as_ref().unwrap();
    assert_eq!((art.symbol.as_str(), art.scale), ("Dot", 2.0));
    assert!(art.bounds.is_some());
    assert_eq!(t.cached_bounds, obj(&s, id).cached_bounds);
}

#[test]
fn create_outlines_turns_inline_graphics_into_symbol_instances() {
    let mut s = session();
    circle_symbol(&mut s, "Dot", 10.0);
    let id = text(&mut s, "a b");
    s.execute("text.insertInline", &json!({"id": id.0, "at": 2, "symbol": "Dot"})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    let r = s.execute("type.createOutlines", &json!({})).unwrap();
    let g = NodeId(r["ids"][0].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    let kids = d.node(g).unwrap().children().unwrap();
    let inst = kids.iter().filter(|c| matches!(&c.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Dot")).count();
    assert_eq!(inst, 1);
    // Between the outlines of `a` and `b`.
    let pos = kids.iter().position(|c| matches!(c.kind, NodeKind::SymbolInstance { .. })).unwrap();
    assert!(pos > 0 && pos + 1 < kids.len(), "{pos} of {}", kids.len());
}
