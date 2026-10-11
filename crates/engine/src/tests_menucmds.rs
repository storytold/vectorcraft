//! Menu long-tail commands (Object/Edit/Select/Type/View/File), driven through `Session::execute`.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{ColorMode, LiveShape, Node, NodeKind, TextKind};
use vectorcraft_geom::Rect;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn text(s: &mut Session, t: &str) -> NodeId {
    id_of(&s.execute("text.create", &json!({"x": 50, "y": 50, "text": t})).unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn bounds(s: &Session, id: NodeId) -> Rect {
    node(s, id).geometric_bounds().unwrap()
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

fn fill(s: &mut Session, id: NodeId, hex: &str) {
    s.execute("paint.setFill", &json!({"ids": [id.0], "color": hex})).unwrap();
}

fn fill_of(s: &Session, id: NodeId) -> Color {
    node(s, id).appearance.fill_paint().color().unwrap()
}

fn plain(s: &Session, id: NodeId) -> String {
    match &node(s, id).kind {
        NodeKind::Text(t) => t.plain_text(),
        _ => panic!("not text"),
    }
}

fn undo_len(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

fn selected(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().selection.objects.clone()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

// ---------- Object → Lock / Hide ----------

#[test]
fn lock_all_artwork_above_only_overlapping_objects_above() {
    let mut s = session();
    let below = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let a = rect(&mut s, 10.0, 10.0, 50.0, 50.0);
    let over = rect(&mut s, 30.0, 30.0, 50.0, 50.0);
    let far = rect(&mut s, 400.0, 400.0, 20.0, 20.0);
    sel(&mut s, &[a]);
    let n = undo_len(&s);
    let r = s.execute("object.lock.above", &json!({})).unwrap();
    assert_eq!(r["count"], 1);
    assert!(node(&s, over).locked);
    assert!(!node(&s, below).locked && !node(&s, far).locked && !node(&s, a).locked);
    assert_eq!(undo_len(&s), n + 1);
}

#[test]
fn hide_all_artwork_above() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 50.0, 50.0);
    let over = rect(&mut s, 30.0, 30.0, 50.0, 50.0);
    sel(&mut s, &[a]);
    s.execute("object.hide.above", &json!({})).unwrap();
    assert!(!node(&s, over).visible);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(node(&s, over).visible);
}

fn key(s: &Session) -> Option<NodeId> {
    s.doc().unwrap().selection.key
}

/// Lock › All Artwork Above deselects the selected objects it locks: here the key object, in a
/// group above the other selected object. Undo selects it again as the key object.
#[test]
fn lock_all_artwork_above_deselects_the_objects_it_locks() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 50.0, 50.0, 100.0, 100.0);
    sel(&mut s, &[b]);
    let g = id_of(&s.execute("object.group", &json!({})).unwrap());
    sel(&mut s, &[a, b]);
    s.execute("select.key", &json!({"id": b.0})).unwrap();
    s.execute("object.lock.above", &json!({})).unwrap();
    assert!(node(&s, g).locked);
    assert_eq!((selected(&s), key(&s)), (vec![a], None));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!node(&s, g).locked);
    assert_eq!((selected(&s), key(&s)), (vec![a, b], Some(b)));
}

/// Hide › All Artwork Above deselects the selected objects it hides (here one in a sublayer above)
/// and selected objects that were hidden before; the key object stays when it is still shown.
#[test]
fn hide_all_artwork_above_deselects_the_objects_it_hides() {
    let mut s = session();
    let hidden = rect(&mut s, 400.0, 400.0, 20.0, 20.0);
    s.execute("layer.setProps", &json!({"id": hidden.0, "visible": false})).unwrap();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let sub = id_of(&s.execute("layer.newSublayer", &json!({})).unwrap());
    let b = rect(&mut s, 50.0, 50.0, 100.0, 100.0);
    sel(&mut s, &[hidden, a, b]);
    s.execute("select.key", &json!({"id": a.0})).unwrap();
    s.execute("object.hide.above", &json!({})).unwrap();
    assert!(!node(&s, sub).visible);
    assert_eq!((selected(&s), key(&s)), (vec![a], Some(a)));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((selected(&s), key(&s)), (vec![hidden, a, b], Some(a)));
}

#[test]
fn lock_and_hide_other_layers() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let l2 = id_of(&s.execute("layer.new", &json!({})).unwrap_or(json!({"id": 0})));
    let l1 = s.doc().unwrap().doc.layers[0].id;
    assert_ne!(l1, l2);
    sel(&mut s, &[a]);
    s.execute("object.lock.otherLayers", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(d.node(l2).unwrap().locked);
    assert!(!d.node(l1).unwrap().locked);
    s.execute("object.hide.otherLayers", &json!({})).unwrap();
    assert!(!s.doc().unwrap().doc.node(l2).unwrap().visible);
}

// ---------- Transform Each ----------

#[test]
fn transform_each_scales_each_about_its_own_center() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let b = rect(&mut s, 200.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a, b]);
    let n = undo_len(&s);
    s.execute("object.transformEach", &json!({"scaleH": 50, "scaleV": 50})).unwrap();
    assert_eq!(bounds(&s, a), Rect::new(25.0, 25.0, 75.0, 75.0));
    assert_eq!(bounds(&s, b), Rect::new(225.0, 25.0, 275.0, 75.0));
    assert_eq!(undo_len(&s), n + 1);
}

#[test]
fn transform_each_move_reference_and_copy() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    // Top-left reference: scaling keeps the top-left corner.
    let r = s.execute("object.transformEach", &json!({"scaleH": 50, "scaleV": 50, "moveH": 10, "moveV": 5, "reference": 0, "copy": true})).unwrap();
    let copy = NodeId(r["ids"][0].as_u64().unwrap());
    assert_ne!(copy, a);
    assert_eq!(bounds(&s, a), Rect::new(0.0, 0.0, 100.0, 100.0));
    assert_eq!(bounds(&s, copy), Rect::new(10.0, 5.0, 60.0, 55.0));
}

#[test]
fn transform_each_random_is_deterministic_with_seed() {
    let run = || {
        let mut s = session();
        let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
        sel(&mut s, &[a]);
        s.execute("object.transformEach", &json!({"moveH": 50, "random": true, "seed": 7})).unwrap();
        bounds(&s, a)
    };
    let (x, y) = (run(), run());
    assert_eq!(x, y);
    assert!(x.x0.abs() <= 50.0 && x.width() == 100.0);
}

#[test]
fn reset_bounding_box_is_a_noop() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    let n = undo_len(&s);
    assert_eq!(s.execute("object.resetBoundingBox", &json!({})).unwrap()["changed"], 0);
    assert_eq!(undo_len(&s), n);
}

// ---------- Expand / Rasterize / Crop / Trim marks ----------

#[test]
fn expand_live_shape_and_text_in_one_undo_step() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let t = text(&mut s, "Hi");
    sel(&mut s, &[a, t]);
    let n = undo_len(&s);
    // Stroke off: the rectangle stays one path (its stroke isn't outlined).
    s.execute("object.expand", &json!({"stroke": false})).unwrap();
    assert!(matches!(node(&s, a).kind, NodeKind::Path { live: None, .. }));
    assert!(s.doc().unwrap().doc.node(t).is_none(), "text became outlines");
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Expand");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(node(&s, a).kind, NodeKind::Path { live: Some(_), .. }));
    assert!(s.doc().unwrap().doc.node(t).is_some());
}

#[test]
fn expand_nothing_errors() {
    let mut s = session();
    let p = id_of(&s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 10, "y": 10}]})).unwrap());
    sel(&mut s, &[p]);
    // A plain path: nothing to expand for object/fill.
    let r = s.execute("object.expand", &json!({"stroke": false}));
    assert!(r.is_err());
}

#[test]
fn rasterize_replaces_selection_with_image() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 40.0, 30.0);
    fill(&mut s, a, "#ff0000");
    sel(&mut s, &[a]);
    let vb = s.doc().unwrap().doc.bounds_of(&[a], true).unwrap();
    let r = s.execute("object.rasterize", &json!({"ppi": 144, "padding": 0})).unwrap();
    let img = id_of(&r);
    assert!(s.doc().unwrap().doc.node(a).is_none());
    let n = node(&s, img);
    let NodeKind::Image(im) = &n.kind else { panic!() };
    // Visual bounds (stroke included) at 2 px/pt.
    assert_eq!((im.width as f64, im.height as f64), ((vb.width() * 2.0).ceil(), (vb.height() * 2.0).ceil()));
    assert!(s.doc().unwrap().doc.images.contains_key(&im.key));
    let b = n.geometric_bounds().unwrap();
    assert!(close(b.x0, vb.x0) && close(b.y0, vb.y0) && b.width() >= vb.width());
    assert_eq!(selected(&s), vec![img]);
}

