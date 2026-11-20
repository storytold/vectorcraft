//! Inline graphics in text: `text.insertInline`, editing over them, undo, following their symbol.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::text::INLINE_CHAR;
use vectorcraft_doc::{Node, NodeId, NodeKind, TextObject};
use vectorcraft_geom::{Affine, Rect};
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

/// A rectangle `r` filled black, without a stroke.
fn filled_rect(d: &mut vectorcraft_doc::Document, r: Rect) -> Arc<Node> {
    let look = vectorcraft_doc::Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0);
    Arc::new(Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(r), look))
}

/// An instance of symbol `name` placed by `xf`.
fn symbol_instance(d: &mut vectorcraft_doc::Document, name: &str, xf: Affine) -> Arc<Node> {
    Arc::new(Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: name.into(), xf }))
}

/// Gives symbol `name` the art `art` in a group, adding the symbol when there is none of that name.
fn set_symbol_art(d: &mut vectorcraft_doc::Document, name: &str, art: Vec<Arc<Node>>) {
    let art = Arc::new(Node::group(d.alloc_id(), art));
    match d.symbols.iter_mut().find(|x| x.name == name) {
        Some(x) => x.art = art,
        None => d.symbols.push(vectorcraft_doc::Symbol { dynamic: false, name: name.into(), art }),
    }
}

