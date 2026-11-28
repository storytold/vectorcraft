//! The text engine's newer features combined through commands: Auto Size and Shrink Text to Fit
//! with inline graphics, per-paragraph attributes, the paragraph composer and vertical
//! alignment; editing and undo across them; and a save round trip with all of them on.
//! (`vectorcraft_text`'s `tests_combos` covers the layout side.)

use serde_json::{Value, json};
use vectorcraft_doc::text::INLINE_CHAR;
use vectorcraft_doc::{AreaFit, Composer, Justify, NodeKind, TextKind, TextObject, VerticalAlign};
use vectorcraft_geom::Rect;

use crate::*;

const STORY: &str = "The quick brown fox jumps over the lazy dog. A second sentence follows the first one here. Then a third one ends it.";

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

/// A symbol `name`: a circle `d` points across, its instance deleted.
fn circle_symbol(s: &mut Session, name: &str, d: f64) {
    s.execute("shape.ellipse", &json!({"x": 600, "y": 500, "width": d, "height": d})).unwrap();
    s.execute("symbol.new", &json!({"name": name})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
}

/// Area type at (40, 40), `w` × `h`, size 12, with `extra` area options; selected.
fn area(s: &mut Session, w: f64, h: f64, text: &str, extra: Value) -> NodeId {
    let mut a = json!({"width": w, "height": h});
    if let (Some(a), Some(e)) = (a.as_object_mut(), extra.as_object()) {
        a.extend(e.clone());
    }
    let id = NodeId(s.execute("text.create", &json!({"x": 40, "y": 40, "size": 12, "area": a, "text": text})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    id
}

fn text(s: &Session, id: NodeId) -> TextObject {
    match &s.doc().unwrap().doc.node(id).unwrap().kind {
        NodeKind::Text(t) => (**t).clone(),
        k => panic!("not text: {k:?}"),
    }
}

fn frame(s: &Session, id: NodeId) -> Rect {
    match text(s, id).kind {
        TextKind::Area { frame } => frame.bounds().unwrap(),
        k => panic!("not area type: {k:?}"),
    }
}

fn lay(t: &TextObject) -> vectorcraft_text::TextLayout {
    vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t)
}

/// Auto Size's frame is tight: the text fits, and a point less overflows.
fn assert_tight(s: &Session, id: NodeId, what: &str) {
    let t = text(s, id);
    assert!(!lay(&t).overflow, "{what}: the frame holds the text");
    let b = frame(s, id);
    let mut less = t.clone();
    less.area.fit = AreaFit::None;
    less.kind = TextKind::Area {
        frame: vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::Shape::to_path(&Rect::new(b.x0, b.y0, b.x1, b.y1 - 1.0), 0.1)),
    };
    assert!(lay(&less).overflow, "{what}: a point less overflows ({b:?})");
}

fn undo(s: &mut Session) {
    s.execute("edit.undo", &json!({})).unwrap();
}

fn redo(s: &mut Session) {
    s.execute("edit.redo", &json!({})).unwrap();
}

fn query(s: &mut Session) -> Value {
    s.execute("text.areaOptions", &json!({})).unwrap()
}

#[test]
fn auto_height_x_inline_graphics() {
    let mut s = session();
    circle_symbol(&mut s, "Tap", 10.0);
    let id = area(&mut s, 200.0, 300.0, "Tap: add one mana.", json!({"fit": "autoHeight"}));
    let one = frame(&s, id);
    assert_tight(&s, id, "one line");
    // Art taller than the line makes the line, and so the frame, taller: in the same undo step.
    let n = s.doc().unwrap().history.undo.len();
    s.execute("text.insertInline", &json!({"id": id.0, "at": 0, "symbol": "Tap", "scale": 3})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), n + 1);
    let tall = frame(&s, id);
    assert!(tall.height() > one.height() + 5.0, "{one:?} → {tall:?}");
    assert_tight(&s, id, "tall art");
    // Enough art to wrap: another line.
    for _ in 0..12 {
        s.execute("text.insertInline", &json!({"id": id.0, "at": 0, "symbol": "Tap", "scale": 1})).unwrap();
    }
    assert!(lay(&text(&s, id)).lines.len() > 1);
    assert_tight(&s, id, "wrapped art");
    // Deleting the art shrinks it back; undo restores each step exactly.
    let wrapped = frame(&s, id);
    let all = text(&s, id).plain_text().rfind(INLINE_CHAR).unwrap() + INLINE_CHAR.len_utf8();
    s.execute("text.editRange", &json!({"id": id.0, "start": 0, "end": all})).unwrap();
    assert_eq!(text(&s, id).plain_text(), "Tap: add one mana.");
    assert_eq!(frame(&s, id), one);
    undo(&mut s);
    assert_eq!(frame(&s, id), wrapped);
    for _ in 0..13 {
        undo(&mut s);
    }
    assert_eq!(frame(&s, id), one);
    redo(&mut s);
    assert_eq!(frame(&s, id), tall);
}

