#![allow(clippy::unwrap_used, clippy::panic)]
use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};

use crate::Session;

const TYPES: [&str; 6] = ["threeD.extrude", "threeD.classicExtrude", "threeD.inflate", "threeD.rotate", "threeD.classicRotate", "threeD.revolve"];

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

/// A clipping group, a blend and a symbol instance, each made through its command.
fn sources(s: &mut Session) -> Vec<(&'static str, NodeId)> {
    let art = rect(s, 20.0, 20.0, 60.0, 60.0);
    let clip = rect(s, 40.0, 10.0, 20.0, 80.0);
    select(s, &[art, clip]);
    s.execute("object.clippingMask.make", &json!({})).unwrap();
    let clipped = s.doc().unwrap().selection.objects[0];
    let a = rect(s, 120.0, 20.0, 20.0, 20.0);
    let b = rect(s, 200.0, 60.0, 30.0, 30.0);
    select(s, &[a, b]);
    let blend = id_of(&s.execute("object.blend.make", &json!({})).unwrap());
    let art = rect(s, 20.0, 200.0, 50.0, 40.0);
    let instance = id_of(&s.execute("symbol.new", &json!({"name": "Box", "ids": [art.0]})).unwrap());
    vec![("clipping group", clipped), ("blend", blend), ("symbol instance", instance)]
}

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

#[test]
fn every_three_d_type_applies_to_clipping_groups_blends_and_symbol_instances_in_one_undo_step() {
    for id in TYPES {
        let mut s = session();
        for (what, target) in sources(&mut s) {
            select(&mut s, &[target]);
            let undo = s.doc().unwrap().history.undo.len();
            s.execute("effect.apply", &json!({"effect": id, "item": null})).unwrap_or_else(|e| panic!("{id} on a {what}: {e}"));
            assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{id} on a {what}");
            let n = s.doc().unwrap().doc.node(target).unwrap().clone();
            assert_eq!(n.appearance.effects.iter().filter(|e| e.id == id).count(), 1, "{id} on a {what}");
            // The object stays what it is, and the effect draws its solid.
            assert!(!matches!(n.kind, NodeKind::Path { .. }), "{id} on a {what}");
            let d = &s.doc().unwrap().doc;
            let art = vectorcraft_render::effects::revolve_art_in(Some(d), &n).or_else(|| vectorcraft_render::effects::solid_art_in(Some(d), &n));
            assert!(art.is_some_and(|a| a.visual_bounds().is_some()), "{id} on a {what} draws its solid");
            // Edited through the dialog's command too.
            s.execute("effect.setParams", &json!({"index": 0, "item": null, "params": {"rotationY": 20.0}}))
                .unwrap_or_else(|e| panic!("{id} on a {what}: {e}"));
        }
    }
}

#[test]
fn a_group_holding_an_image_refuses_every_three_d_type_and_names_image_trace() {
    let mut s = session();
    let a = rect(&mut s, 20.0, 20.0, 60.0, 60.0);
    let image = s
        .edit("Place", |d, _| {
            let id = d.alloc_id();
            let img = vectorcraft_doc::ImageObject {
                key: "k".into(),
                width: 4,
                height: 4,
                xf: vectorcraft_geom::Affine::translate((100.0, 100.0)),
                link: None,
                placement: Default::default(),
                pixels: None,
            };
            let (parent, index, _) = d.position(a).unwrap();
            d.insert(parent, index + 1, vectorcraft_doc::Node::new(id, NodeKind::Image(Box::new(img))))?;
            Ok(id)
        })
        .unwrap();
    select(&mut s, &[a, image]);
    s.execute("object.group", &json!({})).unwrap();
    for target in [image, s.doc().unwrap().selection.objects[0]] {
        select(&mut s, &[target]);
        for id in TYPES {
            let before = s.doc().unwrap().doc.clone();
            let err = s.execute("effect.apply", &json!({"effect": id, "item": null})).unwrap_err().to_string();
            assert!(err.contains("Object › Image Trace › Make"), "{id}: {err}");
            assert_eq!(s.doc().unwrap().doc, before, "{id}: nothing changed");
        }
    }
}