/// An inline graphic of a symbol whose art holds an instance of another symbol is sized by that
/// symbol's art, and follows it when it changes, through undo, redo and a saved copy.
#[test]
fn an_inline_graphic_follows_the_art_of_a_symbol_nested_in_its_symbol() {
    let mut s = session();
    s.edit("Symbols", |d, _| {
        let far = filled_rect(d, Rect::new(100.0, 0.0, 160.0, 20.0));
        set_symbol_art(d, "Far", vec![far]);
        let inner = symbol_instance(d, "Far", Affine::IDENTITY);
        set_symbol_art(d, "Holder", vec![inner]);
        Ok(())
    })
    .unwrap();
    let id = text(&mut s, "ab");
    let w0 = obj(&s, id).bounds().unwrap().width();
    s.execute("text.insertInline", &json!({"id": id.0, "at": 1, "symbol": "Holder", "scale": 1})).unwrap();
    let wide = Some(Rect::new(100.0, 0.0, 160.0, 20.0));
    let t = obj(&s, id);
    assert_eq!(inline_bounds(&t), wide);
    // 20 pt tall at 20 pt type and 3 times as wide: the text grows by 60 pt.
    let w1 = t.bounds().unwrap().width();
    assert!((w1 - w0 - 60.0).abs() < 1.5, "{w0} → {w1}");
    // Far redefined as a 10 × 40 rectangle: the graphic is 20 pt tall and 5 pt wide.
    s.edit("Redefine Far", |d, _| {
        let tall = filled_rect(d, Rect::new(0.0, 0.0, 10.0, 40.0));
        set_symbol_art(d, "Far", vec![tall]);
        Ok(())
    })
    .unwrap();
    let tall = Some(Rect::new(0.0, 0.0, 10.0, 40.0));
    let t = obj(&s, id);
    assert_eq!(inline_bounds(&t), tall);
    assert_eq!(s.doc().unwrap().doc.symbol_art_bounds("Holder"), tall);
    let w2 = t.bounds().unwrap().width();
    assert!((w2 - w0 - 5.0).abs() < 1.5, "{w0} → {w2}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(inline_bounds(&obj(&s, id)), wide);
    assert!((obj(&s, id).bounds().unwrap().width() - w1).abs() < 1e-6);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(inline_bounds(&obj(&s, id)), tall);
    // Opening a saved copy measures it the same way.
    let doc = (*s.doc().unwrap().doc).clone();
    let mut back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    back.title = "copy".into();
    let mut s2 = Session::new();
    s2.add_document(back, None);
    assert_eq!(inline_bounds(&obj(&s2, id)), tall);
    assert!((obj(&s2, id).bounds().unwrap().width() - w2).abs() < 1e-6);
}

/// Symbol C shows Holder inline, Holder holds an instance of B, and B shows A inline: with the
/// symbols listed C, Holder, B, A, a change to A reaches C through B's and Holder's new sizes.
#[test]
fn symbols_are_resolved_after_the_symbols_their_instances_show() {
    let mut s = session();
    circle_symbol(&mut s, "A", 10.0);
    let t = text(&mut s, "z");
    s.execute("text.insertInline", &json!({"id": t.0, "at": 0, "symbol": "A"})).unwrap();
    s.execute("symbol.new", &json!({"name": "B", "ids": [t.0]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    s.edit("Holder", |d, _| {
        let inner = symbol_instance(d, "B", Affine::IDENTITY);
        set_symbol_art(d, "Holder", vec![inner]);
        Ok(())
    })
    .unwrap();
    let t = text(&mut s, "z");
    s.execute("text.insertInline", &json!({"id": t.0, "at": 0, "symbol": "Holder"})).unwrap();
    s.execute("symbol.new", &json!({"name": "C", "ids": [t.0]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    s.edit("Reverse the symbols", |d, _| {
        d.symbols.reverse();
        Ok(())
    })
    .unwrap();
    let h0 = s.doc().unwrap().doc.symbol_art_bounds("Holder");
    redefine_as_wide_ellipse(&mut s, "A");
    let d = &s.doc().unwrap().doc;
    assert_ne!(d.symbol_art_bounds("Holder"), h0);
    assert_eq!(inline_bounds(&art_text(&s, "C")), d.symbol_art_bounds("Holder"));
}

/// Symbol A holds an instance of B, and B's type shows A inline: the size of each would depend on
/// itself, so both are empty slots, and stay so through later edits.
#[test]
fn a_symbol_that_shows_itself_through_an_instance_is_an_empty_slot() {
    let mut s = session();
    circle_symbol(&mut s, "A", 10.0);
    let t = text(&mut s, "ab");
    s.execute("text.insertInline", &json!({"id": t.0, "at": 1, "symbol": "A"})).unwrap();
    s.execute("symbol.new", &json!({"name": "B", "ids": [t.0]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    s.edit("A holds B", |d, _| {
        let inner = symbol_instance(d, "B", Affine::IDENTITY);
        set_symbol_art(d, "A", vec![inner]);
        Ok(())
    })
    .unwrap();
    let line = text(&mut s, "x");
    s.execute("text.insertInline", &json!({"id": line.0, "at": 0, "symbol": "A"})).unwrap();
    for _ in 0..2 {
        for name in ["A", "B"] {
            assert_eq!(s.doc().unwrap().doc.inline_art_bounds(name), None, "{name}");
        }
        assert_eq!(inline_bounds(&art_text(&s, "B")), None);
        assert_eq!(inline_bounds(&obj(&s, line)), None);
        s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    }
    let _ = s.execute("document.serialize", &json!({"format": "png"})).unwrap();
    // Redefined without the instance, A is on no cycle.
    redefine_as_wide_ellipse(&mut s, "A");
    let d = &s.doc().unwrap().doc;
    assert!(d.inline_art_bounds("A").is_some());
    assert_eq!(inline_bounds(&art_text(&s, "B")), d.symbol_art_bounds("A"));
    assert_eq!(inline_bounds(&obj(&s, line)), d.symbol_art_bounds("A"));
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

/// `groups` groups of `each` 5 × 5 black rectangles added to the first layer in one edit; returns
/// the ids of each group's rectangles.
fn rect_groups(s: &mut Session, groups: usize, each: usize) -> Vec<Vec<NodeId>> {
    s.edit("Rectangles", |d, _| {
        let look = vectorcraft_doc::Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0);
        let mut ids = vec![];
        let mut made = vec![];
        for g in 0..groups {
            let kids: Vec<Arc<Node>> = (0..each)
                .map(|i| {
                    let r = Rect::new(i as f64 * 6.0, g as f64 * 6.0, i as f64 * 6.0 + 5.0, g as f64 * 6.0 + 5.0);
                    Arc::new(Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(r), look.clone()))
                })
                .collect();
            ids.push(kids.iter().map(|k| k.id).collect());
            made.push(Arc::new(Node::group(d.alloc_id(), kids)));
        }
        match Arc::make_mut(&mut d.layers[0]).children_mut() {
            Some(kids) => kids.extend(made),
            None => panic!("the first layer holds art"),
        }
        Ok(ids)
    })
    .unwrap()
}

/// Runs `step` on `full`, with every refresh of inline graphics taking the full pass, then on
/// `fast`; checks that both leave the same document and that each refresh returned the same changed
/// texts. Returns whether each of `fast`'s refreshes checked only the changed layer trees.
fn same_as_full_pass(full: &mut Session, fast: &mut Session, step: &dyn Fn(&mut Session)) -> Vec<bool> {
    let take = || cmd::inline::PASSES.with(|p| std::mem::take(&mut *p.borrow_mut()));
    take();
    cmd::inline::FULL_PASS.set(true);
    step(full);
    cmd::inline::FULL_PASS.set(false);
    let want = take();
    step(fast);
    let got = take();
    assert_eq!(*full.doc().unwrap().doc, *fast.doc().unwrap().doc);
    let ids = |passes: &[(bool, Vec<NodeId>)]| passes.iter().map(|(_, ids)| ids.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&got), ids(&want));
    assert!(want.iter().all(|(partial, _)| !partial));
    got.iter().map(|(partial, _)| *partial).collect()
}

/// After an edit, inline graphics are checked again only in the layer trees the edit changed, and
/// the document and the changed texts are those of the full pass; a change to a symbol's art, its
/// natural size or anything else a symbol is measured with checks them all.
#[test]
fn refreshing_only_changed_trees_matches_the_full_pass() {
    // Both sessions lay out type, and the test rescans the fonts: a font another test adds between
    // the full pass and the fast one would change how the type resolves in only one of them.
    let _fonts = crate::adding_fonts();
    let setup = |s: &mut Session| {
        circle_symbol(s, "Tap", 10.0);
        circle_symbol(s, "Dot", 6.0);
        let groups = rect_groups(s, 20, 10);
        let id = text(s, "Tap here");
        s.execute("text.insertInline", &json!({"id": id.0, "at": 0, "symbol": "Tap"})).unwrap();
        // Type showing Dot inline becomes the opacity mask of a rectangle.
        let masked = s.execute("shape.rectangle", &json!({"x": 300, "y": 300, "width": 50, "height": 50})).unwrap()["id"].clone();
        let mask = text(s, "Dot");
        s.execute("text.insertInline", &json!({"id": mask.0, "at": 0, "symbol": "Dot"})).unwrap();
        s.execute("transparency.makeOpacityMask", &json!({"ids": [masked, mask.0]})).unwrap();
        (groups, id)
    };
    let (mut full, mut fast) = (session(), session());
    let (groups, id) = setup(&mut full);
    assert_eq!(setup(&mut fast), (groups.clone(), id));
    let rect = groups[3][4].0;
    let len = obj(&fast, id).plain_text().len();
    let only_changed = |passes: &[bool]| !passes.is_empty() && passes.iter().all(|&p| p);
    let all = |passes: &[bool]| !passes.is_empty() && passes.iter().all(|&p| !p);

    // A rectangle beside 19 unchanged groups moves.
    let passes = same_as_full_pass(&mut full, &mut fast, &|s| {
        s.execute("select.set", &json!({"ids": [rect]})).unwrap();
        s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    });
    assert!(only_changed(&passes), "{passes:?}");
    // Type with an inline graphic is edited, gets a second one, and loses it again.
    let passes = same_as_full_pass(&mut full, &mut fast, &|s| {
        s.execute("text.editRange", &json!({"id": id.0, "start": len, "end": len, "insert": "!"})).unwrap();
        s.execute("text.insertInline", &json!({"id": id.0, "at": len, "symbol": "Dot"})).unwrap();
        s.execute("text.editRange", &json!({"id": id.0, "start": len, "end": len + INLINE_CHAR.len_utf8(), "insert": ""})).unwrap();
    });
    assert!(only_changed(&passes), "{passes:?}");
    // Dot's art is redefined: the type in the opacity mask follows it.
    let passes = same_as_full_pass(&mut full, &mut fast, &|s| redefine_as_wide_ellipse(s, "Dot"));
    assert!(passes.contains(&false), "{passes:?}");
    // Tap's natural size changes, its art unchanged.
    let passes = same_as_full_pass(&mut full, &mut fast, &|s| {
        s.edit("Size", |d, _| {
            d.unknown.insert(vectorcraft_doc::SYMBOL_SIZES.into(), json!({ "Tap": [40.0, 20.0] }));
            Ok(())
        })
        .unwrap();
    });
    assert!(all(&passes), "{passes:?}");
    let (b, want) = (inline_bounds(&obj(&fast, id)), fast.doc().unwrap().doc.symbol_art_bounds("Tap"));
    assert!(b.is_some() && b == want, "{b:?} {want:?}");
    // 9-slice guides for Tap, its art unchanged.
    let passes = same_as_full_pass(&mut full, &mut fast, &|s| {
        s.execute("symbol.options", &json!({"name": "Tap", "nineSlice": true})).unwrap();
    });
    assert!(all(&passes), "{passes:?}");
    // Symbol editing mode, entered and left, a rectangle moved inside it.
    same_as_full_pass(&mut full, &mut fast, &|s| {
        s.execute("symbol.edit", &json!({"name": "Dot"})).unwrap();
        s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 4, "height": 4})).unwrap();
        s.execute("symbol.exitEditing", &json!({})).unwrap();
    });
    // Undo and redo, then edits after them.
    same_as_full_pass(&mut full, &mut fast, &|s| {
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("select.set", &json!({"ids": [rect]})).unwrap();
        s.execute("object.move", &json!({"dx": 0, "dy": 5})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        s.execute("edit.redo", &json!({})).unwrap();
        s.execute("object.move", &json!({"dx": -5, "dy": 0})).unwrap();
    });
    // Fonts scanned again, then another move.
    same_as_full_pass(&mut full, &mut fast, &|s| {
        s.execute("text.rescanFonts", &json!({})).unwrap();
        s.execute("select.set", &json!({"ids": [rect]})).unwrap();
        s.execute("object.move", &json!({"dx": 1, "dy": 1})).unwrap();
    });
}

/// The CPU time this thread has used, from Linux's scheduler statistics; `None` elsewhere.
fn thread_cpu() -> Option<std::time::Duration> {
    let stat = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
    stat.split_whitespace().next()?.parse().ok().map(std::time::Duration::from_nanos)
}

/// The time `f` takes in milliseconds: this thread's CPU time where Linux reports it, otherwise the
/// wall clock.
fn time_ms(f: impl FnOnce()) -> f64 {
    let d = match thread_cpu() {
        Some(before) => {
            f();
            thread_cpu().map_or(std::time::Duration::ZERO, |after| after.saturating_sub(before))
        }
        None => {
            let start = std::time::Instant::now();
            f();
            start.elapsed()
        }
    };
    d.as_secs_f64() * 1000.0
}

/// The median, fastest and slowest of `runs`, as text.
fn spread(runs: &mut [f64]) -> String {
    runs.sort_by(f64::total_cmp);
    format!("{:>9.3} ({:.3}-{:.3})", runs[runs.len() / 2], runs[0], runs[runs.len() - 1])
}

/// Timings of the refresh of inline graphics after one rectangle moves, with the full pass and with
/// only the changed layer trees checked, in documents of 5,000, 50,000 and 200,000 rectangles in 100
/// groups, with three symbols and one line of type showing one of them inline: the refresh alone,
/// on copies of the same edit, and the whole `object.move` command. 7 runs each, taking turns going
/// first. Ignored by default; it prints the median, fastest and slowest run and sets no bound on
/// them: `cargo test --release -p vectorcraft-engine --lib inline_refresh_perf -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark: run with --ignored --nocapture"]
fn inline_refresh_perf() {
    const RUNS: usize = 7;
    eprintln!("inline_refresh_perf: {}", if thread_cpu().is_some() { "thread CPU time" } else { "wall clock" });
    eprintln!("inline_refresh_perf: {:>7} {:<14} {:>28} {:>28}", "objects", "case", "full pass ms (min-max)", "changed trees ms (min-max)");
    for n in [5_000, 50_000, 200_000] {
        let mut s = session();
        for name in ["A", "B", "C"] {
            circle_symbol(&mut s, name, 10.0);
        }
        let groups = rect_groups(&mut s, 100, n / 100);
        let id = text(&mut s, "Type");
        s.execute("text.insertInline", &json!({"id": id.0, "at": 0, "symbol": "B"})).unwrap();
        let rect = groups[50][7];
        s.execute("select.set", &json!({"ids": [rect.0]})).unwrap();
        s.execute("object.move", &json!({"dx": 1, "dy": 0})).unwrap();
        // The refresh alone, after the same move made on a copy of the resolved document.
        let resolved = s.doc().unwrap().inline_resolved.clone().unwrap();
        let before = resolved.0.clone();
        // The rectangle and the group and layer above it become new nodes, as a move makes them.
        let mut after = (*before).clone();
        assert!(after.node_mut(rect).is_some());
        // Each run times EACH refreshes, or EACH commands, and gives the time of one.
        const EACH: usize = 20;
        let (mut full, mut partial) = (vec![], vec![]);
        for run in 0..RUNS {
            let (mut a, mut b) = (vec![after.clone(); EACH], vec![after.clone(); EACH]);
            let mut go_full = || {
                cmd::inline::FULL_PASS.set(true);
                full.push(
                    time_ms(|| {
                        for d in &mut a {
                            cmd::inline::refresh(Some(&resolved), &before, d);
                        }
                    }) / EACH as f64,
                );
                cmd::inline::FULL_PASS.set(false);
            };
            let mut go_partial = || {
                partial.push(
                    time_ms(|| {
                        for d in &mut b {
                            cmd::inline::refresh(Some(&resolved), &before, d);
                        }
                    }) / EACH as f64,
                );
            };
            if run % 2 == 0 {
                go_full();
                go_partial();
            } else {
                go_partial();
                go_full();
            }
            assert_eq!(a, b);
        }
        eprintln!("inline_refresh_perf: {n:>7} {:<14} {:>28} {:>28}", "refresh", spread(&mut full), spread(&mut partial));
        // The whole command, moving the rectangle back and forth.
        let (mut full, mut partial) = (vec![], vec![]);
        for run in 0..2 * RUNS {
            let take_full = (run / 2) % 2 == run % 2;
            cmd::inline::FULL_PASS.set(take_full);
            let ms = time_ms(|| {
                for i in 0..EACH {
                    let dx = if i % 2 == 0 { -1 } else { 1 };
                    s.execute("object.move", &json!({"dx": dx, "dy": 0})).unwrap();
                }
            }) / EACH as f64;
            cmd::inline::FULL_PASS.set(false);
            if take_full { full.push(ms) } else { partial.push(ms) }
        }
        eprintln!("inline_refresh_perf: {n:>7} {:<14} {:>28} {:>28}", "object.move", spread(&mut full), spread(&mut partial));
        cmd::inline::PASSES.with(|p| p.borrow_mut().clear());
    }
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
