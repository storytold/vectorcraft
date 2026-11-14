//! Inline graphics in text: `text.insertInline`, editing over them, undo, following their symbol.

use serde_json::json;
use vectorcraft_doc::text::INLINE_CHAR;
use vectorcraft_doc::{Node, NodeId, NodeKind, TextObject};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

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

/// With Performance › Real-time Drawing and Editing off, the typing waits for its session to end:
/// a graphic inserted while typing applies it first, as a step of its own, and goes in at the
/// caret after it.
#[test]
fn inserting_while_deferred_typing_keeps_the_typing() {
    let mut s = session();
    s.prefs.real_time_drawing = false;
    // The new type starts empty: the caret after "Pay " is past the end of the text until the
    // typing is applied.
    s.prefs.placeholder_text = false;
    circle_symbol(&mut s, "Mana", 10.0);
    s.execute("symbol.setCurrent", &json!({"name": "Mana"})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("type", v).unwrap();
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, 50.0, 300.0), v).unwrap();
    }
    let id = s.doc().unwrap().selection.objects[0];
    s.tool_text("Pay ", v).unwrap();
    s.execute("text.insertInline", &json!({})).unwrap();
    assert_eq!(obj(&s, id).plain_text(), format!("Pay {INLINE_CHAR}"));
    let labels: Vec<String> = s.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect();
    assert_eq!(labels[labels.len() - 2..], ["Typing", "Insert Inline Symbol"], "{labels:?}");
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

/// The first text in tree `n`, its opacity mask art included.
fn first_text(n: &Node) -> Option<TextObject> {
    if let NodeKind::Text(t) = &n.kind {
        return Some((**t).clone());
    }
    if let Some(t) = n.extras().mask.as_deref().and_then(|m| first_text(&m.art)) {
        return Some(t);
    }
    n.children().into_iter().flatten().find_map(|c| first_text(c))
}

/// The first text in the art of symbol `name`.
fn art_text(s: &Session, name: &str) -> TextObject {
    let d = &s.doc().unwrap().doc;
    first_text(&d.symbols.iter().find(|x| x.name == name).unwrap().art).unwrap()
}

/// The bounds of the first inline graphic in `t`.
fn inline_bounds(t: &TextObject) -> Option<vectorcraft_geom::Rect> {
    t.runs.iter().find_map(|r| r.inline.as_ref()).unwrap().bounds
}

/// Redefines symbol `name` as a 40 × 20 ellipse.
fn redefine_as_wide_ellipse(s: &mut Session, name: &str) {
    let e = s.execute("shape.ellipse", &json!({"x": 300, "y": 300, "width": 40, "height": 20})).unwrap();
    s.execute("select.set", &json!({"ids": [e["id"]]})).unwrap();
    s.execute("symbol.update", &json!({"name": name})).unwrap();
}

/// A symbol made from type with an inline graphic: when the symbol the graphic shows changes, the
/// type in the symbol's art is laid out again, the symbol's art takes its new size, and an inline
/// graphic of that symbol follows. Undo, redo and opening a saved copy keep it so.
#[test]
fn inline_graphics_in_symbol_art_follow_their_symbol() {
    let mut s = session();
    circle_symbol(&mut s, "Dot", 10.0);
    let id = text(&mut s, "ab");
    s.execute("text.insertInline", &json!({"id": id.0, "at": 1, "symbol": "Dot", "scale": 1})).unwrap();
    s.execute("symbol.new", &json!({"name": "Holder", "ids": [id.0]})).unwrap();
    let line = text(&mut s, "x");
    s.execute("text.insertInline", &json!({"id": line.0, "at": 0, "symbol": "Holder", "scale": 1})).unwrap();
    let held = art_text(&s, "Holder");
    let w0 = held.cached_bounds.unwrap().width();
    let holder0 = s.doc().unwrap().doc.symbol_art_bounds("Holder").unwrap();
    redefine_as_wide_ellipse(&mut s, "Dot");
    let d = &s.doc().unwrap().doc;
    let held = art_text(&s, "Holder");
    let b = inline_bounds(&held).unwrap();
    assert!((b.width() / b.height() - 2.0).abs() < 0.1, "{b:?}");
    assert_eq!(Some(b), d.symbol_art_bounds("Dot"));
    let w1 = held.cached_bounds.unwrap().width();
    assert!((w1 - w0 - 20.0).abs() < 1.5, "{w0} → {w1}");
    assert!(held.cached_ink.is_some() && !held.cached_baselines.is_empty());
    let holder1 = d.symbol_art_bounds("Holder").unwrap();
    assert!(holder1.width() > holder0.width() + 1.0, "{holder0:?} → {holder1:?}");
    assert_eq!(inline_bounds(&obj(&s, line)), Some(holder1), "an inline graphic of the holder follows it");
    // Undo and redo.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!((art_text(&s, "Holder").cached_bounds.unwrap().width() - w0).abs() < 1e-6);
    s.execute("edit.redo", &json!({})).unwrap();
    assert!((art_text(&s, "Holder").cached_bounds.unwrap().width() - w1).abs() < 1e-6);
    // Opening a saved copy measures it the same way.
    let doc = (*s.doc().unwrap().doc).clone();
    let mut back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    back.title = "copy".into();
    let mut s2 = Session::new();
    s2.add_document(back, None);
    let held2 = art_text(&s2, "Holder");
    assert_eq!(inline_bounds(&held2), s2.doc().unwrap().doc.symbol_art_bounds("Dot"));
    assert!((held2.cached_bounds.unwrap().width() - w1).abs() < 1e-6);
    assert_eq!(inline_bounds(&obj(&s2, line)), s2.doc().unwrap().doc.symbol_art_bounds("Holder"));
}

