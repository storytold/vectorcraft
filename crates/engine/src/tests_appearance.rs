//! Appearance stacks: the active item that paint, stroke, gradient and transparency edits target
//! (`appearance.setActiveItem`, the `item` param).

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color, GradientKind, Paint};
use vectorcraft_doc::{AppearanceItem, Node};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

/// A selected 100×100 rectangle with the default appearance `[Fill white, Stroke black]`.
fn rect(s: &mut Session, x: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": 100, "width": 100, "height": 100})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    id
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn stroke(n: &Node, i: usize) -> &vectorcraft_doc::StrokeLayer {
    n.appearance.stroke_at(Some(i)).unwrap_or_else(|| panic!("item {i} is not a stroke"))
}

fn fill_paint(n: &Node, i: usize) -> &Paint {
    &n.appearance.fill_at(Some(i)).unwrap_or_else(|| panic!("item {i} is not a fill")).paint
}

#[test]
fn stroke_set_with_item_changes_only_that_stroke() {
    let mut s = session();
    let id = rect(&mut s, 100.0);
    run(&mut s, "appearance.addStroke", json!({})); // [Fill, Stroke, Stroke]
    run(&mut s, "stroke.set", json!({"item": 1, "dash": [4, 2], "weight": 6}));
    let n = node(&s, id);
    assert_eq!(stroke(&n, 1).dash.as_ref().unwrap().pattern, vec![4.0, 2.0]);
    assert_eq!(stroke(&n, 1).width, 6.0);
    assert!(stroke(&n, 2).dash.is_none());
    assert_eq!(stroke(&n, 2).width, 1.0);
    run(&mut s, "stroke.setAdvanced", json!({"item": 1, "arrowScale": [50, 200]}));
    let n = node(&s, id);
    assert_eq!((stroke(&n, 1).arrow_scale, stroke(&n, 2).arrow_scale), ((50.0, 200.0), (100.0, 100.0)));
    // An explicit item must be a stroke; nothing changes when it is not.
    assert!(s.execute("stroke.set", &json!({"item": 0, "weight": 9})).is_err());
    assert!(s.execute("stroke.set", &json!({"item": 7, "weight": 9})).is_err());
    assert!(s.execute("stroke.set", &json!({"item": "top", "weight": 9})).is_err());
    assert_eq!(stroke(&node(&s, id), 2).width, 1.0);
    // Without an item the top stroke changes.
    run(&mut s, "stroke.set", json!({"weight": 3}));
    let n = node(&s, id);
    assert_eq!((stroke(&n, 1).width, stroke(&n, 2).width), (6.0, 3.0));
}

#[test]
fn edit_gradient_on_the_lower_fill() {
    let mut s = session();
    let id = rect(&mut s, 100.0);
    run(&mut s, "appearance.addFill", json!({})); // [Fill, Stroke, Fill]
    run(&mut s, "paint.editGradient", json!({"item": 0, "kind": "radial"}));
    let n = node(&s, id);
    let Paint::Gradient(g) = fill_paint(&n, 0) else { panic!("lower fill is not a gradient") };
    assert_eq!(g.gradient.kind, GradientKind::Radial);
    assert_eq!(*fill_paint(&n, 2), Paint::solid(Color::WHITE));
    // The gradient vector of the lower fill, through the active item (the gradient tool's path).
    run(&mut s, "appearance.setActiveItem", json!({"index": 0}));
    run(&mut s, "paint.setGradientGeom", json!({"start": [100, 150], "end": [200, 150]}));
    let n = node(&s, id);
    let Paint::Gradient(g) = fill_paint(&n, 0) else { panic!() };
    assert_eq!(g.geom.unwrap().end.x, 200.0);
    assert_eq!(*fill_paint(&n, 2), Paint::solid(Color::WHITE));
    // A stroke row makes the gradient vector a stroke gradient.
    run(&mut s, "appearance.setActiveItem", json!({"index": 1}));
    run(&mut s, "paint.setGradientGeom", json!({"start": [100, 150], "end": [100, 250]}));
    assert!(matches!(stroke(&node(&s, id), 1).paint, Paint::Gradient(_)));
}

#[test]
fn active_item_targets_paint_and_proxies_and_clears_on_selection_change() {
    let mut s = session();
    let other = rect(&mut s, 250.0);
    let id = rect(&mut s, 100.0);
    run(&mut s, "appearance.addFill", json!({})); // [Fill, Stroke, Fill]
    assert!(s.execute("appearance.setActiveItem", &json!({"index": 5})).is_err());
    assert_eq!(run(&mut s, "appearance.setActiveItem", json!({"index": 0}))["index"], 0);
    assert_eq!(s.appearance_item(), Some(0));
    assert_eq!(inspect::document(&s)["paint"]["appearanceItem"], 0);
    assert!(s.fill_active);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    let n = node(&s, id);
    assert_eq!(fill_paint(&n, 0).color().unwrap().to_hex(), "#ff0000");
    assert_eq!(*fill_paint(&n, 2), Paint::solid(Color::WHITE));
    // The stroke edits fall back to the top stroke while a fill row is active.
    run(&mut s, "paint.setStroke", json!({"color": "#00ff00"}));
    assert_eq!(stroke(&node(&s, id), 1).paint.color().unwrap().to_hex(), "#00ff00");
    // The proxies show the active row for its kind and the top one for the other.
    let (f, st) = s.proxy_paints();
    assert_eq!((f.color().unwrap().to_hex(), st.color().unwrap().to_hex()), ("#ff0000".to_string(), "#00ff00".to_string()));
    // A stroke row brings the Stroke proxy forward.
    run(&mut s, "appearance.setActiveItem", json!({"index": 1}));
    assert!(!s.fill_active);
    // Changing the selection clears it, also when the same object is selected again.
    run(&mut s, "select.set", json!({"ids": [other.0]}));
    assert_eq!(s.appearance_item(), None);
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    assert_eq!(s.appearance_item(), None);
    run(&mut s, "paint.setFill", json!({"color": "#0000ff"}));
    let n = node(&s, id);
    assert_eq!(fill_paint(&n, 2).color().unwrap().to_hex(), "#0000ff");
    assert_eq!(fill_paint(&n, 0).color().unwrap().to_hex(), "#ff0000");
    // Explicit ids never use the active item, and `null` forces the top one.
    run(&mut s, "appearance.setActiveItem", json!({"index": 0}));
    run(&mut s, "paint.setFill", json!({"color": "#222222", "ids": [id.0]}));
    assert_eq!(fill_paint(&node(&s, id), 2).color().unwrap().to_hex(), "#222222");
    run(&mut s, "paint.setFill", json!({"color": "#111111", "item": null}));
    let n = node(&s, id);
    assert_eq!(fill_paint(&n, 2).color().unwrap().to_hex(), "#111111");
    assert_eq!(fill_paint(&n, 0).color().unwrap().to_hex(), "#ff0000");
    run(&mut s, "appearance.setActiveItem", json!({"index": null}));
    assert_eq!(s.appearance_item(), None);
}

#[test]
fn item_rows_follow_remove_and_move() {
    let mut s = session();
    let id = rect(&mut s, 100.0);
    run(&mut s, "appearance.addFill", json!({})); // [Fill, Stroke, Fill]
    run(&mut s, "appearance.setActiveItem", json!({"index": 1}));
    run(&mut s, "appearance.moveItem", json!({"from": 1, "to": 2}));
    assert_eq!(s.appearance_item(), Some(2));
    assert!(matches!(node(&s, id).appearance.items[2], AppearanceItem::Stroke(_)));
    run(&mut s, "appearance.removeItem", json!({"index": 0}));
    assert_eq!(s.appearance_item(), Some(1));
    run(&mut s, "appearance.removeItem", json!({"index": 1}));
    assert_eq!(s.appearance_item(), None);
}

#[test]
fn transparency_goes_to_the_targeted_item() {
    let mut s = session();
    let id = rect(&mut s, 100.0);
    run(&mut s, "transparency.set", json!({"item": 1, "opacity": 40, "blend": "multiply"}));
    let n = node(&s, id);
    assert_eq!((stroke(&n, 1).opacity, stroke(&n, 1).blend), (0.4, BlendMode::Multiply));
    assert_eq!((n.opacity, n.blend), (1.0, BlendMode::Normal));
    run(&mut s, "appearance.setActiveItem", json!({"index": 0}));
    // Opacity and blend go to the active item, isolate stays on the object.
    run(&mut s, "transparency.set", json!({"opacity": 1, "isolate": true}));
    let n = node(&s, id);
    assert!((n.appearance.items[0].opacity() - 0.01).abs() < 1e-6);
    assert!(n.isolate);
    assert_eq!(n.opacity, 1.0);
    run(&mut s, "transparency.set", json!({"item": null, "opacity": 50}));
    assert_eq!(node(&s, id).opacity, 0.5);
    assert!(s.execute("transparency.set", &json!({"item": 9, "opacity": 50})).is_err());
}

#[test]
fn group_rows_are_the_groups_own_stack() {
    let mut s = session();
    let a = rect(&mut s, 100.0);
    let b = rect(&mut s, 250.0);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addFill", json!({}));
    assert_eq!(node(&s, g).appearance.items.len(), 1);
    assert_eq!(node(&s, a).appearance.items.len(), 2);
    run(&mut s, "appearance.setActiveItem", json!({"index": 0}));
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    assert_eq!(fill_paint(&node(&s, g), 0).color().unwrap().to_hex(), "#ff0000");
    assert_eq!(*fill_paint(&node(&s, a), 0), Paint::solid(Color::WHITE));
    // Without a targeted item, a group's fill edit still recolours its contents.
    run(&mut s, "appearance.setActiveItem", json!({"index": null}));
    run(&mut s, "paint.setFill", json!({"color": "#00ff00"}));
    assert_eq!(fill_paint(&node(&s, b), 0).color().unwrap().to_hex(), "#00ff00");
}

#[test]
fn text_items_take_item_paint() {
    let mut s = session();
    let t = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 50, "text": "Hi"}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "paint.setFill", json!({"item": 0, "color": "#ff0000"}));
    let n = node(&s, t);
    assert_eq!(fill_paint(&n, 0).color().unwrap().to_hex(), "#ff0000");
    let vectorcraft_doc::NodeKind::Text(tx) = &n.kind else { panic!() };
    assert_ne!(tx.first_style().fill, Paint::solid(Color::from_hex("#ff0000").unwrap()));
}

