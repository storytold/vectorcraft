//! Appearance editing depth: moving and copying effects between the object and its fills and
//! strokes, duplicating and removing several items, Show All Hidden Attributes.

use serde_json::{Value, json};
use vectorcraft_doc::Node;

use crate::*;

/// A selected 100×100 rectangle with the default appearance `[Fill white, Stroke black]`.
fn session_with_rect() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 100, "height": 100})).unwrap();
    (s, NodeId(r["id"].as_u64().unwrap()))
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// Effect ids of the object (`None`) or of item `i`.
fn fx(s: &Session, id: NodeId, item: Option<usize>) -> Vec<String> {
    node(s, id).appearance.effects_at(item).unwrap().iter().map(|e| e.id.clone()).collect()
}

#[test]
fn effects_reorder_within_a_list_and_undo_in_one_step() {
    let (mut s, id) = session_with_rect();
    for e in ["distort.twist", "distort.roughen", "stylize.dropShadow"] {
        run(&mut s, "effect.apply", json!({ "effect": e }));
    }
    let r = run(&mut s, "effect.move", json!({"from": 0, "to": 2}));
    assert_eq!((r["index"].clone(), r["item"].clone()), (json!(2), Value::Null));
    assert_eq!(fx(&s, id, None), ["distort.roughen", "stylize.dropShadow", "distort.twist"]);
    // Past the end lands last.
    run(&mut s, "effect.move", json!({"from": 0, "to": 9}));
    assert_eq!(fx(&s, id, None), ["stylize.dropShadow", "distort.twist", "distort.roughen"]);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(fx(&s, id, None), ["distort.twist", "distort.roughen", "stylize.dropShadow"]);
    assert!(s.execute("effect.move", &json!({"from": 3, "to": 0})).is_err());
}

#[test]
fn effects_move_and_copy_between_the_object_and_its_items() {
    let (mut s, id) = session_with_rect();
    run(&mut s, "effect.apply", json!({"effect": "distort.twist"}));
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "item": 1}));
    // Into the stroke, above its own effect.
    let r = run(&mut s, "effect.move", json!({"from": 0, "fromItem": null, "to": 1, "toItem": 1}));
    assert_eq!((r["index"].clone(), r["item"].clone()), (json!(1), json!(1)));
    assert!(fx(&s, id, None).is_empty());
    assert_eq!(fx(&s, id, Some(1)), ["path.offsetPath", "distort.twist"]);
    // Out of the stroke onto the fill, as a copy (Alt-drag): the stroke keeps it.
    run(&mut s, "effect.move", json!({"from": 0, "fromItem": 1, "to": 0, "toItem": 0, "copy": true}));
    assert_eq!(fx(&s, id, Some(0)), ["path.offsetPath"]);
    assert_eq!(fx(&s, id, Some(1)), ["path.offsetPath", "distort.twist"]);
    // Back out to the object, one undo step each.
    run(&mut s, "effect.move", json!({"from": 1, "fromItem": 1, "to": 0, "toItem": null}));
    assert_eq!(fx(&s, id, None), ["distort.twist"]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(fx(&s, id, None).is_empty());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(fx(&s, id, Some(0)).is_empty());
    // A missing item is an error, not a silent no-op.
    assert!(s.execute("effect.move", &json!({"from": 0, "fromItem": 1, "to": 0, "toItem": 7})).is_err());
}

#[test]
fn invalid_effect_move_flags_leave_the_effects_alone() {
    let (mut s, id) = session_with_rect();
    run(&mut s, "effect.apply", json!({"effect": "distort.twist"}));
    let before_undo = s.doc().unwrap().history.undo.len();
    let to_fill = json!({"from": 0, "fromItem": null, "to": 0, "toItem": 0});
    for (key, value) in [("copy", json!("true")), ("copy", json!(1)), ("aligned", json!("false"))] {
        let mut params = to_fill.clone();
        params[key] = value;
        assert!(s.execute("effect.move", &params).is_err(), "accepted {params}");
        assert_eq!(fx(&s, id, None), ["distort.twist"], "moved the effect for {params}");
        assert!(fx(&s, id, Some(0)).is_empty(), "copied the effect for {params}");
        assert_eq!(s.doc().unwrap().history.undo.len(), before_undo, "recorded undo for {params}");
    }
    // Null, as for every command, is the same as leaving the flag out: a move.
    let mut params = to_fill;
    params["copy"] = Value::Null;
    params["aligned"] = Value::Null;
    run(&mut s, "effect.move", params);
    assert!(fx(&s, id, None).is_empty());
    assert_eq!(fx(&s, id, Some(0)), ["distort.twist"]);
}

/// `effect.duplicate` copies a fill's effect right after it.
#[test]
fn duplicate_selected_works_for_effects() {
    let (mut s, id) = session_with_rect();
    run(&mut s, "effect.apply", json!({"effect": "distort.roughen", "item": 0}));
    run(&mut s, "effect.duplicate", json!({"index": 0, "item": 0}));
    assert_eq!(fx(&s, id, Some(0)), ["distort.roughen", "distort.roughen"]);
}

