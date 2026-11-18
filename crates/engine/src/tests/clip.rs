//! Clipping sets with any clip shape (M3.26): Make Clipping Mask takes a compound path or text on
//! top and Release gives it back; compound holes, even-odd fills, text outlines and imported
//! multi-shape clip paths clip on screen, and the SVG and PDF output clip the same way.

use serde_json::{Value, json};
use vectorcraft_render::Rendered;

use crate::*;

pub(crate) fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    s
}

pub(crate) fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn ellipse(s: &mut Session, x: f64, w: f64) -> NodeId {
    id_of(&s.execute("shape.ellipse", &json!({"x": x, "y": x, "width": w, "height": w})).unwrap())
}

pub(crate) fn select(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

/// The active document's 100×100 artboard, rendered transparent.
pub(crate) fn render(s: &Session) -> Rendered {
    vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 100, 100, Affine::IDENTITY, &Default::default())
}

pub(crate) fn opaque(r: &Rendered, x: u32, y: u32) -> bool {
    r.pixel(x, y)[3] > 128
}

/// The document exported as `format` and opened again, rendered.
fn reopened(s: &mut Session, format: &str) -> Rendered {
    let data = s.execute("document.export", &json!({"format": format})).unwrap()["dataBase64"].clone();
    let mut o = Session::new();
    o.execute("document.open", &json!({"name": format!("clip.{format}"), "dataBase64": data})).unwrap();
    render(&o)
}

/// The same probes are opaque on screen and in the SVG and PDF output.
pub(crate) fn assert_outputs_agree(s: &mut Session, probes: &[(u32, u32)]) {
    let screen = render(s);
    for format in ["svg", "pdf"] {
        let out = reopened(s, format);
        for &(x, y) in probes {
            assert_eq!(opaque(&out, x, y), opaque(&screen, x, y), "{format} at ({x}, {y})");
        }
    }
}

/// The leaf a click at (`x`, `y`) hits.
pub(crate) fn hit(s: &Session, x: f64, y: f64) -> Option<NodeId> {
    vectorcraft_doc::hit::hit_test(&s.doc().unwrap().doc, vectorcraft_geom::Point::new(x, y), Default::default()).map(|h| h.leaf)
}

/// Art under the whole artboard, then a donut (compound path with a real hole) on top.
fn art_and_donut(s: &mut Session) -> (NodeId, NodeId) {
    let art = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let (outer, inner) = (ellipse(s, 10.0, 80.0), ellipse(s, 35.0, 30.0));
    select(s, &[inner]);
    s.execute("path.reverse", &json!({})).unwrap();
    select(s, &[outer, inner]);
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    let donut = s.doc().unwrap().selection.objects[0];
    (art, donut)
}