#[test]
fn crop_image_to_rect() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    sel(&mut s, &[a]);
    let img = id_of(&s.execute("object.rasterize", &json!({"ppi": 72})).unwrap());
    let r = s.execute("object.cropImage", &json!({"rect": [10, 10, 50, 40]})).unwrap();
    assert_eq!((r["width"].as_u64().unwrap(), r["height"].as_u64().unwrap()), (50, 40));
    let b = bounds(&s, img);
    assert!(close(b.x0, 10.0) && close(b.y0, 10.0) && close(b.width(), 50.0) && close(b.height(), 40.0), "{b:?}");
    // Image fully inside the artboard: default crop has nothing to do.
    assert!(s.execute("object.cropImage", &json!({})).is_err());
}

#[test]
fn crop_image_disabled_without_image() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    assert!(matches!(s.execute("object.cropImage", &json!({})), Err(EngineError::Disabled(..))));
}

#[test]
fn mask_image_clips_it_to_its_outline_and_selects_the_clipping_path() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 20.0, 100.0, 50.0);
    sel(&mut s, &[a]);
    let img = id_of(&s.execute("object.rasterize", &json!({"ppi": 72})).unwrap());
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    let ib = bounds(&s, img);
    let r = s.execute("object.maskImage", &json!({})).unwrap();
    let (g, p) = (id_of(&r), NodeId(r["path"].as_u64().unwrap()));
    let group = node(&s, g);
    assert!(matches!(group.kind, NodeKind::Group { clip: true, .. }));
    assert_eq!(group.children().unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), [p, img]);
    // The clipping path follows the rotated image's outline, so nothing is cut off yet.
    assert!(matches!(node(&s, p).kind, NodeKind::Path { clipping: true, .. }));
    let pb = bounds(&s, p);
    assert!(close(pb.x0, ib.x0) && close(pb.y0, ib.y0) && close(pb.x1, ib.x1) && close(pb.y1, ib.y1), "{pb:?} {ib:?}");
    assert_eq!(s.doc().unwrap().selection.objects, [p]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.doc().unwrap().doc.node(g).is_none() && s.doc().unwrap().doc.node(img).is_some());
    sel(&mut s, &[]);
    assert!(matches!(s.execute("object.maskImage", &json!({})), Err(EngineError::Disabled(..))));
}

#[test]
fn trim_marks_around_selection() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    fill(&mut s, a, "#00ff00");
    s.execute("stroke.set", &json!({"ids": [a.0], "weight": 0})).unwrap();
    sel(&mut s, &[a]);
    let g = id_of(&s.execute("object.createTrimMarks", &json!({"offset": 10, "length": 20})).unwrap());
    let n = node(&s, g);
    assert_eq!(n.children().unwrap().len(), 8);
    assert_eq!(n.name.as_deref(), Some("Trim Marks"));
    let b = n.geometric_bounds().unwrap();
    let vb = s.doc().unwrap().doc.bounds_of(&[a], true).unwrap();
    assert!(close(b.x0, vb.x0 - 30.0) && close(b.y1, vb.y1 + 30.0), "{b:?} {vb:?}");
}

// ---------- Convert to Shape ----------

#[test]
fn convert_rectangle_path_to_live_shape() {
    let mut s = session();
    let p = id_of(
        &s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 40, "y": 0}, {"x": 40, "y": 20}, {"x": 0, "y": 20}], "closed": true}))
            .unwrap(),
    );
    sel(&mut s, &[p]);
    assert_eq!(s.execute("object.shape.convertToShape", &json!({})).unwrap()["converted"], 1);
    let NodeKind::Path { live: Some(LiveShape::Rectangle { w, h, .. }), path, .. } = &node(&s, p).kind else { panic!() };
    assert!(close(*w, 40.0) && close(*h, 20.0));
    assert_eq!(path.bounds(), Some(Rect::new(0.0, 0.0, 40.0, 20.0)));
}

#[test]
fn convert_rotated_rectangle_and_ellipse() {
    let mut s = session();
    let p = id_of(
        &s.execute(
            "path.create",
            &json!({"anchors": [{"x": 50, "y": 0}, {"x": 100, "y": 50}, {"x": 50, "y": 100}, {"x": 0, "y": 50}], "closed": true}),
        )
        .unwrap(),
    );
    let e = id_of(&s.execute("shape.ellipse", &json!({"x": 0, "y": 200, "width": 80, "height": 40})).unwrap());
    sel(&mut s, &[e]);
    s.execute("object.expandShape", &json!({})).unwrap();
    sel(&mut s, &[p, e]);
    assert_eq!(s.execute("object.shape.convertToShape", &json!({})).unwrap()["converted"], 2);
    let b = bounds(&s, p);
    assert!(close(b.x0, 0.0) && close(b.x1, 100.0) && close(b.y1, 100.0), "{b:?}");
    assert!(matches!(node(&s, e).kind, NodeKind::Path { live: Some(LiveShape::Ellipse { .. }), .. }));
}

#[test]
fn convert_to_shape_rejects_other_paths() {
    let mut s = session();
    let p = id_of(&s.execute("path.create", &json!({"anchors": [{"x": 0, "y": 0}, {"x": 40, "y": 0}, {"x": 10, "y": 20}], "closed": true})).unwrap());
    sel(&mut s, &[p]);
    assert!(s.execute("object.shape.convertToShape", &json!({})).is_err());
}

// ---------- Blend ----------

#[test]
fn blend_make_release_options() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 0.0, 30.0, 30.0);
    fill(&mut s, a, "#000000");
    fill(&mut s, b, "#ffffff");
    sel(&mut s, &[a, b]);
    let n = undo_len(&s);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 3})).unwrap());
    assert_eq!(undo_len(&s), n + 1);
    let gn = node(&s, g);
    assert!(matches!(gn.kind, NodeKind::Blend { .. }), "live blend");
    let ch = vectorcraft_doc::live::expand_live(&gn);
    assert_eq!(ch.len(), 5);
    // Middle step: halfway in position, size and colour.
    let mid = &ch[2];
    let mb = mid.geometric_bounds().unwrap();
    assert!(close(mb.x0, 50.0) && close(mb.width(), 20.0), "{mb:?}");
    let c = mid.appearance.fill_paint().color().unwrap().to_rgb();
    assert!((c[0] - 0.5).abs() < 1e-3);
    // Options: 1 step.
    s.execute("object.blend.options", &json!({"steps": 1})).unwrap();
    assert_eq!(vectorcraft_doc::live::expand_live(&node(&s, g)).len(), 3);
    // Release: keys only, back in the layer.
    let r = s.execute("object.blend.release", &json!({})).unwrap();
    assert_eq!(r["ids"].as_array().unwrap().len(), 2);
    assert!(s.doc().unwrap().doc.node(g).is_none());
    assert!(s.doc().unwrap().doc.node(a).is_some() && s.doc().unwrap().doc.node(b).is_some());
}

#[test]
fn blend_expand_and_reverse() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 100.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b]);
    let g = id_of(&s.execute("object.blend.make", &json!({"steps": 2})).unwrap());
    s.execute("object.blend.reverseFrontToBack", &json!({})).unwrap();
    assert_eq!(node(&s, g).children().unwrap().last().unwrap().id, a);
    s.execute("object.blend.reverseSpine", &json!({})).unwrap();
    assert!(close(bounds(&s, a).x0, 100.0));
    s.execute("object.blend.expand", &json!({})).unwrap();
    assert!(matches!(node(&s, g).kind, NodeKind::Group { .. }));
    assert!(s.execute("object.blend.release", &json!({})).is_err(), "no longer a blend");
}

// ---------- Artboards ----------

#[test]
fn convert_to_artboards_and_rearrange() {
    let mut s = session();
    let a = rect(&mut s, 900.0, 0.0, 200.0, 100.0);
    sel(&mut s, &[a]);
    assert_eq!(s.execute("artboard.convertToArtboards", &json!({})).unwrap()["artboards"], 1);
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.artboards.len(), 2);
    assert_eq!(d.artboards[1].rect, Rect::new(900.0, 0.0, 1100.0, 100.0));
    assert!(d.node(a).is_none());
    // Art on artboard 2 moves with it.
    let b = rect(&mut s, 950.0, 25.0, 10.0, 10.0);
    s.execute("artboard.rearrange", &json!({"columns": 1, "spacing": 50})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.artboards[1].rect, Rect::new(0.0, 650.0, 200.0, 750.0));
    assert_eq!(bounds(&s, b), Rect::new(50.0, 675.0, 60.0, 685.0));
}

// ---------- Edit Colors ----------

