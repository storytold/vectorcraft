//! Opacity-mask editing and the Transparency panel's command reach (M3.25): commands act on the
//! masked object while its mask is edited, accept ids without a selection, take opacity in percent,
//! report mixed values, and saves and exports never carry the editing layer.

use serde_json::{Value, json};

use super::*;
use crate::cmd::maskedit::MASK_EDIT_LAYER;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 300, "height": 300})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, w: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": w, "height": 200})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

/// An object (0..200) masked by a white rectangle (0..100), with the mask being edited.
fn editing_mask(s: &mut Session) -> NodeId {
    let obj = rect(s, 0.0, 200.0);
    let m = rect(s, 0.0, 100.0);
    s.execute("paint.setFill", &json!({"color": "#ffffff", "ids": [m.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [obj.0, m.0]})).unwrap();
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    s.execute("transparency.editOpacityMask", &json!({})).unwrap();
    obj
}

fn doc(s: &Session) -> &Document {
    &s.doc().unwrap().doc
}

fn mask(s: &Session, id: NodeId) -> Option<&vectorcraft_doc::OpacityMask> {
    doc(s).node(id).and_then(|n| n.mask.as_deref())
}

fn has_edit_layer(d: &Document) -> bool {
    d.layers.iter().any(|l| l.name.as_deref() == Some(MASK_EDIT_LAYER))
}

fn data(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
}

#[test]
fn mask_options_act_on_the_masked_object_while_editing() {
    let mut s = session();
    let obj = editing_mask(&mut s);
    // The selection is the mask art; the panel's commands still reach the object's mask.
    assert!(!s.doc().unwrap().selection.contains(obj));
    s.execute("transparency.setOpacityMask", &json!({"clip": false, "invert": true})).unwrap();
    s.execute("transparency.unlinkOpacityMask", &json!({})).unwrap();
    s.execute("transparency.disableOpacityMask", &json!({})).unwrap();
    let m = mask(&s, obj).unwrap();
    assert!(!m.clip && m.invert && !m.linked && m.disabled);
    assert!(doc(&s).mask_edit.is_some(), "still editing");
    let info = s.execute("transparency.opacityMaskInfo", &json!({})).unwrap();
    assert_eq!(info[0]["id"], obj.0);
    // Opacity and blend go to the masked object too, not to the mask art.
    s.execute("transparency.set", &json!({"opacity": 40, "blend": "Multiply"})).unwrap();
    let n = doc(&s).node(obj).unwrap();
    assert!((n.opacity - 0.4).abs() < 1e-6 && n.blend == vectorcraft_color::BlendMode::Multiply);
    let info = s.execute("transparency.info", &json!({})).unwrap();
    assert_eq!((info["ids"].clone(), info["opacity"].clone(), info["editingMask"].clone()), (json!([obj.0]), json!(40.0), json!(obj.0)));
}

#[test]
fn release_while_editing_leaves_editing_first() {
    let mut s = session();
    let obj = editing_mask(&mut s);
    // Widen the mask while editing; releasing keeps that edit.
    s.execute("object.move", &json!({"dx": 50, "dy": 0})).unwrap();
    s.execute("transparency.releaseOpacityMask", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert!(st.doc.mask_edit.is_none() && st.isolation.is_none() && !has_edit_layer(&st.doc));
    assert!(mask(&s, obj).is_none());
    let layer = &st.doc.layers[0];
    let kids = layer.children().unwrap();
    assert_eq!(kids.len(), 2, "the object and its released mask art");
    assert_eq!(kids[1].geometric_bounds().unwrap().x0, 50.0);
    assert_eq!(st.active_layer, Some(layer.id));
    // One undo step restores the mask and editing mode.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(mask(&s, obj).is_some() && doc(&s).mask_edit.is_some());
}

#[test]
fn one_object_gets_an_empty_mask_drawn_in_editing_mode() {
    let mut s = session();
    let obj = rect(&mut s, 0.0, 200.0);
    let r = s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    assert_eq!(r, json!({"id": obj.0, "editing": true}));
    let me = doc(&s).mask_edit.unwrap();
    assert_eq!(me.object, obj);
    assert_eq!(s.doc().unwrap().isolation, Some(me.layer));
    assert!(s.doc().unwrap().selection.is_empty());
    // An empty clipping mask hides the object; drawing white in editing mode reveals it there.
    let render = |s: &Session| vectorcraft_render::Renderer::new().render(doc(s), 300, 300, vectorcraft_geom::Affine::IDENTITY, &Default::default());
    assert_eq!(render(&s).pixel(50, 100)[3], 0);
    let m = s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 200})).unwrap()["id"].as_u64().unwrap();
    s.execute("paint.setFill", &json!({"color": "#ffffff", "ids": [m]})).unwrap();
    let img = render(&s);
    assert!(img.pixel(50, 100)[3] > 200);
    assert_eq!(img.pixel(150, 100)[3], 0);
    s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    assert_eq!(mask(&s, obj).unwrap().art.geometric_bounds().unwrap().x1, 100.0);
    // Making a mask while editing one is refused.
    s.execute("transparency.editOpacityMask", &json!({"id": obj.0})).unwrap();
    assert!(s.execute("transparency.makeOpacityMask", &json!({"ids": [obj.0]})).is_err());
    // One undo step undoes making the empty mask and entering editing; isolation follows.
    let mut s = session();
    let obj = rect(&mut s, 0.0, 200.0);
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    let layer = doc(&s).mask_edit.unwrap().layer;
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(mask(&s, obj).is_none() && doc(&s).mask_edit.is_none());
    assert_eq!(s.doc().unwrap().isolation, None);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!((s.doc().unwrap().isolation, s.doc().unwrap().active_layer), (Some(layer), Some(layer)));
}