/// `appearance.duplicateItem` copies a fill/stroke to a given position, and `appearance.removeItem` with `indices` removes several at once.
#[test]
fn items_duplicate_to_a_position_and_several_remove_at_once() {
    let (mut s, id) = session_with_rect();
    run(&mut s, "appearance.addFill", json!({})); // [Fill, Stroke, Fill]
    run(&mut s, "appearance.setItem", json!({"index": 2, "color": "#ff0000"}));
    // Alt-dragging the top fill to the bottom copies it there.
    run(&mut s, "appearance.duplicateItem", json!({"index": 2, "to": 0}));
    let kinds = |s: &Session| node(s, id).appearance.items.iter().map(|i| i.kind_name()).collect::<Vec<_>>();
    assert_eq!(kinds(&s), ["fill", "fill", "stroke", "fill"]);
    assert_eq!(node(&s, id).appearance.items[0].paint(), node(&s, id).appearance.items[3].paint());
    // Duplicating several copies each above itself.
    run(&mut s, "appearance.duplicateItem", json!({"indices": [0, 2]}));
    assert_eq!(kinds(&s), ["fill", "fill", "fill", "stroke", "stroke", "fill"]);
    // Several with `to` are copied as one block there (Alt-dragging highlighted rows).
    run(&mut s, "appearance.duplicateItem", json!({"indices": [0, 1], "to": 3}));
    assert_eq!(kinds(&s), ["fill", "fill", "fill", "fill", "fill", "stroke", "stroke", "fill"]);
    s.execute("edit.undo", &json!({})).unwrap();
    // Removing several in one step; the active item follows.
    run(&mut s, "appearance.setActiveItem", json!({"index": 5}));
    run(&mut s, "appearance.removeItem", json!({"indices": [4, 0, 1]}));
    assert_eq!(kinds(&s), ["fill", "stroke", "fill"]);
    assert_eq!(s.appearance_item(), Some(2));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(node(&s, id).appearance.items.len(), 6);
}

#[test]
fn show_all_hidden_attributes() {
    let (mut s, id) = session_with_rect();
    run(&mut s, "effect.apply", json!({"effect": "distort.twist"}));
    run(&mut s, "effect.apply", json!({"effect": "distort.roughen", "item": 1}));
    assert!(s.execute("appearance.showAllHidden", &json!({})).is_err(), "nothing hidden yet");
    run(&mut s, "appearance.setItem", json!({"index": 0, "visible": false}));
    run(&mut s, "effect.setParams", json!({"index": 0, "visible": false, "item": null}));
    run(&mut s, "effect.setParams", json!({"index": 0, "visible": false, "item": 1}));
    assert!(node(&s, id).appearance.has_hidden());
    let r = run(&mut s, "appearance.showAllHidden", json!({}));
    assert_eq!(r["ids"], json!([id.0]));
    let n = node(&s, id);
    assert!(!n.appearance.has_hidden());
    assert!(n.appearance.effects[0].visible && n.appearance.items[1].effects()[0].visible && n.appearance.items[0].visible());
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(node(&s, id).appearance.has_hidden());
}

/// A Warp applied now takes its style's Bézier net (`version` 1) unless a map is given, an edit
/// keeps the map the effect has (one saved before the version existed keeps the earlier map), and
/// a map this build doesn't draw is refused.
#[test]
fn warps_keep_the_map_their_version_names() {
    let (mut s, id) = session_with_rect();
    let version = |s: &Session| node(s, id).appearance.effects[0].params.get("version").cloned();
    run(&mut s, "effect.apply", json!({"effect": "warp.flag", "params": {"bend": 40}}));
    assert_eq!(version(&s), Some(json!(vectorcraft_doc::live::WARP_NET)));
    run(&mut s, "effect.setParams", json!({"index": 0, "params": {"bend": 20}, "warpStyle": "arc"}));
    assert_eq!(version(&s), Some(json!(vectorcraft_doc::live::WARP_NET)));
    // As a file saved before the version existed gives it.
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    if let Some(Value::Object(p)) = doc.node_mut(id).and_then(|n| n.appearance.effects.first_mut()).map(|e| &mut e.params) {
        p.remove("version");
    }
    run(&mut s, "effect.setParams", json!({"index": 0, "params": {"bend": 10, "horizontal": 20}}));
    assert_eq!(version(&s), None);
    run(&mut s, "effect.setParams", json!({"index": 0, "params": {"version": 1}}));
    assert_eq!(version(&s), Some(json!(1)));
    run(&mut s, "effect.apply", json!({"effect": "warp.arc", "params": {"version": 0}}));
    assert_eq!(node(&s, id).appearance.effects[1].params["version"], json!(0));
    // A whole number given as a float is that version.
    run(&mut s, "effect.setParams", json!({"index": 1, "params": {"version": 1.0}}));
    assert_eq!(node(&s, id).appearance.effects[1].params["version"], json!(1));
    run(&mut s, "effect.setParams", json!({"index": 1, "params": {"version": 0.0}}));
    assert_eq!(node(&s, id).appearance.effects[1].params["version"], json!(0));
    for bad in [json!(2), json!(-1), json!(0.5), json!("1")] {
        assert!(s.execute("effect.apply", &json!({"effect": "warp.arc", "params": {"version": bad}})).is_err(), "{bad}");
        assert!(s.execute("effect.setParams", &json!({"index": 0, "params": {"version": bad}})).is_err(), "{bad}");
    }
    assert_eq!(node(&s, id).appearance.effects.len(), 2);
}