#[test]
fn invert_and_convert_colors() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    fill(&mut s, a, "#ff0000");
    sel(&mut s, &[a]);
    s.execute("edit.colors.invert", &json!({"stroke": false})).unwrap();
    assert_eq!(fill_of(&s, a).to_hex(), "#00ffff");
    s.execute("edit.colors.toCMYK", &json!({})).unwrap();
    assert!(matches!(fill_of(&s, a), Color::Cmyk { .. }));
    s.execute("edit.colors.toGrayscale", &json!({})).unwrap();
    assert!(matches!(fill_of(&s, a), Color::Gray { .. }));
    s.execute("edit.colors.toRGB", &json!({})).unwrap();
    assert!(matches!(fill_of(&s, a), Color::Rgb { .. }));
}

#[test]
fn saturate_and_adjust_balance() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [a.0], "color": [0.8, 0.4, 0.4]})).unwrap();
    sel(&mut s, &[a]);
    s.execute("edit.colors.saturate", &json!({"intensity": -100})).unwrap();
    let [_, sat, _] = fill_of(&s, a).to_hsb();
    assert!(sat < 1e-4);
    s.execute("edit.colors.adjustBalance", &json!({"r": 10, "g": -10})).unwrap();
    let [r, g, _] = fill_of(&s, a).to_rgb();
    assert!((r - 0.9).abs() < 1e-3 && (g - 0.7).abs() < 1e-3, "{r} {g}");
    assert!(s.execute("edit.colors.adjustBalance", &json!({})).is_err());
}

#[test]
fn blend_colors_horizontally() {
    let mut s = session();
    let ids: Vec<NodeId> = (0..3).map(|i| rect(&mut s, 200.0 - i as f64 * 100.0, 0.0, 10.0, 10.0)).collect();
    fill(&mut s, ids[2], "#000000"); // leftmost
    fill(&mut s, ids[0], "#ffffff"); // rightmost
    fill(&mut s, ids[1], "#ff0000");
    sel(&mut s, &ids);
    s.execute("edit.colors.blendHorizontally", &json!({})).unwrap();
    assert_eq!(fill_of(&s, ids[1]).to_hex(), "#808080");
    sel(&mut s, &ids[..2]);
    assert!(s.execute("edit.colors.blendVertically", &json!({})).is_err(), "needs three");
}

#[test]
fn blend_front_to_back_uses_stacking() {
    let mut s = session();
    let ids: Vec<NodeId> = (0..3).map(|_| rect(&mut s, 0.0, 0.0, 10.0, 10.0)).collect();
    fill(&mut s, ids[0], "#000000");
    fill(&mut s, ids[2], "#ffffff");
    sel(&mut s, &ids);
    s.execute("edit.colors.blendFrontToBack", &json!({})).unwrap();
    assert_eq!(fill_of(&s, ids[1]).to_hex(), "#808080");
}

// ---------- Paste without formatting / Find ----------

#[test]
fn paste_without_formatting_resets_text_style() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 10, "y": 10, "text": "Bold", "size": 40})).unwrap());
    sel(&mut s, &[t]);
    s.execute("edit.copy", &json!({})).unwrap();
    let n = undo_len(&s);
    let r = s.execute("edit.pasteWithoutFormatting", &json!({})).unwrap();
    let p = NodeId(r["ids"][0].as_u64().unwrap());
    let NodeKind::Text(tt) = &node(&s, p).kind else { panic!() };
    assert_eq!(tt.plain_text(), "Bold");
    assert_eq!(tt.first_style().size, 12.0);
    assert_eq!(undo_len(&s), n + 1);
    // Clipboard keeps its formatting.
    let NodeKind::Text(ct) = &s.clipboard.nodes[0].kind else { panic!() };
    assert_eq!(ct.first_style().size, 40.0);
}

#[test]
fn find_and_replace_options() {
    let mut s = session();
    let a = text(&mut s, "Cat cat catalog");
    let b = text(&mut s, "the CAT");
    let r = s.execute("edit.findReplace", &json!({"find": "cat", "replace": "dog", "wholeWord": true})).unwrap();
    assert_eq!(r["count"], 3);
    assert_eq!(plain(&s, a), "dog dog catalog");
    assert_eq!(plain(&s, b), "the dog");
    let r = s.execute("edit.findReplace", &json!({"find": "Dog", "replace": "x", "matchCase": true})).unwrap();
    assert_eq!(r["count"], 0);
    assert!(s.execute("edit.findReplace", &json!({"find": ""})).is_err());
}

#[test]
fn find_next_cycles_through_text_objects() {
    let mut s = session();
    let a = text(&mut s, "apple");
    let _ = text(&mut s, "banana");
    let c = text(&mut s, "pineapple");
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(id_of(&s.execute("edit.findNext", &json!({"find": "APPLE"})).unwrap()), a);
    assert_eq!(id_of(&s.execute("edit.findNext", &json!({"find": "apple"})).unwrap()), c);
    assert_eq!(id_of(&s.execute("edit.findNext", &json!({"find": "apple"})).unwrap()), a);
    assert!(s.execute("edit.findNext", &json!({"find": "zzz"})).unwrap()["id"].is_null());
}

// ---------- Select ----------

#[test]
fn select_same_font_size_and_fill() {
    let mut s = session();
    let a = id_of(&s.execute("text.create", &json!({"x": 0, "y": 20, "text": "a", "size": 20})).unwrap());
    let b = id_of(&s.execute("text.create", &json!({"x": 0, "y": 60, "text": "b", "size": 20})).unwrap());
    let c = id_of(&s.execute("text.create", &json!({"x": 0, "y": 90, "text": "c", "size": 30, "font": "Inter"})).unwrap());
    sel(&mut s, &[a]);
    assert_eq!(s.execute("select.same.fontSize", &json!({})).unwrap()["count"], 2);
    assert_eq!(selected(&s), vec![a, b]);
    sel(&mut s, &[c]);
    assert_eq!(s.execute("select.same.fontFamily", &json!({})).unwrap()["count"], 1);
    sel(&mut s, &[a]);
    assert_eq!(s.execute("select.same.textFillColor", &json!({})).unwrap()["count"], 3);
    assert_eq!(s.execute("select.same.fontFamilyStyle", &json!({})).unwrap()["count"], 2);
    let r = rect(&mut s, 0.0, 0.0, 5.0, 5.0);
    sel(&mut s, &[r]);
    assert!(s.execute("select.same.fontSize", &json!({})).is_err());
}

#[test]
fn select_direction_handles_selects_all_anchors() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    assert_eq!(s.execute("select.object.directionHandles", &json!({})).unwrap()["anchors"], 4);
    assert_eq!(s.doc().unwrap().selection.anchors.get(&a).unwrap().len(), 4);
}

#[test]
fn select_point_and_area_text() {
    let mut s = session();
    let p = text(&mut s, "point");
    let a = id_of(&s.execute("text.create", &json!({"x": 0, "y": 0, "text": "area", "area": {"width": 100, "height": 50}})).unwrap());
    let _ = rect(&mut s, 0.0, 0.0, 5.0, 5.0);
    assert_eq!(s.execute("select.object.pointText", &json!({})).unwrap()["count"], 1);
    assert_eq!(selected(&s), vec![p]);
    s.execute("select.object.areaText", &json!({})).unwrap();
    assert_eq!(selected(&s), vec![a]);
    assert_eq!(s.execute("select.object.brushStrokes", &json!({})).unwrap()["count"], 0);
}

#[test]
fn save_and_recall_selection() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a, b]);
    assert_eq!(s.execute("select.save", &json!({})).unwrap()["name"], "Selection 1");
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(s.execute("select.recall", &json!({"name": "Selection 1"})).unwrap()["count"], 2);
    s.execute("select.editSaved", &json!({"name": "Selection 1", "newName": "Pair"})).unwrap();
    assert_eq!(s.execute("select.savedList", &json!({})).unwrap(), json!(["Pair"]));
    s.execute("select.editSaved", &json!({"name": "Pair", "delete": true})).unwrap();
    assert!(s.execute("select.recall", &json!({"name": "Pair"})).is_err());
}

#[test]
fn edit_selection_needs_a_saved_selection_and_a_free_name() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    // Edit Selection… is off until something is saved.
    assert!(s.execute("select.editSaved", &json!({"name": "Selection 1", "newName": "A"})).is_err());
    s.execute("select.save", &json!({"name": "One"})).unwrap();
    s.execute("select.save", &json!({"name": "Two"})).unwrap();
    // A name already saved can't be taken; both selections keep theirs.
    assert!(s.execute("select.editSaved", &json!({"name": "One", "newName": " Two "})).is_err());
    assert_eq!(s.execute("select.savedList", &json!({})).unwrap(), json!(["One", "Two"]));
    // Renaming to its own name, or a blank name, changes nothing; a free name renames.
    s.execute("select.editSaved", &json!({"name": "One", "newName": "One"})).unwrap();
    s.execute("select.editSaved", &json!({"name": "One", "newName": "  "})).unwrap();
    s.execute("select.editSaved", &json!({"name": "One", "newName": " Uno "})).unwrap();
    assert_eq!(s.execute("select.savedList", &json!({})).unwrap(), json!(["Uno", "Two"]));
}