#[test]
fn effects_land_on_the_targeted_item() {
    let mut s = session();
    let id = rect(&mut s, 100.0);
    let r = run(&mut s, "effect.apply", json!({"effect": "distort.roughen", "item": 1}));
    assert_eq!((r["index"].clone(), r["item"].clone()), (json!(0), json!(1)));
    let n = node(&s, id);
    assert_eq!(stroke(&n, 1).effects.len(), 1);
    assert!(n.appearance.effects.is_empty() && n.appearance.items[0].effects().is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert!(stroke(&node(&s, id), 1).effects.is_empty());
    assert!(s.execute("effect.apply", &json!({"effect": "distort.roughen", "item": 9})).is_err());
    assert!(s.execute("effect.apply", &json!({"effect": "distort.roughen", "item": -1})).is_err());
    // The active item takes the Effect menu's effects (and its alias).
    run(&mut s, "appearance.setActiveItem", json!({"index": 0}));
    run(&mut s, "effect.apply", json!({"effect": "distort.twist"}));
    run(&mut s, "appearance.addEffect", json!({"effect": "stylize.roundCorners"}));
    let n = node(&s, id);
    assert_eq!(n.appearance.items[0].effects().iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["distort.twist", "stylize.roundCorners"]);
    let l = run(&mut s, "effect.list", json!({}));
    assert_eq!(l["activeItem"], 0);
    assert_eq!(l["applied"][0]["items"][0]["kind"], "fill");
    assert_eq!(l["applied"][0]["items"][0]["effects"].as_array().unwrap().len(), 2);
    // Sub-row eye, duplicate and delete address the item's list.
    run(&mut s, "effect.setParams", json!({"index": 1, "visible": false}));
    run(&mut s, "effect.duplicate", json!({"index": 0}));
    run(&mut s, "effect.remove", json!({"index": 2}));
    let fx = node(&s, id).appearance.items[0].effects().clone();
    assert_eq!(fx.iter().map(|e| (e.id.as_str(), e.visible)).collect::<Vec<_>>(), [("distort.twist", true), ("distort.twist", true)]);
    assert!(s.execute("effect.remove", &json!({"index": 5})).is_err());
    // `item: null` addresses the object's own effects.
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "item": null}));
    assert_eq!(node(&s, id).appearance.effects.len(), 1);
}

