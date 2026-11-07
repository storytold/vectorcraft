//! Layer clipping masks (M3.75): the Layers panel button makes the top object of the current layer
//! (or of the selected group) clip the rest, and pressing it again releases the mask.

use serde_json::json;

use crate::tests_clip::{assert_outputs_agree, hit, id_of, opaque, render, select, session};
use crate::*;

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn toggle(s: &mut Session) -> bool {
    s.execute("layer.clippingMask.toggle", &json!({})).unwrap()["clip"].as_bool().unwrap()
}

fn layer(s: &Session) -> vectorcraft_doc::Node {
    let st = s.doc().unwrap();
    st.doc.node(st.current_layer().unwrap()).unwrap().clone()
}

/// A full-page square under a 50 × 50 square at 25..75 (the clip to be), nothing selected.
fn art_and_clip(s: &mut Session) -> (NodeId, NodeId) {
    let art = rect(s, 0.0, 0.0, 100.0, 100.0);
    let clip = rect(s, 25.0, 25.0, 50.0, 50.0);
    s.execute("select.none", &json!({})).unwrap();
    (art, clip)
}

#[test]
fn the_top_object_clips_the_layer_and_pressing_again_releases_it() {
    let mut s = session();
    let (art, clip) = art_and_clip(&mut s);
    assert!(toggle(&mut s));
    let l = layer(&s);
    assert!(l.clips());
    // The clipping path loses its paint and goes to the bottom, so art added on top is clipped.
    let c = &l.children().unwrap()[0];
    assert_eq!(c.id, clip);
    assert!(c.appearance.fill_paint().is_none() && matches!(c.kind, NodeKind::Path { clipping: true, .. }));
    let r = render(&s);
    assert!(opaque(&r, 50, 50) && !opaque(&r, 10, 10));
    // Clicks only reach the art inside the clip.
    assert_eq!(hit(&s, 50.0, 50.0), Some(art));
    assert_eq!(hit(&s, 10.0, 10.0), None);
    rect(&mut s, 0.0, 80.0, 100.0, 20.0);
    assert!(!opaque(&render(&s), 50, 90), "new art is clipped");
    // Again: released, the clipping path stays unpainted.
    s.execute("select.none", &json!({})).unwrap();
    assert!(!toggle(&mut s));
    let l = layer(&s);
    assert!(!l.clips());
    let c = &l.children().unwrap()[0];
    assert!(c.appearance.fill_paint().is_none() && matches!(c.kind, NodeKind::Path { clipping: false, .. }));
    let r = render(&s);
    assert!(opaque(&r, 10, 10) && opaque(&r, 50, 90));
}

#[test]
fn making_a_layer_clipping_mask_is_one_undo_step() {
    let mut s = session();
    let (_, clip) = art_and_clip(&mut s);
    let before = layer(&s);
    toggle(&mut s);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(layer(&s), before, "the clip path is back on top with its paint");
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(layer(&s).clips());
    assert_eq!(layer(&s).children().unwrap()[0].id, clip);
}

#[test]
fn layer_clipping_masks_save_and_export() {
    let mut s = session();
    art_and_clip(&mut s);
    toggle(&mut s);
    let doc = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
    assert!(back.layers[0].clips());
    assert_eq!(back, **doc);
    // An unclipped layer writes no `clip` key (older files stay as they were).
    let plain = String::from_utf8(vectorcraft_format::save(&session().doc().unwrap().doc, false)).unwrap();
    assert!(!plain.contains("\"clip\""), "{plain}");
    // SVG and PDF clip the same way as the canvas.
    assert_outputs_agree(&mut s, &[(50, 50), (10, 10), (30, 30), (80, 80), (74, 26)]);
}

#[test]
fn a_selected_group_becomes_a_clip_group() {
    let mut s = session();
    let (art, clip) = art_and_clip(&mut s);
    select(&mut s, &[art, clip]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    select(&mut s, &[g]);
    assert!(toggle(&mut s));
    let n = s.doc().unwrap().doc.node(g).unwrap().clone();
    assert!(matches!(n.kind, NodeKind::Group { clip: true, .. }));
    assert_eq!(n.children().unwrap()[0].id, clip);
    assert!(!layer(&s).clips(), "the layer is left alone");
    assert!(!opaque(&render(&s), 10, 10));
    assert!(!toggle(&mut s));
    assert!(!s.doc().unwrap().doc.node(g).unwrap().clips());
}

#[test]
fn the_top_object_must_be_able_to_clip() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 50.0, 50.0);
    let b = rect(&mut s, 50.0, 50.0, 50.0, 50.0);
    select(&mut s, &[a, b]);
    s.execute("object.group", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let before = layer(&s);
    assert!(s.execute("layer.clippingMask.toggle", &json!({})).is_err(), "a group on top");
    assert_eq!(layer(&s), before);
    // An explicit id picks the layer even with a group selected.
    let l = before.id;
    let sub = id_of(&s.execute("layer.newSublayer", &json!({"parent": l.0})).unwrap());
    assert_eq!(s.doc().unwrap().doc.node(l).unwrap().children().unwrap().last().unwrap().id, sub, "on top of the layer's contents");
    let err = s.execute("layer.clippingMask.toggle", &json!({"id": sub.0}));
    assert!(err.is_err(), "an empty sublayer has nothing to clip by");
}

#[test]
fn send_to_back_in_a_clipping_layer_keeps_the_clipping_path_at_the_bottom() {
    // #3188, for a clipping layer: the art went under the clipping path and became the clip.
    let mut s = session();
    let (art, clip) = art_and_clip(&mut s);
    assert!(toggle(&mut s));
    let top = rect(&mut s, 0.0, 0.0, 40.0, 40.0);
    select(&mut s, &[top]);
    s.execute("object.arrange.sendToBack", &json!({})).unwrap();
    let ids: Vec<NodeId> = layer(&s).children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(ids, [clip, top, art]);
    assert!(opaque(&render(&s), 50, 50), "the art inside the clip still shows");
}