#[test]
fn saved_selections_live_in_the_document() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let b = rect(&mut s, 20.0, 0.0, 10.0, 10.0);
    let list = |s: &mut Session| s.execute("select.savedList", &json!({})).unwrap();
    sel(&mut s, &[a]);
    s.execute("select.save", &json!({"name": "A"})).unwrap();
    sel(&mut s, &[a, b]);
    s.execute("select.save", &json!({"name": "Both"})).unwrap();
    // They travel with the file.
    let loaded = vectorcraft_format::load(&vectorcraft_format::save_file(&s.doc().unwrap().doc)).unwrap();
    assert_eq!(loaded.saved_selections.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["A", "Both"]);
    // Saving is an undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(list(&mut s), json!(["A"]));
    // An existing name is replaced by the current selection.
    sel(&mut s, &[b]);
    s.execute("select.save", &json!({"name": "A"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(s.execute("select.recall", &json!({"name": "A"})).unwrap()["count"], 1);
    assert_eq!(selected(&s), vec![b]);
    // Objects deleted since are left out of a recall.
    sel(&mut s, &[a, b]);
    s.execute("select.save", &json!({"name": "Pair"})).unwrap();
    sel(&mut s, &[a]);
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(s.execute("select.recall", &json!({"name": "Pair"})).unwrap()["count"], 1);
}

#[test]
fn edit_selection_applies_several_edits_as_one_step() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    for n in ["A", "B", "C"] {
        s.execute("select.save", &json!({ "name": n })).unwrap();
    }
    let list = |s: &mut Session| s.execute("select.savedList", &json!({})).unwrap();
    // Edits name the selections as they are, so two can swap names; a delete goes with them.
    s.execute("select.editSaved", &json!({"edits": [{"name": "A", "newName": "B"}, {"name": "B", "newName": "A"}, {"name": "C", "delete": true}]}))
        .unwrap();
    assert_eq!(list(&mut s), json!(["B", "A"]));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(list(&mut s), json!(["A", "B", "C"]));
    // Two ending with one name, or an unknown name, change nothing.
    assert!(s.execute("select.editSaved", &json!({"edits": [{"name": "A", "newName": "B"}]})).is_err());
    assert!(s.execute("select.editSaved", &json!({"edits": [{"name": "Z", "delete": true}]})).is_err());
    assert_eq!(list(&mut s), json!(["A", "B", "C"]));
}

#[test]
fn a_document_keeps_at_most_25_saved_selections() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    for i in 0..25 {
        s.execute("select.save", &json!({ "name": format!("S{i}") })).unwrap();
    }
    assert!(s.execute("select.save", &json!({"name": "One too many"})).is_err());
    // A name already saved is replaced even when the list is full.
    s.execute("select.save", &json!({"name": "S3"})).unwrap();
    // Unnamed saves take the first free "Selection N".
    s.execute("select.editSaved", &json!({"name": "S0", "delete": true})).unwrap();
    assert_eq!(s.execute("select.save", &json!({})).unwrap()["name"], "Selection 1");
}

// ---------- Type ----------

#[test]
fn change_case_variants() {
    let mut s = session();
    let t = text(&mut s, "hello wORLD. again here");
    sel(&mut s, &[t]);
    s.execute("type.changeCase", &json!({"case": "upper"})).unwrap();
    assert_eq!(plain(&s, t), "HELLO WORLD. AGAIN HERE");
    s.execute("type.changeCase", &json!({"case": "lower"})).unwrap();
    assert_eq!(plain(&s, t), "hello world. again here");
    s.execute("type.changeCase", &json!({"case": "title"})).unwrap();
    assert_eq!(plain(&s, t), "Hello World. Again Here");
    s.execute("type.changeCase", &json!({"case": "sentence"})).unwrap();
    assert_eq!(plain(&s, t), "Hello world. Again here");
    assert!(s.execute("type.changeCase", &json!({"case": "weird"})).is_err());
}

#[test]
fn smart_punctuation_quotes_dashes_ellipsis() {
    let mut s = session();
    let t = text(&mut s, "\"Hi\" it's 1--2 and 3---4...");
    sel(&mut s, &[t]);
    assert_eq!(s.execute("type.smartPunctuation", &json!({})).unwrap()["changed"], 1);
    assert_eq!(plain(&s, t), "“Hi” it’s 1–2 and 3—4…");
}

#[test]
fn convert_point_area_roundtrip() {
    let mut s = session();
    let t = text(&mut s, "Some words");
    sel(&mut s, &[t]);
    s.execute("type.convertToAreaType", &json!({})).unwrap();
    let NodeKind::Text(tt) = &node(&s, t).kind else { panic!() };
    assert!(matches!(tt.kind, TextKind::Area { .. }));
    assert_eq!(tt.plain_text(), "Some words");
    s.execute("type.convertToPointType", &json!({})).unwrap();
    let NodeKind::Text(tt) = &node(&s, t).kind else { panic!() };
    assert!(matches!(tt.kind, TextKind::Point));
    assert_eq!(tt.plain_text(), "Some words");
}

#[test]
fn area_to_point_turns_wraps_into_breaks() {
    let mut s = session();
    let a = id_of(
        &s.execute("text.create", &json!({"x": 0, "y": 0, "text": "one two three four five six", "area": {"width": 40, "height": 200}})).unwrap(),
    );
    sel(&mut s, &[a]);
    s.execute("type.convertToPointType", &json!({})).unwrap();
    let txt = plain(&s, a);
    assert!(txt.contains('\n'), "{txt:?}");
    assert_eq!(txt.replace('\n', " "), "one two three four five six");
}

#[test]
fn placeholder_and_insert_characters() {
    let mut s = session();
    let t = text(&mut s, "x");
    sel(&mut s, &[t]);
    s.execute("type.fillPlaceholder", &json!({})).unwrap();
    assert!(plain(&s, t).len() > 10);
    s.execute("text.setText", &json!({"text": "A"})).unwrap();
    s.execute("type.insert", &json!({"char": "emDash"})).unwrap();
    s.execute("type.insert", &json!({"char": "copyright"})).unwrap();
    s.execute("type.insert", &json!({"text": "z"})).unwrap();
    assert_eq!(plain(&s, t), "A—©z");
    assert!(s.execute("type.insert", &json!({"char": "nope"})).is_err());
}

#[test]
fn area_placeholder_fills_frame() {
    let mut s = session();
    let a = id_of(&s.execute("text.create", &json!({"x": 0, "y": 0, "text": "", "area": {"width": 300, "height": 300}})).unwrap());
    sel(&mut s, &[a]);
    s.execute("type.fillPlaceholder", &json!({})).unwrap();
    assert!(plain(&s, a).len() > 400);
}

#[test]
fn font_and_size_menu_items_use_set_style() {
    let mut s = session();
    let t = text(&mut s, "x");
    sel(&mut s, &[t]);
    s.execute("text.setStyle", &json!({"size": 36})).unwrap();
    s.execute("text.setStyle", &json!({"font": "Inter"})).unwrap();
    let NodeKind::Text(tt) = &node(&s, t).kind else { panic!() };
    assert_eq!(tt.first_style().size, 36.0);
    assert_eq!(tt.first_style().font_family, "Inter");
}

// ---------- View → Guides ----------

#[test]
fn make_release_clear_guides() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    assert_eq!(s.execute("view.guides.make", &json!({})).unwrap()["count"], 1);
    assert!(matches!(node(&s, a).kind, NodeKind::Path { guide: true, .. }));
    assert!(selected(&s).is_empty());
    assert_eq!(s.execute("view.guides.release", &json!({})).unwrap()["count"], 1);
    assert!(matches!(node(&s, a).kind, NodeKind::Path { guide: false, .. }));
    sel(&mut s, &[a]);
    s.execute("view.guides.make", &json!({})).unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 100})).unwrap();
    let n = undo_len(&s);
    assert_eq!(s.execute("view.guides.clear", &json!({})).unwrap()["count"], 2);
    assert!(s.doc().unwrap().doc.guides.is_empty());
    assert!(s.doc().unwrap().doc.node(a).is_none());
    assert_eq!(undo_len(&s), n + 1);
}