#[test]
fn compound_clip_keeps_its_hole_and_release_gives_the_compound_back() {
    let mut s = session();
    let (art, donut) = art_and_donut(&mut s);
    select(&mut s, &[art, donut]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    let d = &s.doc().unwrap().doc;
    let NodeKind::Group { children, clip: true } = &d.node(g).unwrap().kind else { panic!("a clip group") };
    assert_eq!(children[1].id, donut, "on top");
    assert!(matches!(children[1].kind, NodeKind::Compound { .. }) && children[1].appearance.fill_paint().is_none());
    let r = render(&s);
    assert!(opaque(&r, 20, 50), "the ring shows the art");
    assert!(!opaque(&r, 50, 50), "the hole stays transparent");
    assert!(!opaque(&r, 3, 3), "outside the ring");
    assert_outputs_agree(&mut s, &[(20, 50), (50, 50), (3, 3), (80, 50)]);
    // Clicks hit the art only where it shows.
    assert_eq!(hit(&s, 20.0, 50.0), Some(art));
    assert_eq!(hit(&s, 50.0, 50.0), None, "the hole");
    // Release: a plain group, the compound path back on top of the art, unpainted.
    select(&mut s, &[g]);
    s.execute("object.clippingMask.release", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    let NodeKind::Group { children, clip: false } = &d.node(g).unwrap().kind else { panic!("a plain group") };
    assert!(matches!(children[1].kind, NodeKind::Compound { .. }) && children[1].id == donut);
    assert!(opaque(&render(&s), 50, 50), "nothing clips any more");
}

#[test]
fn text_on_top_clips_by_its_glyphs() {
    let mut s = session();
    let art = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let t = id_of(&s.execute("text.create", &json!({"x": 5, "y": 80, "text": "O", "size": 90})).unwrap());
    let b = s.doc().unwrap().doc.node(t).unwrap().geometric_bounds().unwrap();
    select(&mut s, &[art, t]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    let d = &s.doc().unwrap().doc;
    let NodeKind::Text(text) = &d.node(t).unwrap().kind else { panic!("still text") };
    assert!(text.runs.iter().all(|r| r.style.fill.is_none() && r.style.stroke.is_none()), "the clipping path loses its paint");
    // Across the middle of the "O": clear, ring, the counter (clear), ring, clear.
    let cy = ((b.y0 + b.y1) / 2.0) as u32;
    let r = render(&s);
    let row: Vec<bool> = (0..100).map(|x| opaque(&r, x, cy)).collect();
    assert_eq!(row.windows(2).filter(|w| w[0] != w[1]).count(), 4, "{row:?}");
    let probes: Vec<(u32, u32)> = (0..100).step_by(3).map(|x| (x, cy)).collect();
    assert_outputs_agree(&mut s, &probes);
    // Text clips clicks by its frame.
    assert_eq!(hit(&s, (b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0), Some(art));
    assert!(b.x1 < 95.0, "{b:?}");
    assert_eq!(hit(&s, b.x1 + 3.0, (b.y0 + b.y1) / 2.0), None, "beside the text");
    // An effect that does not reshape it, such as a drop shadow, leaves clicks on its frame.
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "params": {}, "ids": [t.0], "item": null})).unwrap();
    assert_eq!(hit(&s, (b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0), Some(art), "in the counter, with a drop shadow");
    // Release gives the text back.
    select(&mut s, &[g]);
    s.execute("object.clippingMask.release", &json!({})).unwrap();
    assert!(matches!(s.doc().unwrap().doc.node(t).unwrap().kind, NodeKind::Text(_)));
}

#[test]
fn a_hidden_member_of_a_clipping_group_leaves_clicks_on_the_type_frame() {
    // The clipping path is a group of an "O" and a hidden rectangle with a Transform effect. The
    // hidden rectangle does not clip, so clicks inside the clip group still count the type by its
    // frame, the counter of the "O" included.
    let mut s = session();
    let art = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let t = id_of(&s.execute("text.create", &json!({"x": 5, "y": 80, "text": "O", "size": 90})).unwrap());
    let b = s.doc().unwrap().doc.node(t).unwrap().geometric_bounds().unwrap();
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 10, "height": 10})).unwrap());
    s.execute("effect.apply", &json!({"effect": "distort.transform", "params": {"moveH": 20}, "ids": [r.0], "item": null})).unwrap();
    select(&mut s, &[art, t]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    // Make Clipping Mask takes no group on top, so the group goes in place of the type.
    let mut d = (*s.doc().unwrap().doc).clone();
    let mut rect = (*d.remove(r).unwrap()).clone();
    rect.visible = false;
    let text = d.remove(t).unwrap();
    let group = vectorcraft_doc::Node::group(d.alloc_id(), vec![text, Arc::new(rect)]);
    d.insert(Some(g), 0, group).unwrap();
    let leaf = |x: f64, y: f64| vectorcraft_doc::hit::hit_test(&d, vectorcraft_geom::Point::new(x, y), Default::default()).map(|h| h.leaf);
    let middle = ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
    assert_eq!(leaf(middle.0, middle.1), Some(art), "in the counter, inside the frame");
    assert_eq!(leaf(b.x1 + 3.0, middle.1), None, "beside the text");
}

#[test]
fn clicks_follow_the_region_a_clipping_paths_effects_give_it() {
    // Art from 10 to 90 pt, clipped by the artboard's left half, which a Transform effect moves
    // 50 pt right: the art shows on the right half, as clicks find it.
    let mut s = session();
    let art = id_of(&s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 80, "height": 80})).unwrap());
    let clip = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 50, "height": 100})).unwrap());
    select(&mut s, &[art, clip]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    s.execute("effect.apply", &json!({"effect": "distort.transform", "params": {"moveH": 50}, "ids": [clip.0], "item": null})).unwrap();
    let r = render(&s);
    assert!(opaque(&r, 75, 50) && !opaque(&r, 25, 50), "the canvas clips by the moved region");
    assert_eq!(hit(&s, 75.0, 50.0), Some(art), "where the effect moved the clipping path");
    assert_eq!(hit(&s, 25.0, 50.0), None, "the clipping path's own place");
    let d = &s.doc().unwrap().doc;
    let at = |x: f64| vectorcraft_doc::hit::objects_at(d, vectorcraft_geom::Point::new(x, 50.0), Default::default(), None);
    assert_eq!(at(75.0), [g]);
    assert!(at(25.0).is_empty());
    // A marquee across the art's edge reaches it within the moved region only.
    let marquee = |x0: f64, x1: f64| vectorcraft_doc::hit::marquee(d, vectorcraft_geom::Rect::new(x0, 40.0, x1, 60.0), None, true);
    assert_eq!(marquee(85.0, 95.0), [art], "the right edge, in the moved region");
    assert!(marquee(5.0, 15.0).is_empty(), "the left edge, in the clipping path's own place");

    // The same with a sublayer that clips by its first object.
    let mut d = (**d).clone();
    let members: Vec<_> = d.node(g).unwrap().children().unwrap().to_vec();
    let top = d.layers[0].id;
    d.remove(g).unwrap();
    let mut sublayer = (*d.layers[0]).clone();
    sublayer.id = d.alloc_id();
    if let NodeKind::Layer { children, clip, .. } = &mut sublayer.kind {
        (*children, *clip) = (members, true);
    }
    d.insert(Some(top), 0, sublayer).unwrap();
    let leaf = |x: f64| vectorcraft_doc::hit::hit_test(&d, vectorcraft_geom::Point::new(x, 50.0), Default::default()).map(|h| h.leaf);
    assert_eq!(leaf(75.0), Some(art));
    assert_eq!(leaf(25.0), None);
}

