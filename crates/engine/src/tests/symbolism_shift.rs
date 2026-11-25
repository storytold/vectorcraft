//! The symbolism tools' Shift forms in `symbol.adjust {shift}`: the Symbol Shifter's stacking
//! order, the Symbol Sizer keeping the set's density, and the Symbol Stainer and Symbol Styler
//! keeping their amount while the color or style changes.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeKind};
use vectorcraft_geom::Point;

use crate::*;

/// A document with the 20 pt square symbol "Dot" and nothing selected.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 20, "height": 20})).unwrap();
    s.execute("symbol.new", &json!({"name": "Dot"})).unwrap();
    s.select(|_, sel| sel.clear()).unwrap();
    s
}

/// An instance of Dot centered on (`x`, `y`), not selected.
fn dot(s: &mut Session, x: f64, y: f64) -> NodeId {
    let id = NodeId(s.execute("symbol.place", &json!({"name": "Dot", "x": x, "y": y})).unwrap()["id"].as_u64().unwrap());
    s.select(|_, sel| sel.clear()).unwrap();
    id
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

fn center(s: &Session, id: NodeId) -> Point {
    match node(s, id).kind {
        NodeKind::SymbolInstance { xf, .. } => xf * Point::ZERO,
        other => panic!("not an instance: {other:?}"),
    }
}

fn width(s: &Session, id: NodeId) -> f64 {
    node(s, id).geometric_bounds().unwrap().width()
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn state(s: &Session) -> (Document, usize) {
    ((*s.doc().unwrap().doc).clone(), undo_len(s))
}

fn adjust(s: &mut Session, p: Value) -> Value {
    s.execute("symbol.adjust", &p).unwrap()
}

/// The children of symbol set `set`, in stacking order (the first is at the back).
fn members(s: &Session, set: NodeId) -> Vec<NodeId> {
    node(s, set).children().unwrap().iter().map(|c| c.id).collect()
}

/// The instance's fill: its color as RGB and its amount.
fn fill(s: &Session, id: NodeId) -> Option<([f32; 3], f32)> {
    node(s, id).appearance.fill().map(|f| (f.paint.color().map_or([0.0; 3], |c| c.to_rgb()), f.opacity))
}

/// A symbol set of four instances in a row at x = 40, 120, 200 and 280, selected.
fn row(s: &mut Session) -> (NodeId, Vec<NodeId>) {
    let pts: Vec<Value> = (0..4).map(|i| json!([40.0 + 80.0 * f64::from(i), 150.0])).collect();
    let set = NodeId(s.execute("symbol.spray", &json!({"name": "Dot", "points": pts, "radius": 4})).unwrap()["id"].as_u64().unwrap());
    let ids = members(s, set);
    assert_eq!(ids.len(), 4);
    (set, ids)
}

/// Shift-click with the Symbol Shifter brings the instances under the brush one place forward in
/// the set, and Alt+Shift-click sends them one place backward, without moving them on the canvas.
#[test]
fn shift_click_with_the_shifter_changes_the_stacking_order() {
    let mut s = session();
    let (set, ids) = row(&mut s);
    let centers: Vec<Point> = ids.iter().map(|id| center(&s, *id)).collect();
    let click = |s: &mut Session, x: f64, alt: bool| {
        adjust(s, json!({"tool": "shift", "points": [[x, 150]], "radius": 10, "intensity": 10, "shift": true, "alt": alt}))["count"].clone()
    };
    let steps = undo_len(&s);
    assert_eq!(click(&mut s, 120.0, false), 1);
    assert_eq!(members(&s, set), [ids[0], ids[2], ids[1], ids[3]]);
    assert_eq!(undo_len(&s), steps + 1, "one undo step");
    assert_eq!(ids.iter().map(|id| center(&s, *id)).collect::<Vec<_>>(), centers, "nothing moves on the canvas");
    assert_eq!(click(&mut s, 120.0, false), 1);
    assert_eq!(members(&s, set), [ids[0], ids[2], ids[3], ids[1]]);
    // Already in front of the others: nothing changes and no undo step is added.
    let before = state(&s);
    assert_eq!(click(&mut s, 120.0, false), 0);
    assert!(state(&s) == before);
    // Alt sends it back one place at a time; at the back it stays.
    assert_eq!(click(&mut s, 120.0, true), 1);
    assert_eq!(members(&s, set), [ids[0], ids[2], ids[1], ids[3]]);
    click(&mut s, 120.0, true);
    click(&mut s, 120.0, true);
    assert_eq!(members(&s, set), [ids[1], ids[0], ids[2], ids[3]]);
    assert_eq!(click(&mut s, 120.0, true), 0);
    // Two instances under the brush move together past the next one.
    s.execute("edit.undo", &json!({})).unwrap();
    let both = adjust(&mut s, json!({"tool": "shift", "points": [[40, 150], [120, 150]], "radius": 10, "intensity": 10, "shift": true}));
    assert_eq!(both["count"], 2);
    assert_eq!(members(&s, set), [ids[2], ids[0], ids[1], ids[3]]);
    // Without Shift a drag moves the instances, as before.
    adjust(&mut s, json!({"tool": "shift", "points": [[280, 150], [290, 150]], "radius": 30, "intensity": 10}));
    assert!(center(&s, ids[3]).x > centers[3].x);
}

/// Shift with the Symbol Sizer resizes the instances and keeps their centers where they are, even
/// with Resizing Affects Density.
#[test]
fn shift_with_the_sizer_keeps_the_density() {
    let at = |affects: bool, shift: bool| {
        let mut s = session();
        let a = dot(&mut s, 220.0, 150.0);
        adjust(
            &mut s,
            json!({"tool": "size", "points": [[200, 150], [200, 150]], "radius": 60, "intensity": 10, "affectsDensity": affects, "shift": shift}),
        );
        (center(&s, a), width(&s, a))
    };
    let (moved, _) = at(true, false);
    assert!(moved.x > 220.5, "{moved:?}");
    let (kept, grown) = at(true, true);
    assert!((kept.x - 220.0).abs() < 1e-9 && (kept.y - 150.0).abs() < 1e-9 && grown > 20.5, "{kept:?} {grown}");
    // It resizes as the Sizer does with Resizing Affects Density off.
    assert_eq!(at(false, false), (kept, grown));
    assert_eq!(at(false, true), (kept, grown));
}

/// Shift with the Symbol Stainer keeps the stain's amount and moves its color toward the fill
/// color; an instance without a stain is left as it is.
#[test]
fn shift_with_the_stainer_keeps_the_amount_and_changes_the_color() {
    let mut s = session();
    let (a, b) = (dot(&mut s, 100.0, 150.0), dot(&mut s, 300.0, 150.0));
    adjust(&mut s, json!({"tool": "stain", "points": [[100, 150], [100, 150]], "radius": 5, "intensity": 10, "color": "#ff0000"}));
    let (red, amount) = fill(&s, a).unwrap();
    assert!(amount > 0.2 && red[0] > 0.99 && red[2] < 0.01, "{red:?} {amount}");
    // A second fill below the stain, which Shift leaves as it is.
    s.edit("Add fill", |d, _| {
        let n = d.node_mut(a).unwrap();
        let lower = n.appearance.fill().unwrap().clone();
        n.appearance.items.insert(0, vectorcraft_doc::AppearanceItem::Fill(vectorcraft_doc::FillLayer { opacity: 0.5, ..lower }));
        Ok(())
    })
    .unwrap();
    let lower = node(&s, a).appearance.items[0].clone();
    let stain = |s: &mut Session, x: f64| {
        adjust(s, json!({"tool": "stain", "points": [[x, 150], [x, 150]], "radius": 5, "intensity": 10, "color": "#0000ff", "shift": true}))
    };
    assert_eq!(stain(&mut s, 100.0)["count"], 1);
    let (color, kept) = fill(&s, a).unwrap();
    assert_eq!(kept, amount, "the amount stays");
    assert!(color[0] < red[0] && color[2] > 0.01, "toward blue: {color:?}");
    assert_eq!((node(&s, a).appearance.items.len(), &node(&s, a).appearance.items[0]), (2, &lower), "the other fill stays");
    // More strokes go on toward blue, with the amount still the same.
    for _ in 0..30 {
        stain(&mut s, 100.0);
    }
    let (bluer, kept) = fill(&s, a).unwrap();
    assert_eq!(kept, amount);
    assert!(bluer[2] > 0.9 && bluer[0] < 0.1, "{bluer:?}");
    // Alt has no effect with Shift.
    let green = json!({"tool": "stain", "points": [[100, 150]], "radius": 5, "intensity": 10, "color": "#00ff00", "shift": true});
    adjust(&mut s, green.clone());
    let plain = fill(&s, a);
    s.execute("edit.undo", &json!({})).unwrap();
    let mut with_alt = green;
    with_alt["alt"] = json!(true);
    adjust(&mut s, with_alt);
    assert_eq!(fill(&s, a), plain);
    // No stain on b: nothing changes and no undo step is added.
    let before = state(&s);
    assert_eq!(stain(&mut s, 300.0)["count"], 0);
    assert!(state(&s) == before && fill(&s, b).is_none());
}

/// Shift with the Symbol Styler keeps the amount of the instance's stain or style and moves its
/// fill toward the selected style's fill.
#[test]
fn shift_with_the_styler_keeps_the_amount_and_changes_the_style() {
    let mut s = session();
    let a = dot(&mut s, 100.0, 150.0);
    let id = s.execute("shape.rectangle", &json!({"x": 300, "y": 250, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"color": "#0000ff"})).unwrap();
    s.execute("graphicStyle.new", &json!({"name": "Blue", "id": id})).unwrap();
    s.select(|_, sel| sel.clear()).unwrap();
    // A red stain at part strength.
    adjust(&mut s, json!({"tool": "stain", "points": [[100, 150]], "radius": 5, "intensity": 10, "color": "#ff0000"}));
    let (red, amount) = fill(&s, a).unwrap();
    assert!(red[0] > 0.99 && amount > 0.0 && amount < 0.5, "{red:?} {amount}");
    // Half the brush's strength at a point 2.5 pt from the instance: half way to blue.
    adjust(&mut s, json!({"tool": "style", "points": [[102.5, 150]], "radius": 5, "intensity": 10, "style": "Blue", "shift": true}));
    let (mixed, kept) = fill(&s, a).unwrap();
    assert_eq!(kept, amount);
    assert!(mixed[0] < 0.99 && mixed[2] > 0.01, "{mixed:?}");
    // At the brush's center with full intensity it takes the style's fill.
    adjust(&mut s, json!({"tool": "style", "points": [[100, 150]], "radius": 5, "intensity": 10, "style": "Blue", "shift": true}));
    let (blue, kept) = fill(&s, a).unwrap();
    assert_eq!(kept, amount);
    assert!(blue[2] > 0.99 && blue[0] < 0.01, "{blue:?}");
}

/// A stain or style with no form between it and the new paint (a pattern and a solid color) takes
/// the new paint at once, at any strength, and keeps its amount.
#[test]
fn shift_changes_a_paint_that_has_no_form_in_between() {
    use vectorcraft_color::Paint;
    let mut s = session();
    let a = dot(&mut s, 100.0, 150.0);
    s.edit("Pattern fill", |d, _| {
        let mut f = vectorcraft_doc::FillLayer::new(Paint::Pattern { pattern: "Tile".into(), xf: Default::default() });
        f.opacity = 0.4;
        d.node_mut(a).unwrap().appearance.items.insert(0, vectorcraft_doc::AppearanceItem::Fill(f));
        Ok(())
    })
    .unwrap();
    // Low intensity, near the brush's edge: far below half strength.
    let r = adjust(&mut s, json!({"tool": "stain", "points": [[104, 150]], "radius": 5, "intensity": 1, "color": "#ff0000", "shift": true}));
    assert_eq!(r["count"], 1);
    let f = node(&s, a).appearance.fill().cloned().unwrap();
    assert_eq!(f.opacity, 0.4, "the amount stays");
    assert!(f.paint.color().is_some_and(|c| c.to_rgb()[0] > 0.99), "{:?}", f.paint);
}

/// A style name the document has no style for (the Styler's style from another document, or one
/// since renamed or deleted) moves the fill toward the document's first style after the default
/// one, as no name does, and the stain's amount stays.
#[test]
fn the_styler_takes_the_first_style_for_a_name_the_document_lacks() {
    use vectorcraft_color::{Color, Paint};
    let styled = |style: Option<&str>| {
        let mut s = session();
        let a = dot(&mut s, 100.0, 150.0);
        s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
        s.select(|_, sel| sel.clear()).unwrap();
        adjust(&mut s, json!({"tool": "stain", "points": [[100, 150]], "radius": 5, "intensity": 10, "color": "#ff0000"}));
        let mut p = json!({"tool": "style", "points": [[100, 150]], "radius": 5, "intensity": 10, "shift": true});
        if let Some(n) = style {
            p["style"] = json!(n);
        }
        let stained = node(&s, a).appearance.fill().map(|f| f.opacity);
        adjust(&mut s, p);
        let f = node(&s, a).appearance.fill().cloned().unwrap();
        assert_eq!(Some(f.opacity), stained, "the amount stays");
        let first = s.doc().unwrap().doc.graphic_styles[1].appearance.fill_paint();
        (f.paint, first)
    };
    let (missing, first) = styled(Some("No Such Style"));
    let (none, _) = styled(None);
    assert_eq!(missing, none);
    // The paint is the first style's, not the active fill color.
    assert_eq!(none, first);
    assert_ne!(none, Paint::solid(Color::rgb(0.0, 1.0, 0.0)));
}

/// A `shift` that is not true, false or null is refused and changes nothing.
#[test]
fn a_bad_shift_is_refused() {
    let mut s = session();
    dot(&mut s, 100.0, 150.0);
    for tool in ["shift", "size", "stain", "style"] {
        let before = state(&s);
        let r = s.execute("symbol.adjust", &json!({"tool": tool, "points": [[100, 150]], "shift": "yes"}));
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if msg == "`shift` must be true or false"), "{tool}: {r:?}");
        assert!(state(&s) == before);
    }
}