#[test]
fn ruler_guides_and_lock() {
    let mut s = session();
    assert_eq!(s.execute("guide.add", &json!({"vertical": false, "pos": 50})).unwrap()["index"], 0);
    s.execute("guide.move", &json!({"index": 0, "pos": 70})).unwrap();
    assert_eq!(s.doc().unwrap().doc.guides[0].pos, 70.0);
    assert_eq!(s.execute("view.guides.lock", &json!({})).unwrap()["locked"], true);
    assert!(s.guides_locked());
    assert!(matches!(s.execute("guide.remove", &json!({"index": 0})), Err(EngineError::Disabled(..))));
    s.execute("view.guides.lock", &json!({})).unwrap();
    s.execute("guide.remove", &json!({"index": 0})).unwrap();
    assert!(s.doc().unwrap().doc.guides.is_empty());
    assert!(s.execute("guide.remove", &json!({"index": 0})).is_err());
}

fn guides(s: &Session) -> Vec<(bool, f64)> {
    s.doc().unwrap().doc.guides.iter().map(|g| (g.vertical, g.pos)).collect()
}

fn selected_guides(s: &Session) -> Vec<usize> {
    s.doc().unwrap().selection.guides.clone()
}

/// #414: ruler guides are selected, moved, copied, nudged and deleted like art, over commands.
#[test]
fn ruler_guides_select_move_and_delete() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    for (vertical, pos) in [(true, 100.0), (false, 50.0), (true, 300.0)] {
        s.execute("guide.add", &json!({"vertical": vertical, "pos": pos})).unwrap();
    }
    sel(&mut s, &[a]);
    // Selecting guides deselects the art; toggle adds and removes.
    assert_eq!(s.execute("guide.select", &json!({"indexes": [0]})).unwrap()["selected"], json!([0]));
    assert!(selected(&s).is_empty());
    assert_eq!(s.execute("guide.select", &json!({"indexes": [1], "toggle": true})).unwrap()["selected"], json!([0, 1]));
    let layer = s.doc().unwrap().current_layer().unwrap().0;
    assert_eq!(
        s.execute("guide.list", &json!({})).unwrap()[1],
        json!({"index": 1, "vertical": false, "pos": 50.0, "selected": true, "layer": layer, "shown": true, "editable": true})
    );
    assert!(s.execute("guide.select", &json!({"indexes": [7]})).is_err());
    // A move takes each selected guide along its own axis, in one undo step.
    let n = undo_len(&s);
    s.execute("guide.move", &json!({"dx": 10, "dy": -5})).unwrap();
    assert_eq!(guides(&s), [(true, 110.0), (false, 45.0), (true, 300.0)]);
    assert_eq!(undo_len(&s), n + 1);
    // Alt: copies, which are then the selection.
    s.execute("guide.move", &json!({"dx": 20, "copy": true})).unwrap();
    assert_eq!(guides(&s).len(), 5);
    assert_eq!(&guides(&s)[3..], [(true, 130.0), (false, 45.0)]);
    assert_eq!(selected_guides(&s), [3, 4]);
    // The arrow keys nudge them.
    s.execute("object.nudge", &json!({"dx": 1, "dy": 0, "big": true})).unwrap();
    assert_eq!(guides(&s)[3], (true, 140.0));
    // Delete (edit.clear) deletes the selected guides; the others keep their places.
    s.execute("guide.select", &json!({"indexes": [1, 3]})).unwrap();
    assert_eq!(s.execute("guide.list", &json!({})).unwrap().as_array().unwrap().len(), 5);
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(guides(&s), [(true, 110.0), (true, 300.0), (false, 45.0)]);
    assert!(selected_guides(&s).is_empty());
    assert!(s.doc().unwrap().doc.node(a).is_some(), "the art stays");
    // Removing one keeps the rest of the selection pointing at the same guides.
    s.execute("guide.select", &json!({"indexes": [1, 2]})).unwrap();
    assert_eq!(s.execute("guide.remove", &json!({"index": 0})).unwrap()["count"], 1);
    assert_eq!((guides(&s), selected_guides(&s)), (vec![(true, 300.0), (false, 45.0)], vec![0, 1]));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((guides(&s).len(), selected_guides(&s)), (3, vec![1, 2]));
    // Out of range positions are refused.
    assert!(s.execute("guide.move", &json!({"index": 0, "pos": 1e300})).is_err());
    assert!(s.execute("guide.add", &json!({"vertical": true, "pos": -1e300})).is_err());
    // Locking deselects them, and they can't be picked again until unlocked.
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    assert!(selected_guides(&s).is_empty());
    assert!(matches!(s.execute("guide.select", &json!({"indexes": [0]})), Err(EngineError::Disabled(..))));
    assert!(matches!(s.execute("edit.clear", &json!({})), Err(EngineError::Disabled(..))));
}

/// Shift selects ruler guides and art together, in either order; Delete takes both in one step.
#[test]
fn guides_and_art_are_shift_selected_together() {
    let mut s = session();
    let a = rect(&mut s, 10.0, 10.0, 40.0, 20.0);
    let b = rect(&mut s, 70.0, 50.0, 60.0, 30.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 200})).unwrap();
    s.execute("guide.add", &json!({"vertical": false, "pos": 300})).unwrap();
    // Art first, then a Shift-click on a guide: the art stays selected.
    sel(&mut s, &[a]);
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    assert_eq!((selected(&s), selected_guides(&s)), (vec![a], vec![0]));
    // A guide first, then Shift-added art: the guide stays selected.
    s.execute("guide.select", &json!({"indexes": [1]})).unwrap();
    s.execute("select.add", &json!({"ids": [b.0]})).unwrap();
    assert_eq!((selected(&s), selected_guides(&s)), (vec![b], vec![1]));
    // A new selection drops them.
    sel(&mut s, &[a]);
    assert!(selected_guides(&s).is_empty());
    // Delete takes the art and the guides, in one step; undo brings both back.
    s.execute("guide.select", &json!({"indexes": [1], "toggle": true})).unwrap();
    let n = undo_len(&s);
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!((guides(&s), undo_len(&s)), (vec![(true, 200.0)], n + 1));
    assert!(s.doc().unwrap().doc.node(a).is_none() && s.doc().unwrap().doc.node(b).is_some());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(guides(&s).len(), 2);
    // Ids given: only those go, the guides stay.
    s.execute("guide.select", &json!({"indexes": [0]})).unwrap();
    s.execute("select.add", &json!({"ids": [b.0]})).unwrap();
    s.execute("edit.clear", &json!({"ids": [b.0]})).unwrap();
    assert_eq!(guides(&s).len(), 2);
}

