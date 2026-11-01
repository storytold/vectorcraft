//! Opacity-mask editing and the Transparency panel's command reach (M3.25): commands act on the
//! masked object while its mask is edited, accept ids without a selection, take opacity in percent,
//! report mixed values, and saves and exports never carry the editing layer. Mask editing is not
//! entered in isolation mode, and isolation mode is not entered while a mask is edited.

use serde_json::{Value, json};
use vectorcraft_geom::Affine;
use vectorcraft_render::{RenderOptions, Renderer};
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::cmd::maskedit::MASK_EDIT_LAYER;
use crate::tests::{history, select_with_a_step_to_redo};
use crate::*;

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
    doc(s).node(id).and_then(|n| n.extras().mask.as_deref())
}

fn has_edit_layer(d: &Document) -> bool {
    d.layers.iter().any(|l| l.name.as_deref() == Some(MASK_EDIT_LAYER))
}

fn data(v: &Value) -> Vec<u8> {
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
}

/// #2727: the canvas passes `isolation` while mask art is edited; the editing layer is not drawn on
/// the page, so isolation dimming must not run or the artwork washes out.
#[test]
fn mask_editing_does_not_dim_the_canvas() {
    let mut s = session();
    editing_mask(&mut s);
    let st = s.doc().unwrap();
    let layer = st.isolation.expect("mask editing isolates the temporary layer");
    let render = |opts: &RenderOptions| Renderer::new().render(&st.doc, 300, 300, Affine::IDENTITY, opts);
    let plain = render(&RenderOptions::default());
    let canvas = render(&RenderOptions { isolated: Some(layer), ..Default::default() });
    assert_eq!(plain.pixel(50, 100), canvas.pixel(50, 100), "inside the mask");
    assert_eq!(plain.pixel(250, 100), canvas.pixel(250, 100), "outside the mask");
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

/// Disable, Enable, Unlink and Link Opacity Mask and Opacity Mask Options with the flags the masks
/// already have record no undo step, on the selection, with ids and while the mask is being edited:
/// the step to redo stays, the document stays unmodified and the journal still has the command. A
/// call that changes a flag on one of the masks is one undo step. A target without a mask and a
/// call that gives no flag are still errors.
#[test]
fn mask_options_already_set_record_no_undo_step() {
    let mut s = session();
    let masked = |s: &mut Session, x: f64| {
        let (obj, m) = (rect(s, x, 100.0), rect(s, x, 50.0));
        s.execute("select.set", &json!({"ids": [obj.0, m.0]})).unwrap();
        s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        obj
    };
    // `a`'s mask is as made (clipping, not inverted, enabled, linked); `b`'s has each flag the other
    // way.
    let (a, b) = (masked(&mut s, 0.0), masked(&mut s, 150.0));
    s.execute("transparency.setOpacityMask", &json!({"id": b.0, "clip": false, "invert": true, "disabled": true, "linked": false})).unwrap();
    select_with_a_step_to_redo(&mut s, &[a]);
    let before = history(&s);
    assert_eq!((before.1, before.2), (1, false), "a step to redo, and nothing unsaved");
    let calls = [
        ("transparency.enableOpacityMask", json!({})),
        ("transparency.linkOpacityMask", json!({})),
        ("transparency.setOpacityMask", json!({"clip": true, "invert": false, "disabled": false, "linked": true})),
        ("transparency.disableOpacityMask", json!({"id": b.0})),
        ("transparency.unlinkOpacityMask", json!({"ids": [b.0]})),
        ("transparency.setOpacityMask", json!({"ids": [b.0], "clip": false, "invert": true})),
    ];
    for (id, p) in calls {
        assert_eq!(s.execute(id, &p).unwrap(), Value::Null, "{id} {p}");
        assert_eq!(history(&s), before, "{id} {p}");
        assert_eq!(s.journal.last().unwrap(), &(id.to_string(), p));
    }
    // While `a`'s mask is edited, the selection is its mask art and the commands act on `a`.
    s.execute("transparency.editOpacityMask", &json!({"id": a.0})).unwrap();
    let art = s.doc().unwrap().selection.objects.clone();
    select_with_a_step_to_redo(&mut s, &art);
    let before = history(&s);
    assert_eq!((before.1, before.2), (1, false), "a step to redo, and nothing unsaved while editing");
    let calls = [
        ("transparency.enableOpacityMask", json!({})),
        ("transparency.linkOpacityMask", json!({})),
        ("transparency.setOpacityMask", json!({"clip": true, "linked": true})),
    ];
    for (id, p) in calls {
        assert_eq!(s.execute(id, &p).unwrap(), Value::Null, "{id} {p} while editing");
        assert_eq!(history(&s), before, "{id} {p} while editing");
        assert_eq!(s.journal.last().unwrap(), &(id.to_string(), p));
    }
    assert!(doc(&s).mask_edit.is_some(), "still editing");
    // `b`'s mask is disabled and `a`'s is not: disabling both is one undo step, and it clears Redo.
    s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    select_with_a_step_to_redo(&mut s, &[a, b]);
    let undo = history(&s).0;
    s.execute("transparency.disableOpacityMask", &json!({})).unwrap();
    assert_eq!(history(&s), (undo + 1, 0, true));
    assert!(mask(&s, a).unwrap().disabled && mask(&s, b).unwrap().disabled);
    let plain = rect(&mut s, 0.0, 20.0);
    let err = s.execute("transparency.enableOpacityMask", &json!({"id": plain.0})).unwrap_err();
    assert!(err.to_string().contains("no target object has an opacity mask"), "{err}");
    assert!(s.execute("transparency.setOpacityMask", &json!({"id": a.0})).is_err(), "no flag given");
}

/// #4538: while a mask is edited the Layers panel lists only the mask, so New Layer and New
/// Sublayer are refused instead of adding layers no row shows; they work again afterwards.
#[test]
fn new_layers_are_refused_while_a_mask_is_edited() {
    let mut s = session();
    editing_mask(&mut s);
    let layers = doc(&s).layers.len();
    for cmd in ["layer.new", "layer.newSublayer"] {
        assert!(s.execute(cmd, &json!({})).is_err(), "{cmd} while the mask is edited");
    }
    assert_eq!(doc(&s).layers.len(), layers);
    s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    let layers = doc(&s).layers.len();
    s.execute("layer.new", &json!({})).unwrap();
    assert_eq!(doc(&s).layers.len(), layers + 1);
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

/// Exit Isolation Mode while a mask is edited leaves mask editing as Stop Editing Opacity Mask
/// does: the mask keeps the edit, the editing layer is removed, nothing is isolated, the masked
/// object is selected on its layer, and one undo step returns to mask editing.
#[test]
fn exit_isolation_while_editing_leaves_mask_editing() {
    let mut s = session();
    let obj = editing_mask(&mut s);
    s.execute("object.move", &json!({"dx": 50, "dy": 0})).unwrap();
    s.execute("object.exitIsolation", &json!({})).unwrap();
    let st = s.doc().unwrap();
    assert!(st.doc.mask_edit.is_none() && st.isolation.is_none() && !has_edit_layer(&st.doc));
    assert_eq!(st.selection.objects, vec![obj]);
    assert_eq!(st.active_layer, Some(st.doc.layers[0].id));
    assert_eq!(st.history.undo.last().unwrap().label, "Stop Editing Opacity Mask");
    assert_eq!(mask(&s, obj).unwrap().art.geometric_bounds().unwrap().x0, 50.0);
    s.execute("edit.undo", &json!({})).unwrap();
    let st = s.doc().unwrap();
    let layer = st.doc.mask_edit.map(|m| m.layer);
    assert!(layer.is_some() && st.isolation == layer, "back in mask editing");
}

/// Point type that a Type tool click placed in the mask and left empty is discarded when Stop
/// Editing Opacity Mask or Exit Isolation Mode leaves mask editing, so the mask does not keep it.
#[test]
fn empty_point_type_is_not_saved_into_the_mask() {
    for cmd in ["transparency.stopEditingOpacityMask", "object.exitIsolation"] {
        let mut s = session();
        s.prefs.placeholder_text = false;
        let obj = editing_mask(&mut s);
        let v = ViewInfo::default();
        s.select_tool("type", v).unwrap();
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, 250.0, 250.0), v).unwrap();
        }
        assert!(s.tool_wants_text(), "{cmd}: the click places point type to edit");
        s.execute(cmd, &json!({})).unwrap();
        assert!(!s.tool_wants_text(), "{cmd}");
        let art = &mask(&s, obj).unwrap().art;
        assert!(art.children().is_none() && !matches!(art.kind, vectorcraft_doc::NodeKind::Text(_)), "{cmd}: the mask keeps only the square");
        assert_eq!(art.geometric_bounds().unwrap().x1, 100.0, "{cmd}");
        let labels = s.doc().unwrap().history.undo.iter().map(|e| e.label.as_str()).collect::<Vec<_>>();
        // The mask took the type in when it was placed, so its removal is a step of its own.
        assert_eq!(labels[labels.len() - 3..], ["Type", "Discard Empty Type", "Stop Editing Opacity Mask"], "{cmd}");
    }
}

