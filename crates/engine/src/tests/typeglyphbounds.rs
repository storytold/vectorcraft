//! Preferences › Type › Measure Type by Glyph Bounds (`typeGlyphBounds`): the bounding box, the
//! Transform panel's values and `object.setBounds` measure point type selected on its own by the
//! ink of its glyphs instead of its em box.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind, OrientedBox, TextObject};
use vectorcraft_geom::Rect;

use crate::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn id(v: Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A new document holding point type made by `text.create` with `params`, selected.
fn with_type(params: Value) -> (Session, NodeId) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 800, "height": 800}));
    let t = id(run(&mut s, "text.create", params));
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    (s, t)
}

fn glyphs(s: &mut Session, on: bool) {
    run(s, "prefs.set", json!({"values": {"typeGlyphBounds": on}}));
}

fn text(s: &Session, id: NodeId) -> TextObject {
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("{id:?} is not type") };
    (**t).clone()
}

/// The ink of the type's glyphs on the page, laid out afresh (not from the cache).
fn ink(s: &Session, id: NodeId) -> Rect {
    let t = text(s, id);
    t.xf.transform_rect_bbox(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t).ink_bounds().unwrap())
}

/// The em box (the line box) of point type: what the bounding box measures by default.
fn em(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

fn near(a: Rect, b: Rect, tol: f64) -> bool {
    (a.x0 - b.x0).abs() <= tol && (a.y0 - b.y0).abs() <= tol && (a.x1 - b.x1).abs() <= tol && (a.y1 - b.y1).abs() <= tol
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// The glyph box of `c` in Inter Regular, in font units (x min, y min, x max, y max), and the
/// font's units per em, read from the font file's own `head`, `cmap`, `loca` and `glyf` tables.
fn inter_glyph_box(c: char) -> ([f64; 4], f64) {
    use skrifa::MetadataProvider;
    use skrifa::raw::TableProvider;
    let font = skrifa::FontRef::new(include_bytes!("../../../../assets/fonts/Inter-Regular.ttf")).unwrap();
    let upem = f64::from(font.head().unwrap().units_per_em());
    let gid = font.charmap().map(c).unwrap();
    let glyph = font.loca(None).unwrap().get_glyf(gid, &font.glyf().unwrap()).unwrap().unwrap();
    ([glyph.x_min(), glyph.y_min(), glyph.x_max(), glyph.y_max()].map(f64::from), upem)
}

/// "H" in Inter at 100 pt is drawn with straight lines only, so the box of its outline is the glyph
/// box the font file stores: the bounding box takes its size and place from the font itself.
#[test]
fn point_type_measures_the_glyph_box_the_font_file_stores() {
    let (mut s, t) = with_type(json!({"x": 100, "y": 300, "size": 100, "font": "Inter", "text": "H"}));
    glyphs(&mut s, true);
    let ([x0, y0, x1, y1], upem) = inter_glyph_box('H');
    let k = 100.0 / upem;
    let b = s.transform_box(&[t]).unwrap();
    assert_eq!(b.angle, 0.0);
    // Text space runs down the page from the baseline at y = 300.
    let want = Rect::new(100.0 + x0 * k, 300.0 - y1 * k, 100.0 + x1 * k, 300.0 - y0 * k);
    assert!(near(b.rect, want, 1e-6), "{:?} != {want:?}", b.rect);
    assert!(near(s.transform_bounds(&[t]).unwrap(), want, 1e-6));
    // The em box is taller than the cap height.
    assert!(em(&s, t).height() > b.rect.height() + 10.0, "em box {:?}", em(&s, t));
}

#[test]
fn off_by_default_the_box_is_the_em_box() {
    let (mut s, t) = with_type(json!({"x": 100, "y": 300, "size": 100, "text": "Hg"}));
    assert!(!s.prefs.type_glyph_bounds);
    assert_eq!(s.transform_box(&[t]).unwrap().rect, em(&s, t));
    glyphs(&mut s, true);
    assert!(near(s.transform_box(&[t]).unwrap().rect, ink(&s, t), 1e-9));
    glyphs(&mut s, false);
    assert_eq!(s.transform_box(&[t]).unwrap().rect, em(&s, t));
}

/// A typed height scales the type so its ink takes that height, in one undo step, and the type
/// stays type.
#[test]
fn set_bounds_gives_the_ink_the_typed_size() {
    let (mut s, t) = with_type(json!({"x": 100, "y": 300, "size": 100, "text": "Hg"}));
    glyphs(&mut s, true);
    let xf = text(&s, t).xf;
    let n = undo_len(&s);
    run(&mut s, "object.setBounds", json!({"height": 50}));
    assert_eq!(undo_len(&s), n + 1);
    assert!((ink(&s, t).height() - 50.0).abs() < 1e-6, "{:?}", ink(&s, t));
    assert!(near(s.transform_box(&[t]).unwrap().rect, ink(&s, t), 1e-6));
    assert!(matches!(s.doc().unwrap().doc.node(t).unwrap().kind, NodeKind::Text(_)));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s, t).xf, xf);
}