/// #451: an artboard guide (`guide.add {artboard}`) runs across its artboard only, and is moved,
/// copied and deleted with it; canvas guides stay put.
#[test]
fn artboard_guides_go_with_their_artboard() {
    let mut s = session();
    s.execute("artboard.new", &json!({"x": 900, "y": 0, "width": 400, "height": 300})).unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 1000, "artboard": 1})).unwrap();
    s.execute("guide.add", &json!({"vertical": false, "pos": 100})).unwrap();
    let list = |s: &mut Session| s.execute("guide.list", &json!({})).unwrap();
    let layer = s.doc().unwrap().current_layer().unwrap().0;
    assert_eq!(
        list(&mut s),
        json!([
            {"index": 0, "vertical": true, "pos": 1000.0, "selected": false, "artboard": 1, "layer": layer, "shown": true, "editable": true},
            {"index": 1, "vertical": false, "pos": 100.0, "selected": false, "layer": layer, "shown": true, "editable": true},
        ])
    );
    let spans = |s: &Session| {
        let d = &s.doc().unwrap().doc;
        d.guides.iter().map(|g| d.guide_span(g)).collect::<Vec<_>>()
    };
    assert_eq!(spans(&s), [Some((0.0, 300.0)), None]);
    for bad in [json!(5), json!("x"), json!(-1)] {
        assert!(s.execute("guide.add", &json!({"vertical": true, "pos": 10, "artboard": bad})).is_err(), "{bad}");
    }
    // Moved, the artboard takes its guides along; in Artboard Options too, unless resized.
    s.execute("artboard.move", &json!({"index": 1, "dx": 50, "dy": 20})).unwrap();
    assert_eq!(guides(&s), [(true, 1050.0), (false, 100.0)]);
    s.execute("artboard.setProps", &json!({"index": 1, "x": 960.5})).unwrap();
    assert_eq!(guides(&s)[0], (true, 1060.5));
    s.execute("artboard.setProps", &json!({"index": 1, "x": 900, "width": 500})).unwrap();
    assert_eq!((guides(&s)[0], spans(&s)[0]), ((true, 1060.5), Some((20.0, 320.0))));
    // Renumbered, it keeps them.
    s.execute("artboard.reorder", &json!({"index": 1, "to": 0})).unwrap();
    assert_eq!(list(&mut s)[0]["artboard"], 0);
    // Duplicated (or Alt-dragged), the copy gets copies of them.
    let dup = s.execute("artboard.duplicate", &json!({"index": 0})).unwrap()["index"].as_u64().unwrap();
    let dx = s.doc().unwrap().doc.artboards[dup as usize].rect.x0 - 900.0;
    assert_eq!(guides(&s), [(true, 1060.5), (false, 100.0), (true, 1060.5 + dx)]);
    assert_eq!(list(&mut s)[2]["artboard"], dup);
    s.execute("artboard.move", &json!({"index": 0, "dx": 0, "dy": 400, "copy": true})).unwrap();
    assert_eq!((guides(&s).len(), list(&mut s)[3]["artboard"].clone()), (4, json!(3)));
    // Rearranged, they follow their artboards.
    s.execute("artboard.rearrange", &json!({"columns": 4})).unwrap();
    let d = &s.doc().unwrap().doc;
    for (g, ab) in d.guides.iter().filter_map(|g| Some((g, d.artboards.iter().find(|a| Some(a.id) == g.artboard)?))) {
        assert_eq!(g.pos - ab.rect.x0, 160.5, "{g:?} on {ab:?}");
    }
    // Deleted, the artboard takes its guides; the selected ones left stay selected.
    s.execute("guide.select", &json!({"indexes": [1, 2]})).unwrap();
    let n = undo_len(&s);
    s.execute("artboard.delete", &json!({"index": 0})).unwrap();
    assert_eq!((guides(&s).len(), selected_guides(&s)), (3, vec![0, 1]));
    assert_eq!(
        list(&mut s)[0],
        json!({"index": 0, "vertical": false, "pos": 100.0, "selected": true, "layer": layer, "shown": true, "editable": true})
    );
    assert_eq!(undo_len(&s), n + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((guides(&s).len(), selected_guides(&s)), (4, vec![1, 2]));
    // Cut, too.
    s.execute("artboard.cut", &json!({"index": 0})).unwrap();
    assert_eq!((guides(&s).len(), selected_guides(&s)), (3, vec![0, 1]));
}

/// #451: a guide dragged out of a ruler snaps (to the art's corners, side midpoints and anchors,
/// the artboards' edges and centres) and is made in one undo step where it is released over the
/// canvas; with the Artboard tool it is an artboard guide of the active artboard.
#[test]
fn guides_dragged_out_of_a_ruler_snap_and_take_the_artboard_tool_s_artboard() {
    use vectorcraft_tools::{Overlay, PointerEvent, PointerKind};
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let v = ViewInfo::default();
    let ev = |kind, x, y| PointerEvent::new(kind, x, y);
    let n = undo_len(&s);
    s.ruler_guide(true, &ev(PointerKind::Drag, -10.0, 30.0), false, v).unwrap();
    assert!(guides(&s).is_empty(), "still over the ruler");
    s.ruler_guide(true, &ev(PointerKind::Drag, 153.0, 30.0), true, v).unwrap();
    assert_eq!(guides(&s), [(true, 150.0)], "onto the line through the top and bottom midpoints");
    assert!(s.overlays(v).iter().any(|o| matches!(o, Overlay::Label { text, .. } if text == "center")));
    s.ruler_guide(true, &ev(PointerKind::Drag, 797.0, 30.0), true, v).unwrap();
    assert_eq!(guides(&s), [(true, 800.0)], "the artboard's edge");
    s.ruler_guide(true, &ev(PointerKind::Up, 797.0, 30.0), true, v).unwrap();
    assert_eq!(guides(&s), [(true, 800.0)]);
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "New Guide");
    assert!(s.overlays(v).is_empty());
    // Released off the canvas: none.
    s.ruler_guide(false, &ev(PointerKind::Drag, 30.0, 250.0), true, v).unwrap();
    s.ruler_guide(false, &ev(PointerKind::Up, 30.0, -10.0), false, v).unwrap();
    assert_eq!((guides(&s).len(), undo_len(&s)), (1, n + 1));
    // The Artboard tool: an artboard guide of its active artboard.
    s.execute("artboard.new", &json!({"x": 900, "y": 0, "width": 400, "height": 300})).unwrap();
    s.select_tool("artboard", v).unwrap();
    s.set_tool_option("active", &json!(1));
    s.ruler_guide(false, &ev(PointerKind::Drag, 1000.0, 50.0), true, v).unwrap();
    s.ruler_guide(false, &ev(PointerKind::Up, 1000.0, 50.0), true, v).unwrap();
    let layer = s.doc().unwrap().current_layer().unwrap().0;
    assert_eq!(
        s.execute("guide.list", &json!({})).unwrap()[1],
        json!({"index": 1, "vertical": false, "pos": 50.0, "selected": false, "artboard": 1, "layer": layer, "shown": true, "editable": true})
    );
}

/// Guides saved before guides had layers are put on a layer when the document opens.
#[test]
fn guides_from_older_files_are_put_on_a_layer() {
    let mut s = Session::new();
    let mut d = vectorcraft_doc::Document::new(400.0, 300.0);
    d.guides.push(vectorcraft_doc::Guide::new(true, 50.0));
    d.guides.push(vectorcraft_doc::Guide::new(false, 80.0));
    s.add_document(d, None);
    let st = s.doc().unwrap();
    let top = st.doc.default_layer();
    assert!(top.is_some() && st.doc.guides.iter().all(|g| g.layer == top), "{:?}", st.doc.guides);
}

/// Ruler guides are layer objects: made on the current layer, hidden, locked and deleted with it,
/// and moved to another layer.
#[test]
fn ruler_guides_live_on_layers() {
    let mut s = session();
    let first = s.doc().unwrap().current_layer().unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 200})).unwrap();
    assert_eq!(s.doc().unwrap().doc.guides[0].layer, Some(first), "on the current layer");
    let second = id_of(&s.execute("layer.new", &json!({"name": "Guides"})).unwrap());
    s.execute("guide.add", &json!({"vertical": false, "pos": 300})).unwrap();
    assert_eq!(s.doc().unwrap().doc.guides[1].layer, Some(second), "the new layer is current");
    assert!(s.execute("guide.add", &json!({"vertical": true, "pos": 1, "layer": 99999})).is_err());
    // Hidden or locked with its layer.
    s.execute("layer.setProps", &json!({"ids": [second.0], "visible": false})).unwrap();
    assert_eq!(s.execute("guide.list", &json!({})).unwrap()[1]["shown"], false);
    s.execute("layer.setProps", &json!({"ids": [second.0], "visible": true, "locked": true})).unwrap();
    let row = s.execute("guide.list", &json!({})).unwrap()[1].clone();
    assert_eq!((row["shown"].clone(), row["editable"].clone()), (json!(true), json!(false)));
    s.execute("layer.setProps", &json!({"ids": [second.0], "locked": false})).unwrap();
    // Moved to the first layer, then back.
    s.execute("guide.setLayer", &json!({"index": 1, "layer": first.0})).unwrap();
    assert_eq!(s.doc().unwrap().doc.guides[1].layer, Some(first));
    s.execute("guide.setLayer", &json!({"index": 1, "layer": second.0})).unwrap();
    assert!(s.execute("guide.setLayer", &json!({"index": 1, "layer": 99999})).is_err());
    // Several at once, as dragging their rows onto a layer's row does.
    s.execute("guide.setLayer", &json!({"indexes": [0, 1], "layer": first.0})).unwrap();
    assert!(s.doc().unwrap().doc.guides.iter().all(|g| g.layer == Some(first)));
    s.execute("guide.setLayer", &json!({"indexes": [1], "layer": second.0})).unwrap();
    assert!(s.execute("guide.setLayer", &json!({"indexes": [7], "layer": first.0})).is_err());
    // Deleting a layer deletes its guides; undo brings them back.
    s.execute("layer.delete", &json!({"ids": [second.0]})).unwrap();
    assert_eq!(guides(&s), [(true, 200.0)]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(guides(&s).len(), 2);
}