#[test]
fn per_fill_offset_path_renders() {
    let mut s = session();
    let id = rect(&mut s, 100.0);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let render = |s: &Session| {
        vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 400, 400, vectorcraft_geom::Affine::IDENTITY, &Default::default())
    };
    assert_eq!(render(&s).pixel(90, 150)[3], 0);
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 20}, "item": 0}));
    assert_eq!(node(&s, id).appearance.items[0].effects().len(), 1);
    let px = render(&s).pixel(90, 150);
    assert!(px[0] > 200 && px[3] > 200, "{px:?}");
}

#[test]
fn text_fill_items_composite_with_their_opacity() {
    let mut s = session();
    let t = NodeId(run(&mut s, "text.create", json!({"x": 20, "y": 200, "text": "HIM", "size": 150}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "appearance.setItem", json!({"index": 0, "color": "#ff0000"}));
    let render = |s: &Session| {
        vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 400, 400, vectorcraft_geom::Affine::IDENTITY, &Default::default())
    };
    let opaque = render(&s);
    // A pixel the red fill covers fully (over the black character fill).
    let (x, y) = (0..400u32).flat_map(|y| (0..400u32).map(move |x| (x, y))).find(|&(x, y)| opaque.pixel(x, y) == [255, 0, 0, 255]).expect("red text");
    run(&mut s, "appearance.setItem", json!({"index": 0, "opacity": 50, "blend": "multiply"}));
    let px = render(&s).pixel(x, y);
    // Half of red multiplied over black: black.
    assert!(px[0] < 20 && px[3] == 255, "{px:?}");
    run(&mut s, "appearance.setItem", json!({"index": 0, "blend": "normal"}));
    let px = render(&s).pixel(x, y);
    assert!((110..=145).contains(&px[0]) && px[1] < 10, "{px:?}");
}

#[test]
fn add_fill_and_stroke_copy_the_top_ones() {
    let mut s = session();
    let id = rect(&mut s, 100.0);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "stroke.set", json!({"weight": 4}));
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "appearance.addStroke", json!({})); // [Fill, Stroke, Fill, Stroke]
    let n = node(&s, id);
    assert_eq!(n.appearance.items.iter().map(AppearanceItem::kind_name).collect::<Vec<_>>(), ["fill", "stroke", "fill", "stroke"]);
    assert_eq!(fill_paint(&n, 2).color().unwrap().to_hex(), "#ff0000");
    assert_eq!(stroke(&n, 3).width, 4.0);
    assert!(!n.appearance.is_basic());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, id).appearance.items.len(), 3);
}