#[test]
fn auto_height_x_paragraph_attrs() {
    let mut s = session();
    let id = area(&mut s, 200.0, 300.0, "One\nTwo", json!({"fit": "autoHeight"}));
    let h0 = frame(&s, id).height();
    // Space before on the second paragraph only: the frame grows by exactly that.
    s.execute("text.setFormat", &json!({"ids": [id.0], "spaceBefore": 30, "start": 4, "end": 4})).unwrap();
    let t = text(&s, id);
    assert_eq!((t.para_at(0).space_before, t.para_at(1).space_before), (0.0, 30.0));
    let h1 = frame(&s, id).height();
    assert!((h1 - h0 - 30.0).abs() < 1e-6, "{h0} → {h1}");
    assert_tight(&s, id, "space before");
    // Return in the spaced paragraph: the new one inherits the spacing, the frame follows.
    s.execute("text.editRange", &json!({"id": id.0, "start": 7, "end": 7, "insert": "\nThree"})).unwrap();
    let t = text(&s, id);
    assert_eq!(t.paragraph_styles().iter().map(|p| p.space_before).collect::<Vec<_>>(), [0.0, 30.0, 30.0]);
    assert!(frame(&s, id).height() > h1 + 30.0);
    assert_tight(&s, id, "three paragraphs");
    // Merging paragraphs: the first one's attributes win, the frame shrinks.
    s.execute("text.editRange", &json!({"id": id.0, "start": 3, "end": 4})).unwrap();
    let t = text(&s, id);
    assert_eq!(t.paragraph_styles().iter().map(|p| p.space_before).collect::<Vec<_>>(), [0.0, 30.0]);
    assert_tight(&s, id, "merged");
    undo(&mut s);
    undo(&mut s);
    assert!((frame(&s, id).height() - h1).abs() < 1e-9);
    undo(&mut s);
    assert!((frame(&s, id).height() - h0).abs() < 1e-9);
    assert!(text(&s, id).paras.is_empty(), "back to one style for all");
}

#[test]
fn auto_height_x_composer() {
    // Switching the composer of one paragraph rebreaks it; Auto Size follows either way.
    let mut s = session();
    let story = format!("{STORY} {STORY}\n{STORY}");
    let id = area(&mut s, 130.0, 100.0, &story, json!({"fit": "autoHeight"}));
    assert_tight(&s, id, "every-line");
    for composer in ["singleLine", "everyLine", "singleLine"] {
        s.execute("text.setFormat", &json!({"ids": [id.0], "composer": composer, "start": 0, "end": 0})).unwrap();
        let t = text(&s, id);
        assert_eq!(t.para_at(1).composer, Composer::EveryLine, "the second paragraph keeps its own");
        assert_tight(&s, id, composer);
    }
    s.execute("text.setFormat", &json!({"ids": [id.0], "composer": "singleLine"})).unwrap();
    s.execute("text.setStyle", &json!({"id": id.0, "justify": "justifyAll"})).unwrap();
    assert!(text(&s, id).paragraph_styles().iter().all(|p| p.composer == Composer::SingleLine && p.justify == Justify::JustifyAll));
    assert_tight(&s, id, "justified");
}

