#![allow(clippy::unwrap_used, clippy::panic)]
use serde_json::{Value, json};
use vectorcraft_doc::appearance::AppearanceItem;
use vectorcraft_doc::{NodeId, NodeKind};

use crate::Session;

/// A new document with a filled 100 pt square; `effect` is applied to it when given.
fn setup(effect: Option<&str>) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":400,"height":400})).unwrap();
    let id = NodeId(s.execute("path.create", &json!({"d":"M100 100L200 100L200 200L100 200Z"})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color":"#338ad6"})).unwrap();
    if let Some(effect) = effect {
        s.execute("effect.apply", &json!({"effect":effect,"item":null,"params":{}})).unwrap();
    }
    (s, id)
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn run(s: &mut Session, p: Value) -> crate::Result<Value> {
    s.execute("effect.expandAsWireframes", &p)
}

#[test]
fn a_3d_object_becomes_a_selected_group_of_stroked_edges_in_one_undo_step() {
    for effect in [
        "threeD.extrude",
        "threeD.classicExtrude",
        "threeD.revolve",
        "threeD.classicRevolve",
        "threeD.inflate",
        "threeD.rotate",
        "threeD.classicRotate",
    ] {
        let (mut s, id) = setup(Some(effect));
        std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().name = Some("Box".into());
        let before = s.doc().unwrap().doc.clone();
        let history = undo_len(&s);
        let out = run(&mut s, json!({})).unwrap_or_else(|e| panic!("{effect}: {e}"));
        assert_eq!(out["ids"], json!([id.0]), "{effect}");
        let st = s.doc().unwrap();
        let group = st.doc.node(id).unwrap();
        let NodeKind::Group { children, clip: false } = &group.kind else { panic!("{effect}: a group") };
        assert!(!children.is_empty(), "{effect}");
        assert_eq!(group.name.as_deref(), Some("Box"), "{effect}");
        assert!(group.appearance.effects.is_empty(), "{effect}: the effect is gone");
        for c in children {
            assert!(c.path_data().is_some(), "{effect}: paths only");
            let [AppearanceItem::Stroke(stroke)] = c.appearance.items.as_slice() else { panic!("{effect}: one stroke and no fill") };
            assert_eq!(stroke.width, super::WIRE_WEIGHT);
        }
        assert_eq!(st.selection.objects, vec![id], "{effect}");
        assert_eq!(undo_len(&s), history + 1, "{effect}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc, before, "{effect}");
    }
}

#[test]
fn the_group_keeps_the_object_s_transparency() {
    let (mut s, id) = setup(Some("threeD.extrude"));
    s.execute("transparency.set", &json!({"opacity": 50, "knockout": "on"})).unwrap();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().bbox_angle = 0.25;
    let before = s.doc().unwrap().doc.node(id).unwrap().clone();
    run(&mut s, json!({})).unwrap();
    let group = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!((group.opacity - 0.5).abs() < 1e-6, "{}", group.opacity);
    assert_eq!((group.opacity, group.blend, group.knockout), (before.opacity, before.blend, before.knockout));
    assert_eq!(group.bbox_angle, 0.25, "the bounding box keeps its angle");
}

#[test]
fn a_layer_with_a_3d_effect_stays_a_layer_holding_the_edges() {
    let (mut s, _) = setup(None);
    let layer = s.doc().unwrap().doc.layers[0].id;
    s.execute("effect.apply", &json!({"ids":[layer.0],"effect":"threeD.extrude","item":null,"params":{}})).unwrap();
    let history = undo_len(&s);
    // An id given twice is expanded once.
    let out = run(&mut s, json!({"ids":[layer.0, layer.0]})).unwrap();
    assert_eq!(out["ids"], json!([layer.0]));
    let st = s.doc().unwrap();
    let node = st.doc.node(layer).unwrap();
    assert!(matches!(node.kind, NodeKind::Layer { .. }));
    assert!(node.appearance.effects.is_empty());
    let children = node.children().unwrap();
    assert!(!children.is_empty() && children.iter().all(|c| c.path_data().is_some()));
    assert_eq!(st.selection.objects, children.iter().map(|c| c.id).collect::<Vec<_>>());
    assert_eq!(undo_len(&s), history + 1);
}

/// A layer's ruler guides are not art: they stay on the layer, after the edges, and are not selected.
#[test]
fn a_layer_keeps_its_ruler_guides() {
    let (mut s, _) = setup(None);
    let layer = s.doc().unwrap().doc.layers[0].id;
    s.execute("guide.add", &json!({"vertical": true, "pos": 50})).unwrap();
    let guides = |s: &Session| -> Vec<NodeId> {
        s.doc().unwrap().doc.node(layer).unwrap().children().unwrap().iter().filter(|c| c.is_ruler_guide()).map(|c| c.id).collect()
    };
    let before = guides(&s);
    assert_eq!(before.len(), 1);
    s.execute("effect.apply", &json!({"ids":[layer.0],"effect":"threeD.extrude","item":null,"params":{}})).unwrap();
    run(&mut s, json!({"ids":[layer.0]})).unwrap();
    assert_eq!(guides(&s), before);
    let st = s.doc().unwrap();
    let children = st.doc.node(layer).unwrap().children().unwrap();
    assert!(children.len() > 1 && children.last().is_some_and(|c| c.is_ruler_guide()));
    assert!(!st.selection.objects.is_empty() && st.selection.objects.iter().all(|id| !before.contains(id)));
}

/// The number of segments in the wireframe group `id`.
fn segments(s: &Session, id: NodeId) -> usize {
    let st = s.doc().unwrap();
    st.doc.node(id).unwrap().children().unwrap().iter().flat_map(|c| &c.path_data().unwrap().subpaths).map(|sp| sp.segment_count()).sum()
}

#[test]
fn a_box_expands_to_the_nine_edges_of_its_three_faces() {
    let (mut s, id) = setup(Some("threeD.extrude"));
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    run(&mut s, json!({"ids":[id.0]})).unwrap();
    assert_eq!(segments(&s, id), 9);
    // A stroke is outlined before the effect, so its own solid adds its edges.
    let (mut s, id) = setup(Some("threeD.extrude"));
    run(&mut s, json!({"ids":[id.0]})).unwrap();
    assert!(segments(&s, id) > 9);
}

#[test]
fn objects_without_a_3d_effect_are_left_and_none_at_all_is_an_error() {
    let (mut s, plain) = setup(None);
    let history = undo_len(&s);
    let before = s.doc().unwrap().doc.clone();
    let e = run(&mut s, json!({})).unwrap_err().to_string();
    assert!(e.contains("no selected or given object has a visible 3D effect"), "{e}");
    assert_eq!(undo_len(&s), history, "no undo step when nothing changes");
    assert_eq!(s.doc().unwrap().doc, before);

    // A hidden 3D effect draws nothing to expand.
    s.execute("effect.apply", &json!({"effect":"threeD.extrude","item":null,"params":{}})).unwrap();
    s.execute("effect.setParams", &json!({"index":0,"item":null,"visible":false})).unwrap();
    assert!(run(&mut s, json!({})).is_err());

    // With nothing selected.
    s.execute("select.none", &json!({})).unwrap();
    let e = run(&mut s, json!({})).unwrap_err().to_string();
    assert!(e.contains("select an object with a 3D effect"), "{e}");

    // A second object with a 3D effect expands; the plain one is left as it is.
    let other = NodeId(s.execute("path.create", &json!({"d":"M250 250L300 250L300 300Z"})).unwrap()["id"].as_u64().unwrap());
    s.execute("effect.remove", &json!({"ids":[plain.0],"index":0,"item":null})).unwrap();
    s.execute("paint.setFill", &json!({"ids":[other.0],"color":"#ff0000"})).unwrap();
    s.execute("effect.apply", &json!({"ids":[other.0],"effect":"threeD.extrude","item":null,"params":{}})).unwrap();
    let kept = s.doc().unwrap().doc.node(plain).unwrap().clone();
    let out = run(&mut s, json!({"ids":[plain.0, other.0]})).unwrap();
    assert_eq!(out["ids"], json!([other.0]));
    assert_eq!(s.doc().unwrap().doc.node(plain).unwrap(), &kept);
}

/// A live-effect plug-in after a fill's 3D effect that links its pieces to one of the document's
/// swatches reshapes the edges, as it reshapes the drawn surface.
#[test]
fn a_painted_effect_after_a_fill_s_3d_effect_finds_the_document_s_swatches() {
    use vectorcraft_geom::{PathData, Point, SubPath};
    let mut leaf = serde_json::to_value(vectorcraft_color::Paint::solid(vectorcraft_color::Color::from_hex("#00ff00").unwrap())).unwrap();
    leaf["swatch"] = json!("Leaf");
    let corners = [Point::new(300.0, 300.0), Point::new(320.0, 300.0), Point::new(320.0, 320.0), Point::new(300.0, 320.0)];
    let output = json!({"objects": [{"path": PathData::single(SubPath::polyline(&corners, true)), "fills": [leaf]}]}).to_string();
    let manifest = r#"{"id":"test.engine.wireleaf","name":"Test wireleaf","version":"1","kind":"effect","api":2,"paintOutput":true,"params":{}}"#;
    let wasm = wat::parse_str(vectorcraft_plugins::wat::returning(manifest, &output)).unwrap();
    for effect in ["threeD.extrude", "threeD.revolve"] {
        let (mut s, id) = setup(None);
        s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "replace": true})).unwrap();
        s.execute("paint.setStroke", &json!({"none": true})).unwrap();
        s.execute("swatch.new", &json!({"name": "Leaf", "color": "#00ff00", "global": true})).unwrap();
        let items = &s.doc().unwrap().doc.node(id).unwrap().appearance.items;
        let fill = items.iter().position(|i| matches!(i, AppearanceItem::Fill(_))).unwrap();
        for e in [effect, "plugin.test.engine.wireleaf"] {
            s.execute("effect.apply", &json!({"ids": [id.0], "effect": e, "item": fill, "params": {}})).unwrap();
        }
        run(&mut s, json!({"ids": [id.0]})).unwrap_or_else(|e| panic!("{effect}: {e}"));
        let b = s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (300.0, 300.0, 320.0, 320.0), "{effect}: the plug-in's square");
        assert_eq!(segments(&s, id), 4, "{effect}");
    }
}

#[test]
fn bad_ids_are_errors() {
    let (mut s, _) = setup(Some("threeD.extrude"));
    for p in [json!({"ids":[9999]}), json!({"ids":"x"}), json!({"ids":[-1]})] {
        let history = undo_len(&s);
        assert!(run(&mut s, p.clone()).is_err(), "{p}");
        assert_eq!(undo_len(&s), history, "{p}");
    }
}