#[test]
fn set_item_and_remove_item() {
    let mut s = session();
    let id = rect(&mut s, 100.0);
    run(&mut s, "appearance.setItem", json!({"index": 1, "opacity": 50, "blend": "screen", "weight": 3, "color": "#0000ff"}));
    let n = node(&s, id);
    let st = stroke(&n, 1);
    assert_eq!((st.opacity, st.blend, st.width), (0.5, BlendMode::Screen, 3.0));
    assert_eq!(st.paint.color().unwrap().to_hex(), "#0000ff");
    run(&mut s, "appearance.setItem", json!({"index": 0, "visible": false, "opacity": 25}));
    let n = node(&s, id);
    assert!(!n.appearance.items[0].visible() && n.appearance.items[0].opacity() == 0.25);
    assert!(s.execute("appearance.setItem", &json!({})).is_err());
    run(&mut s, "appearance.removeItem", json!({"index": 0}));
    let n = node(&s, id);
    assert_eq!(n.appearance.items.len(), 1);
    assert!(n.appearance.fill().is_none() && n.appearance.stroke().is_some());
}

#[test]
fn clear_and_reduce_to_basic() {
    let mut s = session();
    let id = rect(&mut s, 100.0);
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "appearance.setItem", json!({"index": 2, "visible": false, "color": "#00ff00"}));
    run(&mut s, "appearance.setItem", json!({"index": 1, "opacity": 50}));
    run(&mut s, "effect.apply", json!({"effect": "distort.roughen", "item": 1}));
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "item": null}));
    run(&mut s, "transparency.set", json!({"opacity": 40, "blend": "multiply"}));
    // Reduce keeps the topmost visible fill and stroke, plain, and the object's transparency.
    run(&mut s, "appearance.reduceToBasic", json!({}));
    let n = node(&s, id);
    assert!(n.appearance.is_basic(), "{:?}", n.appearance);
    assert_eq!(stroke(&n, 1).paint, Paint::solid(Color::BLACK));
    assert_eq!((n.opacity, n.blend), (0.4, BlendMode::Multiply));
    // Clear leaves one None fill and stroke, and resets the object's Opacity row.
    run(&mut s, "appearance.clear", json!({}));
    let n = node(&s, id);
    assert_eq!(n.appearance, vectorcraft_doc::Appearance::basic(Paint::None, Paint::None, 1.0));
    assert_eq!((n.opacity, n.blend), (1.0, BlendMode::Normal));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, id).opacity, 0.4);
}