#[test]
fn auto_height_x_vertical_align() {
    // An Auto Size frame has no space to spare: every alignment sets the lines as Top does, and
    // the frame is as tall as with Top.
    let mut s = session();
    let id = area(&mut s, 160.0, 100.0, STORY, json!({"fit": "autoHeight"}));
    let top_frame = frame(&s, id);
    let top = lay(&text(&s, id));
    for a in ["center", "bottom", "justify"] {
        s.execute("text.areaOptions", &json!({"verticalAlign": a})).unwrap();
        assert_eq!(frame(&s, id), top_frame, "{a}");
        let l = lay(&text(&s, id));
        for (li, l0) in l.lines.iter().zip(&top.lines) {
            assert!((li.baseline - l0.baseline).abs() < 1e-6, "{a}: {} vs {}", li.baseline, l0.baseline);
        }
        assert_tight(&s, id, a);
    }
    // Editing with the alignment on keeps the frame tight.
    s.execute("text.editRange", &json!({"id": id.0, "start": 0, "end": 0, "insert": format!("{STORY} ")})).unwrap();
    assert_eq!(text(&s, id).area.vertical_align, VerticalAlign::Justify);
    assert_tight(&s, id, "longer, justified");
}

#[test]
fn shrink_text_x_inline_graphics_and_paragraphs() {
    let mut s = session();
    circle_symbol(&mut s, "Dot", 8.0);
    let id = area(&mut s, 160.0, 60.0, "Short rules.\nFlavor.", json!({"fit": "shrinkText", "fitMinPercent": 20}));
    assert_eq!(query(&mut s)["fitScale"], 1.0);
    // Inline art and per-paragraph spacing push it over; shrinking makes it fit.
    s.execute("text.setFormat", &json!({"ids": [id.0], "spaceBefore": 12, "start": 13, "end": 13})).unwrap();
    for i in 0..10 {
        s.execute("text.insertInline", &json!({"id": id.0, "at": 0, "symbol": "Dot", "scale": 1.0 + i as f64 * 0.2})).unwrap();
        s.execute("text.editRange", &json!({"id": id.0, "start": 0, "end": 0, "insert": "Add "})).unwrap();
    }
    let q = query(&mut s);
    assert_eq!(q["overflow"], false, "{q}");
    let f = q["fitScale"].as_f64().unwrap();
    assert!((0.2..1.0).contains(&f), "{f}");
    let t = text(&s, id);
    assert_eq!(t.paragraph_count(), 2);
    assert_eq!(t.para_at(1).space_before, 12.0, "inserting in the first paragraph keeps the second's attributes");
    assert_eq!(t.runs[0].style.size, 12.0, "the stored size stays");
    let l = lay(&t);
    assert_eq!(l.inlines.len(), 10);
    assert!(l.inlines.iter().all(|i| i.bounds.height() < 12.0 * 2.8 * f + 1e-6));
    // Undo walks the fit back up.
    for _ in 0..20 {
        undo(&mut s);
    }
    assert_eq!(query(&mut s)["fitScale"], 1.0);
}

#[test]
fn paragraph_attrs_x_inline_editing() {
    // An inline graphic opening a paragraph belongs to it: inserting it keeps the paragraph's
    // attributes, deleting it with the paragraph break merges into the paragraph before.
    let mut s = session();
    circle_symbol(&mut s, "Tap", 10.0);
    let id = area(&mut s, 300.0, 200.0, "Rules\nFlavor", json!({}));
    s.execute("text.setStyle", &json!({"id": id.0, "justify": "center", "start": 6, "end": 6})).unwrap();
    s.execute("text.setFormat", &json!({"ids": [id.0], "composer": "singleLine", "start": 6, "end": 6})).unwrap();
    s.execute("text.insertInline", &json!({"id": id.0, "at": 6, "symbol": "Tap"})).unwrap();
    let t = text(&s, id);
    assert_eq!(t.plain_text(), format!("Rules\n{INLINE_CHAR}Flavor"));
    assert_eq!((t.para_at(0).justify, t.para_at(1).justify), (Justify::Auto, Justify::Center));
    assert_eq!(t.para_at(1).composer, Composer::SingleLine);
    // The art's line is centred.
    let l = lay(&t);
    let g = &l.glyphs[l.inlines[0].glyph];
    let line = &l.lines[g.line];
    assert!(((line.x0 + line.x1) * 0.5 - 150.0).abs() < 1e-6, "centred in the 300 pt frame: {line:?}");
    assert!((l.inlines[0].bounds.x0 - g.origin.x).abs() < 1e-6);
    // Return just before the art: the new paragraph (holding the art) continues the centred one.
    s.execute("text.editRange", &json!({"id": id.0, "start": 6, "end": 6, "insert": "Text\n"})).unwrap();
    let t = text(&s, id);
    assert_eq!(t.paragraph_styles().iter().map(|p| p.justify).collect::<Vec<_>>(), [Justify::Auto, Justify::Center, Justify::Center]);
    undo(&mut s);
    // Delete the break and the art together: one paragraph, the first one's attributes.
    let end = 6 + INLINE_CHAR.len_utf8();
    s.execute("text.editRange", &json!({"id": id.0, "start": 5, "end": end})).unwrap();
    let t = text(&s, id);
    assert_eq!(t.plain_text(), "RulesFlavor");
    assert!(t.paras.is_empty() && t.para.justify == Justify::Auto && t.para.composer == Composer::EveryLine);
    assert!(t.runs.iter().all(|r| r.inline.is_none()));
    undo(&mut s);
    let t = text(&s, id);
    assert_eq!(t.plain_text(), format!("Rules\n{INLINE_CHAR}Flavor"));
    assert_eq!(t.para_at(1).justify, Justify::Center);
    assert_eq!(t.runs.iter().filter(|r| r.inline.is_some()).count(), 1);
}