/// Releasing the masks of a group and of an object inside it puts each mask's art directly above
/// its object, as releasing one mask at a time does.
#[test]
fn release_of_a_masked_group_and_a_masked_object_inside_it_keeps_order_and_ids() {
    let mut s = session();
    let (a, m1) = (rect(&mut s, 0.0, 100.0), rect(&mut s, 0.0, 50.0));
    s.execute("transparency.makeOpacityMask", &json!({"ids": [a.0, m1.0]})).unwrap();
    let b = rect(&mut s, 150.0, 100.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    let m2 = rect(&mut s, 0.0, 300.0);
    s.execute("transparency.makeOpacityMask", &json!({"ids": [group.0, m2.0]})).unwrap();
    let before = doc(&s).clone();
    let n = before.peek_next_id();
    s.execute("transparency.releaseOpacityMask", &json!({"ids": [group.0, a.0]})).unwrap();
    let ids = |n: &vectorcraft_doc::Node| n.children().unwrap().iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(ids(&doc(&s).layers[0]), [group, NodeId(n)]);
    assert_eq!(ids(doc(&s).node(group).unwrap()), [a, NodeId(n + 1), b]);
    assert!(mask(&s, group).is_none() && mask(&s, a).is_none());
    assert_eq!(s.doc().unwrap().selection.objects, [group, NodeId(n), a, NodeId(n + 1)]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(*doc(&s) == before, "Undo gives back the document");
}

/// Opacity mask editing is not entered in pattern editing mode: Edit Opacity Mask and View Opacity
/// Mask are off for a masked object in the tile, and so is Make Opacity Mask for one object, which
/// would edit its new empty mask; a single target given in `ids` is refused. Make Opacity Mask with
/// two objects still masks one with the other. Pattern editing goes on, its layer still isolated.
/// After Done the mask is edited again.
#[test]
fn opacity_mask_editing_is_not_entered_in_pattern_editing_mode() {
    let mut s = session();
    let (obj, m) = (rect(&mut s, 0.0, 200.0), rect(&mut s, 0.0, 100.0));
    s.execute("transparency.makeOpacityMask", &json!({"ids": [obj.0, m.0]})).unwrap();
    s.execute("select.set", &json!({"ids": [obj.0]})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    let tile = doc(&s).pattern_edit.as_ref().unwrap().layer;
    let copy = doc(&s).node(tile).unwrap().children().unwrap()[0].id;
    assert!(mask(&s, copy).is_some(), "the tile's copy has the mask");
    let why = "finish editing the pattern first";
    let off = |s: &mut Session, cmd: &str, p: Value| {
        let err = s.execute(cmd, &p).unwrap_err();
        assert!(matches!(&err, EngineError::Disabled(id, w) if id == cmd && w == why), "{cmd} {p}: {err}");
    };
    off(&mut s, "transparency.editOpacityMask", json!({}));
    off(&mut s, "transparency.editOpacityMask", json!({"id": copy.0}));
    off(&mut s, "transparency.viewOpacityMask", json!({}));
    off(&mut s, "transparency.viewOpacityMask", json!({"on": true, "id": copy.0}));
    let one = rect(&mut s, 220.0, 30.0);
    s.execute("select.set", &json!({"ids": [one.0]})).unwrap();
    off(&mut s, "transparency.makeOpacityMask", json!({}));
    off(&mut s, "transparency.makeOpacityMask", json!({"ids": [one.0]}));
    // With two objects selected the command is on, and one target given in `ids` is refused.
    s.execute("select.set", &json!({"ids": [copy.0, one.0]})).unwrap();
    for target in [one, tile] {
        let err = s.execute("transparency.makeOpacityMask", &json!({"ids": [target.0]})).unwrap_err();
        assert!(err.to_string().ends_with(why), "{err}");
    }
    let st = s.doc().unwrap();
    assert!(st.doc.mask_edit.is_none() && st.mask_view.is_none() && mask(&s, one).is_none());
    assert_eq!((st.isolation, st.active_layer), (Some(tile), Some(tile)));
    // Two objects: the top one masks the other, without mask editing.
    let top = rect(&mut s, 220.0, 10.0);
    s.execute("select.set", &json!({"ids": [one.0, top.0]})).unwrap();
    let r = s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    assert_eq!(r, json!({"id": one.0, "editing": false}));
    assert_eq!(s.doc().unwrap().isolation, Some(tile));
    assert!(doc(&s).pattern_edit.is_some());
    s.execute("object.pattern.done", &json!({})).unwrap();
    s.execute("transparency.editOpacityMask", &json!({"id": obj.0})).unwrap();
    assert_eq!(doc(&s).mask_edit.map(|e| e.object), Some(obj));
}

/// In pattern editing mode a targeted sublayer holding two objects is one target, so Make Opacity
/// Mask is off for it.
#[test]
fn make_opacity_mask_is_off_for_a_targeted_sublayer_in_pattern_editing_mode() {
    let mut s = session();
    let obj = rect(&mut s, 0.0, 200.0);
    s.execute("select.set", &json!({"ids": [obj.0]})).unwrap();
    s.execute("object.pattern.make", &json!({"name": "P"})).unwrap();
    let tile = doc(&s).pattern_edit.as_ref().unwrap().layer;
    let sub = s.execute("layer.newSublayer", &json!({"parent": tile.0})).unwrap()["id"].as_u64().unwrap();
    let (a, b) = (rect(&mut s, 220.0, 30.0), rect(&mut s, 260.0, 30.0));
    s.execute("layer.move", &json!({"ids": [a.0, b.0], "target": sub})).unwrap();
    let kids = doc(&s).node(NodeId(sub)).unwrap().children().unwrap().iter().map(|n| n.id).collect::<Vec<_>>();
    assert_eq!(kids, vec![a, b], "the new objects are drawn in the sublayer");
    s.execute("layer.target", &json!({"id": sub})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects.len(), 2, "the sublayer's two objects are selected");
    let err = s.execute("transparency.makeOpacityMask", &json!({})).unwrap_err();
    let why = "finish editing the pattern first";
    assert!(matches!(&err, EngineError::Disabled(id, w) if id == "transparency.makeOpacityMask" && w == why), "{err}");
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

/// The command `cmd` with `p` is off, for `why`.
fn off(s: &mut Session, cmd: &str, p: Value, why: &str) {
    let err = s.execute(cmd, &p).unwrap_err();
    assert!(matches!(&err, EngineError::Disabled(id, w) if id == cmd && w == why), "{cmd} {p}: {err}");
}

/// Isolation mode is not entered while an opacity mask is edited: Enter Isolation Mode is off for
/// a group in the mask art and for one outside it, and mask editing goes on. After Stop Editing
/// Opacity Mask it isolates again.
#[test]
fn isolation_mode_is_not_entered_while_an_opacity_mask_is_edited() {
    let mut s = session();
    let group = |s: &mut Session, x: f64| {
        let (a, b) = (rect(s, x, 20.0), rect(s, x + 30.0, 20.0));
        s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
        NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap())
    };
    let outside = group(&mut s, 220.0);
    editing_mask(&mut s);
    let layer = s.doc().unwrap().isolation;
    let inside = group(&mut s, 0.0);
    for id in [inside, outside] {
        off(&mut s, "object.isolate", json!({"id": id.0}), "stop editing the opacity mask first");
        assert_eq!(s.doc().unwrap().isolation, layer);
        assert!(doc(&s).mask_edit.is_some(), "still editing");
    }
    s.execute("select.set", &json!({"ids": [inside.0]})).unwrap();
    off(&mut s, "object.isolate", json!({}), "stop editing the opacity mask first");
    s.execute("transparency.stopEditingOpacityMask", &json!({})).unwrap();
    s.execute("object.isolate", &json!({"id": outside.0})).unwrap();
    assert_eq!(s.doc().unwrap().isolation, Some(outside));
}

/// Opacity mask editing is not entered in isolation mode: Edit Opacity Mask and View Opacity Mask
/// are off, and so is Make Opacity Mask for one object, which would edit its new empty mask. Make
/// Opacity Mask with two objects still masks one with the other. After Exit Isolation Mode the mask
/// is edited again.
#[test]
fn opacity_mask_editing_is_not_entered_in_isolation_mode() {
    let mut s = session();
    let (obj, m) = (rect(&mut s, 0.0, 200.0), rect(&mut s, 0.0, 100.0));
    s.execute("transparency.makeOpacityMask", &json!({"ids": [obj.0, m.0]})).unwrap();
    let other = rect(&mut s, 220.0, 30.0);
    s.execute("select.set", &json!({"ids": [obj.0, other.0]})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    s.execute("select.set", &json!({"ids": [obj.0]})).unwrap();
    let why = "exit isolation mode first";
    off(&mut s, "transparency.editOpacityMask", json!({}), why);
    off(&mut s, "transparency.editOpacityMask", json!({"id": obj.0}), why);
    off(&mut s, "transparency.viewOpacityMask", json!({}), why);
    off(&mut s, "transparency.viewOpacityMask", json!({"on": true, "id": obj.0}), why);
    s.execute("select.set", &json!({"ids": [other.0]})).unwrap();
    off(&mut s, "transparency.makeOpacityMask", json!({}), why);
    off(&mut s, "transparency.makeOpacityMask", json!({"ids": [other.0]}), why);
    // A layer given as the one target is refused too.
    let layer = doc(&s).layers[0].id;
    let err = s.execute("transparency.makeOpacityMask", &json!({"ids": [layer.0]})).unwrap_err();
    assert!(err.to_string().ends_with(why), "{err}");
    let st = s.doc().unwrap();
    assert!(st.doc.mask_edit.is_none() && st.mask_view.is_none() && mask(&s, other).is_none());
    assert_eq!(s.doc().unwrap().isolation, Some(g));
    // Two objects: the top one masks the other, without mask editing.
    let top = rect(&mut s, 220.0, 10.0);
    let r = s.execute("transparency.makeOpacityMask", &json!({"ids": [other.0, top.0]})).unwrap();
    assert_eq!(r, json!({"id": other.0, "editing": false}));
    assert_eq!(s.doc().unwrap().isolation, Some(g));
    // Two members of a compound path given in `ids` are two objects: one masks the other.
    let (c1, c2) = (rect(&mut s, 250.0, 10.0), rect(&mut s, 265.0, 10.0));
    s.execute("select.set", &json!({"ids": [c1.0, c2.0]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    let r = s.execute("transparency.makeOpacityMask", &json!({"ids": [c1.0, c2.0]})).unwrap();
    assert_eq!(r["editing"], json!(false));
    assert!(doc(&s).mask_edit.is_none());
    s.execute("object.exitIsolation", &json!({})).unwrap();
    s.execute("transparency.editOpacityMask", &json!({"id": obj.0})).unwrap();
    assert_eq!(doc(&s).mask_edit.map(|e| e.object), Some(obj));
}

/// In isolation mode, Make Opacity Mask is off for a targeted sublayer that holds two objects:
/// the sublayer is its one target, which would get an empty mask and enter mask editing.
#[test]
fn make_opacity_mask_is_off_for_a_targeted_sublayer_in_isolation_mode() {
    let mut s = session();
    let layer = doc(&s).layers[0].id;
    let sub = NodeId(s.execute("layer.newSublayer", &json!({"parent": layer.0})).unwrap()["id"].as_u64().unwrap());
    s.execute("layer.setCurrent", &json!({"id": sub.0})).unwrap();
    let (a, b) = (rect(&mut s, 0.0, 20.0), rect(&mut s, 40.0, 20.0));
    s.execute("object.isolate", &json!({"id": layer.0})).unwrap();
    s.execute("layer.target", &json!({"id": sub.0})).unwrap();
    assert_eq!(s.doc().unwrap().selection.target, Some(sub));
    off(&mut s, "transparency.makeOpacityMask", json!({}), "exit isolation mode first");
    // Two objects given in `ids` still mask one with the other.
    let r = s.execute("transparency.makeOpacityMask", &json!({"ids": [a.0, b.0]})).unwrap();
    assert_eq!(r["editing"], json!(false));
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
    let bytes = data(&s.execute("document.save", &json!({})).unwrap());
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert!(back.mask_edit.is_none() && !has_edit_layer(&back));
    assert_eq!(back.layers.len(), 1);
    // The edit made while editing is in the saved mask.
    assert_eq!(back.node(obj).unwrap().extras().mask.as_ref().unwrap().art.geometric_bounds().unwrap().x0, 50.0);
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