/// How many steps there are to undo and to redo, and whether the document has unsaved changes.
fn history(s: &Session) -> (usize, usize, bool) {
    let st = s.doc().unwrap();
    (st.history.undo.len(), st.history.redo.len(), st.is_dirty())
}

/// Select `ids` with a move of them left to redo, and mark the document saved.
fn select_with_a_step_to_redo(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    s.execute("object.move", &json!({"dx": 5, "dy": 0})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    s.doc_mut().unwrap().mark_saved();
}

/// Fill, stroke and gradient edits that leave every object as it was (the color, stroke options,
/// gradient or item values the objects have already) record no undo step: the step to redo stays
/// and the document stays unmodified. A fill that changes one of them is one step.
#[test]
fn fill_stroke_and_gradient_edits_that_change_nothing_record_no_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 50.0);
    let b = rect(&mut s, 200.0);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "paint.setStroke", json!({"color": "#0000ff"}));
    run(&mut s, "stroke.set", json!({"weight": 3, "dash": [4, 2], "cap": "round"}));
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "appearance.addFill", json!({})); // [Fill, Stroke, Fill, Fill], every fill red
    run(&mut s, "paint.setGradientGeom", json!({"item": 0, "start": [60, 150], "end": [140, 150]}));
    run(&mut s, "paint.editGradient", json!({"item": 2, "kind": "freeform"}));
    run(&mut s, "paint.freeform.setPoint", json!({"item": 2, "index": 0, "color": "#00ff00"}));
    select_with_a_step_to_redo(&mut s, &[a, b]);
    let before = history(&s);
    assert_eq!((before.1, before.2), (1, false), "a step to redo, and nothing unsaved");
    for (id, p) in [
        ("paint.setFill", json!({"color": "#ff0000"})),
        ("paint.setStroke", json!({"color": "#0000ff"})),
        ("paint.setFill", json!({"color": "#ff0000", "ids": [a.0]})),
        ("paint.lastColor", json!({"stroke": false})),
        ("stroke.set", json!({"weight": 3, "dash": [4, 2], "cap": "round"})),
        ("stroke.setAdvanced", json!({"arrowScale": [100, 100]})),
        ("paint.setGradientGeom", json!({"item": 0, "start": [60, 150], "end": [140, 150]})),
        ("paint.editGradient", json!({"item": 0, "kind": "linear"})),
        ("paint.freeform.setPoint", json!({"item": 2, "index": 0, "color": "#00ff00"})),
        ("transparency.set", json!({"item": 1, "opacity": 100, "blend": "normal"})),
    ] {
        run(&mut s, id, p.clone());
        assert_eq!(history(&s), before, "{id} {p}");
    }
    run(&mut s, "paint.setFill", json!({"color": "#00ff00", "ids": [b.0]}));
    assert_eq!(history(&s), (before.0 + 1, 0, true));
    assert_eq!(fill_paint(&node(&s, b), 3).color().unwrap().to_hex(), "#00ff00");
}

