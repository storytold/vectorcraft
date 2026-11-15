//! Path Effects (`addon.path.*`) through the session: live on the canvas, the document's path left
//! alone, and Expand Appearance and a save keeping what they draw.

use serde_json::{Value, json};
use vectorcraft_doc::{Document, Effect, Node, NodeId};
use vectorcraft_geom::{BezPath, Rect};
use vectorcraft_render::Renderer;
use vectorcraft_render::effects::{GeomContext, apply_geometry_bez, effect_catalog, new_effect, preview, scale_effect};

use crate::Session;

/// A black 4 pt stroke along `anchors`, unfilled.
fn stroked(anchors: Value, closed: bool) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = NodeId(s.execute("path.create", &json!({"anchors": anchors, "closed": closed})).unwrap()["id"].as_u64().unwrap());
    s.select(|_, sel| sel.set([id])).unwrap();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 4})).unwrap();
    (s, id)
}

fn wave() -> (Session, NodeId) {
    let anchors = json!([
        {"x": 20, "y": 100},
        {"x": 60, "y": 60, "in": [40, 60], "out": [80, 60]},
        {"x": 100, "y": 100},
        {"x": 140, "y": 140},
        {"x": 180, "y": 100}
    ]);
    stroked(anchors, false)
}

/// Every Path Effect, with parameters that change the wave.
fn effects() -> Vec<(&'static str, Value)> {
    vec![
        ("addon.path.dashes", json!({"dash1": 14, "gap1": 6, "dash2": 3, "gap2": 6})),
        ("addon.path.dashes", json!({"dash1": 14, "gap1": 6, "alignCorners": true})),
        ("addon.path.addAnchors", json!({"count": 3})),
        ("addon.path.addAnchors", json!({"spacing": "distance", "distance": 15, "target": "curved"})),
        ("addon.path.extend", json!({"startLength": 10, "endPercent": 10, "endShape": "arc"})),
        ("addon.path.extend", json!({"startLength": -10, "endLength": -20})),
        ("addon.path.close", json!({})),
        ("addon.path.close", json!({"mode": "smooth"})),
        ("addon.path.smooth", json!({"ratio": 0.3})),
        ("addon.path.retract", json!({})),
        ("addon.path.reverse", json!({})),
        ("addon.path.simplify", json!({"tolerance": 200, "protectCorners": false})),
        ("addon.path.split", json!({"anchors": "both"})),
    ]
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

fn picture_of(d: &Document) -> Vec<u8> {
    let r = Renderer::new().render_region(d, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true);
    (0..200).flat_map(|y| (0..200).map(move |x| (x, y))).flat_map(|(x, y)| r.pixel(x, y)).collect()
}

fn picture(s: &Session) -> Vec<u8> {
    picture_of(&s.doc().unwrap().doc)
}

/// The outline of `n`: its path, or those of the compound path it became.
fn outline(n: &Node) -> BezPath {
    let mut out = n.path_data().map(|p| p.to_bezpath()).unwrap_or_default();
    n.children().into_iter().flatten().for_each(|c| out.extend(outline(c)));
    out
}

/// How many bytes of two pictures differ by more than antialiasing noise.
fn differing(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).filter(|(x, y)| x.abs_diff(**y) > 8).count()
}

#[test]
fn a_path_effect_draws_live_and_leaves_the_document_path() {
    let (mut s, id) = stroked(json!([{"x": 10, "y": 50}, {"x": 190, "y": 50}]), false);
    let solid = picture(&s);
    let path = node(&s, id).path_data().cloned();
    s.execute("effect.apply", &json!({"effect": "addon.path.dashes", "params": {"dash1": 20, "gap1": 20}})).unwrap();
    assert_eq!(node(&s, id).path_data().cloned(), path, "only the effect holds the geometry");
    let dashed = picture(&s);
    let at = |pic: &[u8], x: usize| pic[(50 * 200 + x) * 4];
    assert!(at(&solid, 45) < 40 && at(&dashed, 45) > 200, "a gap: {}", at(&dashed, 45));
    assert!(at(&dashed, 15) < 40, "a dash");
    // Hiding it draws the path again.
    s.execute("effect.setParams", &json!({"index": 0, "visible": false})).unwrap();
    assert_eq!(picture(&s), solid);
}