/// Type with an inline graphic in an opacity mask and in a pattern tile follows its symbol.
#[test]
fn inline_graphics_in_masks_and_pattern_tiles_follow_their_symbol() {
    let mut s = session();
    circle_symbol(&mut s, "Dot", 10.0);
    let masked = text(&mut s, "ab");
    s.execute("text.insertInline", &json!({"id": masked.0, "at": 1, "symbol": "Dot"})).unwrap();
    let owner = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 20, "width": 50, "height": 20})).unwrap()["id"].as_u64().unwrap());
    s.edit("Mask", |d, _| {
        let art = d.remove(masked)?;
        d.node_mut(owner).unwrap().extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new((*art).clone(), true)));
        Ok(())
    })
    .unwrap();
    let tiled = text(&mut s, "cd");
    s.execute("text.insertInline", &json!({"id": tiled.0, "at": 1, "symbol": "Dot"})).unwrap();
    s.execute("object.pattern.make", &json!({"ids": [tiled.0], "name": "Tile", "edit": false})).unwrap();
    redefine_as_wide_ellipse(&mut s, "Dot");
    let d = &s.doc().unwrap().doc;
    let want = d.symbol_art_bounds("Dot");
    let mask = first_text(&d.node(owner).unwrap().extras().mask.as_deref().unwrap().art).unwrap();
    assert_eq!(inline_bounds(&mask), want);
    let tile = d.patterns.iter().find(|p| p.name == "Tile").unwrap().art.iter().find_map(|n| first_text(n)).unwrap();
    assert_eq!(inline_bounds(&tile), want);
}

/// Symbol C shows B inline and B shows A: with the symbols listed C, B, A, a change to A reaches C
/// through B's new size.
#[test]
fn symbols_are_resolved_after_the_symbols_they_show_inline() {
    let mut s = session();
    circle_symbol(&mut s, "A", 10.0);
    for (name, shows) in [("B", "A"), ("C", "B")] {
        let t = text(&mut s, "z");
        s.execute("text.insertInline", &json!({"id": t.0, "at": 0, "symbol": shows})).unwrap();
        s.execute("symbol.new", &json!({"name": name, "ids": [t.0]})).unwrap();
        s.execute("edit.clear", &json!({})).unwrap();
    }
    s.edit("Reverse the symbols", |d, _| {
        d.symbols.reverse();
        Ok(())
    })
    .unwrap();
    let b0 = s.doc().unwrap().doc.symbol_art_bounds("B");
    redefine_as_wide_ellipse(&mut s, "A");
    let d = &s.doc().unwrap().doc;
    assert_eq!(inline_bounds(&art_text(&s, "B")), d.symbol_art_bounds("A"));
    assert_ne!(d.symbol_art_bounds("B"), b0);
    assert_eq!(inline_bounds(&art_text(&s, "C")), d.symbol_art_bounds("B"));
}

/// A symbol whose art shows the symbol itself inline, directly or through another symbol, is an
/// empty slot, and stays one through later edits.
#[test]
fn a_symbol_that_shows_itself_inline_is_an_empty_slot() {
    let mut s = session();
    for name in ["Loop", "P", "Q"] {
        circle_symbol(&mut s, name, 10.0);
    }
    for (name, shows) in [("Loop", "Loop"), ("P", "Q"), ("Q", "P")] {
        let t = text(&mut s, "ab");
        s.execute("text.insertInline", &json!({"id": t.0, "at": 1, "symbol": shows})).unwrap();
        s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
        s.execute("symbol.update", &json!({"name": name})).unwrap();
    }
    let line = text(&mut s, "x");
    s.execute("text.insertInline", &json!({"id": line.0, "at": 0, "symbol": "P"})).unwrap();
    for _ in 0..2 {
        for name in ["Loop", "P", "Q"] {
            assert_eq!(inline_bounds(&art_text(&s, name)), None, "{name}");
            assert_eq!(s.doc().unwrap().doc.inline_art_bounds(name), None, "{name}");
        }
        assert_eq!(inline_bounds(&obj(&s, line)), None);
        s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    }
    let _ = s.execute("document.serialize", &json!({"format": "png"})).unwrap();
    // Redefined without the inline graphic, Q is on no cycle, and neither is P.
    redefine_as_wide_ellipse(&mut s, "Q");
    let d = &s.doc().unwrap().doc;
    assert!(d.inline_art_bounds("P").is_some());
    assert_eq!(inline_bounds(&obj(&s, line)), d.symbol_art_bounds("P"));
    assert_eq!(inline_bounds(&art_text(&s, "P")), d.symbol_art_bounds("Q"));
    assert_eq!(d.inline_art_bounds("Loop"), None);
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