#[test]
fn mask_and_transparency_commands_take_ids_without_a_selection() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 200.0), rect(&mut s, 0.0, 100.0));
    s.execute("select.none", &json!({})).unwrap();
    let r = s.execute("transparency.makeOpacityMask", &json!({"ids": [b.0, a.0], "invert": true})).unwrap();
    assert_eq!(r["id"], a.0, "the top object (b) masks the one below, whatever the order given");
    s.execute("select.none", &json!({})).unwrap();
    s.execute("transparency.setOpacityMask", &json!({"id": a.0, "clip": false})).unwrap();
    s.execute("transparency.disableOpacityMask", &json!({"ids": [a.0]})).unwrap();
    let m = mask(&s, a).unwrap();
    assert!(!m.clip && m.invert && m.disabled);
    s.execute("transparency.set", &json!({"ids": [a.0], "opacity": 25})).unwrap();
    assert!((doc(&s).node(a).unwrap().opacity - 0.25).abs() < 1e-6);
    assert!(s.execute("transparency.set", &json!({"opacity": 25})).is_err(), "no selection, no ids");
    let layer = s.execute("transparency.editOpacityMask", &json!({"id": a.0})).unwrap()["layer"].as_u64().unwrap();
    assert_eq!(doc(&s).mask_edit.unwrap().layer, NodeId(layer));
    s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.execute("transparency.releaseOpacityMask", &json!({"id": a.0})).unwrap();
    assert!(mask(&s, a).is_none());
    let err = s.execute("transparency.releaseOpacityMask", &json!({"id": a.0})).unwrap_err();
    assert!(err.to_string().contains("no target object has an opacity mask"), "{err}");
}

#[test]
fn opacity_is_a_percentage_everywhere() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 100.0);
    s.execute("appearance.setItem", &json!({"index": 0, "opacity": 1})).unwrap();
    let fill_opacity = |s: &Session| match &doc(s).node(a).unwrap().appearance.items[0] {
        vectorcraft_doc::AppearanceItem::Fill(f) => f.opacity,
        vectorcraft_doc::AppearanceItem::Stroke(st) => st.opacity,
    };
    assert!((fill_opacity(&s) - 0.01).abs() < 1e-6);
    s.execute("appearance.setItem", &json!({"index": 0, "opacity": 100})).unwrap();
    assert_eq!(fill_opacity(&s), 1.0);
    s.execute("object.setProps", &json!({"opacity": 1})).unwrap();
    assert!((doc(&s).node(a).unwrap().opacity - 0.01).abs() < 1e-6);
    s.execute("object.setProps", &json!({"opacity": 150})).unwrap();
    assert_eq!(doc(&s).node(a).unwrap().opacity, 1.0);
    s.execute("transparency.set", &json!({"opacity": 0.5})).unwrap();
    assert!((doc(&s).node(a).unwrap().opacity - 0.005).abs() < 1e-6);
    // Appearance items take ids without a selection too.
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute("appearance.setItem", &json!({"index": 0, "opacity": 30})).is_err(), "no selection, no ids");
    s.execute("appearance.setItem", &json!({"index": 0, "ids": [a.0], "opacity": 30})).unwrap();
    assert!((fill_opacity(&s) - 0.3).abs() < 1e-6);
}