#[test]
fn make_refuses_a_top_object_that_cannot_clip() {
    let mut s = session();
    let art = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let (a, b) = (ellipse(&mut s, 10.0, 20.0), ellipse(&mut s, 50.0, 20.0));
    select(&mut s, &[a, b]);
    s.execute("object.group", &json!({})).unwrap();
    let group = s.doc().unwrap().selection.objects[0];
    select(&mut s, &[art, group]);
    let err = s.execute("object.clippingMask.make", &json!({})).unwrap_err();
    assert!(err.to_string().contains("path, compound path or text"), "{err}");
}

/// An SVG clip path made of several shapes (imported as one multi-subpath clipping path) and an
/// even-odd star clip path.
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100">
<defs>
  <clipPath id="two"><rect x="0" y="0" width="30" height="40"/><rect x="60" y="0" width="30" height="40"/></clipPath>
  <clipPath id="star"><path clip-rule="evenodd" d="M70 49 L84.7 94.2 L46.2 66.3 L93.8 66.3 L55.3 94.2 Z"/></clipPath>
</defs>
<g clip-path="url(#two)"><rect x="0" y="0" width="100" height="45" fill="#000"/></g>
<g clip-path="url(#star)"><rect x="40" y="45" width="60" height="55" fill="#000"/></g>
</svg>"##;

#[test]
fn imported_multi_shape_and_even_odd_clip_paths_clip() {
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "clips.svg", "dataBase64": vectorcraft_format::base64_encode(SVG.as_bytes())})).unwrap();
    let d = &s.doc().unwrap().doc;
    let mut kinds = vec![];
    d.walk(|n| {
        if let NodeKind::Group { children, clip: true } = &n.kind {
            kinds.push(children[0].kind_label());
        }
    });
    assert_eq!(kinds.len(), 2, "two clip groups");
    let r = render(&s);
    assert!(opaque(&r, 15, 20) && opaque(&r, 75, 20), "inside both shapes of the clip path");
    assert!(!opaque(&r, 45, 20), "between them");
    assert!(!opaque(&r, 15, 43), "below them");
    // The star's arms cross: the centre of the crossing is outside under even-odd.
    assert!(opaque(&r, 70, 56), "an arm");
    assert!(!opaque(&r, 70, 75), "the even-odd centre");
    assert_outputs_agree(&mut s, &[(15, 20), (75, 20), (45, 20), (15, 43), (70, 56), (70, 75)]);
}

/// The children of node `g`, bottom first.
fn child_ids(s: &Session, g: NodeId) -> Vec<NodeId> {
    s.doc().unwrap().doc.children(Some(g)).unwrap().iter().map(|n| n.id).collect()
}

/// The clip group's clipping path, by its mark or place.
fn clips_by(s: &Session, g: NodeId) -> Option<NodeId> {
    s.doc().unwrap().doc.node(g).unwrap().clipping_path().map(|c| c.id)
}