/// Guides on a hidden or locked layer leave the selection and can't be moved, deleted or moved to
/// another layer; new guides skip a locked current layer as new art does; no guide goes onto a
/// locked layer.
#[test]
fn guides_on_hidden_or_locked_layers_are_left_alone() {
    let mut s = session();
    let first = s.doc().unwrap().current_layer().unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 200})).unwrap();
    let second = id_of(&s.execute("layer.new", &json!({})).unwrap());
    s.execute("guide.add", &json!({"vertical": false, "pos": 300})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0, 1]})).unwrap();
    s.execute("layer.setProps", &json!({"ids": [first.0], "locked": true})).unwrap();
    assert_eq!(selected_guides(&s), [1], "the locked layer's guide is deselected");
    assert_eq!(s.execute("guide.select", &json!({"indexes": [0]})).unwrap()["selected"], json!([]), "and can't be selected");
    assert!(s.execute("guide.move", &json!({"index": 0, "pos": 50})).is_err());
    assert!(s.execute("guide.remove", &json!({"index": 0})).is_err());
    assert!(s.execute("guide.setLayer", &json!({"index": 0, "layer": second.0})).is_err());
    assert!(s.execute("guide.setLayer", &json!({"index": 1, "layer": first.0})).is_err(), "not onto a locked layer");
    s.execute("layer.setProps", &json!({"ids": [second.0], "visible": false})).unwrap();
    assert!(selected_guides(&s).is_empty(), "the hidden layer's guide is deselected");
    // Both layers hidden or locked: a new guide is refused, as new art is.
    assert!(s.execute("guide.add", &json!({"vertical": true, "pos": 10})).is_err());
    s.execute("layer.setProps", &json!({"ids": [second.0], "visible": true, "locked": true})).unwrap();
    s.execute("layer.setProps", &json!({"ids": [first.0], "locked": false})).unwrap();
    // The current layer locked: the new guide goes on the top shown, unlocked layer.
    s.execute("guide.add", &json!({"vertical": true, "pos": 20})).unwrap();
    assert_eq!(s.doc().unwrap().doc.guides[2].layer, Some(first));
    assert_eq!(guides(&s).len(), 3);
}

/// Guides follow their layers through Duplicate, Merge Selected and Flatten Artwork, in one undo
/// step each.
#[test]
fn guides_go_with_their_layers_when_duplicated_merged_and_flattened() {
    let mut s = session();
    let first = s.doc().unwrap().current_layer().unwrap();
    s.execute("guide.add", &json!({"vertical": true, "pos": 200})).unwrap();
    let second = id_of(&s.execute("layer.new", &json!({})).unwrap());
    s.execute("guide.add", &json!({"vertical": false, "pos": 300})).unwrap();
    let on = |s: &Session, l: NodeId| s.doc().unwrap().doc.guides_on(l).map(|(_, g)| (g.vertical, g.pos)).collect::<Vec<_>>();
    // Duplicate Layer copies its guides onto the copy.
    let copy = s.execute("layer.duplicate", &json!({"ids": [second.0]})).unwrap()["ids"][0].as_u64().map(NodeId).unwrap();
    assert_eq!((on(&s, second), on(&s, copy)), (vec![(false, 300.0)], vec![(false, 300.0)]));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(guides(&s).len(), 2);
    // Merged into the first layer, the second layer's guide goes with its art.
    s.execute("layer.merge", &json!({"ids": [second.0, first.0]})).unwrap();
    assert_eq!(on(&s, first), [(true, 200.0), (false, 300.0)]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!((on(&s, first), on(&s, second)), (vec![(true, 200.0)], vec![(false, 300.0)]));
    // Flattened into the first layer, a hidden layer is discarded with its guides.
    s.execute("layer.setProps", &json!({"ids": [second.0], "visible": false})).unwrap();
    s.execute("layer.flatten", &json!({"id": first.0})).unwrap();
    assert_eq!(guides(&s), [(true, 200.0)]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(guides(&s).len(), 2);
}

/// #451: art moved with the Selection tool lands flush on the artboard's edges and centre (Smart
/// Guides).
#[test]
fn moved_art_snaps_to_the_artboard_edges_and_centre() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let mut drag = |from: (f64, f64), to: (f64, f64)| {
        for (kind, (x, y)) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            s.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
        }
        let b = bounds(&s, a);
        (b.x0, b.y0)
    };
    assert_eq!(drag((150.0, 150.0), (53.0, 147.0)), (0.0, 97.0), "3 pt off the left edge: flush with it");
    assert_eq!(drag((50.0, 147.0), (402.0, 298.0)), (350.0, 250.0), "its centre on the artboard's");
}

/// #414: the Selection tool picks a ruler guide over the art and drags it in one undo step; with
/// guides hidden or locked the press goes to the art.
#[test]
fn selection_tool_drags_a_ruler_guide() {
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 150})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    s.select_tool("selection", v).unwrap();
    let drag = |s: &mut Session, v: ViewInfo, from: (f64, f64), to: (f64, f64)| {
        s.pointer(&PointerEvent::new(PointerKind::Down, from.0, from.1), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Drag, to.0, to.1), v).unwrap();
        s.pointer(&PointerEvent::new(PointerKind::Up, to.0, to.1), v).unwrap();
    };
    let n = undo_len(&s);
    drag(&mut s, v, (151.0, 120.0), (171.0, 140.0));
    assert_eq!(guides(&s), [(true, 170.0)]);
    assert_eq!(selected_guides(&s), [0]);
    assert!(selected(&s).is_empty());
    assert_eq!(bounds(&s, a).x0, 100.0, "the art stays");
    assert_eq!(undo_len(&s), n + 1);
    assert_eq!(s.doc().unwrap().history.undo.last().unwrap().label, "Move Guide");
    // Hidden guides: the press takes the art.
    let hidden = ViewInfo { guides: false, ..v };
    drag(&mut s, hidden, (170.0, 120.0), (180.0, 120.0));
    assert_eq!((guides(&s), bounds(&s, a).x0), (vec![(true, 170.0)], 110.0));
    // Locked guides too.
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    drag(&mut s, v, (170.0, 120.0), (180.0, 120.0));
    assert_eq!((guides(&s), bounds(&s, a).x0), (vec![(true, 170.0)], 120.0));
}

// ---------- File ----------

#[test]
fn close_all_documents() {
    let mut s = session();
    s.execute("file.new", &json!({})).unwrap();
    assert_eq!(s.execute("file.closeAll", &json!({})).unwrap()["closed"], 2);
    assert!(s.active().is_none());
    assert!(s.execute("file.closeAll", &json!({})).is_err());
}

#[test]
fn document_color_mode_converts_colours() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    fill(&mut s, a, "#ff0000");
    s.execute("file.documentColorMode", &json!({"mode": "cmyk"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.color_mode, ColorMode::Cmyk);
    // Converted through the colour settings: a press red, mostly magenta and yellow.
    let Some(Color::Cmyk { c, m, y, k }) = node(&s, a).appearance.fill_paint().color() else { panic!("not CMYK") };
    assert!(m > 0.8 && y > 0.8 && c < 0.05 && k < 0.05, "{c} {m} {y} {k}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.color_mode, ColorMode::Rgb);
    assert!(s.execute("file.documentColorMode", &json!({"mode": "lab"})).is_err());
}

/// Four RGB greys (#000000, #333333, #808080, #e6e6e6, as in issue #421) and a red, a 40% tint of a
/// grey global swatch, a gradient to grey and a pattern of a grey tile → (session, the five
/// rectangles, the tinted one, the gradient one).
fn greys_document() -> (Session, Vec<NodeId>, NodeId, NodeId) {
    let mut s = session();
    let ids: Vec<NodeId> = ["#000000", "#333333", "#808080", "#e6e6e6", "#ff0000"]
        .iter()
        .map(|hex| {
            let id = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
            fill(&mut s, id, hex);
            id
        })
        .collect();
    s.execute("swatch.new", &json!({"name": "Ink", "color": "#808080", "global": true})).unwrap();
    let tint = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [tint.0], "swatch": "Ink", "tint": 40})).unwrap();
    let grad = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#333333"}]);
    s.execute("paint.setFill", &json!({"ids": [grad.0], "gradient": {"kind": "linear", "stops": stops}})).unwrap();
    let tile = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    fill(&mut s, tile, "#808080");
    s.execute("object.pattern.make", &json!({"ids": [tile.0], "name": "Grey Tile", "edit": false})).unwrap();
    (s, ids, tint, grad)
}

/// The colour of pattern `name`'s first tile object.
fn pattern_fill(s: &Session, name: &str) -> Color {
    s.doc().unwrap().doc.pattern(name).unwrap().art[0].appearance.fill_paint().color().unwrap()
}

/// K of a colour that prints on the black plate only.
fn k_only(c: Color) -> f32 {
    match c {
        Color::Cmyk { c: 0.0, m: 0.0, y: 0.0, k } => k,
        other => panic!("not K only: {other:?}"),
    }
}