#[test]
fn info_reports_mixed_values_as_null() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 100.0), rect(&mut s, 120.0, 100.0));
    s.execute("transparency.set", &json!({"ids": [a.0], "opacity": 50, "isolate": true})).unwrap();
    s.execute("transparency.set", &json!({"ids": [a.0, b.0], "blend": "Screen"})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let info = s.execute("transparency.info", &json!({})).unwrap();
    assert_eq!(
        info,
        json!({"ids": [a.0, b.0], "opacity": null, "blend": "Screen", "isolate": null, "knockout": "neutral", "knockoutShape": false, "editingMask": null, "pageIsolatedBlending": false, "pageKnockoutGroup": false})
    );
    let one = s.execute("transparency.info", &json!({"id": a.0})).unwrap();
    assert_eq!((one["opacity"].clone(), one["isolate"].clone()), (json!(50.0), json!(true)));
    let i = s.transparency_info();
    assert_eq!((i.opacity, i.blend, i.knockout), (None, Some(vectorcraft_color::BlendMode::Screen), Some(vectorcraft_doc::Knockout::Neutral)));
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(s.execute("transparency.info", &json!({})).unwrap()["opacity"], Value::Null);
    // Many targets are gathered in one walk of the tree.
    let ids: Vec<u64> = (0..12).map(|i| rect(&mut s, i as f64 * 20.0, 10.0).0).collect();
    s.execute("transparency.set", &json!({"ids": ids, "opacity": 30})).unwrap();
    let many = s.execute("transparency.info", &json!({"ids": ids})).unwrap();
    assert_eq!(many["opacity"], json!(30.0));
}

#[test]
fn saving_while_editing_writes_no_editing_layer() {
    let mut s = session();
    let obj = editing_mask(&mut s);
    s.execute("object.move", &json!({"dx": 50, "dy": 0})).unwrap();
    let bytes = data(&s.execute("document.save", &json!({"format": "vectorcraft"})).unwrap());
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert!(back.mask_edit.is_none() && !has_edit_layer(&back));
    assert_eq!(back.layers.len(), 1);
    // The edit made while editing is in the saved mask.
    assert_eq!(back.node(obj).unwrap().mask.as_ref().unwrap().art.geometric_bounds().unwrap().x0, 50.0);
    // Still editing in the session.
    assert!(doc(&s).mask_edit.is_some() && has_edit_layer(doc(&s)));
    // A file saved mid-edit by an older version (editing record and layer written) opens clean.
    let mut v: Value = serde_json::from_slice(&vectorcraft_format::save(doc(&s), false)).unwrap();
    let full = serde_json::to_value(doc(&s)).unwrap();
    v["document"]["layers"] = full["layers"].clone();
    v["document"]["mask_edit"] = json!({"object": obj.0, "layer": doc(&s).mask_edit.unwrap().layer.0});
    let old = vectorcraft_format::load(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(old.mask_edit.is_none() && !has_edit_layer(&old) && old.layers.len() == 1);
}

#[test]
fn exports_while_editing_leave_the_editing_layer_out() {
    let mut s = session();
    editing_mask(&mut s);
    let svg = |s: &mut Session| s.execute("document.serialize", &json!({"format": "svg"})).unwrap()["text"].as_str().unwrap().to_string();
    let pdf = |s: &mut Session| data(&s.execute("document.export", &json!({"format": "pdf"})).unwrap());
    let png = |s: &mut Session| data(&s.execute("document.export", &json!({"format": "png"})).unwrap());
    let (svg_editing, pdf_editing, png_editing) = (svg(&mut s), pdf(&mut s), png(&mut s));
    assert!(!svg_editing.contains("Opacity Mask Editing") && !svg_editing.contains("Opacity_Mask_Editing"));
    // Before the fix the editing layer's art was written too: the PDF would differ from the one
    // the document gives without it (same length: only the creation date may differ).
    let raw = crate::export_pdf(doc(&s), &Default::default()).unwrap();
    assert_ne!(raw.len(), pdf_editing.len(), "the editing layer would add content");
    s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    assert_eq!(svg(&mut s), svg_editing);
    assert_eq!(pdf(&mut s).len(), pdf_editing.len());
    assert_eq!(png(&mut s), png_editing);
}

#[test]
fn undo_and_redo_walk_in_and_out_of_mask_editing() {
    let mut s = session();
    let obj = rect(&mut s, 0.0, 200.0);
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    let layer = doc(&s).mask_edit.unwrap().layer;
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 200})).unwrap();
    s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    let state = |s: &Session| {
        let st = s.doc().unwrap();
        (st.doc.mask_edit.map(|m| m.layer), st.isolation, st.active_layer == Some(layer), has_edit_layer(&st.doc))
    };
    let editing = (Some(layer), Some(layer), true, true);
    assert_eq!(state(&s), (None, None, false, false));
    // Back into editing with the drawn art, then before it, then before the mask.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(state(&s), editing);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(state(&s), editing);
    assert!(doc(&s).node(layer).unwrap().children().unwrap().is_empty());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(state(&s), (None, None, false, false));
    assert!(mask(&s, obj).is_none());
    // And forward again.
    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(state(&s), editing);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(state(&s), (None, None, false, false));
    assert_eq!(mask(&s, obj).unwrap().art.geometric_bounds().unwrap().x1, 100.0);
}