/// A typed X moves the ink box's reference point there and keeps its size; a typed width gives
/// the ink that width.
#[test]
fn set_bounds_moves_and_sizes_the_ink_box() {
    let (mut s, t) = with_type(json!({"x": 100, "y": 300, "size": 100, "text": "Hg"}));
    glyphs(&mut s, true);
    let before = ink(&s, t);
    run(&mut s, "object.setBounds", json!({"x": 400, "reference": 0}));
    let moved = ink(&s, t);
    assert!((moved.x0 - 400.0).abs() < 1e-6 && (moved.y0 - before.y0).abs() < 1e-6, "{moved:?}");
    assert!((moved.width() - before.width()).abs() < 1e-6 && (moved.height() - before.height()).abs() < 1e-6, "{moved:?}");
    run(&mut s, "object.setBounds", json!({"width": 123}));
    assert!((ink(&s, t).width() - 123.0).abs() < 1e-6, "{:?}", ink(&s, t));
    assert!((ink(&s, t).height() - before.height()).abs() < 1e-6, "{:?}", ink(&s, t));
}

#[test]
fn set_bounds_with_the_current_values_records_no_undo_step() {
    let (mut s, t) = with_type(json!({"x": 100, "y": 300, "size": 100, "text": "Hg"}));
    glyphs(&mut s, true);
    let b = s.transform_box(&[t]).unwrap().rect;
    let n = undo_len(&s);
    run(&mut s, "object.setBounds", json!({"x": b.center().x, "y": b.center().y, "width": b.width(), "height": b.height()}));
    assert_eq!(undo_len(&s), n);
    assert_eq!(s.transform_box(&[t]).unwrap().rect, b);
}

/// A proportional width scales both sides of the ink by the same factor, about the reference
/// point of the ink box (here its top-left corner).
#[test]
fn proportional_width_keeps_the_ink_in_proportion_about_the_reference_point() {
    let (mut s, t) = with_type(json!({"x": 100, "y": 300, "size": 100, "text": "Hg"}));
    glyphs(&mut s, true);
    let before = ink(&s, t);
    run(&mut s, "object.setBounds", json!({"width": before.width() * 2.0, "proportional": true, "reference": 0}));
    let after = ink(&s, t);
    assert!((after.width() - before.width() * 2.0).abs() < 1e-6, "{after:?}");
    assert!((after.height() - before.height() * 2.0).abs() < 1e-6, "{after:?}");
    assert!((after.x0 - before.x0).abs() < 1e-6 && (after.y0 - before.y0).abs() < 1e-6, "{before:?} -> {after:?}");
}