#[test]
fn document_color_mode_separates_rgb_greys_through_the_profile() {
    let (mut s, ids, _, _) = greys_document();
    s.execute("file.documentColorMode", &json!({"mode": "cmyk"})).unwrap();
    // As Illustrator converts them: four-colour greys and a rich black.
    let Color::Cmyk { c, m, y, k } = fill_of(&s, ids[2]) else { panic!("not CMYK") };
    assert!(c > 0.05 && m > 0.05 && y > 0.05 && k > 0.0, "a four-colour mid grey: {c} {m} {y} {k}");
    let Color::Cmyk { c, k, .. } = fill_of(&s, ids[0]) else { panic!("not CMYK") };
    assert!(k > 0.9 && c > 0.3, "rich black: {c} {k}");
    // Pattern tiles are swatches too: their art converts with the rest.
    assert!(matches!(pattern_fill(&s, "Grey Tile"), Color::Cmyk { .. }), "{:?}", pattern_fill(&s, "Grey Tile"));
    // Gray colours stay Gray.
    let mut g = session();
    let a = rect(&mut g, 0.0, 0.0, 10.0, 10.0);
    g.execute("paint.setFill", &json!({"ids": [a.0], "color": {"gray": 0.3}})).unwrap();
    g.execute("file.documentColorMode", &json!({"mode": "cmyk"})).unwrap();
    assert_eq!(fill_of(&g, a), Color::gray(0.3));
}

#[test]
fn document_color_mode_can_put_rgb_greys_on_the_black_plate() {
    let (mut profile, pids, ..) = greys_document();
    profile.execute("file.documentColorMode", &json!({"mode": "cmyk"})).unwrap();
    let (mut s, ids, tint, grad) = greys_document();
    let e = s.execute("file.documentColorMode", &json!({"mode": "cmyk", "grays": "rich"})).unwrap_err().to_string();
    assert!(e.contains("grays"), "{e}");
    s.execute("file.documentColorMode", &json!({"mode": "cmyk", "grays": "black"})).unwrap();
    let close = |a: f32, b: f32| (a - b).abs() < 0.005;
    for (id, want) in ids.iter().zip([1.0, 0.8, 0.498, 0.098]) {
        let k = k_only(fill_of(&s, *id));
        assert!(close(k, want), "K {k}, want {want}");
    }
    // Other colours convert through the profile as before.
    assert_eq!(fill_of(&s, ids[4]), fill_of(&profile, pids[4]));
    // Swatches, their tints, gradient stops and pattern tiles follow the same rule.
    let d = &s.doc().unwrap().doc;
    assert!(close(k_only(d.swatch("Ink").unwrap().paint.color().unwrap()), 0.498));
    let Paint::Solid { color, swatch, tint: t } = node(&s, tint).appearance.fill_paint() else { panic!("not solid") };
    assert!(close(k_only(color), 0.498 * 0.4) && swatch.as_deref() == Some("Ink") && t == 0.4, "{color:?} {swatch:?} {t}");
    let Paint::Gradient(g) = node(&s, grad).appearance.fill_paint() else { panic!("not a gradient") };
    let ks: Vec<f32> = g.gradient.stops.iter().map(|st| k_only(st.color)).collect();
    assert!(close(ks[0], 0.0) && close(ks[1], 0.8), "{ks:?}");
    assert!(close(k_only(pattern_fill(&s, "Grey Tile")), 0.498));
    // To RGB, `grays` changes nothing.
    s.execute("file.documentColorMode", &json!({"mode": "rgb", "grays": "black"})).unwrap();
    let rgb = s.doc().unwrap().doc.layers.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("file.documentColorMode", &json!({"mode": "rgb"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers, rgb);
}

#[test]
fn opening_in_cmyk_can_put_rgb_greys_on_the_black_plate() {
    let mut s = session();
    for hex in ["#000000", "#333333", "#808080", "#e6e6e6", "#ff0000"] {
        let id = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
        fill(&mut s, id, hex);
    }
    let svg = s.execute("document.serialize", &json!({"format": "svg"})).unwrap()["text"].as_str().unwrap().to_string();
    let data = vectorcraft_format::base64_encode(svg.as_bytes());
    let mut o = Session::new();
    o.execute("document.open", &json!({"name": "greys.svg", "dataBase64": data, "colorMode": "cmyk", "grays": "black"})).unwrap();
    let d = &o.doc().unwrap().doc;
    assert_eq!(d.color_mode, ColorMode::Cmyk);
    let mut fills = vec![];
    for l in &d.layers {
        l.walk(&mut |n| fills.extend(n.children().is_none().then(|| n.appearance.fill_paint().color()).flatten()));
    }
    assert_eq!(fills.len(), 5, "{fills:?}");
    for (c, want) in fills.iter().zip([1.0, 0.8, 0.498, 0.098]) {
        let k = k_only(*c);
        assert!((k - want).abs() < 0.005, "K {k}, want {want}");
    }
    assert!(matches!(fills[4], Color::Cmyk { m, y, .. } if m > 0.8 && y > 0.8), "red separates as usual: {:?}", fills[4]);
    let e = Session::new().execute("document.open", &json!({"name": "greys.svg", "dataBase64": data, "grays": "k"})).unwrap_err().to_string();
    assert!(e.contains("grays"), "{e}");
}

#[test]
fn file_info_sets_title() {
    let mut s = session();
    let r = s.execute("file.info", &json!({"title": "Poster"})).unwrap();
    assert_eq!(r["title"], "Poster");
    assert_eq!(s.doc().unwrap().doc.title, "Poster");
    assert_eq!(s.execute("file.info", &json!({})).unwrap()["colorMode"], "rgb");
}

#[test]
fn every_new_command_has_params_doc_and_menu() {
    for id in [
        "object.lock.above",
        "object.transformEach",
        "object.rasterize",
        "object.blend.make",
        "edit.colors.invert",
        "edit.findReplace",
        "select.same.fontSize",
        "type.changeCase",
        "view.guides.make",
        "file.closeAll",
    ] {
        let c = find_command(id).unwrap_or_else(|| panic!("{id}"));
        assert!(!c.params.is_empty() && !c.menu.is_empty(), "{id}");
    }
}

#[test]
fn saved_selection_names_are_trimmed_and_capped() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    sel(&mut s, &[a]);
    let long = "n".repeat(1000);
    let name = s.execute("select.save", &json!({ "name": format!(" {long} ") })).unwrap()["name"].clone();
    assert_eq!(name.as_str().map(|n| n.chars().count()), Some(vectorcraft_doc::SavedSelection::MAX_NAME));
    s.execute("select.editSaved", &json!({"name": name, "newName": format!("{long}é")})).unwrap();
    let list = s.execute("select.savedList", &json!({})).unwrap();
    assert_eq!(list[0].as_str().map(|n| n.chars().count()), Some(vectorcraft_doc::SavedSelection::MAX_NAME));
}

#[test]
fn crop_image_trim_cuts_to_the_opaque_pixels_as_they_are() {
    // 100 × 80 px, transparent but for a block at (20, 10)–(50, 40) px; placed at 2 pt a pixel.
    let mut px = image::RgbaImage::new(100, 80);
    for y in 10..40 {
        for x in 20..50 {
            px.put_pixel(x, y, image::Rgba([x as u8, y as u8, 7, if x == 20 { 1 } else { 255 }]));
        }
    }
    let mut png = Vec::new();
    px.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "art.png", "dataBase64": vectorcraft_format::base64_encode(&png)})).unwrap();
    s.execute("select.all", &json!({})).unwrap();
    let img = selected(&s)[0];
    s.execute("object.scale", &json!({"sx": 200, "origin": [0, 0]})).unwrap();
    let r = s.execute("object.cropImage", &json!({"trim": true})).unwrap();
    assert_eq!((r["width"].as_u64().unwrap(), r["height"].as_u64().unwrap()), (30, 30), "{r}");
    // Placed where those pixels were: (40, 20) pt, 60 × 60 pt at 2 pt a pixel.
    let b = bounds(&s, img);
    assert!(close(b.x0, 40.0) && close(b.y0, 20.0) && close(b.width(), 60.0) && close(b.height(), 60.0), "{b:?}");
    // Pixel for pixel, a barely visible (alpha 1) edge included.
    let NodeKind::Image(im) = &node(&s, img).kind else { panic!("an image") };
    let out = image::load_from_memory(&s.doc().unwrap().doc.images[&im.key].bytes).unwrap().to_rgba8();
    assert_eq!(out, image::imageops::crop_imm(&px, 20, 10, 30, 30).to_image());
    assert_eq!(r["trimmed"], true);
    // Nothing transparent left to cut: left as it is, and not an error (scripts trim every image).
    let r = s.execute("object.cropImage", &json!({"trim": true})).unwrap();
    assert_eq!((r["trimmed"].as_bool(), r["width"].as_u64()), (Some(false), Some(30)), "{r}");
    assert_eq!(bounds(&s, img), b);
}