#[test]
fn arranging_inside_a_clip_group_moves_the_art_and_the_clipping_path_and_the_clip_stays() {
    // #3188: Send to Back on clipped art made it the clipping path, and the group showed nothing.
    // Arrange now puts art below the clipping path and the clipping path over the art, and the
    // clip group still clips by the same path.
    let mut s = session();
    let rect =
        |s: &mut Session, x: f64, y: f64, w: f64| id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": w})).unwrap());
    let a = rect(&mut s, 0.0, 0.0, 50.0);
    let b = rect(&mut s, 50.0, 50.0, 50.0);
    let clip = ellipse(&mut s, 20.0, 60.0);
    select(&mut s, &[a, b, clip]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    assert_eq!(child_ids(&s, g), [a, b, clip], "the clipping path on top");
    let send = |s: &mut Session, ids: &[NodeId], cmd: &str| {
        select(s, ids);
        s.execute(cmd, &json!({})).unwrap();
    };

    let clipped = |s: &Session| {
        let r = render(s);
        // Both pieces of art show inside the clip, and nothing outside it.
        opaque(&r, 40, 40) && opaque(&r, 60, 60) && !opaque(&r, 5, 5)
    };
    assert!(clipped(&s));

    send(&mut s, &[b], "object.arrange.sendToBack");
    assert_eq!(child_ids(&s, g), [b, a, clip], "the art goes to the back");
    assert_eq!(clips_by(&s, g), Some(clip));
    assert!(clipped(&s));
    send(&mut s, &[clip], "object.arrange.sendToBack");
    assert_eq!(child_ids(&s, g), [clip, b, a], "the clipping path goes under the art");
    assert_eq!(clips_by(&s, g), Some(clip));
    assert!(clipped(&s));
    send(&mut s, &[clip], "object.arrange.bringForward");
    assert_eq!(child_ids(&s, g), [b, clip, a]);
    send(&mut s, &[a, b], "object.arrange.bringToFront");
    assert_eq!(child_ids(&s, g), [clip, b, a], "several pieces of art keep their order");
    send(&mut s, &[clip, a], "object.arrange.bringToFront");
    assert_eq!(child_ids(&s, g), [b, clip, a], "the clipping path and art move together");
    assert_eq!(clips_by(&s, g), Some(clip));
    assert!(clipped(&s));
}

/// Make Clipping Mask marks the clipping path whatever its kind (a compound path and type too, which
/// had no mark), so that the clip group knows it wherever it is; Release, Ungroup, Deep Ungroup and
/// the Layers panel's button take the mark off.
#[test]
fn the_clipping_path_is_marked_whatever_its_kind_until_the_clip_goes() {
    let marked = |s: &Session, id: NodeId| s.doc().unwrap().doc.node(id).unwrap().is_clipping_path();
    let clips_by = |s: &Session, g: NodeId| s.doc().unwrap().doc.node(g).unwrap().clipping_path().map(|c| c.id);
    // A compound path, then Release.
    let mut s = session();
    let (art, donut) = art_and_donut(&mut s);
    select(&mut s, &[art, donut]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    assert!(marked(&s, donut) && !marked(&s, art));
    assert_eq!(clips_by(&s, g), Some(donut));
    select(&mut s, &[g]);
    s.execute("object.clippingMask.release", &json!({})).unwrap();
    assert!(!marked(&s, donut), "Release takes it off");
    // Type, then Ungroup.
    let mut s = session();
    let art = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let t = id_of(&s.execute("text.create", &json!({"x": 5, "y": 80, "text": "O", "size": 90})).unwrap());
    select(&mut s, &[art, t]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    assert!(marked(&s, t) && !marked(&s, art));
    assert_eq!(clips_by(&s, g), Some(t));
    select(&mut s, &[g]);
    s.execute("object.ungroup", &json!({})).unwrap();
    assert!(!marked(&s, t), "Ungroup takes it off");
    // A compound path, then Deep Ungroup with its clip groups.
    let mut s = session();
    let (art, donut) = art_and_donut(&mut s);
    select(&mut s, &[art, donut]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    s.execute("addon.cleanup.deepUngroup", &json!({"includeClipGroups": true, "ids": [g.0]})).unwrap();
    assert!(!marked(&s, donut), "Deep Ungroup takes it off");
    // The Layers panel's button: the layer's top object, a compound path, clips the layer.
    let mut s = session();
    let (_, donut) = art_and_donut(&mut s);
    let layer = s.doc().unwrap().doc.layers[0].id;
    s.execute("layer.clippingMask.toggle", &json!({"id": layer.0})).unwrap();
    assert!(marked(&s, donut));
    assert_eq!(clips_by(&s, layer), Some(donut));
    s.execute("layer.clippingMask.toggle", &json!({"id": layer.0})).unwrap();
    assert!(!marked(&s, donut), "releasing the layer's clip takes it off");
}

/// A clip group finds its clipping path by the mark, not by its place: moved to the top of its
/// group, the ring still clips the art as before, on screen, in clicks, in its bounds and in the
/// SVG and PDF output.
#[test]
fn a_marked_clipping_path_clips_wherever_it_sits_in_its_group() {
    let mut s = session();
    let (art, donut) = art_and_donut(&mut s);
    select(&mut s, &[art, donut]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    let bounds = |s: &Session| s.doc().unwrap().doc.node(g).unwrap().geometric_bounds();
    let before = bounds(&s);
    // `node.move` puts a row anywhere, the bottom of a clip group included.
    s.execute("node.move", &json!({"id": donut.0, "parent": g.0, "index": 1})).unwrap();
    assert_eq!(child_ids(&s, g), [art, donut]);
    let r = render(&s);
    assert!(opaque(&r, 20, 50), "the ring shows the art");
    assert!(!opaque(&r, 50, 50), "the hole stays transparent");
    assert!(!opaque(&r, 3, 3), "outside the ring");
    assert_eq!(bounds(&s), before);
    assert_eq!(hit(&s, 20.0, 50.0), Some(art));
    assert_eq!(hit(&s, 50.0, 50.0), None, "the hole");
    // The SVG and PDF drawings (the PDF without the native document, which reopens in its place).
    for (format, options) in [("svg", json!({})), ("pdf", json!({"preserveEditing": false}))] {
        let mut params = json!({"format": format});
        params.as_object_mut().unwrap().extend(options.as_object().unwrap().clone());
        let data = s.execute("document.export", &params).unwrap()["dataBase64"].clone();
        let mut o = Session::new();
        o.execute("document.open", &json!({"name": format!("clip.{format}"), "dataBase64": data})).unwrap();
        let out = render(&o);
        for (x, y) in [(20, 50), (50, 50), (3, 3), (80, 50)] {
            assert_eq!(opaque(&out, x, y), opaque(&r, x, y), "{format} at ({x}, {y})");
        }
    }
}

/// The donut clip of [`art_and_donut`] shows: the art in the ring, nothing in the hole or outside.
fn shows_the_ring(s: &Session) -> bool {
    let r = render(s);
    opaque(&r, 20, 50) && !opaque(&r, 50, 50) && !opaque(&r, 3, 3)
}

/// The clipping path stays the clipping path wherever it goes among the members of its clip group
/// (the Layers panel's moves), and so does one without a mark, first among the members (as a clip
/// built in code has it), when art goes below it.
#[test]
fn the_clipping_path_stays_the_clipping_path_wherever_it_goes_among_the_members() {
    let mut s = session();
    let (art, donut) = art_and_donut(&mut s);
    select(&mut s, &[art, donut]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    assert!(shows_the_ring(&s));
    s.execute("layer.move", &json!({"ids": [donut.0], "target": art.0, "place": "below"})).unwrap();
    assert_eq!(child_ids(&s, g), [donut, art], "under the art");
    assert_eq!(clips_by(&s, g), Some(donut));
    assert!(shows_the_ring(&s));
    s.execute("layer.move", &json!({"ids": [donut.0], "target": art.0, "place": "above"})).unwrap();
    assert_eq!(child_ids(&s, g), [art, donut], "back over it");
    assert!(shows_the_ring(&s));

    // Without its mark, first: the clip clips by its first member.
    s.execute("layer.move", &json!({"ids": [donut.0], "target": art.0, "place": "below"})).unwrap();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(donut).unwrap().set_clipping_path(false);
    assert_eq!(clips_by(&s, g), Some(donut));
    // Art moved below it: it stays the clipping path, now marked.
    s.execute("layer.move", &json!({"ids": [art.0], "target": donut.0, "place": "below"})).unwrap();
    assert_eq!(child_ids(&s, g), [art, donut]);
    assert_eq!(clips_by(&s, g), Some(donut));
    assert!(s.doc().unwrap().doc.node(donut).unwrap().is_clipping_path());
    assert!(shows_the_ring(&s));
}

/// The ids of the members of object `id` in saved native JSON `v`, in the file's order.
fn saved_members(v: &Value, id: NodeId) -> Vec<u64> {
    fn find(v: &Value, id: u64) -> Option<&Value> {
        match v {
            Value::Object(o) if o.get("id").and_then(Value::as_u64) == Some(id) && o.contains_key("kind") => Some(v),
            Value::Object(o) => o.values().find_map(|c| find(c, id)),
            Value::Array(a) => a.iter().find_map(|c| find(c, id)),
            _ => None,
        }
    }
    fn children(v: &Value) -> Option<&Vec<Value>> {
        let o = v.as_object()?;
        o.get("children").and_then(Value::as_array).or_else(|| o.get("kind").and_then(children))
    }
    children(find(v, id.0).unwrap()).unwrap().iter().map(|c| c["id"].as_u64().unwrap()).collect()
}

/// Make Clipping Mask leaves the clipping path on top of the clip group. A file keeps it first,
/// under the art, wherever it is in the document (a compound path too, whose mark files don't keep),
/// and opening the file puts it back on top, marked: the clip clips as it did. A document opened
/// from another format has its clipping paths on top too.
#[test]
fn the_clipping_path_is_on_top_in_a_document_and_first_in_its_file() {
    let mut s = session();
    let (art, donut) = art_and_donut(&mut s);
    let small = id_of(&s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap());
    select(&mut s, &[donut]);
    s.execute("object.arrange.bringToFront", &json!({})).unwrap();
    select(&mut s, &[art, small, donut]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    assert_eq!(child_ids(&s, g), [art, small, donut], "the clipping path stays on top");
    // Moved between the art.
    s.execute("layer.move", &json!({"ids": [donut.0], "target": small.0, "place": "below"})).unwrap();
    assert_eq!(child_ids(&s, g), [art, donut, small]);
    assert!(shows_the_ring(&s));
    let saved: Value = serde_json::from_slice(&vectorcraft_format::save(&s.doc().unwrap().doc, true)).unwrap();
    assert_eq!(saved_members(&saved, g), [donut.0, art.0, small.0], "first in the file");
    assert_eq!(child_ids(&s, g), [art, donut, small], "the document stays as it is");

    // Opened again: on top, marked.
    let data = s.execute("document.export", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"].clone();
    let mut o = Session::new();
    o.execute("document.open", &json!({"name": "clip.vectorcraft", "dataBase64": data})).unwrap();
    assert_eq!(child_ids(&o, g), [art, small, donut]);
    assert_eq!(clips_by(&o, g), Some(donut));
    assert!(o.doc().unwrap().doc.node(donut).unwrap().is_clipping_path());
    assert!(shows_the_ring(&o));

    // From SVG.
    let data = s.execute("document.export", &json!({"format": "svg"})).unwrap()["dataBase64"].clone();
    let mut o = Session::new();
    o.execute("document.open", &json!({"name": "clip.svg", "dataBase64": data})).unwrap();
    let mut clips = vec![];
    o.doc().unwrap().doc.walk(|n| {
        if n.clips() {
            clips.push(n.clone());
        }
    });
    assert!(!clips.is_empty());
    for c in clips {
        let last = c.children().unwrap().len() - 1;
        assert_eq!(c.clipping_path_index(), Some(last), "on top");
        assert!(c.children().unwrap()[last].is_clipping_path(), "marked");
    }
    assert!(shows_the_ring(&o));
}

/// Deleting the clipping path, or moving it out of its clip group (into the layer, or into a group
/// made of it and art inside the clip), releases the clip: no other member becomes the clipping
/// path, and the art shows whole. The path moved out is a plain path, and stays one when it goes
/// back. The same for a clipping layer.
#[test]
fn deleting_the_clipping_path_or_moving_it_out_releases_the_clip() {
    let clips = |s: &Session, id: NodeId| s.doc().unwrap().doc.node(id).unwrap().clips();
    let marked = |s: &Session, id: NodeId| s.doc().unwrap().doc.node(id).unwrap().is_clipping_path();
    let whole = |s: &Session| {
        let r = render(s);
        opaque(&r, 3, 3) && opaque(&r, 50, 50)
    };
    let mut s = session();
    let (art, donut) = art_and_donut(&mut s);
    let top = id_of(&s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap());
    select(&mut s, &[donut]);
    s.execute("object.arrange.bringToFront", &json!({})).unwrap();
    select(&mut s, &[art, top, donut]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    assert!(clips(&s, g) && !whole(&s));

    // Deleted.
    s.execute("edit.clear", &json!({"ids": [donut.0]})).unwrap();
    assert!(!clips(&s, g), "released");
    assert_eq!(child_ids(&s, g), [art, top]);
    assert!(!marked(&s, art) && !marked(&s, top));
    assert!(whole(&s));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(clips(&s, g) && shows_the_ring(&s));

    // Moved out into the layer, then back in.
    s.execute("layer.move", &json!({"ids": [donut.0], "target": g.0, "place": "above"})).unwrap();
    assert!(!clips(&s, g), "released");
    assert!(!marked(&s, donut), "a plain path");
    assert!(whole(&s));
    s.execute("layer.move", &json!({"ids": [donut.0], "target": g.0, "place": "inside"})).unwrap();
    assert!(!clips(&s, g) && !marked(&s, donut), "back in, still released");
    assert!(whole(&s));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(clips(&s, g) && marked(&s, donut));

    // `node.move` out.
    let layer = s.doc().unwrap().doc.layers[0].id;
    s.execute("node.move", &json!({"id": donut.0, "parent": layer.0})).unwrap();
    assert!(!clips(&s, g) && !marked(&s, donut));
    s.execute("edit.undo", &json!({})).unwrap();

    // Grouped with art inside the clip.
    select(&mut s, &[top, donut]);
    let inner = id_of(&s.execute("object.group", &json!({})).unwrap());
    assert_eq!(s.doc().unwrap().doc.parent_of(inner), Some(g));
    assert!(!clips(&s, g) && !marked(&s, donut));
    assert!(whole(&s));

    // A clipping layer.
    let mut s = session();
    let art = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let clip = id_of(&s.execute("shape.rectangle", &json!({"x": 25, "y": 25, "width": 50, "height": 50})).unwrap());
    s.execute("select.none", &json!({})).unwrap();
    s.execute("layer.clippingMask.toggle", &json!({})).unwrap();
    let layer = s.doc().unwrap().doc.layers[0].id;
    assert!(clips(&s, layer) && !whole(&s));
    s.execute("edit.clear", &json!({"ids": [clip.0]})).unwrap();
    assert!(!clips(&s, layer), "released");
    assert!(!marked(&s, art));
    assert!(whole(&s));
}

/// A copy of the clipping path alone is a plain object: pasted on its own, or into its clip group
/// behind it, where the clip keeps its clipping path.
#[test]
fn a_copy_of_the_clipping_path_alone_is_a_plain_object() {
    let marked = |s: &Session, id: NodeId| s.doc().unwrap().doc.node(id).unwrap().is_clipping_path();
    let pasted = |v: Value| NodeId(v["ids"][0].as_u64().unwrap());
    let mut s = session();
    let (art, donut) = art_and_donut(&mut s);
    select(&mut s, &[art, donut]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    select(&mut s, &[donut]);
    s.execute("edit.copy", &json!({})).unwrap();
    let copy = pasted(s.execute("edit.paste", &json!({})).unwrap());
    assert!(!marked(&s, copy));
    select(&mut s, &[donut]);
    let behind = pasted(s.execute("edit.pasteInBack", &json!({})).unwrap());
    assert_eq!(s.doc().unwrap().doc.parent_of(behind), Some(g));
    assert!(!marked(&s, behind));
    assert_eq!(clips_by(&s, g), Some(donut));
    assert!(shows_the_ring(&s));
}

/// `document.inspect` and `document.node` name a clip's clipping path wherever it is among the
/// members: the clip group reports its id as `clippingPathId`, and its own entry
/// `clippingPath: true`, as the Layers panel underlines its name.
#[test]
fn inspect_names_the_clipping_path_wherever_it_is() {
    let mut s = session();
    let (art, donut) = art_and_donut(&mut s);
    let small = id_of(&s.execute("shape.rectangle", &json!({"x": 40, "y": 40, "width": 20, "height": 20})).unwrap());
    select(&mut s, &[donut]);
    s.execute("object.arrange.bringToFront", &json!({})).unwrap();
    select(&mut s, &[art, small, donut]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    s.execute("layer.move", &json!({"ids": [donut.0], "target": small.0, "place": "below"})).unwrap();
    assert_eq!(child_ids(&s, g), [art, donut, small], "between the art");
    fn find(v: &Value, id: NodeId) -> Option<&Value> {
        match v {
            Value::Object(o) if o.get("id").and_then(Value::as_u64) == Some(id.0) && o.contains_key("kind") => Some(v),
            Value::Object(o) => o.values().find_map(|c| find(c, id)),
            Value::Array(a) => a.iter().find_map(|c| find(c, id)),
            _ => None,
        }
    }
    let v = s.execute("document.inspect", &json!({})).unwrap();
    let group = find(&v["layers"], g).unwrap();
    assert_eq!(group["clippingPathId"], json!(donut.0));
    let flagged: Vec<u64> =
        group["children"].as_array().unwrap().iter().filter(|c| c["clippingPath"] == json!(true)).map(|c| c["id"].as_u64().unwrap()).collect();
    assert_eq!(flagged, [donut.0], "only the clipping path says so");
    // One node at a time, and listed by id.
    let node = |s: &mut Session, id: NodeId| s.execute("document.node", &json!({"id": id.0, "summary": true})).unwrap();
    assert_eq!(node(&mut s, donut)["clippingPath"], json!(true));
    assert!(node(&mut s, art).get("clippingPath").is_none());
    let listed = s.execute("document.inspect", &json!({"ids": [donut.0, small.0]})).unwrap();
    let nodes = listed["nodes"].as_array().unwrap();
    assert_eq!((nodes[0]["clippingPath"].clone(), nodes[1].get("clippingPath")), (json!(true), None));
    // Released: neither.
    select(&mut s, &[g]);
    s.execute("object.clippingMask.release", &json!({})).unwrap();
    let v = s.execute("document.inspect", &json!({})).unwrap();
    let group = find(&v["layers"], g).unwrap();
    assert!(group.get("clippingPathId").is_none());
    assert!(group["children"].as_array().unwrap().iter().all(|c| c.get("clippingPath").is_none()));
}

/// A half transparent image whose warp envelope is expanded into pieces, each a clip group of a
/// triangle and the image, saved and opened again: each triangle goes on top of its image, and the
/// pieces are still found as the pieces of one image. The SVG output keeps masking where they
/// overlap, so the image keeps its alpha there, and draws one copy of the image, as before saving.
#[test]
fn mesh_warp_pieces_saved_and_opened_again_are_still_pieces_of_one_image() {
    let img = craft_image_codecs::Image::from_raw(
        20,
        20,
        craft_image_codecs::ChannelLayout::Rgba,
        craft_image_codecs::SampleType::U8,
        [255, 0, 0, 128].repeat(400),
    )
    .unwrap();
    let png = craft_image_codecs::encode(&img, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default()).unwrap();
    let mut s = session();
    s.execute("file.place", &json!({"name": "half.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [50, 50]})).unwrap();
    s.execute("object.envelope.makeWithWarp", &json!({"style": "arc", "bend": 50})).unwrap();
    s.execute("object.envelope.expand", &json!({})).unwrap();
    let svg = |s: &mut Session| {
        let v = s.execute("document.export", &json!({"format": "svg"})).unwrap();
        String::from_utf8(vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()).unwrap()
    };
    let before = svg(&mut s);
    let masks = before.matches("<mask").count();
    assert!(masks > 0, "the overlaps are masked");
    assert_eq!(before.matches("data:image/png;base64,").count(), 1);

    let data = s.execute("document.export", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"].clone();
    let mut o = Session::new();
    o.execute("document.open", &json!({"name": "warp.vectorcraft", "dataBase64": data})).unwrap();
    let mut pieces = vec![];
    o.doc().unwrap().doc.walk(|n| {
        if n.extras().warp_piece {
            pieces.push(n.clone());
        }
    });
    assert!(pieces.len() > 1);
    for p in &pieces {
        assert_eq!(p.clipping_path_index(), Some(1), "the triangle on top");
        assert!(p.warp_piece_image().is_some(), "the image found below it");
    }
    let after = svg(&mut o);
    assert_eq!(after.matches("<mask").count(), masks, "still masked");
    assert_eq!(after.matches("data:image/png;base64,").count(), 1, "still one copy");
}

/// Type on top of its art as the clipping path, in a clip group with a geometry effect of its own:
/// the type's outlines take its place and still clip, on the canvas and in the SVG and PDF output.
#[test]
fn type_on_top_still_clips_when_its_clip_groups_effect_outlines_it() {
    let mut s = session();
    let art = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let t = id_of(&s.execute("text.create", &json!({"x": 5, "y": 80, "text": "O", "size": 90})).unwrap());
    let b = s.doc().unwrap().doc.node(t).unwrap().geometric_bounds().unwrap();
    select(&mut s, &[art, t]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    s.execute("effect.apply", &json!({"effect": "distort.transform", "params": {"moveH": 1}, "ids": [g.0], "item": null})).unwrap();
    let cy = ((b.y0 + b.y1) / 2.0) as u32;
    let r = render(&s);
    assert!(b.x1 < 94.0, "{b:?}");
    assert!(!opaque(&r, (b.x1 + 4.0) as u32, cy), "beside the type, the art is clipped away");
    assert!(!opaque(&r, ((b.x0 + b.x1) / 2.0 + 1.0) as u32, cy), "in the counter too");
    let probes: Vec<(u32, u32)> = (0..100).step_by(3).map(|x| (x, cy)).collect();
    assert_outputs_agree(&mut s, &probes);
}

/// A clip group whose Transform effect makes a copy of its clipping path, on top of the art: the
/// copies, a compound path, still clip, and the art shows only inside the circle and its copy.
#[test]
fn a_clipping_path_on_top_still_clips_when_a_transform_effect_copies_it() {
    let mut s = session();
    let art = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    let circle = ellipse(&mut s, 10.0, 30.0);
    select(&mut s, &[art, circle]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    s.execute("effect.apply", &json!({"effect": "distort.transform", "params": {"moveH": 50, "copies": 1}, "ids": [g.0], "item": null})).unwrap();
    let r = render(&s);
    assert!(opaque(&r, 25, 25) && opaque(&r, 75, 25), "inside the circle and its copy");
    assert!(!opaque(&r, 25, 75) && !opaque(&r, 50, 25), "elsewhere the art is clipped away");
}

/// Flattening transparency in a clip group whose clipping path is type on top of the art: the
/// type's outlines still clip the flattened art.
#[test]
fn type_on_top_still_clips_when_its_clip_group_is_flattened() {
    let mut s = session();
    let art = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap());
    s.execute("transparency.set", &json!({"ids": [art.0], "opacity": 70})).unwrap();
    let t = id_of(&s.execute("text.create", &json!({"x": 5, "y": 80, "text": "O", "size": 90})).unwrap());
    let b = s.doc().unwrap().doc.node(t).unwrap().geometric_bounds().unwrap();
    select(&mut s, &[art, t]);
    let g = id_of(&s.execute("object.clippingMask.make", &json!({})).unwrap());
    let cy = ((b.y0 + b.y1) / 2.0) as u32;
    let ring = |r: &Rendered| (b.x0 as u32..b.x1 as u32).any(|x| opaque(r, x, cy));
    let before = render(&s);
    assert!(ring(&before) && !opaque(&before, (b.x1 + 4.0) as u32, cy));
    s.execute("object.flattenTransparency", &json!({"ids": [g.0]})).unwrap();
    let after = render(&s);
    assert!(ring(&after), "the ring shows the art");
    assert!(!opaque(&after, (b.x1 + 4.0) as u32, cy), "beside the type, the art is clipped away");
    assert!(!opaque(&after, ((b.x0 + b.x1) / 2.0) as u32, cy), "in the counter too");
}