/// Rotated type keeps its own angle, and the box is the size of the ink before the rotation. A
/// skewed object's box is the box of its glyph outlines, not of its mapped ink rectangle.
#[test]
fn rotated_and_skewed_type_measure_their_outlines() {
    let (mut s, flat) = with_type(json!({"x": 100, "y": 300, "size": 100, "text": "Hg"}));
    glyphs(&mut s, true);
    let size = s.transform_box(&[flat]).unwrap().rect.size();
    let t = id(run(&mut s, "text.create", json!({"x": 300, "y": 300, "size": 100, "text": "Hg", "rotation": 30})));
    let b = s.transform_box(&[t]).unwrap();
    assert_eq!(b.angle, s.doc().unwrap().doc.oriented_bounds(&[t], false).unwrap().angle);
    assert!(b.angle != 0.0);
    assert!((b.rect.width() - size.width).abs() < 1e-6 && (b.rect.height() - size.height).abs() < 1e-6, "{b:?} vs {size:?}");

    let o = id(run(&mut s, "text.create", json!({"x": 100, "y": 600, "size": 100, "text": "o"})));
    run(&mut s, "select.set", json!({"ids": [o.0]}));
    run(&mut s, "object.transform", json!({"matrix": [1, 0, 0.6, 1, -360, 0]}));
    let b = s.transform_box(&[o]).unwrap();
    let tx = text(&s, o);
    let lay = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &tx);
    let to_local = OrientedBox { angle: b.angle, rect: Rect::ZERO }.to_doc().inverse();
    let want = lay.ink_bounds_in(to_local * tx.xf).unwrap();
    assert!(near(b.rect, want, 1e-9), "{:?} != {want:?}", b.rect);
    let mapped = (to_local * tx.xf).transform_rect_bbox(lay.ink_bounds().unwrap());
    assert!(mapped.width() > b.rect.width() + 1.0, "a round letter's outline is narrower than its slanted box");
}

/// Type that draws nothing is measured by its em box.
#[test]
fn empty_and_space_only_type_keep_the_em_box() {
    for text in ["", "   "] {
        let (mut s, t) = with_type(json!({"x": 100, "y": 300, "size": 40, "text": text}));
        glyphs(&mut s, true);
        assert_eq!(s.transform_box(&[t]).unwrap().rect, em(&s, t), "{text:?}");
    }
}

/// Area type keeps its frame, and type on a path its path.
#[test]
fn area_type_and_type_on_a_path_are_measured_as_before() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 800, "height": 800}));
    let area = id(run(&mut s, "text.create", json!({"x": 100, "y": 100, "size": 24, "text": "ace", "area": {"width": 200, "height": 100}})));
    let line = id(run(&mut s, "shape.line", json!({"x1": 100, "y1": 400, "x2": 500, "y2": 400})));
    let path = id(run(&mut s, "text.createInPath", json!({"path": line.0, "mode": "onPath", "text": "ace", "size": 24})));
    for t in [area, path] {
        let off = s.transform_box(&[t]).unwrap();
        glyphs(&mut s, true);
        assert_eq!(s.transform_box(&[t]).unwrap(), off, "{t:?}");
        glyphs(&mut s, false);
    }
}

#[test]
fn vertical_point_type_measures_its_ink() {
    let (mut s, t) = with_type(json!({"x": 100, "y": 100, "size": 60, "text": "ace", "vertical": true}));
    glyphs(&mut s, true);
    let b = s.transform_box(&[t]).unwrap().rect;
    assert!(near(b, ink(&s, t), 1e-9), "{b:?} != {:?}", ink(&s, t));
    assert!(em(&s, t).width() > b.width() + 1.0, "em box {:?}", em(&s, t));
}

/// With Use Preview Bounds, the characters' strokes paint around the glyphs.
#[test]
fn preview_bounds_add_the_character_strokes_to_the_ink() {
    let (mut s, t) = with_type(json!({"x": 100, "y": 300, "size": 100, "text": "Hg"}));
    run(&mut s, "text.setStyle", json!({"ids": [t.0], "stroke": "#000000", "strokeWidth": 6}));
    run(&mut s, "prefs.set", json!({"values": {"typeGlyphBounds": true, "usePreviewBounds": true}}));
    let o = text(&s, t).char_stroke_outset();
    assert!(o > 0.0);
    let b = s.transform_box(&[t]).unwrap().rect;
    assert!(near(b, ink(&s, t).inflate(o, o), 1e-9), "{b:?}");
    // A typed height is the height with the strokes.
    run(&mut s, "object.setBounds", json!({"height": 80}));
    assert!((s.transform_box(&[t]).unwrap().rect.height() - 80.0).abs() < 1e-6, "{:?}", s.transform_box(&[t]));
}