#[test]
fn vertical_align_x_inline_graphics_in_the_document() {
    // The art the renderer draws (the layout's inline placements) moves with centred lines.
    let mut s = session();
    circle_symbol(&mut s, "Tap", 10.0);
    let id = area(&mut s, 300.0, 200.0, "Add one mana.", json!({}));
    s.execute("text.insertInline", &json!({"id": id.0, "at": 0, "symbol": "Tap"})).unwrap();
    let top = lay(&text(&s, id));
    s.execute("text.areaOptions", &json!({"verticalAlign": "center"})).unwrap();
    let l = lay(&text(&s, id));
    let d = l.lines[0].baseline - top.lines[0].baseline;
    assert!(d > 50.0, "{d}");
    assert!((l.inlines[0].bounds.y0 - top.inlines[0].bounds.y0 - d).abs() < 1e-6, "{:?} vs {:?}", l.inlines[0].bounds, top.inlines[0].bounds);
    // The cached bounds (hit testing, selection) include the moved art.
    let cached = text(&s, id).cached_bounds.unwrap();
    assert!(cached.contains(l.inlines[0].bounds.center()), "{cached:?}");
}

#[test]
fn every_feature_saves_and_reopens() {
    let mut s = session();
    circle_symbol(&mut s, "Tap", 10.0);
    let id = area(
        &mut s,
        220.0,
        70.0,
        &format!(": Add one mana of any color. {STORY}\n{STORY}"),
        json!({"verticalAlign": "center", "fit": "shrinkText", "fitMinPercent": 30}),
    );
    s.execute("text.insertInline", &json!({"id": id.0, "at": 0, "symbol": "Tap", "scale": 0.8, "shift": 0.5})).unwrap();
    let flavor = text(&s, id).plain_text().find('\n').unwrap() + 1;
    s.execute("text.setFormat", &json!({"ids": [id.0], "spaceBefore": 8, "composer": "singleLine", "start": flavor, "end": flavor})).unwrap();
    s.execute("text.setStyle", &json!({"id": id.0, "justify": "justifyAll", "start": flavor, "end": flavor})).unwrap();
    s.execute("text.setStyle", &json!({"id": id.0, "tracking": 20})).unwrap();
    let t = text(&s, id);
    let l = lay(&t);
    assert!(!l.overflow && l.fit_scale < 1.0, "{}", l.fit_scale);
    assert_eq!(l.inlines.len(), 1);
    let doc = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    let NodeKind::Text(t2) = &back.node(id).unwrap().kind else { panic!("not text") };
    let mut t2 = (**t2).clone();
    // Resolved art bounds and layout caches aren't saved: resolve them as opening does.
    back.resolve_text_inline(&mut t2);
    assert_eq!(t2.runs, t.runs);
    assert_eq!(t2.paragraph_styles(), t.paragraph_styles());
    assert_eq!(t2.area, t.area);
    let l2 = lay(&t2);
    assert_eq!((l2.fit_scale, l2.lines.len()), (l.fit_scale, l.lines.len()));
    assert_eq!(l2.inlines.iter().map(|i| i.bounds).collect::<Vec<_>>(), l.inlines.iter().map(|i| i.bounds).collect::<Vec<_>>());
}