/// Appearance panel commands that leave every object as it was record no undo step: Reduce to Basic
/// Appearance on a basic appearance, an item set to the values it has or to an index no object has,
/// Remove Item past every stack, Move Item to where the item is, the Eyedropper and Move Appearance
/// between objects that look the same, and Clear Appearance on cleared objects. Clearing objects that
/// have something to clear is one step.
#[test]
fn appearance_commands_that_change_nothing_record_no_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 50.0);
    let b = rect(&mut s, 200.0);
    select_with_a_step_to_redo(&mut s, &[a, b]);
    let before = history(&s);
    for (id, p) in [
        ("appearance.reduceToBasic", json!({})),
        ("appearance.setItem", json!({"index": 1, "opacity": 100, "blend": "normal", "visible": true, "weight": 1})),
        ("appearance.setItem", json!({"index": 5, "opacity": 50})),
        ("appearance.removeItem", json!({"index": 5})),
        ("appearance.moveItem", json!({"from": 1, "to": 1})),
        ("appearance.copyFrom", json!({"source": b.0, "ids": [a.0]})),
        ("appearance.transfer", json!({"source": b.0, "target": a.0, "copy": true})),
    ] {
        run(&mut s, id, p.clone());
        assert_eq!(history(&s), before, "{id} {p}");
    }
    run(&mut s, "appearance.clear", json!({}));
    assert_eq!(history(&s), (before.0 + 1, 0, true), "the first Clear is one step");
    s.doc_mut().unwrap().mark_saved();
    let cleared = history(&s);
    for (id, p) in [("appearance.clear", json!({})), ("appearance.transfer", json!({"source": b.0, "target": a.0}))] {
        run(&mut s, id, p.clone());
        assert_eq!(history(&s), cleared, "{id} {p}");
    }
}

/// The Appearance panel's commands refuse a weight or opacity that is not a number and an on/off
/// option that is not true or false, given as text or as another value of the wrong kind; the
/// objects and the undo history stay as they were. Numbers, true, false and null still work.
#[test]
fn appearance_commands_refuse_a_number_or_boolean_given_as_text() {
    use super::tests::{not_a_number, not_a_switch, not_refused};
    let mut s = session();
    let a = rect(&mut s, 50.0);
    let b = rect(&mut s, 200.0);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let calls = [
        not_a_number("appearance.setItem", json!({"index": 1, "weight": "3"}), "weight"),
        not_a_number("appearance.setItem", json!({"index": 0, "opacity": "50"}), "opacity"),
        not_a_switch("appearance.setItem", json!({"index": 0, "visible": "false"}), "visible"),
        not_a_switch("appearance.transfer", json!({"source": b.0, "target": a.0, "copy": "true"}), "copy"),
        not_a_switch("appearance.copyFrom", json!({"source": b.0, "ids": [a.0], "reverse": "true"}), "reverse"),
        not_a_switch("appearance.copyFrom", json!({"source": b.0, "ids": [a.0], "append": 1}), "append"),
    ];
    let wrong = not_refused(&mut s, &calls);
    assert!(wrong.is_empty(), "not refused: {wrong:#?}");
    run(&mut s, "appearance.setItem", json!({"index": 1, "weight": 3, "visible": null}));
    run(&mut s, "appearance.transfer", json!({"source": b.0, "target": a.0, "copy": true}));
    assert_eq!((stroke(&node(&s, a), 1).width, fill_paint(&node(&s, a), 0)), (stroke(&node(&s, b), 1).width, fill_paint(&node(&s, b), 0)));
}