#[test]
fn a_path_effect_moves_the_bounds_the_object_is_measured_by() {
    let (mut s, id) = stroked(json!([{"x": 50, "y": 50}, {"x": 150, "y": 50}]), false);
    s.execute("effect.apply", &json!({"effect": "addon.path.extend", "params": {"startLength": 0, "endLength": 30}})).unwrap();
    let drawn = preview::bounds_of(&s.doc().unwrap().doc, &[id], true).unwrap();
    assert!((drawn.x1 - 180.0).abs() < 3.0, "{drawn:?}");
    let pic = picture(&s);
    assert!(pic[(50 * 200 + 170) * 4] < 40, "and draws there");
}

#[test]
fn expand_appearance_writes_the_outline_the_effect_draws() {
    for (effect, params) in effects() {
        let (mut s, id) = wave();
        s.execute("effect.apply", &json!({"effect": effect, "params": params})).unwrap();
        let n = node(&s, id);
        let before = n.path_data().cloned().unwrap();
        let live = apply_geometry_bez(&n.appearance.effects, &before.to_bezpath(), before.bounds().unwrap(), &GeomContext::of(&n));
        let pic = picture(&s);
        s.execute("effect.expandAppearance", &json!({})).unwrap();
        let expanded = node(&s, id);
        assert!(expanded.appearance.effects.is_empty(), "{effect} {params}");
        let (out, want) = (outline(&expanded), live);
        assert_eq!(out.elements().len(), want.elements().len(), "{effect} {params}");
        for (a, b) in out.elements().iter().zip(want.elements()) {
            assert!(a.is_finite() && format!("{a:.4?}") == format!("{b:.4?}"), "{effect} {params}: {a:?} {b:?}");
        }
        assert_eq!(differing(&pic, &picture(&s)), 0, "{effect} {params}");
    }
}

#[test]
fn path_effects_round_trip_through_a_save_and_scale_their_distances() {
    for (effect, params) in effects() {
        let (mut s, id) = wave();
        s.execute("effect.apply", &json!({"effect": effect, "params": params})).unwrap();
        let restored = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
        assert_eq!(restored.node(id).unwrap().appearance.effects, node(&s, id).appearance.effects, "{effect} {params}");
        assert_eq!(picture_of(&restored), picture(&s), "{effect} {params}");
    }
    let mut e = new_effect("addon.path.dashes", &json!({"dash1": 10})).unwrap();
    scale_effect(&mut e, 2.0);
    assert_eq!((e.params["dash1"].as_f64(), e.params["gap1"].as_f64()), (Some(20.0), Some(12.0)));
}

#[test]
fn undo_takes_a_path_effect_off_in_one_step() {
    let (mut s, id) = wave();
    s.execute("effect.apply", &json!({"effect": "addon.path.reverse"})).unwrap();
    assert_eq!(node(&s, id).appearance.effects.len(), 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(node(&s, id).appearance.effects.is_empty());
}

/// A file's effects skip the checks `effect.apply` makes: whatever they hold, the object draws and
/// expands.
#[test]
fn path_effects_with_junk_params_from_a_file_draw_and_expand() {
    let junk = [
        json!(null),
        json!("x"),
        json!({"dash1": 1e308, "gap1": -1e308, "offset": "x", "alignCorners": 3}),
        json!({"count": 1e308, "maxCount": 1e308, "distance": -1e308, "spacing": "distance", "target": []}),
        json!({"startLength": 1e308, "endPercent": -1e308, "endShape": {}, "mode": 1, "ratio": "x", "handles": "shorter", "length": 1e308}),
        json!({"tolerance": 1e308, "protectCorners": "no", "anchors": null, "startPercent": 1e308, "startShape": "arc"}),
    ];
    let ids = effect_catalog().into_iter().map(|e| e.id).filter(|id| id.starts_with("addon.path."));
    let stack: Vec<Effect> =
        ids.flat_map(|id| junk.iter().map(move |params| Effect { id: id.into(), params: params.clone(), visible: true })).collect();
    let (mut s, id) = wave();
    s.edit("Junk", |d, _| {
        if let Some(n) = d.node_mut(id) {
            n.appearance.effects = stack;
        }
        Ok(())
    })
    .unwrap();
    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    picture_of(&vectorcraft_format::load(&saved).unwrap());
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    assert!(node(&s, id).appearance.effects.is_empty());
}

#[test]
fn exports_write_the_dashes_as_paths() {
    let (mut s, _) = wave();
    s.execute("effect.apply", &json!({"effect": "addon.path.dashes", "params": {"dash1": 10, "gap1": 10}})).unwrap();
    let v = s.execute("document.export", &json!({"format": "svg"})).unwrap();
    let svg = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    let moves = svg.iter().filter(|c| **c == b'M').count();
    assert!(moves > 5, "the exported art holds the dashes: {moves}");
}