/// Type inside a selected group is measured as before: the group's box does not change.
#[test]
fn a_group_holding_type_keeps_its_box() {
    let (mut s, _) = with_type(json!({"x": 100, "y": 300, "size": 100, "text": "Hg"}));
    let g = id(run(&mut s, "object.group", json!({})));
    let off = s.transform_box(&[g]).unwrap();
    glyphs(&mut s, true);
    assert_eq!(s.transform_box(&[g]).unwrap(), off);
}

/// Runs in two fonts: the box covers the ink of both, and follows an edit of the type.
#[test]
fn mixed_fonts_measure_the_ink_of_every_run() {
    let (mut s, t) = with_type(json!({"x": 100, "y": 300, "size": 100, "text": "Hgxo"}));
    glyphs(&mut s, true);
    run(&mut s, "text.setRangeStyle", json!({"id": t.0, "font": "Source Serif 4", "start": 2, "end": 4}));
    let tx = text(&s, t);
    assert!(tx.runs.iter().any(|r| r.style.font_family == "Source Serif 4") && tx.runs.len() > 1, "{:?}", tx.runs);
    assert!(tx.ink().is_some(), "the edit refreshed the cached ink");
    assert!(near(s.transform_box(&[t]).unwrap().rect, ink(&s, t), 1e-9));
}

/// A selection of type and a shape is measured by the type's ink and the shape's bounds.
#[test]
fn a_mixed_selection_takes_the_ink_and_the_shapes_bounds() {
    let (mut s, t) = with_type(json!({"x": 100, "y": 300, "size": 100, "text": "Hg"}));
    let r = id(run(&mut s, "shape.rectangle", json!({"x": 500, "y": 500, "width": 50, "height": 50})));
    glyphs(&mut s, true);
    assert!(near(s.transform_bounds(&[t, r]).unwrap(), ink(&s, t).union(Rect::new(500.0, 500.0, 550.0, 550.0)), 1e-9));
}

#[test]
fn the_preference_is_checked_and_saved() {
    let mut s = Session::new();
    assert!(s.execute("prefs.set", &json!({"values": {"typeGlyphBounds": "yes"}})).is_err());
    assert!(!s.prefs.type_glyph_bounds);
    assert!(!serde_json::from_value::<Prefs>(json!({})).unwrap().type_glyph_bounds);
    run(&mut s, "prefs.set", json!({"key": "typeGlyphBounds", "value": true}));
    let saved = serde_json::to_value(&s.prefs).unwrap();
    assert_eq!(saved["typeGlyphBounds"], json!(true));
    assert!(serde_json::from_value::<Prefs>(saved).unwrap().type_glyph_bounds);
    assert_eq!(run(&mut s, "prefs.get", json!({"key": "typeGlyphBounds"})), json!(true));
}

/// The preference is not part of the document: a saved document is the same with it on or off.
#[test]
fn saved_documents_do_not_depend_on_the_preference() {
    let (mut s, _) = with_type(json!({"x": 100, "y": 300, "size": 100, "text": "Hg"}));
    let off = serde_json::to_string(&s.doc().unwrap().doc).unwrap();
    assert!(!off.contains("cached_ink") && !off.contains("cachedInk"), "the cached ink is not saved");
    glyphs(&mut s, true);
    assert_eq!(serde_json::to_string(&s.doc().unwrap().doc).unwrap(), off);
}

/// A transform of type keeps its cached ink, which is in text space.
#[test]
fn the_cached_ink_is_the_laid_out_ink() {
    let (mut s, t) = with_type(json!({"x": 100, "y": 300, "size": 100, "text": "Hg"}));
    let tx = text(&s, t);
    assert_eq!(tx.ink(), Some(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &tx).ink_bounds()));
    run(&mut s, "object.transform", json!({"matrix": [2, 0, 0, 3, 0, 0]}));
    assert_eq!(text(&s, t).ink(), tx.ink());
    // A cache measured with other bounds is not read.
    let mut stale = text(&s, t);
    stale.cached_bounds = Some(Rect::new(0.0, 0.0, 1.0, 1.0));
    assert_eq!(stale.ink(), None);
}