#[test]
fn graphic_style_apply_replaces_the_stack() {
    let mut s = session();
    let a = rect(&mut s, 100.0);
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "effect.apply", json!({"effect": "distort.roughen"}));
    let name = run(&mut s, "graphicStyle.new", json!({"name": "Rough"}))["name"].clone();
    let b = rect(&mut s, 250.0);
    run(&mut s, "graphicStyle.apply", json!({ "name": name }));
    assert_eq!(node(&s, b).appearance, node(&s, a).appearance);
    assert!(s.execute("graphicStyle.apply", &json!({"name": "nope"})).is_err());
}

#[test]
fn an_active_row_of_the_other_kind_leaves_edits_on_the_painted_leaves() {
    let mut s = session();
    let a = rect(&mut s, 100.0);
    let b = rect(&mut s, 250.0);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addFill", json!({}));
    // The group's own fill row is active: stroke edits still go to the members' strokes.
    run(&mut s, "appearance.setActiveItem", json!({"index": 0}));
    run(&mut s, "stroke.set", json!({"weight": 5}));
    run(&mut s, "paint.setStroke", json!({"color": "#00ff00"}));
    assert_eq!(node(&s, g).appearance.items.len(), 1, "no stroke row added to the group");
    for id in [a, b] {
        assert_eq!(stroke(&node(&s, id), 1).width, 5.0);
        assert_eq!(stroke(&node(&s, id), 1).paint.color().unwrap().to_hex(), "#00ff00");
    }
}

#[test]
fn the_active_row_follows_only_edits_of_its_object() {
    let mut s = session();
    let other = rect(&mut s, 250.0);
    rect(&mut s, 100.0);
    run(&mut s, "appearance.setActiveItem", json!({"index": 1}));
    run(&mut s, "appearance.removeItem", json!({"index": 0, "ids": [other.0]}));
    run(&mut s, "appearance.duplicateItem", json!({"index": 0, "ids": [other.0]}));
    assert_eq!(s.appearance_item(), Some(1));
}

#[test]
fn item_transparency_is_one_step_and_checks_the_blend_mode() {
    let mut s = session();
    let id = rect(&mut s, 100.0);
    run(&mut s, "appearance.setActiveItem", json!({"index": 1}));
    let steps = |s: &Session| s.doc().unwrap().history.undo.len();
    let before = steps(&s);
    run(&mut s, "transparency.set", json!({"knockout": true}));
    assert_eq!(steps(&s), before + 1, "isolate/knockout alone do not touch the item");
    assert_eq!(node(&s, id).knockout, vectorcraft_doc::Knockout::On);
    assert!(s.execute("transparency.set", &json!({"blend": "nope"})).is_err());
    assert_eq!(stroke(&node(&s, id), 1).blend, BlendMode::Normal);
}

#[test]
fn clear_and_reduce_keep_a_groups_stack_empty() {
    let mut s = session();
    let a = rect(&mut s, 100.0);
    let b = rect(&mut s, 250.0);
    let members = node(&s, a).appearance;
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.reduceToBasic", json!({}));
    assert!(node(&s, g).appearance.items.is_empty());
    run(&mut s, "appearance.addStroke", json!({}));
    run(&mut s, "appearance.reduceToBasic", json!({}));
    assert_eq!(node(&s, g).appearance.items.iter().map(AppearanceItem::kind_name).collect::<Vec<_>>(), ["stroke"]);
    run(&mut s, "transparency.set", json!({"opacity": 50}));
    run(&mut s, "appearance.clear", json!({}));
    let n = node(&s, g);
    assert!(n.appearance.items.is_empty() && n.opacity == 1.0);
    // The members keep their own appearance.
    assert_eq!(node(&s, a).appearance, members);
}
